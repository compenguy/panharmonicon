use std::collections::{HashMap, VecDeque};
use std::convert::TryFrom;
use std::time::Duration;
use std::{cell::RefCell, rc::Rc};

use anyhow::Result;
//use cpal::traits::{DeviceTrait, HostTrait};
use either::Either;
use log::{debug, error, info, trace, warn};
use std::sync::mpsc;

use crate::config::{Config, PartialConfig};
use crate::errors::Error;
use crate::messages::{Request, State, StopReason};
use crate::pandora::PandoraSession;
use crate::track::Track;

pub(crate) type StateSender = async_broadcast::Sender<State>;
pub(crate) type StateReceiver = async_broadcast::Receiver<State>;
pub(crate) type RequestSender = mpsc::Sender<Request>;
pub(crate) type RequestReceiver = mpsc::Receiver<Request>;

const FETCHLIST_MAX_LEN: usize = 4;
const PLAYLIST_MAX_LEN: usize = 12;

// player/volume: f32
// player/muted: bool
// player/track: Either<Track, StopReason>
// player/progress: Option<Duration>
// player/length: Option<Duration>
// pandora/connected: bool
// pandora/station: Option<(String, String)>
// pandora/stations: HashMap<String, String>
// pandora/readylist: VecDeque<Track>
// pandora/fetchlist: Vec<Track>
// panharmonicon/quitting: bool

#[derive(Debug)]
pub(crate) struct Model {
    player_volume: f32,
    player_muted: bool,
    player_paused: bool,
    player_track: Either<StopReason, Track>,
    player_progress: Option<Duration>,
    player_length: Option<Duration>,
    // TODO: Session operations are great candidates for spawning a task to complete
    // I think they're what makes some message processing loops take up to 1s to complete
    // Instead implement these operations in pandora.rs, and convert it to being a standalone
    // subsystem?
    pandora_session: Option<PandoraSession>,
    pandora_station: Option<(String, String)>,
    pandora_stations: HashMap<String, String>,
    pandora_readylist: VecDeque<Track>,
    pandora_fetchlist: Vec<Track>,
    panharmonicon_quitting: bool,
    request_sender: RequestSender,
    request_receiver: RequestReceiver,
    state_sender: StateSender,
    state_receiver: StateReceiver,
    config: Rc<RefCell<Config>>,
    dirty: bool,
}

impl Model {
    pub(crate) fn new(config: Rc<RefCell<Config>>) -> Self {
        let (request_sender, request_receiver) = mpsc::channel();
        let (state_sender, state_receiver) = async_broadcast::broadcast(64);
        let volume = config.borrow().volume();
        Self {
            player_volume: volume,
            player_muted: false,
            player_paused: false,
            player_track: Either::Left(StopReason::Initializing),
            player_progress: None,
            player_length: None,
            pandora_session: None,
            pandora_station: None,
            pandora_stations: HashMap::with_capacity(16),
            pandora_readylist: VecDeque::with_capacity(PLAYLIST_MAX_LEN),
            pandora_fetchlist: Vec::with_capacity(FETCHLIST_MAX_LEN),
            panharmonicon_quitting: false,
            request_sender,
            request_receiver,
            state_sender,
            state_receiver,
            config,
            dirty: true,
        }
    }

    // For handing out channels to the subsystems for sending us `Request`s
    pub(crate) fn request_channel(&self) -> RequestSender {
        self.request_sender.clone()
    }

    // For handing out channels to the subsystems for getting our `State` updates
    pub(crate) fn updates_channel(&self) -> StateReceiver {
        self.state_receiver.clone()
    }

    // Internal method for sending state updates to the subsystems
    async fn publish_state(&mut self, state: State) -> Result<()> {
        debug!("State update: {state:?}");
        self.state_sender.broadcast(state).await?;
        //log::debug!("state update channel pending message count: {}", self.state_sender.len());
        Ok(())
    }

