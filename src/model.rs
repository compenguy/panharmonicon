use std::collections::{HashMap, VecDeque};
use std::io::{Read, Seek};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};
use std::{cell::RefCell, rc::Rc};

use anyhow::{Context, Result};
use cpal::traits::{DeviceTrait, HostTrait};
use log::{debug, error, info, trace};
use rodio::source::Source;

use pandora_api::json::{station::PlaylistTrack, user::Station};

use crate::caching;
use crate::config::{CachePolicy, Config, PartialConfig};
use crate::errors::Error;
use crate::pandora::PandoraSession;

pub(crate) trait StateMediator {
    fn disconnected(&self) -> bool;
    fn disconnect(&mut self);
    fn fail_authentication(&mut self);
    fn connected(&self) -> bool;
    fn connect(&mut self);
    fn tuned(&self) -> Option<String>;
    fn tune(&mut self, station_id: String);
    fn untune(&mut self);
    fn ready(&self) -> bool;
    fn playing(&self) -> Option<PlaylistTrack>;
    fn update(&mut self) -> bool;
    fn quitting(&self) -> bool;
    fn quit(&mut self);
}

pub(crate) trait StationMediator {
    fn fill_station_list(&mut self);
    fn station_list(&self) -> Vec<(String, String)>;
    fn station_count(&self) -> usize;
}

pub(crate) trait PlaybackMediator {
    fn stopped(&self) -> bool;
    fn stop(&mut self);
    fn started(&self) -> bool;
    fn start(&mut self);

    fn elapsed(&self) -> Duration;
    fn duration(&self) -> Duration;
}

pub(crate) trait AudioMediator {
    fn reset(&mut self);
    fn active(&self) -> bool;
    fn paused(&self) -> bool;
    fn pause(&mut self);
    fn unpause(&mut self);
    fn toggle_pause(&mut self) {
        if self.paused() {
            self.unpause();
        } else {
            self.pause();
        }
    }

    fn volume(&self) -> f32;
    fn set_volume(&mut self, new_volume: f32);
    fn increase_volume(&mut self) {
        self.set_volume(self.volume() + 0.1);
    }

    fn decrease_volume(&mut self) {
        self.set_volume(self.volume() - 0.1);
    }

    fn refresh_volume(&mut self);
    fn muted(&self) -> bool;
    fn mute(&mut self);
    fn unmute(&mut self);
    fn toggle_mute(&mut self) {
        if self.muted() {
            self.unmute();
        } else {
            self.mute();
        }
    }
}

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
    fn play_from_file<P: AsRef<Path>>(&mut self, path: P) -> Result<()> {
        trace!(
            "Reading track at {} for playback",
            path.as_ref().to_string_lossy()
        );
        self.play_from_reader(std::io::BufReader::new(
            std::fs::File::open(path.as_ref()).with_context(|| {
                format!(
                    "Failed reading media file at {}",
                    path.as_ref().to_string_lossy()
                )
            })?,
        ))
    }

    fn play_from_reader<R: Read + Seek + Send + 'static>(&mut self, reader: R) -> Result<()> {
        let start_paused = false;
        let decoder = rodio::decoder::Decoder::new(reader)
            .with_context(|| "Failed initializing media decoder")?
            .pausable(start_paused);

        // Force the sink to be deleted and recreated, ensuring it's in
        // a good state
        self.reset();

        self.sink.append(decoder);
        Ok(())
    }
}

impl AudioMediator for AudioDevice {
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

#[derive(Debug, Clone, Default)]
struct Playing {
    audio_device: AudioDevice,
    cache_policy: CachePolicy,
    last_started: Option<Instant>,
    elapsed: Duration,
    duration: Duration,
    playlist: VecDeque<PlaylistTrack>,
    track_cacher: caching::TrackCacher,
}

impl Playing {
    fn new(cache_policy: CachePolicy, volume: f32) -> Self {
        let mut pl = Self::default();
        pl.cache_policy = cache_policy;
        pl.set_volume(volume);
        pl
    }

    fn playing(&self) -> Option<PlaylistTrack> {
        if self.elapsed() > Duration::default() {
            self.playlist.front().cloned()
        } else {
            None
        }
    }

    fn playlist_len(&self) -> usize {
        self.playlist.len() + self.track_cacher.pending_count()
    }

    fn extend_playlist(&mut self, new_playlist: Vec<PlaylistTrack>) {
        self.track_cacher.enqueue(new_playlist);
        trace!(
            "New playlist length: {}",
            self.playlist.len() + self.track_cacher.pending_count()
        );
    }

