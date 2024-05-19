use std::time::Instant;

use anyhow::{Context, Result};
use log::{debug, error, trace, warn};
use tokio::task::JoinHandle;

use crate::messages::{Request, State};
use crate::model::{RequestSender, StateReceiver};
use crate::track::Track;

const TASK_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(20);

#[derive(Debug)]
pub(crate) struct FetchRequest {
    track: Track,
    completed: bool,
    failed: bool,
    task_handle: Option<(JoinHandle<Result<Track>>, Instant)>,
    retry_count: u8,
}

impl From<Track> for FetchRequest {
    fn from(track: Track) -> Self {
        let completed = track.cached();
        FetchRequest {
            track,
            completed,
            failed: false,
            task_handle: None,
            retry_count: 0,
        }
    }
}

impl FetchRequest {
    async fn update_state(&mut self) {
        // If transfer thread completed and we haven't checked the result yet:
        if let Some((ref mut th, start_time)) = &mut self.task_handle {
            trace!(
                "task started {}s ago (up to a maximum of {}s)",
                start_time.elapsed().as_secs(),
                TASK_TIMEOUT.as_secs()
            );
            if th.is_finished() {
                match th.await {
                    Err(e) if e.is_cancelled() => {
                        debug!("Track fetch task was cancelled");
                        self.failed = true;
                        self.completed = false;
                    }
                    Err(e) if e.is_panic() => {
                        warn!("Track fetch task panicked");
                        self.failed = true;
                        self.completed = false;
                        // TODO: trigger a retry?
                    }
                    Err(e) => {
                        error!("Unhandled track fetch task error: {e:#}");
                        self.failed = true;
                        self.completed = false;
                        // TODO: trigger a retry?
                    }
                    Ok(Err(e)) => {
                        // TODO: dig into these error codes, figure out which ones are worth retrying
                        // the fetch for
                        if let Some(e) = e.downcast_ref::<reqwest::Error>() {
                            error!("reqwest error {e:#}");
                        }
                        self.failed = true;
                        self.completed = false;
                        error!("Error during in-flight request for track {e:#}");
                    }
                    Ok(Ok(_)) => {
                        self.completed = self.track.cached();
                        self.failed = !self.completed;
                        trace!("In-flight request for track completed: {}", &self.completed);
                    }
                }
                self.task_handle = None;
            } else if start_time.elapsed() > TASK_TIMEOUT {
                warn!(
                    "Track fetch task {} exceeded time limit!  Cancelling...",
                    self.track.cache_path.display()
                );
                th.abort();
                self.failed = true;
                self.completed = false;
                self.track.remove_from_cache();
                self.task_handle = None;
                return;
            } else {
                trace!("Fetch task in progress");
            }
        } else if !self.failed && !self.completed {
            warn!("Unexpected condition: no track request in-flight, and it was neither failed nor completed");
            // TODO: this seems like a retry-able condition
            self.failed = true;
        } else {
            trace!(
                "fetch task {} (completed: {} failed: {}) waiting to be reaped.",
                self.track.cache_path.display(),
                self.completed,
                self.failed
            );
        }
    }

    async fn cancel(&mut self) {
        self.update_state().await;
        if let Some((th, _)) = &self.task_handle {
            debug!(
                "Aborting in-flight request for track being saved to {}",
                &self.track.cache_path.display()
            );
            th.abort();
            self.failed = true;
            self.completed = false;
            self.track.remove_from_cache();
        }
        self.task_handle = None;
    }

    fn finished(&self) -> bool {
        self.task_handle.is_none() && self.completed
    }

    fn failed(&self) -> bool {
        self.task_handle.is_none() && self.failed
    }

    fn retriable(&self) -> bool {
        self.retry_count < 3
    }

    async fn start(&mut self, client: reqwest::Client) {
        if self.task_handle.is_some() {
            warn!("Programming error: restarting an already started fetch task");
            return;
        }
        if self.track.cached() {
            trace!("Cache hit!");
            self.completed = true;
        } else {
            trace!("Cache miss!");
            let track = self.track.clone();
            let th = tokio::spawn(async move {
                //trace!("Retrieving track {}...", &track.title);
                track.download_to_cache(&client).await?;
                Ok(track)
            });
            self.task_handle = Some((th, Instant::now()));
        }
    }

    async fn restart(&mut self, client: reqwest::Client) {
        if self.retriable() {
            self.cancel().await;
            self.failed = false;
            self.retry_count += 1;
            self.start(client).await;
        }
    }
}

#[derive(Debug)]
pub(crate) struct TrackCacher {
    client: reqwest::Client,
    requests: Vec<FetchRequest>,
    station_id: Option<String>,
    request_sender: RequestSender,
    state_receiver: StateReceiver,
    dirty: bool,
}