    async fn connect_internal(&mut self, mut session: PandoraSession) -> Result<()> {
        if self.connected() {
            debug!("Pandora session was already connected, but a new connection was requested");
        }
        trace!("Connecting to Pandora...");
        session.partner_login().await?;
        session.user_login().await?;
        self.pandora_session = Some(session);
        if self.connected() {
            trace!("Connected to Pandora.");
        } else {
            error!("Just connected to Pandora, but the session reports that it is not connected!");
        }
        Ok(())
    }

    pub(crate) async fn connect(&mut self) -> Result<()> {
        if self.connected() {
            info!("Connect request ignored. Already connected.");
        } else {
            trace!("Attempting pandora login...");
            self.dirty |= true;
            let session = PandoraSession::new(self.config.clone());
            if let Err(e) = self.connect_internal(session).await {
                let message: String = if e
                    .downcast_ref::<Error>()
                    .map(|e| e.missing_auth_token())
                    .unwrap_or(false)
                {
                    String::from("Required authentication token is missing.")
                } else if let Some(e) = e.downcast_ref::<pandora_api::errors::Error>() {
                    format!("Pandora authentication failure: {e:#}").to_string()
                } else {
                    format!("Unknown error while logging in: {e:#}").to_string()
                };
                error!("{message}");
                self.publish_state(State::AuthFailed(message)).await?;
                self.disconnect().await?;
                return Ok(());
            }
            trace!("Successfully logged into Pandora.");
        }
        trace!("send notification 'connected'");
        self.publish_state(State::Connected).await?;

        // If a station was saved, send a Tuned notification for it
        if let Some(station_id) = self.tuned() {
            trace!("send notification 'tuned'");
            self.publish_state(State::Tuned(station_id)).await?;
        }

        // Notify request_senders what the last set volume was
        self.publish_state(State::Volume(self.volume())).await?;

        Ok(())
    }

    pub(crate) fn connected(&self) -> bool {
        self.pandora_session
            .as_ref()
            .map(|session| session.connected())
            .unwrap_or(false)
    }

    pub(crate) async fn disconnect(&mut self) -> Result<()> {
        if let Some(session) = &mut self.pandora_session {
            trace!("Disconnecting from Pandora...");
            session.partner_logout().await;
            trace!("Disconnected from Pandora.");
        }
        self.pandora_session = None;
        self.clear_stations().await?;
        self.dirty |= true;
        self.publish_state(State::Disconnected).await?;
        Ok(())
    }

    pub(crate) async fn clear_stations(&mut self) -> Result<()> {
        self.pandora_stations.clear();
        self.untune().await
    }

    pub(crate) async fn tune(&mut self, station_id: &str) -> Result<()> {
        if !self.connected() {
            return Err(anyhow::anyhow!(Error::invalid_operation_for_state(
                "tune",
                "Disconnected"
            )));
        }

        // Only tune the requested station if we're not already tuned to it
        if self
            .pandora_station
            .as_ref()
            .map(|(id, _)| id != station_id)
            .unwrap_or(true)
        {
            if let Some(name) = self.pandora_stations.get(station_id).map(|s| s.to_string()) {
                info!("Switched station to {name} ({station_id})");
                self.untune().await?;
                self.pandora_station = Some((station_id.to_string(), name.to_string()));
                self.dirty |= true;
                trace!("send notification 'tuned'");
                self.publish_state(State::Tuned(station_id.to_string()))
                    .await?;
                trace!("Updating station in config");
                self.config
                    .borrow_mut()
                    .update_from(&PartialConfig::default().station(Some(station_id.to_string())));
                self.stop(StopReason::Untuning).await?;
            } else {
                return Err(Error::InvalidStation(station_id.to_string()).into());
            }
        } else {
            debug!("Request to tune station that is already tuned");
        }
        Ok(())
    }

    pub(crate) async fn untune(&mut self) -> Result<()> {
        self.pandora_station = None;
        self.dirty |= true;
        self.config
            .borrow_mut()
            .update_from(&PartialConfig::default().station(None));

        self.clear_playlist();
        if self.get_playing().is_some() {
            self.stop(StopReason::Untuning).await?;
        }

        self.publish_state(State::Connected).await?;
        Ok(())
    }

