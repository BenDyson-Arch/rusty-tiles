//! Image-major leaf crops.
//!
//! Before any leaf is built, [`plan`] re-projects every leaf at its planned density (the
//! layout final preparation builds) and collects the crops its CLAMP pages read
//! ([`CropRequest`]). [`build`] then decodes each source image once, cuts all of its crops
//! (at each leaf's density level, with the binding's wrap) and appends them, as
//! vertical-predictor residuals compressed with the spill block codec, to one scratch log.
//! A leaf page the store covers reads its crops from the log ([`CropStore::crop`]) instead
//! of a decoded source image.
//!
//! Contract:
//! - A stored crop is byte-identical to what [`crops::copy_crop_tiled`] cuts from the decoded
//!   source (or its density level): both call [`crops::copy_crop_pixels`] on the same texels.
//!   The store changes no output.
//! - A page the store does not cover (a byte-budget density retry, a period page, a MASK
//!   binding) cuts from the decoded-source cache as before; uncovered pages are counted as
//!   misses.
//! - Memory: images are cut in fixed batches whose cuts ([`image_charge`]) fit the memory
//!   the caller gives, plus the index ([`INDEX_BYTES_PER_CROP`] per crop), both planned by
//!   the leaf stage.
//! - Scratch: at most the crops' raw bytes; the codec never stores a block larger than its
//!   raw bytes, and the log refuses to grow past them. The caller admits that bound before
//!   the first write.
use super::crops::{self, Crop, SourceSampler};
use super::mips::{self, Class};
use crate::mesh_large::full::FullCaptured;
use crate::mesh_large::hlod::sparse;
use crate::spill::codec;
use crate::{JobError, JobErrorKind};
use std::collections::BTreeMap;
use std::sync::atomic::{AtomicU64, Ordering};

/// One crop a leaf page reads: the source image, its sampling class and density level, the
/// binding's wrap and the crop rectangle at that level.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct CropRequest {
    pub image: u32,
    /// [`class_rank`] of the binding's sampling class.
    pub class: u8,
    pub drop: u8,
    pub wrap: [u32; 2],
    pub origin: [i64; 2],
    pub width: u32,
    pub height: u32,
    pub gutter: u32,
    pub interior: [u32; 2],
}

impl CropRequest {
    pub(crate) fn new(
        image: usize,
        class: Class,
        drop: u32,
        sampler: SourceSampler,
        crop: &Crop,
    ) -> Result<Self, JobError> {
        Ok(Self {
            image: u32::try_from(image).map_err(|_| state("crop image index"))?,
            class: class_rank(class),
            drop: u8::try_from(drop).map_err(|_| state("crop density level"))?,
            wrap: [sampler.wrap_s, sampler.wrap_t],
            origin: crop.origin,
            width: crop.width,
            height: crop.height,
            gutter: crop.gutter,
            interior: [crop.interior_width, crop.interior_height],
        })
    }
    fn crop(&self) -> Crop {
        Crop {
            origin: self.origin,
            width: self.width,
            height: self.height,
            gutter: self.gutter,
            interior_width: self.interior[0],
            interior_height: self.interior[1],
        }
    }
    /// The crop's RGBA8 bytes.
    pub(crate) fn raw_bytes(&self) -> u64 {
        u64::from(self.width) * u64::from(self.height) * 4
    }
    fn sampler(&self) -> SourceSampler {
        SourceSampler {
            wrap_s: self.wrap[0],
            wrap_t: self.wrap[1],
            // Cutting reads texels only; filters do not apply.
            min_filter: 9729,
            mag_filter: 9729,
        }
    }
}

pub(crate) fn class_rank(class: Class) -> u8 {
    match class {
        Class::Colour => 0,
        Class::Normal => 1,
        Class::Data => 2,
        Class::Id => 3,
    }
}
fn class_of(rank: u8) -> Result<Class, JobError> {
    match rank {
        0 => Ok(Class::Colour),
        1 => Ok(Class::Normal),
        2 => Ok(Class::Data),
        3 => Ok(Class::Id),
        _ => Err(state("crop class rank")),
    }
}

fn state(message: &str) -> JobError {
    JobError::new(
        JobErrorKind::InvalidState,
        format!("leaf crop store: {message}"),
    )
}

