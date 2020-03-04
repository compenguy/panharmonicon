use std::time::{Duration, Instant};
use std::{cell::RefCell, rc::Rc};

use cursive::align::HAlign;
use cursive::views::{
    Button, Dialog, DummyView, EditView, HideableView, LinearLayout, PaddedView, Panel, SelectView,
    SliderView, TextView,
};
use cursive::Cursive;
use log::{debug, trace};
// Traits pulled in to add methods to types
use cursive::view::{Nameable, Resizable};

use crate::config::{Config, Credentials};
use crate::errors::Result;
use crate::model::Model;
use crate::model::{AudioMediator, PlaybackMediator, StateMediator, StationMediator};

#[cfg(emoji_labels)]
mod labels {
    pub(crate) const LABEL_PLAY_PAUSE: &str = "⏯️ ";
    pub(crate) const LABEL_SKIP: &str = "⏩";
    pub(crate) const LABEL_THUMBS_UP: &str = "👍";
    pub(crate) const LABEL_THUMBS_DOWN: &str = "👎";
    pub(crate) const LABEL_TIRED: &str = "💤";
}
#[cfg(not(emoji_labels))]
mod labels {
    pub(crate) const LABEL_PLAY_PAUSE: &str = "Play/Pause";
    pub(crate) const LABEL_SKIP: &str = "Skip";
    pub(crate) const LABEL_THUMBS_UP: &str = "|+|";
    pub(crate) const LABEL_THUMBS_DOWN: &str = "|-|";
    pub(crate) const LABEL_TIRED: &str = ".zZ";
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum Store {
    Keyring,
    ConfigFile,
    Session,
}

impl From<Credentials> for Store {
    fn from(cred: Credentials) -> Self {
        match cred {
            Credentials::Keyring(_) => Self::Keyring,
            Credentials::ConfigFile(_, _) => Self::ConfigFile,
            Credentials::Session(_, _) => Self::Session,
            Credentials::Invalid(_) => Self::Session,
        }
    }
}

impl Default for Store {
    fn default() -> Self {
        Self::Keyring
    }
}

pub(crate) struct Terminal {
    config: Rc<RefCell<Config>>,
    model: Rc<RefCell<Model>>,
    siv: Cursive,
}

impl Terminal {
    pub(crate) fn new(config: Rc<RefCell<Config>>) -> Self {
        let model = Rc::new(RefCell::new(Model::new(config.clone())));
        let mut siv = Cursive::crossterm().expect("Failed to initialize terminal");
        siv.set_user_data(model.clone());
        let mut term = Self { config, model, siv };
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
        self.siv.add_global_callback('q', siv_cb::quit);
        self.siv.add_global_callback('.', siv_cb::pause);
        self.siv.add_global_callback('>', siv_cb::unpause);
        self.siv.add_global_callback('p', siv_cb::toggle_pause);
        self.siv.add_global_callback('(', siv_cb::decrease_volume);
        self.siv.add_global_callback(')', siv_cb::increase_volume);
        self.siv.add_global_callback('n', siv_cb::stop);
        self.siv.add_global_callback('t', siv_cb::sleep_track);
        self.siv.add_global_callback('+', siv_cb::rate_track_up);
        self.siv.add_global_callback('-', siv_cb::rate_track_down);
        self.siv.add_global_callback('=', siv_cb::clear_rating);
    }

    fn init_theme(&mut self) {
        self.siv
            .load_toml(include_str!("../theme.toml"))
            .expect("Error loading theme toml file");
        // TODO: Allow loading user-provided theme files at run-time
    }