    pub(crate) fn clear_playlist(&mut self) {
        self.pandora_readylist.clear();
        self.pandora_fetchlist.clear();
    }

    pub(crate) fn tuned(&self) -> Option<String> {
        self.pandora_station
            .as_ref()
            .map(|(station_id, _)| station_id.to_string())
    }

    pub(crate) fn playing(&self) -> Option<&Track> {
        self.player_track.as_ref().right()
    }

    pub(crate) fn ready_next_track(&mut self) -> Result<Option<Track>> {
        if self.tuned().is_none() {
            return Err(anyhow::anyhow!(Error::invalid_operation_for_state(
                "ready_next_track",
                "Untuned"
            )));
        }
        if let Some(track) = self.pandora_readylist.pop_front() {
            debug!("playlist yielded new track for playing");
            Ok(Some(track))
        } else {
            Ok(None)
        }
    }

    pub(crate) fn enqueue_track(&mut self, track: &Track) -> Result<()> {
        if self.tuned().is_none() {
            return Err(anyhow::anyhow!(Error::invalid_operation_for_state(
                "enqueue_track",
                "Untuned"
            )));
        }

        if !track.cached() {
            return Err(Error::TrackNotCached(track.title.clone()).into());
        }

        self.pandora_readylist.push_back(track.clone());
        self.unfetch_track(track);
        Ok(())
    }

    pub(crate) fn playlist_len(&self) -> usize {
        self.pandora_readylist.len()
    }

    pub(crate) fn pending_len(&self) -> usize {
        self.pandora_fetchlist.len()
    }

    pub(crate) async fn quit(&mut self) -> Result<()> {
        info!("Application request to quit");
        self.panharmonicon_quitting = true;
        self.dirty |= true;
        trace!("send notification 'quit'");
        self.publish_state(State::Quit).await?;
        Ok(())
    }

    pub(crate) fn quitting(&self) -> bool {
        self.panharmonicon_quitting
    }

    pub(crate) fn get_next(&self) -> Option<&Track> {
        self.pandora_readylist.front()
    }

    pub(crate) fn get_playing(&self) -> Option<&Track> {
        self.player_track.as_ref().right()
    }

    pub(crate) fn get_playing_mut(&mut self) -> Option<&mut Track> {
        self.player_track.as_mut().right()
    }

    async fn notify_playing(&mut self) -> Result<()> {
        if let Some(track) = self.get_playing() {
            self.publish_state(State::TrackStarting(track.clone()))
                .await?;
        }
        Ok(())
    }

    pub(crate) async fn rate_track(&mut self, rating: Option<bool>) -> Result<()> {
        let track = self.get_playing().cloned().ok_or_else(|| {
            anyhow::anyhow!(Error::invalid_operation_for_state("rate_track", "Stopped"))
        })?;
        let session = self.pandora_session.as_mut().ok_or_else(|| {
            anyhow::anyhow!(Error::invalid_operation_for_state(
                "rate_track",
                "Disconnected"
            ))
        })?;
        let new_rating_value: u32 = if rating.unwrap_or(false) { 1 } else { 0 };
        if let Some(rating) = rating {
            session.add_feedback(&track, rating).await?;
            trace!("Rated track {} with value {}", track.title, rating);
        } else {
            session.delete_feedback_for_track(&track).await?;
            trace!("Successfully removed track rating.");
        }
        self.get_playing_mut()
            .expect("Programming error: active track value disappeared mid-function")
            .song_rating = new_rating_value;
        self.dirty |= true;

        // track metadata changed, resend track info message
        self.notify_playing().await?;
        Ok(())
    }

    async fn add_station(&mut self, station_id: String, station_name: String) -> Result<()> {
        if !self.pandora_stations.contains_key(&station_id) {
            self.pandora_stations
                .insert(station_id.clone(), station_name.clone());
            self.dirty |= true;
            trace!("send notification 'add station {station_name}[{station_id}]'");
            self.publish_state(State::AddStation(station_name, station_id))
                .await?;
        } else {
            trace!("not adding station: already exists");
        }
        Ok(())
    }

