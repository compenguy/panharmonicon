use crate::track::Track;

#[derive(Debug, Clone)]
pub(crate) enum Request {
    Connect,
    Tune(String),
    #[allow(dead_code)]
    Untune,
    FetchFailed(Box<Track>),
    AddTrack(Box<Track>),
    Stop(StopReason),
    UpdateTrackProgress(std::time::Duration),
    RateUp,
    RateDown,
    UnRate,
    Pause,
    Unpause,
    TogglePause,
    #[allow(dead_code)]
    Mute,
    #[allow(dead_code)]
    Unmute,
    Volume(f32),
    VolumeDown,
    VolumeUp,
    /// Add the currently playing track as a station seed (song).
    AddTrackSeed,
    /// Add the currently playing artist as a station seed.
    AddArtistSeed,
    /// Remove a seed by id (from station seeds; UI looks up id for current track/artist).
    RemoveSeed(String),
    Quit,
}

impl PartialEq<Request> for Request {
    fn eq(&self, other: &Request) -> bool {
        match (self, other) {
            (Request::Connect, Request::Connect) => true,
            (Request::Untune, Request::Untune) => true,
            (Request::Tune(a), Request::Tune(b)) => a == b,
            (Request::Quit, Request::Quit) => true,
            (Request::Stop(a), Request::Stop(b)) => a == b,
            (Request::RateUp, Request::RateUp) => true,
            (Request::RateDown, Request::RateDown) => true,
            (Request::UnRate, Request::UnRate) => true,
            (Request::Pause, Request::Pause) => true,
            (Request::Unpause, Request::Unpause) => true,
            (Request::TogglePause, Request::TogglePause) => true,
            (Request::Mute, Request::Mute) => true,
            (Request::Unmute, Request::Unmute) => true,
            (Request::Volume(a), Request::Volume(b)) => (a * 100.0) as u8 == (b * 100.0) as u8,
            (Request::VolumeUp, Request::VolumeUp) => true,
            (Request::VolumeDown, Request::VolumeDown) => true,
            (Request::AddTrackSeed, Request::AddTrackSeed) => true,
            (Request::AddArtistSeed, Request::AddArtistSeed) => true,
            (Request::RemoveSeed(a), Request::RemoveSeed(b)) => a == b,
            _ => false,
        }
    }
}

impl Eq for Request {}

#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) enum StopReason {
    Initializing,
    Untuning,
    TrackInterrupted,
    TrackCompleted,
    UserRequest,
}

impl std::fmt::Display for StopReason {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        match self {
            StopReason::Initializing => write!(f, "Starting..."),
            StopReason::Untuning => write!(f, "Closing Station"),
            StopReason::TrackInterrupted => write!(f, "Track Interrupted"),
            StopReason::TrackCompleted => write!(f, "Track Completed"),
            StopReason::UserRequest => write!(f, "User Request"),
        }
    }
}

/// Minimal station seed info for UI to show "is seed" indicator and to remove seeds by id.
#[derive(Debug, Clone)]
pub(crate) struct StationSeedsForUi {
    pub station_id: String,
    /// For "is seed" display and add-track: music_tokens of song seeds.
    pub song_music_tokens: Vec<String>,
    /// For "is seed" display and add-artist: artist names of artist seeds.
    pub artist_names: Vec<String>,
    /// (music_token, seed_id) for removing a song seed.
    pub song_seeds: Vec<(String, String)>,
    /// (artist_name, seed_id) for removing an artist seed.
    pub artist_seeds: Vec<(String, String)>,
}

#[derive(Debug, Clone)]
pub(crate) enum State {
    AuthFailed(String),
    Connected,
    Disconnected,
    AddStation(String, String),
    Tuned(String),
    /// Seeds for a station (song music_tokens and artist names) so UI can show seed indicator.
    StationSeeds(StationSeedsForUi),
    Buffering,
    TrackCaching(Track),
    TrackStarting(Track),
    #[allow(dead_code)]
    Next(Option<Track>),
    Volume(f32),
    Muted,
    Unmuted,
    Playing(std::time::Duration),
    Paused(std::time::Duration),
    Stopped(StopReason),
    Quit,
}

impl PartialEq<State> for State {
    fn eq(&self, other: &State) -> bool {
        match (self, other) {
            (State::Connected, State::Connected) => true,
            (State::Disconnected, State::Disconnected) => true,
            (State::AddStation(a, x), State::AddStation(b, y)) => a == b && x == y,
            (State::Tuned(a), State::Tuned(b)) => a == b,
            (State::StationSeeds(a), State::StationSeeds(b)) => a.station_id == b.station_id,
            (State::TrackStarting(t), State::TrackStarting(u)) => t.track_token == u.track_token,
            (State::Next(Some(t)), State::Next(Some(u))) => t.track_token == u.track_token,
            (State::Next(None), State::Next(None)) => true,
            (State::Volume(a), State::Volume(b)) => (a * 100.0) as u8 == (b * 100.0) as u8,
            (State::Muted, State::Muted) => true,
            (State::Unmuted, State::Unmuted) => true,
            (State::Playing(a), State::Playing(b)) => a == b,
            (State::Paused(a), State::Paused(b)) => a == b,
            (State::Stopped(_), State::Stopped(_)) => true,
            (State::Quit, State::Quit) => true,
            _ => false,
        }
    }
}

impl Eq for State {}
