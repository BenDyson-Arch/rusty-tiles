//! Exact unsigned interpretation of admitted numeric lexemes; no float rounding.
use super::FormatError;
use serde_json::value::RawValue;
type Raw<'a> = Option<&'a RawValue>;
type FormatResult<T> = Result<T, FormatError>;
fn invalid(s: &str) -> FormatError {
    FormatError::InvalidInput(s.into())
}
fn limit(s: &str) -> FormatError {
    FormatError::ResourceLimit(s.into())
}
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Unsigned {
    Native(u64),
    Overflow,
}
pub(crate) fn exact(raw: Raw<'_>) -> FormatResult<Unsigned> {
    let token = raw
        .ok_or_else(|| invalid("expected unsigned integer"))?
        .get();
    if !matches!(token.as_bytes().first(), Some(b'0'..=b'9' | b'-')) {
        return Err(invalid("expected unsigned integer"));
    }
    let negative = token.starts_with('-');
    let token = token.strip_prefix('-').unwrap_or(token);
    let (mantissa, exponent) = token.split_once(['e', 'E']).unwrap_or((token, "0"));
    let mut digits = 0usize;
    let mut leading = 0usize;
    let mut trailing = 0usize;
    let mut nonzero = false;
    for b in mantissa.bytes().filter(|b| *b != b'.') {
        digits += 1;
        if b == b'0' {
            if !nonzero {
                leading += 1;
            }
            trailing += 1;
        } else {
            nonzero = true;
            trailing = 0;
        }
    }
    if !nonzero {
        return Ok(Unsigned::Native(0));
    }
    if negative {
        return Err(invalid("negative unsigned integer"));
    }
    let fraction = mantissa.split_once('.').map_or(0, |(_, f)| f.len());
    let exponent_negative = exponent.starts_with('-');
    let ceiling = token
        .len()
        .checked_add(32)
        .ok_or_else(|| limit("numeric token host bound overflow"))?;
    let mut magnitude = 0usize;
    for b in exponent.trim_start_matches(['-', '+']).bytes() {
        magnitude = magnitude
            .saturating_mul(10)
            .saturating_add(usize::from(b - b'0'))
            .min(ceiling);
    }
    let scale = if exponent_negative {
        if trailing < fraction || magnitude > trailing - fraction {
            return Err(invalid("fractional unsigned integer"));
        }
        trailing - fraction - magnitude
    } else if trailing >= fraction {
        let cancelled = trailing - fraction;
        if cancelled > 20 || magnitude > 20 || cancelled + magnitude > 20 {
            return Ok(Unsigned::Overflow);
        }
        cancelled + magnitude
    } else {
        let cancelled = fraction - trailing;
        if magnitude < cancelled {
            return Err(invalid("fractional unsigned integer"));
        }
        magnitude - cancelled
    };
    let significant = digits - leading - trailing;
    if significant > 20 || scale > 20 - significant {
        return Ok(Unsigned::Overflow);
    }
    let mut value = 0u64;
    for b in mantissa
        .bytes()
        .filter(|b| *b != b'.')
        .skip(leading)
        .take(significant)
    {
        value = match value
            .checked_mul(10)
            .and_then(|v| v.checked_add(u64::from(b - b'0')))
        {
            Some(v) => v,
            None => return Ok(Unsigned::Overflow),
        };
    }
    for _ in 0..scale {
        value = match value.checked_mul(10) {
            Some(v) => v,
            None => return Ok(Unsigned::Overflow),
        };
    }
    Ok(Unsigned::Native(value))
}
pub(crate) fn work(raw: Raw<'_>, maximum: u64) -> FormatResult<u64> {
    match exact(raw)? {
        Unsigned::Native(n) if n <= maximum => Ok(n),
        _ => Err(limit("unsigned work exceeds allowance")),
    }
}
pub(crate) fn domain(raw: Raw<'_>, maximum: u64) -> FormatResult<u64> {
    match exact(raw)? {
        Unsigned::Native(n) if n <= maximum => Ok(n),
        _ => Err(invalid("unsigned value outside consuming domain")),
    }
}
