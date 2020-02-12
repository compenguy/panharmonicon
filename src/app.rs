use std::cell::RefCell;
use std::io::{Read, Seek, Write};
use std::path::PathBuf;
use std::rc::Rc;
use std::time::{Duration, Instant};

use clap::crate_name;
use log::{debug, trace};
use reqwest;
use rodio::source::Source;
use rodio::DeviceTrait;

use pandora_api::json::station::{AudioFormat, PlaylistTrack};
pub use pandora_api::json::user::Station;

use crate::config;
use crate::config::{Config, PartialConfig};
use crate::crossterm as term;
use crate::errors::{Error, Result};
use crate::pandora::PandoraSession;

#[derive(Debug, Clone, PartialEq, PartialOrd)]
pub(crate) struct Audio {
    quality: Quality,
    url: String,
    bitrate: String,
    encoding: AudioFormat,
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
    track_token: String,
    audio: Vec<Audio>,
    duration: Duration,
    remaining: Duration,
    info: SongInfo,
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
            let partial_update = PartialConfig {
                login: None,
                station_id: Some(self.station.clone()),
                save_station: None,
                audio_quality: None,
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
            let cached_media = self.get_cached_media(&playing)?;
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

    fn get_cached_media(&mut self, playing: &Playing) -> Result<PathBuf> {
        trace!("Caching active track {}", playing.track_token);
        debug!(
            "config-set audio quality: {:?}",
            self.config.borrow().audio_quality
        );
        let audio = match self.config.borrow().audio_quality {
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
        // Adjust track metadata so that it's path/filename-safe
        let artist_filename = filename_formatter(&playing.info.artist);
        let album_filename = filename_formatter(&playing.info.album);
        let song_filename = filename_formatter(&playing.info.name);
        let filename = format!(
            "{} - {}.{}",
            artist_filename,
            song_filename,
            audio.get_extension()
        );

        // Construct full path to the cached file
        let cache_file = dirs::cache_dir()
            .ok_or_else(|| Error::AppDirNotFound)?
            .join(crate_name!())
            .join(artist_filename)
            .join(album_filename)
            .join(filename);

        // Check cache, and if track isn't in the cache, add it
        if cache_file.exists() {
            trace!("Song already in cache.");
        } else {
            trace!("Caching song.");
            let tempdir = dirs::cache_dir()
                .ok_or_else(|| Error::AppDirNotFound)?
                .join(crate_name!())
                .join("tmp");
            // Ensure that target directory exists
            if !tempdir.exists() {
                trace!("Creating temp dir {}", tempdir.to_string_lossy());
                std::fs::create_dir_all(&tempdir)
                    .map_err(|e| Error::FileWriteFailure(Box::new(e)))?;
            }
            let tempdest = mktemp::Temp::new_file_in(tempdir)
                .map_err(|e| Error::FileWriteFailure(Box::new(e)))?
                .release();
            trace!("Saving audio to temp file {}", tempdest.to_string_lossy());
            // Control the scope of temp_rw, so that we control when it closes
            {
                let mut temp_rw = std::fs::OpenOptions::new()
                    .read(true)
                    .write(true)
                    .create(true)
                    .open(&tempdest)
                    .map_err(|e| Error::FileWriteFailure(Box::new(e)))?;
                save_url_to_writer(&audio.url, &temp_rw)?;
                trace!("Audio written to disk.");
                trace!("Tagging mp3...");
                tag_mp3(&mut temp_rw, &playing.info)?;
                trace!("Mp3 tagged.");
            }
            trace!(
                "Persisting saved mp3 into the cache: {:?} -> {:?}",
                &tempdest,
                &cache_file
            );
            trace!(
                "Source exists: {}, Dest exists: {}",
                tempdest.is_file(),
                cache_file.exists()
            );
            if let Some(cache_parent_dir) = cache_file.parent() {
                if !cache_parent_dir.exists() {
                    trace!("Creating cache dir {}", cache_parent_dir.to_string_lossy());
                    std::fs::create_dir_all(&cache_parent_dir)
                        .map_err(|e| Error::FileWriteFailure(Box::new(e)))?;
                }
            }
            std::fs::rename(&tempdest, &cache_file)
                .map_err(|e| Error::FileWriteFailure(Box::new(e)))?;
            trace!("Song added to cache.");
        }
        Ok(cache_file)
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

fn tag_mp3<F: Read + Write + Seek>(mut mp3_rw: &mut F, metadata: &SongInfo) -> Result<()> {
    mp3_rw
        .seek(std::io::SeekFrom::Start(0))
        .map_err(|e| Error::FileReadFailure(Box::new(e)))?;
    trace!("Reading tags from mp3");
    let mut tag = match id3::Tag::read_from(&mut mp3_rw) {
        Err(id3::Error {
            kind: id3::ErrorKind::NoTag,
            ..
        }) => id3::Tag::new(),
        Ok(tag) => tag,
        err => err?,
    };

    trace!("Updating tags with filesystem metadata");
    if tag.artist().is_none() {
        tag.set_artist(&metadata.artist);
    }
    if tag.album().is_none() {
        tag.set_album(&metadata.album);
    }
    if tag.title().is_none() {
        tag.set_title(&metadata.name);
    }

    trace!("Writing tags back to file");
    mp3_rw
        .seek(std::io::SeekFrom::Start(0))
        .map_err(|e| Error::FileWriteFailure(Box::new(e)))?;
    tag.write_to(&mut mp3_rw, id3::Version::Id3v23)
        .map_err(Error::from)
}

fn save_url_to_writer<W: Write>(url: &str, writer: W) -> Result<()> {
    let mut buf_writer = std::io::BufWriter::new(writer);
    let mut resp = reqwest::blocking::get(url).map_err(Error::from)?;
    resp.copy_to(&mut buf_writer)
        .map_err(|e| Error::FileWriteFailure(Box::new(e)))?;
    Ok(())
}

fn filename_formatter(text: &str) -> String {
    text.replace("/", "_")
        .replace("\\", "_")
        .replace(":", "_")
        .replace("-", "_")
}
