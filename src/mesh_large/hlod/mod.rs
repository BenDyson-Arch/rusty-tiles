//! 0.4 HLOD (decision record `docs/architecture/hlod-040-decision.md`, slice S1): classic
//! octree HLOD geometry over the frozen ownership tree.
//!
//! Leaves are H8's, unchanged and byte-identical. Every occupied cell above them gets a
//! parent built from its immediate children by position-welded meshopt simplification to
//! at most 8,192 triangles, locking only its outer interface with other ownership subtrees
//! as complete segments at their leaf-canonical decoded positions (D1). By induction every
//! ancestor keeps the outer segments of its descendants, so disjoint independently refined
//! tiles agree at every depth mix; the bytes are decoded and checked before publication.
//!
//! The stage streams: one spillable incidence pass, then a preorder walk that holds only
//! the tiles waiting for their parent, admitted against the run's declared memory budget
//! (D8) and released as each parent completes. Parents are built by up to `jobs` workers;
//! each is a pure function of its children, so output is identical for any job count or
//! budget. It writes `proxies/` and `proxies.jsonl` (postorder) for H16 delivery, exactly
//! where H17 did. Coarse appearance (S3, [`appearance`]) is fresh content-sized charts
//! baked from the captured source; the error is measured against the source in both
//! directions by S2.
pub(crate) mod appearance;
pub(crate) mod bake;
mod charts;
mod encode;
pub(crate) mod error;
mod geometry;
pub(crate) mod interfaces;
pub(crate) mod source_index;
pub(crate) mod store;
#[cfg(test)]
mod appearance_tests;
#[cfg(test)]
mod tests;

pub(crate) use encode::Mutation;

use self::{
    interfaces::{IncidenceStats, InterfaceRow},
    store::{CoarseTile, HlodRow, SourcePrimitive, StoppingBound, TileGeometry, TileKey},
};
use super::{
    config::{self, TreeConfig},
    full::FullCaptured,
    phases::{Phase, Phases},
    records::{NodeKind, NodeRecord},
    resources::{self, Ledger, MemoryBudget},
};
use crate::{JobError, JobErrorKind};
use serde::Serialize;
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, HashMap},
    fs,
    io::{BufRead, BufWriter, Write},
    path::Path,
    str::FromStr,
    sync::mpsc,
};

/// Coarse-tile method. H17 stays the default until the HLOD slices replace it (S6/S8).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum TilerLod {
    /// H17 certified geometry with the H27 proxy bake (current default).
    #[default]
    H17,
    /// 0.4 classic octree HLOD (untextured coarse tiles, measured source error).
    Hlod,
}

impl FromStr for TilerLod {
    type Err = JobError;
    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "h17" => Ok(Self::H17),
            "hlod" => Ok(Self::Hlod),
            _ => Err(JobError::new(
                JobErrorKind::InvalidRequest,
                "lod must be h17 or hlod",
            )),
        }
    }
}

/// S3's coarse appearance policy (recorded in the stage manifest).
pub(crate) const APPEARANCE_POLICY: &str = "hlod-s3-fresh-charts: content-sized pages at max(G/16, 1e-5 m), \
     1024 cap, never finer than the source's texel spacing, nearest captured-source bake; per-vertex COLOR_0 or flat factors untextured";

/// Hard published triangle cap of every coarse tile (D1).
pub(crate) const TRIANGLE_CAP: usize = 8192;
/// Runtime, allocator slack and output-writer reserve of the stage.
const STAGE_RESERVE: u64 = 256 << 20;
/// Charged workspace per merged input triangle of one parent job (`m` merged triangles, at
/// most `3m` vertices and indices; children, held under their own leases, and the output,
/// charged separately, excluded). Bytes per merged triangle of what is live at once at the
/// peak, inside a protected retry's simplification (`geometry::build_parent`); a hash table
/// of `k` entries of `T` bytes holds at most `16k/7` buckets of `T + 1` bytes:
/// - merged vertices (key 12, depth 1, position 12; grown by doubling, at most twice) and
///   triangles (12), representatives (24): `2 * 3 * 25 + 36 = 186`;
/// - the parent's interface rows, at most six (leaf, item) rows of 36 bytes: 216;
/// - required segment and point sets (at most `3m` each): `171.4 + 89.1`;
/// - the lock and protection flags: 4;
/// - the retry's keep and rest lists (doubling: 24), kept vertices (`3m` u32: 34.3), the lost
///   segment and point sets (at most all required: 260.5), the previous and reduced output
///   (24);
/// - `simplify`'s own arrays: local index map (`3m` u32 pairs, growing from `2m`: 92.6),
///   vertex list (24), indices (12), positions (36), locks (3), destination (12);
/// - meshoptimizer's temporary arrays, all freed when the call returns: per vertex about 100
///   bytes (adjacency offsets, remap and its hash table, wedge, kind, loops, positions,
///   quadric (44), collapse remap and lock), per index 24 (adjacency edge, collapse, order),
///   so 372.
///
/// The sum is under 1,560; earlier phases (the weld index while merging, the interface rows
/// while merging them, the representative tables) peak lower. Measured on adversarial jobs by
/// `parent_jobs_allocate_within_their_charge` (a counting allocator, meshoptimizer routed
/// through it).
const JOB_BYTES_PER_TRIANGLE: u64 = 1600;
/// Finished coarse rows held while an earlier tile is still being built (`walk`); each is a
/// few hundred bytes, charged within `STAGE_RESERVE`.
const ROW_WINDOW: usize = 1024;
/// Encoded GLB, compression scratch and decoded check copy of one coarse tile.
const JOB_ENCODE_BYTES: u64 = 16 << 20;

fn state(message: impl Into<String>) -> JobError {
    JobError::new(JobErrorKind::InvalidState, message.into())
}
fn resource(message: impl Into<String>) -> JobError {
    JobError::new(JobErrorKind::ResourceLimit, message.into())
}

/// Stage settings; production uses [`Settings::default`].
#[derive(Clone, Copy, Debug)]
pub(crate) struct Settings {
    pub cap: usize,
    pub mutation: Mutation,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            cap: TRIANGLE_CAP,
            mutation: Mutation::None,
        }
    }
}

/// D8 admission of the HLOD stage, resolved after capture and before any leaf is built.
#[derive(Clone, Debug, Serialize)]
pub struct HlodMemoryPlan {
    pub budget_bytes: u64,
    /// The budget, capped by detected headroom, less memory-backed scratch.
    pub ceiling_bytes: u64,
    /// Captured metadata, decoded JSON, runtime reserve and stream buffers.
    pub fixed_bytes: u64,
    /// The incidence pass's two live sorters (before the walk).
    pub incidence_bytes: u64,
    /// The sequential walk's worst child window: 8 held tiles per open level.
    pub window_bytes: u64,
    /// One parent job: its workspace plus its output.
    pub job_bytes: u64,
    /// The least ceiling admitting the stage; refused below it.
    pub minimum_bytes: u64,
    /// The walk's lease ledger (`ceiling - fixed`).
    pub ledger_bytes: u64,
    /// Coarse texture/chart/mip/codec workspace of one parent job (zero without images).
    pub texture_workspace_bytes: u64,
    /// The vertex-colour path's query state of one job (zero without COLOR_0).
    pub color_workspace_bytes: u64,
    /// Shared decoded source-chain cache (zero without images).
    pub image_cache_bytes: u64,
    /// Encoder workspace ceiling of one coarse page.
    pub page_bytes: u64,
    /// Encoded page bytes one coarse tile may publish (admitted twice: pages and GLB).
    pub tile_page_bytes: u64,
    pub jobs: u64,
    pub jobs_limited_by: &'static str,
}

