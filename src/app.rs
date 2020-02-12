use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::{Duration, Instant};

use clap::crate_name;
use log::{debug, error, trace};
use reqwest;
use rodio::source::Source;
use rodio::DeviceTrait;

use pandora_api;
use pandora_api::json::auth::{PartnerLogin, UserLogin};
use pandora_api::json::music::*;
use pandora_api::json::station::*;
use pandora_api::json::track::*;
pub use pandora_api::json::user::Station;
use pandora_api::json::user::*;
use pandora_api::json::{PandoraApiRequest, ToEncryptionTokens};

use crate::config;
use crate::config::{Config, PartialConfig};
use crate::crossterm as term;
use crate::errors::{Error, Result};

/// Partner encrypt/decryption data type.
struct PartnerKeys {
    encrypt: String,
    decrypt: String,
}

impl PartnerKeys {
    /// Create a new instance of partner keys with the keys
    /// for the "android" partner.
    fn new_android() -> Self {
        Self {
            encrypt: String::from("6#26FRL$ZWD"),
            decrypt: String::from("R=U!LH$O2B#"),
        }
    }
}

impl ToEncryptionTokens for PartnerKeys {
    fn to_encrypt_key(&self) -> String {
        self.encrypt.clone()
    }

    fn to_decrypt_key(&self) -> String {
        self.decrypt.clone()
    }
}

const ANDROID_ENDPOINT: &'static str = "https://tuner.pandora.com/services/json";

/// Encapsulates all data that needs to be tracked as part of a login session
/// with Pandora.  The actual reqwest Client is created by and stored on the
/// pandora_api::json::PandoraSession, which we wrap here.
#[derive(Debug, Clone)]
pub(crate) struct PandoraSession {
    config: Rc<RefCell<Config>>,
    inner: pandora_api::json::PandoraSession,
}

impl PandoraSession {
    /// Instantiate a new PandoraSession.
    pub fn new(config: Rc<RefCell<Config>>) -> Self {
        let inner: pandora_api::json::PandoraSession = pandora_api::json::PandoraSession::new(
            None,
            &PartnerKeys::new_android(),
            &String::from(ANDROID_ENDPOINT),
        );
        Self { config, inner }
    }

    pub fn connected(&self) -> bool {
        let session_tokens = self.inner.session_tokens();
        session_tokens
            .partner_id
            .as_ref()
            .and(session_tokens.partner_token.as_ref())
            .and(session_tokens.get_sync_time().as_ref())
            .and(session_tokens.user_id.as_ref())
            .and(session_tokens.user_token.as_ref())
            .is_some()
    }

    /// Erase all session tokens, both user and application.
    pub fn partner_logout(&mut self) {
        self.user_logout();
        let session_tokens = self.inner.session_tokens_mut();
        session_tokens.clear_sync_time();
        session_tokens.partner_id = None;
        session_tokens.partner_token = None;
    }

    /// Authenticate the partner (application) with Pandora.  This is separate
    /// from, and a pre-requisite to, user authentication.  It is not generally
    /// necessary to call this function directly, though, as each method will
    /// authenticate as much as necessary to complete the request.
    pub fn partner_login(&mut self) -> Result<()> {
        let session_tokens = self.inner.session_tokens();
        let session_sync_time = session_tokens.get_sync_time();
        if session_tokens
            .partner_id
            .as_ref()
            .and(session_tokens.partner_token.as_ref())
            .and(session_sync_time.as_ref())
            .is_some()
        {
            return Ok(());
        }

        PartnerLogin::new(
            "android",
            "AC7IBG09A3DTSYM4R41UJWL07VLN8JI7",
            "android-generic",
            Some("5".to_string()),
        )
        .merge_response(&mut self.inner)?;

        Ok(())
    }

    pub fn user_logout(&mut self) {
        let session_tokens = self.inner.session_tokens_mut();
        session_tokens.user_id = None;
        session_tokens.user_token = None;
    }

    /// Authenticate the user with Pandora.  If partner (application)
    /// authentication has not already been performed, it will also do that.
    /// It is not generally necessary to call this function directly, though,
    /// as each method will authenticate as much as necessary to complete
    /// the request.
    pub fn user_login(&mut self) -> Result<()> {
        self.partner_login()?;
        let session_tokens = self.inner.session_tokens();
        if session_tokens
            .user_id
            .as_ref()
            .and(session_tokens.user_token.as_ref())
            .is_some()
        {
            return Ok(());
        }

        let username_opt = self.config.borrow().login.get_username();
        let username = username_opt.ok_or_else(|| Error::PanharmoniconMissingAuthToken)?;

        let password_opt = self.config.borrow().login.get_password()?;
        let password = password_opt.ok_or_else(|| Error::PanharmoniconMissingAuthToken)?;

        UserLogin::new(&username, &password).merge_response(&mut self.inner)?;
        Ok(())
    }

