use termion::event::{Event, Key};
use tui::backend::TermionBackend;
use tui::layout::{Constraint, Direction, Layout};
use tui::terminal::Frame;
use tui::widgets::Widget;
use tui::widgets::{Block, Borders};

use crate::config::Config;
use crate::errors::Result;
use crate::term::{Pane, InputEventState};

use std::cell::RefCell;
use std::rc::Rc;

pub(crate) struct PandoraPane {
    focused_idx: Option<usize>,
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
        Ok(PandoraPane { focused_idx: None, layout, config })
    }

    pub fn focus_next(&mut self) -> bool {
        // We return false if the last child was already focused
        // otherwise, we increment the focused_idx and return true
        let new_state = match self.focused_idx {
            Some(x) if x == self.layout_count() - 1 => return false,
            Some(x) => Some(x+1),
            None => Some(0),
        };
        self.focused_idx = new_state;
        true
    }
}

impl Pane for PandoraPane {
    fn render(
        &mut self,
        frame: &mut Frame<TermionBackend<Box<std::io::Write>>>,
    ) {
        let chunks = self.layout.clone().split(frame.size());
        let (login, now_playing) = (chunks[0], chunks[1]);
        let (_login_focused, _now_playing_focused) = (self.focused_idx == Some(0), self.focused_idx == Some(1));
        // TODO: use login_focused and now_playing_focused to adjust the styling
        Block::default()
            .title("Login")
            .borders(Borders::ALL)
            .border_style(self.get_style())
            .render(frame, login);
        Block::default()
            .title("Now Playing")
            .borders(Borders::ALL)
            .border_style(self.get_style())
            .render(frame, now_playing);
    }

    fn get_layout(&self) -> &Layout {
        &self.layout
    }

    fn handle_input(&mut self, event: &termion::event::Event) -> InputEventState {
        match event {
            Event::Key(Key::Char('\t')) => match self.focus_next() {
                    false => InputEventState::Unhandled,
                    true => InputEventState::Handled,
            },
            _ => InputEventState::Unhandled,
        }
    }

    fn is_focusable(&self) -> bool {
        true
    }

    fn is_focused(&self) -> bool {
        self.focused_idx.is_some()
    }

    fn focus(&mut self) {
        self.focus_next();
    }

    fn unfocus(&mut self) {
        self.focused_idx = None
    }

}
