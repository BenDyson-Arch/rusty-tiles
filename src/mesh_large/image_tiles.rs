//! Immutable stage-lifetime RGBA rasters. Only texel tiles are evictable: source
//! reconstruction identity belongs to the caller's once-initialized image table.
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
// Includes map nodes, Arc, allocation headers and ordering index per cached tile.
const TILE_CHARGE: u64 = TILE_BYTES + 512;
pub(crate) const READER_BYTES: u64 = TILE_BYTES + 512;
pub(crate) const IMAGE_METADATA_BYTES: u64 = 64 * 1024;
fn limit(s: &str) -> JobError {
    JobError::new(JobErrorKind::ResourceLimit, s)
}
fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

#[derive(Default)]
struct Tiles {
    clock: u64,
    entries: BTreeMap<(u64, u64), (u64, Arc<Vec<u8>>)>,
    order: BTreeSet<(u64, (u64, u64))>,
    peak: u64,
}
struct Cache {
    capacity: usize,
    state: Mutex<Tiles>,
}
impl Cache {
    fn get(
        &self,
        file: &ByteFile,
        id: u64,
        offset: u64,
        bytes: usize,
    ) -> Result<Arc<Vec<u8>>, JobError> {
        let mut s = lock(&self.state);
        s.clock += 1;
        let clock = s.clock;
        let key = (id, offset);
        if let Some((old, data)) = s.entries.get(&key).cloned() {
            s.order.remove(&(old, key));
            s.order.insert((clock, key));
            s.entries.insert(key, (clock, data.clone()));
            return Ok(data);
        }
        // Read under the lock: no duplicate buffers or uncharged concurrent fills.
        while s.entries.len() >= self.capacity && !s.entries.is_empty() {
            let (old, key) = s.order.pop_first().expect("tile order");
            let _ = old;
            s.entries.remove(&key);
        }
        let mut data = vec![0; bytes];
        file.read_at(offset, &mut data)?;
        let data = Arc::new(data);
        if self.capacity > 0 {
            s.entries.insert(key, (clock, data.clone()));
            s.order.insert((clock, key));
            s.peak = s.peak.max(s.entries.len() as u64 * TILE_CHARGE);
        }
        Ok(data)
    }
}

pub(crate) struct Store {
    resident: bool,
    budget: u64,
    bound: u64,
    bytes: AtomicU64,
    next: AtomicU64,
    cache: Arc<Cache>,
}
impl Store {
    /// `bound` is the admitted total of every raster, including all class/LOD variants.
    pub(crate) fn new(budget: u64, bound: u64) -> Self {
        Self {
            resident: bound <= budget,
            budget,
            bound,
            bytes: AtomicU64::new(0),
            next: AtomicU64::new(0),
            cache: Arc::new(Cache {
                capacity: (budget / TILE_CHARGE) as usize,
                state: Mutex::new(Tiles::default()),
            }),
        }
    }
    pub(crate) fn peak(&self) -> u64 {
        if self.resident {
            self.bytes.load(Ordering::Relaxed)
        } else {
            lock(&self.cache.state).peak
        }
    }
    pub(crate) fn scratch_bytes(&self) -> u64 {
        if self.resident {
            0
        } else {
            self.bytes.load(Ordering::Relaxed)
        }
    }
    pub(crate) fn put(
        &self,
        scratch: &Scratch,
        width: u32,
        height: u32,
        rgba: Vec<u8>,
    ) -> Result<Raster, JobError> {
        let bytes = u64::from(width) * u64::from(height) * 4;
        if width == 0 || height == 0 || rgba.len() as u64 != bytes {
            return Err(limit("image tile shape"));
        }
        self.bytes
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |n| {
                n.checked_add(bytes).filter(|b| *b <= self.bound)
            })
            .map_err(|_| limit("image tile store exceeded admitted bound"))?;
        let pixels = if self.resident {
            debug_assert!(self.bound <= self.budget);
            Pixels::Resident(rgba)
        } else {
            let mut writer = ByteWriter::new(scratch, "image-tiles", TILE_BYTES as usize)?;
            let (w, h) = (width as usize, height as usize);
            for y in (0..h).step_by(EDGE) {
                for x in (0..w).step_by(EDGE) {
                    for row in y..(y + EDGE).min(h) {
                        let start = (row * w + x) * 4;
                        writer.append(&rgba[start..start + (w - x).min(EDGE) * 4])?;
                    }
                }
            }
            Pixels::Tiled {
                file: writer.finish()?,
                id: self.next.fetch_add(1, Ordering::Relaxed),
                cache: self.cache.clone(),
            }
        };
        Ok(Raster {
            width,
            height,
            pixels,
        })
    }
}