/// The appearance part of one parent job and the image cache bounds.
struct AppearancePlan {
    texture: u64,
    color: u64,
    page: u64,
    tile_pages: u64,
    cache_floor: u64,
    cache_total: u64,
}

/// Charged bytes per page texel of the bake: its covered-texel record and raster owner
/// flag. Points, hits, frames and sample requests are per chunk ([`CHUNK_BYTES`]).
const QUERY_BYTES_PER_TEXEL: u64 = 24;
/// One bake chunk's points, hits, source frames, requests and levels of detail.
const CHUNK_BYTES: u64 = bake::CHUNK as u64 * 160;

fn appearance_plan(
    captured: &FullCaptured,
    profile: config::Profile,
    cap: usize,
) -> Result<AppearancePlan, JobError> {
    let layout = captured.triangles.layout().0;
    let colors = layout & super::attributes::COLOR != 0;
    // Vertex-colour queries: at most three per coarse triangle (the source query's own
    // bounded cache is charged with S2's measurement).
    let color = if colors {
        (cap as u64).saturating_mul(3 * QUERY_BYTES_PER_TEXEL)
    } else {
        0
    };
    if captured.image_info.is_empty() {
        return Ok(AppearancePlan {
            texture: 0,
            color,
            page: 0,
            tile_pages: 0,
            cache_floor: 0,
            cache_total: 0,
        });
    }
    let edge = appearance::MAX_PAGE_EDGE;
    let texels = u64::from(edge) * u64::from(edge);
    let slots = captured
        .materials
        .iter()
        .map(|m| m.bindings().count() as u64)
        .max()
        .unwrap_or(0)
        .max(1);
    let page = appearance::page_charge([edge, edge], slots, profile)?;
    let largest = captured
        .image_info
        .iter()
        .map(|i| (u64::from(i.width), u64::from(i.height)))
        .max_by_key(|(w, h)| w * h)
        .unwrap_or((0, 0));
    let largest_chain = bake::chain_bytes(largest.0 as u32, largest.1 as u32);
    // One image decode transient (encoded bytes, RGBA, chain build copy) and the chain a
    // worker holds while sampling, besides the shared cache.
    let decode = (largest.0 * largest.1)
        .saturating_mul(16)
        .saturating_add(32 << 20)
        .saturating_add(largest_chain);
    // One group's page workspace at a time, plus the tile's delivered pages and the GLB
    // being assembled from them (each at most `tile_pages`).
    let tile_pages = appearance::tile_page_bytes(slots, captured.materials.len() as u64, cap as u64);
    let texture = texels
        .saturating_mul(QUERY_BYTES_PER_TEXEL)
        .saturating_add(CHUNK_BYTES)
        .saturating_add(tile_pages.saturating_mul(2))
        .saturating_add(page)
        .saturating_add(decode);
    let cache_total = captured.image_info.iter().fold(0u64, |n, i| {
        n.saturating_add(bake::chain_bytes(i.width, i.height))
    });
    Ok(AppearancePlan {
        texture,
        color,
        page,
        tile_pages,
        cache_floor: largest_chain.min(cache_total),
        cache_total,
    })
}

/// Plan the stage for `captured` under `budget`; refuses when one job cannot fit.
pub(crate) fn plan(
    captured: &FullCaptured,
    config: &TreeConfig,
    budget: &MemoryBudget,
    cap: usize,
) -> Result<HlodMemoryPlan, JobError> {
    let leaf_cap = config.leaf_triangles();
    let held = TileGeometry::charge(leaf_cap.max(cap as u64));
    let height = u64::from(captured.tree.stats.height);
    // Held children of the open path, plus the read transient of the one leaf being read
    // (its lease shrinks to the held footprint once read).
    let read = leaf_cap
        .saturating_mul(store::LEAF_READ_BYTES_PER_TRIANGLE)
        .saturating_add(store::LEAF_READ_FIXED_BYTES);
    let window = held
        .saturating_mul(8)
        .saturating_mul(height + 1)
        .saturating_add(read.saturating_sub(held));
    let merged = leaf_cap.max(cap as u64).saturating_mul(8);
    let geometry_job = merged
        .saturating_mul(JOB_BYTES_PER_TRIANGLE)
        .saturating_add(JOB_ENCODE_BYTES)
        .saturating_add(source_index::QUERY_BYTES + error::WORK_BYTES)
        .saturating_add(bake::RESOLVE_BYTES)
        .saturating_add(bake::LOCAL_BYTES)
        .saturating_add(TileGeometry::charge(cap as u64));
    let appearance = appearance_plan(captured, config.profile, cap)?;
    let job = geometry_job
        .saturating_add(appearance.texture)
        .saturating_add(appearance.color);
    let sorter = config.sorter(1);
    let incidence = interfaces::memory_need(sorter, leaf_cap, captured.tree.stats.leaves);
    let buffer = config.sort.buffer_bytes as u64;
    // Five sequence readers plus the interface stream's merge buffers.
    let streams = buffer
        .saturating_mul(5)
        .saturating_add((config.sort.fan_in as u64 + 1).saturating_mul(buffer));
    let fixed = captured
        .report
        .memory
        .owned_metadata_bytes
        .saturating_add(captured.report.memory.geometry.document_bytes)
        .saturating_add(STAGE_RESERVE)
        .saturating_add(source_index::BUILD_BYTES)
        .saturating_add(streams);
    let minimum = fixed.saturating_add(
        incidence.max(window.saturating_add(job).saturating_add(appearance.cache_floor)),
    );
    let tmpfs = if captured.scratch.memory_backed() {
        captured.report.scratch.need_bytes
    } else {
        0
    };
    let ceiling = budget
        .ceiling(config::available_memory())
        .saturating_sub(tmpfs);
    if ceiling < minimum {
        return Err(resource(format!(
            "HLOD stage needs at least {minimum} bytes (fixed {fixed}, incidence {incidence}, \
             child window {window}, one parent job {job}) but the memory budget allows \
             {ceiling} (budget {}, memory-backed scratch {tmpfs})",
            budget.resolved_bytes
        )));
    }
    let ledger = ceiling - fixed;
    let cores = std::thread::available_parallelism().map_or(1, |n| n.get()) as u64;
    let requested = match config.jobs {
        config::Jobs::Auto => cores,
        config::Jobs::Fixed(n) => n.get() as u64,
    };
    let fit = ((ledger - window - appearance.cache_floor) / job.max(1)).max(1);
    let (jobs, limited_by) = if fit < requested.min(cores) {
        (fit, "memory")
    } else if requested < cores {
        (requested, "request")
    } else {
        (cores, "cores")
    };
    // The cache takes what the admitted jobs leave, up to every chain, at least its floor.
    let image_cache = if appearance.cache_floor == 0 {
        0
    } else {
        appearance.cache_total.min(
            (ledger - window)
                .saturating_sub(jobs.saturating_mul(job))
                .max(appearance.cache_floor),
        )
    };
    Ok(HlodMemoryPlan {
        budget_bytes: budget.resolved_bytes,
        ceiling_bytes: ceiling,
        fixed_bytes: fixed,
        incidence_bytes: incidence,
        window_bytes: window,
        job_bytes: job,
        minimum_bytes: minimum.saturating_add(tmpfs),
        ledger_bytes: ledger,
        texture_workspace_bytes: appearance.texture,
        color_workspace_bytes: appearance.color,
        image_cache_bytes: image_cache,
        page_bytes: appearance.page,
        tile_page_bytes: appearance.tile_pages,
        jobs,
        jobs_limited_by: limited_by,
    })
}

