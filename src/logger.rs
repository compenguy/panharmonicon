use termion::event;
use tui::backend::TermionBackend;
use tui::layout::{Constraint, Direction, Layout};
use tui::terminal::Frame;
use tui::widgets::Widget;
use tui::style::{Color, Style};
use tui_logger::{Dispatcher, TuiWidgetState};
use tui_logger::EventListener;

use crate::config::Config;
use crate::errors::Result;
use crate::term::TerminalPane;

use std::cell::RefCell;
use std::rc::Rc;

pub(crate) struct LogPane {
    layout: Layout,
    config: Rc<RefCell<Config>>,
    dispatcher: Rc<RefCell<Dispatcher<event::Event>>>,
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
            layout,
            config,
            dispatcher: Rc::new(RefCell::new(tui_logger::Dispatcher::<event::Event>::new())),
            state: tui_logger::TuiWidgetState::new(),
        })
    }
}

impl TerminalPane for LogPane {
    fn render(
        &mut self,
        frame: &mut Frame<TermionBackend<Box<std::io::Write>>>,
    ) {
        let chunks = self.layout.clone().split(frame.size());
        let log = chunks[0];
        tui_logger::TuiLoggerSmartWidget::default()
            .border_style(Style::default().fg(Color::Black))
            .style_error(Style::default().fg(Color::Red))
            .style_warn(Style::default().fg(Color::Yellow))
            .style_info(Style::default().fg(Color::Cyan))
            .style_debug(Style::default().fg(Color::Green))
            .style_trace(Style::default().fg(Color::Magenta))
            .state(&mut self.state)
            .dispatcher(self.dispatcher.clone())
            .render(frame, log);
    }
}
