use termion::raw::IntoRawMode;
use tui::backend::TermionBackend;
use tui::layout::Layout;

use crate::errors::{Error, Result};

pub(crate) struct TerminalWin {
    terminal: tui::Terminal<TermionBackend<termion::raw::RawTerminal<std::io::Stdout>>>,
    children: Vec<Box<dyn TerminalPane>>,
}

impl TerminalWin {
    pub fn new() -> Result<Self> {
        let stdout = std::io::stdout()
            .into_raw_mode()
            .map_err(|e| Error::TerminalIoInitFailure(Box::new(e)))?;
        let backend = TermionBackend::new(stdout);
        let terminal =
            tui::Terminal::new(backend).map_err(|e| Error::TerminalInitFailure(Box::new(e)))?;
        Ok(TerminalWin {
            terminal,
            children: Vec::new(),
        })
    }

    pub fn add_pane(&mut self, pane: impl TerminalPane + 'static) -> Result<()> {
        let termpane = Box::new(pane);
        self.children.push(termpane);
        Ok(())
    }
}

pub(crate) trait TerminalPane {
    fn get_layout(&self) -> &Layout;
}
