//! D8: one declared memory budget per run, resolved once at the request boundary, and the
//! lease ledger that admits indivisible work against it before allocation.
//!
//! The budget is a ceiling on what every stage may plan against: capture's and the leaf
//! stage's job plans see `min(detected available, budget)` instead of the detected value,
//! and the HLOD stage admits its fixed terms, its minimum child window and one parent job
//! before it allocates. Memory pressure only serializes work; it never changes output.
use crate::{JobError, JobErrorKind};
use serde::Serialize;
use std::sync::{Arc, Mutex};

/// Default ceiling when the request names none: 4 GiB, or half the detected headroom when
/// that is smaller.
pub(crate) const DEFAULT_CEILING_BYTES: u64 = 4 << 30;
/// Default when the platform reports no available memory.
pub(crate) const UNDETECTED_BYTES: u64 = 2 << 30;

/// How the run's budget was resolved.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum BudgetSource {
    /// The request named a budget; it is never raised.
    Explicit,
    /// The 4 GiB default ceiling.
    DefaultCeiling,
    /// Half the detected host/cgroup headroom (below 4 GiB).
    DefaultHeadroom,
    /// No headroom detected: 2 GiB.
    DefaultUndetected,
}

/// The run's declared memory budget, resolved once before any allocation.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub struct MemoryBudget {
    pub requested_bytes: Option<u64>,
    pub resolved_bytes: u64,
    pub source: BudgetSource,
    /// Detected host/cgroup headroom at resolution (`None` where unreported).
    pub detected_headroom_bytes: Option<u64>,
}

fn invalid(message: impl Into<String>) -> JobError {
    JobError::new(JobErrorKind::InvalidRequest, message.into())
}

impl MemoryBudget {
    pub(crate) fn resolve(requested: Option<u64>) -> Result<Self, JobError> {
        Self::resolve_with(requested, super::config::available_memory())
    }

    pub(crate) fn resolve_with(
        requested: Option<u64>,
        detected: Option<u64>,
    ) -> Result<Self, JobError> {
        let (resolved, source) = match (requested, detected) {
            (Some(0), _) => return Err(invalid("memory budget must be a positive byte count")),
            (Some(bytes), _) => (bytes, BudgetSource::Explicit),
            (None, Some(headroom)) if headroom / 2 < DEFAULT_CEILING_BYTES => {
                (headroom / 2, BudgetSource::DefaultHeadroom)
            }
            (None, Some(_)) => (DEFAULT_CEILING_BYTES, BudgetSource::DefaultCeiling),
            (None, None) => (UNDETECTED_BYTES, BudgetSource::DefaultUndetected),
        };
        if resolved == 0 {
            return Err(JobError::new(
                JobErrorKind::ResourceLimit,
                "no memory headroom detected for a default budget",
            ));
        }
        Ok(Self {
            requested_bytes: requested,
            resolved_bytes: resolved,
            source,
            detected_headroom_bytes: detected,
        })
    }

    /// Memory a stage may plan against: the budget, never above the detected headroom.
    pub(crate) fn ceiling(&self, detected: Option<u64>) -> u64 {
        detected.map_or(self.resolved_bytes, |d| d.min(self.resolved_bytes))
    }
}

/// Parse `--memory-budget`: a positive decimal byte count with an optional binary suffix
/// (`KiB`, `MiB`, `GiB`, `TiB`; `B` for bytes). Zero, signs, fractions, decimal-unit
/// suffixes and overflow are refused.
pub fn parse_memory_budget(text: &str) -> Result<u64, JobError> {
    let refuse = || {
        invalid(format!(
            "memory budget {text:?} must be a positive integer byte count, optionally with a \
             binary suffix KiB, MiB, GiB or TiB (for example 2GiB)"
        ))
    };
    let digits = text.bytes().take_while(u8::is_ascii_digit).count();
    let (number, suffix) = text.split_at(digits);
    let shift = match suffix {
        "" | "B" => 0,
        "KiB" => 10,
        "MiB" => 20,
        "GiB" => 30,
        "TiB" => 40,
        _ => return Err(refuse()),
    };
    if number.is_empty() {
        return Err(refuse());
    }
    let value = number.parse::<u64>().map_err(|_| refuse())?;
    let bytes = value
        .checked_mul(1u64 << shift)
        .ok_or_else(|| invalid(format!("memory budget {text:?} exceeds 2^64 - 1 bytes")))?;
    if bytes == 0 {
        return Err(invalid("memory budget must be a positive byte count"));
    }
    Ok(bytes)
}

/// A counting byte ledger. Leases are taken before the allocation they cover and released
/// on drop; `keep_free` lets read-ahead leave room for the work that consumes it.
#[derive(Debug)]
pub(crate) struct Ledger {
    total: u64,
    state: Mutex<LedgerState>,
}

#[derive(Debug, Default)]
struct LedgerState {
    used: u64,
    peak: u64,
}

/// Bytes held against a [`Ledger`]; released on drop.
#[derive(Debug)]
pub(crate) struct Lease {
    ledger: Arc<Ledger>,
    bytes: u64,
}

impl Ledger {
    pub(crate) fn new(total: u64) -> Arc<Self> {
        Arc::new(Self {
            total,
            state: Mutex::new(LedgerState::default()),
        })
    }

