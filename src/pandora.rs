use tui::layout::{Constraint, Direction, Layout};

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
            .margin(1)
            .constraints([Constraint::Length(2), Constraint::Min(4)].as_ref());
        Ok(PandoraPane { layout, config })
    }
}

impl TerminalPane for PandoraPane {
    fn get_layout(&self) -> &Layout {
        &self.layout
    }
}
