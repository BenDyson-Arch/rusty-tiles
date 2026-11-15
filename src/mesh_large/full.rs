//! Full-detail snapshot owner. Source reads end here; later leaves read only owned
//! spills and admitted material/texture/sampler metadata. No tile output yet.
use super::{
    assembly::{self, AssemblyPlan, AssemblyReport},
    attributes::{self, FullTriangle},
    capture::{self, SourceScan},
    capture_sort::{self, SortLoad, SortPlan},
    config::{self, JobsPlan, MemoryBound, TreeConfig},
    images::{self, ImageCaptureBounds, ImageSnapshot},
    input::StreamSource,
    leaves::{
        atlas::AtlasResources,
        geometry::LatticePlan,
        planning::{self, LeafPlan},
    },
    phases::{Step, StepProgress},
    source_crs::{MeshSourceCrsReport, SourceFrame},
    tree::{self, RetainedTree},
};
use crate::{
    gltf_source::source::{self, ImageInfo, Material},
    spill::{ExternalSorter, Record, Scratch, Sequence},
    JobError, JobErrorKind, JobFailure,
};
use serde::Serialize;
use serde_json::Value;
use std::{mem::size_of, path::Path};

const DECODER_ALLOWANCE: u64 = 192 << 20;
const PLANNING_ALLOWANCE: u64 = 128 << 20;
const SORT_THREAD_BYTES: u64 = 8_000_000;

/// Scratch bytes live at each capture stage's peak, from declared counts and the sorter's file
/// lifetimes ([`config::sort_spill`]). Each figure includes the files retained from earlier
/// stages. As a measurement (`scratch_stage_peaks`) the same fields hold observed peaks;
/// `hlod_incidence` is then 0 because the HLOD stage runs after capture.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize)]
pub struct ScratchStages {
    /// The direct assembly: decoded vertices that spill, the staged runs and the retained
    /// sequence it writes (`assembly`).
    pub capture: u64,
    /// The retained full-triangle sequence (the field keeps its name: capture no longer
    /// writes a separate rank-ordered projection).
    pub projection: u64,
    pub images: u64,
    /// The cell tree and node sort over the retained sequence.
    pub tree: u64,
    pub corner_depths: u64,
    pub leaf_plans: u64,
    /// Everything retained for leaves plus the HLOD stage's declared incidence and source-index
    /// need.
    pub hlod_incidence: u64,
}
impl ScratchStages {
    fn all(&self) -> [u64; 7] {
        [
            self.capture,
            self.projection,
            self.images,
            self.tree,
            self.corner_depths,
            self.leaf_plans,
            self.hlod_incidence,
        ]
    }
}
impl FullScratchNeed {
    /// What every admission of `self` checks is at most `floor`'s: the whole need, and each
    /// stage's entry recheck (its bound plus per-file rounding).
    fn within(&self, floor: &Self) -> bool {
        let entries = |need: &Self| {
            need.stages
                .all()
                .map(|bound| bound.saturating_add(need.file_rounding_bytes))
        };
        self.need_bytes <= floor.need_bytes
            && entries(self)
                .iter()
                .zip(entries(floor))
                .all(|(grown, floor)| *grown <= floor)
    }
}
impl ScratchStages {
    fn max(&self) -> u64 {
        [
            self.capture,
            self.projection,
            self.images,
            self.tree,
            self.corner_depths,
            self.leaf_plans,
            self.hlod_incidence,
        ]
        .into_iter()
        .max()
        .unwrap_or(0)
    }
}

/// Upfront scratch admission: the largest declared stage working set, a 10% model margin and
/// per-file allocation rounding. Stage entries recheck their own working set.
#[derive(Clone, Debug, Serialize)]
pub(crate) struct FullScratchNeed {
    pub stages: ScratchStages,
    pub image_bytes: u64,
    pub max_live_files: u64,
    /// Largest declared stage working set (logical spill bytes).
    pub logical_need_bytes: u64,
    /// The 10% model margin plus per-file allocation rounding and directory metadata.
    pub allocation_reserve_bytes: u64,
    /// Per-file rounding alone, added to each stage-entry recheck.
    pub file_rounding_bytes: u64,
    pub need_bytes: u64,
    /// Output-side admission reservation (leaf staging, archive staging and a replaced
    /// destination), checked on the filesystems holding the work directory and output.
    pub output_reserve_bytes: u64,
}
#[derive(Clone, Debug, Serialize)]
pub(crate) struct FullMemoryBound {
    pub geometry: MemoryBound,
    pub owned_metadata_bytes: u64,
    pub decoder_allowance_bytes: u64,
    pub planning_allowance_bytes: u64,
    /// Descriptors of live spill chunk files (grows with spilled bytes).
    pub chunk_descriptor_bytes: u64,
    /// Block indexes of the compressed retained sequences (full triangles, corner depths).
    pub block_index_bytes: u64,
    /// The corner-depth join's chunk tasks beside its sorts (`corners::wave_bytes`).
    pub corner_join_bytes: u64,
    /// The larger of the scan's used-vertex bitmap (one primitive's at a time) and the
    /// parallel validation's task lists; they never coexist.
    pub preflight_bytes: u64,
    /// The direct assembly's working set (`assembly::AssemblyPlan::memory_bytes`). Capture's
    /// sorts run after it, so the bound charges the larger of it and their allowance.
    pub assembly_bytes: u64,
    pub bound_bytes: u64,
}
impl FullMemoryBound {
    fn base_bytes(&self) -> u64 {
        self.bound_bytes - self.geometry.jobs * self.geometry.per_job_bytes
    }
}
#[derive(Clone, Debug, Serialize)]
pub(crate) struct FullCaptureReport {
    pub source_bytes: u64,
    pub external_files: u64,
    pub external_bytes: u64,
    pub json_bytes: u64,
    pub scan: SourceScan,
    pub jobs: JobsPlan,
    pub memory: FullMemoryBound,
    /// The sorts inside capture, as planned under the budget.
    pub sort: SortPlan,
    /// The direct assembly of the retained sequence: its plan, vertex stores and buckets.
    pub assembly: AssemblyReport,
    pub scratch: FullScratchNeed,
    pub image_capture: ImageCaptureBounds,
    pub scratch_logical_peak_bytes: u64,
    /// Measured peak live scratch of each capture stage (the declared bounds are
    /// `scratch.stages`); `hlod_incidence` is 0 here.
    pub scratch_stage_peaks: ScratchStages,
    pub scratch_memory_backed: bool,
    /// Local encoding contract version: dataset-wide zero origin/aligned lattice.
    pub lattice_version: u32,
    /// The resolved source CRS; absent for local sources.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_crs: Option<MeshSourceCrsReport>,
}

pub(crate) struct FullCaptured {
    pub tree: RetainedTree,
    pub triangles: Sequence<FullTriangle>,
    pub corner_depths: Sequence<CornerDepth>,
    pub leaf_plans: Sequence<LeafPlan>,
    pub images: Vec<ImageSnapshot>,
    pub image_info: Vec<ImageInfo>,
    pub materials: Vec<Material>,
    /// Source material ID to its H28 batching class representative.
    pub material_classes: super::batching::MaterialClasses,
    pub textures: Vec<Value>,
    pub samplers: Vec<Value>,
    pub lattice: LatticePlan,
    pub scan: SourceScan,
    pub report: FullCaptureReport,
    /// Root placement derived from a source CRS (ENU at its origin to ECEF).
    pub placement: Option<crate::mesh_archive::ResolvedPlacement>,
    // Last: all owned spill handles are destroyed before the directory.
    pub scratch: Scratch,
}
impl FullCaptured {
    pub(crate) fn close(self) -> Result<(), JobError> {
        let Self {
            tree,
            triangles,
            corner_depths,
            leaf_plans,
            images,
            scratch,
            ..
        } = self;
        drop(tree);
        drop(triangles);
        drop(corner_depths);
        drop(leaf_plans);
        drop(images);
        scratch.close().map_err(Into::into)
    }
}
fn exhausted(message: &'static str) -> JobError {
    JobError::new(JobErrorKind::ResourceLimit, message)
}
fn invariant(message: &'static str) -> JobError {
    JobError::new(JobErrorKind::InvalidState, message)
}
/// Consecutive batches of image decodes whose summed charges fit `budget`,
/// at most `workers` each; one at a time when memory is not reported. An
/// image whose own charge exceeds the budget is a typed resource refusal.
fn decode_batches(
    charges: &[u64],
    budget: Option<u64>,
    workers: usize,
) -> Result<Vec<std::ops::Range<usize>>, JobError> {
    let mut batches = Vec::new();
    let mut start = 0;
    while start < charges.len() {
        if budget.is_some_and(|budget| charges[start] > budget) {
            return Err(exhausted(
                "decoding an admitted image does not fit available memory",
            ));
        }
        let mut end = start + 1;
        let mut leased = charges[start];
        if let Some(budget) = budget {
            while end < charges.len()
                && end - start < workers.max(1)
                && leased.saturating_add(charges[end]) <= budget
            {
                leased += charges[end];
                end += 1;
            }
        }
        batches.push(start..end);
        start = end;
    }
    Ok(batches)
}

