use std::borrow::Cow;

use thiserror::Error;

pub type ArmatureResult<T> = Result<T, ArmatureError>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ErrorKind {
    InvalidInput,
    InvalidState,
    NotFound,
    Conflict,
    Unavailable,
    NotImplemented,
    Internal,
}

impl ErrorKind {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::InvalidInput => "invalid_input",
            Self::InvalidState => "invalid_state",
            Self::NotFound => "not_found",
            Self::Conflict => "conflict",
            Self::Unavailable => "unavailable",
            Self::NotImplemented => "not_implemented",
            Self::Internal => "internal",
        }
    }
}

#[derive(Debug, Error, Clone, PartialEq, Eq)]
#[error("{kind}: {message}")]
pub struct ArmatureError {
    pub kind: Cow<'static, str>,
    pub message: Cow<'static, str>,
}

impl ArmatureError {
    pub fn new(kind: ErrorKind, message: impl Into<Cow<'static, str>>) -> Self {
        Self {
            kind: Cow::Borrowed(kind.as_str()),
            message: message.into(),
        }
    }

    pub fn invalid_input(message: impl Into<Cow<'static, str>>) -> Self {
        Self::new(ErrorKind::InvalidInput, message)
    }

    pub fn invalid_state(message: impl Into<Cow<'static, str>>) -> Self {
        Self::new(ErrorKind::InvalidState, message)
    }

    pub fn not_found(message: impl Into<Cow<'static, str>>) -> Self {
        Self::new(ErrorKind::NotFound, message)
    }

    pub fn conflict(message: impl Into<Cow<'static, str>>) -> Self {
        Self::new(ErrorKind::Conflict, message)
    }

    pub fn unavailable(message: impl Into<Cow<'static, str>>) -> Self {
        Self::new(ErrorKind::Unavailable, message)
    }

    pub fn not_implemented(message: impl Into<Cow<'static, str>>) -> Self {
        Self::new(ErrorKind::NotImplemented, message)
    }

    pub fn internal(message: impl Into<Cow<'static, str>>) -> Self {
        Self::new(ErrorKind::Internal, message)
    }
}

impl From<std::io::Error> for ArmatureError {
    fn from(error: std::io::Error) -> Self {
        Self::internal(error.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::{ArmatureError, ErrorKind};

    #[test]
    fn formats_kind_and_message() {
        let error = ArmatureError::new(ErrorKind::Unavailable, "daemon is not running");
        assert_eq!(error.to_string(), "unavailable: daemon is not running");
    }
}
