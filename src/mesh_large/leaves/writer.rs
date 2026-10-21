//! H8 numeric-provenance leaf GLB writer. Atlas policy and codecs are resolved
//! before this boundary; meshopt is lossless wire compression, not approximation.
use super::super::{
    attributes::{FullTriangle, COLOR, NORMAL, TANGENT},
    config::CONSUMER_TABLE_ROWS,
    records::Bounds,
};
use super::{
    geometry::{EncodedPositions, LatticePlan, PositionStorage},
    prepared::PreparedAtlas,
};
use crate::{
    glb::{self, MeshoptLayout, MeshoptStream},
    JobError, JobErrorKind,
};
use serde_json::{json, Map, Value};
use std::collections::{BTreeMap, BTreeSet};

const TILE_LIMIT: u64 = 96 << 20;
type PrimitiveKey = (u32, u32, u32);
#[derive(Debug)]
pub(super) struct EncodedLeaf {
    pub glb: Vec<u8>,
    pub bounds: Bounds,
    pub geometry_error: f64,
    /// Decoded published POSITION per PREPARED (source) primitive, in atlas order and
    /// corner order (the lattice decode the writer proves client-order exact, unchanged by
    /// H28 merging).
    pub positions: Vec<Vec<[f32; 3]>>,
    /// The NORMAL a renderer reads per prepared primitive, in corner order (empty without
    /// NORMAL): the octahedral codes decoded by the reference meshopt filter and read
    /// as normalized SHORT, or the published f32 values.
    pub normals: Vec<Vec<[f32; 3]>>,
    /// Published glTF primitives (H28: one per material class, texture set, attribute
    /// set and exact position window).
    pub primitives: usize,
    /// Set by the producer for the accepted attempt (texel ownership, H28).
    pub shared_triangles: u64,
}
/// Only a measured/necessary member-size overrun may request density change.
#[derive(Debug)]
pub(super) enum WriteError {
    Budget(JobError),
    Failure(JobError),
}
impl WriteError {
    pub(super) fn kind(&self) -> JobErrorKind {
        match self {
            Self::Budget(error) | Self::Failure(error) => error.kind(),
        }
    }
}
impl From<JobError> for WriteError {
    fn from(error: JobError) -> Self {
        Self::Failure(error)
    }
}
impl From<WriteError> for JobError {
    fn from(error: WriteError) -> Self {
        match error {
            WriteError::Budget(error) | WriteError::Failure(error) => error,
        }
    }
}
fn invalid(message: &str) -> JobError {
    JobError::new(JobErrorKind::InvalidState, message)
}
fn overflow() -> JobError {
    JobError::new(
        JobErrorKind::ResourceLimit,
        "leaf wire size/allocation exhausted",
    )
}
fn budget() -> JobError {
    JobError::new(
        JobErrorKind::Unsupported,
        "complete leaf GLB exceeds tile byte budget",
    )
}
fn key(row: &FullTriangle) -> PrimitiveKey {
    (
        row.triangle.source.node,
        row.triangle.source.mesh,
        row.triangle.source.primitive,
    )
}
fn allocate(length: usize) -> Result<Vec<u8>, JobError> {
    let mut bytes = Vec::new();
    bytes.try_reserve_exact(length).map_err(|_| overflow())?;
    Ok(bytes)
}
struct Wire {
    binary: Vec<u8>,
    views: Vec<Value>,
    streams: Vec<Option<MeshoptStream>>,
    accessors: Vec<Value>,
}
impl Wire {
    fn new() -> Self {
        Self {
            binary: Vec::new(),
            views: Vec::new(),
            streams: Vec::new(),
            accessors: Vec::new(),
        }
    }
    fn view(&mut self, bytes: &[u8], stream: Option<MeshoptStream>) -> Result<usize, JobError> {
        if bytes.is_empty() {
            return Err(invalid("empty prepared leaf buffer view"));
        }
        let offset = self.binary.len().checked_add(7).ok_or_else(overflow)? & !7;
        let end = offset.checked_add(bytes.len()).ok_or_else(overflow)?;
        self.binary
            .try_reserve(end - self.binary.len())
            .map_err(|_| overflow())?;
        self.binary.resize(offset, 0);
        self.binary.extend_from_slice(bytes);
        let mut view = json!({"buffer":0,"byteOffset":offset,"byteLength":bytes.len()});
        if let Some(
            MeshoptStream::Attributes { stride, .. } | MeshoptStream::Octahedral { stride, .. },
        ) = stream
        {
            view["target"] = json!(34962);
            view["byteStride"] = json!(stride);
        }
        let index = self.views.len();
        self.views.push(view);
        self.streams.push(stream);
        Ok(index)
    }
    fn attribute(
        &mut self,
        bytes: &[u8],
        count: usize,
        stride: usize,
        component: u32,
        shape: &str,
    ) -> Result<usize, JobError> {
        if count.checked_mul(stride) != Some(bytes.len()) {
            return Err(invalid("prepared leaf attribute count/stride mismatch"));
        }
        let view = self.view(bytes, Some(MeshoptStream::Attributes { count, stride }))?;
        let index = self.accessors.len();
        self.accessors.push(json!({"bufferView":view,"componentType":component,"count":count,"type":shape,"normalized":false}));
        Ok(index)
    }
    /// One tightly packed property-table column (no `byteStride` or `target`). Columns of
    /// 4- or 8-byte values are meshopt ATTRIBUTES streams with the value width as stride,
    /// so decoded bytes are unchanged; UINT8 columns stay raw because the extension needs
    /// a stride divisible by 4.
    fn column(&mut self, bytes: &[u8], width: usize) -> Result<usize, JobError> {
        let index = self.view(bytes, None)?;
        if width.is_multiple_of(4) {
            self.streams[index] = Some(MeshoptStream::Attributes {
                count: bytes.len() / width,
                stride: width,
            });
        }
        Ok(index)
    }
    /// Unit normals as `bits`-bit octahedral codes behind the meshopt OCTAHEDRAL filter;
    /// clients decode them to normalized SHORT VEC3 (KHR_mesh_quantization).
    /// Also returns what a renderer reads: the codes through the reference meshopt
    /// OCTAHEDRAL filter decode, as normalized SHORT (`max(c / 32767, -1)`).
    fn octahedral(
        &mut self,
        normals: &[[f32; 3]],
        bits: u8,
    ) -> Result<(usize, Vec<[f32; 3]>), JobError> {
        // Capture refuses non-finite normals, so one here is a broken prepared leaf: a
        // typed state failure, as for non-finite float attributes, never a budget retry.
        if normals.iter().flatten().any(|v| !v.is_finite()) {
            return Err(invalid("non-finite prepared leaf NORMAL"));
        }
        let bytes = glb::encode_octahedral(normals, bits).map_err(|e| invalid(&e.to_string()))?;
        let count = normals.len();
        let filtered = crate::content_integrity::meshopt::unfilter_octahedral(&bytes, count, 8)
            .map_err(|e| invalid(&format!("octahedral NORMAL decode: {e}")))?;
        let decoded = filtered
            .as_chunks::<8>()
            .0
            .iter()
            .map(|e| {
                std::array::from_fn(|a| {
                    (f32::from(i16::from_le_bytes([e[2 * a], e[2 * a + 1]])) / 32767.).max(-1.)
                })
            })
            .collect();
        let view = self.view(&bytes, Some(MeshoptStream::Octahedral { count, stride: 8 }))?;
        let index = self.accessors.len();
        self.accessors.push(json!({"bufferView":view,"componentType":5122,"count":count,"type":"VEC3","normalized":true}));
        Ok((index, decoded))
    }
    fn floats(&mut self, rows: &[Vec<f32>], shape: &str) -> Result<usize, JobError> {
        let width = rows
            .first()
            .ok_or_else(|| invalid("empty float attribute"))?
            .len();
        let length = rows
            .len()
            .checked_mul(width)
            .and_then(|x| x.checked_mul(4))
            .ok_or_else(overflow)?;
        let mut bytes = allocate(length)?;
        for row in rows {
            if row.len() != width || row.iter().any(|v| !v.is_finite()) {
                return Err(invalid("nonfinite/inconsistent prepared float attribute"));
            }
            for value in row {
                bytes.extend_from_slice(&value.to_le_bytes());
            }
        }
        self.attribute(&bytes, rows.len(), width * 4, 5126, shape)
    }
}
fn index(value: &Value, count: usize, message: &str) -> Result<usize, JobError> {
    let n = value
        .as_u64()
        .and_then(|v| usize::try_from(v).ok())
        .ok_or_else(|| invalid(message))?;
    if n >= count {
        return Err(invalid(message));
    }
    Ok(n)
}
fn validate_closure(atlas: &PreparedAtlas) -> Result<bool, JobError> {
    let mut basis = false;
    for page in &atlas.pages {
        let magic: &[u8] = match page.mime {
            "image/png" => b"\x89PNG\r\n\x1a\n",
            "image/ktx2" => {
                basis = true;
                b"\xabKTX 20\xbb\r\n\x1a\n"
            }
            _ => return Err(invalid("prepared page MIME is not PNG or KTX2")),
        };
        if !page.bytes.starts_with(magic) {
            return Err(invalid("prepared page MIME/signature mismatch"));
        }
    }
    for texture in &atlas.textures {
        let source = if let Some(source) = texture
            .get("extensions")
            .and_then(|v| v.get("KHR_texture_basisu"))
            .and_then(|v| v.get("source"))
        {
            let n = index(
                source,
                atlas.pages.len(),
                "prepared basis texture image missing",
            )?;
            if atlas.pages[n].mime != "image/ktx2" {
                return Err(invalid("basis texture refers to non-KTX2 page"));
            }
            n
        } else {
            let n = index(
                &texture["source"],
                atlas.pages.len(),
                "prepared texture image missing",
            )?;
            if atlas.pages[n].mime != "image/png" {
                return Err(invalid("KTX2 texture lacks KHR_texture_basisu"));
            }
            n
        };
        let _ = source;
        if let Some(s) = texture.get("sampler") {
            index(s, atlas.samplers.len(), "prepared texture sampler missing")?;
        }
        if texture
            .get("extensions")
            .and_then(|v| v.as_object())
            .is_some_and(|o| o.keys().any(|k| k != "KHR_texture_basisu"))
        {
            return Err(invalid("unresolved prepared texture extension"));
        }
    }
    for primitive in &atlas.primitives {
        let count = primitive
            .triangles
            .len()
            .checked_mul(3)
            .ok_or_else(overflow)?;
        if count == 0
            || primitive.corner_depths.len() != primitive.triangles.len()
            || primitive.uvs.len() > 5
            || primitive
                .uvs
                .iter()
                .any(|uv| uv.len() != count || uv.iter().flatten().any(|v| !v.is_finite()))
        {
            return Err(invalid("prepared primitive corner/UV closure mismatch"));
        }
        if let Some(material) = &primitive.material {
            if !material.is_object() {
                return Err(invalid("prepared material is not object"));
            }
            for binding in [
                material.get("normalTexture"),
                material.get("occlusionTexture"),
                material.get("emissiveTexture"),
                material
                    .get("pbrMetallicRoughness")
                    .and_then(|p| p.get("baseColorTexture")),
                material
                    .get("pbrMetallicRoughness")
                    .and_then(|p| p.get("metallicRoughnessTexture")),
            ]
            .into_iter()
            .flatten()
            {
                index(
                    &binding["index"],
                    atlas.textures.len(),
                    "prepared material texture missing",
                )?;
                let uv = binding.get("texCoord").cloned().unwrap_or(json!(0));
                index(
                    &uv,
                    primitive.uvs.len(),
                    "prepared material TEXCOORD missing",
                )?;
            }
        }
        for entry in &primitive.feature_id_textures {
            let texture = &entry["texture"];
            let n = index(
                &texture["index"],
                atlas.textures.len(),
                "prepared ID texture missing",
            )?;
            let uv = texture.get("texCoord").cloned().unwrap_or(json!(0));
            index(&uv, primitive.uvs.len(), "prepared ID TEXCOORD missing")?;
            let channels = texture["channels"].as_array();
            if entry["featureCount"].as_u64().is_none_or(|n| n == 0)
                || channels.is_none_or(|c| {
                    c.is_empty()
                        || c.len() > 4
                        || c.iter().any(|v| v.as_u64().is_none_or(|v| v > 3))
                })
                || atlas.textures[n].get("extensions").is_some()
            {
                return Err(invalid(
                    "prepared ID texture entry is not a PNG feature ID texture",
                ));
            }
            let s = index(
                &atlas.textures[n]["sampler"],
                atlas.samplers.len(),
                "prepared ID texture sampler missing",
            )?;
            super::id_texture::check_sampler(&atlas.samplers[s])?;
        }
    }
    Ok(basis)
}

