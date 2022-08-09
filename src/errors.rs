use thiserror::Error;

#[derive(Error, Debug)]
pub(crate) enum Error {
    #[error("Unable to identify platform-appropriate application directory")]
    AppDirNotFound,
    #[error("Pandora login credentials incomplete")]
    PanharmoniconMissingAuthToken,
    #[error("HTTP I/O failure: {0}")]
    HttpIoFailure(#[from] reqwest::Error),
    #[error("Error accessing session keyring {0}")]
    KeyringFailure(#[from] keyring::error::Error),
    #[error("Error invalid operation {0} for state {1}")]
    InvalidOperationForState(String, String),
    #[error("Requested track not in cache")]
    TrackNotCached(String),
}

/*
impl From<surf::Error> for Error {
    fn from(err: surf::Error) -> Self {
        Error::HttpIoFailure(err)
    }
}
*/

impl Error {
    pub(crate) fn missing_auth_token(&self) -> bool {
        matches!(self, Error::PanharmoniconMissingAuthToken)
    }
}
