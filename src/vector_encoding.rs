//! Immutable physical-view compression. Format policy knows no paths or jobs.
use crate::content_integrity::FormatError;
use std::{fmt, mem::size_of};
mod framing;
mod limits;
mod plan;
mod raw;
mod rewrite;
pub(crate) use framing::write_generated_glb;
pub use limits::CompressionLimits;
pub(crate) use limits::ValidatedCompressionLimits;
pub(crate) use plan::prepare;
pub(crate) use rewrite::encode;
#[derive(Debug)]
pub(crate) enum CodecError<E> {
    Format(FormatError),
    EncodingFailure(&'static str),
    Checkpoint(E),
}
impl<E> From<FormatError> for CodecError<E> {
    fn from(e: FormatError) -> Self {
        Self::Format(e)
    }
}
impl<E: fmt::Display> fmt::Display for CodecError<E> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Format(e) => write!(f, "{e}"),
            Self::EncodingFailure(s) => write!(f, "native encoding failed at {s}"),
            Self::Checkpoint(e) => write!(f, "{e}"),
        }
    }
}
impl<E: std::error::Error + 'static> std::error::Error for CodecError<E> {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Format(e) => Some(e),
            Self::Checkpoint(e) => Some(e),
            Self::EncodingFailure(_) => None,
        }
    }
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CompressionReceipt {
    pub before_bytes: usize,
    pub after_bytes: usize,
    pub views: usize,
    pub raw_views: usize,
    pub compressed_views: usize,
    pub existing_views: usize,
    pub accessors: usize,
    pub logical_bytes: usize,
    pub estimated_peak_bytes: usize,
}
pub(crate) struct Encoded {
    pub(crate) bytes: Vec<u8>,
    pub(crate) receipt: CompressionReceipt,
}
#[derive(Clone, Copy, Debug)]
pub(crate) struct GeneratedCounts {
    pub(crate) views: usize,
    pub(crate) accessors: usize,
    pub(crate) logical_bytes: usize,
}
type Result<T> = std::result::Result<T, FormatError>;
fn invalid(s: &'static str) -> FormatError {
    FormatError::InvalidInput(s.into())
}
fn unsupported(s: &'static str) -> FormatError {
    FormatError::Unsupported(s.into())
}
fn limit(s: &'static str) -> FormatError {
    FormatError::ResourceLimit(s.into())
}
fn add(a: usize, b: usize) -> Result<usize> {
    a.checked_add(b)
        .ok_or_else(|| limit("compression byte arithmetic overflow"))
}
fn mul(a: usize, b: usize) -> Result<usize> {
    a.checked_mul(b)
        .ok_or_else(|| limit("compression byte arithmetic overflow"))
}
fn align(n: usize, a: usize) -> Result<usize> {
    Ok(add(n, a - 1)? / a * a)
}
fn exact<T>(n: usize) -> Result<Vec<T>> {
    let bytes = mul(n, size_of::<T>())?;
    if bytes > isize::MAX as usize {
        return Err(limit("compression allocation outside host range"));
    }
    let mut out = Vec::new();
    out.try_reserve_exact(n)
        .map_err(|_| limit("compression owned reservation failed"))?;
    if out.capacity() != n {
        return Err(limit("compression exact capacity mismatch"));
    }
    #[cfg(test)]
    tests::reserved::<T>(n);
    Ok(out)
}
#[derive(Clone, Copy)]
struct Working {
    base: usize,
    maximum: usize,
    peak: usize,
}
fn finite_error_storage() -> usize {
    size_of::<FormatError>() + 3 * (size_of::<String>() + 6 * size_of::<usize>()) + 4 * 128
}
impl Working {
    fn new(base: usize, limits: &ValidatedCompressionLimits) -> Result<Self> {
        let base = add(base, finite_error_storage())?;
        if base > limits.values.working_bytes {
            return Err(limit("retained compression working storage exceeds limit"));
        }
        Ok(Self {
            base,
            maximum: limits.values.working_bytes,
            peak: base,
        })
    }
    fn admit(&mut self, extra: usize) -> Result<()> {
        let bytes = add(self.base, extra)?;
        if bytes > self.maximum {
            return Err(limit("estimated compression working storage exceeds limit"));
        }
        self.peak = self.peak.max(bytes);
        Ok(())
    }
}

#[cfg(test)]
pub(crate) use rewrite::with_capacity_one;

#[cfg(test)]
mod tests;