/// How one published glTF primitive stores POSITION: a quantization window (the shared
/// scale and, per axis, the lowest member offset `lo` and highest member code `hi`, both
/// in codes) or raw f32.
#[derive(Clone, Copy, Debug)]
enum Window {
    Quantized {
        scale: f32,
        lo: [i64; 3],
        hi: [i64; 3],
    },
    Float,
}
impl Window {
    /// The window holding both, when every rebased code still fits UNSIGNED_SHORT on one
    /// shared scale; `None` otherwise (H28: split, never re-quantize).
    fn join(self, other: Self) -> Option<Self> {
        match (self, other) {
            (Self::Float, Self::Float) => Some(Self::Float),
            (
                Self::Quantized { scale, lo, hi },
                Self::Quantized {
                    scale: other_scale,
                    lo: other_lo,
                    hi: other_hi,
                },
            ) if scale.to_bits() == other_scale.to_bits() => {
                let lo: [i64; 3] = std::array::from_fn(|a| lo[a].min(other_lo[a]));
                let hi: [i64; 3] = std::array::from_fn(|a| hi[a].max(other_hi[a]));
                (0..3)
                    .all(|a| hi[a] - lo[a] <= i64::from(u16::MAX))
                    .then_some(Self::Quantized { scale, lo, hi })
            }
            _ => None,
        }
    }
}
/// Prepared (source) primitives published as one glTF primitive, in atlas order.
#[derive(Debug)]
struct Group {
    order: Vec<usize>,
    window: Window,
}
fn canonical_zero(x: f32) -> f32 {
    if x == 0.0 {
        0.0
    } else {
        x
    }
}
/// A quantized node translation in codes: `t / s` is exact (power-of-two `s`) and must be
/// an integer, as the frozen lattice writes it.
fn exact_offset(translation: f32, scale: f32) -> Result<i64, JobError> {
    let q = f64::from(translation) / f64::from(scale);
    if !q.is_finite() || q.fract() != 0. || q.abs() > (1u64 << 40) as f64 {
        return Err(invalid(
            "leaf position translation is not an exact code offset",
        ));
    }
    Ok(q as i64)
}
/// Encode every prepared primitive on the frozen lattice exactly as before H28, then group
/// them into the fewest glTF primitives: equal material class and textures (canonical
/// remapped material), attribute flags, UV sets and extra feature-ID sets, first-fit into
/// quantization windows. Groups are ordered by their first member.
fn publish_groups(
    atlas: &PreparedAtlas,
    lattice: &LatticePlan,
    depth: u8,
) -> Result<(Vec<Group>, Vec<EncodedPositions>), JobError> {
    let mut positions = Vec::with_capacity(atlas.primitives.len());
    let mut keys = BTreeMap::<String, Vec<Group>>::new();
    for (index, primitive) in atlas.primitives.iter().enumerate() {
        let flags = primitive.triangles[0].flags;
        if flags & !31 != 0 || primitive.triangles.iter().any(|p| p.flags != flags) {
            return Err(invalid("prepared primitive mixes attribute presence"));
        }
        let points: Vec<_> = primitive
            .triangles
            .iter()
            .flat_map(|t| t.triangle.positions)
            .collect();
        let depths: Vec<_> = primitive.corner_depths.iter().flatten().copied().collect();
        let position = lattice.encode(&points, &depths, u32::from(depth))?;
        let window = match &position.storage {
            PositionStorage::Quantized {
                codes,
                translation,
                scale,
            } => {
                let lo = [
                    exact_offset(translation[0], *scale)?,
                    exact_offset(translation[1], *scale)?,
                    exact_offset(translation[2], *scale)?,
                ];
                let hi: [i64; 3] = std::array::from_fn(|a| {
                    lo[a] + i64::from(codes.iter().map(|c| c[a]).max().unwrap_or(0))
                });
                Window::Quantized {
                    scale: *scale,
                    lo,
                    hi,
                }
            }
            PositionStorage::Float { .. } => Window::Float,
        };
        positions.push(position);
        let key = json!([
            primitive
                .material
                .as_ref()
                .map(|m| super::super::batching::material_key(m, true)),
            flags,
            primitive.uvs.len(),
            primitive.feature_id_textures,
        ])
        .to_string();
        let windows = keys.entry(key).or_default();
        match windows
            .iter_mut()
            .find_map(|g| g.window.join(window).map(|w| (g, w)))
        {
            Some((group, joined)) => {
                group.window = joined;
                group.order.push(index);
            }
            None => windows.push(Group {
                order: vec![index],
                window,
            }),
        }
    }
    let mut groups: Vec<Group> = keys.into_values().flatten().collect();
    groups.sort_by_key(|g| g.order[0]);
    Ok((groups, positions))
}
/// Hand a merged primitive's renderer NORMALs back to its prepared primitives.
fn split_normals(
    order: &[usize],
    atlas: &PreparedAtlas,
    normals: Vec<[f32; 3]>,
    out: &mut [Vec<[f32; 3]>],
) -> Result<(), JobError> {
    let mut rest = normals.as_slice();
    for &i in order {
        let n = atlas.primitives[i].triangles.len() * 3;
        if rest.len() < n {
            return Err(invalid("merged leaf NORMAL count"));
        }
        out[i] = rest[..n].to_vec();
        rest = &rest[n..];
    }
    if !rest.is_empty() {
        return Err(invalid("merged leaf NORMAL count"));
    }
    Ok(())
}

