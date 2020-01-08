use std::cell::RefCell;
use std::convert::TryFrom;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::{Duration, Instant};

use clap::crate_name;
use log::{error, trace};
use reqwest;

use crate::config;
use crate::config::{Config, PartialConfig};
use crate::crossterm as term;
use crate::errors::{Error, Result};

pub use pandora_rs2::stations::Station;

#[derive(Debug, Clone)]
pub(crate) struct SongInfo {
    pub(crate) name: String,
    pub(crate) artist: String,
    pub(crate) album: Option<String>,
    pub(crate) rating: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, PartialOrd)]
pub(crate) enum Quality {
    High,
    Medium,
    Low,
    Alternate,
}

impl Quality {
    pub(crate) fn get_extension(&self) -> String {
        match self {
            Quality::High => String::from("m4a"),
            Quality::Medium => String::from("m4a"),
            Quality::Low => String::from("m4a"),
            Quality::Alternate => String::from("mp3"),
        }
    }
}

#[derive(Debug, Clone, PartialEq, PartialOrd)]
pub(crate) struct Audio {
    quality: Quality,
    url: String,
    bitrate: Option<String>,
    encoding: Option<String>,
}

impl Audio {
    fn from_track(track: &pandora_rs2::playlist::Track) -> Vec<Audio> {
        let mut sorted_audio_list: Vec<Audio> = Vec::with_capacity(4);
        if let Some(track_audio) = &track.track_audio {
            sorted_audio_list.push(Audio {
                quality: Quality::High,
                url: track_audio.high_quality.audio_url.clone(),
                bitrate: Some(track_audio.high_quality.bitrate.clone()),
                encoding: Some(track_audio.high_quality.encoding.clone()),
            });
            sorted_audio_list.push(Audio {
                quality: Quality::Medium,
                url: track_audio.medium_quality.audio_url.clone(),
                bitrate: Some(track_audio.medium_quality.bitrate.clone()),
                encoding: Some(track_audio.medium_quality.encoding.clone()),
            });
            sorted_audio_list.push(Audio {
                quality: Quality::Low,
                url: track_audio.low_quality.audio_url.clone(),
                bitrate: Some(track_audio.low_quality.bitrate.clone()),
                encoding: Some(track_audio.low_quality.encoding.clone()),
            });
        }
        if let Some(url) = &track.additional_audio_url {
            sorted_audio_list.push(Audio {
                quality: Quality::Alternate,
                url: url.to_string(),
                bitrate: None,
                encoding: None,
            });
        }
        sorted_audio_list
    }

    pub(crate) fn get_extension(&self) -> String {
        if let Some(encoding) = &self.encoding {
            if encoding == "aacplus" {
                return String::from("m4a");
            }
        }
        self.quality.get_extension()
    }
}

#[derive(Debug, Clone)]
pub(crate) struct Playing {
    track_token: String,
    audio: Vec<Audio>,
    duration: Duration,
    remaining: Duration,
    info: SongInfo,
}

impl TryFrom<&pandora_rs2::playlist::Track> for Playing {
    type Error = crate::errors::Error;

    fn try_from(track: &pandora_rs2::playlist::Track) -> std::result::Result<Self, Self::Error> {
        trace!("Parsing playlist track {:?}", track);
        let track_token = track
            .track_token
            .as_ref()
            .ok_or_else(|| Error::PanharmoniconTrackHasNoId)?;
        let name = track
            .song_name
            .as_ref()
            .ok_or_else(|| Error::PanharmoniconTrackHasNoName)?;
        let artist = track
            .artist_name
            .as_ref()
            .ok_or_else(|| Error::PanharmoniconTrackHasNoArtist)?;
        Ok(Self {
            track_token: track_token.to_string(),
            audio: Audio::from_track(&track),
            duration: Duration::from_millis(0),
            remaining: Duration::from_millis(0),
            info: SongInfo {
                name: name.to_string(),
                artist: artist.to_string(),
                album: track.album_name.clone(),
                rating: track.song_rating,
            },
        })
    }
}

impl Playing {
    pub(crate) fn get_audio(&self, quality: Quality) -> Option<Audio> {
        self.audio.iter().find(|&a| a.quality == quality).cloned()
    }

