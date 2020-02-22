use std::boxed::Box;

use pandora_api;

pub(crate) type Result<T> = std::result::Result<T, Error>;

pub(crate) enum Error {
    AppDirNotFound,
    AppDirCreateFailure(Box<std::io::Error>),
    FileWriteFailure(Box<dyn std::error::Error>),
    FlexiLoggerFailure(Box<flexi_logger::FlexiLoggerError>),
    ConfigParseFailure(Box<serde_json::error::Error>),
    ConfigWriteFailure(Box<std::io::Error>),
    JsonSerializeFailure(Box<serde_json::error::Error>),
    KeyringFailure(Box<keyring::KeyringError>),
    PandoraFailure(Box<pandora_api::errors::Error>),
    HttpRequestFailure(Box<reqwest::Error>),
    OutputFailure(Box<std::io::Error>),
    MediaReadFailure(Box<std::io::Error>),
    AudioDecodingFailure(Box<rodio::decoder::DecoderError>),
    Mp3MediaParseFailure(Box<mp3_duration::MP3DurationError>),
    Mp3MetadataParseFailure(Box<id3::Error>),
    PanharmoniconNoStationSelected,
    PanharmoniconMissingAuthToken,
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
            Error::FileWriteFailure(e) => write!(f, "Error writing to file: {}", e),
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
            Error::OutputFailure(e) => write!(f, "Output write error: {}", e),
            Error::MediaReadFailure(e) => write!(f, "Media read error: {}", e),
            Error::AudioDecodingFailure(e) => write!(f, "Media decoding error: {:?}", e),
            Error::Mp3MediaParseFailure(e) => write!(f, "MP3 media parse error: {:?}", e),
            Error::Mp3MetadataParseFailure(e) => write!(f, "MP3 metadata parse error: {:?}", e),
            Error::PanharmoniconNoStationSelected => {
                write!(f, "Unable to complete action, no station selected")
            }
            Error::PanharmoniconMissingAuthToken => {
                write!(f, "Pandora login credentials incomplete")
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
            Error::FileWriteFailure(_) => None,
            Error::FlexiLoggerFailure(e) => Some(e),
            Error::ConfigParseFailure(e) => Some(e),
            Error::ConfigWriteFailure(e) => Some(e),
            Error::JsonSerializeFailure(e) => Some(e),
            Error::KeyringFailure(e) => Some(e),
            Error::PandoraFailure(e) => Some(e),
            Error::HttpRequestFailure(e) => Some(e),
            Error::OutputFailure(e) => Some(e),
            Error::MediaReadFailure(e) => Some(e),
            Error::AudioDecodingFailure(e) => Some(e),
            Error::Mp3MediaParseFailure(_) => None,
            Error::Mp3MetadataParseFailure(e) => Some(e),
            Error::PanharmoniconNoStationSelected => None,
            Error::PanharmoniconMissingAuthToken => None,
            Error::PanharmoniconTrackHasNoAudio => None,
        }
    }
}

impl From<keyring::KeyringError> for Error {
    fn from(err: keyring::KeyringError) -> Self {
        Error::KeyringFailure(Box::new(err))
    }
}

impl From<pandora_api::errors::Error> for Error {
    fn from(err: pandora_api::errors::Error) -> Self {
        Error::PandoraFailure(Box::new(err))
    }
}

impl From<reqwest::Error> for Error {
    fn from(err: reqwest::Error) -> Self {
        Error::HttpRequestFailure(Box::new(err))
    }
}

impl From<rodio::decoder::DecoderError> for Error {
    fn from(err: rodio::decoder::DecoderError) -> Self {
        Error::AudioDecodingFailure(Box::new(err))
    }
}

impl From<mp3_duration::MP3DurationError> for Error {
    fn from(err: mp3_duration::MP3DurationError) -> Self {
        Error::Mp3MediaParseFailure(Box::new(err))
    }
}

impl From<id3::Error> for Error {
    fn from(err: id3::Error) -> Self {
        Error::Mp3MetadataParseFailure(Box::new(err))
    }
}