    fn show_login(&mut self) {
        let mut model = self.model.borrow_mut();
        // Expired connections already have all necessary credentials,
        // and only need that we try to connect.
        model.connect();
        let connected = model.connected();
        let login_prompt_active = self.siv.find_name::<EditView>("username").is_some();
        match (connected, login_prompt_active) {
            (true, true) => {
                debug!("Login prompt active, but we have a valid connection.");
                self.siv.pop_layer();
                return;
            }
            (true, false) => {
                // This is the expected case, generally - logged in and not
                // showing the login dialog
                return;
            }
            (false, true) => {
                // Not connected, and we've already displayed the login dialog
                // nothing left to do until user clicks "Connect" button
                return;
            }
            (false, false) => {
                trace!("Activating login dialog");
            }
        }

        let credentials = self.config.borrow().login_credentials().clone();
        let username = credentials.username().unwrap_or_default();
        let password = credentials.password().ok().flatten().unwrap_or_default();
        let index = match Store::from(credentials) {
            Store::Keyring => 0,
            Store::ConfigFile => 1,
            Store::Session => 2,
        };
        let dialog = Dialog::around(
            LinearLayout::vertical()
                .child(
                    LinearLayout::horizontal()
                        .child(TextView::new("Username:"))
                        .child(PaddedView::lrtb(
                            1,
                            1,
                            0,
                            0,
                            EditView::new()
                                .content(username)
                                .with_name("username")
                                .fixed_width(24),
                        )),
                )
                .child(
                    LinearLayout::horizontal()
                        .child(TextView::new("Password:"))
                        .child(PaddedView::lrtb(
                            1,
                            1,
                            0,
                            0,
                            EditView::new()
                                .content(password)
                                .secret()
                                .with_name("password")
                                .fixed_width(24),
                        )),
                )
                .child(
                    LinearLayout::horizontal()
                        .child(TextView::new("Store credentials in:"))
                        .child(PaddedView::lrtb(
                            1,
                            1,
                            0,
                            0,
                            SelectView::<Store>::new()
                                .popup()
                                .item("User Keyring", Store::Keyring)
                                .item("Config File", Store::ConfigFile)
                                .item("Don't Store", Store::Session)
                                .selected(index)
                                .with_name("store"),
                        )),
                ),
        )
        .button("Connect", siv_cb::connect_button)
        .title("Pandora Login");
        self.siv.add_layer(dialog);
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
                    .child(Button::new(labels::LABEL_PLAY_PAUSE, siv_cb::toggle_pause))
                    .child(Button::new(labels::LABEL_SKIP, siv_cb::stop)),
            )
            .child(
                LinearLayout::horizontal()
                    .child(Button::new(labels::LABEL_THUMBS_UP, siv_cb::rate_track_up))
                    .child(Button::new(labels::LABEL_THUMBS_DOWN, siv_cb::rate_track_down))
                    .child(Button::new(labels::LABEL_TIRED, siv_cb::sleep_track)),
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
            .child(
                HideableView::new(DummyView.full_height())
                    .with_name("spacer_hideable")
            )
            .child(HideableView::new(stations).with_name("stations_hideable"))
            .child(playing);
        self.siv.add_fullscreen_layer(layout);

        siv_cb::ui_scale(&mut self.siv);
        // Catch screen resize requests, and hide/show appropriate controls to
        // fit the most important parts of the interface to the terminal size.
        self.siv
            .set_on_pre_event(cursive::event::Event::WindowResize, siv_cb::ui_scale);
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

    pub(crate) fn run(&mut self) -> Result<()> {
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
                self.show_login();
                self.update_connected();

                self.siv.refresh();
                timeout = Instant::now();
            } else if timeout.elapsed() > heartbeat_frequency {
                self.update_playback_state();

                self.siv.refresh();
                timeout = Instant::now();
            }
        }
        Ok(())
    }
}

mod siv_cb {
    use super::model_cb;
    use super::Store;

    use crate::config::PartialConfig;
    use crate::model::Model;
    use crate::model::StateMediator;
    use cursive::views::{DummyView, EditView, HideableView, LinearLayout, ResizedView, SelectView};
    use cursive::Cursive;
    use log::{error, trace};
    use std::{cell::RefCell, rc::Rc};

    pub(crate) fn quit(s: &mut Cursive) {
        s.with_user_data(model_cb::quit);
    }
    pub(crate) fn pause(s: &mut Cursive) {
        s.with_user_data(model_cb::pause);
    }
    pub(crate) fn unpause(s: &mut Cursive) {
        s.with_user_data(model_cb::unpause);
    }
    pub(crate) fn toggle_pause(s: &mut Cursive) {
        s.with_user_data(model_cb::toggle_pause);
    }
    pub(crate) fn decrease_volume(s: &mut Cursive) {
        s.with_user_data(model_cb::decrease_volume);
    }
    pub(crate) fn increase_volume(s: &mut Cursive) {
        s.with_user_data(model_cb::increase_volume);
    }
    pub(crate) fn stop(s: &mut Cursive) {
        s.with_user_data(model_cb::stop);
    }
    pub(crate) fn sleep_track(s: &mut Cursive) {
        s.with_user_data(model_cb::sleep_track);
    }
    pub(crate) fn rate_track_up(s: &mut Cursive) {
        s.with_user_data(model_cb::rate_track_up);
    }
    pub(crate) fn rate_track_down(s: &mut Cursive) {
        s.with_user_data(model_cb::rate_track_down);
    }
    pub(crate) fn clear_rating(s: &mut Cursive) {
        s.with_user_data(model_cb::clear_rating);
    }

