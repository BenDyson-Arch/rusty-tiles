//! Named progress phases inside a mesh run, and their wall times.
//!
//! The `capture` progress event used to be the last one until the leaves, LOD and proxy
//! bake had all finished. These phases report the work inside it as ordinary progress
//! events (`phase`, `done`, `total`, unchanged schema), and each finished phase as a
//! `log` event naming its wall seconds and the run's elapsed seconds. The same seconds,
//! and the proxy bake's per-step worker seconds and appearance fallbacks, go to the run
//! diagnostics. Diagnostics only: nothing here reaches the archive.
use super::bake::BakeStats;
use crate::{runtime::RunEvent, JobError};
use serde::Serialize;
use std::{sync::Mutex, time::Instant};

/// A named phase of a mesh run, in run order.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Phase {
    Capture,
    /// H8: full-detail leaves (atlas pages, encoding, writing).
    LeafBuild,
    /// H17: the source incidence pre-pass.
    LodPrepass,
    /// H17: the topology pass of every coarse tile (children, islands, simplification).
    LodSimplify,
    /// H17: the per-depth seam audit and consensus re-runs (#236).
    SeamResolution,
    /// H17: the finishing pass (certification, fills, encoding, handoff writes).
    LodFinish,
    /// H27: proxy chart building, texture bake, page encoding and member commits.
    ProxyBake,
    /// H16: hierarchy and `.3tz` archive writing.
    Archive,
}
impl Phase {
    pub(crate) fn name(self) -> &'static str {
        match self {
            Phase::Capture => "capture",
            Phase::LeafBuild => "leaf_build",
            Phase::LodPrepass => "lod_prepass",
            Phase::LodSimplify => "lod_simplify",
            Phase::SeamResolution => "seam_resolution",
            Phase::LodFinish => "lod_finish",
            Phase::ProxyBake => "proxy_bake",
            Phase::Archive => "archive",
        }
    }
}

/// Wall seconds of each phase (summed over depths for the LOD phases).
#[derive(Clone, Debug, Default, Serialize)]
pub struct PhaseSeconds {
    pub capture: f64,
    pub leaf_build: f64,
    pub lod_prepass: f64,
    pub lod_simplify: f64,
    pub seam_resolution: f64,
    pub lod_finish: f64,
    pub proxy_bake: f64,
    pub archive: f64,
}
impl PhaseSeconds {
    fn slot(&mut self, phase: Phase) -> &mut f64 {
        match phase {
            Phase::Capture => &mut self.capture,
            Phase::LeafBuild => &mut self.leaf_build,
            Phase::LodPrepass => &mut self.lod_prepass,
            Phase::LodSimplify => &mut self.lod_simplify,
            Phase::SeamResolution => &mut self.seam_resolution,
            Phase::LodFinish => &mut self.lod_finish,
            Phase::ProxyBake => &mut self.proxy_bake,
            Phase::Archive => &mut self.archive,
        }
    }
}

/// The proxy bake's totals. Step seconds are summed over workers (they overlap), and
/// include every density attempt.
#[derive(Clone, Debug, Default, Serialize)]
pub struct ProxyBakeDiagnostics {
    pub tiles: u64,
    /// Tiles that refused their worker's share of the bake memory and were baked again
    /// alone, after their depth, with the whole allowance (serial time).
    pub retried_tiles: u64,
    pub charts_worker_seconds: f64,
    pub raster_worker_seconds: f64,
    pub encode_worker_seconds: f64,
    pub assemble_worker_seconds: f64,
    /// Parent packet encoding, member commit and child release.
    pub commit_worker_seconds: f64,
    /// Proxy texels whose source witness faces away from the proxy because no triangle
    /// of the face's own source primitive faces it, the faces with any such texel, and
    /// the tiles with any such face.
    pub facing_fallback_samples: u64,
    pub facing_fallback_faces: u64,
    pub facing_fallback_tiles: u64,
    /// Fill texels beyond their certified facing bound (the fill fallback).
    pub fill_fallback_samples: u64,
}

type Emit<'a> = dyn Fn(&RunEvent<'_>) -> Result<(), JobError> + Sync + 'a;

struct State {
    seconds: PhaseSeconds,
    bake: ProxyBakeDiagnostics,
    bake_total: u64,
}

