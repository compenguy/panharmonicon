use std::cell::RefCell;
use std::io::Read;
use std::rc::Rc;

use log::{debug, error};

use crate::config::Config;
use crate::player;
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

pub(crate) fn display_station_list(stations: &[player::StationInfo]) {
    for station in stations {
        println!("{}", station);
    }
}

pub(crate) fn display_station_info(station: &player::StationInfo) {
    println!("{}", station);
}

pub(crate) fn display_song_list(songs: &[player::SongInfo]) {
    for song in songs {
        println!("{}", song);
    }
}

pub(crate) fn display_song_info(song: &player::SongInfo) {
    println!("{}", song);
}

pub(crate) fn display_song_progress(progress: u8) {
    println!("{:03}", progress / std::u8::MAX)
}
