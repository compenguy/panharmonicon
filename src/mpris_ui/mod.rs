use std::time::Duration;

use anyhow::Result;
use log::trace;

use crate::messages::{Request, State, StopReason};
use crate::model::{RequestSender, StateReceiver};
use crate::track::Track;

use mpris_server::{Metadata, Player, Time};

#[derive(Debug)]
pub(crate) struct MprisPlayer {
    player: Player,
    _local_rt: tokio::task::LocalSet,
    state_receiver: StateReceiver,
}

impl MprisPlayer {
    pub(crate) async fn new(
        state_receiver: StateReceiver,
        request_sender: RequestSender,
    ) -> Result<Self> {
        let player = Player::builder("com.github.compenguy.panharmonicon")
            .identity("panharmonicon")
            .can_quit(true)
            .has_track_list(true)
            .can_go_next(true)
            .can_go_previous(false)
            .can_play(true)
            .can_pause(true)
            .can_control(true)
            .build()
            .await?;

        let _local_rt = tokio::task::LocalSet::new();
        _local_rt.spawn_local(player.run());

        let cb_sender = request_sender.clone();
        player.connect_next(move |_| {
            let _ = cb_sender.send(Request::Stop(StopReason::UserRequest));
        });

        let cb_sender = request_sender.clone();
        player.connect_quit(move |_| {
            let _ = cb_sender.send(Request::Quit);
        });

        let cb_sender = request_sender.clone();
        player.connect_pause(move |_| {
            let _ = cb_sender.send(Request::Pause);
        });

        let cb_sender = request_sender.clone();
        player.connect_play(move |_| {
            let _ = cb_sender.send(Request::Unpause);
        });

        let cb_sender = request_sender.clone();
        player.connect_play_pause(move |_| {
            let _ = cb_sender.send(Request::TogglePause);
        });

        let cb_sender = request_sender.clone();
        player.connect_set_volume(move |_, v| {
            let _ = cb_sender.send(Request::Volume(v as f32));
        });

        Ok(Self {
            player,
            _local_rt,
            state_receiver,
        })
    }

    async fn update_state_stopped(&mut self) -> Result<()> {
        self.player
            .set_playback_status(mpris_server::PlaybackStatus::Stopped)
            .await?;
        Ok(())
    }

    async fn playing_track(&mut self, track: Track) -> Result<()> {
        self.player
            .set_playback_status(mpris_server::PlaybackStatus::Playing)
            .await?;
        let metadata = Metadata::builder()
            .length(Time::from_millis(track.track_length.as_millis() as i64))
            .album(track.album_name.clone())
            .artist([track.artist_name.clone()])
            .title(track.title.clone())
            .build();
        self.player.set_metadata(metadata).await?;
        Ok(())
    }

    async fn update_playing(&mut self, elapsed: Duration, paused: bool) -> Result<()> {
        self.player
            .seeked(Time::from_millis(elapsed.as_millis() as i64))
            .await?;
        if paused {
            self.player
                .set_playback_status(mpris_server::PlaybackStatus::Paused)
                .await?;
        } else {
            self.player
                .set_playback_status(mpris_server::PlaybackStatus::Playing)
                .await?;
        }
        Ok(())
    }

    async fn update_volume(&mut self, volume: f32) -> Result<()> {
        self.player.set_volume(volume as f64).await?;
        Ok(())
    }

    async fn process_messages(&mut self) -> Result<()> {
        trace!("checking for player notifications...");
        while let Ok(message) = self.state_receiver.try_recv() {
            match message {
                State::AuthFailed(_) => self.update_state_stopped().await?,
                State::Connected => self.update_state_stopped().await?,
                State::Disconnected => self.update_state_stopped().await?,
                State::AddStation(_, _) => (),
                State::Tuned(_) => (),
                State::TrackStarting(track) => self.playing_track(track).await?,
                State::Next(_) => (),
                State::Playing(elapsed) => self.update_playing(elapsed, false).await?,
                State::Volume(v) => self.update_volume(v).await?,
                State::Paused(elapsed) => self.update_playing(elapsed, true).await?,
                State::Stopped(_) => self.update_state_stopped().await?,
                State::Buffering => self.update_state_stopped().await?,
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
