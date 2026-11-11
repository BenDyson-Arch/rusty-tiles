//! Named progress phases inside a mesh run, and their wall times.
//!
//! The `capture` progress event used to be the last one until the leaves and coarse tiles
//! had all finished. These phases report the work inside it as ordinary progress events
//! (`phase`, `done`, `total`, unchanged schema), and each finished phase as a `log` event
//! naming its wall seconds and the run's elapsed seconds. The same seconds go to the run
//! diagnostics. Diagnostics only: nothing here reaches the archive.
use crate::{
    runtime::RunEvent,
    spill::{SortObserver, SortProgress, SpillError},
    JobError,
};
use serde::Serialize;
use std::{
    cell::Cell,
    sync::Mutex,
    time::{Duration, Instant},
};

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
    cpu: Mutex<(Option<f64>, PhaseSeconds)>,
}
impl<'a> Phases<'a> {
    pub(crate) fn new(emit: &'a Emit<'a>) -> Self {
        Self::with(Some(emit))
    }
    #[cfg_attr(not(test), allow(dead_code))] // Retained for bounded unit-test fixtures; production uses the streamed facade.
    /// Records times; emits nothing.
    pub(crate) fn silent() -> Phases<'static> {
        Phases::with(None)
    }
    fn with(emit: Option<&'a Emit<'a>>) -> Self {
        Self {
            emit,
            started: Instant::now(),
            seconds: Mutex::new(PhaseSeconds::default()),
            cpu: Mutex::new((
                super::resources::process_cpu_seconds(),
                PhaseSeconds::default(),
            )),
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
        // Process CPU includes all workers and the admission work between phases.
        let now = super::resources::process_cpu_seconds();
        {
            let mut cpu = self.cpu.lock().unwrap_or_else(|e| e.into_inner());
            if let (Some(start), Some(end)) = (cpu.0, now) {
                *cpu.1.slot(phase) += (end - start).max(0.);
            }
            cpu.0 = now;
        }

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
    pub(crate) fn cpu_seconds(&self) -> Option<PhaseSeconds> {
        let cpu = self.cpu.lock().unwrap_or_else(|e| e.into_inner());
        cpu.0.map(|_| cpu.1.clone())
    }
    pub(crate) fn seconds(&self) -> PhaseSeconds {
        self.lock().clone()
    }
}

/// A step inside the capture phase. Each is reported as an ordinary progress phase whose
/// total is known before it starts: events at `0`, at the total, and between them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum CaptureStep {
    /// Preflight scan of the selected triangles.
    Scan,
    /// Accessor payload validation before any spill (one unit).
    Validate,
    /// Corner references read from the index accessors and sorted into runs.
    Corners,
    /// Merged corner references joined with their vertex attributes and sorted into runs.
    Positions,
    /// Merged positioned corners reassembled into triangles and sorted into runs.
    Triangles,
    /// Merged triangles written as the retained rank-ordered sequence.
    Sequence,
    /// An intermediate merge pass of one of the sorts (records rewritten).
    Merge,
    /// Image snapshots and admission decodes (two units per image).
    Images,
    /// The cell tree over the retained triangles.
    Tree,
    /// The weld-depth join over every corner.
    CornerDepths,
    /// Leaf planning over the tree's leaves.
    LeafPlans,
}
impl CaptureStep {
    pub(crate) fn name(self) -> &'static str {
        match self {
            Self::Scan => "capture_scan",
            Self::Validate => "capture_validate",
            Self::Corners => "capture_corners",
            Self::Positions => "capture_positions",
            Self::Triangles => "capture_triangles",
            Self::Sequence => "capture_sequence",
            Self::Merge => "capture_merge",
            Self::Images => "capture_images",
            Self::Tree => "capture_tree",
            Self::CornerDepths => "capture_corner_depths",
            Self::LeafPlans => "capture_leaf_plans",
        }
    }
}

/// Longest interval between capture events while capture is checked.
pub(crate) const CAPTURE_HEARTBEAT: Duration = Duration::from_millis(250);

