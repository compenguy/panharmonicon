use std::cell::RefCell;
use std::io::Read;
use std::io::Write;
use std::rc::Rc;

use log::{error, trace};

use crate::app;
use crate::config::Config;
use crate::ui::SessionAuth;

pub(crate) fn username_empty(config: Rc<RefCell<Config>>, auth: SessionAuth) -> bool {
    if auth.use_saved() {
        if let Some(username) = config.borrow().login.get_username() {
            username.is_empty()
        } else {
            true
        }
    } else {
        true
    }
}

pub(crate) fn password_empty(config: Rc<RefCell<Config>>, auth: SessionAuth) -> bool {
    if auth.use_saved() {
        if let Ok(Some(password)) = config.borrow().login.get_password() {
            password.is_empty()
        } else {
            true
        }
    } else {
        true
    }
}

pub(crate) fn login_prompt(config: Rc<RefCell<Config>>, auth: SessionAuth) {
    let mut tmp_auth = auth;
    while username_empty(config.clone(), tmp_auth) {
        let mut username = String::new();
        print!("Pandora user: ");
        std::io::stdout().flush().expect("Error writing to stdout");
        std::io::stdin()
            .read_to_string(&mut username)
            .expect("Failed to read from stdin");
        config.borrow_mut().login.update_username(&username);
        // Ensure that we retry if the updated credentials are blank
        tmp_auth = SessionAuth::UseSaved;
    }

    tmp_auth = auth;
    while password_empty(config.clone(), tmp_auth) {
        let mut password = String::new();
        print!("Pandora password: ");
        std::io::stdin()
            .read_to_string(&mut password)
            .expect("Failed to read from stdin");
        if let Err(e) = config.borrow_mut().login.update_password(&password) {
            error!("Error updating password: {:?}", e);
            // Ensure that we retry if the password failed to update
            tmp_auth = SessionAuth::ForceReauth;
        } else {
            // Ensure that we retry if the updated credentials are blank
            tmp_auth = SessionAuth::UseSaved;
        }
    }
}

pub(crate) fn display_error(msg: &str) {
    error!("Pandora error: {}", msg);
}

pub(crate) fn display_station_list(stations: &[app::Station]) {
    for station in stations {
        display_station_info(station);
    }
}

pub(crate) fn display_station_info(station: &app::Station) {
    println!("{} ({})", station.station_name, station.station_id);
}

pub(crate) fn display_song_list(songs: &[app::SongInfo]) {
    for song in songs {
        display_song_info(song);
    }
}

pub(crate) fn display_song_info(song: &app::SongInfo) {
    println!("{} by {}", song.name, song.artist);
}

pub(crate) fn display_song_progress(remaining: &std::time::Duration) {
    println!(
        "left: {:2}m {:2}s",
        remaining.as_secs() / 60,
        remaining.as_secs() % 60
    );
}

pub(crate) fn station_prompt() -> app::Station {
    trace!("Prompting user for station id");
    let mut station_id = String::new();
    while station_id.is_empty() {
        print!("Station Id: ");
        std::io::stdout().flush().expect("Error writing to stdout");
        std::io::stdin()
            .read_line(&mut station_id)
            .expect("Failed to read from stdin");
    }

    trace!("Got user-supplied station id: {}", station_id);
    return app::Station {
        station_id: station_id.trim().to_string(),
        station_name: String::new(),
    };
}