    pub(crate) fn total(&self) -> u64 {
        self.total
    }

    pub(crate) fn peak(&self) -> u64 {
        self.lock().peak
    }

    pub(crate) fn used(&self) -> u64 {
        self.lock().used
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, LedgerState> {
        self.state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// Lease `bytes` if they fit while `keep_free` bytes stay unleased.
    pub(crate) fn try_lease(self: &Arc<Self>, bytes: u64, keep_free: u64) -> Option<Lease> {
        let mut state = self.lock();
        let after = state.used.checked_add(bytes)?;
        if after.checked_add(keep_free)? > self.total {
            return None;
        }
        state.used = after;
        state.peak = state.peak.max(after);
        Some(Lease {
            ledger: self.clone(),
            bytes,
        })
    }
}

impl Lease {
    pub(crate) fn bytes(&self) -> u64 {
        self.bytes
    }

    /// Give back the part of the lease above `bytes` (an output smaller than its admission).
    pub(crate) fn shrink(&mut self, bytes: u64) {
        if bytes < self.bytes {
            let mut state = self.ledger.lock();
            state.used -= self.bytes - bytes;
            self.bytes = bytes;
        }
    }
}

impl Drop for Lease {
    fn drop(&mut self) {
        let mut state = self.ledger.lock();
        state.used -= self.bytes;
    }
}

/// Whole-process user+system CPU seconds so far (all threads); `None` off Unix.
pub(crate) fn process_cpu_seconds() -> Option<f64> {
    #[cfg(unix)]
    {
        let mut usage = std::mem::MaybeUninit::<libc::rusage>::zeroed();
        // SAFETY: getrusage writes a complete rusage into the provided pointer on success.
        let status = unsafe { libc::getrusage(libc::RUSAGE_SELF, usage.as_mut_ptr()) };
        if status != 0 {
            return None;
        }
        // SAFETY: initialized by the successful call above.
        let usage = unsafe { usage.assume_init() };
        let seconds = |t: libc::timeval| t.tv_sec as f64 + t.tv_usec as f64 * 1e-6;
        Some(seconds(usage.ru_utime) + seconds(usage.ru_stime))
    }
    #[cfg(not(unix))]
    {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn budget_parsing_accepts_bytes_and_binary_suffixes_only() {
        assert_eq!(parse_memory_budget("1").unwrap(), 1);
        assert_eq!(parse_memory_budget("4096B").unwrap(), 4096);
        assert_eq!(parse_memory_budget("3KiB").unwrap(), 3 << 10);
        assert_eq!(parse_memory_budget("2GiB").unwrap(), 2 << 30);
        assert_eq!(parse_memory_budget("1TiB").unwrap(), 1 << 40);
        for bad in [
            "",
            "0",
            "0GiB",
            "-1",
            "+1",
            "1.5GiB",
            "2GB",
            "2gib",
            "2 GiB",
            "GiB",
            "true",
            "false",
            "0x10",
            "1e9",
            "18446744073709551616",
            "16777216TiB",
        ] {
            let error = parse_memory_budget(bad).unwrap_err();
            assert_eq!(error.kind(), JobErrorKind::InvalidRequest, "{bad:?}");
        }
        assert_eq!(
            parse_memory_budget("18446744073709551615").unwrap(),
            u64::MAX
        );
    }

    #[test]
    fn default_resolution_is_the_lesser_of_4gib_and_half_headroom() {
        let r = |requested, detected| MemoryBudget::resolve_with(requested, detected).unwrap();
        let big = r(None, Some(64 << 30));
        assert_eq!(
            (big.resolved_bytes, big.source),
            (4 << 30, BudgetSource::DefaultCeiling)
        );
        let small = r(None, Some(6 << 30));
        assert_eq!(
            (small.resolved_bytes, small.source),
            (3 << 30, BudgetSource::DefaultHeadroom)
        );
        let unknown = r(None, None);
        assert_eq!(
            (unknown.resolved_bytes, unknown.source),
            (2 << 30, BudgetSource::DefaultUndetected)
        );
        // An explicit budget is never raised, even above the default or the headroom.
        let explicit = r(Some(12 << 30), Some(8 << 30));
        assert_eq!(explicit.resolved_bytes, 12 << 30);
        assert_eq!(explicit.ceiling(Some(8 << 30)), 8 << 30);
        assert_eq!(explicit.ceiling(None), 12 << 30);
        assert_eq!(
            MemoryBudget::resolve_with(Some(0), Some(1 << 30))
                .unwrap_err()
                .kind(),
            JobErrorKind::InvalidRequest
        );
    }

    #[test]
    fn ledger_admits_to_equality_keeps_room_and_releases() {
        let ledger = Ledger::new(100);
        let a = ledger.try_lease(60, 0).unwrap();
        assert!(
            ledger.try_lease(30, 20).is_none(),
            "keep_free must stay free"
        );
        let mut b = ledger.try_lease(40, 0).unwrap();
        assert!(ledger.try_lease(1, 0).is_none());
        assert_eq!(ledger.used(), 100);
        b.shrink(10);
        assert_eq!(ledger.used(), 70);
        drop(a);
        drop(b);
        assert_eq!((ledger.used(), ledger.peak()), (0, 100));
    }
}
