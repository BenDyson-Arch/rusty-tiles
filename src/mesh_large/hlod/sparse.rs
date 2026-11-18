//! Sparse coarse chains: the coarse stage's source chains, stored only where a coarse
//! sample can read them.
//!
//! A coarse texel samples its nearest source point: a barycentric point of one source
//! triangle, whose texture coordinates lie inside that triangle's UV hull. At any chain
//! level, a bilinear (or nearest) sample of a UV in the hull reads texels within the
//! hull's texel bounds widened by one tap. [`Needs`] marks, per source image and sampling
//! class, every 64×64 chain tile within those bounds (plus a rounding margin, under the
//! binding's wrap modes) at every level, for every triangle and every texture its material
//! binds. The leaf crop pass, which decodes every source image once, builds the chain
//! ([`super::bake::chain_from`]) and stores those tiles. [`Reader`] then serves every
//! coarse sample with [`super::bake`]'s arithmetic, so the coarse stage never decodes a
//! source image.
//!
//! Contract:
//! - Coverage is complete by construction: a sample whose tap falls in an unstored tile is
//!   an `InvalidState` error, never a silent substitute.
//! - Sampled values are bit-identical to sampling the full chain: the same chain builder,
//!   the same level dimensions and the same trilinear arithmetic.
//! - Memory: the index ([`INDEX_BYTES_PER_TILE`] per chain tile, stored or not) and one
//!   bounded reader per coarse job ([`READER_BYTES`]).
use super::bake;
use crate::gltf_source::source::ImageInfo;
use crate::mesh_large::full::FullCaptured;
use crate::mesh_large::leaves::crops::SourceSampler;
use crate::mesh_large::leaves::mips::{self, Class};
use crate::{JobError, JobErrorKind};
use std::collections::BTreeMap;

/// Chain tile edge in texels.
pub(crate) const EDGE: u32 = 64;
/// Index bytes per chain tile, stored or not (its entry and the level's slot).
pub(crate) const INDEX_BYTES_PER_TILE: u64 = 24;
/// Decoded tiles one reader keeps.
const READER_TILES: usize = 256;
/// Bytes one coarse job's [`Reader`] holds: its decoded tiles, one encoded tile being read
/// and their bookkeeping.
pub(crate) const READER_BYTES: u64 = (READER_TILES as u64 + 2) * (EDGE as u64 * EDGE as u64 * 4)
    + (READER_TILES as u64) * 128
    + 4096;

fn state(message: &str) -> JobError {
    JobError::new(
        JobErrorKind::InvalidState,
        format!("sparse coarse chain: {message}"),
    )
}

/// Every level's (width, height) of the chain [`bake::chain_from`] builds.
pub(crate) fn chain_dims(width: u32, height: u32) -> Vec<(u32, u32)> {
    let mut dims = vec![(width, height)];
    while let Some(&(w, h)) = dims.last() {
        if w <= 1 && h <= 1 {
            break;
        }
        dims.push(((w / 2).max(1), (h / 2).max(1)));
    }
    dims
}

/// At most three inclusive texel spans (the most a wrapped range shorter than its period
/// splits into).
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct Spans {
    items: [(u32, u32); 3],
    len: usize,
}
impl Spans {
    fn one(a: u32, b: u32) -> Self {
        Self {
            items: [(a, b), (0, 0), (0, 0)],
            len: 1,
        }
    }
    pub(crate) fn as_slice(&self) -> &[(u32, u32)] {
        &self.items[..self.len]
    }
}

