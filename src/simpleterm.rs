use std::cell::RefCell;
use std::rc::Rc;
use std::time::Duration;
// Traits included to add required methods to types
use std::io::BufRead;
use std::io::Read;
use std::io::Write;

use log::{error, trace};
use pbr::ProgressBar;
use termion::{async_stdin, color, color::Fg, cursor};

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
        format!("{}[{}] {}{}", msg_color, level, msg, Fg(color::Reset),)
    } else {
        format!("{}{}", Fg(color::Reset), msg,)
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
    outp: std::io::Stdout,
    inp: std::io::BufReader<termion::AsyncReader>,
    now_playing: Option<app::SongInfo>,
    progress: Option<ProgressBar<std::io::Stdout>>,
}

impl Terminal {
    pub(crate) fn new(config: Rc<RefCell<Config>>) -> Self {
        Terminal {
            config,
            outp: std::io::stdout(),
            inp: std::io::BufReader::new(async_stdin()),
            now_playing: None,
            progress: None,
        }
    }

    fn drain_input(&mut self) {
        let mut buffer = [0u8; 16];
        loop {
            if let Ok(count) = self.inp.read(&mut buffer) {
                if count < buffer.len() {
                    break;
                }
            }
        }
    }

    pub(crate) fn prompt_input(&mut self, prompt: &str) -> String {
        let mut input = String::with_capacity(10);
        // Display the prompt
        let result = write!(self.outp, "{}", prompt).map_err(|e| Error::OutputFailure(Box::new(e)));
        self.handle_result(result);

        // Make sure it flushes to screen
        let result = self
            .outp
            .flush()
            .map_err(|e| Error::OutputFailure(Box::new(e)));
        self.handle_result(result);

        // We don't want to read stale input from before we prompt the user
        // for input
        self.drain_input();

        while !input.ends_with("\\n") {
            // Read the user input
            let result = self
                .inp
                .read_line(&mut input)
                .map_err(|e| Error::InputFailure(Box::new(e)));
            // We're doing something resembling blocking for user input
            match result {
                Ok(0) => {
                    std::thread::sleep(Duration::from_millis(50));
                    trace!("No input");
                },
                Ok(_) => trace!("Read user input"),
                e => {
                    self.handle_result(e);
                    error!("Input read error");
                },
            }
        }
        input.trim().to_string()
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

    pub(crate) fn display_playing(&mut self, song: &app::SongInfo, duration: &Duration) {
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
        duration: &Duration,
        remaining: &Duration,
    ) {
        if let Some(progress) = &mut self.progress {
            let dur_secs = duration.as_secs();
            let remain_secs = remaining.as_secs();
            progress.set(dur_secs - remain_secs);
        }
    }

    pub(crate) fn station_prompt(&mut self) -> app::Station {
        let station_id = self.prompt_input("Station id: ");

        app::Station {
            station_id: station_id,
            station_name: String::new(),
        }
    }
}
