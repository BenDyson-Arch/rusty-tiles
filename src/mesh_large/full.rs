//! Full-detail snapshot owner. Source reads end here; later leaves read only owned
//! spills and admitted material/texture/sampler metadata. No tile output yet.
use super::{
    attributes::{self, FullTriangle},
    capture::{self, SourceScan},
    config::{self, JobsPlan, MemoryBound, ScratchNeed, TreeConfig},
    images::{self, ImageCaptureBounds, ImageSnapshot},
    input::StreamSource,
    leaves::{
        atlas::AtlasResources,
        geometry::LatticePlan,
        planning::{self, LeafPlan},
    },
    records::TriangleRecord,
    tree::{self, TreeOutput},
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

#[derive(Clone, Debug, Serialize)]
pub(crate) struct FullScratchNeed {
    pub h1: ScratchNeed,
    pub full_capture_bytes: u64,
    pub retained_full_bytes: u64,
    pub corner_join_bytes: u64,
    pub leaf_plan_bytes: u64,
    pub image_bytes: u64,
    pub extra_live_files: u64,
    pub logical_need_bytes: u64,
    pub allocation_reserve_bytes: u64,
    pub need_bytes: u64,
}
#[derive(Clone, Debug, Serialize)]
pub(crate) struct FullMemoryBound {
    pub geometry: MemoryBound,
    pub owned_metadata_bytes: u64,
    pub decoder_allowance_bytes: u64,
    pub planning_allowance_bytes: u64,
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
    pub scratch_memory_backed: bool,
    /// Local encoding contract version: dataset-wide zero origin/aligned lattice.
    pub lattice_version: u32,
}

pub(crate) struct FullCaptured {
    pub tree: TreeOutput,
    pub triangles: Sequence<FullTriangle>,
    pub corner_depths: Sequence<CornerDepth>,
    pub leaf_plans: Sequence<LeafPlan>,
    pub images: Vec<ImageSnapshot>,
    pub image_info: Vec<ImageInfo>,
    pub materials: Vec<Material>,
    pub textures: Vec<Value>,
    pub samplers: Vec<Value>,
    pub lattice: LatticePlan,
    pub scan: SourceScan,
    pub report: FullCaptureReport,
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
    config: &TreeConfig,
    unit: u64,
    image_count: u64,
    image_bytes: u64,
) -> Result<FullScratchNeed, JobError> {
    let h1 = config::scratch_need_parts(scan.triangles, scan.geometry_bytes, config, unit)
        .ok_or_else(|| exhausted("full capture H1 scratch arithmetic overflow"))?;
    let full_capture_bytes = mul(scan.triangles, 863)?;
    let retained_full_bytes = mul(scan.triangles, FullTriangle::SIZE as u64)?;
    let corner_join_bytes = mul(
        scan.triangles,
        6 * (RankCorner::SIZE + RankDepth::SIZE) as u64 + CornerDepth::SIZE as u64,
    )?;
    // Every leaf holds at least one triangle. Include the owned plan sequence
    // before the immutable lattice and published leaf bytes exist.
    let leaf_plan_bytes = mul(scan.triangles, LeafPlan::SIZE as u64)?;
    let extra_logical = add(
        add(
            add(full_capture_bytes, retained_full_bytes)?,
            corner_join_bytes,
        )?,
        add(image_bytes, leaf_plan_bytes)?,
    )?;
    let capacity = ((config.sort.run_bytes / 2) / size_of::<FullTriangle>()).max(1) as u64;
    let counted = add(
        mul(add(mul(scan.triangles, 16)?.div_ceil(capacity), 5)?, 2)?,
        32,
    )?;
    let inventory = add(mul(config.sort.fan_in as u64, 256)?, 32)?;
    // Add image sequences and retained full/depth sequences outside run inventory.
    let extra_live_files = add(add(inventory.min(counted), image_count)?, 5)?;
    let extra_reserve = add(
        add(mul(add(unit, 64 << 10)?, extra_live_files)?, 1 << 20)?,
        extra_logical,
    )?;
    let logical_need_bytes = add(h1.logical_need_bytes, extra_logical)?;
    let allocation_reserve_bytes = add(h1.allocation_reserve_bytes, extra_reserve)?;
    let need_bytes = add(logical_need_bytes, allocation_reserve_bytes)?;
    Ok(FullScratchNeed {
        h1,
        full_capture_bytes,
        retained_full_bytes,
        corner_join_bytes,
        leaf_plan_bytes,
        image_bytes,
        extra_live_files,
        logical_need_bytes,
        allocation_reserve_bytes,
        need_bytes,
    })
}
fn memory_bound(
    config: &TreeConfig,
    json_bytes: u64,
    image_count: u64,
    jobs: usize,
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
    let bound_bytes = add(
        add(geometry.bound_bytes, owned_metadata_bytes)?,
        add(DECODER_ALLOWANCE, PLANNING_ALLOWANCE)?,
    )?;
    Ok(FullMemoryBound {
        geometry,
        owned_metadata_bytes,
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
    super::member_gate(
        config::tile_lower_bound(
            source.document.triangle_count() as u64,
            config.leaf_triangles(),
        ),
        config.member_ceiling,
    )
    .map_err(super::failure)?;
    let scan = capture::scan(&source, config.sort.buffer_bytes, check).map_err(super::failure)?;
    let (image_count, image_bytes) = image_preflight(&source, check).map_err(super::failure)?;
    let parent = config
        .scratch_dir
        .clone()
        .unwrap_or_else(Scratch::default_parent);
    let volume = crate::spill::inspect_volume(&parent).map_err(|e| super::failure(e.into()))?;
    let need = scratch_need(
        scan,
        config,
        volume.allocation_unit_bytes,
        image_count,
        image_bytes,
    )
    .map_err(super::failure)?;
    let one = memory_bound(config, source.json_bytes, image_count, 1).map_err(super::failure)?;
    let jobs = config::plan_jobs(
        config.jobs,
        std::thread::available_parallelism().map_or(1, |n| n.get()),
        config::available_memory(),
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
    let memory =
        memory_bound(config, source.json_bytes, image_count, workers).map_err(super::failure)?;
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
    let scratch = Scratch::create(config.scratch_dir.as_deref(), need.need_bytes)
        .map_err(|e| super::failure(e.into()))?;
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
        config::preflight_remaining(&scratch, need.need_bytes)?;
        let mut full_stream =
            attributes::capture_full(&source, &scratch, config.sorter(workers), &scan, check)?;
        let mut full_writer = SequenceWriter::<FullTriangle>::new(
            &scratch,
            "full-triangles",
            config.sort.buffer_bytes,
        )?;
        let mut projection =
            ExternalSorter::<TriangleRecord>::new(&scratch, config.sorter(workers))?;
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
            projection.push(row.triangle)?;
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
        let (images, image_capture) =
            images::capture_images(&source, &scratch, config.sort.buffer_bytes, check)?;
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
        let materials = source.document.materials().to_vec();
        let textures = source.document.textures().to_vec();
        let samplers = source.document.samplers().to_vec();
        source.verify_unchanged()?;
        drop(source); // All subsequent operations read owned snapshot records only.
        config::preflight_remaining(&scratch, need.need_bytes)?;
        let retained_allowance = need
            .need_bytes
            .checked_sub(need.h1.need_bytes)
            .ok_or_else(|| invariant("full capture retained scratch allowance underflow"))?;
        let tree = tree::build(
            &scratch,
            projection.finish()?,
            config,
            workers,
            retained_allowance,
            check,
        )?;
        super::member_gate(tree.stats.nodes, config.member_ceiling)?;
        verify_rank_alignment(&triangles, &tree.triangles, check)?;
        config::preflight_remaining(&scratch, need.need_bytes)?;
        let corner_depths = join_corner_depths(&scratch, &tree, config, workers, check)?;
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
        let resources = AtlasResources {
            materials: &materials,
            textures: &textures,
            samplers: &samplers,
            image_info: &image_info,
        };
        let leaf_plans =
            planning::preflight(&scratch, &triangles, &tree.nodes, resources, config, check)?;
        if leaf_plans.len() != tree.stats.leaves {
            return Err(invariant("leaf plan count differs from immutable tree"));
        }
        let lattice = LatticePlan::new(config.profile, scan.bounds, &extents)?;
        config::preflight_remaining(&scratch, need.need_bytes)?;
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
            scratch: need,
            image_capture,
            lattice_version: 1,
        };
        Ok((
            tree,
            triangles,
            corner_depths,
            leaf_plans,
            images,
            image_info,
            materials,
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
            textures,
            samplers,
            lattice,
            scan,
            report,
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

fn verify_rank_alignment(
    full: &Sequence<FullTriangle>,
    projected: &Sequence<TriangleRecord>,
    check: &mut dyn FnMut() -> Result<(), JobError>,
) -> Result<(), JobError> {
    if full.len() != projected.len() {
        return Err(invariant("full/tree rank inventory length mismatch"));
    }
    let mut a = full.reader()?;
    let mut b = projected.reader()?;
    let mut left = [0_u8; TriangleRecord::SIZE];
    let mut right = [0_u8; TriangleRecord::SIZE];
    for rank in 0..full.len() {
        if rank.is_multiple_of(1024) {
            check()?;
        }
        let f = a
            .next()
            .transpose()?
            .ok_or_else(|| invariant("missing full rank"))?;
        let t = b
            .next()
            .transpose()?
            .ok_or_else(|| invariant("missing tree rank"))?;
        f.triangle.encode(&mut left);
        t.encode(&mut right);
        if left != right {
            return Err(invariant("full/tree rank projected wire mismatch"));
        }
    }
    if a.next().transpose()?.is_some() || b.next().transpose()?.is_some() {
        return Err(invariant("extra full/tree rank"));
    }
    Ok(())
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
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct RankDepth {
    rank: u64,
    corner: u8,
    depth: u8,
}
impl Record for RankDepth {
    const SIZE: usize = 10;
    fn encode(&self, out: &mut [u8]) {
        out[..8].copy_from_slice(&self.rank.to_le_bytes());
        out[8] = self.corner;
        out[9] = self.depth;
    }
    fn decode(bytes: &[u8]) -> Self {
        Self {
            rank: u64::from_le_bytes(bytes[..8].try_into().unwrap()),
            corner: bytes[8],
            depth: bytes[9],
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
fn join_corner_depths(
    scratch: &Scratch,
    tree: &TreeOutput,
    config: &TreeConfig,
    jobs: usize,
    check: &mut dyn FnMut() -> Result<(), JobError>,
) -> Result<Sequence<CornerDepth>, JobError> {
    let mut corners = ExternalSorter::<RankCorner>::new(scratch, config.sorter(jobs))?;
    let mut rank = 0_u64;
    for triangle in tree.triangles.reader()? {
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
    if rank != tree.triangles.len() {
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
    if welds.next().transpose()?.is_some() || corner_count != mul(tree.triangles.len(), 3)? {
        return Err(invariant("corner/weld coverage mismatch"));
    }
    drop(welds);
    let mut depths = depths.finish()?;
    let mut writer =
        SequenceWriter::<CornerDepth>::new(scratch, "corner-depths", config.sort.buffer_bytes)?;
    for rank in 0..tree.triangles.len() {
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
        assert!(
            captured.report.image_capture.scratch_bytes > captured.report.scratch.h1.need_bytes,
            "control must exceed the H1-only owner allowance"
        );
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
        verify_rank_alignment(&output.triangles, &output.tree.triangles, &mut || Ok(())).unwrap();
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
            .tree
            .triangles
            .reader()
            .unwrap()
            .zip(output.corner_depths.reader().unwrap())
        {
            let triangle = triangle.unwrap();
            let depths = depths.unwrap();
            for a in 0..3 {
                let key = triangle.positions[a].map(f32::to_bits);
                assert_eq!(
                    depths.depths[a],
                    weld.iter().find(|v| v.position == key).unwrap().depth
                );
            }
        }
        let material = output.materials[0]
            .remap(&[(0, 0)].into_iter().collect())
            .unwrap();
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
        let need = scratch_need(scan, &cfg, 4096, 2, 8192).unwrap();
        assert_eq!(need.full_capture_bytes, 8630);
        assert_eq!(need.retained_full_bytes, 2860);
        assert_eq!(need.corner_join_bytes, 1890);
        assert!(need.need_bytes > need.logical_need_bytes);
        let overflow = SourceScan {
            triangles: u64::MAX,
            ..scan
        };
        assert_eq!(
            scratch_need(overflow, &cfg, 4096, 2, 8192)
                .unwrap_err()
                .kind(),
            JobErrorKind::ResourceLimit
        );
        let memory = memory_bound(&cfg, 1000, 2, 1).unwrap();
        assert_eq!(memory.decoder_allowance_bytes, 192 << 20);
        assert!(memory.bound_bytes > memory.geometry.bound_bytes + memory.decoder_allowance_bytes);
    }
}
