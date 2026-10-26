//! Named progress phases inside a mesh run, and their wall times.
//!
//! The `capture` progress event used to be the last one until the leaves and coarse tiles
//! had all finished. These phases report the work inside it as ordinary progress events
//! (`phase`, `done`, `total`, unchanged schema), and each finished phase as a `log` event
//! naming its wall seconds and the run's elapsed seconds. The same seconds go to the run
//! diagnostics. Diagnostics only: nothing here reaches the archive.
use crate::{runtime::RunEvent, JobError};
use serde::Serialize;
use std::{sync::Mutex, time::Instant};

/// A named phase of a mesh run, in run order.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Phase {
    Capture,
    /// H8: full-detail leaves (atlas pages, encoding, writing).
    LeafBuild,
    /// HLOD: the spillable source-incidence pass.
    HlodIncidence,
    /// HLOD: the bounded postorder walk (simplification, encoding, checks, writes).
    HlodGeometry,
    /// H16: hierarchy and `.3tz` archive writing.
    Archive,
}
impl Phase {
    pub(crate) fn name(self) -> &'static str {
        match self {
            Phase::Capture => "capture",
            Phase::LeafBuild => "leaf_build",
            Phase::HlodIncidence => "hlod_incidence",
            Phase::HlodGeometry => "hlod_geometry",
            Phase::Archive => "archive",
        }
    }
}

/// Wall seconds of each phase.
#[derive(Clone, Debug, Default, Serialize)]
pub struct PhaseSeconds {
    pub capture: f64,
    pub leaf_build: f64,
    pub hlod_incidence: f64,
    pub hlod_geometry: f64,
    pub archive: f64,
}
impl PhaseSeconds {
    fn slot(&mut self, phase: Phase) -> &mut f64 {
        match phase {
            Phase::Capture => &mut self.capture,
            Phase::LeafBuild => &mut self.leaf_build,
            Phase::HlodIncidence => &mut self.hlod_incidence,
            Phase::HlodGeometry => &mut self.hlod_geometry,
            Phase::Archive => &mut self.archive,
        }
    }
}

type Emit<'a> = dyn Fn(&RunEvent<'_>) -> Result<(), JobError> + Sync + 'a;

/// The run's phase reporter. Calls may come from worker threads; events are emitted
/// under one lock, so a phase's `done` counts arrive in order.
pub(crate) struct Phases<'a> {
    emit: Option<&'a Emit<'a>>,
    started: Instant,
    seconds: Mutex<PhaseSeconds>,
}
impl<'a> Phases<'a> {
    pub(crate) fn new(emit: &'a Emit<'a>) -> Self {
        Self::with(Some(emit))
    }
    /// Records times; emits nothing.
    pub(crate) fn silent() -> Phases<'static> {
        Phases::with(None)
    }
    fn with(emit: Option<&'a Emit<'a>>) -> Self {
        Self {
            emit,
            started: Instant::now(),
            seconds: Mutex::new(PhaseSeconds::default()),
        }
    }
    fn lock(&self) -> std::sync::MutexGuard<'_, PhaseSeconds> {
        // A panicking emitter cannot leave the counters inconsistent.
        self.seconds.lock().unwrap_or_else(|e| e.into_inner())
    }
    fn send(&self, event: &RunEvent<'_>) -> Result<(), JobError> {
        self.emit.map_or(Ok(()), |emit| emit(event))
    }
    /// A progress event. With a known total, at most about 200 per phase are emitted:
    /// the first, the last, and evenly spaced ones between.
    pub(crate) fn progress(
        &self,
        phase: Phase,
        done: u64,
        total: Option<u64>,
    ) -> Result<(), JobError> {
        let _serial = self.lock();
        if let Some(total) = total {
            let step = (total / 200).max(1);
            if done != 0 && done != total && !done.is_multiple_of(step) {
                return Ok(());
            }
        }
        self.send(&RunEvent::Progress {
            phase: phase.name(),
            done,
            total,
        })
    }
    /// A finished phase: add its wall seconds and log them.
    pub(crate) fn finished(&self, phase: Phase, seconds: f64) -> Result<(), JobError> {
        let total = {
            let mut state = self.lock();
            let slot = state.slot(phase);
            *slot += seconds;
            *slot
        };
        self.note(&format!(
            "mesh phase {}: {seconds:.3} s (phase total {total:.3} s, run elapsed {:.3} s)",
            phase.name(),
            self.elapsed()
        ))
    }
    pub(crate) fn note(&self, message: &str) -> Result<(), JobError> {
        self.send(&RunEvent::Note { message })
    }
    pub(crate) fn elapsed(&self) -> f64 {
        self.started.elapsed().as_secs_f64()
    }
    pub(crate) fn seconds(&self) -> PhaseSeconds {
        self.lock().clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn progress_is_thinned_but_keeps_first_and_last_and_finished_phases_log() {
        let events = Mutex::new(Vec::new());
        let emit = |e: &RunEvent<'_>| {
            events.lock().unwrap().push(match e {
                RunEvent::Progress { phase, done, total } => format!("{phase} {done} {total:?}"),
                RunEvent::Note { message } => message.to_string(),
                RunEvent::Warning { message, .. } => message.to_string(),
            });
            Ok(())
        };
        let phases = Phases::new(&emit);
        for done in 0..=1000 {
            phases.progress(Phase::LeafBuild, done, Some(1000)).unwrap();
        }
        phases.progress(Phase::Archive, 0, None).unwrap();
        phases.finished(Phase::HlodGeometry, 1.5).unwrap();
        phases.finished(Phase::HlodGeometry, 0.5).unwrap();
        let seconds = phases.seconds();
        let events = events.into_inner().unwrap();
        let leaf: Vec<_> = events
            .iter()
            .filter(|e| e.starts_with("leaf_build"))
            .collect();
        assert_eq!(leaf.len(), 201);
        assert_eq!(leaf[0], "leaf_build 0 Some(1000)");
        assert_eq!(leaf[200], "leaf_build 1000 Some(1000)");
        assert!(events.contains(&"archive 0 None".to_string()));
        assert!(events
            .iter()
            .any(|e| e.starts_with("mesh phase hlod_geometry: 0.500 s (phase total 2.000 s")));
        assert_eq!(seconds.hlod_geometry, 2.);
        // A silent reporter still records.
        let silent = Phases::silent();
        silent.finished(Phase::Archive, 3.).unwrap();
        assert_eq!(silent.seconds().archive, 3.);
    }
}