pub(super) fn encode(
    atlas: &PreparedAtlas,
    lattice: &LatticePlan,
    depth: u8,
    max_bytes: u64,
) -> Result<EncodedLeaf, WriteError> {
    let limit = max_bytes.min(TILE_LIMIT);
    if limit == 0 {
        return Err(JobError::new(
            JobErrorKind::InvalidRequest,
            "leaf byte budget must be positive",
        )
        .into());
    }
    if atlas.primitives.is_empty() {
        return Err(invalid("prepared leaf has no primitives").into());
    }
    let basis = validate_closure(atlas)?;
    let total = atlas.primitives.iter().try_fold(0usize, |sum, p| {
        sum.checked_add(p.triangles.len()).ok_or_else(overflow)
    })?;
    if total > 1 << 24 {
        return Err(invalid("leaf feature ID exceeds exact float integer domain").into());
    }
    // Both property tables have at most `total` rows (one per triangle, one per present
    // primitive). The tree's leaf budget already bounds `total`; a wider table is a broken
    // plan, never a silent consumer-incompatible leaf.
    if total as u64 > CONSUMER_TABLE_ROWS {
        return Err(invalid("leaf property table exceeds the consumer row limit").into());
    }
    // Every triangle's two UINT8 provenance columns stay uncompressed: this lower
    // bound rejects impossible tiles BEFORE allocating geometry or compression.
    let page_bytes = atlas.pages.iter().try_fold(0u64, |sum, p| {
        sum.checked_add(p.bytes.len() as u64).ok_or_else(overflow)
    })?;
    if (total as u64)
        .checked_mul(2)
        .and_then(|n| n.checked_add(page_bytes))
        .ok_or_else(overflow)?
        > limit
    {
        return Err(WriteError::Budget(budget()));
    }
    let mut primitive_rows: BTreeMap<PrimitiveKey, &FullTriangle> = BTreeMap::new();
    let mut identities = BTreeSet::new();
    for p in &atlas.primitives {
        for row in &p.triangles {
            let source = row.triangle.source;
            if !identities.insert((source.node, source.mesh, source.primitive, source.triangle)) {
                return Err(invalid("duplicate source triangle in prepared leaf").into());
            }
            if let Some(previous) = primitive_rows.insert(key(row), row) {
                if previous.expanded_ordinal != row.expanded_ordinal
                    || previous.material != row.material
                {
                    return Err(
                        invalid("source primitive expanded identity/material mismatch").into(),
                    );
                }
            }
        }
    }
    let primitive_ids: BTreeMap<_, _> = primitive_rows
        .keys()
        .enumerate()
        .map(|(i, k)| (*k, i))
        .collect();
    let (groups, mut positions) = publish_groups(atlas, lattice, depth)?;
    let mut wire = Wire::new();
    let mut nodes = Vec::new();
    let mut meshes = Vec::new();
    let mut materials = Vec::new();
    let mut bounds = Bounds::EMPTY;
    let mut geometry_error = 0.0_f64;
    let mut rank = 0usize;
    let mut quantized = false;
    let mut renderer_normals = vec![Vec::new(); atlas.primitives.len()];
    let mut rows: Vec<&FullTriangle> = Vec::with_capacity(total);
    let octahedral = lattice.profile().leaf_normal_octahedral_bits();
    for group in &groups {
        let first = &atlas.primitives[group.order[0]];
        let flags = first.triangles[0].flags;
        let triangles: Vec<&FullTriangle> = group
            .order
            .iter()
            .flat_map(|&i| atlas.primitives[i].triangles.iter())
            .collect();
        let count = triangles.len().checked_mul(3).ok_or_else(overflow)?;
        let mut node = json!({"mesh":meshes.len()});
        let pos = match group.window {
            Window::Quantized { scale, lo, .. } => {
                quantized = true;
                // `lo` is one member's aligned offset: |lo| <= 2^38 and a multiple of 2^15,
                // so `lo as f32` and its power-of-two product are exact (geometry.rs).
                let translation: [f32; 3] = std::array::from_fn(|a| lo[a] as f32 * scale);
                let mut bytes = allocate(count.checked_mul(8).ok_or_else(overflow)?)?;
                let mut min = [u16::MAX; 3];
                let mut max = [0u16; 3];
                for &i in &group.order {
                    let encoded = &positions[i];
                    let PositionStorage::Quantized {
                        codes,
                        translation: own,
                        scale: own_scale,
                    } = &encoded.storage
                    else {
                        return Err(invalid("merged leaf primitive mixes position storage").into());
                    };
                    if own_scale.to_bits() != scale.to_bits() {
                        return Err(invalid("merged leaf primitive mixes position scales").into());
                    }
                    let shift: [i64; 3] = [
                        exact_offset(own[0], scale)? - lo[0],
                        exact_offset(own[1], scale)? - lo[1],
                        exact_offset(own[2], scale)? - lo[2],
                    ];
                    for (code, decoded) in codes.iter().zip(&encoded.decoded) {
                        for a in 0..3 {
                            // Exact integer rebase onto the group offset; never re-quantized.
                            // Both client decode orders must give the member's own bits.
                            let value =
                                u16::try_from(i64::from(code[a]) + shift[a]).map_err(|_| {
                                    invalid("merged leaf position code exceeds UNSIGNED_SHORT")
                                })?;
                            let plain = canonical_zero(f32::from(value) * scale + translation[a]);
                            let fused =
                                canonical_zero(f32::from(value).mul_add(scale, translation[a]));
                            if plain.to_bits() != decoded[a].to_bits()
                                || fused.to_bits() != decoded[a].to_bits()
                            {
                                return Err(invalid(
                                    "merged leaf position decode differs from its source primitive",
                                )
                                .into());
                            }
                            bytes.extend_from_slice(&value.to_le_bytes());
                            min[a] = min[a].min(value);
                            max[a] = max[a].max(value);
                        }
                        bytes.extend_from_slice(&[0, 0]);
                    }
                }
                let id = wire.attribute(&bytes, count, 8, 5123, "VEC3")?;
                wire.accessors[id]["min"] = json!(min);
                wire.accessors[id]["max"] = json!(max);
                // Promote BEFORE Value/JSON construction; f32 shortest decimals
                // parsed as f64 can otherwise define a different real transform.
                node["translation"] = json!(translation.map(f64::from));
                node["scale"] = json!([f64::from(scale), f64::from(scale), f64::from(scale)]);
                id
            }
            Window::Float => {
                let mut values = Vec::with_capacity(count);
                let mut extent = Bounds::EMPTY;
                for &i in &group.order {
                    let PositionStorage::Float { positions: stored } = &positions[i].storage else {
                        return Err(invalid("merged leaf primitive mixes position storage").into());
                    };
                    values.extend(stored.iter().map(|p| p.to_vec()));
                    extent.union(&positions[i].bounds);
                }
                let id = wire.floats(&values, "VEC3")?;
                wire.accessors[id]["min"] = json!(extent.min.map(f64::from));
                wire.accessors[id]["max"] = json!(extent.max.map(f64::from));
                id
            }
        };
        for &i in &group.order {
            bounds.union(&positions[i].bounds);
            geometry_error = geometry_error.max(positions[i].correspondence_error);
        }
        let mut attributes = json!({"POSITION":pos});
        let corners = || triangles.iter().flat_map(|t| t.corners.iter());
        for (flag, name, shape) in [
            (NORMAL, "NORMAL", "VEC3"),
            (TANGENT, "TANGENT", "VEC4"),
            (COLOR, "COLOR_0", "VEC4"),
        ] {
            if flags & flag == 0 {
                continue;
            }
            if let (NORMAL, Some(bits)) = (flag, octahedral) {
                let normals: Vec<[f32; 3]> = corners().map(|c| c.normal).collect();
                let (id, decoded) = wire.octahedral(&normals, bits)?;
                attributes[name] = json!(id);
                split_normals(&group.order, atlas, decoded, &mut renderer_normals)?;
                quantized = true;
            } else {
                if flag == NORMAL {
                    let normals = corners().map(|c| c.normal).collect();
                    split_normals(&group.order, atlas, normals, &mut renderer_normals)?;
                }
                let values: Vec<Vec<f32>> = corners()
                    .map(|c| match flag {
                        NORMAL => c.normal.to_vec(),
                        TANGENT => c.tangent.to_vec(),
                        _ => c.color.to_vec(),
                    })
                    .collect();
                attributes[name] = json!(wire.floats(&values, shape)?);
            }
        }
        for set in 0..first.uvs.len() {
            let values: Vec<_> = group
                .order
                .iter()
                .flat_map(|&i| atlas.primitives[i].uvs[set].iter())
                .map(|p| p.to_vec())
                .collect();
            attributes[format!("TEXCOORD_{set}")] = json!(wire.floats(&values, "VEC2")?);
        }
        for set in 0..2 {
            let mut bytes = allocate(count.checked_mul(4).ok_or_else(overflow)?)?;
            for (i, row) in triangles.iter().enumerate() {
                let id = if set == 0 {
                    primitive_ids[&key(row)]
                } else {
                    rank + i
                };
                for _ in 0..3 {
                    bytes.extend_from_slice(&(id as f32).to_le_bytes());
                }
            }
            attributes[format!("_FEATURE_ID_{set}")] =
                json!(wire.attribute(&bytes, count, 4, 5126, "SCALAR")?);
        }
        let mut feature_ids = vec![
            json!({"featureCount":primitive_ids.len(),"attribute":0,"propertyTable":0,"label":"source_primitive"}),
            json!({"featureCount":total,"attribute":1,"propertyTable":1,"label":"source_triangle"}),
        ];
        feature_ids.extend(first.feature_id_textures.iter().cloned());
        let mut output = json!({"attributes":attributes,"mode":4,"extensions":{"EXT_mesh_features":{"featureIds":feature_ids}}});
        if let Some(material) = &first.material {
            output["material"] = json!(materials.len());
            materials.push(material.clone());
        }
        meshes.push(json!({"primitives":[output]}));
        nodes.push(node);
        rank += triangles.len();
        rows.extend(triangles);
    }
    let decoded_positions: Vec<Vec<[f32; 3]>> = positions
        .iter_mut()
        .map(|p| std::mem::take(&mut p.decoded))
        .collect();
    let mut classes = Map::new();
    let mut tables = Vec::new();
    for (class, input) in [
        (
            "source_primitive",
            primitive_rows.values().copied().collect::<Vec<_>>(),
        ),
        ("source_triangle", rows),
    ] {
        let mut schema = Map::new();
        let mut properties = Map::new();
        let columns = if class == "source_primitive" { 5 } else { 11 };
        for column in 0..columns {
            let (name, component, width) = match column {
                0 => ("node_index", "UINT32", 4),
                1 => ("mesh_index", "UINT32", 4),
                2 => ("primitive_index", "UINT32", 4),
                3 => ("expanded_ordinal", "UINT64", 8),
                4 => ("material_index", "UINT32", 4),
                5 => ("triangle_index", "UINT64", 8),
                6 => ("vertex_0", "UINT64", 8),
                7 => ("vertex_1", "UINT64", 8),
                8 => ("vertex_2", "UINT64", 8),
                9 => ("attribute_flags", "UINT8", 1),
                _ => ("degenerate", "UINT8", 1),
            };
            let mut bytes = allocate(input.len().checked_mul(width).ok_or_else(overflow)?)?;
            for row in &input {
                let source = row.triangle.source;
                let value = match column {
                    0 => u64::from(source.node),
                    1 => u64::from(source.mesh),
                    2 => u64::from(source.primitive),
                    3 => row.expanded_ordinal,
                    4 => u64::from(row.material),
                    5 => source.triangle,
                    6..=8 => row.triangle.vertices[column - 6],
                    9 => u64::from(row.flags),
                    _ => u64::from(row.triangle.degenerate),
                };
                bytes.extend_from_slice(&value.to_le_bytes()[..width]);
            }
            properties.insert(name.into(), json!({"values":wire.column(&bytes,width)?}));
            schema.insert(
                name.into(),
                json!({"type":"SCALAR","componentType":component,"required":true}),
            );
        }
        classes.insert(class.into(), json!({"properties":schema}));
        tables.push(json!({"class":class,"count":input.len(),"properties":properties}));
    }
    let mut images = Vec::new();
    for page in &atlas.pages {
        images.push(json!({"bufferView":wire.view(&page.bytes,None)?,"mimeType":page.mime}));
    }
    let scene_nodes: Vec<_> = (0..nodes.len()).collect();
    let mut document = json!({"asset":{"version":"2.0"},"scene":0,"scenes":[{"nodes":scene_nodes}],"nodes":nodes,"meshes":meshes,
        "buffers":[{"byteLength":wire.binary.len()}],"bufferViews":wire.views,"accessors":wire.accessors,"materials":materials,
        "images":images,"textures":atlas.textures,"samplers":atlas.samplers,
        "extensionsUsed":["EXT_mesh_features","EXT_structural_metadata"],
        "extensions":{"EXT_structural_metadata":{"schema":{"id":"rusty_tiles_large_source_v1","classes":classes},"propertyTables":tables}}});
    for field in ["materials", "images", "textures", "samplers"] {
        if document[field].as_array().is_some_and(|a| a.is_empty()) {
            document
                .as_object_mut()
                .ok_or_else(|| invalid("leaf document object missing"))?
                .remove(field);
        }
    }
    if quantized {
        glb::add_extension(&mut document, "KHR_mesh_quantization", true)
            .map_err(|e| invalid(&e.to_string()))?;
    }
    if basis {
        glb::add_extension(&mut document, "KHR_texture_basisu", true)
            .map_err(|e| invalid(&e.to_string()))?;
    }
    let compressed = glb::meshopt_compress(
        &mut document,
        &wire.binary,
        MeshoptLayout {
            align: 8,
            explicit_filter: true,
        },
        |i, _| Ok(wire.streams[i]),
    )
    .map_err(|e| invalid(&e.to_string()))?;
    drop(wire.binary);
    let json_bytes = serde_json::to_vec(&document).map_err(|e| invalid(&e.to_string()))?;
    let decoded: Value =
        serde_json::from_slice(&json_bytes).map_err(|e| invalid(&e.to_string()))?;
    if decoded["nodes"] != document["nodes"] {
        return Err(invalid("leaf JSON real TRS roundtrip mismatch").into());
    }
    let final_len = glb::glb_len(json_bytes.len(), compressed.len()).map_err(|_| overflow())?;
    if final_len as u64 > limit {
        return Err(WriteError::Budget(budget()));
    }
    let output = glb::encode_glb(&document, &compressed).map_err(|e| invalid(&e.to_string()))?;
    if output.len() != final_len {
        return Err(invalid("leaf GLB framing length mismatch").into());
    }
    Ok(EncodedLeaf {
        glb: output,
        bounds,
        geometry_error,
        positions: decoded_positions,
        normals: renderer_normals,
        primitives: groups.len(),
        shared_triangles: 0,
    })
}

