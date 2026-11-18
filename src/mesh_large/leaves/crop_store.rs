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
/// decode (16 bytes per texel, beyond the decoded-source cache's 12), every class chain its
/// density levels need (16 bytes per texel each, beyond the leaf chain builder's input copy
/// and levels), every encoded crop (at most its raw bytes) and the crop being cut twice
/// (cut and predicted).
pub(crate) fn image_charge(
    info: &crate::gltf_source::source::ImageInfo,
    chains: u64,
    crop_bytes: u64,
    largest_crop: u64,
) -> u64 {
    let texels = u64::from(info.width) * u64::from(info.height);
    texels
        .saturating_mul(16)
        .saturating_add(info.encoded_bytes)
        .saturating_add(texels.saturating_mul(16).saturating_mul(chains))
        .saturating_add(crop_bytes)
        .saturating_add(largest_crop.saturating_mul(2))
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
        let mut block = Vec::new();
        block
            .try_reserve_exact(entry.len as usize)
            .map_err(|_| resource("stored crop read"))?;
        block.resize(entry.len as usize, 0);
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
    let raw = usize::try_from(key.raw_bytes()).map_err(|_| state("crop size"))?;
    let mut residual = Vec::new();
    residual
        .try_reserve_exact(raw)
        .map_err(|_| resource("stored crop decode"))?;
    residual.resize(raw, 0);
    codec::decode(block, stored, &mut residual)
        .map_err(|e| state(&format!("stored crop is corrupt: {e}")))?;
    Ok(unpredict(residual, key.width as usize, key.height as usize))
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
    /// Raw bytes of every crop: the store's scratch bound.
    pub scratch_bytes: u64,
    /// The index's memory ([`INDEX_BYTES_PER_CROP`] per crop).
    pub index_bytes: u64,
    /// The largest single image's cut ([`image_charge`]).
    pub largest_image_bytes: u64,
    pub seconds: f64,
}

/// One image's crops (sorted, deduplicated) and its cut's memory.
fn image_groups(
    captured: &FullCaptured,
    requests: &[CropRequest],
) -> Result<Vec<(usize, u64, std::ops::Range<usize>)>, JobError> {
    let mut groups = Vec::new();
    let mut start = 0;
    while start < requests.len() {
        let image = requests[start].image;
        let end = start
            + requests[start..]
                .iter()
                .take_while(|r| r.image == image)
                .count();
        let info = captured
            .image_info
            .get(image as usize)
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
        groups.push((
            image as usize,
            image_charge(info, chains.len() as u64, total, largest),
            start..end,
        ));
        start = end;
    }
    Ok(groups)
}

/// Re-projects every leaf at its planned density (the layout final preparation builds) and
/// collects its store crops. `None` when the request list (counted before deduplication,
/// as it is held) would exceed `memory` bytes
/// (the stage then reads crops from the decoded sources, as without the store).
pub(crate) fn plan(
    captured: &FullCaptured,
    profile: crate::mesh_large::config::Profile,
    memory: u64,
    progress: &crate::mesh_large::phases::StepProgress<'_>,
) -> Result<Option<CropPlan>, JobError> {
    let started = std::time::Instant::now();
    let resources = super::atlas::AtlasResources {
        materials: &captured.materials,
        classes: &captured.material_classes,
        textures: &captured.textures,
        samplers: &captured.samplers,
        image_info: &captured.image_info,
    };
    let mut triangles = captured.triangles.reader()?;
    let mut requests: Vec<CropRequest> = Vec::new();
    progress.begin(
        crate::mesh_large::phases::Step::LeafCrops,
        captured.leaf_plans.len(),
    )?;
    for (done, plan) in captured.leaf_plans.reader()?.enumerate() {
        let plan = plan?;
        progress.advance(done as u64)?;
        let count = usize::try_from(plan.end - plan.first).map_err(|_| state("leaf size"))?;
        let mut leaf = Vec::new();
        leaf.try_reserve_exact(count)
            .map_err(|_| resource("leaf triangles"))?;
        for i in 0..count {
            if i.is_multiple_of(4096) {
                progress.check()?;
            }
            leaf.push(
                triangles
                    .next()
                    .transpose()?
                    .ok_or_else(|| state("leaf triangles truncated"))?,
            );
        }
        let keys = super::atlas::crop_requests(
            &leaf,
            resources,
            profile,
            u32::from(plan.density_drop),
            &mut || progress.check(),
        )?;
        let total = (requests.len() + keys.len()) as u64;
        if total.saturating_mul(INDEX_BYTES_PER_CROP) > memory {
            progress.end()?;
            return Ok(None);
        }
        requests.extend(keys);
    }
    progress.end()?;
    requests.sort_unstable();
    requests.dedup();
    let groups = image_groups(captured, &requests)?;
    let scratch_bytes = requests
        .iter()
        .try_fold(0u64, |n, r| n.checked_add(r.raw_bytes()))
        .ok_or_else(|| state("crop scratch overflow"))?;
    Ok(Some(CropPlan {
        scratch_bytes,
        index_bytes: (requests.len() as u64).saturating_mul(INDEX_BYTES_PER_CROP),
        largest_image_bytes: groups.iter().map(|g| g.1).max().unwrap_or(0),
        seconds: started.elapsed().as_secs_f64(),
        requests,
    }))
}

