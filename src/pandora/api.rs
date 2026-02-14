#![allow(dead_code)]

use anyhow::{Context, Result};
use log::trace;

use pandora_api::json::auth::{PartnerLogin, UserLogin};
use pandora_api::json::music::*;
use pandora_api::json::station::*;
use pandora_api::json::track::*;
use pandora_api::json::user::*;
use pandora_api::json::{PandoraJsonApiRequest, ToEncryptionTokens};

use crate::config::SharedConfig;
use crate::errors::Error;
use crate::track::Track;

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
    config: SharedConfig,
    inner: pandora_api::json::PandoraSession,
}

// TODO: filter responses for ones that indicate the session is
// no longer valid, re-create session and retry?
impl PandoraSession {
    /// Instantiate a new PandoraSession.
    pub fn new(config: SharedConfig) -> Self {
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
    pub async fn partner_logout(&mut self) {
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
    pub async fn partner_login(&mut self) -> Result<()> {
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
        .merge_response(&mut self.inner)
        .await?;
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
    pub async fn user_login(&mut self) -> Result<()> {
        self.partner_login()
            .await
            .context("Failed to ensure valid partner login before authenticating user")?;
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
        let username_opt = self
            .config
            .read()
            .expect("config read for user_login username")
            .login
            .username();
        let username = username_opt.ok_or(Error::PanharmoniconMissingAuthToken)?;

        let password_opt = self
            .config
            .read()
            .expect("config read for user_login password")
            .login
            .password()?;
        let password = password_opt.ok_or(Error::PanharmoniconMissingAuthToken)?;

        UserLogin::new(&username, &password)
            .merge_response(&mut self.inner)
            .await?;
        trace!("User login successful");
        Ok(())
    }

    pub async fn search(&mut self, text: &str) -> Result<SearchResponse> {
        self.user_login()
            .await
            .context("Failed to ensure valid user login before completing search request")?;
        trace!("search()");
        let request = Search::from(&text);
        // Catch request errors, reconnect, and retry
        match request.response(&mut self.inner).await {
            Err(_) => {
                self.user_login().await.context(
                    "Failed to ensure valid user login before retrying add feedback request",
                )?;
                request.response(&mut self.inner).await
            }
            res => res,
        }
        .map_err(anyhow::Error::from)
    }

    pub async fn get_track(&mut self, music_id: &str) -> Result<GetTrackResponse> {
        trace!("getTrack()");
        let request = GetTrack::from(&music_id);
        // Catch request errors, reconnect, and retry
        match request.response(&mut self.inner).await {
            Err(_) => {
                self.user_login().await.context(
                    "Failed to ensure valid user login before retrying add feedback request",
                )?;
                request.response(&mut self.inner).await
            }
            res => res,
        }
        .map_err(anyhow::Error::from)
    }

    pub async fn add_feedback(
        &mut self,
        track: &Track,
        is_positive: bool,
    ) -> Result<AddFeedbackResponse> {
        trace!("addFeedback()");
        let request = AddFeedback::new(&track.station_id, &track.track_token, is_positive);
        // Catch request errors, reconnect, and retry
        match request.response(&mut self.inner).await {
            Err(_) => {
                self.user_login().await.context(
                    "Failed to ensure valid user login before retrying add feedback request",
                )?;
                request.response(&mut self.inner).await
            }
            res => res,
        }
        .map_err(anyhow::Error::from)
    }

    pub async fn delete_feedback_for_track(&mut self, track: &Track) -> Result<()> {
        trace!("deleteFeedback() [delete_feedback_for_track]");
        trace!("Looking up musicToken for current track");
        let music_token = self.get_track(&track.music_id).await?.music_token;
        trace!("Getting station feedback...");
        if let Some(feedback) = self.get_station(&track.station_id, true).await?.feedback {
            trace!("Looking through station feedback for feedback on this track.");
            let thumbs_up = feedback.thumbs_up.iter();
            let thumbs_down = feedback.thumbs_down.iter();
            if let Some(feedback_id) = thumbs_up
                .chain(thumbs_down)
                .find(|fb| fb.music_token == music_token)
                .map(|fb| fb.feedback_id.clone())
            {
                trace!("Deleting feedback for song {}", track.title);
                self.delete_feedback(&feedback_id).await?;
            } else {
                trace!("No feedback for song {} to delete.", track.title);
            }
        } else {
            trace!("Request to remove feedback for track that is unrated");
        }
        Ok(())
    }

    pub async fn delete_feedback(&mut self, feedback_id: &str) -> Result<()> {
        trace!("deleteFeedback()");
        let request = DeleteFeedback::from(&feedback_id);
        // Catch request errors, reconnect, and retry
        match request.response(&mut self.inner).await {
            Err(_) => {
                self.user_login().await.context(
                    "Failed to ensure valid user login before retrying add feedback request",
                )?;
                request.response(&mut self.inner).await
            }
            res => res,
        }
        .map(|_| ())
        .map_err(anyhow::Error::from)
    }

    pub async fn add_music(
        &mut self,
        station_token: &str,
        music_token: &str,
    ) -> Result<AddMusicResponse> {
        trace!("addMusic()");
        let request = AddMusic::new(station_token, music_token);
        // Catch request errors, reconnect, and retry
        match request.response(&mut self.inner).await {
            Err(_) => {
                self.user_login().await.context(
                    "Failed to ensure valid user login before retrying add feedback request",
                )?;
                request.response(&mut self.inner).await
            }
            res => res,
        }
        .map_err(anyhow::Error::from)
    }

    pub async fn delete_music(&mut self, seed_id: &str) -> Result<()> {
        trace!("deleteMusic()");
        let request = DeleteMusic::from(&seed_id);
        // Catch request errors, reconnect, and retry
        match request.response(&mut self.inner).await {
            Err(_) => {
                self.user_login().await.context(
                    "Failed to ensure valid user login before retrying add feedback request",
                )?;
                request.response(&mut self.inner).await
            }
            res => res,
        }
        .map_err(anyhow::Error::from)
        .map(|_: DeleteMusicResponse| ())
    }

    pub async fn create_station_from_track_song(
        &mut self,
        track_token: &str,
    ) -> Result<CreateStationResponse> {
        trace!("createStation()");
        let request = CreateStation::new_from_track_song(track_token);
        // Catch request errors, reconnect, and retry
        match request.response(&mut self.inner).await {
            Err(_) => {
                self.user_login().await.context(
                    "Failed to ensure valid user login before retrying add feedback request",
                )?;
                request.response(&mut self.inner).await
            }
            res => res,
        }
        .map_err(anyhow::Error::from)
    }

    pub async fn create_station_from_track_artist(
        &mut self,
        track_token: &str,
    ) -> Result<CreateStationResponse> {
        trace!("createStation()");
        let request = CreateStation::new_from_track_artist(track_token);
        // Catch request errors, reconnect, and retry
        match request.response(&mut self.inner).await {
            Err(_) => {
                self.user_login().await.context(
                    "Failed to ensure valid user login before retrying add feedback request",
                )?;
                request.response(&mut self.inner).await
            }
            res => res,
        }
        .map_err(anyhow::Error::from)
    }

    pub async fn create_station_from_music_token(
        &mut self,
        music_token: &str,
    ) -> Result<CreateStationResponse> {
        trace!("createStation()");
        let request = CreateStation::new_from_music_token(music_token);
        // Catch request errors, reconnect, and retry
        match request.response(&mut self.inner).await {
            Err(_) => {
                self.user_login().await.context(
                    "Failed to ensure valid user login before retrying add feedback request",
                )?;
                request.response(&mut self.inner).await
            }
            res => res,
        }
        .map_err(anyhow::Error::from)
    }

    pub async fn delete_station(&mut self, station_token: &str) -> Result<()> {
        trace!("deleteStation()");
        let request = DeleteStation::from(&station_token);
        // Catch request errors, reconnect, and retry
        match request.response(&mut self.inner).await {
            Err(_) => {
                self.user_login().await.context(
                    "Failed to ensure valid user login before retrying add feedback request",
                )?;
                request.response(&mut self.inner).await
            }
            res => res,
        }
        .map(|_: DeleteStationResponse| ())
        .map_err(anyhow::Error::from)
    }

    pub async fn get_genre_stations(&mut self) -> Result<Vec<GenreCategory>> {
        trace!("getGenreStations()");
        let request = GetGenreStations::new();
        // Catch request errors, reconnect, and retry
        match request.response(&mut self.inner).await {
            Err(_) => {
                self.user_login().await.context(
                    "Failed to ensure valid user login before retrying add feedback request",
                )?;
                request.response(&mut self.inner).await
            }
            res => res,
        }
        .map(|gr: GetGenreStationsResponse| gr.categories)
        .map_err(anyhow::Error::from)
    }

    pub async fn get_genre_stations_checksum(&mut self) -> Result<String> {
        trace!("getGenreStationsChecksum()");
        let request = GetGenreStationsChecksum::new();
        // Catch request errors, reconnect, and retry
        match request.response(&mut self.inner).await {
            Err(_) => {
                self.user_login().await.context(
                    "Failed to ensure valid user login before retrying add feedback request",
                )?;
                request.response(&mut self.inner).await
            }
            res => res,
        }
        .map(|cr: GetGenreStationsChecksumResponse| cr.checksum)
        .map_err(anyhow::Error::from)
    }

    pub async fn get_playlist(&mut self, station_token: &str) -> Result<Vec<PlaylistEntry>> {
        trace!("getPlaylist()");
        let request = GetPlaylist::from(&station_token).include_track_length(true);
        // Catch request errors, reconnect, and retry
        match request.response(&mut self.inner).await {
            Err(_) => {
                self.user_login().await.context(
                    "Failed to ensure valid user login before retrying add feedback request",
                )?;
                request.response(&mut self.inner).await
            }
            res => res,
        }
        .map(|pr: GetPlaylistResponse| pr.items)
        .map_err(anyhow::Error::from)
    }

    pub async fn get_station(
        &mut self,
        station_token: &str,
        extended_attributes: bool,
    ) -> Result<GetStationResponse> {
        trace!("getStation()");
        let request =
            GetStation::from(&station_token).include_extended_attributes(extended_attributes);
        // Catch request errors, reconnect, and retry
        match request.response(&mut self.inner).await {
            Err(_) => {
                self.user_login().await.context(
                    "Failed to ensure valid user login before retrying add feedback request",
                )?;
                request.response(&mut self.inner).await
            }
            res => res,
        }
        .map_err(anyhow::Error::from)
    }

    /// Get seeds (artists, songs, genres) for a station. Uses extended station attributes.
    pub async fn get_station_seeds(
        &mut self,
        station_token: &str,
    ) -> Result<crate::pandora::StationSeedsData> {
        use crate::pandora::{ArtistSeedInfo, SongSeedInfo, StationSeedsData};
        let resp = self.get_station(station_token, true).await?;
        let music = resp.music.unwrap_or(StationSeeds {
            songs: vec![],
            artists: vec![],
            genres: vec![],
        });
        let artist_seeds = music
            .artists
            .into_iter()
            .map(|a| ArtistSeedInfo {
                seed_id: a.seed_id,
                music_token: a.music_token,
                artist_name: a.artist_name,
            })
            .collect();
        let song_seeds = music
            .songs
            .into_iter()
            .map(|s| SongSeedInfo {
                seed_id: s.seed_id,
                music_token: s.music_token,
                song_name: s.song_name,
                artist_name: s.artist_name,
            })
            .collect();
        Ok(StationSeedsData {
            artist_seeds,
            song_seeds,
        })
    }

    /// Get rated tracks (thumbs up / thumbs down) for a station. Uses extended station attributes.
    pub async fn get_station_rated_tracks(
        &mut self,
        station_token: &str,
    ) -> Result<crate::pandora::StationRatedTracksData> {
        use crate::pandora::{RatedTrackInfo, StationRatedTracksData};
        let resp = self.get_station(station_token, true).await?;
        let feedback = resp.feedback.unwrap_or(StationFeedback {
            thumbs_up: vec![],
            total_thumbs_up: 0,
            thumbs_down: vec![],
            total_thumbs_down: 0,
        });
        let thumbs_up = feedback
            .thumbs_up
            .into_iter()
            .map(|t| RatedTrackInfo {
                feedback_id: t.feedback_id,
                music_token: t.music_token,
                song_name: t.song_name,
                artist_name: t.artist_name,
                is_positive: t.is_positive,
            })
            .collect();
        let thumbs_down = feedback
            .thumbs_down
            .into_iter()
            .map(|t| RatedTrackInfo {
                feedback_id: t.feedback_id,
                music_token: t.music_token,
                song_name: t.song_name,
                artist_name: t.artist_name,
                is_positive: t.is_positive,
            })
            .collect();
        Ok(StationRatedTracksData {
            thumbs_up,
            thumbs_down,
        })
    }

    pub async fn rename_station(&mut self, station_token: &str, station_name: &str) -> Result<()> {
        trace!("renameStation()");
        let request = RenameStation::new(station_token, station_name);
        // Catch request errors, reconnect, and retry
        match request.response(&mut self.inner).await {
            Err(_) => {
                self.user_login().await.context(
                    "Failed to ensure valid user login before retrying add feedback request",
                )?;
                request.response(&mut self.inner).await
            }
            res => res,
        }
        .map(|_: RenameStationResponse| ())
        .map_err(anyhow::Error::from)
    }

    pub async fn share_station(
        &mut self,
        station_id: &str,
        station_token: &str,
        emails: Vec<String>,
    ) -> Result<()> {
        trace!("shareStation()");
        let mut request = ShareStation::new(station_id, station_token);
        request.emails = emails;
        // Catch request errors, reconnect, and retry
        match request.response(&mut self.inner).await {
            Err(_) => {
                self.user_login().await.context(
                    "Failed to ensure valid user login before retrying add feedback request",
                )?;
                request.response(&mut self.inner).await
            }
            res => res,
        }
        .map(|_: ShareStationResponse| ())
        .map_err(anyhow::Error::from)
    }

    pub async fn transform_shared_station(&mut self, station_token: &str) -> Result<()> {
        trace!("transformSharedStation()");
        let request = TransformSharedStation::from(&station_token);
        // Catch request errors, reconnect, and retry
        match request.response(&mut self.inner).await {
            Err(_) => {
                self.user_login().await.context(
                    "Failed to ensure valid user login before retrying add feedback request",
                )?;
                request.response(&mut self.inner).await
            }
            res => res,
        }
        .map(|_: TransformSharedStationResponse| ())
        .map_err(anyhow::Error::from)
    }

    pub async fn explain_track(&mut self, track_token: &str) -> Result<ExplainTrackResponse> {
        trace!("explainTrack()");
        let request = ExplainTrack::from(&track_token);
        // Catch request errors, reconnect, and retry
        match request.response(&mut self.inner).await {
            Err(_) => {
                self.user_login().await.context(
                    "Failed to ensure valid user login before retrying add feedback request",
                )?;
                request.response(&mut self.inner).await
            }
            res => res,
        }
        .map_err(anyhow::Error::from)
    }

    pub async fn validate_username(&mut self, username: &str) -> Result<ValidateUsernameResponse> {
        self.partner_login().await.context(
            "Failed to ensure valid partner login before completing validate username request",
        )?;

        trace!("validateUsername()");
        ValidateUsername::from(&username)
            .response(&mut self.inner)
            .await
            .map_err(anyhow::Error::from)
    }

    pub async fn password_recovery(&mut self, username: &str) -> Result<()> {
        self.partner_login().await.context(
            "Failed to ensure valid partner login before completing password recovery request",
        )?;

        trace!("emailPassword()");
        EmailPassword::from(&username)
            .response(&mut self.inner)
            .await
            .map(|_: EmailPasswordResponse| ())
            .map_err(anyhow::Error::from)
    }

    pub async fn get_bookmarks(&mut self) -> Result<GetBookmarksResponse> {
        trace!("getBookmarks()");
        let request = GetBookmarks::new();
        // Catch request errors, reconnect, and retry
        match request.response(&mut self.inner).await {
            Err(_) => {
                self.user_login().await.context(
                    "Failed to ensure valid user login before retrying add feedback request",
                )?;
                request.response(&mut self.inner).await
            }
            res => res,
        }
        .map_err(anyhow::Error::from)
    }

    pub async fn get_station_list_checksum(&mut self) -> Result<String> {
        trace!("getStationListChecksum()");
        let request = GetStationListChecksum::new();
        // Catch request errors, reconnect, and retry
        match request.response(&mut self.inner).await {
            Err(_) => {
                self.user_login().await.context(
                    "Failed to ensure valid user login before retrying add feedback request",
                )?;
                request.response(&mut self.inner).await
            }
            res => res,
        }
        .map(|sc: GetStationListChecksumResponse| sc.checksum)
        .map_err(anyhow::Error::from)
    }

    pub async fn get_station_list(&mut self) -> Result<GetStationListResponse> {
        trace!("getStationList()");
        let request = GetStationList::new();
        // Catch request errors, reconnect, and retry
        match request.response(&mut self.inner).await {
            Err(_) => {
                self.user_login().await.context(
                    "Failed to ensure valid user login before retrying add feedback request",
                )?;
                request.response(&mut self.inner).await
            }
            res => res,
        }
        .map_err(anyhow::Error::from)
    }

    pub async fn get_usage_info(&mut self) -> Result<GetUsageInfoResponse> {
        trace!("getUsageInfo()");
        let request = GetUsageInfo::new();
        // Catch request errors, reconnect, and retry
        match request.response(&mut self.inner).await {
            Err(_) => {
                self.user_login().await.context(
                    "Failed to ensure valid user login before retrying add feedback request",
                )?;
                request.response(&mut self.inner).await
            }
            res => res,
        }
        .map_err(anyhow::Error::from)
    }

    pub async fn set_quick_mix(&mut self, quick_mix_station_ids: Vec<String>) -> Result<()> {
        trace!("setQuickMix()");
        let mut request = SetQuickMix::new();
        request.quick_mix_station_ids = quick_mix_station_ids;
        // Catch request errors, reconnect, and retry
        match request.response(&mut self.inner).await {
            Err(_) => {
                self.user_login().await.context(
                    "Failed to ensure valid user login before retrying add feedback request",
                )?;
                request.response(&mut self.inner).await
            }
            res => res,
        }
        .map(|_: SetQuickMixResponse| ())
        .map_err(anyhow::Error::from)
    }
}
