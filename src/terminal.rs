use std::{cell::RefCell, rc::Rc};

use cursive::views::{
    Dialog, DummyView, EditView, LinearLayout, Panel, SelectView, SliderView, TextArea, TextView,
};
use cursive::{Cursive, ScreenId};
use log::error;
// Traits pulled in to add methods to types
use cursive::view::{Nameable, Resizable};

use crate::config::{Config, Credentials, PartialConfig};
use crate::errors::Result;
use crate::model::Model;
use crate::model::{PlaybackMediator, StateMediator};

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
    login_screen: ScreenId,
    playback_screen: ScreenId,
}

impl Terminal {
    pub(crate) fn new(config: Rc<RefCell<Config>>) -> Self {
        let model = Rc::new(RefCell::new(Model::new(config.clone())));
        let mut siv = Cursive::crossterm().expect("Failed to initialize terminal");
        siv.set_user_data(model.clone());
        let login_screen = siv.add_screen();
        let playback_screen = siv.add_screen();
        let mut term = Self {
            config,
            model,
            siv,
            login_screen,
            playback_screen,
        };
        term.initialize();
        term
    }

    pub(crate) fn initialize(&mut self) {
        self.init_theme();
        self.init_login();
        self.init_playback();
    }

    fn init_key_mappings(&mut self) {
        // TODO: read key mappings from config
        self.siv.add_global_callback('q', |s| s.quit());
    }

    fn init_theme(&mut self) {
        self.siv
            .load_toml(include_str!("../theme.toml"))
            .expect("Error loading theme toml file");
        // TODO: Allow loading user-provided theme files at run-time
    }

