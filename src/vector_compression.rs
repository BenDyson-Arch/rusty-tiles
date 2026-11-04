//! Bounded lossless compression and named-entry replacement of one captured GLB.
//!
//! The source and its namespace must remain stable through the final recheck.
//! Replace is not a compare-and-swap: a competing writer after that recheck can
//! be overwritten. Existing readers and hardlink aliases retain the old object.
//! Portable Unix permission bits are copied; inode identity, timestamps, owner,
//! ACLs, extended attributes, special bits and crash durability are not preserved.
use crate::{
    content_integrity::FormatError,
    runtime::{file_publication_supported, Attempt, Staging},
    vector_encoding::{self, CodecError, CompressionReceipt, Encoded},
    CleanupDiagnostic, JobError, JobErrorKind, JobFailure, OutputPolicy, RunControl, RunEvent,
};
use serde::Serialize;
use std::{io::Write, mem::size_of, path::PathBuf};

mod source;
#[cfg(test)]
mod tests;

pub use crate::vector_encoding::CompressionLimits;
const CHUNK_BYTES: usize = 65_536;
// Nine unsigned integers (at most 20 decimal digits each) and fixed keys fit
// below 512 bytes on supported <=64-bit targets. No report path/user string.
const REPORT_BYTES: usize = 512;

/// One regular non-symlink source, replaced only after successful preparation.
#[derive(Clone, Debug)]
pub struct VectorCompressionRequest {
    input: PathBuf,
    limits: CompressionLimits,
}
impl VectorCompressionRequest {
    pub fn new(input: impl Into<PathBuf>) -> Self {
        Self {
            input: input.into(),
            limits: CompressionLimits::default(),
        }
    }
    pub fn with_limits(mut self, limits: CompressionLimits) -> Self {
        self.limits = limits;
        self
    }
}

/// Finite codec facts, frozen and serialized before final observation/publication.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VectorCompressionReport {
    pub before_bytes: usize,
    pub after_bytes: usize,
    pub views: usize,
    pub raw_views: usize,
    pub compressed_views: usize,
    pub existing_views: usize,
    pub accessors: usize,
    pub logical_bytes: usize,
    /// Source-derived requested-storage estimate; not allocator overhead or RSS.
    pub estimated_peak_bytes: usize,
}
impl From<CompressionReceipt> for VectorCompressionReport {
    fn from(r: CompressionReceipt) -> Self {
        Self {
            before_bytes: r.before_bytes,
            after_bytes: r.after_bytes,
            views: r.views,
            raw_views: r.raw_views,
            compressed_views: r.compressed_views,
            existing_views: r.existing_views,
            accessors: r.accessors,
            logical_bytes: r.logical_bytes,
            estimated_peak_bytes: r.estimated_peak_bytes,
        }
    }
}

/// Committed success, including any postcommit cleanup diagnostics.
#[derive(Debug)]
pub struct VectorCompressionResult {
    pub output: PathBuf,
    pub report: VectorCompressionReport,
    pub cleanup_diagnostics: Vec<CleanupDiagnostic>,
    report_json: String,
}
impl VectorCompressionResult {
    /// Already serialized before publication. Adapter output cannot undo commit.
    pub fn report_json(&self) -> &str {
        &self.report_json
    }
}

