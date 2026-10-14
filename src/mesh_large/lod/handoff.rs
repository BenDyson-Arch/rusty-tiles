//! The immutable H17 -> H13 bake handoff: one typed snapshot per FINAL coarse tile.
//!
//! Written during the successful finishing pass only (never by a seam-consensus trial),
//! after the tile's published bytes are encoded, decoded and checked equal to the
//! certified faces, and before the private sidecars are removed. Each tile is one file
//! named by its node identity, so parallel tiles write separate deterministic files; the
//! sink charges every byte against its declared limit and preflights the scratch volume.
//! Nothing here re-quantizes positions: raw POSITION words and TRS bits are copied from
//! the decoded published GLB, the original weld keys from the tile's key-space mesh.
use super::mesh::{Key, Origin};
use crate::{JobError, JobErrorKind};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{
    fs,
    ops::Range,
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};

fn state(message: impl Into<String>) -> JobError {
    JobError::new(JobErrorKind::InvalidState, message.into())
}

/// H1 node identity of a tile.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(in crate::mesh_large) struct NodeIdentity {
    pub cell_depth: u8,
    pub cell: u64,
    pub path_len: u8,
    pub path: u64,
    pub last_octant: u8,
    pub depth: u32,
    /// Source rank range `[first, end)`.
    pub first: u64,
    pub end: u64,
}

/// A source primitive present in the proxy (one `source_primitive` row).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(in crate::mesh_large) struct FrozenPrimitive {
    pub node: u32,
    pub mesh: u32,
    pub primitive: u32,
    pub expanded_ordinal: u64,
    /// Original material ID (`u32::MAX`: none).
    pub material: u32,
    /// Row of this primitive in the `source_primitive` table (its `_FEATURE_ID_0`).
    pub row: u32,
    /// Certified per-primitive proxy -> source bound `e_A` (fills included).
    pub error: f64,
    /// Same-primitive donor/correspondence intervals: the maximal runs of source ranks in
    /// `[first, end)` whose triangles belong to this primitive. Sorted, disjoint, nonempty.
    pub donor_ranges: Vec<Range<u64>>,
}

/// One published glTF primitive.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(in crate::mesh_large) struct FrozenGltfPrimitive {
    pub node: u32,
    pub mesh: u32,
    pub index: u32,
    /// The source primitives it carries (indices into [`FrozenNode::primitives`]): one
    /// glTF primitive holds every source primitive of one flat material, consecutively.
    pub sources: Range<u32>,
    /// Published material index.
    pub material: Option<u32>,
    /// Node translation / scale as f32 bit patterns.
    pub translation: [u32; 3],
    pub scale: [u32; 3],
    /// u16 codes (`KHR_mesh_quantization`) or f32 positions.
    pub quantized: bool,
    /// `EXT_mesh_features.featureIds` exactly as published (any number of sets).
    pub feature_ids: Value,
    /// Its faces in [`FrozenNode::faces`].
    pub faces: Range<u32>,
}

/// An accepted hole fill (`lod_filled` row).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(in crate::mesh_large) struct FrozenFill {
    /// Owning and ONLY donor primitive (index into [`FrozenNode::primitives`]); donors are
    /// restricted to its `donor_ranges`.
    pub source: u32,
    pub loop_key: Key,
    pub loop_vertices: u32,
    pub fan_faces: u32,
    /// Certified `b_A(F)`, at most the tile's error.
    pub bound: f64,
    pub reopen_children: u8,
}

/// A removal record (`lod_vanished` row).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(in crate::mesh_large) struct FrozenVanished {
    pub node: u32,
    pub mesh: u32,
    pub primitive: u32,
    pub component_key: Key,
    pub faces: u32,
    pub reopen_children: u8,
    /// 0 island, 1 lost to a forced seam collapse.
    pub reason: u8,
}

