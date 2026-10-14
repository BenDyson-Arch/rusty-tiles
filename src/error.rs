use std::io;
use std::path::PathBuf;

use thiserror::Error;

#[derive(Debug, Error)]
pub enum Error {
    #[error(transparent)]
    Validation(#[from] crate::validate::ValidationFailure),

    #[error(transparent)]
    Job(#[from] crate::JobFailure),

    #[error("{0}")]
    Message(String),

    #[error("{0}")]
    Environment(String),

    #[error("{0}")]
    Data(String),

    #[error("parent texture projection has no compatible source surface at {position:?}, normal {normal:?}")]
    TextureProjection {
        position: [f32; 3],
        normal: [f32; 3],
    },

    #[error("input not found: {0}")]
    InputNotFound(PathBuf),

    #[error("output exists (pass --force): {0}")]
    OutputExists(PathBuf),

    #[error("no glTF/GLB tile content under {0}")]
    NoContent(PathBuf),

    #[error("package missing tileset.json")]
    MissingTilesetJson,

    #[error("{feature} is not implemented yet: {hint}")]
    NotImplemented { feature: &'static str, hint: String },

    #[error(transparent)]
    Io(#[from] io::Error),

    #[error(transparent)]
    Json(#[from] serde_json::Error),

    #[error(transparent)]
    Gltf(#[from] gltf::Error),

    #[error(transparent)]
    Zip(#[from] zip::result::ZipError),

    #[error(transparent)]
    Image(#[from] image::ImageError),
}

impl Error {
    /// Stable CLI failure categories: usage=2, data=3, environment=4, conflict=5.
    pub fn category(&self) -> (&'static str, u8) {
        match self {
            Self::Validation(failure) => failure.category(),
            Self::Job(failure) if failure.error.kind() == crate::JobErrorKind::ResourceLimit => {
                ("resource_limit", 1)
            }
            Self::Environment(_) => ("environment", 4),
            Self::OutputExists(_) => ("output_conflict", 5),
            Self::Io(_) => ("io", 1),
            Self::NotImplemented { .. } => ("usage", 2),
            _ => ("data", 3),
        }
    }

    pub fn msg(s: impl Into<String>) -> Self {
        Error::Message(s.into())
    }
}

// Adapt the private codec for remaining legacy operation/tool callers. The
// codec owns ordinary format errors and never depends on jobs or publication.
impl From<crate::archive3tz::CodecError> for Error {
    fn from(error: crate::archive3tz::CodecError) -> Self {
        use crate::archive3tz::CodecError;
        match error {
            CodecError::Io(error) => Self::Io(error),
            CodecError::SourceIo { source, .. } => Self::Io(source),
            CodecError::Zip(error) => Self::Zip(error),
            CodecError::Invalid(message) => Self::Message(message),
            CodecError::MissingManifest => Self::MissingTilesetJson,
            CodecError::SourceChanged(path) => {
                Self::Data(format!("source changed while packing: {}", path.display()))
            }
        }
    }
}
