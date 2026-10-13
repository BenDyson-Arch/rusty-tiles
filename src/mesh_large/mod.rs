//! Private streamed mesh foundation. Tile producers will consume these owned spill records.
//! Full capture owns attributes, images and the frozen lattice; H8 publishes full-detail
//! leaves, H17 (`lod`) certifies the coarse tiles inside the H8 producer while the capture is
//! live, and H16 (`delivery`) publishes their hierarchy as a `.3tz`.
#![allow(dead_code)] // First concrete consumer is the diagnostic acceptance driver; tiler follows.

mod attributes;
mod capture;
mod config;
mod delivery;
mod full;
mod images;
mod input;
mod leaves;
mod lod;
mod pipeline;
mod records;
mod tree;
pub use pipeline::{
    mesh_to_tileset, TilerDiagnostics, TilerJobs, TilerProfile, TilerReport, TilerRequest,
    TilerResult,
};

use crate::{spill::Scratch, JobError, JobErrorKind, JobFailure};
use config::{JobsPlan, MemoryBound, ScratchNeed, TreeConfig, MEMBERS_BESIDES_TILES};
use input::StreamSource;
use records::{Bounds, Cube};
use serde::Serialize;
use std::path::Path;
use tree::TreeOutput;

#[derive(Clone, Debug, Serialize)]
pub(crate) struct CaptureReport {
    pub source_bytes: u64,
    pub external_files: u64,
    pub external_bytes: u64,
    pub json_bytes: u64,
    pub triangles: u64,
    pub geometry_bytes: u64,
    pub source_bounds: Bounds,
    pub cube: Cube,
    pub jobs: JobsPlan,
    pub memory: MemoryBound,
    pub scratch: ScratchNeed,
    pub scratch_logical_peak_bytes: u64,
    pub scratch_memory_backed: bool,
}

pub(crate) struct CapturedTree {
    // Spill handles drop before the directory, including on a diagnostic consumer failure.
    pub tree: TreeOutput,
    pub scratch: Scratch,
    pub report: CaptureReport,
}

impl CapturedTree {
    pub(crate) fn close(self) -> Result<(), JobError> {
        let Self { tree, scratch, .. } = self;
        drop(tree);
        scratch.close().map_err(Into::into)
    }
}

fn failure(error: JobError) -> JobFailure {
    JobFailure {
        error,
        secondary: Vec::new(),
        retained_paths: Vec::new(),
        recovery: None,
    }
}

fn exhausted(message: &'static str) -> JobError {
    JobError::new(JobErrorKind::ResourceLimit, message)
}

fn member_gate(tiles: u64, ceiling: u64) -> Result<(), JobError> {
    if tiles
        .checked_add(MEMBERS_BESIDES_TILES)
        .is_none_or(|members| members > ceiling)
    {
        return Err(JobError::new(JobErrorKind::Unsupported,
            format!("3TZ member ceiling: at least {tiles} tiles plus {MEMBERS_BESIDES_TILES} required members exceeds {ceiling}")));
    }
    Ok(())
}

pub(crate) fn build(
    input: &Path,
    config: &TreeConfig,
    check: &mut dyn FnMut() -> Result<(), JobError>,
) -> Result<CapturedTree, JobFailure> {
    config.validate().map_err(failure)?;
    check().map_err(failure)?;
    let source = StreamSource::open(input, check).map_err(failure)?;
    member_gate(
        config::tile_lower_bound(
            source.document.triangle_count() as u64,
            config.leaf_triangles(),
        ),
        config.member_ceiling,
    )
    .map_err(failure)?;
    let scan = capture::scan(&source, config.sort.buffer_bytes, check).map_err(failure)?;
    let parent = config
        .scratch_dir
        .clone()
        .unwrap_or_else(Scratch::default_parent);
    let volume = crate::spill::inspect_volume(&parent).map_err(|error| failure(error.into()))?;
    let need = config::scratch_need_parts(
        scan.triangles,
        scan.geometry_bytes,
        config,
        volume.allocation_unit_bytes,
    )
    .ok_or_else(|| failure(exhausted("H1 scratch bound exceeds u64")))?;
    let scratch = Scratch::create(config.scratch_dir.as_deref(), need.need_bytes)
        .map_err(|error| failure(error.into()))?;
    let result = (|| {
        let need = config::scratch_need_parts(
            scan.triangles,
            scan.geometry_bytes,
            config,
            scratch.allocation_unit_bytes(),
        )
        .ok_or_else(|| exhausted("H1 scratch bound exceeds u64"))?;
        config::preflight_remaining(&scratch, need.need_bytes)?;
        let one = MemoryBound::new(config, source.json_bytes, 1);
        let memory_scratch = if scratch.memory_backed() {
            need.need_bytes
        } else {
            0
        };
        let jobs = config::plan_jobs(
            config.jobs,
            std::thread::available_parallelism().map_or(1, |n| n.get()),
            config::available_memory(),
            memory_scratch,
            one.base_bytes(),
            one.per_job_bytes,
        );
        let memory = MemoryBound::new(config, source.json_bytes, jobs.jobs as usize);
        if memory.bound_bytes == u64::MAX {
            return Err(exhausted("H1 memory bound exceeds u64"));
        }
        check()?;
        let triangles = capture::capture(
            &source,
            &scratch,
            config.sorter(jobs.jobs as usize),
            &scan,
            check,
        )?;
        config::preflight_remaining(&scratch, need.need_bytes)?;
        let tree = tree::build(&scratch, triangles, config, jobs.jobs as usize, 0, check)?;
        member_gate(tree.stats.nodes, config.member_ceiling)?;
        check()?;
        if scratch.peak_bytes() > need.logical_need_bytes {
            return Err(exhausted(
                "H1 scratch accounting exceeded its declared bound",
            ));
        }
        let report = CaptureReport {
            source_bytes: source.source_bytes,
            external_files: source.external_files,
            external_bytes: source.external_bytes,
            json_bytes: source.json_bytes,
            triangles: scan.triangles,
            geometry_bytes: scan.geometry_bytes,
            source_bounds: scan.bounds,
            cube: Cube::around(&scan.bounds),
            jobs,
            memory,
            scratch: need,
            scratch_logical_peak_bytes: scratch.peak_bytes(),
            scratch_memory_backed: scratch.memory_backed(),
        };
        Ok((tree, report))
    })();
    match result {
        Ok((tree, report)) => Ok(CapturedTree {
            tree,
            scratch,
            report,
        }),
        Err(error) => {
            let path = scratch.path().to_owned();
            let mut failed = failure(error);
            if let Err(cleanup) = scratch.close() {
                failed.secondary.push(cleanup.into());
                if path.exists() {
                    failed.retained_paths.push(path);
                }
            }
            Err(failed)
        }
    }
}

#[cfg(test)]
mod tests;
