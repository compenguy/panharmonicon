use std::cell::RefCell;
use std::rc::Rc;
use std::time::{Duration, Instant};

use log::{debug, trace};
use rodio::source::Source;
use rodio::DeviceTrait;

use pandora_api::json::station::{AudioFormat, AudioStream, PlaylistTrack};
pub use pandora_api::json::user::Station;

use crate::caching::get_cached_media;
use crate::config;
use crate::config::{Config, PartialConfig};
use crate::errors::{Error, Result};
use crate::pandora::PandoraSession;
use crate::term;

#[derive(Debug, Clone, PartialEq, PartialOrd)]
pub(crate) struct Audio {
    pub(crate) quality: Quality,
    pub(crate) url: String,
    pub(crate) bitrate: String,
    pub(crate) encoding: AudioFormat,
}

impl Audio {
    fn try_from_stream(quality: Quality, stream: &AudioStream) -> Result<Self> {
        Ok(Audio {
            quality,
            url: stream.audio_url.clone(),
            bitrate: stream.bitrate.clone(),
            encoding: AudioFormat::new_from_audio_url_map(&stream.encoding, &stream.bitrate)?,
        })
    }

    fn list_from_track(track: &PlaylistTrack) -> Vec<Audio> {
        let mut sorted_audio_list: Vec<Audio> = Vec::with_capacity(4);

        match Audio::try_from_stream(Quality::High, &track.audio_url_map.high_quality) {
            Ok(hq_audio) => sorted_audio_list.push(hq_audio),
            Err(e) => debug!("Unsupported hq track encoding: {:?}", e),
        }

        match Audio::try_from_stream(Quality::Medium, &track.audio_url_map.medium_quality) {
            Ok(mq_audio) => sorted_audio_list.push(mq_audio),
            Err(e) => debug!("Unsupported mq track encoding: {:?}", e),
        }

        match Audio::try_from_stream(Quality::Low, &track.audio_url_map.low_quality) {
            Ok(lq_audio) => sorted_audio_list.push(lq_audio),
            Err(e) => debug!("Unsupported lq track encoding: {:?}", e),
        }

        sorted_audio_list.push(Audio {
            quality: Quality::Medium,
            url: track.additional_audio_url.to_string(),
            bitrate: format!("{}", AudioFormat::Mp3128.get_bitrate()),
            encoding: AudioFormat::Mp3128,
        });
        sorted_audio_list
    }

    pub(crate) fn get_extension(&self) -> String {
        self.encoding.get_extension()
    }
}

#[derive(Debug, Clone, PartialEq, PartialOrd)]
pub(crate) enum Quality {
    High,
    Medium,
    Low,
}

#[derive(Debug, Clone)]
pub(crate) struct Playing {
    pub(crate) track_token: String,
    pub(crate) audio: Vec<Audio>,
    pub(crate) started: Instant,
    pub(crate) elapsed: Duration,
    pub(crate) duration: Duration,
    pub(crate) info: SongInfo,
}

impl Playing {
    fn start_timer(&mut self) {
        self.started = Instant::now();
    }

    fn pause_timer(&mut self) {
        self.elapsed += self.started.elapsed();
    }

    fn resume_timer(&mut self) {
        self.started = Instant::now();
    }

    fn get_duration(&self) -> Duration {
        self.duration
    }

    fn get_elapsed(&self) -> Duration {
        self.elapsed + self.started.elapsed()
    }

    fn get_remaining(&self) -> Duration {
        let elapsed = self.get_elapsed();
        if self.duration > elapsed {
            self.duration - elapsed
        } else {
            trace!("Track ran past its end, negative duration remaining.");
            Duration::default()
        }
    }
}

impl From<&PlaylistTrack> for Playing {
    fn from(track: &PlaylistTrack) -> Self {
        let playing = Self {
            track_token: track.track_token.to_string(),
            audio: Audio::list_from_track(track),
            started: Instant::now(),
            elapsed: Duration::default(),
            duration: Duration::default(),
            info: SongInfo::from(track),
        };
        trace!("Parsed playlist track {:?}", &playing.info);
        playing
    }
}

