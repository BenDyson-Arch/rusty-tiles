//! Immutable stage-lifetime RGBA8 rasters. A [`Store`] keeps chains resident while
//! they fit its memory budget and spills the rest, one owned scratch file per chain,
//! in 128x128 texel tiles read through a bounded, shared least-recently-used tile
//! cache. Only tiles are evictable: source reconstruction identity belongs to the
//! caller's once-initialized image table, so eviction never causes a decode.
use super::leaves::mips::Level;
use crate::{
    spill::{ByteFile, ByteWriter, Scratch},
    JobError, JobErrorKind,
};
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc, Mutex,
    },
};

const EDGE: usize = 128;
const TILE_BYTES: u64 = (EDGE * EDGE * 4) as u64;
/// Map nodes, ordering index, `Arc` and allocation headers per cached tile.
const TILE_OVERHEAD: u64 = 512;
/// Tiles a [`Reader`] holds: every tile of one bilinear footprint.
const READER_TILES: usize = 4;
/// Worst-case bytes one live [`Reader`] holds outside the shared cache: its held
/// tiles (which eviction may have dropped from the cache) and one read in flight.
pub(crate) const READER_BYTES: u64 = (READER_TILES as u64 + 1) * (TILE_BYTES + TILE_OVERHEAD);
/// Raster/chain metadata per source image: identity cells and raster records of
/// every class and level (at most about forty rasters of under 100 bytes each).
pub(crate) const IMAGE_METADATA_BYTES: u64 = 4 * 1024;
/// Spill files per source image: one per chain, at most four chains (leaf base image
/// plus three density classes; HLOD colour and data chains).
const FILES_PER_IMAGE: u64 = 4;

fn limit(s: &str) -> JobError {
    JobError::new(JobErrorKind::ResourceLimit, s)
}
fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

type Key = (u64, u64);
#[derive(Default)]
struct Tiles {
    clock: u64,
    entries: BTreeMap<Key, (u64, Arc<Vec<u8>>)>,
    /// (last use, key): O(log n) least-recently-used eviction.
    order: BTreeSet<(u64, Key)>,
    /// Bytes of resident rasters and of cached tiles (with overhead).
    resident: u64,
    /// Primary raster bytes not yet stored: secondary rasters never take their room.
    primary_left: u64,
    cached: u64,
    peak: u64,
}
impl Tiles {
    fn charge(bytes: usize) -> u64 {
        bytes as u64 + TILE_OVERHEAD
    }
    /// Evicts least recently used tiles until `extra` more bytes fit the budget.
    fn evict_for(&mut self, budget: u64, extra: u64) -> bool {
        while self.resident + self.cached + extra > budget {
            let Some((_, key)) = self.order.pop_first() else {
                return false;
            };
            let (_, data) = self.entries.remove(&key).expect("ordered tile");
            self.cached -= Self::charge(data.len());
        }
        true
    }
    fn observe(&mut self) {
        self.peak = self.peak.max(self.resident + self.cached);
    }
}
struct Cache {
    budget: u64,
    state: Mutex<Tiles>,
}
impl Cache {
    fn get(
        &self,
        file: &ByteFile,
        key: Key,
        offset: u64,
        bytes: usize,
    ) -> Result<Arc<Vec<u8>>, JobError> {
        let touch = |s: &mut Tiles| {
            s.clock += 1;
            let clock = s.clock;
            let entry = s.entries.get_mut(&key)?;
            let old = std::mem::replace(&mut entry.0, clock);
            let data = entry.1.clone();
            s.order.remove(&(old, key));
            s.order.insert((clock, key));
            Some(data)
        };
        if let Some(data) = touch(&mut lock(&self.state)) {
            return Ok(data);
        }
        // Read without the lock (the reader's in-flight buffer is charged to it);
        // a concurrent fill of the same tile keeps the first copy.
        let mut data = vec![0; bytes];
        file.read_at(offset, &mut data)?;
        let data = Arc::new(data);
        let mut s = lock(&self.state);
        if let Some(existing) = touch(&mut s) {
            return Ok(existing);
        }
        let charge = Tiles::charge(bytes);
        if s.evict_for(self.budget, charge) {
            let clock = s.clock;
            s.entries.insert(key, (clock, data.clone()));
            s.order.insert((clock, key));
            s.cached += charge;
            s.observe();
        }
        Ok(data)
    }
}

