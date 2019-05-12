use termion::raw::IntoRawMode;
use termion::input::MouseTerminal;
use termion::screen::AlternateScreen;
use tui::backend::TermionBackend;
use tui::terminal::Frame;

use crate::errors::{Error, Result};
use crate::config::Config;

use std::cell::RefCell;
use std::rc::Rc;

pub(crate) struct TerminalWin {
    terminal: RefCell<tui::Terminal<TermionBackend<Box<std::io::Write>>>>,
    children: RefCell<Vec<Box<dyn TerminalPane>>>,
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

    pub fn add_pane(&mut self, pane: impl TerminalPane + 'static) -> Result<()> {
        self.children.borrow_mut().push(Box::new(pane));
        Ok(())
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
}

pub(crate) trait TerminalPane {
    fn render(
        &mut self,
        frame: &mut Frame<TermionBackend<Box<std::io::Write>>>,
    );
}
