use termion::event::{Event, Key};
use tui::backend::TermionBackend;
use tui::layout::{Constraint, Direction, Layout};
use tui::terminal::Frame;
use tui::widgets::Widget;
use tui::style::{Color, Style};
use tui_logger::{Dispatcher, TuiWidgetState};
use tui_logger::EventListener;

use crate::config::Config;
use crate::errors::Result;
use crate::term::{Pane, InputEventState};

use std::cell::RefCell;
use std::rc::Rc;

pub(crate) struct LogPane {
    focused_idx: Option<usize>,
    layout: Layout,
    config: Rc<RefCell<Config>>,
    dispatcher: Rc<RefCell<Dispatcher<Event>>>,
    state: TuiWidgetState,
}

impl LogPane {
    pub fn new(config: Rc<RefCell<Config>>) -> Result<Self> {
        // Minimum of 6 lines - 2 for the border, and at least four for the log
        // output
        let layout = Layout::default()
            .direction(Direction::Vertical)
            .margin(0)
            .constraints([Constraint::Min(6)].as_ref());
        Ok(LogPane {
            focused_idx: None,
            layout,
            config,
            dispatcher: Rc::new(RefCell::new(tui_logger::Dispatcher::<Event>::new())),
            state: tui_logger::TuiWidgetState::new(),
        })
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

impl Pane for LogPane {
    fn render(
        &mut self,
        frame: &mut Frame<TermionBackend<Box<std::io::Write>>>,
    ) {
        let chunks = self.layout.clone().split(frame.size());
        let log = chunks[0];
        tui_logger::TuiLoggerSmartWidget::default()
            .border_style(self.get_style())
            .style_error(Style::default().fg(Color::Red))
            .style_warn(Style::default().fg(Color::Yellow))
            .style_info(Style::default().fg(Color::Cyan))
            .style_debug(Style::default().fg(Color::Green))
            .style_trace(Style::default().fg(Color::Magenta))
            .state(&mut self.state)
            .dispatcher(self.dispatcher.clone())
            .render(frame, log);
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
