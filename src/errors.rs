use std::boxed::Box;

use pandora_rs2;

pub(crate) type Result<T> = std::result::Result<T, Error>;

pub(crate) enum Error {
    AppDirNotFound,
    AppDirCreateFailure(Box<std::io::Error>),
    FilenameEncodingFailure,
    FileWriteFailure(Box<dyn std::error::Error>),
    FileReadFailure(Box<dyn std::error::Error>),
    FlexiLoggerFailure(Box<flexi_logger::FlexiLoggerError>),
    ConfigParseFailure(Box<serde_json::error::Error>),
    ConfigWriteFailure(Box<std::io::Error>),
    JsonSerializeFailure(Box<serde_json::error::Error>),
    KeyringFailure(Box<keyring::KeyringError>),
    PandoraFailure(Box<pandora_rs2::error::Error>),
    HttpRequestFailure(Box<reqwest::Error>),
    CrosstermFailure(Box<crossterm::ErrorKind>),
    OutputFailure(Box<std::io::Error>),
    Mp4MediaParseFailure(Box<mp4parse::Error>),
    Mp3MediaParseFailure(Box<mp3_duration::MP3DurationError>),
    UnspecifiedOrUnsupportedMediaType,
    InvalidMedia,
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
            Error::AppDirNotFound => write!(
                f,
                "Unable to identify platform-appropriate application directory."
            ),
            Error::AppDirCreateFailure(e) => {
                write!(f, "Error creating application directory: {}", e)
            }
            Error::FilenameEncodingFailure => write!(f, "Invalid filename encoding."),
            Error::FileWriteFailure(e) => write!(f, "Error writing to file: {}", e),
            Error::FileReadFailure(e) => write!(f, "Error reading from file: {}", e),
            Error::FlexiLoggerFailure(e) => write!(f, "Error initializing flexi-logger: {}", e),
            Error::ConfigParseFailure(e) => write!(f, "Error parsing configuration file: {}", e),
            Error::ConfigWriteFailure(e) => write!(f, "Error writing configuration file: {}", e),
            Error::JsonSerializeFailure(e) => {
                write!(f, "Error converting structure to json for writing: {}", e)
            }
            Error::KeyringFailure(e) => {
                write!(f, "Error reading password from system keyring: {}", e)
            }
            Error::PandoraFailure(e) => write!(f, "Pandora connection error: {}", e),
            Error::HttpRequestFailure(e) => write!(f, "Http request error: {}", e),
            Error::CrosstermFailure(e) => write!(f, "Crossterm output error: {}", e),
            Error::OutputFailure(e) => write!(f, "Output write error: {}", e),
            Error::Mp4MediaParseFailure(e) => write!(f, "MP4 media parse error: {:?}", e),
            Error::Mp3MediaParseFailure(e) => write!(f, "MP3 media parse error: {:?}", e),
            Error::UnspecifiedOrUnsupportedMediaType => {
                write!(f, "Unspecified or unsupported media type")
            }
            Error::InvalidMedia => write!(f, "Invalid media stream"),
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
            Error::AppDirNotFound => None,
            Error::AppDirCreateFailure(e) => Some(e),
            Error::FilenameEncodingFailure => None,
            Error::FileWriteFailure(_) => None,
            Error::FileReadFailure(_) => None,
            Error::FlexiLoggerFailure(e) => Some(e),
            Error::ConfigParseFailure(e) => Some(e),
            Error::ConfigWriteFailure(e) => Some(e),
            Error::JsonSerializeFailure(e) => Some(e),
            Error::KeyringFailure(e) => Some(e),
            Error::PandoraFailure(e) => Some(e),
            Error::HttpRequestFailure(e) => Some(e),
            Error::CrosstermFailure(e) => Some(e),
            Error::OutputFailure(e) => Some(e),
            Error::Mp4MediaParseFailure(_) => None,
            Error::Mp3MediaParseFailure(_) => None,
            Error::UnspecifiedOrUnsupportedMediaType => None,
            Error::InvalidMedia => None,
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

impl From<crossterm::ErrorKind> for Error {
    fn from(err: crossterm::ErrorKind) -> Self {
        Error::CrosstermFailure(Box::new(err))
    }
}

impl From<mp4parse::Error> for Error {
    fn from(err: mp4parse::Error) -> Self {
        Error::Mp4MediaParseFailure(Box::new(err))
    }
}

impl From<mp3_duration::MP3DurationError> for Error {
    fn from(err: mp3_duration::MP3DurationError) -> Self {
        Error::Mp3MediaParseFailure(Box::new(err))
    }
}
