use std::time::{Duration, Instant};
use std::{cell::RefCell, rc::Rc};

use cursive::views::{EditView, LinearLayout, Panel, SelectView, SliderView, TextView};
use cursive::{CursiveRunnable, CursiveRunner};
use log::{debug, trace};

use crate::config::Config;
use crate::model::Model;
use crate::model::{AudioMediator, PlaybackMediator, StateMediator, StationMediator};

mod callbacks;
mod dialogs;

#[cfg(feature = "emoji_labels")]
mod labels {
    pub(crate) const LABEL_PLAY_PAUSE: &str = "⏯️ ";
    pub(crate) const LABEL_SKIP: &str = "⏩";
    pub(crate) const LABEL_THUMBS_UP: &str = "👍";
    pub(crate) const LABEL_THUMBS_DOWN: &str = "👎";
    pub(crate) const LABEL_TIRED: &str = "💤";
}
#[cfg(not(feature = "emoji_labels"))]
mod labels {
    pub(crate) const LABEL_PLAY_PAUSE: &str = "Play/Pause";
    pub(crate) const LABEL_SKIP: &str = "Skip";
    pub(crate) const LABEL_THUMBS_UP: &str = "|+|";
    pub(crate) const LABEL_THUMBS_DOWN: &str = "|-|";
    pub(crate) const LABEL_TIRED: &str = ".zZ";
}

pub(crate) struct Terminal {
    model: Rc<RefCell<Model>>,
    siv: CursiveRunner<CursiveRunnable>,
}

impl Terminal {
    pub(crate) fn new(config: Rc<RefCell<Config>>) -> Self {
        let model = Rc::new(RefCell::new(Model::new(config)));
        let mut siv = cursive::crossterm().into_runner();
        siv.set_user_data(model.clone());
        let mut term = Self { model, siv };
        term.initialize();
        term
    }

    pub(crate) fn initialize(&mut self) {
        self.init_key_mappings();
        self.init_theme();
        self.init_playback();
    }

    fn init_key_mappings(&mut self) {
        // TODO: read key mappings from config
        self.siv.add_global_callback('q', callbacks::quit);
        self.siv.add_global_callback('.', callbacks::pause);
        self.siv.add_global_callback('>', callbacks::unpause);
        self.siv.add_global_callback('p', callbacks::toggle_pause);
        self.siv
            .add_global_callback('(', callbacks::decrease_volume);
        self.siv
            .add_global_callback(')', callbacks::increase_volume);
        self.siv.add_global_callback('n', callbacks::stop);
        self.siv.add_global_callback('t', callbacks::sleep_track);
        self.siv.add_global_callback('+', callbacks::rate_track_up);
        self.siv
            .add_global_callback('-', callbacks::rate_track_down);
        self.siv.add_global_callback('=', callbacks::clear_rating);
    }

    fn init_theme(&mut self) {
        self.siv
            .load_toml(include_str!("../../assets/theme.toml"))
            .expect("Error loading theme toml file");
        // TODO: Allow loading user-provided theme files at run-time
    }

    fn init_playback(&mut self) {
        self.siv.add_fullscreen_layer(dialogs::playing_view());

        // Catch screen resize requests, and hide/show appropriate controls to
        // fit the most important parts of the interface to the terminal size.
        self.siv
            .set_on_pre_event(cursive::event::Event::WindowResize, callbacks::ui_scale);
    }

    fn update_stations(&mut self) {
        trace!("Checking stations list...");
        let model = self.model.borrow_mut();
        self.siv
            .call_on_name("stations", |v: &mut SelectView<String>| {
                // If the list is empty, or there's exactly one item with an empty value
                // we should populate it a station list
                if v.is_empty()
                    || (v.len() == 1 && v.get_item(0).map(|(_, s)| s.is_empty()).unwrap_or(true))
                {
                    trace!("Updating stations list");
                    v.clear();
                    v.add_item("", String::new());
                    v.add_all(model.station_list().into_iter());
                    v.sort_by_label();
                    if let Some(station_id) = model.tuned() {
                        trace!("Updating selected station in UI to match model");
                        let opt_idx = v
                            .iter()
                            .enumerate()
                            .find(|(_, (_, st_id))| *st_id == &station_id)
                            .map(|(i, _)| i);
                        if let Some(idx) = opt_idx {
                            v.set_selection(idx);
                        }
                    } else {
                        v.set_selection(0);
                    }
                } else if model.station_count() == 0 {
                    trace!("Clearing UI station list to match model");
                    v.clear();
                }
            });
    }

    fn update_track_info(&mut self) {
        trace!("Updating track info box...");
        let model = self.model.borrow_mut();
        let (song_name, artist_name, album_name, song_rating) = model
            .playing()
            .map(|t| (t.song_name, t.artist_name, t.album_name, t.song_rating))
            .unwrap_or_default();
        self.siv.call_on_name("title", |v: &mut TextView| {
            debug!("Playing title {} ({})", song_name, song_rating);
            let mut title = song_name.clone();
            if song_rating > 0 {
                title.push(' ');
                title.push_str(labels::LABEL_THUMBS_UP);
            }
            v.set_content(title);
        });
        self.siv.call_on_name("artist", |v: &mut TextView| {
            debug!("Playing artist {}", artist_name);
            v.set_content(artist_name);
        });
        self.siv.call_on_name("album", |v: &mut TextView| {
            debug!("Playing album {}", album_name);
            v.set_content(album_name);
        });
    }