pub(crate) struct Store {
    /// Rasters stay resident while their total is at most this.
    resident_limit: u64,
    budget: u64,
    bound: u64,
    stored: AtomicU64,
    spilled: AtomicU64,
    next: AtomicU64,
    cache: Arc<Cache>,
}
impl Store {
    /// `budget` bounds resident rasters plus cached tiles; `bound` is the admitted
    /// total of every raster the store will hold (all classes and levels). Every
    /// raster is primary.
    pub(crate) fn new(budget: u64, bound: u64) -> Self {
        Self::with_primary(budget, bound, bound)
    }
    /// A store whose primary rasters ([`Store::put`], at most `primary` bytes) take
    /// residency before secondary ones ([`Store::put_secondary`]). While the primary
    /// total fits the budget it all stays resident. Beyond that, residency shrinks
    /// by the overflow, down to half the budget, leaving the tile cache room for the
    /// spilled bytes: a small overflow spills little and keeps it cached.
    pub(crate) fn with_primary(budget: u64, bound: u64, primary: u64) -> Self {
        let primary = primary.min(bound);
        Self {
            resident_limit: budget
                .saturating_sub(primary.saturating_sub(budget))
                .max(budget / 2),
            budget,
            bound,
            stored: AtomicU64::new(0),
            spilled: AtomicU64::new(0),
            next: AtomicU64::new(0),
            cache: Arc::new(Cache {
                budget,
                state: Mutex::new(Tiles {
                    primary_left: primary,
                    ..Tiles::default()
                }),
            }),
        }
    }
    /// Largest resident raster plus cached tile bytes at once.
    pub(crate) fn peak(&self) -> u64 {
        lock(&self.cache.state).peak
    }
    /// Raster bytes this store may spill: zero when everything stays resident.
    pub(crate) fn spill_bound(&self) -> u64 {
        if self.bound <= self.budget {
            0
        } else {
            self.bound
        }
    }
    /// Raster bytes spilled to scratch.
    pub(crate) fn scratch_bytes(&self) -> u64 {
        self.spilled.load(Ordering::Relaxed)
    }
    /// Stores one primary chain (or a single image as a one-level chain), resident
    /// or spilled to one scratch file. Admission against the bound precedes any write.
    pub(crate) fn put(
        &self,
        scratch: &Scratch,
        levels: Vec<Level>,
    ) -> Result<Vec<Raster>, JobError> {
        self.store(scratch, levels, true)
    }
    /// [`Store::put`] for a secondary chain: resident only beside every primary.
    pub(crate) fn put_secondary(
        &self,
        scratch: &Scratch,
        levels: Vec<Level>,
    ) -> Result<Vec<Raster>, JobError> {
        self.store(scratch, levels, false)
    }
    fn store(
        &self,
        scratch: &Scratch,
        levels: Vec<Level>,
        primary: bool,
    ) -> Result<Vec<Raster>, JobError> {
        let mut total = 0u64;
        for l in &levels {
            let bytes = u64::from(l.width) * u64::from(l.height) * 4;
            if l.width == 0 || l.height == 0 || l.rgba.len() as u64 != bytes {
                return Err(limit("image tile shape"));
            }
            total += bytes;
        }
        // Admit against the bound (an explicit CAS loop: `fetch_update` is deprecated
        // on newer toolchains and its replacement is not on older ones).
        let mut stored = self.stored.load(Ordering::Relaxed);
        loop {
            let next = stored
                .checked_add(total)
                .filter(|b| *b <= self.bound)
                .ok_or_else(|| limit("image tile store exceeded admitted bound"))?;
            match self.stored.compare_exchange_weak(
                stored,
                next,
                Ordering::Relaxed,
                Ordering::Relaxed,
            ) {
                Ok(_) => break,
                Err(current) => stored = current,
            }
        }
        let resident = {
            let mut s = lock(&self.cache.state);
            let reserved = if primary {
                s.primary_left = s.primary_left.saturating_sub(total);
                0
            } else {
                s.primary_left
            };
            let fits = s.resident + total + reserved <= self.resident_limit;
            if fits {
                s.resident += total;
                s.evict_for(self.cache.budget, 0);
                s.observe();
            }
            fits
        };
        if resident {
            return Ok(levels
                .into_iter()
                .map(|l| Raster {
                    width: l.width,
                    height: l.height,
                    pixels: Pixels::Resident(l.rgba),
                })
                .collect());
        }
        self.spilled.fetch_add(total, Ordering::Relaxed);
        let mut writer = ByteWriter::new(scratch, "image-tiles", TILE_BYTES as usize)?;
        let mut starts = Vec::with_capacity(levels.len());
        for l in &levels {
            starts.push(writer.len());
            let (w, h) = (l.width as usize, l.height as usize);
            // Tile rows top to bottom, tiles left to right, each tile's rows contiguous.
            for y in (0..h).step_by(EDGE) {
                for x in (0..w).step_by(EDGE) {
                    for row in y..(y + EDGE).min(h) {
                        let start = (row * w + x) * 4;
                        writer.append(&l.rgba[start..start + (w - x).min(EDGE) * 4])?;
                    }
                }
            }
        }
        let file = Arc::new(writer.finish()?);
        Ok(levels
            .into_iter()
            .zip(starts)
            .map(|(l, start)| Raster {
                width: l.width,
                height: l.height,
                pixels: Pixels::Tiled {
                    file: file.clone(),
                    start,
                    id: self.next.fetch_add(1, Ordering::Relaxed),
                    cache: self.cache.clone(),
                },
            })
            .collect())
    }
}

