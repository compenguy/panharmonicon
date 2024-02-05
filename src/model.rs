use std::collections::{HashMap, VecDeque};
use std::fs::File;
use std::io::BufReader;
use std::io::{Read, Seek};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};
use std::{cell::RefCell, rc::Rc};

use anyhow::{Context, Result};
//use cpal::traits::{DeviceTrait, HostTrait};
use log::{debug, error, info, trace, warn};
use rodio::cpal::traits::{DeviceTrait, HostTrait};
use rodio::{cpal, cpal::FromSample};
use rodio::{Sample, Source};

use pandora_api::json::user::Station;

use crate::config::{Config, PartialConfig};
use crate::errors::Error;
use crate::pandora::PandoraSession;
use crate::track::Track;
use crate::{messages, messages::StopReason};

#[derive(Debug, Clone, Copy)]
enum Volume {
    Muted(f32),
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
            Self::Muted(_) => true,
            Self::Unmuted(_) => false,
        }
    }

    fn mute(&mut self) {
        let volume = self.volume();
        *self = Self::Muted(volume);
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
pub(crate) struct Playing {
    active_track: Track,
    audio_device: AudioDevice,
    last_started: Option<Instant>,
    elapsed: Duration,
    duration: Duration,
    elapsed_polled: Option<Duration>,
}

impl Playing {
    fn new(track: Track, volume: f32) -> Result<Self> {
        let mut s = Self {
            active_track: track,
            audio_device: AudioDevice::new(volume),
            last_started: None,
            elapsed: Duration::default(),
            duration: Duration::default(),
            elapsed_polled: None,
        };
        s.start()?;
        Ok(s)
    }

    fn start(&mut self) -> Result<()> {
        debug!("Starting track: {:?}", self.active_track.song_name);
        if let Some(cached) = self.active_track.cached.as_ref() {
            trace!("Starting decoding of track {}", cached.display());
            self.audio_device
                .play_m4a_from_path(PathBuf::from(&cached))
                .with_context(|| format!("Failed to start track at {}", cached.display()))?;
            self.duration = self.active_track.track_length;

            self.last_started = Some(Instant::now());
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
            Some(&self.active_track)
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

    pub(crate) fn poll_progress(&mut self) -> Option<(Duration, Duration)> {
        let elapsed = self.elapsed();
        if self.elapsed_polled != Some(elapsed) {
            self.elapsed_polled = Some(elapsed);
            Some((elapsed, self.duration()))
        } else {
            None
        }
    }

    fn reset(&mut self) {
        self.audio_device.reset();
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
    }

    fn unpause(&mut self) {
        if self.elapsed.as_millis() > 0 {
            self.last_started.get_or_insert_with(Instant::now);
            self.audio_device.unpause();
        }
    }

    fn volume(&self) -> f32 {
        self.audio_device.volume()
    }

    fn set_volume(&mut self, new_volume: f32) {
        self.audio_device.set_volume(new_volume)
    }

    fn muted(&self) -> bool {
        self.audio_device.muted()
    }

    fn mute(&mut self) {
        self.audio_device.mute();
    }

    fn unmute(&mut self) {
        self.audio_device.unmute();
    }
}

#[derive(Debug, Clone)]
pub(crate) enum ModelState {
    Disconnected,
    Connected {
        session: PandoraSession,
        reason: StopReason,
    },
    Tuned {
        session: PandoraSession,
        station_id: String,
        playlist: VecDeque<Track>,
    },
    Playing {
        session: PandoraSession,
        station_id: String,
        playlist: VecDeque<Track>,
        playing: Box<Playing>,
    },
    Quit,
    Invalid,
}

impl Default for ModelState {
    fn default() -> Self {
        Self::Disconnected
    }
}

impl std::fmt::Display for ModelState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Disconnected => write!(f, "Disconnected"),
            Self::Connected { reason, .. } => write!(f, "Connected ({reason})"),
            Self::Tuned {
                station_id,
                playlist,
                ..
            } => {
                write!(f, "Tuned {{ ")?;
                write!(f, "station id: {station_id}, ")?;
                write!(f, "playlist: [")?;
                let pl = playlist.iter().fold(String::new(), |mut a, b| {
                    a.reserve(b.song_name.len() + 1);
                    a.push_str(&b.song_name);
                    a.push_str(", ");
                    a
                });
                write!(f, "{}]", pl.trim_end_matches(", "))?;
                write!(f, "}}")
            }
            Self::Playing {
                station_id,
                playlist,
                playing,
                ..
            } => {
                write!(f, "Playing {{ ")?;
                write!(f, "station id: {station_id}, ")?;
                write!(
                    f,
                    "track: {}, ",
                    playing
                        .playing()
                        .map(|t| t.song_name.clone())
                        .unwrap_or_else(String::default)
                )?;
                write!(f, "playlist: [")?;
                let pl = playlist.iter().fold(String::new(), |mut a, b| {
                    a.reserve(b.song_name.len() + 1);
                    a.push_str(&b.song_name);
                    a.push_str(", ");
                    a
                });
                write!(f, "{}]", pl.trim_end_matches(", "))?;
                write!(f, "}}")
            }
            Self::Quit => write!(f, "Quit"),
            Self::Invalid => write!(f, "Invalid"),
        }
    }
}

