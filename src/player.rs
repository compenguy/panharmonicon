use std::fs::File;
use std::io::{BufReader, Read, Seek};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use log::{debug, error, info, trace, warn};
use rodio::cpal::traits::{DeviceTrait, HostTrait};
use rodio::{cpal, cpal::FromSample};
use rodio::{Sample, Source};

use crate::messages::{Request, State, StopReason};
use crate::model::{RequestSender, StateReceiver};
use crate::track::Track;

#[derive(Debug, Clone, Copy)]
enum Volume {
    Muted,
    Unmuted(f32),
}

impl Volume {
    fn volume(self) -> f32 {
        if let Self::Unmuted(v) = self {
            v.min(1.0f32).max(0.0f32)
        } else {
            0.0f32
        }
    }

    fn set_volume(&mut self, new_volume: f32) {
        *self = Self::Unmuted(new_volume.min(1.0f32).max(0.0f32));
    }

    fn muted(self) -> bool {
        match self {
            Self::Muted => true,
            Self::Unmuted(_) => false,
        }
    }

    fn mute(&mut self) {
        *self = Self::Muted;
    }

    fn unmute(&mut self) {
        let volume = self.volume();
        *self = Self::Unmuted(volume);
    }
}

impl Default for Volume {
    fn default() -> Self {
        Self::Unmuted(1.0f32)
    }
}

// We can't derive Debug or Clone since the rodio members
// don't implement it
struct AudioDevice {
    device: cpal::Device,
    // If the stream gets dropped, the device (handle) closes
    // so we hold it, but we don't ever use it
    _stream: rodio::OutputStream,
    handle: rodio::OutputStreamHandle,
    sink: rodio::Sink,
    volume: Volume,
}

impl AudioDevice {
    pub(crate) fn new(volume: f32) -> Self {
        let device = cpal::default_host()
            .default_output_device()
            .expect("Failed to locate default audio device");
        let (_stream, handle) = rodio::OutputStream::try_from_device(&device)
            .expect("Failed to initialize audio device for playback");
        let sink =
            rodio::Sink::try_new(&handle).expect("Failed to initialize audio device for playback");
        Self {
            device,
            _stream,
            handle,
            sink,
            volume: Volume::Unmuted(volume),
        }
    }

    fn play_m4a_from_path<P>(&mut self, path: P) -> Result<()>
    where
        P: AsRef<Path>,
    {
        trace!(
            "Creating decoder for track at {} for playback",
            path.as_ref().to_string_lossy()
        );
        let file = File::open(path.as_ref()).with_context(|| {
            format!(
                "Failed opening media file at {}",
                path.as_ref().to_string_lossy()
            )
        })?;
        let metadata = file.metadata().with_context(|| {
            format!(
                "Failed retrieving metadata for media file at {}",
                path.as_ref().to_string_lossy()
            )
        })?;
        let decoder = self.m4a_decoder_for_reader(file, metadata.len())?;
        self.play_from_source(decoder)
    }

    fn m4a_decoder_for_reader<R: Read + Seek + Send + 'static>(
        &mut self,
        reader: R,
        size: u64,
    ) -> Result<redlux::Decoder<BufReader<R>>> {
        let reader = BufReader::new(reader);
        redlux::Decoder::new_mpeg4(reader, size).context("Failed initializing media decoder")
    }

    /*
    fn play_from_source(
        &mut self,
        source: redlux::Decoder<BufReader<std::fs::File>>,
    ) -> Result<()> {
        self.reset();

        let start_paused = false;
        self.sink.append(source.pausable(start_paused));
        self.sink.play();
        Ok(())
    }
    */

    fn play_from_source<S>(&mut self, source: S) -> Result<()>
    where
        S: Source + Send + 'static,
        f32: FromSample<S::Item>,
        S::Item: Sample + Send,
    {
        self.reset();

        let start_paused = false;
        self.sink.append(source.pausable(start_paused));
        self.sink.play();
        Ok(())
    }

    fn reset(&mut self) {
        self.sink = rodio::Sink::try_new(&self.handle)
            .expect("Failed to initialize audio device for playback");
        self.sink.set_volume(self.volume.volume());
    }

    fn active(&self) -> bool {
        !self.sink.empty()
    }

    fn paused(&self) -> bool {
        self.sink.is_paused()
    }

    fn pause(&mut self) {
        self.sink.pause();
    }

    fn unpause(&mut self) {
        self.sink.play()
    }

    fn volume(&self) -> f32 {
        self.volume.volume()
    }

    fn set_volume(&mut self, new_volume: f32) {
        self.volume.set_volume(new_volume);
        self.refresh_volume();
    }

    fn refresh_volume(&mut self) {
        self.sink.set_volume(self.volume.volume());
    }

    fn muted(&self) -> bool {
        self.volume.muted()
    }

    fn mute(&mut self) {
        self.volume.mute();
        self.refresh_volume();
    }

    fn unmute(&mut self) {
        self.volume.unmute();
        self.refresh_volume();
    }
}

