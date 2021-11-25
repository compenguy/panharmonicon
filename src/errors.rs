use thiserror::Error;

#[derive(Error, Debug, PartialEq)]
pub(crate) enum Error {
    #[error("Unable to identify platform-appropriate application directory")]
    AppDirNotFound,
    #[error("Pandora login credentials incomplete")]
    PanharmoniconMissingAuthToken,
    #[error("Error accessing session keyring {0}")]
    KeyringFailure(String),
    #[error("Error invalid operation {0} for state {1}")]
    InvalidOperationForState(String, String),
}

impl From<keyring::KeyringError> for Error {
    fn from(err: keyring::KeyringError) -> Self {
        Error::KeyringFailure(err.to_string())
    }
}