impl ModelState {
    pub(crate) async fn connect(&mut self, mut session: PandoraSession) -> Result<()> {
        session.partner_login().await?;
        session.user_login().await?;
        trace!("changing state from '{}' to 'Connected'", self);
        *self = Self::Connected {
            session,
            reason: StopReason::Initializing,
        };
        Ok(())
    }

    pub(crate) fn connected(&self) -> bool {
        matches!(
            self,
            Self::Connected { .. } | Self::Tuned { .. } | Self::Playing { .. }
        )
    }

    pub(crate) fn disconnect(&mut self) {
        *self = Self::Disconnected;
    }

    fn test_connection(&mut self) -> bool {
        match self {
            Self::Connected { session, .. } if session.connected() => true,
            Self::Tuned { session, .. } if session.connected() => true,
            Self::Playing { session, .. } if session.connected() => true,
            _ => false,
        }
    }

    pub(crate) fn tune(&mut self, station_id: String) -> Result<()> {
        let self_name = self.to_string();
        let old = std::mem::replace(self, Self::Invalid);
        *self = match old {
            Self::Connected { session, .. } => {
                trace!("changing state from '{}' to 'Tuned'", self_name);
                Self::Tuned {
                    session,
                    station_id,
                    playlist: VecDeque::new(),
                }
            }
            Self::Tuned {
                session,
                station_id: cur_station_id,
                playlist,
                ..
            } => {
                if cur_station_id == station_id {
                    trace!("station '{}' already tuned", station_id);
                }
                Self::Tuned {
                    session,
                    station_id,
                    playlist,
                }
            }
            Self::Playing {
                session, playlist, ..
            } => {
                trace!("changing state from '{}' to 'Tuned'", self);
                Self::Tuned {
                    session,
                    station_id,
                    playlist,
                }
            }
            _ => {
                return Err(Error::InvalidOperationForState(String::from("tune"), self_name).into())
            }
        };
        Ok(())
    }

    pub(crate) fn untune(&mut self) -> Result<()> {
        let self_name = self.to_string();
        let old = std::mem::replace(self, Self::Invalid);
        *self = match old {
            Self::Tuned { session, .. } => {
                trace!("changing state from '{}' to 'Connected'", self_name);
                Self::Connected {
                    session,
                    reason: StopReason::Untuning,
                }
            }
            Self::Playing { session, .. } => {
                trace!("changing state from '{}' to 'Connected'", self_name);
                Self::Connected {
                    session,
                    reason: StopReason::Untuning,
                }
            }
            _ => {
                return Err(
                    Error::InvalidOperationForState(String::from("untune"), self_name).into(),
                )
            }
        };
        Ok(())
    }

    pub(crate) fn tuned(&self) -> Option<String> {
        match self {
            Self::Tuned { station_id, .. } => Some(station_id.clone()),
            Self::Playing { station_id, .. } => Some(station_id.clone()),
            _ => None,
        }
    }

