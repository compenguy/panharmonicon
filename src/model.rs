use std::collections::{HashMap, VecDeque};
use std::io::{Read, Seek};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};
use std::{cell::RefCell, rc::Rc};

use log::{error, info, trace};
use rodio::source::Source;
use rodio::DeviceTrait;

use pandora_api::json::{station::PlaylistTrack, user::Station};

use crate::caching::CachedTrack;
use crate::config::{Config, PartialConfig};
use crate::errors::{Error, Result};
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

pub(crate) trait PlaybackMediator {
    fn stopped(&self) -> bool;
    fn stop(&mut self);
    fn started(&self) -> bool;
    fn start(&mut self);
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

    fn elapsed(&self) -> Duration;
    fn duration(&self) -> Duration;

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
enum CachePolicy {
    NoCaching,
    CachePlayingEvictCompleted,
    CacheNextEvictCompleted,
    CacheAllNoEviction,
}

impl CachePolicy {
    pub(crate) fn cache_playing(&self) -> bool {
        match self {
            Self::NoCaching => false,
            Self::CachePlayingEvictCompleted => true,
            Self::CacheNextEvictCompleted => true,
            Self::CacheAllNoEviction => true,
        }
    }

    pub(crate) fn cache_plus_one(&self) -> bool {
        match self {
            Self::NoCaching => false,
            Self::CachePlayingEvictCompleted => false,
            Self::CacheNextEvictCompleted => true,
            Self::CacheAllNoEviction => true,
        }
    }

    pub(crate) fn cache_all(&self) -> bool {
        match self {
            Self::NoCaching => false,
            Self::CachePlayingEvictCompleted => false,
            Self::CacheNextEvictCompleted => false,
            Self::CacheAllNoEviction => true,
        }
    }

    pub(crate) fn evict_completed(&self) -> bool {
        match self {
            Self::NoCaching => false,
            Self::CachePlayingEvictCompleted => true,
            Self::CacheNextEvictCompleted => true,
            Self::CacheAllNoEviction => false,
        }
    }
}

impl Default for CachePolicy {
    fn default() -> Self {
        Self::CachePlayingEvictCompleted
    }
}

#[derive(Debug, Clone, Copy)]
enum Volume {
    Muted(f32),
    Unmuted(f32),
}

impl Volume {
    fn volume(&self) -> f32 {
        if let Self::Unmuted(v) = self {
            v.min(1.0f32).max(0.0f32)
        } else {
            0.0f32
        }
    }

    fn set_volume(&mut self, new_volume: f32) {
        *self = Self::Unmuted(new_volume.min(1.0f32).max(0.0f32));
    }

    fn muted(&self) -> bool {
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
    device: rodio::Device,
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
            std::fs::File::open(path).map_err(|e| Error::MediaReadFailure(Box::new(e)))?,
        ))
    }

    fn play_from_reader<R: Read + Seek + Send + 'static>(&mut self, reader: R) -> Result<()> {
        let start_paused = false;
        let decoder = rodio::decoder::Decoder::new(reader)?.pausable(start_paused);

        // Force the sink to be deleted and recreated, ensuring it's in
        // a good state
        self.stop();

        self.sink.append(decoder);
        Ok(())
    }
}

impl PlaybackMediator for AudioDevice {
    fn stopped(&self) -> bool {
        self.sink.empty()
    }

    fn stop(&mut self) {
        self.sink = rodio::Sink::new(&self.device);
        self.sink.set_volume(self.volume.volume());
    }

    fn started(&self) -> bool {
        !self.stopped()
    }

