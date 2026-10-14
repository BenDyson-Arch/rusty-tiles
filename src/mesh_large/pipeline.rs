//! One operation owns capture, private leaves, archive serialization and publication.
use super::{
    config::{Jobs, TreeConfig},
    delivery::{self, DeliveryFacts, MeshTilesetRequest},
    full,
    input::StreamSource,
    leaves::producer,
};
use crate::{
    mesh_archive::ResolvedPlacement, runtime::Staging, CleanupDiagnostic, JobError, JobErrorKind,
    JobFailure, MeshPlacement, MeshPlacementReport, OutputPolicy, RunControl, RunEvent,
};
use serde::Serialize;
use std::{
    io,
    num::NonZeroUsize,
    path::{Path, PathBuf},
    str::FromStr,
};

/// Provisional texture profiles; their constants are implementation choices.
pub use producer::LeafProfile as TilerProfile;
pub use producer::LeafStageDiagnostics;

impl FromStr for TilerProfile {
    type Err = JobError;
    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "fast" => Ok(Self::Fast),
            "balanced" => Ok(Self::Balanced),
            "quality" => Ok(Self::Quality),
            "lossless" => Ok(Self::Lossless),
            _ => Err(invalid(
                "profile must be fast, balanced, quality or lossless",
            )),
        }
    }
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum TilerJobs {
    #[default]
    Auto,
    Maximum(NonZeroUsize),
}
impl FromStr for TilerJobs {
    type Err = JobError;
    fn from_str(value: &str) -> Result<Self, Self::Err> {
        if value == "auto" {
            return Ok(Self::Auto);
        }
        if !value.is_empty() && value.bytes().all(|b| b.is_ascii_digit()) {
            if let Ok(n) = value.parse::<NonZeroUsize>() {
                return Ok(Self::Maximum(n));
            }
        }
        Err(invalid("jobs must be auto or a positive integer maximum"))
    }
}
impl From<TilerJobs> for Jobs {
    fn from(value: TilerJobs) -> Self {
        match value {
            TilerJobs::Auto => Self::Auto,
            TilerJobs::Maximum(n) => Self::Fixed(n),
        }
    }
}
#[derive(Clone, Debug)]
pub struct TilerRequest {
    input: PathBuf,
    output: PathBuf,
    profile: TilerProfile,
    jobs: TilerJobs,
    scratch_dir: Option<PathBuf>,
    placement: MeshPlacement,
    policy: OutputPolicy,
}
impl TilerRequest {
    pub fn new(input: impl Into<PathBuf>, output: impl Into<PathBuf>) -> Self {
        Self {
            input: input.into(),
            output: output.into(),
            profile: TilerProfile::Balanced,
            jobs: TilerJobs::Auto,
            scratch_dir: None,
            placement: MeshPlacement::Local,
            policy: OutputPolicy::CreateNew,
        }
    }
    pub fn with_profile(mut self, profile: TilerProfile) -> Self {
        self.profile = profile;
        self
    }
    pub fn with_jobs(mut self, jobs: TilerJobs) -> Self {
        self.jobs = jobs;
        self
    }
    pub fn with_scratch_dir(mut self, path: impl Into<PathBuf>) -> Self {
        self.scratch_dir = Some(path.into());
        self
    }
    pub fn with_placement(mut self, placement: MeshPlacement) -> Self {
        self.placement = placement;
        self
    }
    pub fn with_output_policy(mut self, policy: OutputPolicy) -> Self {
        self.policy = policy;
        self
    }
}