    pub(crate) fn ready_next_track(&mut self, volume: f32) -> Result<Option<Track>> {
        let self_name = self.to_string();
        let old = std::mem::replace(self, Self::Invalid);
        match old {
            Self::Tuned {
                session,
                station_id,
                mut playlist,
                ..
            } => {
                if let Some(track) = playlist.pop_front() {
                    trace!("changing state from '{:?}' to 'Playing'", self_name);
                    *self = Self::Playing {
                        session,
                        station_id,
                        playlist,
                        playing: Box::new(Playing::new(track.clone(), volume)?),
                    };
                    Ok(Some(track))
                } else {
                    trace!("no tracks ready - not changing state");
                    *self = Self::Tuned {
                        session,
                        station_id,
                        playlist,
                    };
                    Ok(None)
                }
            }
            _ => Err(
                Error::InvalidOperationForState(String::from("ready_next_track"), self_name).into(),
            ),
        }
    }

    pub(crate) fn enqueue_track(&mut self, track: &Track) -> Result<()> {
        let self_name = self.to_string();
        track
            .cached
            .as_ref()
            .ok_or_else(|| Error::TrackNotCached(track.song_name.clone()))?;

        match self {
            Self::Tuned {
                playlist,
                station_id,
                ..
            } if station_id == &track.station_id => {
                playlist.push_back(track.clone());
                Ok(())
            }
            Self::Playing {
                playlist,
                station_id,
                ..
            } if station_id == &track.station_id => {
                playlist.push_back(track.clone());
                Ok(())
            }
            Self::Tuned { station_id, .. } if station_id != &track.station_id => {
                warn!("Invalid track for current station");
                Ok(())
            }
            Self::Playing { station_id, .. } if station_id != &track.station_id => {
                warn!("Invalid track for current station");
                Ok(())
            }
            _ => Err(
                Error::InvalidOperationForState(String::from("enqueue_track"), self_name).into(),
            ),
        }
    }

    pub(crate) fn playlist_len(&self) -> Result<usize> {
        match self {
            Self::Tuned { playlist, .. } => Ok(playlist.len()),
            Self::Playing { playlist, .. } => Ok(playlist.len()),
            _ => Err(Error::InvalidOperationForState(
                String::from("playlist_len"),
                self.to_string(),
            )
            .into()),
        }
    }

    pub(crate) fn stop(&mut self) -> Result<()> {
        let self_name = self.to_string();
        let old = std::mem::replace(self, Self::Invalid);
        match old {
            Self::Playing {
                session,
                station_id,
                playlist,
                ..
            } => {
                trace!("changing state from '{:?}' to 'Tuned'", self_name);
                *self = Self::Tuned {
                    session,
                    station_id,
                    playlist,
                };
                Ok(())
            }
            _ => {
                *self = old;
                Err(Error::InvalidOperationForState(String::from("stop"), self_name).into())
            }
        }
    }

    pub(crate) fn quit(&mut self) {
        trace!("changing state from '{:?}' to 'Quit'", self);
        *self = Self::Quit;
    }

    pub(crate) fn quitting(&self) -> bool {
        matches!(self, Self::Quit)
    }

    pub(crate) fn get_session_mut(&mut self) -> Option<&mut PandoraSession> {
        match self {
            Self::Connected {
                ref mut session, ..
            } => Some(session),
            Self::Tuned {
                ref mut session, ..
            } => Some(session),
            Self::Playing {
                ref mut session, ..
            } => Some(session),
            _ => None,
        }
    }

    pub(crate) fn get_next(&self) -> Option<&Track> {
        match self {
            Self::Tuned { playlist, .. } => playlist.get(0),
            Self::Playing { playlist, .. } => playlist.get(0),
            _ => None,
        }
    }

    pub(crate) fn get_playing(&self) -> Option<&Playing> {
        if let Self::Playing { playing, .. } = self {
            Some(playing)
        } else {
            None
        }
    }

    pub(crate) fn get_playing_mut(&mut self) -> Option<&mut Playing> {
        match self {
            Self::Playing {
                ref mut playing, ..
            } => Some(playing),
            _ => None,
        }
    }

