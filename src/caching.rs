use anyhow::Result;
use log::{debug, error, trace, warn};
use tokio::task::JoinHandle;

use crate::messages::{Request, State};
use crate::model::{RequestSender, StateReceiver};
use crate::track::Track;

#[derive(Debug)]
pub(crate) struct FetchRequest {
    track: Track,
    completed: bool,
    failed: bool,
    task_handle: Option<JoinHandle<Result<Track>>>,
}

impl From<Track> for FetchRequest {
    fn from(track: Track) -> Self {
        let completed = track.cached();
        FetchRequest {
            track,
            completed,
            failed: false,
            task_handle: None,
        }
    }
}

impl FetchRequest {
    async fn update_state(&mut self) {
        // If transfer thread completed and we haven't checked the result yet:
        if self
            .task_handle
            .as_ref()
            .map(|th| th.is_finished())
            .unwrap_or(false)
        {
            if let Some(ref mut th) = &mut self.task_handle {
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
                        debug!("In-flight request for track completed: {}", &self.completed);
                    }
                }
            } else if !self.failed && !self.completed {
                warn!("Unexpected condition: no track request in-flight, and it was neither failed nor completed");
                // TODO: this seems like a retry-able condition
                self.failed = true;
            }
            self.task_handle = None;
        }
    }

    async fn cancel(&mut self) {
        self.update_state().await;

        if let Some(th) = &self.task_handle {
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

    async fn start(&mut self, client: reqwest::Client) {
        if self.task_handle.is_some() {
            warn!("Programming error: restarting an already started fetch task");
            return;
        }
        if self.track.cached() {
            debug!("Cache hit!");
        } else {
            debug!("Cache miss!");
            let track = self.track.clone();
            let th = tokio::spawn(async move {
                debug!("Retrieving track {}...", &track.title);
                track.download_to_cache(&client).await?;
                Ok(track)
            });
            self.task_handle = Some(th);
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
        self.request_sender.send(request)?;
        Ok(())
    }

    async fn fetch_track(&mut self, track: Track) -> Result<()> {
        if track.cached() {
            debug!("Track {} in cache, not fetching.", &track.title);
            self.publish_request(Request::AddTrack(Box::new(track)))?;
        } else {
            debug!("Track {} not in cache, fetching...", &track.title);
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
        // Make sure each request's state is current
        for request in self.requests.iter_mut() {
            debug!("Checking state of in-flight request {request:?}...");
            request.update_state().await;
        }

        // We have to be a little careful how we do this, because requests aren't clonable
        // and we want to remove the completed requests from the list and send notifications for
        // them
        // We also can't notify while we're iterating, because that requires a mutable borrow of
        // self, which is why we need to build two local lists from the data before moving on
        let mut completed_requests = Vec::new();
        let mut active_requests = Vec::new();
        for request in self.requests.drain(..) {
            if request.finished() || request.failed() {
                completed_requests.push(request);
            } else {
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
                    self.publish_request(Request::AddTrack(Box::new(track)))?;
                } else {
                    self.publish_request(Request::FetchFailed(Box::new(track)))?;
                }
            } else if request.failed() {
                self.publish_request(Request::FetchFailed(Box::new(track)))?;
            }
        }

        Ok(())
    }

    async fn process_messages(&mut self) -> Result<()> {
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
                        self.fetch_track(t).await?;
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
        self.process_messages().await?;
        self.update_requests().await?;
        let dirty = self.dirty;
        self.dirty = false;
        Ok(dirty)
    }
}
