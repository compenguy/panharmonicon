use std::collections::{HashMap, VecDeque};
use std::time::Duration;

use anyhow::Result;
use either::Either;
use log::{debug, error, info, trace, warn};
use tokio::sync::mpsc;

use crate::config::{PartialConfig, SharedConfig};
use crate::errors::Error;
use crate::messages::{Request, State, StopReason};
use crate::pandora::{PandoraCommand, PandoraResult};
use crate::track::Track;

pub(crate) type StateSender = async_broadcast::Sender<State>;
pub(crate) type StateReceiver = async_broadcast::Receiver<State>;
pub(crate) type RequestSender = mpsc::Sender<Request>;
pub(crate) type RequestReceiver = mpsc::Receiver<Request>;

/// Bounded request channel capacity; allows try_send() from sync contexts (UI, player).
const REQUEST_CHANNEL_CAP: usize = 256;

const FETCHLIST_MAX_LEN: usize = 8;
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
    session_connected: bool,
    pending_connect: bool,
    pending_station_list: bool,
    pending_playlist: bool,
    pandora_station: Option<(String, String)>,
    pandora_stations: HashMap<String, String>,
    pandora_readylist: VecDeque<Track>,
    pandora_fetchlist: Vec<Track>,
    panharmonicon_quitting: bool,
    request_sender: RequestSender,
    request_receiver: RequestReceiver,
    state_sender: StateSender,
    state_receiver: StateReceiver,
    pandora_cmd_tx: mpsc::Sender<PandoraCommand>,
    pandora_result_rx: mpsc::Receiver<PandoraResult>,
    config: SharedConfig,
    dirty: bool,
}

