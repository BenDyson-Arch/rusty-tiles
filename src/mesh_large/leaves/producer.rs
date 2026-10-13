//! The H8 producer publishes leaf GLBs and diagnostic manifests, without a hierarchy.
use super::super::{
    attributes::FullTriangle,
    config::{self, Jobs, Profile, TreeConfig, TILE_BYTES},
    full::{self, CornerDepth},
    records::{NodeKind, NodeRecord},
};
use super::{
    atlas,
    planning::LeafPlan,
    schedule::{MemoryGate, SourceCache},
    writer,
};
use crate::spill::SequenceReader;
use crate::stage_timing::{self as timing, Counter, Stage};
use crate::{
    runtime::directory::DirectoryTarget, CleanupDiagnostic, JobError, JobErrorKind, JobFailure,
    OutputPolicy, RunControl, RunEvent,
};
use serde::Serialize;
use serde_json::json;
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    fs::{self, File},
    io::{BufWriter, Write},
    num::NonZeroUsize,
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
};

#[derive(Clone, Copy, Debug, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum LeafProfile {
    Fast,
    Balanced,
    Quality,
    Lossless,
}
impl From<LeafProfile> for Profile {
    fn from(value: LeafProfile) -> Self {
        match value {
            LeafProfile::Fast => Self::Fast,
            LeafProfile::Balanced => Self::Balanced,
            LeafProfile::Quality => Self::Quality,
            LeafProfile::Lossless => Self::Lossless,
        }
    }
}
#[derive(Clone, Debug)]
pub struct LeafOutputRequest {
    input: PathBuf,
    output: PathBuf,
    profile: LeafProfile,
    jobs: Jobs,
    scratch_dir: Option<PathBuf>,
}
impl LeafOutputRequest {
    pub fn new(
        input: impl Into<PathBuf>,
        output: impl Into<PathBuf>,
        profile: LeafProfile,
    ) -> Self {
        Self {
            input: input.into(),
            output: output.into(),
            profile,
            jobs: Jobs::Auto,
            scratch_dir: None,
        }
    }
    pub fn with_jobs(mut self, jobs: NonZeroUsize) -> Self {
        self.jobs = Jobs::Fixed(jobs);
        self
    }
    pub fn with_scratch_dir(mut self, path: impl Into<PathBuf>) -> Self {
        self.scratch_dir = Some(path.into());
        self
    }
}
#[derive(Clone, Debug, Serialize)]
pub struct LeafOutputReport {
    pub schema_version: u32,
    pub profile: LeafProfile,
    pub triangles: u64,
    pub leaves: u64,
    pub glb_bytes: u64,
    pub maximum_glb_bytes: u64,
    pub maximum_geometry_error: f64,
    pub density_reduced_leaves: u64,
}
#[derive(Debug)]
pub struct LeafOutputResult {
    pub output: PathBuf,
    pub report: LeafOutputReport,
    pub cleanup_diagnostics: Vec<CleanupDiagnostic>,
}
fn invariant(message: &str) -> JobError {
    JobError::new(JobErrorKind::InvalidState, message)
}
fn resource(message: &str) -> JobError {
    JobError::new(JobErrorKind::ResourceLimit, message)
}
fn close_failed_capture(captured: full::FullCaptured, failure: &mut JobFailure) {
    let path = captured.scratch.path().to_owned();
    if let Err(error) = captured.close() {
        failure.secondary.push(error);
        if path.exists() {
            failure.retained_paths.push(path);
        }
    }
}
fn hex(bytes: impl AsRef<[u8]>) -> String {
    bytes.as_ref().iter().map(|b| format!("{b:02x}")).collect()
}

