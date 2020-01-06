use std::cell::RefCell;
use std::convert::TryFrom;
use std::io::Write;
use std::rc::Rc;

use clap::crate_name;
use log::trace;
use reqwest;

use crate::config::{Config, PartialConfig};
use crate::errors::{Error, Result};
use crate::ui;

pub use pandora_rs2::stations::Station;

#[derive(Debug, Clone)]
pub(crate) struct SongInfo {
    pub(crate) name: String,
    pub(crate) artist: String,
    pub(crate) rating: Option<u32>,
}

#[derive(Debug, Clone)]
pub(crate) struct Playing {
    track_token: String,
    bitrate: String,
    encoding: String,
    url: String,
    protocol: String,
    remaining: std::time::Duration,
    info: SongInfo,
}

impl TryFrom<&pandora_rs2::playlist::Track> for Playing {
    type Error = crate::errors::Error;

    fn try_from(track: &pandora_rs2::playlist::Track) -> std::result::Result<Self, Self::Error> {
        let track_token = track
            .track_token
            .as_ref()
            .ok_or_else(|| Error::PanharmoniconTrackHasNoId)?;
        let audio = track
            .track_audio
            .as_ref()
            .ok_or_else(|| Error::PanharmoniconTrackHasNoAudio)?;
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
            bitrate: audio.high_quality.bitrate.to_string(),
            encoding: audio.high_quality.encoding.to_string(),
            url: audio.high_quality.audio_url.to_string(),
            protocol: audio.high_quality.protocol.to_string(),
            remaining: std::time::Duration::from_millis(0),
            info: SongInfo {
                name: name.to_string(),
                artist: artist.to_string(),
                rating: track.song_rating,
            },
        })
    }
}

#[derive(Debug)]
pub(crate) struct Panharmonicon {
    ui: ui::Session,
    config: Rc<RefCell<Config>>,
    connection: Option<pandora_rs2::Pandora>,
    station: Option<Station>,
    playlist: std::collections::VecDeque<pandora_rs2::playlist::Track>,
    playing: Option<Playing>,
}

impl Panharmonicon {
    pub(crate) fn new(config: Rc<RefCell<Config>>, ui: ui::Session) -> Self {
        let station = config.borrow().station_id.clone().map(|id| Station {
            station_id: id,
            station_name: String::new(),
        });
        Self {
            ui,
            config,
            connection: None,
            station: station,
            playlist: std::collections::VecDeque::with_capacity(6),
            playing: None,
        }
    }

    pub(crate) fn run(&mut self) -> Result<()> {
        loop {
            if self.has_track() {
                self.play_track()?;
            } else if self.has_playlist() {
                self.advance_playlist()?;
            } else if self.has_station() {
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
            std::thread::sleep(std::time::Duration::from_millis(100));
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

    fn update_credentials(&self, retry: bool) -> Result<()> {
        trace!("Requesting login credentials");
        if retry {
            self.ui.login(ui::SessionAuth::ForceReauth);
        } else {
            self.ui.login(ui::SessionAuth::UseSaved);
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
        let station = self.ui.select_station();
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
            let playing = Playing::try_from(&track)?;
            self.ui.display_song_info(&playing.info);
            self.playing = Some(playing);
            self.cache_playing()?;
        }
        Ok(())
    }

    fn cache_playing(&mut self) -> Result<()> {
        trace!("Caching active track");
        if let Some(playing) = self.playing.as_mut() {
            trace!("Sufficient track information to cache song");
            let filename = format!("{} - {}.mp3", playing.info.artist, playing.info.name);

            let cache_file = dirs::cache_dir()
                .ok_or_else(|| Error::CacheDirNotFound)?
                .join(crate_name!())
                .join(playing.track_token.clone())
                .join(filename);
            if cache_file.exists() {
                trace!("Song already in cache.");
            } else {
                trace!("Caching song.");
                if let Some(dir) = cache_file.parent() {
                    std::fs::create_dir_all(&dir)
                        .map_err(|e| Error::CacheDirCreateFailure(Box::new(e)))?;
                }
                let mut cached = std::io::BufWriter::new(
                    std::fs::File::create(&cache_file)
                        .map_err(|e| Error::FileCachingFailure(Box::new(e)))?,
                );

                let mut resp = reqwest::blocking::get(&playing.url).map_err(Error::from)?;
                resp.copy_to(&mut cached).map_err(Error::from)?;
                trace!("Song added to cache.");
            }
            playing.remaining = std::time::Duration::from_secs(5 * 60);
        }
        Ok(())
    }

    fn has_track(&self) -> bool {
        self.playing.is_some()
    }

    fn play_track(&mut self) -> Result<()> {
        trace!("Playing active track");
        if let Some(playing) = self.playing.as_mut() {
            let zero = std::time::Duration::from_millis(0);
            if playing.remaining > zero {
                trace!("playing active track");
                let cur = std::time::Instant::now();
                std::thread::sleep(std::time::Duration::from_millis(100));
                let elapsed = cur.elapsed();
                if elapsed < playing.remaining {
                    playing.remaining -= elapsed;
                } else {
                    playing.remaining = zero;
                }

                self.ui.update_song_progress(&playing.remaining);
            } else {
                trace!("active track completed");
                self.playing = None;
            }
        }
        Ok(())
    }
}
