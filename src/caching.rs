use std::collections::VecDeque;
use std::convert::TryFrom;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use clap::crate_name;
use log::{error, trace, warn};
use tokio::io::AsyncWriteExt;

use crate::errors::Error;
use crate::messages;
use crate::track::Track;

#[derive(Debug, Clone)]
pub(crate) struct FetchRequest {
    uri: String,
    path: PathBuf,
}

impl TryFrom<&Track> for FetchRequest {
    type Error = anyhow::Error;

    fn try_from(track: &Track) -> Result<Self, Self::Error> {
        Ok(FetchRequest {
            uri: track.audio_stream.clone(),
            path: cached_path_for_track(track, true)?,
        })
    }
}

#[derive(Debug, Clone)]
pub(crate) struct TrackCacher {
    client: reqwest::Client,
    waitqueue: VecDeque<Track>,
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
            waitqueue: VecDeque::new(),
            station_id: None,
            subscriber,
            publisher,
        }
    }

    pub(crate) async fn update(&mut self) -> Result<bool> {
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
                        self.waitqueue.clear();
                    }
                    self.station_id = Some(new_s);
                }
                messages::Notification::Connected => {
                    // No longer tuned to a station
                    self.waitqueue.clear();
                    self.station_id = None;
                }
                messages::Notification::PreCaching(t) => {
                    if self
                        .station_id
                        .as_ref()
                        .map(|s| s == &t.station_id)
                        .unwrap_or(false)
                    {
                        trace!("Adding track to fetcher waitqueue");
                        self.waitqueue.push_back(t);
                        dirty = true;
                    } else {
                        warn!("Request to cache track that's not from the current station");
                    }
                }
                _ => (),
            }
        }

        if let Some(mut track) = self.waitqueue.pop_front() {
            let request = FetchRequest::try_from(&track)?;
            trace!("Fetching a track with audio url {:?}", &request.uri);
            if request.path.exists() {
                trace!("Cache hit!");
            } else {
                trace!("Cache miss!");
                self.save_url_to_file(&request.uri, &request.path).await?;
                trace!("Track caching completed.");
                if let Err(e) = tag_m4a(&track, &request.path) {
                    error!(
                        "Error tagging track at {}: {:?}",
                        &request.path.display(),
                        &e
                    );
                }
            }
            track.cached = Some(request.path);
            self.publisher
                .broadcast(messages::Request::AddTrack(Box::new(track)))
                .await?;
            dirty = true;
        }
        Ok(dirty)
    }

    async fn save_url_to_file<P: AsRef<Path>>(&self, url: &str, path: P) -> Result<()> {
        trace!(
            "Retrieving track from {} to {}...",
            url,
            path.as_ref().to_string_lossy()
        );
        let mut resp = self
            .client
            .get(url)
            .send()
            .await
            .map_err(Error::from)
            .with_context(|| format!("Error fetching url {url}"))?;
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
