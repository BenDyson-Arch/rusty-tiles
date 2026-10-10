//! Payload-specific consuming bounds; shared exact lexeme owner is its actual dependency.
use super::records::Raw;
use super::{invalid, FormatResult};
pub(super) use crate::content_integrity::numbers::{domain, work};
#[cfg(test)]
pub(super) use crate::content_integrity::numbers::{exact, Unsigned};
pub(super) fn uint(raw: Raw<'_>) -> FormatResult<usize> {
    usize::try_from(domain(raw, usize::MAX as u64)?)
        .map_err(|_| invalid("unsigned value outside host range"))
}
pub(super) fn off(raw: Raw<'_>) -> FormatResult<usize> {
    raw.map_or(Ok(0), |v| uint(Some(v)))
}
pub(super) fn at<T>(values: &[T], raw: Raw<'_>) -> FormatResult<usize> {
    let n = domain(raw, values.len().saturating_sub(1) as u64)?;
    if values.is_empty() {
        return Err(invalid("reference out of range"));
    }
    usize::try_from(n).map_err(|_| invalid("reference outside host range"))
}
