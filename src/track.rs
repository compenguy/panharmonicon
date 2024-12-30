use futures::StreamExt;
use std::fs::File;
use std::io::BufReader;
use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{Context, Result};
use log::{debug, error, info, trace};
use pandora_api::json::station::PlaylistTrack;

use crate::errors::Error;

#[derive(Debug, Clone)]
pub(crate) struct Track {
    /// The unique id (token) for the track to be played.
    pub track_token: String,
    /// The music id (token) used with GetTrack to request additional track
    /// information.
    pub music_id: String,
    /// The unique id (token) for the station from which this track was
    /// requested.
    pub station_id: String,
    /// The url to stream the audio from
    pub audio_stream: String,
    /// The name of the artist for this track.
    pub artist_name: String,
    /// The name of the album for this track.
    pub album_name: String,
    /// The name of the song for this track.
    pub title: String,
    /// The rating of the song for this track.
    pub song_rating: u32,
    /// The track length, if provided
    pub track_length: Duration,
    /// The path where the song would be cached, if already fetched
    pub cache_path: std::path::PathBuf,
}

impl std::convert::TryFrom<PlaylistTrack> for Track {
    type Error = anyhow::Error;

    fn try_from(pl_track: PlaylistTrack) -> std::result::Result<Self, Self::Error> {
        let cache_path = cache_file_path(
            &pl_track.song_name,
            &pl_track.artist_name,
            &pl_track.album_name,
        )
        .context("Failed to calculate a path to store a playlist track at")?;
        let track = Track {
            track_token: pl_track.track_token,
            music_id: pl_track.music_id,
            station_id: pl_track.station_id,
            audio_stream: pl_track.audio_url_map.high_quality.audio_url,
            artist_name: pl_track.artist_name,
            album_name: pl_track.album_name,
            title: pl_track.song_name,
            song_rating: pl_track.song_rating,
            track_length: pl_track
                .optional
                .get("trackLength")
                .and_then(|v| v.as_u64())
                .map(Duration::from_secs)
                .unwrap_or_default(),
            cache_path,
        };
        Ok(track)
    }
}

impl std::convert::TryFrom<&PlaylistTrack> for Track {
    type Error = anyhow::Error;

    fn try_from(pl_track: &PlaylistTrack) -> std::result::Result<Self, Self::Error> {
        Self::try_from(pl_track.clone())
    }
}

#[cfg(feature = "mpris_server")]
use std::convert::TryFrom;
#[cfg(feature = "mpris_server")]
impl std::convert::From<&Track> for mpris_server::Metadata {
    fn from(track: &Track) -> mpris_server::Metadata {
        mpris_server::Metadata::builder()
            .length(mpris_server::Time::from_millis(
                track.track_length.as_millis() as i64,
            ))
            .trackid(mpris_server::TrackId::try_from(track.track_token.as_str()).expect("Failed to convert track token to TrackId"))
            .album(track.album_name.clone())
            .artist([track.artist_name.clone()])
            .title(track.title.clone())
            .build()
    }
}

impl Track {
    pub(crate) fn cached(&self) -> bool {
        // Ensure that the track in the cache is playable, it will be deleted if it isn't
        self.cache_path.exists() && self.get_m4a_decoder().is_ok()
    }

    pub(crate) fn get_m4a_decoder(&self) -> Result<redlux::Decoder<BufReader<File>>> {
        if !self.cache_path.exists() {
            return Err(Error::TrackNotCached(self.title.clone()).into());
        }

        match get_m4a_decoder(&self.cache_path) {
            Err(e) => {
                error!(
                    "Failed reading media file at {}: {e:#}",
                    self.cache_path.display()
                );
                self.remove_from_cache();
                Err(e)
            }
            Ok(decoder) => Ok(decoder),
        }
    }

    pub(crate) async fn download_to_cache(&self, client: &reqwest::Client) -> Result<()> {
        if self.cached() {
            info!("Ignoring request to download track - valid local copy exists in cache");
            return Ok(());
        }

        let req_builder = client.get(&self.audio_stream);

        if let Err(e) = download_to_cache(req_builder, &self.cache_path).await {
            // TODO: fill in the actual error type for Connection reset by peer
            /*
            if let Some(e) = e.source().and_then(|e| e.downcast_ref::<reqwest::Error>()) {
                error!("reqwest error {e:?}");
            }
            */
            error!("Failed to download requested file: {}", e.source().unwrap());
            self.remove_from_cache();
            Err(e)
        } else {
            self.tag_cached_file()
                .context("Failed to apply metadata tags to playlist track")?;
            // Let's make sure the track is playable before we report success adding it to the
            // cache
            self.get_m4a_decoder()
                .context("Failed while validating format of playlist track after downloading")?;
            Ok(())
        }
    }

