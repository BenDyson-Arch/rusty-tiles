//! Public contract for bounded read-only archive inspection.
use super::limits::{LimitDerivation, LimitKey, LimitPlan, LimitRefusal};
use super::rows::PayloadRows;
use serde::Serialize;
use std::{
    io,
    path::{Path, PathBuf},
};
/// A delivery profile the caller requires the archive to satisfy, independently of any
/// label inside the archive.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ExpectedProfile {
    /// 0.4 octree HLOD: source identity on every payload and every HLOD delivery check
    /// (continuity, cap, omission, cover, interfaces) are mandatory. Missing identity is a
    /// refusal, never a fall-back to generic validation.
    Hlod,
}
#[derive(Clone, Debug)]
pub struct ValidationRequest {
    input: PathBuf,
    plan: LimitPlan,
    scratch_dir: Option<PathBuf>,
    expected: Option<ExpectedProfile>,
}
impl ValidationRequest {
    pub fn new(input: impl Into<PathBuf>) -> Self {
        Self {
            input: input.into(),
            plan: LimitPlan::default(),
            scratch_dir: None,
            expected: None,
        }
    }
    /// Requires `profile`. An archive declaring HLOD itself (`asset.extras.rustyTiles.lod` or
    /// the conversion report's `lod`, equal to `"hlod"`) is held to it without this call.
    pub fn expect_profile(mut self, profile: ExpectedProfile) -> Self {
        self.expected = Some(profile);
        self
    }
    /// Overrides one configurable limit (VS-C-3): a Class B volume cap is set, a Class C
    /// budget is replaced. Values below the default floors are allowed. A later call for the
    /// same key replaces the earlier value.
    pub fn max_volume(mut self, key: LimitKey, value: u64) -> Self {
        self.plan.set(key, value);
        self
    }
    /// Places validation scratch inside exactly this existing directory (VS-C-7).
    pub fn scratch_dir(mut self, dir: impl Into<PathBuf>) -> Self {
        self.scratch_dir = Some(dir.into());
        self
    }
    pub(crate) fn input(&self) -> &Path {
        &self.input
    }
    pub(crate) fn plan(&self) -> &LimitPlan {
        &self.plan
    }
    pub(crate) fn scratch(&self) -> Option<&Path> {
        self.scratch_dir.as_deref()
    }
    pub(crate) fn expected_profile(&self) -> Option<ExpectedProfile> {
        self.expected
    }
}
/// A validator scratch failure (VS-C-5 T1): never a verdict on the archive.
#[derive(Debug, thiserror::Error)]
pub enum ScratchRefusal {
    /// Every scratch candidate is memory-backed (tmpfs) where RAM scratch is not allowed.
    #[error("every scratch candidate is memory-backed (tmpfs), which would hold the spill in RAM: {}; set TMPDIR to a disk-backed directory", .candidates.iter().map(|path| path.display().to_string()).collect::<Vec<_>>().join(", "))]
    MemoryBackedOnly { candidates: Vec<PathBuf> },
    /// The free-space preflight refused the directory.
    #[error("{message}")]
    InsufficientSpace {
        path: PathBuf,
        needed: u64,
        available: u64,
        message: String,
    },
    #[error(transparent)]
    Io(io::Error),
}
impl ScratchRefusal {
    /// The scratch refusals among spill failures; any other failure is returned unchanged.
    pub(crate) fn from_spill(
        error: crate::spill::SpillError,
    ) -> Result<Self, crate::spill::SpillError> {
        use crate::spill::SpillError;
        match error {
            SpillError::MemoryBackedOnly { candidates } => {
                Ok(Self::MemoryBackedOnly { candidates })
            }
            SpillError::InsufficientSpace(cause) => Ok(Self::InsufficientSpace {
                message: cause.to_string(),
                path: cause.path,
                needed: cause.needed,
                available: cause.available,
            }),
            SpillError::Io { ref source, .. } => {
                Ok(Self::Io(io::Error::new(source.kind(), error.to_string())))
            }
            other => Err(other),
        }
    }
    /// `environment`/4 for a host without disk-backed scratch, otherwise `io`/1.
    pub fn category(&self) -> (&'static str, u8) {
        match self {
            Self::MemoryBackedOnly { .. } => ("environment", 4),
            Self::InsufficientSpace { .. } | Self::Io(_) => ("io", 1),
        }
    }
}
#[derive(Debug, thiserror::Error)]
pub enum ValidationFailure {
    #[error("{0}")]
    InvalidInput(String),
    #[error("{0}")]
    Unsupported(String),
    #[error("{0}")]
    ResourceLimit(LimitRefusal),
    #[error(transparent)]
    Io(#[from] io::Error),
    /// The validator's own scratch storage failed.
    #[error("validation scratch: {0}")]
    Scratch(ScratchRefusal),
    /// The request itself is invalid (a scratch directory that is missing or not a
    /// directory); raised before the input is opened.
    #[error("{0}")]
    InvalidRequest(String),
    /// The run control was cancelled (`inspect_with`).
    #[error("validation cancelled")]
    Cancelled,
    /// The run control's observer failed (`inspect_with`).
    #[error("{0}")]
    Observer(String),
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
            Self::Scratch(refusal) => refusal.category(),
            Self::InvalidRequest(_) => ("usage", 2),
            Self::Cancelled => ("cancelled", 1),
            Self::Observer(_) => ("observer_failure", 1),
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
        Self::ResourceLimit(LimitRefusal::from(message.into()))
    }
    /// The structured cause of a configurable-limit refusal, if this is one.
    pub fn exceeded(&self) -> Option<&super::limits::LimitExceeded> {
        match self {
            Self::ResourceLimit(refusal) => refusal.limit.as_ref(),
            _ => None,
        }
    }
}
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ValidationLimits {
    pub json_bytes: u64,
    pub json_depth: u64,
    pub member_bytes: u64,
    /// Class B: an opt-in volume cap, `None` (JSON `null`) when unset.
    pub archive_stored_bytes: Option<u64>,
    /// Class B: an opt-in volume cap, `None` (JSON `null`) when unset.
    pub archive_entries: Option<u64>,
    pub accessor_elements: u64,
    pub document_decoded_bytes: u64,
    pub hierarchy_visits: u64,
    pub hierarchy_depth: u64,
    pub references: u64,
    pub total_payload_elements: u64,
    pub total_bytes_read: u64,
    /// Class B: an opt-in volume cap, `None` (JSON `null`) when unset.
    pub source_archive_bytes: Option<u64>,
    /// Class B: an opt-in volume cap, `None` (JSON `null`) when unset.
    pub central_directory_bytes: Option<u64>,
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
            archive_stored_bytes: None,
            archive_entries: None,
            accessor_elements: 4_000_000,
            document_decoded_bytes: 128 * 1024 * 1024,
            hierarchy_visits: 1 << 18,
            hierarchy_depth: 128,
            references: 1 << 20,
            total_payload_elements: 16_000_000,
            total_bytes_read: 2 * 1024 * 1024 * 1024,
            source_archive_bytes: None,
            central_directory_bytes: None,
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
    /// S: the admitted stored member bytes, the index included (VS-C-4).
    pub stored_bytes: u64,
    pub limits: ValidationLimits,
    /// How each amplification budget in `limits` was derived (VS-C-4).
    pub limit_derivation: LimitDerivation,
    /// One row per inspected payload URI, in URI byte order, read on demand.
    pub payloads: PayloadRows,
    /// Exact item incidence derived from decoded leaf placements. Original-source
    /// fidelity and source-open-edge exemptions require the source-aware audit.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub interface_census: Option<InterfaceCensus>,
    /// Present only when the catalog or the payload rows spilled to scratch.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub scratch: Option<ScratchReport>,
}
/// Archive-only incidence, cap and omission observations; no producer counters are used.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InterfaceCensus {
    pub basis: &'static str,
    pub shared_points: u64,
    pub shared_segments: u64,
    pub source_open_edges: u64,
    pub source_open_edges_with_shared_endpoints: u64,
    /// The largest payload, counted over all its scene instances; every payload is at
    /// most the hard 8,192 (`triangleCap` refuses otherwise).
    pub max_payload_triangles: u64,
    /// Content-less delivered cells (placeholders excluded), by cell depth, and those
    /// whose cell children all render. Derived from the hierarchy, compared with
    /// producer reports only by the source-aware audit.
    pub omitted_cells: u64,
    pub omission_sources: u64,
    pub omitted_by_depth: Vec<u64>,
    /// Tiles of the first renderable cover and the renderable leaves it covers once.
    pub frontier_tiles: u64,
    pub frontier_leaves: u64,
    /// D11 contracted borders: arcs delivered as one chord over two or more leaf
    /// edges, counted per tile, and the leaf border vertices those chords contract.
    pub contracted_border_arcs: u64,
    pub contracted_border_vertices: u64,
    /// D11 coarse drops: child components coarse payloads no longer hold, by cell depth.
    /// Each matches its payload's declaration and lies within its geometricError.
    pub dropped_components: u64,
    pub dropped_by_depth: Vec<u64>,
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
        value["storedBytes"] = self.stored_bytes.into();
        value["limits"] = serde_json::to_value(&self.limits)?;
        value["limitDerivation"] = serde_json::to_value(&self.limit_derivation)?;
        if let Some(census) = &self.interface_census {
            value["interfaceCensus"] = serde_json::to_value(census)?;
        }
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
