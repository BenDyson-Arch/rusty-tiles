//! Capture's sort configuration, resolved once from the admitted memory budget (D8).
//!
//! The request's sort configuration (`TreeConfig::sort`) is the **floor**: capture's jobs
//! plan and its minimum budget are computed with it, so a budget that admits the floor is
//! never refused for want of larger runs. Above the floor, capture's run buffers grow to the
//! smallest size at which every sort inside capture has at most `F` initial runs, so each
//! merges exactly once, in its final stream, at the fan-in among 64, 128 and 256 that
//! admits the most background run writers (up to one fewer than the workers and one fewer
//! than the most-loaded sort's runs; the smaller fan-in on a tie). A budget too small for one pass at any of them gets the largest run
//! buffer that fits at the largest fan-in that fits, without writers. Every candidate is
//! judged by the caller's own admission (`fits`), so the chosen configuration is the one
//! charged in the declared bound.
//!
//! Sort output is the stable sort of input order for any run size, fan-in or worker count
//! (spill/sort.rs), so this plan changes resources only, never records or archive bytes.
use crate::spill::SortConfig;
use serde::Serialize;

/// Fan-ins tried in order. 256 keeps a merge's open run files well below common
/// per-process descriptor limits.
pub(crate) const FAN_INS: [usize; 3] = [64, 128, 256];

/// One sort inside capture: its record count and resident record size (`size_of`, which
/// sets how many records a run buffer holds).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct SortLoad {
    pub records: u64,
    pub record_bytes: u64,
}

/// What the plan chose and why; reported in the capture diagnostics.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub struct SortPlan {
    /// `R`: each live sorter's run buffer.
    pub run_bytes: u64,
    /// `F`: runs merged at once.
    pub fan_in: u64,
    /// `b`: buffer per open run reader and writer.
    pub buffer_bytes: u64,
    /// `w`: background run writers per sorter (0: runs are written by the producing thread).
    pub writers: u64,
    /// Initial runs of the capture sort with the most (the largest per-record load).
    pub max_runs: u64,
    /// Every capture sort merges once, in its final stream (`max_runs ≤ F`).
    pub single_pass: bool,
    /// `floor`: the request's configuration (nothing larger fits); `single_pass`: a one-pass
    /// configuration, its runs grown to the smallest one-pass size and writers added as the
    /// budget admits; `budget`: runs grown as far as the budget admits, still more than one
    /// pass; `unknown_memory` or
    /// `memory_backed_scratch`: kept at the floor because the budget cannot be planned against
    /// (no detected memory) or the spill itself is charged to memory.
    pub limited_by: &'static str,
}

impl SortPlan {
    /// The planned configuration of one live sorter with `jobs` sorting threads.
    pub(crate) fn sorter(&self, jobs: usize) -> SortConfig {
        SortConfig {
            run_bytes: self.run_bytes as usize,
            fan_in: self.fan_in as usize,
            buffer_bytes: self.buffer_bytes as usize,
            jobs,
            writers: self.writers as usize,
        }
    }
}

/// Run buffer bytes at which every load has at most `fan_in` runs.
fn one_pass_run_bytes(loads: &[SortLoad], fan_in: u64) -> Option<u64> {
    loads.iter().try_fold(0u64, |most, load| {
        let run_records = load.records.div_ceil(fan_in).max(1);
        Some(most.max(run_records.checked_mul(load.record_bytes)?))
    })
}

/// Initial runs of the most-loaded sort with `run_bytes` buffers.
fn max_runs(loads: &[SortLoad], run_bytes: u64) -> u64 {
    loads
        .iter()
        .map(|load| {
            let run_records = (run_bytes / load.record_bytes.max(1)).max(1);
            load.records.div_ceil(run_records)
        })
        .max()
        .unwrap_or(0)
}

/// A per-sorter configuration as the shared [`crate::mesh_large::config::TreeConfig::sort`]
/// form, whose `run_bytes` is split between the two sorters live at once.
fn shared(floor: SortConfig, run_bytes: u64, fan_in: usize, writers: usize) -> Option<SortConfig> {
    Some(SortConfig {
        run_bytes: usize::try_from(run_bytes.checked_mul(2)?).ok()?,
        fan_in,
        writers,
        ..floor
    })
}

