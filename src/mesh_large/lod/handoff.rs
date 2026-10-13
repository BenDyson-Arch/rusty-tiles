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

const MAGIC: &[u8; 8] = b"H17FRZ\x01\0";

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
            b.push(u8::try_from(f.features.len()).map_err(|_| state("too many feature-ID sets"))?);
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

    pub(in crate::mesh_large) fn decode(b: &[u8]) -> Result<Self, JobError> {
        let bad = || state("frozen tile packet is corrupt");
        if b.get(..8) != Some(MAGIC) {
            return Err(bad());
        }
        let mut at = 8usize;
        let take = |at: &mut usize, n: usize| -> Result<&[u8], JobError> {
            let s = b.get(*at..*at + n).ok_or_else(bad)?;
            *at += n;
            Ok(s)
        };
        let u64_ = |s: &[u8]| u64::from_le_bytes(s.try_into().unwrap());
        let u32_ = |s: &[u8]| u32::from_le_bytes(s.try_into().unwrap());
        let n = u64_(take(&mut at, 8)?) as usize;
        let head: Head = serde_json::from_slice(take(&mut at, n)?).map_err(|_| bad())?;
        let count = u64_(take(&mut at, 8)?) as usize;
        let mut faces = Vec::with_capacity(count.min(1 << 22));
        for _ in 0..count {
            let s = take(&mut at, 20)?;
            let (gltf, source) = (u32_(&s[0..4]), u32_(&s[4..8]));
            let origin = Origin {
                node: u32_(&s[8..12]),
                mesh: u32_(&s[12..16]),
                primitive: u32_(&s[16..20]),
                triangle: u64_(take(&mut at, 8)?),
            };
            let fill = u32_(take(&mut at, 4)?);
            let sets = take(&mut at, 1)?[0] as usize;
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

/// Where frozen tiles go: one file per node identity under an owned scratch folder, with
/// every byte charged against `limit` and the volume preflighted before each write.
pub(in crate::mesh_large) struct HandoffSink {
    dir: PathBuf,
    limit: u64,
    live: AtomicU64,
    peak: AtomicU64,
    written: AtomicU64,
}

impl HandoffSink {
    pub(in crate::mesh_large) fn new(dir: PathBuf, limit: u64) -> Result<Self, JobError> {
        fs::create_dir_all(&dir).map_err(|e| JobError::io("create bake handoff", &dir, e))?;
        Ok(Self {
            dir,
            limit,
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
        let live = self.live.fetch_add(size, Ordering::Relaxed) + size;
        if live > self.limit {
            self.live.fetch_sub(size, Ordering::Relaxed);
            return Err(JobError::new(
                JobErrorKind::ResourceLimit,
                format!(
                    "bake handoff exceeds its {} byte scratch allowance",
                    self.limit
                ),
            ));
        }
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

    pub(in crate::mesh_large) fn read(
        &self,
        first: u64,
        depth: u32,
    ) -> Result<FrozenNode, JobError> {
        let path = self.path(first, depth);
        let bytes = fs::read(&path).map_err(|e| JobError::io("read bake handoff", &path, e))?;
        FrozenNode::decode(&bytes)
    }

    /// Drop a consumed packet and its charge.
    pub(in crate::mesh_large) fn release(&self, first: u64, depth: u32) -> Result<(), JobError> {
        let path = self.path(first, depth);
        let size = fs::metadata(&path)
            .map_err(|e| JobError::io("inspect bake handoff", &path, e))?
            .len();
        fs::remove_file(&path).map_err(|e| JobError::io("remove bake handoff", &path, e))?;
        self.live.fetch_sub(size, Ordering::Relaxed);
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