#[derive(Debug, Clone)]
pub(crate) struct SongInfo {
    pub(crate) name: String,
    pub(crate) artist: String,
    pub(crate) album: String,
    pub(crate) rating: u32,
}

impl From<&PlaylistTrack> for SongInfo {
    fn from(track: &PlaylistTrack) -> Self {
        Self::from(track.clone())
    }
}

impl From<PlaylistTrack> for SongInfo {
    fn from(track: PlaylistTrack) -> Self {
        Self {
            name: track.song_name,
            artist: track.artist_name,
            album: track.album_name,
            rating: track.song_rating,
        }
    }
}

impl Playing {
    pub(crate) fn get_best_audio(&self) -> Option<Audio> {
        self.audio.first().cloned()
    }

    pub(crate) fn get_audio_format(&self, format: AudioFormat) -> Option<Audio> {
        self.audio.iter().find(|&a| a.encoding == format).cloned()
    }

    #[allow(dead_code)]
    pub(crate) fn get_audio(&self, quality: Quality) -> Option<Audio> {
        self.audio.iter().find(|&a| a.quality == quality).cloned()
    }
}

pub(crate) struct Panharmonicon {
    shutting_down: bool,
    ui: term::Terminal,
    config: Rc<RefCell<Config>>,
    session: PandoraSession,
    audio_device: rodio::Device,
    audio_sink: rodio::Sink,
    station: Option<String>,
    playlist: std::collections::VecDeque<PlaylistTrack>,
    playing: Option<Playing>,
    unmute_volume: Option<f32>,
}

impl Panharmonicon {
    pub(crate) fn new(config: Rc<RefCell<Config>>, ui: term::Terminal) -> Self {
        let audio_device =
            rodio::default_output_device().expect("Failed to locate default audio output sink");
        debug!(
            "Selected output device {}",
            audio_device.name().unwrap_or_else(|_| String::new())
        );
        let audio_sink = rodio::Sink::new(&audio_device);
        audio_sink.set_volume(config.borrow().volume);
        let station = config.borrow().station_id.clone();
        Self {
            shutting_down: false,
            ui,
            config: config.clone(),
            audio_device,
            audio_sink,
            session: PandoraSession::new(config),
            station,
            playlist: std::collections::VecDeque::with_capacity(6),
            playing: None,
            unmute_volume: None,
        }
    }

    pub(crate) fn reconnect(&mut self) {
        self.session.partner_logout();
    }

    pub(crate) fn run(&mut self) -> Result<()> {
        let loop_granularity = Duration::from_millis(100);
        while !self.shutting_down() {
            let now = std::time::Instant::now();
            self.process_user_events();

            if self.has_track() {
                self.play_track()?;
            } else if self.has_playlist() {
                self.advance_playlist()?;
            } else if self.has_station() && self.has_connection() {
                self.refill_playlist()?;
            } else if self.has_connection() {
                self.select_station()?;
            } else if self.has_credentials() {
                match self.make_connection() {
                    Err(Error::PanharmoniconMissingAuthToken) => self.update_credentials(false)?,
                    Err(Error::PandoraFailure(_)) => self.update_credentials(true)?,
                    a @ Err(_) => return a,
                    _ => trace!("Successful login"),
                }
            } else {
                self.update_credentials(false)?;
            }

            let elapsed = now.elapsed();
            if elapsed < loop_granularity {
                std::thread::sleep(loop_granularity - elapsed);
            }
        }
        Ok(())
    }