/// The delivery stage's memory plan. Pages lease their actual workspace from
/// one byte gate shared by all workers, so `jobs` threads never hold more than
/// `gate_bytes` of page workspace whatever page sizes they meet. Decoded
/// sources live in a separately budgeted cache; in-flight leaves own their
/// geometry closure and encoded members up to the tile budget each.
#[derive(Clone, Copy, Debug, Serialize)]
struct StagePlan {
    /// Per-page limit for internal allocation checks (largest page lease).
    page_bytes: u64,
    gate_bytes: u64,
    cache_bytes: u64,
    global_bytes: u64,
    leaf_bytes: u64,
    window: u64,
    jobs: u64,
    jobs_limited_by: &'static str,
    available_bytes: Option<u64>,
}
/// Bytes reserved for the runtime, allocator slack and the output writer.
const STAGE_RESERVE: u64 = 256 << 20;
fn stage_plan(
    profile: Profile,
    captured: &full::FullCaptured,
    requested: Jobs,
) -> Result<StagePlan, JobError> {
    let edge: u64 = match profile {
        Profile::Fast => 2048,
        Profile::Lossless => 8192,
        _ => 4096,
    };
    let pixels = edge
        .checked_mul(edge)
        .ok_or_else(|| resource("leaf page memory overflow"))?;
    let largest_source = captured
        .image_info
        .iter()
        .map(|i| u64::from(i.width) * u64::from(i.height))
        .max()
        .unwrap_or(0);
    let total_source = captured
        .image_info
        .iter()
        .try_fold(0u64, |n, i| {
            n.checked_add(u64::from(i.width) * u64::from(i.height) * 4)
        })
        .ok_or_else(|| resource("leaf source byte overflow"))?;
    let texture = if profile == Profile::Lossless {
        pixels.checked_mul(40)
    } else {
        pixels.checked_mul(176)
    }
    .ok_or_else(|| resource("leaf texture workspace overflow"))?;
    // The largest page: codec workspace, assembly, MIP-binding source chain
    // and the largest source decode transient (density reduction included).
    let page_bytes = texture
        .checked_add(if profile == Profile::Lossless {
            0
        } else {
            (128u64 << 20)
                .checked_add(pixels.checked_mul(6).ok_or_else(|| resource("page"))?)
                .ok_or_else(|| resource("page"))?
        })
        .and_then(|n| n.checked_add(largest_source.checked_mul(32)?))
        .and_then(|n| n.checked_add(64 << 20))
        .ok_or_else(|| resource("leaf workspace overflow"))?;
    let global_bytes = captured
        .report
        .memory
        .owned_metadata_bytes
        .checked_add(captured.report.memory.geometry.document_bytes)
        .ok_or_else(|| resource("leaf global ownership overflow"))?;
    let mut leaf_owned = 0;
    for plan in captured.leaf_plans.reader()? {
        leaf_owned = leaf_owned.max(plan?.owned_bytes);
    }
    // An in-flight leaf owns its closure and at most one member of encoded
    // pages plus the GLB being assembled from them.
    let leaf_bytes = leaf_owned
        .checked_add(2 * TILE_BYTES)
        .ok_or_else(|| resource("leaf ownership overflow"))?;
    let tmpfs = if captured.scratch.memory_backed() {
        captured.report.scratch.need_bytes
    } else {
        0
    };
    let available = config::available_memory();
    let cores = std::thread::available_parallelism().map_or(1, |n| n.get()) as u64;
    let usable = available.map(|a| {
        a.saturating_sub(tmpfs)
            .saturating_sub(global_bytes)
            .saturating_sub(STAGE_RESERVE)
    });
    // Cache every decoded source when it fits half the usable memory;
    // otherwise half the usable memory, and always at least two images.
    let cache_bytes = match usable {
        Some(u) => total_source.min((u / 2).max(largest_source * 8)),
        None => total_source.min(largest_source * 8 * 4),
    };
    // Workers share one page-workspace gate, which must admit the largest
    // page. A running leaf owns its closure and encoded pages; a finished
    // leaf waiting for the ordered writer keeps only its closure and receipts
    // (its member is already written), so the window is four leaves per
    // worker and absorbs slow leaves without idling the others.
    let per_job = leaf_bytes
        .checked_add(
            leaf_owned
                .checked_mul(4)
                .ok_or_else(|| resource("window"))?,
        )
        .ok_or_else(|| resource("per-job overflow"))?;
    let (jobs, limited_by) = match requested {
        Jobs::Fixed(n) => (n.get() as u64, "request"),
        Jobs::Auto => match usable.map(|u| {
            u.saturating_sub(cache_bytes)
                .saturating_sub(page_bytes)
                .saturating_sub(2 * leaf_owned)
                / per_job.max(1)
        }) {
            Some(0) => (1, "floor"),
            Some(n) if n < cores => (n, "memory"),
            _ => (cores, "cores"),
        },
    };
    let window = jobs
        .checked_mul(4)
        .and_then(|n| n.checked_add(2))
        .ok_or_else(|| resource("window"))?;
    let leaves = jobs
        .checked_mul(leaf_bytes)
        .and_then(|n| n.checked_add(window.checked_mul(leaf_owned)?))
        .ok_or_else(|| resource("leaf window overflow"))?;
    let gate_bytes = match usable {
        Some(u) => u.saturating_sub(cache_bytes).saturating_sub(leaves),
        None => page_bytes.saturating_mul(jobs),
    };
    if gate_bytes < page_bytes {
        return Err(resource(
            "one leaf job cannot fit available memory including owned scratch",
        ));
    }
    Ok(StagePlan {
        page_bytes,
        gate_bytes,
        cache_bytes,
        global_bytes,
        leaf_bytes,
        window,
        jobs,
        jobs_limited_by: limited_by,
        available_bytes: available,
    })
}
fn write_file(path: &Path, bytes: &[u8]) -> Result<(), JobError> {
    let mut file = File::create(path).map_err(|e| JobError::io("create leaf member", path, e))?;
    file.write_all(bytes)
        .and_then(|_| file.sync_all())
        .map_err(|e| JobError::io("write leaf member", path, e))
}
/// Retry only the physical member budget, never fidelity or other failures.
fn encode_with_density_budget<T>(
    profile: Profile,
    initial_drop: u32,
    mut encode: impl FnMut(u32) -> Result<T, atlas::PrepareError>,
) -> Result<(T, u32, Vec<serde_json::Value>), JobError> {
    let mut density_drop = initial_drop;
    let mut reasons = Vec::new();
    loop {
        match encode(density_drop) {
            Ok(encoded) => return Ok((encoded, density_drop, reasons)),
            Err(atlas::PrepareError::Budget(error))
                if profile != Profile::Lossless && density_drop < 12 =>
            {
                reasons.push(json!({"densityDrop":density_drop,"reason":error.to_string()}));
                density_drop += 1;
            }
            Err(atlas::PrepareError::Budget(error) | atlas::PrepareError::Failure(error)) => {
                return Err(error);
            }
        }
    }
}
/// Keep the writer's retryable-budget / hard-failure split when its error
/// enters the density retry loop: only `Budget` may lower texture density.
fn write_error_class(error: writer::WriteError) -> atlas::PrepareError {
    match error {
        writer::WriteError::Budget(e) => atlas::PrepareError::Budget(e),
        writer::WriteError::Failure(e) => atlas::PrepareError::Failure(e),
    }
}
/// One leaf's frozen inputs, read serially from the captured streams.
struct LeafInput {
    node: NodeRecord,
    plan: LeafPlan,
    records: Vec<FullTriangle>,
    corner_depths: Vec<[u8; 3]>,
}
/// A leaf whose member is already written by its worker; only its identity
/// and receipts travel to the ordered manifest writer.
struct LeafEncoded {
    name: String,
    sha256: String,
    bytes: u64,
    bounds: super::super::records::Bounds,
    geometry_error: f64,
    appearance: Vec<serde_json::Value>,
    density_drop: u32,
    density_reasons: Vec<serde_json::Value>,
}
fn read_leaf(
    node: NodeRecord,
    triangles: &mut SequenceReader<'_, FullTriangle>,
    depths: &mut SequenceReader<'_, CornerDepth>,
    plans: &mut SequenceReader<'_, LeafPlan>,
    check: &dyn Fn() -> Result<(), JobError>,
) -> Result<LeafInput, JobError> {
    let count = usize::try_from(node.triangles()).map_err(|_| resource("leaf count overflow"))?;
    let mut records = Vec::new();
    records
        .try_reserve_exact(count)
        .map_err(|_| resource("leaf triangle allocation"))?;
    let mut corner_depths = Vec::new();
    corner_depths
        .try_reserve_exact(count)
        .map_err(|_| resource("leaf depth allocation"))?;
    for i in 0..count {
        if i.is_multiple_of(4096) {
            check()?;
        }
        records.push(
            triangles
                .next()
                .transpose()?
                .ok_or_else(|| invariant("truncated full leaf triangles"))?,
        );
        corner_depths.push(
            depths
                .next()
                .transpose()?
                .ok_or_else(|| invariant("truncated leaf corner depths"))?
                .depths,
        );
    }
    let plan = plans
        .next()
        .transpose()?
        .ok_or_else(|| invariant("truncated frozen leaf plans"))?;
    if plan.first != node.first || plan.end != node.end || u32::from(plan.depth) != node.depth() {
        return Err(invariant("leaf payload plan differs from frozen tree"));
    }
    Ok(LeafInput {
        node,
        plan,
        records,
        corner_depths,
    })
}
fn encode_leaf(
    input: &LeafInput,
    services: &atlas::Services<'_>,
    config: &TreeConfig,
    leaves: &Path,
    index: u64,
) -> Result<LeafEncoded, JobError> {
    let node = &input.node;
    let captured = services.captured;
    let ((glb, appearance), density_drop, density_reasons) = encode_with_density_budget(
        config.profile,
        u32::from(input.plan.density_drop),
        |density_drop| {
            (services.check)().map_err(atlas::PrepareError::from)?;
            atlas::prepare(
                (input.records.clone(), input.corner_depths.clone()),
                services,
                config.profile,
                node.depth() as u8,
                density_drop,
            )
            .and_then(|(atlas, receipt)| {
                timing::time(Stage::GlbWrite, || {
                    writer::encode(
                        &atlas,
                        &captured.lattice,
                        node.depth() as u8,
                        config.tile_bytes(),
                    )
                })
                .map_err(write_error_class)
                .map(|leaf| (leaf, receipt))
            })
        },
    )?;
    // Members are named by frozen leaf order, so workers write them in
    // parallel; the manifest row is still appended strictly in order.
    let name = format!("{index:012}.glb");
    let member = leaves.join(&name);
    timing::time(Stage::MemberWrite, || write_file(&member, &glb.glb))?;
    Ok(LeafEncoded {
        name,
        sha256: hex(Sha256::digest(&glb.glb)),
        bytes: glb.glb.len() as u64,
        bounds: glb.bounds,
        geometry_error: glb.geometry_error,
        appearance,
        density_drop,
        density_reasons,
    })
}