#[cfg(test)]
mod tests {
    use super::super::super::{
        attributes::{CornerAttributes, UV0, UV1},
        config::Profile,
        records::{Provenance, TriangleRecord},
    };
    use super::super::prepared::{PreparedPage, PreparedPrimitive};
    use super::*;
    fn row(ordinal: u64, x: f32, degenerate: bool) -> FullTriangle {
        let first = [x, -999_999.5, -999_999.0];
        FullTriangle {
            triangle: TriangleRecord {
                key: 0,
                source: Provenance {
                    node: 3,
                    mesh: 2,
                    primitive: 1,
                    triangle: ordinal,
                },
                vertices: [(1 << 60) + 3, (1 << 60) + 5, (1 << 60) + 7],
                positions: [
                    first,
                    if degenerate {
                        first
                    } else {
                        [x + 0.25, -999_999.5, -999_999.0]
                    },
                    [x, -999_999.0, -999_999.0],
                ],
                bytes: 272,
                degenerate,
            },
            expanded_ordinal: (1 << 54) + 9,
            material: 7,
            flags: NORMAL | TANGENT | UV0 | UV1 | COLOR,
            corners: [CornerAttributes {
                normal: [0., 0., 1.],
                tangent: [1., 0., 0., -1.],
                texcoords: [[0., 0.], [1., 1.]],
                color: [0.3, 0.5, 0.7, 0.9],
            }; 3],
        }
    }
    fn atlas() -> PreparedAtlas {
        let mut png = std::io::Cursor::new(Vec::new());
        image::DynamicImage::ImageRgba8(image::RgbaImage::from_pixel(
            1,
            1,
            image::Rgba([128, 128, 255, 255]),
        ))
        .write_to(&mut png, image::ImageFormat::Png)
        .unwrap();
        let material = json!({"normalTexture":{"index":0,"texCoord":2,"scale":-2.0},"occlusionTexture":{"index":0,"texCoord":3,"strength":0.5},
            "emissiveTexture":{"index":0,"texCoord":4},"emissiveFactor":[0.1,0.2,0.3],
            "pbrMetallicRoughness":{"baseColorTexture":{"index":0,"texCoord":0},"metallicRoughnessTexture":{"index":0,"texCoord":1},"roughnessFactor":0.75}});
        PreparedAtlas {
            primitives: vec![PreparedPrimitive {
                triangles: vec![
                    row((1 << 53) + 7, -999_999.75, false),
                    row((1 << 53) + 9, -999_990.25, true),
                ],
                corner_depths: vec![[1; 3]; 2],
                uvs: (0..5)
                    .map(|set| vec![[set as f32 / 8.0, 0.25]; 6])
                    .collect(),
                material: Some(material),
                feature_id_textures: Vec::new(),
            }],
            pages: vec![PreparedPage {
                bytes: png.into_inner(),
                mime: "image/png",
            }],
            textures: vec![json!({"source":0,"sampler":0})],
            samplers: vec![json!({"magFilter":9729,"minFilter":9729,"wrapS":33071,"wrapT":33071})],
            preencode: Vec::new(),
        }
    }
    fn plan(profile: Profile) -> LatticePlan {
        LatticePlan::new(
            profile,
            Bounds {
                min: [-1_000_000.; 3],
                max: [1_000_000.; 3],
            },
            &[2_000_000., 20.],
        )
        .unwrap()
    }
    fn parse(bytes: &[u8]) -> (Value, &[u8]) {
        assert_eq!(&bytes[..4], b"glTF");
        assert_eq!(u32::from_le_bytes(bytes[4..8].try_into().unwrap()), 2);
        assert_eq!(
            u32::from_le_bytes(bytes[8..12].try_into().unwrap()) as usize,
            bytes.len()
        );
        assert_eq!(&bytes[16..20], b"JSON");
        let length = u32::from_le_bytes(bytes[12..16].try_into().unwrap()) as usize;
        let doc = serde_json::from_slice(&bytes[20..20 + length]).unwrap();
        let start = 20 + length;
        assert_eq!(&bytes[start + 4..start + 8], b"BIN\0");
        let bin_length = u32::from_le_bytes(bytes[start..start + 4].try_into().unwrap()) as usize;
        assert_eq!(start + 8 + bin_length, bytes.len());
        (doc, &bytes[start + 8..])
    }
    fn view(doc: &Value, bin: &[u8], id: usize) -> Vec<u8> {
        let v = &doc["bufferViews"][id];
        if let Some(e) = v
            .get("extensions")
            .and_then(|x| x.get("EXT_meshopt_compression"))
        {
            use crate::content_integrity::meshopt::{self, Filter, Mode};
            assert_eq!(e["mode"], "ATTRIBUTES");
            let filter = Filter::parse(e["filter"].as_str().unwrap()).unwrap();
            let start = e["byteOffset"].as_u64().unwrap() as usize;
            let length = e["byteLength"].as_u64().unwrap() as usize;
            meshopt::check_header(&bin[start..start + length], Mode::Attributes).unwrap();
            meshopt::decode_filtered(
                &bin[start..start + length],
                e["count"].as_u64().unwrap() as usize,
                e["byteStride"].as_u64().unwrap() as usize,
                Mode::Attributes,
                filter,
            )
            .unwrap()
        } else {
            assert_eq!(v["buffer"], 0);
            let start = v["byteOffset"].as_u64().unwrap() as usize;
            let length = v["byteLength"].as_u64().unwrap() as usize;
            bin[start..start + length].to_vec()
        }
    }
    fn column(doc: &Value, bin: &[u8], table: usize, name: &str) -> Vec<u8> {
        view(
            doc,
            bin,
            doc["extensions"]["EXT_structural_metadata"]["propertyTables"][table]["properties"]
                [name]["values"]
                .as_u64()
                .unwrap() as usize,
        )
    }
    #[test]
    fn quantized_meshopt_real_trs_bounds_full_attributes_slots_and_wide_identity() {
        let input = atlas();
        let lattice = plan(Profile::Balanced);
        let leaf = encode(&input, &lattice, 1, TILE_LIMIT).unwrap();
        let (doc, bin) = parse(&leaf.glb);
        assert!(doc["extensionsRequired"]
            .as_array()
            .unwrap()
            .contains(&json!("EXT_meshopt_compression")));
        assert!(doc["extensionsRequired"]
            .as_array()
            .unwrap()
            .contains(&json!("KHR_mesh_quantization")));
        let primitive = &doc["meshes"][0]["primitives"][0];
        assert!(primitive.get("indices").is_none());
        let attributes = &primitive["attributes"];
        assert_eq!(attributes.as_object().unwrap().len(), 11);
        let id = attributes["POSITION"].as_u64().unwrap() as usize;
        let accessor = &doc["accessors"][id];
        assert_eq!(accessor["componentType"], 5123);
        assert_eq!(accessor["normalized"], false);
        assert_eq!(accessor["count"], 6);
        let decoded = view(&doc, bin, accessor["bufferView"].as_u64().unwrap() as usize);
        let source: Vec<_> = input.primitives[0]
            .triangles
            .iter()
            .flat_map(|r| r.triangle.positions)
            .collect();
        let expected = lattice.encode(&source, &[1; 6], 1).unwrap();
        for (i, bytes) in decoded.as_chunks::<8>().0.iter().enumerate() {
            for a in 0..3 {
                let code = u16::from_le_bytes(bytes[a * 2..a * 2 + 2].try_into().unwrap());
                let scale = doc["nodes"][0]["scale"][a].as_f64().unwrap();
                let translation = doc["nodes"][0]["translation"][a].as_f64().unwrap();
                let real = f64::from(code) * scale + translation;
                assert_eq!(real, f64::from(expected.decoded[i][a]));
                assert!(
                    real >= f64::from(leaf.bounds.min[a]) && real <= f64::from(leaf.bounds.max[a])
                );
            }
        }
        assert_eq!(leaf.geometry_error, expected.correspondence_error);
        // Balanced NORMAL: 12-bit octahedral codes, decoded as normalized SHORT VEC3.
        let normal = &doc["accessors"][attributes["NORMAL"].as_u64().unwrap() as usize];
        assert_eq!(
            (
                &normal["componentType"],
                &normal["normalized"],
                &normal["type"]
            ),
            (&json!(5122), &json!(true), &json!("VEC3"))
        );
        let normal_view = &doc["bufferViews"][normal["bufferView"].as_u64().unwrap() as usize];
        assert_eq!(normal_view["byteStride"], 8);
        assert_eq!(
            normal_view["extensions"]["EXT_meshopt_compression"]["filter"],
            "OCTAHEDRAL"
        );
        let decoded = view(&doc, bin, normal["bufferView"].as_u64().unwrap() as usize);
        let shorts: Vec<_> = decoded
            .as_chunks::<2>()
            .0
            .iter()
            .map(|b| i16::from_le_bytes(*b))
            .collect();
        assert_eq!(shorts, [0, 0, 32767, 0].repeat(6));
        for (name, truth) in [
            ("TANGENT", vec![1., 0., 0., -1.]),
            ("COLOR_0", vec![0.3, 0.5, 0.7, 0.9]),
        ] {
            let accessor = &doc["accessors"][attributes[name].as_u64().unwrap() as usize];
            let bytes = view(&doc, bin, accessor["bufferView"].as_u64().unwrap() as usize);
            let floats: Vec<_> = bytes
                .as_chunks::<4>()
                .0
                .iter()
                .map(|b| f32::from_le_bytes(*b))
                .collect();
            assert_eq!(floats, truth.repeat(6));
        }
        for set in 0..5 {
            let accessor =
                &doc["accessors"][attributes[format!("TEXCOORD_{set}")].as_u64().unwrap() as usize];
            assert_eq!(accessor["count"], 6);
        }
        assert_eq!(
            doc["materials"][0],
            input.primitives[0].material.clone().unwrap()
        );
        assert_eq!(doc["materials"][0]["normalTexture"]["scale"], -2.0);
        let triangle_bytes = column(&doc, bin, 1, "triangle_index");
        assert_eq!(
            u64::from_le_bytes(triangle_bytes[..8].try_into().unwrap()),
            (1 << 53) + 7
        );
        let expanded = column(&doc, bin, 1, "expanded_ordinal");
        assert_eq!(
            u64::from_le_bytes(expanded[..8].try_into().unwrap()),
            (1 << 54) + 9
        );
        let vertices = column(&doc, bin, 1, "vertex_2");
        assert_eq!(
            u64::from_le_bytes(vertices[..8].try_into().unwrap()),
            (1 << 60) + 7
        );
        assert_eq!(column(&doc, bin, 1, "degenerate"), [0, 1]);
        let fid = &doc["accessors"][attributes["_FEATURE_ID_1"].as_u64().unwrap() as usize];
        let bytes = view(&doc, bin, fid["bufferView"].as_u64().unwrap() as usize);
        let values: Vec<_> = bytes
            .as_chunks::<4>()
            .0
            .iter()
            .map(|b| f32::from_le_bytes(*b))
            .collect();
        assert_eq!(values, [0., 0., 0., 1., 1., 1.]);
        let page = view(
            &doc,
            bin,
            doc["images"][0]["bufferView"].as_u64().unwrap() as usize,
        );
        assert_eq!(page, input.pages[0].bytes);
    }
    /// Every provenance column decodes to the exact source values in every profile: 4- and
    /// 8-byte columns are tightly packed meshopt ATTRIBUTES streams (value width as stride,
    /// no parent `byteStride`/`target`), UINT8 columns stay raw, and the schema is unchanged.
    #[test]
    fn provenance_columns_are_lossless_meshopt_streams_with_the_unchanged_schema() {
        let mut input = atlas();
        let triangles = &mut input.primitives[0].triangles;
        for (i, t) in triangles.iter_mut().enumerate() {
            t.triangle.vertices = [u64::MAX - i as u64, (1 << 33) + i as u64, 7 * i as u64];
        }
        for profile in [Profile::Balanced, Profile::Lossless] {
            let leaf = encode(&input, &plan(profile), 1, TILE_LIMIT).unwrap();
            let (doc, bin) = parse(&leaf.glb);
            let metadata = &doc["extensions"]["EXT_structural_metadata"];
            assert_eq!(metadata["schema"]["id"], "rusty_tiles_large_source_v1");
            let rows = &input.primitives[0].triangles;
            let class = &metadata["schema"]["classes"]["source_triangle"]["properties"];
            for (name, width, expected) in [
                (
                    "node_index",
                    4,
                    rows.iter()
                        .map(|r| u64::from(r.triangle.source.node))
                        .collect::<Vec<_>>(),
                ),
                (
                    "mesh_index",
                    4,
                    rows.iter()
                        .map(|r| u64::from(r.triangle.source.mesh))
                        .collect(),
                ),
                (
                    "primitive_index",
                    4,
                    rows.iter()
                        .map(|r| u64::from(r.triangle.source.primitive))
                        .collect(),
                ),
                (
                    "expanded_ordinal",
                    8,
                    rows.iter().map(|r| r.expanded_ordinal).collect(),
                ),
                (
                    "material_index",
                    4,
                    rows.iter().map(|r| u64::from(r.material)).collect(),
                ),
                (
                    "triangle_index",
                    8,
                    rows.iter().map(|r| r.triangle.source.triangle).collect(),
                ),
                (
                    "vertex_0",
                    8,
                    rows.iter().map(|r| r.triangle.vertices[0]).collect(),
                ),
                (
                    "vertex_1",
                    8,
                    rows.iter().map(|r| r.triangle.vertices[1]).collect(),
                ),
                (
                    "vertex_2",
                    8,
                    rows.iter().map(|r| r.triangle.vertices[2]).collect(),
                ),
                (
                    "attribute_flags",
                    1,
                    rows.iter().map(|r| u64::from(r.flags)).collect(),
                ),
                (
                    "degenerate",
                    1,
                    rows.iter()
                        .map(|r| u64::from(r.triangle.degenerate))
                        .collect(),
                ),
            ] {
                let component = match width {
                    1 => "UINT8",
                    4 => "UINT32",
                    _ => "UINT64",
                };
                assert_eq!(
                    class[name],
                    json!({"type":"SCALAR","componentType":component,"required":true})
                );
                let index = metadata["propertyTables"][1]["properties"][name]["values"]
                    .as_u64()
                    .unwrap() as usize;
                let parent = &doc["bufferViews"][index];
                assert!(parent.get("byteStride").is_none() && parent.get("target").is_none());
                assert_eq!(parent["byteLength"], rows.len() * width);
                let meshopt = parent
                    .get("extensions")
                    .map(|e| &e["EXT_meshopt_compression"]);
                if width == 1 {
                    assert!(meshopt.is_none(), "{name}");
                } else {
                    let e = meshopt.unwrap();
                    assert_eq!(
                        (&e["mode"], &e["filter"], &e["byteStride"], &e["count"]),
                        (
                            &json!("ATTRIBUTES"),
                            &json!("NONE"),
                            &json!(width),
                            &json!(rows.len())
                        ),
                        "{name}"
                    );
                }
                let decoded: Vec<u64> = column(&doc, bin, 1, name)
                    .chunks_exact(width)
                    .map(|c| {
                        let mut b = [0; 8];
                        b[..width].copy_from_slice(c);
                        u64::from_le_bytes(b)
                    })
                    .collect();
                assert_eq!(decoded, expected, "{name}");
            }
            let normal = &doc["accessors"][doc["meshes"][0]["primitives"][0]["attributes"]["NORMAL"]
                .as_u64()
                .unwrap() as usize];
            let lossless = profile == Profile::Lossless;
            assert_eq!(normal["componentType"], if lossless { 5126 } else { 5122 });
            assert_eq!(normal["normalized"], !lossless);
            // Filter output is specified to one unit, so octahedral NORMAL declares no
            // bounds a decoder on another host could miss by that unit.
            assert!(normal.get("min").is_none() && normal.get("max").is_none());
        }
    }
    /// A non-finite prepared NORMAL is a typed state failure in every profile (the octahedral
    /// encoder refuses it as data), never a budget retry or a silently encoded vector.
    #[test]
    fn non_finite_normals_are_typed_failures() {
        assert!(matches!(
            glb::encode_octahedral(&[[0., f32::NAN, 1.]], 12),
            Err(crate::error::Error::Data(_))
        ));
        for profile in [Profile::Balanced, Profile::Lossless] {
            let mut input = atlas();
            input.primitives[0].triangles[1].corners[2].normal = [f32::INFINITY, 0., 0.];
            match encode(&input, &plan(profile), 1, TILE_LIMIT) {
                Err(WriteError::Failure(error)) => {
                    assert_eq!(error.kind(), JobErrorKind::InvalidState, "{profile:?}");
                    assert!(error.message().contains("finite"), "{error}");
                }
                other => panic!("{profile:?}: {other:?}"),
            }
        }
    }
    /// The 12-bit octahedral NORMAL bound over a dense sphere: decoded through the same
    /// filter clients use, against the f32 source direction.
    #[test]
    fn octahedral_normals_stay_within_the_profile_angle() {
        use crate::content_integrity::meshopt::{decode_filtered, Filter, Mode};
        let n = 200_000;
        let golden = std::f64::consts::PI * (3. - 5f64.sqrt());
        let normals: Vec<[f32; 3]> = (0..n)
            .map(|i| {
                let z = 1. - 2. * (i as f64 + 0.5) / n as f64;
                let r = (1. - z * z).sqrt();
                let a = golden * i as f64;
                [(r * a.cos()) as f32, (r * a.sin()) as f32, z as f32]
            })
            .chain([[0., 0., 1.], [0., 0., -1.], [1., 0., 0.], [0., -1., 0.]])
            .collect();
        let raw = glb::encode_octahedral(&normals, 12).unwrap();
        let mut binary = Vec::new();
        let mut doc = json!({"bufferViews":[{"buffer":0,"byteOffset":0,"byteLength":raw.len(),"byteStride":8}]});
        let count = normals.len();
        let out = glb::meshopt_compress(
            &mut doc,
            &raw,
            MeshoptLayout {
                align: 8,
                explicit_filter: true,
            },
            |_, _| Ok(Some(MeshoptStream::Octahedral { count, stride: 8 })),
        )
        .unwrap();
        binary.extend_from_slice(&out);
        let decoded =
            decode_filtered(&binary, count, 8, Mode::Attributes, Filter::Octahedral).unwrap();
        let (mut max, mut sum) = (0f64, 0f64);
        for (source, element) in normals.iter().zip(decoded.as_chunks::<8>().0) {
            let q: [f64; 3] = std::array::from_fn(|a| {
                f64::from(i16::from_le_bytes([element[2 * a], element[2 * a + 1]])) / 32767.
            });
            let s = source.map(f64::from);
            let dot = (0..3).map(|a| q[a] * s[a]).sum::<f64>()
                / ((q.iter().map(|v| v * v).sum::<f64>()).sqrt()
                    * (s.iter().map(|v| v * v).sum::<f64>()).sqrt());
            let angle = dot.clamp(-1., 1.).acos().to_degrees();
            max = max.max(angle);
            sum += angle;
        }
        let mean = sum / count as f64;
        assert!(max < 0.07 && mean < 0.03, "oct12 max {max} mean {mean}");
    }
    #[test]
    fn id_texture_sets_follow_the_source_sets_and_need_nearest() {
        use super::super::id_texture::{bake, IdSet, Request};
        // Five sets: the leaf carries 2 + 5 featureIds; nothing caps the count.
        let sets = [IdSet { bytes: 1 }; 5];
        let ids: Vec<u32> = (0..16 * 5).map(|i| (i % 7) as u32).collect();
        let support = vec![1; 16];
        let id = bake(&Request {
            width: 4,
            height: 4,
            sets: &sets,
            ids: &ids,
            support: &support,
            memory_limit: 1 << 30,
        })
        .unwrap();
        let mut input = atlas();
        let refs = id.append_level(&mut input, 0).unwrap();
        input.primitives[0].feature_id_textures = refs
            .iter()
            .enumerate()
            .map(|(k, r)| r.feature_id(0, &format!("annotation_{k}")).unwrap())
            .collect();
        let leaf = encode(&input, &plan(Profile::Lossless), 1, TILE_LIMIT).unwrap();
        let (doc, _) = parse(&leaf.glb);
        let ids =
            &doc["meshes"][0]["primitives"][0]["extensions"]["EXT_mesh_features"]["featureIds"];
        assert_eq!(ids.as_array().unwrap().len(), 7);
        for (k, r) in refs.iter().enumerate() {
            let entry = &ids[2 + k];
            assert_eq!(entry["nullFeatureId"], 255);
            let texture = &doc["textures"][entry["texture"]["index"].as_u64().unwrap() as usize];
            let sampler = &doc["samplers"][texture["sampler"].as_u64().unwrap() as usize];
            assert_eq!(
                (sampler["magFilter"].as_u64(), sampler["minFilter"].as_u64()),
                (Some(9728), Some(9728))
            );
            assert_eq!(
                doc["images"][texture["source"].as_u64().unwrap() as usize]["mimeType"],
                "image/png"
            );
            assert_eq!(entry["texture"]["channels"], json!(r.channels));
        }
        // Mutant: a LINEAR sampler on an ID texture is refused by the writer.
        let s = input.textures[refs[0].texture]["sampler"].as_u64().unwrap() as usize;
        input.samplers[s]["magFilter"] = json!(9729);
        assert!(encode(&input, &plan(Profile::Lossless), 1, TILE_LIMIT).is_err());
    }
    #[test]
    fn float_profiles_keep_position_bits_and_each_node_owns_its_trs() {
        for profile in [Profile::Quality, Profile::Lossless] {
            let mut input = atlas();
            input.primitives.push(PreparedPrimitive {
                triangles: vec![row((1 << 53) + 11, -999_980.0, false)],
                corner_depths: vec![[1; 3]],
                uvs: vec![],
                material: None,
                feature_id_textures: Vec::new(),
            });
            let leaf = encode(&input, &plan(profile), 1, TILE_LIMIT).unwrap();
            let (doc, bin) = parse(&leaf.glb);
            assert_eq!(doc["nodes"].as_array().unwrap().len(), 2);
            for (i, p) in input.primitives.iter().enumerate() {
                assert!(doc["nodes"][i].get("translation").is_none());
                assert!(doc["nodes"][i].get("scale").is_none());
                let attr = &doc["meshes"][i]["primitives"][0]["attributes"];
                let accessor = &doc["accessors"][attr["POSITION"].as_u64().unwrap() as usize];
                assert_eq!(accessor["componentType"], 5126);
                let bytes = view(&doc, bin, accessor["bufferView"].as_u64().unwrap() as usize);
                let positions: Vec<_> = bytes
                    .as_chunks::<4>()
                    .0
                    .iter()
                    .map(|b| u32::from_le_bytes(*b))
                    .collect();
                for set in 0..2 {
                    let features = &doc["meshes"][i]["primitives"][0]["extensions"]
                        ["EXT_mesh_features"]["featureIds"][set];
                    let feature_count = features["featureCount"].as_u64().unwrap() as usize;
                    let table_count = doc["extensions"]["EXT_structural_metadata"]["propertyTables"]
                        [set]["count"]
                        .as_u64()
                        .unwrap() as usize;
                    assert_eq!(feature_count, table_count);
                    let id_attr = &doc["accessors"]
                        [attr[format!("_FEATURE_ID_{set}")].as_u64().unwrap() as usize];
                    let id_bytes =
                        view(&doc, bin, id_attr["bufferView"].as_u64().unwrap() as usize);
                    for bytes in id_bytes.as_chunks::<4>().0 {
                        let id = f32::from_le_bytes(*bytes);
                        assert_eq!(id.fract(), 0.0);
                        assert!(id >= 0.0 && (id as usize) < feature_count);
                        if i == 1 && set == 1 {
                            assert_eq!(id, 2.0);
                        }
                    }
                }
                let expected: Vec<_> = p
                    .triangles
                    .iter()
                    .flat_map(|r| r.triangle.positions)
                    .flat_map(|v| v.map(f32::to_bits))
                    .collect();
                assert_eq!(positions, expected);
            }
            assert_eq!(leaf.geometry_error, 0.0);
        }
    }
    /// A prepared primitive of source primitive `primitive`, `count` triangles from `x`,
    /// with the fixture atlas material and five UV sets.
    fn source_primitive(primitive: u32, count: usize, x: f32) -> PreparedPrimitive {
        let triangles: Vec<FullTriangle> = (0..count)
            .map(|i| {
                let mut t = row(
                    100 * u64::from(primitive) + i as u64,
                    x + i as f32 * 0.5,
                    false,
                );
                t.triangle.source.primitive = primitive;
                t.material = primitive;
                t
            })
            .collect();
        PreparedPrimitive {
            corner_depths: vec![[1; 3]; count],
            uvs: (0..5)
                .map(|set| vec![[set as f32 / 8.0, 0.25]; 3 * count])
                .collect(),
            material: atlas().primitives[0].material.clone(),
            feature_id_textures: Vec::new(),
            triangles,
        }
    }
    fn f32s(doc: &Value, bin: &[u8], accessor: &Value) -> Vec<f32> {
        view(doc, bin, accessor["bufferView"].as_u64().unwrap() as usize)
            .as_chunks::<4>()
            .0
            .iter()
            .map(|b| f32::from_le_bytes(*b))
            .collect()
    }
    /// Every published triangle, in published order: decoded corner bits (client order
    /// `u * s + t` in f32, or the stored f32), and its `source_primitive` and
    /// `source_triangle` rows as (node, mesh, primitive) and (primitive, triangle).
    #[allow(clippy::type_complexity)]
    fn published(glb: &[u8]) -> Vec<([[u32; 3]; 3], (u64, u64, u64), (u64, u64))> {
        let (doc, bin) = parse(glb);
        let column = |table: usize, name: &str| -> Vec<u64> {
            let width = match name {
                "node_index" | "mesh_index" | "primitive_index" => 4,
                _ => 8,
            };
            column(&doc, bin, table, name)
                .chunks_exact(width)
                .map(|c| {
                    let mut b = [0; 8];
                    b[..width].copy_from_slice(c);
                    u64::from_le_bytes(b)
                })
                .collect()
        };
        let (nodes, meshes, primitives) = (
            column(0, "node_index"),
            column(0, "mesh_index"),
            column(0, "primitive_index"),
        );
        let (triangle_primitives, triangles) =
            (column(1, "primitive_index"), column(1, "triangle_index"));
        let mut out = Vec::new();
        for node in doc["nodes"].as_array().unwrap() {
            let mesh = &doc["meshes"][node["mesh"].as_u64().unwrap() as usize];
            assert_eq!(mesh["primitives"].as_array().unwrap().len(), 1);
            let attributes = &mesh["primitives"][0]["attributes"];
            let accessor = &doc["accessors"][attributes["POSITION"].as_u64().unwrap() as usize];
            let bytes = view(&doc, bin, accessor["bufferView"].as_u64().unwrap() as usize);
            let corners: Vec<[u32; 3]> = if accessor["componentType"] == 5123 {
                let t = &node["translation"];
                let s = &node["scale"];
                bytes
                    .as_chunks::<8>()
                    .0
                    .iter()
                    .map(|e| {
                        std::array::from_fn(|a| {
                            let u = f32::from(u16::from_le_bytes([e[2 * a], e[2 * a + 1]]));
                            let p =
                                u * s[a].as_f64().unwrap() as f32 + t[a].as_f64().unwrap() as f32;
                            if p == 0. {
                                0
                            } else {
                                p.to_bits()
                            }
                        })
                    })
                    .collect()
            } else {
                assert!(node.get("translation").is_none() && node.get("scale").is_none());
                bytes
                    .as_chunks::<12>()
                    .0
                    .iter()
                    .map(|e| {
                        std::array::from_fn(|a| {
                            u32::from_le_bytes(e[4 * a..4 * a + 4].try_into().unwrap())
                        })
                    })
                    .collect()
            };
            let ids = |set: usize| {
                f32s(
                    &doc,
                    bin,
                    &doc["accessors"]
                        [attributes[format!("_FEATURE_ID_{set}")].as_u64().unwrap() as usize],
                )
            };
            let (primitive_ids, triangle_ids) = (ids(0), ids(1));
            for (k, corner) in corners.as_chunks::<3>().0.iter().enumerate() {
                let (p, t) = (primitive_ids[3 * k] as usize, triangle_ids[3 * k] as usize);
                assert_eq!(primitive_ids[3 * k + 2], p as f32);
                assert_eq!(triangle_ids[3 * k + 2], t as f32);
                out.push((
                    [corner[0], corner[1], corner[2]],
                    (nodes[p], meshes[p], primitives[p]),
                    (triangle_primitives[t], triangles[t]),
                ));
            }
        }
        out
    }
    /// What the pre-H28 writer published per triangle: each prepared primitive encoded on
    /// its own by the frozen lattice, in atlas order.
    #[allow(clippy::type_complexity)]
    fn per_primitive_reference(
        input: &PreparedAtlas,
        lattice: &LatticePlan,
    ) -> Vec<([[u32; 3]; 3], (u64, u64, u64), (u64, u64))> {
        let mut out = Vec::new();
        for p in &input.primitives {
            let points: Vec<_> = p
                .triangles
                .iter()
                .flat_map(|t| t.triangle.positions)
                .collect();
            let depths: Vec<_> = p.corner_depths.iter().flatten().copied().collect();
            let decoded = lattice.encode(&points, &depths, 1).unwrap().decoded;
            for (t, corners) in p.triangles.iter().zip(decoded.as_chunks::<3>().0) {
                let s = t.triangle.source;
                out.push((
                    std::array::from_fn(|c| corners[c].map(f32::to_bits)),
                    (u64::from(s.node), u64::from(s.mesh), u64::from(s.primitive)),
                    (u64::from(s.primitive), s.triangle),
                ));
            }
        }
        out
    }
    fn sorted<T: PartialOrd + Clone>(mut v: Vec<T>) -> Vec<T> {
        v.sort_by(|a, b| a.partial_cmp(b).unwrap());
        v
    }
    /// H28: source primitives of one material class, texture set, attribute set and
    /// position window publish as ONE glTF primitive. Every triangle keeps its decoded
    /// corner bits and its source_primitive/source_triangle rows, in every profile.
    #[test]
    fn same_class_primitives_merge_with_identical_positions_and_identity() {
        for profile in [
            Profile::Fast,
            Profile::Balanced,
            Profile::Quality,
            Profile::Lossless,
        ] {
            let mut input = atlas();
            input.primitives = vec![
                source_primitive(4, 3, -999_999.75),
                source_primitive(2, 2, -999_990.25),
                source_primitive(9, 1, -999_995.0),
            ];
            // Same class as written differently: an explicit default and integer numbers.
            let material = input.primitives[1].material.as_mut().unwrap();
            material["pbrMetallicRoughness"]["metallicFactor"] = json!(1);
            material["normalTexture"]["scale"] = json!(-2);
            let lattice = plan(profile);
            let leaf = encode(&input, &lattice, 1, TILE_LIMIT).unwrap();
            let (doc, _) = parse(&leaf.glb);
            assert_eq!(leaf.primitives, 1, "{profile:?}");
            assert_eq!(doc["meshes"].as_array().unwrap().len(), 1);
            assert_eq!(doc["nodes"].as_array().unwrap().len(), 1);
            assert_eq!(doc["materials"].as_array().unwrap().len(), 1);
            let reference = per_primitive_reference(&input, &lattice);
            // Atlas order is kept inside the merged primitive.
            assert_eq!(published(&leaf.glb), reference, "{profile:?}");
            // The witness hand-off stays per prepared primitive.
            for (p, decoded) in input.primitives.iter().zip(&leaf.positions) {
                let points: Vec<_> = p
                    .triangles
                    .iter()
                    .flat_map(|t| t.triangle.positions)
                    .collect();
                let expected = lattice.encode(&points, &vec![1; points.len()], 1).unwrap();
                assert_eq!(
                    decoded
                        .iter()
                        .map(|c| c.map(f32::to_bits))
                        .collect::<Vec<_>>(),
                    expected
                        .decoded
                        .iter()
                        .map(|c| c.map(f32::to_bits))
                        .collect::<Vec<_>>()
                );
            }
            assert_eq!(
                leaf.normals.iter().map(Vec::len).collect::<Vec<_>>(),
                [9, 6, 3]
            );
            let tables = &doc["extensions"]["EXT_structural_metadata"]["propertyTables"];
            assert_eq!(
                (tables[0]["count"].as_u64(), tables[1]["count"].as_u64()),
                (Some(3), Some(6))
            );
        }
    }
    /// H28 rebase across a lattice cell boundary: members on each side of an aligned cell
    /// edge (depth 1, scale 2^-8, 128 m cells; the edge is -7812 * 128 = -999936) carry
    /// different own translations, so merging them shifts codes by a nonzero amount. The
    /// published decode still equals the per-primitive reference bit for bit.
    #[test]
    fn merge_across_a_cell_boundary_rebases_a_nonzero_shift() {
        for profile in [Profile::Fast, Profile::Balanced] {
            let mut input = atlas();
            input.primitives = vec![
                source_primitive(4, 2, -999_936.5),
                source_primitive(2, 2, -999_935.5),
                source_primitive(9, 1, -999_999.75),
            ];
            let lattice = plan(profile);
            let own: Vec<[f32; 3]> = input
                .primitives
                .iter()
                .map(|p| {
                    let points: Vec<_> = p
                        .triangles
                        .iter()
                        .flat_map(|t| t.triangle.positions)
                        .collect();
                    let depths: Vec<_> = p.corner_depths.iter().flatten().copied().collect();
                    match lattice.encode(&points, &depths, 1).unwrap().storage {
                        PositionStorage::Quantized { translation, .. } => translation,
                        PositionStorage::Float { .. } => panic!("{profile:?} must quantize"),
                    }
                })
                .collect();
            assert!(own[0][0] < own[1][0], "{profile:?}: {own:?}");
            // One 128 m cell apart: a shift of 2^15 codes for the upper member.
            assert_eq!(own[1][0] - own[0][0], 128., "{profile:?}: {own:?}");
            let leaf = encode(&input, &lattice, 1, TILE_LIMIT).unwrap();
            let (doc, _) = parse(&leaf.glb);
            assert_eq!(leaf.primitives, 1, "{profile:?}");
            // The group offset is the lowest member offset, so the shifts are nonzero.
            let group = doc["nodes"][0]["translation"][0].as_f64().unwrap();
            assert_eq!(
                group,
                f64::from(own.iter().map(|t| t[0]).fold(f32::MAX, f32::min))
            );
            assert!(own.iter().any(|t| f64::from(t[0]) != group));
            assert_eq!(
                published(&leaf.glb),
                per_primitive_reference(&input, &lattice),
                "{profile:?}"
            );
        }
    }
    /// H28 splits: material class (a factor), texture identity (another page), attribute
    /// set (no COLOR_0) and an exact position window that UNSIGNED_SHORT cannot hold.
    /// Each split keeps every triangle's decoded corners and identity.
    #[test]
    fn class_texture_attribute_and_window_differences_split() {
        let base = || {
            let mut input = atlas();
            input.primitives = vec![
                source_primitive(1, 2, -999_999.75),
                source_primitive(2, 2, -999_998.25),
            ];
            input.pages.push(PreparedPage {
                bytes: input.pages[0].bytes.clone(),
                mime: "image/png",
            });
            input.textures.push(json!({"source":1,"sampler":0}));
            input
        };
        type Mutation = (&'static str, fn(&mut PreparedAtlas));
        let mutations: [Mutation; 4] = [
            ("factor", |a| {
                a.primitives[1].material.as_mut().unwrap()["pbrMetallicRoughness"]
                    ["roughnessFactor"] = json!(0.5)
            }),
            ("texture", |a| {
                a.primitives[1].material.as_mut().unwrap()["normalTexture"]["index"] = json!(1)
            }),
            ("attributes", |a| {
                for t in &mut a.primitives[1].triangles {
                    t.flags &= !COLOR;
                }
            }),
            ("window", |a| {
                // 1000 m away: each primitive fits its own window, the two do not.
                for (i, t) in a.primitives[1].triangles.iter_mut().enumerate() {
                    for p in &mut t.triangle.positions {
                        p[0] += 999.0 + i as f32;
                    }
                }
            }),
        ];
        let lattice = plan(Profile::Balanced);
        let merged = encode(&base(), &lattice, 1, TILE_LIMIT).unwrap();
        assert_eq!(merged.primitives, 1);
        for (name, mutate) in mutations {
            let mut input = base();
            mutate(&mut input);
            let leaf = encode(&input, &lattice, 1, TILE_LIMIT).unwrap();
            assert_eq!(leaf.primitives, 2, "{name}");
            let (doc, _) = parse(&leaf.glb);
            assert_eq!(doc["meshes"].as_array().unwrap().len(), 2, "{name}");
            assert_eq!(
                published(&leaf.glb),
                per_primitive_reference(&input, &lattice),
                "{name}"
            );
            if name == "window" {
                assert_ne!(
                    doc["nodes"][0]["translation"],
                    doc["nodes"][1]["translation"]
                );
            }
        }
        // Float profiles have no window: the far primitive still merges.
        let mut far = base();
        (mutations[3].1)(&mut far);
        let lossless = plan(Profile::Lossless);
        let leaf = encode(&far, &lossless, 1, TILE_LIMIT).unwrap();
        assert_eq!(leaf.primitives, 1);
        assert_eq!(
            sorted(published(&leaf.glb)),
            sorted(per_primitive_reference(&far, &lossless))
        );
    }
    #[test]
    fn quantization_windows_join_only_within_unsigned_short_on_one_scale() {
        let q = |scale: f32, lo: i64, hi: i64| Window::Quantized {
            scale,
            lo: [lo, 0, -(1 << 15)],
            hi: [hi, 0, -(1 << 15)],
        };
        let joined = q(0.25, 0, 40_000).join(q(0.25, 1 << 15, (1 << 15) + 32_767));
        assert!(matches!(
            joined,
            Some(Window::Quantized {
                lo: [0, 0, _],
                hi: [65_535, 0, _],
                ..
            })
        ));
        assert!(q(0.25, 0, 40_000)
            .join(q(0.25, 1 << 15, (1 << 15) + 32_768))
            .is_none());
        assert!(q(0.25, 0, 1).join(q(0.5, 0, 1)).is_none());
        assert!(q(0.25, 0, 1).join(Window::Float).is_none());
        assert!(matches!(
            Window::Float.join(Window::Float),
            Some(Window::Float)
        ));
        // Negative offsets rebase exactly too.
        assert!(q(0.25, -(1 << 16), -(1 << 16) + 100)
            .join(q(0.25, -(1 << 15), -(1 << 15) + 100))
            .is_some());
        assert_eq!(exact_offset(-8.0, 0.25).unwrap(), -32);
        assert!(exact_offset(0.1, 0.25).is_err());
    }
    #[test]
    fn complete_budget_and_missing_slot_uv_are_typed_failures() {
        let input = atlas();
        assert_eq!(
            encode(&input, &plan(Profile::Balanced), 1, 100)
                .unwrap_err()
                .kind(),
            JobErrorKind::Unsupported
        );
        let mut bad = atlas();
        bad.primitives[0].uvs.pop();
        assert_eq!(
            encode(&bad, &plan(Profile::Balanced), 1, TILE_LIMIT)
                .unwrap_err()
                .kind(),
            JobErrorKind::InvalidState
        );
        let mut duplicate = atlas();
        duplicate.primitives[0].triangles[1] = duplicate.primitives[0].triangles[0];
        assert_eq!(
            encode(&duplicate, &plan(Profile::Balanced), 1, TILE_LIMIT)
                .unwrap_err()
                .kind(),
            JobErrorKind::InvalidState
        );
    }
    /// H16 consumer cap: every profile's leaf budget is at most 8192 triangles, an 8192-row
    /// leaf encodes with both tables at the cap, and an 8193-triangle plan is refused as a
    /// broken plan (not a retryable budget) before any table is built.
    #[test]
    fn property_tables_stay_within_the_consumer_row_limit() {
        for profile in [
            Profile::Fast,
            Profile::Balanced,
            Profile::Quality,
            Profile::Lossless,
        ] {
            assert!(
                profile.leaf_triangles() <= CONSUMER_TABLE_ROWS,
                "{profile:?}"
            );
        }
        let sized = |count: usize| {
            let mut input = atlas();
            let primitive = &mut input.primitives[0];
            primitive.triangles = (0..count)
                .map(|i| row(i as u64, -999_999.75 + (i % 64) as f32 * 0.25, false))
                .collect();
            primitive.corner_depths = vec![[1; 3]; count];
            for uv in &mut primitive.uvs {
                *uv = vec![[0.25, 0.25]; 3 * count];
            }
            input
        };
        let at_cap = encode(&sized(8192), &plan(Profile::Balanced), 1, TILE_LIMIT).unwrap();
        let (doc, _) = parse(&at_cap.glb);
        let counts: Vec<u64> = doc["extensions"]["EXT_structural_metadata"]["propertyTables"]
            .as_array()
            .unwrap()
            .iter()
            .map(|table| table["count"].as_u64().unwrap())
            .collect();
        assert_eq!(counts, [1, 8192]);
        assert!(matches!(
            encode(&sized(8193), &plan(Profile::Balanced), 1, TILE_LIMIT),
            Err(WriteError::Failure(error)) if error.kind() == JobErrorKind::InvalidState
                && error.message().contains("consumer row limit")
        ));
    }
    #[test]
    fn only_actual_member_size_errors_are_retryable_budget() {
        let input = atlas();
        assert!(matches!(
            encode(&input, &plan(Profile::Balanced), 1, 100),
            Err(WriteError::Budget(_))
        ));
        let complete = encode(&input, &plan(Profile::Balanced), 1, TILE_LIMIT).unwrap();
        assert!(matches!(
            encode(
                &input,
                &plan(Profile::Balanced),
                1,
                complete.glb.len() as u64 - 1
            ),
            Err(WriteError::Budget(_))
        ));
        // A valid prepared closure with geometry outside the frozen source frame
        // fails directly, rather than coarsening textures to conceal that defect.
        let mut geometry = atlas();
        geometry.primitives[0].triangles[0].triangle.positions[0][0] = 1_000_001.0;
        assert!(matches!(
            encode(&geometry, &plan(Profile::Balanced), 1, TILE_LIMIT),
            Err(WriteError::Failure(_))
        ));
        let mut closure = atlas();
        closure.primitives[0].uvs.pop();
        assert!(matches!(
            encode(&closure, &plan(Profile::Balanced), 1, TILE_LIMIT),
            Err(WriteError::Failure(_))
        ));
        let unsupported: WriteError =
            JobError::new(JobErrorKind::Unsupported, "fidelity gate control").into();
        assert_eq!(unsupported.kind(), JobErrorKind::Unsupported);
        assert!(matches!(unsupported, WriteError::Failure(_)));
    }
}