    fn process_user_events(&mut self) {
        while let Some(user_request) = self.ui.pop_signal() {
            match user_request {
                term::ApplicationSignal::Quit => self.shut_down(),
                term::ApplicationSignal::VolumeUp => self.volume_up(),
                term::ApplicationSignal::VolumeDown => self.volume_down(),
                term::ApplicationSignal::Mute => self.mute(),
                term::ApplicationSignal::Unmute => self.unmute(),
                term::ApplicationSignal::ToggleMuteUnmute => self.toggle_mute(),
                term::ApplicationSignal::Play => self.play(),
                term::ApplicationSignal::Pause => self.pause(),
                term::ApplicationSignal::TogglePlayPause => self.toggle_pause(),
                term::ApplicationSignal::ThumbsUpTrack => trace!("TODO: ThumbsUpTrack"),
                term::ApplicationSignal::ThumbsDownTrack => trace!("TODO: ThumbsUpTrack"),
                term::ApplicationSignal::RemoveTrackRating => trace!("TODO: RemoveTrackRating"),
                term::ApplicationSignal::SleepTrack => trace!("TODO: SleepTrack"),
                term::ApplicationSignal::NextTrack => self.stop(),
                term::ApplicationSignal::ChangeStation => self.select_station().unwrap_or_default(),
                term::ApplicationSignal::ShowPlaylist => {
                    let song_list: Vec<SongInfo> =
                        self.playlist.iter().map(SongInfo::from).collect();
                    self.ui.display_song_list(&song_list);
                }
            }
        }
    }

    fn shut_down(&mut self) {
        self.shutting_down = true;
    }

    fn shutting_down(&self) -> bool {
        self.shutting_down
    }

    fn volume_up(&mut self) {
        self.unmute();
        let cur_volume = self.audio_sink.volume();
        // Clamp max volume at 1.0
        let new_volume = if cur_volume + 0.1f32 <= 1.0f32 {
            cur_volume + 0.1f32
        } else {
            1.0f32
        };
        self.audio_sink.set_volume(new_volume);
    }

    fn volume_down(&mut self) {
        self.unmute();
        let cur_volume = self.audio_sink.volume();
        // Clamp min volume at 0.0
        let new_volume = if cur_volume - 0.1f32 >= 0.0f32 {
            cur_volume - 0.1f32
        } else {
            0.0f32
        };
        self.audio_sink.set_volume(new_volume);
    }

    fn mute(&mut self) {
        if self.unmute_volume.is_none() {
            self.unmute_volume = Some(self.audio_sink.volume());
            self.audio_sink.set_volume(0.0f32);
        }
    }

    fn unmute(&mut self) {
        let new_volume: Option<f32> = self.unmute_volume.take();
        if let Some(volume) = new_volume {
            self.audio_sink.set_volume(volume);
        }
    }

    fn is_muted(&self) -> bool {
        self.unmute_volume.is_some()
    }

    fn toggle_mute(&mut self) {
        if self.is_muted() {
            self.unmute()
        } else {
            self.mute()
        }
    }

    fn pause(&mut self) {
        if !self.is_paused() {
            trace!("Pausing playback of current track at user request.");
            if let Some(playing) = self.playing.as_mut() {
                playing.pause_timer();
            }
            self.audio_sink.pause();
        }
    }

    fn play(&mut self) {
        if self.is_paused() {
            trace!("Resuming playback of current track at user request.");
            if let Some(playing) = self.playing.as_mut() {
                playing.resume_timer();
            }
            self.audio_sink.play();
        }
    }

    fn stop(&mut self) {
        trace!("Stopping current track at user request.");
        let volume = self.audio_sink.volume();
        self.audio_sink = rodio::Sink::new(&self.audio_device);
        self.audio_sink.set_volume(volume);
        self.playing = None;
    }

    fn is_paused(&self) -> bool {
        self.audio_sink.is_paused()
    }

    fn toggle_pause(&mut self) {
        if self.is_paused() {
            self.play()
        } else {
            self.pause()
        }
    }

    fn has_credentials(&self) -> bool {
        if self.config.borrow().login.get_username().is_some() {
            if let Ok(Some(_)) = self.config.borrow().login.get_password() {
                return true;
            }
        }
        false
    }

    fn update_credentials(&mut self, retry: bool) -> Result<()> {
        trace!("Requesting login credentials");
        if retry {
            self.ui.login(term::SessionAuth::ForceReauth)?;
        } else {
            self.ui.login(term::SessionAuth::UseSaved)?;
        }
        Ok(())
    }

    fn has_connection(&self) -> bool {
        self.session.connected()
    }

    fn make_connection(&mut self) -> Result<()> {
        trace!("Starting login session to pandora");
        self.session.user_login()
    }

    fn has_station(&self) -> bool {
        self.station.is_some()
    }