/// Resolves capture's sort configuration. `floor` is the request's shared configuration
/// (`run_bytes` split between two live sorters); `loads` are the sorts inside capture;
/// `fits` is the caller's admission of a candidate shared configuration. `blocked` names a
/// reason the plan must stay at the floor; `max_writers` caps the background run writers;
/// `fan_ins` are the fan-ins tried, ascending ([`FAN_INS`] for capture).
/// Returns the shared configuration and its report.
pub(crate) fn plan(
    floor: SortConfig,
    loads: &[SortLoad],
    blocked: Option<&'static str>,
    max_writers: usize,
    fan_ins: &[usize],
    fits: impl Fn(SortConfig) -> bool,
) -> (SortConfig, SortPlan) {
    let floor_run = (floor.run_bytes / 2).max(1) as u64;
    let report = |config: SortConfig, limited_by| {
        let run_bytes = (config.run_bytes / 2).max(1) as u64;
        let runs = max_runs(loads, run_bytes);
        SortPlan {
            run_bytes,
            fan_in: config.fan_in as u64,
            buffer_bytes: config.buffer_bytes as u64,
            writers: config.writers as u64,
            max_runs: runs,
            single_pass: runs <= config.fan_in as u64,
            limited_by,
        }
    };
    let floor_single = max_runs(loads, floor_run) <= floor.fan_in as u64;
    if let Some(reason) = blocked {
        return (floor, report(floor, reason));
    }
    // One-pass candidates: the smallest one-pass run (never below the floor's) at each
    // fan-in, with as many writers as fit; the most writers wins, then the smaller fan-in.
    let mut best: Option<SortConfig> = None;
    for fan_in in fan_ins.iter().copied().filter(|&f| f >= floor.fan_in) {
        let Some(run) = one_pass_run_bytes(loads, fan_in as u64).map(|run| run.max(floor_run))
        else {
            continue;
        };
        let at = |writers| shared(floor, run, fan_in, writers).filter(|&c| fits(c));
        let Some(mut found) = at(0) else {
            continue;
        };
        // A sort of `n` runs hands at most `n - 1` of them to writers (the last is written
        // as the input ends), so more would be charged and never used.
        let useful = usize::try_from(max_runs(loads, run).saturating_sub(1)).unwrap_or(usize::MAX);
        let max_writers = max_writers.min(useful);
        // The admission is monotone in the writer count.
        let (mut lo, mut hi) = (0, max_writers);
        while lo < hi {
            let mid = lo + (hi - lo).div_ceil(2);
            match at(mid) {
                Some(candidate) => {
                    found = candidate;
                    lo = mid;
                }
                None => hi = mid - 1,
            }
        }
        if best.is_none_or(|b| found.writers > b.writers) {
            best = Some(found);
        }
        if found.writers == max_writers {
            break;
        }
    }
    if let Some(best) = best {
        let limited_by = if best == floor {
            "floor"
        } else {
            "single_pass"
        };
        return (best, report(best, limited_by));
    }
    if floor_single {
        return (floor, report(floor, "floor"));
    }
    // Otherwise the largest run buffer that fits, at the largest fan-in that fits. The
    // admission is monotone in the run buffer, so bisect in MiB steps above the floor.
    for fan_in in fan_ins.iter().copied().rev().filter(|&f| f >= floor.fan_in) {
        let at = |run: u64| shared(floor, run, fan_in, 0).filter(|&c| fits(c));
        let Some(mut best) = at(floor_run) else {
            continue;
        };
        let top = one_pass_run_bytes(loads, fan_in as u64).unwrap_or(u64::MAX / 4);
        let (mut lo, mut hi) = (0u64, top.saturating_sub(floor_run) >> 20);
        while lo < hi {
            let mid = lo + (hi - lo).div_ceil(2);
            match at(floor_run + (mid << 20)) {
                Some(candidate) => {
                    best = candidate;
                    lo = mid;
                }
                None => hi = mid - 1,
            }
        }
        if best == floor {
            break;
        }
        return (best, report(best, "budget"));
    }
    (floor, report(floor, "floor"))
}

#[cfg(test)]
mod tests {
    use super::*;

    const FLOOR: SortConfig = SortConfig {
        run_bytes: 160 << 20,
        fan_in: 64,
        buffer_bytes: 1 << 20,
        jobs: 1,
        writers: 0,
    };

    /// A memory model shaped like capture's: two sorters at `SortConfig::memory_bound`'s
    /// run and buffer terms.
    fn cost(config: SortConfig) -> u64 {
        let run = (config.run_bytes / 2) as u64;
        let w = config.writers as u64;
        let runs = if w == 0 {
            run + run / 2
        } else {
            run + w * (run + run / 2)
        };
        2 * (runs + (config.fan_in as u64 + 1 + w) * config.buffer_bytes as u64)
    }

    fn loads(triangles: u64) -> Vec<SortLoad> {
        vec![
            SortLoad {
                records: 3 * triangles,
                record_bytes: 112,
            },
            SortLoad {
                records: triangles,
                record_bytes: 304,
            },
        ]
    }

