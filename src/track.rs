use std::fs::File;
use std::io::BufReader;
use std::path::PathBuf;
use std::time::Duration;

use anyhow::{Context, Result};
use log::trace;
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
    pub song_name: String,
    /// The rating of the song for this track.
    pub song_rating: u32,
    /// The track length, if provided
    pub track_length: Duration,
    /// If the song is cached locally, the path for it
    pub cached: Option<std::path::PathBuf>,
}

impl From<PlaylistTrack> for Track {
    fn from(playlist_track: PlaylistTrack) -> Self {
        Track {
            track_token: playlist_track.track_token,
            music_id: playlist_track.music_id,
            station_id: playlist_track.station_id,
            audio_stream: playlist_track.audio_url_map.high_quality.audio_url,
            artist_name: playlist_track.artist_name,
            album_name: playlist_track.album_name,
            song_name: playlist_track.song_name,
            song_rating: playlist_track.song_rating,
            track_length: playlist_track
                .optional
                .get("trackLength")
                .and_then(|v| v.as_u64())
                .map(Duration::from_secs)
                .unwrap_or_default(),
            cached: None,
        }
    }
}

impl From<&PlaylistTrack> for Track {
    fn from(playlist_track: &PlaylistTrack) -> Self {
        Track {
            track_token: playlist_track.track_token.clone(),
            music_id: playlist_track.music_id.clone(),
            station_id: playlist_track.station_id.clone(),
            audio_stream: playlist_track.audio_url_map.high_quality.audio_url.clone(),
            artist_name: playlist_track.artist_name.clone(),
            album_name: playlist_track.album_name.clone(),
            song_name: playlist_track.song_name.clone(),
            song_rating: playlist_track.song_rating,
            track_length: playlist_track
                .optional
                .get("trackLength")
                .and_then(|v| v.as_u64())
                .map(Duration::from_secs)
                .unwrap_or_default(),
            cached: None,
        }
    }
}

impl Track {
    pub(crate) fn exists(&self) -> bool {
        self.cached.as_ref().map(|p| p.exists()).unwrap_or(false)
    }

    pub(crate) fn valid_path(&self) -> Option<PathBuf> {
        self.cached.clone().filter(|p| p.exists())
    }

    pub(crate) fn cache(&mut self, create_path: bool) -> Result<PathBuf> {
        if let Some(track_cache_path) = &self.cached {
            Ok(track_cache_path.clone())
        } else {
            let artist = sanitize_filename(&self.artist_name);
            let album = sanitize_filename(&self.album_name);
            let song = sanitize_filename(&self.song_name);

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
            self.cached = Some(track_cache_path.clone());
            Ok(track_cache_path)
        }
    }

    pub(crate) fn get_m4a_decoder(&self) -> Result<redlux::Decoder<BufReader<File>>> {
        let path = self
            .valid_path()
            .ok_or_else(|| Error::TrackNotCached(self.track_token.clone()))?;

        trace!(
            "Creating decoder for track at {} for playback",
            path.display()
        );
        let file = File::open(&path)
            .with_context(|| format!("Failed opening media file at {}", path.display()))?;
        let metadata = file.metadata().with_context(|| {
            format!(
                "Failed retrieving metadata for media file at {}",
                path.display()
            )
        })?;
        let reader = BufReader::new(file);
        redlux::Decoder::new_mpeg4(reader, metadata.len())
            .context("Failed initializing media decoder")
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
        .join(clap::crate_name!()))
}
