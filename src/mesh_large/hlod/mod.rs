//! 0.4 HLOD (decision record `docs/architecture/hlod-040-decision.md`, D11/S1b):
//! classic octree HLOD geometry over the frozen ownership tree.
//!
//! Leaves remain byte-identical. Capped immediate children are reconstructed against
//! common depth-wise border contracts, then simplified to at most 8,192 triangles.
//! Source-parameter certificates give identical same-depth edges and mixed-depth gaps
//! bounded by the coarse geometric error / 8. Terminal leaf contacts remain exact.
//! Impossible capped parents publish no content and omission propagates to the root;
//! bounded descendants provide the reported last-resort frontier.
//!
//! The stage streams: one spillable incidence pass, then a preorder walk that holds only
//! the tiles waiting for their parent, admitted against the run's declared memory budget
//! (D8) and released as each parent completes. Parents are built by up to `jobs` workers;
//! each is a pure function of its children, so output is identical for any job count or
//! budget. It writes `proxies/` and `proxies.jsonl` (postorder) for H16 delivery. Coarse
//! appearance (S3, [`appearance`]) is fresh content-sized charts
//! baked from the captured source; the error is measured against the source in both
//! directions by S2 (`error`). Omitted cells publish no content or measurement;
//! topology trials may query source geometry, but no appearance work runs for them.
pub(crate) mod appearance;
#[cfg(test)]
mod appearance_tests;
mod audit;
pub(crate) mod bake;
mod borders;
mod charts;
pub(crate) mod decode;
mod encode;
pub(crate) mod error;
mod geometry;
pub(crate) mod interfaces;
pub(crate) mod scratch;
pub(crate) mod source_index;
pub(crate) mod store;
#[cfg(test)]
mod tests;
#[cfg(test)]
pub(crate) use tests::test_settings;

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
    sync::mpsc,
};

/// S3's coarse appearance policy (recorded in the stage manifest).
pub(crate) const APPEARANCE_POLICY: &str = "hlod-s3-fresh-charts: content-sized pages, charts isolated through COARSE_ISLAND_ISOLATED_LEVELS mips, at max(G/16, 1e-5 m), \
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
/// These simplifier terms sum to under 1,570. The contracted face list adds 12
/// bytes per triangle and the exact-capacity reference clone adds at most 111, still
/// below 1,700. After meshopt returns (its temporaries and `simplify`'s arrays
/// freed), fold rejection holds source face/edge counts (under 330 bytes per triangle),
/// a sorted incidence array (168), and face flags. Its smaller face-group array is
/// freed before incidence allocation; all these tables are released before the next
/// meshopt call. This phase remains below the simplifier peak. Topology and constrained
/// remeshing run after the simplifier workspace is released; every output remains capped.
/// Earlier phases (the weld index while merging, the interface rows while merging them,
/// the representative tables) peak lower. Measured on adversarial jobs by
/// `parent_jobs_allocate_within_their_charge` (a counting allocator, meshoptimizer routed
/// through it).
const JOB_BYTES_PER_TRIANGLE: u64 = 1700;
/// Finished coarse rows held while an earlier tile is still being built (`walk`); each is a
/// few hundred bytes, charged within `STAGE_RESERVE`.
const ROW_WINDOW: usize = 1024;
/// Encoded GLB, compression scratch and decoded check copy of one coarse tile.
const JOB_ENCODE_BYTES: u64 = 16 << 20;

/// One parent job's admission for `merged` input triangles: its workspace, encode allowance,
/// S2's source query, measurement and nearest-search workspace, and a cap-sized output.
fn job_charge(merged: u64, settings: &Settings) -> u64 {
    merged
        .saturating_mul(JOB_BYTES_PER_TRIANGLE)
        .saturating_add(JOB_ENCODE_BYTES)
        .saturating_add(source_index::QUERY_BYTES)
        .saturating_add(error::WORK_BYTES)
        .saturating_add(error::search_bytes(settings.nearest_steps))
        .saturating_add(TileGeometry::charge(settings.cap as u64))
}

fn state(message: impl Into<String>) -> JobError {
    JobError::new(JobErrorKind::InvalidState, message.into())
}
fn resource(message: impl Into<String>) -> JobError {
    JobError::new(JobErrorKind::ResourceLimit, message.into())
}

