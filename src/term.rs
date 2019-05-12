use termion::raw::IntoRawMode;
use tui::backend::TermionBackend;
use tui::terminal::Frame;

use crate::errors::{Error, Result};

use std::cell::RefCell;

pub(crate) struct TerminalWin {
    terminal: RefCell<tui::Terminal<TermionBackend<termion::raw::RawTerminal<std::io::Stdout>>>>,
    children: RefCell<Vec<Box<dyn TerminalPane>>>,
}

impl TerminalWin {
    pub fn new() -> Result<Self> {
        let stdout = std::io::stdout()
            .into_raw_mode()
            .map_err(|e| Error::TerminalIoInitFailure(Box::new(e)))?;
        let backend = TermionBackend::new(stdout);
        let mut terminal =
            tui::Terminal::new(backend).map_err(|e| Error::TerminalInitFailure(Box::new(e)))?;
        terminal
            .clear()
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
        frame: &mut Frame<TermionBackend<termion::raw::RawTerminal<std::io::Stdout>>>,
    );
}