    pub fn search(&mut self, text: &str) -> Result<SearchResponse> {
        self.user_login()?;
        Search::from(&text)
            .response(&self.inner)
            .map_err(Error::from)
    }

    pub fn get_track(&mut self, music_id: &str) -> Result<GetTrackResponse> {
        self.user_login()?;
        GetTrack::from(&music_id)
            .response(&self.inner)
            .map_err(Error::from)
    }

    pub fn add_feedback(
        &mut self,
        station_token: &str,
        track_token: &str,
        is_positive: bool,
    ) -> Result<AddFeedbackResponse> {
        self.user_login()?;
        AddFeedback::new(station_token, track_token, is_positive)
            .response(&self.inner)
            .map_err(Error::from)
    }

    pub fn delete_feedback(&mut self, feedback_id: &str) -> Result<()> {
        self.user_login()?;
        DeleteFeedback::from(&feedback_id)
            .response(&self.inner)
            .map_err(Error::from)?;
        Ok(())
    }

    pub fn add_music(
        &mut self,
        station_token: &str,
        music_token: &str,
    ) -> Result<AddMusicResponse> {
        self.user_login()?;
        AddMusic::new(station_token, music_token)
            .response(&self.inner)
            .map_err(Error::from)
    }

    pub fn delete_music(&mut self, seed_id: &str) -> Result<()> {
        self.user_login()?;
        DeleteMusic::from(&seed_id)
            .response(&self.inner)
            .map(|_: DeleteMusicResponse| ())
            .map_err(Error::from)
    }

    pub fn create_station_from_track_song(
        &mut self,
        track_token: &str,
    ) -> Result<CreateStationResponse> {
        self.user_login()?;
        CreateStation::new_from_track_song(track_token)
            .response(&self.inner)
            .map_err(Error::from)
    }

    pub fn create_station_from_track_artist(
        &mut self,
        track_token: &str,
    ) -> Result<CreateStationResponse> {
        self.user_login()?;
        CreateStation::new_from_track_artist(track_token)
            .response(&self.inner)
            .map_err(Error::from)
    }

    pub fn create_station_from_music_token(
        &mut self,
        music_token: &str,
    ) -> Result<CreateStationResponse> {
        self.user_login()?;
        CreateStation::new_from_music_token(music_token)
            .response(&self.inner)
            .map_err(Error::from)
    }

    pub fn delete_station(&mut self, station_token: &str) -> Result<()> {
        self.user_login()?;
        DeleteStation::from(&station_token)
            .response(&self.inner)
            .map(|_: DeleteStationResponse| ())
            .map_err(Error::from)
    }

    pub fn get_genre_stations(&mut self) -> Result<Vec<GenreCategory>> {
        self.user_login()?;
        GetGenreStations::new()
            .response(&self.inner)
            .map(|gr: GetGenreStationsResponse| gr.categories)
            .map_err(Error::from)
    }

    pub fn get_genre_stations_checksum(&mut self) -> Result<String> {
        self.user_login()?;
        GetGenreStationsChecksum::new()
            .response(&self.inner)
            .map(|cr: GetGenreStationsChecksumResponse| cr.checksum)
            .map_err(Error::from)
    }

    pub fn get_playlist(&mut self, station_token: &str) -> Result<Vec<PlaylistEntry>> {
        self.user_login()?;
        GetPlaylist::from(&station_token)
            .response(&self.inner)
            .map(|pr: GetPlaylistResponse| pr.items)
            .map_err(Error::from)
    }

    pub fn get_station(
        &mut self,
        station_token: &str,
        extended_attributes: bool,
    ) -> Result<GetStationResponse> {
        self.user_login()?;
        let mut gs = GetStation::from(&station_token);
        gs.include_extended_attributes = Some(extended_attributes);
        gs.response(&self.inner).map_err(Error::from)
    }

    pub fn rename_station(&mut self, station_token: &str, station_name: &str) -> Result<()> {
        self.user_login()?;
        RenameStation::new(station_token, station_name)
            .response(&self.inner)
            .map(|_: RenameStationResponse| ())
            .map_err(Error::from)
    }