/// The run's phase reporter. Calls may come from worker threads; events are emitted
/// under one lock, so a phase's `done` counts arrive in order.
pub(crate) struct Phases<'a> {
    emit: Option<&'a Emit<'a>>,
    started: Instant,
    state: Mutex<State>,
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
            state: Mutex::new(State {
                seconds: PhaseSeconds::default(),
                bake: ProxyBakeDiagnostics::default(),
                bake_total: 0,
            }),
        }
    }
    fn lock(&self) -> std::sync::MutexGuard<'_, State> {
        // A panicking emitter cannot leave the counters inconsistent.
        self.state.lock().unwrap_or_else(|e| e.into_inner())
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
        self.progress_locked(phase, done, total)
    }
    fn progress_locked(&self, phase: Phase, done: u64, total: Option<u64>) -> Result<(), JobError> {
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
        self.finished_part(phase, None, seconds)
    }
    /// One finished LOD depth of a phase.
    pub(crate) fn finished_depth(
        &self,
        phase: Phase,
        depth: u32,
        seconds: f64,
    ) -> Result<(), JobError> {
        self.finished_part(phase, Some(depth), seconds)
    }
    fn finished_part(
        &self,
        phase: Phase,
        depth: Option<u32>,
        seconds: f64,
    ) -> Result<(), JobError> {
        let total = {
            let mut state = self.lock();
            let slot = state.seconds.slot(phase);
            *slot += seconds;
            *slot
        };
        let part = depth.map_or(String::new(), |d| format!(" depth {d}"));
        self.note(&format!(
            "mesh phase {}{part}: {seconds:.3} s (phase total {total:.3} s, run elapsed {:.3} s)",
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
    /// The proxy bake starts over `total` tiles.
    pub(crate) fn begin_bake(&self, total: u64) -> Result<(), JobError> {
        let mut state = self.lock();
        state.bake_total = total;
        state.bake.tiles = 0;
        self.progress_locked(Phase::ProxyBake, 0, Some(total))
    }
    /// One baked tile.
    pub(crate) fn baked_tile(
        &self,
        stats: &BakeStats,
        commit_seconds: f64,
    ) -> Result<(), JobError> {
        let mut state = self.lock();
        let b = &mut state.bake;
        b.tiles += 1;
        b.charts_worker_seconds += stats.charts_seconds;
        b.raster_worker_seconds += stats.raster_seconds;
        b.encode_worker_seconds += stats.encode_seconds;
        b.assemble_worker_seconds += stats.assemble_seconds;
        b.commit_worker_seconds += commit_seconds;
        b.facing_fallback_samples += stats.facing_fallback_samples;
        b.facing_fallback_faces += stats.facing_fallback_faces;
        b.facing_fallback_tiles += u64::from(stats.facing_fallback_faces > 0);
        b.fill_fallback_samples += stats.fill_fallback_samples;
        let (done, total) = (b.tiles, state.bake_total);
        self.progress_locked(Phase::ProxyBake, done, Some(total.max(done)))
    }
    /// The bake totals as one log line.
    pub(crate) fn bake_summary(&self) -> Result<(), JobError> {
        let b = self.bake();
        self.note(&format!(
            "mesh proxy bake: {} tiles ({} retried alone); worker seconds charts {:.3}, \
             raster {:.3}, encode {:.3}, assemble {:.3}, commit {:.3}; facing fallback {} \
             samples on {} faces in {} tiles; fill fallback {} samples",
            b.tiles,
            b.retried_tiles,
            b.charts_worker_seconds,
            b.raster_worker_seconds,
            b.encode_worker_seconds,
            b.assemble_worker_seconds,
            b.commit_worker_seconds,
            b.facing_fallback_samples,
            b.facing_fallback_faces,
            b.facing_fallback_tiles,
            b.fill_fallback_samples
        ))
    }
    /// One finished bake depth: its tiles, how many were retried alone, wall seconds.
    pub(crate) fn baked_depth(
        &self,
        depth: u32,
        tiles: u64,
        retried: u64,
        seconds: f64,
    ) -> Result<(), JobError> {
        let (faces, fallback_tiles) = {
            let mut state = self.lock();
            state.bake.retried_tiles += retried;
            (
                state.bake.facing_fallback_faces,
                state.bake.facing_fallback_tiles,
            )
        };
        self.note(&format!(
            "mesh proxy bake depth {depth}: {tiles} tiles ({retried} retried alone with the \
             whole allowance) in {seconds:.3} s (run elapsed {:.3} s); facing fallback so far \
             on {faces} faces in {fallback_tiles} tiles",
            self.elapsed()
        ))
    }
    pub(crate) fn seconds(&self) -> PhaseSeconds {
        self.lock().seconds.clone()
    }
    pub(crate) fn bake(&self) -> ProxyBakeDiagnostics {
        self.lock().bake.clone()
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
        phases.finished_depth(Phase::LodSimplify, 3, 1.5).unwrap();
        phases.finished_depth(Phase::LodSimplify, 2, 0.5).unwrap();
        phases.begin_bake(2).unwrap();
        let stats = BakeStats {
            raster_seconds: 2.,
            facing_fallback_samples: 7,
            facing_fallback_faces: 2,
            ..Default::default()
        };
        phases.baked_tile(&stats, 0.25).unwrap();
        phases.baked_tile(&BakeStats::default(), 0.25).unwrap();
        phases.baked_depth(7, 2, 1, 4.).unwrap();
        phases.bake_summary().unwrap();
        let (seconds, bake) = (phases.seconds(), phases.bake());
        let events = events.into_inner().unwrap();
        let leaf: Vec<_> = events
            .iter()
            .filter(|e| e.starts_with("leaf_build"))
            .collect();
        assert_eq!(leaf.len(), 201);
        assert_eq!(leaf[0], "leaf_build 0 Some(1000)");
        assert_eq!(leaf[200], "leaf_build 1000 Some(1000)");
        assert!(events.contains(&"archive 0 None".to_string()));
        assert!(events.iter().any(
            |e| e.starts_with("mesh phase lod_simplify depth 2: 0.500 s (phase total 2.000 s")
        ));
        assert_eq!(
            events
                .iter()
                .filter(|e| e.starts_with("proxy_bake"))
                .count(),
            3
        );
        assert_eq!(seconds.lod_simplify, 2.);
        assert_eq!(
            (
                bake.tiles,
                bake.facing_fallback_samples,
                bake.facing_fallback_faces,
                bake.facing_fallback_tiles
            ),
            (2, 7, 2, 1)
        );
        assert_eq!(bake.commit_worker_seconds, 0.5);
        assert_eq!(bake.retried_tiles, 1);
        assert!(events.iter().any(|e| e
            .starts_with("mesh proxy bake depth 7: 2 tiles (1 retried alone")
            && e.ends_with("facing fallback so far on 2 faces in 1 tiles")));
        assert!(events.iter().any(
            |e| e.starts_with("mesh proxy bake: 2 tiles (1 retried alone)")
                && e.contains("facing fallback 7 samples on 2 faces in 1 tiles")
        ));
        // A silent reporter still records.
        let silent = Phases::silent();
        silent.finished(Phase::Archive, 3.).unwrap();
        assert_eq!(silent.seconds().archive, 3.);
    }
}