/// Stable semantic report, also stored verbatim as conversion.json in the archive.
#[derive(Clone, Debug, Serialize)]
pub struct TilerReport {
    pub schema_version: u32,
    pub operation: &'static str,
    pub tiles_version: &'static str,
    pub gltf_version: &'static str,
    pub profile: TilerProfile,
    pub source_bytes: u64,
    pub external_files: u64,
    pub external_bytes: u64,
    pub triangles: u64,
    pub leaves: u64,
    pub glb_bytes: u64,
    pub maximum_glb_bytes: u64,
    pub maximum_leaf_geometry_error: f64,
    pub density_reduced_leaves: u64,
    /// Lossy pages a stronger UASTC step delivered after a gate miss.
    pub escalated_pages: producer::EscalatedPages,
    /// Pages delivered as exact level-0 PNG after every lossy step missed.
    pub lossless_fallback_pages: u64,
    /// Escalated pages by what the profile step missed: `gate` or `codec_verify`.
    pub escalation_reasons: producer::EscalationReasons,
    /// The first escalated pages in leaf order, naming leaf, slot, sources,
    /// step and each missed gate with its level.
    pub escalation_diagnostics: Vec<producer::EscalationDiagnostic>,
    pub escalation_diagnostics_omitted: u64,
    pub tiles: u64,
    pub explicit_tiles: u64,
    pub implicit_roots: u64,
    pub implicit_tiles: u64,
    pub documents: u64,
    pub max_depth: u32,
    pub synthetic_leaves: u64,
    pub packed_leaves: u64,
    pub members: u64,
    pub member_ceiling: u64,
    pub root_geometric_error: f64,
    pub tileset_geometric_error: f64,
    pub placement: MeshPlacementReport,
    pub source_coordinates: &'static str,
    pub coordinates: &'static str,
    pub root_transform: Option<[f64; 16]>,
}
impl TilerReport {
    fn new(
        capture: &full::FullCaptureReport,
        leaf: &producer::LeafOutputReport,
        facts: &DeliveryFacts,
    ) -> Self {
        Self {
            schema_version: 1,
            operation: "mesh_tiler",
            tiles_version: "1.1",
            gltf_version: "2.0",
            profile: leaf.profile,
            source_bytes: capture.source_bytes,
            external_files: capture.external_files,
            external_bytes: capture.external_bytes,
            triangles: leaf.triangles,
            leaves: leaf.leaves,
            glb_bytes: leaf.glb_bytes,
            maximum_glb_bytes: leaf.maximum_glb_bytes,
            maximum_leaf_geometry_error: leaf.maximum_geometry_error,
            density_reduced_leaves: leaf.density_reduced_leaves,
            escalated_pages: leaf.escalated_pages,
            lossless_fallback_pages: leaf.lossless_fallback_pages,
            escalation_reasons: leaf.escalation_reasons,
            escalation_diagnostics: leaf.escalation_diagnostics.clone(),
            escalation_diagnostics_omitted: leaf.escalation_diagnostics_omitted,
            tiles: facts.tiles,
            explicit_tiles: facts.explicit_tiles,
            implicit_roots: facts.implicit_roots,
            implicit_tiles: facts.implicit_tiles,
            documents: facts.documents,
            max_depth: facts.max_depth,
            synthetic_leaves: facts.synthetic_leaves,
            packed_leaves: facts.packed_leaves,
            members: facts.members,
            member_ceiling: facts.member_ceiling,
            root_geometric_error: facts.root_geometric_error,
            tileset_geometric_error: facts.tileset_geometric_error,
            placement: facts.placement.clone(),
            source_coordinates: "local-gltf",
            coordinates: if facts.root_transform.is_some() {
                "wgs84-ecef"
            } else {
                "local-gltf"
            },
            root_transform: facts.root_transform,
        }
    }
}
#[derive(Clone, Debug, Serialize)]
pub struct CaptureDiagnostics {
    pub jobs: u64,
    pub jobs_limited_by: &'static str,
    pub available_memory_bytes: Option<u64>,
    pub memory_bound_bytes: u64,
    pub scratch_need_bytes: u64,
    pub scratch_logical_need_bytes: u64,
    pub scratch_logical_peak_bytes: u64,
    pub scratch_memory_backed: bool,
    /// Declared largest working set of each capture stage, retained files included.
    pub scratch_stage_bytes: super::full::ScratchStages,
    /// Measured peak of each capture stage (`lod_prepass` runs later and reads 0).
    pub scratch_stage_peak_bytes: super::full::ScratchStages,
    /// Output-side admission reservation on the work directory and output filesystems.
    pub output_reserve_bytes: u64,
}
#[derive(Clone, Debug, Serialize)]
pub struct TilerDiagnostics {
    pub requested_jobs: String,
    pub capture: CaptureDiagnostics,
    pub leaf_delivery: LeafStageDiagnostics,
    pub archive_bytes: u64,
    pub peak_held_records: u64,
}
#[derive(Debug)]
pub struct TilerResult {
    pub output: PathBuf,
    pub report: TilerReport,
    pub diagnostics: TilerDiagnostics,
    pub cleanup_diagnostics: Vec<CleanupDiagnostic>,
}
fn invalid(message: &str) -> JobError {
    JobError::new(JobErrorKind::InvalidRequest, message)
}

