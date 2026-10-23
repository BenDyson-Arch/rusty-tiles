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
use crate::mesh_large::phases::{Phase, Phases};
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

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
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
    lod_mutation: super::super::lod::Mutation,
    lod_target: usize,
    baker: Option<std::sync::Arc<dyn super::super::bake_pass::ProxyBaker>>,
    lod: super::super::hlod::TilerLod,
    memory_budget: Option<u64>,
    hlod: super::super::hlod::Settings,
    #[cfg(test)]
    leaf_triangles: Option<u64>,
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
            lod_mutation: super::super::lod::Mutation::None,
            lod_target: super::super::lod::TARGET_FACES,
            baker: None,
            lod: super::super::hlod::TilerLod::H17,
            memory_budget: None,
            hlod: super::super::hlod::Settings::default(),
            #[cfg(test)]
            leaf_triangles: None,
        }
    }
    /// Build coarse tiles with the 0.4 HLOD stage instead of H17.
    pub(crate) fn with_lod(mut self, lod: super::super::hlod::TilerLod) -> Self {
        self.lod = lod;
        self
    }
    /// Declared memory budget (HLOD runs).
    pub(crate) fn with_memory_budget(mut self, bytes: u64) -> Self {
        self.memory_budget = Some(bytes);
        self
    }
    /// Test controls: HLOD triangle cap and mutations.
    #[cfg(test)]
    pub(crate) fn with_hlod_settings(mut self, settings: super::super::hlod::Settings) -> Self {
        self.hlod = settings;
        self
    }
    pub fn with_jobs(mut self, jobs: NonZeroUsize) -> Self {
        self.jobs = Jobs::Fixed(jobs);
        self
    }
    pub fn with_scratch_dir(mut self, path: impl Into<PathBuf>) -> Self {
        self.scratch_dir = Some(path.into());
        self
    }
    /// Test controls: run the coarse levels with a certificate mutation.
    #[cfg(test)]
    pub(crate) fn with_lod_mutation(mut self, mutation: super::super::lod::Mutation) -> Self {
        self.lod_mutation = mutation;
        self
    }
    /// Bake the coarse tiles' appearance (H13) after they are certified. Without a baker
    /// the proxies stay untextured.
    pub(in crate::mesh_large) fn with_baker(
        mut self,
        baker: std::sync::Arc<dyn super::super::bake_pass::ProxyBaker>,
    ) -> Self {
        self.baker = Some(baker);
        self
    }
    /// Test fixtures: a smaller coarse-tile face target, so small inputs get deep LOD.
    #[cfg(test)]
    pub(crate) fn with_lod_target(mut self, target: usize) -> Self {
        self.lod_target = target;
        self
    }
    /// Test fixtures: smaller leaves, so small inputs get several leaves and coarse levels.
    #[cfg(test)]
    pub(crate) fn with_leaf_triangles(mut self, leaf_triangles: u64) -> Self {
        self.leaf_triangles = Some(leaf_triangles);
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
    /// H17 coarse tiles (untextured certified proxies) written beside the leaves.
    pub proxies: u64,
    pub proxy_bytes: u64,
    /// Lossy pages delivered by a stronger UASTC step after their profile's
    /// own setting missed a decoded-image gate, by the step that passed.
    pub escalated_pages: EscalatedPages,
    /// Pages delivered as exact level-0 PNG after every lossy step missed.
    pub lossless_fallback_pages: u64,
    /// Escalated pages (both counts above) by what the profile step missed.
    pub escalation_reasons: EscalationReasons,
    /// The first [`ESCALATION_DIAGNOSTICS`] escalated pages in leaf order.
    pub escalation_diagnostics: Vec<EscalationDiagnostic>,
    /// Escalated pages beyond the bounded diagnostics list.
    pub escalation_diagnostics_omitted: u64,
    /// H28 texture-space editability of the published tiles.
    pub texel_ownership: TexelOwnership,
}
/// H28 texture-space editability: a tile is `unique` when no two of its triangles'
/// published decoded-UV footprints share interior area on any page, so a texel edit
/// changes exactly one triangle; otherwise `shared` (overlapping, mirrored or repeated
/// source UVs), and the tile is editable in vertex space only. Never a refusal.
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub struct TexelOwnership {
    pub unique_leaves: u64,
    pub shared_leaves: u64,
    /// Triangles of shared leaves whose footprint shares area with another's.
    pub shared_triangles: u64,
    /// Coarse tiles: unique by construction (non-overlapping charts on disjoint packed
    /// rectangles, and the raster's double-coverage refusal).
    pub unique_proxies: u64,
    /// The first [`TEXEL_OWNERSHIP_DIAGNOSTICS`] shared leaves, in leaf order; every
    /// leaf not listed (and not omitted) is unique.
    pub shared: Vec<SharedTexels>,
    pub shared_omitted: u64,
}
/// Bound of [`TexelOwnership::shared`].
pub const TEXEL_OWNERSHIP_DIAGNOSTICS: usize = 64;
/// One leaf whose texels are not uniquely owned.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct SharedTexels {
    /// Leaf member, `leaves/<index>.glb` in the leaf stage.
    pub leaf: String,
    /// Archive tile `f<first>-d<depth>`.
    pub first: u64,
    pub depth: u32,
    /// Always `shared`.
    pub texel_ownership: &'static str,
    /// Triangles whose footprint shares area with another's.
    pub triangles: u64,
}
/// Escalated pages by the profile step's miss: a decoded-image gate, or the
/// codec's own verification floor.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize)]
pub struct EscalationReasons {
    pub gate: u64,
    pub codec_verify: u64,
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize)]
pub struct EscalatedPages {
    pub uastc_level2: u64,
    pub uastc_level3: u64,
}
/// Bound of [`LeafOutputReport::escalation_diagnostics`].
pub const ESCALATION_DIAGNOSTICS: usize = 32;
/// One escalated page: its leaf, slot, sources, delivering step and every
/// gate it missed on the way.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct EscalationDiagnostic {
    /// Leaf member, `leaves/<index>.glb` in the leaf stage.
    pub leaf: String,
    /// The leaf's first source triangle rank and depth: archive tile
    /// `content/f<first>-d<depth>.glb` when explicit.
    pub first: u64,
    pub depth: u32,
    pub page: u64,
    pub slot: &'static str,
    pub sources: Vec<EscalationSource>,
    pub step: &'static str,
    pub misses: Vec<EscalationMiss>,
}
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct EscalationSource {
    pub image: u64,
    pub material: u32,
}
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct EscalationMiss {
    pub step: &'static str,
    /// `gate` or `codec_verify`.
    pub reason: &'static str,
    pub mip: Option<u64>,
    pub gate: String,
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
pub(crate) struct StagePlan {
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
    bound_bytes: u64,
    /// The least available memory (memory-backed scratch included) admitting one leaf job.
    minimum_bytes: u64,
}
/// Bytes reserved for the runtime, allocator slack and the output writer.
const STAGE_RESERVE: u64 = 256 << 20;
pub(crate) fn stage_plan(
    config: &TreeConfig,
    captured: &full::FullCaptured,
) -> Result<StagePlan, JobError> {
    let profile = config.profile;
    let requested = config.jobs;
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
    // Image-free input builds no page, so it reserves no texture workspace (D8).
    let page_bytes = if captured.image_info.is_empty() {
        0
    } else {
        texture
            .checked_add(if profile == Profile::Lossless {
                0
            } else {
                (128u64 << 20)
                    .checked_add(pixels.checked_mul(6).ok_or_else(|| resource("page"))?)
                    .ok_or_else(|| resource("page"))?
            })
            .and_then(|n| n.checked_add(largest_source.checked_mul(32)?))
            .and_then(|n| n.checked_add(64 << 20))
            .ok_or_else(|| resource("leaf workspace overflow"))?
    };
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
    let available = config.available_memory();
    let cores = std::thread::available_parallelism().map_or(1, |n| n.get()) as u64;
    // Cache every decoded source when it fits half the usable memory;
    // otherwise half the usable memory, and always at least two images.
    let cache_for = |usable: Option<u64>| match usable {
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
    let fixed_for = |cache: u64| {
        cache
            .checked_add(page_bytes)
            .and_then(|n| n.checked_add(2 * leaf_owned))
            .ok_or_else(|| resource("leaf fixed memory overflow"))
    };
    // One leaf job fits usable memory `u` when `u >= cache(u) + page + one job's window`,
    // and `u - cache(u)` never decreases in `u`: the least such `u`, by bisection.
    let fits = |u: u64| -> bool {
        fixed_for(cache_for(Some(u)))
            .ok()
            .and_then(|f| f.checked_add(per_job))
            .is_some_and(|need| u >= need)
    };
    let minimum_usable = {
        let (mut lo, mut hi) = (0u64, u64::MAX / 2);
        if !fits(hi) {
            return Err(resource("one leaf job's memory exceeds any budget"));
        }
        while lo < hi {
            let mid = lo + (hi - lo) / 2;
            if fits(mid) {
                hi = mid;
            } else {
                lo = mid + 1;
            }
        }
        lo
    };
    let minimum_bytes = minimum_usable
        .checked_add(tmpfs)
        .and_then(|n| n.checked_add(global_bytes))
        .and_then(|n| n.checked_add(STAGE_RESERVE))
        .ok_or_else(|| resource("leaf minimum overflow"))?;
    let usable = available.map(|a| {
        a.saturating_sub(tmpfs)
            .saturating_sub(global_bytes)
            .saturating_sub(STAGE_RESERVE)
    });
    if usable.is_some_and(|u| !fits(u)) {
        return Err(resource(&format!(
            "one leaf job cannot fit available memory including owned scratch: needs {minimum_bytes} bytes, has {}",
            available.unwrap_or(0)
        )));
    }
    let cache_bytes = cache_for(usable);
    let fixed = fixed_for(cache_bytes)?;
    let plan = config::plan_jobs(requested, cores as usize, usable, 0, fixed, per_job);
    let jobs = plan.jobs;
    let limited_by = plan.limited_by;
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
    let bound_bytes = global_bytes
        .checked_add(STAGE_RESERVE)
        .and_then(|n| n.checked_add(cache_bytes))
        .and_then(|n| n.checked_add(gate_bytes))
        .and_then(|n| n.checked_add(leaves))
        .ok_or_else(|| resource("leaf declared memory overflow"))?;
    Ok(StagePlan {
        bound_bytes,
        page_bytes,
        gate_bytes,
        cache_bytes,
        global_bytes,
        leaf_bytes,
        window,
        jobs,
        jobs_limited_by: limited_by,
        available_bytes: available,
        minimum_bytes,
    })
}
impl StagePlan {
    pub(crate) fn jobs(&self) -> u64 {
        self.jobs
    }
    pub(crate) fn minimum_bytes(&self) -> u64 {
        self.minimum_bytes
    }
    pub(crate) fn bound_bytes(&self) -> u64 {
        self.bound_bytes
    }
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
/// Write one prepared atlas. This is the single place the writer's
/// retryable-budget / hard-failure split enters the density retry loop: only
/// `Budget` may lower texture density, a writer `Failure` must surface as is.
fn write_prepared<A, R, L>(
    prepared: Result<(A, R), atlas::PrepareError>,
    write: impl FnOnce(&A) -> Result<L, writer::WriteError>,
) -> Result<(L, R), atlas::PrepareError> {
    prepared.and_then(|(atlas, receipt)| {
        write(&atlas)
            .map_err(|error| match error {
                writer::WriteError::Budget(e) => atlas::PrepareError::Budget(e),
                writer::WriteError::Failure(e) => atlas::PrepareError::Failure(e),
            })
            .map(|leaf| (leaf, receipt))
    })
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
    escalations: Vec<atlas::PageEscalation>,
    density_drop: u32,
    density_reasons: Vec<serde_json::Value>,
    /// Published glTF primitives (H28).
    primitives: usize,
    /// Triangles whose decoded-UV footprint shares a page texel area with another's.
    shared_triangles: u64,
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
/// Any leaf failure but cancellation names the leaf member and its rank.
fn encode_leaf(
    input: &LeafInput,
    services: &atlas::Services<'_>,
    config: &TreeConfig,
    leaves: &Path,
    index: u64,
    preencode: Option<&super::super::bake_pass::PreencodeStore>,
) -> Result<LeafEncoded, JobError> {
    let node = &input.node;
    leaf_encode(input, services, config, leaves, index, preencode).map_err(|error| {
        if error.kind() == JobErrorKind::Cancelled {
            return error;
        }
        JobError::with_cause(
            error.kind(),
            format!(
                "leaf {index:012} (first {}, depth {}): {error}",
                node.first,
                node.depth()
            ),
            error,
        )
    })
}
fn leaf_encode(
    input: &LeafInput,
    services: &atlas::Services<'_>,
    config: &TreeConfig,
    leaves: &Path,
    index: u64,
    preencode: Option<&super::super::bake_pass::PreencodeStore>,
) -> Result<LeafEncoded, JobError> {
    let node = &input.node;
    let captured = services.captured;
    // The bake witness names each face by its global sorted rank: the leaf's records are
    // read in rank order from `node.first`.
    let ranks: std::collections::HashMap<_, _> = if preencode.is_some() {
        input
            .records
            .iter()
            .zip(node.first..)
            .map(|(row, rank)| {
                let s = row.triangle.source;
                ((s.node, s.mesh, s.primitive, s.triangle), rank)
            })
            .collect()
    } else {
        Default::default()
    };
    let ((glb, appearance), density_drop, density_reasons) = encode_with_density_budget(
        config.profile,
        u32::from(input.plan.density_drop),
        |density_drop| {
            (services.check)().map_err(atlas::PrepareError::from)?;
            write_prepared(
                atlas::prepare(
                    (input.records.clone(), input.corner_depths.clone()),
                    services,
                    config.profile,
                    node.depth() as u8,
                    density_drop,
                    index,
                ),
                |atlas| {
                    let mut leaf = timing::time(Stage::GlbWrite, || {
                        writer::encode(
                            atlas,
                            &captured.lattice,
                            node.depth() as u8,
                            config.tile_bytes(),
                        )
                    })?;
                    // Only the accepted density attempt reaches here.
                    leaf.shared_triangles = timing::time(Stage::GlbWrite, || {
                        crate::mesh_large::batching::leaf_shared_triangles(atlas, &mut || {
                            (services.check)()
                        })
                    })?;
                    let positions = std::mem::take(&mut leaf.positions);
                    let normals = std::mem::take(&mut leaf.normals);
                    if let Some(store) = preencode {
                        let witness = super::witness::build(atlas, &positions, &normals, &ranks)?;
                        store.write_leaf(index, density_drop, atlas, &witness)?;
                    }
                    Ok(leaf)
                },
            )
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
        appearance: appearance.receipts,
        escalations: appearance.escalations,
        density_drop,
        density_reasons,
        primitives: glb.primitives,
        shared_triangles: glb.shared_triangles,
    })
}

/// Leaves are read in frozen order, encoded by a pool of `jobs` workers (pages
/// of one leaf also run in parallel), and written strictly in leaf order, so
/// every member, manifest row and the first reported failure are independent
/// of the worker count.
#[allow(clippy::too_many_arguments)]
pub(crate) fn produce(
    path: &Path,
    captured: &full::FullCaptured,
    config: &TreeConfig,
    stage: &StagePlan,
    profile: LeafProfile,
    coarse: Coarse<'_>,
    phases: &Phases<'_>,
    check: &(dyn Fn() -> Result<(), JobError> + Sync),
) -> Result<LeafStageOutput, JobError> {
    use super::super::{bake_pass, lod::handoff::HandoffSink};
    let (mutation, target, baker) = match coarse {
        Coarse::H17 {
            mutation,
            target,
            baker,
        } => (mutation, target, baker),
        Coarse::Hlod { .. } => (super::super::lod::Mutation::None, 0, None),
    };
    let leaf_started = std::time::Instant::now();
    let total_leaves = captured.tree.stats.leaves;
    phases.progress(Phase::LeafBuild, 0, Some(total_leaves))?;
    // With a baker: owned scratch for the accepted leaf pre-encode pages and the frozen
    // tile packets.
    let bake_dir = captured.scratch.path().join("bake");
    // One allowance for both stores: half the scratch volume's free space, measured before
    // the leaves are written (a failed probe is an error, never an allowance of zero). Both
    // stores fill during H8 and H17 and drain only as the baker consumes them.
    let (preencode, handoff) = match baker {
        Some(_) => {
            let volume = crate::spill::inspect_volume(captured.scratch.path())?;
            let budget = std::sync::Arc::new(super::super::lod::handoff::ScratchBudget::new(
                volume.available_bytes / 2,
            ));
            (
                Some(bake_pass::PreencodeStore::new(
                    bake_dir.join("preencode"),
                    budget.clone(),
                )?),
                Some(HandoffSink::new(bake_dir.join("handoff"), budget)?),
            )
        }
        None => (None, None),
    };
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
        proxies: 0,
        proxy_bytes: 0,
        escalated_pages: EscalatedPages::default(),
        lossless_fallback_pages: 0,
        escalation_reasons: EscalationReasons::default(),
        escalation_diagnostics: Vec::new(),
        escalation_diagnostics_omitted: 0,
        texel_ownership: TexelOwnership::default(),
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
        preencode: preencode.as_ref(),
    };
    let preencode_ref = preencode.as_ref();
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
                                encode_leaf(
                                    &input,
                                    services,
                                    config,
                                    leaves_dir,
                                    index,
                                    preencode_ref,
                                )
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
                    if written != total_leaves {
                        phases.progress(Phase::LeafBuild, written, Some(total_leaves))?;
                    }
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
    phases.progress(Phase::LeafBuild, total_leaves, Some(total_leaves))?;
    phases.finished(Phase::LeafBuild, leaf_started.elapsed().as_secs_f64())?;
    if let Coarse::Hlod { plan, settings } = coarse {
        // HLOD needs no decoded image or leaf worker: record the cache and page-gate
        // observations, then release the decoded images and the pool before its own
        // admission takes effect (the gate and services own no memory).
        let stats = cache.stats();
        let gate_peak = gate.peak();
        drop(cache);
        drop(pool);
        let (hlod, hlod_diagnostics) = timing::time(Stage::Lod, || {
            super::super::hlod::produce(
                super::super::hlod::HlodInput {
                    stage: path,
                    captured,
                    config,
                    plan,
                    settings,
                    phases,
                },
                check,
            )
        })?;
        report.proxies = hlod.tiles;
        report.texel_ownership.unique_proxies = hlod.tiles;
        report.proxy_bytes = hlod.bytes;
        let doc = json!({"schemaVersion":1,"kind":"full_detail_leaf_payloads","report":report,"lattice":captured.lattice,"provenanceSchema":"rusty_tiles_large_source_v1","leafManifest":"leaves.jsonl","proxyManifest":"proxies.jsonl","hlod":hlod,"losslessMipGuarantee":"PNG level0 exact; viewer-generated minification levels outside guarantee"});
        write_file(
            &path.join("manifest.json"),
            &serde_json::to_vec_pretty(&doc).map_err(|e| invariant(&e.to_string()))?,
        )?;
        let resources = json!({"hlod":{"plan":plan,"diagnostics":hlod_diagnostics},"capture":captured.report,"deliveryJobs":stage.jobs,"delivery":stage,"pageGatePeakBytes":gate_peak,"sourceCache":stats});
        write_file(
            &path.join("resources.json"),
            &serde_json::to_vec_pretty(&resources).map_err(|e| invariant(&e.to_string()))?,
        )?;
        return Ok(LeafStageOutput {
            report,
            diagnostics: LeafStageDiagnostics {
                memory_bound_bytes: stage.bound_bytes,
                jobs: stage.jobs,
                jobs_limited_by: stage.jobs_limited_by,
                available_bytes: stage.available_bytes,
                page_bytes: stage.page_bytes,
                gate_bytes: stage.gate_bytes,
                cache_bytes: stage.cache_bytes,
                global_bytes: stage.global_bytes,
                leaf_bytes: stage.leaf_bytes,
                window: stage.window,
                page_gate_peak_bytes: gate_peak,
                cache_peak_resident_bytes: stats.peak_resident_bytes,
                cache_hits: stats.hits,
                cache_misses: stats.misses,
                lod_prepass_scratch_peak_bytes: hlod_diagnostics.incidence_scratch_peak_bytes,
            },
            hlod: Some(hlod_diagnostics),
        });
    }
    let mut lod = timing::time(Stage::Lod, || {
        super::super::lod::produce(
            super::super::lod::LodInput {
                stage: path,
                captured,
                scratch: &captured.scratch,
                sort: config.sorter(stage.jobs as usize),
                jobs: stage.jobs as usize,
                mutation,
                target,
                handoff: handoff.as_ref(),
                phases,
            },
            check,
        )
    })?;
    report.proxies = lod.proxies;
    report.texel_ownership.unique_proxies = lod.proxies;
    report.proxy_bytes = lod.proxy_bytes;
    // The bake hook: capture and staging are still live here.
    if let (Some(baker), Some(handoff), Some(preencode)) = (baker, &handoff, &preencode) {
        let pass = bake_pass::BakePass::new(
            captured,
            config.profile,
            config.tile_bytes(),
            stage.jobs as usize,
            path,
            handoff,
            preencode,
            phases,
            check,
        );
        let bake_started = std::time::Instant::now();
        let baked = baker.bake(&pass);
        // The totals are logged on failure too: they show how far the bake got.
        let summary = phases.bake_summary();
        baked?;
        summary?;
        phases.finished(Phase::ProxyBake, bake_started.elapsed().as_secs_f64())?;
        // The manifest reports the published (baked) proxy bytes, and keeps the certified
        // untextured figure beside them.
        let baked = pass.finish()?;
        lod.untextured_proxy_bytes = Some(lod.proxy_bytes);
        lod.proxy_bytes = baked;
        report.proxy_bytes = baked;
    }
    drop(handoff);
    drop(preencode);
    if bake_dir.exists() {
        fs::remove_dir_all(&bake_dir)
            .map_err(|e| JobError::io("remove bake scratch", &bake_dir, e))?;
    }
    let doc = json!({"schemaVersion":1,"kind":"full_detail_leaf_payloads","report":report,"lattice":captured.lattice,"provenanceSchema":"rusty_tiles_large_source_v1","leafManifest":"leaves.jsonl","proxyManifest":"proxies.jsonl","lod":lod,"losslessMipGuarantee":"PNG level0 exact; viewer-generated minification levels outside guarantee"});
    write_file(
        &path.join("manifest.json"),
        &serde_json::to_vec_pretty(&doc).map_err(|e| invariant(&e.to_string()))?,
    )?;
    let resources = json!({"lod":{"jobs":lod.jobs,"tileRuns":lod.tile_runs,"seconds":lod.seconds,"prepassSeconds":lod.prepass_seconds,"auditSeconds":lod.audit_seconds,"phaseSeconds":{"loadChildren":lod.phase_seconds[0],"islandsSeams":lod.phase_seconds[1],"simplify":lod.phase_seconds[2],"encodeComposed":lod.phase_seconds[3],"direct":lod.phase_seconds[4],"primitivesFills":lod.phase_seconds[5],"finalEncode":lod.phase_seconds[6],"encodeUnfilled":lod.phase_seconds[7],"proxyToChildren":lod.phase_seconds[8],"childrenToProxy":lod.phase_seconds[9]}},"capture":captured.report,"deliveryJobs":stage.jobs,"delivery":stage,"pageGatePeakBytes":gate.peak(),"sourceCache":cache.stats(),"nativeWorkspace":"conservative provisional charge; measured scale acceptance recorded separately"});
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
    let stats = cache.stats();
    Ok(LeafStageOutput {
        report,
        diagnostics: LeafStageDiagnostics {
            memory_bound_bytes: stage.bound_bytes,
            jobs: stage.jobs,
            jobs_limited_by: stage.jobs_limited_by,
            available_bytes: stage.available_bytes,
            page_bytes: stage.page_bytes,
            gate_bytes: stage.gate_bytes,
            cache_bytes: stage.cache_bytes,
            global_bytes: stage.global_bytes,
            leaf_bytes: stage.leaf_bytes,
            window: stage.window,
            page_gate_peak_bytes: gate.peak(),
            cache_peak_resident_bytes: stats.peak_resident_bytes,
            cache_hits: stats.hits,
            cache_misses: stats.misses,
            lod_prepass_scratch_peak_bytes: lod.prepass_scratch_peak_bytes,
        },
        hlod: None,
    })
}

#[derive(Clone, Debug, Serialize)]
pub struct LeafStageDiagnostics {
    pub memory_bound_bytes: u64,
    pub jobs: u64,
    pub jobs_limited_by: &'static str,
    pub available_bytes: Option<u64>,
    pub page_bytes: u64,
    pub gate_bytes: u64,
    pub cache_bytes: u64,
    pub global_bytes: u64,
    pub leaf_bytes: u64,
    pub window: u64,
    pub page_gate_peak_bytes: u64,
    pub cache_peak_resident_bytes: u64,
    pub cache_hits: u64,
    pub cache_misses: u64,
    /// Measured scratch peak of H17's pre-pass, retained capture files included (0 when
    /// the tree has no internal tile and the pre-pass does not run).
    pub lod_prepass_scratch_peak_bytes: u64,
}
pub(crate) struct LeafStageOutput {
    pub report: LeafOutputReport,
    pub diagnostics: LeafStageDiagnostics,
    /// HLOD stage run facts (HLOD runs only).
    pub hlod: Option<super::super::hlod::HlodDiagnostics>,
}

/// The coarse-tile stage run inside the producer while the capture is live.
#[derive(Clone, Copy)]
pub(crate) enum Coarse<'a> {
    /// H17 certified geometry, optionally textured by the H27 bake.
    H17 {
        mutation: super::super::lod::Mutation,
        target: usize,
        baker: Option<&'a dyn super::super::bake_pass::ProxyBaker>,
    },
    /// 0.4 HLOD geometry under its admitted memory plan.
    Hlod {
        plan: &'a super::super::hlod::HlodMemoryPlan,
        settings: super::super::hlod::Settings,
    },
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
        escalations,
        density_drop,
        density_reasons,
        primitives,
        shared_triangles,
    } = encoded;
    if name != format!("{:012}.glb", report.leaves) {
        return Err(invariant("leaf member order differs from manifest order"));
    }
    timing::count(Counter::Leaves, 1);
    let receipt = json!({"schemaVersion":1,"uri":format!("leaves/{name}"),"sha256":sha256,"bytes":bytes,"first":node.first,"end":node.end,"cellDepth":node.cell_depth,"cell":node.cell,"pathLength":node.path_len,"path":node.path,"lastOctant":node.last_octant,"depth":node.depth(),"packed":node.is_packed(),"bounds":{"min":bounds.min.map(f64::from),"max":bounds.max.map(f64::from)},"geometryError":geometry_error,"densityDrop":density_drop,"densityReasons":density_reasons,"primitives":primitives,"texelOwnership":if shared_triangles == 0 {"unique"} else {"shared"},"sharedTriangles":shared_triangles,"plannedFootprint":{"densityDrop":plan.density_drop,"pages":plan.pages,"baseTexels":plan.base_texels,"chainTexels":plan.chain_texels,"geometryBytes":plan.geometry_bytes,"planningOwnedBytes":plan.owned_bytes},"appearance":appearance});
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
    let ownership = &mut report.texel_ownership;
    if shared_triangles == 0 {
        ownership.unique_leaves += 1;
    } else {
        ownership.shared_leaves += 1;
        ownership.shared_triangles += shared_triangles;
        if ownership.shared.len() < TEXEL_OWNERSHIP_DIAGNOSTICS {
            ownership.shared.push(SharedTexels {
                leaf: format!("leaves/{name}"),
                first: node.first,
                depth: node.depth(),
                texel_ownership: "shared",
                triangles: shared_triangles,
            });
        } else {
            ownership.shared_omitted += 1;
        }
    }
    for page in escalations {
        use super::delivery::{MissReason, Step};
        match page.escalation.step {
            Step::UastcLevel2 => report.escalated_pages.uastc_level2 += 1,
            Step::UastcLevel3 => report.escalated_pages.uastc_level3 += 1,
            Step::LosslessPng => report.lossless_fallback_pages += 1,
            Step::Profile => return Err(invariant("unescalated page reported as escalated")),
        }
        match page.escalation.misses.first().map(|m| m.reason) {
            Some(MissReason::Gate) => report.escalation_reasons.gate += 1,
            Some(MissReason::CodecVerify) => report.escalation_reasons.codec_verify += 1,
            None => return Err(invariant("escalated page without a miss")),
        }
        if report.escalation_diagnostics.len() >= ESCALATION_DIAGNOSTICS {
            report.escalation_diagnostics_omitted += 1;
            continue;
        }
        report.escalation_diagnostics.push(EscalationDiagnostic {
            leaf: format!("leaves/{name}"),
            first: node.first,
            depth: node.depth(),
            page: page.page as u64,
            slot: atlas::slot_property(page.slot),
            sources: page
                .sources
                .iter()
                .map(|&(image, material)| EscalationSource {
                    image: image as u64,
                    material,
                })
                .collect(),
            step: page.escalation.step.name(),
            misses: page
                .escalation
                .misses
                .iter()
                .map(|m| EscalationMiss {
                    step: m.step.name(),
                    reason: m.reason.name(),
                    mip: m.mip.map(|mip| mip as u64),
                    gate: m.gate.clone(),
                })
                .collect(),
        });
    }
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
    config.lod = request.lod;
    #[cfg(test)]
    if let Some(leaf_triangles) = request.leaf_triangles {
        config = config.with_leaf_triangles(leaf_triangles);
    }
    match request.lod {
        super::super::hlod::TilerLod::Hlod => {
            config.memory_budget = Some(
                crate::mesh_large::resources::MemoryBudget::resolve(request.memory_budget)
                    .map_err(|e| attempt.fail(e))?,
            );
        }
        super::super::hlod::TilerLod::H17 if request.memory_budget.is_some() => {
            return Err(attempt.fail(JobError::new(
                JobErrorKind::InvalidRequest,
                "a memory budget requires the HLOD coarse-tile method",
            )));
        }
        super::super::hlod::TilerLod::H17 => {}
    }
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
    let stage_plan = match stage_plan(&config, &captured) {
        Ok(plan) => plan,
        Err(error) => {
            let mut failed = attempt.fail(error);
            close_failed_capture(captured, &mut failed);
            return Err(failed);
        }
    };
    let hlod_plan = match &config.memory_budget {
        Some(budget) if config.lod == super::super::hlod::TilerLod::Hlod => {
            match super::super::hlod::plan(&captured, &config, budget, request.hlod.cap) {
                Ok(plan) => Some(plan),
                Err(error) => {
                    let mut failed = attempt.fail(error);
                    close_failed_capture(captured, &mut failed);
                    return Err(failed);
                }
            }
        }
        _ => None,
    };
    let coarse = match &hlod_plan {
        Some(plan) => Coarse::Hlod {
            plan,
            settings: request.hlod,
        },
        None => Coarse::H17 {
            mutation: request.lod_mutation,
            target: request.lod_target,
            baker: request.baker.as_deref(),
        },
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
        coarse,
        &Phases::silent(),
        &|| attempt.check(),
    );
    let cleanup_path = captured.scratch.path().to_owned();
    let cleanup = captured.close();
    let report = match result {
        Ok(output) => output.report,
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
                    Some(error) => write_prepared(Ok(((), ())), |_| Err::<(), _>(error)),
                    None => Ok(((), ())),
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
        textured_fixture(parent, count, min_filter, Texture::Smooth, 1024)
    }
    #[derive(Clone, Copy)]
    enum Texture {
        Smooth,
        /// Every n-th 4x4 block holds four scattered random colours.
        Hard(u64),
    }
    /// [`scattered_fixture`]; `Texture::Hard(n)` replaces every n-th 4x4 texture
    /// block with four scattered random colours (1: every block), so pages
    /// miss the colour gate at the profile's encoder setting.
    fn textured_fixture(
        parent: &Path,
        count: usize,
        min_filter: u32,
        pattern: Texture,
        edge: u32,
    ) -> PathBuf {
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
        let hash = |x: u32, y: u32, s: u32| {
            let mut h = u64::from(x).wrapping_mul(0x9E37_79B9)
                ^ u64::from(y).wrapping_mul(0x85EB_CA6B)
                ^ u64::from(s).wrapping_mul(0x27D4_EB2F);
            h ^= h >> 13;
            h = h.wrapping_mul(0xC2B2_AE35);
            h ^ (h >> 16)
        };
        let texture = image::RgbaImage::from_fn(edge, edge, |x, y| {
            if let Texture::Hard(every) = pattern {
                if hash(x / 4, y / 4, 77).is_multiple_of(every) {
                    let c = hash(x / 4, y / 4, (hash(x, y, 9) % 4) as u32);
                    return image::Rgba([c as u8, (c >> 8) as u8, (c >> 16) as u8, 255]);
                }
            }
            // Every channel stays below 256 for x, y < 1024 (`edge` <= 1024).
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
        // H17 coarse tiles are part of the deterministic output too.
        let proxies = fs::read_to_string(output.join("proxies.jsonl")).unwrap();
        files.push(proxies.as_bytes().to_vec());
        for row in proxies.lines() {
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

    /// Pages that miss the colour gate at the profile setting escalate; the
    /// report counts them by step, agrees with the leaf manifest and lists
    /// them with leaf, slot, sources and the missed level, and the published
    /// bytes and report are the same for any worker count.
    #[test]
    fn escalated_pages_are_reported_and_identical_across_jobs() {
        let work = tempfile::tempdir().unwrap();
        let input = textured_fixture(
            &work.path().join("source"),
            9_000,
            9729,
            Texture::Hard(4),
            256,
        );
        let mut previous = None;
        for jobs in [1, 3] {
            let output = work.path().join(format!("balanced{jobs}"));
            let result = mesh_to_leaf_payloads(
                LeafOutputRequest::new(&input, &output, LeafProfile::Balanced)
                    .with_jobs(NonZeroUsize::new(jobs).unwrap()),
                &RunControl::default(),
            )
            .unwrap();
            let report = &result.report;
            let total = report.escalated_pages.uastc_level2
                + report.escalated_pages.uastc_level3
                + report.lossless_fallback_pages;
            assert!(total > 0, "{report:?}");
            let mut manifest = BTreeMap::<String, u64>::new();
            let mut pages = 0;
            for row in fs::read_to_string(output.join("leaves.jsonl"))
                .unwrap()
                .lines()
            {
                let row: serde_json::Value = serde_json::from_str(row).unwrap();
                for page in row["appearance"].as_array().unwrap() {
                    if page.get("levels").is_none() {
                        continue;
                    }
                    pages += 1;
                    if let Some(step) = page["escalation"]["step"].as_str() {
                        *manifest.entry(step.to_owned()).or_default() += 1;
                        assert_eq!(page["escalation"]["misses"][0]["step"], "profile");
                        assert_eq!(
                            page["mime"],
                            if step == "lossless_png" {
                                "image/png"
                            } else {
                                "image/ktx2"
                            }
                        );
                    }
                }
            }
            println!(
                "escalation jobs{jobs} pages{pages} level2 {} level3 {} png {}",
                report.escalated_pages.uastc_level2,
                report.escalated_pages.uastc_level3,
                report.lossless_fallback_pages
            );
            let count = |step: &str| manifest.get(step).copied().unwrap_or(0);
            assert_eq!(count("uastc_level2"), report.escalated_pages.uastc_level2);
            assert_eq!(count("uastc_level3"), report.escalated_pages.uastc_level3);
            assert_eq!(count("lossless_png"), report.lossless_fallback_pages);
            assert_eq!(
                report.escalation_diagnostics.len() as u64 + report.escalation_diagnostics_omitted,
                total
            );
            assert!(report.escalation_diagnostics.len() <= ESCALATION_DIAGNOSTICS);
            for diagnostic in &report.escalation_diagnostics {
                assert!(diagnostic.leaf.starts_with("leaves/"));
                assert_eq!(diagnostic.slot, "baseColorTexture");
                assert_eq!(
                    diagnostic.sources,
                    [EscalationSource {
                        image: 0,
                        material: 0
                    }]
                );
                assert_eq!(diagnostic.misses[0].step, "profile");
                assert!(diagnostic.misses.iter().all(|m| m.mip.is_some()));
            }
            let files = output_files(&output);
            let report = serde_json::to_value(report).unwrap();
            if let Some((prior_files, prior_report)) = &previous {
                assert_eq!(prior_files, &files, "jobs {jobs} changed published bytes");
                assert_eq!(prior_report, &report, "jobs {jobs} changed the report");
            } else {
                previous = Some((files, report));
            }
        }
    }
    /// A mip-sampled page has no lossless step; when every lossy step misses,
    /// the refusal names the leaf, page, slot, source image and material, and
    /// each step's missed level.
    #[test]
    fn unrecoverable_page_failure_names_leaf_page_slot_sources_and_level() {
        let work = tempfile::tempdir().unwrap();
        let input = textured_fixture(
            &work.path().join("source"),
            640,
            9987,
            Texture::Hard(1),
            256,
        );
        let failure = mesh_to_leaf_payloads(
            LeafOutputRequest::new(&input, work.path().join("out"), LeafProfile::Balanced)
                .with_jobs(NonZeroUsize::new(2).unwrap()),
            &RunControl::default(),
        )
        .unwrap_err();
        let message = failure.error.to_string();
        assert_eq!(failure.error.kind(), JobErrorKind::Unsupported, "{message}");
        for text in [
            "leaf 000000000000 (first 0, depth ",
            "page 0 baseColorTexture (source image 0 material 0)",
            "profile mip",
            "uastc_level3 mip",
            "no lossless PNG step",
        ] {
            assert!(message.contains(text), "{text}: {message}");
        }
    }

    /// Several dedicated mip-sampled pages that miss the codec's verification floor at
    /// the profile setting: the escalation reasons, diagnostics and published bytes
    /// are the same for one worker and for many (#234).
    #[test]
    fn codec_verification_escalation_reasons_are_identical_across_jobs() {
        const QUADS: usize = 8;
        let work = tempfile::tempdir().unwrap();
        let source = work.path().join("source");
        fs::create_dir_all(&source).unwrap();
        let mut bytes = Vec::<u8>::new();
        let (mut views, mut accessors, mut primitives, mut materials) =
            (Vec::new(), Vec::new(), Vec::new(), Vec::new());
        let (mut images, mut textures) = (Vec::new(), Vec::new());
        for quad in 0..QUADS {
            let x0 = 5.0 * quad as f32;
            let positions: [f32; 18] = [
                x0,
                0.,
                0.,
                x0 + 4.,
                0.,
                0.,
                x0,
                4.,
                0.,
                x0 + 4.,
                0.,
                0.,
                x0 + 4.,
                4.,
                0.,
                x0,
                4.,
                0.,
            ];
            let uvs: [f32; 12] = [0., 0., 1., 0., 0., 1., 1., 0., 1., 1., 0., 1.];
            for (data, ty, count, min_max) in [
                (
                    &positions[..],
                    "VEC3",
                    6,
                    Some(([x0, 0., 0.], [x0 + 4., 4., 0.])),
                ),
                (&uvs[..], "VEC2", 6, None),
            ] {
                views
                    .push(json!({"buffer":0,"byteOffset":bytes.len(),"byteLength":data.len() * 4}));
                bytes.extend(data.iter().flat_map(|v| v.to_le_bytes()));
                let mut accessor = json!({"bufferView":views.len() - 1,"componentType":5126,
                    "count":count,"type":ty});
                if let Some((min, max)) = min_max {
                    accessor["min"] = json!(min);
                    accessor["max"] = json!(max);
                }
                accessors.push(accessor);
            }
            image::RgbaImage::from_fn(64, 64, |x, y| {
                let v = |k: u32| ((x * 7 + y * 13 + k * 31 + quad as u32 * 17) % 97 + 80) as u8;
                image::Rgba([v(0), v(1), v(2), 255])
            })
            .save(source.join(format!("sharp{quad}.png")))
            .unwrap();
            images.push(json!({"uri":format!("sharp{quad}.png")}));
            textures.push(json!({"source":quad,"sampler":0}));
            materials.push(json!({"pbrMetallicRoughness":{"baseColorTexture":{"index":quad}}}));
            primitives.push(
                json!({"attributes":{"POSITION":2 * quad,"TEXCOORD_0":2 * quad + 1},
                "material":quad}),
            );
        }
        fs::write(source.join("quads.bin"), &bytes).unwrap();
        let doc = json!({"asset":{"version":"2.0"},"scene":0,"scenes":[{"nodes":[0]}],
            "nodes":[{"mesh":0}],"meshes":[{"primitives":primitives}],
            "buffers":[{"uri":"quads.bin","byteLength":bytes.len()}],
            "bufferViews":views,"accessors":accessors,"images":images,"textures":textures,
            "samplers":[{"minFilter":9987,"magFilter":9729}],"materials":materials});
        let input = source.join("quads.gltf");
        fs::write(&input, serde_json::to_vec(&doc).unwrap()).unwrap();
        let mut previous = None;
        for jobs in [1, 4] {
            let output = work.path().join(format!("out{jobs}"));
            let result = mesh_to_leaf_payloads(
                LeafOutputRequest::new(&input, &output, LeafProfile::Balanced)
                    .with_jobs(NonZeroUsize::new(jobs).unwrap()),
                &RunControl::default(),
            )
            .unwrap();
            let report = &result.report;
            let total = report.escalated_pages.uastc_level2
                + report.escalated_pages.uastc_level3
                + report.lossless_fallback_pages;
            println!("jobs {jobs}: {:?} total {total}", report.escalation_reasons);
            assert!(report.escalation_reasons.codec_verify > 0, "{report:?}");
            assert_eq!(
                report.escalation_reasons.gate + report.escalation_reasons.codec_verify,
                total
            );
            let by_reason = |reason: &str| {
                report
                    .escalation_diagnostics
                    .iter()
                    .filter(|d| d.misses[0].reason == reason)
                    .count() as u64
            };
            assert_eq!(
                report.escalation_diagnostics.len() as u64 + report.escalation_diagnostics_omitted,
                total
            );
            assert_eq!(
                by_reason("codec_verify"),
                report.escalation_reasons.codec_verify
            );
            assert_eq!(by_reason("gate"), report.escalation_reasons.gate);
            let files = output_files(&output);
            let reasons = serde_json::to_value(report.escalation_reasons).unwrap();
            let report = serde_json::to_value(report).unwrap();
            if let Some((prior_files, prior_reasons, prior_report)) = &previous {
                assert_eq!(
                    prior_reasons, &reasons,
                    "jobs {jobs} changed escalation_reasons"
                );
                assert_eq!(prior_files, &files, "jobs {jobs} changed published bytes");
                assert_eq!(prior_report, &report, "jobs {jobs} changed the report");
            } else {
                previous = Some((files, reasons, report));
            }
        }
    }

    /// Pages the codec's own verification refuses at the profile setting
    /// escalate and are counted apart from gate misses.
    #[test]
    fn codec_verification_misses_are_reported_by_reason() {
        let work = tempfile::tempdir().unwrap();
        // One quad covering the whole 64x64 sharp texture, mip-sampled: a dedicated
        // page whose authored levels are the texture's own class chain.
        let source = work.path().join("source");
        fs::create_dir_all(&source).unwrap();
        let positions: [f32; 18] = [
            0., 0., 0., 4., 0., 0., 0., 4., 0., 4., 0., 0., 4., 4., 0., 0., 4., 0.,
        ];
        let uvs: [f32; 12] = [0., 0., 1., 0., 0., 1., 1., 0., 1., 1., 0., 1.];
        let bytes: Vec<u8> = positions
            .iter()
            .chain(&uvs)
            .flat_map(|v| v.to_le_bytes())
            .collect();
        fs::write(source.join("quad.bin"), &bytes).unwrap();
        image::RgbaImage::from_fn(64, 64, |x, y| {
            let v = |k: u32| ((x * 7 + y * 13 + k * 31) % 97 + 80) as u8;
            image::Rgba([v(0), v(1), v(2), 255])
        })
        .save(source.join("sharp.png"))
        .unwrap();
        let doc = json!({"asset":{"version":"2.0"},"scene":0,"scenes":[{"nodes":[0]}],"nodes":[{"mesh":0}],
            "meshes":[{"primitives":[{"attributes":{"POSITION":0,"TEXCOORD_0":1},"material":0}]}],
            "buffers":[{"uri":"quad.bin","byteLength":bytes.len()}],
            "bufferViews":[{"buffer":0,"byteOffset":0,"byteLength":72},{"buffer":0,"byteOffset":72,"byteLength":48}],
            "accessors":[{"bufferView":0,"componentType":5126,"count":6,"type":"VEC3","min":[0,0,0],"max":[4,4,0]},
                         {"bufferView":1,"componentType":5126,"count":6,"type":"VEC2"}],
            "images":[{"uri":"sharp.png"}],"textures":[{"source":0,"sampler":0}],
            "samplers":[{"minFilter":9987,"magFilter":9729}],
            "materials":[{"pbrMetallicRoughness":{"baseColorTexture":{"index":0}}}]});
        let input = source.join("quad.gltf");
        fs::write(&input, serde_json::to_vec(&doc).unwrap()).unwrap();
        let output = work.path().join("out");
        let result = mesh_to_leaf_payloads(
            LeafOutputRequest::new(&input, &output, LeafProfile::Balanced)
                .with_jobs(NonZeroUsize::new(2).unwrap()),
            &RunControl::default(),
        )
        .unwrap();
        let report = &result.report;
        let total = report.escalated_pages.uastc_level2
            + report.escalated_pages.uastc_level3
            + report.lossless_fallback_pages;
        println!(
            "codec-verify escalation {:?} {:?} png {}",
            report.escalation_reasons, report.escalated_pages, report.lossless_fallback_pages
        );
        assert!(report.escalation_reasons.codec_verify > 0, "{report:?}");
        assert_eq!(
            report.escalation_reasons.gate + report.escalation_reasons.codec_verify,
            total
        );
        let codec = report
            .escalation_diagnostics
            .iter()
            .filter(|d| d.misses[0].reason == "codec_verify")
            .count() as u64;
        assert_eq!(codec, report.escalation_reasons.codec_verify);
        assert!(report
            .escalation_diagnostics
            .iter()
            .all(|d| d.misses[0].step == "profile"));
    }
}
