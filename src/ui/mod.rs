use std::cell::RefCell;
use std::rc::Rc;

mod dumbterm;
// mod cursive;
use crate::ui;
use crate::config::Config;

#[derive(PartialEq)]
pub(crate) enum SessionAuth {
    UseSaved,
    ForceReauth,
}

impl SessionAuth {
    pub fn use_saved(&self) -> bool {
        SessionAuth::UseSaved == *self
    }
}

pub(crate) enum Session {
    DumbTerminal(Rc<RefCell<Config>>),
    // Cursive(Rc<RefCell<Cursive>>),
}

impl Session {
    pub fn new_dumb_terminal(config: Rc<RefCell<Config>>) -> Self {
        Session::DumbTerminal(config)
    }

    /*
    pub fn new_cursive(config: Rc<RefCell<Config>>) -> Self {
        let mut window = Cursive::default();
        window.add_global_callback('~', cursive::Cursive::toggle_debug_console);
        window.add_global_callback('q', |s| s.quit());

        window.set_user_data(config);
        Session::Cursive(Rc::new(RefCell::new(window)))
    }
    */

    pub fn login(&self, auth: SessionAuth) {
        match self {
            Session::DumbTerminal(cf) => ui::dumbterm::login_prompt(cf.clone(), auth),
            // Session::Cursive(cu) => ui::cursive::login_prompt(cu.clone(), auth),
        }
    }
}

