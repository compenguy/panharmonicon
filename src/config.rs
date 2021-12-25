use std::fs::{create_dir_all, File};
use std::io::{BufReader, BufWriter};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use clap::crate_name;
use log::{debug, trace};
use serde_derive::{Deserialize, Serialize};

use crate::errors::Error;

#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq)]
pub(crate) enum CachePolicy {
    // This entry is now superceded by, and behaves the same as, EvictCompleted
    // and is kept only to preserve compatibility for config files generated
    // prior to the introduction of the new values
    CachePlayingEvictCompleted,
    EvictCompleted,
    NoEviction,
}

impl CachePolicy {
    pub(crate) fn evict_completed(self) -> bool {
        match self {
            Self::CachePlayingEvictCompleted => true,
            Self::EvictCompleted => true,
            Self::NoEviction => false,
        }
    }
}

impl Default for CachePolicy {
    fn default() -> Self {
        Self::EvictCompleted
    }
}

#[derive(Serialize, Deserialize, Clone)]
pub(crate) enum Credentials {
    Keyring(String),
    ConfigFile(String, String),
    #[serde(with = "serde_session")]
    Session(Option<String>, Option<String>),
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
        }
    }

    pub(crate) fn password(&self) -> Result<Option<String>> {
        match self {
            Credentials::Keyring(u) => Credentials::get_from_keyring(u).with_context(|| {
                format!(
                    "Failed retrieving secrets for user {} from session keyring",
                    &u,
                )
            }),
            Credentials::ConfigFile(_, p) if p.is_empty() => Ok(None),
            Credentials::ConfigFile(_, p) => Ok(Some(p.clone())),
            Credentials::Session(_, o_p) if o_p.as_ref().map(|p| p.is_empty()).unwrap_or(true) => {
                Ok(None)
            }
            Credentials::Session(_, o_p) => Ok(o_p.clone()),
        }
    }

    #[must_use = "Credentials may not be mutated in-place. Calling \"update_<field>()\" creates a copy with the updated value."]
    pub(crate) fn update_username(&self, username: &str) -> Credentials {
        let mut dup = self.clone();
        let username = username.to_string();
        match dup {
            Credentials::Keyring(ref mut u) => {
                *u = username;
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
        }
        dup
    }

    #[must_use = "Credentials may not be mutated in-place. Calling \"update_<field>()\" creates a copy with the updated value."]
    pub(crate) fn update_password(&self, password: &str) -> Result<Credentials> {
        let mut dup = self.clone();
        match &mut dup {
            Credentials::Keyring(u) => {
                Credentials::set_on_keyring(u, password).with_context(|| {
                    format!("Failed updating secret for user {} on session keyring", &u)
                })?
            }
            Credentials::ConfigFile(_, ref mut p) => {
                if *p != password {
                    *p = password.to_string();
                }
            }
            Credentials::Session(_, ref mut p) => {
                *p = if password.is_empty() {
                    None
                } else {
                    Some(password.to_string())
                };
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
            Self::Session(u, _) => Self::Session(u.clone().map(String::from), None),
            Self::ConfigFile(u, _) => Self::ConfigFile(u.to_string(), String::new()),
            Self::Keyring(_) => self.clone(),
        }
    }

    fn get_from_keyring(username: &str) -> Result<Option<String>> {
        let service = String::from(crate_name!());
        let keyring = keyring::Entry::new(&service, username);
        match keyring.get_password() {
            Ok(p) => Ok(Some(p)),
            Err(keyring::error::Error::NoEntry) => Ok(None),
            Err(e) => Err(Error::from(e)).with_context(|| {
                format!("Error contacting session keyring for user {}", &username)
            }),
        }
    }

    fn set_on_keyring(username: &str, password: &str) -> Result<()> {
        let service = String::from(crate_name!());
        let keyring = keyring::Entry::new(&service, username);
        keyring
            .set_password(password)
            .map_err(Error::from)
            .with_context(|| {
                format!(
                    "Failed updating secret for user {} on session keyring",
                    &username
                )
            })
    }
}

// Custom implementation to avoid spilling secrets in log files, for example
impl std::fmt::Debug for Credentials {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Credentials::")?;
        match self {
            Self::Keyring(u) => write!(f, "Keyring({}, ******)", u),
            Self::ConfigFile(u, _) => write!(f, "ConfigFile({}, ******)", u),
            Self::Session(Some(u), Some(_)) => write!(f, "ConfigFile({}, ******)", u),
            Self::Session(Some(u), None) => write!(f, "ConfigFile({}, [missing])", u),
            Self::Session(None, Some(_)) => write!(f, "ConfigFile([missing], ******)"),
            Self::Session(None, None) => write!(f, "ConfigFile([missing], [missing])"),
        }
    }
}

