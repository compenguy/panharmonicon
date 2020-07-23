use std::time::{Duration, Instant};
use std::{cell::RefCell, rc::Rc};

use cursive::align::HAlign;
use cursive::views::{
    Button, DummyView, HideableView, LinearLayout, Panel, SelectView, SliderView, TextView,
};
use cursive::{Cursive, CursiveExt};
use log::{debug, trace};
// Traits pulled in to add methods to types
use cursive::view::{Nameable, Resizable};

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
    siv: Cursive,
}

impl Terminal {
    pub(crate) fn new(config: Rc<RefCell<Config>>) -> Self {
        let model = Rc::new(RefCell::new(Model::new(config)));
        let mut siv = Cursive::crossterm().expect("Failed to initialize terminal");
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
        let stations = LinearLayout::horizontal()
            .child(TextView::new("Station:"))
            .child(
                SelectView::<String>::new()
                    .popup()
                    .item("No Stations", String::from(""))
                    .selected(0)
                    .on_submit(|s: &mut Cursive, item: &String| {
                        s.with_user_data(|m: &mut Rc<RefCell<Model>>| {
                            trace!("Tuning to station {}", item.clone());
                            m.borrow_mut().tune(item.clone())
                        });
                    })
                    .with_name("stations")
                    .fixed_height(1),
            );

        let controls_bar = LinearLayout::vertical()
            .child(
                LinearLayout::horizontal()
                    .child(TextView::new("Volume").fixed_width(7))
                    .child(
                        SliderView::horizontal(11)
                            .on_change(|s, v| {
                                let new_volume: f32 = ((v as f32) / 10.0).min(1.0f32).max(0.0f32);
                                trace!(
                                    "Submitting updated volume from slider: {} ({:.2})",
                                    v,
                                    new_volume
                                );
                                s.with_user_data(|m: &mut Rc<RefCell<Model>>| {
                                    m.borrow_mut().set_volume(new_volume)
                                });
                            })
                            .with_name("volume"),
                    ),
            )
            .child(
                LinearLayout::horizontal()
                    .child(Button::new(
                        labels::LABEL_PLAY_PAUSE,
                        callbacks::toggle_pause,
                    ))
                    .child(Button::new(labels::LABEL_SKIP, callbacks::stop)),
            )
            .child(
                LinearLayout::horizontal()
                    .child(Button::new(
                        labels::LABEL_THUMBS_UP,
                        callbacks::rate_track_up,
                    ))
                    .child(Button::new(
                        labels::LABEL_THUMBS_DOWN,
                        callbacks::rate_track_down,
                    ))
                    .child(Button::new(labels::LABEL_TIRED, callbacks::sleep_track)),
            );
        let playing = Panel::new(
            LinearLayout::horizontal()
                .child(
                    LinearLayout::vertical()
                        .child(
                            LinearLayout::horizontal()
                                .child(TextView::new("Title").fixed_width(7))
                                .child(TextView::empty().with_name("title")),
                        )
                        .child(
                            LinearLayout::horizontal()
                                .child(TextView::new("Artist").fixed_width(7))
                                .child(TextView::empty().with_name("artist")),
                        )
                        .child(
                            LinearLayout::horizontal()
                                .child(TextView::new("Album").fixed_width(7))
                                .child(TextView::empty().with_name("album")),
                        )
                        .max_height(3)
                        .full_width(),
                )
                .child(DummyView.min_width(4))
                .child(controls_bar),
        )
        .title("Disconnected")
        .title_position(HAlign::Left)
        .with_name("playing");

        let layout = LinearLayout::vertical()
            .child(HideableView::new(DummyView.full_height()).with_name("spacer_hideable"))
            .child(HideableView::new(stations).with_name("stations_hideable"))
            .child(playing);
        self.siv.add_fullscreen_layer(layout);

        callbacks::ui_scale(&mut self.siv);
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
                title.push_str(" ");
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

    pub(crate) fn run(&mut self) {
        self.siv.set_fps(1);
        self.siv.refresh();
        let heartbeat_frequency = Duration::from_millis(500);
        let mut timeout = Instant::now();
        while !self.model.borrow().quitting() {
            self.siv.step();

            // Drive the UI state, then if the UI yielded an event, drive the
            // model state and refresh all the controls, otherwise, do a
            // heartbeat update of the playback state.
            if self.model.borrow_mut().update() {
                if let Some(dialog) = dialogs::login_dialog(&mut self.siv, self.model.clone()) {
                    self.siv.add_layer(dialog);
                }
                self.update_connected();

                self.siv.refresh();
                timeout = Instant::now();
            } else if timeout.elapsed() > heartbeat_frequency {
                self.update_playback_state();

                self.siv.refresh();
                timeout = Instant::now();
            }
        }
    }
}
