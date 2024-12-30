use std::collections::HashMap;
use std::time::Duration;
use std::convert::TryFrom;

use anyhow::Result;
use log::trace;

use crate::messages::{Request, State, StopReason};
use crate::model::{RequestSender, StateReceiver};
use crate::track::Track;

use mpris_server::zbus;
use mpris_server::{LoopStatus, PlaybackRate, PlaylistId, PlaylistOrdering, TrackId, Uri};
use mpris_server::{Metadata, PlaybackStatus, Playlist, Time, Volume};
use mpris_server::{PlayerInterface, PlaylistsInterface, RootInterface, TrackListInterface};

pub(crate) struct MprisInterface {
    state_receiver: StateReceiver,
    request_sender: RequestSender,
    playlists: HashMap<String, String>,
    active_playlist: Option<(String, String)>,
    tracklist: Vec<Track>,
    playing: Option<(Track, Duration, bool)>,
    volume: f32,
}

impl MprisInterface {
    pub(crate) fn new(state_receiver: StateReceiver, request_sender: RequestSender) -> Self {
        Self {
            state_receiver,
            request_sender,
            playlists: HashMap::with_capacity(8),
            active_playlist: None,
            tracklist: Vec::with_capacity(4),
            playing: None,
            volume: 0.0f32,
        }
    }

    fn publish_zrequest(&self, request: Request) -> zbus::Result<()> {
        self.request_sender
            .send(request)
            .map_err(|e| zbus::Error::Failure(e.to_string()))?;
        Ok(())
    }

    fn playing_track(&mut self, track: Track) {
        self.playing = Some((track, std::time::Duration::from_millis(0), false));
    }

    fn update_playing(&mut self, elapsed: Duration, paused: bool) {
        if let Some((_, e, p)) = &mut self.playing {
            *e = elapsed;
            *p = paused;
        }
    }

    fn update_volume(&mut self, volume: f32) {
        self.volume = volume;
    }

    fn add_playlist(&mut self, name: String, id: String) {
        self.playlists.insert(id, name);
    }

    fn start_playlist(&mut self, name: String) {
        let playlist_id = self.playlists.iter().find_map(|(k, v)| {
            if v == name.as_str() {
                Some((k, v))
            } else {
                None
            }
        });
        self.active_playlist = playlist_id.map(|(a, b)| (a.to_owned(), b.to_owned()));
    }

    fn update_tracklist(&mut self, track: Option<Track>) {
        let tracklist: Vec<Track> = self
            .playing
            .as_ref()
            .map(|(t, _, _)| t.clone())
            .iter()
            .chain(track.iter())
            .cloned()
            .collect();
        self.tracklist = tracklist;
    }

    async fn process_messages(&mut self) -> Result<()> {
        trace!("checking for player notifications...");
        while let Ok(message) = self.state_receiver.try_recv() {
            match message {
                State::AuthFailed(_) => (),
                State::Connected => (),
                State::Disconnected => (),
                State::AddStation(name, id) => self.add_playlist(name, id),
                State::Tuned(name) => self.start_playlist(name),
                State::TrackStarting(track) => self.playing_track(track),
                State::Next(track) => self.update_tracklist(track),
                State::Playing(elapsed) => self.update_playing(elapsed, false),
                State::Volume(v) => self.update_volume(v),
                State::Paused(elapsed) => self.update_playing(elapsed, true),
                State::Stopped(_) => (),
                State::Buffering => (),
                State::TrackCaching(_) => (),
                State::Muted => (),
                State::Unmuted => (),
                State::Quit => (),
            }
        }
        Ok(())
    }

    pub(crate) async fn update(&mut self) -> zbus::Result<()> {
        self.process_messages().await.map_err(|e| zbus::Error::Failure(e.to_string()))?;
        Ok(())
    }
}

// mpris-server dbus interfaces
impl RootInterface for MprisInterface {
    async fn raise(&self) -> zbus::fdo::Result<()> {
        Ok(())
    }

    async fn quit(&self) -> zbus::fdo::Result<()> {
        self.publish_zrequest(Request::Quit)?;
        Ok(())
    }

    async fn can_quit(&self) -> zbus::fdo::Result<bool> {
        Ok(true)
    }

