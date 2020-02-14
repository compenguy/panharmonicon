use std::collections::{HashMap, VecDeque};
use std::thread::JoinHandle;

use crossterm::event;
use lazy_static::lazy_static;
use log::{debug, trace};

use crate::errors::{Error, Result};

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

pub(crate) struct EventQueue {
    application_signals: VecDeque<ApplicationSignal>,
    _input_thread: JoinHandle<()>,
    recv: crossbeam_channel::Receiver<event::Event>,
}

impl EventQueue {
    pub(crate) fn new() -> Self {
        let (send, recv) = crossbeam_channel::unbounded();
        let input_thread: JoinHandle<()> = std::thread::spawn(move || {
            let _ = InputThread::new(send).run();
        });
        Self {
            application_signals: VecDeque::new(),
            _input_thread: input_thread,
            recv,
        }
    }

    pub(crate) fn pop(&mut self) -> Option<ApplicationSignal> {
        let _ = self.process_messages();
        self.application_signals.pop_front()
    }

    // TODO: move this whole module back into crossterm.rs so that
    // we can use cursor positioning, etc, to echo input (and backspace)
    // back to the screen.
    pub(crate) fn prompt_input(&mut self) -> Result<String> {
        let mut input = String::with_capacity(10);
        // Clear the event queue and start listening for user input
        self.process_messages()?;
        loop {
            match self.recv.recv().map_err(Error::from)? {
                // TODO: needs echoing
                event::Event::Key(event::KeyEvent {
                    code: event::KeyCode::Char(c),
                    ..
                }) => input.push(c),
                // TODO: Move cursor left one
                event::Event::Key(event::KeyEvent {
                    code: event::KeyCode::Backspace,
                    ..
                }) => {
                    let _ = input.pop();
                }
                event::Event::Key(event::KeyEvent {
                    code: event::KeyCode::Enter,
                    ..
                }) => break,
                x => debug!("Unexpected input prompt event: {:?}", x),
            }
        }
        // Trim off the trailing newline
        Ok(input.trim().into())
    }

    pub(crate) fn process_messages(&mut self) -> Result<()> {
        loop {
            match self.recv.try_recv() {
                Ok(event::Event::Key(key_event)) => self.translate_key_input(key_event),
                Ok(e) => trace!("Unhandled input event: {:?}", e),
                Err(crossbeam_channel::TryRecvError::Empty) => return Ok(()),
                Err(e) => return Err(Error::from(e)),
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

pub(crate) struct InputThread {
    send: crossbeam_channel::Sender<event::Event>,
}

impl InputThread {
    pub(crate) fn new(send: crossbeam_channel::Sender<event::Event>) -> Self {
        Self { send }
    }

    pub(crate) fn run(&mut self) -> Result<()> {
        loop {
            self.send
                .send(event::read().map_err(Error::from)?)
                .map_err(Error::from)?;
        }
    }
}
