use std::{cell::RefCell, rc::Rc};

use cursive::views::{DummyView, EditView, HideableView, LinearLayout, ResizedView, SelectView};
use cursive::Cursive;
use log::{error, trace};

use crate::model::Model;
use crate::model::{AudioMediator, PlaybackMediator, StateMediator};
use crate::term_ui::dialogs::Store;

use crate::config::PartialConfig;

fn model_quit(m: &mut Rc<RefCell<Model>>) {
    m.borrow_mut().quit();
}
fn model_pause(m: &mut Rc<RefCell<Model>>) {
    m.borrow_mut().pause();
}
fn model_unpause(m: &mut Rc<RefCell<Model>>) {
    m.borrow_mut().unpause();
}
fn model_toggle_pause(m: &mut Rc<RefCell<Model>>) {
    m.borrow_mut().toggle_pause();
}
fn model_decrease_volume(m: &mut Rc<RefCell<Model>>) {
    m.borrow_mut().decrease_volume();
}
fn model_increase_volume(m: &mut Rc<RefCell<Model>>) {
    m.borrow_mut().increase_volume();
}
fn model_stop(m: &mut Rc<RefCell<Model>>) {
    m.borrow_mut().stop();
}
fn model_sleep_track(m: &mut Rc<RefCell<Model>>) {
    m.borrow_mut().sleep_track();
}
fn model_rate_track_up(m: &mut Rc<RefCell<Model>>) {
    m.borrow_mut().rate_track(Some(true));
}
fn model_rate_track_down(m: &mut Rc<RefCell<Model>>) {
    let mut model = m.borrow_mut();
    model.rate_track(Some(false));
    model.stop();
}
fn model_clear_rating(m: &mut Rc<RefCell<Model>>) {
    m.borrow_mut().rate_track(None);
}

pub(crate) fn quit(s: &mut Cursive) {
    s.with_user_data(model_quit);
}
pub(crate) fn pause(s: &mut Cursive) {
    s.with_user_data(model_pause);
}
pub(crate) fn unpause(s: &mut Cursive) {
    s.with_user_data(model_unpause);
}
pub(crate) fn toggle_pause(s: &mut Cursive) {
    s.with_user_data(model_toggle_pause);
}
pub(crate) fn decrease_volume(s: &mut Cursive) {
    s.with_user_data(model_decrease_volume);
}
pub(crate) fn increase_volume(s: &mut Cursive) {
    s.with_user_data(model_increase_volume);
}
pub(crate) fn stop(s: &mut Cursive) {
    s.with_user_data(model_stop);
}
pub(crate) fn sleep_track(s: &mut Cursive) {
    s.with_user_data(model_sleep_track);
}
pub(crate) fn rate_track_up(s: &mut Cursive) {
    s.with_user_data(model_rate_track_up);
}
pub(crate) fn rate_track_down(s: &mut Cursive) {
    s.with_user_data(model_rate_track_down);
}
pub(crate) fn clear_rating(s: &mut Cursive) {
    s.with_user_data(model_clear_rating);
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
                config
                    .borrow_mut()
                    .update_from(&PartialConfig::default().login(c));
                model.connect();
            }
            Err(e) => {
                error!("Failed while updating password: {:?}", e);
            }
        }
    });
    s.pop_layer();
}

pub(crate) fn ui_scale(s: &mut Cursive) {
    let size = s.screen_size();
    trace!("Window resize. New size: {},{}", size.x, size.y);
    match size.y {
        // Disable spacer for height<5, we already use up the whole vertical space
        // Also disable the stations list so that we see more of the current track info
        _ if size.y <= 5 => {
            trace!("Hiding spacer ({:?})", s.debug_name("spacer_hideable"));
            s.call_on_name(
                "spacer_hideable",
                |v: &mut HideableView<ResizedView<DummyView>>| {
                    v.hide();
                    trace!("Spacer hidden.")
                },
            );
            trace!("Hiding stations ({:?})", s.debug_name("stations_hideable"));
            s.call_on_name("stations_hideable", |v: &mut HideableView<LinearLayout>| {
                v.hide();
                trace!("Stations hidden.")
            });
        }

        // Disable spacer for height==6, we already use up the whole vertical space
        // But show the station list - there's just enough room for it and all track info
        _ if size.y == 6 => {
            trace!("Hiding spacer ({:?})", s.debug_name("spacer_hideable"));
            s.call_on_name(
                "spacer_hideable",
                |v: &mut HideableView<ResizedView<DummyView>>| {
                    v.hide();
                    trace!("Spacer hidden.")
                },
            );
            trace!("Showing stations ({:?})", s.debug_name("stations_hideable"));
            s.call_on_name("stations_hideable", |v: &mut HideableView<LinearLayout>| {
                v.unhide();
                trace!("Stations unhidden.")
            });
        }

        // Enable spacer for height>6 so that the controls all appear at the bottom
        // Also ensure station list is visible
        _ => {
            trace!("Showing spacer ({:?})", s.debug_name("spacer_hideable"));
            s.call_on_name(
                "spacer_hideable",
                |v: &mut HideableView<ResizedView<DummyView>>| {
                    v.unhide();
                    trace!("Spacer unhidden.")
                },
            );
            trace!("Showing stations ({:?})", s.debug_name("stations_hideable"));
            s.call_on_name("stations_hideable", |v: &mut HideableView<LinearLayout>| {
                v.unhide();
                trace!("Stations unhidden.")
            });
        }
    }
    s.clear();
}