    fn init_login(&mut self) {
        let dialog = Dialog::around(
            LinearLayout::vertical()
                .child(
                    LinearLayout::horizontal()
                        .child(TextView::new("Username:"))
                        .child(EditView::new().with_name("username").fixed_width(20)),
                )
                .child(
                    LinearLayout::horizontal()
                        .child(TextView::new("Password:"))
                        .child(
                            EditView::new()
                                .secret()
                                .with_name("password")
                                .fixed_width(20),
                        ),
                )
                .child(
                    LinearLayout::horizontal()
                        .child(TextView::new("Store credentials in:"))
                        .child(
                            SelectView::<Store>::new()
                                .popup()
                                .item("User Keyring", Store::Keyring)
                                .item("Config File", Store::ConfigFile)
                                .item("Don't Store", Store::Session)
                                .with_name("store"),
                        ),
                ),
        )
        .button("Connect", |s| {
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
                        if let Err(e) = config
                            .borrow_mut()
                            .update_from(&PartialConfig::new_login(c))
                        {
                            error!("Error while updating configuration settings: {:?}", e);
                        } else {
                            model.connect();
                        }
                    }
                    Err(e) => {
                        error!("Error updating password: {:?}", e);
                    }
                }
            });
        })
        .title("Pandora Login");
        self.siv.set_screen(self.login_screen);
        self.siv.screen_mut().add_layer(dialog);
    }

    fn init_playback(&mut self) {
        let stations = LinearLayout::horizontal()
            .child(TextView::new("Station:"))
            .child(
                SelectView::<String>::new()
                    .popup()
                    .on_submit(|s: &mut Cursive, item: &String| {
                        s.with_user_data(|m: &mut Rc<RefCell<Model>>| {
                            m.borrow_mut().tune(item.clone())
                        });
                    })
                    .with_name("stations")
                    .fixed_height(1),
            );
        // connect up on_submit
        let playing = Panel::new(
            LinearLayout::horizontal()
                .child(
                    TextArea::new()
                        .disabled()
                        .fixed_width(50)
                        .fixed_height(3)
                        .with_name("track_info"),
                )
                .child(DummyView)
                .child(TextView::new("Volume:"))
                .child(
                    SliderView::horizontal(11)
                        .on_change(|s, v| {
                            let new_volume = ((v as f32) / 10.0).min(0.0f32).max(1.0f32);
                            s.with_user_data(|m: &mut Rc<RefCell<Model>>| {
                                m.borrow_mut().set_volume(new_volume)
                            });
                        })
                        .with_name("volume"),
                ),
        )
        .title("Disconnected")
        .with_name("playing");

        let layout = LinearLayout::vertical()
            .child(DummyView)
            .child(stations)
            .child(playing);
        self.siv.set_screen(self.playback_screen);
        self.siv.screen_mut().add_layer(layout);
    }

    fn update_connected(&mut self) {
        let model = self.model.borrow_mut();
        if !model.connected() {
            return;
        }
        self.siv
            .call_on_name("stations", |v: &mut SelectView<String>| {
                if v.len() > 0 {
                    return;
                }
                v.add_all(model.station_list().into_iter());
            });

        self.siv.call_on_name("track_info", |v: &mut TextArea| {
            let text = model
                .playing()
                .map(|t| {
                    format!(
                        "{:<8} {:<25}\n{:<8} {:<25}\n{:<8} {:<25}",
                        "Title:",
                        t.song_name,
                        "Artist:",
                        t.artist_name,
                        "Album:",
                        t.album_name
                    )
                })
                .unwrap_or_default();
            v.set_content(text);
        });

        self.siv.call_on_name("volume", |v: &mut SliderView| {
            let volume = ((model.volume() * 10.0).round() as usize).min(0).max(10);
            v.set_value(volume);
        });

        self.siv
            .call_on_name("playing", |v: &mut Panel<LinearLayout>| {
                if model.playing().is_some() {
                    let playpause = if model.paused() { "Play" } else { "Pause" };
                    // TODO: get real values here
                    let elapsed_minutes = 0;
                    let elapsed_seconds = 0;
                    let duration_minutes = 0;
                    let duration_seconds = 0;
                    let text = format!(
                        "{} [{:02}:{:02}/{:02}:{:02}]",
                        playpause,
                        elapsed_minutes,
                        elapsed_seconds,
                        duration_minutes,
                        duration_seconds
                    );
                    v.set_title(text);
                } else if model.ready() {
                    v.set_title("Waiting on playlist");
                } else if model.tuned().is_some() {
                    v.set_title("Tuned to station");
                } else if model.connected() {
                    v.set_title("Connected");
                } else {
                    v.set_title("Disconnected");
                }
            });

        self.siv.set_screen(self.playback_screen);
    }

    fn update_disconnected(&mut self) {
        let model = self.model.borrow_mut();
        if model.connected() {
            return;
        }

        let credentials = self.config.borrow().login_credentials().clone();
        let username = credentials.username().unwrap_or_default();
        let password = credentials.password().ok().flatten().unwrap_or_default();
        self.siv.call_on_name("username", |v: &mut EditView| {
            v.set_content(username);
        });
        self.siv.call_on_name("password", |v: &mut EditView| {
            v.set_content(password);
        });
        self.siv.call_on_name("store", |v: &mut SelectView<Store>| {
            let index = match Store::from(credentials) {
                Store::Keyring => 0,
                Store::ConfigFile => 1,
                Store::Session => 2,
            };
            v.set_selection(index);
        });
        self.siv.set_screen(self.login_screen);
    }

    pub(crate) fn run(&mut self) -> Result<()> {
        self.siv.set_fps(8);
        // TODO: This causes a crash in a selectview
        // I think it's the login prompt view
        //self.siv.refresh();
        while self.siv.is_running() {
            if self.siv.step() {
                self.siv.refresh();
            }
            if self.model.borrow_mut().update() {
                self.update_connected();
                self.update_disconnected();
                // TODO: This causes a crash in a selectview
                // I think it's the login prompt view
                //self.siv.refresh();
            }
        }
        Ok(())
    }
}
