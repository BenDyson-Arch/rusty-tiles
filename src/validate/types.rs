//! Public contract for bounded read-only archive inspection.
use super::rows::PayloadRows;
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
    /// An internal invariant of the validator itself failed; never a verdict on the input.
    #[error("{0}")]
    InvalidState(String),
}
impl ValidationFailure {
    pub fn category(&self) -> (&'static str, u8) {
        match self {
            Self::InvalidInput(_) => ("invalid_input", 3),
            Self::Unsupported(_) => ("unsupported", 2),
            Self::ResourceLimit(_) => ("resource_limit", 3),
            Self::Io(_) => ("io", 1),
            Self::InvalidState(_) => ("invalid_state", 1),
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
    /// C2 decoded-content containment: maximum normative bit span of any exact quantity.
    pub containment_span_bits: u64,
    /// C2 precheck: maximum accumulated work units over all content references.
    pub containment_work_units: u64,
    /// C2 precheck: maximum archive bytes re-read by containment streaming.
    pub containment_bytes_read: u64,
    /// C2 precheck: maximum bytes materialized by containment re-decoding.
    pub containment_decoded_bytes: u64,
    /// C2 precheck: maximum selected glTF node path depth.
    pub containment_node_depth: u64,
    /// Tileset documents being traversed at once: `Σ (raw bytes + 128 · JSON value nodes)`
    /// over the documents on the external-tileset path (VS-B-7).
    pub live_document_bytes: u64,
}
impl Default for ValidationLimits {
    fn default() -> Self {
        Self {
            json_bytes: 8 * 1024 * 1024,
            json_depth: 64,
            member_bytes: 128 * 1024 * 1024,
            archive_stored_bytes: 1024 * 1024 * 1024,
            archive_entries: 1 << 19,
            accessor_elements: 4_000_000,
            document_decoded_bytes: 128 * 1024 * 1024,
            hierarchy_visits: 1 << 18,
            hierarchy_depth: 128,
            references: 1 << 20,
            total_payload_elements: 16_000_000,
            total_bytes_read: 2 * 1024 * 1024 * 1024,
            source_archive_bytes: 1024 * 1024 * 1024,
            central_directory_bytes: 32 * 1024 * 1024,
            document_items: 1 << 17,
            containment_span_bits: 4096,
            containment_work_units: 1 << 29,
            containment_bytes_read: 2 * 1024 * 1024 * 1024,
            containment_decoded_bytes: 2 * 1024 * 1024 * 1024,
            containment_node_depth: 64,
            live_document_bytes: 256 * 1024 * 1024,
        }
    }
}
/// Amplification budget per stored byte (VS-B-2): `numerator / denominator`.
struct Kappa(u64, u64);
impl Kappa {
    fn of(&self, stored_bytes: u64) -> u64 {
        // Saturating: an over-u64 product is the largest budget.
        u64::try_from(u128::from(stored_bytes) * u128::from(self.0) / u128::from(self.1))
            .unwrap_or(u64::MAX)
    }
}
impl ValidationLimits {
    /// The seven amplification budgets at `max(self, κ·S)` for `S` admitted stored bytes;
    /// every other field unchanged (VS-B-2). `self` holds the floors.
    pub(crate) fn scaled(&self, stored_bytes: u64) -> Self {
        let at_least = |floor: u64, kappa: Kappa| floor.max(kappa.of(stored_bytes));
        Self {
            hierarchy_visits: at_least(self.hierarchy_visits, Kappa(1, 8)),
            references: at_least(self.references, Kappa(1, 8)),
            total_bytes_read: at_least(self.total_bytes_read, Kappa(4, 1)),
            total_payload_elements: at_least(self.total_payload_elements, Kappa(16, 1)),
            containment_work_units: at_least(self.containment_work_units, Kappa(128, 1)),
            containment_bytes_read: at_least(self.containment_bytes_read, Kappa(4, 1)),
            containment_decoded_bytes: at_least(self.containment_decoded_bytes, Kappa(16, 1)),
            ..self.clone()
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
    /// Archive-named `notInspected` labels were dropped beyond the 64 kept (VS-B-9).
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub not_inspected_truncated: bool,
    pub limits: ValidationLimits,
    /// One row per inspected payload URI, in URI byte order, read on demand.
    pub payloads: PayloadRows,
    /// Present only when the catalog or the payload rows spilled to scratch.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub scratch: Option<ScratchReport>,
}
/// The validation's scratch use, reported whenever any spill file was written.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ScratchReport {
    /// The scratch directory is RAM (tmpfs or ramfs): the spill was held in memory. This
    /// happens only when no disk-backed candidate was available or fitted.
    pub memory_backed: bool,
    /// Largest number of bytes held in scratch files at once.
    pub peak_bytes: u64,
}
impl ValidationReport {
    /// The serialized report without its `payloads` member: a small JSON object.
    pub fn summary_json(&self) -> serde_json::Result<serde_json::Value> {
        let mut value = serde_json::json!({
            "ok": self.ok,
            "archive": self.archive,
            "tiles": self.tiles,
            "contentReferences": self.content_references,
            "entries": self.entries,
            "checks": self.checks,
            "notInspected": self.not_inspected,
        });
        if self.not_inspected_truncated {
            value["notInspectedTruncated"] = serde_json::Value::Bool(true);
        }
        value["limits"] = serde_json::to_value(&self.limits)?;
        if let Some(scratch) = self.scratch {
            value["scratch"] = serde_json::to_value(scratch)?;
        }
        Ok(value)
    }
    /// Writes the report as compact JSON, byte-identical to the `Display` of
    /// `serde_json::to_value(self)` (keys in sorted order), streaming the payload rows
    /// one at a time instead of materializing them.
    pub fn write_json(&self, out: &mut impl io::Write) -> io::Result<()> {
        let mut summary = self.summary_json()?;
        let Some(object) = summary.as_object_mut() else {
            unreachable!("the summary is an object");
        };
        object.insert("payloads".into(), serde_json::Value::Null);
        out.write_all(b"{")?;
        for (index, (key, value)) in object.iter().enumerate() {
            if index > 0 {
                out.write_all(b",")?;
            }
            serde_json::to_writer(&mut *out, key)?;
            out.write_all(b":")?;
            if key == "payloads" {
                out.write_all(b"[")?;
                for (row, report) in self.payloads.iter().enumerate() {
                    if row > 0 {
                        out.write_all(b",")?;
                    }
                    let report = report.map_err(io::Error::other)?;
                    serde_json::to_writer(&mut *out, &serde_json::to_value(report)?)?;
                }
                out.write_all(b"]")?;
            } else {
                serde_json::to_writer(&mut *out, value)?;
            }
        }
        out.write_all(b"}")
    }
}