    pub fn share_station(
        &mut self,
        station_id: &str,
        station_token: &str,
        emails: Vec<String>,
    ) -> Result<()> {
        self.user_login()?;
        let mut ss = ShareStation::new(station_id, station_token);
        ss.emails = emails;
        ss.response(&self.inner)
            .map(|_: ShareStationResponse| ())
            .map_err(Error::from)
    }

    pub fn transform_shared_station(&mut self, station_token: &str) -> Result<()> {
        self.user_login()?;
        TransformSharedStation::from(&station_token)
            .response(&self.inner)
            .map(|_: TransformSharedStationResponse| ())
            .map_err(Error::from)
    }

    pub fn explain_track(&mut self, track_token: &str) -> Result<ExplainTrackResponse> {
        self.user_login()?;
        ExplainTrack::from(&track_token)
            .response(&self.inner)
            .map_err(Error::from)
    }

    pub fn validate_username(&mut self, username: &str) -> Result<ValidateUsernameResponse> {
        self.partner_login()?;
        ValidateUsername::from(&username)
            .response(&self.inner)
            .map_err(Error::from)
    }

    pub fn password_recovery(&mut self, username: &str) -> Result<()> {
        self.partner_login()?;
        EmailPassword::from(&username)
            .response(&self.inner)
            .map(|_: EmailPasswordResponse| ())
            .map_err(Error::from)
    }

    pub fn get_bookmarks(&mut self) -> Result<GetBookmarksResponse> {
        self.user_login()?;
        GetBookmarks::new()
            .response(&self.inner)
            .map_err(Error::from)
    }

    pub fn get_station_list_checksum(&mut self) -> Result<String> {
        self.user_login()?;
        GetStationListChecksum::new()
            .response(&self.inner)
            .map(|sc: GetStationListChecksumResponse| sc.checksum)
            .map_err(Error::from)
    }

    pub fn get_station_list(&mut self) -> Result<GetStationListResponse> {
        self.user_login()?;
        GetStationList::new()
            .response(&self.inner)
            .map_err(Error::from)
    }

    pub fn get_usage_info(&mut self) -> Result<GetUsageInfoResponse> {
        self.user_login()?;
        GetUsageInfo::new()
            .response(&self.inner)
            .map_err(Error::from)
    }

    pub fn set_quick_mix(&mut self, quick_mix_station_ids: Vec<String>) -> Result<()> {
        self.user_login()?;
        let mut sqm = SetQuickMix::new();
        sqm.quick_mix_station_ids = quick_mix_station_ids;
        sqm.response(&self.inner)
            .map(|_: SetQuickMixResponse| ())
            .map_err(Error::from)
    }

    pub fn sleep_song(&mut self, track_token: &str) -> Result<()> {
        self.user_login()?;
        SleepSong::from(&track_token)
            .response(&self.inner)
            .map(|_: SleepSongResponse| ())
            .map_err(Error::from)
    }
}

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

    pub(crate) fn get_audio(&self, quality: Quality) -> Option<Audio> {
        self.audio.iter().find(|&a| a.quality == quality).cloned()
    }
}

pub(crate) struct Panharmonicon {
    ui: term::Terminal,
    config: Rc<RefCell<Config>>,
    session: PandoraSession,
    audio_device: rodio::Device,
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
            audio_device,
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
            .map(|pt| SongInfo::from(pt))
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
            let mut playing = Playing::from(&track);
            let cached_media = self.get_cached_media(&playing)?;
            let duration = read_media_duration(&cached_media)?;
            playing.duration = duration;
            playing.remaining = duration;
            self.ui.display_playing(&playing.info, &duration);
            self.playing = Some(playing);
            let source = rodio::decoder::Decoder::new(
                std::io::BufReader::new(
                    std::fs::File::open(cached_media)
                        .map_err(|e| Error::MediaReadFailure(Box::new(e)))?
                ),
            )?
            /*
            .amplify(track.track_gain.parse::<f32>().unwrap_or(1.0f32))
            // In spite of how this looks, this actually makes the source
            // pausable, it just makes it not initially paused.
            .pausable(false);
            */;
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
            /*
            self.audio_sink.sleep_until_end();
            self.playing = None;
            */
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
    mp3_duration::from_read(&mut stream).map_err(|_| Error::Mp3MediaParseFailure)
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
