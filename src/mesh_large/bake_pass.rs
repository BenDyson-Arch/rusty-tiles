//! Root side of the proxy texture bake (H13/H27): the hook after `lod::produce`, the
//! retained pre-encode leaf appearance, and the complete-member GLB adapter.
//!
//! The bake core (`bake/*`, owned by the bake author) implements [`ProxyBaker`]. The root
//! runs it right after the coarse tiles are certified, while the capture (source triangles,
//! images, materials) and the staging directory are still live. For each tile, postorder
//! (children before parents), the baker reads the tile's [`FrozenNode`], the retained
//! pre-encode appearance of its children, and calls [`BakePass::assemble`] with a
//! [`PreparedNode`]. The adapter copies the certified POSITION words and TRS exactly,
//! attaches every feature-ID set and every property table, the requested corner attributes,
//! images, samplers and materials, counts the complete GLB, and reads all of it back from the
//! actual bytes. Only an `Assembly::Fit` result may be committed; the tile's geometric error
//! never changes. Nothing here decodes PNG/KTX2 as donor input.
use super::{
    config::Profile,
    full::FullCaptured,
    leaves::prepared::{PreencodeLevel, PreencodePage, PreparedAtlas},
    lod::{
        codec,
        handoff::{read_bounded, FrozenNode, HandoffSink, Lease, ScratchBudget},
        mesh::Origin,
    },
};
use crate::{
    glb::{self, MeshoptLayout, MeshoptStream},
    gltf_source::source::Material,
    JobError, JobErrorKind,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeSet,
    fs,
    io::{BufRead, BufReader, Write},
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};

fn state(message: impl Into<String>) -> JobError {
    JobError::new(JobErrorKind::InvalidState, message.into())
}

/// The bake core seam. Implemented by `bake/*`; `None` (the default) leaves proxies
/// untextured.
pub(in crate::mesh_large) trait ProxyBaker:
    Send + Sync + std::fmt::Debug
{
    /// Bake every coarse tile through `pass`, children before parents, committing each
    /// accepted member with [`BakePass::commit`].
    fn bake(&self, pass: &BakePass<'_>) -> Result<(), JobError>;
}

// ---------------------------------------------------------------------------------------
// Retained pre-encode appearance.

/// One leaf primitive's pre-encode appearance closure.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(in crate::mesh_large) struct LeafPrimitivePreencode {
    /// Remapped leaf material (texture indices into the leaf's pages), if any.
    pub material: Option<Value>,
    /// Per triangle, `(node, mesh, primitive, triangle)` source provenance.
    pub triangles: Vec<(u32, u32, u32, u64)>,
    /// Per TEXCOORD set, three page UVs per triangle.
    pub uvs: Vec<Vec<[f32; 2]>>,
    pub feature_id_textures: Vec<Value>,
}

/// A leaf's accepted pre-encode appearance (its frozen layout, density and drop values).
#[derive(Clone, Debug, PartialEq)]
pub(in crate::mesh_large) struct LeafPreencode {
    pub pages: Vec<PreencodePage>,
    pub primitives: Vec<LeafPrimitivePreencode>,
    pub textures: Vec<Value>,
    pub samplers: Vec<Value>,
}

#[derive(Serialize, Deserialize)]
struct LeafHead {
    /// The accepted density attempt (its page files are kept).
    drop: u32,
    /// Page metadata (pixel buffers live in the page files).
    pages: Vec<PreencodePage>,
    primitives: Vec<LeafPrimitivePreencode>,
    textures: Vec<Value>,
    samplers: Vec<Value>,
}

const PAGE_MAGIC: &[u8; 8] = b"H17PAG\x01\0";
const LEAF_MAGIC: &[u8; 8] = b"H17PRE\x02\0";

/// A page's pixel bytes: every level's RGBA, then the support mask as u32 LE.
fn page_pixel_bytes(page: &PreencodePage) -> u64 {
    page.levels.iter().map(|l| l.rgba.len() as u64).sum::<u64>()
        + page.support.as_ref().map_or(0, |s| 4 * s.len() as u64)
}

/// Metadata of a page: its pixel buffers emptied (`support` stays `Some` when present).
fn page_metadata(page: &PreencodePage) -> PreencodePage {
    PreencodePage {
        page: page.page,
        slot: page.slot.clone(),
        class: page.class.clone(),
        width: page.width,
        height: page.height,
        levels: page
            .levels
            .iter()
            .map(|l| PreencodeLevel {
                width: l.width,
                height: l.height,
                rgba: Vec::new(),
            })
            .collect(),
        support: page.support.as_ref().map(|_| Vec::new()),
        min_filter: page.min_filter,
        mag_filter: page.mag_filter,
        density_drop: page.density_drop,
        source_mips: page.source_mips,
    }
}

/// Fill a page's metadata with the pixels of its page file: the file's size must equal
/// what the metadata implies, and every buffer is charged to `lease` before it is
/// allocated; reads are chunked with `check` between them.
fn read_page(
    mut page: PreencodePage,
    path: &Path,
    lease: &mut Lease,
    check: &dyn Fn() -> Result<(), JobError>,
) -> Result<PreencodePage, JobError> {
    use std::io::Read;
    let bad = || state("retained pre-encode page is corrupt");
    let texels = |w: u32, h: u32| -> Result<usize, JobError> {
        (w as usize)
            .checked_mul(h as usize)
            .and_then(|x| x.checked_mul(4))
            .ok_or_else(bad)
    };
    let mut expected = 8u64;
    for l in &page.levels {
        expected = expected
            .checked_add(texels(l.width, l.height)? as u64)
            .ok_or_else(bad)?;
    }
    if page.support.is_some() {
        expected = expected
            .checked_add(texels(page.width, page.height)? as u64)
            .ok_or_else(bad)?;
    }
    check()?;
    let io = |e| JobError::io("read pre-encode page", path, e);
    let mut file = fs::File::open(path).map_err(io)?;
    if file.metadata().map_err(io)?.len() != expected {
        return Err(bad());
    }
    let mut read = |out: &mut [u8]| -> Result<(), JobError> {
        for chunk in out.chunks_mut(1 << 20) {
            check()?;
            file.read_exact(chunk).map_err(io)?;
        }
        Ok(())
    };
    let mut magic = [0u8; 8];
    read(&mut magic)?;
    if &magic != PAGE_MAGIC {
        return Err(bad());
    }
    for l in &mut page.levels {
        let len = texels(l.width, l.height)?;
        lease.charge(len as u64)?;
        l.rgba = vec![0; len];
        read(&mut l.rgba)?;
    }
    if page.support.is_some() {
        let len = texels(page.width, page.height)?;
        // The raw bytes and the decoded words coexist briefly.
        lease.charge(2 * len as u64)?;
        let mut raw = vec![0u8; len];
        read(&mut raw)?;
        page.support = Some(
            raw.as_chunks::<4>()
                .0
                .iter()
                .map(|c| u32::from_le_bytes(*c))
                .collect(),
        );
    }
    Ok(page)
}

/// Owned scratch for pre-encode appearance: accepted leaf packets (written by H8 while
/// encoding) and bake-produced proxy packets (opaque to the root), each kept until its
/// parent has consumed it. Every byte is charged against `limit`.
pub(in crate::mesh_large) struct PreencodeStore {
    dir: PathBuf,
    budget: std::sync::Arc<ScratchBudget>,
    live: AtomicU64,
    peak: AtomicU64,
    /// Per in-flight leaf, the `(drop, page)` files written by its density attempts; the
    /// accepted attempt's are kept by [`PreencodeStore::write_leaf`], the others removed.
    attempts: std::sync::Mutex<std::collections::BTreeMap<u64, Vec<(u32, u32)>>>,
}