    pub(crate) async fn rate_track(&mut self, station: &str, rating: Option<bool>) -> Result<()> {
        if let Self::Playing {
            session, playing, ..
        } = self
        {
            let new_rating_value: u32 = if rating.unwrap_or(false) { 1 } else { 0 };
            if let Some(rating) = rating {
                session
                    .add_feedback(station, &playing.active_track.track_token, rating)
                    .await?;
                playing.active_track.song_rating = new_rating_value;
                trace!(
                    "Rated track {} with value {}",
                    playing.active_track.song_name,
                    rating
                );
            } else {
                session
                    .delete_feedback_for_track(station, &playing.active_track)
                    .await?;
                playing.active_track.song_rating = new_rating_value;
                trace!("Successfully removed track rating.");
            }
            return Ok(());
        }
        Err(Error::InvalidOperationForState(String::from("rate_track"), self.to_string()).into())
    }

    pub(crate) async fn fetch_station_list(&mut self) -> Vec<Station> {
        if let Some(session) = self.get_session_mut() {
            if let Ok(list) = session.get_station_list().await {
                return list.stations;
            }
        }
        Vec::new()
    }

    pub(crate) async fn fetch_playlist(&mut self) -> Result<Vec<Track>> {
        if let ModelState::Tuned {
            session,
            station_id,
            ..
        } = self
        {
            let playlist = session.get_playlist(station_id).await.map(|pl| {
                pl.into_iter()
                    .filter_map(|pe| pe.get_track().map(|pt| pt.into()))
                    .collect()
            });
            trace!("Successfully fetched new playlist.");
            return playlist;
        }

        Err(
            Error::InvalidOperationForState(String::from("fetch_playlist"), self.to_string())
                .into(),
        )
    }
}

#[derive(Debug, Clone)]
pub(crate) struct Model {
    state: ModelState,
    config: Rc<RefCell<Config>>,
    station_list: HashMap<String, Station>,
    dirty: bool,
    channel_in: Option<async_broadcast::Receiver<messages::Request>>,
    channel_out: Option<async_broadcast::Sender<messages::Notification>>,
}

impl Model {
    pub(crate) fn new(config: Rc<RefCell<Config>>) -> Self {
        Self {
            state: ModelState::default(),
            config,
            station_list: HashMap::new(),
            dirty: true,
            channel_in: None,
            channel_out: None,
        }
    }

    pub(crate) fn init_request_channel(&mut self) -> async_broadcast::Sender<messages::Request> {
        let (s, r) = async_broadcast::broadcast(32);
        self.channel_in = Some(r);
        s
    }

    pub(crate) fn init_notification_channel(
        &mut self,
    ) -> async_broadcast::Receiver<messages::Notification> {
        let (s, r) = async_broadcast::broadcast(32);
        self.channel_out = Some(s);
        r
    }

    pub(crate) async fn process_messages(&mut self) -> Result<()> {
        while let Some(Ok(do_msg)) = self.channel_in.as_mut().map(|c| c.try_recv()) {
            trace!("received request {:?}", do_msg);
            match do_msg {
                messages::Request::Connect => self.connect().await?,
                messages::Request::Tune(s) => self.tune(s).await?,
                messages::Request::Untune => self.untune().await?,
                messages::Request::AddTrack(t) => self.add_track(t.as_ref())?,
                messages::Request::Stop => self.stop(Some(StopReason::UserRequest)).await?,
                messages::Request::Pause => self.pause(),
                messages::Request::Unpause => self.unpause(),
                messages::Request::TogglePause => self.toggle_pause(),
                messages::Request::Mute => self.mute().await?,
                messages::Request::Unmute => self.unmute().await?,
                messages::Request::Volume(v) => self.set_volume(v).await?,
                messages::Request::VolumeUp => self.increase_volume().await?,
                messages::Request::VolumeDown => self.decrease_volume().await?,
                messages::Request::Quit => self.quit().await?,
                messages::Request::RateUp => self.rate_track(Some(true)).await?,
                messages::Request::RateDown => self.rate_track(Some(false)).await?,
                messages::Request::UnRate => self.rate_track(None).await?,
            }
            self.dirty |= true;
        }
        // Check the health of the outgoing message channel, as well
        /*
        trace!(
            "Pending messages in notification channel: {}",
            self.channel_out.as_ref().map(|c| c.len()).unwrap_or(0)
        );
        trace!(
            "Number of listeners for notification channel: {}",
            self.channel_out
                .as_ref()
                .map(|c| c.receiver_count())
                .unwrap_or(0)
        );
        */
        Ok(())
    }

