use std::cell::RefCell;
use std::collections::VecDeque;
use std::rc::Rc;
use std::time::Duration;
// Traits included to add required methods to types
use std::io::Write;

use crossterm::{cursor, event, style, QueueableCommand};
use log::{error, trace};
use pbr::ProgressBar;

use crate::app;
use crate::config::Config;
use crate::errors::{Error, Result};

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

#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) enum SessionAuth {
    UseSaved,
    ForceReauth,
}

impl SessionAuth {
    pub(crate) fn use_saved(self) -> bool {
        SessionAuth::UseSaved == self
    }
}

fn username_empty(config: Rc<RefCell<Config>>, auth: SessionAuth) -> bool {
    if auth.use_saved() {
        if let Some(username) = config.borrow().login.get_username() {
            username.is_empty()
        } else {
            true
        }
    } else {
        true
    }
}

fn password_empty(config: Rc<RefCell<Config>>, auth: SessionAuth) -> bool {
    if auth.use_saved() {
        if let Ok(Some(password)) = config.borrow().login.get_password() {
            password.is_empty()
        } else {
            true
        }
    } else {
        true
    }
}

pub(crate) struct Terminal {
    config: Rc<RefCell<Config>>,
    outp: std::io::Stdout,
    events: VecDeque<event::Event>,
    now_playing: Option<app::SongInfo>,
    progress: Option<ProgressBar<std::io::Stdout>>,
}

impl Terminal {
    pub(crate) fn new(config: Rc<RefCell<Config>>) -> Self {
        Terminal {
            config,
            outp: std::io::stdout(),
            events: VecDeque::new(),
            now_playing: None,
            progress: None,
        }
    }

    pub(crate) fn drain_events(&mut self) {
        while let Ok(true) = event::poll(Duration::from_millis(0)) {
            if let Ok(event) = event::read() {
                trace!("Ignored input: {:?}", event);
            }
        }
    }

    pub(crate) fn prompt_input(&mut self, prompt: &str) -> String {
        let mut input = String::with_capacity(10);
        // Display the prompt
        let result = write!(self.outp, "{}", prompt).map_err(|e| Error::OutputFailure(Box::new(e)));
        self.handle_result(result);

        let _ = self.outp.flush();

        // Clear the event queue and start listening for user input
        self.drain_events();
        loop {
            match event::read().map_err(Error::from) {
                Ok(event::Event::Key(event::KeyEvent {
                    code: event::KeyCode::Char(c),
                    ..
                })) => input.push(c),
                Ok(event::Event::Key(event::KeyEvent {
                    code: event::KeyCode::Backspace,
                    ..
                })) => {
                    let _ = input.pop();
                }
                Ok(event::Event::Key(event::KeyEvent {
                    code: event::KeyCode::Enter,
                    ..
                })) => break,
                err => self.handle_result(err),
            }
        }

        input
    }

    pub(crate) fn handle_result<T>(&mut self, result: Result<T>) {
        if let Err(e) = result {
            self.display_error(e.to_string().as_str());
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
        for station in stations {
            self.display_station_info(station);
        }
        let result = writeln!(self.outp).map_err(|e| Error::OutputFailure(Box::new(e)));
        self.handle_result(result);
    }

    pub(crate) fn display_station_info(&mut self, station: &app::Station) {
        let result = write!(self.outp, "{} ", station.station_name)
            .map_err(|e| Error::OutputFailure(Box::new(e)));
        self.handle_result(result);

        let _ = self
            .outp
            .queue(style::SetForegroundColor(style::Color::Grey));

        let result = writeln!(self.outp, "({})", station.station_id)
            .map_err(|e| Error::OutputFailure(Box::new(e)));
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
        if let Some(progress) = &mut self.progress {
            progress.finish();
        }
        self.now_playing = Some(song.clone());
        let mut progress = ProgressBar::new(duration.as_secs());
        progress.format("╢█▌ ╟");
        progress.show_speed = false;
        progress.show_percent = false;
        progress.show_counter = false;
        progress.message(format!("{} - {} ", song.name, song.artist).as_str());
        self.progress = Some(progress);
    }

    pub(crate) fn update_playing_progress(&mut self, duration: &Duration, remaining: &Duration) {
        if let Some(progress) = &mut self.progress {
            let dur_secs = duration.as_secs();
            let remain_secs = remaining.as_secs();
            progress.set(dur_secs - remain_secs);
        }
    }

    pub(crate) fn station_prompt(&mut self) -> app::Station {
        let station_id = self.prompt_input("Station id: ");

        app::Station {
            station_id,
            station_name: String::new(),
        }
    }
}