fn add(a: u64, b: u64) -> Result<u64, JobError> {
    a.checked_add(b)
        .ok_or_else(|| exhausted("full capture byte arithmetic overflow"))
}
fn mul(a: u64, b: u64) -> Result<u64, JobError> {
    a.checked_mul(b)
        .ok_or_else(|| exhausted("full capture byte arithmetic overflow"))
}
/// One image decode's charge: the encoded image at its actual length, which decoding holds
/// whole, plus 12 bytes per texel (the decoder's admitted allocation of at most 8 bytes per
/// texel and the RGBA8 copy).
fn decode_charge(encoded: u64, texels: u64) -> Result<u64, JobError> {
    add(mul(texels, 12)?, encoded)
}
/// Declared images: their count and snapshot scratch bytes, each image's header
/// dimensions, and the largest single decode charge.
struct ImagePreflight {
    count: u64,
    scratch_bytes: u64,
    dimensions: Vec<(u32, u32)>,
    largest_encoded_bytes: u64,
    /// The largest decode charge, with its encoded length and dimensions.
    largest_decode: Option<(u64, u64, (u32, u32))>,
}
/// Admits every declared image's length and header (format, edge and texel ceilings) from
/// at most 64 KiB of it, so decode charges are known before capture writes any scratch.
fn image_preflight(
    source: &StreamSource,
    check: &mut dyn FnMut() -> Result<(), JobError>,
) -> Result<ImagePreflight, JobError> {
    let declared = source.document.image_sources().len();
    let mut preflight = ImagePreflight {
        count: u64::try_from(declared).map_err(|_| exhausted("image count overflow"))?,
        scratch_bytes: 0,
        dimensions: Vec::new(),
        largest_encoded_bytes: 0,
        largest_decode: None,
    };
    preflight
        .dimensions
        .try_reserve_exact(declared)
        .map_err(|_| exhausted("image preflight allocation failed"))?;
    for index in 0..declared {
        check()?;
        let length = source.image_len(index)?;
        images::admitted_length(length)?;
        let (width, height) = source::probe_header_dimensions(
            length,
            source.document.image_view(index)?.mime,
            source::DecodeAdmission::LARGE_MESH,
            &mut |offset, out| source.read_image_chunk(index, offset, out),
            check,
        )?;
        preflight.dimensions.push((width, height));
        let charge = decode_charge(length, u64::from(width) * u64::from(height))?;
        if preflight.largest_decode.is_none_or(|(c, _, _)| charge > c) {
            preflight.largest_decode = Some((charge, length, (width, height)));
        }
        preflight.largest_encoded_bytes = preflight.largest_encoded_bytes.max(length);
        let chunks = add(length, 4093)? / 4094;
        preflight.scratch_bytes = add(preflight.scratch_bytes, mul(chunks, 4096)?)?;
    }
    Ok(preflight)
}
/// Refuses, as a typed resource limit, a decode charge `budget` cannot hold.
fn admit_decode(budget: Option<u64>, preflight: &ImagePreflight) -> Result<(), JobError> {
    match (budget, preflight.largest_decode) {
        (Some(budget), Some((charge, encoded, (width, height)))) if charge > budget => {
            Err(JobError::new(
                JobErrorKind::ResourceLimit,
                format!(
                    "image capture: decoding a source image needs {charge} bytes ({encoded} \
                     encoded plus 12 per texel of {width}x{height}) but the memory budget \
                     leaves {budget} for image decoding"
                ),
            ))
        }
        _ => Ok(()),
    }
}
#[allow(clippy::too_many_arguments)]
/// Declared scratch of every capture stage. `inside` configures the sorts inside capture
/// (its [`capture_sort`](super::capture_sort) plan); `config` is the request, whose floor
/// configuration the HLOD stage's sorts use.
fn scratch_need(
    scan: SourceScan,
    layout: crate::spill::Layout,
    inside: &TreeConfig,
    config: &TreeConfig,
    unit: u64,
    image_count: u64,
    image_bytes: u64,
    assembly: assembly::ScratchShape,
) -> Result<FullScratchNeed, JobError> {
    let vertex_bytes = assembly.vertex_bytes;
    use super::{
        corners::{Candidate, RankDepth},
        records::{NodeRecord, Span},
    };
    let overflow = || exhausted("full capture scratch arithmetic overflow");
    let sorter = inside.sorter(1); // Run sizes do not depend on the worker count.
    let n = scan.triangles;
    let corners = mul(n, 3)?;
    // Capture's records spill under the run's attribute layout.
    let chunk = crate::spill::CHUNK_BYTES;
    let full =
        config::sort_spill_in::<FullTriangle>(n, sorter, chunk, layout).ok_or_else(overflow)?;
    let nodes = config::node_bound(n, scan.geometry_bytes, config);
    let node_sort = config::sort_spill::<NodeRecord>(nodes, sorter).ok_or_else(overflow)?;
    let node_bytes = mul(nodes, NodeRecord::SIZE as u64)?;
    let span_bytes = mul(nodes, Span::SIZE as u64)?;
    // At most every corner is a join candidate, and at most every candidate is patched.
    let candidates = config::sort_spill::<Candidate>(corners, sorter).ok_or_else(overflow)?;
    let patches = config::sort_spill::<RankDepth>(corners, sorter).ok_or_else(overflow)?;
    let corner_depth_bytes = mul(n, CornerDepth::SIZE as u64)?;
    // Every leaf holds at least one triangle and is a tree node.
    let leaf_plan_bytes = mul(n.min(nodes), LeafPlan::SIZE as u64)?;
    let max = |values: &[Option<u64>]| -> Result<u64, JobError> {
        values
            .iter()
            .try_fold(0, |m: u64, v| v.map(|v| m.max(v)))
            .ok_or_else(overflow)
    };
    let t = full.bytes;
    // Assembly, at raw size, assuming every primitive spills and is routed:
    // - routing: the decoded vertices, the placed corners written so far (at most every
    //   triangle's), and the two corner sorts of one primitive (at most every corner);
    // - distributing: the decoded vertices and placed corners, and the staged runs (every
    //   record);
    // - sorting: the staged runs not yet written plus the sequence written so far (one
    //   staging group, at most an eighth of the records, overlaps), plus the external sort of
    //   a bucket over the in-memory size, at worst every record beyond the staged copy its
    //   sort replaces.
    let by_vertex =
        config::sort_spill::<assembly::CornerVertex>(corners, sorter).ok_or_else(overflow)?;
    let placed_sort =
        config::sort_spill_in::<assembly::PlacedCorner>(corners, sorter, chunk, layout)
            .ok_or_else(overflow)?;
    let placed = mul(corners, assembly::vertex_record_bytes(layout))?;
    let stores = add(vertex_bytes, placed)?;
    let routing = max(&[
        Some(by_vertex.finish_peak),
        by_vertex.flow(placed_sort.push_peak),
        Some(placed_sort.finish_peak),
        placed_sort.flow(placed),
    ])?;
    // Between distributing and sorting: the staged runs, their listing and its sort.
    let run_list = assembly::run_list_bytes(n, layout).ok_or_else(overflow)?;
    let capture = max(&[
        Some(add(stores, routing)?),
        Some(add(stores, t)?),
        Some(add(t, mul(run_list, 3)?)?),
        add(add(t, t.div_ceil(8))?, full.finish_peak.saturating_sub(t)).ok(),
    ])?;
    // The retained sequence, written by the assembly.
    let projection_stage = t;
    let images = add(t, image_bytes)?;
    let retained = add(t, image_bytes)?;
    let tree = add(
        retained,
        max(&[
            node_sort.push_peak.checked_add(span_bytes),
            node_sort
                .flow(node_bytes)
                .and_then(|v| v.checked_add(span_bytes)),
        ])?,
    )?;
    let base = add(retained, node_bytes)?;
    let corner_depths = add(
        base,
        max(&[
            Some(candidates.finish_peak),
            candidates.flow(patches.push_peak),
            Some(patches.finish_peak),
            patches.flow(corner_depth_bytes),
        ])?,
    )?;
    let kept = base;
    let leaf_plans = add(add(kept, corner_depth_bytes)?, leaf_plan_bytes)?;
    // HLOD's interface incidence pass (hlod/interfaces.rs) admits its phases as they start
    // (it declares nothing here); S2's source index, always built beside the walk's rows, is
    // preflighted up front too and admitted again through the walk's ledger before its first
    // write.
    let hlod_incidence = add(
        leaf_plans,
        add(
            super::hlod::interfaces::scratch_need(n, config.sorter(1)).ok_or_else(overflow)?,
            super::hlod::source_index::scratch_need(n).ok_or_else(overflow)?,
        )?,
    )?;
    let stages = ScratchStages {
        capture,
        projection: projection_stage,
        images,
        tree,
        corner_depths,
        leaf_plans,
        hlod_incidence,
    };
    // H1 run inventory plus full-capture runs, image sequences and retained sequences, for
    // the larger of the capture and request configurations.
    let mut max_live_files = 0;
    for config in [inside, config] {
        let h1 = config::scratch_need_parts(n, scan.geometry_bytes, config, unit)
            .ok_or_else(overflow)?;
        let capacity = ((config.sort.run_bytes / 2) / size_of::<FullTriangle>()).max(1) as u64;
        let counted = add(mul(add(mul(n, 16)?.div_ceil(capacity), 5)?, 2)?, 32)?;
        let inventory = add(mul(config.sort.fan_in as u64, 256)?, 32)?;
        max_live_files = max_live_files.max(add(
            h1.max_live_files,
            add(add(inventory.min(counted), image_count)?, 5)?,
        )?);
    }
    // The assembly's spilled vertex stores and staging files.
    let max_live_files = add(max_live_files, assembly.files)?;
    let logical_need_bytes = stages.max();
    // Runs are chunk files of about CHUNK_BYTES each (at least half of it, for any record).
    let chunk_files = logical_need_bytes.div_ceil((crate::spill::CHUNK_BYTES / 2) as u64);
    let max_live_files = add(max_live_files, chunk_files)?;
    // HLOD's overlap join holds a bounded set of its own files besides its runs.
    let max_live_files = add(max_live_files, super::hlod::interfaces::INCIDENCE_MAX_FILES)?;
    let file_rounding_bytes = config::file_rounding(unit, max_live_files).ok_or_else(overflow)?;
    let allocation_reserve_bytes = add(logical_need_bytes.div_ceil(10), file_rounding_bytes)?;
    let need_bytes = add(logical_need_bytes, allocation_reserve_bytes)?;
    // One copy each of leaf staging, archive staging and a replaced destination.
    let output_reserve_bytes = mul(
        add(mul(n, config::OUTPUT_BYTES_PER_TRIANGLE)?, image_bytes)?,
        3,
    )?;
    Ok(FullScratchNeed {
        stages,
        image_bytes,
        max_live_files,
        logical_need_bytes,
        allocation_reserve_bytes,
        file_rounding_bytes,
        need_bytes,
        output_reserve_bytes,
    })
}

/// Admission of the upfront scratch need and output reservation, summed per filesystem.
/// Returns the bytes the scratch filesystem must hold (its preflight figure).
fn output_admission(
    parent: &Path,
    scratch_bytes: u64,
    output_reserve_bytes: u64,
    config: &TreeConfig,
    stage: crate::spill::PreflightStage,
) -> Result<u64, JobError> {
    let Some((work, output)) = &config.output_parents else {
        return Ok(scratch_bytes);
    };
    let copy = output_reserve_bytes / 3;
    // (path, bytes): leaf staging beside the work directory, archive staging and a replaced
    // destination beside the output. Filesystems are told apart by device id
    // (`same_filesystem`): btrfs subvolumes of one pool report distinct ids, so their demands
    // are checked separately against the same free space and can be under-reserved.
    let mut demands: Vec<(&Path, u64)> = vec![(parent, scratch_bytes)];
    for (path, bytes) in [(work.as_path(), copy), (output.as_path(), mul(copy, 2)?)] {
        let mut placed = false;
        for demand in &mut demands {
            if crate::spill::same_filesystem(demand.0, path)
                .map_err(|e| JobError::io("inspect output filesystem", path, e))?
            {
                demand.1 = add(demand.1, bytes)?;
                placed = true;
                break;
            }
        }
        if !placed {
            demands.push((path, bytes));
        }
    }
    for (path, bytes) in &demands[1..] {
        crate::spill::preflight(path, *bytes, crate::spill::SpaceRole::Output, stage)?;
    }
    Ok(demands[0].1)
}

/// Output reservation once image dimensions are admitted: three copies (leaf staging,
/// archive staging, a replaced destination) of the geometry estimate plus the larger of the
/// encoded source images and the profile's texel estimate.
fn texture_output_reserve(
    n: u64,
    image_bytes: u64,
    texels: u64,
    profile: config::Profile,
) -> Result<u64, JobError> {
    let textures = config::output_texture_bytes(profile, texels)
        .ok_or_else(|| exhausted("output reservation arithmetic overflow"))?;
    mul(
        add(
            mul(n, config::OUTPUT_BYTES_PER_TRIANGLE)?,
            image_bytes.max(textures),
        )?,
        3,
    )
}

/// Ends a stage: its measured peak, which must stay within the declared stage bound.
fn end_stage(scratch: &Scratch, bound: u64, name: &'static str) -> Result<u64, JobError> {
    let peak = scratch.stage_peak_bytes();
    if peak > bound {
        return Err(JobError::new(
            JobErrorKind::ResourceLimit,
            format!("full capture {name} scratch {peak} bytes exceeded its declared stage bound {bound}"),
        ));
    }
    Ok(peak)
}
/// Starts a stage: rechecks free space for its own working set plus file rounding.
fn begin_stage(scratch: &Scratch, need: &FullScratchNeed, bound: u64) -> Result<(), JobError> {
    config::preflight_remaining(scratch, add(bound, need.file_rounding_bytes)?)?;
    scratch.begin_stage();
    Ok(())
}

/// Every sort inside capture, by count and resident record size.
fn capture_sort_loads(scan: SourceScan, config: &TreeConfig) -> [SortLoad; 4] {
    use super::{
        corners::{Candidate, RankDepth},
        records::NodeRecord,
    };
    let n = scan.triangles;
    let corners = n.saturating_mul(3);
    let nodes = config::node_bound(n, scan.geometry_bytes, config);
    let load = |records, bytes: usize| SortLoad {
        records,
        record_bytes: bytes as u64,
    };
    [
        // An assembly bucket over the in-memory size is sorted externally: at most every
        // record.
        load(n, size_of::<FullTriangle>()),
        load(nodes, size_of::<NodeRecord>()),
        load(corners, size_of::<Candidate>()),
        load(corners, size_of::<RankDepth>()),
    ]
}

fn memory_bound(
    config: &TreeConfig,
    json_bytes: u64,
    image_count: u64,
    jobs: usize,
    spill_bytes: u64,
    preflight_bytes: u64,
    assembly_bytes: u64,
) -> Result<FullMemoryBound, JobError> {
    let mut geometry = MemoryBound::new(config, json_bytes, jobs);
    let sorter = config.sorter(jobs);
    let one = sorter.memory_bound::<FullTriangle>();
    if one == u64::MAX || geometry.bound_bytes == u64::MAX {
        return Err(exhausted("full capture memory arithmetic overflow"));
    }
    geometry.sort_bytes = mul(
        one.checked_sub(mul(jobs as u64, SORT_THREAD_BYTES)?)
            .ok_or_else(|| exhausted("full sort allowance underflow"))?,
        2,
    )?;
    // Existing H1 stream allowance also covers projection/sequence writers/tree.
    geometry.stream_bytes = add(
        geometry.stream_bytes,
        mul(config.sort.buffer_bytes.max(16) as u64, 6)?,
    )?;
    geometry.stream_bytes = add(
        geometry.stream_bytes,
        add(mul(config.sort.buffer_bytes.max(4096) as u64, 2)?, 2 * 4096)?,
    )?;
    let base = add(
        add(
            add(geometry.document_bytes, geometry.runtime_bytes)?,
            geometry.descriptor_bytes,
        )?,
        add(geometry.sort_bytes, geometry.stream_bytes)?,
    )?;
    geometry.bound_bytes = add(base, mul(geometry.jobs, geometry.per_job_bytes)?)?;
    let path = config.scratch_dir.as_ref().map_or_else(
        || Scratch::default_parent().as_os_str().len(),
        |p| p.as_os_str().len(),
    ) as u64;
    // Source Document plus exact metadata clone may coexist. JSON amplification
    // charge covers Material/Value ownership; image lengths/info/path arrays explicit.
    let owned_metadata_bytes = add(
        mul(json_bytes, 128)?,
        mul(
            image_count,
            add(
                add(
                    size_of::<ImageSnapshot>() as u64,
                    size_of::<ImageInfo>() as u64 + 32,
                )?,
                mul(add(path, 128)?, 2)?,
            )?,
        )?,
    )?;
    // Chunk descriptors of every live spill file (runs are 8 MiB chunk files), on top of the
    // run descriptor inventories already in `geometry.descriptor_bytes`.
    let chunk_descriptor_bytes = ExternalSorter::<FullTriangle>::inventory_memory_bound(
        config.sort,
        path as usize,
        spill_bytes,
    );
    // The retained full-triangle and corner-depth sequences are block-compressed; their block
    // indexes stay resident with the capture (both hold at most the spill's bytes).
    let block_index_bytes =
        crate::spill::block_index_memory_bound(spill_bytes, config.sort.buffer_bytes, 2);
    let corner_join_bytes = super::corners::wave_bytes(jobs, config.leaf_triangles())
        .ok_or_else(|| exhausted("corner join memory arithmetic overflow"))?;
    let bound_bytes = add(
        add(
            add(
                add(geometry.bound_bytes, owned_metadata_bytes)?,
                chunk_descriptor_bytes,
            )?,
            add(add(block_index_bytes, corner_join_bytes)?, preflight_bytes)?,
        )?,
        add(
            add(DECODER_ALLOWANCE, PLANNING_ALLOWANCE)?,
            assembly_bytes.saturating_sub(geometry.sort_bytes),
        )?,
    )?;
    Ok(FullMemoryBound {
        geometry,
        owned_metadata_bytes,
        chunk_descriptor_bytes,
        block_index_bytes,
        corner_join_bytes,
        preflight_bytes,
        assembly_bytes,
        decoder_allowance_bytes: DECODER_ALLOWANCE,
        planning_allowance_bytes: PLANNING_ALLOWANCE,
        bound_bytes,
    })
}

