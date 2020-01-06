use std::cell::RefCell;
use std::rc::Rc;

mod dumbterm;
// mod cursive;
use crate::app;
use crate::config::Config;
use crate::ui;

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

#[derive(Debug, Clone)]
pub(crate) enum Session {
    DumbTerminal(Rc<RefCell<Config>>),
    // Cursive(Rc<RefCell<Cursive>>),
}

impl Session {
    pub(crate) fn new_dumb_terminal(config: Rc<RefCell<Config>>) -> Self {
        Session::DumbTerminal(config)
    }

    /*
    pub(crate) fn new_cursive(config: Rc<RefCell<Config>>) -> Self {
        let mut window = Cursive::default();
        window.add_global_callback('~', cursive::Cursive::toggle_debug_console);
        window.add_global_callback('q', |s| s.quit());

        window.set_user_data(config);
        Session::Cursive(Rc::new(RefCell::new(window)))
    }
    */

    pub(crate) fn login(&self, auth: SessionAuth) {
        match self {
            Session::DumbTerminal(cf) => ui::dumbterm::login_prompt(cf.clone(), auth),
            // Session::Cursive(cu) => ui::cursive::login_prompt(cu.clone(), auth),
        }
    }

    pub(crate) fn display_error(&self, msg: &str) {
        match self {
            Session::DumbTerminal(_) => ui::dumbterm::display_error(msg),
            // Session::Cursive(cu) => ui::cursive::display_error(cu.clone(), msg),
        }
    }

    pub(crate) fn display_station_list(&self, stations: &[app::Station]) {
        match self {
            Session::DumbTerminal(_) => ui::dumbterm::display_station_list(stations),
            // Session::Cursive(cu) => ui::cursive::display_station_list(cu.clone(), stations),
        }
    }

    pub(crate) fn display_station_info(&self, station: &app::Station) {
        match self {
            Session::DumbTerminal(_) => ui::dumbterm::display_station_info(station),
            // Session::Cursive(cu) => ui::cursive::display_station_info(cu.clone(), station),
        }
    }

    pub(crate) fn display_song_list(&self, songs: &[app::SongInfo]) {
        match self {
            Session::DumbTerminal(_) => ui::dumbterm::display_song_list(songs),
            // Session::Cursive(cu) => ui::cursive::display_song_list(cu.clone(), songs),
        }
    }

    pub(crate) fn display_song_info(&self, song: &app::SongInfo) {
        match self {
            Session::DumbTerminal(_) => ui::dumbterm::display_song_info(song),
            // Session::Cursive(cu) => ui::cursive::display_song_info(cu.clone(), song),
        }
    }

    pub(crate) fn update_song_progress(&self, remaining: &std::time::Duration) {
        match self {
            Session::DumbTerminal(_) => ui::dumbterm::display_song_progress(remaining),
            // Session::Cursive(cu) => ui::cursive::display_song_progress(cu.clone(), remaining),
        }
    }

    pub(crate) fn select_station(&self) -> app::Station {
        match self {
            Session::DumbTerminal(_) => ui::dumbterm::station_prompt(),
            // Session::Cursive(cu) => ui::cursive::select_station(cu.clone()),
        }
    }
}
