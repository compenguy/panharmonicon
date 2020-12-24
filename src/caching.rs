use std::collections::HashMap;
use std::collections::VecDeque;
use std::convert::TryFrom;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use clap::crate_name;
use crossbeam_channel::{Receiver, Sender};
use log::{error, trace, warn};
use pandora_api::json::station::PlaylistTrack;

use crate::errors::Error;

#[derive(Debug, Clone)]
struct FetchRequest {
    track_token: String,
    uri: String,
    path: String,
}

impl TryFrom<&PlaylistTrack> for FetchRequest {
    type Error = anyhow::Error;

    fn try_from(track: &PlaylistTrack) -> Result<Self, Self::Error> {
        let path = cached_path_for_track(track, true)?;
        Ok(Self {
            track_token: track.track_token.clone(),
            uri: track.additional_audio_url.clone(),
            path: path.to_string_lossy().to_string(),
        })
    }
}

#[derive(Debug)]
struct FetchResponse {
    track_token: String,
    path: String,
    result: Result<()>,
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
        // Bounded channel, so that we don't build up a backlog of tracks to fetch
        // in case we get clear()ed.
        let (send_to_fetcher, fetcher_recv) = crossbeam_channel::bounded(1);
        // Processing messages about completed fetches are cheap, though, so that can be
        // unbounded
        let (fetcher_send, recv_from_fetcher) = crossbeam_channel::unbounded();
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
        while !self.waiting.is_empty() && !self.send_to_fetcher.is_full() {
            trace!("Track fetcher is ready");
            if let Some(mut track) = self.waiting.pop_front() {
                trace!("Sending a track for fetching");
                let request = FetchRequest::try_from(&track)?;
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
                    track.optional.insert(
                        String::from("cached"),
                        serde_json::value::Value::String(request.path),
                    );
                    self.ready.push(track);
                    continue;
                }
                trace!("Cache miss!");
                self.send_to_fetcher.send(request)?;
                self.in_work.insert(track.track_token.clone(), track);
                trace!("Track is being fetched");
            }
        }
        Ok(())
    }

    fn make_ready(&mut self) {
        for response in self.recv_from_fetcher.try_iter() {
            // Could switch to filter_map()ing out bad results, but for now we
            // want traceability on them
            if let Err(e) = response.result {
                error!(
                    "Dropping track {}.  Failed attempting to cache it: {}",
                    response.track_token, e
                );
                continue;
            }
            // caching completed, add to ready queue
            if let Some(mut track) = self.in_work.remove(&response.track_token) {
                if let Err(e) = tag_mp3(&track, &response.path) {
                    error!("Error tagging track at {}: {:?}", response.path, &e);
                    let _ = std::fs::remove_file(&response.path);
                } else {
                    track.optional.insert(
                        String::from("cached"),
                        serde_json::value::Value::String(response.path),
                    );
                    self.ready.push(track);
                }
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

    fn run_thread(recv: Receiver<FetchRequest>, send: Sender<FetchResponse>) {
        trace!("[fetcher thread] Starting track fetcher thread...");
        for msg in recv.iter() {
            trace!(
                "[fetcher thread] Got request to fetch a track: {}",
                &msg.path
            );
            let result = save_url_to_file(&msg.uri, &msg.path);
            trace!("[fetcher thread] Track fetched: {}", &msg.path);
            let _todo = send.send(FetchResponse {
                track_token: msg.track_token,
                path: msg.path,
                result,
            });
            trace!("[fetcher thread] Sleeping until next track fetch request...");
        }
        trace!("[fetcher thread] Track fetcher thread terminating...");
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
    if let Some(serde_json::value::Value::String(path_str)) = track.optional.get("cached") {
        let path = PathBuf::from(path_str);
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
    let filename = format!("{} - {}.{}", artist, song, "mp3");
    track_cache_path.push(filename);
    Ok(track_cache_path)
}

fn tag_mp3<P: AsRef<Path>>(track: &PlaylistTrack, path: P) -> Result<()> {
    let id3_ver = id3::Version::Id3v23;
    trace!("Reading tags from mp3");
    let mut tag = match id3::Tag::read_from_path(path.as_ref()) {
        Ok(tag) => tag,
        Err(id3::Error {
            kind: id3::ErrorKind::NoTag,
            ..
        }) => id3::Tag::new(),
        err => err.with_context(|| {
            format!(
                "Failed reading mp3 file at {}",
                path.as_ref().to_string_lossy()
            )
        })?,
    };

    let duration: Option<u32> = track
        .optional
        .get("trackLength")
        .and_then(|v| v.as_u64())
        .map(|n| n as u32);

    // TODO: if track.replaygain parses correctly, create replaygain
    // frame for the:
    //   * RVA2 tag (if using v2.4)
    //   * XRVA tag (http://id3.org/Experimental%20RVA2)
    //   * http://id3.org/id3v2.4.0-frames section 4.11

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
    if tag.duration().is_none() {
        if let Some(duration) = duration {
            tag.set_duration(duration);
            dirty = true;
        }
    }

    trace!("Writing tags back to file");
    if dirty {
        tag.write_to_path(path.as_ref(), id3_ver).with_context(|| {
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