impl Clone for AudioDevice {
    fn clone(&self) -> Self {
        // Since we can't clone the device, we're going to look for the device
        // from the output devices list that has the same name as the our
        // current one.  If none matches, we'll use the default output device.
        let device = cpal::default_host()
            .devices()
            .map(|mut devs| devs.find(|d| d.name().ok() == self.device.name().ok()))
            .ok()
            .flatten()
            .unwrap_or_else(|| {
                cpal::default_host()
                    .default_output_device()
                    .expect("Failed to locate default audio device")
            });
        let (_stream, handle) = rodio::OutputStream::try_from_device(&device)
            .expect("Failed to initialize audio device for playback");
        let sink =
            rodio::Sink::try_new(&handle).expect("Failed to initialize audio device for playback");

        AudioDevice {
            device,
            _stream,
            handle,
            sink,
            volume: self.volume,
        }
    }
}

impl std::fmt::Debug for AudioDevice {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let queued = format!("{} queued", self.sink.len());
        let paused = if self.sink.is_paused() {
            "paused"
        } else {
            "not paused"
        };

        // rodio, around version 0.12, stopped making attributes of the
        // underlying audio device available, so we can't report anything
        // about it
        write!(
            f,
            "AudioDevice {{ sink: ({}, {}, volume {:.2}), volume: {:?} }}",
            queued,
            paused,
            self.sink.volume(),
            self.volume
        )
    }
}

impl Default for AudioDevice {
    fn default() -> Self {
        let device = cpal::default_host()
            .default_output_device()
            .expect("Failed to locate default audio device");
        let (_stream, handle) = rodio::OutputStream::try_from_device(&device)
            .expect("Failed to initialize audio device for playback");
        let sink =
            rodio::Sink::try_new(&handle).expect("Failed to initialize audio device for playback");
        Self {
            device,
            _stream,
            handle,
            sink,
            volume: Volume::default(),
        }
    }
}

#[derive(Debug, Clone)]
pub(crate) struct Player {
    active_track: Option<Track>,
    audio_device: AudioDevice,
    last_started: Option<Instant>,
    elapsed: Duration,
    duration: Duration,
    elapsed_polled: Option<Duration>,
    request_sender: RequestSender,
    state_receiver: StateReceiver,
    dirty: bool,
}

impl Player {
    pub(crate) fn new(state_receiver: StateReceiver, request_sender: RequestSender) -> Self {
        Self {
            active_track: None,
            audio_device: AudioDevice::new(0.0),
            last_started: None,
            elapsed: Duration::default(),
            duration: Duration::default(),
            elapsed_polled: None,
            request_sender,
            state_receiver,
            dirty: false,
        }
    }

    fn publish_request(&mut self, request: Request) -> Result<()> {
        self.request_sender.send(request)?;
        Ok(())
    }

    fn start(&mut self, track: &Track) -> Result<()> {
        if let Some(active_track) = &self.active_track {
            if active_track.track_token == track.track_token {
                warn!("The requested track is already playing");
                return Ok(());
            } else {
                info!("New track requested while track already playing. Stopping current track...");
                self.stop();
            }
        }
        self.active_track = Some(track.clone());
        debug!("Starting track: {:?}", track.song_name);
        if let Some(cached) = track.cached.as_ref() {
            trace!("Starting decoding of track {}", cached.display());
            self.audio_device
                .play_m4a_from_path(PathBuf::from(&cached))
                .with_context(|| format!("Failed to start track at {}", cached.display()))?;
            self.duration = track.track_length;

            self.last_started = Some(Instant::now());
            self.dirty |= true;
        } else {
            error!("Uncached track! Stopping...");
            self.stop();
        }
        Ok(())
    }