/// A child tile.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(in crate::mesh_large) struct FrozenChild {
    pub first: u64,
    pub end: u64,
    pub depth: u32,
    pub slot: u8,
    /// For a leaf child: its H8 leaf index (`leaves/<index>.glb`) and actual texture
    /// density drop. `None` for a proxy child.
    pub leaf: Option<LeafChild>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(in crate::mesh_large) struct LeafChild {
    pub index: u64,
    pub density_drop: u32,
}

/// A published property table, every column (unreferenced V/F tables included).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(in crate::mesh_large) struct FrozenTable {
    pub class: String,
    pub count: u64,
    /// `(name, componentType, values)`; FLOAT64 values are bit patterns.
    pub columns: Vec<(String, String, Vec<u64>)>,
}

/// One oriented corner in published order.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::mesh_large) struct FrozenCorner {
    /// Exact POSITION storage words (u16 codes widened, or f32 bits).
    pub raw: [u32; 3],
    /// Client-decoded position bits.
    pub decoded: [u32; 3],
    /// ORIGINAL pre-quantization weld key (normalized baked source position bits).
    pub weld: Key,
    /// Shallowest incident leaf depth `l(v)` used by its encoding.
    pub ell: u8,
}

/// One oriented face in published order.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(in crate::mesh_large) struct FrozenFace {
    /// Index into [`FrozenNode::gltf`].
    pub gltf: u32,
    /// Index into [`FrozenNode::primitives`].
    pub source: u32,
    pub corners: [FrozenCorner; 3],
    /// The source triangle it descends from (`triangle == u64::MAX` for a fill).
    pub origin: Origin,
    /// Index into [`FrozenNode::fills`] for a fill face.
    pub fill: Option<u32>,
    /// Per published feature-ID set (in `featureIds` order): its feature ID and table.
    pub features: Vec<(u32, Option<u32>)>,
}