    pub(crate) fn connect_button(s: &mut Cursive) {
        let username: Option<String> =
            s.call_on_name("username", |v: &mut EditView| v.get_content().to_string());
        let password: Option<String> =
            s.call_on_name("password", |v: &mut EditView| v.get_content().to_string());
        let store: Option<Store> = s
            .call_on_name("store", |v: &mut SelectView<Store>| {
                v.selection().map(|s| (*s).clone())
            })
            .flatten();
        s.with_user_data(|m: &mut Rc<RefCell<Model>>| {
            let mut model = m.borrow_mut();
            let config = model.config();
            let new_cred = match store.unwrap_or_default() {
                Store::Keyring => config
                    .borrow()
                    .login_credentials()
                    .as_keyring()
                    .expect("Error updating keyring with password"),
                Store::ConfigFile => config.borrow().login_credentials().as_configfile(),
                Store::Session => config.borrow().login_credentials().as_session(),
            };
            let new_cred = username
                .map(|u| new_cred.update_username(&u))
                .unwrap_or(new_cred);
            let new_cred = password
                .map(|u| new_cred.update_password(&u))
                .unwrap_or(Ok(new_cred));
            match new_cred {
                Ok(c) => {
                    let result = config
                        .borrow_mut()
                        .update_from(&PartialConfig::default().login(c));
                    if let Err(e) = result {
                        error!("Failed while updating configuration settings: {:?}", e);
                    } else {
                        model.connect();
                    }
                }
                Err(e) => {
                    error!("Failed while updating password: {:?}", e);
                }
            }
        });
        s.pop_layer();
    }

    pub(crate) fn ui_scale(s: &mut Cursive) {
        let height = s.screen_size().y;
        trace!("Window resize. New height: {}", height);
        match height {
            h if h <= 5 => {
                trace!("Hiding spacer ({:?})", s.debug_name("spacer_hideable"));
                s.call_on_name("spacer_hideable", |v: &mut HideableView<ResizedView<DummyView>>| {
                    v.hide();
                    trace!("Spaced hidden.")
                });
                trace!("Hiding stations ({:?})", s.debug_name("stations_hideable"));
                s.call_on_name(
                    "stations_hideable",
                    |v: &mut HideableView<LinearLayout>| {
                        v.hide();
                        trace!("Stations hidden.")
                    },
                );
            }
            h if h <= 6 => {
                trace!("Hiding spacer ({:?})", s.debug_name("spacer_hideable"));
                s.call_on_name("spacer_hideable", |v: &mut HideableView<ResizedView<DummyView>>| {
                    v.hide();
                    trace!("Spacer hidden.")
                });
                trace!("Showing stations ({:?})", s.debug_name("stations_hideable"));
                s.call_on_name(
                    "stations_hideable",
                    |v: &mut HideableView<LinearLayout>| {
                        v.unhide();
                        trace!("Stations unhidden.")
                    },
                );
            }
            _ => {
                trace!("Showing spacer ({:?})", s.debug_name("spacer_hideable"));
                s.call_on_name("spacer_hideable", |v: &mut HideableView<ResizedView<DummyView>>| {
                    v.unhide();
                    trace!("Spacer unhidden.")
                });
                trace!("Showing stations ({:?})", s.debug_name("stations_hideable"));
                s.call_on_name(
                    "stations_hideable",
                    |v: &mut HideableView<LinearLayout>| {
                        v.unhide();
                        trace!("Stations unhidden.")
                    },
                );
            }
        }
    }
}

mod model_cb {
    use crate::model::Model;
    use crate::model::{AudioMediator, PlaybackMediator, StateMediator};
    use std::{cell::RefCell, rc::Rc};

    pub(crate) fn quit(m: &mut Rc<RefCell<Model>>) {
        m.borrow_mut().quit();
    }
    pub(crate) fn pause(m: &mut Rc<RefCell<Model>>) {
        m.borrow_mut().pause();
    }
    pub(crate) fn unpause(m: &mut Rc<RefCell<Model>>) {
        m.borrow_mut().unpause();
    }
    pub(crate) fn toggle_pause(m: &mut Rc<RefCell<Model>>) {
        m.borrow_mut().toggle_pause();
    }
    pub(crate) fn decrease_volume(m: &mut Rc<RefCell<Model>>) {
        m.borrow_mut().decrease_volume();
    }
    pub(crate) fn increase_volume(m: &mut Rc<RefCell<Model>>) {
        m.borrow_mut().increase_volume();
    }
    pub(crate) fn stop(m: &mut Rc<RefCell<Model>>) {
        m.borrow_mut().stop();
    }
    pub(crate) fn sleep_track(m: &mut Rc<RefCell<Model>>) {
        m.borrow_mut().sleep_track();
    }
    pub(crate) fn rate_track_up(m: &mut Rc<RefCell<Model>>) {
        m.borrow_mut().rate_track(Some(true));
    }
    pub(crate) fn rate_track_down(m: &mut Rc<RefCell<Model>>) {
        let mut model = m.borrow_mut();
        model.rate_track(Some(false));
        model.stop();
    }
    pub(crate) fn clear_rating(m: &mut Rc<RefCell<Model>>) {
        m.borrow_mut().rate_track(None);
    }
}
