//! Accepted-leaf renderer witness for the H27 proxy bake (requests #3 and #4).
//!
//! The witness is taken from the accepted density attempt, after the leaf writer has
//! published it. It holds, in published primitive, face and corner order:
//! - the decoded POSITION of every corner, in the leaf's local metres. These are the
//!   writer's decoded lattice points: `u * s + t` in f32, which the writer proves equal
//!   for fused and unfused client decode. Float storage holds the published f32 values.
//! - the published NORMAL and TANGENT, when the primitive carries them. They are written
//!   as FLOAT through a lossless meshopt stream, so they are stored exactly as written.
//! - the source identity of each face: node, mesh, primitive, expanded instance ordinal,
//!   material, source triangle and the face's global sorted rank.
//! - each material slot binding exactly as published: page, TEXCOORD set, sampler and the
//!   literal `normalTexture.scale`. A face whose geometry or page-UV triangle has zero
//!   area does not rasterise for that binding. It is kept and marked in `zero_raster`,
//!   never dropped.
//! - the measured texel spacing: metres per page texel along the worst direction of the
//!   decoded-position / page-texel Jacobian, over every rasterising bound face, rounded up
//!   one ulp. Page sizes are the accepted post-layout, post-drop pages.
//!
//! The binary encoding is little-endian and versioned by its magic. Decoding refuses any
//! count that the remaining input cannot back, and charges its owned bytes against a
//! caller lease before it allocates.
use super::super::attributes::{FullTriangle, NORMAL, TANGENT};
use super::prepared::PreparedAtlas;
use crate::{gltf_source::source::TextureSlot, JobError, JobErrorKind};
use serde_json::Value;
use std::collections::HashMap;

/// Material slots in witness order (bit `i` of [`FaceWitness::zero_raster`]).
pub(in crate::mesh_large) const SLOTS: [TextureSlot; 5] = [
    TextureSlot::BaseColor,
    TextureSlot::MetallicRoughness,
    TextureSlot::Normal,
    TextureSlot::Occlusion,
    TextureSlot::Emissive,
];
/// Encoding version; byte 5 of the magic.
pub(in crate::mesh_large) const VERSION: u32 = 1;
const MAGIC: &[u8; 8] = b"H8WIT\x01\0\0";
const NONE_U32: u32 = u32::MAX;

#[derive(Clone, Debug, PartialEq)]
pub(in crate::mesh_large) struct LeafWitness {
    /// Leaf maximum of [`BindingWitness::texel_spacing`]; `None` when no bound face rasterises.
    pub texel_spacing: Option<f64>,
    pub primitives: Vec<PrimitiveWitness>,
}
#[derive(Clone, Debug, PartialEq)]
pub(in crate::mesh_large) struct PrimitiveWitness {
    pub node: u32,
    pub mesh: u32,
    pub primitive: u32,
    pub expanded_ordinal: u64,
    /// `u32::MAX` when the primitive has no material.
    pub material: u32,
    /// Published attribute presence (`attributes::NORMAL`, `TANGENT`, ...).
    pub flags: u8,
    pub bindings: Vec<BindingWitness>,
    pub faces: Vec<FaceWitness>,
    /// Three per face.
    pub positions: Vec<[f32; 3]>,
    /// Three per face when `flags` has NORMAL, else empty.
    pub normals: Vec<[f32; 3]>,
    /// Three per face when `flags` has TANGENT, else empty.
    pub tangents: Vec<[f32; 4]>,
}
#[derive(Clone, Copy, Debug, PartialEq)]
pub(in crate::mesh_large) struct FaceWitness {
    pub triangle: u64,
    /// Global sorted rank of the source triangle (the leaf covers `[first, end)`).
    pub rank: u64,
    /// Bit `i`: the face does not rasterise for its `SLOTS[i]` binding.
    pub zero_raster: u8,
}
#[derive(Clone, Copy, Debug, PartialEq)]
pub(in crate::mesh_large) struct BindingWitness {
    /// Index into [`SLOTS`].
    pub slot: u8,
    pub page: u32,
    pub texcoord: u32,
    pub sampler: Option<u32>,
    /// Literal `normalTexture.scale`, when the material states one.
    pub normal_scale: Option<f64>,
    pub texel_spacing: Option<f64>,
}

fn invalid(message: &str) -> JobError {
    JobError::new(JobErrorKind::InvalidState, message.to_owned())
}
fn resource(message: &str) -> JobError {
    JobError::new(JobErrorKind::ResourceLimit, message.to_owned())
}