impl TrackCacher {
    pub(crate) fn new(state_receiver: StateReceiver, request_sender: RequestSender) -> Self {
        TrackCacher {
            client: reqwest::Client::new(),
            requests: Vec::with_capacity(8),
            station_id: None,
            request_sender,
            state_receiver,
            dirty: false,
        }
    }

    fn publish_request(&mut self, request: Request) -> Result<()> {
        self.request_sender
            .send(request)
            .context("Failed sending application update request")?;
        Ok(())
    }

    async fn fetch_track(&mut self, track: Track) -> Result<()> {
        if track.cached() {
            trace!("Track {} in cache, not fetching.", &track.title);
            self.publish_request(Request::AddTrack(Box::new(track)))
                .context(
                "Failed sending application update request for a new track being ready for play",
            )?;
        } else {
            trace!("Track {} not in cache, fetching...", &track.title);
            let mut fetch_request = FetchRequest::from(track);
            fetch_request.start(self.client.clone()).await;
            self.requests.push(fetch_request);
        }
        self.dirty |= true;
        Ok(())
    }

    async fn cancel_requests(&mut self) {
        for mut request in self.requests.drain(..) {
            request.cancel().await;
            self.dirty |= true;
        }
    }

    async fn update_requests(&mut self) -> Result<()> {
        trace!("updating cache requests");
        // Make sure each request's state is current
        for request in self.requests.iter_mut() {
            trace!(
                "Checking state of in-flight request {}...",
                &request.track.title
            );
            request.update_state().await;
        }

        // We have to be a little careful how we do this, because requests aren't clonable
        // and we want to remove the completed requests from the list and send notifications for
        // them
        // We also can't notify while we're iterating, because that requires a mutable borrow of
        // self, which is why we need to build two local lists from the data before moving on
        let mut completed_requests = Vec::new();
        let mut active_requests = Vec::new();
        for mut request in self.requests.drain(..) {
            if request.finished() || (request.failed() && !request.retriable()) {
                trace!("request completed: {}", &request.track.title);
                completed_requests.push(request);
            } else if request.failed() && request.retriable() {
                warn!(
                    "retrying failed fetch request for {} (retries {})",
                    &request.track.title, request.retry_count
                );
                request.restart(self.client.clone()).await;
                active_requests.push(request);
            } else {
                trace!("request pending: {}", &request.track.title);
                active_requests.push(request);
            }
        }
        self.requests = active_requests;

        // Notify of all tracks removed from the fetchlist
        for request in completed_requests.into_iter() {
            let track = request.track.clone();
            if request.finished() {
                self.dirty |= true;
                if !request.failed() && track.cached() {
                    trace!("completed request was successful: {}", &request.track.title);
                    self.publish_request(Request::AddTrack(Box::new(track))).context("Failed sending application update request for a new track being ready for play")?;
                } else {
                    debug!("completed request failed: {}", &request.track.title);
                    self.publish_request(Request::FetchFailed(Box::new(track))).context("Failed sending application update request for a track failing to download correctly")?;
                }
            } else if request.failed() {
                debug!("request failed before completion: {}", &request.track.title);
                self.publish_request(Request::FetchFailed(Box::new(track))).context("Failed sending application update request for a track failing to download correctly")?;
            }
        }

        Ok(())
    }

    async fn process_messages(&mut self) -> Result<()> {
        trace!("processing messages");
        while let Ok(message) = self.state_receiver.try_recv() {
            match message {
                State::Tuned(new_s) => {
                    // if we're changing stations, clear the waitqueue
                    if self
                        .station_id
                        .as_ref()
                        .map(|old_s| old_s != &new_s)
                        .unwrap_or(true)
                    {
                        trace!("Tuned new station - cancelling in-flight track requests");
                        self.cancel_requests().await;
                    }
                    self.station_id = Some(new_s);
                    self.dirty = true;
                }
                State::Connected => {
                    // No longer tuned to a station
                    trace!("Reconnected to pandora - cancelling in-flight track requests");
                    self.cancel_requests().await;
                    self.station_id = None;
                    self.dirty = true;
                }
                State::TrackCaching(t) => {
                    if self
                        .station_id
                        .as_ref()
                        .map(|s| s == &t.station_id)
                        .unwrap_or(false)
                    {
                        trace!("Request to cache a track - adding to in-flight track list");
                        self.fetch_track(t)
                            .await
                            .context("Failed while attempting to fetch a track for playback")?;
                        self.dirty = true;
                    } else {
                        warn!("Request to cache track that's not from the current station (track station: {}, current station: {:?})", &t.station_id, &self.station_id);
                    }
                }
                _ => (),
            }
        }
        Ok(())
    }

    pub(crate) async fn update(&mut self) -> Result<bool> {
        self.process_messages()
            .await
            .context("Failure while processing requests for track fetching")?;
        self.update_requests()
            .await
            .context("Failure while updating state of in-flight track fetch requests")?;
        let dirty = self.dirty;
        self.dirty = false;
        Ok(dirty)
    }
}