    fn stop_all(&mut self) {
        self.stop();
        self.playlist.clear();
        self.track_cacher.clear();
    }

    fn precache_playlist_track(&mut self) {
        if let Err(e) = self.track_cacher.update() {
            error!("Error while updating track cache: {}", e);
        }
        self.playlist
            .extend(self.track_cacher.get_ready().drain(..));
    }

    fn evict_playing(&mut self) {
        if let Some(track) = self.playlist.pop_front() {
            if !self.cache_policy.evict_completed() {
                return;
            }
            if let Some(serde_json::value::Value::String(path)) = track.optional.get("cached") {
                let path = PathBuf::from(path);
                trace!("Evicting track from cache: {}", path.to_string_lossy());
                if let Err(e) = std::fs::remove_file(&path) {
                    error!(
                        "Error evicting track {} from cache: {:?}",
                        path.to_string_lossy(),
                        e
                    );
                }
            }
        }
    }
}

impl PlaybackMediator for Playing {
    fn stopped(&self) -> bool {
        if self.active() && self.elapsed() == Duration::default() {
            panic!("Application state error: audio device is active, but no track playtime has elapsed.");
        }
        !self.active()
    }

    fn stop(&mut self) {
        if self.elapsed().as_millis() > 0 {
            self.reset();
            self.evict_playing();
            self.last_started = None;
            self.elapsed = Duration::default();
            self.duration = Duration::default();
        }
    }

    fn started(&self) -> bool {
        if self.active() && self.elapsed() == Duration::default() {
            panic!("Application state error: audio device is active, but no track playtime has elapsed.");
        }
        self.active()
    }

    fn start(&mut self) {
        if self.started() {
            trace!("A track is already playing. It needs to be stopped first.");
            return;
        }
        if let Some(track) = self.playlist.front_mut() {
            debug!("Track: {:?}", &track);
            if let Some(serde_json::value::Value::String(cached)) = track.optional.get("cached") {
                trace!("Starting decoding of track {}", cached);
                if let Err(e) = self.audio_device.play_from_file(PathBuf::from(&cached)) {
                    error!("Error starting track at {}: {:?}", cached, e);
                } else {
                    self.duration = track
                        .optional
                        .get("trackLength")
                        .and_then(|v| v.as_u64())
                        .map(Duration::from_secs)
                        .unwrap_or_default();

                    self.last_started = Some(Instant::now());
                    trace!("Started track at {}.", cached);
                }
            } else {
                error!("Uncached track in playlist! Evicting...");
                self.stop();
            }
        } else {
            trace!("Cannot start track if the playlist is empty.");
        }
    }

    fn elapsed(&self) -> Duration {
        let elapsed_since_last_started = self.last_started.map(|i| i.elapsed()).unwrap_or_default();
        self.elapsed + elapsed_since_last_started
    }

    fn duration(&self) -> Duration {
        self.duration
    }
}

impl AudioMediator for Playing {
    fn reset(&mut self) {
        self.audio_device.reset();
    }

    fn active(&self) -> bool {
        self.audio_device.active()
    }