    pub(crate) async fn update(&mut self) -> Result<bool> {
        self.process_messages().await?;

        if self.state.connected() && !self.state.test_connection() {
            self.disconnect().await?;
            // If we have an active track started, we should send a notification that we stopped
            // because the session timed out
            if self.started() {
                if let Some(channel_out) = self.channel_out.as_mut() {
                    channel_out
                        .broadcast(messages::Notification::Stopped(StopReason::SessionTimedOut))
                        .await?;
                }
            }
            self.dirty = false;
            return Ok(true);
        }

        // Disconnected
        //   UI drives credential entry, saving them in config
        // Disconnected -> Connected:
        //   Connect using credentials saved in config
        // Connected
        //   UI drives station selection, saving it in config
        // Connected -> Tuned:
        //   Tune station saved in config
        // Tuned
        //   Start caching tracks
        //   Add cached tracks to playlist
        // Tuned -> Playing:
        //   There is a ready track in playlist
        // Playing
        //   Notify UI of player progress
        //   Check if track is completed
        match self.state {
            ModelState::Disconnected => {
                if self.config.borrow().login_credentials().get().is_some() {
                    self.connect().await?;
                } else {
                    self.disconnect().await?;
                }
            }
            ModelState::Connected { .. } => {
                if self.station_list.is_empty() {
                    self.fill_station_list().await?;
                }
                let opt_station_id = self.config.borrow().station_id();
                if let Some(station_id) = opt_station_id {
                    self.tune(station_id).await?;
                }
            }
            ModelState::Tuned { .. } => {
                self.refill_playlist().await?;
                self.start().await?;
            }
            ModelState::Playing { .. } => {
                self.poll_progress().await?;
            }
            ModelState::Quit => (),
            ModelState::Invalid => unreachable!("Invalid state"),
        }
        let old_dirty = self.dirty;
        self.dirty = false;
        Ok(old_dirty)
    }

    async fn poll_progress(&mut self) -> Result<()> {
        self.validate_playing().await?;
        if let Some((elapsed, duration)) =
            self.state.get_playing_mut().and_then(|p| p.poll_progress())
        {
            self.dirty = true;
            let progress_notification = if self.paused() {
                messages::Notification::Paused(elapsed, duration)
            } else {
                messages::Notification::Playing(elapsed, duration)
            };
            trace!("send notification 'Playing/Paused'");
            if let Some(channel_out) = self.channel_out.as_mut() {
                channel_out.broadcast(progress_notification).await?;
            }

            let next_track = self.state.get_next().cloned();
            trace!("send notification 'Next({:?})'", next_track);
            let next_notification = messages::Notification::Next(next_track);
            if let Some(channel_out) = self.channel_out.as_mut() {
                channel_out.broadcast(next_notification).await?;
            }
        }

        Ok(())
    }

    async fn validate_playing(&mut self) -> Result<()> {
        // If a track was started, but the audio device is no longer playing it
        // force that track out of the playlist
        if self
            .state
            .get_playing_mut()
            .map(|p| p.stopped())
            .unwrap_or_default()
        {
            trace!("Current track finished playing. Evicting from playlist...");
            self.stop(Some(StopReason::TrackCompleted)).await?;
        }
        Ok(())
    }

    pub(crate) fn playlist_len(&self) -> Result<usize> {
        self.state
            .playlist_len()
            .context("Failed to get playlist length")
    }

    pub(crate) async fn extend_playlist(&mut self, new_playlist: Vec<Track>) -> Result<()> {
        self.dirty |= !new_playlist.is_empty();
        for track in new_playlist {
            if let Some(channel_out) = self.channel_out.as_mut() {
                channel_out
                    .broadcast(messages::Notification::PreCaching(track))
                    .await?;
            }
        }
        Ok(())
    }