/// Index bytes charged per stored crop: the key, its entry and the ordered map's node
/// overhead (also covers the request list the index is built from).
pub(crate) const INDEX_BYTES_PER_CROP: u64 = 160;

/// Bytes one image's cut holds until its blocks are written: the encoded image and its
/// decode (16 bytes per texel, beyond the decoded-source cache's 12), then the larger of
/// its two phases: every class chain the leaf density levels need (16 bytes per texel
/// each, beyond the leaf chain builder's input copy and levels) and the crop being cut
/// twice (cut and predicted); or, with `coarse` tiles, a copy of the image and the coarse
/// chain built from it with one tile being cut twice. Every encoded crop and tile is held
/// (at most its raw bytes) until the batch is written.
pub(crate) fn image_charge(
    info: &crate::gltf_source::source::ImageInfo,
    chains: u64,
    crop_bytes: u64,
    largest_crop: u64,
    coarse: Option<u64>,
) -> u64 {
    let texels = u64::from(info.width) * u64::from(info.height);
    let leaf = texels
        .saturating_mul(16)
        .saturating_mul(chains)
        .saturating_add(largest_crop.saturating_mul(2));
    let coarse_phase = coarse.map_or(0, |_| {
        texels
            .saturating_mul(4)
            .saturating_add(crate::mesh_large::hlod::bake::chain_bytes(
                info.width,
                info.height,
            ))
            .saturating_add(2 * u64::from(sparse::EDGE) * u64::from(sparse::EDGE) * 4)
    });
    texels
        .saturating_mul(16)
        .saturating_add(info.encoded_bytes)
        .saturating_add(leaf.max(coarse_phase))
        .saturating_add(crop_bytes)
        .saturating_add(coarse.unwrap_or(0))
}

#[derive(Clone, Copy, Debug)]
struct Entry {
    offset: u64,
    len: u32,
    stored: bool,
}

/// What the store reports (diagnostics only; nothing here reaches the archive).
#[derive(Clone, Copy, Debug, Default, serde::Serialize)]
pub struct CropStoreStats {
    pub crops: u64,
    pub images: u64,
    /// Raw RGBA8 bytes of every stored crop: the admitted scratch bound.
    pub raw_bytes: u64,
    pub written_bytes: u64,
    /// Bytes the concurrent image cuts were allowed, and the most they leased at once.
    pub memory_bytes: u64,
    pub memory_peak_bytes: u64,
    pub index_bytes: u64,
    pub plan_seconds: f64,
    pub build_seconds: f64,
    /// Crop reads the store could not serve (cut from the decoded source instead).
    pub misses: u64,
    /// The coarse stage's sparse chains: chains, stored tiles of all their tiles, raw
    /// (admitted) and written bytes and index bytes (zero when the coarse stage samples
    /// decoded sources).
    pub coarse_chains: u64,
    pub coarse_tiles: u64,
    pub coarse_chain_tiles: u64,
    pub coarse_raw_bytes: u64,
    pub coarse_written_bytes: u64,
    pub coarse_index_bytes: u64,
}

/// The cut crops: one scratch log and its index.
pub(crate) struct CropStore {
    log: crate::spill::ByteFile,
    index: BTreeMap<CropRequest, Entry>,
    stats: CropStoreStats,
    misses: AtomicU64,
}

impl CropStore {
    /// The stored crop for `key`, or `None` when the store lacks it.
    pub(crate) fn crop(&self, key: &CropRequest) -> Result<Option<Vec<u8>>, JobError> {
        let Some(entry) = self.index.get(key) else {
            self.misses.fetch_add(1, Ordering::Relaxed);
            return Ok(None);
        };
        let len = usize::try_from(entry.len).map_err(|_| state("crop block size"))?;
        let mut block = Vec::new();
        block
            .try_reserve_exact(len)
            .map_err(|_| resource("stored crop read"))?;
        block.resize(len, 0);
        self.log.read_at(entry.offset, &mut block)?;
        decode_block(&block, entry.stored, key).map(Some)
    }
    /// Whether every crop in `keys` is stored; a page reads the store only then.
    pub(crate) fn covers(&self, keys: &[CropRequest]) -> bool {
        let covered = keys.iter().all(|k| self.index.contains_key(k));
        if !covered {
            self.misses.fetch_add(1, Ordering::Relaxed);
        }
        covered
    }
    pub(crate) fn stats(&self) -> CropStoreStats {
        CropStoreStats {
            misses: self.misses.load(Ordering::Relaxed),
            ..self.stats
        }
    }
}

