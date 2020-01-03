use std::cell::RefCell;
use std::rc::Rc;
use std::sync::mpsc;

use log::debug;

use crate::config::Config;
use crate::errors::{Error, Result};
use crate::player;
use crate::ui;

pub(crate) enum AppState {
    Reset,
    Connected,
    Play,
    Pause,
}

pub(crate) struct App {
    last_activity: std::time::Instant,
    state: AppState,
    ui: ui::Session,
    config: Rc<RefCell<Config>>,
    thread_handle: std::thread::JoinHandle<()>,
    recv_channel: mpsc::Receiver<player::FromPandora>,
    send_channel: mpsc::Sender<player::ToPandora>,
}

impl App {
    pub(crate) fn new(config: Rc<RefCell<Config>>, ui: ui::Session) -> Self {
        let (send_to_pandora, recv_from_ui) = mpsc::channel();
        let (send_to_ui, recv_from_pandora) = mpsc::channel();

        let builder = std::thread::Builder::new();

        let handle = builder
            .name("player_player".to_string())
            .spawn(move || {
                player::run(send_to_ui, recv_from_ui, None);
            })
            .expect("Failed to spawn audio player thread");

        Self {
            last_activity: std::time::Instant::now(),
            state: AppState::Reset,
            ui,
            config,
            thread_handle: handle,
            recv_channel: recv_from_pandora,
            send_channel: send_to_pandora,
        }
    }

    fn update_watchdog(&mut self) {
        self.last_activity = std::time::Instant::now();
    }

    pub(crate) fn try_run(&mut self) -> Result<()> {
        loop {
            match self.recv_channel.try_recv() {
                Ok(msg) => self.process_msg(msg)?,
                Err(mpsc::TryRecvError::Empty) => break,
                Err(e) => Err(e)?,
            };
            self.update_watchdog();
        }
        Ok(())
    }

    pub(crate) fn run(&mut self) -> Result<()> {
        // Run through all pending messages
        self.try_run()?;
        // Then block on next message
        // TODO: before blocking on message, check to see if watchdog expired
        // else we could be waiting a *very* long time
        self.process_msg(self.recv_channel.recv().map_err(Error::from)?)?;
        self.update_watchdog();
        Ok(())
    }

    fn process_msg(&mut self, msg: player::FromPandora) -> Result<()> {
        debug!("UI update message: {:?}", msg);
        match msg {
            player::FromPandora::Reset => self.msg_reset()?,
            player::FromPandora::AuthAccepted => self.msg_auth_accepted()?,
            player::FromPandora::AuthFailed => self.msg_auth_failed()?,
            player::FromPandora::StationList(sl) => self.msg_station_list(&sl)?,
            player::FromPandora::StationInfo(si) => self.msg_station_info(&si)?,
            player::FromPandora::SongList(sl) => self.msg_song_list(&sl)?,
            player::FromPandora::SongInfo(si) => self.msg_song_info(&si)?,
            player::FromPandora::SongProgress(p) => self.msg_song_progress(p)?,
            player::FromPandora::Error(e) => self.msg_error(&e)?,
        }
        Ok(())
    }

    fn msg_error(&mut self, msg: &str) -> Result<()> {
        self.ui.display_error(msg);
        Ok(())
    }

    fn msg_reset(&mut self) -> Result<()> {
        self.state = AppState::Reset;
        self.ui.login(ui::SessionAuth::UseSaved);
        todo!();
        Ok(())
    }

    fn msg_auth_accepted(&mut self) -> Result<()> {
        self.state = AppState::Connected;
        // Ensure that login credentials are commited to config
        self.config.borrow_mut().flush()?;
        todo!();
        Ok(())
    }

    fn msg_auth_failed(&mut self) -> Result<()> {
        self.state = AppState::Reset;
        self.ui.login(ui::SessionAuth::ForceReauth);
        todo!();
        Ok(())
    }

    fn msg_station_list(&mut self, stations: &[player::StationInfo]) -> Result<()> {
        self.ui.display_station_list(stations);
        Ok(())
    }

    fn msg_station_info(&mut self, station: &player::StationInfo) -> Result<()> {
        self.ui.display_station_info(station);
        Ok(())
    }

    fn msg_song_list(&mut self, songs: &[player::SongInfo]) -> Result<()> {
        self.ui.display_song_list(songs);
        Ok(())
    }

    fn msg_song_info(&mut self, song: &player::SongInfo) -> Result<()> {
        self.ui.display_song_info(song);
        Ok(())
    }

    fn msg_song_progress(&mut self, progress: u8) -> Result<()> {
        self.ui.update_song_progress(progress);
        Ok(())
    }
}
