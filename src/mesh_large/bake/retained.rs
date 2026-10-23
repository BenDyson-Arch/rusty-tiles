//! A retained child as its parent reads it. Geometry and page metadata are resident;
//! page texels (RGBA and terminal samples) are read on demand, a tile at a time, through
//! a cache of fixed size. A parent therefore never holds its children's pages whole:
//! its child residency is the children's faces plus one bounded cache, whatever the
//! size of the subtree below it.
use super::{
    invalid, packet,
    packet::TILE,
    resource,
    types::{ChildFace, PreencodeChild, TerminalSample},
};
use crate::{gltf_source::source::TextureSlot, mesh_large::records::NodeRecord, JobError};
use std::{
    collections::HashMap,
    hash::{BuildHasherDefault, Hasher},
    path::PathBuf,
};

pub(crate) struct ChildLevel {
    pub width: u32,
    pub height: u32,
}
/// One child page's metadata: everything a parent reads of it but its texels.
pub(crate) struct ChildPage {
    pub width: u32,
    pub height: u32,
    pub slot: TextureSlot,
    /// The material class representative of every face on the page (H28).
    pub material: u32,
    pub sampler: serde_json::Value,
    pub normal_scale: f64,
    pub levels: Vec<ChildLevel>,
}
enum Texels {
    PlainResident(Vec<Vec<Vec<u8>>>),
    /// RGBA and terminal samples of every level, by page then level.
    Resident(Vec<Vec<(Vec<u8>, Vec<TerminalSample>)>>),
    /// The child's packet and the body offset of each page level in it.
    Packet {
        file: std::fs::File,
        path: PathBuf,
        offsets: Vec<Vec<u64>>,
    },
}
/// A retained child: what its parent samples, with page texels resident or in its
/// packet.
pub(crate) struct ChildView {
    pub node: NodeRecord,
    pub texel_size: f64,
    pub faces: Vec<ChildFace>,
    pub pages: Vec<ChildPage>,
    texels: Texels,
    /// Bytes owned beyond the faces: page metadata, and resident texels or offsets.
    owned: u64,
}
fn metadata_bytes(pages: &[ChildPage]) -> u64 {
    pages
        .iter()
        .map(|p| {
            (std::mem::size_of::<ChildPage>()
                + p.levels.capacity() * std::mem::size_of::<ChildLevel>()) as u64
                + super::json_charge(&p.sampler)
        })
        .sum()
}
impl ChildView {
    /// A child held whole in memory (a leaf child, which has no pages, or a test's).
    /// Only what a parent samples is kept of its pages.
    pub(crate) fn resident(child: PreencodeChild) -> Self {
        let mut pages = Vec::with_capacity(child.pages.len());
        let mut texels = Vec::with_capacity(child.pages.len());
        let mut owned = 0u64;
        for page in child.pages {
            let mut levels = Vec::with_capacity(page.levels.len());
            let mut data = Vec::with_capacity(page.levels.len());
            for level in page.levels {
                levels.push(ChildLevel {
                    width: level.width,
                    height: level.height,
                });
                owned += level.rgba.capacity() as u64
                    + (level.samples.capacity() * std::mem::size_of::<TerminalSample>()) as u64;
                data.push((level.rgba, level.samples));
            }
            pages.push(ChildPage {
                width: page.width,
                height: page.height,
                slot: page.slot,
                material: page.material,
                sampler: page.sampler,
                normal_scale: page.normal_scale,
                levels,
            });
            texels.push(data);
        }
        owned += metadata_bytes(&pages);
        Self {
            node: child.node,
            texel_size: child.texel_size,
            faces: child.faces,
            pages,
            texels: Texels::Resident(texels),
            owned,
        }
    }
    /// Research base-colour leaf bridge: ordinary pre-encode pixels, no ledger.
    pub(super) fn plain_leaf(
        mut child: PreencodeChild,
        leaf: &crate::mesh_large::bake_pass::LeafPreencode,
    ) -> Result<Self, JobError> {
        use super::types::ChildBinding;
        let mut pages = Vec::new();
        let mut pixels = Vec::new();
        let mut owned = 0;
        let mut maps = std::collections::BTreeMap::new();
        let mut at = 0;
        for (w, p) in leaf.witness.primitives.iter().zip(&leaf.primitives) {
            for b in &w.bindings {
                // Prototype only changes base colour. Other PBR slots use direct source.
                if b.slot != 0 {
                    continue;
                }
                let key = (b.page, w.material);
                let page_index = if let Some(&i) = maps.get(&key) {
                    i
                } else {
                    let raw = leaf
                        .pages
                        .iter()
                        .find(|p| p.page == b.page)
                        .ok_or_else(|| invalid("experiment leaf page"))?;
                    let mut levels: Vec<ChildLevel> = raw
                        .levels
                        .iter()
                        .map(|l| ChildLevel {
                            width: l.width,
                            height: l.height,
                        })
                        .collect();
                    let mut data: Vec<Vec<u8>> =
                        raw.levels.iter().map(|l| l.rgba.clone()).collect();
                    while levels.last().is_some_and(|l| l.width > 1 || l.height > 1) {
                        let old = levels.last().unwrap();
                        let width = (old.width / 2).max(1);
                        let height = (old.height / 2).max(1);
                        let mut next = vec![0; (width * height * 4) as usize];
                        let prev = data.last().unwrap();
                        for y in 0..height {
                            for x in 0..width {
                                for c in 0..4 {
                                    let mut v = 0.;
                                    for dy in 0..2 {
                                        for dx in 0..2 {
                                            let i = ((((2 * y + dy).min(old.height - 1))
                                                * old.width
                                                + (2 * x + dx).min(old.width - 1))
                                                * 4
                                                + c)
                                                as usize;
                                            let a = f64::from(prev[i]) / 255.;
                                            v += if c < 3 {
                                                if a <= 0.04045 {
                                                    a / 12.92
                                                } else {
                                                    ((a + 0.055) / 1.055).powf(2.4)
                                                }
                                            } else {
                                                a
                                            };
                                        }
                                    }
                                    v /= 4.;
                                    if c < 3 {
                                        v = if v <= 0.0031308 {
                                            v * 12.92
                                        } else {
                                            1.055 * v.powf(1. / 2.4) - 0.055
                                        };
                                    }
                                    next[((y * width + x) * 4 + c) as usize] =
                                        (v * 255.).round().clamp(0., 255.) as u8;
                                }
                            }
                        }
                        levels.push(ChildLevel { width, height });
                        data.push(next);
                    }
                    owned += data.iter().map(|p| p.capacity() as u64).sum::<u64>();
                    let i = pages.len();
                    pages.push(ChildPage {width:raw.width,height:raw.height,slot:TextureSlot::BaseColor,
                        material:w.material,sampler:serde_json::json!({"minFilter":raw.min_filter,"magFilter":raw.mag_filter,"wrapS":33071,"wrapT":33071}),normal_scale:1.,levels});
                    pixels.push(data);
                    maps.insert(key, i);
                    i
                };
                let uv = &p.uvs[b.texcoord as usize];
                for i in 0..w.faces.len() {
                    if w.faces[i].zero_raster & 1 == 0 {
                        child.faces[at + i].bindings[0] = Some(ChildBinding {
                            page: page_index,
                            uv: std::array::from_fn(|c| uv[3 * i + c]),
                        });
                    }
                }
            }
            at += w.faces.len();
        }
        owned += metadata_bytes(&pages);
        Ok(Self {
            node: child.node,
            texel_size: child.texel_size,
            faces: child.faces,
            pages,
            texels: Texels::PlainResident(pixels),
            owned,
        })
    }
    /// A child whose texels stay in its packet `file` (`path`), from its opened head.
    pub(super) fn packet(opened: packet::Opened, file: std::fs::File, path: PathBuf) -> Self {
        let pages: Vec<ChildPage> = opened
            .pages
            .into_iter()
            .map(|p| ChildPage {
                width: p.width,
                height: p.height,
                slot: p.slot,
                material: p.material,
                sampler: p.sampler,
                normal_scale: p.normal_scale,
                levels: p
                    .levels
                    .iter()
                    .map(|&[width, height]| ChildLevel { width, height })
                    .collect(),
            })
            .collect();
        // The decode charged the faces too; they are charged with every child's faces.
        let faces = (opened.faces.capacity() * std::mem::size_of::<ChildFace>()) as u64;
        Self {
            node: opened.node,
            texel_size: opened.texel_size,
            faces: opened.faces,
            owned: opened
                .owned
                .saturating_sub(faces)
                .max(metadata_bytes(&pages)),
            pages,
            texels: Texels::Packet {
                file,
                path,
                offsets: opened.offsets,
            },
        }
    }
    /// Bytes this child owns beyond its faces.
    pub(super) fn owned_bytes(&self) -> u64 {
        self.owned
    }
    /// Whether its texels are read from its packet (through a [`TexelCache`]).
    pub(super) fn reads_packet(&self) -> bool {
        matches!(self.texels, Texels::Packet { .. }) && !self.pages.is_empty()
    }
}