    fn add_track(&mut self, track: &Track) -> Result<()> {
        self.state.enqueue_track(track)
    }

    async fn refill_playlist(&mut self) -> Result<()> {
        if self.tuned().is_some() {
            let playlist_len = self.playlist_len()?;
            trace!("Playlist length: {}", playlist_len);
            // If there's at least three pending tracks in the queue,
            // then we don't refill.
            if playlist_len >= 3 {
                trace!("Not refilling.");
                return Ok(());
            }

            let new_playlist = self
                .state
                .fetch_playlist()
                .await
                .context("Failed while fetching new playlist")?;
            self.extend_playlist(new_playlist).await?;
        } else {
            debug!("Not refilling playlist while not tuned");
        }
        Ok(())
    }

    pub(crate) async fn rate_track(&mut self, rating: Option<bool>) -> Result<()> {
        if let Some(station_id) = self.tuned() {
            self.state
                .rate_track(&station_id, rating)
                .await
                .context("Failed to rate track")?;
            if let Some(rating) = rating.map(|r| if r { 1 } else { 0 }) {
                trace!("send notification 'rated'");
                if let Some(channel_out) = self.channel_out.as_mut() {
                    channel_out
                        .broadcast(messages::Notification::Rated(rating))
                        .await?;
                }
            } else {
                trace!("send notification 'unrated'");
                if let Some(channel_out) = self.channel_out.as_mut() {
                    channel_out
                        .broadcast(messages::Notification::Unrated)
                        .await?;
                }
            }
            self.dirty |= true;
        }
        Ok(())
    }

    async fn disconnect(&mut self) -> Result<()> {
        self.state.disconnect();
        self.dirty |= true;
        trace!("send notification 'disconnected'");
        if let Some(channel_out) = self.channel_out.as_mut() {
            channel_out
                .broadcast(messages::Notification::Disconnected)
                .await?;
        }

        Ok(())
    }

    async fn fail_authentication(&mut self) -> Result<()> {
        // TODO: send notification instead of clearing config?
        // allows retry without erasing stored credentials
        let failed_auth =
            PartialConfig::default().login(self.config.borrow().login_credentials().as_invalid());
        self.dirty |= true;
        self.config.borrow_mut().update_from(&failed_auth);
        self.disconnect().await?;
        Ok(())
    }

    fn connected(&self) -> bool {
        self.state.connected()
    }

    async fn connect(&mut self) -> Result<()> {
        if self.connected() {
            info!("Connect request ignored. Already connected.");
            trace!("send notification 'connected'");
            if let Some(channel_out) = self.channel_out.as_mut() {
                channel_out
                    .broadcast(messages::Notification::Connected)
                    .await?;
            }
        } else {
            trace!("Attempting pandora login...");
            self.dirty |= true;
            let session = PandoraSession::new(self.config.clone());
            if let Err(e) = self.state.connect(session).await {
                if e.downcast_ref::<Error>()
                    .map(|e| e.missing_auth_token())
                    .unwrap_or(false)
                {
                    error!("Required authentication token is missing.");
                    self.fail_authentication().await?;
                } else if let Some(e) = e.downcast_ref::<pandora_api::errors::Error>() {
                    error!("Pandora authentication failure: {:?}", e);
                    self.fail_authentication().await?;
                } else {
                    error!("Unknown error while logging in: {:?}", e);
                    self.fail_authentication().await?;
                }
                trace!("send notification 'disconnected'");
                if let Some(channel_out) = self.channel_out.as_mut() {
                    channel_out
                        .broadcast(messages::Notification::Disconnected)
                        .await?;
                }
                return Ok(());
            }
            trace!("Successfully logged into Pandora.");
        }
        trace!("send notification 'connected'");
        if let Some(channel_out) = self.channel_out.as_mut() {
            channel_out
                .broadcast(messages::Notification::Connected)
                .await?;
        }

        // If a station was saved, send a Tuned notification for it
        if let Some(station_id) = self.tuned() {
            trace!("send notification 'tuned'");
            if let Some(channel_out) = self.channel_out.as_mut() {
                channel_out
                    .broadcast(messages::Notification::Tuned(station_id))
                    .await?;
            }
        }

        // Notify listeners what the last set volume was
        let volume = self.volume();
        if let Some(channel_out) = self.channel_out.as_mut() {
            channel_out
                .broadcast(messages::Notification::Volume(volume))
                .await?;
        }

        Ok(())
    }

