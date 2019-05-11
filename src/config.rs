use std::fs::File;
use std::io::BufReader;
use std::io::BufWriter;
use std::path::Path;

use crate::errors::{Error, Result};

use serde_derive::{Deserialize, Serialize};

#[derive(Deserialize, Debug)]
pub(crate) struct PartialConfig {
    fun_mode: Option<bool>,
}

#[derive(Serialize, Debug)]
pub(crate) struct Config {
    fun_mode: bool,
}

impl std::default::Default for Config {
    fn default() -> Self {
        Config { fun_mode: true }
    }
}

impl Config {
    pub fn get_config<P: AsRef<Path> + Clone>(file_path: P, write_config: bool) -> Result<Config> {
        let mut config = Self::default();
        if let Ok(config_file) = File::open(file_path.clone()) {
            config.update_from(
                &serde_json::from_reader(BufReader::new(config_file))
                    .map_err(|e| Error::ConfigParseFailure(Box::new(e)))?,
            )?;
        }

        if write_config {
            config.write(file_path)?;
        }
        Ok(config)
    }

    pub fn write<P: AsRef<Path>>(&self, file_path: P) -> Result<()> {
        if let Some(dir) = file_path.as_ref().parent() {
            std::fs::create_dir_all(dir).map_err(|e| Error::ConfigDirCreateFailure(Box::new(e)))?;
        }
        let updated_config_file =
            File::create(file_path).map_err(|e| Error::ConfigWriteFailure(Box::new(e)))?;
        serde_json::to_writer_pretty(BufWriter::new(updated_config_file), self)
            .map_err(|e| Error::JsonSerializeFailure(Box::new(e)))
    }

    pub fn update_from(&mut self, other: &PartialConfig) -> Result<()> {
        if let Some(fun_mode) = other.fun_mode {
            self.fun_mode = fun_mode;
        }
        Ok(())
    }
}