/// What the stage records in the leaf manifest (deterministic for a given input).
#[derive(Clone, Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct HlodReport {
    pub schema_version: u32,
    pub method: &'static str,
    pub error_policy: &'static str,
    pub appearance: &'static str,
    pub triangle_cap: u64,
    pub tiles: u64,
    pub triangles: u64,
    pub max_triangles: u64,
    pub input_triangles: u64,
    pub stopped_by_input: u64,
    pub stopped_by_triangle_budget: u64,
    pub tiles_with_protected_faces: u64,
    pub protected_faces: u64,
    pub retries: u64,
    pub interface_segments: u64,
    pub interface_vertices: u64,
    pub quantized_tiles: u64,
    pub float_tiles: u64,
    pub bytes: u64,
    /// Internal cells with a single child that fits the cap: the parent would be that
    /// child again, so the cell publishes no content and the child stands for it.
    pub pass_through: u64,
    pub max_error_by_depth: Vec<f64>,
    pub incidence: IncidenceStats,
    pub error_measurement: error::ErrorReport,
    #[serde(skip)]
    pub error_cpu_seconds: f64,
    #[serde(skip)]
    pub error_cpu: error::CpuReport,
    /// Coarse appearance per tile depth (S3).
    pub appearance_by_depth: Vec<DepthAppearance>,
}

/// Coarse appearance of one tile depth: page texels, achieved against requested spacing
/// (`max(G/16, 1e-5 m)`) and cap hits.
#[derive(Clone, Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DepthAppearance {
    pub depth: u32,
    pub tiles: u64,
    pub textured_tiles: u64,
    pub vertex_color_tiles: u64,
    pub pages: u64,
    pub layout_texels: u64,
    pub page_texels: u64,
    pub covered_texels: u64,
    /// Exact nearest searches of the bake (the rest kept a recent donor).
    pub searches: u64,
    pub donor_reads: u64,
    pub capped_tiles: u64,
    /// Tiles whose page density the source's own texel spacing set (not `G/16`).
    pub source_limited_tiles: u64,
    pub escalated_pages: u64,
    /// Pages delivered as exact level-0 PNG without mips.
    pub unmipped_pages: u64,
    pub requested_spacing_min: f64,
    pub requested_spacing_max: f64,
    pub achieved_spacing_min: f64,
    pub achieved_spacing_max: f64,
    /// Mean of achieved / requested spacing over textured tiles (1 when uncapped).
    pub mean_spacing_ratio: f64,
}

/// Run facts of the stage outside the deterministic manifest.
#[derive(Clone, Debug, Default, Serialize)]
pub struct HlodDiagnostics {
    pub tiles: u64,
    pub jobs: u64,
    /// Whole-process user+system CPU seconds of the stage (incidence, walk, encoding).
    pub cpu_seconds: Option<f64>,
    pub incidence_cpu_seconds: Option<f64>,
    /// Source-index construction plus summed worker-thread measurement/identity CPU.
    pub error_cpu_seconds: Option<f64>,
    pub error_cpu: error::CpuReport,
    pub error_measurement: error::ErrorReport,
    pub wall_seconds: f64,
    pub incidence_scratch_peak_bytes: u64,
    /// Highest simultaneously leased bytes of the walk's ledger.
    pub lease_peak_bytes: u64,
    pub ledger_bytes: u64,
    /// Summed worker seconds of coarse charts, source queries, sampling and page encoding.
    pub appearance_seconds: appearance::Timing,
    pub image_cache: bake::CacheStats,
    /// The manifest's per-depth appearance summary, repeated for the run report.
    pub appearance_by_depth: Vec<DepthAppearance>,
}

pub(crate) struct HlodInput<'a> {
    pub stage: &'a Path,
    pub captured: &'a FullCaptured,
    pub config: &'a TreeConfig,
    pub plan: &'a HlodMemoryPlan,
    pub settings: Settings,
    pub phases: &'a Phases<'a>,
}

/// One closed frame (a parent whose subtree has been read), in postorder.
struct Closed {
    id: u64,
    seq: u64,
    key: TileKey,
    children: Vec<u64>,
    /// The cell's only child is a leaf (which cannot stand for it): the parent is reduced
    /// to half its input, so it is a genuinely coarser level, not a copy of the leaf.
    single_leaf: bool,
}

/// What an unconsumed child of the walk is.
#[derive(Clone, Copy)]
enum Child {
    /// A leaf (it always publishes at its own cell).
    Leaf,
    /// A coarse tile and its row's sequence number.
    Coarse(u64),
}

/// A coarse row's published depth: its own cell's, or the top of the pass-through chain
/// above it (final once its parent cell has a second child or the walk ends).
#[derive(Clone, Copy)]
struct Publish {
    depth: u32,
    done: bool,
}

/// Push the completed child `id` into `frame`. A cell that has begun a second child is no
/// pass-through, so its coarse children's published depths are final (known as soon as the
/// second child starts, so no row waits on a sibling's subtree).
fn adopt(
    frame: &mut Frame,
    id: u64,
    kinds: &HashMap<u64, Child>,
    publish: &mut HashMap<u64, Publish>,
) {
    frame.children.push(id);
    settle(frame, kinds, publish);
}

fn settle(frame: &Frame, kinds: &HashMap<u64, Child>, publish: &mut HashMap<u64, Publish>) {
    if frame.started >= 2 {
        for c in &frame.children {
            if let Some(Child::Coarse(seq)) = kinds.get(c) {
                if let Some(p) = publish.get_mut(seq) {
                    p.done = true;
                }
            }
        }
    }
}

struct Frame {
    id: u64,
    node: NodeRecord,
    leaf_begin: u32,
    children: Vec<u64>,
    /// Direct children begun (in preorder), complete or not.
    started: usize,
}

type Done = Result<(TileGeometry, HlodRow, appearance::Timing), JobError>;