    async fn tune(&mut self, station_id: String) -> Result<()> {
        self.untune().await?;
        if self
            .tuned()
            .as_ref()
            .map(|s| s == &station_id)
            .unwrap_or_default()
        {
            trace!("Requested station is already tuned.");
            return Ok(());
        }
        trace!("Updating station on model");
        self.config
            .borrow_mut()
            .update_from(&PartialConfig::default().station(Some(station_id.clone())));

        self.state
            .tune(station_id.clone())
            .context("Failed to tune station")?;

        trace!("send notification 'tuned'");
        if let Some(channel_out) = self.channel_out.as_mut() {
            channel_out
                .broadcast(messages::Notification::Tuned(station_id))
                .await?;
        }

        self.dirty |= true;
        Ok(())
    }

    async fn untune(&mut self) -> Result<()> {
        self.stop(Some(StopReason::Untuning)).await?;
        if self.tuned().is_none() {
            return Ok(());
        }

        self.state.untune().context("Failed to untune station")?;
        self.config
            .borrow_mut()
            .update_from(&PartialConfig::default().station(None));

        trace!("send notification 'Connected'");
        if let Some(channel_out) = self.channel_out.as_mut() {
            channel_out
                .broadcast(messages::Notification::Connected)
                .await?;
        }

        self.dirty |= true;
        Ok(())
    }

    fn tuned(&self) -> Option<String> {
        self.state.tuned()
    }

    async fn quit(&mut self) -> Result<()> {
        trace!("Start quitting the application.");
        self.state.quit();
        self.dirty |= true;
        trace!("send notification 'quit'");
        if let Some(channel_out) = self.channel_out.as_mut() {
            channel_out.broadcast(messages::Notification::Quit).await?;
        }
        Ok(())
    }

    async fn fill_station_list(&mut self) -> Result<()> {
        if !self.station_list.is_empty() {
            return Ok(());
        }
        trace!("Filling station list");
        for station in self.state.fetch_station_list().await {
            if self.station_list.contains_key(&station.station_id) {
                continue;
            } else {
                self.station_list
                    .insert(station.station_id.clone(), station.clone());
                trace!(
                    "send notification 'add station {}[{}]'",
                    station.station_name,
                    station.station_id
                );
                if let Some(channel_out) = self.channel_out.as_mut() {
                    channel_out
                        .broadcast(messages::Notification::AddStation(
                            station.station_name,
                            station.station_id,
                        ))
                        .await?;
                }
            }
        }
        self.dirty |= true;
        Ok(())
    }

    async fn stop(&mut self, reason: Option<StopReason>) -> Result<()> {
        if self.config.borrow().cache_policy().evict_completed() {
            trace!("Eviction policy requires evicting track");
            if let Some(cached_path) = self
                .state
                .get_playing()
                .and_then(|p| p.playing())
                .and_then(|t| t.cached.as_ref())
            {
                std::fs::remove_file(cached_path).with_context(|| {
                    format!("Failed to evict {} from track cache", cached_path.display())
                })?;
                trace!("Evicted {} from track cache.", cached_path.display());
            }
        } else {
            trace!("Not evicting completed track, per configured cache eviction policy");
        }

        if self.state.get_playing().is_some() {
            self.state.stop().context("Failed to stop active track")?;
            trace!("send notification 'stopped'");
            if let Some(channel_out) = self.channel_out.as_mut() {
                channel_out
                    .broadcast(messages::Notification::Stopped(
                        reason.unwrap_or(StopReason::TrackInterrupted),
                    ))
                    .await?;
            }
            self.dirty |= true;
        } else {
            trace!("No track is currently playing. Nothing to do.");
        }
        self.refill_playlist().await?;
        Ok(())
    }