fn preflight(request: &TilerRequest) -> Result<(PathBuf, ResolvedPlacement), JobError> {
    if !request
        .output
        .extension()
        .is_some_and(|e| e.eq_ignore_ascii_case("3tz"))
    {
        return Err(invalid("mesh output must have the .3tz extension"));
    }
    let placement = ResolvedPlacement::resolve(&request.placement)?;
    match std::fs::symlink_metadata(&request.output) {
        Ok(m) if !m.is_file() => return Err(invalid("mesh output must be a regular file path")),
        Ok(_) => {}
        Err(e) if e.kind() == io::ErrorKind::NotFound => {}
        Err(e) => return Err(JobError::io("inspect mesh output", &request.output, e)),
    }
    let output = crate::output_path::resolve(&request.output)?;
    let input = crate::output_path::resolve(&request.input)?;
    if output == input
        || (output.exists()
            && input.exists()
            && same_file::is_same_file(&input, &output)
                .map_err(|e| JobError::io("inspect mesh output alias", &output, e))?)
    {
        return Err(invalid("mesh output aliases the source"));
    }
    if request.policy == OutputPolicy::CreateNew && output.exists() {
        return Err(JobError::conflict(
            &output,
            io::ErrorKind::AlreadyExists.into(),
        ));
    }
    Ok((output, placement))
}
fn add_cleanup(failure: &mut JobFailure, path: PathBuf, result: Result<(), JobError>) {
    if let Err(error) = result {
        failure.secondary.push(error);
        if path.exists() {
            failure.retained_paths.push(path);
        }
    }
}

