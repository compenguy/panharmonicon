use tui::backend::TermionBackend;
use tui::layout::{Constraint, Direction, Layout};
use tui::terminal::Frame;
use tui::widgets::Widget;
use tui::widgets::{Block, Borders};

use crate::config::Config;
use crate::errors::Result;
use crate::term::TerminalPane;

use std::cell::RefCell;
use std::rc::Rc;

pub(crate) struct PandoraPane {
    layout: Layout,
    config: Rc<RefCell<Config>>,
}

impl PandoraPane {
    pub fn new(config: Rc<RefCell<Config>>) -> Result<Self> {
        // Two lines for login/session info
        // At least four lines for playlist + now playing
        let layout = Layout::default()
            .direction(Direction::Vertical)
            .margin(0)
            .constraints([Constraint::Length(4), Constraint::Min(6)].as_ref());
        Ok(PandoraPane { layout, config })
    }
}

impl TerminalPane for PandoraPane {
    fn render(
        &mut self,
        frame: &mut Frame<TermionBackend<termion::raw::RawTerminal<std::io::Stdout>>>,
    ) {
        let chunks = self.layout.clone().split(frame.size());
        let (login, now_playing) = (chunks[0], chunks[1]);
        Block::default()
            .title("Login")
            .borders(Borders::ALL)
            .render(frame, login);
        Block::default()
            .title("Now Playing")
            .borders(Borders::ALL)
            .render(frame, now_playing);
    }
}