impl PreencodeStore {
    pub(in crate::mesh_large) fn new(
        dir: PathBuf,
        budget: std::sync::Arc<ScratchBudget>,
    ) -> Result<Self, JobError> {
        fs::create_dir_all(&dir).map_err(|e| JobError::io("create pre-encode store", &dir, e))?;
        Ok(Self {
            dir,
            budget,
            live: AtomicU64::new(0),
            peak: AtomicU64::new(0),
            attempts: Default::default(),
        })
    }
    fn charge(&self, size: u64) -> Result<(), JobError> {
        self.budget.charge(size, "retained pre-encode appearance")?;
        let live = self.live.fetch_add(size, Ordering::Relaxed) + size;
        self.peak.fetch_max(live, Ordering::Relaxed);
        Ok(())
    }
    fn put(&self, path: &Path, bytes: &[u8]) -> Result<(), JobError> {
        self.charge(bytes.len() as u64)?;
        crate::spill::preflight(
            &self.dir,
            bytes.len() as u64,
            crate::spill::SpaceRole::Scratch,
            crate::spill::PreflightStage::Texture,
        )?;
        fs::write(path, bytes).map_err(|e| JobError::io("write pre-encode store", path, e))
    }
    fn remove(&self, path: &Path) -> Result<(), JobError> {
        let size = fs::metadata(path)
            .map_err(|e| JobError::io("inspect pre-encode store", path, e))?
            .len();
        fs::remove_file(path).map_err(|e| JobError::io("remove pre-encode store", path, e))?;
        self.live.fetch_sub(size, Ordering::Relaxed);
        self.budget.refund(size);
        Ok(())
    }
    fn leaf_path(&self, index: u64) -> PathBuf {
        self.dir.join(format!("leaf-{index:012}.pre"))
    }
    fn page_path(&self, index: u64, drop: u32, page: u32) -> PathBuf {
        self.dir
            .join(format!("leaf-{index:012}-d{drop}-p{page}.pag"))
    }
    fn proxy_path(&self, first: u64, depth: u32) -> PathBuf {
        self.dir.join(format!("proxy-f{first}-d{depth}.pre"))
    }
    /// Spill one encoded page's pre-encode pixels while the page's memory lease is held
    /// (no copy: streamed from `page`); returns its metadata for the leaf's atlas. Called
    /// by H8 for every page of every density attempt of leaf `index`.
    pub(in crate::mesh_large) fn write_page(
        &self,
        index: u64,
        drop: u32,
        page: &PreencodePage,
    ) -> Result<PreencodePage, JobError> {
        let path = self.page_path(index, drop, page.page);
        let size = 8 + page_pixel_bytes(page);
        self.charge(size)?;
        crate::spill::preflight(
            &self.dir,
            size,
            crate::spill::SpaceRole::Scratch,
            crate::spill::PreflightStage::Texture,
        )?;
        let io = |e| JobError::io("write pre-encode page", &path, e);
        let mut out = std::io::BufWriter::new(fs::File::create(&path).map_err(io)?);
        out.write_all(PAGE_MAGIC).map_err(io)?;
        for l in &page.levels {
            out.write_all(&l.rgba).map_err(io)?;
        }
        if let Some(support) = &page.support {
            for v in support {
                out.write_all(&v.to_le_bytes()).map_err(io)?;
            }
        }
        out.flush().map_err(io)?;
        self.attempts
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .entry(index)
            .or_default()
            .push((drop, page.page));
        Ok(page_metadata(page))
    }
    /// Record a leaf's ACCEPTED density attempt: its head (page metadata, primitives,
    /// textures, samplers; no pixels) is written, the attempt's page files are kept and every
    /// other attempt's page files are removed.
    pub(in crate::mesh_large) fn write_leaf(
        &self,
        index: u64,
        drop: u32,
        atlas: &PreparedAtlas,
    ) -> Result<(), JobError> {
        let head = serde_json::to_vec(&LeafHead {
            drop,
            pages: atlas.preencode.clone(),
            primitives: atlas
                .primitives
                .iter()
                .map(|p| LeafPrimitivePreencode {
                    material: p.material.clone(),
                    triangles: p
                        .triangles
                        .iter()
                        .map(|t| {
                            let s = t.triangle.source;
                            (s.node, s.mesh, s.primitive, s.triangle)
                        })
                        .collect(),
                    uvs: p.uvs.clone(),
                    feature_id_textures: p.feature_id_textures.clone(),
                })
                .collect(),
            textures: atlas.textures.clone(),
            samplers: atlas.samplers.clone(),
        })
        .map_err(|e| state(e.to_string()))?;
        let mut bytes = Vec::with_capacity(16 + head.len());
        bytes.extend_from_slice(LEAF_MAGIC);
        bytes.extend_from_slice(&(head.len() as u64).to_le_bytes());
        bytes.extend_from_slice(&head);
        self.put(&self.leaf_path(index), &bytes)?;
        let attempts = self
            .attempts
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .remove(&index)
            .unwrap_or_default();
        let kept: std::collections::BTreeSet<u32> =
            atlas.preencode.iter().map(|p| p.page).collect();
        for (d, page) in attempts {
            if d != drop {
                self.remove(&self.page_path(index, d, page))?;
            } else if !kept.contains(&page) {
                return Err(state("a retained page is not in its accepted leaf"));
            }
        }
        for page in &kept {
            if !self.page_path(index, drop, *page).exists() {
                return Err(state("an accepted leaf page was not retained"));
            }
        }
        Ok(())
    }
    /// A leaf's accepted pre-encode appearance within `memory_limit` bytes: the head
    /// file, its decoded structures and every page buffer are charged before allocation,
    /// page files must have exactly the size their metadata implies, and every read and
    /// decode step checks `check`.
    pub(in crate::mesh_large) fn read_leaf(
        &self,
        index: u64,
        memory_limit: u64,
        check: &dyn Fn() -> Result<(), JobError>,
    ) -> Result<LeafPreencode, JobError> {
        let path = self.leaf_path(index);
        let mut lease = Lease::new(memory_limit, "reading retained leaf appearance");
        let b = read_bounded(&path, None, &mut lease, check, "read pre-encode store")?;
        let bad = || state("retained pre-encode packet is corrupt");
        if b.get(..8) != Some(LEAF_MAGIC) {
            return Err(bad());
        }
        let n = u64::from_le_bytes(b.get(8..16).ok_or_else(bad)?.try_into().unwrap()) as usize;
        if 16usize.checked_add(n) != Some(b.len()) {
            return Err(bad());
        }
        // Decoded head structures: at most sixteen bytes per byte of JSON text.
        lease.charge_items(n as u64, 16)?;
        let head: LeafHead = serde_json::from_slice(&b[16..]).map_err(|_| bad())?;
        drop(b);
        let mut pages = Vec::with_capacity(head.pages.len());
        for page in head.pages {
            let path = self.page_path(index, head.drop, page.page);
            pages.push(read_page(page, &path, &mut lease, check)?);
        }
        Ok(LeafPreencode {
            pages,
            primitives: head.primitives,
            textures: head.textures,
            samplers: head.samplers,
        })
    }
    pub(in crate::mesh_large) fn release_leaf(&self, index: u64) -> Result<(), JobError> {
        let path = self.leaf_path(index);
        let b = fs::read(&path).map_err(|e| JobError::io("read pre-encode store", &path, e))?;
        let n = b
            .get(8..16)
            .map(|x| u64::from_le_bytes(x.try_into().unwrap()) as usize)
            .ok_or_else(|| state("retained pre-encode packet is corrupt"))?;
        let head: LeafHead = serde_json::from_slice(b.get(16..16 + n).unwrap_or_default())
            .map_err(|_| state("retained pre-encode packet is corrupt"))?;
        for page in &head.pages {
            self.remove(&self.page_path(index, head.drop, page.page))?;
        }
        self.remove(&path)
    }
    /// A bake-produced proxy packet (pages + source support), opaque to the root.
    pub(in crate::mesh_large) fn write_proxy(
        &self,
        first: u64,
        depth: u32,
        bytes: &[u8],
    ) -> Result<(), JobError> {
        self.put(&self.proxy_path(first, depth), bytes)
    }
    /// A bake-produced proxy packet within `memory_limit` bytes (bounded, cancellable).
    pub(in crate::mesh_large) fn read_proxy(
        &self,
        first: u64,
        depth: u32,
        memory_limit: u64,
        check: &dyn Fn() -> Result<(), JobError>,
    ) -> Result<Vec<u8>, JobError> {
        let path = self.proxy_path(first, depth);
        let mut lease = Lease::new(memory_limit, "reading a retained proxy packet");
        read_bounded(&path, None, &mut lease, check, "read pre-encode store")
    }
    /// Bytes of a retained proxy packet (a size query before any read).
    pub(in crate::mesh_large) fn proxy_size(
        &self,
        first: u64,
        depth: u32,
    ) -> Result<u64, JobError> {
        let path = self.proxy_path(first, depth);
        Ok(fs::metadata(&path)
            .map_err(|e| JobError::io("inspect pre-encode store", &path, e))?
            .len())
    }
    pub(in crate::mesh_large) fn release_proxy(
        &self,
        first: u64,
        depth: u32,
    ) -> Result<(), JobError> {
        self.remove(&self.proxy_path(first, depth))
    }
    /// `(live, peak)` charged bytes.
    pub(in crate::mesh_large) fn usage(&self) -> (u64, u64) {
        (
            self.live.load(Ordering::Relaxed),
            self.peak.load(Ordering::Relaxed),
        )
    }
}

// ---------------------------------------------------------------------------------------
// The adapter contract.

/// An image of the baked member.
#[derive(Clone, Debug, PartialEq)]
pub(in crate::mesh_large) struct PreparedImage {
    pub bytes: Vec<u8>,
    /// `image/png` or `image/ktx2`.
    pub mime: &'static str,
}

/// One output glTF primitive: a subset of ONE frozen glTF primitive's faces (render corners
/// may be split across several output primitives; each frozen face appears exactly once
/// over the whole node), with its corner attributes (three per face, in face order).
#[derive(Clone, Debug, PartialEq, Default)]
pub(in crate::mesh_large) struct PreparedPrimitive {
    /// Index into [`FrozenNode::gltf`]: its POSITION words, TRS and feature sets are copied.
    pub gltf: u32,
    /// Frozen face indices, each oriented as frozen.
    pub faces: Vec<u32>,
    /// Index into [`PreparedNode::materials`].
    pub material: Option<u32>,
    /// `TEXCOORD_n` streams.
    pub uvs: Vec<Vec<[f32; 2]>>,
    pub normals: Option<Vec<[f32; 3]>>,
    pub tangents: Option<Vec<[f32; 4]>>,
    pub colors: Option<Vec<[f32; 4]>>,
    /// Texture-based feature-ID sets (categorical ID pages), appended after the copied
    /// attribute sets; any number.
    pub extra_feature_ids: Vec<Value>,
    /// Its faces have no raster support (zero-area or no page): its material, if any,
    /// must bind no texture and it carries no UV sets. Raster primitives keep every slot
    /// their source materials bind.
    pub no_raster: bool,
}

/// Everything the baker asks the root to assemble.
#[derive(Clone, Debug, PartialEq, Default)]
pub(in crate::mesh_large) struct PreparedNode {
    pub primitives: Vec<PreparedPrimitive>,
    /// Output materials (texture indices into `textures`); source factors preserved, the
    /// source normalTexture.scale already absorbed (published as 1).
    pub materials: Vec<Value>,
    pub textures: Vec<Value>,
    pub samplers: Vec<Value>,
    pub images: Vec<PreparedImage>,
}

/// The decoded identity of one face of an assembled member, read back from its bytes:
/// geometry from POSITION, `source`/`material` from its `source_primitive` row (set 0) and
/// `origin` from that row plus its `proxy_source` row (set 1), each checked against the
/// frozen packet.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(in crate::mesh_large) struct FaceIdentity {
    /// Frozen face index.
    pub face: u32,
    pub raw: [[u32; 3]; 3],
    pub decoded: [[u32; 3]; 3],
    pub origin: Origin,
    /// `(node, mesh, primitive, expandedOrdinal)` of its source primitive.
    pub source: (u32, u32, u32, u64),
    /// Original material ID.
    pub material: u32,
    pub features: Vec<(u32, Option<u32>)>,
}

/// Only [`assemble`] can make one (private fields, not `Clone`): binds a `Fit` to the
/// frozen tile and budget it was verified for AND to the exact bytes it read back
/// (length and SHA-256), so any later change to those bytes is refused at commit.
#[derive(Debug, PartialEq, Eq)]
pub(in crate::mesh_large) struct Verified {
    first: u64,
    depth: u32,
    untextured: [u8; 32],
    tile_bytes: u64,
    length: u64,
    sha256: [u8; 32],
}

/// The adapter's verdict on one prepared node.
#[derive(Debug)]
pub(in crate::mesh_large) enum Assembly {
    /// The complete member fits the tile budget and every requested value read back.
    Fit {
        glb: Vec<u8>,
        decoded: Vec<FaceIdentity>,
        /// Proof that [`assemble`] produced and verified it, for this frozen tile.
        verified: Verified,
    },
    /// The complete member is larger than the tile budget (the only error that permits a
    /// density drop and rebake).
    PhysicalBudget { actual_bytes: u64 },
}

// ---------------------------------------------------------------------------------------
// The pass.

/// The bake pass over one producer run's coarse tiles.
pub(in crate::mesh_large) struct BakePass<'a> {
    pub captured: &'a FullCaptured,
    pub profile: Profile,
    pub tile_bytes: u64,
    pub jobs: usize,
    pub handoff: &'a HandoffSink,
    pub preencode: &'a PreencodeStore,
    pub check: &'a (dyn Fn() -> Result<(), JobError> + Sync),
    stage: PathBuf,
}

impl<'a> BakePass<'a> {
    #[allow(clippy::too_many_arguments)]
    pub(in crate::mesh_large) fn new(
        captured: &'a FullCaptured,
        profile: Profile,
        tile_bytes: u64,
        jobs: usize,
        stage: &Path,
        handoff: &'a HandoffSink,
        preencode: &'a PreencodeStore,
        check: &'a (dyn Fn() -> Result<(), JobError> + Sync),
    ) -> Self {
        Self {
            captured,
            profile,
            tile_bytes,
            jobs,
            handoff,
            preencode,
            check,
            stage: stage.to_owned(),
        }
    }