/// The texel spans (inclusive, in `0..n`) that texels `lo..=hi` read under wrap `mode`
/// (glTF CLAMP_TO_EDGE, MIRRORED_REPEAT, otherwise REPEAT), as [`bake`]'s wrap maps them.
/// `lo` and `hi` are integral texel coordinates within ±2^53 (exact in `f64`).
pub(crate) fn wrapped_spans(lo: f64, hi: f64, n: u32, mode: u32) -> Spans {
    if n == 0 || hi < lo {
        return Spans::default();
    }
    let last_texel = f64::from(n - 1);
    if mode == 33071 {
        return Spans::one(
            index(lo.clamp(0., last_texel), n),
            index(hi.clamp(0., last_texel), n),
        );
    }
    let mirror = mode == 33648;
    let width = f64::from(n);
    let period = if mirror { 2. * width } else { width };
    if hi - lo + 1. >= period {
        return Spans::one(0, n - 1);
    }
    let mut spans = Spans::default();
    let mut x = lo;
    while x <= hi {
        let m = x.rem_euclid(period);
        // The rest of this monotone piece: to the period's end, or (mirrored) to `n - 1`.
        let piece_end = if mirror && m < width {
            width - 1.
        } else {
            period - 1.
        };
        let end = hi.min(x + (piece_end - m));
        let last = m + (end - x);
        let span = if mirror && m >= width {
            (index(period - 1. - last, n), index(period - 1. - m, n))
        } else {
            (index(m, n), index(last, n))
        };
        // A range shorter than its period has at most three pieces; a fourth would only
        // add texels already read, so the whole axis is a safe superset.
        if spans.len == spans.items.len() {
            return Spans::one(0, n - 1);
        }
        spans.items[spans.len] = span;
        spans.len += 1;
        x = end + 1.;
    }
    spans
}

/// The texel index of an integral `v` in `0..n`: the largest `i < n` with `i <= v`, found
/// by comparison so no float-to-integer cast is needed.
fn index(v: f64, n: u32) -> u32 {
    let (mut lo, mut hi) = (0u32, n.saturating_sub(1));
    while lo < hi {
        let mid = lo + (hi - lo).div_ceil(2);
        if f64::from(mid) <= v {
            lo = mid;
        } else {
            hi = mid - 1;
        }
    }
    lo
}

/// One chain's tiles, level by level: `cols` tiles per row and a bit per tile.
#[derive(Clone, Debug)]
pub(crate) struct Marks {
    pub dims: Vec<(u32, u32)>,
    levels: Vec<(usize, Vec<u64>)>,
}
impl Marks {
    fn new(width: u32, height: u32) -> Self {
        let dims = chain_dims(width, height);
        let levels = dims
            .iter()
            .map(|&(w, h)| {
                let cols = w.div_ceil(EDGE) as usize;
                let tiles = cols * h.div_ceil(EDGE) as usize;
                (cols, vec![0u64; tiles.div_ceil(64)])
            })
            .collect();
        Self { dims, levels }
    }
    fn set(&mut self, level: usize, x: u32, y: u32) {
        let (cols, bits) = &mut self.levels[level];
        let t = (y / EDGE) as usize * *cols + (x / EDGE) as usize;
        bits[t / 64] |= 1 << (t % 64);
    }
    pub(crate) fn has(&self, level: usize, tile: usize) -> bool {
        let bits = &self.levels[level].1;
        bits.get(tile / 64)
            .is_some_and(|b| b & (1 << (tile % 64)) != 0)
    }
    /// Tiles per row of `level`.
    pub(crate) fn cols(&self, level: usize) -> usize {
        self.levels[level].0
    }
    /// Every tile of the chain, marked or not.
    pub(crate) fn tiles(&self) -> u64 {
        self.dims
            .iter()
            .map(|&(w, h)| u64::from(w.div_ceil(EDGE)) * u64::from(h.div_ceil(EDGE)))
            .sum()
    }
    /// Marked tiles and their raw RGBA8 bytes.
    pub(crate) fn marked(&self) -> (u64, u64) {
        let (mut count, mut bytes) = (0, 0);
        for (level, &(w, h)) in self.dims.iter().enumerate() {
            let cols = self.cols(level);
            let rows = h.div_ceil(EDGE) as usize;
            for t in 0..cols * rows {
                if self.has(level, t) {
                    let (tw, th) = tile_size(w, h, cols, t);
                    count += 1;
                    bytes += (tw * th * 4) as u64;
                }
            }
        }
        (count, bytes)
    }
    /// Marks every tile a sample of a UV inside the hull of `uv` can read at every level.
    fn mark(&mut self, sampler: SourceSampler, uv: [[f32; 2]; 3]) {
        // `bake`'s non-finite UV reads texel (0, 0); every chain keeps that tile.
        for level in 0..self.dims.len() {
            self.set(level, 0, 0);
        }
        if uv.iter().flatten().any(|v| !v.is_finite()) {
            return;
        }
        let bounds = |a: usize| {
            let v = uv.map(|p| f64::from(p[a]));
            (
                v.iter().copied().fold(f64::INFINITY, f64::min),
                v.iter().copied().fold(f64::NEG_INFINITY, f64::max),
                v.iter().map(|x| x.abs()).fold(0., f64::max),
            )
        };
        let (u, v) = (bounds(0), bounds(1));
        for level in 0..self.dims.len() {
            let (w, h) = self.dims[level];
            let xs = taps(u, w, sampler.wrap_s);
            let ys = taps(v, h, sampler.wrap_t);
            for &(x0, x1) in xs.as_slice() {
                for &(y0, y1) in ys.as_slice() {
                    for ty in y0 / EDGE..=y1 / EDGE {
                        for tx in x0 / EDGE..=x1 / EDGE {
                            self.set(level, tx * EDGE, ty * EDGE);
                        }
                    }
                }
            }
        }
    }
}