    pub(crate) fn get_best_audio(&self) -> Option<Audio> {
        self.audio.first().cloned()
    }

    pub(crate) fn get_alternate_or_best_audio(&self) -> Option<Audio> {
        self.get_audio(Quality::Alternate)
            .or_else(|| self.get_best_audio())
    }
}

pub(crate) struct Panharmonicon {
    ui: term::Terminal,
    config: Rc<RefCell<Config>>,
    connection: Option<pandora_rs2::Pandora>,
    station: Option<Station>,
    playlist: std::collections::VecDeque<pandora_rs2::playlist::Track>,
    playing: Option<Playing>,
}

impl Panharmonicon {
    pub(crate) fn new(config: Rc<RefCell<Config>>, ui: term::Terminal) -> Self {
        let station = config.borrow().station_id.clone().map(|id| Station {
            station_id: id,
            station_name: String::new(),
        });
        Self {
            ui,
            config,
            connection: None,
            station,
            playlist: std::collections::VecDeque::with_capacity(6),
            playing: None,
        }
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
        self.connection.is_some()
    }

    fn make_connection(&mut self) -> Result<()> {
        trace!("Starting login session to pandora");
        let username_opt = self.config.borrow().login.get_username();
        let username = username_opt.ok_or_else(|| Error::PanharmoniconMissingAuthToken)?;

        let password_opt = self.config.borrow().login.get_password()?;
        let password = password_opt.ok_or_else(|| Error::PanharmoniconMissingAuthToken)?;

        self.connection =
            Some(pandora_rs2::Pandora::new(&username, &password).map_err(Error::from)?);
        Ok(())
    }

    fn has_station(&self) -> bool {
        self.station.is_some()
    }

