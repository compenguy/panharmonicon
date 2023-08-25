use std::{cell::RefCell, rc::Rc};

use cursive::views::{
    Button, Dialog, DummyView, EditView, HideableView, LinearLayout, PaddedView, Panel, SelectView,
    SliderView, TextView,
};
use cursive::Cursive;
use cursive::{theme::ColorStyle, utils::markup::StyledString};
// Traits pulled in to add methods to types
use cursive::align::HAlign;
use cursive::view::{Nameable, Resizable};

use log::trace;

use crate::config::{Config, Credentials};
use crate::messages;
use crate::term_ui::{callbacks, labels, TerminalContext};

#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) enum Store {
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
        }
    }
}

impl Default for Store {
    fn default() -> Self {
        Self::Keyring
    }
}

pub(crate) fn playing_view() -> LinearLayout {
    let stations = LinearLayout::horizontal()
        .child(TextView::new(StyledString::styled(
            "Station ",
            ColorStyle::title_secondary(),
        )))
        .child(
            SelectView::<String>::new()
                .popup()
                .item("No Stations", String::from(""))
                .selected(0)
                .on_submit(|s: &mut Cursive, item: &String| {
                    trace!("send request 'tune'");
                    s.with_user_data(|ctx: &mut TerminalContext| {
                        trace!("Tuning to station {}", item.clone());
                        let _ = ctx
                            .publisher
                            .try_broadcast(messages::Request::Tune(item.clone()));
                    });
                })
                .with_name("stations")
                .fixed_height(1),
        );

    let next_up = LinearLayout::horizontal()
        .child(
            TextView::new(StyledString::styled(
                "Next up",
                ColorStyle::title_secondary(),
            ))
            .fixed_width(9),
        )
        .child(
            TextView::new(StyledString::plain("..."))
                .with_name("next_up")
                .fixed_height(1),
        );

    let station_row = LinearLayout::horizontal()
        .child(stations)
        .child(DummyView.full_width())
        .child(next_up);

    let controls_bar = LinearLayout::vertical()
        .child(
            LinearLayout::horizontal()
                .child(
                    TextView::new(StyledString::styled(
                        "Volume",
                        ColorStyle::title_secondary(),
                    ))
                    .fixed_width(7),
                )
                .child(
                    SliderView::horizontal(11)
                        .on_change(|s, v| {
                            let new_volume: f32 = ((v as f32) / 10.0).min(1.0f32).max(0.0f32);
                            trace!(
                                "Submitting updated volume from slider: {} ({:.2})",
                                v,
                                new_volume
                            );
                            trace!("send request 'volume'");
                            s.with_user_data(|ctx: &mut TerminalContext| {
                                let _ = ctx
                                    .publisher
                                    .try_broadcast(messages::Request::Volume(new_volume));
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
                )),
        );
    let playing = Panel::new(
        LinearLayout::horizontal()
            .child(
                LinearLayout::vertical()
                    .child(
                        LinearLayout::horizontal()
                            .child(
                                TextView::new(StyledString::styled(
                                    "Title",
                                    ColorStyle::title_secondary(),
                                ))
                                .fixed_width(7),
                            )
                            .child(TextView::empty().with_name("title")),
                    )
                    .child(
                        LinearLayout::horizontal()
                            .child(
                                TextView::new(StyledString::styled(
                                    "Artist",
                                    ColorStyle::title_secondary(),
                                ))
                                .fixed_width(7),
                            )
                            .child(TextView::empty().with_name("artist")),
                    )
                    .child(
                        LinearLayout::horizontal()
                            .child(
                                TextView::new(StyledString::styled(
                                    "Album",
                                    ColorStyle::title_secondary(),
                                ))
                                .fixed_width(7),
                            )
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

    LinearLayout::vertical()
        .child(playing)
        .child(HideableView::new(station_row).with_name("stations_hideable"))
}

pub(crate) fn login_dialog(config: Rc<RefCell<Config>>) -> Option<Dialog> {
    let credentials = config.borrow().login_credentials().clone();
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
    .button("Connect", crate::term_ui::callbacks::connect_button)
    .title("Pandora Login");

    Some(dialog)
}
