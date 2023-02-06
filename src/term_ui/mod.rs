use std::time::Duration;
use std::{cell::RefCell, rc::Rc};

use anyhow::Result;
use cursive::views::{EditView, LinearLayout, Panel, SelectView, SliderView, TextView};
use cursive::{CursiveRunnable, CursiveRunner};
use log::{debug, trace};

use crate::config::Config;
use crate::track::Track;
use crate::{messages, messages::StopReason};

mod callbacks;
mod dialogs;

#[cfg(feature = "emoji_labels")]
mod labels {
    pub(crate) const LABEL_PLAY_PAUSE: &str = "⏯️ ";
    pub(crate) const LABEL_SKIP: &str = "⏩";
    pub(crate) const LABEL_THUMBS_UP: &str = "👍";
    pub(crate) const LABEL_THUMBS_DOWN: &str = "👎";
}
#[cfg(not(feature = "emoji_labels"))]
mod labels {
    pub(crate) const LABEL_PLAY_PAUSE: &str = "Play/Pause";
    pub(crate) const LABEL_SKIP: &str = "Skip";
    pub(crate) const LABEL_THUMBS_UP: &str = "|+|";
    pub(crate) const LABEL_THUMBS_DOWN: &str = "|-|";
}

#[derive(Debug, Clone)]
pub(crate) struct TerminalContext {
    config: Rc<RefCell<Config>>,
    publisher: async_broadcast::Sender<messages::Request>,
}

pub(crate) struct Terminal {
    siv: CursiveRunner<CursiveRunnable>,
    subscriber: async_broadcast::Receiver<messages::Notification>,
    context: TerminalContext,
}

impl Terminal {
    pub(crate) fn new(
        config: Rc<RefCell<Config>>,
        subscriber: async_broadcast::Receiver<messages::Notification>,
        publisher: async_broadcast::Sender<messages::Request>,
    ) -> Self {
        let mut siv = cursive::crossterm().into_runner();
        let context = TerminalContext { config, publisher };
        siv.set_user_data(context.clone());
        siv.set_fps(5);
        siv.set_window_title("panharmonicon");
        let mut term = Self {
            siv,
            subscriber,
            context,
        };
        term.initialize();
        term
    }