    fn select_station(&mut self) -> Result<()> {
        trace!("Requesting station");
        let station_list = self.session.get_station_list()?.stations;

        self.ui.display_station_list(&station_list);
        let station_id = self.ui.station_prompt();
        if let Some(station) = station_list.iter().find(|s| s.station_id == station_id) {
            self.ui
                .display_station_info(&station.station_id, &station.station_name);
            self.station = Some(station.station_id.clone());
        } else {
            self.station = None;
        }

        if self.config.borrow().save_station {
            let mut partial_update = PartialConfig::default();
            partial_update.station_id = Some(self.station.clone());
            self.config.borrow_mut().update_from(&partial_update)?;
            self.config.borrow_mut().flush()?;
        }
        Ok(())
    }

    fn has_playlist(&self) -> bool {
        !self.playlist.is_empty()
    }

    fn refill_playlist(&mut self) -> Result<()> {
        trace!("Refilling playlist");
        let station = self
            .station
            .as_ref()
            .ok_or_else(|| Error::PanharmoniconNoStationSelected)?;
        let playlist = self.session.get_playlist(station)?;
        self.playlist
            .extend(playlist.iter().filter_map(|pe| pe.get_track()));
        //debug!("Playlist: {:?}", self.playlist);
        trace!("Playlist refilled with {} tracks", self.playlist.len());
        Ok(())
    }

    fn advance_playlist(&mut self) -> Result<()> {
        trace!("Advancing playlist");
        if let Some(track) = self.playlist.pop_front() {
            trace!("Getting another song off the playlist");
            let playing = Playing::from(&track);

            let quality = self.config.borrow().audio_quality;

            debug!("config-set audio quality: {:?}", quality);
            let audio = match quality {
                config::AudioQuality::PreferBest => {
                    debug!("Selecting best available audio stream...");
                    playing
                        .get_best_audio()
                        .ok_or_else(|| Error::PanharmoniconTrackHasNoAudio)?
                }
                config::AudioQuality::PreferMp3 => {
                    debug!("Selecting mp3 audio stream...");
                    playing
                        .get_audio_format(AudioFormat::Mp3128)
                        .ok_or_else(|| Error::PanharmoniconTrackHasNoAudio)?
                }
            };

            debug!("Selected audio stream {:?}", audio);

            let cached_media = get_cached_media(&playing, audio)?;

            trace!("Starting media decoding...");
            // Setting pausable(false) actually makes the source
            // the source pausable but not initially paused, in spite
            // of how it may look.
            let source = rodio::decoder::Decoder::new(std::io::BufReader::new(
                std::fs::File::open(&cached_media)
                    .map_err(|e| Error::MediaReadFailure(Box::new(e)))?,
            ))?
            .pausable(false);
            self.audio_sink.append(source);
            /* Applying the provided track gain values in my experience causes
             * clipping
            .amplify(track.track_gain.parse::<f32>().unwrap_or(1.0f32))
            */

            // Setting track as playing
            self.playing = Some(playing);
            let track_duration = mp3_duration::from_path(&cached_media)?;

            if let Some(playing) = self.playing.as_mut() {
                playing.duration = track_duration;
                self.ui.display_playing(&playing.info, &track_duration);
                playing.start_timer();
            }
        }
        Ok(())
    }

    fn has_track(&self) -> bool {
        self.playing.is_some()
    }

    fn play_track(&mut self) -> Result<()> {
        if let Some(playing) = self.playing.as_mut() {
            let elapsed = playing.get_elapsed();
            let remaining = playing.get_remaining();
            let duration = playing.get_duration();
            trace!(
                "Playing {} ({}/{})",
                &playing.info.name,
                elapsed.as_secs(),
                duration.as_secs()
            );
            let zero = Duration::from_millis(0);
            if self.audio_sink.empty() {
                self.playing = None;
                trace!("Playback of Active track completed");
            } else if duration > zero && remaining > zero {
                self.ui.update_playing_progress(&elapsed);
                trace!("Track has time left on it.")
            } else {
                debug!("Sink is not empty, but there's no time left on the clock for the current playing item.");
            }
        }
        Ok(())
    }
}
