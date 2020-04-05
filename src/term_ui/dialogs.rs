use std::{cell::RefCell, rc::Rc};

use cursive::views::{Dialog, EditView, LinearLayout, PaddedView, SelectView, TextView};
use log::{debug, trace};
// Traits pulled in to add methods to types
use cursive::view::{Nameable, Resizable};
use cursive::Cursive;

use crate::config::Credentials;
use crate::model::Model;
use crate::model::StateMediator;

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

pub(crate) fn login_dialog(siv: &mut Cursive, model: Rc<RefCell<Model>>) -> Option<Dialog> {
    let mut model = model.borrow_mut();
    // Expired connections already have all necessary credentials,
    // and only need that we try to connect.
    model.connect();
    let connected = model.connected();
    let login_prompt_active = siv.find_name::<EditView>("username").is_some();
    match (connected, login_prompt_active) {
        (true, true) => {
            debug!("Login prompt active, but we have a valid connection.");
            siv.pop_layer();
            return None;
        }
        (true, false) => {
            // This is the expected case, generally - logged in and not
            // showing the login dialog
            return None;
        }
        (false, true) => {
            // Not connected, and we've already displayed the login dialog
            // nothing left to do until user clicks "Connect" button
            return None;
        }
        (false, false) => {
            trace!("Activating login dialog");
        }
    }

    let config = model.config();
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
