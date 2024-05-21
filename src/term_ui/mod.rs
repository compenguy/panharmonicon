use std::time::Duration;
use std::{cell::RefCell, rc::Rc};

use anyhow::Result;
use cursive::views::{EditView, LinearLayout, Panel, SelectView, SliderView, TextView};
use cursive::{theme::ColorStyle, utils::markup::StyledString};
use cursive::{CursiveRunnable, CursiveRunner};
use log::{debug, trace};

use crate::config::Config;
use crate::messages::{Request, State, StopReason};
use crate::model::{RequestSender, StateReceiver};
use crate::track::Track;

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
    request_sender: RequestSender,
}

impl TerminalContext {
    fn publish_request(&mut self, request: Request) -> Result<()> {
        self.request_sender.send(request)?;
        Ok(())
    }
}

pub(crate) struct Terminal {
    siv: CursiveRunner<CursiveRunnable>,
    context: TerminalContext,
    state_receiver: StateReceiver,
    active_track: Option<Track>,
    dirty: bool,
}

impl Terminal {
    pub(crate) fn new(
        config: Rc<RefCell<Config>>,
        state_receiver: StateReceiver,
        request_sender: RequestSender,
    ) -> Self {
        let mut siv = cursive::crossterm().into_runner();
        let context = TerminalContext {
            config,
            request_sender,
        };
        siv.set_user_data(context.clone());
        siv.set_fps(5);
        siv.set_window_title("panharmonicon");
        let mut term = Self {
            siv,
            context,
            state_receiver,
            active_track: None,
            dirty: true,
        };
        term.initialize();
        term
    }

    pub(crate) fn initialize(&mut self) {
        self.init_key_mappings();
        self.init_theme();
        self.init_playback();
        self.drive_ui();
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
        self.dirty |= true;
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
        self.dirty |= true;
    }

    fn playing_track(&mut self, track: Track) {
        trace!("Updating track info box...");
        self.active_track = Some(track.clone());
        let Track {
            title,
            artist_name,
            album_name,
            song_rating,
            ..
        } = track;
        self.siv.call_on_name("title", |v: &mut TextView| {
            debug!("Playing title {} ({})", title, song_rating);
            let mut title = title.clone();
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
        self.update_playing(Duration::default(), false);
        self.dirty |= true;
    }

    fn next_track(&mut self, track: Option<Track>) {
        trace!("Updating next track...");
        let styled_text = if let Some(Track {
            title, artist_name, ..
        }) = &track
        {
            let mut styled = StyledString::new();
            styled.append_plain(title);
            styled.append_styled(" by ", ColorStyle::secondary());
            styled.append_plain(artist_name);
            styled
        } else {
            StyledString::plain("...")
        };

        self.siv.call_on_name("next_up", |v: &mut TextView| {
            debug!("Next up: {:?}", track);
            v.set_content(styled_text);
        });
        self.dirty |= true;
    }

    fn update_playing(&mut self, elapsed: Duration, paused: bool) {
        trace!("Updating track duration...");
        let total_duration = self
            .active_track
            .as_ref()
            .map(|t| t.track_length.as_secs())
            .unwrap_or(0);
        self.siv
            .call_on_name("playing", |v: &mut Panel<LinearLayout>| {
                let playpause = if paused { "Paused" } else { "Play" };
                let total_elapsed = elapsed.as_secs();
                let elapsed_minutes = total_elapsed / 60;
                let elapsed_seconds = total_elapsed % 60;
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
        self.dirty |= true;
    }

    fn update_state_disconnected(&mut self, message: Option<String>) {
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

            if let Some(dialog) = dialogs::login_dialog(self.context.config.clone(), message) {
                self.siv.add_layer(dialog);
            }
        }
        self.dirty |= true;
    }

    fn update_state_stopped(&mut self, reason: StopReason) {
        self.active_track = None;
        self.siv
            .call_on_name("playing", |v: &mut Panel<LinearLayout>| {
                trace!("Playing panel title: stopped");
                v.set_title(format!("Stopped ({reason})"));
            });
        if self.siv.find_name::<EditView>("username").is_some() {
            debug!("Login prompt active, but we have a valid connection.");
            trace!("Deactivating login dialog");
            self.siv.pop_layer();
        }
        self.dirty |= true;
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
        self.dirty |= true;
    }

    async fn process_messages(&mut self) -> Result<()> {
        trace!("checking for player notifications...");
        while let Ok(message) = self.state_receiver.try_recv() {
            match message {
                State::AuthFailed(r) => self.update_state_disconnected(Some(r.to_string())),
                State::Connected => self.update_state_stopped(StopReason::Initializing),
                State::Disconnected => self.update_state_disconnected(None),
                State::AddStation(name, id) => self.added_station(name, id),
                State::Tuned(name) => self.tuned_station(name),
                State::TrackStarting(track) => self.playing_track(track),
                State::Next(track) => self.next_track(track),
                State::Playing(elapsed) => self.update_playing(elapsed, false),
                State::Volume(v) => self.update_volume(v),
                State::Paused(elapsed) => self.update_playing(elapsed, true),
                State::Stopped(r) => self.update_state_stopped(r),
                State::TrackCaching(_) => (),
                State::Muted => (),
                State::Unmuted => (),
                State::Quit => (),
            }
        }
        Ok(())
    }

    fn drive_ui(&mut self) -> bool {
        self.dirty |= self.siv.step();
        if self.dirty {
            trace!("forcing ui update");
            self.siv.refresh();
            self.dirty = false;
            true
        } else {
            false
        }
    }

    pub(crate) async fn update(&mut self) -> Result<bool> {
        self.process_messages().await?;
        let dirty = self.drive_ui();
        Ok(dirty)
    }
}
