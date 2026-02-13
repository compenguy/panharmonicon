use cursive::views::{DummyView, EditView, HideableView, LinearLayout, ResizedView, SelectView};
use cursive::Cursive;
use cursive::View;
use log::{error, trace};

use crate::messages::{Request, StopReason};
use crate::term_ui::dialogs::Store;
use crate::term_ui::TerminalContext;

use crate::config::PartialConfig;

pub(crate) fn quit(s: &mut Cursive) {
    s.with_user_data(|ctx: &mut TerminalContext| {
        trace!("send request 'quit'");
        let _ = ctx.publish_request(Request::Quit);
    });
}
pub(crate) fn pause(s: &mut Cursive) {
    s.with_user_data(|ctx: &mut TerminalContext| {
        trace!("send request 'pause'");
        let _ = ctx.publish_request(Request::Pause);
    });
}
pub(crate) fn unpause(s: &mut Cursive) {
    s.with_user_data(|ctx: &mut TerminalContext| {
        trace!("send request 'unpause'");
        let _ = ctx.publish_request(Request::Unpause);
    });
}
pub(crate) fn toggle_pause(s: &mut Cursive) {
    s.with_user_data(|ctx: &mut TerminalContext| {
        trace!("send request 'toggle pause'");
        let _ = ctx.publish_request(Request::TogglePause);
    });
}
pub(crate) fn decrease_volume(s: &mut Cursive) {
    s.with_user_data(|ctx: &mut TerminalContext| {
        trace!("send request 'volume down'");
        let _ = ctx.publish_request(Request::VolumeDown);
    });
}
pub(crate) fn increase_volume(s: &mut Cursive) {
    s.with_user_data(|ctx: &mut TerminalContext| {
        trace!("send request 'volume down'");
        let _ = ctx.publish_request(Request::VolumeUp);
    });
}
pub(crate) fn stop(s: &mut Cursive) {
    s.with_user_data(|ctx: &mut TerminalContext| {
        trace!("send request 'stop'");
        let _ = ctx.publish_request(Request::Stop(StopReason::UserRequest));
    });
}
pub(crate) fn rate_track_up(s: &mut Cursive) {
    s.with_user_data(|ctx: &mut TerminalContext| {
        trace!("send request 'rate up'");
        let _ = ctx.publish_request(Request::RateUp);
    });
}
pub(crate) fn rate_track_down(s: &mut Cursive) {
    s.with_user_data(|ctx: &mut TerminalContext| {
        trace!("send request 'rate down and stop'");
        let _ = ctx.publish_request(Request::RateDown);
        let _ = ctx.publish_request(Request::Stop(StopReason::UserRequest));
    });
}
pub(crate) fn clear_rating(s: &mut Cursive) {
    s.with_user_data(|ctx: &mut TerminalContext| {
        trace!("send request 'unrate'");
        let _ = ctx.publish_request(Request::UnRate);
    });
}

pub(crate) fn connect_button(s: &mut Cursive) {
    let username: Option<String> =
        s.call_on_name("username", |v: &mut EditView| v.get_content().to_string());
    let password: Option<String> =
        s.call_on_name("password", |v: &mut EditView| v.get_content().to_string());
    let store: Option<Store> = s
        .call_on_name("store", |v: &mut SelectView<Store>| {
            v.selection().map(|s| (*s))
        })
        .flatten();
    s.with_user_data(|ctx: &mut TerminalContext| {
        let new_cred = match store.unwrap_or_default() {
            Store::Keyring => ctx
                .config
                .read()
                .expect("config read for password store as_keyring")
                .login_credentials()
                .as_keyring()
                .expect("Error updating keyring with password"),
            Store::ConfigFile => ctx
                .config
                .read()
                .expect("config read for password as_configfile")
                .login_credentials()
                .as_configfile(),
            Store::Session => ctx
                .config
                .read()
                .expect("config read for password as_session")
                .login_credentials()
                .as_session(),
        };
        let new_cred = username
            .map(|u| new_cred.update_username(&u))
            .unwrap_or(new_cred);
        let new_cred = password
            .map(|u| new_cred.update_password(&u))
            .unwrap_or(Ok(new_cred));
        match new_cred {
            Ok(c) => {
                ctx.config
                    .write()
                    .expect("config write for password update")
                    .update_from(&PartialConfig::default().login(c));
                trace!("send request 'connect'");
                let _ = ctx.publish_request(Request::Connect);
            }
            Err(e) => {
                error!("Failed while updating password: {e:?}");
            }
        }
    });
    s.pop_layer();
}

pub(crate) fn ui_scale(s: &mut Cursive) {
    let size = s.screen_size();
    trace!("Window resize. New size: {},{}", size.x, size.y);

    // Hide the station selector if there's less than 6 vertical lines
    s.call_on_name("stations_hideable", |v: &mut HideableView<LinearLayout>| {
        if size.y < 5 {
            v.hide();
            trace!("Stations hidden.")
        } else {
            v.unhide();
            trace!("Stations unhidden.")
        }
    });

    // Hide the spacer if there's less than 6 vertical lines
    s.call_on_name(
        "spacer_hideable",
        |v: &mut HideableView<ResizedView<DummyView>>| {
            if size.y < 7 {
                v.hide();
                trace!("Spacer hidden.")
            } else {
                v.unhide();
                trace!("Spacer unhidden.")
            }
        },
    );

    // Force a layout update
    s.screen_mut().layout(size);

    // This is the default action for this event, which we have replaced
    s.clear();
}