#[derive(Clone, Copy)]
struct StepState {
    step: CaptureStep,
    done: u64,
    total: u64,
    last: Instant,
}

/// Capture's progress reporter and cancellation checkpoint, driven from the capture thread.
///
/// [`CaptureProgress::check`] is capture's only cancellation check. It also re-emits the
/// current step's latest count whenever [`CAPTURE_HEARTBEAT`] has passed since the last
/// event, so an observer that checks for interruption (the Python callback) runs at least
/// that often wherever capture checks, including inside sort run writes and merge passes.
/// Diagnostics only: nothing here reaches the archive.
pub(crate) struct CaptureProgress<'a> {
    check: &'a (dyn Fn() -> Result<(), JobError> + 'a),
    emit: Option<&'a Emit<'a>>,
    state: Cell<Option<StepState>>,
    /// The step a sort's merge pass interrupted, resumed when the pass ends.
    suspended: Cell<Option<StepState>>,
}
impl<'a> CaptureProgress<'a> {
    pub(crate) fn new(
        check: &'a (dyn Fn() -> Result<(), JobError> + 'a),
        emit: Option<&'a Emit<'a>>,
    ) -> Self {
        Self {
            check,
            emit,
            state: Cell::new(None),
            suspended: Cell::new(None),
        }
    }
    fn send(&self, state: &mut StepState) -> Result<(), JobError> {
        state.last = Instant::now();
        self.emit.map_or(Ok(()), |emit| {
            emit(&RunEvent::Progress {
                phase: state.step.name(),
                done: state.done,
                total: Some(state.total),
            })
        })
    }
    /// Starts `step` of `total` units: checks, then emits its `0` event.
    pub(crate) fn begin(&self, step: CaptureStep, total: u64) -> Result<(), JobError> {
        (self.check)()?;
        let mut state = StepState {
            step,
            done: 0,
            total,
            last: Instant::now(),
        };
        let sent = self.send(&mut state);
        self.state.set(Some(state));
        sent
    }
    /// Records `done` units of the current step and checks.
    pub(crate) fn advance(&self, done: u64) -> Result<(), JobError> {
        if let Some(mut state) = self.state.get() {
            state.done = done.min(state.total);
            self.state.set(Some(state));
        }
        self.check()
    }
    /// Checks for cancellation, emitting a heartbeat of the current step when one is due.
    pub(crate) fn check(&self) -> Result<(), JobError> {
        (self.check)()?;
        match self.state.get() {
            Some(mut state) if state.last.elapsed() >= CAPTURE_HEARTBEAT => {
                let sent = self.send(&mut state);
                self.state.set(Some(state));
                sent
            }
            _ => Ok(()),
        }
    }
    /// Completes the current step: emits its total.
    pub(crate) fn end(&self) -> Result<(), JobError> {
        (self.check)()?;
        match self.state.take() {
            Some(mut state) => {
                state.done = state.total;
                self.send(&mut state)
            }
            None => Ok(()),
        }
    }
}
#[cfg(test)]
impl CaptureProgress<'static> {
    /// Never cancelled; emits nothing. Leaked so fixtures can borrow it for any scratch.
    pub(crate) fn silent() -> &'static Self {
        Box::leak(Box::new(Self::new(&|| Ok(()), None)))
    }
}
impl SortObserver for CaptureProgress<'_> {
    /// Run writes are heartbeats of the current step. A merge pass is its own step; the step
    /// it interrupted (if any) resumes when the pass ends.
    fn checkpoint(&self, progress: SortProgress) -> Result<(), SpillError> {
        let stop = |error: JobError| SpillError::Stopped(Box::new(error));
        match progress {
            SortProgress::Wrote => self.check().map_err(stop),
            SortProgress::MergeStart { total } => {
                self.suspended.set(self.state.take());
                self.begin(CaptureStep::Merge, total).map_err(stop)
            }
            SortProgress::Merged { done } => self.advance(done).map_err(stop),
            SortProgress::MergeEnd => {
                let ended = self.end().map_err(stop);
                self.state.set(self.suspended.take());
                ended
            }
        }
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

    type Recorded = std::sync::Arc<Mutex<Vec<String>>>;

    fn recorder() -> (
        Recorded,
        impl Fn(&RunEvent<'_>) -> Result<(), JobError> + Sync,
    ) {
        let events = std::sync::Arc::new(Mutex::new(Vec::new()));
        let sink = events.clone();
        let emit = move |e: &RunEvent<'_>| {
            if let RunEvent::Progress { phase, done, total } = e {
                sink.lock()
                    .unwrap()
                    .push(format!("{phase} {done}/{}", total.unwrap()));
            }
            Ok(())
        };
        (events, emit)
    }

    #[test]
    fn capture_steps_report_totals_and_heartbeats_bound_the_event_interval() {
        let (events, emit) = recorder();
        let checks = std::sync::atomic::AtomicU64::new(0);
        let check = || {
            checks.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            Ok(())
        };
        let progress = CaptureProgress::new(&check, Some(&emit));
        progress.begin(CaptureStep::Corners, 30).unwrap();
        // Checks within the heartbeat interval emit nothing.
        for done in 0..10 {
            progress.advance(done).unwrap();
        }
        assert_eq!(events.lock().unwrap().len(), 1);
        std::thread::sleep(CAPTURE_HEARTBEAT + Duration::from_millis(20));
        progress.advance(12).unwrap();
        // A plain check re-emits the latest count once the interval has passed again.
        std::thread::sleep(CAPTURE_HEARTBEAT + Duration::from_millis(20));
        progress.check().unwrap();
        progress.end().unwrap();
        // Outside a step nothing is emitted, but the check still runs.
        std::thread::sleep(CAPTURE_HEARTBEAT + Duration::from_millis(20));
        let before = checks.load(std::sync::atomic::Ordering::Relaxed);
        progress.check().unwrap();
        assert_eq!(
            checks.load(std::sync::atomic::Ordering::Relaxed),
            before + 1
        );
        assert_eq!(
            *events.lock().unwrap(),
            [
                "capture_corners 0/30",
                "capture_corners 12/30",
                "capture_corners 12/30",
                "capture_corners 30/30"
            ]
        );
    }

    #[test]
    fn a_merge_pass_is_its_own_step_and_resumes_the_step_it_interrupted() {
        let (events, emit) = recorder();
        let progress = CaptureProgress::new(&|| Ok(()), Some(&emit));
        progress.begin(CaptureStep::CornerDepths, 70).unwrap();
        progress.advance(30).unwrap();
        let sort: &dyn SortObserver = &progress;
        sort.checkpoint(SortProgress::MergeStart { total: 500 })
            .unwrap();
        sort.checkpoint(SortProgress::Merged { done: 100 }).unwrap();
        sort.checkpoint(SortProgress::Wrote).unwrap();
        sort.checkpoint(SortProgress::MergeEnd).unwrap();
        std::thread::sleep(CAPTURE_HEARTBEAT + Duration::from_millis(20));
        progress.advance(40).unwrap();
        progress.end().unwrap();
        assert_eq!(
            *events.lock().unwrap(),
            [
                "capture_corner_depths 0/70",
                "capture_merge 0/500",
                "capture_merge 500/500",
                "capture_corner_depths 40/70",
                "capture_corner_depths 70/70"
            ]
        );
    }

    #[test]
    fn a_failing_check_stops_a_sort_with_its_own_error() {
        let cancelled = || {
            Err(JobError::new(
                crate::JobErrorKind::Cancelled,
                "operation cancelled",
            ))
        };
        let progress = CaptureProgress::new(&cancelled, None);
        let error = SortObserver::checkpoint(&progress, SortProgress::Wrote).unwrap_err();
        let error: JobError = error.into();
        assert_eq!(error.kind(), crate::JobErrorKind::Cancelled);
    }
}
