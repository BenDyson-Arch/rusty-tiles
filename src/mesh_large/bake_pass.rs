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
        handoff::{FrozenNode, HandoffSink},
        mesh::Origin,
    },
};
use crate::{
    glb::{self, MeshoptLayout, MeshoptStream},
    JobError, JobErrorKind,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use sha2::{Digest, Sha256};
use std::{
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
    pages: Vec<PreencodePage>,
    primitives: Vec<LeafPrimitivePreencode>,
    textures: Vec<Value>,
    samplers: Vec<Value>,
}

impl LeafPreencode {
    fn from_atlas(atlas: &PreparedAtlas) -> Self {
        Self {
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
        }
    }

    /// Page texels are written raw after a JSON head (no text-encoded pixels).
    fn encode(&self) -> Result<Vec<u8>, JobError> {
        let mut pages = self.pages.clone();
        let mut blob: Vec<u8> = Vec::new();
        for p in &mut pages {
            for l in &mut p.levels {
                blob.extend_from_slice(&std::mem::take(&mut l.rgba));
            }
            if let Some(support) = &mut p.support {
                for v in std::mem::take(support) {
                    blob.extend_from_slice(&v.to_le_bytes());
                }
            }
        }
        let head = serde_json::to_vec(&LeafHead {
            pages,
            primitives: self.primitives.clone(),
            textures: self.textures.clone(),
            samplers: self.samplers.clone(),
        })
        .map_err(|e| state(e.to_string()))?;
        let mut out = Vec::with_capacity(16 + head.len() + blob.len());
        out.extend_from_slice(b"H17PRE\x01\0");
        out.extend_from_slice(&(head.len() as u64).to_le_bytes());
        out.extend_from_slice(&head);
        out.extend_from_slice(&blob);
        Ok(out)
    }

    fn decode(b: &[u8]) -> Result<Self, JobError> {
        let bad = || state("retained pre-encode packet is corrupt");
        if b.get(..8) != Some(b"H17PRE\x01\0") {
            return Err(bad());
        }
        let n = u64::from_le_bytes(b.get(8..16).ok_or_else(bad)?.try_into().unwrap()) as usize;
        let mut head: LeafHead =
            serde_json::from_slice(b.get(16..16 + n).ok_or_else(bad)?).map_err(|_| bad())?;
        let mut at = 16 + n;
        for p in &mut head.pages {
            for l in &mut p.levels {
                let len = (l.width as usize)
                    .checked_mul(l.height as usize)
                    .and_then(|x| x.checked_mul(4))
                    .ok_or_else(bad)?;
                l.rgba = b.get(at..at + len).ok_or_else(bad)?.to_vec();
                at += len;
            }
            if p.support.is_some() {
                let len = p.width as usize * p.height as usize;
                let raw = b.get(at..at + 4 * len).ok_or_else(bad)?;
                p.support = Some(
                    raw.chunks_exact(4)
                        .map(|c| u32::from_le_bytes(c.try_into().unwrap()))
                        .collect(),
                );
                at += 4 * len;
            }
        }
        if at != b.len() {
            return Err(bad());
        }
        Ok(Self {
            pages: head.pages,
            primitives: head.primitives,
            textures: head.textures,
            samplers: head.samplers,
        })
    }
}

/// Owned scratch for pre-encode appearance: accepted leaf packets (written by H8 while
/// encoding) and bake-produced proxy packets (opaque to the root), each kept until its
/// parent has consumed it. Every byte is charged against `limit`.
pub(in crate::mesh_large) struct PreencodeStore {
    dir: PathBuf,
    limit: u64,
    live: AtomicU64,
    peak: AtomicU64,
}

impl PreencodeStore {
    pub(in crate::mesh_large) fn new(dir: PathBuf, limit: u64) -> Result<Self, JobError> {
        fs::create_dir_all(&dir).map_err(|e| JobError::io("create pre-encode store", &dir, e))?;
        Ok(Self {
            dir,
            limit,
            live: AtomicU64::new(0),
            peak: AtomicU64::new(0),
        })
    }
    fn charge(&self, size: u64) -> Result<(), JobError> {
        let live = self.live.fetch_add(size, Ordering::Relaxed) + size;
        if live > self.limit {
            self.live.fetch_sub(size, Ordering::Relaxed);
            return Err(JobError::new(
                JobErrorKind::ResourceLimit,
                format!(
                    "retained pre-encode appearance exceeds its {} byte allowance",
                    self.limit
                ),
            ));
        }
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
        Ok(())
    }
    fn leaf_path(&self, index: u64) -> PathBuf {
        self.dir.join(format!("leaf-{index:012}.pre"))
    }
    fn proxy_path(&self, first: u64, depth: u32) -> PathBuf {
        self.dir.join(format!("proxy-f{first}-d{depth}.pre"))
    }
    /// Record a leaf's accepted pages (called once, for the accepted density attempt).
    pub(in crate::mesh_large) fn write_leaf(
        &self,
        index: u64,
        atlas: &PreparedAtlas,
    ) -> Result<(), JobError> {
        self.put(
            &self.leaf_path(index),
            &LeafPreencode::from_atlas(atlas).encode()?,
        )
    }
    pub(in crate::mesh_large) fn read_leaf(&self, index: u64) -> Result<LeafPreencode, JobError> {
        let path = self.leaf_path(index);
        LeafPreencode::decode(
            &fs::read(&path).map_err(|e| JobError::io("read pre-encode store", &path, e))?,
        )
    }
    pub(in crate::mesh_large) fn release_leaf(&self, index: u64) -> Result<(), JobError> {
        self.remove(&self.leaf_path(index))
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
    pub(in crate::mesh_large) fn read_proxy(
        &self,
        first: u64,
        depth: u32,
    ) -> Result<Vec<u8>, JobError> {
        let path = self.proxy_path(first, depth);
        fs::read(&path).map_err(|e| JobError::io("read pre-encode store", &path, e))
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

/// The decoded identity of one face of an assembled member, read back from its bytes.
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

/// The adapter's verdict on one prepared node.
#[derive(Debug)]
pub(in crate::mesh_large) enum Assembly {
    /// The complete member fits the tile budget and every requested value read back.
    Fit {
        glb: Vec<u8>,
        decoded: Vec<FaceIdentity>,
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

    pub(in crate::mesh_large) fn frozen(
        &self,
        first: u64,
        depth: u32,
    ) -> Result<FrozenNode, JobError> {
        self.handoff.read(first, depth)
    }

    /// Drop a tile's packet once it is baked.
    pub(in crate::mesh_large) fn release(&self, first: u64, depth: u32) -> Result<(), JobError> {
        self.handoff.release(first, depth)
    }

    /// The complete-member adapter.
    pub(in crate::mesh_large) fn assemble(
        &self,
        frozen: &FrozenNode,
        prepared: &PreparedNode,
    ) -> Result<Assembly, JobError> {
        assemble(frozen, prepared, self.tile_bytes)
    }

    /// Publish an accepted member over the tile's untextured proxy (geometric error
    /// unchanged; the proxy row's size and hash are refreshed by [`BakePass::finish`]).
    pub(in crate::mesh_large) fn commit(
        &self,
        frozen: &FrozenNode,
        glb: &[u8],
    ) -> Result<(), JobError> {
        (self.check)()?;
        let id = &frozen.identity;
        let path = self
            .stage
            .join(format!("proxies/f{}-d{}.glb", id.first, id.depth));
        if !path.exists() {
            return Err(state("a baked member has no proxy to replace"));
        }
        let mut file =
            fs::File::create(&path).map_err(|e| JobError::io("write baked proxy", &path, e))?;
        file.write_all(glb)
            .and_then(|_| file.sync_all())
            .map_err(|e| JobError::io("write baked proxy", &path, e))
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

/// Build the complete member for `prepared` and verify it from its own bytes.
pub(in crate::mesh_large) fn assemble(
    frozen: &FrozenNode,
    prepared: &PreparedNode,
    tile_bytes: u64,
) -> Result<Assembly, JobError> {
    frozen.validate()?;
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
            let mut bytes = Vec::with_capacity(count * 4);
            for &f in &p.faces {
                let id = frozen.faces[f as usize]
                    .features
                    .get(slot)
                    .ok_or_else(|| state("frozen face lacks a feature-ID set"))?
                    .0;
                for _ in 0..3 {
                    bytes.extend_from_slice(&(id as f32).to_le_bytes());
                }
            }
            slot += 1;
            let v = w.view(&bytes, Some(MeshoptStream::Attributes { count, stride: 4 }));
            w.accessors
                .push(json!({"bufferView":v,"componentType":5126,"count":count,"type":"SCALAR"}));
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
    Ok(Assembly::Fit { glb: out, decoded })
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
        if features != face.features {
            return Err(bad("feature ID or table link"));
        }
        let s = &frozen.primitives[face.source as usize];
        identities.push(FaceIdentity {
            face: f,
            raw: d.raw,
            decoded: d.p.map(|q| q.map(f32::to_bits)),
            origin: face.origin,
            source: (s.node, s.mesh, s.primitive, s.expanded_ordinal),
            material: s.material,
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
    use crate::{
        mesh_large::{
            mesh_leaves_to_3tz, mesh_to_leaf_payloads, LeafOutputRequest, LeafProfile,
            MeshTilesetRequest,
        },
        validate::{inspect, ValidationRequest},
        RunControl,
    };
    use std::{num::NonZeroUsize, sync::Arc, sync::Mutex};

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
                    pos.push([x, (x / 3.).sin() * (z / 4.).cos(), z]);
                    uv.push([i as f32 / g as f32, j as f32 / g as f32]);
                }
            }
            let mut idx = Vec::new();
            for i in i0..i1 {
                for j in 0..g {
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
    fn flat(frozen: &FrozenNode) -> PreparedNode {
        let primitives = frozen
            .gltf
            .iter()
            .enumerate()
            .map(|(i, g)| {
                let n = (g.faces.end - g.faces.start) as usize * 3;
                PreparedPrimitive {
                    gltf: i as u32,
                    faces: g.faces.clone().collect(),
                    material: Some(0),
                    uvs: vec![vec![[0.25, 0.75]; n]],
                    normals: Some(vec![[0., 1., 0.]; n]),
                    ..Default::default()
                }
            })
            .collect();
        PreparedNode {
            primitives,
            materials: vec![
                json!({"pbrMetallicRoughness":{"baseColorTexture":{"index":0},"metallicFactor":0.0}}),
            ],
            textures: vec![json!({"source":0,"sampler":0})],
            samplers: vec![json!({"magFilter":9729,"minFilter":9729,"wrapS":33071,"wrapT":33071})],
            images: vec![PreparedImage {
                bytes: png([200, 100, 50, 255]),
                mime: "image/png",
            }],
        }
    }

    /// A test baker: postorder, reads every packet and child appearance, assembles and
    /// commits a flat textured member, records what it saw.
    #[derive(Debug, Default)]
    struct FlatBaker {
        seen: Mutex<Vec<String>>,
    }
    impl ProxyBaker for FlatBaker {
        fn bake(&self, pass: &BakePass<'_>) -> Result<(), JobError> {
            let mut done: Vec<(u64, u32)> = Vec::new();
            for node in pass.nodes()? {
                let (first, depth) = node?;
                let frozen = pass.frozen(first, depth)?;
                assert_eq!(
                    (frozen.identity.first, frozen.identity.depth),
                    (first, depth)
                );
                // Children come first (postorder) and their appearance is retained.
                let mut leaf_pages = 0;
                for c in &frozen.children {
                    match c.leaf {
                        Some(leaf) => {
                            let pre = pass.preencode.read_leaf(leaf.index)?;
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
                            assert_eq!(pass.preencode.read_proxy(c.first, c.depth)?, b"pages");
                            pass.preencode.release_proxy(c.first, c.depth)?;
                        }
                    }
                }
                let prepared = flat(&frozen);
                // The physical budget path: a tiny budget reports the actual size.
                match assemble(&frozen, &prepared, 64)? {
                    Assembly::PhysicalBudget { actual_bytes } => assert!(actual_bytes > 64),
                    Assembly::Fit { .. } => panic!("a 64-byte member cannot fit"),
                }
                let Assembly::Fit { glb, decoded } = pass.assemble(&frozen, &prepared)? else {
                    panic!("the flat member fits");
                };
                assert_eq!(decoded.len(), frozen.faces.len());
                for d in &decoded {
                    let f = &frozen.faces[d.face as usize];
                    assert_eq!(d.origin, f.origin);
                    assert_eq!(d.decoded, f.corners.map(|c| c.decoded));
                }
                pass.commit(&frozen, &glb)?;
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

    fn faces_of(glb: &[u8]) -> Vec<[[u32; 3]; 3]> {
        let mut f: Vec<_> = codec::decode(glb)
            .unwrap()
            .faces
            .iter()
            .map(|f| f.p.map(|p| p.map(f32::to_bits)))
            .collect();
        f.sort_unstable();
        f
    }

    /// The handoff end to end on real H8 + H17 output: packets are complete and consistent,
    /// leaf pre-encode pages are retained, the baked members carry the certified geometry
    /// unchanged with every table, and the archive validates with decoded containment.
    #[test]
    fn bake_pass_hands_off_frozen_tiles_and_commits_complete_members() {
        let work = tempfile::tempdir().unwrap();
        let input = fixture(&work.path().join("src"), 140);
        let run = |name: &str, baker: Option<Arc<FlatBaker>>| {
            let out = work.path().join(name);
            let scratch = work.path().join(format!("{name}-scratch"));
            fs::create_dir_all(&scratch).unwrap();
            let mut request = LeafOutputRequest::new(&input, &out, LeafProfile::Balanced)
                .with_jobs(NonZeroUsize::new(2).unwrap())
                .with_scratch_dir(&scratch)
                .with_lod_target(1024);
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
        let input = fixture(&work.path().join("src"), 140);
        #[derive(Debug, Default)]
        struct Probe {
            checked: Mutex<bool>,
        }
        impl ProxyBaker for Probe {
            fn bake(&self, pass: &BakePass<'_>) -> Result<(), JobError> {
                let (first, depth) = pass.nodes()?.next().unwrap()?;
                let frozen = pass.frozen(first, depth)?;
                // Packet round trip.
                assert_eq!(FrozenNode::decode(&frozen.encode()?)?, frozen);
                for f in &frozen.faces {
                    assert!(!frozen.donor_ranges(f).is_empty());
                }
                let good = flat(&frozen);
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
                    assemble(&frozen, &missing, u64::MAX).unwrap_err().kind(),
                    JobErrorKind::InvalidState
                );
                let Assembly::Fit { glb, .. } = assemble(&frozen, &good, u64::MAX)? else {
                    panic!()
                };
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
        mesh_to_leaf_payloads(
            LeafOutputRequest::new(&input, work.path().join("out"), LeafProfile::Balanced)
                .with_jobs(NonZeroUsize::new(1).unwrap())
                .with_lod_target(1024)
                .with_baker(probe.clone()),
            &RunControl::default(),
        )
        .unwrap();
        assert!(*probe.checked.lock().unwrap());
    }
}
