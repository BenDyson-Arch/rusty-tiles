//! Full-detail snapshot owner. Source reads end here; later leaves read only owned
//! spills and admitted material/texture/sampler metadata. No tile output yet.
use super::{
    attributes::{self, FullTriangle},
    capture::{self, SourceScan},
    config::{self, JobsPlan, MemoryBound, TreeConfig},
    images::{self, ImageCaptureBounds, ImageSnapshot},
    input::StreamSource,
    leaves::{
        atlas::AtlasResources,
        geometry::LatticePlan,
        planning::{self, LeafPlan},
    },
    records::TriangleRecord,
    source_crs::{MeshSourceCrsReport, SourceFrame},
    tree::{self, RetainedTree},
};
use crate::{
    gltf_source::source::{self, ImageInfo, Material},
    spill::{ExternalSorter, Record, Scratch, Sequence, SequenceWriter},
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
    /// Corner, positioned-corner and full-triangle sorts of the attribute capture.
    pub capture: u64,
    /// The sorted full-triangle stream and its retained sequence (the field keeps its name:
    /// capture no longer writes a separate rank-ordered projection).
    pub projection: u64,
    pub images: u64,
    /// The cell tree and node sort over the retained sequence, and the weld-depth pass.
    pub tree: u64,
    pub corner_depths: u64,
    pub leaf_plans: u64,
    /// Everything retained for leaves plus the HLOD stage's declared incidence and source-index
    /// need.
    pub hlod_incidence: u64,
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
fn image_preflight(
    source: &StreamSource,
    check: &mut dyn FnMut() -> Result<(), JobError>,
) -> Result<(u64, u64), JobError> {
    let count = u64::try_from(source.document.image_sources().len())
        .map_err(|_| exhausted("image count overflow"))?;
    let mut bytes = 0;
    for index in 0..source.document.image_sources().len() {
        check()?;
        let length = source.image_len(index)?;
        if length == 0 {
            return Err(JobError::new(
                JobErrorKind::InvalidInput,
                "encoded image is empty",
            ));
        }
        if length > images::MAX_ENCODED_IMAGE_BYTES {
            return Err(JobError::new(
                JobErrorKind::Unsupported,
                "encoded image exceeds 32 MiB admission ceiling",
            ));
        }
        let chunks = add(length, 4093)? / 4094;
        bytes = add(bytes, mul(chunks, 4096)?)?;
    }
    Ok((count, bytes))
}
fn scratch_need(
    scan: SourceScan,
    layout: crate::spill::Layout,
    config: &TreeConfig,
    unit: u64,
    image_count: u64,
    image_bytes: u64,
) -> Result<FullScratchNeed, JobError> {
    use super::{
        records::{CornerRef, NodeRecord, Span},
        tree::{VertexDepth, WeldDepth},
    };
    let overflow = || exhausted("full capture scratch arithmetic overflow");
    let sorter = config.sorter(1); // Run sizes do not depend on the worker count.
    let n = scan.triangles;
    let corners = mul(n, 3)?;
    let corner_refs = config::sort_spill::<CornerRef>(corners, sorter).ok_or_else(overflow)?;
    // Capture's attribute records spill under the run's attribute layout.
    let chunk = crate::spill::CHUNK_BYTES;
    let positioned =
        config::sort_spill_in::<attributes::FullCorner>(corners, sorter, chunk, layout)
            .ok_or_else(overflow)?;
    let full =
        config::sort_spill_in::<FullTriangle>(n, sorter, chunk, layout).ok_or_else(overflow)?;
    let nodes = config::node_bound(n, scan.geometry_bytes, config);
    let node_sort = config::sort_spill::<NodeRecord>(nodes, sorter).ok_or_else(overflow)?;
    let node_bytes = mul(nodes, NodeRecord::SIZE as u64)?;
    let span_bytes = mul(nodes, Span::SIZE as u64)?;
    let vertex_depths = config::sort_spill::<VertexDepth>(corners, sorter).ok_or_else(overflow)?;
    let weld_bytes = mul(corners, WeldDepth::SIZE as u64)?;
    let rank_corners = config::sort_spill::<RankCorner>(corners, sorter).ok_or_else(overflow)?;
    let rank_depths = config::sort_spill::<RankDepth>(corners, sorter).ok_or_else(overflow)?;
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
    let capture = max(&[
        Some(corner_refs.finish_peak),
        corner_refs.flow(positioned.push_peak),
        Some(positioned.finish_peak),
        positioned.flow(full.push_peak),
        Some(full.finish_peak),
    ])?;
    // The full-triangle stream feeds its retained sequence: in rank order already, it is
    // the tree's input and the corner join's, with no projected copy.
    let projection_stage = full.flow(t).ok_or_else(overflow)?;
    let images = add(t, image_bytes)?;
    let retained = add(t, image_bytes)?;
    let welding = max(&[
        Some(vertex_depths.finish_peak),
        vertex_depths.flow(weld_bytes),
    ])?;
    let tree = add(
        retained,
        max(&[
            node_sort.push_peak.checked_add(span_bytes),
            node_sort
                .flow(node_bytes)
                .and_then(|v| v.checked_add(span_bytes)),
            welding.checked_add(node_bytes + span_bytes),
        ])?,
    )?;
    let base = add(add(retained, node_bytes)?, weld_bytes)?;
    let corner_depths = add(
        base,
        max(&[
            Some(rank_corners.finish_peak),
            rank_corners.flow(rank_depths.push_peak),
            Some(rank_depths.finish_peak),
            rank_depths.flow(corner_depth_bytes),
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
    let h1 =
        config::scratch_need_parts(n, scan.geometry_bytes, config, unit).ok_or_else(overflow)?;
    let capacity = ((config.sort.run_bytes / 2) / size_of::<FullTriangle>()).max(1) as u64;
    let counted = add(mul(add(mul(n, 16)?.div_ceil(capacity), 5)?, 2)?, 32)?;
    let inventory = add(mul(config.sort.fan_in as u64, 256)?, 32)?;
    // H1 run inventory plus full-capture runs, image sequences and retained sequences.
    let max_live_files = add(
        h1.max_live_files,
        add(add(inventory.min(counted), image_count)?, 5)?,
    )?;
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

fn memory_bound(
    config: &TreeConfig,
    json_bytes: u64,
    image_count: u64,
    jobs: usize,
    spill_bytes: u64,
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
    let bound_bytes = add(
        add(
            add(geometry.bound_bytes, owned_metadata_bytes)?,
            chunk_descriptor_bytes,
        )?,
        add(DECODER_ALLOWANCE, PLANNING_ALLOWANCE)?,
    )?;
    Ok(FullMemoryBound {
        geometry,
        owned_metadata_bytes,
        chunk_descriptor_bytes,
        decoder_allowance_bytes: DECODER_ALLOWANCE,
        planning_allowance_bytes: PLANNING_ALLOWANCE,
        bound_bytes,
    })
}

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
    capture_admitted(source, config, check)
}

pub(crate) fn capture_admitted(
    source: StreamSource,
    config: &TreeConfig,
    check: &mut dyn FnMut() -> Result<(), JobError>,
) -> Result<FullCaptured, JobFailure> {
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
    let (mut scan, source_bounds) =
        capture::scan_with_source_bounds(&source, config.sort.buffer_bytes, check)
            .map_err(super::failure)?;
    // One frame, resolved once from the used post-node source bounds, serves capture; with a
    // source CRS, capture replaces the scan's placeholder box with the mapped one.
    let mut frame = match &config.source_crs {
        None => SourceFrame::Local,
        Some(spec) => SourceFrame::resolve(spec, source_bounds).map_err(super::failure)?,
    };
    let source_crs = frame.placement();
    let (image_count, image_bytes) = image_preflight(&source, check).map_err(super::failure)?;
    let layout = attributes::attribute_layout(&source);
    let parent = config
        .scratch_dir
        .clone()
        .unwrap_or_else(Scratch::default_parent);
    let volume = crate::spill::inspect_volume(&parent).map_err(|e| super::failure(e.into()))?;
    let need = scratch_need(
        scan,
        layout,
        config,
        volume.allocation_unit_bytes,
        image_count,
        image_bytes,
    )
    .map_err(super::failure)?;
    let one = memory_bound(
        config,
        source.json_bytes,
        image_count,
        1,
        need.logical_need_bytes,
    )
    .map_err(super::failure)?;
    let jobs = config::plan_jobs(
        config.jobs,
        std::thread::available_parallelism().map_or(1, |n| n.get()),
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
    let memory = memory_bound(
        config,
        source.json_bytes,
        image_count,
        workers,
        need.logical_need_bytes,
    )
    .map_err(super::failure)?;
    let required_memory =
        add(memory.bound_bytes, jobs.memory_backed_scratch_bytes).map_err(super::failure)?;
    if jobs
        .available_memory_bytes
        .is_some_and(|available| required_memory > available)
    {
        return Err(super::failure(exhausted(
            "full capture resident bound and memory-backed scratch do not fit available memory",
        )));
    }
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
        let mut full_stream = attributes::capture_full_in(
            &source,
            &mut frame,
            &scratch,
            config.sorter(workers),
            layout,
            &mut scan,
            check,
        )?;
        peaks.capture = end_stage(&scratch, stages.capture, "attribute capture")?;
        begin_stage(&scratch, &need, stages.projection)?;
        let mut full_writer = SequenceWriter::<FullTriangle>::with_layout(
            &scratch,
            "full-triangles",
            config.sort.buffer_bytes,
            layout,
        )?;
        let mut previous: Option<TriangleRecord> = None;
        let mut count = 0_u64;
        for row in full_stream.by_ref() {
            let row = row?;
            if previous
                .as_ref()
                .is_some_and(|p| p.cmp(&row.triangle) != std::cmp::Ordering::Less)
            {
                return Err(invariant(
                    "full source provenance projection is not unique strict order",
                ));
            }
            previous = Some(row.triangle);
            full_writer.push(&row)?;
            count = add(count, 1)?;
            if count.is_multiple_of(1024) {
                check()?;
            }
        }
        drop(full_stream);
        let triangles = full_writer.finish()?;
        if count != scan.triangles {
            return Err(invariant("full capture rank count differs from scan"));
        }
        peaks.projection = end_stage(&scratch, stages.projection, "projection")?;
        begin_stage(&scratch, &need, stages.images)?;
        let (images, image_capture) =
            images::capture_images(&source, &scratch, config.sort.buffer_bytes, check)?;
        peaks.images = end_stage(&scratch, stages.images, "image capture")?;
        if image_capture.scratch_bytes != image_bytes || image_capture.images != image_count {
            return Err(invariant("image capture disagrees with preflight"));
        }
        let mut image_info = Vec::new();
        image_info
            .try_reserve_exact(images.len())
            .map_err(|_| exhausted("image info allocation failed"))?;
        // Every declared image is decoded once here to admit it; pixels are
        // dropped and delivery decodes again on demand. Each decode is
        // charged from its admitted header dimensions: the encoded snapshot
        // plus 12 bytes per texel (decoder output and the RGBA8 copy). The
        // budget is the declared decoder allowance plus whatever available
        // memory capture's own resident bound leaves free. An image that
        // cannot fit is refused; the others decode in parallel batches that
        // fit, with cancellation observed between batches.
        let admission = source::DecodeAdmission::LARGE_MESH;
        let mut charges = Vec::new();
        charges
            .try_reserve_exact(images.len())
            .map_err(|_| exhausted("image charge allocation failed"))?;
        for image in &images {
            check()?;
            let encoded = image.reconstruct(check)?;
            let info = source::probe_image_admitted(
                &encoded,
                image.identity.mime.as_deref(),
                admission,
                &mut || check(),
            )?;
            let texels = u64::from(info.width) * u64::from(info.height);
            charges.push(add(mul(texels, 12)?, encoded.len() as u64)?);
        }
        let budget = jobs.available_memory_bytes.map(|available| {
            available
                .saturating_sub(required_memory)
                .saturating_add(DECODER_ALLOWANCE)
        });
        let pool = rayon::ThreadPoolBuilder::new()
            .num_threads(workers.max(1))
            .build()
            .map_err(|_| exhausted("image admission worker pool"))?;
        for range in decode_batches(&charges, budget, workers)? {
            check()?;
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
        check()?;
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
        // The full-triangle sequence is already in strict rank order (checked as it was
        // written): the tree is built over it directly, with no projected copy.
        let tree = tree::build_ranked(
            &scratch,
            &triangles,
            config,
            workers,
            Some(add(stages.tree, need.file_rounding_bytes)?),
            check,
        )?;
        peaks.tree = end_stage(&scratch, stages.tree, "cell tree")?;
        super::member_gate(tree.stats.nodes, config.member_ceiling)?;
        if tree.stats.triangles != triangles.len() {
            return Err(invariant(
                "full capture tree rank count differs from sequence",
            ));
        }
        begin_stage(&scratch, &need, stages.corner_depths)?;
        let corner_depths =
            join_corner_depths(&scratch, &tree, &triangles, config, workers, check)?;
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
        let leaf_plans =
            planning::preflight(&scratch, &triangles, &tree.nodes, resources, config, check)?;
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
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct RankCorner {
    position: [u32; 3],
    rank: u64,
    corner: u8,
}
impl Record for RankCorner {
    const SIZE: usize = 21;
    fn encode(&self, out: &mut [u8]) {
        for a in 0..3 {
            out[a * 4..a * 4 + 4].copy_from_slice(&self.position[a].to_le_bytes());
        }
        out[12..20].copy_from_slice(&self.rank.to_le_bytes());
        out[20] = self.corner;
    }
    fn decode(bytes: &[u8]) -> Self {
        Self {
            position: std::array::from_fn(|a| {
                u32::from_le_bytes(bytes[a * 4..a * 4 + 4].try_into().unwrap())
            }),
            rank: u64::from_le_bytes(bytes[12..20].try_into().unwrap()),
            corner: bytes[20],
        }
    }
}
/// Exclusive bound of a triangle rank in the packed corner records ([`RankDepth`]): 40
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
/// A typed refusal, never a truncation, for a run with more triangles than [`RANK_LIMIT`].
pub(crate) fn check_rank_limit(triangles: u64) -> Result<(), JobError> {
    RecordLimits::PACKED.check_triangles(triangles)
}
/// A corner's depth, sorted back to rank order. Encoded in 6 bytes: the 40-bit rank and
/// `corner | depth << 2` (a depth is below the tree height, at most 32).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct RankDepth {
    rank: u64,
    corner: u8,
    depth: u8,
}
impl Record for RankDepth {
    const SIZE: usize = 6;
    fn encode(&self, out: &mut [u8]) {
        debug_assert!(self.rank < RANK_LIMIT && self.corner < 4 && self.depth < 64);
        out[..5].copy_from_slice(&self.rank.to_le_bytes()[..5]);
        out[5] = self.corner | self.depth << 2;
    }
    fn decode(bytes: &[u8]) -> Self {
        let mut rank = [0u8; 8];
        rank[..5].copy_from_slice(&bytes[..5]);
        Self {
            rank: u64::from_le_bytes(rank),
            corner: bytes[5] & 3,
            depth: bytes[5] >> 2,
        }
    }
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
/// Each corner's welded depth, in rank order, for the tree built over `ranked`.
fn join_corner_depths(
    scratch: &Scratch,
    tree: &RetainedTree,
    ranked: &dyn tree::RankedTriangles,
    config: &TreeConfig,
    jobs: usize,
    check: &mut dyn FnMut() -> Result<(), JobError>,
) -> Result<Sequence<CornerDepth>, JobError> {
    // Defence: capture refused the count as soon as it knew it.
    let limits = RecordLimits::of(config);
    let triangles = tree.stats.triangles;
    limits.check_triangles(triangles)?;
    if tree.stats.height > config::HEIGHT_CAP {
        return Err(invariant("tree height exceeds the packed corner depth"));
    }
    let mut corners = ExternalSorter::<RankCorner>::new(scratch, config.sorter(jobs))?;
    let mut rank = 0_u64;
    for triangle in ranked.ranked()? {
        if rank.is_multiple_of(1024) {
            check()?;
        }
        let triangle = triangle?;
        for corner in 0..3 {
            corners.push(RankCorner {
                position: triangle.positions[corner].map(f32::to_bits),
                rank,
                corner: corner as u8,
            })?;
        }
        rank = add(rank, 1)?;
    }
    if rank != triangles {
        return Err(invariant("corner rank inventory length mismatch"));
    }
    let mut corners = corners.finish()?;
    let mut welds = tree.weld_depths.reader()?;
    let mut weld = welds.next().transpose()?;
    let mut previous_position = None;
    let mut depths = ExternalSorter::<RankDepth>::new(scratch, config.sorter(jobs))?;
    let mut corner_count = 0_u64;
    for row in corners.by_ref() {
        let row = row?;
        if previous_position.is_some_and(|p| p != row.position) {
            weld = welds.next().transpose()?;
        }
        let value =
            weld.ok_or_else(|| invariant("corner position missing from complete weld table"))?;
        if value.position != row.position || value.depth as u32 >= tree.stats.height {
            return Err(invariant("corner/weld position/depth mismatch"));
        }
        previous_position = Some(row.position);
        depths.push(RankDepth {
            rank: row.rank,
            corner: row.corner,
            depth: value.depth,
        })?;
        corner_count = add(corner_count, 1)?;
        if corner_count.is_multiple_of(3072) {
            check()?;
        }
    }
    drop(corners);
    if welds.next().transpose()?.is_some() || corner_count != mul(triangles, 3)? {
        return Err(invariant("corner/weld coverage mismatch"));
    }
    drop(welds);
    let mut depths = depths.finish()?;
    let mut writer =
        SequenceWriter::<CornerDepth>::new(scratch, "corner-depths", config.sort.buffer_bytes)?;
    for rank in 0..triangles {
        if rank.is_multiple_of(1024) {
            check()?;
        }
        let mut result = [0; 3];
        for (corner, result_depth) in result.iter_mut().enumerate() {
            let row = depths
                .next()
                .transpose()?
                .ok_or_else(|| invariant("missing ranked corner depth"))?;
            if (row.rank, row.corner) != (rank, corner as u8) {
                return Err(invariant("ranked corner depth order mismatch"));
            }
            *result_depth = row.depth;
        }
        writer.push(&CornerDepth { depths: result })?;
    }
    if depths.next().transpose()?.is_some() {
        return Err(invariant("extra ranked corner depth"));
    }
    drop(depths);
    Ok(writer.finish()?)
}

#[cfg(test)]
pub(super) mod tests {
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
        let weld: Vec<_> = output
            .tree
            .weld_depths
            .reader()
            .unwrap()
            .map(Result::unwrap)
            .collect();
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
                assert_eq!(
                    depths.depths[a],
                    weld.iter().find(|v| v.position == key).unwrap().depth
                );
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
    fn shuffled_grid(parent: &Path, side: usize, seed: u64) -> PathBuf {
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
        let run = |jobs: usize, run_bytes: usize, fan_in: usize| -> Outputs {
            let mut config = TreeConfig::new(Profile::Balanced).with_leaf_triangles(16);
            config.jobs = Jobs::Fixed(NonZeroUsize::new(jobs).unwrap());
            config.scratch_dir = Some(parent.clone());
            config.sort.run_bytes = run_bytes;
            config.sort.fan_in = fan_in;
            config.sort.buffer_bytes = 61;
            let output = capture(&input, &config, &mut || Ok(())).unwrap();
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
            let weld: Vec<_> = output
                .tree
                .weld_depths
                .reader()
                .unwrap()
                .map(|w| w.map(|w| (w.position, w.depth)).unwrap())
                .collect();
            let lookup: std::collections::BTreeMap<_, _> = weld.iter().copied().collect();
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
        }
        assert_eq!(fs::read_dir(parent).unwrap().count(), 0);
    }
    /// The packed rank-order corner record keeps its limits: the largest rank, every corner
    /// and depths up to the tree height cap; more triangles than the
    /// 40-bit rank holds are refused before any record is packed.
    #[test]
    fn rank_depth_round_trips_at_its_limits() {
        for rank in [0, 1, u64::from(u32::MAX) + 7, RANK_LIMIT - 1] {
            for corner in 0..3 {
                for depth in [0, 1, 20, (config::HEIGHT_CAP - 1) as u8] {
                    let row = RankDepth {
                        rank,
                        corner,
                        depth,
                    };
                    let mut bytes = [0u8; RankDepth::SIZE];
                    row.encode(&mut bytes);
                    assert_eq!(RankDepth::decode(&bytes), row);
                }
            }
        }
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
            let retained = RetainedTree {
                nodes,
                weld_depths,
                stats,
            };
            join_corner_depths(&scratch, &retained, &triangles, config, 1, &mut || Ok(()))
                .map(|_| ())
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
        let need = scratch_need(scan, crate::spill::Layout::FULL, &cfg, 4096, 2, 8192).unwrap();
        // 30 corners: CornerRef 750, FullCorner 2910; FullTriangle 2860. One run per sort, so
        // no merge slack: the positioned stream beside the full-triangle sort bounds capture;
        // the full stream and its sequence bound projection (no projected copy is written).
        // Chunks free as they are read, so a stream and its consumer's output trade off.
        assert_eq!(need.stages.capture, 2910 + 2910);
        assert_eq!(need.stages.projection, 2 * 2860);
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
            scratch_need(overflow, crate::spill::Layout::FULL, &cfg, 4096, 2, 8192)
                .unwrap_err()
                .kind(),
            JobErrorKind::ResourceLimit
        );
        let memory = memory_bound(&cfg, 1000, 2, 1, 0).unwrap();
        let large = memory_bound(&cfg, 1000, 2, 1, 150 << 30).unwrap();
        // 150 GiB of live spill is about 19k chunk descriptors.
        assert!(large.chunk_descriptor_bytes > memory.chunk_descriptor_bytes + 19_000 * 64);
        assert_eq!(memory.decoder_allowance_bytes, 192 << 20);
        assert!(memory.bound_bytes > memory.geometry.bound_bytes + memory.decoder_allowance_bytes);
    }
}