    fn started(&self) -> bool {
        assert!(
            !(self.active() && self.elapsed() == Duration::default()),
            "Application state error: audio device is active, but no track playtime has elapsed."
        );
        self.active()
    }

    fn stop(&mut self) {
        if self.elapsed().as_millis() > 0 {
            self.reset();
            self.last_started = None;
            self.elapsed = Duration::default();
            self.duration = Duration::default();
            self.dirty |= true;
        }
    }

    fn stopped(&self) -> bool {
        assert!(
            !(self.active() && self.elapsed() == Duration::default()),
            "Application state error: audio device is active, but no track playtime has elapsed."
        );
        !self.active()
    }

    fn playing(&self) -> Option<&Track> {
        if self.elapsed() > Duration::default() {
            self.active_track.as_ref()
        } else {
            None
        }
    }

    fn elapsed(&self) -> Duration {
        let elapsed_since_last_started = self.last_started.map(|i| i.elapsed()).unwrap_or_default();
        self.elapsed + elapsed_since_last_started
    }

    fn duration(&self) -> Duration {
        self.duration
    }

    fn check_playing(&mut self) -> Result<()> {
        if self.active_track.is_some() && !self.active() {
            // We were playing a track, but we've stopped
            if self.elapsed() >= self.duration {
                self.publish_request(Request::Stop(StopReason::TrackCompleted))?;
            } else {
                self.publish_request(Request::Stop(StopReason::TrackInterrupted))?;
            }
            self.stop();
        }
        Ok(())
    }

    pub(crate) async fn poll_progress(&mut self) -> Result<()> {
        let elapsed = self.elapsed();
        if self
            .elapsed_polled
            .map(|last| last.as_secs() != elapsed.as_secs())
            .unwrap_or(false)
        {
            self.elapsed_polled = Some(elapsed);
            self.dirty |= true;
            self.publish_request(Request::UpdateTrackProgress(elapsed))?;
        }
        Ok(())
    }

    fn reset(&mut self) {
        self.audio_device.reset();
        self.dirty |= true;
    }

    fn active(&self) -> bool {
        self.audio_device.active()
    }

    fn paused(&self) -> bool {
        // This returns true when a track has actually been started, but time
        // is not elapsing on it.
        assert!(
            !(self.audio_device.paused() && self.last_started.is_some()),
            "Application state error: track is paused, but track playtime still increasing."
        );
        self.audio_device.paused()
    }

    fn pause(&mut self) {
        self.elapsed += self
            .last_started
            .take()
            .map(|inst| inst.elapsed())
            .unwrap_or_default();
        self.audio_device.pause();
        self.dirty |= true;
    }

    fn unpause(&mut self) {
        if self.elapsed.as_millis() > 0 {
            self.last_started.get_or_insert_with(Instant::now);
            self.audio_device.unpause();
            self.dirty |= true;
        }
    }

    fn volume(&self) -> f32 {
        self.audio_device.volume()
    }

    fn set_volume(&mut self, new_volume: f32) {
        self.audio_device.set_volume(new_volume);
        self.dirty |= true;
    }

    fn muted(&self) -> bool {
        self.audio_device.muted()
    }

    fn mute(&mut self) {
        self.audio_device.mute();
        self.dirty |= true;
    }

    fn unmute(&mut self) {
        self.audio_device.unmute();
        self.dirty |= true;
    }

    pub(crate) async fn process_messages(&mut self) -> Result<bool> {
        while let Ok(msg) = self.state_receiver.try_recv() {
            match msg {
                State::Connected => self.stop(),
                State::Disconnected => self.stop(),
                State::TrackStarting(track) => self.start(&track)?,
                State::Volume(v) => self.set_volume(v),
                State::Playing(_) => self.unpause(),
                State::Paused(_) => self.pause(),
                State::Muted => self.mute(),
                State::Unmuted => self.unmute(),
                State::Stopped(_) => self.stop(),
                State::Quit => self.stop(),
                _ => (),
            }
            self.dirty |= true;
        }
        let dirty = self.dirty;
        self.dirty = false;
        Ok(dirty)
    }

    pub(crate) async fn update(&mut self) -> Result<bool> {
        self.check_playing()?;
        self.poll_progress().await?;
        self.process_messages().await
    }
}
