use std::cell::RefCell;
use std::rc::Rc;
use std::time::{Duration, Instant};

use log::{debug, trace};
use rodio::source::Source;
use rodio::DeviceTrait;

use pandora_api::json::station::{AudioFormat, PlaylistTrack};
pub use pandora_api::json::user::Station;

use crate::caching::get_cached_media;
use crate::config;
use crate::config::{Config, PartialConfig};
use crate::crossterm as term;
use crate::errors::{Error, Result};
use crate::pandora::PandoraSession;

#[derive(Debug, Clone, PartialEq, PartialOrd)]
pub(crate) struct Audio {
    pub(crate) quality: Quality,
    pub(crate) url: String,
    pub(crate) bitrate: String,
    pub(crate) encoding: AudioFormat,
}

impl Audio {
    fn from_track(track: &PlaylistTrack) -> Vec<Audio> {
        let mut sorted_audio_list: Vec<Audio> = Vec::with_capacity(4);

        let hq_audio = &track.audio_url_map.high_quality;
        let mq_audio = &track.audio_url_map.medium_quality;
        let lq_audio = &track.audio_url_map.low_quality;

        // TODO: change these "expect()" calls to log the failure,
        // and then omit them from the audio list.
        sorted_audio_list.push(Audio {
            quality: Quality::High,
            url: hq_audio.audio_url.clone(),
            bitrate: hq_audio.bitrate.clone(),
            encoding: AudioFormat::new_from_audio_url_map(&hq_audio.encoding, &hq_audio.bitrate)
                .expect("Unsupported high quality audio format returned by Pandora"),
        });
        sorted_audio_list.push(Audio {
            quality: Quality::Medium,
            url: mq_audio.audio_url.clone(),
            bitrate: mq_audio.bitrate.clone(),
            encoding: AudioFormat::new_from_audio_url_map(&mq_audio.encoding, &mq_audio.bitrate)
                .expect("Unsupported medium quality audio format returned by Pandora"),
        });
        sorted_audio_list.push(Audio {
            quality: Quality::Low,
            url: lq_audio.audio_url.clone(),
            bitrate: lq_audio.bitrate.clone(),
            encoding: AudioFormat::new_from_audio_url_map(&lq_audio.encoding, &lq_audio.bitrate)
                .expect("Unsupported low quality audio format returned by Pandora"),
        });

        sorted_audio_list.push(Audio {
            quality: Quality::Medium,
            url: track.additional_audio_url.to_string(),
            bitrate: String::from("128"),
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
    pub(crate) duration: Duration,
    pub(crate) remaining: Duration,
    pub(crate) info: SongInfo,
}

impl From<&PlaylistTrack> for Playing {
    fn from(track: &PlaylistTrack) -> Self {
        trace!("Parsing playlist track {:?}", track);
        Self {
            track_token: track.track_token.to_string(),
            audio: Audio::from_track(track),
            duration: Duration::from_millis(0),
            remaining: Duration::from_millis(0),
            info: SongInfo::from(track),
        }
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
    ui: term::Terminal,
    config: Rc<RefCell<Config>>,
    session: PandoraSession,
    _audio_device: rodio::Device,
    audio_sink: rodio::Sink,
    station: Option<String>,
    playlist: std::collections::VecDeque<PlaylistTrack>,
    playing: Option<Playing>,
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
            ui,
            config: config.clone(),
            _audio_device: audio_device,
            audio_sink,
            session: PandoraSession::new(config),
            station,
            playlist: std::collections::VecDeque::with_capacity(6),
            playing: None,
        }
    }

    pub(crate) fn reconnect(&mut self) {
        self.session.partner_logout();
        self.station = None;
    }

    pub(crate) fn run(&mut self) -> Result<()> {
        // TODO: add a way to quit
        loop {
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
            std::thread::sleep(Duration::from_millis(100));
        }
    }

    fn pause(&self) {
        self.audio_sink.pause();
    }

    fn play(&self) {
        self.audio_sink.play();
    }

    fn is_paused(&self) -> bool {
        self.audio_sink.is_paused()
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
        let infolist: Vec<SongInfo> = playlist
            .iter()
            .filter_map(|t| t.get_track())
            .map(SongInfo::from)
            .collect();
        self.ui.display_song_list(&infolist);

        self.playlist
            .extend(playlist.iter().filter_map(|pe| pe.get_track()));
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
            self.playing = Some(playing);
            trace!("Starting media decoding...");
            // Setting pausable(false) actually makes the source
            // the source pausable but not initially paused, in spite
            // of how it may look.
            let source = rodio::decoder::Decoder::new(std::io::BufReader::new(
                std::fs::File::open(cached_media)
                    .map_err(|e| Error::MediaReadFailure(Box::new(e)))?,
            ))?
            .pausable(false);
            /* Applying the provided track gain values in my experience causes
             * clipping
            .amplify(track.track_gain.parse::<f32>().unwrap_or(1.0f32))
            */
            let duration = source.total_duration().unwrap_or_default();
            if let Some(playing) = self.playing.as_mut() {
                playing.duration = duration;
                playing.remaining = duration;
                self.ui.display_playing(&playing.info, &duration);
            }
            self.audio_sink.append(source);
        }
        Ok(())
    }

    fn has_track(&self) -> bool {
        self.playing.is_some()
    }

    fn play_track(&mut self) -> Result<()> {
        if let Some(playing) = self.playing.as_mut() {
            let zero = Duration::from_millis(0);
            if self.audio_sink.empty() {
                self.playing = None;
                trace!("Playback of Active track completed");
            } else if playing.remaining > zero {
                let cur = Instant::now();
                std::thread::sleep(Duration::from_millis(100));
                let elapsed = cur.elapsed();
                if elapsed < playing.remaining {
                    playing.remaining -= elapsed;
                } else {
                    playing.remaining = zero;
                }

                self.ui
                    .update_playing_progress(&playing.duration, &playing.remaining);
            } else {
                debug!("Sink is empty, but there's still time left on the clock for the current playing item.");
            }
        }
        Ok(())
    }
}