/// Convert an admitted local-metre glTF/GLB into one Tiles 1.1 .3tz.
/// One control attempt owns every stage and publishes only after cleanup and observers finish.
pub fn mesh_to_tileset(
    request: TilerRequest,
    control: &RunControl,
) -> Result<TilerResult, JobFailure> {
    let attempt = control.begin()?;
    attempt.check().map_err(|e| attempt.fail(e))?;
    let (output, placement) = preflight(&request).map_err(|e| attempt.fail(e))?;
    let mut config = TreeConfig::new(request.profile.into());
    config.jobs = request.jobs.into();
    config.scratch_dir = request
        .scratch_dir
        .as_deref()
        .map(crate::output_path::resolve)
        .transpose()
        .map_err(|e| attempt.fail(e))?;
    config.validate().map_err(|e| attempt.fail(e))?;
    let source =
        StreamSource::open(&request.input, &mut || attempt.check()).map_err(|e| attempt.fail(e))?;
    source.reject_output(&output).map_err(|e| attempt.fail(e))?;
    let mut staging = Staging::create(&output, &attempt)?;
    let parent = output.parent().unwrap_or(Path::new("."));
    let work_parent = config.scratch_dir.as_deref().unwrap_or(parent);
    let work = match tempfile::Builder::new()
        .prefix(".tiles-mesh-")
        .tempdir_in(work_parent)
    {
        Ok(work) => work,
        Err(error) => {
            return Err(staging.fail(JobError::io("create mesh intermediate", work_parent, error)))
        }
    };
    let work_path = work.path().to_owned();
    // Capture admission also reserves output-side space where leaves and the archive stage.
    config.output_parents = Some((work_parent.to_owned(), parent.to_owned()));
    let captured = match attempt
        .emit(&RunEvent::Progress {
            phase: "capture",
            done: 0,
            total: None,
        })
        .map_err(super::failure)
        .and_then(|_| full::capture_admitted(source, &config, &mut || attempt.check()))
    {
        Ok(captured) => captured,
        Err(inner) => {
            let mut failure = staging.fail(inner.error);
            failure.secondary.extend(inner.secondary);
            failure.retained_paths.extend(inner.retained_paths);
            add_cleanup(
                &mut failure,
                work_path.clone(),
                work.close()
                    .map_err(|e| JobError::io("remove mesh intermediate", &work_path, e)),
            );
            return Err(failure);
        }
    };
    let capture_path = captured.scratch.path().to_owned();
    let result = (|| {
        attempt.emit(&RunEvent::Progress {
            phase: "capture",
            done: 1,
            total: Some(1),
        })?;
        let plan = producer::stage_plan(config.profile, &captured, config.jobs)?;
        let leaf = producer::produce(
            work.path(),
            &captured,
            &config,
            &plan,
            request.profile,
            super::lod::Mutation::None,
            super::lod::TARGET_FACES,
            // No production proxy baker yet (H27 plugs in here); proxies stay untextured.
            None,
            &|| attempt.check(),
        )?;
        attempt.emit(&RunEvent::Progress {
            phase: "leaf_payloads",
            done: leaf.report.leaves,
            total: Some(leaf.report.leaves),
        })?;
        // H17 certified the coarse tiles inside the producer, while the capture was live;
        // delivery publishes them with the leaves.
        let delivery_request = MeshTilesetRequest::new(work.path(), &output);
        let prepared = delivery::prepare_tiler(
            &delivery_request,
            placement,
            &mut || attempt.check(),
            |facts| {
                serde_json::to_vec(&TilerReport::new(&captured.report, &leaf.report, facts))
                    .map_err(|e| JobError::new(JobErrorKind::InvalidState, e.to_string()))
            },
        )?;
        let report = TilerReport::new(&captured.report, &leaf.report, &prepared.facts());
        let (delivered, archive_bytes) = delivery::write_prepared(
            prepared,
            &mut || attempt.check(),
            staging.writer(),
            work.path(),
        )?;
        let c = &captured.report;
        let diagnostics = TilerDiagnostics {
            requested_jobs: c.jobs.requested.clone(),
            capture: CaptureDiagnostics {
                jobs: c.jobs.jobs,
                jobs_limited_by: c.jobs.limited_by,
                available_memory_bytes: c.jobs.available_memory_bytes,
                memory_bound_bytes: c.memory.bound_bytes,
                scratch_need_bytes: c.scratch.need_bytes,
                scratch_logical_need_bytes: c.scratch.logical_need_bytes,
                scratch_logical_peak_bytes: c.scratch_logical_peak_bytes,
                scratch_memory_backed: c.scratch_memory_backed,
                scratch_stage_bytes: c.scratch.stages,
                scratch_stage_peak_bytes: c.scratch_stage_peaks,
                output_reserve_bytes: c.scratch.output_reserve_bytes,
            },
            leaf_delivery: leaf.diagnostics,
            archive_bytes,
            peak_held_records: delivered.peak_held_records,
        };
        Ok::<_, JobError>((report, diagnostics))
    })();
    let capture_cleanup = captured.close();
    let work_cleanup = work
        .close()
        .map_err(|e| JobError::io("remove mesh intermediate", &work_path, e));
    let (report, diagnostics) = match result {
        Err(error) => {
            let mut failure = staging.fail(error);
            add_cleanup(&mut failure, capture_path, capture_cleanup);
            add_cleanup(&mut failure, work_path, work_cleanup);
            return Err(failure);
        }
        Ok(result) => {
            if let Err(error) = capture_cleanup {
                let mut failure = staging.fail(error);
                if capture_path.exists() {
                    failure.retained_paths.push(capture_path);
                }
                add_cleanup(&mut failure, work_path, work_cleanup);
                return Err(failure);
            }
            if let Err(error) = work_cleanup {
                let mut failure = staging.fail(error);
                if work_path.exists() {
                    failure.retained_paths.push(work_path);
                }
                return Err(failure);
            }
            result
        }
    };
    if let Err(error) = attempt
        .emit(&RunEvent::Progress {
            phase: "ready_to_publish",
            done: report.leaves,
            total: Some(report.leaves),
        })
        .and_then(|_| attempt.close_events())
    {
        return Err(staging.fail(error));
    }
    let published = staging.seal()?.publish(&output, request.policy)?;
    Ok(TilerResult {
        output: published.output,
        report,
        diagnostics,
        cleanup_diagnostics: published.cleanup_diagnostics,
    })
}