/// The frozen tile.
#[derive(Clone, Debug, PartialEq)]
pub(in crate::mesh_large) struct FrozenNode {
    pub identity: NodeIdentity,
    pub profile: String,
    /// Accepted published geometric error (never changed by baking).
    pub geometric_error: f64,
    pub proxy_to_source: f64,
    pub source_to_proxy: f64,
    pub lattice_sha256: String,
    /// The certified untextured proxy GLB this packet was read from.
    pub glb_sha256: [u8; 32],
    pub glb_bytes: u64,
    pub primitives: Vec<FrozenPrimitive>,
    pub gltf: Vec<FrozenGltfPrimitive>,
    pub faces: Vec<FrozenFace>,
    pub fills: Vec<FrozenFill>,
    pub vanished: Vec<FrozenVanished>,
    pub children: Vec<FrozenChild>,
    pub tables: Vec<FrozenTable>,
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Head {
    identity: NodeIdentity,
    profile: String,
    geometric_error: f64,
    proxy_to_source: f64,
    source_to_proxy: f64,
    lattice_sha256: String,
    glb_sha256: String,
    glb_bytes: u64,
    primitives: Vec<FrozenPrimitive>,
    gltf: Vec<FrozenGltfPrimitive>,
    fills: Vec<FrozenFill>,
    vanished: Vec<FrozenVanished>,
    children: Vec<FrozenChild>,
    tables: Vec<FrozenTable>,
}

/// Wire v2: magic; the certified member's length (u64) and SHA-256 (a fixed prefix, so
/// it is read without the head); head length and JSON head; face count and faces, each
/// with a u32 feature-ID set count (v1 capped it at 255 in a u8).
const MAGIC: &[u8; 8] = b"H17FRZ\x02\0";

/// A caller's memory allowance for one bounded read, decode or assembly. Every owned
/// allocation is charged BEFORE it is made; beyond the limit is `ResourceLimit`.
#[derive(Debug)]
pub(in crate::mesh_large) struct Lease {
    limit: u64,
    used: u64,
    what: &'static str,
}

impl Lease {
    pub(in crate::mesh_large) fn new(limit: u64, what: &'static str) -> Self {
        Self {
            limit,
            used: 0,
            what,
        }
    }
    pub(in crate::mesh_large) fn charge(&mut self, bytes: u64) -> Result<(), JobError> {
        match self.used.checked_add(bytes) {
            Some(used) if used <= self.limit => {
                self.used = used;
                Ok(())
            }
            _ => Err(JobError::new(
                JobErrorKind::ResourceLimit,
                format!(
                    "{} needs more than its {} byte memory lease",
                    self.what, self.limit
                ),
            )),
        }
    }
    /// Charge `count` items of `size` bytes each (overflow is over the lease).
    pub(in crate::mesh_large) fn charge_items(
        &mut self,
        count: u64,
        size: usize,
    ) -> Result<(), JobError> {
        self.charge(count.checked_mul(size as u64).unwrap_or(u64::MAX))
    }
    pub(in crate::mesh_large) fn used(&self) -> u64 {
        self.used
    }
    pub(in crate::mesh_large) fn limit(&self) -> u64 {
        self.limit
    }
}

/// Read a whole file under `lease`: its size is queried and charged before any
/// allocation, it is read in bounded chunks with `check` between them, and a file that
/// changes size while read is `InvalidState`. `expected` pins the size when known.
pub(in crate::mesh_large) fn read_bounded(
    path: &Path,
    expected: Option<u64>,
    lease: &mut Lease,
    check: &dyn Fn() -> Result<(), JobError>,
    what: &str,
) -> Result<Vec<u8>, JobError> {
    use std::io::Read;
    check()?;
    let io = |e| JobError::io(what, path, e);
    let mut file = fs::File::open(path).map_err(io)?;
    let size = file.metadata().map_err(io)?.len();
    if expected.is_some_and(|e| e != size) {
        return Err(state(format!(
            "{what}: {} holds {size} bytes, not the {} its owner recorded",
            path.display(),
            expected.unwrap_or_default()
        )));
    }
    lease.charge(size)?;
    let size = usize::try_from(size).map_err(|_| state(format!("{what}: file too large")))?;
    let mut out = vec![0u8; size];
    let mut at = 0;
    while at < size {
        check()?;
        let end = (at + (1 << 20)).min(size);
        file.read_exact(&mut out[at..end]).map_err(io)?;
        at = end;
    }
    let mut probe = [0u8; 1];
    if file.read(&mut probe).map_err(io)? != 0 {
        return Err(state(format!("{what}: {} grew while read", path.display())));
    }
    Ok(out)
}

impl FrozenNode {
    /// The same-primitive donor intervals of a face (its owning primitive's ranges).
    pub(in crate::mesh_large) fn donor_ranges(&self, face: &FrozenFace) -> &[Range<u64>] {
        &self.primitives[face.source as usize].donor_ranges
    }

