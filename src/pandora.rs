#![allow(dead_code)]

use std::cell::RefCell;
use std::rc::Rc;

use log::trace;

use pandora_api;
use pandora_api::json::auth::{PartnerLogin, UserLogin};
use pandora_api::json::music::*;
use pandora_api::json::station::*;
use pandora_api::json::track::*;
pub use pandora_api::json::user::Station;
use pandora_api::json::user::*;
use pandora_api::json::{PandoraApiRequest, ToEncryptionTokens};

use crate::config::Config;
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

const ANDROID_ENDPOINT: &str = "https://tuner.pandora.com/services/json";

/// Encapsulates all data that needs to be tracked as part of a login session
/// with Pandora.  The actual reqwest Client is created by and stored on the
/// pandora_api::json::PandoraSession, which we wrap here.
#[derive(Debug, Clone)]
pub(crate) struct PandoraSession {
    config: Rc<RefCell<Config>>,
    inner: pandora_api::json::PandoraSession,
}

// TODO: filter responses for ones that indicate the session is
// no longer valid, re-create session and retry?
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
        trace!("Partner logout");
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

        trace!("Partner login");
        PartnerLogin::new(
            "android",
            "AC7IBG09A3DTSYM4R41UJWL07VLN8JI7",
            "android-generic",
            Some("5".to_string()),
        )
        .merge_response(&mut self.inner)?;
        trace!("Partner login successful");

        Ok(())
    }

    pub fn user_logout(&mut self) {
        trace!("User logout");
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

        trace!("User login");
        let username_opt = self.config.borrow().login.get_username();
        let username = username_opt.ok_or_else(|| Error::PanharmoniconMissingAuthToken)?;

        let password_opt = self.config.borrow().login.get_password()?;
        let password = password_opt.ok_or_else(|| Error::PanharmoniconMissingAuthToken)?;

        UserLogin::new(&username, &password).merge_response(&mut self.inner)?;
        trace!("User login successful");
        Ok(())
    }

    pub fn search(&mut self, text: &str) -> Result<SearchResponse> {
        self.user_login()?;
        trace!("search()");
        Search::from(&text)
            .response(&self.inner)
            .map_err(Error::from)
    }

    pub fn get_track(&mut self, music_id: &str) -> Result<GetTrackResponse> {
        self.user_login()?;
        trace!("getTrack()");
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
        trace!("addFeedback()");
        AddFeedback::new(station_token, track_token, is_positive)
            .response(&self.inner)
            .map_err(Error::from)
    }

    pub fn delete_feedback(&mut self, feedback_id: &str) -> Result<()> {
        self.user_login()?;
        trace!("deleteFeedback()");
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
        trace!("addMusic()");
        AddMusic::new(station_token, music_token)
            .response(&self.inner)
            .map_err(Error::from)
    }

    pub fn delete_music(&mut self, seed_id: &str) -> Result<()> {
        self.user_login()?;
        trace!("deleteMusic()");
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
        trace!("createStation()");
        CreateStation::new_from_track_song(track_token)
            .response(&self.inner)
            .map_err(Error::from)
    }

    pub fn create_station_from_track_artist(
        &mut self,
        track_token: &str,
    ) -> Result<CreateStationResponse> {
        self.user_login()?;
        trace!("createStation()");
        CreateStation::new_from_track_artist(track_token)
            .response(&self.inner)
            .map_err(Error::from)
    }

    pub fn create_station_from_music_token(
        &mut self,
        music_token: &str,
    ) -> Result<CreateStationResponse> {
        self.user_login()?;
        trace!("createStation()");
        CreateStation::new_from_music_token(music_token)
            .response(&self.inner)
            .map_err(Error::from)
    }

    pub fn delete_station(&mut self, station_token: &str) -> Result<()> {
        self.user_login()?;
        trace!("deleteStation()");
        DeleteStation::from(&station_token)
            .response(&self.inner)
            .map(|_: DeleteStationResponse| ())
            .map_err(Error::from)
    }

    pub fn get_genre_stations(&mut self) -> Result<Vec<GenreCategory>> {
        self.user_login()?;
        trace!("getGenreStations()");
        GetGenreStations::new()
            .response(&self.inner)
            .map(|gr: GetGenreStationsResponse| gr.categories)
            .map_err(Error::from)
    }

    pub fn get_genre_stations_checksum(&mut self) -> Result<String> {
        self.user_login()?;
        trace!("getGenreStationsChecksum()");
        GetGenreStationsChecksum::new()
            .response(&self.inner)
            .map(|cr: GetGenreStationsChecksumResponse| cr.checksum)
            .map_err(Error::from)
    }

    pub fn get_playlist(&mut self, station_token: &str) -> Result<Vec<PlaylistEntry>> {
        self.user_login()?;
        trace!("getPlaylist()");
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
        trace!("getStation()");
        let mut gs = GetStation::from(&station_token);
        gs.include_extended_attributes = Some(extended_attributes);
        gs.response(&self.inner).map_err(Error::from)
    }

    pub fn rename_station(&mut self, station_token: &str, station_name: &str) -> Result<()> {
        self.user_login()?;
        trace!("renameStation()");
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
        trace!("shareStation()");
        let mut ss = ShareStation::new(station_id, station_token);
        ss.emails = emails;
        ss.response(&self.inner)
            .map(|_: ShareStationResponse| ())
            .map_err(Error::from)
    }

    pub fn transform_shared_station(&mut self, station_token: &str) -> Result<()> {
        self.user_login()?;
        trace!("transformSharedStation()");
        TransformSharedStation::from(&station_token)
            .response(&self.inner)
            .map(|_: TransformSharedStationResponse| ())
            .map_err(Error::from)
    }

    pub fn explain_track(&mut self, track_token: &str) -> Result<ExplainTrackResponse> {
        self.user_login()?;
        trace!("explainTrack()");
        ExplainTrack::from(&track_token)
            .response(&self.inner)
            .map_err(Error::from)
    }

    pub fn validate_username(&mut self, username: &str) -> Result<ValidateUsernameResponse> {
        self.partner_login()?;
        trace!("validateUsername()");
        ValidateUsername::from(&username)
            .response(&self.inner)
            .map_err(Error::from)
    }

    pub fn password_recovery(&mut self, username: &str) -> Result<()> {
        self.partner_login()?;
        trace!("emailPassword()");
        EmailPassword::from(&username)
            .response(&self.inner)
            .map(|_: EmailPasswordResponse| ())
            .map_err(Error::from)
    }

    pub fn get_bookmarks(&mut self) -> Result<GetBookmarksResponse> {
        self.user_login()?;
        trace!("getBookmarks()");
        GetBookmarks::new()
            .response(&self.inner)
            .map_err(Error::from)
    }

    pub fn get_station_list_checksum(&mut self) -> Result<String> {
        self.user_login()?;
        trace!("getStationListChecksum()");
        GetStationListChecksum::new()
            .response(&self.inner)
            .map(|sc: GetStationListChecksumResponse| sc.checksum)
            .map_err(Error::from)
    }

    pub fn get_station_list(&mut self) -> Result<GetStationListResponse> {
        self.user_login()?;
        trace!("getStationList()");
        GetStationList::new()
            .response(&self.inner)
            .map_err(Error::from)
    }

    pub fn get_usage_info(&mut self) -> Result<GetUsageInfoResponse> {
        self.user_login()?;
        trace!("getUsageInfo()");
        GetUsageInfo::new()
            .response(&self.inner)
            .map_err(Error::from)
    }

    pub fn set_quick_mix(&mut self, quick_mix_station_ids: Vec<String>) -> Result<()> {
        self.user_login()?;
        trace!("setQuickMix()");
        let mut sqm = SetQuickMix::new();
        sqm.quick_mix_station_ids = quick_mix_station_ids;
        sqm.response(&self.inner)
            .map(|_: SetQuickMixResponse| ())
            .map_err(Error::from)
    }

    pub fn sleep_song(&mut self, track_token: &str) -> Result<()> {
        self.user_login()?;
        trace!("sleepSong()");
        SleepSong::from(&track_token)
            .response(&self.inner)
            .map(|_: SleepSongResponse| ())
            .map_err(Error::from)
    }
}
