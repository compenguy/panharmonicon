use std::cell::RefCell;
use std::rc::Rc;

use cursive::traits::*;
use cursive::views::{Dialog, EditView, LinearLayout, SelectView};
use cursive::Cursive;
use log::debug;

use crate::config::{Config, Credentials};
use crate::errors::Result;

// login control names
const USERNAME_CONTROL_ID: &str = "login::request_username";
const PASSWORD_CONTROL_ID: &str = "login::request_password";
const SAVE_TO_CONTROL_ID: &str = "login::save_to";

// SelectView entry IDs
#[derive(Clone)]
enum CredentialsEntry {
    ConfigFile,
    UserKeyring,
    SessionOnly,
}

impl From<&Credentials> for CredentialsEntry {
    fn from(cred: &Credentials) -> Self {
        match cred {
            Credentials::ConfigFile(_, _) => CredentialsEntry::ConfigFile,
            Credentials::Keyring(_) => CredentialsEntry::UserKeyring,
            Credentials::Session(_, _) => CredentialsEntry::SessionOnly,
        }
    }
}

impl From<usize> for CredentialsEntry {
    fn from(id: usize) -> Self {
        match id {
            0 => CredentialsEntry::ConfigFile,
            1 => CredentialsEntry::UserKeyring,
            2 => CredentialsEntry::SessionOnly,
            _ => CredentialsEntry::SessionOnly,
        }
    }
}

impl Into<usize> for CredentialsEntry {
    fn into(self) -> usize {
        match self {
            CredentialsEntry::ConfigFile => 0,
            CredentialsEntry::UserKeyring => 1,
            CredentialsEntry::SessionOnly => 2,
        }
    }
}

impl ToString for CredentialsEntry {
    fn to_string(&self) -> String {
        match self {
            CredentialsEntry::ConfigFile => "Config File".to_string(),
            CredentialsEntry::UserKeyring => "User Keyring".to_string(),
            CredentialsEntry::SessionOnly => "Don't Save".to_string(),
        }
    }
}

fn make_credentials(entry: CredentialsEntry, user: String, pass: String) -> Result<Credentials> {
    // TODO: check for empty string?
    match entry {
        CredentialsEntry::ConfigFile => Ok(Credentials::ConfigFile(user, pass)),
        CredentialsEntry::UserKeyring => {
            let mut cred = Credentials::Keyring(user);
            cred.update_password(&pass)?;
            Ok(cred)
        }
        CredentialsEntry::SessionOnly => Ok(Credentials::Session(Some(user), Some(pass))),
    }
}

fn login_submit(win: &mut Cursive) {
    let username = win
        .find_id::<EditView>(USERNAME_CONTROL_ID)
        .expect("Username entry control doesn't exist.")
        .get_content()
        .to_string();
    let password = win
        .find_id::<EditView>(PASSWORD_CONTROL_ID)
        .expect("Password entry control doesn't exist.")
        .get_content()
        .to_string();
    let cred_entry_rc: Rc<CredentialsEntry> = win
        .find_id::<SelectView<CredentialsEntry>>(SAVE_TO_CONTROL_ID)
        .expect("Credential storage selection control doesn't exist.")
        .selection()
        .expect("Credential storage selection control incorrectly populated.");
    win.with_user_data(|config: &mut Rc<RefCell<Config>>| {
        match make_credentials((*cred_entry_rc).clone(), username, password) {
            Ok(cred) => config.borrow_mut().login = cred,
            Err(_) => {
                debug!("TODO: failed making login credentials, probably unable to create keyring entry.");
                // TODO: try again? Error message popup?
            }
        }
    });
    win.pop_layer();
    win.quit();
}

pub(crate) fn login_prompt(config: Rc<RefCell<Config>>, win: Rc<RefCell<Cursive>>) {
    let current_username = config.borrow().login.get_username().unwrap_or_default();
    let current_password = config
        .borrow()
        .login
        .get_password()
        .unwrap_or_default()
        .unwrap_or_default();
    let cred_entry = CredentialsEntry::from(&config.borrow().login);

    win.borrow_mut().add_layer(
        Dialog::around(
            LinearLayout::vertical()
                .child(
                    EditView::new()
                        .content(current_username)
                        .on_submit(|win, _| login_submit(win))
                        .with_id(USERNAME_CONTROL_ID),
                )
                .child(
                    EditView::new()
                        .secret()
                        .content(current_password)
                        .on_submit(|win, _| login_submit(win))
                        .with_id(PASSWORD_CONTROL_ID),
                )
                .child(
                    SelectView::<CredentialsEntry>::new()
                        .item(
                            CredentialsEntry::ConfigFile.to_string(),
                            CredentialsEntry::ConfigFile,
                        )
                        .item(
                            CredentialsEntry::UserKeyring.to_string(),
                            CredentialsEntry::UserKeyring,
                        )
                        .item(
                            CredentialsEntry::SessionOnly.to_string(),
                            CredentialsEntry::SessionOnly,
                        )
                        .selected(cred_entry.into())
                        .on_submit(|win, _| login_submit(win))
                        .with_id(SAVE_TO_CONTROL_ID),
                ),
        )
        // Padding is (left, right, top, bottom)
        .padding((1, 1, 1, 0))
        .title("Pandora Login"),
    );
    win.borrow_mut().run();
}

pub(crate) fn login_cursive(config: Rc<RefCell<Config>>, win: Rc<RefCell<Cursive>>) -> Result<()> {
    if config.borrow().login.get_username().is_none()
        || config.borrow().login.get_password()?.is_none()
    {
        login_prompt(config, win.clone());
    }
    Ok(())
}