/// Capture, encode and publish one replacement under one consumed RunControl.
/// Windows readonly sources are Unsupported before callbacks or staging.
pub fn compress_vector_file(
    request: VectorCompressionRequest,
    control: &RunControl,
) -> Result<VectorCompressionResult, JobFailure> {
    // A refused/invalid request also consumes this control, as other F0 jobs do.
    let attempt = control.begin()?;
    let limits = request.limits.validate().map_err(|e| {
        attempt.fail(JobError::with_cause(
            JobErrorKind::InvalidRequest,
            "invalid vector compression limits",
            e,
        ))
    })?;
    file_publication_supported().map_err(|e| attempt.fail(e))?;
    let fixed = fixed_storage().map_err(|e| attempt.fail(e))?;
    let captured = source::Captured::capture(
        request.input,
        limits.values.source_bytes,
        limits.values.working_bytes,
        fixed,
        &attempt,
    )
    .map_err(|e| attempt.fail(e))?;
    let retained = checked_sum(&[
        captured.persistent_storage().map_err(|e| attempt.fail(e))?,
        fixed,
    ])
    .map_err(|e| attempt.fail(e))?;
    let mut checkpoint = || attempt.check();
    attempt
        .emit(&RunEvent::Progress {
            phase: "vector_compression_captured",
            done: 1,
            total: Some(1),
        })
        .map_err(|e| attempt.fail(e))?;
    let plan = vector_encoding::prepare(&captured.bytes, retained, &limits, &mut checkpoint)
        .map_err(|e| attempt.fail(external_codec_error(e)))?;
    let encoded = vector_encoding::encode(plan, &mut checkpoint)
        .map_err(|e| attempt.fail(external_codec_error(e)))?;
    let candidate_work =
        checked_sum(&[retained, encoded.bytes.capacity()]).map_err(|e| attempt.fail(e))?;
    if candidate_work > limits.values.working_bytes {
        return Err(attempt.fail(resource(
            "candidate capacity exceeds operation working byte limit",
        )));
    }
    let mut report = VectorCompressionReport::from(encoded.receipt);
    report.estimated_peak_bytes = report
        .estimated_peak_bytes
        .max(candidate_work)
        .max(captured.peak_bytes);
    // Reserve the small report exactly; retain its capacity in the preflight
    // fixed-storage term, never serialize a dynamic report after installation.
    let mut report_bytes = Vec::new();
    report_bytes.try_reserve_exact(REPORT_BYTES).map_err(|e| {
        attempt.fail(JobError::with_cause(
            JobErrorKind::ResourceLimit,
            "reserve vector receipt",
            e,
        ))
    })?;
    if report_bytes.capacity() > REPORT_BYTES {
        return Err(attempt.fail(resource("receipt capacity exceeds admitted fixed storage")));
    }
    serde_json::to_writer(&mut report_bytes, &report).map_err(|e| {
        attempt.fail(JobError::with_cause(
            JobErrorKind::InvalidState,
            "serialize vector receipt",
            e,
        ))
    })?;
    if report_bytes.len() > REPORT_BYTES {
        return Err(attempt.fail(JobError::new(
            JobErrorKind::InvalidState,
            "finite vector receipt exceeded its representation",
        )));
    }
    let report_json = String::from_utf8(report_bytes).map_err(|e| {
        attempt.fail(JobError::with_cause(
            JobErrorKind::InvalidState,
            "vector receipt is not UTF-8",
            e,
        ))
    })?;
    let mut staging = Staging::create(&captured.canonical, &attempt)?;
    // TempPath owns boxed storage; verify the actual path against the reserved
    // parent + separator + pinned Builder prefix/random-name length.
    let stage_bytes = staging.path().as_os_str().as_encoded_bytes().len();
    let stage_reserved =
        match checked_sum(&[captured.parent.as_os_str().as_encoded_bytes().len(), 20]) {
            Ok(n) => n,
            Err(e) => return Err(staging.fail(e)),
        };
    if stage_bytes > stage_reserved {
        return Err(staging.fail(resource("staging path exceeds admitted storage")));
    }
    if let Err(e) = write_candidate(&mut staging, &encoded.bytes, &attempt) {
        return Err(staging.fail(e));
    }
    // Complete the private file before granting its portable source permissions.
    let permissions = captured.candidate_permissions();
    if let Err(e) = staging.writer().set_permissions(permissions) {
        let error = JobError::io("copy source permissions to candidate", staging.path(), e);
        return Err(staging.fail(error));
    }
    if let Err(e) = attempt.emit(&RunEvent::Progress {
        phase: "vector_compression_ready",
        done: 1,
        total: Some(1),
    }) {
        return Err(staging.fail(e));
    }
    let mut chunk = [0u8; CHUNK_BYTES];
    if let Err(e) = captured.verify(&mut chunk, &attempt) {
        return Err(staging.fail(e));
    }
    // Release all source/identity handles before Windows installation. Candidate
    // bytes and captured source are no longer live in the publication phase.
    drop(encoded);
    let output = captured.into_output();
    if let Err(e) = attempt.close_events() {
        return Err(staging.fail(e));
    }
    let published = staging.seal()?.publish(&output, OutputPolicy::Replace)?;
    Ok(VectorCompressionResult {
        output: published.output,
        report,
        cleanup_diagnostics: published.cleanup_diagnostics,
        report_json,
    })
}