    /// Structural checks every reader relies on.
    pub(in crate::mesh_large) fn validate(&self) -> Result<(), JobError> {
        let id = &self.identity;
        if id.first >= id.end || self.faces.is_empty() || self.primitives.is_empty() {
            return Err(state("frozen tile without faces or rank range"));
        }
        for (i, p) in self.primitives.iter().enumerate() {
            if p.row != i as u32 {
                return Err(state(
                    "frozen primitive row is not its source_primitive row",
                ));
            }
            if p.donor_ranges.is_empty()
                || p.donor_ranges
                    .iter()
                    .any(|r| r.start >= r.end || r.start < id.first || r.end > id.end)
                || p.donor_ranges.windows(2).any(|w| w[0].end >= w[1].start)
            {
                return Err(state(
                    "frozen donor ranges are not sorted, disjoint and in range",
                ));
            }
        }
        let mut next = 0u32;
        for g in &self.gltf {
            if g.faces.start != next
                || g.faces.end < g.faces.start
                || g.sources.start >= g.sources.end
                || g.sources.end as usize > self.primitives.len()
            {
                return Err(state("frozen glTF primitives do not partition the faces"));
            }
            next = g.faces.end;
        }
        if next as usize != self.faces.len() {
            return Err(state("frozen glTF primitives do not cover the faces"));
        }
        for f in &self.faces {
            let g = self
                .gltf
                .get(f.gltf as usize)
                .ok_or_else(|| state("frozen face without its glTF primitive"))?;
            // Set 0 is the source_primitive row of the face's own primitive, and every
            // declared attribute set has one value per face (corner ell is checked against
            // the lattice when the packet is written).
            let attribute_sets = g.feature_ids.as_array().map_or(0, |sets| {
                sets.iter().filter(|s| s.get("attribute").is_some()).count()
            });
            if f.features.len() != attribute_sets
                || f.features.first().map(|s| s.0)
                    != self.primitives.get(f.source as usize).map(|p| p.row)
            {
                return Err(state("frozen face feature IDs are inconsistent"));
            }
            if !g.sources.contains(&f.source)
                || f.fill.is_some_and(|i| {
                    self.fills
                        .get(i as usize)
                        .is_none_or(|fill| fill.source != f.source)
                })
                || (f.fill.is_some()) != (f.origin.triangle == u64::MAX)
            {
                return Err(state("frozen face identity is inconsistent"));
            }
        }
        Ok(())
    }

    pub(in crate::mesh_large) fn encode(&self) -> Result<Vec<u8>, JobError> {
        let head = Head {
            identity: self.identity,
            profile: self.profile.clone(),
            geometric_error: self.geometric_error,
            proxy_to_source: self.proxy_to_source,
            source_to_proxy: self.source_to_proxy,
            lattice_sha256: self.lattice_sha256.clone(),
            glb_sha256: self.glb_sha256.iter().map(|b| format!("{b:02x}")).collect(),
            glb_bytes: self.glb_bytes,
            primitives: self.primitives.clone(),
            gltf: self.gltf.clone(),
            fills: self.fills.clone(),
            vanished: self.vanished.clone(),
            children: self.children.clone(),
            tables: self.tables.clone(),
        };
        let head = serde_json::to_vec(&head).map_err(|e| state(e.to_string()))?;
        let mut b = Vec::with_capacity(head.len() + self.faces.len() * 140 + 32);
        b.extend_from_slice(MAGIC);
        b.extend_from_slice(&self.glb_bytes.to_le_bytes());
        b.extend_from_slice(&self.glb_sha256);
        b.extend_from_slice(&(head.len() as u64).to_le_bytes());
        b.extend_from_slice(&head);
        b.extend_from_slice(&(self.faces.len() as u64).to_le_bytes());
        let u32s = |b: &mut Vec<u8>, v: &[u32]| {
            for x in v {
                b.extend_from_slice(&x.to_le_bytes());
            }
        };
        for f in &self.faces {
            u32s(
                &mut b,
                &[
                    f.gltf,
                    f.source,
                    f.origin.node,
                    f.origin.mesh,
                    f.origin.primitive,
                ],
            );
            b.extend_from_slice(&f.origin.triangle.to_le_bytes());
            u32s(&mut b, &[f.fill.unwrap_or(u32::MAX)]);
            u32s(
                &mut b,
                &[u32::try_from(f.features.len())
                    .map_err(|_| state("too many feature-ID sets"))?],
            );
            for (id, table) in &f.features {
                u32s(&mut b, &[*id, table.unwrap_or(u32::MAX)]);
            }
            for c in &f.corners {
                u32s(&mut b, &c.raw);
                u32s(&mut b, &c.decoded);
                u32s(&mut b, &c.weld);
                b.push(c.ell);
            }
        }
        Ok(b)
    }

