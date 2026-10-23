//! Disk spill for streaming producers: typed fixed-size records, buffered sorted runs and a
//! bounded fan-in k-way merge in an owned scratch directory.
//!
//! Resident memory is configuration only (run buffer `R`, fan-in `F`, per-stream buffer `b`);
//! disk is the only thing that grows with the input, and it is preflighted against free space
//! before the scratch directory exists. Output order is the stable sort of input order, so it is
//! independent of `R`, `F` and `jobs`. Every I/O failure is a typed [`SpillError`]; scratch files
//! are removed on drop and on error.
#![allow(dead_code)] // ML2 item 1 lands before its producers (ML2 items B and C).

mod scratch;
mod sequence;
mod sort;
mod volume;

#[allow(unused_imports)]
pub(crate) use scratch::{FileIo, Placement, Scratch, SpillIo};
#[allow(unused_imports)]
pub(crate) use sequence::{
    ByteFile, ByteWriter, RandomRecords, Sequence, SequenceReader, SequenceWriter, SlotFile,
};
#[allow(unused_imports)]
pub(crate) use sort::{
    chunk_records, ExternalSorter, SortConfig, SortStats, SortedStream, CHUNK_BYTES,
};
#[allow(unused_imports)]
pub(crate) use volume::{
    inspect_volume, preflight, same_filesystem, PreflightStage, SpaceRole, Volume,
};

use crate::{JobError, JobErrorKind};
use std::{fmt, io, path::PathBuf};

/// A fixed-size spill record with a platform-independent encoding.
///
/// `Ord` is the sort order. Records that compare equal keep their input order, so producers that
/// need byte-identical output under any configuration only need a deterministic input order.
///
/// A record may also have narrower run-level encodings ([`Layout`]): fields that a whole run
/// leaves at their default are omitted from every record of that run's sorts and sequences, and
/// decode restores them. The containers carry their layout, so readers are unchanged.
pub(crate) trait Record: Copy + Ord + Send {
    /// Encoded size in bytes under [`Layout::FULL`]; must be nonzero.
    const SIZE: usize;
    /// Writes exactly `SIZE` bytes into `out` (`out.len() == SIZE`).
    fn encode(&self, out: &mut [u8]);
    /// Reads a record previously written by [`Record::encode`] (`bytes.len() == SIZE`).
    fn decode(bytes: &[u8]) -> Self;
    /// Encoded size under `layout`: nonzero, at most `SIZE`, and `SIZE` for [`Layout::FULL`].
    fn size_in(layout: Layout) -> usize {
        let _ = layout;
        Self::SIZE
    }
    /// Writes exactly `size_in(layout)` bytes. The caller guarantees every field the layout
    /// omits holds its default; [`Layout::FULL`] is [`Record::encode`].
    fn encode_in(&self, layout: Layout, out: &mut [u8]) {
        let _ = layout;
        self.encode(out)
    }
    /// Reads a record written by [`Record::encode_in`] under the same layout.
    fn decode_in(layout: Layout, bytes: &[u8]) -> Self {
        let _ = layout;
        Self::decode(bytes)
    }
}

/// A run-level record encoding: a set of fields present in the run, interpreted by each
/// [`Record`] type. [`Layout::FULL`] keeps every field and is every record's `SIZE` encoding.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Layout(pub u8);
impl Layout {
    pub(crate) const FULL: Self = Self(u8::MAX);
}

/// Encoded size of `T` under `layout`, checked against the [`Record`] contract.
fn record_size<T: Record>(layout: Layout) -> Result<usize, SpillError> {
    match T::size_in(layout) {
        0 => Err(SpillError::InvalidConfig("record size must be nonzero")),
        size if size > T::SIZE => Err(SpillError::InvalidConfig(
            "record layout wider than its full encoding",
        )),
        size => Ok(size),
    }
}

/// Structured cause of a free-space refusal (`JobErrorKind::Io`, decision E28).
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct InsufficientSpace {
    pub path: PathBuf,
    pub needed: u64,
    pub available: u64,
    pub filesystem: SpaceRole,
    pub stage: PreflightStage,
}

impl fmt::Display for InsufficientSpace {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "insufficient free space on the {} filesystem at {} ({} preflight): need {} bytes, {} available",
            self.filesystem.as_str(),
            self.path.display(),
            self.stage.as_str(),
            self.needed,
            self.available
        )
    }
}

impl std::error::Error for InsufficientSpace {}

#[derive(Debug, thiserror::Error)]
pub(crate) enum SpillError {
    #[error("{operation}: {source} ({})", path.display())]
    Io {
        operation: &'static str,
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error(transparent)]
    InsufficientSpace(InsufficientSpace),
    /// Every candidate is memory-backed (tmpfs) and RAM scratch is not allowed (VS-C-5 T1).
    #[error("every scratch candidate is memory-backed (tmpfs), which would hold the spill in RAM: {}; set TMPDIR to a disk-backed directory", .candidates.iter().map(|path| path.display().to_string()).collect::<Vec<_>>().join(", "))]
    MemoryBackedOnly { candidates: Vec<PathBuf> },
    #[error("spill run is corrupt: {detail} ({})", path.display())]
    CorruptRun { path: PathBuf, detail: String },
    #[error("invalid spill configuration: {0}")]
    InvalidConfig(&'static str),
    #[error("spill size overflows u64")]
    Overflow,
    #[error("spill sorter already failed")]
    Poisoned,
}

impl SpillError {
    pub(crate) fn io(operation: &'static str, path: impl Into<PathBuf>, source: io::Error) -> Self {
        Self::Io {
            operation,
            path: path.into(),
            source,
        }
    }
}

impl From<SpillError> for JobError {
    fn from(error: SpillError) -> Self {
        match error {
            SpillError::Io {
                operation,
                path,
                source,
            } => JobError::io(operation, &path, source),
            SpillError::InsufficientSpace(cause) => JobError::with_cause_at(
                JobErrorKind::Io,
                cause.to_string(),
                &cause.path.clone(),
                cause,
            ),
            SpillError::MemoryBackedOnly { ref candidates } => {
                let path = candidates.first().cloned().unwrap_or_default();
                JobError::with_cause_at(JobErrorKind::Io, error.to_string(), &path, error)
            }
            SpillError::InvalidConfig(_) => {
                JobError::new(JobErrorKind::InvalidRequest, error.to_string())
            }
            SpillError::Overflow => JobError::new(JobErrorKind::ResourceLimit, error.to_string()),
            SpillError::Poisoned => JobError::new(JobErrorKind::InvalidState, error.to_string()),
            SpillError::CorruptRun { ref path, .. } => {
                let path = path.clone();
                JobError::with_cause_at(JobErrorKind::Io, error.to_string(), &path, error)
            }
        }
    }
}

#[cfg(test)]
mod tests;
