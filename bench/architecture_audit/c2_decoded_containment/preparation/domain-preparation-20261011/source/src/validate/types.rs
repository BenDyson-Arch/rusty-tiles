//! Public contract for bounded read-only archive inspection.
use serde::Serialize;
use std::{io, path::PathBuf};
#[derive(Clone, Debug)]
pub struct ValidationRequest {
    input: PathBuf,
}
impl ValidationRequest {
    pub fn new(input: impl Into<PathBuf>) -> Self {
        Self {
            input: input.into(),
        }
    }
    pub(crate) fn input(&self) -> &std::path::Path {
        &self.input
    }
}
#[derive(Debug, thiserror::Error)]
pub enum ValidationFailure {
    #[error("{0}")]
    InvalidInput(String),
    #[error("{0}")]
    Unsupported(String),
    #[error("{0}")]
    ResourceLimit(String),
    #[error(transparent)]
    Io(#[from] io::Error),
}
impl ValidationFailure {
    pub fn category(&self) -> (&'static str, u8) {
        match self {
            Self::InvalidInput(_) => ("invalid_input", 3),
            Self::Unsupported(_) => ("unsupported", 2),
            Self::ResourceLimit(_) => ("resource_limit", 3),
            Self::Io(_) => ("io", 1),
        }
    }
    pub(crate) fn invalid(message: impl Into<String>) -> Self {
        Self::InvalidInput(message.into())
    }
    pub(crate) fn unsupported(message: impl Into<String>) -> Self {
        Self::Unsupported(message.into())
    }
    pub(crate) fn limit(message: impl Into<String>) -> Self {
        Self::ResourceLimit(message.into())
    }
}
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ValidationLimits {
    pub json_bytes: u64,
    pub json_depth: u64,
    pub member_bytes: u64,
    pub archive_stored_bytes: u64,
    pub archive_entries: u64,
    pub accessor_elements: u64,
    pub document_decoded_bytes: u64,
    pub hierarchy_visits: u64,
    pub hierarchy_depth: u64,
    pub references: u64,
    pub total_payload_elements: u64,
    pub total_bytes_read: u64,
    pub source_archive_bytes: u64,
    pub central_directory_bytes: u64,
    pub document_items: u64,
}
impl Default for ValidationLimits {
    fn default() -> Self {
        Self {
            json_bytes: 8 * 1024 * 1024,
            json_depth: 64,
            member_bytes: 64 * 1024 * 1024,
            archive_stored_bytes: 1024 * 1024 * 1024,
            archive_entries: 65536,
            accessor_elements: 4_000_000,
            document_decoded_bytes: 64 * 1024 * 1024,
            hierarchy_visits: 65536,
            hierarchy_depth: 128,
            references: 262144,
            total_payload_elements: 16_000_000,
            total_bytes_read: 2 * 1024 * 1024 * 1024,
            source_archive_bytes: 1024 * 1024 * 1024,
            central_directory_bytes: 16 * 1024 * 1024,
            document_items: 65536,
        }
    }
}
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PayloadReport {
    pub uri: String,
    pub accessors_checked: u64,
    pub primitives_checked: u64,
    pub vertices: u64,
}
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ValidationReport {
    pub ok: bool,
    pub archive: PathBuf,
    pub tiles: u64,
    pub content_references: u64,
    pub entries: u64,
    pub checks: Vec<String>,
    pub not_inspected: Vec<String>,
    pub limits: ValidationLimits,
    pub payloads: Vec<PayloadReport>,
}