    /// Coarse tiles in postorder (children before parents) as `(first, depth)`, streamed
    /// from `proxies.jsonl`.
    pub(in crate::mesh_large) fn nodes(
        &self,
    ) -> Result<impl Iterator<Item = Result<(u64, u32), JobError>> + '_, JobError> {
        let path = self.stage.join("proxies.jsonl");
        let file = fs::File::open(&path).map_err(|e| JobError::io("read proxy rows", &path, e))?;
        Ok(BufReader::new(file).lines().map(move |line| {
            let line = line.map_err(|e| JobError::io("read proxy rows", &path, e))?;
            let row: Value = serde_json::from_str(&line).map_err(|e| state(e.to_string()))?;
            Ok((
                row["first"]
                    .as_u64()
                    .ok_or_else(|| state("proxy row without first"))?,
                row["depth"]
                    .as_u64()
                    .ok_or_else(|| state("proxy row without depth"))? as u32,
            ))
        }))
    }

    /// A tile's frozen packet, read and decoded within `memory_limit` bytes (charged
    /// before any allocation; [`BakePass::frozen_size`] gives the file size first).
    pub(in crate::mesh_large) fn frozen(
        &self,
        first: u64,
        depth: u32,
        memory_limit: u64,
    ) -> Result<FrozenNode, JobError> {
        self.handoff.read(first, depth, memory_limit, self.check)
    }

    /// Bytes of a tile's packet on disk.
    pub(in crate::mesh_large) fn frozen_size(
        &self,
        first: u64,
        depth: u32,
    ) -> Result<u64, JobError> {
        self.handoff.size(first, depth)
    }

    /// The certified untextured member of a tile, exactly as H17 published it: its length
    /// and SHA-256 come from the tile's packet (the sink), the file size is checked
    /// against them and charged to `memory_limit` before any allocation, the read is
    /// bounded and cancellable, and a hash mismatch (e.g. after `commit`) is
    /// `InvalidState`.
    pub(in crate::mesh_large) fn read_certified_proxy(
        &self,
        first: u64,
        depth: u32,
        memory_limit: u64,
    ) -> Result<Vec<u8>, JobError> {
        let (length, sha) = self.handoff.certified(first, depth, self.check)?;
        let path = self.stage.join(format!("proxies/f{first}-d{depth}.glb"));
        let mut lease = Lease::new(memory_limit, "reading a certified proxy");
        let bytes = read_bounded(
            &path,
            Some(length),
            &mut lease,
            self.check,
            "read certified proxy",
        )?;
        if <[u8; 32]>::from(Sha256::digest(&bytes)) != sha {
            return Err(state(format!(
                "certified proxy f{first}-d{depth} no longer matches its frozen SHA-256"
            )));
        }
        Ok(bytes)
    }

    /// Drop a tile's packet once it is baked.
    pub(in crate::mesh_large) fn release(&self, first: u64, depth: u32) -> Result<(), JobError> {
        self.handoff.release(first, depth)
    }

    /// The complete-member adapter, within `memory_limit` bytes for every buffer it owns
    /// (see [`assembly_bytes`]); refused with `ResourceLimit` before any allocation.
    pub(in crate::mesh_large) fn assemble(
        &self,
        frozen: &FrozenNode,
        prepared: &PreparedNode,
        memory_limit: u64,
    ) -> Result<Assembly, JobError> {
        (self.check)()?;
        assemble(
            frozen,
            prepared,
            &self.captured.materials,
            self.tile_bytes,
            memory_limit,
        )
    }

    /// Publish an accepted member over the tile's untextured proxy (geometric error
    /// unchanged; the proxy row's size and hash are refreshed by [`BakePass::finish`]).
    /// Only a `Fit` that [`BakePass::assemble`] verified for this tile is accepted; its
    /// bytes are checked again against the tile budget and decoded against the frozen
    /// faces, then written atomically (temporary file and rename).
    /// Consumes the assembly: only a `Fit` whose bytes are still exactly the bytes
    /// [`BakePass::assemble`] read back (length and SHA-256), for this tile and budget.
    pub(in crate::mesh_large) fn commit(
        &self,
        frozen: &FrozenNode,
        assembly: Assembly,
    ) -> Result<(), JobError> {
        (self.check)()?;
        let Assembly::Fit { glb, verified, .. } = assembly else {
            return Err(state("only a Fit assembly may be committed"));
        };
        let glb = &glb;
        let id = &frozen.identity;
        if verified.first != id.first
            || verified.depth != id.depth
            || verified.untextured != frozen.glb_sha256
            || verified.tile_bytes != self.tile_bytes
        {
            return Err(state("a Fit assembly committed to another tile or budget"));
        }
        if glb.len() as u64 != verified.length
            || <[u8; 32]>::from(Sha256::digest(glb)) != verified.sha256
        {
            return Err(state(
                "a Fit assembly's bytes changed after they were verified",
            ));
        }
        if glb.len() as u64 > self.tile_bytes {
            return Err(state("a committed member exceeds the tile budget"));
        }
        check_faces(frozen, glb)?;
        let path = self
            .stage
            .join(format!("proxies/f{}-d{}.glb", id.first, id.depth));
        if !path.exists() {
            return Err(state("a baked member has no proxy to replace"));
        }
        let next = path.with_extension("glb.next");
        let mut file =
            fs::File::create(&next).map_err(|e| JobError::io("write baked proxy", &next, e))?;
        file.write_all(glb)
            .and_then(|_| file.sync_all())
            .map_err(|e| JobError::io("write baked proxy", &next, e))?;
        drop(file);
        fs::rename(&next, &path).map_err(|e| JobError::io("replace baked proxy", &path, e))
    }

    /// Rewrite `proxies.jsonl` with every member's actual size and SHA-256 (streamed, one
    /// member at a time); returns the total proxy bytes.
    pub(in crate::mesh_large) fn finish(self) -> Result<u64, JobError> {
        let rows_path = self.stage.join("proxies.jsonl");
        let next_path = self.stage.join("proxies.jsonl.next");
        let input = fs::File::open(&rows_path)
            .map_err(|e| JobError::io("read proxy rows", &rows_path, e))?;
        let mut out = std::io::BufWriter::new(
            fs::File::create(&next_path)
                .map_err(|e| JobError::io("write proxy rows", &next_path, e))?,
        );
        let mut total = 0u64;
        for line in BufReader::new(input).lines() {
            let line = line.map_err(|e| JobError::io("read proxy rows", &rows_path, e))?;
            let mut row: Value = serde_json::from_str(&line).map_err(|e| state(e.to_string()))?;
            let uri = row["uri"]
                .as_str()
                .ok_or_else(|| state("proxy row without uri"))?
                .to_owned();
            let member = self.stage.join(&uri);
            let bytes = fs::read(&member).map_err(|e| JobError::io("read proxy", &member, e))?;
            total += bytes.len() as u64;
            row["bytes"] = json!(bytes.len());
            row["sha256"] = json!(Sha256::digest(&bytes)
                .iter()
                .map(|b| format!("{b:02x}"))
                .collect::<String>());
            serde_json::to_writer(&mut out, &row).map_err(|e| state(e.to_string()))?;
            out.write_all(b"\n")
                .map_err(|e| JobError::io("write proxy rows", &next_path, e))?;
        }
        out.flush()
            .map_err(|e| JobError::io("write proxy rows", &next_path, e))?;
        drop(out);
        fs::rename(&next_path, &rows_path)
            .map_err(|e| JobError::io("replace proxy rows", &rows_path, e))?;
        Ok(total)
    }
}

#[derive(Default)]
struct Wire {
    binary: Vec<u8>,
    views: Vec<Value>,
    streams: Vec<Option<MeshoptStream>>,
    accessors: Vec<Value>,
}

impl Wire {
    fn view(&mut self, bytes: &[u8], stream: Option<MeshoptStream>) -> usize {
        let offset = glb::append_aligned(&mut self.binary, 8, bytes);
        let mut v = json!({"buffer":0,"byteOffset":offset,"byteLength":bytes.len()});
        if let Some(MeshoptStream::Attributes { stride, .. }) = stream {
            v["byteStride"] = json!(stride);
            v["target"] = json!(34962);
        }
        self.views.push(v);
        self.streams.push(stream);
        self.views.len() - 1
    }
    fn floats<'a>(
        &mut self,
        values: impl Iterator<Item = &'a f32>,
        count: usize,
        width: usize,
        shape: &str,
    ) -> usize {
        let mut bytes = Vec::with_capacity(count * width * 4);
        for v in values {
            bytes.extend_from_slice(&v.to_le_bytes());
        }
        let v = self.view(
            &bytes,
            Some(MeshoptStream::Attributes {
                count,
                stride: width * 4,
            }),
        );
        self.accessors
            .push(json!({"bufferView":v,"componentType":5126,"count":count,"type":shape}));
        self.accessors.len() - 1
    }
}

fn component_width(component: &str) -> Result<usize, JobError> {
    Ok(match component {
        "UINT8" | "INT8" => 1,
        "UINT16" | "INT16" => 2,
        "UINT32" | "INT32" | "FLOAT32" => 4,
        "UINT64" | "INT64" | "FLOAT64" => 8,
        _ => return Err(state("frozen table column of unknown component type")),
    })
}

/// Upper bound of the memory [`assemble`] owns at once for `prepared`, summed over every
/// buffer it may hold: the face partition map, per-stream corner copies and wire bytes
/// (each stream twice: built, then appended), property tables, images, the JSON document,
/// meshopt output, the GLB, and the readback (decoded faces, streams, attributes, tables,
/// image copies and identities). Computed from sizes alone, without allocating.
pub(in crate::mesh_large) fn assembly_bytes(frozen: &FrozenNode, prepared: &PreparedNode) -> u64 {
    fn json_len(value: &impl Serialize) -> u64 {
        struct Count(u64);
        impl Write for Count {
            fn write(&mut self, b: &[u8]) -> std::io::Result<usize> {
                self.0 += b.len() as u64;
                Ok(b.len())
            }
            fn flush(&mut self) -> std::io::Result<()> {
                Ok(())
            }
        }
        let mut count = Count(0);
        let _ = serde_json::to_writer(&mut count, value);
        count.0
    }
    let s = |v: u64, w: u64| v.saturating_mul(w);
    let mut total = frozen.faces.len() as u64; // partition map
    let mut streams = 0u64; // accessor/view count for the JSON estimate
    let mut wire = 0u64;
    let mut readback = 0u64;
    let sets = |g: u32| {
        frozen
            .gltf
            .get(g as usize)
            .map_or(0, |g| g.feature_ids.as_array().map_or(0, Vec::len)) as u64
    };
    for p in &prepared.primitives {
        let faces = p.faces.len() as u64;
        let corners = 3 * faces;
        let sets = sets(p.gltf);
        // Corner copies, then per corner: POSITION (12), feature IDs (4 each, plus the
        // per-face ID list), NORMAL (12), TANGENT (16), COLOR (16), each UV set (8).
        total = total.saturating_add(s(
            corners,
            std::mem::size_of::<super::lod::handoff::FrozenCorner>() as u64,
        ));
        let per_corner = 12
            + 4 * sets
            + 12 * u64::from(p.normals.is_some())
            + 16 * u64::from(p.tangents.is_some())
            + 16 * u64::from(p.colors.is_some())
            + 8 * p.uvs.len() as u64;
        wire = wire.saturating_add(s(corners, per_corner) + s(faces, 4 * sets));
        streams += 1 + sets + 3 + p.uvs.len() as u64;
        // Readback: decoded faces with their feature lists, decoded/raw positions,
        // identities, and float attributes read back.
        readback = readback
            .saturating_add(s(
                faces,
                std::mem::size_of::<codec::DecodedFace>() as u64 + 16 * sets,
            ))
            .saturating_add(s(
                faces,
                std::mem::size_of::<FaceIdentity>() as u64 + 8 * sets,
            ))
            .saturating_add(s(corners, 24 + per_corner));
        total = total.saturating_add(json_len(&p.extra_feature_ids));
    }
    for t in &frozen.tables {
        for (_, _, values) in &t.columns {
            // Wire bytes (at most 8 per value) and the decoded u64 column.
            wire = wire.saturating_add(s(values.len() as u64, 8));
            readback = readback.saturating_add(s(values.len() as u64, 8));
            streams += 1;
        }
    }
    let images: u64 = prepared.images.iter().map(|i| i.bytes.len() as u64).sum();
    wire = wire.saturating_add(images);
    readback = readback.saturating_add(images);
    // JSON: the document tree and its text, generously per stream plus the literal
    // materials, textures, samplers and feature-ID declarations.
    let literal = json_len(&prepared.materials)
        + json_len(&prepared.textures)
        + json_len(&prepared.samplers)
        + frozen
            .gltf
            .iter()
            .map(|g| json_len(&g.feature_ids))
            .sum::<u64>();
    let json = s(streams + prepared.images.len() as u64 + 8, 1024).saturating_add(s(literal, 8));
    // Each stream is built then appended (2x); meshopt output is bounded by twice its
    // input plus block headers; the GLB holds the JSON text and the compressed buffer;
    // the readback decodes meshopt streams into buffers of the original size.
    total
        .saturating_add(s(wire, 2))
        .saturating_add(s(wire, 2).saturating_add(1 << 16))
        .saturating_add(s(wire, 2).saturating_add(json))
        .saturating_add(json)
        .saturating_add(s(wire, 2))
        .saturating_add(readback)
}

