use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use log::trace;
use tokio::sync::RwLock;

use crate::messages::State;
use crate::model::{RequestSender, StateReceiver};
use crate::track::Track;

use mpris_server::{Metadata, Property, Signal, Time};
mod mpris_intf;
use mpris_intf::{MprisInterface, MprisState};

#[derive(Debug)]
pub(crate) struct MprisUi {
    server: mpris_server::Server<MprisInterface>,
    state_receiver: StateReceiver,
    shared_state: Arc<RwLock<MprisState>>,
}

impl MprisUi {
    pub(crate) async fn new(
        state_receiver: StateReceiver,
        request_sender: RequestSender,
    ) -> Result<Self> {
        let shared_state = Arc::new(RwLock::new(MprisState::default()));
        let mpris_intf = MprisInterface::new(shared_state.clone(), request_sender);
        let server = mpris_server::Server::new_with_all(clap::crate_name!(), mpris_intf).await?;

        Ok(Self {
            server,
            state_receiver,
            shared_state,
        })
    }

    async fn update_state_stopped(&mut self) -> Result<()> {
        {
            let mut state = self.shared_state.write().await;
            state.playing = None;
        }
        self.server
            .properties_changed([Property::PlaybackStatus(
                mpris_server::PlaybackStatus::Stopped,
            )])
            .await?;
        Ok(())
    }

    async fn playing_track(&mut self, track: Track) -> Result<()> {
        {
            let mut state = self.shared_state.write().await;
            state.playing = Some((track.clone(), Duration::ZERO, false));
            state.tracklist = vec![track.clone()];
        }
        let metadata = Metadata::builder()
            .length(Time::from_millis(track.track_length.as_millis() as i64))
            .album(track.album_name.clone())
            .artist([track.artist_name.clone()])
            .title(track.title.clone())
            .build();
        self.server
            .properties_changed([
                Property::Metadata(metadata),
                Property::PlaybackStatus(mpris_server::PlaybackStatus::Playing),
            ])
            .await?;
        Ok(())
    }

    async fn update_playing(&mut self, elapsed: Duration, paused: bool) -> Result<()> {
        {
            let mut state = self.shared_state.write().await;
            if let Some((_, ref mut e, ref mut p)) = state.playing {
                *e = elapsed;
                *p = paused;
            }
        }
        let playback_status = if paused {
            mpris_server::PlaybackStatus::Paused
        } else {
            mpris_server::PlaybackStatus::Playing
        };
        self.server
            .properties_changed([Property::PlaybackStatus(playback_status)])
            .await?;
        self.server
            .emit(Signal::Seeked {
                position: Time::from_millis(elapsed.as_millis() as i64),
            })
            .await?;
        Ok(())
    }

    async fn update_volume(&mut self, volume: f32) -> Result<()> {
        {
            let mut state = self.shared_state.write().await;
            state.volume = volume;
        }
        self.server
            .properties_changed([Property::Volume(volume as f64)])
            .await?;
        Ok(())
    }

    async fn process_messages(&mut self) -> Result<()> {
        trace!("checking for player notifications...");
        while let Ok(message) = self.state_receiver.try_recv() {
            match message {
                State::AuthFailed(_) => self.update_state_stopped().await?,
                State::Connected => self.update_state_stopped().await?,
                State::Disconnected => self.update_state_stopped().await?,
                State::AddStation(name, id) => {
                    let mut state = self.shared_state.write().await;
                    state.playlists.insert(id, name);
                }
                State::Tuned(name) => {
                    let mut state = self.shared_state.write().await;
                    state.active_playlist = state
                        .playlists
                        .iter()
                        .find(|(_, v)| *v == &name)
                        .map(|(k, v)| (k.clone(), v.clone()));
                }
                State::TrackStarting(track) => self.playing_track(track).await?,
                State::Next(next_track) => {
                    let mut state = self.shared_state.write().await;
                    if let Some(t) = next_track {
                        if !state
                            .tracklist
                            .iter()
                            .any(|x| x.track_token == t.track_token)
                        {
                            state.tracklist.push(t);
                        }
                    }
                }
                State::Playing(elapsed) => self.update_playing(elapsed, false).await?,
                State::Volume(v) => self.update_volume(v).await?,
                State::Paused(elapsed) => self.update_playing(elapsed, true).await?,
                State::Stopped(_) => self.update_state_stopped().await?,
                State::Buffering => self.update_state_stopped().await?,
                State::StationSeeds(_) => (),
                State::TrackCaching(_) => (),
                State::Muted => (),
                State::Unmuted => (),
                State::Quit => (),
            }
        }
        Ok(())
    }

    pub(crate) async fn update(&mut self) -> Result<bool> {
        self.process_messages().await?;
        Ok(false)
    }
}
