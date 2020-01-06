use std::boxed::Box;

use pandora_rs2;

pub(crate) type Result<T> = std::result::Result<T, Error>;

pub(crate) enum Error {
    ConfigDirNotFound,
    ConfigDirCreateFailure(Box<std::io::Error>),
    CacheDirNotFound,
    CacheDirCreateFailure(Box<std::io::Error>),
    FlexiLoggerFailure(Box<flexi_logger::FlexiLoggerError>),
    LoggerFileFailure(Box<std::io::Error>),
    ConfigParseFailure(Box<serde_json::error::Error>),
    ConfigWriteFailure(Box<std::io::Error>),
    JsonSerializeFailure(Box<serde_json::error::Error>),
    KeyringFailure(Box<keyring::KeyringError>),
    PandoraFailure(Box<pandora_rs2::error::Error>),
    FileCachingFailure(Box<std::io::Error>),
    HttpRequestFailure(Box<reqwest::Error>),
    PanharmoniconNotConnected,
    PanharmoniconNoStationSelected,
    PanharmoniconMissingAuthToken,
    PanharmoniconTrackHasNoId,
    PanharmoniconTrackHasNoName,
    PanharmoniconTrackHasNoArtist,
    PanharmoniconTrackHasNoAudio,
}

impl PartialEq<Error> for Error {
    fn eq(&self, other: &Error) -> bool {
        std::mem::discriminant(self) == std::mem::discriminant(&other)
    }
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        match self {
            Error::ConfigDirNotFound => write!(
                f,
                "Unable to identify platform-appropriate configuration directory."
            ),
            Error::ConfigDirCreateFailure(e) => {
                write!(f, "Error creating configuration directory: {}", e)
            }
            Error::CacheDirNotFound => write!(
                f,
                "Unable to identify platform-appropriate cache directory."
            ),
            Error::CacheDirCreateFailure(e) => write!(f, "Error creating cache directory: {}", e),
            Error::FlexiLoggerFailure(e) => write!(f, "Error initializing flexi-logger: {}", e),
            Error::LoggerFileFailure(e) => write!(f, "Error opening log file: {}", e),
            Error::ConfigParseFailure(e) => write!(f, "Error parsing configuration file: {}", e),
            Error::ConfigWriteFailure(e) => write!(f, "Error writing configuration file: {}", e),
            Error::JsonSerializeFailure(e) => {
                write!(f, "Error converting structure to json for writing: {}", e)
            }
            Error::KeyringFailure(e) => {
                write!(f, "Error reading password from system keyring: {}", e)
            }
            Error::PandoraFailure(e) => write!(f, "Pandora connection error: {}", e),
            Error::FileCachingFailure(e) => write!(f, "File caching error: {}", e),
            Error::HttpRequestFailure(e) => write!(f, "Http request error: {}", e),
            Error::PanharmoniconNotConnected => {
                write!(f, "Unable to complete action, not connected to Pandora")
            }
            Error::PanharmoniconNoStationSelected => {
                write!(f, "Unable to complete action, no station selected")
            }
            Error::PanharmoniconMissingAuthToken => {
                write!(f, "Pandora login credentials incomplete")
            }
            Error::PanharmoniconTrackHasNoId => write!(f, "Pandora track is missing track id"),
            Error::PanharmoniconTrackHasNoName => write!(f, "Pandora track is missing track name"),
            Error::PanharmoniconTrackHasNoArtist => {
                write!(f, "Pandora track is missing track artist")
            }
            Error::PanharmoniconTrackHasNoAudio => {
                write!(f, "Pandora track is missing track audio")
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
            Error::ConfigDirCreateFailure(e) => Some(e),
            Error::CacheDirNotFound => None,
            Error::CacheDirCreateFailure(e) => Some(e),
            Error::FlexiLoggerFailure(e) => Some(e),
            Error::LoggerFileFailure(e) => Some(e),
            Error::ConfigParseFailure(e) => Some(e),
            Error::ConfigWriteFailure(e) => Some(e),
            Error::JsonSerializeFailure(e) => Some(e),
            Error::KeyringFailure(e) => Some(e),
            Error::PandoraFailure(e) => Some(e),
            Error::FileCachingFailure(e) => Some(e),
            Error::HttpRequestFailure(e) => Some(e),
            Error::PanharmoniconNotConnected => None,
            Error::PanharmoniconNoStationSelected => None,
            Error::PanharmoniconMissingAuthToken => None,
            Error::PanharmoniconTrackHasNoId => None,
            Error::PanharmoniconTrackHasNoName => None,
            Error::PanharmoniconTrackHasNoArtist => None,
            Error::PanharmoniconTrackHasNoAudio => None,
        }
    }
}

impl From<keyring::KeyringError> for Error {
    fn from(err: keyring::KeyringError) -> Self {
        Error::KeyringFailure(Box::new(err))
    }
}

impl From<pandora_rs2::error::Error> for Error {
    fn from(err: pandora_rs2::error::Error) -> Self {
        Error::PandoraFailure(Box::new(err))
    }
}

impl From<reqwest::Error> for Error {
    fn from(err: reqwest::Error) -> Self {
        Error::HttpRequestFailure(Box::new(err))
    }
}
