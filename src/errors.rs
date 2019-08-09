use std::boxed::Box;

use log;

pub(crate) type Result<T> = std::result::Result<T, Error>;

pub(crate) enum Error {
    ConfigDirNotFound,
    LoggerFailure(Box<log::SetLoggerError>),
    FlexiLoggerFailure(Box<flexi_logger::FlexiLoggerError>),
    LoggerFileFailure(Box<std::io::Error>),
    ConfigParseFailure(Box<serde_json::error::Error>),
    ConfigWriteFailure(Box<std::io::Error>),
    ConfigDirCreateFailure(Box<std::io::Error>),
    JsonSerializeFailure(Box<serde_json::error::Error>),
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        match self {
            Error::ConfigDirNotFound => write!(
                f,
                "Unable to identify platform-appropriate configuration directory."
            ),
            Error::LoggerFailure(e) => write!(f, "Error initializing logging: {}", e),
            Error::FlexiLoggerFailure(e) => write!(f, "Error initializing flexi-logger: {}", e),
            Error::LoggerFileFailure(e) => write!(f, "Error opening log file: {}", e),
            Error::ConfigParseFailure(e) => write!(f, "Error parsing configuration file: {}", e),
            Error::ConfigWriteFailure(e) => write!(f, "Error writing configuration file: {}", e),
            Error::JsonSerializeFailure(e) => {
                write!(f, "Error converting structure to json for writing: {}", e)
            }
            Error::ConfigDirCreateFailure(e) => {
                write!(f, "Error creating configuration directory: {}", e)
            }
        }
    }
}

impl std::fmt::Debug for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        write!(f, "{}", self.to_string())
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Error::ConfigDirNotFound => None,
            Error::LoggerFailure(e) => Some(e),
            Error::FlexiLoggerFailure(e) => Some(e),
            Error::LoggerFileFailure(e) => Some(e),
            Error::ConfigParseFailure(e) => Some(e),
            Error::ConfigWriteFailure(e) => Some(e),
            Error::JsonSerializeFailure(e) => Some(e),
            Error::ConfigDirCreateFailure(e) => Some(e),
        }
    }
}