fn binding(material: &Value, slot: TextureSlot) -> &Value {
    let name = match slot {
        TextureSlot::BaseColor => "baseColorTexture",
        TextureSlot::MetallicRoughness => "metallicRoughnessTexture",
        TextureSlot::Normal => "normalTexture",
        TextureSlot::Occlusion => "occlusionTexture",
        TextureSlot::Emissive => "emissiveTexture",
    };
    if matches!(
        slot,
        TextureSlot::BaseColor | TextureSlot::MetallicRoughness
    ) {
        &material["pbrMetallicRoughness"][name]
    } else {
        &material[name]
    }
}
fn u32_of(value: &Value, message: &str) -> Result<u32, JobError> {
    value
        .as_u64()
        .and_then(|v| u32::try_from(v).ok())
        .ok_or_else(|| invalid(message))
}

/// Metres per page texel along the worst direction: the largest singular value of
/// d(position)/d(texel), rounded up one ulp. `None` when the face or its page-UV
/// triangle has zero area.
pub(in crate::mesh_large) fn texel_spacing(p: [[f64; 3]; 3], uv: [[f64; 2]; 3]) -> Option<f64> {
    let sub = |a: [f64; 3], b: [f64; 3]| [a[0] - b[0], a[1] - b[1], a[2] - b[2]];
    let mul = |a: [f64; 3], s: f64| [a[0] * s, a[1] * s, a[2] * s];
    let dot = |a: [f64; 3], b: [f64; 3]| a[0] * b[0] + a[1] * b[1] + a[2] * b[2];
    let e = sub(p[1], p[0]);
    let f = sub(p[2], p[0]);
    let n = [
        e[1] * f[2] - e[2] * f[1],
        e[2] * f[0] - e[0] * f[2],
        e[0] * f[1] - e[1] * f[0],
    ];
    if dot(n, n) <= 0. {
        return None;
    }
    let a = [uv[1][0] - uv[0][0], uv[1][1] - uv[0][1]];
    let b = [uv[2][0] - uv[0][0], uv[2][1] - uv[0][1]];
    let det = a[0] * b[1] - a[1] * b[0];
    if det == 0. || !det.is_finite() {
        return None;
    }
    let x = mul(sub(mul(e, b[1]), mul(f, a[1])), 1. / det);
    let y = mul(sub(mul(f, a[0]), mul(e, b[0])), 1. / det);
    let (aa, ab, bb) = (dot(x, x), dot(x, y), dot(y, y));
    let sigma = ((aa + bb + ((aa - bb) * (aa - bb) + 4. * ab * ab).sqrt()) * 0.5).sqrt();
    (sigma.is_finite() && sigma > 0.).then_some(sigma.next_up())
}