enum Pixels {
    Resident(Vec<u8>),
    Tiled {
        file: Arc<ByteFile>,
        start: u64,
        id: u64,
        cache: Arc<Cache>,
    },
}
pub(crate) struct Raster {
    pub width: u32,
    pub height: u32,
    pixels: Pixels,
}
/// Texel access to one raster. Holds up to [`READER_TILES`] tiles, so a bilinear
/// footprint across a tile corner touches the shared cache once per tile.
pub(crate) struct Reader<'a> {
    raster: &'a Raster,
    held: [Option<(u64, Arc<Vec<u8>>)>; READER_TILES],
    next: usize,
}
impl Raster {
    pub(crate) fn reader(&self) -> Reader<'_> {
        Reader {
            raster: self,
            held: Default::default(),
            next: 0,
        }
    }
    /// The whole raster as tightly packed RGBA8 rows.
    pub(crate) fn materialize(&self) -> Result<Vec<u8>, JobError> {
        let (w, h) = (self.width as usize, self.height as usize);
        let (file, start, id, cache) = match &self.pixels {
            Pixels::Resident(v) => return Ok(v.clone()),
            Pixels::Tiled {
                file,
                start,
                id,
                cache,
            } => (file, *start, *id, cache),
        };
        let mut out = vec![0; w * h * 4];
        for ty in (0..h).step_by(EDGE) {
            let th = (h - ty).min(EDGE);
            for tx in (0..w).step_by(EDGE) {
                let tw = (w - tx).min(EDGE);
                let offset = start + ((ty * w + tx * th) * 4) as u64;
                let tile = cache.get(file, (id, offset), offset, tw * th * 4)?;
                for row in 0..th {
                    let dst = ((ty + row) * w + tx) * 4;
                    out[dst..dst + tw * 4].copy_from_slice(&tile[row * tw * 4..(row + 1) * tw * 4]);
                }
            }
        }
        Ok(out)
    }
}
impl Reader<'_> {
    pub(crate) fn pixel(&mut self, x: usize, y: usize) -> Result<[u8; 4], JobError> {
        let r = self.raster;
        if x >= r.width as usize || y >= r.height as usize {
            return Err(limit("image tile read outside raster"));
        }
        match &r.pixels {
            Pixels::Resident(v) => {
                let i = (y * r.width as usize + x) * 4;
                Ok(v[i..i + 4].try_into().expect("pixel"))
            }
            Pixels::Tiled {
                file,
                start,
                id,
                cache,
            } => {
                let (tx, ty) = (x / EDGE * EDGE, y / EDGE * EDGE);
                let w = (r.width as usize - tx).min(EDGE);
                let h = (r.height as usize - ty).min(EDGE);
                let offset = start + ((ty * r.width as usize + tx * h) * 4) as u64;
                let i = ((y - ty) * w + x - tx) * 4;
                if let Some((_, data)) = self.held.iter().flatten().find(|(o, _)| *o == offset) {
                    return Ok(data[i..i + 4].try_into().expect("tile pixel"));
                }
                let data = cache.get(file, (*id, offset), offset, w * h * 4)?;
                let pixel = data[i..i + 4].try_into().expect("tile pixel");
                self.held[self.next] = Some((offset, data));
                self.next = (self.next + 1) % READER_TILES;
                Ok(pixel)
            }
        }
    }
}

