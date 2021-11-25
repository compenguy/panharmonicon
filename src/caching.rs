use std::collections::HashMap;
use std::collections::VecDeque;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use async_broadcast::{Receiver, Sender};
use clap::crate_name;
use log::{debug, error, trace, warn};
use pandora_api::json::station::PlaylistTrack;

use crate::errors::Error;

pub(crate) trait Cacheable {
    type Error;
    fn get_path(&self) -> Option<std::path::PathBuf>;
    fn set_path<P: AsRef<std::path::Path>>(&mut self, path: P);
    fn to_cache_request(&self) -> std::result::Result<FetchRequest, Self::Error>;
}

impl Cacheable for PlaylistTrack {
    type Error = anyhow::Error;

    fn get_path(&self) -> Option<std::path::PathBuf> {
        if let Some(serde_json::value::Value::String(path_str)) = self.optional.get("cached") {
            let path = PathBuf::from(path_str);
            if path.exists() {
                return Some(path);
            } else {
                trace!(
                    "Marked as cached, but doesn't exist: {}",
                    path.to_string_lossy()
                );
            }
        }
        None
    }

    fn set_path<P: AsRef<std::path::Path>>(&mut self, path: P) {
        self.optional.insert(
            String::from("cached"),
            serde_json::value::Value::String(path.as_ref().display().to_string()),
        );
    }

    fn to_cache_request(&self) -> std::result::Result<FetchRequest, anyhow::Error> {
        let path = cached_path_for_track(self, true)?.display().to_string();
        let uri = self.audio_url_map.high_quality.audio_url.clone();
        Ok(FetchRequest {
            track_token: self.track_token.clone(),
            uri,
            path,
        })
    }
}

#[derive(Debug, Clone)]
pub(crate) struct FetchRequest {
    track_token: String,
    uri: String,
    path: String,
}

#[derive(Debug, Clone)]
pub(crate) struct FetchResponse {
    track_token: String,
    path: String,
    result: std::result::Result<(), String>,
}

#[derive(Debug, Clone)]
pub(crate) struct TrackCacher {
    waiting: VecDeque<PlaylistTrack>,
    in_work: HashMap<String, PlaylistTrack>,
    ready: Vec<PlaylistTrack>,
    send_to_fetcher: Sender<FetchRequest>,
    recv_from_fetcher: Receiver<FetchResponse>,
}

impl TrackCacher {
    pub(crate) fn new() -> Self {
        let (send_to_fetcher, fetcher_recv) = async_broadcast::broadcast(8);
        // Processing messages about completed fetches are cheap, though, so keep a longer list
        let (fetcher_send, recv_from_fetcher) = async_broadcast::broadcast(16);
        std::thread::spawn(move || TrackCacher::run_thread(fetcher_recv, fetcher_send));
        TrackCacher {
            waiting: VecDeque::new(),
            in_work: HashMap::new(),
            ready: Vec::new(),
            send_to_fetcher,
            recv_from_fetcher,
        }
    }

    // Enqueue tracks for caching
    pub(crate) fn enqueue(&mut self, mut playlist: Vec<PlaylistTrack>) {
        trace!(
            "Adding {} new tracks to caching fetch queue.",
            playlist.len()
        );
        self.waiting.extend(playlist.drain(..));
        trace!("Fetch queue length: {}", self.waiting.len());
    }

    // If there are no tracks currently being fetched, but there are tracks
    // waiting to be fetched, send another track to the fetcher
    fn fetch_waiting(&mut self) -> Result<()> {
        debug!("self.waiting.len() = {}", self.waiting.len());
        debug!(
            "self.send_to_fetcher.is_full() = {}",
            self.send_to_fetcher.is_full()
        );
        while !self.waiting.is_empty() && !self.send_to_fetcher.is_full() {
            trace!("Track fetcher is ready");
            if let Some(mut track) = self.waiting.pop_front() {
                let request = track.to_cache_request()?;
                trace!(
                    "Sending a track for fetching with audio url {:?}",
                    &request.uri
                );
                // If the track is already being fetched from an earlier request,
                // we quietly drop this request
                if self.in_work.contains_key(&request.track_token) {
                    trace!("Cache collision!");
                    continue;
                }
                // If the track already exists in the cache, we just move it straight
                // into the ready queue
                if PathBuf::from(&request.path).exists() {
                    trace!("Cache hit!");
                    track.set_path(request.path);
                    self.ready.push(track);
                    continue;
                }
                trace!("Cache miss!");
                trace!("send request {:?}", request);
                self.send_to_fetcher.try_broadcast(request)?;
                self.in_work.insert(track.track_token.clone(), track);
                trace!("Track is being fetched");
            }
        }
        Ok(())
    }