    async fn fullscreen(&self) -> zbus::fdo::Result<bool> {
        Ok(false)
    }

    async fn set_fullscreen(&self, _fullscreen: bool) -> zbus::Result<()> {
        Ok(())
    }

    async fn can_set_fullscreen(&self) -> zbus::fdo::Result<bool> {
        Ok(false)
    }

    async fn can_raise(&self) -> zbus::fdo::Result<bool> {
        Ok(false)
    }

    async fn has_track_list(&self) -> zbus::fdo::Result<bool> {
        Ok(true)
    }

    async fn identity(&self) -> zbus::fdo::Result<String> {
        Ok(clap::crate_name!().to_string())
    }

    async fn desktop_entry(&self) -> zbus::fdo::Result<String> {
        Ok(clap::crate_name!().to_string())
    }

    async fn supported_uri_schemes(&self) -> zbus::fdo::Result<Vec<String>> {
        Ok(vec![])
    }

    async fn supported_mime_types(&self) -> zbus::fdo::Result<Vec<String>> {
        Ok(vec![])
    }
}

impl PlayerInterface for MprisInterface {
    async fn next(&self) -> zbus::fdo::Result<()> {
        self.publish_zrequest(Request::Stop(StopReason::UserRequest))?;
        Ok(())
    }

    async fn previous(&self) -> zbus::fdo::Result<()> {
        Ok(())
    }

    async fn pause(&self) -> zbus::fdo::Result<()> {
        self.publish_zrequest(Request::Pause)?;
        Ok(())
    }

    async fn play_pause(&self) -> zbus::fdo::Result<()> {
        self.publish_zrequest(Request::TogglePause)?;
        Ok(())
    }

    async fn stop(&self) -> zbus::fdo::Result<()> {
        Ok(())
    }

    async fn play(&self) -> zbus::fdo::Result<()> {
        self.publish_zrequest(Request::Unpause)?;
        Ok(())
    }

    async fn seek(&self, _offset: Time) -> zbus::fdo::Result<()> {
        Ok(())
    }

    async fn set_position(&self, _track_id: TrackId, _position: Time) -> zbus::fdo::Result<()> {
        Ok(())
    }

    async fn open_uri(&self, _uri: String) -> zbus::fdo::Result<()> {
        Ok(())
    }

    async fn playback_status(&self) -> zbus::fdo::Result<PlaybackStatus> {
        let status = match self.playing {
            Some((_, _, false)) => PlaybackStatus::Playing,
            Some((_, _, true)) => PlaybackStatus::Paused,
            None => PlaybackStatus::Stopped,
        };
        Ok(status)
    }

    async fn loop_status(&self) -> zbus::fdo::Result<LoopStatus> {
        Ok(LoopStatus::None)
    }

    async fn set_loop_status(&self, _loop_status: LoopStatus) -> zbus::Result<()> {
        Ok(())
    }

    async fn rate(&self) -> zbus::fdo::Result<PlaybackRate> {
        Ok(PlaybackRate::default())
    }

    async fn set_rate(&self, _rate: PlaybackRate) -> zbus::Result<()> {
        Ok(())
    }

    async fn shuffle(&self) -> zbus::fdo::Result<bool> {
        Ok(false)
    }

    async fn set_shuffle(&self, _shuffle: bool) -> zbus::Result<()> {
        Ok(())
    }

    async fn metadata(&self) -> zbus::fdo::Result<Metadata> {
        if let Some((t, _, _)) = &self.playing {
            Ok(Metadata::from(t))
        } else {
            Ok(Metadata::default())
        }
    }

    async fn volume(&self) -> zbus::fdo::Result<Volume> {
        Ok(self.volume as f64)
    }

    async fn set_volume(&self, volume: Volume) -> zbus::Result<()> {
        self.publish_zrequest(Request::Volume(volume as f32))?;
        Ok(())
    }

    async fn position(&self) -> zbus::fdo::Result<Time> {
        if let Some((_, pos, _)) = self.playing {
            Ok(Time::from_millis(pos.as_millis() as i64))
        } else {
            Ok(Time::ZERO)
        }
    }