/// One stored block back to the crop's RGBA8 texels; a block that does not decode to
/// exactly the crop's bytes is corrupt (`InvalidState`).
fn decode_block(block: &[u8], stored: bool, key: &CropRequest) -> Result<Vec<u8>, JobError> {
    let (w, h) = (
        usize::try_from(key.width).map_err(|_| state("crop width"))?,
        usize::try_from(key.height).map_err(|_| state("crop height"))?,
    );
    unpack(block, stored, w, h)
}

/// A `width`×`height` RGBA8 block as stored: its vertical-predictor residual through the
/// spill block codec.
pub(crate) fn pack(
    rgba: &[u8],
    width: usize,
    height: usize,
    scratch: &mut Vec<u8>,
) -> codec::Encoded {
    codec::encode(&predict(rgba, width, height), scratch)
}

/// [`pack`]'s inverse; a block that does not decode to exactly `width`×`height` texels is
/// corrupt (`InvalidState`).
pub(crate) fn unpack(
    block: &[u8],
    stored: bool,
    width: usize,
    height: usize,
) -> Result<Vec<u8>, JobError> {
    let raw = width
        .checked_mul(height)
        .and_then(|n| n.checked_mul(4))
        .ok_or_else(|| state("block size"))?;
    let mut residual = Vec::new();
    residual
        .try_reserve_exact(raw)
        .map_err(|_| resource("stored block decode"))?;
    residual.resize(raw, 0);
    codec::decode(block, stored, &mut residual)
        .map_err(|e| state(&format!("stored block is corrupt: {e}")))?;
    Ok(unpredict(residual, width, height))
}

#[cfg(test)]
thread_local! {
    /// Tests only: plan no store on this thread (the stage then reads crops from the
    /// decoded sources), to compare outputs with and without it.
    pub(crate) static DISABLED: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    /// Tests only: the scratch left on this thread's volume for the store's crops beyond
    /// the leaf stage's own image scratch.
    pub(crate) static STORE_SCRATCH_FREE: std::cell::Cell<Option<u64>> =
        const { std::cell::Cell::new(None) };
}

fn resource(what: &str) -> JobError {
    JobError::new(
        JobErrorKind::ResourceLimit,
        format!("leaf crop store: {what} allocation"),
    )
}

/// Planar channels, each row minus the row above (wrapping): photographic rows are close,
/// so the block codec finds more to compress. Exactly inverted by [`unpredict`].
fn predict(rgba: &[u8], width: usize, height: usize) -> Vec<u8> {
    let n = width * height;
    let mut out = vec![0u8; n * 4];
    for c in 0..4 {
        let plane = &mut out[c * n..(c + 1) * n];
        for (v, px) in plane.iter_mut().zip(rgba.as_chunks::<4>().0) {
            *v = px[c];
        }
        for y in (1..height).rev() {
            let (above, row) = plane.split_at_mut(y * width);
            for (r, a) in row[..width].iter_mut().zip(&above[(y - 1) * width..]) {
                *r = r.wrapping_sub(*a);
            }
        }
    }
    out
}
fn unpredict(mut planes: Vec<u8>, width: usize, height: usize) -> Vec<u8> {
    let n = width * height;
    let mut rgba = vec![0u8; n * 4];
    for c in 0..4 {
        let plane = &mut planes[c * n..(c + 1) * n];
        for y in 1..height {
            let (above, row) = plane.split_at_mut(y * width);
            for (r, a) in row[..width].iter_mut().zip(&above[(y - 1) * width..]) {
                *r = r.wrapping_add(*a);
            }
        }
        for (px, v) in rgba.as_chunks_mut::<4>().0.iter_mut().zip(plane.iter()) {
            px[c] = *v;
        }
    }
    rgba
}

/// Every leaf's crops, sorted by image and deduplicated, and what cutting them needs.
pub(crate) struct CropPlan {
    pub requests: Vec<CropRequest>,
    /// Raw bytes of every crop: the leaf log's scratch bound.
    pub scratch_bytes: u64,
    /// The crop index's memory ([`INDEX_BYTES_PER_CROP`] per crop).
    pub index_bytes: u64,
    pub seconds: f64,
    /// The coarse stage's sparse chains, cut beside the crops.
    pub coarse: Option<CoarsePlan>,
}

