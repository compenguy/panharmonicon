use pandora_api::json::station::PlaylistTrack;

#[derive(Debug, Clone)]
pub(crate) enum Request {
    Connect,
    Tune(String),
    Quit,
    Stop,
    Start,
    SleepTrack,
    RateUp,
    RateDown,
    UnRate,
    Pause,
    Unpause,
    TogglePause,
    Mute,
    Unmute,
    Volume(f32),
    VolumeDown,
    VolumeUp,
}

#[derive(Debug, Clone)]
pub(crate) enum Notification {
    Connected,
    Disconnected,
    AddStation(String, String),
    Tuned(String),
    Starting(PlaylistTrack),
    Rated(u32),
    Unrated,
    Next(PlaylistTrack),
    Volume(f32),
    Muted,
    Unmuted,
    Playing(std::time::Duration, std::time::Duration),
    Paused(std::time::Duration, std::time::Duration),
    Stopped,
    Quit,
}