impl std::cmp::PartialEq<Credentials> for Credentials {
    fn eq(&self, other: &Credentials) -> bool {
        if std::mem::discriminant(self) != std::mem::discriminant(other) {
            return false;
        }

        if self.username() != other.username() {
            return false;
        }

        if self.password().ok() != other.password().ok() {
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
    pub(crate) fn login(mut self, cred: Credentials) -> Self {
        self.login = Some(cred);
        self
    }

    pub(crate) fn cache_policy(mut self, policy: CachePolicy) -> Self {
        self.policy = Some(policy);
        self
    }

    pub(crate) fn station(mut self, station: Option<String>) -> Self {
        self.station_id = Some(station);
        self
    }

    /* TODO: There's no UI to expose this configuration option
    pub(crate) fn save_station(mut self, save: bool) -> Self {
        self.save_station = Some(save);
        self
    }
    */

    pub(crate) fn volume(mut self, volume: f32) -> Self {
        self.volume = Some(volume);
        self
    }
}

impl From<Credentials> for PartialConfig {
    fn from(cred: Credentials) -> Self {
        Self::default().login(cred)
    }
}

impl From<CachePolicy> for PartialConfig {
    fn from(policy: CachePolicy) -> Self {
        Self::default().cache_policy(policy)
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
            login: Credentials::Session(None, None),
            policy: CachePolicy::default(),
            station_id: None,
            save_station: true,
            path: None,
            volume: 1.0f32,
        }
    }
}

impl Config {
    #[allow(clippy::field_reassign_with_default)]
    pub(crate) fn get_config<P: AsRef<Path> + Clone>(
        file_path: P,
        write_back: bool,
    ) -> Result<Self> {
        let mut config = Self::default();
        config.path = Some(file_path.as_ref().to_path_buf());
        if let Ok(config_file) = File::open(file_path.as_ref()) {
            trace!("Reading config file");
            config.update_from(
                &serde_json::from_reader(BufReader::new(config_file)).with_context(|| {
                    format!(
                        "Error parsing application configuration file at {}",
                        file_path.as_ref().to_string_lossy()
                    )
                })?,
            );
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
            create_dir_all(dir).with_context(|| {
                format!(
                    "Failed to create directory for application configuration file as {}",
                    dir.to_string_lossy()
                )
            })?;
        }
        let updated_config_file = std::fs::File::create(file_path.as_ref()).with_context(|| {
            format!(
                "Failed writing to application configuration file as {}",
                file_path.as_ref().to_string_lossy()
            )
        })?;
        serde_json::to_writer_pretty(BufWriter::new(updated_config_file), self).with_context(|| {
            format!(
                "Failed while serializing configuration settings as JSON to disk as {}",
                file_path.as_ref().to_string_lossy()
            )
        })
    }

    pub(crate) fn flush(&mut self) -> Result<()> {
        trace!("Flushing config file...");
        if let Some(path) = self.path.as_ref() {
            trace!("Using config file at {}", path.to_string_lossy());
            if self.dirty || !path.exists() {
                trace!(
                    "Current settings differ from those on disk, writing updated settings to disk"
                );
                self.write(&path).with_context(|| {
                    format!(
                        "Failed while flushing application configuration changes to disk as {}",
                        path.to_string_lossy()
                    )
                })?;
            }
            self.dirty = false;
        }
        Ok(())
    }

    pub(crate) fn update_from(&mut self, other: &PartialConfig) {
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
    }

    pub(crate) fn login_credentials(&self) -> &Credentials {
        &self.login
    }

    pub(crate) fn cache_policy(&self) -> CachePolicy {
        self.policy
    }

    pub(crate) fn station_id(&self) -> Option<String> {
        self.station_id.clone()
    }

    /* TODO: There's no UI to expose this configuration option
    pub(crate) fn save_station(&self) -> bool {
        self.save_station
    }
    */

    pub(crate) fn volume(&self) -> f32 {
        self.volume
    }
}