/// Leaves are read in frozen order, encoded by a pool of `jobs` workers (pages
/// of one leaf also run in parallel), and written strictly in leaf order, so
/// every member, manifest row and the first reported failure are independent
/// of the worker count.
fn produce(
    path: &Path,
    captured: &full::FullCaptured,
    config: &TreeConfig,
    stage: &StagePlan,
    profile: LeafProfile,
    check: &(dyn Fn() -> Result<(), JobError> + Sync),
) -> Result<LeafOutputReport, JobError> {
    let leaves = path.join("leaves");
    fs::create_dir(&leaves).map_err(|e| JobError::io("create leaf directory", &leaves, e))?;
    let manifest_path = path.join("leaves.jsonl");
    let file = File::create(&manifest_path)
        .map_err(|e| JobError::io("create leaf manifest", &manifest_path, e))?;
    let mut manifest = BufWriter::new(file);
    let mut triangles = captured.triangles.reader()?;
    let mut depths = captured.corner_depths.reader()?;
    let mut plans = captured.leaf_plans.reader()?;
    let mut report = LeafOutputReport {
        schema_version: 1,
        profile,
        triangles: 0,
        leaves: 0,
        glb_bytes: 0,
        maximum_glb_bytes: 0,
        maximum_geometry_error: 0.,
        density_reduced_leaves: 0,
    };
    let jobs = usize::try_from(stage.jobs).map_err(|_| resource("delivery jobs overflow"))?;
    let window = usize::try_from(stage.window).map_err(|_| resource("delivery window overflow"))?;
    let pool = rayon::ThreadPoolBuilder::new()
        .num_threads(jobs)
        .thread_name(|i| format!("rt-leaf-{i}"))
        .build()
        .map_err(|e| resource(&format!("delivery worker pool: {e}")))?;
    let cache = SourceCache::new(stage.cache_bytes);
    let gate = MemoryGate::new(stage.gate_bytes);
    // Set when the ordered writer stops: queued leaves then end promptly.
    let stopped = AtomicBool::new(false);
    let leaf_check = || {
        if stopped.load(Ordering::Relaxed) {
            return Err(JobError::new(
                JobErrorKind::Cancelled,
                "an earlier leaf ended the delivery stage",
            ));
        }
        check()
    };
    let services = atlas::Services {
        captured,
        cache: &cache,
        gate: &gate,
        memory: stage.page_bytes,
        check: &leaf_check,
    };
    let mut nodes = captured.tree.nodes.reader()?;
    let result = pool.in_place_scope(|scope| -> Result<(), JobError> {
        type Done = Result<(LeafEncoded, LeafInput), JobError>;
        let (sender, receiver) = mpsc::channel::<(u64, Done)>();
        let mut pending = BTreeMap::new();
        let (mut spawned, mut written) = (0u64, 0u64);
        let mut exhausted = false;
        let mut next_first = 0u64;
        let outcome = (|| -> Result<(), JobError> {
            loop {
                while !exhausted && spawned - written < window as u64 {
                    let node = loop {
                        match nodes.next().transpose()? {
                            None => break None,
                            Some(node) if node.kind == NodeKind::Leaf => break Some(node),
                            Some(_) => {}
                        }
                    };
                    let Some(node) = node else {
                        exhausted = true;
                        break;
                    };
                    check()?;
                    crate::spill::preflight(
                        &leaves,
                        TILE_BYTES + (1 << 20),
                        crate::spill::SpaceRole::Output,
                        crate::spill::PreflightStage::Texture,
                    )?;
                    if node.first != next_first || node.triangles() > config.leaf_triangles() {
                        return Err(invariant(
                            "leaf ranges or triangle limit differ from frozen tree",
                        ));
                    }
                    next_first = node.end;
                    let input = read_leaf(node, &mut triangles, &mut depths, &mut plans, check)?;
                    let index = spawned;
                    let sender = sender.clone();
                    let services = &services;
                    let leaves_dir = leaves.as_path();
                    scope.spawn(move |_| {
                        // A panicking worker still reports, so the ordered
                        // writer never waits for a result that cannot come.
                        let encoded =
                            std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                                encode_leaf(&input, services, config, leaves_dir, index)
                            }))
                            .unwrap_or_else(|_| Err(invariant("leaf worker panicked")));
                        // The receiver outlives every task of this scope.
                        let _ = sender.send((index, encoded.map(|e| (e, input))));
                    });
                    spawned += 1;
                }
                if spawned == written {
                    return Ok(());
                }
                let (index, encoded) = receiver
                    .recv()
                    .map_err(|_| invariant("leaf worker ended without a result"))?;
                pending.insert(index, encoded);
                while let Some(encoded) = pending.remove(&written) {
                    let (encoded, input) = encoded?;
                    write_leaf(&mut manifest, &manifest_path, &mut report, &input, encoded)?;
                    written += 1;
                }
            }
        })();
        if outcome.is_err() {
            stopped.store(true, Ordering::Relaxed);
        }
        outcome
    });
    result?;
    if report.triangles != captured.scan.triangles
        || report.leaves != captured.tree.stats.leaves
        || triangles.next().transpose()?.is_some()
        || depths.next().transpose()?.is_some()
        || plans.next().transpose()?.is_some()
    {
        return Err(invariant(
            "full leaf output does not cover captured source exactly",
        ));
    }
    manifest
        .flush()
        .and_then(|_| manifest.get_ref().sync_all())
        .map_err(|e| JobError::io("finish leaf manifest", &manifest_path, e))?;
    let doc = json!({"schemaVersion":1,"kind":"full_detail_leaf_payloads","report":report,"lattice":captured.lattice,"provenanceSchema":"rusty_tiles_large_source_v1","leafManifest":"leaves.jsonl","losslessMipGuarantee":"PNG level0 exact; viewer-generated minification levels outside guarantee"});
    write_file(
        &path.join("manifest.json"),
        &serde_json::to_vec_pretty(&doc).map_err(|e| invariant(&e.to_string()))?,
    )?;
    let resources = json!({"capture":captured.report,"deliveryJobs":stage.jobs,"delivery":stage,"pageGatePeakBytes":gate.peak(),"sourceCache":cache.stats(),"nativeWorkspace":"conservative provisional charge; measured scale acceptance recorded separately"});
    write_file(
        &path.join("resources.json"),
        &serde_json::to_vec_pretty(&resources).map_err(|e| invariant(&e.to_string()))?,
    )?;
    let source_texels: u64 = captured
        .image_info
        .iter()
        .map(|i| u64::from(i.width) * u64::from(i.height))
        .sum();
    timing::finish(
        json!({"profile":profile,"leaves":report.leaves,"triangles":report.triangles,"glbBytes":report.glb_bytes,"sourceImages":captured.image_info.len(),"sourceTexels":source_texels,"delivery":stage,"pageGatePeakBytes":gate.peak(),"sourceCache":cache.stats()}),
    );
    Ok(report)
}