    pub(crate) async fn fill_station_list(&mut self) -> Result<()> {
        let session = self.pandora_session.as_mut().ok_or_else(|| {
            anyhow::anyhow!(Error::invalid_operation_for_state(
                "fetch_station_list",
                "Disconnected"
            ))
        })?;
        for station in session.get_station_list().await?.stations.into_iter() {
            self.add_station(station.station_id, station.station_name)
                .await?;
        }
        if let Some(station_id) = self.tuned() {
            // Check if we're tuned to a station that doesn't appear in the station list
            if !self.pandora_stations.contains_key(&station_id) {
                warn!("Tuned station {station_id} does not appear in station list");
                self.untune().await?;
            }
        }
        Ok(())
    }

    pub(crate) async fn refill_playlist(&mut self) -> Result<()> {
        if self.pending_len() > FETCHLIST_MAX_LEN {
            debug!("Enough tracks in-flight already - not requesting more tracks for playlist");
            return Ok(());
        }
        if self.playlist_len() > PLAYLIST_MAX_LEN {
            debug!("Enough tracks in playlist already - not requesting more tracks for playlist");
            return Ok(());
        }
        let station_id = self.tuned().ok_or_else(|| {
            anyhow::anyhow!(Error::invalid_operation_for_state(
                "fetch_playlist",
                "Untuned"
            ))
        })?;
        let session = self.pandora_session.as_mut().ok_or_else(|| {
            anyhow::anyhow!(Error::invalid_operation_for_state(
                "fetch_playlist",
                "Disconnected"
            ))
        })?;
        debug!("getting new tracks to refill playlist");
        let playlist: Result<Vec<Track>> = session
            .get_playlist(&station_id)
            .await?
            .into_iter()
            .filter_map(|pe| pe.get_track().map(Track::try_from))
            .collect();
        let playlist = playlist?;
        debug!("refilling playlist with new tracks");
        self.extend_playlist(playlist).await?;
        self.dirty |= true;
        trace!("Successfully refilled playlist.");
        Ok(())
    }

    async fn update_track_progress(&mut self, elapsed: &Duration) -> Result<()> {
        trace!(
            "Update track progress: last update {}s current update {}s",
            self.player_progress.map(|p| p.as_secs()).unwrap_or(0),
            elapsed.as_secs()
        );
        if self.player_progress.map(|p| p.as_secs()) != Some(elapsed.as_secs()) {
            trace!("Track time elapsed updated: {elapsed:?}");
            self.player_progress = Some(*elapsed);
            self.dirty |= true;
            if self.player_paused {
                warn!("Unexpected track progress request while track paused");
                self.publish_state(State::Paused(*elapsed)).await?;
            } else {
                self.publish_state(State::Playing(*elapsed)).await?;
            }
        }
        Ok(())
    }

    async fn handle_request(&mut self, req: &Request) -> Result<()> {
        debug!("Request: {req:?}");
        match req {
            Request::Connect => self.connect().await?,
            Request::Tune(s) => self.tune(s).await?,
            Request::Untune => self.untune().await?,
            Request::FetchFailed(track) => self.unfetch_track(track.as_ref()),
            Request::AddTrack(track) => self.add_track(track.as_ref()).await?,
            Request::Stop(reason) => self.stop(*reason).await?,
            Request::UpdateTrackProgress(elapsed) => self.update_track_progress(elapsed).await?,
            Request::Pause => self.pause().await?,
            Request::Unpause => self.unpause().await?,
            Request::TogglePause => self.toggle_pause().await?,
            Request::Mute => self.mute().await?,
            Request::Unmute => self.unmute().await?,
            Request::Volume(v) => self.set_volume(*v).await?,
            Request::VolumeDown => self.change_volume(-0.1).await?,
            Request::VolumeUp => self.change_volume(0.1).await?,
            Request::RateUp => self.rate_track(Some(true)).await?,
            Request::RateDown => self.rate_track(Some(false)).await?,
            Request::UnRate => self.rate_track(None).await?,
            Request::Quit => self.quit().await?,
        }
        self.dirty |= true;
        Ok(())
    }

