//! Explicit feature rejection is the only skippable failure. Infrastructure
//! errors enter through `From` as fatal errors, never as rejected features.
use crate::Error;
use std::fmt;

pub(crate) type FeatureResult<T> = Result<T, FeatureFailure>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RejectionKind {
    Feature,
    OutlineFallback,
}

#[derive(Debug, Clone)]
pub(crate) struct FeatureRejection {
    pub kind: RejectionKind,
    pub message: String,
}

#[derive(Debug)]
pub(crate) enum FeatureFailure {
    Rejected(FeatureRejection),
    Fatal(Error),
}

impl FeatureFailure {
    pub(crate) fn reject(message: impl Into<String>) -> Self {
        Self::Rejected(FeatureRejection {
            kind: RejectionKind::Feature,
            message: message.into(),
        })
    }

    pub(crate) fn outline(message: impl Into<String>) -> Self {
        Self::Rejected(FeatureRejection {
            kind: RejectionKind::OutlineFallback,
            message: message.into(),
        })
    }

    pub(crate) fn is_outline(&self) -> bool {
        matches!(self, Self::Rejected(rejection) if rejection.kind == RejectionKind::OutlineFallback)
    }

    /// Outside feature admission, a rejection stops the whole conversion.
    pub(crate) fn into_error(self) -> Error {
        match self {
            Self::Rejected(rejection) => Error::Data(rejection.message),
            Self::Fatal(error) => error,
        }
    }
}

impl fmt::Display for FeatureFailure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Rejected(rejection) => f.write_str(&rejection.message),
            Self::Fatal(error) => error.fmt(f),
        }
    }
}

impl From<Error> for FeatureFailure {
    fn from(error: Error) -> Self {
        Self::Fatal(error)
    }
}

impl From<FeatureFailure> for Error {
    fn from(error: FeatureFailure) -> Self {
        error.into_error()
    }
}

impl From<std::io::Error> for FeatureFailure {
    fn from(error: std::io::Error) -> Self {
        Self::Fatal(error.into())
    }
}

impl From<serde_json::Error> for FeatureFailure {
    fn from(error: serde_json::Error) -> Self {
        Self::Fatal(error.into())
    }
}