impl Model {
    pub(crate) fn new(
        config: SharedConfig,
        pandora_cmd_tx: mpsc::Sender<PandoraCommand>,
        pandora_result_rx: mpsc::Receiver<PandoraResult>,
    ) -> Self {
        let (request_sender, request_receiver) = mpsc::channel(REQUEST_CHANNEL_CAP);
        let (state_sender, state_receiver) = async_broadcast::broadcast(64);
        let volume = config.read().expect("config read for volume").volume();
        Self {
            player_volume: volume,
            player_muted: false,
            player_paused: false,
            player_track: Either::Left(StopReason::Initializing),
            player_progress: None,
            player_length: None,
            session_connected: false,
            pending_connect: false,
            pending_station_list: false,
            pending_playlist: false,
            pandora_station: None,
            pandora_stations: HashMap::with_capacity(16),
            pandora_readylist: VecDeque::with_capacity(PLAYLIST_MAX_LEN),
            pandora_fetchlist: Vec::with_capacity(FETCHLIST_MAX_LEN),
            panharmonicon_quitting: false,
            request_sender,
            request_receiver,
            state_sender,
            state_receiver,
            pandora_cmd_tx,
            pandora_result_rx,
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

    async fn publish_state(&mut self, state: State) -> Result<()> {
        debug!("State update: {state:?}");
        self.state_sender.broadcast(state).await?;
        Ok(())
    }

    pub(crate) async fn connect(&mut self) -> Result<()> {
        if self.connected() {
            info!("Connect request ignored. Already connected.");
            return Ok(());
        }
        if self.pending_connect {
            trace!("Connect already in progress.");
            return Ok(());
        }
        trace!("Attempting pandora login...");
        self.dirty |= true;
        self.pending_connect = true;
        let _ = self.pandora_cmd_tx.send(PandoraCommand::Connect).await;
        Ok(())
    }

    pub(crate) fn connected(&self) -> bool {
        self.session_connected
    }

    pub(crate) async fn disconnect(&mut self) -> Result<()> {
        self.session_connected = false;
        self.pending_connect = false;
        let _ = self.pandora_cmd_tx.send(PandoraCommand::Disconnect).await;
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

        if self.tuned().as_deref() != Some(station_id) {
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
                    .write()
                    .expect("config write for station")
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
            .write()
            .expect("config write for untune")
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
        self.pandora_station.as_ref().map(|(id, _)| id.clone())
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
        let _ = self.pandora_cmd_tx.send(PandoraCommand::Quit).await;
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
        if !self.connected() {
            return Err(anyhow::anyhow!(Error::invalid_operation_for_state(
                "rate_track",
                "Disconnected"
            )));
        }
        let _ = self
            .pandora_cmd_tx
            .send(PandoraCommand::RateTrack(track, rating))
            .await;
        // Local state and notify_playing will be updated when we receive PandoraResult::Rated
        self.dirty |= true;
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
        if !self.connected() {
            return Err(anyhow::anyhow!(Error::invalid_operation_for_state(
                "fetch_station_list",
                "Disconnected"
            )));
        }
        if self.pending_station_list {
            return Ok(());
        }
        self.pending_station_list = true;
        let _ = self
            .pandora_cmd_tx
            .send(PandoraCommand::GetStationList)
            .await;
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
        if !self.connected() {
            return Err(anyhow::anyhow!(Error::invalid_operation_for_state(
                "fetch_playlist",
                "Disconnected"
            )));
        }
        if self.pending_playlist {
            return Ok(());
        }
        self.pending_playlist = true;
        debug!("getting new tracks to refill playlist");
        let _ = self
            .pandora_cmd_tx
            .send(PandoraCommand::GetPlaylist(station_id))
            .await;
        Ok(())
    }

    async fn update_track_progress(&mut self, elapsed: &Duration) -> Result<()> {
        let prev_secs = self.player_progress.map(|p| p.as_secs());
        trace!(
            "Update track progress: last update {}s current update {}s",
            prev_secs.unwrap_or(0),
            elapsed.as_secs()
        );
        if prev_secs != Some(elapsed.as_secs()) {
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
        while self.state_receiver.try_recv().is_ok() {}
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
            if self
                .config
                .read()
                .expect("config read for login check")
                .login_credentials()
                .get()
                .is_some()
            {
                self.connect().await?;
            } else {
                self.disconnect().await?;
            }
        } else if self.tuned().is_none() {
            if self.pandora_stations.is_empty() {
                self.fill_station_list().await?;
            }
            // Only tune once we have received the station list (StationList result populates
            // pandora_stations). Otherwise tune() would fail with "not in the station list".
            if !self.pandora_stations.is_empty() {
                let opt_station_id = self
                    .config
                    .read()
                    .expect("config read for station_id")
                    .station_id();
                if let Some(station_id) = opt_station_id {
                    info!("Station list is populated, and user config file indicates we should tune to a station");
                    self.tune(&station_id).await?;
                }
            }
        } else if self.get_playing().is_none() {
            self.refill_playlist().await?;
            self.start().await?;
        } else {
            trace!("Happily playing our track");
        }
        Ok(())
    }

    #[allow(dead_code)] // kept for single-tick use (e.g. tests); normal run uses run_until_quit
    pub(crate) async fn update(&mut self) -> Result<bool> {
        self.process_messages().await?;
        self.ensure_connection().await?;
        self.drive_state().await?;

        let old_dirty = self.dirty;
        self.dirty = false;
        Ok(old_dirty)
    }

    async fn handle_pandora_result(&mut self, result: PandoraResult) -> Result<()> {
        match result {
            PandoraResult::Connected => {
                self.session_connected = true;
                self.pending_connect = false;
                trace!("send notification 'connected'");
                self.publish_state(State::Connected).await?;
                if let Some(station_id) = self.tuned() {
                    trace!("send notification 'tuned'");
                    self.publish_state(State::Tuned(station_id)).await?;
                }
                self.publish_state(State::Volume(self.volume())).await?;
            }
            PandoraResult::AuthFailed(message) => {
                self.session_connected = false;
                self.pending_connect = false;
                error!("{message}");
                self.publish_state(State::AuthFailed(message)).await?;
                self.clear_stations().await?;
            }
            PandoraResult::Disconnected => {
                self.session_connected = false;
                self.pending_connect = false;
            }
            PandoraResult::StationList(list) => {
                self.pending_station_list = false;
                for (station_id, station_name) in list {
                    self.add_station(station_id, station_name).await?;
                }
                if let Some(station_id) = self.tuned() {
                    if !self.pandora_stations.contains_key(&station_id) {
                        warn!("Tuned station {station_id} does not appear in station list");
                        self.untune().await?;
                    }
                }
            }
            PandoraResult::Playlist(tracks) => {
                self.pending_playlist = false;
                debug!("refilling playlist with new tracks");
                self.extend_playlist(tracks).await?;
                self.dirty |= true;
                trace!("Successfully refilled playlist.");
            }
            PandoraResult::Rated(new_value) => {
                if let Some(track) = self.get_playing_mut() {
                    track.song_rating = new_value;
                    self.dirty |= true;
                    self.notify_playing().await?;
                }
            }
            PandoraResult::Error(msg) => {
                error!("Pandora task error: {msg}");
                self.pending_station_list = false;
                self.pending_playlist = false;
            }
            PandoraResult::QuitAck => {}
        }
        Ok(())
    }

    /// Event-driven loop: wake on request, pandora result, or on timer. Runs until quitting.
    pub(crate) async fn run_until_quit(&mut self, naptime: Duration) -> Result<()> {
        while !self.quitting() {
            tokio::select! {
                biased;
                Some(req) = self.request_receiver.recv() => {
                    debug!("received request {req:?}");
                    self.handle_request(&req).await?;
                    self.dirty |= true;
                    while let Ok(req) = self.request_receiver.try_recv() {
                        debug!("received request {req:?}");
                        self.handle_request(&req).await?;
                        self.dirty |= true;
                    }
                    while self.state_receiver.try_recv().is_ok() {}
                }
                Some(result) = self.pandora_result_rx.recv() => {
                    self.handle_pandora_result(result).await?;
                    while let Ok(result) = self.pandora_result_rx.try_recv() {
                        self.handle_pandora_result(result).await?;
                    }
                }
                _ = tokio::time::sleep(naptime) => {
                    self.process_messages().await?;
                    self.ensure_connection().await?;
                    self.drive_state().await?;
                }
            }
        }
        Ok(())
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
        if let Some(idx) = self
            .pandora_fetchlist
            .iter()
            .position(|t| t.track_token == track.track_token)
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
        trace!("send notification 'Next({next_track:?})'");
        self.publish_state(State::Next(next_track)).await?;
        Ok(())
    }

    async fn stop(&mut self, reason: StopReason) -> Result<()> {
        if self.get_playing().is_some() {
            info!("Stopping track: {reason}");
            if self
                .config
                .read()
                .expect("config read for cache_policy")
                .cache_policy()
                .evict_completed()
            {
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
            return Ok(());
        }
        debug!(
            "No tracks started yet. Starting next track. playlist: {} + {} pending",
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
            self.publish_state(State::Buffering).await?;
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
                .write()
                .expect("config write for volume")
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
        if let Err(e) = self.config.write().expect("config write for flush").flush() {
            error!("Failed commiting configuration changes to file: {e:?}");
        }
        trace!("Application data model has been dropped");
    }
}