    pub(crate) async fn process_messages(&mut self) -> Result<()> {
        while let Ok(req) = self.request_receiver.try_recv() {
            debug!("received request {req:?}");
            self.handle_request(&req).await?;
        }
        // We have a copy of the broadcast state_receiver, and if we don't drain messages from our copy
        // then everyone's copies will get backed up
        while self.state_receiver.try_recv().is_ok() {}

        /*
        // Check the health of the outgoing message channel, as well
        info!(
            "Pending messages in notification channel: {}",
            self.state_sender.len()
        );
        info!(
            "Number of state_receivers for notification channel: {}",
            self.state_receiver.receiver_count()
        );
        */
        Ok(())
    }

    async fn ensure_connection(&mut self) -> Result<()> {
        if !self.connected() {
            self.dirty = true;
            debug!("Connection no longer active.  Reconnecting...");
            self.connect().await?;
        }
        Ok(())
    }

    async fn drive_state(&mut self) -> Result<()> {
        // Disconnected
        //   UI drives credential entry, saving them in config
        // Disconnected -> Connected:
        //   Connect using credentials saved in config
        // Connected
        //   UI drives station selection, saving it in config
        // Connected -> Tuned:
        //   Tune station saved in config
        // Tuned
        //   Start caching tracks
        //   Add cached tracks to playlist
        // Tuned -> Playing:
        //   There is a ready track in playlist
        // Playing
        //   Notify UI of player progress
        //   Check if track is completed
        if !self.connected() {
            if self.config.borrow().login_credentials().get().is_some() {
                self.connect().await?;
            } else {
                self.disconnect().await?;
            }
        } else if self.tuned().is_none() {
            if self.pandora_stations.is_empty() {
                self.fill_station_list().await?;
            }
            let opt_station_id = self.config.borrow().station_id();
            if let Some(station_id) = opt_station_id {
                // We're not tuned to a station, so if one is saved in the config, go ahead and start
                // tuning to that
                info!("Station list is populated, and user config file indicates we should tune to a station");
                self.tune(&station_id).await?;
            }
        } else if self.playing().is_none() {
            self.refill_playlist().await?;
            self.start().await?;
        } else {
            trace!("Happily playing our track");
        }
        Ok(())
    }

    pub(crate) async fn update(&mut self) -> Result<bool> {
        self.process_messages().await?;
        self.ensure_connection().await?;
        self.drive_state().await?;

        let old_dirty = self.dirty;
        self.dirty = false;
        Ok(old_dirty)
    }

    pub(crate) async fn extend_playlist(&mut self, new_playlist: Vec<Track>) -> Result<()> {
        self.dirty |= !new_playlist.is_empty();
        debug!("Extending playlist with {new_playlist:?}");
        for track in new_playlist {
            debug!("Adding track to fetchlist: {}", &track.title);
            self.pandora_fetchlist.push(track.clone());
            self.publish_state(State::TrackCaching(track)).await?;
        }
        Ok(())
    }

    fn unfetch_track(&mut self, track: &Track) {
        // Remove the enqueued track from the fetchlist
        if let Some(idx) = self
            .pandora_fetchlist
            .iter()
            .enumerate()
            .find(|(_, t)| t.track_token == track.track_token)
            .map(|(idx, _)| idx)
        {
            self.pandora_fetchlist.swap_remove(idx);
            self.dirty |= true;
        }
    }

    async fn add_track(&mut self, track: &Track) -> Result<()> {
        let list_was_empty = self.playlist_len() == 0;
        self.enqueue_track(track)?;

        // We didn't have a next-up track, but now we do, so send a notification
        if list_was_empty {
            self.notify_next().await?;
        }

        Ok(())
    }

    async fn notify_next(&mut self) -> Result<()> {
        let next_track = self.get_next().cloned();
        trace!("send notification 'Next({:?})'", next_track);
        self.publish_state(State::Next(next_track)).await?;
        Ok(())
    }

