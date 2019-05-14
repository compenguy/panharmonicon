use termion::raw::IntoRawMode;
use termion::input::MouseTerminal;
use termion::screen::AlternateScreen;
use termion::event::{Event, Key};
use tui::backend::TermionBackend;
use tui::terminal::Frame;
use tui::layout::Layout;

use log::{trace};

use crate::errors::{Error, Result};
use crate::config::Config;

use std::cell::RefCell;
use std::rc::Rc;

#[derive(Eq, Ord, PartialEq, PartialOrd)]
pub(crate) enum InputEventState {
    Handled,
    Unhandled,
}

pub(crate) trait FocusablePane: Pane {
}

pub(crate) trait Pane {
    fn render(
        &mut self,
        frame: &mut Frame<TermionBackend<Box<std::io::Write>>>,
    );
    fn get_layout(&self) -> &Layout;

    fn layout_count(&self) -> usize {
        // *sigh*
        // Here's how we ask how many things are in the layout
        self.get_layout().clone().split(tui::layout::Rect::default()).len()
    }

    // focusable panes should handle \t
    // unfocusable panes should ignore most input
    #[allow(unused_variables)]
    fn handle_input(&mut self, event: &termion::event::Event) -> InputEventState { InputEventState::Unhandled }

    fn is_focusable(&self) -> bool { false }

    fn is_focused(&self) -> bool { false }

    fn focus(&mut self) { }

    fn unfocus(&mut self) { }

    fn get_unfocused_style(&self) -> tui::style::Style {
        tui::style::Style::default()
            .fg(tui::style::Color::Gray)
            .bg(tui::style::Color::Black)
    }

    fn get_focused_style(&self) -> tui::style::Style {
        tui::style::Style::default()
            .fg(tui::style::Color::White)
            .bg(tui::style::Color::Black)
    }

    fn get_style(&self) -> tui::style::Style {
        if self.is_focused() {
            self.get_focused_style()
        } else {
            self.get_unfocused_style()
        }
    }
}

pub(crate) struct TerminalWin {
    terminal: RefCell<tui::Terminal<TermionBackend<Box<std::io::Write>>>>,
    children: RefCell<Vec<Box<dyn Pane>>>,
}

impl TerminalWin {
    pub fn new(config: Rc<RefCell<Config>>) -> Result<Self> {
        let stdout = std::io::stdout()
            .into_raw_mode()
            .map_err(|e| Error::TerminalIoInitFailure(Box::new(e)))?;

        // Type erasure using Box lets us nest Termion writers to our heart's content
        // It also lets us compose them arbitrarily, such as optionally having a mouse
        // writer
        let mut stdout: Box<std::io::Write> = Box::new(AlternateScreen::from(stdout));
        if config.borrow().mouse_mode {
            stdout = Box::new(MouseTerminal::from(stdout));
        }
        let backend = TermionBackend::new(stdout);

        let mut terminal =
            tui::Terminal::new(backend).map_err(|e| Error::TerminalInitFailure(Box::new(e)))?;
        terminal
            .clear()
            .map_err(|e| Error::TerminalInitFailure(Box::new(e)))?;
        terminal
            .hide_cursor()
            .map_err(|e| Error::TerminalInitFailure(Box::new(e)))?;
        Ok(TerminalWin {
            terminal: RefCell::new(terminal),
            children: RefCell::new(Vec::new()),
        })
    }

    pub fn add_pane(&mut self, pane: impl Pane + 'static) -> Result<()> {
        self.children.borrow_mut().push(Box::new(pane));
        Ok(())
    }

    // We expect the handle_input on a Pane to ignore most input if it's not focused
    pub fn handle_input(&mut self, event: &Event) {
        for wrapped_child in self.children.borrow_mut().as_mut_slice() {
            if wrapped_child.handle_input(event) == InputEventState::Handled {
                return;
            }
        }

        // Nothing claimed to handle the input, so we'll deal with what's left
        match event {
            Event::Key(Key::Char('\t')) => self.focus_next(),
            _ => trace!(target: "Key rx", "Unhandled input event {:?}", event),
        }
    }

    pub fn render(&mut self) -> Result<()> {
        self.terminal
            .borrow_mut()
            .draw(|mut f| {
                for wrapped_child in self.children.borrow_mut().as_mut_slice() {
                    wrapped_child.render(&mut f);
                }
            })
            .map_err(|e| Error::TerminalDrawFailure(Box::new(e)))
    }

    pub fn focus_next(&mut self) {
        // We need a flag to know if we need to shift the focus from one pane to the next
        // in the list
        let mut focus_next_pane = false;
        for wrapped_child in self.children.borrow_mut().as_mut_slice() {
            // Previous pane was focused, but didn't accept the input event
            // so we're handling that event by moving the focus to the next pane
            if focus_next_pane {
                wrapped_child.focus();
                return;
            }

            // This pane is focused, so we need to unfocus it, and set the flag
            // telling the next pane in the iteration to focus itself
            if wrapped_child.is_focused() {
                wrapped_child.unfocus();
                focus_next_pane = true;
            }
        }
        // Either nothing was focused, or the last pane was focused
        // in either case, let's focus the first pane
        if let Some(child) = self.children.borrow_mut().first_mut() {
            child.focus();
        }
    }
}