    pub(crate) fn initialize(&mut self) {
        self.init_key_mappings();
        self.init_theme();
        self.init_playback();
        self.siv.refresh();
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

    fn added_station(&mut self, name: String, id: String) {
        trace!("Adding station {}[{}] to list...", name, id);
        self.siv
            .call_on_name("stations", |v: &mut SelectView<String>| {
                // If we were disconnected, the station list contains one entry: "No Stations"
                if v.len() == 1
                    && v.get_item(0).map(|(a, _)| a).unwrap_or_default() == "No Stations"
                {
                    v.clear();
                }

                // If the station list is empty, initialize it with an empty entry
                // for no station selected
                if v.is_empty() {
                    v.add_item("", String::new());
                    v.set_selection(0);
                }

                // New stations are inserted in their sorted position, but always
                // after the first entry
                let insert_pt = {
                    let mut station_iter = v.iter().enumerate();
                    // We want to preserve the first element as a "no station selected" item
                    station_iter.next();
                    station_iter
                        .find(|(_, (lbl, _))| lbl > &name.as_str())
                        .map(|f| f.0)
                };

                // Insert the new station into its correct location in the list
                // If no insertion point found, append it to the list
                if let Some(insert_pt) = insert_pt {
                    v.insert_item(insert_pt, name, id);
                } else {
                    v.add_item(name, id);
                }
            });
    }

    fn tuned_station(&mut self, id: String) {
        trace!("Tuning station {}...", id);
        self.siv
            .call_on_name("stations", |v: &mut SelectView<String>| {
                let opt_idx = v
                    .iter()
                    .enumerate()
                    .find(|(_, (_, st_id))| *st_id == &id)
                    .map(|(i, _)| i);
                if let Some(idx) = opt_idx {
                    v.set_selection(idx);
                } else {
                    v.set_selection(0);
                }
            });
    }

    fn playing_track(&mut self, track: Track) {
        trace!("Updating track info box...");
        let Track {
            song_name,
            artist_name,
            album_name,
            song_rating,
            ..
        } = track;
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

    fn rated_track(&mut self, rating: u32) {
        trace!("Updating track rating...");
        self.siv.call_on_name("title", |v: &mut TextView| {
            let mut title = v
                .get_content()
                .source()
                .trim_end_matches(labels::LABEL_THUMBS_UP)
                .trim_end()
                .to_string();

            debug!("Rating title {}", title);

            if rating > 0 {
                title.push(' ');
                title.push_str(labels::LABEL_THUMBS_UP);
            }
            v.set_content(title);
        });
    }

    fn unrated_track(&mut self) {
        trace!("Removing track rating...");
        self.siv.call_on_name("title", |v: &mut TextView| {
            let title = v
                .get_content()
                .source()
                .trim_end_matches(labels::LABEL_THUMBS_UP)
                .trim_end()
                .to_string();

            v.set_content(title);
        });
    }

    fn next_track(&mut self, track: Track) {
        trace!("TODO: UI for displaying next track ({})", track.song_name);
    }

    fn update_playing(&mut self, elapsed: Duration, duration: Duration, paused: bool) {
        trace!("Updating track duration...");
        self.siv
            .call_on_name("playing", |v: &mut Panel<LinearLayout>| {
                let playpause = if paused { "Paused" } else { "Play" };
                let total_elapsed = elapsed.as_secs();
                let elapsed_minutes = total_elapsed / 60;
                let elapsed_seconds = total_elapsed % 60;
                let total_duration = duration.as_secs();
                let duration_minutes = total_duration / 60;
                let duration_seconds = total_duration % 60;
                let text = if total_duration > 0 {
                    format!(
                        "{playpause:<6} [{elapsed_minutes:>2}:{elapsed_seconds:02}/{duration_minutes:>2}:{duration_seconds:02}]"
                    )
                } else {
                    format!(
                        "{playpause:<6} [{elapsed_minutes:>2}:{elapsed_seconds:02}]"
                    )
                };
                trace!("Playing panel title: {}", text);
                v.set_title(text);
            });
    }

    fn update_state_disconnected(&mut self) {
        self.siv
            .call_on_name("playing", |v: &mut Panel<LinearLayout>| {
                trace!("Playing panel title: disconnected");
                v.set_title("Disconnected");
            });
        if self.siv.find_name::<EditView>("username").is_none()
            && self
                .context
                .config
                .borrow()
                .login_credentials()
                .get()
                .is_none()
        {
            trace!("Activating login dialog");

            if let Some(dialog) = dialogs::login_dialog(self.context.config.clone()) {
                self.siv.add_layer(dialog);
            }
        }
    }

    fn update_state_stopped(&mut self, reason: StopReason) {
        self.siv
            .call_on_name("playing", |v: &mut Panel<LinearLayout>| {
                trace!("Playing panel title: stopped");
                v.set_title(reason.to_string());
            });
        if self.siv.find_name::<EditView>("username").is_some() {
            debug!("Login prompt active, but we have a valid connection.");
            trace!("Deactivating login dialog");
            self.siv.pop_layer();
        }
    }

    fn update_volume(&mut self, volume: f32) {
        trace!("Updating volume...");
        self.siv.call_on_name("volume", |v: &mut SliderView| {
            let volume_adj = ((volume * 10.0).round() as usize).min(10).max(0);
            trace!(
                "Converted model volume from {:.2} to {}",
                volume,
                volume_adj
            );
            v.set_value(volume_adj);
        });
    }

    pub(crate) async fn update(&mut self) -> Result<bool> {
        let mut dirty = false;
        trace!("checking for player notifications...");
        while let Ok(Ok(message)) = tokio::time::timeout(
            std::time::Duration::from_millis(250),
            self.subscriber.recv(),
        )
        .await
        {
            match message {
                messages::Notification::Connected => {
                    self.update_state_stopped(StopReason::Initializing)
                }
                messages::Notification::Disconnected => self.update_state_disconnected(),
                messages::Notification::AddStation(name, id) => self.added_station(name, id),
                messages::Notification::Tuned(name) => self.tuned_station(name),
                messages::Notification::Starting(track) => self.playing_track(track),
                messages::Notification::Rated(val) => self.rated_track(val),
                messages::Notification::Unrated => self.unrated_track(),
                messages::Notification::Next(track) => self.next_track(track),
                messages::Notification::Playing(elapsed, duration) => {
                    self.update_playing(elapsed, duration, false)
                }
                messages::Notification::Volume(v) => self.update_volume(v),
                messages::Notification::Paused(elapsed, duration) => {
                    self.update_playing(elapsed, duration, true)
                }
                messages::Notification::Stopped(r) => self.update_state_stopped(r),
                messages::Notification::PreCaching(_) => (),
                messages::Notification::Muted => (),
                messages::Notification::Unmuted => (),
                messages::Notification::Quit => (),
            }
            dirty = true;
        }
        trace!("forcing ui update");
        dirty |= self.siv.step();
        if dirty {
            self.siv.refresh();
        }
        Ok(dirty)
    }
}
