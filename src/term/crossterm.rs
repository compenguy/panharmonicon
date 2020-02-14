use std::cell::RefCell;
use std::rc::Rc;
use std::time::Duration;
// Traits included to add required methods to types
use std::io::Write;

use crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen,
};
use crossterm::{style, QueueableCommand};
use ellipse::Ellipse;
use log::error;
use pbr::ProgressBar;

use crate::app;
use crate::config::Config;
use crate::errors::{Error, Result};
use crate::term::{crossterm_input, password_empty, username_empty, SessionAuth};

fn display_main<W: std::io::Write>(outp: &mut W, msg: &str, level: Option<log::LevelFilter>) {
    let color = match level {
        Some(log::LevelFilter::Off) => style::Color::Reset,
        Some(log::LevelFilter::Error) => style::Color::Red,
        Some(log::LevelFilter::Warn) => style::Color::Yellow,
        Some(log::LevelFilter::Info) => style::Color::Reset,
        Some(log::LevelFilter::Debug) => style::Color::Green,
        Some(log::LevelFilter::Trace) => style::Color::Grey,
        None => style::Color::Reset,
    };
    if let Err(e) = outp.queue(style::SetForegroundColor(color)) {
        error!("Error enqueueing output action to output handle: {:?}", e);
    }
    if let Err(e) = writeln!(outp, "{}", msg) {
        error!("Error writing to ui output handle: {:?}", e);
    }
    if let Err(e) = outp.queue(style::ResetColor) {
        error!("Error enqueueing output action to output handle: {:?}", e);
    }
    if let Err(e) = outp.flush() {
        error!("Error flusing to output handle: {:?}", e);
    }
}

pub(crate) struct Terminal {
    config: Rc<RefCell<Config>>,
    outp: std::io::Stdout,
    // TODO: add mechanism to force spawned thread to terminate
    input_event_queue: crossterm_input::EventQueue,
    now_playing: Option<app::SongInfo>,
    progress: Option<ProgressBar<std::io::Stdout>>,
}

impl Terminal {
    pub(crate) fn new(config: Rc<RefCell<Config>>) -> Self {
        let mut outp = std::io::stdout();
        let _ = outp.queue(EnterAlternateScreen);
        let _ = enable_raw_mode();
        let _ = outp.flush();

        Terminal {
            config,
            outp,
            input_event_queue: crossterm_input::EventQueue::new(),
            now_playing: None,
            progress: None,
        }
    }

    pub(crate) fn pop_signal(&mut self) -> Option<crossterm_input::ApplicationSignal> {
        self.input_event_queue.pop()
    }

    pub(crate) fn handle_result<T>(&mut self, result: Result<T>) -> Option<T> {
        match result {
            Ok(t) => Some(t),
            Err(e) => {
                self.display_error(e.to_string().as_str());
                None
            }
        }
    }

    pub(crate) fn display_error(&mut self, msg: &str) {
        display_main(&mut self.outp, msg, Some(log::LevelFilter::Error));
    }

    pub(crate) fn login(&mut self, auth: SessionAuth) -> Result<()> {
        let mut tmp_auth = auth;
        while username_empty(self.config.clone(), tmp_auth) {
            let username = self.prompt_input("Pandora user: ");
            self.config.borrow_mut().login.update_username(&username);
            // Ensure that we retry if the updated credentials are blank
            tmp_auth = SessionAuth::UseSaved;
        }

        tmp_auth = auth;
        while password_empty(self.config.clone(), tmp_auth) {
            let password = self.prompt_input("Pandora password: ");
            let result = self.config.borrow_mut().login.update_password(&password);
            if let Err(e) = result {
                self.display_error(format!("Error updating password: {:?}", e).as_str());
                // Ensure that we retry if the password failed to update
                tmp_auth = SessionAuth::ForceReauth;
            } else {
                // Ensure that we retry if the updated credentials are blank
                tmp_auth = SessionAuth::UseSaved;
            }
        }
        Ok(())
    }

    pub(crate) fn display_station_list(&mut self, stations: &[app::Station]) {
        // TODO: use cursor goto to position each entry, and clear each line before displaying
        for station in stations {
            self.display_station_info(&station.station_id, &station.station_name);
        }
        let result = writeln!(self.outp).map_err(|e| Error::OutputFailure(Box::new(e)));
        self.handle_result(result);
    }

    pub(crate) fn display_station_info(&mut self, station_id: &str, station_name: &str) {
        let result =
            write!(self.outp, "{} ", station_name).map_err(|e| Error::OutputFailure(Box::new(e)));
        self.handle_result(result);

        let _ = self
            .outp
            .queue(style::SetForegroundColor(style::Color::Grey));

        let result =
            writeln!(self.outp, "({})", station_id).map_err(|e| Error::OutputFailure(Box::new(e)));
        self.handle_result(result);

        let _ = self
            .outp
            .queue(style::SetForegroundColor(style::Color::Reset));

        let result = self
            .outp
            .flush()
            .map_err(|e| Error::OutputFailure(Box::new(e)));
        self.handle_result(result);
    }

    pub(crate) fn display_song_list(&mut self, songs: &[app::SongInfo]) {
        // TODO: use cursor goto to position each entry, and clear each line before displaying
        for song in songs {
            self.display_song_info(song);
        }
        let result = writeln!(self.outp).map_err(|e| Error::OutputFailure(Box::new(e)));
        self.handle_result(result);
    }

    pub(crate) fn display_song_info(&mut self, song: &app::SongInfo) {
        let result = writeln!(self.outp, "{} by {}", song.name, song.artist)
            .map_err(|e| Error::OutputFailure(Box::new(e)));
        self.handle_result(result);
    }

    pub(crate) fn display_playing(&mut self, song: &app::SongInfo, duration: &Duration) {
        // TODO: use cursor goto to position, and clear line before displaying
        if let Some(progress) = &mut self.progress {
            progress.finish();
        }
        self.now_playing = Some(song.clone());
        let mut progress = ProgressBar::new(duration.as_secs());
        progress.format("╢█▌ ╟");
        progress.show_speed = false;
        progress.show_percent = false;
        progress.show_counter = false;

        let song_name = song.name.as_str().truncate_ellipse(20usize);
        let artist = song.artist.as_str().truncate_ellipse(15usize);
        progress.message(format!("{} - {} ", song_name, artist).as_str());
        self.progress = Some(progress);
    }

    pub(crate) fn update_playing_progress(&mut self, elapsed: &Duration) {
        if let Some(progress) = &mut self.progress {
            // TODO: use cursor goto to position, and clear line before displaying
            let elapsed_secs = elapsed.as_secs();
            progress.set(elapsed_secs);
        }
    }

    pub(crate) fn station_prompt(&mut self) -> String {
        self.prompt_input("Station id: ")
    }

    pub(crate) fn prompt_input(&mut self, prompt: &str) -> String {
        // TODO: use cursor goto to position, and clear line before displaying
        let result =
            writeln!(self.outp, "{}", prompt).map_err(|e| Error::OutputFailure(Box::new(e)));
        self.handle_result(result);

        let result = self.input_event_queue.prompt_input();
        self.handle_result(result).unwrap_or_default()
    }
}

impl Drop for Terminal {
    fn drop(&mut self) {
        let _ = self.outp.queue(LeaveAlternateScreen);
        let _ = disable_raw_mode();
    }
}
