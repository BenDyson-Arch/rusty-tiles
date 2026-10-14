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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mesh_large::{
        bake_pass::{BakePass, LeafPreencode, ProxyBaker},
        leaves::producer::{mesh_to_leaf_payloads, LeafOutputRequest, LeafProfile},
    };
    use crate::RunControl;
    use serde_json::json;
    use std::{
        collections::BTreeMap,
        fs,
        num::NonZeroUsize,
        path::{Path, PathBuf},
        sync::{Arc, Mutex},
    };

    fn sample() -> LeafWitness {
        LeafWitness {
            texel_spacing: Some(0.25f64.next_up()),
            primitives: vec![
                PrimitiveWitness {
                    node: 3,
                    mesh: 2,
                    primitive: 1,
                    expanded_ordinal: (1 << 54) + 9,
                    material: 7,
                    flags: NORMAL | TANGENT,
                    bindings: vec![
                        BindingWitness {
                            slot: 0,
                            page: 0,
                            texcoord: 1,
                            sampler: Some(0),
                            normal_scale: None,
                            texel_spacing: Some(0.25f64.next_up()),
                        },
                        BindingWitness {
                            slot: 2,
                            page: 1,
                            texcoord: 0,
                            sampler: None,
                            normal_scale: Some(-0.0),
                            texel_spacing: None,
                        },
                    ],
                    faces: vec![
                        FaceWitness {
                            triangle: u64::MAX - 1,
                            rank: 1 << 40,
                            zero_raster: 0,
                        },
                        FaceWitness {
                            triangle: 5,
                            rank: 0,
                            zero_raster: 0b10101,
                        },
                    ],
                    positions: (0..6)
                        .map(|i| [i as f32, -0.0, f32::MIN_POSITIVE])
                        .collect(),
                    normals: (0..6).map(|i| [0., i as f32, 1.]).collect(),
                    tangents: (0..6).map(|i| [1., 0., i as f32, -1.]).collect(),
                },
                PrimitiveWitness {
                    node: 0,
                    mesh: 0,
                    primitive: 0,
                    expanded_ordinal: 0,
                    material: u32::MAX,
                    flags: 0,
                    bindings: Vec::new(),
                    faces: vec![FaceWitness {
                        triangle: 0,
                        rank: 2,
                        zero_raster: 0,
                    }],
                    positions: vec![[1., 2., 3.]; 3],
                    normals: Vec::new(),
                    tangents: Vec::new(),
                },
            ],
        }
    }
    fn bits(w: &LeafWitness) -> String {
        // Debug of f32/f64 does not separate -0.0 from 0.0 in every place; compare bits.
        let mut out = format!("{:?}", w.texel_spacing.map(f64::to_bits));
        for p in &w.primitives {
            out += &format!(
                "{:?}{:?}{:?}{:?}",
                p.bindings
                    .iter()
                    .map(|b| (
                        b.normal_scale.map(f64::to_bits),
                        b.texel_spacing.map(f64::to_bits)
                    ))
                    .collect::<Vec<_>>(),
                p.positions
                    .iter()
                    .flatten()
                    .map(|v| v.to_bits())
                    .collect::<Vec<_>>(),
                p.normals
                    .iter()
                    .flatten()
                    .map(|v| v.to_bits())
                    .collect::<Vec<_>>(),
                p.tangents
                    .iter()
                    .flatten()
                    .map(|v| v.to_bits())
                    .collect::<Vec<_>>(),
            );
        }
        out
    }

    #[test]
    fn witness_round_trips_bit_exact_within_its_lease() {
        let witness = sample();
        let bytes = witness.encode().unwrap();
        assert_eq!(bytes.len() as u64, witness.encoded_len());
        let decoded = LeafWitness::decode(&bytes, witness.owned_bytes()).unwrap();
        assert_eq!(decoded, witness);
        assert_eq!(bits(&decoded), bits(&witness));
        // The lease is charged before allocation: one byte short is refused.
        let error = LeafWitness::decode(&bytes, witness.owned_bytes() - 1).unwrap_err();
        assert_eq!(error.kind(), JobErrorKind::ResourceLimit);
    }

    #[test]
    fn corrupt_truncated_or_unbacked_witnesses_are_refused() {
        let bytes = sample().encode().unwrap();
        let refused = |b: &[u8]| LeafWitness::decode(b, u64::MAX).unwrap_err().kind();
        assert_eq!(
            refused(&bytes[..bytes.len() - 1]),
            JobErrorKind::InvalidState
        );
        let mut trailing = bytes.clone();
        trailing.push(0);
        assert_eq!(refused(&trailing), JobErrorKind::InvalidState);
        let mut magic = bytes.clone();
        magic[5] = 2;
        assert_eq!(refused(&magic), JobErrorKind::InvalidState);
        // A primitive count the input cannot back fails before any allocation.
        let mut count = bytes.clone();
        count[17..21].copy_from_slice(&u32::MAX.to_le_bytes());
        assert_eq!(refused(&count), JobErrorKind::InvalidState);
        // An optional flag other than 0/1, and a zero-raster bit beyond the five slots.
        let mut flag = bytes.clone();
        flag[8] = 2;
        assert_eq!(refused(&flag), JobErrorKind::InvalidState);
        let first_face = 21 + PRIMITIVE_BYTES as usize + 2 * BINDING_BYTES as usize;
        let mut raster = bytes.clone();
        raster[first_face + 16] = 1 << 5;
        assert_eq!(refused(&raster), JobErrorKind::InvalidState);
    }

    #[test]
    fn texel_spacing_is_the_worst_direction_in_metres_per_texel() {
        let p = [[0., 0., 0.], [2., 0., 0.], [0., 3., 0.]];
        let uv = [[0., 0.], [1., 0.], [0., 1.]];
        assert_eq!(texel_spacing(p, uv), Some(3f64.next_up()));
        // Twice the texels over the same surface halves the spacing.
        let dense = uv.map(|t| t.map(|v| v * 2.));
        assert_eq!(texel_spacing(p, dense), Some(1.5f64.next_up()));
        // Zero-area geometry or page-UV triangles do not rasterise.
        assert_eq!(
            texel_spacing([[0.; 3], [1., 0., 0.], [2., 0., 0.]], uv),
            None
        );
        assert_eq!(texel_spacing(p, [[0., 0.], [1., 1.], [2., 2.]]), None);
    }

    /// Two primitives: one with NORMAL, TANGENT, a LINEAR-sampled base colour and a
    /// normal map with a literal negative scale; one mip-sampled base colour only.
    /// `nudge` moves one source normal, to give a leaf whose published values differ.
    fn fixture(dir: &Path, g: usize, nudge: f32) -> PathBuf {
        fs::create_dir_all(dir).unwrap();
        let mut bin = Vec::new();
        let (mut views, mut accessors, mut meshes) = (vec![], vec![], vec![]);
        let half = g / 2;
        for (b, (i0, i1)) in [(0, half), (half, g)].into_iter().enumerate() {
            let w = g + 1;
            let (mut pos, mut nrm, mut tan, mut uv) = (vec![], vec![], vec![], vec![]);
            for i in i0..=i1 {
                for j in 0..=g {
                    let (x, z) = (i as f32 * 0.37, j as f32 * 0.41);
                    pos.push([x, (x / 2.).sin() * (z / 3.).cos(), z]);
                    let n = [-(x / 2.).cos() * 0.5, 1., (z / 3.).sin() * 0.3];
                    let l = (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt();
                    nrm.push(n.map(|v| v / l));
                    tan.push([1., 0., 0., -1.]);
                    // The second primitive's first column repeats the second's UVs, so one
                    // face of each of its quads has a zero-area page-UV triangle.
                    let jj = if b == 1 { j.max(1) } else { j };
                    uv.push([i as f32 / g as f32, (jj as f32 / g as f32).powf(1.3)]);
                }
            }
            let m = [nrm[3][0] + nudge, nrm[3][1], nrm[3][2]];
            let l = (m[0] * m[0] + m[1] * m[1] + m[2] * m[2]).sqrt();
            nrm[3] = m.map(|v| v / l);
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
            let floats = |rows: &[Vec<f32>]| -> Vec<u8> {
                rows.iter()
                    .flatten()
                    .flat_map(|v| v.to_le_bytes())
                    .collect()
            };
            let (lo, hi) = pos
                .iter()
                .fold(([f32::MAX; 3], [f32::MIN; 3]), |(l, h), p| {
                    (
                        std::array::from_fn(|a| l[a].min(p[a])),
                        std::array::from_fn(|a| h[a].max(p[a])),
                    )
                });
            let first = accessors.len();
            let p = push(floats(&pos.iter().map(|v| v.to_vec()).collect::<Vec<_>>()));
            accessors.push(json!({"bufferView":p,"componentType":5126,"count":pos.len(),"type":"VEC3","min":lo,"max":hi}));
            let n = push(floats(&nrm.iter().map(|v| v.to_vec()).collect::<Vec<_>>()));
            accessors
                .push(json!({"bufferView":n,"componentType":5126,"count":nrm.len(),"type":"VEC3"}));
            let t = push(floats(&tan.iter().map(|v| v.to_vec()).collect::<Vec<_>>()));
            accessors
                .push(json!({"bufferView":t,"componentType":5126,"count":tan.len(),"type":"VEC4"}));
            let u = push(floats(&uv.iter().map(|v| v.to_vec()).collect::<Vec<_>>()));
            accessors
                .push(json!({"bufferView":u,"componentType":5126,"count":uv.len(),"type":"VEC2"}));
            let i = push(idx.iter().flat_map(|v| v.to_le_bytes()).collect());
            accessors.push(
                json!({"bufferView":i,"componentType":5125,"count":idx.len(),"type":"SCALAR"}),
            );
            let attributes = if b == 0 {
                json!({"POSITION":first,"NORMAL":first+1,"TANGENT":first+2,"TEXCOORD_0":first+3})
            } else {
                json!({"POSITION":first,"TEXCOORD_0":first+3})
            };
            meshes.push(
                json!({"primitives":[{"attributes":attributes,"indices":first+4,"material":b}]}),
            );
        }
        fs::write(dir.join("scene.bin"), &bin).unwrap();
        for (name, seed) in [("colour.png", 0u32), ("normal.png", 1)] {
            image::RgbaImage::from_fn(64, 64, |x, y| {
                let v = |k: u32| (40 + x + y + k * 20 + seed * 9 + (x * 37 + y * 91) % 5) as u8;
                if seed == 1 {
                    let (nx, ny) = ((x as f32 / 63. - 0.5) * 0.4, (y as f32 / 63. - 0.5) * 0.4);
                    let n = [nx, ny, (1. - nx * nx - ny * ny).sqrt()];
                    let [r, g, b] = n.map(|c| ((c * 0.5 + 0.5) * 255.).round() as u8);
                    image::Rgba([r, g, b, 255])
                } else {
                    image::Rgba([v(0), v(1), v(2), 255])
                }
            })
            .save(dir.join(name))
            .unwrap();
        }
        let doc = json!({"asset":{"version":"2.0"},"scene":0,"scenes":[{"nodes":[0,1]}],"nodes":[{"mesh":0},{"mesh":1}],
            "meshes":meshes,"buffers":[{"uri":"scene.bin","byteLength":bin.len()}],"bufferViews":views,"accessors":accessors,
            "images":[{"uri":"colour.png"},{"uri":"normal.png"}],
            "samplers":[{"minFilter":9729,"magFilter":9729},{"minFilter":9987,"magFilter":9729}],
            "textures":[{"source":0,"sampler":0},{"source":1,"sampler":0},{"source":0,"sampler":1}],
            "materials":[{"pbrMetallicRoughness":{"baseColorTexture":{"index":0}},"normalTexture":{"index":1,"scale":-0.5}},
                         {"pbrMetallicRoughness":{"baseColorTexture":{"index":2}}}]});
        let path = dir.join("scene.gltf");
        fs::write(&path, serde_json::to_vec(&doc).unwrap()).unwrap();
        path
    }

    /// Reads every leaf child's retained packet; bakes nothing.
    #[derive(Debug, Default)]
    struct Probe {
        leaves: Mutex<BTreeMap<u64, LeafPreencode>>,
    }
    impl ProxyBaker for Probe {
        fn bake(&self, pass: &BakePass<'_>) -> Result<(), JobError> {
            for node in pass.nodes()? {
                let (first, depth) = node?;
                for child in pass.frozen(first, depth)?.children {
                    if let Some(leaf) = child.leaf {
                        let packet = pass.preencode.read_leaf(leaf.index)?;
                        self.leaves.lock().unwrap().insert(leaf.index, packet);
                    }
                }
            }
            Ok(())
        }
    }

    /// One published primitive, decoded from the GLB alone (meshopt + node TRS).
    #[derive(Clone, Debug, PartialEq)]
    struct Published {
        positions: Vec<[u32; 3]>,
        normals: Vec<[u32; 3]>,
        tangents: Vec<[u32; 4]>,
        uvs: Vec<Vec<[f32; 2]>>,
        material: Option<Value>,
    }
    fn published(glb: &[u8]) -> (Value, Vec<Published>) {
        let json_len = u32::from_le_bytes(glb[12..16].try_into().unwrap()) as usize;
        let doc: Value = serde_json::from_slice(&glb[20..20 + json_len]).unwrap();
        let bin = &glb[20 + json_len + 8..];
        let view = |id: usize| -> Vec<u8> {
            let v = &doc["bufferViews"][id];
            let e = &v["extensions"]["EXT_meshopt_compression"];
            let start = e["byteOffset"].as_u64().unwrap() as usize;
            let length = e["byteLength"].as_u64().unwrap() as usize;
            crate::content_integrity::meshopt::decode(
                &bin[start..start + length],
                e["count"].as_u64().unwrap() as usize,
                e["byteStride"].as_u64().unwrap() as usize,
                crate::content_integrity::meshopt::Mode::Attributes,
            )
            .unwrap()
        };
        let floats = |accessor: &Value| -> Vec<f32> {
            view(accessor["bufferView"].as_u64().unwrap() as usize)
                .as_chunks::<4>()
                .0
                .iter()
                .map(|b| f32::from_le_bytes(*b))
                .collect()
        };
        let mut out = Vec::new();
        for (n, node) in doc["nodes"].as_array().unwrap().iter().enumerate() {
            let primitive =
                &doc["meshes"][node["mesh"].as_u64().unwrap() as usize]["primitives"][0];
            assert_eq!(node["mesh"], n);
            let attribute = |name: &str| {
                primitive["attributes"]
                    .get(name)
                    .map(|a| &doc["accessors"][a.as_u64().unwrap() as usize])
            };
            let position = attribute("POSITION").unwrap();
            let positions: Vec<[u32; 3]> = if position["componentType"] == 5123 {
                let t: [f32; 3] =
                    std::array::from_fn(|a| node["translation"][a].as_f64().unwrap() as f32);
                let s: [f32; 3] =
                    std::array::from_fn(|a| node["scale"][a].as_f64().unwrap() as f32);
                view(position["bufferView"].as_u64().unwrap() as usize)
                    .as_chunks::<8>()
                    .0
                    .iter()
                    .map(|c| {
                        std::array::from_fn(|a| {
                            let code = u16::from_le_bytes([c[2 * a], c[2 * a + 1]]);
                            (f32::from(code) * s[a] + t[a]).to_bits()
                        })
                    })
                    .collect()
            } else {
                assert!(node.get("translation").is_none() && node.get("scale").is_none());
                floats(position)
                    .as_chunks::<3>()
                    .0
                    .iter()
                    .map(|p| p.map(f32::to_bits))
                    .collect()
            };
            let normals = attribute("NORMAL").map_or(Vec::new(), |a| {
                floats(a)
                    .as_chunks::<3>()
                    .0
                    .iter()
                    .map(|p| p.map(f32::to_bits))
                    .collect()
            });
            let tangents = attribute("TANGENT").map_or(Vec::new(), |a| {
                floats(a)
                    .as_chunks::<4>()
                    .0
                    .iter()
                    .map(|p| p.map(f32::to_bits))
                    .collect()
            });
            let uvs = (0..)
                .map_while(|set| attribute(&format!("TEXCOORD_{set}")))
                .map(|a| floats(a).as_chunks::<2>().0.to_vec())
                .collect();
            let material = primitive
                .get("material")
                .map(|m| doc["materials"][m.as_u64().unwrap() as usize].clone());
            out.push(Published {
                positions,
                normals,
                tangents,
                uvs,
                material,
            });
        }
        (doc, out)
    }

    /// Every witness field against the published GLB (and the retained page sizes).
    fn compare(leaf: &LeafPreencode, glb: &[u8], first: u64, end: u64) -> Result<(), String> {
        let w = &leaf.witness;
        let (doc, published) = published(glb);
        if published.len() != w.primitives.len() {
            return Err("primitive count".into());
        }
        let mut ranks = Vec::new();
        let mut leaf_spacing: Option<f64> = None;
        for (k, (p, g)) in w.primitives.iter().zip(&published).enumerate() {
            let corner = |v: &[[f32; 3]]| v.iter().map(|c| c.map(f32::to_bits)).collect::<Vec<_>>();
            if corner(&p.positions) != g.positions {
                return Err(format!("primitive {k} POSITION"));
            }
            if corner(&p.normals) != g.normals {
                return Err(format!("primitive {k} NORMAL"));
            }
            if p.tangents
                .iter()
                .map(|c| c.map(f32::to_bits))
                .collect::<Vec<_>>()
                != g.tangents
            {
                return Err(format!("primitive {k} TANGENT"));
            }
            let source = &leaf.primitives[k];
            for (face, (f, s)) in p.faces.iter().zip(&source.triangles).enumerate() {
                if (p.node, p.mesh, p.primitive, f.triangle) != *s {
                    return Err(format!("primitive {k} face {face} source identity"));
                }
                ranks.push(f.rank);
            }
            // Bindings from the published material, texture and sampler JSON.
            let material = g.material.as_ref();
            let mut expected = Vec::new();
            for (index, &slot) in SLOTS.iter().enumerate() {
                let Some(b) = material.map(|m| binding(m, slot)).filter(|b| !b.is_null()) else {
                    continue;
                };
                let texture = &doc["textures"][b["index"].as_u64().unwrap() as usize];
                let page = texture
                    .get("source")
                    .unwrap_or(&texture["extensions"]["KHR_texture_basisu"]["source"])
                    .as_u64()
                    .unwrap() as u32;
                let texcoord = b.get("texCoord").map_or(0, |t| t.as_u64().unwrap()) as u32;
                let retained = leaf.pages.iter().find(|q| q.page == page).unwrap();
                let mut spacing: Option<f64> = None;
                let mut raster = Vec::new();
                for face in 0..p.faces.len() {
                    let pos = std::array::from_fn(|c| {
                        g.positions[face * 3 + c].map(|v| f64::from(f32::from_bits(v)))
                    });
                    let uv = std::array::from_fn(|c| {
                        let t = g.uvs[texcoord as usize][face * 3 + c];
                        [
                            f64::from(t[0]) * f64::from(retained.width),
                            f64::from(t[1]) * f64::from(retained.height),
                        ]
                    });
                    let s = texel_spacing(pos, uv);
                    raster.push(s.is_none());
                    if let Some(s) = s {
                        spacing = Some(spacing.map_or(s, |m| m.max(s)));
                    }
                }
                for (face, zero) in raster.into_iter().enumerate() {
                    if (p.faces[face].zero_raster >> index) & 1 != u8::from(zero) {
                        return Err(format!("primitive {k} face {face} zero-raster"));
                    }
                }
                if let Some(s) = spacing {
                    leaf_spacing = Some(leaf_spacing.map_or(s, |m| m.max(s)));
                }
                expected.push(BindingWitness {
                    slot: index as u8,
                    page,
                    texcoord,
                    sampler: texture.get("sampler").map(|s| s.as_u64().unwrap() as u32),
                    normal_scale: (slot == TextureSlot::Normal)
                        .then(|| b.get("scale").map(|s| s.as_f64().unwrap()))
                        .flatten(),
                    texel_spacing: spacing,
                });
            }
            if p.bindings != expected {
                return Err(format!(
                    "primitive {k} bindings {:?} != {expected:?}",
                    p.bindings
                ));
            }
        }
        if w.texel_spacing.map(f64::to_bits) != leaf_spacing.map(f64::to_bits) {
            return Err("leaf texel spacing".into());
        }
        ranks.sort_unstable();
        if ranks != (first..end).collect::<Vec<_>>() {
            return Err("ranks are not the leaf's [first, end)".into());
        }
        Ok(())
    }

    fn run(
        dir: &Path,
        input: &Path,
        profile: LeafProfile,
    ) -> (PathBuf, BTreeMap<u64, LeafPreencode>) {
        let out = dir.join("out");
        let scratch = dir.join("scratch");
        fs::create_dir_all(&scratch).unwrap();
        let probe = Arc::new(Probe::default());
        mesh_to_leaf_payloads(
            LeafOutputRequest::new(input, &out, profile)
                .with_jobs(NonZeroUsize::new(2).unwrap())
                .with_scratch_dir(&scratch)
                .with_lod_target(256)
                .with_leaf_triangles(1024)
                .with_baker(probe.clone()),
            &RunControl::default(),
        )
        .unwrap();
        let leaves = std::mem::take(&mut *probe.leaves.lock().unwrap());
        (out, leaves)
    }
    fn rows(out: &Path) -> Vec<Value> {
        fs::read_to_string(out.join("leaves.jsonl"))
            .unwrap()
            .lines()
            .map(|l| serde_json::from_str(l).unwrap())
            .collect()
    }

    #[test]
    fn witness_matches_the_published_leaf_decode_and_tracks_a_mutated_leaf() {
        let work = tempfile::tempdir().unwrap();
        for profile in [LeafProfile::Balanced, LeafProfile::Quality] {
            let name = format!("{profile:?}");
            let input = fixture(&work.path().join(&name).join("src"), 48, 0.);
            let (out, leaves) = run(&work.path().join(&name), &input, profile);
            let listed = rows(&out);
            assert!(listed.len() >= 3, "several leaves");
            assert_eq!(leaves.len(), listed.len(), "every leaf child read once");
            let (mut spaced, mut zero_raster) = (0, 0);
            for (index, row) in listed.iter().enumerate() {
                let leaf = &leaves[&(index as u64)];
                let glb = fs::read(out.join(row["uri"].as_str().unwrap())).unwrap();
                let (first, end) = (row["first"].as_u64().unwrap(), row["end"].as_u64().unwrap());
                compare(leaf, &glb, first, end)
                    .unwrap_or_else(|e| panic!("{name} leaf {index}: {e}"));
                spaced += usize::from(leaf.witness.texel_spacing.is_some());
                zero_raster += leaf
                    .witness
                    .primitives
                    .iter()
                    .flat_map(|p| &p.faces)
                    .filter(|f| f.zero_raster != 0)
                    .count();
                // Mutated witnesses are caught.
                let mut normal = leaf.clone();
                if let Some(p) = normal
                    .witness
                    .primitives
                    .iter_mut()
                    .find(|p| !p.normals.is_empty())
                {
                    p.normals[0][0] = p.normals[0][0].next_up();
                    assert!(compare(&normal, &glb, first, end)
                        .unwrap_err()
                        .contains("NORMAL"));
                }
                let mut position = leaf.clone();
                position.witness.primitives[0].positions[1][2] += 1. / 64.;
                assert!(compare(&position, &glb, first, end)
                    .unwrap_err()
                    .contains("POSITION"));
                let mut rank = leaf.clone();
                rank.witness.primitives[0].faces[0].rank = end;
                assert!(compare(&rank, &glb, first, end)
                    .unwrap_err()
                    .contains("ranks"));
                let mut scale = leaf.clone();
                for p in &mut scale.witness.primitives {
                    for b in &mut p.bindings {
                        if let Some(s) = &mut b.normal_scale {
                            assert_eq!(*s, -0.5, "literal normal scale");
                            *s = 1.;
                        }
                    }
                }
                if scale != *leaf {
                    assert!(compare(&scale, &glb, first, end)
                        .unwrap_err()
                        .contains("bindings"));
                }
            }
            assert!(spaced > 0, "textured leaves carry a measured spacing");
            assert!(zero_raster > 0, "zero-area UV faces are kept and marked");
            // A leaf published from a mutated source normal: its own witness matches its
            // own GLB, and the original witness no longer does.
            let nudged = fixture(
                &work.path().join(format!("{name}-nudged")).join("src"),
                48,
                0.25,
            );
            let (out2, leaves2) = run(
                &work.path().join(format!("{name}-nudged")),
                &nudged,
                profile,
            );
            let mut differed = 0;
            for (index, row) in rows(&out2).iter().enumerate() {
                let glb = fs::read(out2.join(row["uri"].as_str().unwrap())).unwrap();
                let (first, end) = (row["first"].as_u64().unwrap(), row["end"].as_u64().unwrap());
                compare(&leaves2[&(index as u64)], &glb, first, end).unwrap();
                if let Some(original) = leaves.get(&(index as u64)) {
                    differed += usize::from(compare(original, &glb, first, end).is_err());
                }
            }
            assert!(
                differed > 0,
                "{name}: the original witness must not match the mutated leaf"
            );
        }
    }
}