    fn tag_cached_file(&self) -> Result<()> {
        if !self.cache_path.exists() {
            return Err(Error::TrackNotCached(self.title.clone()).into());
        }

        if let Err(e) = tag_cached_file(
            &self.cache_path,
            &self.title,
            &self.artist_name,
            &self.album_name,
        ) {
            error!("Failed to download requested file: {e:#}");
            self.remove_from_cache();
            Err(e)
        } else {
            Ok(())
        }
    }

    pub(crate) fn remove_from_cache(&self) {
        let _ = std::fs::remove_file(&self.cache_path);
    }
}

fn get_m4a_decoder<P: AsRef<Path>>(path: P) -> Result<redlux::Decoder<BufReader<File>>> {
    let path = path.as_ref();
    trace!(
        "Creating decoder for track at {} for playback",
        path.display()
    );
    let file = File::open(path)
        .with_context(|| format!("Failed opening media file at {}", path.display()))?;
    let metadata = file.metadata().with_context(|| {
        format!(
            "Failed retrieving metadata for media file at {}",
            path.display()
        )
    })?;
    let reader = BufReader::new(file);
    redlux::Decoder::new_mpeg4(reader, metadata.len()).context("Failed initializing media decoder")
}

async fn download_to_cache<P: AsRef<std::path::Path>>(
    req_builder: reqwest::RequestBuilder,
    path: P,
) -> Result<()> {
    let path = path.as_ref();
    if let Some(parent_dir) = path.parent() {
        std::fs::create_dir_all(parent_dir)
            .context("Failed to create directory for caching playlist track")?;
    }

    let resp = req_builder
        .send()
        .await
        .map_err(Error::from)
        .with_context(|| format!("Error completing fetch request to file {}", path.display()))?;
    let file = tokio::fs::File::create(&path)
        .await
        .with_context(|| format!("Failed creating file on disk as {}", path.display()))?;
    let mut file = tokio::io::BufWriter::new(file);

    /*
    while let Some(chunk) = resp.chunk().await? {
        file.write(&chunk)
            .await
            .with_context(|| format!("Error writing fetched track to file {}", path.display()))?;
    }
    */

    let mut bytes_stream = resp.bytes_stream();
    while let Some(chunk) = bytes_stream.next().await {
        tokio::io::copy(&mut chunk?.as_ref(), &mut file)
            .await
            .with_context(|| format!("Error writing fetched track to file {}", path.display()))?;
    }

    debug!("Track data streamed to file successfully.");
    Ok(())
}

fn tag_cached_file<P: AsRef<Path>>(path: P, title: &str, artist: &str, album: &str) -> Result<()> {
    let path = path.as_ref();
    debug!("Reading tags from m4a");
    let mut tag = match mp4ameta::Tag::read_from_path(path) {
        Ok(tag) => tag,
        Err(e) if matches!(e.kind, mp4ameta::ErrorKind::AtomNotFound(_)) => {
            mp4ameta::Tag::default()
        }
        Err(e) => {
            Err(e).with_context(|| format!("Failed reading m4a file at {}", path.display()))?
        }
    };

    debug!("Updating tags with pandora metadata");
    let mut dirty = false;

    if tag.artist().is_none() {
        tag.set_artist(artist);
        dirty = true;
    }

    if tag.album().is_none() {
        tag.set_album(album);
        dirty = true;
    }

    if tag.title().is_none() {
        tag.set_title(title);
        dirty = true;
    }

    if dirty {
        debug!("Writing tags back to file");
        tag.write_to_path(path).with_context(|| {
            format!(
                "Failed while writing updated M4A tags back to {}",
                path.display()
            )
        })?;
    }
    Ok(())
}

fn cache_file_path(title: &str, artist: &str, album: &str) -> Result<PathBuf> {
    let artist = sanitize_filename(artist);
    let title = sanitize_filename(title);
    let album = sanitize_filename(album);

    let mut path = app_cache_dir()
        .context("Failed to determine the correct application cache directory for this platform")?
        .join(&artist)
        .join(album);

    let filename = format!("{artist} - {title}.{}", "m4a");
    path.push(filename);
    Ok(path)
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
        .join(clap::crate_name!()))
}