/// Build the complete member for `prepared` and verify it from its own bytes. Refused
/// with `ResourceLimit`, before any allocation, when [`assembly_bytes`] exceeds
/// `memory_limit` (on the PhysicalBudget path too).
pub(in crate::mesh_large) fn assemble(
    frozen: &FrozenNode,
    prepared: &PreparedNode,
    source_materials: &[Material],
    tile_bytes: u64,
    memory_limit: u64,
) -> Result<Assembly, JobError> {
    let needed = assembly_bytes(frozen, prepared);
    if needed > memory_limit {
        return Err(JobError::new(
            JobErrorKind::ResourceLimit,
            format!(
                "assembling this member may hold {needed} bytes, above its {memory_limit} byte \
                 memory lease"
            ),
        ));
    }
    frozen.validate()?;
    check_transforms(frozen)?;
    audit_materials(frozen, prepared, source_materials)?;
    // Every frozen face exactly once, each within its declared glTF primitive.
    let mut seen = vec![false; frozen.faces.len()];
    for p in &prepared.primitives {
        let g = frozen
            .gltf
            .get(p.gltf as usize)
            .ok_or_else(|| state("prepared primitive names no frozen glTF primitive"))?;
        if p.faces.is_empty() {
            return Err(state("prepared primitive without faces"));
        }
        let corners = 3 * p.faces.len();
        for &f in &p.faces {
            if !g.faces.contains(&f) || std::mem::replace(&mut seen[f as usize], true) {
                return Err(state(
                    "prepared faces are not a partition of the frozen faces",
                ));
            }
        }
        if p.uvs.iter().any(|u| u.len() != corners)
            || p.normals.as_ref().is_some_and(|v| v.len() != corners)
            || p.tangents.as_ref().is_some_and(|v| v.len() != corners)
            || p.colors.as_ref().is_some_and(|v| v.len() != corners)
            || p.material
                .is_some_and(|m| m as usize >= prepared.materials.len())
        {
            return Err(state("prepared corner attributes do not match their faces"));
        }
    }
    if seen.iter().any(|s| !s) {
        return Err(state("prepared node omits a frozen face"));
    }
    let mut w = Wire::default();
    let mut nodes = Vec::new();
    let mut meshes = Vec::new();
    let mut quantized = false;
    for (i, p) in prepared.primitives.iter().enumerate() {
        let g = &frozen.gltf[p.gltf as usize];
        let count = 3 * p.faces.len();
        let corners: Vec<_> = p
            .faces
            .iter()
            .flat_map(|&f| frozen.faces[f as usize].corners)
            .collect();
        let mut node = json!({"mesh":i});
        let position = if g.quantized {
            quantized = true;
            let mut bytes = Vec::with_capacity(count * 8);
            let mut min = [u32::MAX; 3];
            let mut max = [0u32; 3];
            for c in &corners {
                for a in 0..3 {
                    let code =
                        u16::try_from(c.raw[a]).map_err(|_| state("frozen code exceeds u16"))?;
                    bytes.extend_from_slice(&code.to_le_bytes());
                    min[a] = min[a].min(c.raw[a]);
                    max[a] = max[a].max(c.raw[a]);
                }
                bytes.extend_from_slice(&[0, 0]);
            }
            let v = w.view(&bytes, Some(MeshoptStream::Attributes { count, stride: 8 }));
            w.accessors.push(json!({"bufferView":v,"componentType":5123,"count":count,"type":"VEC3","normalized":false,"min":min,"max":max}));
            node["translation"] = json!(g.translation.map(|b| f64::from(f32::from_bits(b))));
            node["scale"] = json!(g.scale.map(|b| f64::from(f32::from_bits(b))));
            w.accessors.len() - 1
        } else {
            let mut bytes = Vec::with_capacity(count * 12);
            let mut min = [f32::INFINITY; 3];
            let mut max = [f32::NEG_INFINITY; 3];
            for c in &corners {
                for a in 0..3 {
                    bytes.extend_from_slice(&c.raw[a].to_le_bytes());
                    let v = f32::from_bits(c.raw[a]);
                    min[a] = min[a].min(v);
                    max[a] = max[a].max(v);
                }
            }
            let v = w.view(
                &bytes,
                Some(MeshoptStream::Attributes { count, stride: 12 }),
            );
            w.accessors.push(json!({"bufferView":v,"componentType":5126,"count":count,"type":"VEC3","min":min.map(f64::from),"max":max.map(f64::from)}));
            w.accessors.len() - 1
        };
        let mut attributes = json!({"POSITION":position});
        // Copied attribute feature-ID sets, each under its published attribute number.
        let sets = g.feature_ids.as_array().cloned().unwrap_or_default();
        let mut slot = 0usize;
        for set in &sets {
            let Some(attribute) = set.get("attribute").and_then(Value::as_u64) else {
                continue;
            };
            let ids = p
                .faces
                .iter()
                .map(|&f| {
                    frozen.faces[f as usize]
                        .features
                        .get(slot)
                        .map(|s| s.0)
                        .ok_or_else(|| state("frozen face lacks a feature-ID set"))
                })
                .collect::<Result<Vec<u32>, JobError>>()?;
            slot += 1;
            let (component, bytes) = feature_id_stream(&ids)?;
            let v = w.view(&bytes, Some(MeshoptStream::Attributes { count, stride: 4 }));
            w.accessors.push(
                json!({"bufferView":v,"componentType":component,"count":count,"type":"SCALAR"}),
            );
            attributes[format!("_FEATURE_ID_{attribute}")] = json!(w.accessors.len() - 1);
        }
        if let Some(n) = &p.normals {
            attributes["NORMAL"] = json!(w.floats(n.iter().flatten(), count, 3, "VEC3"));
        }
        if let Some(t) = &p.tangents {
            attributes["TANGENT"] = json!(w.floats(t.iter().flatten(), count, 4, "VEC4"));
        }
        if let Some(c) = &p.colors {
            attributes["COLOR_0"] = json!(w.floats(c.iter().flatten(), count, 4, "VEC4"));
        }
        for (set, uv) in p.uvs.iter().enumerate() {
            attributes[format!("TEXCOORD_{set}")] =
                json!(w.floats(uv.iter().flatten(), count, 2, "VEC2"));
        }
        let mut feature_ids = sets.clone();
        feature_ids.extend(p.extra_feature_ids.iter().cloned());
        let mut primitive = json!({"attributes":attributes,"mode":4,
            "extensions":{"EXT_mesh_features":{"featureIds":feature_ids}}});
        if let Some(m) = p.material {
            primitive["material"] = json!(m);
        }
        meshes.push(json!({"primitives":[primitive]}));
        nodes.push(node);
    }
    // Every frozen property table, in order (feature sets reference them by index).
    let mut classes = Map::new();
    let mut tables = Vec::new();
    for t in &frozen.tables {
        let mut schema = Map::new();
        let mut properties = Map::new();
        for (name, component, values) in &t.columns {
            let width = component_width(component)?;
            let mut bytes = Vec::with_capacity(values.len() * width);
            for v in values {
                bytes.extend_from_slice(&v.to_le_bytes()[..width]);
            }
            let id = w.view(&bytes, None);
            properties.insert(name.clone(), json!({"values":id}));
            schema.insert(
                name.clone(),
                json!({"type":"SCALAR","componentType":component,"required":true}),
            );
        }
        classes.insert(t.class.clone(), json!({"properties":schema}));
        tables.push(json!({"class":t.class,"count":t.count,"properties":properties}));
    }
    let mut images = Vec::new();
    let mut basis = false;
    for image in &prepared.images {
        basis |= image.mime == "image/ktx2";
        let v = w.view(&image.bytes, None);
        images.push(json!({"bufferView":v,"mimeType":image.mime}));
    }
    let Wire {
        binary,
        views,
        streams,
        accessors,
    } = w;
    let scene_nodes: Vec<usize> = (0..nodes.len()).collect();
    let mut document = json!({"asset":{"version":"2.0","generator":"rusty-tiles H17 proxy (baked)"},"scene":0,
        "scenes":[{"nodes":scene_nodes}],"nodes":nodes,"meshes":meshes,
        "buffers":[{"byteLength":binary.len()}],"bufferViews":views,"accessors":accessors,
        "extensionsUsed":["EXT_mesh_features","EXT_structural_metadata"],
        "extensions":{"EXT_structural_metadata":{"schema":{"id":"rusty_tiles_large_proxy_v1","classes":classes},"propertyTables":tables}}});
    for (field, list) in [
        ("materials", &prepared.materials),
        ("textures", &prepared.textures),
        ("samplers", &prepared.samplers),
    ] {
        if !list.is_empty() {
            document[field] = json!(list);
        }
    }
    if !images.is_empty() {
        document["images"] = json!(images);
    }
    if quantized {
        glb::add_extension(&mut document, "KHR_mesh_quantization", true)
            .map_err(|e| state(e.to_string()))?;
    }
    if basis {
        glb::add_extension(&mut document, "KHR_texture_basisu", true)
            .map_err(|e| state(e.to_string()))?;
    }
    let compressed = glb::meshopt_compress(
        &mut document,
        &binary,
        MeshoptLayout {
            align: 8,
            explicit_filter: true,
        },
        |i, _| Ok(streams[i]),
    )
    .map_err(|e| state(e.to_string()))?;
    let out = glb::encode_glb(&document, &compressed).map_err(|e| state(e.to_string()))?;
    if out.len() as u64 > tile_bytes {
        return Ok(Assembly::PhysicalBudget {
            actual_bytes: out.len() as u64,
        });
    }
    let decoded = verify(frozen, prepared, &out)?;
    let verified = Verified {
        first: frozen.identity.first,
        depth: frozen.identity.depth,
        untextured: frozen.glb_sha256,
        tile_bytes,
        length: out.len() as u64,
        sha256: Sha256::digest(&out).into(),
    };
    Ok(Assembly::Fit {
        glb: out,
        decoded,
        verified,
    })
}

/// The member's decoded faces are exactly the frozen faces (raw words, decoded bits and
/// feature IDs per oriented face), as a multiset: output primitives may reorder faces.
fn check_faces(frozen: &FrozenNode, glb: &[u8]) -> Result<(), JobError> {
    let decoded = codec::decode(glb)?;
    type FaceKey = ([[u32; 3]; 3], [[u32; 3]; 3], Vec<(u32, Option<u32>)>);
    let mut got: Vec<FaceKey> = decoded
        .faces
        .iter()
        .map(|d| {
            (
                d.raw,
                d.p.map(|q| q.map(f32::to_bits)),
                d.features
                    .iter()
                    .map(|(id, t)| (*id, t.map(|t| t as u32)))
                    .collect(),
            )
        })
        .collect();
    let mut want: Vec<FaceKey> = frozen
        .faces
        .iter()
        .map(|f| {
            (
                f.corners.map(|c| c.raw),
                f.corners.map(|c| c.decoded),
                f.features.clone(),
            )
        })
        .collect();
    got.sort_unstable();
    want.sort_unstable();
    if got != want {
        return Err(state(
            "a committed member does not decode to its frozen faces",
        ));
    }
    Ok(())
}

/// The supported H17 transform, checked at the owner before anything is assembled: every
/// frozen glTF primitive has a finite translation and a bit-identical, positive, finite
/// uniform scale (identity included). Anything else is a contract error.
fn check_transforms(frozen: &FrozenNode) -> Result<(), JobError> {
    for g in &frozen.gltf {
        let t = g.translation.map(f32::from_bits);
        let scale = f32::from_bits(g.scale[0]);
        if t.iter().any(|v| !v.is_finite())
            || g.scale.iter().any(|&b| b != g.scale[0])
            || !(scale.is_finite() && scale > 0.)
        {
            return Err(state(format!(
                "frozen glTF primitive {}/{}: transform is not a finite translation with a \
                 positive uniform scale",
                g.mesh, g.index
            )));
        }
    }
    Ok(())
}

/// The literal (non-texture) factors a baked material must keep from its source material,
/// with glTF defaults filled in.
fn literal_factors(material: &Value) -> [Value; 7] {
    let pbr = |k: &str, default: Value| {
        material
            .get("pbrMetallicRoughness")
            .and_then(|p| p.get(k))
            .cloned()
            .unwrap_or(default)
    };
    let top = |k: &str, default: Value| material.get(k).cloned().unwrap_or(default);
    let alpha = top("alphaMode", json!("OPAQUE"));
    let mask = alpha == json!("MASK");
    [
        pbr("baseColorFactor", json!([1.0, 1.0, 1.0, 1.0])),
        pbr("metallicFactor", json!(1.0)),
        pbr("roughnessFactor", json!(1.0)),
        top("emissiveFactor", json!([0.0, 0.0, 0.0])),
        top("doubleSided", json!(false)),
        alpha,
        if mask {
            top("alphaCutoff", json!(0.5))
        } else {
            Value::Null
        },
    ]
}

