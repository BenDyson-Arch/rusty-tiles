//! One operation owns capture, private leaves, archive serialization and publication.
use super::{
    config::{Jobs, TreeConfig},
    delivery::{self, DeliveryFacts, MeshTilesetRequest},
    full,
    hlod::{HlodDiagnostics, HlodMemoryPlan},
    input::StreamSource,
    leaves::producer,
    phases::{Phase, PhaseSeconds, Phases},
    resources::MemoryBudget,
    source_crs::{MeshSourceCrs, MeshSourceCrsReport},
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
    source_crs: Option<MeshSourceCrs>,
    policy: OutputPolicy,
    memory_budget: Option<u64>,
    #[cfg(test)]
    hlod: super::hlod::Settings,
    /// Test seam: smaller leaves, so small inputs get coarse levels.
    #[cfg(test)]
    leaf_triangles: Option<u64>,
    /// Test seam: sort configuration and largest spill chunk, so small inputs spill.
    #[cfg(test)]
    test_spill: Option<(crate::spill::SortConfig, usize)>,
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
            source_crs: None,
            policy: OutputPolicy::CreateNew,
            memory_budget: None,
            #[cfg(test)]
            hlod: super::hlod::Settings::default(),
            #[cfg(test)]
            leaf_triangles: None,
            #[cfg(test)]
            test_spill: None,
        }
    }
    /// Declared memory budget in bytes, a ceiling for every stage of the run: never raised,
    /// and a run that cannot fit under it is refused as `resource_limit` before the binding
    /// stage allocates. Without one the run resolves the lesser of 4 GiB and half the
    /// detected host/cgroup headroom (2 GiB when headroom is undetected). Zero is refused as
    /// `invalid_request` before any source I/O.
    pub fn with_memory_budget(mut self, bytes: u64) -> Self {
        self.memory_budget = Some(bytes);
        self
    }
    #[cfg(test)]
    pub(crate) fn with_hlod_settings(mut self, settings: super::hlod::Settings) -> Self {
        self.hlod = settings;
        self
    }
    #[cfg(test)]
    pub(crate) fn with_test_leaf_triangles(mut self, leaf_triangles: u64) -> Self {
        self.leaf_triangles = Some(leaf_triangles);
        self
    }
    #[cfg(test)]
    pub(crate) fn with_test_spill(mut self, sort: crate::spill::SortConfig, chunk: usize) -> Self {
        self.test_spill = Some((sort, chunk));
        self
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
    /// Map source positions through a declared CRS at capture. Mutually exclusive with a
    /// rigid placement: the root transform is derived from the source.
    pub fn with_source_crs(mut self, source_crs: MeshSourceCrs) -> Self {
        self.source_crs = Some(source_crs);
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
    /// H28 texture-space editability per tile (`unique` unless listed as `shared`).
    pub texel_ownership: producer::TexelOwnership,
    /// The resolved source CRS; omitted for local sources.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_crs: Option<MeshSourceCrsReport>,
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
            source_coordinates: if capture.source_crs.is_some() {
                "source-crs"
            } else {
                "local-gltf"
            },
            coordinates: if facts.root_transform.is_some() {
                "wgs84-ecef"
            } else {
                "local-gltf"
            },
            root_transform: facts.root_transform,
            texel_ownership: leaf.texel_ownership.clone(),
            source_crs: capture.source_crs.clone(),
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
    /// Measured peak of each capture stage (`hlod_incidence` runs later and reads 0).
    pub scratch_stage_peak_bytes: super::full::ScratchStages,
    /// Output-side admission reservation on the work directory and output filesystems.
    pub output_reserve_bytes: u64,
    /// Whole-run spill peak, including the HLOD stage; at most `scratch_logical_need_bytes`.
    pub run_scratch_peak_bytes: u64,
}
#[derive(Clone, Debug, Serialize)]
pub struct TilerDiagnostics {
    pub requested_jobs: String,
    pub capture: CaptureDiagnostics,
    pub leaf_delivery: LeafStageDiagnostics,
    pub archive_bytes: u64,
    pub peak_held_records: u64,
    /// Wall seconds of each named phase of the run.
    pub phase_seconds: PhaseSeconds,
    /// The declared budget and each stage's admission.
    pub memory: MemoryDiagnostics,
    /// HLOD stage run facts.
    pub hlod: HlodDiagnostics,
}

/// D8: the resolved budget and what each stage planned and needed under it.
#[derive(Clone, Debug, Serialize)]
pub struct MemoryDiagnostics {
    pub budget: MemoryBudget,
    /// The least budget admitting every stage (the largest stage minimum); one byte
    /// less is refused before the binding stage allocates.
    pub minimum_bytes: u64,
    pub capture_minimum_bytes: u64,
    pub capture_bound_bytes: u64,
    pub leaf_minimum_bytes: u64,
    pub leaf_bound_bytes: u64,
    pub hlod: HlodMemoryPlan,
    /// Archive preparation and writing: captured metadata, runtime reserve, memory-backed
    /// scratch and H16's declared resident bound.
    pub archive_minimum_bytes: u64,
    /// Memory-backed output and intermediate storage charged to the budget
    /// (`budget.memory_backed_storage_bytes` is their sum) and what was measured against it.
    pub memory_backed_storage: StorageDiagnostics,
}

/// D8 charges of memory-backed (tmpfs) storage after capture, `None` for an owner on disk.
/// Each owner is charged one copy of capture's output reservation (`output_reserve_bytes`
/// holds three: leaf staging, archive staging and a replaced destination, whose existing
/// bytes the detected headroom already excludes) plus a fixed 32 MiB for files that do not
/// scale with triangles. A measured owner past its charge is a `resource_limit` refusal, so
/// the budget bounds the run.
#[derive(Clone, Debug, Serialize)]
pub struct StorageDiagnostics {
    /// The private work directory: leaf and coarse payloads, forest staging and the
    /// archive's scratch.
    pub work_charge_bytes: Option<u64>,
    /// The archive staged beside the output.
    pub archive_charge_bytes: Option<u64>,
    /// The charged work directory's measured files plus the archive's declared scratch,
    /// checked before the archive is written.
    pub work_bytes: Option<u64>,
    /// The charged staged archive's length, checked as it is written.
    pub archive_bytes: Option<u64>,
}

/// Fixed part of each memory-backed storage charge: leaf and coarse manifests, forest
/// documents, the conversion report and the archive's index and directory scratch, which do
/// not scale with triangles and dominate a small source's files.
const STORAGE_FIXED_BYTES: u64 = 32 << 20;

/// One copy of the output reservation, plus [`STORAGE_FIXED_BYTES`], for each of the work
/// directory and the output parent that is memory-backed.
fn storage_charges(
    work: &Path,
    output_parent: &Path,
    scratch: &super::full::FullScratchNeed,
) -> Result<delivery::StorageCharges, JobError> {
    let copy = (scratch.output_reserve_bytes / 3).saturating_add(STORAGE_FIXED_BYTES);
    let charge = |path: &Path| -> Result<Option<u64>, JobError> {
        let volume = crate::spill::inspect_volume(path).map_err(JobError::from)?;
        Ok(volume.memory_backed.then_some(copy))
    };
    Ok(delivery::StorageCharges {
        work: charge(work)?,
        archive: charge(output_parent)?,
    })
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

/// Budget request rules, checked before any filesystem access.
fn check_memory_budget(request: &TilerRequest) -> Result<(), JobError> {
    match request.memory_budget {
        Some(bytes) => super::resources::validate_memory_budget(bytes).map(|_| ()),
        None => Ok(()),
    }
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
    if let Some(source_crs) = &request.source_crs {
        if request.placement != MeshPlacement::Local {
            return Err(invalid(
                "a source CRS cannot be combined with --anchor, --orientation-xyzw or --scene-offset",
            ));
        }
        source_crs.check()?;
    }
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
    // Request intent is refused first: no path, source or output is touched for a bad budget.
    check_memory_budget(&request).map_err(|e| attempt.fail(e))?;
    let (output, placement) = preflight(&request).map_err(|e| attempt.fail(e))?;
    let mut config = TreeConfig::new(request.profile.into());
    config.jobs = request.jobs.into();
    config.source_crs = request.source_crs.clone();
    let budget = MemoryBudget::resolve(request.memory_budget).map_err(|e| attempt.fail(e))?;
    config.memory_budget = Some(budget);
    #[cfg(test)]
    if let Some(leaf_triangles) = request.leaf_triangles {
        config = config.with_leaf_triangles(leaf_triangles);
    }
    #[cfg(test)]
    if let Some((sort, chunk)) = request.test_spill {
        config.sort = sort;
        config.test_chunk_bytes = Some(chunk);
    }
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
    let emit = |event: &RunEvent<'_>| attempt.emit(event);
    let phases = Phases::new(&emit);
    let capture_started = std::time::Instant::now();
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
        phases.finished(Phase::Capture, capture_started.elapsed().as_secs_f64())?;
        // D8: memory-backed storage written from here on is charged before any later stage
        // plans, so every stage (leaf, HLOD, archive) sees the budget it leaves.
        let charges = storage_charges(work.path(), parent, &captured.report.scratch)?;
        let storage = charges
            .work
            .unwrap_or(0)
            .checked_add(charges.archive.unwrap_or(0))
            .ok_or_else(|| JobError::new(JobErrorKind::ResourceLimit, "storage charge overflow"))?;
        let budget = MemoryBudget {
            memory_backed_storage_bytes: storage,
            ..budget
        };
        let mut config = config.clone();
        config.memory_budget = Some(budget);
        let config = &config;
        let plan = producer::stage_plan(config, &captured)?;
        // HLOD admits its own stage against the declared budget before any leaf is built.
        #[cfg(test)]
        let settings = request.hlod;
        #[cfg(not(test))]
        let settings = super::hlod::Settings::default();
        let hlod = super::hlod::plan(&captured, config, &budget, &settings)?;
        let c = &captured.report;
        let capture_minimum = c
            .jobs
            .base_memory_bytes
            .saturating_add(c.jobs.per_job_memory_bytes)
            .saturating_add(c.jobs.memory_backed_scratch_bytes);
        let tmpfs = if captured.scratch.memory_backed() {
            c.scratch.need_bytes
        } else {
            0
        };
        let archive_minimum = c
            .memory
            .owned_metadata_bytes
            .saturating_add(c.memory.geometry.document_bytes)
            .saturating_add(256 << 20)
            .saturating_add(tmpfs)
            .saturating_add(delivery::resident_bound(
                delivery::source::TILER_MAX_GLB_JSON,
            ));
        let available = budget.ceiling(super::config::available_memory());
        if available < archive_minimum {
            return Err(JobError::new(
                JobErrorKind::ResourceLimit,
                format!(
                    "archive preparation needs at least {archive_minimum} bytes but the \
                     memory budget allows {available}"
                ),
            ));
        }
        // Stages after capture admit against the budget less the storage charge.
        let mut memory = MemoryDiagnostics {
            budget,
            minimum_bytes: capture_minimum.max(
                plan.minimum_bytes()
                    .max(hlod.minimum_bytes)
                    .max(archive_minimum)
                    .saturating_add(storage),
            ),
            archive_minimum_bytes: archive_minimum,
            capture_minimum_bytes: capture_minimum,
            capture_bound_bytes: c.memory.bound_bytes,
            leaf_minimum_bytes: plan.minimum_bytes(),
            leaf_bound_bytes: plan.bound_bytes(),
            hlod: hlod.clone(),
            memory_backed_storage: StorageDiagnostics {
                work_charge_bytes: charges.work,
                archive_charge_bytes: charges.archive,
                work_bytes: None,
                archive_bytes: None,
            },
        };
        let leaf = producer::produce(
            work.path(),
            &captured,
            config,
            &plan,
            request.profile,
            producer::Coarse {
                plan: &hlod,
                settings,
            },
            &phases,
            &|| attempt.check(),
        )?;
        attempt.emit(&RunEvent::Progress {
            phase: "leaf_payloads",
            done: leaf.report.leaves,
            total: Some(leaf.report.leaves),
        })?;
        // HLOD built the coarse tiles inside the producer, while the capture was live;
        // delivery publishes them with the leaves.
        let delivery_request = MeshTilesetRequest::new(work.path(), &output);
        // A source CRS derives the root placement from the captured origin.
        let placement = captured.placement.clone().unwrap_or(placement);
        let archive_started = std::time::Instant::now();
        phases.progress(Phase::Archive, 0, Some(1))?;
        let prepared = delivery::prepare_tiler(
            &delivery_request,
            placement,
            &mut || attempt.check(),
            |facts| {
                serde_json::to_vec(&TilerReport::new(&captured.report, &leaf.report, facts))
                    .map_err(|e| JobError::new(JobErrorKind::InvalidState, e.to_string()))
            },
            // The run's budget charged archive preparation for its own small members.
            delivery::source::TILER_MAX_GLB_JSON,
        )?;
        let report = TilerReport::new(&captured.report, &leaf.report, &prepared.facts());
        let (delivered, archive_bytes, work_bytes) = delivery::write_prepared(
            prepared,
            &mut || attempt.check(),
            staging.writer(),
            work.path(),
            charges,
        )?;
        memory.memory_backed_storage.work_bytes = work_bytes;
        memory.memory_backed_storage.archive_bytes = charges.archive.map(|_| archive_bytes);
        phases.progress(Phase::Archive, 1, Some(1))?;
        phases.finished(Phase::Archive, archive_started.elapsed().as_secs_f64())?;
        let c = &captured.report;
        // Every spill stage after capture (HLOD incidence, source index, coarse levels) shares
        // the admitted scratch: the whole-run peak must stay within the declared need.
        let run_scratch_peak_bytes = captured.scratch.peak_bytes();
        if run_scratch_peak_bytes > c.scratch.logical_need_bytes {
            return Err(JobError::new(
                JobErrorKind::ResourceLimit,
                format!(
                    "mesh run spill peak {run_scratch_peak_bytes} bytes exceeded the declared scratch need {}",
                    c.scratch.logical_need_bytes
                ),
            ));
        }
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
                run_scratch_peak_bytes,
            },
            leaf_delivery: leaf.diagnostics,
            archive_bytes,
            peak_held_records: delivered.peak_held_records,
            phase_seconds: phases.seconds(),
            memory,
            hlod: leaf.hlod,
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
