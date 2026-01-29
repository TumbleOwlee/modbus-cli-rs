use std::fmt::Display;
use tokio::sync::mpsc::error::SendError;

pub enum InstanceError {
    AlreadyActive,
    NotRunning,
    CancelFailed,
    SendError(SendError<net::Command>),
    InvalidOperation,
}

impl Display for InstanceError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            InstanceError::AlreadyActive => write!(f, "Instance is already active"),
            InstanceError::NotRunning => write!(f, "Instance is not running"),
            InstanceError::CancelFailed => write!(f, "Failed to cancel instance"),
            InstanceError::SendError(e) => {
                write!(f, "Failed to send command to instance: {}", e)
            }
            InstanceError::InvalidOperation => write!(f, "Invalid operation specified"),
        }
    }
}

pub enum Error {
    Net(net::Error),
    Instance(InstanceError),
}

impl From<InstanceError> for Error {
    fn from(e: InstanceError) -> Self {
        Error::Instance(e)
    }
}

impl From<net::Error> for Error {
    fn from(e: net::Error) -> Self {
        Error::Net(e)
    }
}

impl Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::Net(e) => write!(f, "Network error: {}", e),
            Error::Instance(s) => write!(f, "Instance error: {}", s),
        }
    }
}