    /// Decode a packet, charging every owned allocation to `lease` before it is made and
    /// checking for cancellation as faces are read.
    pub(in crate::mesh_large) fn decode(
        b: &[u8],
        lease: &mut Lease,
        check: &dyn Fn() -> Result<(), JobError>,
    ) -> Result<Self, JobError> {
        let bad = || state("frozen tile packet is corrupt");
        if b.get(..8) != Some(MAGIC) {
            return Err(bad());
        }
        let mut at = 48usize;
        let take = |at: &mut usize, n: usize| -> Result<&[u8], JobError> {
            let s = b
                .get(*at..at.checked_add(n).ok_or_else(bad)?)
                .ok_or_else(bad)?;
            *at += n;
            Ok(s)
        };
        let u64_ = |s: &[u8]| u64::from_le_bytes(s.try_into().unwrap());
        let u32_ = |s: &[u8]| u32::from_le_bytes(s.try_into().unwrap());
        let n = usize::try_from(u64_(take(&mut at, 8)?)).map_err(|_| bad())?;
        // Decoded head structures: every JSON value is at least one byte of text and
        // decodes into at most eight bytes plus its container share.
        lease.charge_items(n as u64, 16)?;
        let head: Head = serde_json::from_slice(take(&mut at, n)?).map_err(|_| bad())?;
        let count = u64_(take(&mut at, 8)?);
        // Each face is at least its fixed wire size, so a forged count fails here.
        if count > (b.len() - at) as u64 / 147 {
            return Err(bad());
        }
        lease.charge_items(count, std::mem::size_of::<FrozenFace>())?;
        let mut faces = Vec::with_capacity(count as usize);
        for k in 0..count {
            if k.is_multiple_of(1 << 14) {
                check()?;
            }
            let s = take(&mut at, 20)?;
            let (gltf, source) = (u32_(&s[0..4]), u32_(&s[4..8]));
            let origin = Origin {
                node: u32_(&s[8..12]),
                mesh: u32_(&s[12..16]),
                primitive: u32_(&s[16..20]),
                triangle: u64_(take(&mut at, 8)?),
            };
            let fill = u32_(take(&mut at, 4)?);
            let sets = u32_(take(&mut at, 4)?) as usize;
            if sets > (b.len() - at) / 8 {
                return Err(bad());
            }
            lease.charge_items(sets as u64, std::mem::size_of::<(u32, Option<u32>)>())?;
            let mut features = Vec::with_capacity(sets);
            for _ in 0..sets {
                let s = take(&mut at, 8)?;
                let table = u32_(&s[4..8]);
                features.push((u32_(&s[0..4]), (table != u32::MAX).then_some(table)));
            }
            let mut corners = [FrozenCorner {
                raw: [0; 3],
                decoded: [0; 3],
                weld: [0; 3],
                ell: 0,
            }; 3];
            for c in &mut corners {
                let s = take(&mut at, 37)?;
                let w = |o: usize| {
                    [
                        u32_(&s[o..o + 4]),
                        u32_(&s[o + 4..o + 8]),
                        u32_(&s[o + 8..o + 12]),
                    ]
                };
                *c = FrozenCorner {
                    raw: w(0),
                    decoded: w(12),
                    weld: w(24),
                    ell: s[36],
                };
            }
            faces.push(FrozenFace {
                gltf,
                source,
                corners,
                origin,
                fill: (fill != u32::MAX).then_some(fill),
                features,
            });
        }
        if at != b.len() {
            return Err(bad());
        }
        let mut sha = [0u8; 32];
        if head.glb_sha256.len() != 64 {
            return Err(bad());
        }
        for (i, byte) in sha.iter_mut().enumerate() {
            *byte =
                u8::from_str_radix(&head.glb_sha256[2 * i..2 * i + 2], 16).map_err(|_| bad())?;
        }
        // The fixed prefix must agree with the head.
        if b.get(8..16) != Some(&head.glb_bytes.to_le_bytes()[..])
            || b.get(16..48) != Some(&sha[..])
        {
            return Err(bad());
        }
        let node = Self {
            identity: head.identity,
            profile: head.profile,
            geometric_error: head.geometric_error,
            proxy_to_source: head.proxy_to_source,
            source_to_proxy: head.source_to_proxy,
            lattice_sha256: head.lattice_sha256,
            glb_sha256: sha,
            glb_bytes: head.glb_bytes,
            primitives: head.primitives,
            gltf: head.gltf,
            faces,
            fills: head.fills,
            vanished: head.vanished,
            children: head.children,
            tables: head.tables,
        };
        node.validate()?;
        Ok(node)
    }
}

/// One scratch allowance shared by every bake-pass store (frozen packets and retained
/// pre-encode pages), so together they never claim more than it.
#[derive(Debug)]
pub(in crate::mesh_large) struct ScratchBudget {
    limit: u64,
    live: AtomicU64,
    peak: AtomicU64,
}

impl ScratchBudget {
    pub(in crate::mesh_large) fn new(limit: u64) -> Self {
        Self {
            limit,
            live: AtomicU64::new(0),
            peak: AtomicU64::new(0),
        }
    }
    /// Charge `size` bytes for `what`; `ResourceLimit` beyond the allowance (nothing kept).
    pub(in crate::mesh_large) fn charge(&self, size: u64, what: &str) -> Result<(), JobError> {
        let live = self.live.fetch_add(size, Ordering::Relaxed) + size;
        if live > self.limit {
            self.live.fetch_sub(size, Ordering::Relaxed);
            return Err(JobError::new(
                JobErrorKind::ResourceLimit,
                format!(
                    "{what} exceeds the bake pass's {} byte scratch allowance",
                    self.limit
                ),
            ));
        }
        self.peak.fetch_max(live, Ordering::Relaxed);
        Ok(())
    }
    pub(in crate::mesh_large) fn refund(&self, size: u64) {
        self.live.fetch_sub(size, Ordering::Relaxed);
    }
    /// `(live, peak, limit)` bytes.
    pub(in crate::mesh_large) fn usage(&self) -> (u64, u64, u64) {
        (
            self.live.load(Ordering::Relaxed),
            self.peak.load(Ordering::Relaxed),
            self.limit,
        )
    }
}

/// Where frozen tiles go: one file per node identity under an owned scratch folder, with
/// every byte charged against the shared [`ScratchBudget`] and the volume preflighted
/// before each write.
pub(in crate::mesh_large) struct HandoffSink {
    dir: PathBuf,
    budget: std::sync::Arc<ScratchBudget>,
    live: AtomicU64,
    peak: AtomicU64,
    written: AtomicU64,
}

impl HandoffSink {
    pub(in crate::mesh_large) fn new(
        dir: PathBuf,
        budget: std::sync::Arc<ScratchBudget>,
    ) -> Result<Self, JobError> {
        fs::create_dir_all(&dir).map_err(|e| JobError::io("create bake handoff", &dir, e))?;
        Ok(Self {
            dir,
            budget,
            live: AtomicU64::new(0),
            peak: AtomicU64::new(0),
            written: AtomicU64::new(0),
        })
    }