/// Stage settings; production uses [`Settings::default`].
#[derive(Clone, Debug)]
pub(crate) struct Settings {
    pub audit_dir: Option<std::sync::Arc<std::path::PathBuf>>,
    pub cap: usize,
    pub mutation: Mutation,
    /// S2's per-query nearest-search budget (`error::NEAREST_STEPS`).
    pub nearest_steps: u64,
    /// S2's sampling schedule; delivery admits only production's outside library tests.
    pub sampling: error::Schedule,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            audit_dir: std::env::var_os("RT_HLOD_AUDIT_DIR").map(|p| std::sync::Arc::new(p.into())),
            cap: TRIANGLE_CAP,
            mutation: Mutation::None,
            nearest_steps: error::NEAREST_STEPS,
            sampling: error::Schedule::PRODUCTION,
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
    let tile_pages =
        appearance::tile_page_bytes(slots, captured.materials.len() as u64, cap as u64);
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
    settings: &Settings,
) -> Result<HlodMemoryPlan, JobError> {
    let cap = settings.cap;
    let leaf_cap = config.leaf_triangles();
    let held = TileGeometry::charge(leaf_cap.max(cap as u64));

    // Held children of the open path, plus the read transient of the one leaf being read
    // (its lease shrinks to the held footprint once read).
    let read = leaf_cap
        .saturating_mul(store::LEAF_READ_BYTES_PER_TRIANGLE)
        .saturating_add(store::LEAF_READ_FIXED_BYTES);
    let window = held
        .saturating_mul(8)
        .saturating_add(read.saturating_sub(held));
    // S3's donor search (its resolve state and a local surface) and appearance workspace
    // ride on S1/S2's job.
    let appearance = appearance_plan(captured, config.profile, cap)?;
    let job = job_charge(leaf_cap.max(cap as u64).saturating_mul(8), settings)
        .saturating_add(held.saturating_mul(8))
        .saturating_add(bake::RESOLVE_BYTES)
        .saturating_add(bake::LOCAL_BYTES)
        .saturating_add(appearance.texture)
        .saturating_add(appearance.color);
    let sorter = config.sorter(1);
    let incidence = interfaces::memory_need(sorter, leaf_cap, captured.tree.stats.leaves)
        .max(borders::memory_need(sorter));
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
        .saturating_add(borders::WALK_CACHE_BYTES)
        .saturating_add(streams);
    let minimum = fixed.saturating_add(
        incidence.max(
            window
                .saturating_add(job)
                .saturating_add(appearance.cache_floor),
        ),
    );
    let mut tmpfs = if captured.scratch.memory_backed() {
        memory_backed_need(captured, config, settings.cap)?
    } else {
        0
    };
    if let Some(path) = &settings.audit_dir {
        if audit::memory_backed(path)? {
            tmpfs = tmpfs.saturating_add(audit::admitted_bound(
                audit::bound(
                    captured
                        .triangles
                        .len()
                        .saturating_mul(6)
                        .saturating_mul(u64::from(captured.tree.stats.height)),
                    captured.tree.stats.nodes,
                    captured
                        .triangles
                        .len()
                        .checked_mul(u64::from(captured.tree.stats.height))
                        .ok_or_else(|| resource("audit triangle-depth bound overflows"))?,
                )
                .ok_or_else(|| resource("audit memory bound overflows"))?,
                true,
            ));
        }
    }
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
    let fit = ((ledger - window).saturating_sub(appearance.cache_floor) / job.max(1)).max(1);
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
    /// Content-less cells (omitted coarse tiles), by cell depth, and of them the sources:
    /// cells whose exact interface (or topology plain simplification cannot reduce) needs
    /// more triangles than the cap. Omission propagates to every ancestor (no content above a
    /// content-less cell), so the rest are those ancestors, pass-through chain cells above an
    /// omitted cell included; the capped tiles below are the refinement frontier. Equal to
    /// the content-less tiles of the published tileset.
    pub omitted_proxies: u64,
    pub omission_sources: u64,
    pub omitted_by_depth: Vec<u64>,
    /// The largest merged input of any parent job (at most eight capped children).
    pub max_job_input_triangles: u64,
    /// Simplified faces kept on their source orientation (`geometry::orient`).
    pub reoriented_faces: u64,
    pub interface_segments: u64,
    pub interface_vertices: u64,
    pub borders_by_depth: Vec<borders::DepthReceipt>,
    pub audit: Option<audit::Summary>,
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
    #[serde(skip)]
    pub error_cpu_by_depth: Vec<f64>,
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
    /// Pages delivered as the exact level-0 PNG.
    pub png_pages: u64,
    /// Donor searches that ran out of budget (best candidate so far used).
    pub donor_budget_exhausted: u64,
    /// Donor searches that began from their greedy seed (no candidate yet).
    pub donor_seeded: u64,
    /// Most steps one donor search took.
    pub donor_search_steps_max: u64,
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
    pub audit: Option<audit::FileReceipt>,
    pub(crate) borders_by_depth: Vec<borders::DepthReceipt>,
    /// Published content-less cells and the cap failures that originated them.
    pub omitted_proxies: u64,
    pub omission_sources: u64,
    /// One entry per captured tree depth, including depths with no omissions.
    pub omitted_by_depth: Vec<u64>,
    pub tiles: u64,
    pub jobs: u64,
    /// Whole-process user+system CPU seconds of the stage (incidence, walk, encoding).
    pub cpu_seconds: Option<f64>,
    pub incidence_cpu_seconds: Option<f64>,
    /// Source-index construction plus summed worker-thread measurement/identity CPU.
    pub error_cpu_seconds: Option<f64>,
    pub error_cpu: error::CpuReport,
    /// Summed worker S2 CPU seconds by encoded depth (index construction excluded).
    pub error_cpu_by_depth: Vec<f64>,
    pub error_measurement: error::ErrorReport,
    pub wall_seconds: f64,
    pub incidence_scratch_peak_bytes: u64,
    /// The incidence pass's staged scratch declarations (its declared bound is their
    /// largest) against which that peak is checked as the pass returns.
    pub incidence_scratch: interfaces::ScratchPhases,
    /// The largest admission of each walk scratch component (`scratch::ScratchLedger`: S2's
    /// source index, S3 pages and charts, ...).
    pub scratch_components: std::collections::BTreeMap<&'static str, u64>,
    /// The stage's scratch peak from the incidence pass's start to the walk's end, and its
    /// bound: the incidence pass's own, or the walk's base plus its largest simultaneous
    /// admission, whichever is larger.
    pub stage_scratch_peak_bytes: u64,
    pub stage_scratch_bound_bytes: u64,
    /// Highest simultaneously leased bytes of the walk's ledger.
    pub lease_peak_bytes: u64,
    pub held_spill: store::spill::Stats,
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
    pub settings: &'a Settings,
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
#[derive(Clone)]
struct Publish {
    depth: u32,
    done: bool,
    /// Depths of the pass-through cells above it (content-less too if it is omitted).
    chain: Vec<u32>,
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

/// A finished parent: its geometry (held for its own parent) and its published row, or
/// neither when its coarse tile is omitted.
type Done = Result<(Option<TileGeometry>, Finished, appearance::Timing), JobError>;

#[derive(Clone, Copy, Serialize)]
#[serde(rename_all = "snake_case")]
enum OmissionReason {
    ContractLowerBound,
    RemeshStalled,
    DescendantOmission,
}
impl OmissionReason {
    fn source(self) -> bool {
        !matches!(self, Self::DescendantOmission)
    }
}

/// A parent's outcome in postorder.
enum Finished {
    Row(Box<HlodRow>),
    /// Omitted at its cell depth: it could not keep its interface within the cap (`source`),
    /// or a child was omitted (propagated: omission reaches the root).
    Omitted {
        key: TileKey,
        reason: OmissionReason,
        frontier: Vec<TileKey>,
        topology: Box<geometry::TopologyReceipt>,
        topology_attempts: u32,
        construction_topology: Option<audit::Index>,
    },
}

/// Build, check and write every coarse tile; writes `proxies/` and `proxies.jsonl`.
pub(crate) fn produce(
    input: HlodInput<'_>,
    check: &(dyn Fn() -> Result<(), JobError> + Sync),
) -> Result<(HlodReport, HlodDiagnostics), JobError> {
    let started = std::time::Instant::now();
    let cpu_started = resources::process_cpu_seconds();
    let captured = input.captured;
    let settings = &input.settings;
    let mut report = HlodReport {
        schema_version: 1,
        method: "octree-hlod",
        error_policy: error::POLICY,
        appearance: APPEARANCE_POLICY,
        triangle_cap: settings.cap as u64,
        omitted_by_depth: vec![0; captured.tree.stats.height as usize],
        borders_by_depth: (0..captured.tree.stats.height)
            .map(|depth| borders::DepthReceipt {
                depth,
                ..Default::default()
            })
            .collect(),
        error_measurement: error::ErrorReport::new(settings.nearest_steps, settings.sampling),
        ..Default::default()
    };
    let mut diagnostics = HlodDiagnostics {
        jobs: input.plan.jobs,
        ledger_bytes: input.plan.ledger_bytes,
        omitted_by_depth: report.omitted_by_depth.clone(),
        borders_by_depth: report.borders_by_depth.clone(),
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
    // The incidence pass is capture's declared `hlod_incidence` scratch stage.
    input.phases.progress(Phase::HlodIncidence, 0, Some(1))?;
    let incidence_started = std::time::Instant::now();
    captured.scratch.begin_stage();
    let admit = |bound: u64| admit_scratch(captured, input.config, settings.cap, bound);
    let (rows, stats, source_open_ends) = interfaces::build(
        &captured.scratch,
        input.config.sorter(1),
        &captured.tree.nodes,
        &captured.triangles,
        &captured.corner_depths,
        &captured.lattice,
        &admit,
        check,
    )?;
    // The pass checks its own bytes as it returns, before any downstream stage writes: its
    // phases are over, only its row stream remains (consumed by the walk).
    diagnostics.incidence_scratch = stats.scratch;
    diagnostics.incidence_scratch_peak_bytes = captured.scratch.stage_peak_bytes();
    let bound = stats.scratch.declared();
    if diagnostics.incidence_scratch_peak_bytes > bound {
        return Err(resource(format!(
            "HLOD incidence scratch {} bytes exceeded its declared bound {bound}",
            diagnostics.incidence_scratch_peak_bytes
        )));
    }
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
    // The walk's scratch: the incidence row stream (at most its walk phase) beside every
    // component a downstream slice admits before allocating: S2's source index, admitted
    // before its first write and held through the walk.
    let ledger = scratch::ScratchLedger::new(
        &captured.scratch,
        diagnostics.incidence_scratch.walk,
        &admit,
    );
    let border_lease = ledger.admit(
        borders::Borders::COMPONENT,
        borders::scratch_need(
            stats.rows,
            stats.shared_points,
            stats.shared_segments,
            captured.tree.stats.nodes,
            captured.tree.stats.leaves,
            captured.tree.stats.height - 1,
            input.config.sorter(1),
        )
        .ok_or_else(|| resource("shared-border scratch bound overflows"))?,
    )?;
    let borders = borders::Borders::build(
        &captured.scratch,
        input.config.sorter(1),
        &captured.tree.nodes,
        rows,
        source_open_ends.reader()?,
        captured.tree.stats.per_depth[0].maximum_extent,
        &border_lease,
        check,
    )?;
    drop(source_open_ends);
    report.borders_by_depth = (0..captured.tree.stats.height)
        .map(|depth| borders::DepthReceipt {
            depth,
            scale: borders.scale(depth),
            ..Default::default()
        })
        .collect();
    let index_start = resources::process_cpu_seconds();
    let index_lease = ledger.admit(
        source_index::SourceIndex::COMPONENT,
        source_index::scratch_need(captured.triangles.len())
            .ok_or_else(|| resource("source index scratch overflows"))?,
    )?;
    let source = source_index::SourceIndex::build(captured, &index_lease, check)?;
    let sampling = error::SamplingPlan::new(captured, settings.sampling, &|first, end, depth| {
        borders.can_collapse(first, end, depth)
    })?;
    let index_cpu = resources::process_cpu_seconds()
        .zip(index_start)
        .map(|(b, a)| b - a);
    // S3: the shared decoded source-chain cache and what every appearance job reads.
    let cache = bake::ImageCache::new(input.plan.image_cache_bytes);
    let layout = captured.triangles.layout().0;
    let services = appearance::Services {
        captured,
        cache: &cache,
        profile: input.config.profile,
        max_edge: appearance::MAX_PAGE_EDGE,
        page_memory: input.plan.page_bytes,
        tile_page_bytes: input.plan.tile_page_bytes,
        colors: layout & super::attributes::COLOR != 0,
        normals: layout & super::attributes::NORMAL != 0,
        check,
    };
    let audit_bound = borders
        .audit_bound(
            captured
                .triangles
                .len()
                .checked_mul(u64::from(captured.tree.stats.height))
                .ok_or_else(|| resource("audit triangle-depth bound overflows"))?,
        )
        .ok_or_else(|| resource("audit bound overflows"))?;
    let audit_bound = if let Some(path) = &settings.audit_dir {
        audit::admitted_bound(audit_bound, audit::memory_backed(path)?)
    } else {
        audit_bound
    };
    let audit_lease = settings
        .audit_dir
        .as_ref()
        .map(|_| ledger.admit_external("s1b-audit", audit_bound))
        .transpose()?;
    let audit = settings
        .audit_dir
        .as_ref()
        .zip(audit_lease.as_ref())
        .map(|(path, lease)| audit::Writer::new(path, audit_bound, lease))
        .transpose()?;
    let job = Job {
        input: &input,
        services: &services,
        source: &source,
        borders: &borders,
        sampling: &sampling,
        check,
    };
    let result = walk(
        &job,
        borders.leaf_rows(),
        &rows_path,
        &mut report,
        audit.as_ref(),
        &ledger,
    );
    diagnostics.error_cpu_seconds = index_cpu.map(|s| s + report.error_cpu_seconds);
    diagnostics.error_cpu = report.error_cpu;
    diagnostics.error_cpu_by_depth = report.error_cpu_by_depth.clone();
    diagnostics.error_cpu.index = index_cpu.unwrap_or(0.);
    let (lease_peak, timings, held_spill) = result?;
    diagnostics.held_spill = held_spill;
    diagnostics.appearance_seconds = timings;
    diagnostics.image_cache = cache.stats();
    diagnostics.audit = audit.as_ref().map(audit::Writer::finish).transpose()?;
    report.audit = diagnostics.audit.as_ref().map(audit::Summary::from);
    drop(audit);
    drop(audit_lease);
    ledger.check()?;
    drop(source);
    drop(index_lease);
    drop(borders);
    drop(border_lease);
    diagnostics.scratch_components = ledger.components();
    diagnostics.stage_scratch_peak_bytes = captured.scratch.stage_peak_bytes();
    let stage_bound = diagnostics
        .incidence_scratch
        .declared()
        .max(ledger.peak_bound());
    diagnostics.stage_scratch_bound_bytes = stage_bound;
    if diagnostics.stage_scratch_peak_bytes > stage_bound {
        return Err(resource(format!(
            "HLOD stage scratch {} bytes exceeded its declared bound {stage_bound} \
             (incidence {:?}, components {:?})",
            diagnostics.stage_scratch_peak_bytes,
            diagnostics.incidence_scratch,
            diagnostics.scratch_components
        )));
    }
    input
        .phases
        .finished(Phase::HlodGeometry, walk_started.elapsed().as_secs_f64())?;
    diagnostics.error_measurement = report.error_measurement.clone();
    diagnostics.tiles = report.tiles;
    diagnostics.omitted_proxies = report.omitted_proxies;
    diagnostics.omission_sources = report.omission_sources;
    diagnostics.omitted_by_depth = report.omitted_by_depth.clone();
    diagnostics.borders_by_depth = report.borders_by_depth.clone();
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
    rows: std::iter::Peekable<crate::spill::SequenceReader<'a, InterfaceRow>>,
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
            last.absorb(&row);
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
    shrink(&mut tile);
    Ok(tile)
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

/// Memory-backed scratch charged to memory by the HLOD plan: capture's declared need, raised
/// to the incidence pass's worst case beside capture's retained files and S2's source index
/// (staged admission cannot take memory nobody charged).
fn memory_backed_need(
    captured: &FullCaptured,
    config: &TreeConfig,
    cap: usize,
) -> Result<u64, JobError> {
    let need = &captured.report.scratch;
    let n = captured.triangles.len();
    interfaces::memory_backed_need(n, config.sorter(1))
        .and_then(|b| b.checked_add(need.stages.leaf_plans))
        // S2's source index, admitted at the walk's start beside the incidence rows.
        .and_then(|b| b.checked_add(source_index::scratch_need(n)?))
        .and_then(|b| {
            b.checked_add(borders::scratch_need(
                n.checked_mul(6)?,
                n.checked_mul(3)?,
                n.checked_mul(3)?,
                captured.tree.stats.nodes,
                captured.tree.stats.leaves,
                captured.tree.stats.height - 1,
                config.sorter(1),
            )?)
        })
        .and_then(|b| b.checked_add(held_spill_bound(captured, config, cap)?))
        .map(|b| b.max(need.need_bytes))
        .ok_or_else(|| resource("HLOD memory-backed scratch need overflows"))
}

fn held_spill_bound(captured: &FullCaptured, config: &TreeConfig, cap: usize) -> Option<u64> {
    store::spill::bound(
        captured.triangles.len(),
        captured.tree.stats.nodes,
        config.leaf_triangles(),
        cap as u64,
        captured.tree.stats.height,
        captured.scratch.allocation_unit_bytes(),
    )
}

/// Staged admission of one incidence phase declared at `bound` bytes (live files included):
/// free space for what is not yet written plus per-file rounding (capture's live files and
/// the phase's chunk files), as capture admits its own stages. Memory-backed scratch was
/// charged to memory at capture for its declared need, so a phase past that need is refused
/// rather than taken from memory nobody charged.
fn admit_scratch(
    captured: &FullCaptured,
    config: &TreeConfig,
    cap: usize,
    bound: u64,
) -> Result<(), JobError> {
    let need = &captured.report.scratch;
    let scratch = &captured.scratch;
    if scratch.memory_backed() {
        let charged = memory_backed_need(captured, config, cap)?;
        if bound > charged {
            return Err(resource(format!(
                "HLOD incidence needs {bound} bytes of memory-backed scratch, past the \
                 {charged} charged to memory"
            )));
        }
    }
    let files = bound.div_ceil((crate::spill::CHUNK_BYTES / 2) as u64);
    let total = config::file_rounding(scratch.allocation_unit_bytes(), files)
        .and_then(|r| r.checked_add(need.file_rounding_bytes))
        .and_then(|r| r.checked_add(bound))
        .ok_or_else(|| resource("HLOD incidence scratch admission overflows"))?;
    crate::spill::preflight(
        scratch.path(),
        total.saturating_sub(scratch.live_bytes()),
        crate::spill::SpaceRole::Scratch,
        crate::spill::PreflightStage::Lod,
    )?;
    Ok(())
}

/// What every parent job of the walk shares.
struct Job<'j, 'a> {
    input: &'j HlodInput<'a>,
    /// S3's appearance services: the image cache, profile and page bounds.
    services: &'j appearance::Services<'j>,
    /// S2's source index: bounded queries of each tile's descendant source range.
    source: &'j source_index::SourceIndex,
    borders: &'j borders::Borders,
    sampling: &'j error::SamplingPlan,
    check: &'j (dyn Fn() -> Result<(), JobError> + Sync),
}

struct Prepared {
    positions: super::leaves::geometry::CoarsePositions,
    error: store::TileError,
    measurement: error::Summary,
    error_cpu: error::CpuReport,
    error_cpu_seconds: f64,
}

/// Final decode, topology eligibility, D4 and S2, before any appearance work.
fn prepare(
    job: &Job<'_, '_>,
    built: &mut geometry::Built,
    contract: &borders::Contract,
    query: &mut source_index::SourceQuery<'_>,
    nearest: &mut error::Nearest,
) -> Result<Option<Prepared>, JobError> {
    let (captured, check) = (job.input.captured, job.check);
    let cpu_start = error::thread_cpu();
    // Keep S1's open-border vertices at their leaf-canonical decode. Prepare the final
    // positions once: representatives and measurement both consume this publication.
    let border = encode::open_border(&built.geometry.triangles, built.geometry.positions.len());
    let fixed: Vec<bool> = built
        .locked
        .iter()
        .zip(&border)
        .map(|(l, b)| *l || *b)
        .collect();
    let mut positions = captured.lattice.encode_coarse(
        &built.geometry.positions,
        &built.geometry.ells,
        &fixed,
        built.geometry.key.depth,
    )?;
    // Coarse snapping can collapse an otherwise valid thin surface onto itself.
    // Preserve its existing canonical points in that case; do not delete a twin
    // after decoding or hide the resulting fold by changing winding.
    if !geometry::folds::decode_valid(
        &built.geometry.positions,
        &positions.decoded,
        &built.geometry.triangles,
    ) {
        positions = captured.lattice.encode_coarse(
            &built.geometry.positions,
            &built.geometry.ells,
            &vec![true; fixed.len()],
            built.geometry.key.depth,
        )?;
        if !geometry::folds::decode_valid(
            &built.geometry.positions,
            &positions.decoded,
            &built.geometry.triangles,
        ) {
            return Ok(None);
        }
    }
    let Some(controls) = geometry::measurement::assess(
        &mut built.change,
        &positions.decoded,
        &built.geometry.triangles,
        contract.scale,
        contract.scale.max(built.children_error),
        &mut |p, position| {
            let hit = nearest.source(query, p)?;
            if !position {
                return Ok((p, hit.distance2.sqrt()));
            }
            let t = query
                .triangle(hit)?
                .triangle
                .positions
                .map(|p| p.map(f64::from));
            Ok((source_index::bary(t, hit.bary), hit.distance2.sqrt()))
        },
        check,
    )?
    else {
        return Ok(None);
    };
    built.controls = controls;
    // D4 identity on the final decoded centroid, in the descendant source range (bounded
    // nearest search). Charting/encoding must preserve these positions and rows in S3.
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
        let hit = nearest.source(query, p)?;
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
    // The error is measured on the final decoded positions before the appearance bake,
    // which needs G for its texel spacing. Charting never moves a position, and the encoded
    // bytes are checked below to decode to exactly these position bits, so this is S2's
    // measurement of the published tile.
    let count = job.sampling.count(tile.key)?;
    let cpu_start = error::thread_cpu();
    let (mut error, measurement, mut error_cpu) = error::measure(
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
        query,
        nearest,
        count,
        &centroid_checks,
        &mut built.controls,
        check,
    )?;
    // Check the actual final S2 patch samples too, including codec displacement and
    // centroid controls. A rejected closure gets a bounded boundary-preserving retry.
    built.change.receipt.opened_hole_to_proxy = built
        .change
        .receipt
        .opened_hole_to_proxy
        .max(built.controls.opening_source_max);
    built.change.receipt.opened_hole_boundary_to_source = built
        .change
        .receipt
        .opened_hole_boundary_to_source
        .max(built.controls.opening_proxy_max);
    if !built.controls.eligible(contract.scale) {
        return Ok(None);
    }
    built.change.receipt.hole_patch_to_source = built
        .change
        .receipt
        .hole_patch_to_source
        .max(built.controls.closure_max);
    error.geometric_error = error.geometric_error.max(contract.scale);
    built.change.receipt.published(error.geometric_error);
    built.topology = built.change.receipt;
    error_cpu.representatives = identity_cpu.unwrap_or(0.);
    let error_cpu_seconds = error::thread_cpu()
        .zip(cpu_start)
        .zip(identity_cpu)
        .map_or(0., |((b, a), i)| b - a + i);
    Ok(Some(Prepared {
        positions,
        error,
        measurement,
        error_cpu,
        error_cpu_seconds,
    }))
}

/// One parent: geometry, error, appearance, encoding, the pre-publication check and its
/// member. Topology eligibility may query source geometry for a rejected proposal.
/// An omitted parent runs no appearance work and publishes no S2 measurement.
fn build_tile(
    job: &Job<'_, '_>,
    closed: &Closed,
    children: Vec<TileGeometry>,
    audit: Option<&audit::Writer<'_>>,
) -> Done {
    let (captured, settings, check) = (job.input.captured, &job.input.settings, job.check);
    let contract = job
        .borders
        .contract(closed.key, 6 * settings.cap)?
        .filter(|c| c.minimum_triangles() <= settings.cap);
    let mut prepared = None;
    let mut topology_attempts = 0;
    let mut published_topology_attempt = None;
    let mut query = job.source.query(captured, closed.key)?;
    let mut nearest = error::Nearest::with_steps(settings.nearest_steps);
    let outcome = if let Some(contract) = &contract {
        let half = if closed.single_leaf {
            children[0].triangles.len().div_ceil(2).min(settings.cap)
        } else if closed.key.depth <= 3 {
            // D11 scale-gate budget: reserve payload room for appearance at the first
            // four depths. A failed target still gets the unchanged hard-cap attempt.
            (settings.cap * 3 / 4).max(1)
        } else {
            settings.cap
        };
        geometry::build_parent(
            closed.key,
            &children,
            &[half, settings.cap],
            contract,
            &|k| job.borders.mapping(closed.key.depth, k),
            &mut |built| {
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
                prepared = prepare(job, built, contract, &mut query, &mut nearest)?;
                let accepted = prepared.is_some();
                if let Some(audit) = audit {
                    topology_attempts += 1;
                    let index = audit.topology_attempt(built, topology_attempts, accepted)?;
                    if accepted {
                        published_topology_attempt = Some(index);
                    }
                }
                Ok(accepted)
            },
            check,
        )?
    } else {
        geometry::Outcome {
            built: None,
            attempted_topology: Default::default(),
            construction_dropped: Vec::new(),
        }
    };
    let Some(mut built) = outcome.built else {
        // Its interface cannot fit the cap: omitted, nothing held for its parent.
        let construction_topology = audit
            .filter(|_| !outcome.construction_dropped.is_empty())
            .map(|a| a.construction(closed.key, &outcome.construction_dropped))
            .transpose()?;
        let omitted = Finished::Omitted {
            key: closed.key,
            reason: if contract.is_some() {
                OmissionReason::RemeshStalled
            } else {
                OmissionReason::ContractLowerBound
            },
            frontier: children
                .iter()
                .map(|c| {
                    Ok(TileKey {
                        depth: job.borders.publication_depth(c.key)?,
                        ..c.key
                    })
                })
                .collect::<Result<_, JobError>>()?,
            topology: Box::new(outcome.attempted_topology),
            topology_attempts,
            construction_topology,
        };
        return Ok((None, omitted, appearance::Timing::default()));
    };
    drop(children);
    built.change = Default::default();
    built.controls = Default::default();
    let Prepared {
        positions,
        error,
        measurement,
        error_cpu,
        error_cpu_seconds,
    } = prepared.ok_or_else(|| state("accepted geometry has no final decoded measurement"))?;
    let tile = &built.geometry;
    let contract = contract.as_ref().expect("built tile has contract");
    // S3: fresh charts and a cheap source bake on the final decoded positions, through
    // the same bounded source query.
    let appearance = appearance::build(
        job.services,
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
    let encoded = encode::encode(
        tile,
        &built.locked,
        positions,
        &appearance,
        settings.mutation,
    )?;
    drop(appearance.groups);
    if settings.mutation != Mutation::MoveVertexUnchecked {
        // Every decoded corner carries exactly the measured (and charted) position bits.
        encode::check(tile, &built.locked, &encoded)?;
    } else {
        decode::decode(&encoded.glb)?;
    }
    if !error.geometric_error.is_finite() || error.geometric_error < built.children_error {
        return Err(state("coarse error is nonfinite or below its children's"));
    }
    let uri = tile.key.uri();
    let path = job.input.stage.join(&uri);
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
        nearest_search: nearest.stats,
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
        reoriented_faces: built.reoriented,
        quantized: encoded.quantized,
        bounds: encoded.bounds.into(),
        appearance: appearance.facts,
        border_scale: contract.scale,
        border_epsilon: contract
            .certificates
            .iter()
            .map(|c| c.epsilon)
            .fold(0., f64::max),
        topology: built.topology,
        attempted_topology: outcome.attempted_topology,
        terminal_locks: contract.terminal_locks(),
        audit_record: None,
        topology_attempts,
        published_topology_attempt,
    };
    let mut geometry = built.geometry;
    geometry.geometric_error = error.geometric_error;
    shrink(&mut geometry);
    Ok((
        Some(geometry),
        Finished::Row(Box::new(row)),
        appearance.timing,
    ))
}

fn tally(report: &mut HlodReport, row: &HlodRow) {
    let borders = &mut report.borders_by_depth[row.depth as usize];
    borders.tiles += 1;
    borders.segments += u64::from(row.interface_segments);
    borders.terminal_locks += row.terminal_locks;
    borders.max_epsilon = borders.max_epsilon.max(row.border_epsilon);
    borders.topology.add(row.topology);
    borders.attempted_topology.add(row.attempted_topology);
    report.error_cpu_seconds += row.error_cpu_seconds;
    report.error_cpu.add(row.error_cpu);
    report.error_measurement.nearest.add(row.nearest_search);
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
    report.reoriented_faces += u64::from(row.reoriented_faces);

    report.interface_segments += u64::from(row.interface_segments);
    report.interface_vertices += u64::from(row.interface_vertices);
    if row.quantized {
        report.quantized_tiles += 1;
    } else {
        report.float_tiles += 1;
    }
    report.bytes += row.bytes;
    // S2's summaries are keyed by encoded depth (seeds and schedules); a tile published at
    // the top of its pass-through chain keeps them.
    let encoded = row.encoded_depth as usize;
    let by_depth = &mut report.error_measurement.by_depth;
    if by_depth.len() <= encoded {
        by_depth.resize_with(encoded + 1, Default::default);
    }
    by_depth[encoded].add(row.measurement, row.geometric_error, row.nearest_search);
    let cpu = &mut report.error_cpu_by_depth;
    if cpu.len() <= encoded {
        cpu.resize(encoded + 1, 0.);
    }
    cpu[encoded] += row.error_cpu_seconds;
    if row.depth == 0 {
        report.error_measurement.root = Some(row.measurement);
    }
    let depth = row.depth as usize;
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
    d.png_pages += u64::from(a.png_pages);
    d.donor_budget_exhausted += a.donor_budget_exhausted;
    d.donor_seeded += a.donor_seeded;
    d.donor_search_steps_max = d.donor_search_steps_max.max(a.donor_search_steps_max);
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

/// The preorder walk; returns the ledger's lease peak and the summed appearance timing.
fn walk(
    job: &Job<'_, '_>,
    rows: &crate::spill::Sequence<InterfaceRow>,
    rows_path: &Path,
    report: &mut HlodReport,
    audit: Option<&audit::Writer<'_>>,
    scratch_ledger: &scratch::ScratchLedger<'_>,
) -> Result<(u64, appearance::Timing, store::spill::Stats), JobError> {
    let (input, check) = (job.input, job.check);
    let captured = input.captured;
    let plan = input.plan;
    let settings = input.settings;
    let ledger = Ledger::new(plan.ledger_bytes);
    let job_bytes = plan.job_bytes;
    // The most a job merges: eight capped children (an omitted child is never merged).
    let job_inputs = input
        .config
        .leaf_triangles()
        .max(settings.cap as u64)
        .saturating_mul(8);
    let out_bytes = TileGeometry::charge(settings.cap as u64);
    let job_peak = std::cell::Cell::new(0u64);
    let mut timings = appearance::Timing::default();
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
        rows: rows.reader()?.peekable(),
    };
    let spill = std::cell::RefCell::new(store::spill::Manager::new(
        &captured.scratch,
        scratch_ledger,
        held_spill_bound(captured, input.config, settings.cap)
            .ok_or_else(|| resource("held spill bound overflows"))?,
    ));
    let ready_limit = store::spill::limit(captured.tree.stats.height);
    let total = captured.tree.stats.internal;
    input.phases.progress(Phase::HlodGeometry, 0, Some(total))?;
    let mut passed = 0u64;
    let outcome = pool.in_place_scope(|scope| -> Result<(), JobError> {
        let (sender, receiver) = mpsc::channel::<(u64, u64, Done)>();
        let mut stack: Vec<Frame> = Vec::new();
        // Finished children: their geometry, or none when omitted.
        let mut ready: HashMap<u64, Option<store::spill::Held>> = HashMap::new();
        let mut pending: Vec<Closed> = Vec::new();
        let mut finished_rows: BTreeMap<u64, Finished> = BTreeMap::new();
        let (mut next_id, mut next_seq, mut written, mut inflight) = (0u64, 0u64, 0u64, 0usize);
        let mut leaf = 0u32;
        let stage = input.stage;
        // Spawn every pending parent whose children are ready, in postorder, while workers
        // and the ledger admit its job.
        let spawn = |pending: &mut Vec<Closed>,
                     ready: &mut HashMap<u64, Option<store::spill::Held>>,
                     inflight: &mut usize|
         -> Result<(), JobError> {
            let mut i = 0;
            while i < pending.len() && *inflight < jobs {
                if !pending[i].children.iter().all(|c| ready.contains_key(c)) {
                    i += 1;
                    continue;
                }
                // An omitted child omits this cell too, before any merge: nothing to build,
                // its other children's geometry released (omission reaches the root).
                if pending[i].children.iter().any(|c| ready[c].is_none()) {
                    let closed = pending.remove(i);
                    let mut frontier = Vec::new();
                    for c in &closed.children {
                        if let Some(geometry) = ready.remove(c).flatten() {
                            frontier.push(TileKey {
                                depth: job.borders.publication_depth(geometry.key())?,
                                ..geometry.key()
                            });
                        }
                    }
                    *inflight += 1;
                    let omitted = Finished::Omitted {
                        key: closed.key,
                        reason: OmissionReason::DescendantOmission,
                        frontier,
                        topology: Default::default(),
                        topology_attempts: 0,
                        construction_topology: None,
                    };
                    let _ = sender.send((
                        closed.id,
                        closed.seq,
                        Ok((None, omitted, appearance::Timing::default())),
                    ));
                    continue;
                }
                // Children are capped (an omitted child propagates instead of merging), so
                // every job is within its planned charge.
                let merged: u64 = pending[i]
                    .children
                    .iter()
                    .map(|c| ready[c].as_ref().map_or(0, |g| g.triangles()))
                    .sum();
                if merged > job_inputs {
                    return Err(state(format!(
                        "a parent job merges {merged} triangles, past its planned {job_inputs}"
                    )));
                }
                job_peak.set(job_peak.get().max(merged));
                let job_lease = loop {
                    if let Some(lease) = ledger.try_lease(job_bytes, 0) {
                        break Some(lease);
                    }
                    if !spill.borrow_mut().spill_one(ready)? {
                        break None;
                    }
                };
                let Some(job_lease) = job_lease else {
                    break;
                };
                let closed = pending.remove(i);
                let children: Vec<TileGeometry> = closed
                    .children
                    .iter()
                    .map(|c| ready.remove(c).flatten().unwrap().load())
                    .collect::<Result<_, JobError>>()?;
                let sender = sender.clone();
                *inflight += 1;
                scope.spawn(move |_| {
                    let (id, seq) = (closed.id, closed.seq);
                    let done = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                        let mut built = build_tile(job, &closed, children, audit)?;
                        // The job's admission includes its output: keep that part while the
                        // tile is held for its parent, release the workspace.
                        if let Some(tile) = built.0.as_mut() {
                            let mut lease = job_lease;
                            let held = tile.check_charge()?;
                            if held > out_bytes {
                                return Err(state("a coarse tile exceeds its admitted output"));
                            }
                            lease.shrink(held);
                            tile.lease = Some(lease);
                        }
                        Ok(built)
                    }))
                    .unwrap_or_else(|_| Err(state("HLOD worker panicked")));
                    let _ = sender.send((id, seq, done));
                });
            }
            Ok(())
        };
        let mut receive = |pending: &mut Vec<Closed>,
                           ready: &mut HashMap<u64, Option<store::spill::Held>>,
                           inflight: &mut usize,
                           finished_rows: &mut BTreeMap<u64, Finished>|
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
            ready.insert(id, geometry.map(store::spill::Held::Resident));
            #[cfg(test)]
            if settings.mutation == Mutation::ForceSpill {
                while spill.borrow_mut().spill_one(ready)? {}
            }
            finished_rows.insert(seq, row);
            spawn(pending, ready, inflight)
        };
        // Write finished rows in postorder once their published depth is final.
        let mut flush = |finished_rows: &mut BTreeMap<u64, Finished>,
                         publish: &mut HashMap<u64, Publish>,
                         written: &mut u64|
         -> Result<(), JobError> {
            while publish.get(written).is_some_and(|p| p.done)
                && finished_rows.contains_key(written)
            {
                let finished = finished_rows.remove(written).unwrap();
                let Publish { depth, chain, .. } = publish.remove(written).unwrap();
                let mut row = match finished {
                    Finished::Row(row) => *row,
                    Finished::Omitted {
                        key,
                        reason,
                        frontier,
                        topology,
                        topology_attempts,
                        construction_topology,
                    } => {
                        let source = reason.source();
                        report.borders_by_depth[key.depth as usize]
                            .attempted_topology
                            .add(*topology);
                        report.borders_by_depth[key.depth as usize].frontier_tiles +=
                            frontier.len() as u64;
                        // The cell and every pass-through cell above it publish no content.
                        report.omission_sources += u64::from(source);
                        for d in std::iter::once(key.depth).chain(chain) {
                            if let Some(audit) = audit {
                                audit.omitted(
                                    TileKey { depth: d, ..key },
                                    if d == key.depth {
                                        reason
                                    } else {
                                        OmissionReason::DescendantOmission
                                    },
                                    if d == key.depth { &frontier } else { &[] },
                                    if d == key.depth {
                                        *topology
                                    } else {
                                        Default::default()
                                    },
                                    if d == key.depth { topology_attempts } else { 0 },
                                    if d == key.depth {
                                        construction_topology
                                    } else {
                                        None
                                    },
                                )?;
                            }
                            report.omitted_proxies += 1;
                            let d = d as usize;
                            report.omitted_by_depth[d] += 1;
                            report.borders_by_depth[d].omitted_cells += 1;
                            report.borders_by_depth[d].omission_sources +=
                                u64::from(source && d == key.depth as usize);
                        }
                        *written += 1;
                        continue;
                    }
                };
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
                // Publication may collapse a single-child chain. Resolve its contract at
                // the published depth even when durable audit output is disabled.
                let key = TileKey {
                    first: row.first,
                    end: row.end,
                    depth: row.depth,
                    leaf_begin: row.leaf_begin,
                    leaf_end: row.leaf_end,
                };
                let contract = job
                    .borders
                    .contract(key, 6 * job.input.settings.cap)?
                    .ok_or_else(|| state("published tile has no bounded contract"))?;
                row.border_epsilon = contract
                    .certificates
                    .iter()
                    .map(|c| c.epsilon)
                    .fold(0., f64::max);
                row.terminal_locks = contract.terminal_locks();
                if let Some(audit) = audit {
                    row.audit_record = Some(audit.tile(&row, &contract)?);
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
                let border_chain = !matches!(kinds.get(only), Some(Child::Coarse(_)))
                    || job.borders.can_collapse(
                        frame.node.first,
                        frame.node.end,
                        frame.node.depth(),
                    )?;
                let only = *only;
                if let Some(Child::Coarse(seq)) = kinds.get(&only).copied().filter(|_| border_chain)
                {
                    let p = publish
                        .get_mut(&seq)
                        .ok_or_else(|| state("a coarse child without its row"))?;
                    p.depth = frame.node.depth();
                    p.chain.push(p.depth);
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
                    chain: Vec::new(),
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
                while (finished_rows.len() >= ROW_WINDOW
                    || ready.len() + pending.len() + inflight >= ready_limit)
                    && inflight > 0
                {
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
                    if inflight == 0 && spill.borrow_mut().spill_one(&mut ready)? {
                        continue;
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
                        ready.insert(id, Some(store::spill::Held::Resident(tile)));
                        #[cfg(test)]
                        if settings.mutation == Mutation::ForceSpill {
                            while spill.borrow_mut().spill_one(&mut ready)? {}
                        }
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
    report.max_job_input_triangles = job_peak.get();
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
    Ok((ledger.peak(), timings, spill.into_inner().stats()))
}