/// Exact chain bytes, including thin and odd-sized images (4/3 is not a bound
/// for 1xN).
pub(crate) fn chain_bytes(mut w: u32, mut h: u32) -> u64 {
    let mut bytes = 0;
    loop {
        bytes += u64::from(w) * u64::from(h) * 4;
        if w == 1 && h == 1 {
            return bytes;
        }
        w = (w / 2).max(1);
        h = (h / 2).max(1);
    }
}
/// Bytes of every raster a stage can store: one chain per (image, sampling class)
/// for HLOD; for leaves, each image once plus the reduced levels of its density
/// chains per class.
pub(crate) fn bound(captured: &super::full::FullCaptured, leaf: bool) -> u64 {
    use crate::gltf_source::source::TextureSlot;
    let mut keys = BTreeSet::new();
    for material in &captured.materials {
        for binding in material.bindings() {
            if let Some(image) = captured
                .textures
                .get(binding.texture)
                .and_then(|t| t["source"].as_u64())
            {
                // HLOD samples colour or data chains; leaf density chains are
                // per colour, normal or data class.
                let class = match binding.slot {
                    TextureSlot::BaseColor | TextureSlot::Emissive => 0,
                    TextureSlot::Normal if leaf => 1,
                    _ => 2,
                };
                keys.insert((image as usize, class));
            }
        }
    }
    let mut bases = BTreeSet::new();
    keys.into_iter()
        .map(|(image, _)| {
            let Some(info) = captured.image_info.get(image) else {
                return 0;
            };
            let base = u64::from(info.width) * u64::from(info.height) * 4;
            let chain = chain_bytes(info.width, info.height);
            if leaf {
                chain - base + if bases.insert(image) { base } else { 0 }
            } else {
                chain
            }
        })
        .fold(0u64, u64::saturating_add)
}
/// Allocation rounding of every spill file a stage's store can own.
pub(crate) fn file_rounding(captured: &super::full::FullCaptured) -> u64 {
    captured.images.len() as u64 * FILES_PER_IMAGE * captured.scratch.allocation_unit_bytes()
}

