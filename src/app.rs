use std::cell::RefCell;
use std::rc::Rc;
use std::sync::mpsc;

use log::{debug, trace};

use crate::config::Config;
use crate::errors::{Error, Result};
use crate::player;
use crate::ui;

#[derive(Debug, Clone, PartialEq)]
pub(crate) enum AppState {
    Reset,
    Connected,
    Play,
    Pause,
    Quit,
}

#[derive(Debug)]
pub(crate) struct App {
    last_activity: std::time::Instant,
    state: AppState,
    ui: ui::Session,
    config: Rc<RefCell<Config>>,
    // TODO: make this an Option<JoinHandle> so that reset can respawn the thread?
    thread_handle: std::thread::JoinHandle<()>,
    recv_channel: mpsc::Receiver<player::FromPandora>,
    send_channel: mpsc::Sender<player::ToPandora>,
}

impl App {
    pub(crate) fn new(config: Rc<RefCell<Config>>, ui: ui::Session) -> Self {
        let (send_to_pandora, recv_from_ui) = mpsc::channel();
        let (send_to_ui, recv_from_pandora) = mpsc::channel();

        let builder = std::thread::Builder::new();

        trace!("Spawning audio playback thread");
        let handle = builder
            .name("panharmonicon_player".to_string())
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
            trace!("Processed message from player thread");
            self.update_watchdog();
        }
        Ok(())
    }

    pub(crate) fn run(&mut self) -> Result<()> {
        // Run through all pending messages
        while self.state != AppState::Quit {
            trace!("Spinning on messages from player thread");
            self.try_run()?;
            std::thread::sleep(std::time::Duration::from_millis(100));
            // TODO: check to see if watchdog expired
        }
        Ok(())
    }

    fn process_msg(&mut self, msg: player::FromPandora) -> Result<()> {
        trace!("UI update message: {:?}", msg);
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
            player::FromPandora::Quit => self.msg_quit()?,
        }
        Ok(())
    }

    fn msg_error(&mut self, msg: &str) -> Result<()> {
        trace!("Calling UI to display error");
        self.ui.display_error(msg);
        Ok(())
    }

    fn msg_reset(&mut self) -> Result<()> {
        trace!("Resetting application state");
        self.state = AppState::Reset;
        self.ui.login(ui::SessionAuth::UseSaved);
        todo!();
        Ok(())
    }

    fn msg_auth_accepted(&mut self) -> Result<()> {
        trace!("Pandora authentication complete");
        self.state = AppState::Connected;
        // Ensure that login credentials are commited to config
        self.config.borrow_mut().flush()?;
        todo!();
        Ok(())
    }

    fn msg_auth_failed(&mut self) -> Result<()> {
        // TODO: Updated auth failed message with explanation?
        trace!("Pandora authentication failed");
        self.state = AppState::Reset;
        self.ui.login(ui::SessionAuth::ForceReauth);
        todo!();
        Ok(())
    }

    fn msg_station_list(&mut self, stations: &[player::StationInfo]) -> Result<()> {
        trace!("Calling UI to display station list");
        self.ui.display_station_list(stations);
        Ok(())
    }

    fn msg_station_info(&mut self, station: &player::StationInfo) -> Result<()> {
        trace!("Calling UI to display station info");
        self.ui.display_station_info(station);
        Ok(())
    }

    fn msg_song_list(&mut self, songs: &[player::SongInfo]) -> Result<()> {
        trace!("Calling UI to display song list");
        self.ui.display_song_list(songs);
        Ok(())
    }

    fn msg_song_info(&mut self, song: &player::SongInfo) -> Result<()> {
        trace!("Calling UI to display song info");
        self.ui.display_song_info(song);
        Ok(())
    }

    fn msg_song_progress(&mut self, progress: u8) -> Result<()> {
        trace!("Calling UI to update song progress");
        self.ui.update_song_progress(progress);
        Ok(())
    }

    fn msg_quit(&mut self) -> Result<()> {
        trace!("Quitting application");
        self.state = AppState::Quit;
        Ok(())
    }
}
