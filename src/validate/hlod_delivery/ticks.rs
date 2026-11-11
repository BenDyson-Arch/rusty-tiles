//! Progress and cancellation checkpoints of the delivery stage (#342).
//!
//! The stage reports one `validate.delivery` phase whose `done` counts the passes started of
//! [`PASSES`], and one phase per geometry replay (`validate.delivery.leaves`, `.edges`,
//! `.coarse`) whose `done` counts the placements replayed of the placements to replay, with an
//! event every 1,024 placements. Every charged row is a cancellation checkpoint; every 65,536
//! of them re-emit the current event, so a Python callback's signal check runs at that
//! interval too. Long SQL statements are
//! checkpointed by SQLite's progress handler ([`Heartbeat`]). Events never change a verdict.
use super::{super::run_failure, Error};
use crate::runtime::AttemptProbe;
use std::{
    cell::Cell,
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        Arc,
    },
};

/// Passes of the delivery stage, in `finish_inner` order (the `validate.delivery` total).
pub(super) const PASSES: u64 = 20;
pub(super) const DELIVERY: &str = "validate.delivery";
/// Replayed placements, and SQLite progress calls (10,000 operations each), per event.
const EVERY: u64 = 1024;
/// Charged rows per heartbeat event; every row is still a cancellation checkpoint.
const EVERY_CHECK: u64 = 1 << 16;

/// The stage's event state; without a run control every call is a no-op.
#[derive(Default)]
pub(super) struct Ticks {
    probe: Option<Arc<AttemptProbe>>,
    phase: Cell<(&'static str, u64, Option<u64>)>,
    checks: Cell<u64>,
    /// Passes started, shared with the SQL heartbeat.
    passes: Arc<AtomicU64>,
    /// Set while a progress total is counted: its SQL is not charged as work.
    uncharged: Arc<AtomicBool>,
}

impl Ticks {
    pub(super) fn new(probe: Option<AttemptProbe>) -> Self {
        Self {
            probe: probe.map(Arc::new),
            ..Self::default()
        }
    }

    fn emit(&self) -> Result<(), Error> {
        let Some(probe) = &self.probe else {
            return Ok(());
        };
        let (phase, done, total) = self.phase.get();
        probe.check().map_err(run_failure)?;
        probe
            .emit(&crate::RunEvent::Progress { phase, done, total })
            .map_err(run_failure)
    }

    /// A checkpoint inside the current phase: every row the stage charges.
    pub(super) fn check(&self) -> Result<(), Error> {
        let Some(probe) = &self.probe else {
            return Ok(());
        };
        let checks = self.checks.get() + 1;
        self.checks.set(checks);
        if checks.is_multiple_of(EVERY_CHECK) {
            self.emit()
        } else {
            probe.check().map_err(run_failure)
        }
    }

    /// Starts the next pass: `validate.delivery` at `done` = passes started before it.
    pub(super) fn pass(&self) -> Result<(), Error> {
        let done = self.passes.fetch_add(1, Ordering::Relaxed);
        self.phase.set((DELIVERY, done.min(PASSES), Some(PASSES)));
        self.emit()
    }

    /// Starts a replay phase over `total` placements.
    pub(super) fn replay(&self, phase: &'static str, total: u64) -> Result<(), Error> {
        self.phase.set((phase, 0, Some(total)));
        self.emit()
    }

    /// One replayed placement.
    pub(super) fn step(&self) -> Result<(), Error> {
        let (phase, done, total) = self.phase.get();
        let done = done + 1;
        self.phase.set((phase, done, total));
        if done.is_multiple_of(EVERY) || Some(done) == total {
            self.emit()
        } else {
            self.check()
        }
    }

    /// The `'static` checkpoint run by SQLite's progress handler.
    pub(super) fn heartbeat(&self) -> Heartbeat {
        Heartbeat {
            probe: self.probe.clone(),
            passes: self.passes.clone(),
            uncharged: self.uncharged.clone(),
            calls: 0,
        }
    }

    /// Runs `f` with its SQL not charged as work: progress totals and the replay lookahead,
    /// so neither changes a work count or a refusal.
    pub(super) fn uncharged<T>(&self, f: impl FnOnce() -> T) -> T {
        let before = self.uncharged.swap(true, Ordering::Relaxed);
        let result = f();
        self.uncharged.store(before, Ordering::Relaxed);
        result
    }

    /// The control's state now: a cancellation or observer failure seen inside SQL wins over
    /// the statement's interrupt error.
    pub(super) fn stopped(&self) -> Option<Error> {
        let probe = self.probe.as_ref()?;
        probe.check().err().map(run_failure)
    }

    /// A progress total counted uncharged; without a run control nothing is counted.
    pub(super) fn total(&self, count: impl FnOnce() -> Result<u64, Error>) -> Result<u64, Error> {
        if self.probe.is_none() {
            return Ok(0);
        }
        self.uncharged(count)
    }
}

/// See [`Ticks::heartbeat`]: called every 10,000 SQLite VM operations; true interrupts the
/// statement. It checks the control at every call and re-emits the `validate.delivery` event
/// every 1,024 calls.
pub(super) struct Heartbeat {
    probe: Option<Arc<AttemptProbe>>,
    passes: Arc<AtomicU64>,
    uncharged: Arc<AtomicBool>,
    calls: u64,
}

impl Heartbeat {
    /// Whether the current statement's operations are charged as work.
    pub(super) fn charged(&self) -> bool {
        !self.uncharged.load(Ordering::Relaxed)
    }

    pub(super) fn interrupt(&mut self) -> bool {
        let Some(probe) = &self.probe else {
            return false;
        };
        self.calls += 1;
        if !self.calls.is_multiple_of(EVERY) {
            return probe.check().is_err();
        }
        let done = self
            .passes
            .load(Ordering::Relaxed)
            .saturating_sub(1)
            .min(PASSES);
        probe
            .emit(&crate::RunEvent::Progress {
                phase: DELIVERY,
                done,
                total: Some(PASSES),
            })
            .is_err()
    }
}
