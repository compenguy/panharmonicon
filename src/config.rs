use std::fs::{create_dir_all, File};
use std::io::{BufReader, BufWriter};
use std::path::{Path, PathBuf};

use clap::crate_name;
use log::{debug, trace, warn};
use serde_derive::{Deserialize, Serialize};

use crate::errors::{Error, Result};

#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq)]
pub(crate) enum CachePolicy {
    NoCaching,
    CachePlayingEvictCompleted,
    CacheNextEvictCompleted,
    CacheAllNoEviction,
}

impl CachePolicy {
    pub(crate) fn cache_playing(self) -> bool {
        match self {
            Self::NoCaching => false,
            Self::CachePlayingEvictCompleted => true,
            Self::CacheNextEvictCompleted => true,
            Self::CacheAllNoEviction => true,
        }
    }

    pub(crate) fn cache_plus_one(self) -> bool {
        match self {
            Self::NoCaching => false,
            Self::CachePlayingEvictCompleted => false,
            Self::CacheNextEvictCompleted => true,
            Self::CacheAllNoEviction => true,
        }
    }

    pub(crate) fn cache_all(self) -> bool {
        match self {
            Self::NoCaching => false,
            Self::CachePlayingEvictCompleted => false,
            Self::CacheNextEvictCompleted => false,
            Self::CacheAllNoEviction => true,
        }
    }

    pub(crate) fn evict_completed(self) -> bool {
        match self {
            Self::NoCaching => false,
            Self::CachePlayingEvictCompleted => true,
            Self::CacheNextEvictCompleted => true,
            Self::CacheAllNoEviction => false,
        }
    }
}

impl Default for CachePolicy {
    fn default() -> Self {
        Self::CachePlayingEvictCompleted
    }
}

// TODO: switch to tagged for more obvious user editing?
// TODO: custom Debug implementation to prevent spilling secrets
#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(untagged)]
pub(crate) enum Credentials {
    Keyring(String),
    ConfigFile(String, String),
    #[serde(with = "serde_session")]
    Session(Option<String>, Option<String>),
    #[serde(skip)]
    Invalid(String),
}

impl Credentials {
    pub(crate) fn get(&self) -> Option<(String, String)> {
        match (self.username(), self.password().ok().flatten()) {
            (Some(u), Some(p)) if !u.is_empty() && !p.is_empty() => Some((u, p)),
            _ => None,
        }
    }

    pub(crate) fn username(&self) -> Option<String> {
        match self {
            Credentials::Keyring(u) if u.is_empty() => None,
            Credentials::Keyring(u) => Some(u.clone()),
            Credentials::ConfigFile(u, _) if u.is_empty() => None,
            Credentials::ConfigFile(u, _) => Some(u.clone()),
            Credentials::Session(o_u, _) if o_u.as_ref().map(|u| u.is_empty()).unwrap_or(true) => {
                None
            }
            Credentials::Session(o_u, _) => o_u.clone(),
            Credentials::Invalid(u) if u.is_empty() => None,
            Credentials::Invalid(u) => Some(u.clone()),
        }
    }

    pub(crate) fn password(&self) -> Result<Option<String>> {
        match self {
            Credentials::Keyring(u) => Credentials::get_from_keyring(&u),
            Credentials::ConfigFile(_, p) if p.is_empty() => Ok(None),
            Credentials::ConfigFile(_, p) => Ok(Some(p.clone())),
            Credentials::Session(_, o_p) if o_p.as_ref().map(|p| p.is_empty()).unwrap_or(true) => {
                Ok(None)
            }
            Credentials::Session(_, o_p) => Ok(o_p.clone()),
            Credentials::Invalid(_) => Ok(None),
        }
    }

    #[must_use = "Credentials may not be mutated in-place. Calling \"update_<field>()\" creates a copy with the updated value."]
    pub(crate) fn update_username(&self, username: &str) -> Credentials {
        let mut dup = self.clone();
        let username = username.to_string();
        match dup {
            Credentials::Keyring(ref mut u) => {
                *u = username;
                todo!("Keyring not being updated with new username.");
            }
            Credentials::ConfigFile(ref mut u, _) => {
                *u = username;
            }
            Credentials::Session(ref mut u, _) => {
                *u = if username.is_empty() {
                    None
                } else {
                    Some(username)
                };
            }
            Credentials::Invalid(ref mut u) => {
                *u = username;
            }
        }
        dup
    }

