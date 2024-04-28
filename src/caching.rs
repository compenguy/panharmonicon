use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use clap::crate_name;
use log::{debug, error, trace, warn};
use tokio::io::AsyncWriteExt;
use tokio::task::JoinHandle;

use crate::errors::Error;
use crate::messages;
use crate::track::Track;

#[derive(Debug)]
pub(crate) struct FetchRequest {
    track: Track,
    completed: bool,
    failed: bool,
    task_handle: Option<JoinHandle<Result<()>>>,
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
            trace!("Cache hit!");
            return;
        }
        trace!("Cache miss!");
        if let Some(path) = self.track.cached.clone() {
            let track = self.track.clone();
            let path = path.clone();
            let req_builder = client.get(&track.audio_stream);
            let th = tokio::spawn(async move {
                trace!("Retrieving track {}...", path.display());
                save_request_to_file(req_builder, &path).await?;
                tag_m4a(&track, &path)
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
    subscriber: async_broadcast::Receiver<messages::Notification>,
    publisher: async_broadcast::Sender<messages::Request>,
}

impl TrackCacher {
    pub(crate) fn new(
        subscriber: async_broadcast::Receiver<messages::Notification>,
        publisher: async_broadcast::Sender<messages::Request>,
    ) -> Self {
        TrackCacher {
            client: reqwest::Client::new(),
            requests: Vec::with_capacity(8),
            station_id: None,
            subscriber,
            publisher,
        }
    }

    async fn fetch_track(&mut self, mut t: Track) -> Result<()> {
        t.cached = Some(cached_path_for_track(&t, true)?);
        trace!("Fetching track {:?}", &t.cached);
        let mut fetch_request = FetchRequest::from(t);
        fetch_request.start(self.client.clone()).await;
        self.requests.push(fetch_request);
        Ok(())
    }

    async fn cancel_requests(&mut self) {
        for mut request in self.requests.drain(..) {
            request.cancel().await;
        }
    }

    async fn update_requests(&mut self) -> Result<bool> {
        let mut dirty = false;
        // Notify of all successfully fetched tracks
        for request in self.requests.iter_mut() {
            request.update_state().await;
            if request.finished() {
                self.publisher
                    .broadcast(messages::Request::AddTrack(Box::new(request.track.clone())))
                    .await?;
                dirty = true;
            }
        }

        // Remove all completed requests (successfully or not)
        self.requests.retain(|r| !r.finished() && !r.failed());

        self.publisher
            .broadcast(messages::Request::FetchPending(self.requests.len()))
            .await?;

        Ok(dirty)
    }

    async fn process_messages(&mut self) -> Result<bool> {
        let mut dirty = false;
        while let Ok(message) = self.subscriber.try_recv() {
            match message {
                messages::Notification::Tuned(new_s) => {
                    // if we're changing stations, clear the waitqueue
                    if self
                        .station_id
                        .as_ref()
                        .map(|old_s| old_s != &new_s)
                        .unwrap_or(true)
                    {
                        self.cancel_requests().await;
                    }
                    self.station_id = Some(new_s);
                    dirty = true;
                }
                messages::Notification::Connected => {
                    // No longer tuned to a station
                    self.cancel_requests().await;
                    self.station_id = None;
                    dirty = true;
                }
                messages::Notification::PreCaching(t) => {
                    if self
                        .station_id
                        .as_ref()
                        .map(|s| s == &t.station_id)
                        .unwrap_or(false)
                    {
                        self.fetch_track(t).await?;
                        dirty = true;
                    } else {
                        warn!("Request to cache track that's not from the current station");
                    }
                }
                _ => (),
            }
        }
        Ok(dirty)
    }

    pub(crate) async fn update(&mut self) -> Result<bool> {
        let mut dirty = self.process_messages().await?;
        dirty |= self.update_requests().await?;
        trace!("Sending FetchPending({})...", self.requests.len());
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
    trace!("Track data streamed to file successfully.");
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
    trace!("Reading tags from m4a");
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

    trace!("Updating tags with pandora metadata");
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

    trace!("Writing tags back to file");
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