    fn paused(&self) -> bool {
        // This returns true when a track has actually been started, but time
        // is not elapsing on it.
        if self.audio_device.paused() && self.last_started.is_some() {
            panic!(
                "Application state error: track is paused, but track playtime still increasing."
            );
        }
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

    fn refresh_volume(&mut self) {
        self.audio_device.refresh_volume();
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
pub(crate) struct Model {
    config: Rc<RefCell<Config>>,
    session: PandoraSession,
    station_list: HashMap<String, Station>,
    playing: Playing,
    quitting: bool,
    dirty: bool,
}

impl Model {
    pub(crate) fn new(config: Rc<RefCell<Config>>) -> Self {
        let policy = config.borrow_mut().cache_policy();
        let volume = config.borrow_mut().volume();
        Self {
            config: config.clone(),
            session: PandoraSession::new(config),
            station_list: HashMap::new(),
            playing: Playing::new(policy, volume),
            quitting: false,
            dirty: true,
        }
    }

    pub(crate) fn config(&self) -> Rc<RefCell<Config>> {
        self.config.clone()
    }

    fn refill_playlist(&mut self) {
        // If the playing track and at least one more are still
        // in the queue, then we don't refill.
        let playlist_len = self.playing.playlist_len();
        if playlist_len >= 2 {
            return;
        }

        trace!("Playlist length: {}", playlist_len);
        if let Some(station) = self.tuned() {
            match self.session.get_playlist(&station) {
                Ok(playlist) => {
                    trace!("Extending playlist.");
                    let playlist: Vec<PlaylistTrack> = playlist
                        .into_iter()
                        .filter_map(|pe| pe.get_track())
                        .collect();
                    self.playing.extend_playlist(playlist);
                    self.dirty |= true;
                }
                Err(e) => error!("Failed while fetching new playlist: {:?}", e),
            }
        }
    }

    fn cache_track(&mut self) {
        self.playing.precache_playlist_track();
    }

    pub(crate) fn rate_track(&mut self, rating: Option<bool>) {
        if let (Some(track), Some(st_id)) = (self.playing(), self.tuned()) {
            let new_rating_value: u32 = if rating.unwrap_or(false) { 1 } else { 0 };
            if let Some(rating) = rating {
                if let Err(e) = self
                    .session
                    .add_feedback(&st_id, &track.track_token, rating)
                {
                    error!("Failed submitting track rating: {:?}", e);
                } else {
                    if let Some(t) = self.playing.playlist.front_mut() {
                        t.song_rating = new_rating_value;
                    }
                    self.dirty |= true;
                    trace!("Rated track {} with value {}", track.song_name, rating);
                }
            } else if let Err(e) = self.session.delete_feedback_for_track(&st_id, &track) {
                error!("Failed submitting track rating: {:?}", e);
            } else {
                self.playing
                    .playlist
                    .front_mut()
                    .map(|t| t.song_rating = new_rating_value);
                self.dirty |= true;
                trace!("Successfully removed track rating.");
            }
        }
    }

    pub(crate) fn sleep_track(&mut self) {
        if let Err(e) = self
            .playing()
            .map(|t| self.session.sleep_song(&t.track_token))
            .transpose()
        {
            error!("Failed to sleep track: {:?}", e);
        }
        self.stop();
    }
}

impl StateMediator for Model {
    fn disconnected(&self) -> bool {
        !self.session.connected()
    }

    fn disconnect(&mut self) {
        // TODO: Evaluate whether session.user_logout() would better suit
        self.session.partner_logout();
        self.dirty |= true;
    }

    fn fail_authentication(&mut self) {
        let failed_auth =
            PartialConfig::default().login(self.config.borrow().login_credentials().as_invalid());
        self.dirty |= true;
        self.config.borrow_mut().update_from(&failed_auth);
    }

    fn connected(&self) -> bool {
        self.session.connected()
    }

    fn connect(&mut self) {
        if !self.connected() {
            trace!("Attempting pandora login...");
            if let Err(e) = self.session.user_login() {
                if e.downcast_ref::<Error>()
                    .map(|e| *e == Error::PanharmoniconMissingAuthToken)
                    .unwrap_or(false)
                {
                    error!("Required authentication token is missing.");
                    self.fail_authentication();
                } else if let Some(e) = e.downcast_ref::<pandora_api::errors::Error>() {
                    error!("Pandora authentication failure: {:?}", e);
                    self.fail_authentication();
                } else {
                    error!("Unknown error while logging in: {:?}", e);
                    self.fail_authentication();
                }
            } else {
                trace!("Successfully logged into Pandora.");
            }
            self.dirty |= true;
        } else {
            info!("Connect request ignored. Already connected.");
        }
    }

    fn tuned(&self) -> Option<String> {
        self.config.borrow().station_id()
    }

    fn tune(&mut self, station_id: String) {
        if self
            .tuned()
            .as_ref()
            .map(|s| s == &station_id)
            .unwrap_or(false)
        {
            trace!("Requested station is already tuned.");
            return;
        }
        trace!("Updating station on model");
        self.config
            .borrow_mut()
            .update_from(&PartialConfig::default().station(Some(station_id)));
        self.dirty |= true;

        if !self.connected() {
            info!("Cannot start station until connected, but saving station for when connected.");
        }
        // This will stop the current track and flush the playlist of all queue
        // tracks so that later we can fill it with tracks from the new station
        if self.started() {
            self.playing.stop_all();
            self.dirty |= true;
        }
    }

    fn untune(&mut self) {
        if self.tuned().is_some() {
            self.config
                .borrow_mut()
                .update_from(&PartialConfig::default().station(None));
            self.dirty |= true;
        }

        // This will stop the current track and flush the playlist of all queue
        if self.started() {
            self.playing.stop_all();
            self.dirty |= true;
        }
    }

    fn ready(&self) -> bool {
        self.stopped()
    }

    fn playing(&self) -> Option<PlaylistTrack> {
        self.playing.playing()
    }

    fn update(&mut self) -> bool {
        let mut old_dirty = self.dirty;
        // If a track was started, but the audio device is no longer playing it
        // force that track out of the playlist
        if self.elapsed().as_millis() > 0 && !self.active() {
            trace!("Current track finished playing. Evicting from playlist...");
            self.playing.stop();
        }
        if self.connected() {
            self.fill_station_list();
            if !old_dirty && (old_dirty != self.dirty) {
                trace!("fill_station_list dirtied");
                old_dirty = self.dirty;
            }
            self.refill_playlist();
            if !old_dirty && (old_dirty != self.dirty) {
                trace!("refill_playlist dirtied");
                old_dirty = self.dirty;
            }
            self.cache_track();
            if !old_dirty && (old_dirty != self.dirty) {
                trace!("cache_track dirtied");
                old_dirty = self.dirty;
            }
            self.start();
            if !old_dirty && (old_dirty != self.dirty) {
                trace!("start dirtied");
            }
        } else if self.config.borrow().login_credentials().get().is_some() {
            self.disconnect();
            self.connect();
            if !old_dirty && (old_dirty != self.dirty) {
                trace!("connect dirtied");
            }
        }
        let old_dirty = self.dirty;
        self.dirty = false;
        old_dirty
    }

    fn quitting(&self) -> bool {
        self.quitting
    }

    fn quit(&mut self) {
        trace!("Start quitting the application.");
        self.quitting = true;
        self.dirty |= true;
    }
}

impl StationMediator for Model {
    fn fill_station_list(&mut self) {
        if !self.station_list.is_empty() {
            return;
        }
        trace!("Filling station list");
        self.station_list = self
            .session
            .get_station_list()
            .ok()
            .map(|sl| {
                sl.stations
                    .into_iter()
                    .map(|s| (s.station_id.clone(), s))
                    .collect()
            })
            .unwrap_or_default();

        self.dirty |= true;
    }

    fn station_list(&self) -> Vec<(String, String)> {
        self.station_list
            .values()
            .map(|s| (s.station_name.clone(), s.station_id.clone()))
            .collect()
    }

    fn station_count(&self) -> usize {
        self.station_list.len()
    }
}

impl PlaybackMediator for Model {
    fn stopped(&self) -> bool {
        self.playing.stopped()
    }

    fn stop(&mut self) {
        if !self.stopped() {
            self.playing.stop();
            self.dirty |= true;
        }
    }

    fn started(&self) -> bool {
        self.playing.started()
    }

    fn start(&mut self) {
        if self.started() {
            //trace!("Track already started.");
        } else {
            trace!("No tracks started yet. Starting next track.");
            self.playing.start();
            self.dirty |= true;
        }
    }

    fn elapsed(&self) -> Duration {
        self.playing.elapsed()
    }

    fn duration(&self) -> Duration {
        self.playing.duration()
    }
}

impl AudioMediator for Model {
    // TODO: this might require some finesse to get the right
    // behavior between either dropping the current-playing track
    // or restarting it from the beginning.
    fn reset(&mut self) {
        self.playing.reset();
        self.dirty |= true;
    }

    fn active(&self) -> bool {
        self.playing.active()
    }

    fn paused(&self) -> bool {
        self.playing.paused()
    }

    fn pause(&mut self) {
        if !self.paused() {
            self.playing.pause();
            self.dirty |= true;
        }
    }

    fn unpause(&mut self) {
        if self.paused() {
            self.playing.unpause();
            self.dirty |= true;
        }
    }

    fn volume(&self) -> f32 {
        self.playing.volume()
    }

    fn set_volume(&mut self, new_volume: f32) {
        self.playing.set_volume(new_volume);
        self.config
            .borrow_mut()
            .update_from(&PartialConfig::default().volume(new_volume));
        self.dirty |= true;
    }

    fn refresh_volume(&mut self) {
        self.playing.refresh_volume();
        self.dirty |= true;
    }

    fn muted(&self) -> bool {
        self.playing.muted()
    }

    fn mute(&mut self) {
        if !self.muted() {
            self.playing.mute();
            self.dirty |= true;
        }
    }

    fn unmute(&mut self) {
        if self.muted() {
            self.playing.unmute();
            self.dirty |= true;
        }
    }
}

impl Drop for Model {
    fn drop(&mut self) {
        // If there have been any configuration changes, commit them to disk
        trace!("Flushing config file to disk...");
        if let Err(e) = self.config.borrow_mut().flush() {
            error!("Failed commiting configuration changes to file: {:?}", e);
        }
    }
}