/// The texel spans of one axis that bilinear or nearest taps of coordinates in
/// `[min, max]` read at a level `n` texels wide: `floor(c·n − ½)` and one past, widened by
/// two texels and by the f32 interpolation's rounding of the coordinate.
fn taps((min, max, magnitude): (f64, f64, f64), n: u32, mode: u32) -> Spans {
    let scale = f64::from(n);
    // An interpolated f32 coordinate can stray a few ulps of its magnitude past the hull.
    let margin = 2. + (magnitude * scale * 4. * f64::from(f32::EPSILON)).ceil();
    let lo = (min * scale - 0.5).floor() - margin;
    let hi = (max * scale - 0.5).floor() + 1. + margin;
    // Beyond f64's exact integers every texel is read.
    if !(lo.is_finite() && hi.is_finite()) || lo < -9e15 || hi > 9e15 {
        return Spans::one(0, n.max(1) - 1);
    }
    wrapped_spans(lo, hi, n, mode)
}

/// Width and height of tile `t` of a `w`×`h` level with `cols` tiles per row.
fn tile_size(w: u32, h: u32, cols: usize, t: usize) -> (usize, usize) {
    let edge = EDGE as usize;
    let (x, y) = ((t % cols) * edge, (t / cols) * edge);
    ((w as usize - x).min(edge), (h as usize - y).min(edge))
}

/// The class a source texture of `slot` is sampled as (sRGB colour or raw data).
pub(crate) fn source_class(slot: crate::gltf_source::source::TextureSlot) -> Class {
    match super::appearance::slot_class(slot) {
        Class::Colour => Class::Colour,
        _ => Class::Data,
    }
}

/// The tiles every coarse sample can read, per (source image, class rank).
#[derive(Debug, Default)]
pub(crate) struct Needs {
    /// Per material: (image, class, sampler, texcoord set) of each slot it binds.
    bindings: Vec<Vec<(usize, Class, SourceSampler, usize)>>,
    pub chains: BTreeMap<(u32, u8), Marks>,
}
impl Needs {
    pub(crate) fn new(captured: &FullCaptured) -> Result<Self, JobError> {
        let mut bindings = Vec::with_capacity(captured.materials.len());
        for (m, material) in captured.materials.iter().enumerate() {
            let mut slots = Vec::new();
            for b in material.bindings() {
                // The coarse bake resolves a slot to the material's first binding of it.
                if slots.iter().any(|&(s, _)| s == b.slot) {
                    continue;
                }
                // A binding the bake cannot resolve fails the bake where it samples it, as
                // before; it marks nothing here.
                let found = u32::try_from(m)
                    .ok()
                    .and_then(|m| bake::binding(captured, m, b.slot).ok().flatten());
                slots.push((b.slot, found));
            }
            bindings.push(
                slots
                    .into_iter()
                    .filter_map(|(slot, found)| {
                        found.map(|(image, sampler, set)| (image, source_class(slot), sampler, set))
                    })
                    .collect(),
            );
        }
        Ok(Self {
            bindings,
            chains: BTreeMap::new(),
        })
    }
    /// Marks the tiles a coarse sample on `triangle` can read in every texture its
    /// material binds.
    pub(crate) fn mark(
        &mut self,
        images: &[ImageInfo],
        triangle: &crate::mesh_large::attributes::FullTriangle,
    ) -> Result<(), JobError> {
        let Some(bindings) = self.bindings.get(triangle.material as usize) else {
            return Ok(());
        };
        for &(image, class, sampler, set) in bindings {
            let info = images.get(image).ok_or_else(|| state("image missing"))?;
            let key = (
                u32::try_from(image).map_err(|_| state("image index"))?,
                crate::mesh_large::leaves::crop_store::class_rank(class),
            );
            let uv = triangle.corners.map(|c| c.texcoords[set]);
            self.chains
                .entry(key)
                .or_insert_with(|| Marks::new(info.width, info.height))
                .mark(sampler, uv);
        }
        Ok(())
    }
    /// Index bytes of every chain ([`INDEX_BYTES_PER_TILE`] per chain tile).
    pub(crate) fn index_bytes(&self) -> u64 {
        self.chains
            .values()
            .map(|m| {
                m.tiles()
                    .saturating_mul(INDEX_BYTES_PER_TILE)
                    .saturating_add(256)
            })
            .fold(0u64, u64::saturating_add)
    }
}