#[cfg(test)]
mod tests {
    use super::*;
    fn pattern(w: u32, h: u32, seed: u32) -> Vec<u8> {
        (0..w * h * 4)
            .map(|i| ((i * 71 + i / 13 + seed * 31) % 251) as u8)
            .collect()
    }
    fn chain(shapes: &[(u32, u32)], seed: u32) -> Vec<Level> {
        shapes
            .iter()
            .map(|&(width, height)| Level {
                width,
                height,
                rgba: pattern(width, height, seed),
            })
            .collect()
    }
    fn file_count(scratch: &Scratch) -> usize {
        std::fs::read_dir(scratch.path()).unwrap().count()
    }
    #[test]
    fn tile_seams_odd_thin_images_and_eviction_preserve_every_byte() {
        let shapes = [(1, 513), (513, 1), (129, 257), (256, 256), (3, 7)];
        let bound: u64 = shapes
            .iter()
            .map(|&(w, h)| u64::from(w * h * 4))
            .sum::<u64>()
            * 2;
        for budget in [0, TILE_BYTES, 3 * TILE_BYTES, bound] {
            let scratch = Scratch::create(None, 0).unwrap();
            let store = Store::new(budget, bound);
            let rasters: Vec<_> = (0..2)
                .flat_map(|seed| {
                    let levels = chain(&shapes, seed);
                    let references: Vec<_> = levels.iter().map(|l| l.rgba.clone()).collect();
                    store
                        .put(&scratch, levels)
                        .unwrap()
                        .into_iter()
                        .zip(references)
                })
                .collect();
            if budget < bound {
                // One spill file per spilled chain, never per level.
                assert!(file_count(&scratch) <= 2);
                assert!(store.scratch_bytes() > 0);
            } else {
                assert_eq!(store.scratch_bytes(), 0);
            }
            for _ in 0..3 {
                for (r, reference) in rasters.iter().rev() {
                    assert_eq!(r.materialize().unwrap(), *reference);
                    let mut reader = r.reader();
                    for y in (0..r.height as usize).rev() {
                        for x in (0..r.width as usize).rev() {
                            let i = (y * r.width as usize + x) * 4;
                            assert_eq!(reader.pixel(x, y).unwrap(), reference[i..i + 4]);
                        }
                    }
                    assert!(reader.pixel(r.width as usize, 0).is_err());
                    assert!(reader.pixel(0, r.height as usize).is_err());
                }
            }
            assert!(store.peak() <= budget, "{} > {budget}", store.peak());
        }
    }
    #[test]
    fn resident_chains_fill_half_the_budget_when_the_bound_does_not_fit() {
        let scratch = Scratch::create(None, 0).unwrap();
        let one = u64::from(64 * 64 * 4u32);
        let store = Store::new(4 * one, 8 * one);
        let held: Vec<_> = (0..8)
            .map(|seed| store.put(&scratch, chain(&[(64, 64)], seed)).unwrap())
            .collect();
        assert_eq!(store.scratch_bytes(), 6 * one);
        assert_eq!(file_count(&scratch), 6);
        drop(held);
        assert_eq!(file_count(&scratch), 0);
        let all = Store::new(8 * one, 8 * one);
        for seed in 0..8 {
            all.put(&scratch, chain(&[(64, 64)], seed)).unwrap();
        }
        assert_eq!(all.scratch_bytes(), 0);
        assert_eq!(all.peak(), 8 * one);
    }
    #[test]
    fn primaries_keep_residency_and_small_overflows_stay_cached() {
        let scratch = Scratch::create(None, 0).unwrap();
        let one = u64::from(64 * 64 * 4u32);
        // Eight primaries fit exactly; secondaries requested first must not take
        // their room.
        let store = Store::with_primary(8 * one, 12 * one, 8 * one);
        let mut held = Vec::new();
        for seed in 0..4 {
            held.push(
                store
                    .put_secondary(&scratch, chain(&[(64, 64)], seed))
                    .unwrap(),
            );
        }
        assert_eq!(store.scratch_bytes(), 4 * one);
        for seed in 0..8 {
            held.push(store.put(&scratch, chain(&[(64, 64)], seed)).unwrap());
        }
        assert_eq!(store.scratch_bytes(), 4 * one);
        // A primary total one image over the budget spills two images, not half.
        let store = Store::new(8 * one, 9 * one);
        for seed in 0..9 {
            held.push(store.put(&scratch, chain(&[(64, 64)], seed)).unwrap());
        }
        assert_eq!(store.scratch_bytes(), 2 * one);
        assert!(store.spill_bound() > 0);
        drop(held);
    }
    #[test]
    fn concurrent_readers_see_identical_texels_within_the_budget() {
        let scratch = Scratch::create(None, 0).unwrap();
        let shapes = [(300, 200), (150, 100), (75, 50)];
        let budget = 2 * (TILE_BYTES + TILE_OVERHEAD);
        let store = Store::new(budget, 1 << 30);
        let rasters = store.put(&scratch, chain(&shapes, 5)).unwrap();
        let references: Vec<_> = chain(&shapes, 5).into_iter().map(|l| l.rgba).collect();
        std::thread::scope(|scope| {
            for t in 0..6 {
                let (rasters, references) = (&rasters, &references);
                scope.spawn(move || {
                    for (r, reference) in rasters.iter().zip(references) {
                        let mut reader = r.reader();
                        let (w, h) = (r.width as usize, r.height as usize);
                        for k in 0..w * h {
                            // Thread-specific scatter order across tiles.
                            let p = (k * (2 * t + 7919)) % (w * h);
                            let (x, y) = (p % w, p / w);
                            let i = (y * w + x) * 4;
                            assert_eq!(reader.pixel(x, y).unwrap(), reference[i..i + 4]);
                        }
                    }
                });
            }
        });
        assert!(store.peak() <= budget);
    }
    #[test]
    fn admission_is_checked_before_writing() {
        let scratch = Scratch::create(None, 0).unwrap();
        let store = Store::new(0, 4);
        assert!(store.put(&scratch, chain(&[(2, 1)], 0)).is_err());
        assert_eq!(scratch.live_bytes(), 0);
        assert!(store
            .put(
                &scratch,
                vec![Level {
                    width: 2,
                    height: 2,
                    rgba: vec![0; 4]
                }]
            )
            .is_err());
        let image = store.put(&scratch, chain(&[(1, 1)], 3)).unwrap();
        assert_eq!(image[0].materialize().unwrap(), pattern(1, 1, 3));
        assert!(store.put(&scratch, chain(&[(1, 1)], 4)).is_err());
    }
    #[test]
    fn truncated_scratch_read_is_an_error_not_a_pixel() {
        let scratch = Scratch::create(None, 0).unwrap();
        let store = Store::new(0, 4);
        let image = store.put(&scratch, chain(&[(1, 1)], 9)).unwrap();
        let path = std::fs::read_dir(scratch.path())
            .unwrap()
            .next()
            .unwrap()
            .unwrap()
            .path();
        std::fs::OpenOptions::new()
            .write(true)
            .open(path)
            .unwrap()
            .set_len(0)
            .unwrap();
        assert!(image[0].reader().pixel(0, 0).is_err());
        assert!(image[0].materialize().is_err());
    }

    #[test]
    fn chain_bound_handles_thin_images() {
        assert_eq!(chain_bytes(1, 8), 60);
        assert_eq!(chain_bytes(8, 1), 60);
        assert_eq!(chain_bytes(3, 7), 4 * (21 + 3 + 1));
    }
}