    async fn minimum_rate(&self) -> zbus::fdo::Result<PlaybackRate> {
        Ok(PlaybackRate::default())
    }

    async fn maximum_rate(&self) -> zbus::fdo::Result<PlaybackRate> {
        Ok(PlaybackRate::default())
    }

    async fn can_go_next(&self) -> zbus::fdo::Result<bool> {
        Ok(true)
    }

    async fn can_go_previous(&self) -> zbus::fdo::Result<bool> {
        Ok(false)
    }

    async fn can_play(&self) -> zbus::fdo::Result<bool> {
        Ok(true)
    }

    async fn can_pause(&self) -> zbus::fdo::Result<bool> {
        Ok(true)
    }

    async fn can_seek(&self) -> zbus::fdo::Result<bool> {
        Ok(false)
    }

    async fn can_control(&self) -> zbus::fdo::Result<bool> {
        Ok(true)
    }
}

impl TrackListInterface for MprisInterface {
    async fn get_tracks_metadata(
        &self,
        track_ids: Vec<TrackId>,
    ) -> zbus::fdo::Result<Vec<Metadata>> {
        let mut tracks_metadata = Vec::new();
        for track_id in track_ids {
            if let Some(md) = self.tracklist.iter().find_map(|t| {
                if t.track_token.as_str() == track_id.as_str() {
                    Some(Metadata::from(t))
                } else {
                    None
                }
            }) {
                tracks_metadata.push(md);
            } else {
                tracks_metadata.push(Metadata::default());
            }
        }
        Ok(tracks_metadata)
    }

    async fn add_track(
        &self,
        _uri: Uri,
        _after_track: TrackId,
        _set_as_current: bool,
    ) -> zbus::fdo::Result<()> {
        trace!("AddTrack: unsupported");
        Ok(())
    }

    async fn remove_track(&self, _track_id: TrackId) -> zbus::fdo::Result<()> {
        trace!("RemoveTrack: unsupported");
        Ok(())
    }

    async fn go_to(&self, _track_id: TrackId) -> zbus::fdo::Result<()> {
        trace!("GoTo: unsupported");
        Ok(())
    }

    async fn tracks(&self) -> zbus::fdo::Result<Vec<TrackId>> {
        let tracklist: Vec<TrackId> = self
            .tracklist
            .iter()
            .map(|t| {
                TrackId::try_from(t.track_token.as_str())
                    .expect("Failed to convert track token to TrackId")
            })
            .collect();
        Ok(tracklist)
    }

    async fn can_edit_tracks(&self) -> zbus::fdo::Result<bool> {
        Ok(false)
    }
}

impl PlaylistsInterface for MprisInterface {
    async fn activate_playlist(&self, playlist_id: PlaylistId) -> zbus::fdo::Result<()> {
        self.publish_zrequest(Request::Tune(playlist_id.as_str().to_string()))?;
        Ok(())
    }

    async fn get_playlists(
        &self,
        _index: u32,
        max_count: u32,
        _order: PlaylistOrdering,
        _reverse_order: bool,
    ) -> zbus::fdo::Result<Vec<Playlist>> {
        let mut playlists: Vec<Playlist> = self
            .playlists
            .iter()
            .map(|p| {
                Playlist {
                    id: PlaylistId::try_from(p.0.as_str()) .expect("Failed to convert playlist id to PlaylistId"),
                    name: p.1.to_string(),
                    icon: Uri::new(),
                }
            })
            .collect();
        playlists.truncate(max_count as usize);
        Ok(playlists)
    }

    async fn playlist_count(&self) -> zbus::fdo::Result<u32> {
        Ok(self.playlists.len() as u32)
    }

    async fn orderings(&self) -> zbus::fdo::Result<Vec<PlaylistOrdering>> {
        Ok(vec![PlaylistOrdering::UserDefined])
    }

    async fn active_playlist(&self) -> zbus::fdo::Result<Option<Playlist>> {
        Ok(self.active_playlist.as_ref().map(|(id, name)| Playlist {
            id: PlaylistId::try_from(id.as_str())
                .expect("Failed to convert station id to PlaylistId"),
            name: name.to_string(),
            icon: Uri::new(),
        }))
    }
}