/// The witness of an accepted leaf. `positions` are the writer's decoded positions per
/// primitive; `ranks` maps a source triangle identity to its global sorted rank.
pub(in crate::mesh_large) fn build(
    atlas: &PreparedAtlas,
    positions: &[Vec<[f32; 3]>],
    ranks: &HashMap<(u32, u32, u32, u64), u64>,
) -> Result<LeafWitness, JobError> {
    if positions.len() != atlas.primitives.len() {
        return Err(invalid("witness decoded primitive count"));
    }
    let mut leaf_spacing: Option<f64> = None;
    let mut primitives = Vec::new();
    primitives
        .try_reserve_exact(atlas.primitives.len())
        .map_err(|_| resource("witness primitives"))?;
    for (primitive, decoded) in atlas.primitives.iter().zip(positions) {
        let first: &FullTriangle = primitive
            .triangles
            .first()
            .ok_or_else(|| invalid("witness empty primitive"))?;
        let count = primitive.triangles.len();
        if decoded.len() != count * 3 || primitive.uvs.iter().any(|uv| uv.len() != count * 3) {
            return Err(invalid("witness corner count"));
        }
        let flags = first.flags;
        let mut bindings = Vec::new();
        let mut faces: Vec<FaceWitness> = Vec::new();
        faces
            .try_reserve_exact(count)
            .map_err(|_| resource("witness faces"))?;
        for row in &primitive.triangles {
            let s = row.triangle.source;
            let identity = (s.node, s.mesh, s.primitive, s.triangle);
            faces.push(FaceWitness {
                triangle: s.triangle,
                rank: *ranks
                    .get(&identity)
                    .ok_or_else(|| invalid("witness face outside the leaf's ranks"))?,
                zero_raster: 0,
            });
        }
        if let Some(material) = &primitive.material {
            for (index, &slot) in SLOTS.iter().enumerate() {
                let b = binding(material, slot);
                if b.is_null() {
                    continue;
                }
                let texture = atlas
                    .textures
                    .get(u32_of(&b["index"], "witness binding texture")? as usize)
                    .ok_or_else(|| invalid("witness binding texture"))?;
                let page = texture
                    .get("source")
                    .or_else(|| texture["extensions"]["KHR_texture_basisu"].get("source"))
                    .ok_or_else(|| invalid("witness binding page"))?;
                let page = u32_of(page, "witness binding page")?;
                let sampler = texture
                    .get("sampler")
                    .map(|s| u32_of(s, "witness binding sampler"))
                    .transpose()?;
                let texcoord = b
                    .get("texCoord")
                    .map_or(Ok(0), |t| u32_of(t, "witness binding TEXCOORD"))?;
                let normal_scale = if slot == TextureSlot::Normal {
                    b.get("scale")
                        .map(|v| v.as_f64().ok_or_else(|| invalid("witness normal scale")))
                        .transpose()?
                } else {
                    None
                };
                let dims = atlas
                    .preencode
                    .iter()
                    .find(|p| p.page == page)
                    .map(|p| [f64::from(p.width), f64::from(p.height)])
                    .ok_or_else(|| invalid("witness binding page is not retained"))?;
                let uvs = primitive
                    .uvs
                    .get(texcoord as usize)
                    .ok_or_else(|| invalid("witness binding TEXCOORD set"))?;
                let mut spacing: Option<f64> = None;
                for (face, w) in faces.iter_mut().enumerate() {
                    let p = std::array::from_fn(|c| decoded[face * 3 + c].map(f64::from));
                    let uv = std::array::from_fn(|c| {
                        let t = uvs[face * 3 + c];
                        [f64::from(t[0]) * dims[0], f64::from(t[1]) * dims[1]]
                    });
                    match texel_spacing(p, uv) {
                        Some(s) => spacing = Some(spacing.map_or(s, |m| m.max(s))),
                        None => w.zero_raster |= 1 << index,
                    }
                }
                if let Some(s) = spacing {
                    leaf_spacing = Some(leaf_spacing.map_or(s, |m| m.max(s)));
                }
                bindings.push(BindingWitness {
                    slot: index as u8,
                    page,
                    texcoord,
                    sampler,
                    normal_scale,
                    texel_spacing: spacing,
                });
            }
        }
        let corners = || primitive.triangles.iter().flat_map(|t| t.corners.iter());
        primitives.push(PrimitiveWitness {
            node: first.triangle.source.node,
            mesh: first.triangle.source.mesh,
            primitive: first.triangle.source.primitive,
            expanded_ordinal: first.expanded_ordinal,
            material: first.material,
            flags,
            bindings,
            faces,
            positions: decoded.clone(),
            normals: if flags & NORMAL != 0 {
                corners().map(|c| c.normal).collect()
            } else {
                Vec::new()
            },
            tangents: if flags & TANGENT != 0 {
                corners().map(|c| c.tangent).collect()
            } else {
                Vec::new()
            },
        });
    }
    Ok(LeafWitness {
        texel_spacing: leaf_spacing,
        primitives,
    })
}

const PRIMITIVE_BYTES: u64 = 4 + 4 + 4 + 8 + 4 + 1 + 4 + 4;
const BINDING_BYTES: u64 = 1 + 4 + 4 + 4 + 1 + 8 + 1 + 8;
const FACE_BYTES: u64 = 8 + 8 + 1;
fn corner_bytes(flags: u8) -> u64 {
    12 + if flags & NORMAL != 0 { 12 } else { 0 } + if flags & TANGENT != 0 { 16 } else { 0 }
}