/// Material, remap and no-raster semantics at the owner, independent of the baker:
/// every texture, image, sampler and UV-set reference resolves; a normal texture is
/// published with scale 1 (the source scale is absorbed into the baked normals); each
/// output material keeps its faces' source literal factors; a raster primitive keeps every
/// slot its sources bind, and a no-raster primitive binds none and carries no UVs.
fn audit_materials(
    frozen: &FrozenNode,
    prepared: &PreparedNode,
    sources: &[Material],
) -> Result<(), JobError> {
    let bad = |what: String| Err(state(format!("prepared appearance: {what}")));
    for (i, t) in prepared.textures.iter().enumerate() {
        let source = t
            .get("source")
            .or_else(|| t.pointer("/extensions/KHR_texture_basisu/source"))
            .and_then(Value::as_u64);
        if source.is_none_or(|s| s as usize >= prepared.images.len()) {
            return bad(format!("texture {i} names no image"));
        }
        if t.get("sampler")
            .and_then(Value::as_u64)
            .is_some_and(|s| s as usize >= prepared.samplers.len())
        {
            return bad(format!("texture {i} names no sampler"));
        }
    }
    let infos = |m: &Value| -> Vec<(String, Value)> {
        Material::slot_pointers()
            .filter_map(|p| m.pointer(&p).map(|v| (p, v.clone())))
            .collect()
    };
    for (i, m) in prepared.materials.iter().enumerate() {
        for (slot, info) in infos(m) {
            if info
                .get("index")
                .and_then(Value::as_u64)
                .is_none_or(|t| t as usize >= prepared.textures.len())
            {
                return bad(format!("material {i} {slot} names no texture"));
            }
            if slot.ends_with("normalTexture")
                && info.get("scale").is_some_and(|s| s.as_f64() != Some(1.0))
            {
                return bad(format!(
                    "material {i} publishes normalTexture.scale {} (the source scale must be \
                     absorbed and published as 1)",
                    info["scale"]
                ));
            }
        }
    }
    for (k, p) in prepared.primitives.iter().enumerate() {
        let material = p.material.and_then(|m| prepared.materials.get(m as usize));
        let bound = material.map(infos).unwrap_or_default();
        for (slot, info) in &bound {
            let uv = info.get("texCoord").and_then(Value::as_u64).unwrap_or(0) as usize;
            if uv >= p.uvs.len() {
                return bad(format!("primitive {k} {slot} reads TEXCOORD_{uv} it lacks"));
            }
        }
        for set in &p.extra_feature_ids {
            if let Some(texture) = set.get("texture") {
                let uv = texture.get("texCoord").and_then(Value::as_u64).unwrap_or(0) as usize;
                if texture
                    .get("index")
                    .and_then(Value::as_u64)
                    .is_none_or(|t| t as usize >= prepared.textures.len())
                    || uv >= p.uvs.len()
                {
                    return bad(format!("primitive {k} has a dangling feature-ID texture"));
                }
            }
        }
        if p.no_raster && (!bound.is_empty() || !p.uvs.is_empty()) {
            return bad(format!(
                "no-raster primitive {k} binds a texture or carries UVs"
            ));
        }
        // Source materials of the primitive's faces (identity survives local splits).
        let mut seen = BTreeSet::new();
        for &f in &p.faces {
            let Some(face) = frozen.faces.get(f as usize) else {
                return bad(format!("primitive {k} names no frozen face {f}"));
            };
            let id = frozen
                .primitives
                .get(face.source as usize)
                .map_or(u32::MAX, |s| s.material);
            if !seen.insert(id) || id == u32::MAX {
                continue;
            }
            let Some(source) = sources.get(id as usize) else {
                return bad(format!("source material {id} is unknown"));
            };
            let Some(material) = material else {
                return bad(format!("primitive {k} drops source material {id}"));
            };
            if literal_factors(material) != literal_factors(source.literal()) {
                return bad(format!(
                    "primitive {k} changes literal factors of source material {id}"
                ));
            }
            if !p.no_raster {
                for slot in source.bound_slot_pointers() {
                    if material.pointer(&slot).is_none() {
                        return bad(format!(
                            "raster primitive {k} detaches {slot} of source material {id}"
                        ));
                    }
                }
            }
        }
    }
    Ok(())
}

/// A feature-ID attribute stream holding `ids` (one per face, three corners each)
/// EXACTLY, in the narrowest type glTF admits for a feature-ID vertex attribute:
/// UNSIGNED_BYTE up to 255, UNSIGNED_SHORT up to 65535, FLOAT up to 2^24 (every integer
/// there is exact). glTF forbids UNSIGNED_INT for application attributes, so an ID above
/// 2^24 has no exact attribute encoding and is refused rather than rounded. Elements are
/// padded to a four-byte stride (vertex-attribute alignment, meshopt streams).
fn feature_id_stream(ids: &[u32]) -> Result<(u32, Vec<u8>), JobError> {
    let max = ids.iter().copied().max().unwrap_or(0);
    let component = match max {
        0..=255 => 5121,
        256..=65535 => 5123,
        _ if max <= 1 << 24 => 5126,
        _ => {
            return Err(JobError::new(
                JobErrorKind::Unsupported,
                format!(
                    "feature ID {max} exceeds 2^24: glTF feature-ID attributes cannot hold it \
                     exactly (UNSIGNED_INT is not allowed for application attributes)"
                ),
            ))
        }
    };
    let mut bytes = Vec::with_capacity(ids.len() * 12);
    for &id in ids {
        let element: [u8; 4] = match component {
            5121 => [id as u8, 0, 0, 0],
            5123 => {
                let [a, b] = (id as u16).to_le_bytes();
                [a, b, 0, 0]
            }
            _ => (id as f32).to_le_bytes(),
        };
        for _ in 0..3 {
            bytes.extend_from_slice(&element);
        }
    }
    Ok((component, bytes))
}

/// Read the assembled member back from its bytes and compare every requested value.
fn verify(
    frozen: &FrozenNode,
    prepared: &PreparedNode,
    glb: &[u8],
) -> Result<Vec<FaceIdentity>, JobError> {
    let bad = |what: &str| state(format!("assembled member reads back a different {what}"));
    let decoded = codec::decode(glb)?;
    let back = codec::readback(glb)?;
    let order: Vec<u32> = prepared
        .primitives
        .iter()
        .flat_map(|p| p.faces.iter().copied())
        .collect();
    if decoded.faces.len() != order.len() || decoded.primitives.len() != prepared.primitives.len() {
        return Err(bad("face set"));
    }
    let mut identities = Vec::with_capacity(order.len());
    for (d, &f) in decoded.faces.iter().zip(&order) {
        let face = &frozen.faces[f as usize];
        let g = &frozen.gltf[face.gltf as usize];
        let p = &decoded.primitives[d.primitive as usize];
        if p.translation != g.translation || p.scale != g.scale || p.quantized != g.quantized {
            return Err(bad("POSITION transform"));
        }
        for c in 0..3 {
            if d.raw[c] != face.corners[c].raw
                || d.p[c].map(f32::to_bits) != face.corners[c].decoded
            {
                return Err(bad("POSITION corner"));
            }
        }
        let features: Vec<(u32, Option<u32>)> = d
            .features
            .iter()
            .map(|(id, t)| (*id, t.map(|t| t as u32)))
            .collect();
        if features != face.features || !d.uniform_features {
            return Err(bad("feature ID or table link"));
        }
        // Source primitive, material and source triangle as the member itself encodes them:
        // the set-0 `source_primitive` row and the set-1 `proxy_source` row.
        let row = |label: &str| -> Result<(&codec::Table, usize), JobError> {
            let (row, table) = d
                .feature(&decoded.tables, label)
                .ok_or_else(|| bad(label))?;
            Ok((&decoded.tables[table], row as usize))
        };
        let column = |(t, r): (&codec::Table, usize), name: &str| -> Result<u64, JobError> {
            t.columns
                .get(name)
                .and_then(|c| c.get(r))
                .copied()
                .ok_or_else(|| bad(name))
        };
        let primitive = row("source_primitive")?;
        let source = (
            column(primitive, "node_index")? as u32,
            column(primitive, "mesh_index")? as u32,
            column(primitive, "primitive_index")? as u32,
            column(primitive, "expanded_ordinal")?,
        );
        let material = column(primitive, "material_index")? as u32;
        let triangle_row = row("proxy_source")?;
        let triangle = if column(triangle_row, "fill")? != 0 {
            u64::MAX
        } else {
            column(triangle_row, "triangle_index")?
        };
        let origin = Origin {
            node: source.0,
            mesh: source.1,
            primitive: source.2,
            triangle,
        };
        let s = &frozen.primitives[face.source as usize];
        if source != (s.node, s.mesh, s.primitive, s.expanded_ordinal)
            || material != s.material
            || origin != face.origin
        {
            return Err(bad("source primitive, material or origin"));
        }
        identities.push(FaceIdentity {
            face: f,
            raw: d.raw,
            decoded: d.p.map(|q| q.map(f32::to_bits)),
            origin,
            source,
            material,
            features,
        });
    }
    // Feature-ID declarations (labels, counts, tables, texture sets).
    for (p, d) in prepared.primitives.iter().zip(&decoded.primitives) {
        let mut expected = frozen.gltf[p.gltf as usize]
            .feature_ids
            .as_array()
            .cloned()
            .unwrap_or_default();
        expected.extend(p.extra_feature_ids.iter().cloned());
        if d.feature_ids != Value::Array(expected) || d.material != p.material {
            return Err(bad("feature-ID declaration or material binding"));
        }
    }
    // Every table and column.
    if decoded.tables.len() != frozen.tables.len() {
        return Err(bad("table set"));
    }
    for (t, f) in decoded.tables.iter().zip(&frozen.tables) {
        if t.class != f.class || t.count as u64 != f.count || t.columns.len() != f.columns.len() {
            return Err(bad("table"));
        }
        for (name, component, values) in &f.columns {
            if t.columns.get(name) != Some(values) || t.components.get(name) != Some(component) {
                return Err(bad("table column"));
            }
        }
    }
    // Corner attributes exactly as requested.
    for (p, attrs) in prepared.primitives.iter().zip(&back.attributes) {
        let flat = |v: Option<Vec<f32>>| v.unwrap_or_default();
        let want = |name: &str, values: Option<Vec<f32>>| -> Result<(), JobError> {
            let got = attrs.get(name).cloned();
            match (values, got) {
                (None, None) => Ok(()),
                (Some(w), Some(g))
                    if w.iter()
                        .map(|x| x.to_bits())
                        .eq(g.iter().map(|x| x.to_bits())) =>
                {
                    Ok(())
                }
                _ => Err(bad(name)),
            }
        };
        want(
            "NORMAL",
            p.normals
                .as_ref()
                .map(|v| v.iter().flatten().copied().collect()),
        )?;
        want(
            "TANGENT",
            p.tangents
                .as_ref()
                .map(|v| v.iter().flatten().copied().collect()),
        )?;
        want(
            "COLOR_0",
            p.colors
                .as_ref()
                .map(|v| v.iter().flatten().copied().collect()),
        )?;
        for (set, uv) in p.uvs.iter().enumerate() {
            want(
                &format!("TEXCOORD_{set}"),
                Some(flat(Some(uv.iter().flatten().copied().collect()))),
            )?;
        }
        let expected_names = 1
            + usize::from(p.normals.is_some())
            + usize::from(p.tangents.is_some())
            + usize::from(p.colors.is_some())
            + p.uvs.len();
        if attrs.len() + 1 != expected_names {
            return Err(bad("attribute set"));
        }
    }
    // Images, textures, samplers and materials exactly as requested.
    if back.images.len() != prepared.images.len()
        || back
            .images
            .iter()
            .zip(&prepared.images)
            .any(|(b, p)| *b != p.bytes)
    {
        return Err(bad("image"));
    }
    for (field, list) in [
        ("materials", &prepared.materials),
        ("textures", &prepared.textures),
        ("samplers", &prepared.samplers),
    ] {
        let got = back
            .doc
            .get(field)
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        if got != *list {
            return Err(bad(field));
        }
    }
    Ok(identities)
}

