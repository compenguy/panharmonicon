use std::cell::RefCell;
use std::rc::Rc;

use crate::config::Config;

mod crossterm;
pub(crate) use crate::term::crossterm::{ApplicationSignal, Terminal};
/*
mod crossterm_input;
pub(crate) use crate::term::crossterm_input::ApplicationSignal;
*/

#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) enum SessionAuth {
    UseSaved,
    ForceReauth,
}

impl SessionAuth {
    pub(crate) fn use_saved(self) -> bool {
        SessionAuth::UseSaved == self
    }
}

fn username_empty(config: Rc<RefCell<Config>>, auth: SessionAuth) -> bool {
    // Ignore the saved value
    !auth.use_saved()
        || config
            .borrow()
            .login
            .get_username()
            // There is a username, but it's empty
            .map(|u| u.is_empty())
            // There is no username
            .unwrap_or(true)

    /*
    if auth.use_saved() {
        if let Some(username) = config.borrow().login.get_username() {
            username.is_empty()
        } else {
            true
        }
    } else {
        true
    }
    */
}

fn password_empty(config: Rc<RefCell<Config>>, auth: SessionAuth) -> bool {
    // Ignore the saved value
    !auth.use_saved()
        || config
            .borrow()
            .login
            .get_password()
            // Check that we were successfully able to query for the password
            .ok()
            // And that the query returned some value
            .and_then(|x| x)
            // There is a password, but it's empty
            .map(|p| p.is_empty())
            // There was no password
            .unwrap_or(true)
    /*
    if auth.use_saved() {
        if let Ok(Some(password)) = config.borrow().login.get_password() {
            password.is_empty()
        } else {
            true
        }
    } else {
        true
    }
    */
}