    fn start(&mut self) {
        trace!("Noop");
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

    fn elapsed(&self) -> Duration {
        unreachable!();
    }

    fn duration(&self) -> Duration {
        unreachable!();
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
        let device = rodio::default_output_device()
            .expect("Failed to locate/initialize default audio device");
        let sink = rodio::Sink::new(&device);
        Self {
            device,
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
        let device = rodio::output_devices()
            .map(|mut devs| devs.find(|d| d.name().ok() == self.device.name().ok()))
            .ok()
            .flatten()
            .unwrap_or_else(|| {
                rodio::default_output_device()
                    .expect("Failed to locate/initialize default audio device")
            });
        let sink = rodio::Sink::new(&device);
        return AudioDevice {
            device,
            sink,
            volume: self.volume.clone(),
        };
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

        write!(
            f,
            "AudioDevice {{ device: {}, sink: ({}, {}, volume {:.2}), volume: {:?} }}",
            self.device.name().expect("Error retrieving device name"),
            queued,
            paused,
            self.sink.volume(),
            self.volume
        )
    }
}

#[derive(Debug, Clone)]
struct Playing {
    audio_device: AudioDevice,
    last_started: Option<Instant>,
    elapsed: Duration,
    duration: Duration,
    playlist: VecDeque<PlaylistTrack>,
}

impl Playing {
    fn playing(&self) -> Option<PlaylistTrack> {
        if self.elapsed().as_millis() > 0 {
            self.playlist.front().cloned()
        } else {
            None
        }
    }

    fn playlist(&self) -> &VecDeque<PlaylistTrack> {
        &self.playlist
    }

    fn extend_playlist(&mut self, new_playlist: Vec<PlaylistTrack>) {
        self.playlist.extend(new_playlist.into_iter());
    }

    fn stop_all(&mut self) {
        self.stop();
        self.playlist.clear();
    }

    fn duration_from_path<P: AsRef<Path>>(&mut self, path: P) {
        match mp3_duration::from_path(&path) {
            Ok(duration) => self.duration = duration,
            Err(e) => {
                self.duration = Duration::default();
                trace!("Error calculating track duration: {:?}", e);
            }
        }
        trace!("Set track duration to {:?}", &self.duration);
    }

    fn precache_playlist_track(&mut self) {
        for track in self.playlist.iter_mut() {
            if track.optional.get("cached").is_some() {
                continue;
            }
            match CachedTrack::add_to_cache(track) {
                Ok(path) => trace!("Cached track to {}", path.to_string_lossy()),
                Err(e) => trace!("Error caching track to path: {:?}", e),
            }
        }
    }
}

impl PlaybackMediator for Playing {
    fn stopped(&self) -> bool {
        self.audio_device.stopped()
    }

    fn stop(&mut self) {
        if self.elapsed().as_millis() > 0 {
            self.audio_device.stop();
            self.playlist.pop_front();
            self.last_started = None;
            self.elapsed = Duration::default();
            self.duration = Duration::default();
        }
    }

    fn started(&self) -> bool {
        !self.audio_device.stopped()
    }

    fn start(&mut self) {
        if self.started() {
            trace!("A track is already playing. It needs to be stopped first.");
            return;
        }
        if let Some(track) = self.playlist.front_mut() {
            let cached = match track.optional.get("cached") {
                Some(serde_json::value::Value::String(cached)) => PathBuf::from(cached.clone()),
                _ => match CachedTrack::add_to_cache(track) {
                    Err(e) => {
                        error!("Failed caching track: {:?}", e);
                        return;
                    }
                    Ok(cached) => cached,
                },
            };
            trace!("Starting decoding of track {}", cached.to_string_lossy());
            if let Err(e) = self.audio_device.play_from_file(PathBuf::from(&cached)) {
                error!(
                    "Error starting track at {}: {:?}",
                    cached.to_string_lossy(),
                    e
                );
            } else {
                self.duration_from_path(&cached);
                self.last_started = Some(Instant::now());
                trace!("Started track at {}.", cached.to_string_lossy());
            }
        } else {
            trace!("Cannot start track if the playlist is empty.");
        }
    }

    fn paused(&self) -> bool {
        assert_eq!(self.last_started.is_none(), self.audio_device.paused());
        self.last_started.is_none() && self.elapsed.as_millis() > 0
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

    fn elapsed(&self) -> Duration {
        let elapsed_since_last_started = self.last_started.map(|i| i.elapsed()).unwrap_or_default();
        trace!(
            "elapsed since last started: {:?}",
            elapsed_since_last_started
        );
        self.elapsed + self.last_started.map(|i| i.elapsed()).unwrap_or_default()
    }

    fn duration(&self) -> Duration {
        self.duration.clone()
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

impl Default for Playing {
    fn default() -> Self {
        Self {
            audio_device: AudioDevice::default(),
            last_started: None,
            elapsed: Duration::default(),
            duration: Duration::default(),
            playlist: VecDeque::default(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum State {
    Disconnected,
    AuthenticationFailed,
    Connected,
    Tuned,
    Ready,
    Playing,
}

impl Default for State {
    fn default() -> Self {
        Self::Disconnected
    }
}

#[derive(Debug, Clone)]
pub(crate) struct Model {
    config: Rc<RefCell<Config>>,
    session: PandoraSession,
    state: State,
    station: Option<String>,
    station_list: HashMap<String, Station>,
    playing: Playing,
    quitting: bool,
    dirty: bool,
}

impl Model {
    pub(crate) fn new(config: Rc<RefCell<Config>>) -> Self {
        let mut playing = Playing::default();
        playing.set_volume(config.borrow().volume());
        Self {
            config: config.clone(),
            session: PandoraSession::new(config.clone()),
            state: State::default(),
            station: config.borrow().station_id().cloned(),
            station_list: HashMap::new(),
            playing,
            quitting: false,
            dirty: true,
        }
    }

    pub(crate) fn config(&self) -> Rc<RefCell<Config>> {
        self.config.clone()
    }

    // TODO: move station methods onto a trait
    pub(crate) fn fill_station_list(&mut self) {
        if self.station_list.len() > 0 {
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

    pub(crate) fn clear_station_list(&mut self) {
        self.station_list.clear();
        self.dirty |= true;
    }

    pub(crate) fn station_list(&self) -> Vec<(String, String)> {
        self.station_list
            .values()
            .map(|s| (s.station_name.clone(), s.station_id.clone()))
            .collect()
    }

    pub(crate) fn station(&self, station_id: &str) -> Option<&Station> {
        self.station_list.get(station_id)
    }

    pub(crate) fn station_count(&self) -> usize {
        self.station_list.len()
    }

    fn refill_playlist(&mut self) {
        // If the playing track and at least one more are still
        // in the queue, then we don't refill.
        let playlist_len = self.playing.playlist().len();
        if playlist_len >= 2 {
            return;
        }

        trace!("Playlist length: {}", playlist_len);
        if let Some(station) = self.station.clone() {
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
            PartialConfig::new_login(self.config.borrow().login_credentials().as_invalid());
        self.dirty |= true;
        if let Err(e) = self.config.borrow_mut().update_from(&failed_auth) {
            error!(
                "Failed while updating configuration for failed authentication: {:?}",
                e
            );
        }
    }

    fn connected(&self) -> bool {
        self.session.connected()
    }

    fn connect(&mut self) {
        if !self.connected() {
            match self.session.user_login() {
                Ok(_) => self.state = State::Connected,
                Err(Error::PanharmoniconMissingAuthToken) => {
                    error!("Required authentication token is missing.");
                    self.fail_authentication();
                }
                Err(Error::PandoraFailure(e)) => {
                    error!("Pandora authentication failure: {:?}", e);
                    self.fail_authentication();
                }
                Err(e) => {
                    error!("Unknown error while logging in: {:?}", e);
                    self.fail_authentication();
                }
            }
            self.dirty |= true;
        } else {
            info!("Connect request ignored. Already connected.");
        }
    }

    fn tuned(&self) -> Option<String> {
        if self.connected() {
            self.station.clone()
        } else {
            None
        }
    }

    fn tune(&mut self, station_id: String) {
        if self
            .station
            .as_ref()
            .map(|s| s == &station_id)
            .unwrap_or(false)
        {
            trace!("Requested station is already playing.");
        }
        trace!("Updating station on model");
        self.station = Some(station_id);
        self.dirty |= true;

        if self.connected() {
            self.state = State::Tuned;
        } else {
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
        if self.station.is_some() {
            self.station = None;
            self.dirty |= true;
        }
        if self.connected() {
            self.state = State::Connected;
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
        self.playing.playing().clone()
    }

    fn update(&mut self) -> bool {
        let mut old_dirty = self.dirty;
        // If a track was started, but the audio device is no longer playing it
        // force that track out of the playlist
        if self.elapsed().as_millis() > 0 && self.playing.audio_device.stopped() {
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
                old_dirty = self.dirty;
            }
        } else if self.config.borrow().login_credentials().get().is_some() {
            self.connect();
            if !old_dirty && (old_dirty != self.dirty) {
                trace!("connect dirtied");
                old_dirty = self.dirty;
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
        if !self.started() {
            self.playing.start();
            self.dirty |= true;
        }
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

    fn elapsed(&self) -> Duration {
        self.playing.elapsed()
    }

    fn duration(&self) -> Duration {
        self.playing.duration()
    }

    fn volume(&self) -> f32 {
        self.playing.volume()
    }

    fn set_volume(&mut self, new_volume: f32) {
        self.playing.set_volume(new_volume);
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
        if let Err(e) = self.config.borrow_mut().flush() {
            error!("Failed commiting configuration changes to file: {:?}", e);
        }
    }
}