impl LeafWitness {
    /// Exact length of [`LeafWitness::encode`].
    pub(in crate::mesh_large) fn encoded_len(&self) -> u64 {
        let mut n = 8 + 9 + 4;
        for p in &self.primitives {
            n += PRIMITIVE_BYTES
                + BINDING_BYTES * p.bindings.len() as u64
                + (FACE_BYTES + 3 * corner_bytes(p.flags)) * p.faces.len() as u64;
        }
        n
    }
    /// Heap bytes the decoded witness owns.
    pub(in crate::mesh_large) fn owned_bytes(&self) -> u64 {
        let mut n = (self.primitives.len() * size_of::<PrimitiveWitness>()) as u64;
        for p in &self.primitives {
            n += (p.bindings.len() * size_of::<BindingWitness>()
                + p.faces.len() * size_of::<FaceWitness>()
                + p.positions.len() * 12
                + p.normals.len() * 12
                + p.tangents.len() * 16) as u64;
        }
        n
    }
    pub(in crate::mesh_large) fn encode(&self) -> Result<Vec<u8>, JobError> {
        let length =
            usize::try_from(self.encoded_len()).map_err(|_| resource("witness encoding size"))?;
        let mut out = Vec::new();
        out.try_reserve_exact(length)
            .map_err(|_| resource("witness encoding"))?;
        let optional = |out: &mut Vec<u8>, v: Option<f64>| {
            out.push(u8::from(v.is_some()));
            out.extend_from_slice(&v.unwrap_or(0.).to_le_bytes());
        };
        out.extend_from_slice(MAGIC);
        optional(&mut out, self.texel_spacing);
        let count =
            u32::try_from(self.primitives.len()).map_err(|_| invalid("witness primitive count"))?;
        out.extend_from_slice(&count.to_le_bytes());
        for p in &self.primitives {
            let faces = p.faces.len();
            let has = |flag: u8, len: usize| {
                if p.flags & flag != 0 {
                    len == faces * 3
                } else {
                    len == 0
                }
            };
            if p.positions.len() != faces * 3
                || !has(NORMAL, p.normals.len())
                || !has(TANGENT, p.tangents.len())
                || p.bindings.len() > SLOTS.len()
            {
                return Err(invalid("witness primitive shape"));
            }
            for v in [p.node, p.mesh, p.primitive] {
                out.extend_from_slice(&v.to_le_bytes());
            }
            out.extend_from_slice(&p.expanded_ordinal.to_le_bytes());
            out.extend_from_slice(&p.material.to_le_bytes());
            out.push(p.flags);
            out.extend_from_slice(&(p.bindings.len() as u32).to_le_bytes());
            out.extend_from_slice(
                &u32::try_from(faces)
                    .map_err(|_| invalid("witness face count"))?
                    .to_le_bytes(),
            );
            for b in &p.bindings {
                out.push(b.slot);
                for v in [b.page, b.texcoord, b.sampler.unwrap_or(NONE_U32)] {
                    out.extend_from_slice(&v.to_le_bytes());
                }
                optional(&mut out, b.normal_scale);
                optional(&mut out, b.texel_spacing);
            }
            for (i, f) in p.faces.iter().enumerate() {
                out.extend_from_slice(&f.triangle.to_le_bytes());
                out.extend_from_slice(&f.rank.to_le_bytes());
                out.push(f.zero_raster);
                for c in i * 3..i * 3 + 3 {
                    for v in p.positions[c] {
                        out.extend_from_slice(&v.to_le_bytes());
                    }
                    if p.flags & NORMAL != 0 {
                        for v in p.normals[c] {
                            out.extend_from_slice(&v.to_le_bytes());
                        }
                    }
                    if p.flags & TANGENT != 0 {
                        for v in p.tangents[c] {
                            out.extend_from_slice(&v.to_le_bytes());
                        }
                    }
                }
            }
        }
        if out.len() != length {
            return Err(invalid("witness encoding length"));
        }
        Ok(out)
    }
    /// Decodes [`LeafWitness::encode`] output, charging every owned allocation against
    /// `memory_limit` before making it.
    pub(in crate::mesh_large) fn decode(bytes: &[u8], memory_limit: u64) -> Result<Self, JobError> {
        let mut r = Cursor { bytes, at: 0 };
        let mut lease = 0u64;
        let mut charge = |n: u64| -> Result<(), JobError> {
            lease = lease
                .checked_add(n)
                .ok_or_else(|| resource("leaf witness lease"))?;
            if lease > memory_limit {
                return Err(resource("leaf witness exceeds its memory lease"));
            }
            Ok(())
        };
        if r.take(8)? != MAGIC {
            return Err(corrupt());
        }
        let texel_spacing = r.optional()?;
        let count = u64::from(r.u32()?);
        // Every primitive needs at least its fixed record.
        if count * PRIMITIVE_BYTES > r.remaining() {
            return Err(corrupt());
        }
        charge(count * size_of::<PrimitiveWitness>() as u64)?;
        let mut primitives = Vec::with_capacity(count as usize);
        for _ in 0..count {
            let (node, mesh, primitive) = (r.u32()?, r.u32()?, r.u32()?);
            let expanded_ordinal = r.u64()?;
            let material = r.u32()?;
            let flags = r.take(1)?[0];
            let binding_count = u64::from(r.u32()?);
            let faces = u64::from(r.u32()?);
            if binding_count > SLOTS.len() as u64
                || binding_count * BINDING_BYTES + faces * (FACE_BYTES + 3 * corner_bytes(flags))
                    > r.remaining()
            {
                return Err(corrupt());
            }
            let (normals, tangents) = (flags & NORMAL != 0, flags & TANGENT != 0);
            charge(
                binding_count * size_of::<BindingWitness>() as u64
                    + faces
                        * (size_of::<FaceWitness>() as u64
                            + 36
                            + if normals { 36 } else { 0 }
                            + if tangents { 48 } else { 0 }),
            )?;
            let mut bindings = Vec::with_capacity(binding_count as usize);
            for _ in 0..binding_count {
                let slot = r.take(1)?[0];
                let (page, texcoord, sampler) = (r.u32()?, r.u32()?, r.u32()?);
                let normal_scale = r.optional()?;
                let spacing = r.optional()?;
                if usize::from(slot) >= SLOTS.len() {
                    return Err(corrupt());
                }
                bindings.push(BindingWitness {
                    slot,
                    page,
                    texcoord,
                    sampler: (sampler != NONE_U32).then_some(sampler),
                    normal_scale,
                    texel_spacing: spacing,
                });
            }
            let n = faces as usize;
            let mut w = PrimitiveWitness {
                node,
                mesh,
                primitive,
                expanded_ordinal,
                material,
                flags,
                bindings,
                faces: Vec::with_capacity(n),
                positions: Vec::with_capacity(n * 3),
                normals: Vec::with_capacity(if normals { n * 3 } else { 0 }),
                tangents: Vec::with_capacity(if tangents { n * 3 } else { 0 }),
            };
            for _ in 0..n {
                let triangle = r.u64()?;
                let rank = r.u64()?;
                let zero_raster = r.take(1)?[0];
                if zero_raster >> SLOTS.len() != 0 {
                    return Err(corrupt());
                }
                w.faces.push(FaceWitness {
                    triangle,
                    rank,
                    zero_raster,
                });
                for _ in 0..3 {
                    w.positions.push([r.f32()?, r.f32()?, r.f32()?]);
                    if normals {
                        w.normals.push([r.f32()?, r.f32()?, r.f32()?]);
                    }
                    if tangents {
                        w.tangents.push([r.f32()?, r.f32()?, r.f32()?, r.f32()?]);
                    }
                }
            }
            primitives.push(w);
        }
        if r.remaining() != 0 {
            return Err(corrupt());
        }
        Ok(Self {
            texel_spacing,
            primitives,
        })
    }
}