    #[must_use = "Credentials may not be mutated in-place. Calling \"update_<field>()\" creates a copy with the updated value."]
    pub(crate) fn update_password(&self, password: &str) -> Result<Credentials> {
        let mut dup = self.clone();
        match &mut dup {
            Credentials::Keyring(u) => Credentials::set_on_keyring(&u, password)?,
            Credentials::ConfigFile(_, ref mut p) => {
                *p = password.to_string();
            }
            Credentials::Session(_, ref mut p) => {
                *p = if password.is_empty() {
                    None
                } else {
                    Some(password.to_string())
                };
            }
            Credentials::Invalid(_) => {
                warn!("Ignoring request to update password on Invalid type credentials.");
            }
        }
        Ok(dup)
    }

    #[must_use = "Credentials may not be converted between variants in-place. Calling \"as_<type>\" creates a copy as another variant."]
    pub(crate) fn as_keyring(&self) -> Result<Credentials> {
        match self {
            Self::Keyring(_) => Ok(self.clone()),
            c => {
                let username = c.username().unwrap_or_default();
                let password = c.password().ok().flatten().unwrap_or_default();
                if !username.is_empty() && !password.is_empty() {
                    Credentials::set_on_keyring(&username, &password)?;
                }
                Ok(Self::Keyring(username))
            }
        }
    }

    #[must_use = "Credentials may not be converted between variants in-place. Calling \"as_<type>\" creates a copy as another variant."]
    pub(crate) fn as_configfile(&self) -> Credentials {
        match self {
            Self::ConfigFile(_, _) => self.clone(),
            c => {
                let username = c.username().unwrap_or_default();
                let password = c.password().ok().flatten().unwrap_or_default();
                Self::ConfigFile(username, password)
            }
        }
    }

    #[must_use = "Credentials may not be converted between variants in-place. Calling \"as_<type>\" creates a copy as another variant."]
    pub(crate) fn as_session(&self) -> Credentials {
        match self {
            Self::Session(_, _) => self.clone(),
            c => {
                let username = c.username();
                let password = c.password().ok().flatten();
                Self::Session(username, password)
            }
        }
    }

    #[must_use = "Credentials may not be converted between variants in-place. Calling \"as_<type>\" creates a copy as another variant."]
    pub(crate) fn as_invalid(&self) -> Credentials {
        match self {
            Self::Invalid(_) => self.clone(),
            c => {
                let username = c.username().unwrap_or_default();
                Self::Invalid(username)
            }
        }
    }

    fn get_from_keyring(username: &str) -> Result<Option<String>> {
        let service = String::from(crate_name!());
        let keyring = keyring::Keyring::new(&service, username);
        match keyring.get_password() {
            Ok(p) => Ok(Some(p)),
            Err(keyring::KeyringError::NoPasswordFound) => Ok(None),
            Err(e) => Err(Error::KeyringFailure(Box::new(e))),
        }
    }

    fn set_on_keyring(username: &str, password: &str) -> Result<()> {
        let service = String::from(crate_name!());
        let keyring = keyring::Keyring::new(&service, username);
        keyring
            .set_password(password)
            .map_err(|e| Error::KeyringFailure(Box::new(e)))
    }
}

impl std::cmp::PartialEq<Credentials> for Credentials {
    fn eq(&self, other: &Credentials) -> bool {
        if std::mem::discriminant(self) != std::mem::discriminant(&other) {
            return false;
        }

        if self.username() != other.username() {
            return false;
        }

        if self.password() != other.password() {
            return false;
        }

        true
    }
}

#[derive(Deserialize, Debug, Default)]
#[serde(rename = "Config")]
pub(crate) struct PartialConfig {
    pub(crate) login: Option<Credentials>,
    pub(crate) policy: Option<CachePolicy>,
    pub(crate) station_id: Option<Option<String>>,
    pub(crate) save_station: Option<bool>,
    pub(crate) volume: Option<f32>,
}

impl PartialConfig {
    pub(crate) fn new_login(cred: Credentials) -> Self {
        Self::from(cred)
    }

    pub(crate) fn new_cache_policy(policy: CachePolicy) -> Self {
        Self::from(policy)
    }

    pub(crate) fn new_station(station: String) -> Self {
        let mut pc = Self::default();
        pc.station_id = Some(Some(station));
        pc
    }

    pub(crate) fn no_station() -> Self {
        let mut pc = Self::default();
        pc.station_id = Some(None);
        pc
    }

    pub(crate) fn new_save_station(save: bool) -> Self {
        let mut pc = Self::default();
        pc.save_station = Some(save);
        pc
    }

    pub(crate) fn new_volume(volume: f32) -> Self {
        let mut pc = Self::default();
        pc.volume = Some(volume);
        pc
    }
}

impl From<Credentials> for PartialConfig {
    fn from(cred: Credentials) -> Self {
        let mut pc = Self::default();
        pc.login = Some(cred);
        pc
    }
}

