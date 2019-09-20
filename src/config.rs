use std::fs::{File, create_dir_all};
use std::io::BufReader;
use std::io::BufWriter;
use std::path::{Path, PathBuf};
use std::mem;

use clap::crate_name;
use serde_derive::{Deserialize, Serialize};

use crate::errors::{Error, Result};

#[derive(Deserialize, Debug)]
#[serde(rename = "Config")]
pub(crate) struct PartialConfig {
    login: Credentials,
}

#[derive(Serialize, Debug)]
pub(crate) struct Config {
    pub(crate) login: Credentials,
    #[serde(skip)]
    pub(crate) path: Option<PathBuf>,
    #[serde(skip)]
    pub(crate) dirty: bool,
}

pub(crate) mod serde_session {
    use serde::de::Deserializer;
    use serde::ser::Serializer;

    pub(crate) fn serialize<S>(_: &Option<String>, _: &Option<String>, s: S) -> Result<S::Ok, S::Error>
        where S: Serializer
    {
        s.serialize_unit()
    }

    pub(crate) fn deserialize<'de, D>(_: D) -> Result<(Option<String>, Option<String>), D::Error>
        where D: Deserializer<'de>
    {
        Ok((None, None))
    }
}

#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(untagged)]
pub(crate) enum Credentials {
    Keyring(String),
    ConfigFile(String, String),
    #[serde(with = "serde_session")]
    Session(Option<String>, Option<String>),
}

impl std::cmp::PartialEq<Credentials> for Credentials {
    fn eq(&self, other: &Credentials) -> bool {
        if std::mem::discriminant(self) != std::mem::discriminant(&other) {
            return false;
        }

        if self.get_username() != other.get_username() {
            return false;
        }

        if self.get_password() != other.get_password() {
            return false;
        }

        true
    }
}

impl Credentials {
    pub(crate) fn get_username(&self) -> Option<String> {
        match self {
            Credentials::Keyring(u) => Some(u.clone()),
            Credentials::ConfigFile(u, _) => Some(u.clone()),
            Credentials::Session(u, _) => u.clone(),
        }
    }

    pub(crate) fn update_username(&mut self, username: &str) {
        match self {
            Credentials::Keyring(ref mut u) => {
                mem::replace::<String>(u, username.to_string());
            }
            Credentials::ConfigFile(ref mut u, _) => {
                mem::replace::<String>(u, username.to_string());
            }
            Credentials::Session(ref mut u, _) => {
                mem::replace::<Option<String>>(u, Some(username.to_string()));
            }
        }
    }

    pub(crate) fn get_password(&self) -> Result<Option<String>> {
        match self {
            Credentials::Keyring(u) => Credentials::get_from_keyring(&u),
            Credentials::ConfigFile(_, p) => Ok(Some(p.clone())),
            Credentials::Session(_, p) => Ok(p.clone()),
        }
    }

    pub(crate) fn update_password(&mut self, password: &str) -> Result<()> {
        match self {
            Credentials::Keyring(u) => Credentials::set_on_keyring(&u, password),
            Credentials::ConfigFile(_, ref mut p) => {
                mem::replace::<String>(p, password.to_string());
                Ok(())
            }
            Credentials::Session(_, ref mut p) => {
                p.replace(password.to_string());
                Ok(())
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
        keyring.set_password(password).map_err(|e| Error::KeyringFailure(Box::new(e)))
    }
}


impl std::default::Default for Config {
    fn default() -> Self {
        Self {
            login: Credentials::Session(None, None),
            path: None,
            dirty: false,
        }
    }
}

impl Config {
    pub(crate) fn get_config<P: AsRef<Path> + Clone>(file_path: P, write_back: bool) -> Result<Self> {
        let mut config = Self::default();
        config.path = Some(file_path.as_ref().to_path_buf());
        if let Ok(config_file) = File::open(file_path.as_ref()) {
            config.update_from(
                &serde_json::from_reader(BufReader::new(config_file))
                    .map_err(|e| Error::ConfigParseFailure(Box::new(e)))?
            )?;
        }
        if write_back {
            config.flush()?;
        }
        Ok(config)
    }

    pub(crate) fn write<P: AsRef<Path>>(&self, file_path: P) -> Result<()> {
        if let Some(dir) = file_path.as_ref().parent() {
            create_dir_all(dir).map_err(|e| Error::ConfigDirCreateFailure(Box::new(e)))?;
        }
        let updated_config_file =
            std::fs::File::create(file_path).map_err(|e| Error::ConfigWriteFailure(Box::new(e)))?;
        serde_json::to_writer_pretty(BufWriter::new(updated_config_file), self)
            .map_err(|e| Error::JsonSerializeFailure(Box::new(e)))
    }

    pub(crate) fn flush(&mut self) -> Result<()> {
        if let Some(path) = self.path.as_ref() {
            if self.dirty || !path.exists() {
                self.write(&path)?;
            }
            self.dirty = false;
        }
        Ok(())
    }

    pub(crate) fn update_from(&mut self, other: &PartialConfig) -> Result<()> {
        if self.login != other.login {
            self.dirty |= true;
            self.login = other.login.clone();
        }

        Ok(())
    }
}