fn corrupt() -> JobError {
    invalid("retained leaf witness is corrupt")
}
struct Cursor<'a> {
    bytes: &'a [u8],
    at: usize,
}
impl Cursor<'_> {
    fn remaining(&self) -> u64 {
        (self.bytes.len() - self.at) as u64
    }
    fn take(&mut self, n: usize) -> Result<&[u8], JobError> {
        let end = self.at.checked_add(n).ok_or_else(corrupt)?;
        let slice = self.bytes.get(self.at..end).ok_or_else(corrupt)?;
        self.at = end;
        Ok(slice)
    }
    fn array<const N: usize>(&mut self) -> Result<[u8; N], JobError> {
        Ok(self.take(N)?.try_into().unwrap())
    }
    fn u32(&mut self) -> Result<u32, JobError> {
        Ok(u32::from_le_bytes(self.array()?))
    }
    fn u64(&mut self) -> Result<u64, JobError> {
        Ok(u64::from_le_bytes(self.array()?))
    }
    fn f32(&mut self) -> Result<f32, JobError> {
        Ok(f32::from_le_bytes(self.array()?))
    }
    fn optional(&mut self) -> Result<Option<f64>, JobError> {
        let flag = self.take(1)?[0];
        let value = f64::from_le_bytes(self.array()?);
        match flag {
            0 if value.to_bits() == 0 => Ok(None),
            1 => Ok(Some(value)),
            _ => Err(corrupt()),
        }
    }
}