    pub(in crate::mesh_large) fn path(&self, first: u64, depth: u32) -> PathBuf {
        self.dir
            .join(format!("d{depth}"))
            .join(format!("f{first}.frz"))
    }

    /// Write one tile's packet (cancellable; refuses beyond the declared limit).
    pub(in crate::mesh_large) fn write(
        &self,
        node: &FrozenNode,
        check: &dyn Fn() -> Result<(), JobError>,
    ) -> Result<u64, JobError> {
        check()?;
        node.validate()?;
        let bytes = node.encode()?;
        let size = bytes.len() as u64;
        self.budget.charge(size, "the bake handoff")?;
        let live = self.live.fetch_add(size, Ordering::Relaxed) + size;
        self.peak.fetch_max(live, Ordering::Relaxed);
        self.written.fetch_add(1, Ordering::Relaxed);
        let path = self.path(node.identity.first, node.identity.depth);
        let parent = path.parent().expect("packet paths have a folder");
        fs::create_dir_all(parent).map_err(|e| JobError::io("create bake handoff", parent, e))?;
        crate::spill::preflight(
            parent,
            size,
            crate::spill::SpaceRole::Scratch,
            crate::spill::PreflightStage::Lod,
        )?;
        fs::write(&path, bytes).map_err(|e| JobError::io("write bake handoff", &path, e))?;
        Ok(size)
    }