/// Build, check and write every coarse tile; writes `proxies/` and `proxies.jsonl`.
pub(crate) fn produce(
    input: HlodInput<'_>,
    check: &(dyn Fn() -> Result<(), JobError> + Sync),
) -> Result<(HlodReport, HlodDiagnostics), JobError> {
    let started = std::time::Instant::now();
    let cpu_started = resources::process_cpu_seconds();
    let captured = input.captured;
    let settings = input.settings;
    let mut report = HlodReport {
        schema_version: 1,
        method: "octree-hlod",
        error_policy: error::POLICY,
        appearance: APPEARANCE_POLICY,
        triangle_cap: settings.cap as u64,
        error_measurement: error::ErrorReport {policy:error::POLICY,seed_rule:"SplitMix64; 0x5332202610240001 xor first*0x9e3779b97f4a7c15 xor rotl(end,23) xor depth; P-to-S xor 0xa5a5a5a5a5a5a5a5",..Default::default()},
        ..Default::default()
    };
    let mut diagnostics = HlodDiagnostics {
        jobs: input.plan.jobs,
        ledger_bytes: input.plan.ledger_bytes,
        ..Default::default()
    };
    let proxies_dir = input.stage.join("proxies");
    fs::create_dir_all(&proxies_dir)
        .map_err(|e| JobError::io("create coarse tile folder", &proxies_dir, e))?;
    let rows_path = input.stage.join("proxies.jsonl");
    if captured.tree.stats.internal == 0 {
        fs::write(&rows_path, b"").map_err(|e| JobError::io("write coarse rows", &rows_path, e))?;
        diagnostics.wall_seconds = started.elapsed().as_secs_f64();
        return Ok((report, diagnostics));
    }
    // The incidence pass is capture's declared `lod_prepass` scratch stage.
    input.phases.progress(Phase::HlodIncidence, 0, Some(1))?;
    let incidence_started = std::time::Instant::now();
    captured.scratch.begin_stage();
    let (rows, stats) = interfaces::build(
        &captured.scratch,
        input.config.sorter(1),
        &captured.tree.nodes,
        &captured.triangles,
        &captured.corner_depths,
        &captured.lattice,
        check,
    )?;
    report.incidence = stats;
    diagnostics.incidence_cpu_seconds = resources::process_cpu_seconds()
        .zip(cpu_started)
        .map(|(b, a)| b - a);
    input.phases.progress(Phase::HlodIncidence, 1, Some(1))?;
    input.phases.finished(
        Phase::HlodIncidence,
        incidence_started.elapsed().as_secs_f64(),
    )?;
    let walk_started = std::time::Instant::now();
    let index_start = resources::process_cpu_seconds();
    let source = source_index::SourceIndex::build(captured, check)?;
    let sampling = error::SamplingPlan::new(captured)?;
    let index_cpu = resources::process_cpu_seconds()
        .zip(index_start)
        .map(|(b, a)| b - a);
    let result = walk(
        &input,
        rows,
        &rows_path,
        &mut report,
        &source,
        &sampling,
        check,
    );
    diagnostics.error_cpu_seconds = index_cpu.map(|s| s + report.error_cpu_seconds);
    diagnostics.error_cpu = report.error_cpu;
    diagnostics.error_cpu.index = index_cpu.unwrap_or(0.);
    diagnostics.incidence_scratch_peak_bytes = captured.scratch.stage_peak_bytes();
    let bound = captured.report.scratch.stages.lod_prepass;
    let (lease_peak, timings, cache) = result?;
    diagnostics.appearance_seconds = timings;
    diagnostics.image_cache = cache;
    if diagnostics.incidence_scratch_peak_bytes > bound {
        return Err(resource(format!(
            "HLOD scratch {} bytes exceeded its declared stage bound {bound}",
            diagnostics.incidence_scratch_peak_bytes
        )));
    }
    input
        .phases
        .finished(Phase::HlodGeometry, walk_started.elapsed().as_secs_f64())?;
    diagnostics.error_measurement = report.error_measurement.clone();
    diagnostics.tiles = report.tiles;
    diagnostics.appearance_by_depth = report.appearance_by_depth.clone();
    diagnostics.lease_peak_bytes = lease_peak;
    diagnostics.cpu_seconds = resources::process_cpu_seconds()
        .zip(cpu_started)
        .map(|(b, a)| b - a);
    diagnostics.wall_seconds = started.elapsed().as_secs_f64();
    Ok((report, diagnostics))
}

/// Leaf `geometryError` values in leaf order, from the published leaf rows.
struct LeafErrors {
    lines: std::io::Lines<std::io::BufReader<fs::File>>,
    path: std::path::PathBuf,
}

impl LeafErrors {
    fn next(&mut self, first: u64) -> Result<f64, JobError> {
        let line = self
            .lines
            .next()
            .ok_or_else(|| state("leaf rows end before the tree's leaves"))?
            .map_err(|e| JobError::io("read leaf rows", &self.path, e))?;
        let row: Value = serde_json::from_str(&line).map_err(|e| state(e.to_string()))?;
        if row["first"].as_u64() != Some(first) {
            return Err(state("leaf rows differ from the tree's leaf order"));
        }
        let error = row["geometryError"]
            .as_f64()
            .filter(|e| e.is_finite() && *e >= 0.)
            .ok_or_else(|| state("leaf row without a finite geometryError"))?;
        Ok(error)
    }
}

/// The rank-ordered streams the walk reads leaves from.
struct LeafStreams<'a> {
    triangles: crate::spill::SequenceReader<'a, super::attributes::FullTriangle>,
    depths: crate::spill::SequenceReader<'a, super::full::CornerDepth>,
    rows: std::iter::Peekable<crate::spill::SortedStream<'a, InterfaceRow>>,
}

fn read_leaf(
    node: &NodeRecord,
    leaf: u32,
    captured: &FullCaptured,
    streams: &mut LeafStreams<'_>,
    error: f64,
    check: &(dyn Fn() -> Result<(), JobError> + Sync),
) -> Result<TileGeometry, JobError> {
    let LeafStreams {
        triangles,
        depths,
        rows,
    } = streams;
    let count = usize::try_from(node.triangles()).map_err(|_| resource("leaf count overflow"))?;
    // Every vector is sized for its proven worst case up front, so nothing regrows; see
    // `store::LEAF_READ_BYTES_PER_TRIANGLE` for the resulting peak.
    let mut tile = TileGeometry {
        key: TileKey {
            first: node.first,
            end: node.end,
            depth: node.depth(),
            leaf_begin: leaf,
            leaf_end: leaf + 1,
        },
        ells: Vec::with_capacity(3 * count),
        positions: Vec::with_capacity(3 * count),
        triangles: Vec::with_capacity(count),
        representative: Vec::new(),
        primitives: Vec::new(),
        interface: Vec::new(),
        geometric_error: error,
        lease: None,
        measurement: None,
    };
    let mut reps = Vec::with_capacity(count);
    {
        let mut index: HashMap<interfaces::Key, u32> = HashMap::with_capacity(3 * count);
        for i in 0..count {
            if i.is_multiple_of(4096) {
                check()?;
            }
            let t = triangles
                .next()
                .transpose()?
                .ok_or_else(|| state("truncated leaf triangles"))?;
            let ell = depths
                .next()
                .transpose()?
                .ok_or_else(|| state("truncated leaf corner depths"))?
                .depths;
            // Welded by canonical decoded position. Corners that coincide carry no surface;
            // such triangles are kept only for a parent whose descendants have no surface.
            let mut corners = [[0f32; 3]; 3];
            for k in 0..3 {
                corners[k] = captured
                    .lattice
                    .canonical(t.triangle.positions[k], ell[k])?;
            }
            let mut local = [0u32; 3];
            for k in 0..3 {
                let next = tile.positions.len() as u32;
                let id = *index.entry(interfaces::key(corners[k])).or_insert(next);
                if id == next {
                    tile.ells.push(ell[k]);
                    tile.positions.push(corners[k]);
                } else {
                    let e = &mut tile.ells[id as usize];
                    *e = (*e).max(ell[k]);
                }
                local[k] = id;
            }
            tile.triangles.push(local);
            let s = t.triangle.source;
            reps.push(SourcePrimitive {
                node: s.node,
                mesh: s.mesh,
                primitive: s.primitive,
                expanded_ordinal: t.expanded_ordinal,
                material: t.material,
            });
        }
    }
    tile.ells.shrink_to_fit();
    tile.positions.shrink_to_fit();
    let mut table = reps.clone();
    table.sort_unstable();
    table.dedup();
    table.shrink_to_fit();
    tile.representative = reps
        .iter()
        .map(|r| table.binary_search(r).unwrap() as u32)
        .collect();
    drop(reps);
    tile.primitives = table;
    // Rows arrive sorted by (leaf, item, interval): merge each item as it streams. Every
    // item is a point or segment of the leaf's own surface (at most six per triangle).
    let items = 6 * count;
    tile.interface.reserve_exact(items);
    while let Some(row) = rows.next_if(|r| matches!(r, Ok(r) if r.leaf == leaf)) {
        let row = row?;
        if let Some(last) = tile.interface.last_mut().filter(|l| l.item() == row.item()) {
            last.lo = last.lo.min(row.lo);
            last.hi = last.hi.max(row.hi);
        } else if tile.interface.len() == items {
            return Err(state(
                "more interface items than the leaf has points and segments",
            ));
        } else {
            tile.interface.push(row);
        }
    }
    if let Some(Err(_)) = rows.peek() {
        return Err(rows.next().unwrap().unwrap_err().into());
    }
    if let Some(Ok(row)) = rows.peek() {
        if row.leaf < leaf {
            return Err(state("interface rows out of leaf order"));
        }
    }
    keep_open_edges(&mut tile, leaf, items)?;
    shrink(&mut tile);
    Ok(tile)
}