    #[test]
    fn small_inputs_keep_the_floor_run_and_add_the_writers_that_fit() {
        let (config, report) = plan(FLOOR, &loads(1_000_000), None, 0, &FAN_INS, |_| true);
        assert_eq!(config, FLOOR);
        assert_eq!((report.limited_by, report.single_pass), ("floor", true));
        let (config, report) = plan(FLOOR, &loads(1_000_000), None, 7, &FAN_INS, |c| {
            cost(c) <= 1 << 30
        });
        assert_eq!(config.run_bytes, FLOOR.run_bytes);
        assert_eq!((config.fan_in, config.writers), (64, 3));
        assert!(
            cost(SortConfig {
                writers: 4,
                ..config
            }) > 1 << 30
        );
        assert_eq!((report.limited_by, report.writers), ("single_pass", 3));
        // Writers beyond the runs they could take are never charged: 3M corners of 112
        // bytes fill 5 runs of 80 MiB, so at most 4 writers, whatever the budget.
        let (config, report) = plan(FLOOR, &loads(1_000_000), None, 7, &FAN_INS, |_| true);
        assert_eq!((report.max_runs, config.writers), (5, 4));
        let (config, _) = plan(FLOOR, &loads(100_000), None, 7, &FAN_INS, |_| true);
        assert_eq!(config, FLOOR, "one run: no writer");
    }

    #[test]
    fn runs_grow_to_exactly_one_pass_when_the_budget_allows() {
        let loads = loads(828_000_000);
        let (config, report) = plan(FLOOR, &loads, None, 0, &FAN_INS, |c| cost(c) <= 24 << 30);
        assert_eq!((report.limited_by, report.fan_in), ("single_pass", 64));
        assert!(report.single_pass && report.max_runs <= 64);
        // The smallest such run: one record fewer would need a 65th run.
        let run = report.run_bytes;
        assert_eq!(run, 3 * 828_000_000u64.div_ceil(64) * 112);
        assert!(max_runs(&loads, run - 112) > 64);
        assert_eq!(config.run_bytes as u64, 2 * run);
    }

    #[test]
    fn fan_in_rises_when_a_larger_run_does_not_fit_or_admits_more_writers() {
        let loads = loads(828_000_000);
        let (_, at_8) = plan(FLOOR, &loads, None, 0, &FAN_INS, |c| cost(c) <= 8 << 30);
        assert_eq!((at_8.limited_by, at_8.fan_in), ("single_pass", 128));
        let (_, at_4) = plan(FLOOR, &loads, None, 0, &FAN_INS, |c| cost(c) <= 4 << 30);
        assert_eq!((at_4.limited_by, at_4.fan_in), ("single_pass", 256));
        assert!(at_4.max_runs <= 256);
        // With writers allowed, the fan-in that admits the most of them wins: at 24 GiB,
        // F = 64 admits 1, F = 128 admits 3 and F = 256 all 7.
        let (config, report) = plan(FLOOR, &loads, None, 7, &FAN_INS, |c| cost(c) <= 24 << 30);
        assert_eq!((report.fan_in, report.writers), (256, 7));
        assert!(report.single_pass && cost(config) <= 24 << 30);
        let (_, report) = plan(FLOOR, &loads, None, 2, &FAN_INS, |c| cost(c) <= 24 << 30);
        assert_eq!(
            (report.fan_in, report.writers),
            (128, 2),
            "a tie keeps the smaller F"
        );
    }

    #[test]
    fn a_tight_budget_takes_the_largest_run_that_fits_and_never_goes_below_the_floor() {
        let loads = loads(828_000_000);
        let limit = 2 << 30;
        let (config, report) = plan(FLOOR, &loads, None, 7, &FAN_INS, |c| cost(c) <= limit);
        assert_eq!(
            (report.limited_by, report.fan_in, report.writers),
            ("budget", 256, 0)
        );
        assert!(!report.single_pass);
        assert!(cost(config) <= limit);
        let more = SortConfig {
            run_bytes: config.run_bytes + (2 << 20),
            ..config
        };
        assert!(
            cost(more) > limit,
            "within one MiB of the largest fitting run"
        );
        // Nothing above the floor fits: the floor, unchanged.
        let (config, report) = plan(FLOOR, &loads, None, 7, &FAN_INS, |c| c == FLOOR);
        assert_eq!((config, report.limited_by), (FLOOR, "floor"));
        // Blocked plans never grow.
        let (config, report) = plan(FLOOR, &loads, Some("unknown_memory"), 7, &FAN_INS, |_| true);
        assert_eq!((config, report.limited_by), (FLOOR, "unknown_memory"));
    }
}
