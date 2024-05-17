use std::time::Duration;

use anyhow::Result;
use log::{debug, trace};
use mpris_server::{Metadata, PlaybackStatus, Player, Time};

use crate::messages::{Request, State, StopReason};
use crate::model::{RequestSender, StateReceiver};
use crate::track::Track;

#[derive(Debug)]
pub(crate) struct MprisServer {
    inner: Player,
    state_receiver: StateReceiver,
    request_sender: RequestSender,
    dirty: bool,
}

impl MprisServer {
    pub(crate) async fn new(
        state_receiver: StateReceiver,
        request_sender: RequestSender,
    ) -> anyhow::Result<Self> {
        let inner = Player::builder(clap::crate_name!())
            .can_quit(true)
            .can_set_fullscreen(false)
            .can_raise(false)
            .has_track_list(true)
            .identity(clap::crate_name!())
            .desktop_entry(clap::crate_name!())
            .can_go_next(true)
            .can_go_previous(false)
            .can_play(true)
            .can_pause(true)
            .can_seek(false)
            .can_control(true)
            .build()
            .await?;
        let mut mpris_server = MprisServer {
            inner,
            state_receiver,
            request_sender,
            dirty: false,
        };
        mpris_server.init()?;
        mpris_server.inner.run().await;
        Ok(mpris_server)
    }

    fn init(&mut self) -> Result<()> {
        let request_sender = self.request_sender.clone();
        self.inner.connect_quit(move |_| {
            let _ = request_sender.send(Request::Quit);
        });
        let request_sender = self.request_sender.clone();
        self.inner.connect_next(move |_| {
            let _ = request_sender.send(Request::Stop(StopReason::UserRequest));
        });
        let request_sender = self.request_sender.clone();
        self.inner.connect_pause(move |_| {
            let _ = request_sender.send(Request::Pause);
        });
        let request_sender = self.request_sender.clone();
        self.inner.connect_play_pause(move |_| {
            let _ = request_sender.send(Request::TogglePause);
        });
        let request_sender = self.request_sender.clone();
        self.inner.connect_play(move |_| {
            let _ = request_sender.send(Request::Unpause);
        });
        let request_sender = self.request_sender.clone();
        self.inner.connect_set_volume(move |_, v| {
            let _ = request_sender.send(Request::Volume(v.clamp(0.0, 1.0) as f32));
        });
        Ok(())
    }

    async fn playing_track(&mut self, track: Track) -> Result<()> {
        self.inner.set_metadata(Metadata::from(track)).await?;
        self.dirty |= true;
        Ok(())
    }

    async fn update_playing(&mut self, elapsed: Duration, paused: bool) -> Result<()> {
        self.inner
            .seeked(Time::from_millis(elapsed.as_millis() as i64))
            .await?;
        let status = if paused {
            PlaybackStatus::Paused
        } else {
            PlaybackStatus::Playing
        };
        self.inner.set_playback_status(status).await?;
        self.dirty |= true;
        Ok(())
    }

    async fn update_state_stopped(&mut self) -> Result<()> {
        self.inner
            .set_playback_status(PlaybackStatus::Stopped)
            .await?;
        self.dirty |= true;
        Ok(())
    }

    async fn update_volume(&mut self, volume: f32) -> Result<()> {
        self.inner.set_volume(volume as f64).await?;
        debug!("Updating player about volume change...");
        self.dirty |= true;
        Ok(())
    }

    async fn process_messages(&mut self) -> Result<()> {
        trace!("checking for player notifications...");
        while let Ok(message) = self.state_receiver.try_recv() {
            match message {
                State::Connected => self.update_state_stopped().await?,
                State::TrackStarting(track) => self.playing_track(track).await?,
                State::Playing(elapsed) => self.update_playing(elapsed, false).await?,
                State::Volume(v) => self.update_volume(v).await?,
                State::Paused(elapsed) => self.update_playing(elapsed, true).await?,
                State::Stopped(_) => self.update_state_stopped().await?,
                _ => (),
            }
        }
        Ok(())
    }

    pub(crate) async fn update(&mut self) -> Result<bool> {
        self.process_messages().await?;
        let dirty = self.dirty;
        self.dirty = false;
        Ok(dirty)
    }
}