/// Close the leaf's rows over what keeping them implies, conservatively (never a crack):
/// - a kept segment keeps its endpoints (the overlap join records only the segment): each
///   endpoint is a kept point with at least the segment's interval;
/// - an open edge of the leaf between two kept points is kept too (a T-junction side's
///   subdivided border, a hole chord ending on interfaces). Such an edge is outer only where
///   both its endpoints are, so it takes the narrower interval (an existing row for the
///   same segment widens to the union).
///
/// The rows stay merged and sorted by item within the `items` reserved: every row is a point
/// or segment of the leaf's own surface.
fn keep_open_edges(tile: &mut TileGeometry, leaf: u32, items: usize) -> Result<(), JobError> {
    if tile.interface.is_empty() {
        return Ok(());
    }
    let capacity = (2 * tile.interface.len()).min(3 * tile.triangles.len());
    let mut points: HashMap<interfaces::Key, (u32, u32)> = HashMap::with_capacity(capacity);
    for r in &tile.interface {
        let ends = if r.is_vertex() {
            [r.a, r.a]
        } else {
            [r.a, r.b]
        };
        for p in ends {
            let e = points.entry(p).or_insert((r.lo, r.hi));
            e.0 = e.0.min(r.lo);
            e.1 = e.1.max(r.hi);
        }
    }
    let merged = tile.interface.len();
    for (&p, &(lo, hi)) in &points {
        match tile.interface[..merged].binary_search_by_key(&(p, p), InterfaceRow::item) {
            Ok(at) => {
                let row = &mut tile.interface[at];
                row.lo = row.lo.min(lo);
                row.hi = row.hi.max(hi);
            }
            Err(_) if tile.interface.len() == items => {
                return Err(state(
                    "more interface items than the leaf has points and segments",
                ));
            }
            Err(_) => tile.interface.push(InterfaceRow {
                leaf,
                a: p,
                b: p,
                lo,
                hi,
            }),
        }
    }
    let key = |v: u32| interfaces::key(tile.positions[v as usize]);
    let mut edges: Vec<(interfaces::Key, interfaces::Key)> =
        Vec::with_capacity(3 * tile.triangles.len());
    for t in tile
        .triangles
        .iter()
        .filter(|t| t[0] != t[1] && t[1] != t[2] && t[0] != t[2])
    {
        for k in 0..3 {
            let (a, b) = (key(t[k]), key(t[(k + 1) % 3]));
            edges.push((a.min(b), a.max(b)));
        }
    }
    edges.sort_unstable();
    let mut i = 0;
    while i < edges.len() {
        let mut j = i + 1;
        while j < edges.len() && edges[j] == edges[i] {
            j += 1;
        }
        let (a, b) = edges[i];
        let faces = j - i;
        i = j;
        let (Some(pa), Some(pb)) = (points.get(&a), points.get(&b)) else {
            continue;
        };
        if faces == 2 {
            continue;
        }
        let (lo, hi) = (pa.0.max(pb.0), pa.1.min(pb.1));
        match tile.interface[..merged].binary_search_by_key(&(a, b), InterfaceRow::item) {
            Ok(at) => {
                let row = &mut tile.interface[at];
                row.lo = row.lo.min(lo);
                row.hi = row.hi.max(hi);
            }
            Err(_) if tile.interface.len() == items => {
                return Err(state(
                    "more interface items than the leaf has points and segments",
                ));
            }
            Err(_) => tile.interface.push(InterfaceRow { leaf, a, b, lo, hi }),
        }
    }
    tile.interface.sort_unstable_by_key(InterfaceRow::item);
    Ok(())
}

/// Release spare capacity so the held charge is the tile's true footprint.
fn shrink(tile: &mut TileGeometry) {
    tile.ells.shrink_to_fit();
    tile.positions.shrink_to_fit();
    tile.triangles.shrink_to_fit();
    tile.representative.shrink_to_fit();
    tile.primitives.shrink_to_fit();
    tile.interface.shrink_to_fit();
}

