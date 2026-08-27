use std::io;
use std::path::PathBuf;

use thiserror::Error;

#[derive(Debug, Error)]
pub enum Error {
    #[error("{0}")]
    Message(String),

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
}

impl Error {
    pub fn msg(s: impl Into<String>) -> Self {
        Error::Message(s.into())
    }
}