/// Re-export for bake: a level type for building [`PreencodePage`]-shaped packets.
pub(in crate::mesh_large) type Level = PreencodeLevel;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn feature_id_streams_are_exact_and_narrowest() {
        let decode = |component: u32, bytes: &[u8]| -> Vec<u32> {
            bytes
                .as_chunks::<4>()
                .0
                .iter()
                .map(|e| match component {
                    5121 => u32::from(e[0]),
                    5123 => u32::from(u16::from_le_bytes([e[0], e[1]])),
                    _ => f32::from_le_bytes(*e) as u32,
                })
                .collect()
        };
        for (ids, component) in [
            (vec![0u32, 7, 255], 5121),
            (vec![0, 256], 5123),
            (vec![65_535], 5123),
            (vec![65_536, 3], 5126),
            (vec![16_777_215], 5126),
            (vec![16_777_216, 0], 5126),
        ] {
            let (c, bytes) = feature_id_stream(&ids).unwrap();
            assert_eq!(c, component, "{ids:?}");
            let expected: Vec<u32> = ids.iter().flat_map(|&i| [i; 3]).collect();
            assert_eq!(decode(c, &bytes), expected);
        }
        for id in [16_777_217u32, u32::MAX] {
            assert_eq!(
                feature_id_stream(&[id]).unwrap_err().kind(),
                JobErrorKind::Unsupported
            );
        }
    }
    use crate::{
        mesh_large::{
            delivery::{mesh_leaves_to_3tz, MeshTilesetRequest},
            leaves::producer::{mesh_to_leaf_payloads, LeafOutputRequest, LeafProfile},
        },
        validate::{inspect, ValidationRequest},
        RunControl,
    };
    use std::{collections::BTreeMap, num::NonZeroUsize, sync::Arc, sync::Mutex};

    /// A textured wavy grid over two primitives (shared seam), enough for several leaves.
    fn fixture(dir: &Path, g: usize) -> PathBuf {
        fs::create_dir_all(dir).unwrap();
        let mut bin = Vec::new();
        let (mut views, mut accessors, mut meshes) = (vec![], vec![], vec![]);
        let half = g / 2;
        for (b, (i0, i1)) in [(0, half), (half, g)].into_iter().enumerate() {
            let w = g + 1;
            let mut pos = Vec::new();
            let mut uv = Vec::new();
            for i in i0..=i1 {
                for j in 0..=g {
                    let (x, z) = (i as f32 * 0.5, j as f32 * 0.5);
                    pos.push([x, 3. * (x / 3.).sin() * (z / 4.).cos(), z]);
                    uv.push([i as f32 / g as f32, j as f32 / g as f32]);
                }
            }
            let mut idx = Vec::new();
            for i in i0..i1 {
                for j in 0..g {
                    // Single-quad holes, small enough to be filled at coarse depths.
                    if i % 17 == 5 && j % 13 == 7 {
                        continue;
                    }
                    let v = |a: usize, c: usize| ((a - i0) * w + c) as u32;
                    idx.extend([
                        v(i, j),
                        v(i, j + 1),
                        v(i + 1, j),
                        v(i + 1, j),
                        v(i, j + 1),
                        v(i + 1, j + 1),
                    ]);
                }
            }
            let mut push = |bytes: Vec<u8>| {
                let start = bin.len();
                bin.extend(bytes);
                views.push(json!({"buffer":0,"byteOffset":start,"byteLength":bin.len()-start}));
                views.len() - 1
            };
            let (lo, hi) = pos
                .iter()
                .fold(([f32::MAX; 3], [f32::MIN; 3]), |(l, h), p| {
                    (
                        std::array::from_fn(|a| l[a].min(p[a])),
                        std::array::from_fn(|a| h[a].max(p[a])),
                    )
                });
            let p = push(pos.iter().flatten().flat_map(|v| v.to_le_bytes()).collect());
            accessors.push(json!({"bufferView":p,"componentType":5126,"count":pos.len(),"type":"VEC3","min":lo,"max":hi}));
            let t = push(uv.iter().flatten().flat_map(|v| v.to_le_bytes()).collect());
            accessors
                .push(json!({"bufferView":t,"componentType":5126,"count":uv.len(),"type":"VEC2"}));
            let i = push(idx.iter().flat_map(|v| v.to_le_bytes()).collect());
            accessors.push(
                json!({"bufferView":i,"componentType":5125,"count":idx.len(),"type":"SCALAR"}),
            );
            meshes.push(json!({"primitives":[{"attributes":{"POSITION":accessors.len()-3,"TEXCOORD_0":accessors.len()-2},"indices":accessors.len()-1,"material":b}]}));
        }
        fs::write(dir.join("scene.bin"), &bin).unwrap();
        image::RgbaImage::from_pixel(64, 64, image::Rgba([120, 160, 90, 255]))
            .save(dir.join("texture.png"))
            .unwrap();
        let doc = json!({"asset":{"version":"2.0"},"scene":0,"scenes":[{"nodes":[0,1]}],"nodes":[{"mesh":0},{"mesh":1}],
            "meshes":meshes,"buffers":[{"uri":"scene.bin","byteLength":bin.len()}],"bufferViews":views,"accessors":accessors,
            "images":[{"uri":"texture.png"}],"textures":[{"source":0}],
            "materials":[{"pbrMetallicRoughness":{"baseColorTexture":{"index":0},"metallicFactor":0}},
                         {"pbrMetallicRoughness":{"baseColorTexture":{"index":0},"baseColorFactor":[0.8,0.9,1,1],"metallicFactor":0}}]});
        let path = dir.join("scene.gltf");
        fs::write(&path, serde_json::to_vec(&doc).unwrap()).unwrap();
        path
    }

    fn png(rgba: [u8; 4]) -> Vec<u8> {
        let mut out = std::io::Cursor::new(Vec::new());
        image::DynamicImage::ImageRgba8(image::RgbaImage::from_pixel(2, 2, image::Rgba(rgba)))
            .write_to(&mut out, image::ImageFormat::Png)
            .unwrap();
        out.into_inner()
    }

    /// The prepared node a minimal baker would hand over: every frozen glTF primitive as one
    /// output primitive, a flat 2x2 page, UV and normal per corner.
    /// A flat textured member: per frozen glTF primitive, its sources' material with every
    /// bound slot pointing at one page (literal factors kept, normal scale published as 1).
    fn flat(frozen: &FrozenNode, sources: &[Material]) -> PreparedNode {
        let mut materials = Vec::new();
        let primitives = frozen
            .gltf
            .iter()
            .enumerate()
            .map(|(i, g)| {
                let n = (g.faces.end - g.faces.start) as usize * 3;
                let ids: Vec<u32> = g
                    .sources
                    .clone()
                    .map(|s| frozen.primitives[s as usize].material)
                    .filter(|&m| m != u32::MAX)
                    .collect();
                let material = ids.first().map(|&first| {
                    let mut value = sources[first as usize].literal().clone();
                    for &id in &ids {
                        for slot in sources[id as usize].bound_slot_pointers() {
                            let (parent, last) = slot.rsplit_once('/').unwrap();
                            let parent = if parent.is_empty() {
                                &mut value
                            } else {
                                value.pointer_mut(parent).unwrap()
                            };
                            parent[last] = json!({"index": 0});
                        }
                    }
                    materials.push(value);
                    (materials.len() - 1) as u32
                });
                PreparedPrimitive {
                    gltf: i as u32,
                    faces: g.faces.clone().collect(),
                    material,
                    uvs: vec![vec![[0.25, 0.75]; n]],
                    normals: Some(vec![[0., 1., 0.]; n]),
                    ..Default::default()
                }
            })
            .collect();
        PreparedNode {
            primitives,
            materials,
            textures: vec![json!({"source":0,"sampler":0})],
            samplers: vec![json!({"magFilter":9729,"minFilter":9729,"wrapS":33071,"wrapT":33071})],
            images: vec![PreparedImage {
                bytes: png([200, 100, 50, 255]),
                mime: "image/png",
            }],
        }
    }

    /// Independent packet check against the untextured member it was frozen from and the
    /// captured source: published face order, raw and decoded corners, feature IDs and
    /// tables, source primitive and origin from the member's own tables, fill marking, and
    /// donor ranges by brute force over the source ranks.
    fn check_packet(pass: &BakePass<'_>, frozen: &FrozenNode) -> Result<(), String> {
        let id = &frozen.identity;
        let path = pass
            .stage
            .join(format!("proxies/f{}-d{}.glb", id.first, id.depth));
        let d = codec::decode(&fs::read(path).map_err(|e| e.to_string())?)
            .map_err(|e| e.to_string())?;
        if d.faces.len() != frozen.faces.len() {
            return Err("face count".into());
        }
        let column = |table: usize, name: &str, row: u32| -> Result<u64, String> {
            d.tables[table]
                .columns
                .get(name)
                .and_then(|c| c.get(row as usize))
                .copied()
                .ok_or_else(|| format!("column {name}"))
        };
        for (k, (g, f)) in d.faces.iter().zip(&frozen.faces).enumerate() {
            let features: Vec<(u32, Option<u32>)> = g
                .features
                .iter()
                .map(|(v, t)| (*v, t.map(|t| t as u32)))
                .collect();
            if g.raw != f.corners.map(|c| c.raw)
                || g.p.map(|q| q.map(f32::to_bits)) != f.corners.map(|c| c.decoded)
                || features != f.features
                || g.primitive != f.gltf
            {
                return Err(format!(
                    "face {k} differs from the published member (order)"
                ));
            }
            let (row0, t0) = g.feature(&d.tables, "source_primitive").ok_or("set 0")?;
            let (row1, t1) = g.feature(&d.tables, "proxy_source").ok_or("set 1")?;
            let p = &frozen.primitives[f.source as usize];
            let source = (
                column(t0, "node_index", row0)? as u32,
                column(t0, "mesh_index", row0)? as u32,
                column(t0, "primitive_index", row0)? as u32,
            );
            let fill = column(t1, "fill", row1)? != 0;
            let triangle = if fill {
                u64::MAX
            } else {
                column(t1, "triangle_index", row1)?
            };
            if source != (p.node, p.mesh, p.primitive)
                || (f.origin.node, f.origin.mesh, f.origin.primitive) != source
                || f.origin.triangle != triangle
                || f.fill.is_some() != fill
            {
                return Err(format!(
                    "face {k} provenance differs from the member's tables"
                ));
            }
            if let Some(i) = f.fill {
                let r = frozen.fills.get(i as usize).ok_or("fill record")?;
                if r.source != f.source
                    || r.bound
                        .partial_cmp(&frozen.geometric_error)
                        .is_none_or(|o| o.is_gt())
                {
                    return Err(format!("face {k} fill record"));
                }
            }
        }
        // Donor ranges: the maximal runs of each primitive's source ranks in the tile.
        for p in &frozen.primitives {
            let mut runs: Vec<std::ops::Range<u64>> = Vec::new();
            let triangles = pass
                .captured
                .triangles
                .range(id.first, id.end, 1 << 16)
                .map_err(|e| e.to_string())?;
            for (rank, t) in (id.first..).zip(triangles) {
                let t = t.map_err(|e| e.to_string())?.triangle.source;
                if (t.node, t.mesh, t.primitive) == (p.node, p.mesh, p.primitive) {
                    match runs.last_mut() {
                        Some(r) if r.end == rank => r.end = rank + 1,
                        _ => runs.push(rank..rank + 1),
                    }
                }
            }
            if runs != p.donor_ranges {
                return Err("donor ranges differ from the source ranks".into());
            }
        }
        Ok(())
    }

    /// A test baker: postorder, reads every packet and child appearance, assembles and
    /// commits a flat textured member, records what it saw.
    #[derive(Debug, Default)]
    struct FlatBaker {
        seen: Mutex<Vec<String>>,
        fills: Mutex<usize>,
        last: Mutex<Option<Vec<u8>>>,
    }
    impl ProxyBaker for FlatBaker {
        fn bake(&self, pass: &BakePass<'_>) -> Result<(), JobError> {
            let mut done: Vec<(u64, u32)> = Vec::new();
            for node in pass.nodes()? {
                let (first, depth) = node?;
                let frozen = pass.frozen(first, depth, 1 << 30)?;
                assert_eq!(
                    (frozen.identity.first, frozen.identity.depth),
                    (first, depth)
                );
                // Children come first (postorder) and their appearance is retained.
                let mut leaf_pages = 0;
                for c in &frozen.children {
                    match c.leaf {
                        Some(leaf) => {
                            // A lease below the head and pages is refused before allocation.
                            assert_eq!(
                                pass.preencode
                                    .read_leaf(leaf.index, 16, pass.check)
                                    .unwrap_err()
                                    .kind(),
                                JobErrorKind::ResourceLimit
                            );
                            let pre = pass.preencode.read_leaf(leaf.index, 1 << 30, pass.check)?;
                            assert!(!pre.pages.is_empty(), "leaf pre-encode pages retained");
                            assert!(pre.pages.iter().all(
                                |p| p.levels[0].rgba.len() == (p.width * p.height * 4) as usize
                            ));
                            assert!(pre
                                .primitives
                                .iter()
                                .all(|p| p.uvs.iter().all(|u| u.len() == 3 * p.triangles.len())));
                            leaf_pages += pre.pages.len();
                        }
                        None => {
                            assert!(
                                done.contains(&(c.first, c.depth)),
                                "child proxy baked before its parent"
                            );
                            assert_eq!(pass.preencode.proxy_size(c.first, c.depth)?, 5);
                            assert!(pass
                                .preencode
                                .read_proxy(c.first, c.depth, 4, pass.check)
                                .is_err());
                            assert_eq!(
                                pass.preencode
                                    .read_proxy(c.first, c.depth, 1 << 30, pass.check)?,
                                b"pages"
                            );
                            pass.preencode.release_proxy(c.first, c.depth)?;
                        }
                    }
                }
                check_packet(pass, &frozen).unwrap();
                *self.fills.lock().unwrap() += frozen.fills.len();
                let prepared = flat(&frozen, &pass.captured.materials);
                // The physical budget path: a tiny budget reports the actual size.
                match assemble(&frozen, &prepared, &pass.captured.materials, 64, u64::MAX)? {
                    Assembly::PhysicalBudget { actual_bytes } => assert!(actual_bytes > 64),
                    Assembly::Fit { .. } => panic!("a 64-byte member cannot fit"),
                }
                let fit = pass.assemble(&frozen, &prepared, 1 << 30)?;
                let Assembly::Fit { decoded, .. } = &fit else {
                    panic!("the flat member fits");
                };
                assert_eq!(decoded.len(), frozen.faces.len());
                for d in decoded.iter() {
                    let f = &frozen.faces[d.face as usize];
                    assert_eq!(d.origin, f.origin);
                    assert_eq!(d.decoded, f.corners.map(|c| c.decoded));
                }
                // Commit consumes only a verified Fit for this very tile whose bytes are
                // unchanged since verification; every refusal leaves the member alone.
                let fresh = || pass.assemble(&frozen, &prepared, 1 << 30);
                let budget = Assembly::PhysicalBudget { actual_bytes: 1 };
                assert!(pass.commit(&frozen, budget).is_err());
                let mut other = frozen.clone();
                other.identity.first += 1;
                assert!(pass.commit(&other, fresh()?).is_err());
                let mutate = |edit: &dyn Fn(&mut Vec<u8>)| -> Result<Assembly, JobError> {
                    let Assembly::Fit {
                        mut glb,
                        decoded,
                        verified,
                    } = fresh()?
                    else {
                        panic!("fits")
                    };
                    edit(&mut glb);
                    Ok(Assembly::Fit {
                        glb,
                        decoded,
                        verified,
                    })
                };
                let previous = self.last.lock().unwrap().clone();
                if let Some(previous) = previous {
                    let foreign = mutate(&|glb| *glb = previous.clone())?;
                    assert!(pass.commit(&frozen, foreign).is_err(), "foreign bytes");
                }
                // Same-size mutations of the image, a UV and a material factor.
                let image = mutate(&|glb| {
                    let at = glb.windows(4).position(|w| w == b"\x89PNG").unwrap();
                    glb[at + 40] ^= 1;
                })?;
                assert!(pass.commit(&frozen, image).is_err(), "image mutation");
                let uv = mutate(&|glb| {
                    let jl = u32::from_le_bytes(glb[12..16].try_into().unwrap()) as usize;
                    let doc: Value = serde_json::from_slice(&glb[20..20 + jl]).unwrap();
                    let accessor = doc["meshes"][0]["primitives"][0]["attributes"]["TEXCOORD_0"]
                        .as_u64()
                        .unwrap() as usize;
                    let view = &doc["bufferViews"]
                        [doc["accessors"][accessor]["bufferView"].as_u64().unwrap() as usize];
                    let offset = view["extensions"]["EXT_meshopt_compression"]["byteOffset"]
                        .as_u64()
                        .unwrap_or(0) as usize;
                    glb[28 + jl + offset + 1] ^= 1;
                })?;
                assert!(pass.commit(&frozen, uv).is_err(), "UV mutation");
                let material = mutate(&|glb| {
                    let needle = b"\"metallicFactor\":0";
                    let at = glb.windows(needle.len()).position(|w| w == needle).unwrap();
                    glb[at + needle.len() - 1] = b'1';
                })?;
                assert!(pass.commit(&frozen, material).is_err(), "material mutation");
                pass.read_certified_proxy(first, depth, 1 << 30)?;
                let fit = fresh()?;
                if let Assembly::Fit { glb, .. } = &fit {
                    *self.last.lock().unwrap() = Some(glb.clone());
                }
                pass.commit(&frozen, fit)?;
                // The member is now the baked one: the certified reader refuses it.
                assert!(pass.read_certified_proxy(first, depth, 1 << 30).is_err());
                pass.preencode.write_proxy(first, depth, b"pages")?;
                pass.release(first, depth)?;
                self.seen
                    .lock()
                    .unwrap()
                    .push(format!("f{first}-d{depth}:{leaf_pages}"));
                done.push((first, depth));
            }
            Ok(())
        }
    }

    /// Raw and decoded corners per face, in published order (order-sensitive).
    type Corners = [[u32; 3]; 3];

    fn faces_of(glb: &[u8]) -> Vec<(Corners, Corners)> {
        codec::decode(glb)
            .unwrap()
            .faces
            .iter()
            .map(|f| (f.raw, f.p.map(|p| p.map(f32::to_bits))))
            .collect()
    }

    /// The handoff end to end on real H8 + H17 output: packets are complete and consistent,
    /// leaf pre-encode pages are retained, the baked members carry the certified geometry
    /// unchanged with every table, and the archive validates with decoded containment.
    #[test]
    fn bake_pass_hands_off_frozen_tiles_and_commits_complete_members() {
        let work = tempfile::tempdir().unwrap();
        let input = fixture(&work.path().join("src"), 64);
        let run = |name: &str, baker: Option<Arc<FlatBaker>>| {
            let out = work.path().join(name);
            let scratch = work.path().join(format!("{name}-scratch"));
            fs::create_dir_all(&scratch).unwrap();
            let mut request = LeafOutputRequest::new(&input, &out, LeafProfile::Balanced)
                .with_jobs(NonZeroUsize::new(2).unwrap())
                .with_scratch_dir(&scratch)
                .with_lod_target(256)
                .with_leaf_triangles(1024);
            if let Some(b) = baker {
                request = request.with_baker(b);
            }
            mesh_to_leaf_payloads(request, &RunControl::default()).unwrap();
            assert!(
                fs::read_dir(&scratch).unwrap().next().is_none(),
                "bake scratch removed"
            );
            out
        };
        let plain = run("plain", None);
        let baker = Arc::new(FlatBaker::default());
        let baked = run("baked", Some(baker.clone()));
        let seen = baker.seen.lock().unwrap().clone();
        assert!(
            *baker.fills.lock().unwrap() > 0,
            "the fixture's holes give fill faces in some packet: {}",
            String::from_utf8_lossy(&fs::read(baked.join("manifest.json")).unwrap())
        );
        // The manifest reports the published (baked) bytes and keeps the untextured figure.
        let manifest = |dir: &Path| -> Value {
            serde_json::from_slice(&fs::read(dir.join("manifest.json")).unwrap()).unwrap()
        };
        let (plain_lod, baked_lod) = (
            manifest(&plain)["lod"].clone(),
            manifest(&baked)["lod"].clone(),
        );
        assert_eq!(baked_lod["untexturedProxyBytes"], plain_lod["proxyBytes"]);
        assert!(plain_lod.get("untexturedProxyBytes").is_none());
        let rows: Vec<Value> = fs::read_to_string(baked.join("proxies.jsonl"))
            .unwrap()
            .lines()
            .map(|l| serde_json::from_str(l).unwrap())
            .collect();
        assert_eq!(seen.len(), rows.len());
        assert!(rows.len() >= 3);
        assert!(
            seen.iter().any(|s| !s.ends_with(":0")),
            "some tile read leaf pages"
        );
        let plain_rows: Vec<Value> = fs::read_to_string(plain.join("proxies.jsonl"))
            .unwrap()
            .lines()
            .map(|l| serde_json::from_str(l).unwrap())
            .collect();
        assert_eq!(
            baked_lod["proxyBytes"].as_u64().unwrap(),
            rows.iter()
                .map(|r| r["bytes"].as_u64().unwrap())
                .sum::<u64>()
        );
        for (b, p) in rows.iter().zip(&plain_rows) {
            assert_eq!(
                b["geometricError"], p["geometricError"],
                "geometric error unchanged"
            );
            let bytes = fs::read(baked.join(b["uri"].as_str().unwrap())).unwrap();
            assert_eq!(b["bytes"].as_u64().unwrap(), bytes.len() as u64);
            let unbaked = fs::read(plain.join(p["uri"].as_str().unwrap())).unwrap();
            assert_eq!(
                faces_of(&bytes),
                faces_of(&unbaked),
                "certified geometry unchanged"
            );
            let back = codec::readback(&bytes).unwrap();
            assert_eq!(back.images.len(), 1, "baked member carries its page");
            let tables = codec::decode(&bytes).unwrap().tables;
            assert_eq!(tables.len(), codec::decode(&unbaked).unwrap().tables.len());
        }
        let archive = work.path().join("baked.3tz");
        mesh_leaves_to_3tz(
            MeshTilesetRequest::new(&baked, &archive),
            &RunControl::default(),
        )
        .unwrap();
        let checked = inspect(ValidationRequest::new(&archive)).unwrap();
        assert!(
            checked
                .checks
                .iter()
                .any(|c| c == "decodedContentContainment"),
            "{:?}",
            checked.not_inspected
        );
    }

    /// Adapter controls: a partition gap and every kind of readback mismatch are refused.
    #[test]
    fn adapter_refuses_incomplete_or_mismatched_members() {
        let work = tempfile::tempdir().unwrap();
        let input = fixture(&work.path().join("src"), 64);
        #[derive(Debug, Default)]
        struct Probe {
            checked: Mutex<bool>,
        }
        impl ProxyBaker for Probe {
            fn bake(&self, pass: &BakePass<'_>) -> Result<(), JobError> {
                let (first, depth) = pass.nodes()?.next().unwrap()?;
                let frozen = pass.frozen(first, depth, 1 << 30)?;
                // Packet round trip.
                let check = || Ok(());
                let mut lease = Lease::new(1 << 30, "test");
                assert_eq!(
                    FrozenNode::decode(&frozen.encode()?, &mut lease, &check)?,
                    frozen
                );
                // A lease below the packet's size is refused before anything is read.
                let size = pass.frozen_size(first, depth)?;
                assert_eq!(
                    pass.frozen(first, depth, size - 1).unwrap_err().kind(),
                    JobErrorKind::ResourceLimit
                );
                // The certified member reads back by its frozen hash, within its lease.
                let certified = pass.read_certified_proxy(first, depth, 1 << 30)?;
                assert_eq!(certified.len() as u64, frozen.glb_bytes);
                assert_eq!(
                    <[u8; 32]>::from(Sha256::digest(&certified)),
                    frozen.glb_sha256
                );
                assert_eq!(
                    pass.read_certified_proxy(first, depth, frozen.glb_bytes - 1)
                        .unwrap_err()
                        .kind(),
                    JobErrorKind::ResourceLimit
                );
                // Sink corruption controls: the independent packet check refuses each.
                let filled = pass
                    .nodes()?
                    .map(|n| n.and_then(|(f, d)| pass.frozen(f, d, 1 << 30)))
                    .collect::<Result<Vec<_>, _>>()?
                    .into_iter()
                    .find(|n| !n.fills.is_empty())
                    .expect("a packet with fills");
                check_packet(pass, &frozen).unwrap();
                check_packet(pass, &filled).unwrap();
                let plain = frozen.faces.iter().position(|f| f.fill.is_none()).unwrap();
                let mut corrupt = Vec::new();
                let mut c = frozen.clone();
                c.faces.swap(0, 1);
                corrupt.push(("face order", c));
                let mut c = frozen.clone();
                c.faces[0].features[1].0 ^= 1;
                corrupt.push(("feature", c));
                let mut c = frozen.clone();
                c.faces[plain].origin.triangle ^= 1;
                corrupt.push(("origin", c));
                let mut c = frozen.clone();
                let r = &mut c.primitives[0].donor_ranges[0];
                if r.end - r.start > 1 {
                    r.end -= 1;
                } else {
                    r.end += 1;
                }
                corrupt.push(("donor ranges", c));
                let mut c = filled.clone();
                let unfilled = c.faces.iter().position(|f| f.fill.is_none()).unwrap();
                c.faces[unfilled].fill = Some(0);
                corrupt.push(("fill marker", c));
                let mut c = filled.clone();
                c.fills[0].bound = c.geometric_error * 2. + 1.;
                corrupt.push(("fill bound", c));
                for (what, c) in &corrupt {
                    assert!(
                        check_packet(pass, c).is_err(),
                        "{what} corruption not caught"
                    );
                }
                for f in &frozen.faces {
                    assert!(!frozen.donor_ranges(f).is_empty());
                }
                let good = flat(&frozen, &pass.captured.materials);
                let mut missing = good.clone();
                missing.primitives[0].faces.pop();
                let corners = missing.primitives[0].faces.len() * 3;
                missing.primitives[0].uvs[0].truncate(corners);
                missing.primitives[0]
                    .normals
                    .as_mut()
                    .unwrap()
                    .truncate(corners);
                assert_eq!(
                    assemble(
                        &frozen,
                        &missing,
                        &pass.captured.materials,
                        u64::MAX,
                        u64::MAX
                    )
                    .unwrap_err()
                    .kind(),
                    JobErrorKind::InvalidState
                );
                // Owner checks: transform domain, materials, remaps and no-raster rules.
                let materials = &pass.captured.materials;
                let refused = |f: &FrozenNode, p: &PreparedNode| {
                    assemble(f, p, materials, u64::MAX, u64::MAX).is_err()
                };
                let mut f = frozen.clone();
                f.gltf[0].scale[1] = 2f32.to_bits();
                assert!(refused(&f, &good), "non-uniform scale");
                let mut f = frozen.clone();
                f.gltf[0].scale = [(-1f32).to_bits(); 3];
                assert!(refused(&f, &good), "reflecting scale");
                let mut f = frozen.clone();
                f.gltf[0].translation[0] = f32::NAN.to_bits();
                assert!(refused(&f, &good), "non-finite translation");
                let mut p = good.clone();
                p.materials[0]["normalTexture"] = json!({"index": 0, "scale": 0.5});
                assert!(refused(&frozen, &p), "unabsorbed normal scale");
                let mut p = good.clone();
                p.materials[0]["normalTexture"] = json!({"index": 0, "scale": 1.0});
                assert!(!refused(&frozen, &p), "normal scale 1 is accepted");
                let mut p = good.clone();
                p.materials[0]["pbrMetallicRoughness"]["roughnessFactor"] = json!(0.25);
                assert!(refused(&frozen, &p), "changed literal factor");
                let mut p = good.clone();
                p.textures[0]["source"] = json!(9);
                assert!(refused(&frozen, &p), "texture names no image");
                let mut p = good.clone();
                p.materials[0]["pbrMetallicRoughness"]["baseColorTexture"]["texCoord"] = json!(1);
                assert!(refused(&frozen, &p), "binding reads a missing UV set");
                let mut p = good.clone();
                p.materials[0]["pbrMetallicRoughness"]
                    .as_object_mut()
                    .unwrap()
                    .remove("baseColorTexture");
                assert!(
                    refused(&frozen, &p),
                    "raster primitive detaches a source slot"
                );
                let mut p = good.clone();
                p.primitives[0].no_raster = true;
                assert!(refused(&frozen, &p), "no-raster primitive keeps a binding");
                // A no-raster primitive with its bindings detached and no UVs is accepted.
                let mut p = good.clone();
                let mut detached = p.materials[p.primitives[0].material.unwrap() as usize].clone();
                for slot in Material::slot_pointers() {
                    let (parent, last) = slot.rsplit_once('/').unwrap();
                    let parent = if parent.is_empty() {
                        Some(&mut detached)
                    } else {
                        detached.pointer_mut(parent)
                    };
                    if let Some(object) = parent.and_then(Value::as_object_mut) {
                        object.remove(last);
                    }
                }
                p.materials.push(detached);
                p.primitives[0].material = Some((p.materials.len() - 1) as u32);
                p.primitives[0].no_raster = true;
                p.primitives[0].uvs.clear();
                assert!(!refused(&frozen, &p), "detached no-raster primitive");
                // The memory lease is checked before anything is built, on both paths.
                let needed = assembly_bytes(&frozen, &good);
                for tile in [u64::MAX, 64] {
                    assert_eq!(
                        assemble(&frozen, &good, &pass.captured.materials, tile, needed - 1)
                            .unwrap_err()
                            .kind(),
                        JobErrorKind::ResourceLimit
                    );
                }
                assert!(assemble(&frozen, &good, &pass.captured.materials, 64, needed).is_ok());
                let Assembly::Fit { glb, .. } =
                    assemble(&frozen, &good, &pass.captured.materials, u64::MAX, u64::MAX)?
                else {
                    panic!()
                };
                assert!(
                    needed >= 2 * glb.len() as u64,
                    "the bound covers the member"
                );
                // The readback catches a writer that ignored a requested value.
                let mut uv = good.clone();
                uv.primitives[0].uvs[0][0] = [0.5, 0.5];
                let mut image = good.clone();
                image.images[0].bytes = png([1, 2, 3, 255]);
                let mut material = good.clone();
                material.materials[0]["pbrMetallicRoughness"]["metallicFactor"] = json!(0.5);
                let mut extra = good.clone();
                extra.primitives[0].colors =
                    Some(vec![[1.; 4]; good.primitives[0].faces.len() * 3]);
                for wrong in [uv, image, material, extra] {
                    assert_eq!(
                        verify(&frozen, &wrong, &glb).unwrap_err().kind(),
                        JobErrorKind::InvalidState
                    );
                }
                // ...and a member whose geometry or tables differ from the packet.
                let mut moved = frozen.clone();
                moved.faces[0].corners[0].raw[0] ^= 1;
                assert!(verify(&moved, &good, &glb).is_err());
                let mut table = frozen.clone();
                table.tables[0].columns[0].2[0] ^= 1;
                assert!(verify(&table, &good, &glb).is_err());
                // Feature IDs are exact integers: values past f32's integer range up to
                // 2^24 read back unchanged, and anything above 2^24 is refused, not rounded.
                // (A third, table-less attribute set carries the probed value.)
                let with_id = |id: u32| {
                    let mut high = frozen.clone();
                    for g in &mut high.gltf {
                        g.feature_ids.as_array_mut().unwrap().push(
                            json!({"featureCount": 1u64 << 25, "attribute": 7, "label": "high"}),
                        );
                    }
                    for (k, f) in high.faces.iter_mut().enumerate() {
                        f.features.push((if k == 0 { id } else { 1 }, None));
                    }
                    high
                };
                for id in [255u32, 256, 65_535, 65_536, 16_777_215, 16_777_216] {
                    let high = with_id(id);
                    let Assembly::Fit { decoded, .. } =
                        assemble(&high, &good, &pass.captured.materials, u64::MAX, u64::MAX)?
                    else {
                        panic!("fits")
                    };
                    let face = decoded.iter().find(|d| d.face == 0).unwrap();
                    assert_eq!(face.features[2].0, id);
                }
                let high = with_id(16_777_217);
                assert_eq!(
                    assemble(&high, &good, &pass.captured.materials, u64::MAX, u64::MAX)
                        .unwrap_err()
                        .kind(),
                    JobErrorKind::Unsupported
                );
                // One mutant per remaining verify branch.
                let mut frozen_mutants: Vec<(&str, FrozenNode)> = Vec::new();
                let mut m = frozen.clone();
                m.faces[0].features[1].0 ^= 1;
                frozen_mutants.push(("feature value", m));
                let mut m = frozen.clone();
                m.faces[0].features[0].1 = Some(99);
                frozen_mutants.push(("table link", m));
                let mut m = frozen.clone();
                if m.gltf[0].quantized {
                    m.gltf[0].translation[0] ^= 1;
                } else {
                    m.gltf[0].scale[0] ^= 1;
                }
                frozen_mutants.push(("TRS", m));
                let mut m = frozen.clone();
                let component = &mut m.tables[0].columns[0].1;
                *component = if component == "UINT64" {
                    "UINT32"
                } else {
                    "UINT64"
                }
                .into();
                frozen_mutants.push(("component type", m));
                let mut m = frozen.clone();
                m.tables[0].columns.last_mut().unwrap().2[0] ^= 1;
                frozen_mutants.push(("last column", m));
                let mut m = frozen.clone();
                m.primitives[0].material ^= 1;
                frozen_mutants.push(("source material", m));
                let mut m = frozen.clone();
                m.faces[plain].origin.triangle ^= 1;
                frozen_mutants.push(("origin", m));
                for (what, m) in &frozen_mutants {
                    assert!(
                        verify(m, &good, &glb).is_err(),
                        "{what} mismatch not caught"
                    );
                }
                let mut prepared_mutants: Vec<(&str, PreparedNode)> = Vec::new();
                let mut m = good.clone();
                m.primitives[0]
                    .extra_feature_ids
                    .push(json!({"featureCount":1,"texture":{"index":0},"label":"extra"}));
                prepared_mutants.push(("feature-ID declaration", m));
                let mut m = good.clone();
                let n = &mut m.primitives[0].normals.as_mut().unwrap()[0][0];
                *n = f32::from_bits(n.to_bits() ^ 1);
                prepared_mutants.push(("NORMAL bits", m));
                let mut m = good.clone();
                m.primitives[0].tangents =
                    Some(vec![[1., 0., 0., 1.]; good.primitives[0].faces.len() * 3]);
                prepared_mutants.push(("TANGENT", m));
                let mut m = good.clone();
                m.primitives[0].material = None;
                prepared_mutants.push(("material binding", m));
                for (what, m) in &prepared_mutants {
                    assert!(
                        verify(&frozen, m, &glb).is_err(),
                        "{what} mismatch not caught"
                    );
                }
                *self.checked.lock().unwrap() = true;
                // Leave every tile untextured.
                for node in pass.nodes()? {
                    let (f, d) = node?;
                    pass.release(f, d)?;
                }
                Ok(())
            }
        }
        let probe = Arc::new(Probe::default());
        let request = |out: &str| {
            LeafOutputRequest::new(&input, work.path().join(out), LeafProfile::Balanced)
                .with_jobs(NonZeroUsize::new(1).unwrap())
                .with_lod_target(256)
                .with_leaf_triangles(1024)
        };
        mesh_to_leaf_payloads(
            request("out").with_baker(probe.clone()),
            &RunControl::default(),
        )
        .unwrap();
        assert!(*probe.checked.lock().unwrap());
        // With the handoff wired but nothing committed, every published byte equals a run
        // without a baker (the manifest only adds the bake fields).
        mesh_to_leaf_payloads(request("plain"), &RunControl::default()).unwrap();
        // Every published file, found through the row manifests (no directory listing, no
        // OS path separators in the keys).
        let files = |dir: &Path| -> BTreeMap<String, Vec<u8>> {
            let mut out = BTreeMap::new();
            for rows in ["leaves.jsonl", "proxies.jsonl"] {
                let text = fs::read_to_string(dir.join(rows)).unwrap();
                for line in text.lines() {
                    let row: Value = serde_json::from_str(line).unwrap();
                    let uri = row["uri"].as_str().unwrap().to_owned();
                    out.insert(uri.clone(), fs::read(dir.join(&uri)).unwrap());
                }
                out.insert(rows.to_owned(), text.into_bytes());
            }
            out
        };
        assert!(
            files(&work.path().join("out")) == files(&work.path().join("plain")),
            "the handoff wiring changed published bytes"
        );
        let manifest = |dir: &str| -> Value {
            let mut m: Value = serde_json::from_slice(
                &fs::read(work.path().join(dir).join("manifest.json")).unwrap(),
            )
            .unwrap();
            let lod = m["lod"].as_object_mut().unwrap();
            lod.remove("untexturedProxyBytes");
            lod.remove("handoffBytes");
            m
        };
        assert_eq!(manifest("out"), manifest("plain"));
    }
}