    /// Read and decode one packet within `memory_limit` bytes (file bytes plus decoded
    /// structures), cancellable throughout.
    pub(in crate::mesh_large) fn read(
        &self,
        first: u64,
        depth: u32,
        memory_limit: u64,
        check: &dyn Fn() -> Result<(), JobError>,
    ) -> Result<FrozenNode, JobError> {
        let path = self.path(first, depth);
        let mut lease = Lease::new(memory_limit, "reading a frozen tile packet");
        let bytes = read_bounded(&path, None, &mut lease, check, "read bake handoff")?;
        FrozenNode::decode(&bytes, &mut lease, check)
    }

    /// Bytes of a packet on disk (a size query before any read).
    pub(in crate::mesh_large) fn size(&self, first: u64, depth: u32) -> Result<u64, JobError> {
        let path = self.path(first, depth);
        Ok(fs::metadata(&path)
            .map_err(|e| JobError::io("inspect bake handoff", &path, e))?
            .len())
    }

    /// The certified untextured member's recorded length and SHA-256, from the packet's
    /// fixed 48-byte prefix (no head parse, no allocation).
    pub(in crate::mesh_large) fn certified(
        &self,
        first: u64,
        depth: u32,
        check: &dyn Fn() -> Result<(), JobError>,
    ) -> Result<(u64, [u8; 32]), JobError> {
        use std::io::Read;
        check()?;
        let path = self.path(first, depth);
        let io = |e| JobError::io("read bake handoff head", &path, e);
        let mut file = fs::File::open(&path).map_err(io)?;
        let mut prefix = [0u8; 48];
        file.read_exact(&mut prefix).map_err(io)?;
        if &prefix[..8] != MAGIC {
            return Err(state("frozen tile packet is corrupt"));
        }
        let length = u64::from_le_bytes(prefix[8..16].try_into().unwrap());
        Ok((length, prefix[16..48].try_into().unwrap()))
    }

    /// Drop a consumed packet and its charge.
    pub(in crate::mesh_large) fn release(&self, first: u64, depth: u32) -> Result<(), JobError> {
        let path = self.path(first, depth);
        let size = fs::metadata(&path)
            .map_err(|e| JobError::io("inspect bake handoff", &path, e))?
            .len();
        fs::remove_file(&path).map_err(|e| JobError::io("remove bake handoff", &path, e))?;
        self.live.fetch_sub(size, Ordering::Relaxed);
        self.budget.refund(size);
        Ok(())
    }

    /// `(live, peak)` charged bytes and packets written.
    pub(in crate::mesh_large) fn usage(&self) -> (u64, u64, u64) {
        (
            self.live.load(Ordering::Relaxed),
            self.peak.load(Ordering::Relaxed),
            self.written.load(Ordering::Relaxed),
        )
    }

