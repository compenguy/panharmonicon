use std::path::PathBuf;
use std::time::Duration;

use pandora_api::json::station::PlaylistTrack;

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
        if let Some(cached) = self.path() {
            log::trace!("Track cache location specified: {}", cached.display());
            log::trace!("Track cache file exists: {}", cached.exists());
            true
        } else {
            log::warn!("No track cache location specified!");
            false
        }
    }

    pub(crate) fn path(&self) -> Option<PathBuf> {
        self.cached.clone().filter(|p| p.exists())
    }
}
