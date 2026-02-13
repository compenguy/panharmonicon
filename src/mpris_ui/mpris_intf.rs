use std::collections::HashMap;
use std::convert::TryFrom;
use std::sync::Arc;
use std::time::Duration;

use log::trace;
use tokio::sync::RwLock;

use crate::messages::{Request, StopReason};
use crate::model::RequestSender;
use crate::track::Track;

use mpris_server::zbus;
use mpris_server::{LoopStatus, PlaybackRate, PlaylistId, PlaylistOrdering, TrackId, Uri};
use mpris_server::{Metadata, PlaybackStatus, Playlist, Time, Volume};
use mpris_server::{PlayerInterface, PlaylistsInterface, RootInterface, TrackListInterface};

/// Shared state between MprisUi (writer) and MprisInterface (reader) so D-Bus
/// property reads see the same playback state as emitted signals.
#[derive(Debug, Default)]
pub(crate) struct MprisState {
    pub(crate) playlists: HashMap<String, String>,
    pub(crate) active_playlist: Option<(String, String)>,
    pub(crate) tracklist: Vec<Track>,
    pub(crate) playing: Option<(Track, Duration, bool)>,
    pub(crate) volume: f32,
}

pub(crate) struct MprisInterface {
    state: Arc<RwLock<MprisState>>,
    request_sender: RequestSender,
}

impl MprisInterface {
    pub(crate) fn new(state: Arc<RwLock<MprisState>>, request_sender: RequestSender) -> Self {
        Self {
            state,
            request_sender,
        }
    }

    fn publish_zrequest(&self, request: Request) -> zbus::Result<()> {
        self.request_sender
            .send(request)
            .map_err(|e| zbus::Error::Failure(e.to_string()))?;
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
        self.publish_zrequest(Request::Stop(StopReason::UserRequest))?;
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
        let guard = self.state.read().await;
        let status = match &guard.playing {
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
        let guard = self.state.read().await;
        if let Some((t, _, _)) = &guard.playing {
            Ok(Metadata::from(t))
        } else {
            Ok(Metadata::default())
        }
    }

    async fn volume(&self) -> zbus::fdo::Result<Volume> {
        let guard = self.state.read().await;
        Ok(guard.volume as f64)
    }

    async fn set_volume(&self, volume: Volume) -> zbus::Result<()> {
        self.publish_zrequest(Request::Volume(volume as f32))?;
        Ok(())
    }

    async fn position(&self) -> zbus::fdo::Result<Time> {
        let guard = self.state.read().await;
        if let Some((_, pos, _)) = &guard.playing {
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
        let guard = self.state.read().await;
        let mut tracks_metadata = Vec::with_capacity(track_ids.len());
        for track_id in track_ids {
            let md = guard
                .tracklist
                .iter()
                .find(|t| t.track_token.as_str() == track_id.as_str())
                .map(Metadata::from)
                .unwrap_or_default();
            tracks_metadata.push(md);
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
        let guard = self.state.read().await;
        let tracklist: Vec<TrackId> = guard
            .tracklist
            .iter()
            .filter_map(|t| TrackId::try_from(t.track_token.as_str()).ok())
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
        let guard = self.state.read().await;
        let playlists: Vec<Playlist> = guard
            .playlists
            .iter()
            .filter_map(|(id, name)| {
                Some(Playlist {
                    id: PlaylistId::try_from(id.as_str()).ok()?,
                    name: name.clone(),
                    icon: Uri::new(),
                })
            })
            .take(max_count as usize)
            .collect();
        Ok(playlists)
    }

    async fn playlist_count(&self) -> zbus::fdo::Result<u32> {
        let guard = self.state.read().await;
        Ok(guard.playlists.len() as u32)
    }

    async fn orderings(&self) -> zbus::fdo::Result<Vec<PlaylistOrdering>> {
        Ok(vec![PlaylistOrdering::UserDefined])
    }

    async fn active_playlist(&self) -> zbus::fdo::Result<Option<Playlist>> {
        let guard = self.state.read().await;
        Ok(guard.active_playlist.as_ref().and_then(|(id, name)| {
            Some(Playlist {
                id: PlaylistId::try_from(id.as_str()).ok()?,
                name: name.clone(),
                icon: Uri::new(),
            })
        }))
    }
}
