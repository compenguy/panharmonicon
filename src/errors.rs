use std::boxed::Box;

use log;

pub(crate) type Result<T> = std::result::Result<T, Error>;

#[derive(Debug)]
pub(crate) enum Error {
    ConfigDirNotFound,
    LoggerError(Box<log::SetLoggerError>),
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        match self {
            Error::ConfigDirNotFound => write!(f, "Unable to identify platform-appropriate configuration directory."),
            Error::LoggerError(e) => write!(f, "Error initializing logging: {}", e),
        }
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Error::ConfigDirNotFound => None,
            Error::LoggerError(e) => Some(e),
        }
    }
}

