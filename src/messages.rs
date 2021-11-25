use pandora_api::json::station::PlaylistTrack;

#[derive(Debug, Clone)]
pub(crate) enum Request {
    Connect,
    Tune(String),
    Untune,
    Quit,
    Stop,
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

impl PartialEq<Request> for Request {
    fn eq(&self, other: &Request) -> bool {
        match (self, other) {
            (Request::Connect, Request::Connect) => true,
            (Request::Untune, Request::Untune) => true,
            (Request::Tune(a), Request::Tune(b)) => a == b,
            (Request::Quit, Request::Quit) => true,
            (Request::Stop, Request::Stop) => true,
            (Request::SleepTrack, Request::SleepTrack) => true,
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
            _ => false,
        }
    }
}

impl Eq for Request {}

#[derive(Debug, Clone)]
pub(crate) enum Notification {
    Connected,
    Disconnected,
    AddStation(String, String),
    Tuned(String),
    Starting(PlaylistTrack),
    Next(PlaylistTrack),
    Rated(u32),
    Unrated,
    Volume(f32),
    Muted,
    Unmuted,
    Playing(std::time::Duration, std::time::Duration),
    Paused(std::time::Duration, std::time::Duration),
    Stopped,
    Quit,
}

impl PartialEq<Notification> for Notification {
    fn eq(&self, other: &Notification) -> bool {
        match (self, other) {
            (Notification::Connected, Notification::Connected) => true,
            (Notification::Disconnected, Notification::Disconnected) => true,
            (Notification::AddStation(a, x), Notification::AddStation(b, y)) => a == b && x == y,
            (Notification::Tuned(a), Notification::Tuned(b)) => a == b,
            (Notification::Starting(t), Notification::Starting(u)) => {
                t.track_token == u.track_token
            }
            (Notification::Next(t), Notification::Next(u)) => t.track_token == u.track_token,
            (Notification::Rated(a), Notification::Rated(b)) => a == b,
            (Notification::Unrated, Notification::Unrated) => true,
            (Notification::Volume(a), Notification::Volume(b)) => {
                (a * 100.0) as u8 == (b * 100.0) as u8
            }
            (Notification::Muted, Notification::Muted) => true,
            (Notification::Unmuted, Notification::Unmuted) => true,
            (Notification::Playing(a, x), Notification::Playing(b, y)) => a == x && b == y,
            (Notification::Paused(a, x), Notification::Paused(b, y)) => a == x && b == y,
            (Notification::Stopped, Notification::Stopped) => true,
            (Notification::Quit, Notification::Quit) => true,
            _ => false,
        }
    }
}

impl Eq for Notification {}