/// Bytes one cache slot may own: a decoded tile, its slot record and its map entry.
pub(crate) const SLOT_BYTES: u64 = (TILE * TILE) as u64
    * (4 + std::mem::size_of::<TerminalSample>() as u64)
    + std::mem::size_of::<Slot>() as u64
    + 64;
/// The fewest slots a cache may have: a bilinear tap pair on two levels touches at most
/// eight tiles.
pub(crate) const MIN_SLOTS: usize = 8;

/// A tile key's hash: keys are dense integers, so one multiply spreads them.
#[derive(Default)]
struct TileHasher(u64);
impl Hasher for TileHasher {
    fn finish(&self) -> u64 {
        self.0
    }
    fn write(&mut self, bytes: &[u8]) {
        for &b in bytes {
            self.0 = (self.0 ^ u64::from(b)).wrapping_mul(0x0000_0100_0000_01b3);
        }
    }
    fn write_u64(&mut self, n: u64) {
        self.0 = n.wrapping_mul(0x9e37_79b9_7f4a_7c15);
    }
}
struct Slot {
    key: u64,
    referenced: bool,
    /// The tile's clipped width.
    width: u32,
    rgba: Vec<u8>,
    samples: Vec<TerminalSample>,
}
/// Decoded child tiles, at most `capacity` of them, evicted by the clock (second
/// chance) rule. Eviction changes only which tiles are resident, never a texel value.
pub(super) struct TexelCache {
    capacity: usize,
    slots: Vec<Slot>,
    map: HashMap<u64, u32, BuildHasherDefault<TileHasher>>,
    hand: usize,
    buf: Vec<u8>,
    /// The latest tile read and its slot: neighbouring taps mostly share a tile.
    last: Option<(u64, u32)>,
    /// The number of the first tile of each child page level, by child, page and level.
    bases: Vec<Vec<Vec<u64>>>,
    /// Tiles read from packets (diagnostics).
    pub reads: u64,
}
impl TexelCache {
    /// A cache owning at most `memory` bytes. A child set that reads no packet needs no
    /// cache and gets an empty one.
    pub(super) fn new(children: &[ChildView], memory: u64) -> Result<Self, JobError> {
        let lazy = children.iter().any(ChildView::reads_packet);
        let capacity = if lazy {
            let slots = usize::try_from(memory / SLOT_BYTES).unwrap_or(usize::MAX);
            if slots < MIN_SLOTS {
                return Err(resource("child texel cache lease"));
            }
            slots
        } else {
            0
        };
        let mut next = 0u64;
        let bases = children
            .iter()
            .map(|c| {
                c.pages
                    .iter()
                    .map(|p| {
                        p.levels
                            .iter()
                            .map(|l| {
                                let base = next;
                                next += u64::from(l.width.div_ceil(TILE))
                                    * u64::from(l.height.div_ceil(TILE));
                                base
                            })
                            .collect()
                    })
                    .collect()
            })
            .collect();
        Ok(Self {
            capacity,
            slots: Vec::new(),
            map: HashMap::default(),
            hand: 0,
            buf: Vec::new(),
            last: None,
            bases,
            reads: 0,
        })
    }
    /// Texel `(x, y)` of `level` of `page` of `children[child]`: its RGBA and terminal
    /// sample.
    pub(super) fn texel(
        &mut self,
        children: &[ChildView],
        child: usize,
        page: usize,
        level: usize,
        x: u32,
        y: u32,
    ) -> Result<([u8; 4], TerminalSample), JobError> {
        let view = &children[child];
        let shape = &view.pages[page].levels[level];
        if x >= shape.width || y >= shape.height {
            return Err(invalid("child terminal ledger shape"));
        }
        let (file, path, offset) = match &view.texels {
            Texels::PlainResident(levels) => {
                let i = (y as usize * shape.width as usize + x as usize) * 4;
                return Ok((
                    levels[page][level][i..i + 4].try_into().unwrap(),
                    super::experiment::placeholder(view.node.first),
                ));
            }
            Texels::Resident(levels) => {
                let (rgba, samples) = &levels[page][level];
                let i = y as usize * shape.width as usize + x as usize;
                let t = *samples
                    .get(i)
                    .ok_or_else(|| invalid("child terminal ledger shape"))?;
                let c = rgba
                    .get(i * 4..i * 4 + 4)
                    .ok_or_else(|| invalid("child terminal ledger shape"))?;
                return Ok((c.try_into().unwrap(), t));
            }
            Texels::Packet {
                file,
                path,
                offsets,
            } => (file, path, offsets[page][level]),
        };
        let (tx, ty) = (x / TILE, y / TILE);
        let key = self.bases[child][page][level]
            + u64::from(ty) * u64::from(shape.width.div_ceil(TILE))
            + u64::from(tx);
        let slot = match self.last {
            Some((k, s)) if k == key => s as usize,
            _ => {
                let slot = match self.map.get(&key) {
                    Some(&s) => s as usize,
                    None => {
                        let s = self.victim()?;
                        let slot = &mut self.slots[s];
                        packet::read_tile(
                            file,
                            path,
                            &view.node,
                            [shape.width, shape.height],
                            offset,
                            [tx, ty],
                            &mut self.buf,
                            &mut slot.rgba,
                            &mut slot.samples,
                        )?;
                        slot.key = key;
                        slot.width = TILE.min(shape.width - tx * TILE);
                        self.map.insert(key, s as u32);
                        self.reads += 1;
                        s
                    }
                };
                self.last = Some((key, slot as u32));
                slot
            }
        };
        let s = &mut self.slots[slot];
        s.referenced = true;
        let i = ((y - ty * TILE) * s.width + (x - tx * TILE)) as usize;
        Ok((s.rgba[i * 4..i * 4 + 4].try_into().unwrap(), s.samples[i]))
    }
    /// A free slot: a new one below capacity, else the first unreferenced one from the
    /// clock hand (clearing reference bits on the way), whose tile is dropped.
    fn victim(&mut self) -> Result<usize, JobError> {
        if self.slots.len() < self.capacity {
            let mut samples = Vec::new();
            samples
                .try_reserve_exact((TILE * TILE) as usize)
                .map_err(|_| resource("child texel cache slot"))?;
            self.slots.push(Slot {
                key: u64::MAX,
                referenced: false,
                width: 0,
                rgba: vec![0; (TILE * TILE * 4) as usize],
                samples,
            });
            return Ok(self.slots.len() - 1);
        }
        if self.capacity == 0 {
            return Err(invalid("child texel cache has no slot"));
        }
        loop {
            let s = self.hand;
            self.hand = (self.hand + 1) % self.capacity;
            let slot = &mut self.slots[s];
            if slot.referenced {
                slot.referenced = false;
                continue;
            }
            self.map.remove(&slot.key);
            if self.last.is_some_and(|(_, l)| l as usize == s) {
                self.last = None;
            }
            return Ok(s);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mesh_large::{bake::packet::tests as fixture, batching::MaterialClasses};

    /// Eight children of large pages, read in a scattered order through the smallest
    /// cache: every texel equals the resident child's, the cache never holds more than
    /// its slots (so it evicts and reads tiles again), and a packet child owns only its
    /// head however large its pages.
    #[test]
    fn many_large_children_read_through_a_small_cache_return_resident_texels() {
        let mut lazy = Vec::new();
        let mut resident = Vec::new();
        let mut dirs = Vec::new();
        for c in 0..8u64 {
            let mut child = fixture::child();
            child
                .pages
                .push(fixture::page(96 + c as u32, 80, TextureSlot::BaseColor, c));
            child
                .pages
                .push(fixture::page(40, 33, TextureSlot::Emissive, 100 + c));
            let bytes = fixture::encode(&child);
            let (dir, path, file) = fixture::file(&bytes);
            let opened = packet::open(&file, &path, &MaterialClasses::EMPTY, 1 << 20, &mut || {
                Ok(())
            })
            .unwrap();
            lazy.push(ChildView::packet(opened, file, path));
            resident.push(ChildView::resident(child));
            dirs.push(dir);
        }
        // A packet child owns its head only: the same for a page sixteen times larger.
        let small = {
            let mut child = fixture::child();
            child
                .pages
                .push(fixture::page(64, 64, TextureSlot::BaseColor, 0));
            fixture::encode(&child)
        };
        let large = {
            let mut child = fixture::child();
            child
                .pages
                .push(fixture::page(256, 256, TextureSlot::BaseColor, 0));
            fixture::encode(&child)
        };
        let owned = |bytes: &[u8]| {
            let (_dir, path, file) = fixture::file(bytes);
            packet::open(&file, &path, &MaterialClasses::EMPTY, 1 << 20, &mut || {
                Ok(())
            })
            .unwrap()
            .owned
        };
        // (Two more levels add two offsets.)
        assert!(owned(&large) <= owned(&small) + 64);
        assert!(large.len() > 16 * owned(&large) as usize);

        let mut cache = TexelCache::new(&lazy, MIN_SLOTS as u64 * SLOT_BYTES).unwrap();
        let mut whole = TexelCache::new(&resident, 0).unwrap();
        assert!(TexelCache::new(&lazy, MIN_SLOTS as u64 * SLOT_BYTES - 1).is_err());
        let mut state = 0x2545_f491_4f6c_dd1du64;
        let mut next = move |n: u32| {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            (state % u64::from(n)) as u32
        };
        for _ in 0..50_000 {
            let child = next(8) as usize;
            let page = next(2) as usize;
            let levels = &lazy[child].pages[page].levels;
            let level = next(levels.len() as u32) as usize;
            let (w, h) = (levels[level].width, levels[level].height);
            let (x, y) = (next(w), next(h));
            let (a, s) = cache.texel(&lazy, child, page, level, x, y).unwrap();
            let (b, t) = whole.texel(&resident, child, page, level, x, y).unwrap();
            assert_eq!(a, b);
            assert_eq!(s.source_rank, t.source_rank);
            for (u, v) in [
                (s.barycentric, t.barycentric),
                (s.centre, t.centre),
                (s.support_min, t.support_min),
                (s.support_max, t.support_max),
            ] {
                assert_eq!(u.map(f64::to_bits), v.map(f64::to_bits));
            }
            assert_eq!(
                (s.weight, s.centre_is_centroid),
                (t.weight, t.centre_is_centroid)
            );
            assert!(cache.slots.len() <= MIN_SLOTS);
        }
        assert!(cache.reads > 1000, "{} tile reads", cache.reads);
        assert_eq!(whole.reads, 0);
        // A texel outside its level is refused.
        assert!(cache.texel(&lazy, 0, 0, 0, 96, 0).is_err());
    }
}