enum Pixels {
    Resident(Vec<u8>),
    Tiled {
        file: ByteFile,
        id: u64,
        cache: Arc<Cache>,
    },
}
pub(crate) struct Raster {
    pub width: u32,
    pub height: u32,
    pixels: Pixels,
}
pub(crate) struct Reader<'a> {
    raster: &'a Raster,
    last: Option<(u64, Arc<Vec<u8>>)>,
}
impl Raster {
    pub(crate) fn reader(&self) -> Reader<'_> {
        Reader {
            raster: self,
            last: None,
        }
    }
    pub(crate) fn materialize(&self) -> Result<Vec<u8>, JobError> {
        if let Pixels::Resident(v) = &self.pixels {
            return Ok(v.clone());
        }
        let mut out = vec![0; self.width as usize * self.height as usize * 4];
        let mut reader = self.reader();
        for y in 0..self.height as usize {
            for x in 0..self.width as usize {
                out[(y * self.width as usize + x) * 4..(y * self.width as usize + x) * 4 + 4]
                    .copy_from_slice(&reader.pixel(x, y)?);
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
            Pixels::Tiled { file, id, cache } => {
                let (tx, ty) = (x / EDGE * EDGE, y / EDGE * EDGE);
                let w = (r.width as usize - tx).min(EDGE);
                let h = (r.height as usize - ty).min(EDGE);
                let offset = ((ty * r.width as usize + tx * h) * 4) as u64;
                if self.last.as_ref().is_none_or(|(o, _)| *o != offset) {
                    self.last = Some((offset, cache.get(file, *id, offset, w * h * 4)?));
                }
                let data = &self.last.as_ref().expect("tile").1;
                let i = ((y - ty) * w + x - tx) * 4;
                Ok(data[i..i + 4].try_into().expect("tile pixel"))
            }
        }
    }
}

/// Exact chain bytes, including thin and odd-sized images (4/3 is not a bound
/// for 1xN). Four classes bound all HLOD or leaf density variants.
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
            let info = &captured.image_info[image];
            let base = u64::from(info.width) * u64::from(info.height) * 4;
            let chain = chain_bytes(info.width, info.height);
            if leaf {
                chain - base + if bases.insert(image) { base } else { 0 }
            } else {
                chain
            }
        })
        .sum()
}
/// At most 32 levels in four classes; one independently owned spill file per level.
pub(crate) fn file_rounding(captured: &super::full::FullCaptured) -> u64 {
    captured.images.len() as u64 * 128 * captured.scratch.allocation_unit_bytes()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn tile_seams_odd_thin_images_and_eviction_preserve_every_byte() {
        let scratch = Scratch::create(None, 0).unwrap();
        let shapes = [(1, 513), (513, 1), (129, 257), (256, 256), (3, 7)];
        let bound = shapes.iter().map(|&(w, h)| w * h * 4).sum::<u32>() as u64;
        for budget in [0, TILE_CHARGE, bound] {
            let store = Store::new(budget, bound);
            let rasters: Vec<_> = shapes
                .iter()
                .map(|&(w, h)| {
                    let bytes: Vec<_> = (0..w * h * 4)
                        .map(|i| ((i * 71 + i / 13) % 251) as u8)
                        .collect();
                    (store.put(&scratch, w, h, bytes.clone()).unwrap(), bytes)
                })
                .collect();
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
                }
            }
            assert!(store.peak() <= budget);
        }
    }
    #[test]
    fn admission_is_checked_before_writing() {
        let scratch = Scratch::create(None, 0).unwrap();
        let store = Store::new(0, 4);
        assert!(store.put(&scratch, 2, 1, vec![0; 8]).is_err());
        assert_eq!(scratch.live_bytes(), 0);
        let image = store.put(&scratch, 1, 1, vec![3; 4]).unwrap();
        assert_eq!(image.materialize().unwrap(), [3; 4]);
        assert!(store.put(&scratch, 1, 1, vec![4; 4]).is_err());
    }
    #[test]
    fn truncated_scratch_read_is_an_error_not_a_pixel() {
        let scratch = Scratch::create(None, 0).unwrap();
        let store = Store::new(0, 4);
        let image = store.put(&scratch, 1, 1, vec![9; 4]).unwrap();
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
        assert!(image.reader().pixel(0, 0).is_err());
    }

    #[test]
    fn chain_bound_handles_thin_images() {
        assert_eq!(chain_bytes(1, 8), 60);
        assert_eq!(chain_bytes(8, 1), 60);
        assert_eq!(chain_bytes(3, 7), 4 * (21 + 3 + 1));
    }
}
