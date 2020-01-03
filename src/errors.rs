use std::boxed::Box;

pub(crate) type Result<T> = std::result::Result<T, Error>;

pub(crate) enum Error {
    ConfigDirNotFound,
    //FlexiLoggerFailure(Box<flexi_logger::FlexiLoggerError>),
    //LoggerFileFailure(Box<std::io::Error>),
    ConfigParseFailure(Box<serde_json::error::Error>),
    ConfigWriteFailure(Box<std::io::Error>),
    ConfigDirCreateFailure(Box<std::io::Error>),
    JsonSerializeFailure(Box<serde_json::error::Error>),
    KeyringFailure(Box<keyring::KeyringError>),
    ThreadChannelFailure(Box<std::sync::mpsc::RecvError>),
    ThreadChannelTryFailure(Box<std::sync::mpsc::TryRecvError>),
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
            //Error::FlexiLoggerFailure(e) => write!(f, "Error initializing flexi-logger: {}", e),
            //Error::LoggerFileFailure(e) => write!(f, "Error opening log file: {}", e),
            Error::ConfigParseFailure(e) => write!(f, "Error parsing configuration file: {}", e),
            Error::ConfigWriteFailure(e) => write!(f, "Error writing configuration file: {}", e),
            Error::JsonSerializeFailure(e) => {
                write!(f, "Error converting structure to json for writing: {}", e)
            }
            Error::ConfigDirCreateFailure(e) => {
                write!(f, "Error creating configuration directory: {}", e)
            }
            Error::KeyringFailure(e) => {
                write!(f, "Error reading password from system keyring: {}", e)
            }
            Error::ThreadChannelFailure(e) => {
                write!(f, "Thread communication channel error: {}", e)
            }
            Error::ThreadChannelTryFailure(e) => {
                write!(f, "Thread communication channel error: {}", e)
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
            //Error::FlexiLoggerFailure(e) => Some(e),
            //Error::LoggerFileFailure(e) => Some(e),
            Error::ConfigParseFailure(e) => Some(e),
            Error::ConfigWriteFailure(e) => Some(e),
            Error::JsonSerializeFailure(e) => Some(e),
            Error::ConfigDirCreateFailure(e) => Some(e),
            Error::KeyringFailure(e) => Some(e),
            Error::ThreadChannelFailure(e) => Some(e),
            Error::ThreadChannelTryFailure(e) => Some(e),
        }
    }
}

impl From<keyring::KeyringError> for Error {
    fn from(err: keyring::KeyringError) -> Self {
        Error::KeyringFailure(Box::new(err))
    }
}

impl From<std::sync::mpsc::RecvError> for Error {
    fn from(err: std::sync::mpsc::RecvError) -> Self {
        Error::ThreadChannelFailure(Box::new(err))
    }
}

impl From<std::sync::mpsc::TryRecvError> for Error {
    fn from(err: std::sync::mpsc::TryRecvError) -> Self {
        Error::ThreadChannelTryFailure(Box::new(err))
    }
}