/// The tiles of every chain the coarse stage samples ([`sparse`]).
pub(crate) struct CoarsePlan {
    pub needs: sparse::Needs,
    /// Raw bytes of every marked tile: the coarse log's scratch bound.
    pub scratch_bytes: u64,
    /// The chains' index memory, held until the coarse stage ends.
    pub index_bytes: u64,
}

impl CropPlan {
    /// Scratch of both logs.
    pub(crate) fn total_scratch_bytes(&self) -> u64 {
        self.scratch_bytes
            .saturating_add(self.coarse.as_ref().map_or(0, |c| c.scratch_bytes))
    }
    /// Index memory of both stores.
    pub(crate) fn total_index_bytes(&self) -> u64 {
        self.index_bytes
            .saturating_add(self.coarse.as_ref().map_or(0, |c| c.index_bytes))
    }
    /// The largest single image's cut ([`image_charge`]) beside the logs' write buffers:
    /// the least memory the build runs in.
    pub(crate) fn largest_image_bytes(&self, captured: &FullCaptured) -> Result<u64, JobError> {
        Ok(image_groups(captured, self)?
            .iter()
            .map(|g| g.charge)
            .max()
            .unwrap_or(0)
            .saturating_add(LOG_BUFFER_BYTES.saturating_mul(2)))
    }
    /// The plan without its coarse chains (the coarse stage then decodes its sources).
    pub(crate) fn without_coarse(self) -> Self {
        Self {
            coarse: None,
            ..self
        }
    }
}

/// One image's cut: its crops (a range of the sorted requests), its coarse chains and
/// their memory.
struct Group<'p> {
    image: usize,
    charge: u64,
    crops: std::ops::Range<usize>,
    coarse: Vec<(u8, &'p sparse::Marks)>,
}

/// Every image with crops or coarse tiles to cut, in image order.
fn image_groups<'p>(
    captured: &FullCaptured,
    plan: &'p CropPlan,
) -> Result<Vec<Group<'p>>, JobError> {
    let requests = &plan.requests;
    let coarse = plan.coarse.as_ref().map(|c| &c.needs.chains);
    let mut images: Vec<u32> = requests.iter().map(|r| r.image).collect();
    images.extend(coarse.into_iter().flat_map(|c| c.keys().map(|k| k.0)));
    images.sort_unstable();
    images.dedup();
    let mut groups = Vec::with_capacity(images.len());
    let mut start = 0;
    for image in images {
        let end = start
            + requests[start..]
                .iter()
                .take_while(|r| r.image == image)
                .count();
        let info = captured
            .image_info
            .get(usize::try_from(image).map_err(|_| state("crop image index"))?)
            .ok_or_else(|| state("crop image missing"))?;
        let mut chains: Vec<u8> = Vec::new();
        let (mut largest, mut total) = (0u64, 0u64);
        for r in &requests[start..end] {
            largest = largest.max(r.raw_bytes());
            total = total.saturating_add(r.raw_bytes());
            if r.drop > 0 && !chains.contains(&r.class) {
                chains.push(r.class);
            }
        }
        let marks: Vec<(u8, &sparse::Marks)> = coarse
            .into_iter()
            .flat_map(|c| c.range((image, 0)..=(image, u8::MAX)))
            .map(|(k, m)| (k.1, m))
            .collect();
        let tiles = (!marks.is_empty()).then(|| {
            marks
                .iter()
                .map(|(_, m)| m.marked().1)
                .fold(0u64, u64::saturating_add)
        });
        groups.push(Group {
            image: usize::try_from(image).map_err(|_| state("crop image index"))?,
            charge: image_charge(info, count(chains.len()), total, largest, tiles),
            crops: start..end,
            coarse: marks,
        });
        start = end;
    }
    if start != requests.len() {
        return Err(state("crop requests are not sorted by image"));
    }
    Ok(groups)
}

