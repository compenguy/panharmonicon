use std::cell::RefCell;
use std::rc::Rc;
use std::io::Read;

use crate::config::Config;
use crate::ui::SessionAuth;

// force_login is intended for the case where login fails, we want to force re-entering the
// credentials
pub(crate) fn login_prompt(config: Rc<RefCell<Config>>, auth: SessionAuth) {
    let username_empty = if let Some(username) = config.borrow().login.get_username() {
        username.is_empty()
    } else {
        true
    };
    if username_empty || !auth.use_saved() {
        let mut username = String::new();
        print!("Pandora user: ");
        std::io::stdin().read_to_string(&mut username).expect("Failed to read from stdin");
        config.borrow_mut().login.update_username(&username);
    };

    let password_empty = if let Ok(Some(password)) = config.borrow().login.get_password() {
        password.is_empty()
    } else {
        true
    };
    if password_empty || !auth.use_saved() {
        let mut password = String::new();
        print!("Pandora password: ");
        std::io::stdin().read_to_string(&mut password).expect("Failed to read from stdin");
        config.borrow_mut().login.update_password(&password);
    };
}
