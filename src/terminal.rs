use std::{cell::RefCell, rc::Rc};

use cursive::views::{
    Dialog, DummyView, EditView, LinearLayout, Panel, SelectView, SliderView, TextView,
};
use cursive::{Cursive, ScreenId};
// Traits pulled in to add methods to types
use cursive::view::{Nameable, Resizable};

use crate::config::Config;
use crate::errors::Result;
use crate::model::Model;

#[derive(Debug, Clone, Copy, PartialEq)]
enum Store {
    Keyring,
    ConfigFile,
    Session,
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
            s.with_user_data(|m: &mut Model| {
                todo!();
            });
        })
        .title("Pandora Login");
        self.siv.set_screen(self.login_screen);
        self.siv.screen_mut().add_layer(dialog);
    }

    fn init_playback(&mut self) {
        // TODO: fix height to 1, connect up on_submit
        let stations = LinearLayout::horizontal()
            .child(TextView::new("Station:"))
            .child(SelectView::<Store>::new().popup().with_name("stations"));
        // connect up on_submit
        let playing = Panel::new(
            LinearLayout::horizontal()
                .child(TextView::new("No Track").with_name("track"))
                .child(TextView::new("[00:00]").with_name("progress"))
                .child(DummyView)
                .child(TextView::new("Volume:"))
                .child(SliderView::horizontal(11).with_name("volume")),
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

    pub(crate) fn run(&mut self) -> Result<()> {
        loop {
            if self.siv.step() {
                // TODO: Add a dirty flag to model that gets set
                // by all &mut self methods on it, and use that
                // as a trigger for forcing refresh
                self.siv.refresh();
            }
            todo!("Advance application state.");
        }
        Ok(())
    }
}