/// Re-projects every leaf at its planned density (the layout final preparation builds) and
/// collects its store crops, and (with `coarse`) marks every chain tile a coarse sample can
/// read ([`sparse::Needs`]). `memory` bounds what the pass holds: the request list (counted
/// before deduplication, as it is held) beside one leaf's working set, `leaf_bytes` (its
/// triangles and their layout). `None` when that would exceed `memory` (the stage then reads
/// crops from the decoded sources, as without the store); the plan has no coarse chains
/// when their index would not fit beside the crops'.
pub(crate) fn plan(
    captured: &FullCaptured,
    profile: crate::mesh_large::config::Profile,
    memory: u64,
    leaf_bytes: u64,
    coarse: bool,
    progress: &crate::mesh_large::phases::StepProgress<'_>,
) -> Result<Option<CropPlan>, JobError> {
    let started = std::time::Instant::now();
    if leaf_bytes > memory {
        return Ok(None);
    }
    let resources = super::atlas::AtlasResources {
        materials: &captured.materials,
        classes: &captured.material_classes,
        textures: &captured.textures,
        samplers: &captured.samplers,
        image_info: &captured.image_info,
    };
    let mut needs = coarse.then(|| sparse::Needs::new(captured)).transpose()?;
    let mut triangles = captured.triangles.reader()?;
    let mut requests: Vec<CropRequest> = Vec::new();
    progress.begin(
        crate::mesh_large::phases::Step::LeafCrops,
        captured.leaf_plans.len(),
    )?;
    for (done, plan) in captured.leaf_plans.reader()?.enumerate() {
        let plan = plan?;
        progress.advance(count(done))?;
        let triangle_count =
            usize::try_from(plan.end - plan.first).map_err(|_| state("leaf size"))?;
        let mut leaf = Vec::new();
        leaf.try_reserve_exact(triangle_count)
            .map_err(|_| resource("leaf triangles"))?;
        for i in 0..triangle_count {
            if i.is_multiple_of(4096) {
                progress.check()?;
            }
            let triangle = triangles
                .next()
                .transpose()?
                .ok_or_else(|| state("leaf triangles truncated"))?;
            if let Some(needs) = &mut needs {
                needs.mark(&captured.image_info, &triangle)?;
            }
            leaf.push(triangle);
        }
        let keys = super::atlas::crop_requests(
            leaf,
            resources,
            profile,
            u32::from(plan.density_drop),
            &mut || progress.check(),
        )?;
        let held = count(requests.len() + keys.len()).saturating_mul(INDEX_BYTES_PER_CROP);
        if held.saturating_add(leaf_bytes) > memory {
            progress.end()?;
            return Ok(None);
        }
        requests.extend(keys);
    }
    progress.end()?;
    requests.sort_unstable();
    requests.dedup();
    let scratch_bytes = requests
        .iter()
        .try_fold(0u64, |n, r| n.checked_add(r.raw_bytes()))
        .ok_or_else(|| state("crop scratch overflow"))?;
    let index_bytes = count(requests.len()).saturating_mul(INDEX_BYTES_PER_CROP);
    let coarse = needs
        .map(|needs| CoarsePlan {
            scratch_bytes: needs
                .chains
                .values()
                .map(|m| m.marked().1)
                .fold(0u64, u64::saturating_add),
            index_bytes: needs.index_bytes(),
            needs,
        })
        // No chain to cut (no triangle binds an image), or no room for their index.
        .filter(|c| {
            !c.needs.chains.is_empty() && index_bytes.saturating_add(c.index_bytes) <= memory
        });
    Ok(Some(CropPlan {
        scratch_bytes,
        index_bytes,
        seconds: started.elapsed().as_secs_f64(),
        requests,
        coarse,
    }))
}

/// A count as `u64` (saturating; counts here never approach it).
fn count(n: usize) -> u64 {
    u64::try_from(n).unwrap_or(u64::MAX)
}

/// Each log's write buffer, held through the build beside the image cuts.
pub(crate) const LOG_BUFFER_BYTES: u64 = 1 << 20;
/// How long the build's caller waits for a finished image before its next heartbeat; well
/// under [`crate::mesh_large::phases::STEP_HEARTBEAT`].
const HEARTBEAT_WAIT: std::time::Duration = std::time::Duration::from_millis(100);