/// One parent: geometry, encoding, the pre-publication check, error and its member.
fn build_tile(
    closed: &Closed,
    children: Vec<TileGeometry>,
    input: &HlodInput<'_>,
    services: &appearance::Services<'_>,
    source: &source_index::SourceIndex,
    sampling: &error::SamplingPlan,
    check: &(dyn Fn() -> Result<(), JobError> + Sync),
) -> Done {
    let captured = input.captured;
    let settings = input.settings;
    let stage = input.stage;
    let reusable = children
        .iter()
        .find(|c| c.key.first == closed.key.first && c.key.end == closed.key.end)
        .and_then(|c| c.measurement.map(|m| (c.key.depth, m)));
    let mut built = if closed.single_leaf {
        let half = children[0].triangles.len().div_ceil(2).min(settings.cap);
        match geometry::build_parent(closed.key, &children, half) {
            // Its interface cannot be halved: the copy within the cap.
            Err(e) if e.kind() == JobErrorKind::Unsupported => {
                geometry::build_parent(closed.key, &children, settings.cap)?
            }
            built => built?,
        }
    } else {
        geometry::build_parent(closed.key, &children, settings.cap)?
    };
    drop(children);
    if settings.mutation == Mutation::DropSegment {
        let tile = &mut built.geometry;
        let locked: Vec<bool> = built.locked.clone();
        if let Some(t) = tile
            .triangles
            .iter()
            .position(|t| t.iter().filter(|v| locked[**v as usize]).count() >= 2)
        {
            tile.triangles.remove(t);
            tile.representative.remove(t);
        }
    }
    let cpu_start = error::thread_cpu();
    let mut query = source.query(captured, built.geometry.key)?;
    // Identity is chosen on the final decoded centroid, in the descendant SOURCE range.
    // Charting/encoding must preserve these positions and source rows in S3.
    let positions = encode::positions(
        &built.geometry,
        &built.locked,
        &captured.lattice,
        settings.mutation,
    )?;
    let mut representatives = Vec::with_capacity(built.geometry.triangles.len());
    let mut centroid_checks = Vec::with_capacity(built.geometry.triangles.len());
    for (i, t) in built.geometry.triangles.iter().enumerate() {
        if i % 256 == 0 {
            check()?;
        }
        let p = std::array::from_fn(|a| {
            t.iter()
                .map(|&v| f64::from(positions.decoded[v as usize][a]))
                .sum::<f64>()
                / 3.
        });
        let hit = query.nearest(p)?;
        representatives.push(query.primitive(hit));
        centroid_checks.push(hit.distance2.sqrt());
    }
    let mut table = representatives.clone();
    table.sort_unstable();
    table.dedup();
    built.geometry.representative = representatives
        .iter()
        .map(|p| table.binary_search(p).unwrap() as u32)
        .collect();
    built.geometry.primitives = table;
    let identity_cpu = error::thread_cpu().zip(cpu_start).map(|(b, a)| b - a);
    let tile = &built.geometry;
    // The error is measured on the published decode before the appearance bake, which
    // needs G for its texel spacing; charting never moves a position, and the encoded
    // bytes are checked to decode to exactly these position bits below. A child's
    // measurement is reused when the decoded geometry and identity are byte-identical.
    let digest = geometry_digest(tile, &positions.decoded);
    let count = sampling.count(tile.key)?;
    let cpu_start = error::thread_cpu();
    let (error, measurement, mut error_cpu) = if let Some((depth, cached)) = reusable
        .filter(|(_, c)| c.sha256 == digest && c.summary.area_samples_per_direction >= count)
    {
        let (g, summary) = error::reuse(cached.summary, depth, built.children_error)?;
        (g, summary, error::CpuReport::default())
    } else {
        error::measure(
            &CoarseTile {
                key: tile.key,
                decoded: &positions.decoded,
                triangles: &tile.triangles,
                locked: &built.locked,
                representative: &tile.representative,
                primitives: &tile.primitives,
                bounds: positions.bounds,
                children_error: built.children_error,
                simplifier_error: built.simplifier_error,
                encode_displacement: positions.displacement,
                stopping: built.stopping,
            },
            &mut query,
            count,
            &centroid_checks,
            check,
        )?
    };
    error_cpu.representatives = identity_cpu.unwrap_or(0.);
    let error_cpu_seconds = error::thread_cpu()
        .zip(cpu_start)
        .zip(identity_cpu)
        .map_or(0., |((b, a), i)| b - a + i);
    // S3: fresh charts and a cheap source bake on the final decoded positions, through
    // the same bounded source query.
    let appearance = appearance::build(
        services,
        &mut query,
        &appearance::TileView {
            decoded: &positions.decoded,
            triangles: &tile.triangles,
            materials: tile
                .representative
                .iter()
                .map(|r| tile.primitives[*r as usize].material)
                .collect(),
            geometric_error: error.geometric_error,
        },
    )?;
    let encoded = encode::encode(tile, positions, &appearance, settings.mutation)?;
    drop(appearance.groups);
    if settings.mutation != Mutation::MoveVertexUnchecked {
        // Every decoded corner carries exactly the measured (and charted) position bits.
        encode::check(tile, &built.locked, &encoded)?;
    }
    if !error.geometric_error.is_finite() || error.geometric_error < built.children_error {
        return Err(state("coarse error is nonfinite or below its children's"));
    }
    let uri = tile.key.uri();
    let path = stage.join(&uri);
    let mut file =
        fs::File::create(&path).map_err(|e| JobError::io("create coarse tile", &path, e))?;
    file.write_all(&encoded.glb)
        .and_then(|_| file.sync_all())
        .map_err(|e| JobError::io("write coarse tile", &path, e))?;
    let row = HlodRow {
        schema_version: 1,
        uri,
        sha256: Sha256::digest(&encoded.glb)
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect(),
        bytes: encoded.glb.len() as u64,
        first: tile.key.first,
        end: tile.key.end,
        depth: tile.key.depth,
        encoded_depth: tile.key.depth,
        leaf_begin: tile.key.leaf_begin,
        leaf_end: tile.key.leaf_end,
        geometric_error: error.geometric_error,
        error_policy: error.policy,
        measurement,
        error_cpu_seconds,
        error_cpu,
        children_error: built.children_error,
        simplifier_error: built.simplifier_error,
        encode_displacement: encoded.displacement,
        primitives: tile.primitives.len() as u32,
        gltf_primitives: encoded.gltf_primitives as u32,
        triangles: tile.triangles.len() as u32,
        input_triangles: built.input_triangles,
        stopping_bound: built.stopping,
        protected_faces: built.protected_faces,
        retries: built.retries,
        interface_vertices: built.interface_vertices,
        interface_segments: built.interface_segments,
        quantized: encoded.quantized,
        bounds: encoded.bounds.into(),
        appearance: appearance.facts,
    };
    let mut geometry = built.geometry;
    geometry.geometric_error = error.geometric_error;
    geometry.measurement = Some(store::CachedMeasurement {
        sha256: digest,
        summary: measurement,
    });
    Ok((geometry, row, appearance.timing))
}

/// SHA-256 of what the error measurement reads: decoded positions, triangles and the
/// representative identity.
fn geometry_digest(tile: &TileGeometry, decoded: &[[f32; 3]]) -> [u8; 32] {
    let mut h = Sha256::new();
    for p in decoded {
        for v in p {
            h.update(v.to_bits().to_le_bytes());
        }
    }
    for t in &tile.triangles {
        for v in t {
            h.update(v.to_le_bytes());
        }
    }
    for r in &tile.representative {
        h.update(r.to_le_bytes());
    }
    for p in &tile.primitives {
        for v in [u64::from(p.node), u64::from(p.mesh), u64::from(p.primitive), p.expanded_ordinal, u64::from(p.material)] {
            h.update(v.to_le_bytes());
        }
    }
    h.finalize().into()
}

fn tally(report: &mut HlodReport, row: &HlodRow) {
    report.error_cpu_seconds += row.error_cpu_seconds;
    report.error_cpu.add(row.error_cpu);
    report.tiles += 1;
    report.triangles += u64::from(row.triangles);
    report.max_triangles = report.max_triangles.max(u64::from(row.triangles));
    report.input_triangles += u64::from(row.input_triangles);
    match row.stopping_bound {
        StoppingBound::Input => report.stopped_by_input += 1,
        StoppingBound::TriangleBudget => report.stopped_by_triangle_budget += 1,
    }
    report.tiles_with_protected_faces += u64::from(row.protected_faces > 0);
    report.protected_faces += u64::from(row.protected_faces);
    report.retries += u64::from(row.retries);
    report.interface_segments += u64::from(row.interface_segments);
    report.interface_vertices += u64::from(row.interface_vertices);
    if row.quantized {
        report.quantized_tiles += 1;
    } else {
        report.float_tiles += 1;
    }
    report.bytes += row.bytes;
    let depth = row.depth as usize;
    if report.error_measurement.by_depth.len() <= depth {
        report
            .error_measurement
            .by_depth
            .resize_with(depth + 1, Default::default);
    }
    report.error_measurement.by_depth[depth].add(row.measurement, row.geometric_error);
    if depth == 0 {
        report.error_measurement.root = Some(row.measurement);
    }
    if report.max_error_by_depth.len() <= depth {
        report.max_error_by_depth.resize(depth + 1, 0.);
    }
    report.max_error_by_depth[depth] = report.max_error_by_depth[depth].max(row.geometric_error);
    let a = &row.appearance;
    if report.appearance_by_depth.len() <= depth {
        let start = report.appearance_by_depth.len();
        report
            .appearance_by_depth
            .extend((start..=depth).map(|d| DepthAppearance {
                depth: d as u32,
                ..Default::default()
            }));
    }
    let d = &mut report.appearance_by_depth[depth];
    let first = d.tiles == 0;
    d.tiles += 1;
    d.vertex_color_tiles += u64::from(a.vertex_color_groups > 0);
    d.pages += u64::from(a.pages);
    d.layout_texels += a.layout_texels;
    d.page_texels += a.page_texels;
    d.covered_texels += a.covered_texels;
    d.searches += a.searches;
    d.donor_reads += a.donor_reads;
    d.escalated_pages += u64::from(a.escalated_pages);
    d.unmipped_pages += u64::from(a.unmipped_pages);
    if first {
        d.requested_spacing_min = a.requested_spacing;
        d.requested_spacing_max = a.requested_spacing;
    }
    d.requested_spacing_min = d.requested_spacing_min.min(a.requested_spacing);
    d.requested_spacing_max = d.requested_spacing_max.max(a.requested_spacing);
    if a.textured_groups > 0 {
        if d.textured_tiles == 0 {
            d.achieved_spacing_min = a.achieved_spacing;
            d.achieved_spacing_max = a.achieved_spacing;
        }
        let n = d.textured_tiles as f64;
        d.mean_spacing_ratio =
            (d.mean_spacing_ratio * n + a.achieved_spacing / a.requested_spacing) / (n + 1.);
        d.textured_tiles += 1;
        d.capped_tiles += u64::from(a.capped);
        d.source_limited_tiles += u64::from(a.source_limited);
        d.achieved_spacing_min = d.achieved_spacing_min.min(a.achieved_spacing);
        d.achieved_spacing_max = d.achieved_spacing_max.max(a.achieved_spacing);
    }
}