    pub(in crate::mesh_large) fn dir(&self) -> &Path {
        &self.dir
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// A one-face packet with `sets` attribute feature-ID sets (set 0 the primitive row).
    fn packet(sets: usize) -> FrozenNode {
        let feature_ids: Vec<Value> = (0..sets)
            .map(|k| json!({"featureCount": 1 << 20, "attribute": k, "label": format!("set{k}")}))
            .collect();
        let corner = |x: u32| FrozenCorner {
            raw: [x, 0, 0],
            decoded: [x, 0, 0],
            weld: [x, 0, 0],
            ell: 3,
        };
        FrozenNode {
            identity: NodeIdentity {
                cell_depth: 0,
                cell: 0,
                path_len: 0,
                path: 0,
                last_octant: 0,
                depth: 0,
                first: 0,
                end: 4,
            },
            profile: "balanced".into(),
            geometric_error: 1.,
            proxy_to_source: 1.,
            source_to_proxy: 1.,
            lattice_sha256: "00".repeat(32),
            glb_sha256: [7; 32],
            glb_bytes: 1234,
            primitives: vec![FrozenPrimitive {
                node: 0,
                mesh: 0,
                primitive: 0,
                expanded_ordinal: 0,
                material: u32::MAX,
                row: 0,
                error: 1.,
                donor_ranges: vec![0..4],
            }],
            gltf: vec![FrozenGltfPrimitive {
                node: 0,
                mesh: 0,
                index: 0,
                sources: 0..1,
                material: Some(0),
                translation: [0; 3],
                scale: [1f32.to_bits(); 3],
                quantized: false,
                feature_ids: Value::Array(feature_ids),
                faces: 0..1,
            }],
            faces: vec![FrozenFace {
                gltf: 0,
                source: 0,
                corners: [corner(1), corner(2), corner(3)],
                origin: Origin {
                    node: 0,
                    mesh: 0,
                    primitive: 0,
                    triangle: 2,
                },
                fill: None,
                // Values above 2^24 survive the wire exactly.
                features: (0..sets)
                    .map(|k| {
                        let id = if k == 0 { 0 } else { (1 << 24) + k as u32 };
                        (id, Some(k as u32))
                    })
                    .collect(),
            }],
            fills: vec![],
            vanished: vec![],
            children: vec![],
            tables: vec![],
        }
    }

    #[test]
    fn packets_round_trip_more_than_255_feature_sets_and_refuse_short_leases() {
        let check = || Ok(());
        for sets in [1, 255, 256, 300] {
            let node = packet(sets);
            let bytes = node.encode().unwrap();
            let mut lease = Lease::new(u64::MAX, "test");
            assert_eq!(
                FrozenNode::decode(&bytes, &mut lease, &check).unwrap(),
                node
            );
            // The decoded structures are charged; a lease just below fails before them.
            let needed = lease.used();
            let mut short = Lease::new(needed - 1, "test");
            assert_eq!(
                FrozenNode::decode(&bytes, &mut short, &check)
                    .unwrap_err()
                    .kind(),
                JobErrorKind::ResourceLimit
            );
        }
    }

    #[test]
    fn forged_counts_prefix_and_cancellation_are_refused() {
        let node = packet(3);
        let bytes = node.encode().unwrap();
        let check = || Ok(());
        let head = u64::from_le_bytes(bytes[48..56].try_into().unwrap()) as usize;
        let faces_at = 56 + head;
        let decode = |b: &[u8]| FrozenNode::decode(b, &mut Lease::new(u64::MAX, "t"), &check);
        // A forged face count far beyond the remaining bytes.
        let mut forged = bytes.clone();
        forged[faces_at..faces_at + 8].copy_from_slice(&u64::MAX.to_le_bytes());
        assert!(decode(&forged).is_err());
        // A forged per-face set count.
        let mut forged = bytes.clone();
        let sets_at = faces_at + 8 + 20 + 8 + 4;
        forged[sets_at..sets_at + 4].copy_from_slice(&u32::MAX.to_le_bytes());
        assert!(decode(&forged).is_err());
        // The certified prefix must agree with the head.
        let mut forged = bytes.clone();
        forged[20] ^= 1;
        assert!(decode(&forged).is_err());
        // Cancellation reaches the decode.
        let cancelled = || Err(JobError::new(JobErrorKind::Cancelled, "cancelled"));
        assert_eq!(
            FrozenNode::decode(&bytes, &mut Lease::new(u64::MAX, "t"), &cancelled)
                .unwrap_err()
                .kind(),
            JobErrorKind::Cancelled
        );
    }
}