fn write_candidate(
    staging: &mut Staging<'_>,
    bytes: &[u8],
    attempt: &Attempt,
) -> Result<(), JobError> {
    for chunk in bytes.chunks(CHUNK_BYTES) {
        attempt.check()?;
        staging
            .writer()
            .write_all(chunk)
            .map_err(|e| JobError::io("write vector candidate", staging.path(), e))?;
    }
    attempt.check()
}

fn fixed_storage() -> Result<usize, JobError> {
    // Conservative additive headers, not a universal Q coefficient. Caller-owned
    // RunControl/observer storage and OS/native/allocator overhead are excluded.
    checked_sum(&[
        CHUNK_BYTES,
        REPORT_BYTES,
        size_of::<source::Captured>(),
        size_of::<Encoded>(),
        size_of::<VectorCompressionResult>(),
        size_of::<VectorCompressionReport>(),
        size_of::<CompressionLimits>(),
        size_of::<Staging<'static>>(),
        size_of::<std::fs::Metadata>() * 3,
        size_of::<same_file::Handle>() * 2,
        size_of::<PathBuf>() * 2,
    ])
}
fn checked_sum(parts: &[usize]) -> Result<usize, JobError> {
    parts
        .iter()
        .try_fold(0usize, |total, n| total.checked_add(*n))
        .filter(|n| *n <= isize::MAX as usize)
        .ok_or_else(|| resource("requested storage arithmetic exceeds host addressing"))
}
fn resource(message: &'static str) -> JobError {
    JobError::new(JobErrorKind::ResourceLimit, message)
}

/// Producer adapters retain the original attempt cause and classify internally
/// authored malformed/unsupported bytes as an invariant failure.
pub(crate) fn authored_codec_error(error: CodecError<JobError>) -> JobError {
    map_codec_error(error, true)
}
fn external_codec_error(error: CodecError<JobError>) -> JobError {
    map_codec_error(error, false)
}
fn map_codec_error(error: CodecError<JobError>, authored: bool) -> JobError {
    match error {
        CodecError::Checkpoint(error) => error,
        CodecError::Format(error) => format_error(error, authored),
        error @ CodecError::EncodingFailure(_) => JobError::with_cause(
            JobErrorKind::InvalidState,
            "native vector encoding failed",
            error,
        ),
    }
}
fn format_error(error: FormatError, authored: bool) -> JobError {
    let kind = match (&error, authored) {
        (FormatError::ResourceLimit(_), _) => JobErrorKind::ResourceLimit,
        (_, true) => JobErrorKind::InvalidState,
        (FormatError::InvalidInput(_), false) => JobErrorKind::InvalidInput,
        (FormatError::Unsupported(_), false) => JobErrorKind::Unsupported,
    };
    let message = match kind {
        JobErrorKind::ResourceLimit => "vector compression resource limit exceeded",
        JobErrorKind::InvalidInput => "invalid vector content",
        JobErrorKind::Unsupported => "unsupported vector content",
        _ => "internally authored vector content violates its codec contract",
    };
    JobError::with_cause(kind, message, error)
}