fn write_leaf(
    manifest: &mut BufWriter<File>,
    manifest_path: &Path,
    report: &mut LeafOutputReport,
    input: &LeafInput,
    encoded: LeafEncoded,
) -> Result<(), JobError> {
    let node = &input.node;
    let plan = &input.plan;
    let LeafEncoded {
        name,
        sha256,
        bytes,
        bounds,
        geometry_error,
        appearance,
        density_drop,
        density_reasons,
    } = encoded;
    if name != format!("{:012}.glb", report.leaves) {
        return Err(invariant("leaf member order differs from manifest order"));
    }
    timing::count(Counter::Leaves, 1);
    let receipt = json!({"schemaVersion":1,"uri":format!("leaves/{name}"),"sha256":sha256,"bytes":bytes,"first":node.first,"end":node.end,"cellDepth":node.cell_depth,"cell":node.cell,"pathLength":node.path_len,"path":node.path,"lastOctant":node.last_octant,"depth":node.depth(),"packed":node.is_packed(),"bounds":{"min":bounds.min.map(f64::from),"max":bounds.max.map(f64::from)},"geometryError":geometry_error,"densityDrop":density_drop,"densityReasons":density_reasons,"plannedFootprint":{"densityDrop":plan.density_drop,"pages":plan.pages,"baseTexels":plan.base_texels,"chainTexels":plan.chain_texels,"geometryBytes":plan.geometry_bytes,"planningOwnedBytes":plan.owned_bytes},"appearance":appearance});
    serde_json::to_writer(&mut *manifest, &receipt)
        .map_err(|e| invariant(&format!("leaf manifest serialization: {e}")))?;
    manifest
        .write_all(b"\n")
        .map_err(|e| JobError::io("write leaf manifest", manifest_path, e))?;
    report.triangles = report
        .triangles
        .checked_add(node.triangles())
        .ok_or_else(|| resource("triangle report overflow"))?;
    report.leaves += 1;
    report.glb_bytes = report
        .glb_bytes
        .checked_add(bytes)
        .ok_or_else(|| resource("leaf output size overflow"))?;
    report.maximum_glb_bytes = report.maximum_glb_bytes.max(bytes);
    report.maximum_geometry_error = report.maximum_geometry_error.max(geometry_error);
    report.density_reduced_leaves += u64::from(density_drop != 0);
    Ok(())
}