/// A stored tile: its offset and length in the coarse tile log, and whether its block is
/// stored raw.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Entry {
    pub offset: u64,
    pub len: u32,
    pub stored: bool,
}
/// One chain's stored tiles.
#[derive(Debug)]
pub(crate) struct Chain {
    pub dims: Vec<(u32, u32)>,
    /// Per level: tiles per row and each tile's entry (`None` when not stored).
    pub levels: Vec<(usize, Vec<Option<Entry>>)>,
}
impl Chain {
    pub(crate) fn new(marks: &Marks) -> Self {
        Self {
            dims: marks.dims.clone(),
            levels: marks
                .dims
                .iter()
                .zip(&marks.levels)
                .map(|(&(_, h), (cols, _))| (*cols, vec![None; cols * h.div_ceil(EDGE) as usize]))
                .collect(),
        }
    }
}

/// Every sparse chain and the log their tiles are in.
pub(crate) struct Chains {
    pub(crate) log: crate::spill::ByteFile,
    pub(crate) chains: BTreeMap<(u32, u8), Chain>,
    /// Tiles every finished reader decoded.
    pub(crate) decodes: std::sync::atomic::AtomicU64,
}
impl Chains {
    /// Tiles the readers decoded so far.
    pub(crate) fn decodes(&self) -> u64 {
        self.decodes.load(std::sync::atomic::Ordering::Relaxed)
    }
    /// The chain sampled for `image` as `class`.
    pub(crate) fn get(&self, image: usize, class: Class) -> Result<&Chain, JobError> {
        let key = (
            u32::try_from(image).map_err(|_| state("image index"))?,
            crate::mesh_large::leaves::crop_store::class_rank(class),
        );
        self.chains
            .get(&key)
            .ok_or_else(|| state("a coarse sample reads an image no source triangle binds"))
    }
}

/// A tile's (image, class, level, tile).
type TileId = (u32, u8, usize, usize);
/// A decoded tile a [`Reader`] holds: its id, its last use and its texels.
type HeldTile = (TileId, u64, Vec<u8>);