impl From<CachePolicy> for PartialConfig {
    fn from(policy: CachePolicy) -> Self {
        let mut pc = Self::default();
        pc.policy = Some(policy);
        pc
    }
}

pub(crate) mod serde_session {
    use serde::de::Deserializer;
    use serde::ser::Serializer;

    pub(crate) fn serialize<S>(
        _: &Option<String>,
        _: &Option<String>,
        s: S,
    ) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        s.serialize_unit()
    }

    pub(crate) fn deserialize<'de, D>(_: D) -> Result<(Option<String>, Option<String>), D::Error>
    where
        D: Deserializer<'de>,
    {
        Ok((None, None))
    }
}

#[derive(Serialize, Debug)]
pub(crate) struct Config {
    #[serde(skip)]
    pub(crate) path: Option<PathBuf>,
    #[serde(skip)]
    pub(crate) dirty: bool,
    pub(crate) login: Credentials,
    pub(crate) policy: CachePolicy,
    pub(crate) station_id: Option<String>,
    pub(crate) save_station: bool,
    pub(crate) volume: f32,
}

impl std::default::Default for Config {
    fn default() -> Self {
        Self {
            dirty: false,
            //TODO: login: Credentials::Session(None, None),
            login: Credentials::Keyring(String::from("compenguy@gmail.com")),
            policy: CachePolicy::default(),
            station_id: None,
            save_station: true,
            path: None,
            volume: 1.0f32,
        }
    }
}

impl Config {
    pub(crate) fn get_config<P: AsRef<Path> + Clone>(
        file_path: P,
        write_back: bool,
    ) -> Result<Self> {
        let mut config = Self::default();
        config.path = Some(file_path.as_ref().to_path_buf());
        if let Ok(config_file) = File::open(file_path.as_ref()) {
            trace!("Reading config file");
            config.update_from(
                &serde_json::from_reader(BufReader::new(config_file))
                    .map_err(|e| Error::ConfigParseFailure(Box::new(e)))?,
            )?;
            config.dirty = false;
        }
        if write_back {
            trace!("Updating config file for newly-added options");
            config.dirty = true;
            config.flush()?;
        }
        Ok(config)
    }

    pub(crate) fn write<P: AsRef<Path>>(&self, file_path: P) -> Result<()> {
        if let Some(dir) = file_path.as_ref().parent() {
            create_dir_all(dir).map_err(|e| Error::AppDirCreateFailure(Box::new(e)))?;
        }
        let updated_config_file =
            std::fs::File::create(file_path).map_err(|e| Error::ConfigWriteFailure(Box::new(e)))?;
        serde_json::to_writer_pretty(BufWriter::new(updated_config_file), self)
            .map_err(|e| Error::JsonSerializeFailure(Box::new(e)))
    }

    pub(crate) fn flush(&mut self) -> Result<()> {
        trace!("Flushing config file...");
        if let Some(path) = self.path.as_ref() {
            trace!("Using config file at {}", path.to_string_lossy());
            if self.dirty || !path.exists() {
                trace!(
                    "Current settings differ from those on disk, writing updated settings to disk"
                );
                self.write(&path)?;
            }
            self.dirty = false;
        }
        Ok(())
    }

    pub(crate) fn update_from(&mut self, other: &PartialConfig) -> Result<()> {
        debug!("Settings before update: {:?}", self);
        debug!("Settings being applied: {:?}", other);
        if let Some(login) = &other.login {
            if self.login != *login {
                self.dirty |= true;
                self.login = login.clone();
            }
        }

        if let Some(policy) = &other.policy {
            if self.policy != *policy {
                self.dirty |= true;
                self.policy = *policy;
            }
        }

        if let Some(station_id) = &other.station_id {
            if self.station_id != *station_id {
                self.dirty |= true;
                self.station_id = station_id.clone();
            }
        }

        if let Some(save_station) = other.save_station {
            if self.save_station != save_station {
                self.dirty |= true;
                self.save_station = save_station;
            }
        }

        if let Some(volume) = other.volume {
            if (self.volume - volume).abs() > std::f32::EPSILON {
                self.dirty |= true;
                self.volume = volume;
            }
        }
        debug!("Settings after update: {:?}", self);
        Ok(())
    }

    pub(crate) fn login_credentials(&self) -> &Credentials {
        &self.login
    }

    pub(crate) fn cache_policy(&self) -> CachePolicy {
        self.policy
    }

    pub(crate) fn station_id(&self) -> Option<&String> {
        self.station_id.as_ref()
    }

    pub(crate) fn save_station(&self) -> bool {
        self.save_station
    }

    pub(crate) fn path(&self) -> Option<&PathBuf> {
        self.path.as_ref()
    }

    pub(crate) fn volume(&self) -> f32 {
        self.volume
    }
}