pub fn mesh_to_leaf_payloads(
    request: LeafOutputRequest,
    control: &RunControl,
) -> Result<LeafOutputResult, JobFailure> {
    let attempt = control.begin()?;
    let target = DirectoryTarget::prepare(&request.output, OutputPolicy::CreateNew)
        .map_err(|e| attempt.fail(e))?;
    let mut config = TreeConfig::new(request.profile.into());
    config.jobs = request.jobs;
    config.scratch_dir = request.scratch_dir;
    let captured = match timing::time(Stage::Capture, || {
        full::capture(&request.input, &config, &mut || attempt.check())
    }) {
        Ok(captured) => captured,
        Err(failure) => {
            let mut finished = attempt.fail(failure.error);
            finished.secondary.extend(failure.secondary);
            finished.retained_paths.extend(failure.retained_paths);
            finished.recovery = failure.recovery;
            return Err(finished);
        }
    };
    let stage_plan = match stage_plan(config.profile, &captured, config.jobs) {
        Ok(plan) => plan,
        Err(error) => {
            let mut failed = attempt.fail(error);
            close_failed_capture(captured, &mut failed);
            return Err(failed);
        }
    };
    let stage = match target.stage_pending(&attempt) {
        Ok(stage) => stage,
        Err(error) => {
            let mut failed = attempt.fail(error);
            close_failed_capture(captured, &mut failed);
            return Err(failed);
        }
    };
    let result = produce(
        stage.path(),
        &captured,
        &config,
        &stage_plan,
        request.profile,
        &|| attempt.check(),
    );
    let cleanup_path = captured.scratch.path().to_owned();
    let cleanup = captured.close();
    let report = match result {
        Ok(report) => report,
        Err(error) => {
            let mut failed = stage.fail(error);
            if let Err(error) = cleanup {
                failed.secondary.push(error);
                if cleanup_path.exists() {
                    failed.retained_paths.push(cleanup_path);
                }
            }
            return Err(failed);
        }
    };
    if let Err(error) = cleanup {
        let mut failed = stage.fail(error);
        if cleanup_path.exists() {
            failed.retained_paths.push(cleanup_path);
        }
        return Err(failed);
    }
    if let Err(error) = attempt
        .emit(&RunEvent::Progress {
            phase: "mesh_leaf_payloads",
            done: report.leaves,
            total: Some(report.leaves),
        })
        .and_then(|_| attempt.close_events())
    {
        return Err(stage.fail(error));
    }
    let published = stage.seal()?.publish()?;
    Ok(LeafOutputResult {
        output: published.output,
        report,
        cleanup_diagnostics: published.cleanup_diagnostics,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn fidelity_failure_never_retries_but_physical_budget_advances_density() {
        let mut attempts = Vec::new();
        let error = encode_with_density_budget(Profile::Balanced, 2, |drop| {
            attempts.push(drop);
            if drop == 2 {
                Err(atlas::PrepareError::Failure(JobError::new(
                    JobErrorKind::Unsupported,
                    "forced MASK fidelity residual",
                )))
            } else {
                Ok(())
            }
        })
        .unwrap_err();
        assert_eq!(error.kind(), JobErrorKind::Unsupported);
        assert_eq!(attempts, [2]);

        attempts.clear();
        let (_, drop, reasons) = encode_with_density_budget(Profile::Balanced, 2, |drop| {
            attempts.push(drop);
            if drop == 2 {
                Err(atlas::PrepareError::Budget(JobError::new(
                    JobErrorKind::Unsupported,
                    "physical GLB byte budget",
                )))
            } else {
                Ok(())
            }
        })
        .unwrap();
        assert_eq!(attempts, [2, 3]);
        assert_eq!(drop, 3);
        assert_eq!(reasons.len(), 1);
        assert_eq!(reasons[0]["densityDrop"], 2);
        for (profile, initial) in [(Profile::Lossless, 0), (Profile::Balanced, 12)] {
            attempts.clear();
            let result = encode_with_density_budget::<()>(profile, initial, |drop| {
                attempts.push(drop);
                Err(atlas::PrepareError::Budget(JobError::new(
                    JobErrorKind::Unsupported,
                    "physical GLB byte budget",
                )))
            });
            assert!(result.is_err());
            assert_eq!(attempts, [initial]);
        }
    }
    /// A writer `Failure` (invalid state, consumer row limit, frame escape) must
    /// reach the caller after one attempt; reclassified as `Budget` it would
    /// silently coarsen textures and retry up to twelve times.
    #[test]
    fn writer_failure_is_never_a_density_retry_but_writer_budget_is() {
        for (writer_error, retried) in [
            (
                writer::WriteError::Failure(JobError::new(
                    JobErrorKind::InvalidState,
                    "writer failure",
                )),
                false,
            ),
            (
                writer::WriteError::Budget(JobError::new(
                    JobErrorKind::ResourceLimit,
                    "member budget",
                )),
                true,
            ),
        ] {
            let kind = writer_error.kind();
            let mut attempts = Vec::new();
            let mut writer_error = Some(writer_error);
            let result = encode_with_density_budget(Profile::Balanced, 2, |drop| {
                attempts.push(drop);
                match writer_error.take() {
                    Some(error) => Err(write_error_class(error)),
                    None => Ok(()),
                }
            });
            if retried {
                let (_, drop, reasons) = result.unwrap();
                assert_eq!(
                    (attempts.as_slice(), drop, reasons.len()),
                    (&[2, 3][..], 3, 1)
                );
            } else {
                assert_eq!(result.unwrap_err().kind(), kind);
                assert_eq!(attempts, [2]);
            }
        }
    }
    #[test]
    fn real_leaf_output_all_profiles_and_exact_member_limit() {
        let work = tempfile::tempdir().unwrap();
        let input = full::tests::fixture(work.path(), 2);
        for (name, profile) in [
            ("fast", LeafProfile::Fast),
            ("balanced", LeafProfile::Balanced),
            ("quality", LeafProfile::Quality),
            ("lossless", LeafProfile::Lossless),
        ] {
            let output = work.path().join(name);
            let result = mesh_to_leaf_payloads(
                LeafOutputRequest::new(&input, &output, profile)
                    .with_jobs(NonZeroUsize::new(1).unwrap()),
                &RunControl::default(),
            )
            .unwrap();
            assert_eq!(result.report.triangles, 2);
            assert_eq!(result.report.leaves, 1);
            assert!(result.report.maximum_glb_bytes <= TILE_BYTES);
            let manifest: serde_json::Value =
                serde_json::from_slice(&fs::read(output.join("manifest.json")).unwrap()).unwrap();
            assert_eq!(manifest["kind"], "full_detail_leaf_payloads");
            assert!(!output.join("tileset.json").exists());
            let leaf = fs::read(output.join("leaves/000000000000.glb")).unwrap();
            assert_eq!(&leaf[..4], b"glTF");
            assert_eq!(
                u32::from_le_bytes(leaf[8..12].try_into().unwrap()) as usize,
                leaf.len()
            );
            let json_len = u32::from_le_bytes(leaf[12..16].try_into().unwrap()) as usize;
            let document: serde_json::Value =
                serde_json::from_slice(&leaf[20..20 + json_len]).unwrap();
            for sampler in document["samplers"].as_array().unwrap() {
                assert_eq!(
                    sampler["minFilter"], 9987,
                    "unspecified source minification uses authored mips"
                );
            }
        }
    }
    #[test]
    fn synthetic_depth_payloads_are_identical_across_jobs_and_paths() {
        let work = tempfile::tempdir().unwrap();
        let input = full::tests::fixture(work.path(), 8193);
        let mut previous = None;
        for jobs in [1, 2] {
            let scratch = work.path().join(format!("scratch{jobs}"));
            fs::create_dir(&scratch).unwrap();
            let output = work.path().join(format!("output{jobs}"));
            let result = mesh_to_leaf_payloads(
                LeafOutputRequest::new(&input, &output, LeafProfile::Lossless)
                    .with_jobs(NonZeroUsize::new(jobs).unwrap())
                    .with_scratch_dir(&scratch),
                &RunControl::default(),
            )
            .unwrap();
            assert_eq!(result.report.triangles, 8193);
            assert!(result.report.leaves >= 2);
            assert!(fs::read_dir(scratch).unwrap().next().is_none());
            let rows = fs::read_to_string(output.join("leaves.jsonl")).unwrap();
            let mut files = vec![
                fs::read(output.join("manifest.json")).unwrap(),
                rows.as_bytes().to_vec(),
            ];
            for row in rows.lines() {
                let row: serde_json::Value = serde_json::from_str(row).unwrap();
                assert!(row["depth"].as_u64().unwrap() > 21);
                files.push(fs::read(output.join(row["uri"].as_str().unwrap())).unwrap());
            }
            if let Some(ref prior) = previous {
                assert_eq!(prior, &files);
            } else {
                previous = Some(files);
            }
        }
    }
    /// Triangles on a spatial grid; every 64 consecutive triangles share a
    /// small chart at a pseudo-random place in one procedural 1024x1024
    /// texture, like a photogrammetry atlas: a leaf's hull covers most of the
    /// texture while its charts are a small part of it.
    fn scattered_fixture(parent: &Path, count: usize, min_filter: u32) -> PathBuf {
        fs::create_dir_all(parent).unwrap();
        let mut state = 0x9e37_79b9_7f4a_7c15u64;
        let mut next = || {
            state = state
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            ((state >> 33) as f32) / (1u64 << 31) as f32
        };
        let (mut positions, mut uvs) = (Vec::new(), Vec::new());
        let side = (count as f64).sqrt().ceil() as usize;
        let chart = 6. / 1024.;
        let (mut u0, mut v0) = (0., 0.);
        for i in 0..count {
            let (x, y) = ((i % side) as f32, (i / side) as f32);
            for p in [[x, y, 0.], [x + 1., y, 0.], [x, y + 1., 0.]] {
                positions.extend(p);
            }
            if i % 64 == 0 {
                (u0, v0) = (next() * 0.95 + 0.02, next() * 0.95 + 0.02);
            }
            let k = i % 64;
            let (u, v) = (
                u0 + (k % 8) as f32 * chart / 8.,
                v0 + (k / 8) as f32 * chart / 8.,
            );
            for p in [[u, v], [u + chart / 8., v], [u, v + chart / 8.]] {
                uvs.extend(p);
            }
        }
        let mut bytes = Vec::new();
        for value in positions.iter().chain(&uvs) {
            bytes.extend(value.to_le_bytes());
        }
        fs::write(parent.join("attributes.bin"), &bytes).unwrap();
        // Smooth gradients with mild texel detail, like photographed surfaces.
        let texture = image::RgbaImage::from_fn(1024, 1024, |x, y| {
            // Every channel stays below 256 for x, y < 1024.
            let detail = (x * 37 + y * 91) % 13;
            image::Rgba([
                (x / 8 + detail) as u8,
                (y / 8 + 40) as u8,
                ((x + y) / 16 + detail) as u8,
                255,
            ])
        });
        texture.save(parent.join("texture.png")).unwrap();
        let split = positions.len() * 4;
        let doc = json!({"asset":{"version":"2.0"},"scene":0,"scenes":[{"nodes":[0]}],"nodes":[{"mesh":0}],
            "meshes":[{"primitives":[{"attributes":{"POSITION":0,"TEXCOORD_0":1},"material":0}]}],
            "buffers":[{"uri":"attributes.bin","byteLength":bytes.len()}],
            "bufferViews":[{"buffer":0,"byteOffset":0,"byteLength":split},{"buffer":0,"byteOffset":split,"byteLength":bytes.len()-split}],
            "accessors":[{"bufferView":0,"componentType":5126,"count":count*3,"type":"VEC3","min":[0,0,0],"max":[positions.chunks(3).map(|p| p[0]).fold(0f32, f32::max),positions.chunks(3).map(|p| p[1]).fold(0f32, f32::max),0]},
                         {"bufferView":1,"componentType":5126,"count":count*3,"type":"VEC2"}],
            "images":[{"uri":"texture.png"}],"textures":[{"source":0,"sampler":0}],
            "samplers":[{"minFilter":min_filter,"magFilter":9729}],
            "materials":[{"pbrMetallicRoughness":{"baseColorTexture":{"index":0}}}]});
        let input = parent.join("scene.gltf");
        fs::write(&input, serde_json::to_vec(&doc).unwrap()).unwrap();
        input
    }
    fn output_files(output: &Path) -> Vec<Vec<u8>> {
        let rows = fs::read_to_string(output.join("leaves.jsonl")).unwrap();
        let mut files = vec![
            fs::read(output.join("manifest.json")).unwrap(),
            rows.as_bytes().to_vec(),
        ];
        for row in rows.lines() {
            let row: serde_json::Value = serde_json::from_str(row).unwrap();
            files.push(fs::read(output.join(row["uri"].as_str().unwrap())).unwrap());
        }
        files
    }
    #[test]
    fn scattered_atlas_islands_are_identical_across_jobs_and_shrink_pages() {
        let work = tempfile::tempdir().unwrap();
        let input = scattered_fixture(&work.path().join("source"), 20_000, 9729);
        let mut previous = None;
        for jobs in [1, 3] {
            let output = work.path().join(format!("balanced{jobs}"));
            let result = mesh_to_leaf_payloads(
                LeafOutputRequest::new(&input, &output, LeafProfile::Balanced)
                    .with_jobs(NonZeroUsize::new(jobs).unwrap()),
                &RunControl::default(),
            )
            .unwrap();
            assert_eq!(result.report.triangles, 20_000);
            assert!(result.report.leaves >= 2);
            let files = output_files(&output);
            for row in fs::read_to_string(output.join("leaves.jsonl"))
                .unwrap()
                .lines()
            {
                let row: serde_json::Value = serde_json::from_str(row).unwrap();
                let bindings: Vec<_> = row["appearance"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .filter(|r| r.get("islands").is_some())
                    .collect();
                assert!(!bindings.is_empty());
                for binding in bindings {
                    // Scattered footprints become many small islands whose
                    // real texels are a small part of the 256x256 hull.
                    assert!(binding["islands"].as_array().unwrap().len() > 1);
                    assert!(binding["islandInteriorTexels"].as_u64().unwrap() < 1024 * 1024 / 5);
                }
                for page in row["appearance"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .filter(|r| r.get("levels").is_some())
                {
                    assert!(
                        page["width"].as_u64().unwrap() * page["height"].as_u64().unwrap()
                            <= 1024 * 1024 / 4
                    );
                }
            }
            if let Some(ref prior) = previous {
                assert_eq!(prior, &files, "jobs {jobs} changed published bytes");
            } else {
                previous = Some(files);
            }
        }
    }
    #[test]
    fn lossless_mipmapped_islands_carry_aligned_eight_texel_context() {
        let work = tempfile::tempdir().unwrap();
        let input = scattered_fixture(&work.path().join("source"), 640, 9987);
        let output = work.path().join("lossless");
        mesh_to_leaf_payloads(
            LeafOutputRequest::new(&input, &output, LeafProfile::Lossless)
                .with_jobs(NonZeroUsize::new(2).unwrap()),
            &RunControl::default(),
        )
        .unwrap();
        let rows = fs::read_to_string(output.join("leaves.jsonl")).unwrap();
        let row: serde_json::Value = serde_json::from_str(rows.lines().next().unwrap()).unwrap();
        let binding = row["appearance"]
            .as_array()
            .unwrap()
            .iter()
            .find(|r| r.get("islands").is_some())
            .unwrap();
        let islands = binding["islands"].as_array().unwrap();
        assert!(islands.len() > 1);
        assert_eq!(binding["islandGutter"], 8);
        for island in islands {
            for axis in 0..2 {
                // Allocation origin and size are multiples of eight texels.
                let interior = island[2 + axis].as_u64().unwrap();
                let page = island[4 + axis].as_u64().unwrap();
                assert_eq!((page - 8) % 8, 0, "{island}");
                assert_eq!((interior + 16) % 8, 0, "{island}");
            }
        }
    }
    #[test]
    fn cancelled_output_does_not_create_a_private_or_public_tree() {
        let work = tempfile::tempdir().unwrap();
        let input = full::tests::fixture(work.path(), 1);
        let output = work.path().join("cancelled");
        let control = RunControl::default();
        control.cancellation_handle().cancel();
        let failure = mesh_to_leaf_payloads(
            LeafOutputRequest::new(input, &output, LeafProfile::Lossless),
            &control,
        )
        .unwrap_err();
        assert_eq!(failure.error.kind(), JobErrorKind::Cancelled);
        assert!(!output.exists());
        assert!(!fs::read_dir(work.path()).unwrap().any(|entry| entry
            .unwrap()
            .file_name()
            .to_string_lossy()
            .starts_with(".tiles-dir-")));
    }
}