    fn make_ready(&mut self) {
        while let Ok(response) = self.recv_from_fetcher.try_recv() {
            trace!("received response {:?}", &response);
            let (track_token, path) = match response {
                FetchResponse {
                    track_token,
                    path,
                    result: Ok(()),
                } => (track_token, path),
                FetchResponse {
                    track_token,
                    path: _,
                    result: Err(e),
                } => {
                    error!(
                        "Dropping track {}.  Failed attempting to cache it: {}",
                        track_token, e
                    );
                    continue;
                }
            };
            // caching completed, add to ready queue
            if let Some(mut track) = self.in_work.remove(&track_token) {
                trace!("Track caching completed.");
                if let Err(e) = tag_m4a(&track, &path) {
                    error!("Error tagging track at {}: {:?}", path, &e);
                }
                track.set_path(path);
                self.ready.push(track);
            } else {
                // This can happen if clear() was called on the track cacher after
                // the track was sent for fetching, before it was fetched
                warn!("Cached track not in the in_work map, not adding to ready queue.");
            }
        }
    }

    pub(crate) fn get_ready(&mut self) -> Vec<PlaylistTrack> {
        self.make_ready();
        self.ready.drain(..).collect()
    }

    pub(crate) fn pending_count(&self) -> usize {
        self.in_work.len() + self.ready.len()
    }

    pub(crate) fn update(&mut self) -> Result<usize> {
        self.fetch_waiting()?;
        self.make_ready();
        trace!(
            "Fetcher waiting/in-work/ready queue lengths: {}/{}/{}",
            self.waiting.len(),
            self.in_work.len(),
            self.ready.len()
        );
        Ok(self.pending_count())
    }

    pub(crate) fn clear(&mut self) {
        self.in_work.clear();
        self.ready.clear();
    }

    fn run_thread(mut recv: Receiver<FetchRequest>, send: Sender<FetchResponse>) {
        while let Ok(msg) = recv.try_recv() {
            let result = save_url_to_file(&msg.uri, &msg.path).map_err(|e| e.to_string());
            let _todo = send.try_broadcast(FetchResponse {
                track_token: msg.track_token,
                path: msg.path,
                result,
            });
        }
    }
}

impl Default for TrackCacher {
    fn default() -> Self {
        Self::new()
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

fn precached_path_for_track(track: &PlaylistTrack) -> Option<PathBuf> {
    if let Some(path) = track.get_path() {
        if path.exists() {
            return Some(path);
        } else {
            trace!(
                "Marked as precached, but doesn't exist: {}",
                path.to_string_lossy()
            );
        }
    }
    None
}

fn cached_path_for_track(track: &PlaylistTrack, create_path: bool) -> Result<PathBuf> {
    if let Some(precached) = precached_path_for_track(track) {
        return Ok(precached);
    }

    let artist = sanitize_filename(&track.artist_name);
    let album = sanitize_filename(&track.album_name);
    let song = sanitize_filename(&track.song_name);

    let mut track_cache_path = app_cache_dir()?.join(&artist).join(&album);

    if create_path {
        std::fs::create_dir_all(&track_cache_path).with_context(|| {
            format!(
                "Failed to create directory for caching track as {}",
                track_cache_path.to_string_lossy()
            )
        })?;
    }
    let filename = format!("{} - {}.{}", artist, song, "m4a");
    track_cache_path.push(filename);
    Ok(track_cache_path)
}

fn tag_m4a<P: AsRef<Path>>(track: &PlaylistTrack, path: P) -> Result<()> {
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

fn save_url_to_file<P: AsRef<Path>>(url: &str, path: P) -> Result<()> {
    trace!(
        "Retrieving track from {} to {}...",
        url,
        path.as_ref().to_string_lossy()
    );
    let mut resp = reqwest::blocking::get(url)
        .with_context(|| format!("Failed while retrieving content from url {}", url))?
        .error_for_status()
        .with_context(|| format!("Error response while retrieving content from url {}", url))?;
    trace!("Got response");

    let file = std::fs::File::create(path.as_ref()).with_context(|| {
        format!(
            "Failed creating file on disk as {}",
            path.as_ref().to_string_lossy()
        )
    })?;

    trace!("Streaming response data to file....");

    resp.copy_to(&mut std::io::BufWriter::new(file))
        .with_context(|| {
            format!(
                "Failed writing content from url {} as file {}",
                url,
                path.as_ref().to_string_lossy()
            )
        })?;
    trace!("Track data streamed to file successfully.");
    Ok(())
}