    async fn stop(&mut self, reason: StopReason) -> Result<()> {
        if self.get_playing().is_some() {
            info!("Stopping track: {reason}");
            if self.config.borrow().cache_policy().evict_completed() {
                debug!("Checking for track to evict...");
                if let Some(track) = self.get_playing() {
                    trace!("Eviction policy requires evicting track");
                    track.remove_from_cache();
                }
            } else {
                trace!("Not evicting completed track, per configured cache eviction policy");
            }

            debug!("Currently playing track stopped");
            self.player_paused = false;
            self.player_track = Either::Left(reason);
            self.player_progress = None;
            self.player_length = None;

            self.dirty |= true;
            self.publish_state(State::Stopped(reason)).await?;
        } else {
            debug!("No track is currently playing. Nothing to do.");
        }
        Ok(())
    }

    fn started(&self) -> bool {
        self.player_progress
            .map(|p| p.as_millis() > 0)
            .unwrap_or(false)
    }

    async fn start(&mut self) -> Result<()> {
        if self.started() {
            debug!("Track already started.");
        } else {
            debug!("No tracks started yet. Starting next track.");
            debug!(
                "playlist length: {} + {} pending",
                self.playlist_len(),
                self.pending_len()
            );
            if let Some(track) = self.ready_next_track()? {
                trace!("send notification 'starting'");
                self.player_track = Either::Right(track.clone());
                self.player_progress = Some(Duration::from_secs(0));
                self.player_length = Some(track.track_length);
                self.dirty |= true;

                self.notify_playing().await?;
                self.notify_next().await?;
            } else {
                debug!("requested to start track, but no tracks are ready");
                self.publish_state(State::Buffering)
                    .await?;
            }
        }
        Ok(())
    }

    fn paused(&self) -> bool {
        self.player_paused
    }

    async fn pause(&mut self) -> Result<()> {
        if !self.paused() {
            if let Some(progress) = self.get_playing().and(self.player_progress) {
                self.player_paused = true;
                self.dirty |= true;
                self.publish_state(State::Paused(progress)).await?;
            }
        }
        Ok(())
    }

    async fn unpause(&mut self) -> Result<()> {
        if self.paused() {
            if let Some(progress) = self.get_playing().and(self.player_progress) {
                self.player_paused = false;
                self.dirty |= true;
                self.publish_state(State::Playing(progress)).await?;
            }
        }
        Ok(())
    }

    async fn toggle_pause(&mut self) -> Result<()> {
        if self.paused() {
            self.unpause().await?;
        } else {
            self.pause().await?;
        }
        Ok(())
    }

    fn volume(&self) -> f32 {
        self.player_volume
    }

    async fn set_volume(&mut self, new_volume: f32) -> Result<()> {
        if new_volume != self.player_volume {
            self.player_volume = new_volume;
            self.dirty |= true;
            self.config
                .borrow_mut()
                .update_from(&PartialConfig::default().volume(new_volume));
            trace!("send notification 'volume'");
            self.publish_state(State::Volume(new_volume)).await?;
        }
        Ok(())
    }

    async fn change_volume(&mut self, increment: f32) -> Result<()> {
        let new_volume = self.player_volume + increment;
        self.set_volume(new_volume.clamp(0.0, 1.0)).await
    }

    fn muted(&self) -> bool {
        self.player_muted
    }

    async fn mute(&mut self) -> Result<()> {
        if !self.muted() {
            self.player_muted = true;
            self.dirty |= true;
            self.publish_state(State::Muted).await?;
        }
        Ok(())
    }

    async fn unmute(&mut self) -> Result<()> {
        if self.muted() {
            self.player_muted = false;
            self.dirty |= true;
            self.publish_state(State::Unmuted).await?;
        }
        Ok(())
    }
}

impl Drop for Model {
    fn drop(&mut self) {
        // If there have been any configuration changes, commit them to disk
        trace!("Flushing config file to disk...");
        if let Err(e) = self.config.borrow_mut().flush() {
            error!("Failed commiting configuration changes to file: {:?}", e);
        }
        trace!("Application data model has been dropped");
    }
}
