use std::boxed::Box;

use log;

pub(crate) type Result<T> = std::result::Result<T, Error>;

pub(crate) enum Error {
    ConfigDirNotFound,
    LoggerFailure(Box<log::SetLoggerError>),
    ConfigParseFailure(Box<serde_json::error::Error>),
    ConfigWriteFailure(Box<std::io::Error>),
    ConfigDirCreateFailure(Box<std::io::Error>),
    JsonSerializeFailure(Box<serde_json::error::Error>),
    TerminalIoInitFailure(Box<std::io::Error>),
    TerminalInitFailure(Box<std::io::Error>),
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        match self {
            Error::ConfigDirNotFound => write!(
                f,
                "Unable to identify platform-appropriate configuration directory."
            ),
            Error::LoggerFailure(e) => write!(f, "Error initializing logging: {}", e),
            Error::ConfigParseFailure(e) => write!(f, "Error parsing configuration file: {}", e),
            Error::ConfigWriteFailure(e) => write!(f, "Error writing configuration file: {}", e),
            Error::JsonSerializeFailure(e) => {
                write!(f, "Error converting structure to json for writing: {}", e)
            }
            Error::ConfigDirCreateFailure(e) => {
                write!(f, "Error creating configuration directory: {}", e)
            }
            Error::TerminalIoInitFailure(e) => write!(f, "Error initializing terminal: {}", e),
            Error::TerminalInitFailure(e) => write!(f, "Error initializing terminal: {}", e),
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
            Error::ConfigParseFailure(e) => Some(e),
            Error::ConfigWriteFailure(e) => Some(e),
            Error::JsonSerializeFailure(e) => Some(e),
            Error::ConfigDirCreateFailure(e) => Some(e),
            Error::TerminalIoInitFailure(e) => Some(e),
            Error::TerminalInitFailure(e) => Some(e),
        }
    }
}