    fn started(&self) -> bool {
        self.state
            .get_playing()
            .map(|p| p.started())
            .unwrap_or_default()
    }

    async fn start(&mut self) -> Result<()> {
        if self.started() {
            //trace!("Track already started.");
        } else {
            trace!("No tracks started yet. Starting next track.");
            let volume = self.config.borrow().volume();
            if let Ok(Some(track)) = self.state.ready_next_track(volume) {
                trace!("send notification 'starting'");
                if let Some(channel_out) = self.channel_out.as_mut() {
                    channel_out
                        .broadcast(messages::Notification::Starting(track))
                        .await?;
                }
                trace!("send notification 'volume'");
                if let Some(channel_out) = self.channel_out.as_mut() {
                    channel_out
                        .broadcast(messages::Notification::Volume(volume))
                        .await?;
                }
                self.dirty |= true;
            }
        }
        Ok(())
    }

    fn paused(&self) -> bool {
        self.state
            .get_playing()
            .map(|p| p.paused())
            .unwrap_or_default()
    }

    fn pause(&mut self) {
        if !self.paused() {
            if let Some(playing) = self.state.get_playing_mut() {
                playing.pause();
                self.dirty |= true;
                // No notification because that happens via the progress update
            }
        }
    }

    fn unpause(&mut self) {
        if self.paused() {
            if let Some(playing) = self.state.get_playing_mut() {
                playing.unpause();
                self.dirty |= true;
                // No notification because that happens via the progress update
            }
        }
    }

    fn toggle_pause(&mut self) {
        if self.paused() {
            self.unpause();
        } else {
            self.pause();
        }
    }

    fn volume(&self) -> f32 {
        self.state
            .get_playing()
            .map(|p| p.volume())
            .unwrap_or_default()
    }

    async fn set_volume(&mut self, new_volume: f32) -> Result<()> {
        if let Some(playing) = self.state.get_playing_mut() {
            playing.set_volume(new_volume);
        }
        self.config
            .borrow_mut()
            .update_from(&PartialConfig::default().volume(new_volume));
        self.dirty |= true;
        trace!("send notification 'volume'");
        if let Some(channel_out) = self.channel_out.as_mut() {
            channel_out
                .broadcast(messages::Notification::Volume(new_volume))
                .await?;
        }
        Ok(())
    }

    async fn increase_volume(&mut self) -> Result<()> {
        let new_volume = self.volume() + 0.1;
        self.set_volume(new_volume.clamp(0.0, 1.0)).await
    }

    async fn decrease_volume(&mut self) -> Result<()> {
        let new_volume = self.volume() - 0.1;
        self.set_volume(new_volume.clamp(0.0, 1.0)).await
    }

    fn muted(&self) -> bool {
        self.state
            .get_playing()
            .map(|p| p.muted())
            .unwrap_or_default()
    }

    async fn mute(&mut self) -> Result<()> {
        if !self.muted() {
            if let Some(playing) = self.state.get_playing_mut() {
                playing.mute();
                self.dirty |= true;
            }
        }
        trace!("send notification 'mute'");
        if let Some(channel_out) = self.channel_out.as_mut() {
            channel_out.broadcast(messages::Notification::Muted).await?;
        }
        Ok(())
    }

    async fn unmute(&mut self) -> Result<()> {
        if self.muted() {
            if let Some(playing) = self.state.get_playing_mut() {
                playing.unmute();
                self.dirty |= true;
            }
        }
        trace!("send notification 'unmute'");
        if let Some(channel_out) = self.channel_out.as_mut() {
            channel_out
                .broadcast(messages::Notification::Unmuted)
                .await?;
        }
        Ok(())
    }

    pub(crate) fn quitting(&self) -> bool {
        self.state.quitting()
    }
}

impl Drop for Model {
    fn drop(&mut self) {
        // If there have been any configuration changes, commit them to disk
        trace!("Flushing config file to disk...");
        if let Err(e) = self.config.borrow_mut().flush() {
            error!("Failed commiting configuration changes to file: {:?}", e);
        }
        trace!("Application data model has been dropped");
    }
}