    fn select_station(&mut self) -> Result<()> {
        trace!("Requesting station");
        if let Some(connection) = self.connection.as_ref() {
            self.ui.display_station_list(&connection.stations().list()?);
        }
        let station = self.ui.station_prompt();
        self.ui.display_station_info(&station);
        self.station = Some(station.clone());

        if self.config.borrow().save_station {
            let partial_update = PartialConfig {
                login: None,
                station_id: Some(Some(station.station_id)),
                save_station: None,
            };
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
        let connection = self
            .connection
            .as_ref()
            .ok_or_else(|| Error::PanharmoniconNotConnected)?;
        let station = self
            .station
            .as_ref()
            .ok_or_else(|| Error::PanharmoniconNoStationSelected)?;
        let playlist = pandora_rs2::playlist::Playlist::new(&connection, station).list()?;
        let infolist: Vec<SongInfo> = playlist
            .iter()
            .filter_map(|t| Playing::try_from(t).ok())
            .map(|p| p.info)
            .collect();
        self.ui.display_song_list(&infolist);
        self.playlist.extend(playlist);
        trace!("Playlist refilled with {} tracks", self.playlist.len());
        Ok(())
    }

    fn advance_playlist(&mut self) -> Result<()> {
        trace!("Advancing playlist");
        if let Some(track) = self.playlist.pop_front() {
            trace!("Getting another song off the playlist");
            let mut playing = Playing::try_from(&track)?;
            let cached_media = self.get_cached_media(&playing)?;
            let duration = read_media_duration(&cached_media)?;
            playing.duration = duration;
            playing.remaining = duration;
            self.ui.display_playing(&playing.info, &duration);
            self.playing = Some(playing);
        }
        Ok(())
    }

    fn get_cached_media(&mut self, playing: &Playing) -> Result<PathBuf> {
        trace!("Caching active track {}", playing.track_token);
        // TODO: Add config option to select between mp3 wherever possible or
        // best available
        let audio = match self.config.borrow().audio_quality {
            config::AudioQuality::PreferBest => playing
                .get_best_audio()
                .ok_or_else(|| Error::PanharmoniconTrackHasNoAudio)?,
            config::AudioQuality::PreferMp3 => playing
                .get_alternate_or_best_audio()
                .ok_or_else(|| Error::PanharmoniconTrackHasNoAudio)?,
        };
        // Adjust track metadata so that it's path/filename-safe
        let artist_filename = filename_formatter(&playing.info.artist);
        let album_filename = playing.info.album.clone().map(|n| filename_formatter(&n));
        let song_filename = filename_formatter(&playing.info.name);
        let filename = format!(
            "{} - {}.{}",
            artist_filename,
            song_filename,
            audio.get_extension()
        );

        // Construct full path to the cached file
        let mut cache_file = dirs::cache_dir()
            .ok_or_else(|| Error::AppDirNotFound)?
            .join(crate_name!())
            .join(playing.info.artist.clone());
        if let Some(album) = &album_filename {
            cache_file = cache_file.join(album);
        }
        cache_file = cache_file.join(filename);

        // Check cache, and if track isn't in the cache, add it
        if cache_file.exists() {
            trace!("Song already in cache.");
        } else {
            trace!("Caching song.");
            if let Err(e) = save_url_to_file(&audio.url, &cache_file) {
                error!("Error caching track for playback: {:?}", e);
            } else {
                trace!("Song added to cache.");
            }
        }
        Ok(cache_file)
    }

    fn has_track(&self) -> bool {
        self.playing.is_some()
    }

    fn play_track(&mut self) -> Result<()> {
        if let Some(playing) = self.playing.as_mut() {
            let zero = Duration::from_millis(0);
            if playing.remaining > zero {
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
                self.playing = None;
                trace!("Playback of Active track completed");
            }
        }
        Ok(())
    }
}

fn read_media_duration(media_path: &Path) -> Result<Duration> {
    let reader = std::io::BufReader::new(
        std::fs::File::open(media_path).map_err(|e| Error::FileReadFailure(Box::new(e)))?,
    );
    if let Some(extension) = media_path.extension() {
        match extension
            .to_str()
            .ok_or_else(|| Error::FilenameEncodingFailure)?
        {
            "m4a" => read_mp4_media_duration(reader),
            "mp4" => read_mp4_media_duration(reader),
            "mp3" => read_mp3_media_duration(reader),
            _ => Err(Error::UnspecifiedOrUnsupportedMediaType),
        }
    } else {
        Err(Error::UnspecifiedOrUnsupportedMediaType)
    }
}

fn read_mp4_media_duration<R: std::io::Read>(mut stream: R) -> Result<Duration> {
    let mut context = mp4parse::MediaContext::new();
    mp4parse::read_mp4(&mut stream, &mut context).map_err(Error::from)?;
    let track = context
        .tracks
        .iter()
        .find(|t| t.track_type == mp4parse::TrackType::Audio)
        .ok_or(Error::InvalidMedia)?;
    let timescale = track.timescale.ok_or(Error::InvalidMedia)?;
    let unscaled_duration = track.duration.ok_or(Error::InvalidMedia)?;
    let duration = Duration::from_secs(unscaled_duration.0 / timescale.0);
    Ok(duration)
}

fn read_mp3_media_duration<R: std::io::Read>(mut stream: R) -> Result<Duration> {
    mp3_duration::from_read(&mut stream).map_err(Error::from)
}

fn save_url_to_file(url: &str, file: &Path) -> Result<()> {
    if let Err(e) = _save_url_to_file(url, file) {
        // We suppress the result of attempting to remove the file because
        // 1. The file may not have been created in the first place
        // 2. We're too busy trying to return the original error anyway
        let _ = std::fs::remove_file(file);
        Err(e)
    } else {
        Ok(())
    }
}

fn _save_url_to_file(url: &str, file: &Path) -> Result<()> {
    // Ensure that target directory exists
    if let Some(dir) = file.parent() {
        std::fs::create_dir_all(&dir).map_err(|e| Error::FileWriteFailure(Box::new(e)))?;
    }

    let mut writer = std::io::BufWriter::new(
        std::fs::File::create(&file).map_err(|e| Error::FileWriteFailure(Box::new(e)))?,
    );

    // Fetch the url and write the body to the open file
    let mut resp = reqwest::blocking::get(url).map_err(Error::from)?;
    resp.copy_to(&mut writer)
        .map_err(|e| Error::FileWriteFailure(Box::new(e)))?;
    Ok(())
}

fn filename_formatter(text: &str) -> String {
    text.replace("/", "_")
        .replace("\\", "_")
        .replace(":", "_")
        .replace("-", "_")
}
