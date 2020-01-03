use std::sync::mpsc;
use std::cell::RefCell;
use std::rc::Rc;

use log::debug;

use crate::errors::{Result, Error};
use crate::config::Config;
use crate::panharmonicon;

pub(crate) enum AppState {
    Reset,
    Login,
    StationSelect,
    Play,
    Pause,
}

pub(crate) struct App {
    last_activity: std::time::Instant,
    state: AppState,
    config: Rc<RefCell<Config>>,
    thread_handle: std::thread::JoinHandle<()>,
    recv_channel: mpsc::Receiver<panharmonicon::FromPandora>,
    send_channel: mpsc::Sender<panharmonicon::ToPandora>,
}

impl App {
    pub fn new(config: Rc<RefCell<Config>>) -> Self {
        let (send_to_pandora, recv_from_ui) = mpsc::channel();
        let (send_to_ui, recv_from_pandora) = mpsc::channel();

        let builder = std::thread::Builder::new();

        let handle = builder.name("panharmonicon_player".to_string()).spawn(move|| {
            panharmonicon::run(send_to_ui, recv_from_ui, None);
        }).expect("Failed to spawn audio player thread");

        Self {
            last_activity: std::time::Instant::now(),
            state: AppState::Reset,
            config: config.clone(),
            thread_handle: handle,
            recv_channel: recv_from_pandora,
            send_channel: send_to_pandora,
        }
    }

    fn update_watchdog(&mut self) {
        self.last_activity = std::time::Instant::now();
    }

    pub fn try_run(&mut self) -> Result<()> {
        loop {
            match self.recv_channel.try_recv() {
                Ok(msg) => self.process_msg(msg),
                Err(mpsc::TryRecvError::Empty) => break,
                Err(e) => Err(e)?,
            }
            self.update_watchdog();
        }
        Ok(())
    }

    pub fn run(&mut self) -> Result<()> {
        // Run through all pending messages
        self.try_run()?;
        // Then block on next message
        // TODO: before blocking on message, check to see if watchdog expired
        // else we could be waiting a *very* long time
        self.process_msg(self.recv_channel.recv().map_err(Error::from)?);
        self.update_watchdog();
        Ok(())
    }

    fn process_msg(&mut self, msg: panharmonicon::FromPandora) {
        debug!("UI update message: {:?}", msg);
        match msg {
            panharmonicon::FromPandora::Reset => todo!(),
            panharmonicon::FromPandora::AuthAccepted => todo!(),
            panharmonicon::FromPandora::AuthFailed => todo!(),
            panharmonicon::FromPandora::StationList => todo!(),
            panharmonicon::FromPandora::StationInfo => todo!(),
            panharmonicon::FromPandora::SongList => todo!(),
            panharmonicon::FromPandora::SongInfo => todo!(),
            panharmonicon::FromPandora::SongProgress => todo!(),
            panharmonicon::FromPandora::AddToPlaylist => todo!(),
            panharmonicon::FromPandora::Error(_e)=> todo!(),
        }
    }
}
