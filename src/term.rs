use std::cell::RefCell;
use std::rc::Rc;
// Traits included to add required methods to types
use std::io::BufRead;
use std::io::Write;

use log::error;
use pbr::ProgressBar;
use termion::{async_stdin, color, color::Fg, cursor, screen};

use crate::app;
use crate::config::Config;
use crate::errors::{Error, Result};

fn display_main<W: std::io::Write>(outp: &mut W, msg: &str, level: Option<log::LevelFilter>) {
    let formatted_msg = if let Some(level) = level {
        let msg_color: Box<dyn core::fmt::Display> = match level {
            log::LevelFilter::Off => Box::new(Fg(color::Reset)),
            log::LevelFilter::Error => Box::new(Fg(color::Red)),
            log::LevelFilter::Warn => Box::new(Fg(color::Yellow)),
            log::LevelFilter::Info => Box::new(Fg(color::Reset)),
            log::LevelFilter::Debug => Box::new(Fg(color::Green)),
            log::LevelFilter::Trace => Box::new(Fg(color::LightBlack)),
        };
        format!(
            "{}{}[{}] {}{}{}",
            screen::ToMainScreen,
            msg_color,
            level,
            msg,
            Fg(color::Reset),
            screen::ToAlternateScreen,
        )
    } else {
        format!(
            "{}{}{}{}",
            screen::ToMainScreen,
            Fg(color::Reset),
            msg,
            screen::ToAlternateScreen,
        )
    };

    if let Err(e) = writeln!(outp, "{}", formatted_msg) {
        error!("Error writing to ui output handle: {:?}", e);
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
    outp: screen::AlternateScreen<std::io::Stdout>,
    inp: std::io::BufReader<termion::AsyncReader>,
    now_playing: Option<app::SongInfo>,
    progress: Option<ProgressBar<std::io::Stdout>>,
}

impl Terminal {
    pub(crate) fn new(config: Rc<RefCell<Config>>) -> Self {
        let mut term = Terminal {
            config,
            outp: screen::AlternateScreen::from(std::io::stdout()),
            inp: std::io::BufReader::new(async_stdin()),
            now_playing: None,
            progress: None,
        };

        term.hide_cursor();
        term
    }

    pub(crate) fn handle_result<T>(&mut self, result: Result<T>) {
        if let Err(e) = result {
            self.display_error(e.to_string().as_str());
        }
    }

    pub(crate) fn display_error(&mut self, msg: &str) {
        display_main(&mut self.outp, msg, Some(log::LevelFilter::Error));
    }

    pub(crate) fn hide_cursor(&mut self) {
        let result =
            write!(self.outp, "{}", cursor::Hide).map_err(|e| Error::OutputFailure(Box::new(e)));
        self.handle_result(result);
        let result = self
            .outp
            .flush()
            .map_err(|e| Error::OutputFailure(Box::new(e)));
        self.handle_result(result);
    }

    pub(crate) fn show_cursor(&mut self) {
        let result =
            write!(self.outp, "{}", cursor::Show).map_err(|e| Error::OutputFailure(Box::new(e)));
        self.handle_result(result);
        let result = self
            .outp
            .flush()
            .map_err(|e| Error::OutputFailure(Box::new(e)));
        self.handle_result(result);
    }

    pub(crate) fn login(&mut self, auth: SessionAuth) -> Result<()> {
        let mut tmp_auth = auth;
        while username_empty(self.config.clone(), tmp_auth) {
            let mut username = String::new();
            let result =
                write!(self.outp, "Pandora user: ").map_err(|e| Error::OutputFailure(Box::new(e)));
            self.handle_result(result);
            let result = self
                .outp
                .flush()
                .map_err(|e| Error::OutputFailure(Box::new(e)));
            self.handle_result(result);
            self.show_cursor();
            let result = self
                .inp
                .read_line(&mut username)
                .map_err(|e| Error::InputFailure(Box::new(e)));
            self.handle_result(result);
            self.hide_cursor();
            self.config.borrow_mut().login.update_username(&username);
            // Ensure that we retry if the updated credentials are blank
            tmp_auth = SessionAuth::UseSaved;
        }

        tmp_auth = auth;
        while password_empty(self.config.clone(), tmp_auth) {
            let mut password = String::new();
            let result = write!(self.outp, "Pandora password: ")
                .map_err(|e| Error::OutputFailure(Box::new(e)));
            self.handle_result(result);
            let result = self
                .outp
                .flush()
                .map_err(|e| Error::OutputFailure(Box::new(e)));
            self.handle_result(result);
            self.show_cursor();
            let result = self
                .inp
                .read_line(&mut password)
                .map_err(|e| Error::InputFailure(Box::new(e)));
            self.handle_result(result);
            self.hide_cursor();
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
        let result = writeln!(
            self.outp,
            "{} {}({}){}",
            station.station_name,
            Fg(color::LightBlack),
            station.station_id,
            Fg(color::Reset)
        )
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

    pub(crate) fn display_playing(&mut self, song: &app::SongInfo, duration: &std::time::Duration) {
        if let Some(progress) = &mut self.progress {
            progress.finish();
        }
        self.now_playing = Some(song.clone());
        let mut progress = ProgressBar::new(duration.as_secs());
        progress.format("╢▌▌░╟");
        progress.show_speed = false;
        progress.show_percent = false;
        progress.show_counter = false;
        progress.message(format!("{} - {} ", song.name, song.artist).as_str());
        self.progress = Some(progress);
    }

    pub(crate) fn update_playing_progress(
        &mut self,
        duration: &std::time::Duration,
        remaining: &std::time::Duration,
    ) {
        if let Some(progress) = &mut self.progress {
            let dur_secs = duration.as_secs();
            let remain_secs = remaining.as_secs();
            progress.set(dur_secs - remain_secs);
        }
        /*
        let msg = format!(
            "{:2}:{:02}/{:2}:{:02}",
            remain_secs / 60,
            remain_secs % 60,
            dur_secs / 60,
            dur_secs % 60
        );
        let result = write!(
            self.outp,
            "{}{}",
            msg,
            cursor::Left(
                msg.len()
                    .try_into()
                    .expect("Message length exceeded valid line size")
            ),
        )
        .map_err(|e| Error::OutputFailure(Box::new(e)));
        self.handle_result(result);
        let result = self
            .outp
            .flush()
            .map_err(|e| Error::OutputFailure(Box::new(e)));
        self.handle_result(result);
        */
    }

    pub(crate) fn station_prompt(&mut self) -> app::Station {
        let mut station_id = String::new();
        while station_id.is_empty() {
            let result =
                write!(self.outp, "Station Id: ").map_err(|e| Error::OutputFailure(Box::new(e)));
            self.handle_result(result);
            let result = self
                .outp
                .flush()
                .map_err(|e| Error::OutputFailure(Box::new(e)));
            self.handle_result(result);
            self.show_cursor();
            let result = self
                .inp
                .read_line(&mut station_id)
                .map_err(|e| Error::InputFailure(Box::new(e)));
            self.handle_result(result);
            self.hide_cursor();
        }

        app::Station {
            station_id: station_id.trim().to_string(),
            station_name: String::new(),
        }
    }
}