/// One coarse job's tile reader: the last tile of each level and a small LRU of decoded
/// tiles ([`READER_BYTES`]).
pub(crate) struct Reader<'a> {
    chains: &'a Chains,
    tiles: Vec<HeldTile>,
    /// Where each held tile is in `tiles`.
    slots: std::collections::HashMap<TileId, usize>,
    /// The tile read last (neighbouring taps usually share it).
    last: usize,
    clock: u64,
    /// Tiles decoded from the log (added to [`Chains::decodes`] when the reader drops).
    decodes: u64,
}
impl Drop for Reader<'_> {
    fn drop(&mut self) {
        self.chains
            .decodes
            .fetch_add(self.decodes, std::sync::atomic::Ordering::Relaxed);
    }
}
impl<'a> Reader<'a> {
    pub(crate) fn new(chains: &'a Chains) -> Self {
        Self {
            chains,
            tiles: Vec::with_capacity(READER_TILES),
            slots: std::collections::HashMap::with_capacity(READER_TILES),
            last: 0,
            clock: 0,
            decodes: 0,
        }
    }
    fn texel(
        &mut self,
        key: (u32, u8),
        chain: &Chain,
        level: usize,
        x: usize,
        y: usize,
    ) -> Result<[u8; 4], JobError> {
        let (w, h) = chain.dims[level];
        let (cols, entries) = &chain.levels[level];
        let edge = EDGE as usize;
        let t = (y / edge) * cols + x / edge;
        let (tw, _) = tile_size(w, h, *cols, t);
        let id = (key.0, key.1, level, t);
        self.clock += 1;
        let found = match self.tiles.get(self.last) {
            Some(e) if e.0 == id => Some(self.last),
            _ => self.slots.get(&id).copied(),
        };
        let at = match found {
            Some(at) => at,
            None => {
                let entry = entries
                    .get(t)
                    .copied()
                    .flatten()
                    .ok_or_else(|| state("a coarse sample reads a tile outside its chain"))?;
                let (tw, th) = tile_size(w, h, *cols, t);
                let mut block = vec![0u8; entry.len as usize];
                self.chains.log.read_at(entry.offset, &mut block)?;
                let rgba =
                    crate::mesh_large::leaves::crop_store::unpack(&block, entry.stored, tw, th)?;
                self.decodes += 1;
                let at = if self.tiles.len() < READER_TILES {
                    self.tiles.push((id, self.clock, rgba));
                    self.tiles.len() - 1
                } else {
                    let oldest = self
                        .tiles
                        .iter()
                        .enumerate()
                        .min_by_key(|(_, e)| e.1)
                        .map(|(i, _)| i)
                        .ok_or_else(|| state("reader"))?;
                    self.slots.remove(&self.tiles[oldest].0);
                    self.tiles[oldest] = (id, self.clock, rgba);
                    oldest
                };
                self.slots.insert(id, at);
                at
            }
        };
        self.last = at;
        let tile = &mut self.tiles[at];
        tile.1 = self.clock;
        let i = ((y % edge) * tw + x % edge) * 4;
        let p = &tile.2;
        Ok([p[i], p[i + 1], p[i + 2], p[i + 3]])
    }

    /// [`bake::sample_with`] over the sparse chain of `image` as `class`.
    pub(crate) fn sample(
        &mut self,
        image: usize,
        class: Class,
        sampler: SourceSampler,
        uv: [f32; 2],
        lod: f64,
    ) -> Result<[f64; 4], JobError> {
        let chains = self.chains;
        let chain = chains.get(image, class)?;
        let key = (
            u32::try_from(image).map_err(|_| state("image index"))?,
            crate::mesh_large::leaves::crop_store::class_rank(class),
        );
        bake::trilinear(
            chain.dims.len(),
            |i| chain.dims[i],
            class,
            sampler,
            uv,
            lod,
            |level, x, y| self.texel(key, chain, level, x, y),
        )
    }
}

/// The marked tiles of the chain built from `level0`, encoded, as (level, tile, block) in
/// level and tile order.
pub(crate) fn cut(
    level0: mips::Level,
    class: Class,
    marks: &Marks,
    check: &dyn Fn() -> Result<(), JobError>,
) -> Result<Vec<(usize, usize, crate::spill::codec::Encoded)>, JobError> {
    let levels = bake::chain_from(level0, class, check)?;
    if levels
        .iter()
        .map(|l| (l.width, l.height))
        .ne(marks.dims.iter().copied())
    {
        return Err(state("chain levels differ from their planned dimensions"));
    }
    let mut blocks = Vec::new();
    let mut scratch = Vec::new();
    let mut tile = Vec::new();
    for (index, level) in levels.iter().enumerate() {
        check()?;
        let (w, h) = (level.width, level.height);
        let cols = marks.cols(index);
        let rows = h.div_ceil(EDGE) as usize;
        for t in (0..cols * rows).filter(|&t| marks.has(index, t)) {
            let (tw, th) = tile_size(w, h, cols, t);
            let (x, y) = ((t % cols) * EDGE as usize, (t / cols) * EDGE as usize);
            tile.clear();
            for row in y..y + th {
                let at = (row * w as usize + x) * 4;
                tile.extend_from_slice(&level.rgba[at..at + tw * 4]);
            }
            let block = crate::mesh_large::leaves::crop_store::pack(&tile, tw, th, &mut scratch);
            blocks.push((index, t, block));
        }
    }
    Ok(blocks)
}

#[cfg(test)]
#[path = "sparse_tests.rs"]
mod tests;
