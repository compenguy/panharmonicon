use std::cell::RefCell;
use std::collections::{HashMap, VecDeque};
use std::rc::Rc;
use std::time::{Duration, Instant};
// Traits included to add required methods to types
use std::io::Write;

use crossterm::terminal;
use crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, Clear, ClearType, EnterAlternateScreen, LeaveAlternateScreen,
};
use crossterm::{cursor, event, style, QueueableCommand};
use ellipse::Ellipse;
use lazy_static::lazy_static;
use log::{error, trace};
use pbr::ProgressBar;

use crate::app;
use crate::config::Config;
use crate::errors::{Error, Result};
use crate::term::{password_empty, username_empty, SessionAuth};

#[derive(Debug, PartialOrd, PartialEq, Eq, Clone, Copy, Hash)]
pub(crate) enum ApplicationSignal {
    Quit,
    VolumeUp,
    VolumeDown,
    Mute,
    Unmute,
    ToggleMuteUnmute,
    Play,
    Pause,
    TogglePlayPause,
    ThumbsUpTrack,
    ThumbsDownTrack,
    RemoveTrackRating,
    SleepTrack,
    NextTrack,
    ChangeStation,
    ShowPlaylist,
}

lazy_static! {
    static ref INPUT_MAPPING: HashMap<event::KeyEvent, ApplicationSignal> = {
        let mut mapping = HashMap::new();
        mapping.insert(
            event::KeyEvent::new(event::KeyCode::Char('c'), event::KeyModifiers::CONTROL),
            ApplicationSignal::Quit,
        );
        mapping.insert(
            event::KeyEvent::from(event::KeyCode::Char('q')),
            ApplicationSignal::Quit,
        );
        mapping.insert(
            event::KeyEvent::from(event::KeyCode::Char('(')),
            ApplicationSignal::VolumeDown,
        );
        mapping.insert(
            event::KeyEvent::from(event::KeyCode::Char(')')),
            ApplicationSignal::VolumeUp,
        );
        mapping.insert(
            event::KeyEvent::from(event::KeyCode::PageDown),
            ApplicationSignal::Mute,
        );
        mapping.insert(
            event::KeyEvent::from(event::KeyCode::PageUp),
            ApplicationSignal::Unmute,
        );
        mapping.insert(
            event::KeyEvent::from(event::KeyCode::Char('*')),
            ApplicationSignal::ToggleMuteUnmute,
        );
        mapping.insert(
            event::KeyEvent::from(event::KeyCode::Char('>')),
            ApplicationSignal::Play,
        );
        mapping.insert(
            event::KeyEvent::from(event::KeyCode::Char('.')),
            ApplicationSignal::Pause,
        );
        mapping.insert(
            event::KeyEvent::from(event::KeyCode::Char('p')),
            ApplicationSignal::TogglePlayPause,
        );
        mapping.insert(
            event::KeyEvent::from(event::KeyCode::Char('+')),
            ApplicationSignal::ThumbsUpTrack,
        );
        mapping.insert(
            event::KeyEvent::from(event::KeyCode::Char('-')),
            ApplicationSignal::ThumbsDownTrack,
        );
        mapping.insert(
            event::KeyEvent::from(event::KeyCode::Char('=')),
            ApplicationSignal::RemoveTrackRating,
        );
        mapping.insert(
            event::KeyEvent::from(event::KeyCode::Char('t')),
            ApplicationSignal::SleepTrack,
        );
        mapping.insert(
            event::KeyEvent::from(event::KeyCode::Char('n')),
            ApplicationSignal::NextTrack,
        );
        mapping.insert(
            event::KeyEvent::from(event::KeyCode::Char('s')),
            ApplicationSignal::ChangeStation,
        );
        mapping.insert(
            event::KeyEvent::from(event::KeyCode::Char('l')),
            ApplicationSignal::ShowPlaylist,
        );
        mapping
    };
}

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
    application_signals: VecDeque<ApplicationSignal>,
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
            application_signals: VecDeque::new(),
            now_playing: None,
            progress: None,
        }
    }

    pub(crate) fn pop_signal(&mut self) -> Option<ApplicationSignal> {
        self.application_signals.pop_front()
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
        let result = self
            .outp
            .queue(cursor::MoveUp(stations.len() as u16))
            .map(drop)
            .map_err(Error::from);
        self.handle_result(result);
        for station in stations {
            self.display_station_info(&station.station_id, &station.station_name);
            let result = self
                .outp
                .queue(cursor::MoveToNextLine(1))
                .map(drop)
                .map_err(Error::from);
            self.handle_result(result);
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
        let result = self
            .outp
            .queue(cursor::MoveUp(songs.len() as u16))
            .map(drop)
            .map_err(Error::from);
        self.handle_result(result);
        for song in songs {
            self.display_song_info(song);
            let result = self
                .outp
                .queue(cursor::MoveToNextLine(1))
                .map(drop)
                .map_err(Error::from);
            self.handle_result(result);
        }
        let result = writeln!(self.outp).map_err(|e| Error::OutputFailure(Box::new(e)));
        self.handle_result(result);
    }

    pub(crate) fn display_song_info(&mut self, song: &app::SongInfo) {
        let result = self
            .outp
            .queue(cursor::MoveToPreviousLine(1))
            .map(drop)
            .map_err(Error::from);
        self.handle_result(result);
        let result = writeln!(self.outp, "{} by {}", song.name, song.artist)
            .map_err(|e| Error::OutputFailure(Box::new(e)));
        self.handle_result(result);
        let result = self
            .outp
            .queue(cursor::MoveToNextLine(1))
            .map(drop)
            .map_err(Error::from);
        self.handle_result(result);
    }

    pub(crate) fn display_playing(&mut self, song: &app::SongInfo, duration: &Duration) {
        if self.progress.is_some() {
            let (_, max_row) = self
                .handle_result(terminal::size().map_err(Error::from))
                .unwrap_or_default();
            let result = self
                .outp
                .queue(cursor::MoveTo(0, max_row - 1))
                .map(drop)
                .map_err(Error::from);
            self.handle_result(result);
            let result = self
                .outp
                .queue(Clear(ClearType::CurrentLine))
                .map(drop)
                .map_err(Error::from);
            self.handle_result(result);
        }
        if let Some(progress) = &mut self.progress {
            progress.finish();
            // TODO: Possibly scroll down by one line to preserve the finished
            // progressbar on screen?
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
        if self.progress.is_some() {
            let (_, max_row) = self
                .handle_result(terminal::size().map_err(Error::from))
                .unwrap_or_default();
            let result = self
                .outp
                .queue(cursor::MoveTo(0, max_row - 1))
                .map(drop)
                .map_err(Error::from);
            self.handle_result(result);
            let result = self
                .outp
                .queue(Clear(ClearType::CurrentLine))
                .map(drop)
                .map_err(Error::from);
            self.handle_result(result);
        }
        if let Some(progress) = &mut self.progress {
            let elapsed_secs = elapsed.as_secs();
            progress.set(elapsed_secs);
        }
    }

    pub(crate) fn station_prompt(&mut self) -> String {
        self.prompt_input("Station id: ")
    }

    pub(crate) fn prompt_input(&mut self, prompt: &str) -> String {
        // Save cursor position, move up a line, clear it, display prompt, and show cursor
        let result = self
            .outp
            .queue(cursor::SavePosition)
            .map(drop)
            .map_err(Error::from);
        self.handle_result(result);
        let result = self
            .outp
            .queue(cursor::MoveToPreviousLine(1))
            .map(drop)
            .map_err(Error::from);
        self.handle_result(result);
        let result = self
            .outp
            .queue(Clear(ClearType::CurrentLine))
            .map(drop)
            .map_err(Error::from);
        self.handle_result(result);
        let result =
            writeln!(self.outp, "{}", prompt).map_err(|e| Error::OutputFailure(Box::new(e)));
        self.handle_result(result);
        let result = self.outp.queue(cursor::Show).map(drop).map_err(Error::from);
        self.handle_result(result);
        let result = self
            .outp
            .queue(cursor::EnableBlinking)
            .map(drop)
            .map_err(Error::from);
        self.handle_result(result);
        let result = self
            .outp
            .flush()
            .map_err(|e| Error::OutputFailure(Box::new(e)));
        self.handle_result(result);

        // TODO: track edit position, and accept arrow keys to edit string in-place
        let mut input = String::with_capacity(10);
        // Drain the input buffer before we switch to blocking-style input
        self.process_messages(Duration::default());
        loop {
            trace!("Blocking on user input. Accumulated input: {}", input);
            // No need to poll - we're doing blocking-style input
            if let Some(event::Event::Key(event::KeyEvent { code, modifiers })) =
                self.handle_result(event::read().map_err(Error::from))
            {
                match (code, modifiers) {
                    (event::KeyCode::Char('c'), event::KeyModifiers::CONTROL) => {
                        // Clear input, clear current line, restore cursor position,
                        // and exit loop
                        input.clear();
                        let result = self
                            .outp
                            .queue(Clear(ClearType::CurrentLine))
                            .map(drop)
                            .map_err(Error::from);
                        self.handle_result(result);
                        let result = self
                            .outp
                            .flush()
                            .map_err(|e| Error::OutputFailure(Box::new(e)));
                        self.handle_result(result);
                        break;
                    }
                    (event::KeyCode::Esc, _) => {
                        // Clear input, clear current line, restore cursor position,
                        // and exit loop
                        input.clear();
                        let result = self
                            .outp
                            .queue(Clear(ClearType::CurrentLine))
                            .map(drop)
                            .map_err(Error::from);
                        self.handle_result(result);
                        let result = self
                            .outp
                            .flush()
                            .map_err(|e| Error::OutputFailure(Box::new(e)));
                        self.handle_result(result);
                        break;
                    }
                    (event::KeyCode::Enter, _) => {
                        // Clear current line and exit loop
                        let result = self
                            .outp
                            .queue(Clear(ClearType::CurrentLine))
                            .map(drop)
                            .map_err(Error::from);
                        self.handle_result(result);
                        let result = self
                            .outp
                            .flush()
                            .map_err(|e| Error::OutputFailure(Box::new(e)));
                        self.handle_result(result);
                        break;
                    }
                    (event::KeyCode::Char(c), _) => {
                        // Add a char to the input buffer,
                        // display char at current cursor position
                        let result = write!(self.outp, "{}", c)
                            .map_err(|e| Error::OutputFailure(Box::new(e)));
                        self.handle_result(result);
                        let result = self
                            .outp
                            .flush()
                            .map_err(|e| Error::OutputFailure(Box::new(e)));
                        self.handle_result(result);
                        input.push(c);
                    }
                    (event::KeyCode::Backspace, _) => {
                        // Remove a saved char, move cursor left one,
                        // and clear the line from that point on
                        let result = self
                            .outp
                            .queue(cursor::MoveLeft(1))
                            .map(drop)
                            .map_err(Error::from);
                        self.handle_result(result);
                        let result = self
                            .outp
                            .queue(Clear(ClearType::UntilNewLine))
                            .map(drop)
                            .map_err(Error::from);
                        self.handle_result(result);
                        let result = self
                            .outp
                            .flush()
                            .map_err(|e| Error::OutputFailure(Box::new(e)));
                        self.handle_result(result);
                        let _ = input.pop();
                    }
                    x => trace!("Ignored input event during prompt: {:?}", x),
                }
            }
        }
        // Hide cursor, restore cursor position, and return result
        let result = self
            .outp
            .queue(cursor::DisableBlinking)
            .map(drop)
            .map_err(Error::from);
        self.handle_result(result);
        let result = self.outp.queue(cursor::Hide).map(drop).map_err(Error::from);
        self.handle_result(result);
        let result = self
            .outp
            .queue(cursor::RestorePosition)
            .map(drop)
            .map_err(Error::from);
        self.handle_result(result);
        input
    }

    pub(crate) fn process_messages(&mut self, timeout: Duration) {
        let now = Instant::now();
        loop {
            let remaining = timeout.checked_sub(now.elapsed()).unwrap_or_default();
            if let Ok(true) = event::poll(remaining) {
                match self.handle_result(event::read().map_err(Error::from)) {
                    Some(event::Event::Key(ke)) => self.translate_key_input(ke),
                    Some(oe) => trace!("Unhandled input event: {:?}", oe),
                    None => {}
                }
            } else {
                break;
            }
        }
    }

    pub(crate) fn translate_key_input(&mut self, key_event: event::KeyEvent) {
        if let Some(signal) = INPUT_MAPPING.get(&key_event) {
            trace!(
                "Keycode {:?} matched application event {:?}",
                key_event,
                signal
            );
            self.application_signals.push_back(*signal);
        } else {
            trace!("Unhandled keycode: {:?}", key_event);
        }
    }
}

impl Drop for Terminal {
    fn drop(&mut self) {
        let _ = self.outp.queue(LeaveAlternateScreen);
        let _ = disable_raw_mode();
    }
}