/// The preorder walk; returns the ledger's lease peak.
fn walk(
    input: &HlodInput<'_>,
    rows: crate::spill::SortedStream<'_, InterfaceRow>,
    rows_path: &Path,
    report: &mut HlodReport,
    source: &source_index::SourceIndex,
    sampling: &error::SamplingPlan,
    check: &(dyn Fn() -> Result<(), JobError> + Sync),
) -> Result<(u64, appearance::Timing, bake::CacheStats), JobError> {
    let captured = input.captured;
    let plan = input.plan;
    let settings = input.settings;
    let ledger = Ledger::new(plan.ledger_bytes);
    let stage = input.stage;
    let mut timings = appearance::Timing::default();
    let cache = bake::ImageCache::new(plan.image_cache_bytes);
    let layout = captured.triangles.layout().0;
    let services = appearance::Services {
        captured,
        cache: &cache,
        profile: input.config.profile,
        max_edge: appearance::MAX_PAGE_EDGE,
        page_memory: plan.page_bytes,
        tile_page_bytes: plan.tile_page_bytes,
        colors: layout & super::attributes::COLOR != 0,
        normals: layout & super::attributes::NORMAL != 0,
        check,
    };
    let job_bytes = plan.job_bytes;
    let out_bytes = TileGeometry::charge(settings.cap as u64);
    let jobs = usize::try_from(plan.jobs.max(1)).map_err(|_| resource("jobs overflow"))?;
    let pool = rayon::ThreadPoolBuilder::new()
        .num_threads(jobs)
        .thread_name(|i| format!("rt-hlod-{i}"))
        .build()
        .map_err(|e| resource(format!("HLOD worker pool: {e}")))?;
    let leaf_rows_path = input.stage.join("leaves.jsonl");
    let mut leaf_errors = LeafErrors {
        lines: std::io::BufReader::new(
            fs::File::open(&leaf_rows_path)
                .map_err(|e| JobError::io("read leaf rows", &leaf_rows_path, e))?,
        )
        .lines(),
        path: leaf_rows_path.clone(),
    };
    let file = fs::File::create(rows_path)
        .map_err(|e| JobError::io("create coarse rows", rows_path, e))?;
    let mut writer = BufWriter::new(file);
    let mut nodes = captured.tree.nodes.reader()?;
    let mut streams = LeafStreams {
        triangles: captured.triangles.reader()?,
        depths: captured.corner_depths.reader()?,
        rows: rows.peekable(),
    };
    let total = captured.tree.stats.internal;
    input.phases.progress(Phase::HlodGeometry, 0, Some(total))?;
    let mut passed = 0u64;
    let outcome = pool.in_place_scope(|scope| -> Result<(), JobError> {
        let (sender, receiver) = mpsc::channel::<(u64, u64, Done)>();
        let mut stack: Vec<Frame> = Vec::new();
        let mut ready: HashMap<u64, TileGeometry> = HashMap::new();
        let mut pending: Vec<Closed> = Vec::new();
        let mut finished_rows: BTreeMap<u64, HlodRow> = BTreeMap::new();
        let (mut next_id, mut next_seq, mut written, mut inflight) = (0u64, 0u64, 0u64, 0usize);
        let mut leaf = 0u32;
        let services = &services;
        // Spawn every pending parent whose children are ready, in postorder, while workers
        // and the ledger admit its job.
        let spawn = |pending: &mut Vec<Closed>,
                     ready: &mut HashMap<u64, TileGeometry>,
                     inflight: &mut usize|
         -> Result<(), JobError> {
            let mut i = 0;
            while i < pending.len() && *inflight < jobs {
                if !pending[i].children.iter().all(|c| ready.contains_key(c)) {
                    i += 1;
                    continue;
                }
                let Some(job_lease) = ledger.try_lease(job_bytes, 0) else {
                    break;
                };
                let closed = pending.remove(i);
                let children: Vec<TileGeometry> = closed
                    .children
                    .iter()
                    .map(|c| ready.remove(c).unwrap())
                    .collect();
                let sender = sender.clone();
                *inflight += 1;
                scope.spawn(move |_| {
                    let (id, seq) = (closed.id, closed.seq);
                    let done = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                        let mut built = build_tile(
                            &closed, children, input, services, source, sampling, check,
                        )?;
                        // The job's admission includes its output: keep that part while the
                        // tile is held for its parent, release the workspace.
                        let mut lease = job_lease;
                        let held = built.0.check_charge()?;
                        if held > out_bytes {
                            return Err(state("a coarse tile exceeds its admitted output"));
                        }
                        lease.shrink(held);
                        built.0.lease = Some(lease);
                        Ok(built)
                    }))
                    .unwrap_or_else(|_| Err(state("HLOD worker panicked")));
                    let _ = sender.send((id, seq, done));
                });
            }
            Ok(())
        };
        let mut receive = |pending: &mut Vec<Closed>,
                       ready: &mut HashMap<u64, TileGeometry>,
                       inflight: &mut usize,
                       finished_rows: &mut BTreeMap<u64, HlodRow>|
         -> Result<(), JobError> {
            if *inflight == 0 {
                return Err(resource(
                    "HLOD stage cannot admit its next tile within the memory budget",
                ));
            }
            let (id, seq, done) = receiver
                .recv()
                .map_err(|_| state("HLOD worker ended without a result"))?;
            *inflight -= 1;
            let (geometry, row, timing) = done?;
            timings.surface += timing.surface;
            timings.charts += timing.charts;
            timings.query += timing.query;
            timings.sample += timing.sample;
            timings.encode += timing.encode;
            ready.insert(id, geometry);
            finished_rows.insert(seq, row);
            spawn(pending, ready, inflight)
        };
        // Write finished rows in postorder once their published depth is final.
        let mut flush = |finished_rows: &mut BTreeMap<u64, HlodRow>,
                         publish: &mut HashMap<u64, Publish>,
                         written: &mut u64|
         -> Result<(), JobError> {
            while publish.get(written).is_some_and(|p| p.done)
                && finished_rows.contains_key(written)
            {
                let mut row = finished_rows.remove(written).unwrap();
                let depth = publish.remove(written).unwrap().depth;
                if depth != row.depth {
                    // Published at the top of its single-child chain: the member is named
                    // for that cell.
                    let key = TileKey {
                        first: row.first,
                        end: row.end,
                        depth,
                        leaf_begin: row.leaf_begin,
                        leaf_end: row.leaf_end,
                    };
                    let (from, to) = (stage.join(&row.uri), stage.join(key.uri()));
                    fs::rename(&from, &to)
                        .map_err(|e| JobError::io("rename coarse tile", &from, e))?;
                    row.uri = key.uri();
                    row.depth = depth;
                }
                tally(report, &row);
                serde_json::to_writer(&mut writer, &row).map_err(|e| state(e.to_string()))?;
                writer
                    .write_all(b"\n")
                    .map_err(|e| JobError::io("write coarse rows", rows_path, e))?;
                *written += 1;
                input
                    .phases
                    .progress(Phase::HlodGeometry, *written, Some(total))?;
            }
            Ok(())
        };
        let mut kinds: HashMap<u64, Child> = HashMap::new();
        let mut publish: HashMap<u64, Publish> = HashMap::new();
        let passed = &mut passed;
        let close = |stack: &mut Vec<Frame>,
                     pending: &mut Vec<Closed>,
                     leaf: u32,
                     next_seq: &mut u64,
                     kinds: &mut HashMap<u64, Child>,
                     publish: &mut HashMap<u64, Publish>,
                     passed: &mut u64|
         -> Result<(), JobError> {
            let frame = stack.pop().ok_or_else(|| state("no open frame"))?;
            if frame.children.is_empty() {
                return Err(state("an internal tile closed without children"));
            }
            // Pass-through: a single coarse child is the whole cell, and its parent would be
            // the same tile (input within the cap is kept as is). The child is published at
            // the top of the chain instead, and the cells below it there publish no content.
            // Same rank range and leaf interval, so the same outer interface.
            if let ([only], 1) = (&frame.children[..], frame.started) {
                let only = *only;
                if let Some(Child::Coarse(seq)) = kinds.get(&only).copied() {
                    let p = publish
                        .get_mut(&seq)
                        .ok_or_else(|| state("a coarse child without its row"))?;
                    p.depth = frame.node.depth();
                    match stack.last_mut() {
                        Some(parent) => adopt(parent, only, kinds, publish),
                        None => p.done = true,
                    }
                    *passed += 1;
                    return Ok(());
                }
            }
            let single_leaf = matches!(
                frame.children[..],
                [only] if matches!(kinds.get(&only), Some(Child::Leaf))
            );
            for c in &frame.children {
                kinds.remove(c);
            }
            let closed = Closed {
                id: frame.id,
                seq: *next_seq,
                key: TileKey {
                    first: frame.node.first,
                    end: frame.node.end,
                    depth: frame.node.depth(),
                    leaf_begin: frame.leaf_begin,
                    leaf_end: leaf,
                },
                children: frame.children,
                single_leaf,
            };
            *next_seq += 1;
            kinds.insert(closed.id, Child::Coarse(closed.seq));
            publish.insert(
                closed.seq,
                Publish {
                    depth: closed.key.depth,
                    done: stack.is_empty(),
                },
            );
            if let Some(parent) = stack.last_mut() {
                adopt(parent, closed.id, kinds, publish);
            }
            pending.push(closed);
            Ok(())
        };
        let run = (|| -> Result<(), JobError> {
            for node in nodes.by_ref() {
                let node = node?;
                check()?;
                // Finished rows wait for every earlier tile in postorder: while too many wait,
                // read nothing further and let in-flight jobs finish. No earlier tile needs
                // a leaf not yet read (it is closed), so this never blocks the walk.
                while finished_rows.len() >= ROW_WINDOW && inflight > 0 {
                    receive(&mut pending, &mut ready, &mut inflight, &mut finished_rows)?;
                    flush(&mut finished_rows, &mut publish, &mut written)?;
                }
                while stack.last().is_some_and(|f| f.node.end <= node.first) {
                    close(
                        &mut stack,
                        &mut pending,
                        leaf,
                        &mut next_seq,
                        &mut kinds,
                        &mut publish,
                        passed,
                    )?;
                    flush(&mut finished_rows, &mut publish, &mut written)?;
                    spawn(&mut pending, &mut ready, &mut inflight)?;
                }
                let id = next_id;
                next_id += 1;
                if let Some(parent) = stack.last_mut() {
                    parent.started += 1;
                    settle(parent, &kinds, &mut publish);
                }
                if node.kind == NodeKind::Internal {
                    stack.push(Frame {
                        id,
                        node,
                        leaf_begin: leaf,
                        children: Vec::new(),
                        started: 0,
                    });
                    continue;
                }
                let charge = node
                    .triangles()
                    .saturating_mul(store::LEAF_READ_BYTES_PER_TRIANGLE)
                    .saturating_add(store::LEAF_READ_FIXED_BYTES);
                let mut lease = loop {
                    // Read-ahead keeps one job's admission free for the parents it feeds.
                    if let Some(lease) = ledger.try_lease(charge, job_bytes) {
                        break lease;
                    }
                    receive(&mut pending, &mut ready, &mut inflight, &mut finished_rows)?;
                    flush(&mut finished_rows, &mut publish, &mut written)?;
                };
                let error = leaf_errors.next(node.first)?;
                let mut tile = read_leaf(&node, leaf, captured, &mut streams, error, check)?;
                lease.shrink(tile.check_charge()?);
                tile.lease = Some(lease);
                leaf += 1;
                match stack.last_mut() {
                    Some(parent) => {
                        kinds.insert(id, Child::Leaf);
                        adopt(parent, id, &kinds, &mut publish);
                        ready.insert(id, tile);
                    }
                    None => {
                        return Err(state(
                            "a leaf without a parent in a tree with internal tiles",
                        ))
                    }
                }
            }
            while !stack.is_empty() {
                close(
                    &mut stack,
                    &mut pending,
                    leaf,
                    &mut next_seq,
                    &mut kinds,
                    &mut publish,
                    passed,
                )?;
            }
            spawn(&mut pending, &mut ready, &mut inflight)?;
            while !pending.is_empty() || inflight > 0 {
                receive(&mut pending, &mut ready, &mut inflight, &mut finished_rows)?;
                flush(&mut finished_rows, &mut publish, &mut written)?;
            }
            flush(&mut finished_rows, &mut publish, &mut written)?;
            if written + *passed != total || !publish.is_empty() || streams.rows.next().is_some() {
                return Err(state("coarse tiles or interface rows differ from the tree"));
            }
            Ok(())
        })();
        if run.is_err() {
            // Let running workers finish so the scope can end; their results are dropped.
            while inflight > 0 {
                if receiver.recv().is_err() {
                    break;
                }
                inflight -= 1;
            }
        }
        run
    });
    outcome?;
    report.pass_through = passed;
    if passed > 0 {
        // Pass-through cells complete with their child.
        input
            .phases
            .progress(Phase::HlodGeometry, total, Some(total))?;
    }
    if streams.triangles.next().transpose()?.is_some() || leaf_errors.lines.next().is_some() {
        return Err(state("HLOD walk did not consume every leaf"));
    }
    writer
        .flush()
        .and_then(|_| writer.get_ref().sync_all())
        .map_err(|e| JobError::io("finish coarse rows", rows_path, e))?;
    Ok((ledger.peak(), timings, cache.stats()))
}