    fn update_volume(&mut self) {
        trace!("Updating volume...");
        let model = self.model.borrow_mut();
        self.siv.call_on_name("volume", |v: &mut SliderView| {
            let volume = ((model.volume() * 10.0).round() as usize).min(10).max(0);
            trace!(
                "Converted model volume from {:.2} to {}",
                model.volume(),
                volume
            );
            v.set_value(volume);
        });
    }

    fn update_playback_state(&mut self) {
        trace!("Updating track info box title...");
        let model = self.model.borrow_mut();
        self.siv
            .call_on_name("playing", |v: &mut Panel<LinearLayout>| {
                if model.playing().is_some() {
                    let playpause = if model.paused() { "Paused" } else { "Play" };
                    let total_elapsed = model.elapsed().as_secs();
                    let elapsed_minutes = total_elapsed / 60;
                    let elapsed_seconds = total_elapsed % 60;
                    let total_duration = model.duration().as_secs();
                    let duration_minutes = total_duration / 60;
                    let duration_seconds = total_duration % 60;
                    let text = if total_duration > 0 {
                        format!(
                            "{:<6} [{:>2}:{:02}/{:>2}:{:02}]",
                            playpause,
                            elapsed_minutes,
                            elapsed_seconds,
                            duration_minutes,
                            duration_seconds
                        )
                    } else {
                        format!(
                            "{:<6} [{:>2}:{:02}]",
                            playpause, elapsed_minutes, elapsed_seconds
                        )
                    };
                    trace!("track is {}", text);
                    v.set_title(text);
                } else if model.ready() {
                    trace!("Playing panel title: waiting on playlist");
                    v.set_title("Waiting on playlist");
                } else if model.tuned().is_some() {
                    trace!("Playing panel title: tuned to station");
                    v.set_title("Tuned to station");
                } else if model.connected() {
                    trace!("Playing panel title: connected");
                    v.set_title("Connected");
                } else {
                    trace!("Playing panel title: disconnected");
                    v.set_title("Disconnected");
                }
            });
    }

    fn update_connected(&mut self) {
        if !self.model.borrow().connected() {
            trace!("Not connected. Not updating UI widgets that reflect connection status.");
            return;
        }

        self.update_stations();
        self.update_track_info();
        self.update_volume();
        self.update_playback_state();
    }

    fn update_connection(&mut self) {
        let connected = {
            let mut model = self.model.borrow_mut();

            // Expired connections already have all necessary credentials,
            // and only need that we try to connect.
            model.connect();

            model.connected()
        };
        let login_prompt_active = self.siv.find_name::<EditView>("username").is_some();
        match (connected, login_prompt_active) {
            (true, true) => {
                debug!("Login prompt active, but we have a valid connection.");
                self.siv.pop_layer();
            }
            (true, false) => {
                // Connection is valid, and the login prompt is disabled
            }
            (false, true) => {
                // No connection, and the login prompt is already visible
            }
            (false, false) => {
                trace!("Activating login dialog");

                if let Some(dialog) = dialogs::login_dialog(self.model.clone()) {
                    self.siv.add_layer(dialog);
                }
            }
        }

        log::debug!("model update reported state change");
        self.update_connected();
    }

    pub(crate) fn run(&mut self) {
        // When idle, how long to sleep between checking for input
        let input_polling_frequency = Duration::from_millis(100);
        // Make sure the UI doesn't go more than 0.5s between updates
        // so that track playtime gets updated consistently
        let heartbeat_frequency = Duration::from_millis(500);

        // reference time for measuring heartbeat
        let mut timeout = Instant::now();
        // something changed state, drive updates to UI and model
        let mut dirty = true;

        // `refresh()` needs to be called before the first `step()` or
        // else the `callbacks::scale_ui()` callback will be told to
        // scale for a window size of 0,0
        self.siv.refresh();

        while !self.model.borrow().quitting() {
            if dirty {
                dirty = self.siv.step();
                self.siv.refresh();
            } else {
                // Nothing has happened, so we'll sleep in increments to wait
                // out the heartbeat timer.
                // But we'll check every once in awhile to see if there's input
                // we should handle, and if so, we'll break out and handle it.
                while timeout.elapsed() <= heartbeat_frequency {
                    let heartbeat_remaining = heartbeat_frequency - timeout.elapsed();
                    std::thread::sleep(input_polling_frequency.min(heartbeat_remaining));
                    if self.siv.process_events() {
                        self.siv.post_events(true);
                        self.siv.refresh();
                        break;
                    }
                }
            }

            // Update the model state, then if the model yielded an event,
            // refresh all the controls
            // If the model state didn't change, and the heartbeat timer
            // expired, update only the playback state controls to update the
            // track elapsed timer.
            if self.model.borrow_mut().update() {
                self.update_connection();
                dirty = true;
                timeout = Instant::now();
            } else if timeout.elapsed() > heartbeat_frequency {
                log::debug!("timer expired with no events, drive a playback state update");
                self.update_playback_state();
                dirty = true;
                timeout = Instant::now();
            }
        }
    }
}