#[cfg_attr(not(test), allow(dead_code))] // Retained for bounded unit-test fixtures; production uses the streamed facade.
pub(crate) fn capture(
    input: &Path,
    config: &TreeConfig,
    check: &mut dyn FnMut() -> Result<(), JobError>,
) -> Result<FullCaptured, JobFailure> {
    config.validate().map_err(super::failure)?;
    if config.sort.run_bytes / 2 < size_of::<FullTriangle>() {
        return Err(super::failure(JobError::new(
            JobErrorKind::InvalidRequest,
            "full capture half-run budget must hold one resident FullTriangle",
        )));
    }
    check().map_err(super::failure)?;
    let source = StreamSource::open(input, check).map_err(super::failure)?;
    let check = std::cell::RefCell::new(check);
    let check = || (check.borrow_mut())();
    capture_admitted(source, config, &StepProgress::new(&check, None))
}

/// Captures an admitted source. `progress` is capture's cancellation check and receives its
/// steps ([`Step`]).
pub(crate) fn capture_admitted(
    source: StreamSource,
    config: &TreeConfig,
    progress: &StepProgress<'_>,
) -> Result<FullCaptured, JobFailure> {
    let mut check_progress = || progress.check();
    let check: &mut dyn FnMut() -> Result<(), JobError> = &mut check_progress;
    config.validate().map_err(super::failure)?;
    check().map_err(super::failure)?;
    super::member_gate(
        config::tile_lower_bound(
            source.document.triangle_count() as u64,
            config.leaf_triangles(),
        ),
        config.member_ceiling,
    )
    .map_err(super::failure)?;
    // Fail fast: the instanced triangle count is known before any geometry is read.
    let limits = RecordLimits::of(config);
    limits
        .check_triangles(source.document.triangle_count() as u64)
        .map_err(super::failure)?;
    progress
        .begin(Step::Scan, source.document.triangle_count() as u64)
        .map_err(super::failure)?;
    // The scan holds two bounded readers per worker, within the per-job term; its workers
    // are planned like capture's own before capture's larger base is known.
    let cores = std::thread::available_parallelism().map_or(1, |n| n.get());
    // The scan's used-vertex bitmap is charged in its own plan and, with validation's task
    // lists (whichever is larger), in capture's resident bound below.
    let scan_bitmap = capture::scan_bitmap_bytes(&source);
    let preflight_bytes = scan_bitmap.max(source.validation_task_bytes());
    let scan_memory = MemoryBound::new(config, source.json_bytes, 1);
    let scan_workers = config::plan_jobs(
        config.jobs,
        cores,
        config.available_memory(),
        0,
        scan_memory.base_bytes().saturating_add(scan_bitmap),
        scan_memory.per_job_bytes,
    )
    .jobs;
    let (mut scan, source_bounds) = capture::scan_with_source_bounds(
        &source,
        config.sort.buffer_bytes,
        usize::try_from(scan_workers).unwrap_or(1),
        &mut |done| progress.advance(done),
    )
    .map_err(super::failure)?;
    debug_assert_eq!(
        scan.triangles,
        source.document.triangle_count() as u64,
        "the scan step's total is the scanned triangle count"
    );
    progress.end().map_err(super::failure)?;
    // One frame, resolved once from the used post-node source bounds, serves capture; with a
    // source CRS, capture replaces the scan's placeholder box with the mapped one.
    let frame = match &config.source_crs {
        None => SourceFrame::Local,
        Some(spec) => SourceFrame::resolve(spec, source_bounds).map_err(super::failure)?,
    };
    let source_crs = frame.placement();
    let preflight = image_preflight(&source, check).map_err(super::failure)?;
    let (image_count, image_bytes) = (preflight.count, preflight.scratch_bytes);
    let layout = attributes::attribute_layout(&source);
    let parent = config
        .scratch_dir
        .clone()
        .unwrap_or_else(Scratch::default_parent);
    let volume = crate::spill::inspect_volume(&parent).map_err(|e| super::failure(e.into()))?;
    let shape = assembly::ScratchShape::of(&source, layout, scan.triangles)
        .ok_or_else(|| super::failure(exhausted("assembly scratch arithmetic overflow")))?;
    let need = scratch_need(
        scan,
        layout,
        config,
        config,
        volume.allocation_unit_bytes,
        image_count,
        image_bytes,
        shape,
    )
    .map_err(super::failure)?;
    // The assembly's fixed working sets for `workers`, from the sort allowance of `memory`.
    let assembly_for = |workers: usize, memory: &FullMemoryBound| {
        let plan = AssemblyPlan::fixed(workers, memory.geometry.sort_bytes);
        plan.memory_bytes(
            workers,
            scan.triangles,
            layout,
            config.sort.buffer_bytes,
            config.sorter(workers),
        )
        .map(|bytes| (plan, bytes))
        .ok_or_else(|| super::failure(exhausted("assembly memory arithmetic overflow")))
    };
    let bound_with = |config: &TreeConfig, workers: usize, spill: u64, assembly: u64| {
        memory_bound(
            config,
            source.json_bytes,
            image_count,
            workers,
            spill,
            preflight_bytes,
            assembly,
        )
    };
    let one = bound_with(config, 1, need.logical_need_bytes, 0).map_err(super::failure)?;
    let one = bound_with(config, 1, need.logical_need_bytes, assembly_for(1, &one)?.1)
        .map_err(super::failure)?;
    let jobs = config::plan_jobs(
        config.jobs,
        cores,
        config.available_memory(),
        if volume.memory_backed {
            need.need_bytes
        } else {
            0
        },
        one.base_bytes(),
        one.geometry.per_job_bytes,
    );
    let workers = usize::try_from(jobs.jobs)
        .map_err(|_| super::failure(exhausted("full worker count overflow")))?;
    let floor_memory =
        bound_with(config, workers, need.logical_need_bytes, 0).map_err(super::failure)?;
    let (assembly_plan, assembly_bytes) = assembly_for(workers, &floor_memory)?;
    let floor_memory = bound_with(config, workers, need.logical_need_bytes, assembly_bytes)
        .map_err(super::failure)?;
    // The floor's resident bound: what capture holds once its sorts have ended (the image
    // admission decodes below plan against it).
    let required_memory =
        add(floor_memory.bound_bytes, jobs.memory_backed_scratch_bytes).map_err(super::failure)?;
    if jobs
        .available_memory_bytes
        .is_some_and(|available| required_memory > available)
    {
        return Err(super::failure(exhausted(
            "full capture resident bound and memory-backed scratch do not fit available memory",
        )));
    }
    // D8: the sorts inside capture grow above the floor into what the budget leaves, judged
    // by the same scratch and memory admission and charged in the declared bound.
    let blocked = if volume.memory_backed {
        Some("memory_backed_scratch")
    } else if jobs.available_memory_bytes.is_none() {
        Some("unknown_memory")
    } else {
        None
    };
    #[cfg(test)]
    let blocked = blocked.or((!config.test_sort_plan).then_some("floor"));
    let candidate = |sort: crate::spill::SortConfig| {
        let mut capture = config.clone();
        capture.sort = sort;
        let grown = scratch_need(
            scan,
            layout,
            &capture,
            config,
            volume.allocation_unit_bytes,
            image_count,
            image_bytes,
            shape,
        )
        .ok()?;
        // Never more declared scratch than the floor's, in total or at any stage, so no
        // scratch admission (up front, at a stage's entry or at the texture recheck) refuses
        // a run the floor would admit.
        let disk = grown.within(&need);
        let memory =
            bound_with(&capture, workers, grown.logical_need_bytes, assembly_bytes).ok()?;
        let fits = jobs
            .available_memory_bytes
            .is_some_and(|available| memory.bound_bytes <= available);
        (disk && fits).then_some((capture, grown, memory))
    };
    let loads = capture_sort_loads(scan, config);
    // The producing thread plus its writers stay within the planned workers.
    let max_writers = workers.saturating_sub(1);
    let (sort, sort_plan) = capture_sort::plan(
        config.sort,
        &loads,
        blocked,
        max_writers,
        &capture_sort::FAN_INS,
        |c| candidate(c).is_some(),
    );
    let (capture_config, need, memory) = if sort == config.sort {
        (config.clone(), need, floor_memory)
    } else {
        candidate(sort)
            .ok_or_else(|| super::failure(invariant("capture sort plan does not fit")))?
    };
    let capture_config = &capture_config;
    // The vertex allowance: what the budget leaves beside the declared bound (the assembly
    // may also use the sort allowance, unused while it runs), at most every vertex.
    let free = match jobs.available_memory_bytes {
        Some(available) => available
            .saturating_sub(
                add(memory.bound_bytes, jobs.memory_backed_scratch_bytes)
                    .map_err(super::failure)?,
            )
            .saturating_add(
                memory
                    .geometry
                    .sort_bytes
                    .saturating_sub(memory.assembly_bytes),
            ),
        None => assembly::DEFAULT_VERTEX_ALLOWANCE,
    };
    let assembly_plan = assembly_plan.with_vertex_allowance(free.min(shape.vertex_bytes));
    #[cfg(test)]
    let assembly_plan = config.test_assembly.unwrap_or(assembly_plan);
    let assembly_bytes = assembly_plan
        .memory_bytes(
            workers,
            scan.triangles,
            layout,
            config.sort.buffer_bytes,
            capture_config.sorter(workers),
        )
        .ok_or_else(|| super::failure(exhausted("assembly memory arithmetic overflow")))?;
    let memory = bound_with(
        capture_config,
        workers,
        need.logical_need_bytes,
        assembly_bytes,
    )
    .map_err(super::failure)?;
    // One source frame per worker for the assembly's decode tasks, each resolved like the
    // first (resolution is deterministic).
    let mut frames = vec![std::sync::Mutex::new(frame)];
    if let Some(spec) = &config.source_crs {
        for _ in 1..workers {
            frames.push(std::sync::Mutex::new(
                SourceFrame::resolve(spec, source_bounds).map_err(super::failure)?,
            ));
        }
    }
    let frames = assembly::Frames(frames);
    // Image admission decodes beside capture's resident bound, within the declared decoder
    // allowance plus whatever available memory that bound leaves free. Every decode charge
    // is known from the preflight headers, so the largest must fit before any scratch
    // exists; the batches below re-derive each charge from the decoder's own header.
    let decode_budget = jobs.available_memory_bytes.map(|available| {
        available
            .saturating_sub(required_memory)
            .saturating_add(DECODER_ALLOWANCE)
    });
    admit_decode(decode_budget, &preflight).map_err(super::failure)?;
    // All checked counts, memory and physical allocation happen before creation.
    check().map_err(super::failure)?;
    let scratch_admission = output_admission(
        &parent,
        need.need_bytes,
        need.output_reserve_bytes,
        config,
        crate::spill::PreflightStage::Geometry,
    )
    .map_err(super::failure)?;
    let scratch = Scratch::create(config.scratch_dir.as_deref(), scratch_admission)
        .map_err(|e| super::failure(e.into()))?;
    if let Some(stop) = &config.stop {
        scratch.watch(stop.clone());
    }
    #[cfg(test)]
    if let Some(chunk) = config.test_chunk_bytes {
        scratch.set_chunk_bytes(chunk);
    }
    let source_totals = (
        source.source_bytes,
        source.external_files,
        source.external_bytes,
        source.json_bytes,
    );
    let result = (|| {
        if scratch.allocation_unit_bytes() != volume.allocation_unit_bytes
            || scratch.memory_backed() != volume.memory_backed
        {
            return Err(invariant(
                "scratch volume changed after full capture preflight",
            ));
        }
        let stages = need.stages;
        let mut peaks = ScratchStages::default();
        begin_stage(&scratch, &need, stages.capture)?;
        progress.begin(Step::Validate, 1)?;
        // On capture's workers, at most two bounded readers each (within the per-job term).
        source.validate_payload_parallel(config.sort.buffer_bytes, workers, check)?;
        progress.end()?;
        let (triangles, assembly_report) = assembly::assemble(
            &source,
            &frames,
            &scratch,
            layout,
            &mut scan,
            assembly_plan,
            workers,
            capture_config.sorter(workers),
            progress,
        )?;
        peaks.capture = end_stage(&scratch, stages.capture, "assembly")?;
        begin_stage(&scratch, &need, stages.projection)?;
        peaks.projection = end_stage(&scratch, stages.projection, "projection")?;
        begin_stage(&scratch, &need, stages.images)?;
        // Two units per image: its snapshot and its admission decode.
        progress.begin(Step::Images, image_count.saturating_mul(2))?;
        let (images, image_capture) =
            images::capture_images(&source, &scratch, config.sort.buffer_bytes, check)?;
        peaks.images = end_stage(&scratch, stages.images, "image capture")?;
        if image_capture.scratch_bytes != image_bytes
            || image_capture.images != image_count
            || image_capture.largest_encoded_bytes != preflight.largest_encoded_bytes
        {
            return Err(invariant("image capture disagrees with preflight"));
        }
        let mut image_info = Vec::new();
        image_info
            .try_reserve_exact(images.len())
            .map_err(|_| exhausted("image info allocation failed"))?;
        // Every declared image is decoded once here to admit it; pixels are
        // dropped and delivery decodes again on demand. Each decode is
        // charged from its admitted header dimensions ([`decode_charge`]). The
        // budget is the declared decoder allowance plus whatever available
        // memory capture's own resident bound leaves free. An image that
        // cannot fit is refused; the others decode in parallel batches that
        // fit, with cancellation observed between batches.
        let admission = source::DecodeAdmission::LARGE_MESH;
        let mut charges = Vec::new();
        charges
            .try_reserve_exact(images.len())
            .map_err(|_| exhausted("image charge allocation failed"))?;
        for (index, (image, declared)) in images.iter().zip(&preflight.dimensions).enumerate() {
            progress.advance(index as u64)?;
            let encoded = image.reconstruct(check)?;
            let info = source::probe_image_admitted(
                &encoded,
                image.identity.mime.as_deref(),
                admission,
                &mut || check(),
            )?;
            if (info.width, info.height) != *declared {
                return Err(JobError::new(
                    JobErrorKind::InvalidInput,
                    "image header dimensions disagree with the decoder's",
                ));
            }
            let texels = u64::from(info.width) * u64::from(info.height);
            charges.push(decode_charge(info.encoded_bytes, texels)?);
        }
        let budget = decode_budget;
        let pool = rayon::ThreadPoolBuilder::new()
            .num_threads(workers.max(1))
            .build()
            .map_err(|_| exhausted("image admission worker pool"))?;
        for range in decode_batches(&charges, budget, workers)? {
            progress.advance(image_count.saturating_add(range.start as u64))?;
            let (start, end) = (range.start, range.end);
            let infos: Vec<Result<ImageInfo, JobError>> = pool.install(|| {
                use rayon::prelude::*;
                images[start..end]
                    .par_iter()
                    .map(|image| {
                        let encoded = image.reconstruct(&mut || Ok(()))?;
                        let (info, rgba) = source::decode_image_admitted(
                            &encoded,
                            image.identity.mime.as_deref(),
                            admission,
                            &mut || Ok(()),
                        )?;
                        drop(rgba);
                        Ok(info)
                    })
                    .collect()
            });
            for info in infos {
                image_info.push(info?);
            }
        }
        progress.end()?;
        // Second output admission now that texel counts are known: lossless PNG pages can be
        // far larger than the encoded source images.
        let texels = image_info.iter().try_fold(0_u64, |sum, info| {
            add(sum, mul(u64::from(info.width), u64::from(info.height))?)
        })?;
        let output_reserve_bytes =
            texture_output_reserve(scan.triangles, image_bytes, texels, config.profile)?;
        let scratch_remaining = need.need_bytes.saturating_sub(scratch.live_bytes());
        let scratch_total = output_admission(
            &parent,
            scratch_remaining,
            output_reserve_bytes,
            config,
            crate::spill::PreflightStage::Texture,
        )?;
        crate::spill::preflight(
            &parent,
            scratch_total,
            crate::spill::SpaceRole::Scratch,
            crate::spill::PreflightStage::Texture,
        )?;
        let materials = source.document.materials().to_vec();
        let textures = source.document.textures().to_vec();
        let samplers = source.document.samplers().to_vec();
        source.verify_unchanged()?;
        drop(source); // All subsequent operations read owned snapshot records only.
        begin_stage(&scratch, &need, stages.tree)?;
        progress.begin(Step::Tree, triangles.len())?;
        // The full-triangle sequence is already in strict rank order (checked as it was
        // written): the tree is built over it directly, with no projected copy.
        let tree = tree::build_ranked(
            &scratch,
            &triangles,
            capture_config,
            workers,
            Some(progress),
            check,
        )?;
        progress.end()?;
        peaks.tree = end_stage(&scratch, stages.tree, "cell tree")?;
        super::member_gate(tree.stats.nodes, config.member_ceiling)?;
        if tree.stats.triangles != triangles.len() {
            return Err(invariant(
                "full capture tree rank count differs from sequence",
            ));
        }
        begin_stage(&scratch, &need, stages.corner_depths)?;
        let corner_depths = join_corner_depths(
            &scratch,
            &tree,
            &triangles,
            capture_config,
            workers,
            progress,
        )?;
        peaks.corner_depths = end_stage(&scratch, stages.corner_depths, "corner depth join")?;
        let height = usize::try_from(tree.stats.height)
            .map_err(|_| invariant("tree height conversion failed"))?;
        if height == 0 || height > 32 {
            return Err(invariant("accepted tree has invalid lattice height"));
        }
        let mut extents = vec![0.0; height];
        for summary in &tree.stats.per_depth {
            let depth = usize::try_from(summary.depth)
                .map_err(|_| invariant("tree depth conversion failed"))?;
            if depth >= height {
                return Err(invariant("tree depth summary exceeds height"));
            }
            extents[depth] = summary.maximum_extent;
        }
        // H28: material classes are resolved once here; both tile writers consume them.
        let material_classes =
            super::batching::MaterialClasses::new(&materials, &textures, &samplers)?;
        let resources = AtlasResources {
            materials: &materials,
            classes: &material_classes,
            textures: &textures,
            samplers: &samplers,
            image_info: &image_info,
        };
        begin_stage(&scratch, &need, stages.leaf_plans)?;
        progress.begin(Step::LeafPlans, tree.stats.leaves)?;
        // The declared budget (explicit, else the 4 GiB default ceiling; never detected
        // headroom) left for one leaf job after every term the leaf stage charges besides
        // the job itself, memory-backed scratch and work/archive storage included.
        let storage = match &config.output_parents {
            Some((work, output)) => super::pipeline::storage_charge_bytes((work, output), &need)?,
            None => 0,
        };
        let leaf_memory = config.memory_budget.map(|budget| {
            let declared = budget
                .requested_bytes
                .unwrap_or(super::resources::DEFAULT_CEILING_BYTES);
            let tmpfs = if volume.memory_backed {
                need.need_bytes
            } else {
                0
            };
            [
                storage,
                tmpfs,
                memory.owned_metadata_bytes,
                memory.geometry.document_bytes,
                super::leaves::producer::STAGE_RESERVE,
                super::leaves::producer::cache_floor(&image_info),
            ]
            .iter()
            .fold(declared, |n, &b| n.saturating_sub(b))
        });
        let leaf_plans = planning::preflight(
            &scratch,
            &triangles,
            &tree.nodes,
            resources,
            config,
            leaf_memory,
            check,
        )?;
        progress.end()?;
        peaks.leaf_plans = end_stage(&scratch, stages.leaf_plans, "leaf planning")?;
        if leaf_plans.len() != tree.stats.leaves {
            return Err(invariant("leaf plan count differs from immutable tree"));
        }
        let lattice = LatticePlan::new(config.profile, scan.bounds, &extents)?;
        check()?;
        if scratch.peak_bytes() > need.logical_need_bytes {
            return Err(exhausted(
                "full capture scratch exceeded declared logical peak",
            ));
        }
        let report = FullCaptureReport {
            source_bytes: source_totals.0,
            external_files: source_totals.1,
            external_bytes: source_totals.2,
            json_bytes: source_totals.3,
            scan,
            jobs,
            memory,
            sort: sort_plan,
            assembly: assembly_report,
            scratch_logical_peak_bytes: scratch.peak_bytes(),
            scratch_memory_backed: scratch.memory_backed(),
            scratch: FullScratchNeed {
                output_reserve_bytes,
                ..need
            },
            scratch_stage_peaks: peaks,
            image_capture,
            lattice_version: 1,
            source_crs: source_crs.as_ref().map(|(_, report)| report.clone()),
        };
        Ok((
            tree,
            triangles,
            corner_depths,
            leaf_plans,
            images,
            image_info,
            materials,
            material_classes,
            textures,
            samplers,
            lattice,
            report,
        ))
    })();
    match result {
        Ok((
            tree,
            triangles,
            corner_depths,
            leaf_plans,
            images,
            image_info,
            materials,
            material_classes,
            textures,
            samplers,
            lattice,
            report,
        )) => Ok(FullCaptured {
            tree,
            triangles,
            corner_depths,
            leaf_plans,
            images,
            image_info,
            materials,
            material_classes,
            textures,
            samplers,
            lattice,
            scan,
            report,
            placement: source_crs.map(|(placement, _)| placement),
            scratch,
        }),
        Err(error) => {
            // Closure locals and their open handles have dropped before close.
            let path = scratch.path().to_owned();
            let mut failed = super::failure(error);
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

/// Full capture's retained sequence is the rank order its tree is built over.
impl tree::RankedTriangles for Sequence<FullTriangle> {
    fn ranked(&self) -> Result<tree::RankedReader<'_>, JobError> {
        Ok(Box::new(self.reader()?.map(|row| Ok(row?.triangle))))
    }
    fn range(&self, first: u64, end: u64) -> Result<tree::RankedReader<'_>, JobError> {
        let rows = Sequence::range(self, first, end, self.buffer_bytes())?;
        Ok(Box::new(rows.map(|row| Ok(row?.triangle))))
    }
    fn key_at(&self, rank: u64) -> Result<u64, JobError> {
        let (mut encoded, mut out) = (Vec::new(), Vec::new());
        self.random()?.read(rank, 1, &mut encoded, &mut out)?;
        Ok(out[0].triangle.key)
    }
    fn count(&self) -> u64 {
        self.len()
    }
}

/// Exclusive bound of a triangle rank in the packed corner records (`corners`): 40
/// bits, about 1.1 trillion triangles.
pub(crate) const RANK_LIMIT: u64 = 1 << 40;
/// The range of capture's packed corner records: at most `triangles` triangles (40-bit
/// ranks). Capture checks it as soon as the count is known, and the corner join checks it
/// again; a run past it is refused with a typed error, never truncated.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct RecordLimits {
    pub triangles: u64,
}
impl RecordLimits {
    pub(crate) const PACKED: Self = Self {
        triangles: RANK_LIMIT,
    };
    /// The packed limits; tests lower them to drive the refusals through capture.
    fn of(config: &TreeConfig) -> Self {
        #[cfg(test)]
        if let Some(limits) = config.test_record_limits {
            return limits;
        }
        let _ = config;
        Self::PACKED
    }
    pub(crate) fn check_triangles(self, triangles: u64) -> Result<(), JobError> {
        if triangles > self.triangles {
            return Err(JobError::new(
                JobErrorKind::Unsupported,
                format!(
                    "{triangles} triangles exceed the {} the packed corner records' rank holds",
                    self.triangles
                ),
            ));
        }
        Ok(())
    }
}
#[cfg_attr(not(test), allow(dead_code))] // Retained for bounded unit-test fixtures; production uses the streamed facade.
/// A typed refusal, never a truncation, for a run with more triangles than [`RANK_LIMIT`].
pub(crate) fn check_rank_limit(triangles: u64) -> Result<(), JobError> {
    RecordLimits::PACKED.check_triangles(triangles)
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct CornerDepth {
    pub depths: [u8; 3],
}
impl Record for CornerDepth {
    const SIZE: usize = 3;
    fn encode(&self, out: &mut [u8]) {
        out.copy_from_slice(&self.depths);
    }
    fn decode(bytes: &[u8]) -> Self {
        Self {
            depths: bytes.try_into().unwrap(),
        }
    }
}
/// Each corner's welded depth, in rank order, for the tree built over `triangles`.
fn join_corner_depths<T: super::corners::Corners>(
    scratch: &Scratch,
    tree: &RetainedTree,
    triangles: &Sequence<T>,
    config: &TreeConfig,
    jobs: usize,
    progress: &StepProgress<'_>,
) -> Result<Sequence<CornerDepth>, JobError> {
    // Defence: capture refused the count as soon as it knew it.
    let limits = RecordLimits::of(config);
    limits.check_triangles(tree.stats.triangles)?;
    if tree.stats.height > config::HEIGHT_CAP {
        return Err(invariant("tree height exceeds the packed corner depth"));
    }
    if tree.stats.triangles != triangles.len() {
        return Err(invariant("corner rank inventory length mismatch"));
    }
    super::corners::corner_depths(
        scratch,
        &tree.nodes,
        tree.stats.height,
        triangles,
        config,
        jobs,
        progress,
    )
}

#[cfg(test)]
pub(super) mod tests {
    use super::super::records::TriangleRecord;
    use super::super::{
        attributes::{COLOR, NORMAL, TANGENT, UV0, UV1},
        config::{Jobs, Profile},
    };
    use super::*;
    use std::{fs, num::NonZeroUsize, path::PathBuf};
    pub(crate) fn fixture(parent: &Path, count: usize) -> PathBuf {
        fs::create_dir_all(parent).unwrap();
        let mut bytes = Vec::new();
        let mut views = Vec::new();
        let mut accessors = Vec::new();
        let attributes = [
            ("VEC3", vec![0., 0., 0., 1., 0., 0., 0., 1., 0.]),
            ("VEC3", vec![0., 0., 1., 0., 0., 1., 0., 0., 1.]),
            ("VEC4", vec![1., 0., 0., 1., 1., 0., 0., 1., 1., 0., 0., 1.]),
            ("VEC2", vec![0., 0., 1., 0., 0., 1.]),
            ("VEC2", vec![0., 0., 1., 0., 0., 1.]),
            (
                "VEC4",
                vec![1., 0.25, 0.5, 1., 1., 0.25, 0.5, 1., 1., 0.25, 0.5, 1.],
            ),
        ];
        for (i, (kind, values)) in attributes.iter().enumerate() {
            let start = bytes.len();
            for _ in 0..count {
                for value in values {
                    bytes.extend((*value as f32).to_le_bytes());
                }
            }
            views.push(
                serde_json::json!({"buffer":0,"byteOffset":start,"byteLength":bytes.len()-start}),
            );
            let mut accessor = serde_json::json!({"bufferView":i,"componentType":5126,"count":count*3,"type":kind});
            if i == 0 {
                accessor["min"] = serde_json::json!([0, 0, 0]);
                accessor["max"] = serde_json::json!([1, 1, 0]);
            }
            accessors.push(accessor);
        }
        fs::write(parent.join("attributes.bin"), &bytes).unwrap();
        image::RgbaImage::from_pixel(1, 1, image::Rgba([120, 80, 40, 255]))
            .save(parent.join("pixel.png"))
            .unwrap();
        let doc = serde_json::json!({"asset":{"version":"2.0"},"scene":0,
            "scenes":[{"nodes":[0]}],"nodes":[{"mesh":0}],
            "meshes":[{"primitives":[{"attributes":{"POSITION":0,"NORMAL":1,"TANGENT":2,"TEXCOORD_0":3,"TEXCOORD_1":4,"COLOR_0":5},"material":0}]}],
            "buffers":[{"uri":"attributes.bin","byteLength":bytes.len()}],"bufferViews":views,"accessors":accessors,
            "images":[{"uri":"pixel.png"},{"uri":"pixel.png"}],
            "textures":[{"source":0,"sampler":0}],"samplers":[{"wrapS":33071,"wrapT":33071}],
            "materials":[{"pbrMetallicRoughness":{"baseColorFactor":[0.6,0.7,0.8,1],"baseColorTexture":{"index":0,"texCoord":1},"metallicFactor":0.25,"roughnessFactor":0.75}}]});
        let input = parent.join("scene.gltf");
        fs::write(&input, serde_json::to_vec(&doc).unwrap()).unwrap();
        input
    }
    #[test]
    fn image_decode_batches_fit_the_budget_and_refuse_oversized_images() {
        assert_eq!(
            decode_batches(&[4, 4, 4, 9, 1], Some(9), 8).unwrap(),
            vec![0..2, 2..3, 3..4, 4..5]
        );
        assert_eq!(
            decode_batches(&[1, 1, 1, 1], Some(100), 3).unwrap(),
            vec![0..3, 3..4]
        );
        assert_eq!(decode_batches(&[5, 5], None, 8).unwrap(), vec![0..1, 1..2]);
        assert_eq!(
            decode_batches(&[1, 10], Some(9), 8).unwrap_err().kind(),
            JobErrorKind::ResourceLimit
        );
    }
    /// #338: decode charges dominated by encoded length (a 100 MB 8192² PNG, a 1 GiB image
    /// of 64² texels) batch by their whole charge under several workers, and an image whose
    /// charge alone exceeds the budget is a resource refusal.
    #[test]
    fn encoded_dominated_decode_charges_batch_within_the_budget() {
        let mib = 1u64 << 20;
        let charges = [
            decode_charge(100_000_000, 8192 * 8192).unwrap(),
            decode_charge(1 << 30, 64 * 64).unwrap(),
            decode_charge(600 * mib, 256 * 256).unwrap(),
            decode_charge(300 * mib, 1).unwrap(),
            decode_charge(300 * mib, 1).unwrap(),
            decode_charge(300 * mib, 1).unwrap(),
        ];
        assert_eq!(charges[0], 100_000_000 + 12 * 8192 * 8192);
        assert_eq!(charges[1], (1 << 30) + 12 * 64 * 64);
        let budget = 2 << 30;
        let batches = decode_batches(&charges, Some(budget), 3).unwrap();
        // 0+1 is 1.84 GiB and +2 exceeds 2 GiB; 2+3+4 is 1.17 GiB, three workers.
        assert_eq!(batches, vec![0..2, 2..5, 5..6]);
        for batch in &batches {
            let leased: u64 = charges[batch.clone()].iter().sum();
            assert!(leased <= budget && batch.len() <= 3, "{batch:?}");
        }
        assert_eq!(
            decode_batches(&charges, Some(budget), 1).unwrap(),
            (0..6).map(|i| i..i + 1).collect::<Vec<_>>()
        );
        let error = decode_batches(&charges, Some(charges[1] - 1), 3).unwrap_err();
        assert_eq!(error.kind(), JobErrorKind::ResourceLimit);
        assert!(decode_batches(&charges, Some(charges[1]), 3).is_ok());
    }
    /// #338: the pre-scratch refusal admits the largest decode charge exactly at the budget
    /// and refuses it one byte over, naming the image's encoded length and dimensions.
    #[test]
    fn largest_decode_charge_is_admitted_at_the_budget_and_refused_one_byte_over() {
        let charge = decode_charge(200 << 20, 256 * 256).unwrap();
        let preflight = ImagePreflight {
            count: 2,
            scratch_bytes: 0,
            dimensions: vec![(256, 256), (16, 16)],
            largest_encoded_bytes: 200 << 20,
            largest_decode: Some((charge, 200 << 20, (256, 256))),
        };
        assert!(admit_decode(Some(charge), &preflight).is_ok());
        assert!(admit_decode(None, &preflight).is_ok());
        let error = admit_decode(Some(charge - 1), &preflight).unwrap_err();
        assert_eq!(error.kind(), JobErrorKind::ResourceLimit);
        assert_eq!(
            error.to_string(),
            format!(
                "image capture: decoding a source image needs {charge} bytes (209715200 \
                 encoded plus 12 per texel of 256x256) but the memory budget leaves {} for \
                 image decoding",
                charge - 1
            )
        );
        let none = ImagePreflight {
            largest_decode: None,
            ..preflight
        };
        assert!(admit_decode(Some(0), &none).is_ok());
    }
    #[test]
    fn admitted_large_image_owners_survive_tree_scratch_recheck() {
        use image::ImageEncoder;
        let work = tempfile::tempdir().unwrap();
        let input = fixture(work.path(), 1);
        let mut pixels = Vec::with_capacity(2048 * 1024 * 4);
        let mut state = 0x7159_ac23u32;
        for _ in 0..2048 * 1024 * 4 {
            state ^= state << 13;
            state ^= state >> 17;
            state ^= state << 5;
            pixels.push(state as u8);
        }
        let file = fs::File::create(work.path().join("pixel.png")).unwrap();
        image::codecs::png::PngEncoder::new_with_quality(
            file,
            image::codecs::png::CompressionType::Fast,
            image::codecs::png::FilterType::NoFilter,
        )
        .write_image(&pixels, 2048, 1024, image::ExtendedColorType::Rgba8)
        .unwrap();
        let mut config = TreeConfig::new(Profile::Balanced);
        config.sort.run_bytes = 8192;
        config.sort.fan_in = 2;
        config.sort.buffer_bytes = 64;
        let captured = capture(&input, &config, &mut || Ok(())).unwrap();
        assert_eq!(captured.scan.triangles, 1);
        assert_eq!(captured.images.len(), 2);
        let scratch = &captured.report.scratch;
        assert!(
            captured.report.image_capture.scratch_bytes > scratch.stages.capture,
            "control must exceed the geometry-only stage"
        );
        assert!(scratch.stages.images >= captured.report.image_capture.scratch_bytes);
        assert!(captured.report.scratch_stage_peaks.images <= scratch.stages.images);
        assert!(
            captured.report.scratch_logical_peak_bytes
                <= captured.report.scratch.logical_need_bytes
        );
        captured.close().unwrap();
    }
    fn config(parent: &Path) -> TreeConfig {
        let mut config = TreeConfig::new(Profile::Balanced).with_leaf_triangles(2);
        config.jobs = Jobs::Fixed(NonZeroUsize::new(1).unwrap());
        config.scratch_dir = Some(parent.to_owned());
        config.sort.run_bytes = 4096;
        config.sort.fan_in = 2;
        config.sort.buffer_bytes = 73;
        config
    }
    #[test]
    fn owned_full_attributes_images_rank_and_synthetic_depth_survive_source_removal() {
        let dir = tempfile::tempdir().unwrap();
        let source_dir = dir.path().join("source");
        let parent = dir.path().join("scratch");
        fs::create_dir(&parent).unwrap();
        let input = fixture(&source_dir, 19);
        let output = capture(&input, &config(&parent), &mut || Ok(())).unwrap();
        fs::remove_dir_all(source_dir).unwrap();
        assert_eq!(output.images.len(), 2);
        assert_eq!(output.image_info.len(), 2);
        assert_eq!(
            output.images[0].identity.sha256,
            output.images[1].identity.sha256
        );
        for image in &output.images {
            assert!(!image.reconstruct(&mut || Ok(())).unwrap().is_empty());
        }
        assert_eq!(output.image_info[0].width, 1);
        assert_eq!(output.triangles.len(), 19);
        assert_eq!(output.corner_depths.len(), 19);
        assert!(output.tree.stats.height > 22);
        for (rank, row) in output.triangles.reader().unwrap().enumerate() {
            let row = row.unwrap();
            assert_eq!(row.triangle.source.triangle, rank as u64);
            assert_eq!(row.flags, NORMAL | TANGENT | UV0 | UV1 | COLOR);
            assert_eq!(row.corners[0].normal, [0., 0., 1.]);
            assert_eq!(row.corners[0].color, [1., 0.25, 0.5, 1.]);
            assert_eq!(row.material, 0);
        }
        let weld = welded_minimum(&output);
        for (triangle, depths) in output
            .triangles
            .reader()
            .unwrap()
            .zip(output.corner_depths.reader().unwrap())
        {
            let triangle = triangle.unwrap().triangle;
            let depths = depths.unwrap();
            for a in 0..3 {
                let key = triangle.positions[a].map(f32::to_bits);
                assert_eq!(depths.depths[a], weld[&key]);
            }
        }
        let material = output.materials[0].literal();
        assert_eq!(material["pbrMetallicRoughness"]["metallicFactor"], 0.25);
        assert_eq!(
            material["pbrMetallicRoughness"]["baseColorTexture"]["texCoord"],
            1
        );
        assert!(
            output.report.scratch_logical_peak_bytes <= output.report.scratch.logical_need_bytes
        );
        assert_eq!(
            output.lattice.steps().len(),
            output.tree.stats.height as usize
        );
        output.close().unwrap();
        assert_eq!(fs::read_dir(parent).unwrap().count(), 0);
    }
    #[test]
    fn malformed_unused_image_is_still_rejected_and_cleaned() {
        let dir = tempfile::tempdir().unwrap();
        let input = fixture(dir.path(), 1);
        fs::write(dir.path().join("bad.png"), b"not an image").unwrap();
        let mut doc: Value = serde_json::from_slice(&fs::read(&input).unwrap()).unwrap();
        doc["images"][1]["uri"] = serde_json::json!("bad.png");
        fs::write(&input, serde_json::to_vec(&doc).unwrap()).unwrap();
        let parent = dir.path().join("scratch");
        fs::create_dir(&parent).unwrap();
        let error = match capture(&input, &config(&parent), &mut || Ok(())) {
            Ok(_) => panic!("invalid unused image admitted"),
            Err(e) => e,
        };
        assert_eq!(error.error.kind(), JobErrorKind::InvalidInput);
        assert_eq!(fs::read_dir(parent).unwrap().count(), 0);
    }
    #[test]
    fn cancellation_and_full_record_budget_fail_before_source_capture() {
        let dir = tempfile::tempdir().unwrap();
        let input = fixture(dir.path(), 1);
        let parent = dir.path().join("scratch");
        fs::create_dir(&parent).unwrap();
        let cancel = match capture(&input, &config(&parent), &mut || {
            Err(JobError::new(JobErrorKind::Cancelled, "control"))
        }) {
            Ok(_) => panic!("cancel ignored"),
            Err(e) => e,
        };
        assert_eq!(cancel.error.kind(), JobErrorKind::Cancelled);
        assert_eq!(fs::read_dir(&parent).unwrap().count(), 0);
        let mut too_small = config(&parent);
        too_small.sort.run_bytes = 2 * (size_of::<FullTriangle>() - 1);
        let invalid = match capture(&input, &too_small, &mut || Ok(())) {
            Ok(_) => panic!("full half-run admitted"),
            Err(e) => e,
        };
        assert_eq!(invalid.error.kind(), JobErrorKind::InvalidRequest);
        assert_eq!(fs::read_dir(parent).unwrap().count(), 0);
    }
    #[test]
    fn cancellation_after_workspace_creation_drops_all_handles() {
        let dir = tempfile::tempdir().unwrap();
        let input = fixture(dir.path(), 3);
        let parent = dir.path().join("scratch");
        fs::create_dir(&parent).unwrap();
        let failed = match capture(&input, &config(&parent), &mut || {
            if fs::read_dir(&parent).unwrap().next().is_some() {
                Err(JobError::new(JobErrorKind::Cancelled, "workspace control"))
            } else {
                Ok(())
            }
        }) {
            Ok(_) => panic!("capture cancellation ignored"),
            Err(e) => e,
        };
        assert_eq!(failed.error.kind(), JobErrorKind::Cancelled);
        assert_eq!(fs::read_dir(parent).unwrap().count(), 0);
    }
    /// A position-only grid of welded quads in seeded shuffled order, with repeated
    /// triangles so the rank and corner sorts see equal positions from distinct ranks.
    pub(crate) fn shuffled_grid(parent: &Path, side: usize, seed: u64) -> PathBuf {
        fs::create_dir_all(parent).unwrap();
        let vertex = |x: usize, y: usize| {
            let z = ((x * 7 + y * 13) % 5) as f32 * 0.125;
            [x as f32, y as f32, z]
        };
        let mut triangles = Vec::new();
        for y in 0..side {
            for x in 0..side {
                let (a, b) = (vertex(x, y), vertex(x + 1, y));
                let (c, d) = (vertex(x, y + 1), vertex(x + 1, y + 1));
                triangles.push([a, b, d]);
                triangles.push([a, d, c]);
            }
        }
        for i in (0..triangles.len()).step_by(17) {
            triangles.push(triangles[i]);
        }
        let mut state = seed | 1;
        for i in (1..triangles.len()).rev() {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            triangles.swap(i, (state % (i as u64 + 1)) as usize);
        }
        let mut bytes = Vec::new();
        for triangle in &triangles {
            for value in triangle.iter().flatten() {
                bytes.extend(value.to_le_bytes());
            }
        }
        fs::write(parent.join("positions.bin"), &bytes).unwrap();
        let doc = serde_json::json!({
            "asset":{"version":"2.0"},"scene":0,"scenes":[{"nodes":[0]}],
            "nodes":[{"mesh":0}],"meshes":[{"primitives":[{"attributes":{"POSITION":0}}]}],
            "buffers":[{"uri":"positions.bin","byteLength":bytes.len()}],
            "bufferViews":[{"buffer":0,"byteLength":bytes.len()}],
            "accessors":[{"bufferView":0,"componentType":5126,"count":triangles.len()*3,
                "type":"VEC3","min":[0.,0.,0.],"max":[side as f32,side as f32,0.5]}]
        });
        let input = parent.join("grid.gltf");
        fs::write(&input, serde_json::to_vec(&doc).unwrap()).unwrap();
        input
    }
    /// Each position's least leaf depth over every corner, computed in memory from the
    /// tree's leaves: the corner join's oracle.
    pub(crate) fn welded_minimum(
        output: &FullCaptured,
    ) -> std::collections::BTreeMap<[u32; 3], u8> {
        let triangles: Vec<_> = output
            .triangles
            .reader()
            .unwrap()
            .map(Result::unwrap)
            .collect();
        let mut least = std::collections::BTreeMap::new();
        for node in output.tree.nodes.reader().unwrap() {
            let node = node.unwrap();
            if node.kind != super::super::records::NodeKind::Leaf {
                continue;
            }
            for t in &triangles[node.first as usize..node.end as usize] {
                for p in t.triangle.positions {
                    let depth = least.entry(p.map(f32::to_bits)).or_insert(u8::MAX);
                    *depth = (*depth).min(node.depth() as u8);
                }
            }
        }
        least
    }
    /// Corner depths, tree rank order and weld depths are identical for every worker count
    /// and run size, from in-memory sorts down to one-record runs merged two at a time; every
    /// corner depth is its position's welded minimum.
    #[test]
    fn corner_depth_join_is_identical_across_jobs_and_run_sizes() {
        type Outputs = (Vec<CornerDepth>, Vec<Vec<u8>>, Vec<([u32; 3], u8)>);
        let dir = tempfile::tempdir().unwrap();
        let parent = dir.path().join("scratch");
        fs::create_dir(&parent).unwrap();
        let input = shuffled_grid(&dir.path().join("source"), 20, 0x5eed_c0de_2026);
        // The plan needs disk-backed scratch (memory-backed spill keeps the floor), and the
        // system temporary directory may be tmpfs: plan beside the test binary instead.
        let disk =
            tempfile::tempdir_in(std::env::current_exe().unwrap().parent().unwrap()).unwrap();
        let planned = |jobs: usize, run_bytes: usize, fan_in: usize, plan: bool| {
            let mut config = TreeConfig::new(Profile::Balanced).with_leaf_triangles(16);
            config.jobs = Jobs::Fixed(NonZeroUsize::new(jobs).unwrap());
            config.scratch_dir = Some(if plan {
                disk.path().to_owned()
            } else {
                parent.clone()
            });
            config.sort.run_bytes = run_bytes;
            config.sort.fan_in = fan_in;
            config.sort.buffer_bytes = 61;
            // An explicit budget on every run: where the platform reports no available memory
            // (`available_memory()` is Linux-only), an unbudgeted capture keeps one worker and
            // reports `unknown_memory`, so neither the worker counts nor the floor would be
            // exercised there.
            config.memory_budget = Some(
                super::super::resources::MemoryBudget::resolve_with(Some(2 << 30), None).unwrap(),
            );
            if plan {
                // The budget plan grows these floors: one pass, with writers when jobs allow.
                config.test_sort_plan = true;
            }
            let output = capture(&input, &config, &mut || Ok(())).unwrap();
            let sort = output.report.sort;
            if plan {
                assert!(
                    !output.report.scratch_memory_backed,
                    "planned scratch on disk"
                );
                assert_eq!((sort.limited_by, sort.single_pass), ("single_pass", true));
                assert_eq!(sort.writers > 0, jobs > 1, "{sort:?}");
                assert!(output.report.memory.bound_bytes <= 2 << 30);
            } else if output.report.scratch_memory_backed {
                // Spill in RAM is charged to memory: the plan stays at the floor.
                assert_eq!(sort.limited_by, "memory_backed_scratch");
            } else {
                assert_eq!(sort.limited_by, "floor");
            }
            output
        };
        let run_planned = |jobs: usize, run_bytes: usize, fan_in: usize, plan: bool| -> Outputs {
            let output = planned(jobs, run_bytes, fan_in, plan);
            let depths: Vec<_> = output
                .corner_depths
                .reader()
                .unwrap()
                .map(Result::unwrap)
                .collect();
            let ranks: Vec<_> = output
                .triangles
                .reader()
                .unwrap()
                .map(|t| {
                    let mut bytes = vec![0; TriangleRecord::SIZE];
                    t.unwrap().triangle.encode(&mut bytes);
                    bytes
                })
                .collect();
            let lookup = welded_minimum(&output);
            let weld: Vec<_> = lookup.iter().map(|(&p, &d)| (p, d)).collect();
            for (triangle, depths) in output.triangles.reader().unwrap().zip(&depths) {
                let triangle = triangle.unwrap().triangle;
                for corner in 0..3 {
                    let key = triangle.positions[corner].map(f32::to_bits);
                    assert_eq!(depths.depths[corner], lookup[&key]);
                }
            }
            output.close().unwrap();
            (depths, ranks, weld)
        };
        let run = |jobs: usize, run_bytes: usize, fan_in: usize| -> Outputs {
            run_planned(jobs, run_bytes, fan_in, false)
        };
        let expected = run(1, 64 << 20, 64);
        assert_eq!(
            expected.0.len(),
            20 * 20 * 2 + (20 * 20 * 2usize).div_ceil(17)
        );
        let smallest = 2 * size_of::<FullTriangle>();
        for jobs in [1, 2, 4, 8] {
            for (run_bytes, fan_in) in [(smallest, 2), (smallest * 3, 3), (4096, 2), (1 << 16, 64)]
            {
                assert!(
                    run(jobs, run_bytes, fan_in) == expected,
                    "jobs {jobs}, run bytes {run_bytes}, fan-in {fan_in}"
                );
            }
            assert!(
                run_planned(jobs, smallest, 2, true) == expected,
                "planned, jobs {jobs}"
            );
        }
        assert_eq!(fs::read_dir(parent).unwrap().count(), 0);
        assert_eq!(fs::read_dir(disk.path()).unwrap().count(), 0);
    }
    /// More triangles than the packed corner records' 40-bit rank holds are refused before
    /// any record is packed (the records' round trips: `corners::tests`).
    #[test]
    fn rank_limit_is_refused_past_its_end() {
        assert!(check_rank_limit(RANK_LIMIT).is_ok());
        assert_eq!(
            check_rank_limit(RANK_LIMIT + 1).unwrap_err().kind(),
            JobErrorKind::Unsupported
        );
    }
    /// Capture refuses runs past the packed-record limit as soon as the triangle count is
    /// known, before any scratch exists. Exactly at the limit the run is accepted and its
    /// outputs unchanged. The limit is lowered through the test seam; the packed limit
    /// shares the same path.
    #[test]
    fn capture_refuses_past_record_limits_up_front() {
        let dir = tempfile::tempdir().unwrap();
        let parent = dir.path().join("scratch");
        fs::create_dir(&parent).unwrap();
        let input = shuffled_grid(&dir.path().join("source"), 6, 0x11_e7e5);
        let mut config = TreeConfig::new(Profile::Balanced).with_leaf_triangles(16);
        config.jobs = Jobs::Fixed(NonZeroUsize::new(1).unwrap());
        config.scratch_dir = Some(parent.clone());
        let depths = |captured: &FullCaptured| -> Vec<CornerDepth> {
            captured
                .corner_depths
                .reader()
                .unwrap()
                .map(Result::unwrap)
                .collect()
        };
        let free = capture(&input, &config, &mut || Ok(())).unwrap();
        let triangles = free.scan.triangles;
        let expected = depths(&free);
        free.close().unwrap();

        config.test_record_limits = Some(RecordLimits { triangles });
        let exact = capture(&input, &config, &mut || Ok(())).unwrap();
        assert_eq!(depths(&exact), expected);
        exact.close().unwrap();

        // One triangle over: refused before the scan, so no scratch is ever created.
        config.test_record_limits = Some(RecordLimits {
            triangles: triangles - 1,
        });
        let scratch_seen = std::cell::Cell::new(false);
        let failed = match capture(&input, &config, &mut || {
            scratch_seen.set(scratch_seen.get() || fs::read_dir(&parent).unwrap().count() > 0);
            Ok(())
        }) {
            Ok(_) => panic!("capture past the triangle limit was accepted"),
            Err(failed) => failed,
        };
        assert_eq!(failed.error.kind(), JobErrorKind::Unsupported);
        assert!(failed.error.to_string().contains("triangles exceed"));
        assert!(
            !scratch_seen.get(),
            "triangle refusal came after scratch creation"
        );
    }
    /// The corner join keeps its own refusal of the triangle limit as defence, on a tree
    /// built directly: one past the limit is refused, exactly at it is joined.
    #[test]
    fn corner_join_refuses_past_record_limits() {
        let scratch = Scratch::create(None, 0).unwrap();
        let mut config = TreeConfig::new(Profile::Balanced).with_leaf_triangles(2);
        config.sort.buffer_bytes = 64;
        let p = |x: f32, y: f32| [x, y, 0.];
        let triangles = [
            [p(0., 0.), p(1., 0.), p(0., 1.)],
            [p(1., 0.), p(1., 1.), p(0., 1.)],
            [p(1., 0.), p(2., 0.), p(1., 1.)],
            [p(2., 0.), p(2., 1.), p(1., 1.)],
        ];
        let tree = || {
            let mut sorter = ExternalSorter::new(&scratch, config.sorter(1)).unwrap();
            for (i, positions) in triangles.iter().enumerate() {
                sorter
                    .push(TriangleRecord {
                        key: (i as u64) << 60,
                        source: super::super::records::Provenance {
                            triangle: i as u64,
                            ..Default::default()
                        },
                        vertices: [0, 1, 2],
                        positions: *positions,
                        bytes: 56,
                        degenerate: false,
                    })
                    .unwrap();
            }
            tree::build(
                &scratch,
                sorter.finish().unwrap(),
                &config,
                1,
                None,
                &mut || Ok(()),
            )
            .unwrap()
        };
        let built = tree();
        assert_eq!((built.triangles.len(), built.weld_depths.len()), (4, 6));
        drop(built);
        let join = |config: &TreeConfig| {
            let tree::TreeOutput {
                triangles,
                nodes,
                weld_depths,
                stats,
            } = tree();
            let retained = RetainedTree { nodes, stats };
            let depths = join_corner_depths(
                &scratch,
                &retained,
                &triangles,
                config,
                1,
                StepProgress::silent(),
            )?;
            // The join agrees with H1's weld table over the same tree.
            let weld: std::collections::BTreeMap<_, _> = weld_depths
                .reader()
                .unwrap()
                .map(|w| w.map(|w| (w.position, w.depth)).unwrap())
                .collect();
            for (triangle, depths) in triangles.reader().unwrap().zip(depths.reader().unwrap()) {
                let (triangle, depths) = (triangle.unwrap(), depths.unwrap());
                for corner in 0..3 {
                    let key = triangle.positions[corner].map(f32::to_bits);
                    assert_eq!(depths.depths[corner], weld[&key]);
                }
            }
            Ok::<(), JobError>(())
        };
        let mut limited = config.clone();
        for (limits, refusal) in [(3, Some("triangles exceed")), (4, None)] {
            limited.test_record_limits = Some(RecordLimits { triangles: limits });
            match (join(&limited), refusal) {
                (Ok(()), None) => {}
                (Err(error), Some(message)) => {
                    assert_eq!(error.kind(), JobErrorKind::Unsupported);
                    assert!(error.to_string().contains(message), "{error}");
                }
                (result, _) => panic!("{limits:?}: unexpected {result:?}"),
            }
        }
        assert_eq!(scratch.live_bytes(), 0);
    }
    /// Joins the corner depths of `rows` (Morton key, positions) under leaves of `leaf`
    /// triangles on `jobs` workers and checks them against H1's weld table over the same tree.
    /// Returns the tree's statistics.
    fn assert_join_matches_weld(
        rows: &[(u64, [[f32; 3]; 3])],
        leaf: u64,
        jobs: usize,
    ) -> (tree::TreeStats, Vec<usize>) {
        let scratch = Scratch::create(None, 0).unwrap();
        let mut config = TreeConfig::new(Profile::Balanced).with_leaf_triangles(leaf);
        config.sort.buffer_bytes = 64;
        let mut sorter = ExternalSorter::new(&scratch, config.sorter(1)).unwrap();
        for (i, (key, positions)) in rows.iter().enumerate() {
            sorter
                .push(TriangleRecord {
                    key: *key,
                    source: super::super::records::Provenance {
                        triangle: i as u64,
                        ..Default::default()
                    },
                    vertices: [0, 1, 2],
                    positions: *positions,
                    bytes: 56,
                    degenerate: false,
                })
                .unwrap();
        }
        let tree::TreeOutput {
            triangles,
            nodes,
            weld_depths,
            stats,
        } = tree::build(
            &scratch,
            sorter.finish().unwrap(),
            &config,
            1,
            None,
            &mut || Ok(()),
        )
        .unwrap();
        let weld: std::collections::BTreeMap<_, _> = weld_depths
            .reader()
            .unwrap()
            .map(|w| w.map(|w| (w.position, w.depth)).unwrap())
            .collect();
        let (levels, _) = super::super::corners::grid_levels(&nodes);
        let retained = RetainedTree {
            nodes,
            stats: stats.clone(),
        };
        let depths = join_corner_depths(
            &scratch,
            &retained,
            &triangles,
            &config,
            jobs,
            StepProgress::silent(),
        )
        .unwrap();
        for (t, d) in triangles.reader().unwrap().zip(depths.reader().unwrap()) {
            let (t, d) = (t.unwrap(), d.unwrap());
            for corner in 0..3 {
                assert_eq!(
                    d.depths[corner],
                    weld[&t.positions[corner].map(f32::to_bits)]
                );
            }
        }
        (stats, levels)
    }

    /// A position written as -0.0 is a different (f32-bit) position from +0.0 for the join,
    /// as for the weld table. Capture canonicalises zeros before both, so this pins the
    /// join's own key.
    #[test]
    fn corner_join_keys_positions_by_their_bits() {
        let mut rows = Vec::new();
        for i in 0..200u64 {
            let x = (i % 10) as f32;
            let zero = if i % 2 == 0 { 0. } else { -0. };
            rows.push((i << 50, [[x, zero, 0.], [x + 1., 0., zero], [x, 1., 0.]]));
        }
        for jobs in [1, 3] {
            let (stats, _) = assert_join_matches_weld(&rows, 4, jobs);
            assert!(stats.leaves > 10);
        }
    }

    /// One leaf of huge triangles (its box spans the whole scene, so the finest grid level
    /// cannot list it) among many small leaves sharing its corners.
    #[test]
    fn corner_join_with_one_huge_leaf_among_small_ones() {
        let mut rows = Vec::new();
        // Small triangles on a 30 x 30 grid, keys spread over the low key range.
        for i in 0..900u64 {
            let (x, y) = ((i % 30) as f32, (i / 30) as f32);
            rows.push((i << 40, [[x, y, 0.], [x + 1., y, 0.], [x, y + 1., 0.]]));
        }
        // Huge triangles corner to corner, under keys of their own, sharing grid corners.
        for i in 0..6u64 {
            let o = i as f32;
            rows.push((
                (7 << 60) + i,
                [[o, 0., 0.], [30., 30. - o, 0.], [0., 30., o]],
            ));
        }
        for (leaf, jobs) in [(3, 1), (3, 4), (8, 2)] {
            let (stats, levels) = assert_join_matches_weld(&rows, leaf, jobs);
            assert!(stats.leaves > 50, "{stats:?}");
            // The huge leaf's box is listed on a coarser grid level.
            assert!(
                levels.len() > 1 && levels[1..].iter().sum::<usize>() > 0,
                "{levels:?}"
            );
        }
    }

    /// The corner join over adversarial trees equals H1's weld table and an in-memory oracle
    /// for every worker count: a point shared by corners in nearly every leaf, slivers whose
    /// corners lie in far leaves' boxes, coincident triangles (one key: degenerate runs and
    /// packed octants) and keys unrelated to positions, so leaf boxes overlap everywhere.
    #[test]
    fn corner_join_matches_the_weld_table_on_adversarial_trees() {
        let scratch = Scratch::create(None, 0).unwrap();
        let mut state = 0x2545_f491_4f6c_dd1du64;
        let mut next = move || {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            state
        };
        let mut rows: Vec<(u64, [[f32; 3]; 3])> = Vec::new();
        let grid = |v: u64| (v % 9) as f32 * 0.25;
        for i in 0..600u64 {
            let key = next() >> 1; // 63-bit Morton keys.
            let a = [grid(next()), grid(next()), grid(next())];
            let b = [grid(next()), grid(next()), 0.];
            let positions = match i % 5 {
                // A fan: one corner at the shared point.
                0 => [[0.5, 0.5, 0.5], a, b],
                // A sliver from a corner of the grid to the opposite one.
                1 => [[0., 0., 0.], [2., 2., 2.], a],
                _ => [a, b, [grid(next()), 0., grid(next())]],
            };
            rows.push((key, positions));
        }
        // Coincident triangles under one key, and repeats of earlier triangles.
        for i in 0..40 {
            rows.push((7 << 40, rows[i % 3].1));
            rows.push((next() >> 1, rows[i * 7].1));
        }
        for (jobs, leaf) in [(1, 3), (2, 3), (3, 16), (8, 1), (4, 200)] {
            let mut config = TreeConfig::new(Profile::Balanced).with_leaf_triangles(leaf);
            config.sort.buffer_bytes = 64;
            let mut sorter = ExternalSorter::new(&scratch, config.sorter(1)).unwrap();
            for (i, (key, positions)) in rows.iter().enumerate() {
                sorter
                    .push(TriangleRecord {
                        key: *key,
                        source: super::super::records::Provenance {
                            triangle: i as u64,
                            ..Default::default()
                        },
                        vertices: [0, 1, 2],
                        positions: *positions,
                        bytes: 56,
                        degenerate: false,
                    })
                    .unwrap();
            }
            let tree::TreeOutput {
                triangles,
                nodes,
                weld_depths,
                stats,
            } = tree::build(
                &scratch,
                sorter.finish().unwrap(),
                &config,
                1,
                None,
                &mut || Ok(()),
            )
            .unwrap();
            // Forty coincident triangles exceed every leaf size but the largest.
            assert!(
                stats.leaves > 2 && (leaf > 40 || stats.degenerate_cells > 0),
                "{stats:?}"
            );
            let weld: std::collections::BTreeMap<_, _> = weld_depths
                .reader()
                .unwrap()
                .map(|w| w.map(|w| (w.position, w.depth)).unwrap())
                .collect();
            let mut oracle = std::collections::BTreeMap::new();
            let ranked: Vec<_> = triangles.reader().unwrap().map(Result::unwrap).collect();
            for node in nodes.reader().unwrap().map(Result::unwrap) {
                if node.kind == super::super::records::NodeKind::Leaf {
                    for t in &ranked[node.first as usize..node.end as usize] {
                        for p in t.positions {
                            let d = oracle.entry(p.map(f32::to_bits)).or_insert(u8::MAX);
                            *d = (*d).min(node.depth() as u8);
                        }
                    }
                }
            }
            assert_eq!(weld, oracle);
            let retained = RetainedTree { nodes, stats };
            let depths = join_corner_depths(
                &scratch,
                &retained,
                &triangles,
                &config,
                jobs,
                StepProgress::silent(),
            )
            .unwrap();
            assert_eq!(depths.len(), triangles.len());
            let mut patched = 0;
            for (t, d) in ranked.iter().zip(depths.reader().unwrap()) {
                let d = d.unwrap();
                for corner in 0..3 {
                    assert_eq!(
                        d.depths[corner],
                        oracle[&t.positions[corner].map(f32::to_bits)]
                    );
                }
                patched += usize::from(d.depths.iter().any(|&x| x != d.depths[0]));
            }
            // Leaves of different depths share points, so some corners are patched.
            assert!(patched > 0 || leaf == 200, "jobs {jobs}, leaf {leaf}");
        }
        assert_eq!(scratch.live_bytes(), 0);
    }
    /// Indexed grids of `side` quads per primitive (POSITION only), one mesh and node each;
    /// a `shuffled` primitive stores its vertices in a seeded random order, its indices
    /// remapped, so its corners have no index locality.
    fn indexed_grids(parent: &Path, primitives: &[(usize, bool)]) -> PathBuf {
        fs::create_dir_all(parent).unwrap();
        let mut bytes = Vec::new();
        let (mut views, mut accessors, mut meshes, mut nodes) =
            (Vec::new(), Vec::new(), Vec::new(), Vec::new());
        let mut state = 0x9e37_79b9_7f4a_7c15u64;
        for (index, &(side, shuffled)) in primitives.iter().enumerate() {
            let row = side + 1;
            let count = row * row;
            let mut order: Vec<usize> = (0..count).collect();
            if shuffled {
                for i in (1..count).rev() {
                    state ^= state << 13;
                    state ^= state >> 7;
                    state ^= state << 17;
                    order.swap(i, (state % (i as u64 + 1)) as usize);
                }
            }
            // `order[v]` is where grid vertex `v` is stored.
            let mut positions = vec![[0f32; 3]; count];
            for v in 0..count {
                let (x, y) = ((v % row) as f32, (v / row) as f32);
                positions[order[v]] = [x + 100. * index as f32, y, ((v * 7) % 5) as f32 * 0.125];
            }
            let mut indices = Vec::new();
            for y in 0..side {
                for x in 0..side {
                    let a = y * row + x;
                    for v in [a, a + 1, a + row + 1, a, a + row + 1, a + row] {
                        indices.push(order[v] as u32);
                    }
                }
            }
            let (mut min, mut max) = ([f32::MAX; 3], [f32::MIN; 3]);
            for p in &positions {
                for a in 0..3 {
                    min[a] = min[a].min(p[a]);
                    max[a] = max[a].max(p[a]);
                }
            }
            let start = bytes.len();
            bytes.extend(positions.iter().flatten().flat_map(|v| v.to_le_bytes()));
            views.push(serde_json::json!({"buffer":0,"byteOffset":start,"byteLength":count * 12}));
            accessors.push(
                serde_json::json!({"bufferView":views.len() - 1,"componentType":5126,
                "count":count,"type":"VEC3","min":min,"max":max}),
            );
            let start = bytes.len();
            bytes.extend(indices.iter().flat_map(|i| i.to_le_bytes()));
            views.push(
                serde_json::json!({"buffer":0,"byteOffset":start,"byteLength":indices.len() * 4}),
            );
            accessors.push(
                serde_json::json!({"bufferView":views.len() - 1,"componentType":5125,
                "count":indices.len(),"type":"SCALAR"}),
            );
            meshes.push(
                serde_json::json!({"primitives":[{"attributes":{"POSITION":accessors.len() - 2},
                "indices":accessors.len() - 1}]}),
            );
            nodes.push(serde_json::json!({"mesh":index}));
        }
        fs::write(parent.join("grids.bin"), &bytes).unwrap();
        let doc = serde_json::json!({
            "asset":{"version":"2.0"},"scene":0,"scenes":[{"nodes":(0..primitives.len()).collect::<Vec<_>>()}],
            "nodes":nodes,"meshes":meshes,"buffers":[{"uri":"grids.bin","byteLength":bytes.len()}],
            "bufferViews":views,"accessors":accessors
        });
        let input = parent.join("grids.gltf");
        fs::write(&input, serde_json::to_vec(&doc).unwrap()).unwrap();
        input
    }

    /// Captures `input` with `plan` (or the derived one): the retained records, the corner
    /// depths and the assembly report.
    fn capture_planned(
        input: &Path,
        parent: &Path,
        jobs: usize,
        plan: Option<super::super::assembly::AssemblyPlan>,
    ) -> (Vec<Vec<u8>>, Vec<CornerDepth>, AssemblyReport) {
        fs::create_dir_all(parent).unwrap();
        let mut config = config(parent);
        config.sort.buffer_bytes = 64;
        config.jobs = Jobs::Fixed(NonZeroUsize::new(jobs).unwrap());
        config.test_assembly = plan;
        let output = capture(input, &config, &mut || Ok(())).unwrap();
        let rows = output
            .triangles
            .reader()
            .unwrap()
            .map(|t| {
                let mut bytes = vec![0; FullTriangle::SIZE];
                t.unwrap().encode(&mut bytes);
                bytes
            })
            .collect();
        let depths = output
            .corner_depths
            .reader()
            .unwrap()
            .map(Result::unwrap)
            .collect();
        let report = output.report.assembly.clone();
        output.close().unwrap();
        (rows, depths, report)
    }

    /// Spilled plans whose vertex caches hold one block, or every block.
    fn spilled(cache_blocks: usize) -> super::super::assembly::AssemblyPlan {
        super::super::assembly::AssemblyPlan {
            vertex_allowance_bytes: 0,
            cache_blocks,
            bucket_bytes: 64 << 10,
            gather_bytes: 1 << 20,
            window: 2,
        }
    }

    /// A shuffled primitive routed through the corner sorts captures the same records as
    /// through the block caches and in memory, and the same triangles as H1's corner-sort
    /// capture (the path the assembly replaced), on one and three workers.
    #[test]
    fn routed_corners_capture_identical_sequences() {
        let dir = tempfile::tempdir().unwrap();
        let input = indexed_grids(&dir.path().join("source"), &[(40, true)]);
        let (rows, depths, report) = capture_planned(&input, &dir.path().join("memory"), 1, None);
        assert_eq!(
            (report.spilled_primitives, report.routed_primitives),
            (0, 0)
        );
        for jobs in [1, 3] {
            for (cache_blocks, routed) in [(1 << 20, 0), (1, 1)] {
                let parent = dir.path().join(format!("spilled-{jobs}-{cache_blocks}"));
                let (other, other_depths, report) =
                    capture_planned(&input, &parent, jobs, Some(spilled(cache_blocks)));
                assert_eq!(report.routed_primitives, routed, "{report:?}");
                assert!(other == rows, "{jobs} jobs, {cache_blocks} cache blocks");
                assert_eq!(other_depths, depths);
            }
        }
        // H1's capture sorts corners by (primitive, vertex) and back: the develop path.
        let source = StreamSource::open(&input, &mut || Ok(())).unwrap();
        let scan = super::super::capture::scan(&source, 64, &mut || Ok(())).unwrap();
        let scratch = Scratch::create(None, 0).unwrap();
        let h1: Vec<Vec<u8>> = super::super::capture::capture(
            &source,
            &scratch,
            config(dir.path()).sorter(1),
            &scan,
            &mut || Ok(()),
        )
        .unwrap()
        .map(|t| {
            let mut bytes = vec![0; TriangleRecord::SIZE];
            t.unwrap().encode(&mut bytes);
            bytes
        })
        .collect();
        let projected: Vec<&[u8]> = rows.iter().map(|r| &r[..TriangleRecord::SIZE]).collect();
        assert!(projected == h1.iter().map(Vec::as_slice).collect::<Vec<_>>());
    }

    /// One capture where a shuffled primitive is routed and an ordered one keeps its block
    /// cache: the records equal the in-memory capture's.
    #[test]
    fn mixed_routes_capture_identical_sequences() {
        let dir = tempfile::tempdir().unwrap();
        let input = indexed_grids(&dir.path().join("source"), &[(40, true), (3, false)]);
        let (rows, depths, _) = capture_planned(&input, &dir.path().join("memory"), 2, None);
        // One vertex per block (64-byte buffers): the ordered 16-vertex grid touches at most
        // 16 blocks per chunk, the shuffled one far more.
        let (other, other_depths, report) =
            capture_planned(&input, &dir.path().join("mixed"), 2, Some(spilled(32)));
        assert_eq!(
            (report.spilled_primitives, report.routed_primitives),
            (2, 1),
            "{report:?}"
        );
        assert!(other == rows);
        assert_eq!(other_depths, depths);
    }

    /// A stop request while a primitive's corners are routed ends the capture as cancelled
    /// and removes its scratch.
    #[test]
    fn a_stop_during_routed_sorts_is_honoured() {
        use super::super::assembly::ROUTING;
        let dir = tempfile::tempdir().unwrap();
        let input = indexed_grids(&dir.path().join("source"), &[(60, true)]);
        let parent = dir.path().join("scratch");
        fs::create_dir_all(&parent).unwrap();
        let mut config = config(&parent);
        config.sort.buffer_bytes = 64;
        config.test_assembly = Some(spilled(1));
        ROUTING.with(|r| r.set(false));
        let error = match capture(&input, &config, &mut || {
            if ROUTING.with(|r| r.get()) {
                Err(JobError::new(
                    JobErrorKind::Cancelled,
                    "operation cancelled",
                ))
            } else {
                Ok(())
            }
        }) {
            Ok(_) => panic!("the stop was not honoured"),
            Err(failed) => failed.error,
        };
        assert!(
            ROUTING.with(|r| r.replace(false)),
            "the corners were routed"
        );
        assert_eq!(error.kind(), JobErrorKind::Cancelled, "{error}");
        assert_eq!(fs::read_dir(&parent).unwrap().count(), 0);
    }

    /// The assembly writes the same retained sequence, corner depths and tree for every
    /// plan: decoded vertices in memory or spilled (one-block caches), one-record buckets
    /// (every bucket over the in-memory size, sorted externally), the smallest gathering
    /// buffers and a one-bucket window, on one and four workers.
    #[test]
    fn assembly_plans_capture_identical_sequences() {
        use super::super::assembly::AssemblyPlan;
        let dir = tempfile::tempdir().unwrap();
        let input = shuffled_grid(&dir.path().join("source"), 24, 0x0a55_e3b1_7e00);
        let capture_with = |jobs: usize, plan: Option<AssemblyPlan>| {
            let parent = dir
                .path()
                .join(format!("scratch-{jobs}-{}", plan.is_some()));
            fs::create_dir_all(&parent).unwrap();
            let mut config = config(&parent);
            config.jobs = Jobs::Fixed(NonZeroUsize::new(jobs).unwrap());
            config.test_assembly = plan;
            let output = capture(&input, &config, &mut || Ok(())).unwrap();
            let rows: Vec<Vec<u8>> = output
                .triangles
                .reader()
                .unwrap()
                .map(|t| {
                    let mut bytes = vec![0; FullTriangle::SIZE];
                    t.unwrap().encode(&mut bytes);
                    bytes
                })
                .collect();
            let depths: Vec<_> = output
                .corner_depths
                .reader()
                .unwrap()
                .map(Result::unwrap)
                .collect();
            let report = output.report.assembly.clone();
            output.close().unwrap();
            (rows, depths, report)
        };
        let (rows, depths, report) = capture_with(1, None);
        assert_eq!(report.spilled_primitives, 0);
        let plans = [
            AssemblyPlan {
                vertex_allowance_bytes: 0,
                cache_blocks: 1,
                bucket_bytes: 1,
                gather_bytes: 0,
                window: 1,
            },
            AssemblyPlan {
                vertex_allowance_bytes: 0,
                cache_blocks: 2,
                bucket_bytes: 64 << 10,
                gather_bytes: 1 << 20,
                window: 3,
            },
            AssemblyPlan {
                vertex_allowance_bytes: 1 << 40,
                cache_blocks: 1,
                bucket_bytes: 4096,
                gather_bytes: 0,
                window: 8,
            },
        ];
        for jobs in [1, 4] {
            for plan in plans {
                let (other, other_depths, report) = capture_with(jobs, Some(plan));
                assert!(other == rows, "{jobs} jobs, {plan:?}");
                assert_eq!(other_depths, depths, "{jobs} jobs, {plan:?}");
                assert_eq!(
                    report.spilled_primitives > 0,
                    plan.vertex_allowance_bytes == 0,
                    "{report:?}"
                );
                if plan.bucket_bytes == 1 {
                    // Every cell is a bucket; those holding more than one triangle are over
                    // the size and sorted externally.
                    assert!(
                        report.heavy_buckets > 0 && report.buckets > 100,
                        "{report:?}"
                    );
                }
            }
        }
    }
    /// Every capture stage's measured peak stays within its declared bound across sorter
    /// regimes: one run, intermediate merges with slack, and inventory compaction; for every
    /// attribute (the full layout) and for positions only (the narrowest layout). A
    /// violation already fails capture closed; the asserts state the contract.
    #[test]
    fn measured_stage_peaks_stay_within_the_stage_model() {
        let dir = tempfile::tempdir().unwrap();
        let smallest = 2 * size_of::<FullTriangle>();
        for (count, run_bytes, fan_in, grid) in [
            (19, 160 << 20, 64, false),
            (300, smallest, 2, false),
            (300, 4096, 3, false),
            (1200, 1 << 16, 4, false),
            (19, 160 << 20, 64, true),
            (300, smallest, 2, true),
            (1200, 4096, 3, true),
        ] {
            let case = format!("{count}-{run_bytes}-{fan_in}-{grid}");
            let source = dir.path().join(format!("source-{case}"));
            let parent = dir.path().join(format!("scratch-{case}"));
            fs::create_dir(&parent).unwrap();
            let input = if grid {
                // About `count` triangles: 2·side² quads plus every 17th repeated.
                let side = ((count as f64 / 2.).sqrt() as usize).max(2);
                shuffled_grid(&source, side, count as u64)
            } else {
                fixture(&source, count)
            };
            let mut config = config(&parent);
            config.sort.run_bytes = run_bytes;
            config.sort.fan_in = fan_in;
            let output = capture(&input, &config, &mut || Ok(())).unwrap();
            let declared = output.report.scratch.stages;
            let measured = output.report.scratch_stage_peaks;
            let pairs = [
                ("capture", measured.capture, declared.capture),
                ("projection", measured.projection, declared.projection),
                ("images", measured.images, declared.images),
                ("tree", measured.tree, declared.tree),
                (
                    "corner_depths",
                    measured.corner_depths,
                    declared.corner_depths,
                ),
                ("leaf_plans", measured.leaf_plans, declared.leaf_plans),
            ];
            for (name, peak, bound) in pairs {
                assert!(peak <= bound, "{case} {name}: {peak} > {bound}");
            }
            assert!(measured.projection > 0 && measured.tree > 0);
            assert!(
                output.report.scratch_logical_peak_bytes
                    <= output.report.scratch.logical_need_bytes
            );
            output.close().unwrap();
        }
    }
    #[test]
    fn sort_spill_slack_follows_the_merge_regime() {
        let sorter = |run_bytes, fan_in| crate::spill::SortConfig {
            run_bytes,
            fan_in,
            buffer_bytes: 64,
            jobs: 1,
            writers: 0,
        };
        let row = size_of::<FullTriangle>();
        let chunk = std::cell::Cell::new(crate::spill::CHUNK_BYTES);
        let spill = |records, run_records: usize, fan_in| {
            config::sort_spill_with::<FullTriangle>(
                records,
                sorter(run_records * row, fan_in),
                chunk.get(),
            )
            .unwrap()
        };
        let s = |records: u64| records * FullTriangle::SIZE as u64;
        // Two records per chunk file, so F partly read chunks are smaller than S.
        chunk.set(2 * FullTriangle::SIZE);
        let chunks = |fan_in: u64| fan_in * s(2);
        // At most F runs: no intermediate merge; the stream holds partly read chunks.
        let one = spill(40, 10, 4);
        assert_eq!((one.push_peak, one.finish_peak), (s(40), s(40)));
        assert_eq!(one.stream_slack, chunks(4));
        // More than F runs: an intermediate merge frees inputs while writing its output.
        let some = spill(60, 10, 4);
        assert_eq!(
            (some.push_peak, some.finish_peak),
            (s(60), s(60) + chunks(4))
        );
        // Multi-level passes are bounded the same way, group by group.
        let deep = spill(170, 10, 4);
        assert_eq!(deep.finish_peak, s(170) + chunks(4));
        // Inventory compaction from 64F runs merges while pushing.
        let compacting = spill(2560, 10, 4);
        assert_eq!(compacting.push_peak, s(2560) + chunks(4));
        // The flow of a stream into proportional output: the larger side plus the slack.
        assert_eq!(one.flow(s(100)), Some(s(100) + chunks(4)));
        chunk.set(crate::spill::CHUNK_BYTES);
        // Unchunked-size data: slack never exceeds S.
        let small = spill(60, 10, 4);
        assert_eq!(small.finish_peak, 2 * s(60));
    }
    /// mgal 2048 (10M triangles, 89 images of 2048x2048, 367 MB source) published lossless
    /// needed 4.51 GB of output-side space at once (leaf staging and archive staging). The flat
    /// per-triangle reservation (3 x (119 B/tri + encoded image bytes)) covered only 3.86 GB;
    /// the texel term covers it, scaled by profile.
    #[test]
    fn output_reservation_covers_textured_lossless_and_scales_by_profile() {
        let n = 10_000_000;
        let image_bytes = 98_000_000; // the 2048 copy's JPEG images, rounded down
        let texels = 89 * 2048 * 2048;
        let measured = 4_510_000_000;
        let flat = 3 * (n * config::OUTPUT_BYTES_PER_TRIANGLE + image_bytes);
        assert!(
            flat < measured,
            "control: the flat reservation under-reserves"
        );
        let lossless = texture_output_reserve(n, image_bytes, texels, Profile::Lossless).unwrap();
        assert!(lossless >= measured, "{lossless}");
        // Two copies are live at once; the third is a replaced destination.
        assert!(lossless / 3 * 2 >= measured);
        let balanced = texture_output_reserve(n, image_bytes, texels, Profile::Balanced).unwrap();
        assert!(balanced < lossless && balanced > flat);
        // Untextured inputs keep the geometry estimate.
        assert_eq!(
            texture_output_reserve(n, 0, 0, Profile::Lossless).unwrap(),
            3 * n * config::OUTPUT_BYTES_PER_TRIANGLE
        );
    }
    #[test]
    fn checked_full_scratch_and_decoder_memory_charges() {
        let cfg = TreeConfig::new(Profile::Balanced);
        let scan = SourceScan {
            triangles: 10,
            bounds: super::super::records::Bounds {
                min: [0.; 3],
                max: [1.; 3],
            },
            geometry_bytes: 2000,
        };
        let need = scratch_need(
            scan,
            crate::spill::Layout::FULL,
            &cfg,
            &cfg,
            4096,
            2,
            8192,
            assembly::ScratchShape::default(),
        )
        .unwrap();
        // 10 full triangles, 2,860 bytes. Capture's stage is the largest assembly step: at
        // least the staged records plus an eighth for the staging group that overlaps the
        // written sequence, and at least every corner placed (30 decoded vertices of 72
        // bytes) beside its routing sorts. The retained sequence is the projection stage.
        assert!(need.stages.capture >= 2860 + 2860_u64.div_ceil(8));
        assert!(need.stages.capture >= 30 * 72);
        assert_eq!(need.stages.projection, 2860);
        // Spilled decoded vertices sit beside the staged records while distributing.
        let spilled = scratch_need(
            scan,
            crate::spill::Layout::FULL,
            &cfg,
            &cfg,
            4096,
            2,
            8192,
            assembly::ScratchShape {
                vertex_bytes: 1000,
                files: 3,
            },
        )
        .unwrap();
        assert!(spilled.stages.capture > need.stages.capture);
        assert!(spilled.stages.capture <= need.stages.capture + 1000);
        assert_eq!(spilled.max_live_files, need.max_live_files + 3);
        assert_eq!(need.stages.images, 2860 + 8192);
        // The HLOD stage's declared share: its incidence and source-index scratch beside
        // everything retained for leaves.
        assert_eq!(
            need.stages.hlod_incidence,
            need.stages.leaf_plans
                + super::super::hlod::interfaces::scratch_need(10, cfg.sorter(1)).unwrap()
                + super::super::hlod::source_index::scratch_need(10).unwrap()
        );
        assert_eq!(need.logical_need_bytes, need.stages.max());
        // Control: the old reserve re-added the whole logical need as extent overhead. Now the
        // reserve is a 10% model margin plus per-file rounding only.
        assert_eq!(
            need.allocation_reserve_bytes,
            need.logical_need_bytes.div_ceil(10) + need.file_rounding_bytes
        );
        assert_eq!(
            need.file_rounding_bytes,
            config::file_rounding(4096, need.max_live_files).unwrap()
        );
        assert_eq!(
            need.need_bytes,
            need.logical_need_bytes + need.allocation_reserve_bytes
        );
        let h1 = config::scratch_need_parts(10, 2000, &cfg, 4096).unwrap();
        assert_eq!(
            h1.need_bytes - h1.logical_need_bytes,
            config::file_rounding(4096, h1.max_live_files).unwrap()
        );
        assert_eq!(need.output_reserve_bytes, 3 * (10 * 119 + 8192));
        let overflow = SourceScan {
            triangles: u64::MAX,
            ..scan
        };
        assert_eq!(
            scratch_need(
                overflow,
                crate::spill::Layout::FULL,
                &cfg,
                &cfg,
                4096,
                2,
                8192,
                assembly::ScratchShape::default(),
            )
            .unwrap_err()
            .kind(),
            JobErrorKind::ResourceLimit
        );
        let memory = memory_bound(&cfg, 1000, 2, 1, 0, 0, 0).unwrap();
        let large = memory_bound(&cfg, 1000, 2, 1, 150 << 30, 0, 0).unwrap();
        // 150 GiB of live spill is about 19k chunk descriptors.
        assert!(large.chunk_descriptor_bytes > memory.chunk_descriptor_bytes + 19_000 * 64);
        assert_eq!(memory.decoder_allowance_bytes, 192 << 20);
        assert!(memory.bound_bytes > memory.geometry.bound_bytes + memory.decoder_allowance_bytes);
    }
}