fn lock<T>(m: &std::sync::Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// The error a cut returns when another image of its batch failed first.
fn stopped_by_sibling() -> JobError {
    JobError::new(
        JobErrorKind::Cancelled,
        "leaf crop store: stopped after another image failed",
    )
}

/// One image's cut: its crops' blocks and, per coarse class, its chain's tiles.
type Cut = (
    Vec<(CropRequest, codec::Encoded)>,
    Vec<(u8, Vec<(usize, usize, codec::Encoded)>)>,
);

/// Cuts every crop and coarse tile of `plan` image by image on the caller's thread pool.
/// Images go in fixed batches whose cuts together lease at most `memory` bytes less the
/// logs' buffers (each at least one image, which the caller has checked fits), so output
/// order and peak memory do not depend on the worker count. While a batch runs, the caller
/// reports progress and keeps the step's heartbeat; the first image that fails stops the
/// rest of its batch. Crop cuts take `page_memory`, the leaf stage's page workspace, as
/// their memory limit, as pages cutting from decoded sources do. Neither log grows past
/// its plan's raw bytes.
pub(crate) fn build(
    captured: &FullCaptured,
    plan: CropPlan,
    memory: u64,
    page_memory: u64,
    pool: &rayon::ThreadPool,
    progress: &crate::mesh_large::phases::StepProgress<'_>,
    check: &(dyn Fn() -> Result<(), JobError> + Sync),
) -> Result<(CropStore, Option<sparse::Chains>), JobError> {
    let started = std::time::Instant::now();
    let groups = image_groups(captured, &plan)?;
    let memory = memory.saturating_sub(LOG_BUFFER_BYTES.saturating_mul(2));
    if groups.iter().any(|g| g.charge > memory) {
        return Err(JobError::new(
            JobErrorKind::ResourceLimit,
            "leaf crop store: one source image's cut exceeds its memory",
        ));
    }
    let refused = || {
        JobError::new(
            JobErrorKind::ResourceLimit,
            "leaf crop store exceeded its admitted scratch",
        )
    };
    let buffer = usize::try_from(LOG_BUFFER_BYTES).map_err(|_| state("log buffer"))?;
    let mut writer = crate::spill::ByteWriter::new(&captured.scratch, "leaf-crops", buffer)?;
    let mut coarse_writer = plan
        .coarse
        .as_ref()
        .map(|_| crate::spill::ByteWriter::new(&captured.scratch, "coarse-tiles", buffer))
        .transpose()?;
    let mut index = BTreeMap::new();
    let mut chains: BTreeMap<(u32, u8), sparse::Chain> = BTreeMap::new();
    let mut coarse_tiles = 0u64;
    let mut peak = 0u64;
    progress.begin(
        crate::mesh_large::phases::Step::LeafCropStore,
        count(groups.len()),
    )?;
    let mut done = 0usize;
    while done < groups.len() {
        // The next batch: images in order while their cuts fit `memory` together.
        let mut end = done;
        let mut leased = 0u64;
        while end < groups.len() && (end == done || leased + groups[end].charge <= memory) {
            leased += groups[end].charge;
            end += 1;
        }
        peak = peak.max(leased);
        let cuts = cut_batch(
            captured,
            &groups[done..end],
            &plan.requests,
            page_memory,
            (pool, progress, done),
            check,
        )?;
        for (group, (crops, coarse)) in groups[done..end].iter().zip(cuts) {
            for (key, block) in crops {
                let offset = writer.len();
                if offset.saturating_add(count(block.bytes.len())) > plan.scratch_bytes {
                    return Err(refused());
                }
                writer.append(&block.bytes)?;
                index.insert(
                    key,
                    Entry {
                        offset,
                        len: u32::try_from(block.bytes.len())
                            .map_err(|_| state("crop block size"))?,
                        stored: block.stored,
                    },
                );
            }
            let (Some(log), Some(bound)) = (
                coarse_writer.as_mut(),
                plan.coarse.as_ref().map(|c| c.scratch_bytes),
            ) else {
                continue;
            };
            for ((class, tiles), (_, marks)) in coarse.into_iter().zip(&group.coarse) {
                let mut chain = sparse::Chain::new(marks);
                for (level, tile, block) in tiles {
                    let offset = log.len();
                    if offset.saturating_add(count(block.bytes.len())) > bound {
                        return Err(refused());
                    }
                    log.append(&block.bytes)?;
                    coarse_tiles += 1;
                    let slot = chain
                        .levels
                        .get_mut(level)
                        .and_then(|l| l.1.get_mut(tile))
                        .ok_or_else(|| state("coarse tile outside its chain"))?;
                    *slot = Some(sparse::Entry {
                        offset,
                        len: u32::try_from(block.bytes.len())
                            .map_err(|_| state("coarse tile block size"))?,
                        stored: block.stored,
                    });
                }
                let image = u32::try_from(group.image).map_err(|_| state("crop image index"))?;
                chains.insert((image, class), chain);
            }
        }
        done = end;
        progress.advance(count(done))?;
    }
    progress.end()?;
    let written = writer.len();
    let log = writer.finish()?;
    let coarse_written = coarse_writer.as_ref().map_or(0, |w| w.len());
    let coarse = plan.coarse.as_ref();
    let stats = CropStoreStats {
        crops: count(index.len()),
        images: count(groups.len()),
        raw_bytes: plan.scratch_bytes,
        written_bytes: written,
        memory_bytes: memory,
        memory_peak_bytes: peak,
        index_bytes: plan.index_bytes,
        plan_seconds: plan.seconds,
        build_seconds: started.elapsed().as_secs_f64(),
        misses: 0,
        coarse_chains: count(chains.len()),
        coarse_tiles,
        coarse_chain_tiles: coarse.map_or(0, |c| c.needs.chains.values().map(|m| m.tiles()).sum()),
        coarse_raw_bytes: coarse.map_or(0, |c| c.scratch_bytes),
        coarse_written_bytes: coarse_written,
        coarse_index_bytes: coarse.map_or(0, |c| c.index_bytes),
    };
    let coarse = coarse_writer
        .map(|w| -> Result<sparse::Chains, JobError> {
            Ok(sparse::Chains {
                log: w.finish()?,
                chains,
                decodes: Default::default(),
            })
        })
        .transpose()?;
    Ok((
        CropStore {
            stats,
            log,
            index,
            misses: AtomicU64::new(0),
        },
        coarse,
    ))
}

/// One batch's cuts, in image order, run on `pool` while the caller (`progress`, at step
/// count `done` before the batch) reports each finished image and keeps the heartbeat.
/// The first failing image (or the caller's own check) stops the batch's other cuts; the
/// error returned is the caller's, else the first real failure in image order.
fn cut_batch(
    captured: &FullCaptured,
    batch: &[Group<'_>],
    requests: &[CropRequest],
    page_memory: u64,
    (pool, progress, done): (
        &rayon::ThreadPool,
        &crate::mesh_large::phases::StepProgress<'_>,
        usize,
    ),
    check: &(dyn Fn() -> Result<(), JobError> + Sync),
) -> Result<Vec<Cut>, JobError> {
    use std::sync::atomic::AtomicBool;
    let results: Vec<std::sync::Mutex<Option<Result<Cut, JobError>>>> =
        batch.iter().map(|_| std::sync::Mutex::new(None)).collect();
    let stop = AtomicBool::new(false);
    let finished = (std::sync::Mutex::new(0usize), std::sync::Condvar::new());
    let cut_check = || {
        if stop.load(Ordering::Relaxed) {
            Err(stopped_by_sibling())
        } else {
            check()
        }
    };
    let mut caller_error = None;
    pool.in_place_scope(|scope| {
        for (group, slot) in batch.iter().zip(&results) {
            let (cut_check, stop, finished) = (&cut_check, &stop, &finished);
            scope.spawn(move |_| {
                let result = cut_image(
                    captured,
                    group,
                    page_memory,
                    &requests[group.crops.clone()],
                    cut_check,
                );
                if result.is_err() {
                    stop.store(true, Ordering::Relaxed);
                }
                *lock(slot) = Some(result);
                *lock(&finished.0) += 1;
                finished.1.notify_one();
            });
        }
        let mut finished_count = lock(&finished.0);
        while *finished_count < batch.len() {
            if caller_error.is_none() {
                if let Err(error) = progress.advance(count(done + *finished_count)) {
                    stop.store(true, Ordering::Relaxed);
                    caller_error = Some(error);
                }
            }
            finished_count = finished
                .1
                .wait_timeout(finished_count, HEARTBEAT_WAIT)
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .0;
        }
    });
    if let Some(error) = caller_error {
        return Err(error);
    }
    let results: Vec<Result<Cut, JobError>> = results
        .into_iter()
        .map(|slot| {
            slot.into_inner()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .unwrap_or_else(|| Err(state("an image cut did not finish")))
        })
        .collect();
    let sibling = stopped_by_sibling().to_string();
    if let Some(error) = results
        .iter()
        .find_map(|r| r.as_ref().err().filter(|e| e.to_string() != sibling))
    {
        return Err(error.clone());
    }
    results.into_iter().collect()
}

/// Decodes one source image once and cuts each of its crops (at its density level, with
/// its wrap) exactly as [`crops::copy_crop_tiled`] would from the decoded source, then the
/// marked tiles of each coarse chain built from it ([`sparse::cut`]).
fn cut_image(
    captured: &FullCaptured,
    group: &Group<'_>,
    page_memory: u64,
    requests: &[CropRequest],
    check: &(dyn Fn() -> Result<(), JobError> + Sync),
) -> Result<Cut, JobError> {
    check()?;
    let image = group.image;
    let info = captured
        .image_info
        .get(image)
        .ok_or_else(|| state("crop image missing"))?;
    let snapshot = captured
        .images
        .get(image)
        .ok_or_else(|| state("crop image missing"))?;
    let encoded = snapshot.reconstruct(&mut || check())?;
    let (decoded, rgba) = crate::gltf_source::source::decode_image_admitted(
        &encoded,
        snapshot.identity.mime.as_deref(),
        crate::gltf_source::source::DecodeAdmission::LARGE_MESH,
        &mut || check(),
    )?;
    drop(encoded);
    if decoded.width != info.width || decoded.height != info.height {
        return Err(state("decoded image size differs from its capture"));
    }
    // The page workspace bounds chain builds and cuts, as for pages cutting from decoded
    // sources, so the store refuses exactly what they would.
    let limit = usize::try_from(page_memory).unwrap_or(usize::MAX);
    let mut levels: BTreeMap<u8, Vec<mips::Level>> = BTreeMap::new();
    let mut scratch = Vec::new();
    let mut out = Vec::with_capacity(requests.len());
    for r in requests {
        check()?;
        let drop = u32::from(r.drop).min(31 - info.width.max(info.height).leading_zeros());
        let (w, h, texels): (u32, u32, &[u8]) = if drop == 0 {
            (info.width, info.height, &rgba)
        } else {
            if let std::collections::btree_map::Entry::Vacant(slot) = levels.entry(r.class) {
                let chain = mips::build(
                    info.width,
                    info.height,
                    rgba.clone(),
                    class_of(r.class)?,
                    None,
                    limit,
                )?;
                slot.insert(chain.levels);
            }
            let level = levels
                .get(&r.class)
                .and_then(|l| l.get(usize::try_from(drop).ok()?))
                .ok_or_else(|| state("density level outside its chain"))?;
            (level.width, level.height, &level.rgba)
        };
        // The level cut from has the size the leaf's layout planned at this density, as a
        // page cutting from the decoded density level checks.
        let planned = [
            (info.width >> u32::from(r.drop)).max(1),
            (info.height >> u32::from(r.drop)).max(1),
        ];
        if [w, h] != planned {
            return Err(state("density level differs from the planned source size"));
        }
        let stride = usize::try_from(w).map_err(|_| state("level width"))?;
        let cut = crops::copy_crop_pixels(w, h, &r.crop(), r.sampler(), page_memory, |x, y| {
            let i = (y * stride + x) * 4;
            Ok([texels[i], texels[i + 1], texels[i + 2], texels[i + 3]])
        })?;
        let (cw, ch) = (
            usize::try_from(r.width).map_err(|_| state("crop width"))?,
            usize::try_from(r.height).map_err(|_| state("crop height"))?,
        );
        out.push((*r, pack(&cut, cw, ch, &mut scratch)));
    }
    // The leaf density chains are done; the coarse chains are built one at a time, the
    // last from the decoded image itself.
    drop(levels);
    let mut coarse = Vec::with_capacity(group.coarse.len());
    let mut source = Some(rgba);
    for (k, &(class, marks)) in group.coarse.iter().enumerate() {
        let rgba = if k + 1 == group.coarse.len() {
            source.take()
        } else {
            source.clone()
        }
        .ok_or_else(|| state("coarse chain source"))?;
        let level0 = mips::Level {
            width: info.width,
            height: info.height,
            rgba,
        };
        coarse.push((class, sparse::cut(level0, class_of(class)?, marks, check)?));
    }
    Ok((out, coarse))
}

#[cfg(test)]
#[path = "crop_store_tests.rs"]
mod tests;
