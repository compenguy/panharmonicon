use std::path::{Path, PathBuf};
use std::time::Instant;

use anyhow::{Context, Result};
use clap::crate_name;
use log::{debug, error, trace, warn};
use tokio::io::AsyncWriteExt;
use tokio::task::JoinHandle;

use crate::errors::Error;
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
        let completed = track.exists();
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
                if let Err(e) = th.await {
                    self.failed = true;
                    self.completed = false;
                    error!("Error during in-flight request for track {e:#}");
                } else {
                    self.completed = self.track.exists();
                    self.failed = !self.completed;
                    debug!("In-flight request for track completed: {}", &self.completed);
                }
            } else if !self.failed && !self.completed {
                warn!("Unexpected condition: no track request in-flight, and it was neither failed nor completed");
                self.failed = true;
            }
            self.task_handle = None;
        }
    }

    async fn cancel(&mut self) {
        self.update_state().await;

        if let Some(th) = &self.task_handle {
            debug!(
                "Aborting in-flight request for track being saved to {:?}",
                &self.track.cached
            );
            th.abort();
            self.failed = true;
            self.completed = false;
            if let Some(path) = &self.track.cached {
                if let Err(e) = tokio::fs::remove_file(path).await {
                    warn!("Failed to delete cancelled cache request: {e:#}");
                }
            }
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
        if self.completed {
            debug!("Cache hit!");
            return;
        }
        debug!("Cache miss!");
        if let Some(path) = self.track.cached.clone() {
            let track = self.track.clone();
            let path = path.clone();
            let req_builder = client.get(&track.audio_stream);
            let th = tokio::spawn(async move {
                debug!("Retrieving track {}...", path.display());
                debug!("retrieval start time: {:?}", Instant::now());

                if let Err(e) = save_request_to_file(req_builder, path.with_extension("tmp")).await
                {
                    error!("Failed to fetch requested file: {e:#}");
                    let _ = tokio::fs::remove_file(path.with_extension("tmp")).await;
                    return Err(e);
                }
                debug!("retrieval finish time: {:?}", Instant::now());
                trace!("applying tags to track...");
                if let Err(e) = tag_m4a(&track, &path.with_extension("tmp")) {
                    error!("Failed to tag requested file: {e:#}");
                    let _ = tokio::fs::remove_file(path.with_extension("tmp")).await;
                    return Err(e);
                }
                if let Err(e) = std::fs::rename(path.with_extension("tmp"), &path) {
                    error!("Failed to finalize requested file: {e:#}");
                    let _ = tokio::fs::remove_file(path.with_extension("tmp")).await;
                    return Err(e.into());
                }
                if path.exists() {
                    Ok(track)
                } else {
                    error!("Track fetch, tag, and rename completed successfully, but final track file does not exist.");
                    Err(anyhow::anyhow!("Downloaded track does not exist"))
                }
            });
            self.task_handle = Some(th);
        } else {
            warn!("Not fetching requested track: no cache location supplied for track");
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

    async fn fetch_track(&mut self, mut t: Track) -> Result<()> {
        t.cached = Some(cached_path_for_track(&t, true)?);
        debug!("Fetching track {:?}", &t.cached);
        let mut fetch_request = FetchRequest::from(t);
        fetch_request.start(self.client.clone()).await;
        self.requests.push(fetch_request);
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
                if !request.failed() && track.cached.clone().map(|p| p.exists()).unwrap_or(false) {
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

// https://en.wikipedia.org/wiki/Filename#Reserved_characters_and_words
fn sanitize_filename(text: &str) -> String {
    text.chars()
        .map(|c| match c {
            '/' => '_',
            '\\' => '_',
            '?' => '_',
            '*' => '_',
            ':' => '_',
            '|' => '_',
            '<' => '_',
            '>' => '_',
            _ => c,
        })
        .collect()
}

fn app_cache_dir() -> Result<PathBuf> {
    Ok(dirs::cache_dir()
        .ok_or(Error::AppDirNotFound)?
        .join(crate_name!()))
}

fn precached_path_for_track(track: &Track) -> Option<PathBuf> {
    track
        .cached
        .as_ref()
        .and_then(|p| if p.exists() { Some(p.clone()) } else { None })
}

async fn save_request_to_file<P: AsRef<Path>>(
    req_builder: reqwest::RequestBuilder,
    path: P,
) -> Result<()> {
    let mut resp = req_builder
        .send()
        .await
        .map_err(Error::from)
        .with_context(|| {
            format!(
                "Error completing fetch request to file {}",
                path.as_ref().display()
            )
        })?;
    let mut file = tokio::fs::File::create(path.as_ref())
        .await
        .with_context(|| {
            format!(
                "Failed creating file on disk as {}",
                path.as_ref().to_string_lossy()
            )
        })?;

    while let Some(chunk) = resp.chunk().await? {
        file.write(&chunk).await.with_context(|| {
            format!(
                "Error writing fetched track to file {}",
                path.as_ref().display()
            )
        })?;
    }

    /*
    tokio::io::copy(&mut resp.bytes_stream(), &mut file)
        .await
        .with_context(|| {
            format!(
                "Error writing fetched track to file {}",
                path.as_ref().display()
            )
        })?;
        */
    debug!("Track data streamed to file successfully.");
    Ok(())
}

fn cached_path_for_track(track: &Track, create_path: bool) -> Result<PathBuf> {
    if let Some(precached) = precached_path_for_track(track) {
        return Ok(precached);
    }

    let artist = sanitize_filename(&track.artist_name);
    let album = sanitize_filename(&track.album_name);
    let song = sanitize_filename(&track.song_name);

    let mut track_cache_path = app_cache_dir()?.join(&artist).join(album);

    if create_path {
        std::fs::create_dir_all(&track_cache_path).with_context(|| {
            format!(
                "Failed to create directory for caching track as {}",
                track_cache_path.to_string_lossy()
            )
        })?;
    }
    let filename = format!("{artist} - {song}.{}", "m4a");
    track_cache_path.push(filename);
    Ok(track_cache_path)
}

fn tag_m4a<P: AsRef<Path>>(track: &Track, path: P) -> Result<()> {
    debug!("Reading tags from m4a");
    let mut tag = match mp4ameta::Tag::read_from_path(path.as_ref()) {
        Ok(tag) => tag,
        Err(mp4ameta::Error {
            kind: mp4ameta::ErrorKind::NoTag,
            ..
        }) => mp4ameta::Tag::default(),
        err => err.with_context(|| {
            format!(
                "Failed reading m4a file at {}",
                path.as_ref().to_string_lossy()
            )
        })?,
    };

    debug!("Updating tags with pandora metadata");
    let mut dirty = false;

    if tag.artist().is_none() {
        tag.set_artist(&track.artist_name);
        dirty = true;
    }
    if tag.album().is_none() {
        tag.set_album(&track.album_name);
        dirty = true;
    }
    if tag.title().is_none() {
        tag.set_title(&track.song_name);
        dirty = true;
    }

    debug!("Writing tags back to file");
    if dirty {
        tag.write_to_path(path.as_ref()).with_context(|| {
            format!(
                "Failed while writing updated MP3 tags back to {}",
                path.as_ref().to_string_lossy()
            )
        })?;
    }
    Ok(())
}