/// Cuts every crop of `plan` image by image on the caller's thread pool. Images go in
/// fixed batches whose cuts together lease at most `memory` bytes (each at least one
/// image, which the caller has checked fits), so output order and peak memory do not
/// depend on the worker count. The log never grows past the plan's raw bytes.
pub(crate) fn build(
    captured: &FullCaptured,
    plan: CropPlan,
    memory: u64,
    pool: &rayon::ThreadPool,
    progress: &crate::mesh_large::phases::StepProgress<'_>,
    check: &(dyn Fn() -> Result<(), JobError> + Sync),
) -> Result<CropStore, JobError> {
    use rayon::prelude::*;
    let started = std::time::Instant::now();
    let groups = image_groups(captured, &plan.requests)?;
    if groups.iter().any(|g| g.1 > memory) {
        return Err(JobError::new(
            JobErrorKind::ResourceLimit,
            "leaf crop store: one source image's cut exceeds its memory",
        ));
    }
    let mut writer = crate::spill::ByteWriter::new(&captured.scratch, "leaf-crops", 1 << 20)?;
    let mut index = BTreeMap::new();
    let mut peak = 0u64;
    progress.begin(
        crate::mesh_large::phases::Step::LeafCropStore,
        groups.len() as u64,
    )?;
    let mut done = 0usize;
    while done < groups.len() {
        // The next batch: images in order while their cuts fit `memory` together.
        let mut end = done;
        let mut leased = 0u64;
        while end < groups.len() && (end == done || leased + groups[end].1 <= memory) {
            leased += groups[end].1;
            end += 1;
        }
        peak = peak.max(leased);
        let requests = &plan.requests;
        let cut: Vec<Result<Vec<(CropRequest, codec::Encoded)>, JobError>> = pool.install(|| {
            groups[done..end]
                .par_iter()
                .map(|(image, charge, range)| {
                    cut_image(captured, *image, *charge, &requests[range.clone()], check)
                })
                .collect()
        });
        for blocks in cut {
            for (key, block) in blocks? {
                let offset = writer.len();
                if offset.saturating_add(block.bytes.len() as u64) > plan.scratch_bytes {
                    return Err(JobError::new(
                        JobErrorKind::ResourceLimit,
                        "leaf crop store exceeded its admitted scratch",
                    ));
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
        }
        done = end;
        progress.advance(done as u64)?;
    }
    progress.end()?;
    let written = writer.len();
    let log = writer.finish()?;
    Ok(CropStore {
        stats: CropStoreStats {
            crops: index.len() as u64,
            images: groups.len() as u64,
            raw_bytes: plan.scratch_bytes,
            written_bytes: written,
            memory_bytes: memory,
            memory_peak_bytes: peak,
            index_bytes: plan.index_bytes,
            plan_seconds: plan.seconds,
            build_seconds: started.elapsed().as_secs_f64(),
            misses: 0,
        },
        log,
        index,
        misses: AtomicU64::new(0),
    })
}

/// Decodes one source image once and cuts each of its crops (at its density level, with
/// its wrap) exactly as [`crops::copy_crop_tiled`] would from the decoded source.
fn cut_image(
    captured: &FullCaptured,
    image: usize,
    charge: u64,
    requests: &[CropRequest],
    check: &(dyn Fn() -> Result<(), JobError> + Sync),
) -> Result<Vec<(CropRequest, codec::Encoded)>, JobError> {
    check()?;
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
    let limit = usize::try_from(charge).unwrap_or(usize::MAX);
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
                .and_then(|l| l.get(drop as usize))
                .ok_or_else(|| state("density level outside its chain"))?;
            (level.width, level.height, &level.rgba)
        };
        let stride = w as usize;
        let cut = crops::copy_crop_pixels(w, h, &r.crop(), r.sampler(), charge, |x, y| {
            let i = (y * stride + x) * 4;
            Ok([texels[i], texels[i + 1], texels[i + 2], texels[i + 3]])
        })?;
        let block = codec::encode(
            &predict(&cut, r.width as usize, r.height as usize),
            &mut scratch,
        );
        out.push((*r, block));
    }
    Ok(out)
}

#[cfg(test)]
#[path = "crop_store_tests.rs"]
mod tests;
