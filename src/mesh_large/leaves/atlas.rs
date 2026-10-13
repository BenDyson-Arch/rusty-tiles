//! Bounded per-leaf preparation: positive source-pixel maps and owned local closure.
use super::super::{
    attributes::{FullTriangle, COLOR},
    config::{Profile, TILE_BYTES},
    full::FullCaptured,
};
use super::{
    crops::{self, Crop, SourceSampler},
    delivery::{self, MaskDelivery},
    geometry::LatticePlan,
    mask::SurfaceSample,
    mips::{self, Class},
    prepared::{PreparedAtlas, PreparedPage, PreparedPrimitive},
    source_sampler_mips::{self, SamplerCheck, SamplerSample},
};
use crate::{
    gltf_source::source::{self, TextureSlot},
    JobError, JobErrorKind,
};
use serde_json::{json, Value};
use std::collections::BTreeMap;

fn error(kind: JobErrorKind, message: &str) -> JobError {
    JobError::new(kind, message)
}
fn resource() -> JobError {
    error(
        JobErrorKind::ResourceLimit,
        "leaf atlas allocation/count overflow",
    )
}
fn invariant() -> JobError {
    error(
        JobErrorKind::InvalidState,
        "leaf atlas source closure mismatch",
    )
}
fn role(slot: TextureSlot) -> u8 {
    match slot {
        TextureSlot::BaseColor => 0,
        TextureSlot::MetallicRoughness => 1,
        TextureSlot::Normal => 2,
        TextureSlot::Occlusion => 3,
        TextureSlot::Emissive => 4,
    }
}
fn class(slot: TextureSlot) -> Class {
    match slot {
        TextureSlot::BaseColor | TextureSlot::Emissive => Class::Colour,
        TextureSlot::Normal => Class::Normal,
        _ => Class::Data,
    }
}
struct Binding {
    primitive: usize,
    slot: TextureSlot,
    source_uv: usize,
    output_uv: usize,
    image: usize,
    sampler: SourceSampler,
    source_mips: bool,
    crop: Crop,
    source_size: [u32; 2],
    offset: [u32; 2],
    page: usize,
}
struct PagePlan {
    bindings: Vec<usize>,
    slot: TextureSlot,
    sampler: SourceSampler,
    source_mips: bool,
    width: u32,
    height: u32,
}

/// Stable shelves ordered by descending crop dimensions and canonical binding order.
fn pack(
    bindings: &mut [Binding],
    max_edge: u32,
    masked: &[bool],
) -> Result<Vec<PagePlan>, JobError> {
    let mut buckets = BTreeMap::<(u8, u32, u32, Option<usize>), Vec<usize>>::new();
    for (i, b) in bindings.iter().enumerate() {
        let mask = if b.source_mips {
            Some(i)
        } else {
            (b.slot == TextureSlot::BaseColor && masked[b.primitive]).then_some(b.primitive)
        };
        buckets
            .entry((
                role(b.slot),
                b.sampler.min_filter,
                b.sampler.mag_filter,
                mask,
            ))
            .or_default()
            .push(i);
    }
    let mut pages = Vec::new();
    for (_, mut ids) in buckets {
        ids.sort_by_key(|&i| {
            (
                std::cmp::Reverse(bindings[i].crop.height),
                std::cmp::Reverse(bindings[i].crop.width),
                i,
            )
        });
        let first = &bindings[ids[0]];
        let mut page = PagePlan {
            bindings: Vec::new(),
            slot: first.slot,
            sampler: first.sampler,
            source_mips: first.source_mips,
            width: 0,
            height: 0,
        };
        let (mut x, mut y, mut row) = (0u32, 0u32, 0u32);
        for i in ids {
            let b = &mut bindings[i];
            if x.checked_add(b.crop.width).ok_or_else(resource)? > max_edge {
                x = 0;
                y = y.checked_add(row).ok_or_else(resource)?;
                row = 0;
            }
            if y.checked_add(b.crop.height).ok_or_else(resource)? > max_edge {
                page.width = page.width.max(4).next_power_of_two();
                page.height = page.height.max(4).next_power_of_two();
                pages.push(page);
                page = PagePlan {
                    bindings: Vec::new(),
                    slot: b.slot,
                    sampler: b.sampler,
                    source_mips: b.source_mips,
                    width: 0,
                    height: 0,
                };
                x = 0;
                y = 0;
                row = 0;
            }
            b.page = pages.len();
            b.offset = [x, y];
            page.bindings.push(i);
            x = x.checked_add(b.crop.width).ok_or_else(resource)?;
            row = row.max(b.crop.height);
            page.width = page.width.max(x);
            page.height = page.height.max(y + row);
        }
        if !page.bindings.is_empty() {
            page.width = page.width.max(4).next_power_of_two();
            page.height = page.height.max(4).next_power_of_two();
            pages.push(page);
        }
    }
    Ok(pages)
}

fn triangles_by_primitive(
    triangles: Vec<FullTriangle>,
    depths: Vec<[u8; 3]>,
) -> Result<Vec<PreparedPrimitive>, JobError> {
    if triangles.len() != depths.len() {
        return Err(invariant());
    }
    let mut groups = BTreeMap::<(u32, u32, u32, u32, u8), (Vec<FullTriangle>, Vec<[u8; 3]>)>::new();
    for (t, d) in triangles.into_iter().zip(depths) {
        let p = t.triangle.source;
        let pair = groups
            .entry((p.node, p.mesh, p.primitive, t.material, t.flags))
            .or_default();
        pair.0.push(t);
        pair.1.push(d);
    }
    Ok(groups
        .into_values()
        .map(|(triangles, corner_depths)| PreparedPrimitive {
            triangles,
            corner_depths,
            uvs: Vec::new(),
            material: None,
        })
        .collect())
}

fn source_image(
    captured: &FullCaptured,
    index: usize,
    check: &mut dyn FnMut() -> Result<(), JobError>,
) -> Result<(source::ImageInfo, Vec<u8>), JobError> {
    let snapshot = captured.images.get(index).ok_or_else(invariant)?;
    let encoded = snapshot.reconstruct(check)?;
    source::decode_image(&encoded, snapshot.identity.mime.as_deref(), &mut || check())
}

fn density_image(
    captured: &FullCaptured,
    index: usize,
    slot: TextureSlot,
    drop: u32,
    memory: u64,
    check: &mut dyn FnMut() -> Result<(), JobError>,
) -> Result<([u32; 2], Vec<u8>), JobError> {
    let (info, rgba) = source_image(captured, index, check)?;
    if drop == 0 {
        return Ok(([info.width, info.height], rgba));
    }
    let mut chain = mips::build(
        info.width,
        info.height,
        rgba,
        class(slot),
        None,
        usize::try_from(memory).map_err(|_| resource())?,
    )?;
    let index = (drop as usize).min(chain.levels.len() - 1);
    let level = chain.levels.swap_remove(index);
    Ok(([level.width, level.height], level.rgba))
}

fn source_alpha(
    rgba: &[u8],
    size: [u32; 2],
    uv: [f64; 2],
    sampler: SourceSampler,
    linear: bool,
) -> f64 {
    let wrap = |i: i64, n: u32, w: u32| {
        let n = i64::from(n);
        (match w {
            10497 => i.rem_euclid(n),
            33648 => {
                let p = i.rem_euclid(2 * n);
                if p < n {
                    p
                } else {
                    2 * n - 1 - p
                }
            }
            _ => i.clamp(0, n - 1),
        }) as usize
    };
    let tap = |x: i64, y: i64| {
        f64::from(
            rgba[(wrap(y, size[1], sampler.wrap_t) * size[0] as usize
                + wrap(x, size[0], sampler.wrap_s))
                * 4
                + 3],
        ) / 255.
    };
    let x = uv[0] * f64::from(size[0]);
    let y = uv[1] * f64::from(size[1]);
    if !linear {
        return tap(x.floor() as i64, y.floor() as i64);
    }
    let x = x - 0.5;
    let y = y - 0.5;
    let ix = x.floor() as i64;
    let iy = y.floor() as i64;
    let fx = x - ix as f64;
    let fy = y - iy as f64;
    (tap(ix, iy) * (1. - fx) + tap(ix + 1, iy) * fx) * (1. - fy)
        + (tap(ix, iy + 1) * (1. - fx) + tap(ix + 1, iy + 1) * fx) * fy
}
/// Sixteen equal-area subtriangle centroids; geometric multiplicity is retained.
fn barycentrics() -> Vec<[f64; 3]> {
    let mut out = Vec::with_capacity(16);
    for i in 0..4 {
        for j in 0..4 - i {
            let corners = [(i, j), (i + 1, j), (i, j + 1)];
            let u = corners.iter().map(|p| p.0 as f64).sum::<f64>() / 12.;
            let v = corners.iter().map(|p| p.1 as f64).sum::<f64>() / 12.;
            out.push([1. - u - v, u, v]);
            if i + j < 3 {
                let corners = [(i + 1, j), (i + 1, j + 1), (i, j + 1)];
                let u = corners.iter().map(|p| p.0 as f64).sum::<f64>() / 12.;
                let v = corners.iter().map(|p| p.1 as f64).sum::<f64>() / 12.;
                out.push([1. - u - v, u, v]);
            }
        }
    }
    out
}
fn samples(
    primitive: &PreparedPrimitive,
    b: &Binding,
    lattice: &LatticePlan,
    depth: u8,
    source: &[u8],
    source_size: [u32; 2],
    (factor, cutoff): (f64, f64),
) -> Result<Vec<SurfaceSample>, JobError> {
    let points: Vec<_> = primitive
        .triangles
        .iter()
        .flat_map(|t| t.triangle.positions)
        .collect();
    let ells: Vec<_> = primitive.corner_depths.iter().flatten().copied().collect();
    let geometry = lattice.encode(&points, &ells, u32::from(depth))?;
    let mut samples = Vec::new();
    samples
        .try_reserve_exact(
            primitive
                .triangles
                .len()
                .checked_mul(16)
                .ok_or_else(resource)?,
        )
        .map_err(|_| resource())?;
    let linear = b.sampler.mag_filter == 9729;
    for (index, t) in primitive.triangles.iter().enumerate() {
        let p = &geometry.decoded[index * 3..index * 3 + 3];
        let a: [f64; 3] = std::array::from_fn(|i| f64::from(p[1][i]) - f64::from(p[0][i]));
        let c: [f64; 3] = std::array::from_fn(|i| f64::from(p[2][i]) - f64::from(p[0][i]));
        let cross = [
            a[1] * c[2] - a[2] * c[1],
            a[2] * c[0] - a[0] * c[2],
            a[0] * c[1] - a[1] * c[0],
        ];
        let area = cross.iter().map(|x| x * x).sum::<f64>().sqrt() * 0.5;
        if area == 0. {
            continue;
        }
        for weights in barycentrics() {
            let uv = std::array::from_fn(|axis| {
                (0..3)
                    .map(|i| weights[i] * f64::from(t.corners[i].texcoords[b.source_uv][axis]))
                    .sum()
            });
            let output = std::array::from_fn(|axis| {
                (0..3)
                    .map(|i| {
                        weights[i] * f64::from(primitive.uvs[b.output_uv][index * 3 + i][axis])
                    })
                    .sum()
            });
            let alpha = if t.flags & COLOR != 0 {
                (0..3)
                    .map(|i| weights[i] * f64::from(t.corners[i].color[3]))
                    .sum()
            } else {
                1.
            };
            samples.push(SurfaceSample {
                uv: output,
                alpha,
                weight: area / 16.,
                source_pass: source_alpha(source, source_size, uv, b.sampler, linear)
                    * alpha
                    * factor
                    >= cutoff,
            });
        }
    }
    Ok(samples)
}

/// Finite isotropic surface-footprint reference in the actual decoded triangle frame.
fn sampler_samples(
    primitive: &PreparedPrimitive,
    binding: &Binding,
    lattice: &LatticePlan,
    depth: u8,
    source_size: [u32; 2],
    page_size: [u32; 2],
) -> Result<Vec<SamplerSample>, JobError> {
    let points: Vec<_> = primitive
        .triangles
        .iter()
        .flat_map(|t| t.triangle.positions)
        .collect();
    let depths: Vec<_> = primitive.corner_depths.iter().flatten().copied().collect();
    let geometry = lattice.encode(&points, &depths, u32::from(depth))?;
    let mut locations = barycentrics();
    locations.extend([
        [1., 0., 0.],
        [0., 1., 0.],
        [0., 0., 1.],
        [0.5, 0.5, 0.],
        [0., 0.5, 0.5],
        [0.5, 0., 0.5],
    ]);
    let mut result = Vec::new();
    result
        .try_reserve_exact(
            primitive
                .triangles
                .len()
                .checked_mul(locations.len())
                .ok_or_else(resource)?,
        )
        .map_err(|_| resource())?;
    for (index, triangle) in primitive.triangles.iter().enumerate() {
        let p = &geometry.decoded[index * 3..index * 3 + 3];
        let edge1: [f64; 3] = std::array::from_fn(|a| f64::from(p[1][a]) - f64::from(p[0][a]));
        let edge2: [f64; 3] = std::array::from_fn(|a| f64::from(p[2][a]) - f64::from(p[0][a]));
        let first_length = edge1.iter().map(|v| v * v).sum::<f64>().sqrt();
        if first_length == 0. {
            continue;
        }
        let projection = edge1.iter().zip(edge2).map(|(a, b)| a * b).sum::<f64>() / first_length;
        let cross = [
            edge1[1] * edge2[2] - edge1[2] * edge2[1],
            edge1[2] * edge2[0] - edge1[0] * edge2[2],
            edge1[0] * edge2[1] - edge1[1] * edge2[0],
        ];
        let cross_length = cross.iter().map(|v| v * v).sum::<f64>().sqrt();
        if cross_length == 0. {
            continue;
        }
        let height = cross_length / first_length;
        let source_uv: [[f64; 2]; 3] = std::array::from_fn(|c| {
            triangle.corners[c].texcoords[binding.source_uv].map(f64::from)
        });
        let page_uv: [[f64; 2]; 3] =
            std::array::from_fn(|c| primitive.uvs[binding.output_uv][index * 3 + c].map(f64::from));
        let rho = |uv: [[f64; 2]; 3], size: [u32; 2]| {
            let du: [f64; 2] = std::array::from_fn(|a| (uv[1][a] - uv[0][a]) / first_length);
            let dv: [f64; 2] =
                std::array::from_fn(|a| (uv[2][a] - uv[0][a] - projection * du[a]) / height);
            du.iter()
                .zip(size)
                .map(|(v, n)| (v * f64::from(n)).powi(2))
                .sum::<f64>()
                .sqrt()
                .max(
                    dv.iter()
                        .zip(size)
                        .map(|(v, n)| (v * f64::from(n)).powi(2))
                        .sum::<f64>()
                        .sqrt(),
                )
        };
        let original_rho = rho(source_uv, source_size);
        let page_rho = rho(page_uv, page_size);
        let bias = if original_rho == 0. && page_rho == 0. {
            0.
        } else if original_rho == 0. || page_rho == 0. {
            return Err(error(
                JobErrorKind::Unsupported,
                "atlas changed zero UV Jacobian",
            ));
        } else {
            (original_rho / page_rho).log2()
        };
        if !bias.is_finite() {
            return Err(invariant());
        }
        for weights in &locations {
            result.push(SamplerSample {
                source_uv: std::array::from_fn(|a| {
                    (0..3).map(|c| weights[c] * source_uv[c][a]).sum()
                }),
                page_uv: std::array::from_fn(|a| (0..3).map(|c| weights[c] * page_uv[c][a]).sum()),
                weight: cross_length * 0.5 / locations.len() as f64,
                source_lod_bias: bias,
            });
        }
    }
    Ok(result)
}

#[derive(Clone, Copy)]
pub(in crate::mesh_large) struct AtlasResources<'a> {
    pub materials: &'a [source::Material],
    pub textures: &'a [Value],
    pub samplers: &'a [Value],
    pub image_info: &'a [source::ImageInfo],
}
pub(super) struct LeafProjection {
    pub pages: u64,
    pub base_texels: u64,
    pub chain_texels: u64,
    pub geometry_bytes: u64,
    pub owned_bytes: u64,
}
struct Layout {
    primitives: Vec<PreparedPrimitive>,
    bindings: Vec<Binding>,
    masked: Vec<bool>,
    pages: Vec<PagePlan>,
}
/// Shares the exact physical crop/packing representation with final preparation.
/// Counts planning owners and raw streams; it does not predict compressed file size.
pub(super) fn project(
    triangles: &[FullTriangle],
    resources: AtlasResources<'_>,
    profile: Profile,
    drop: u32,
    check: &mut dyn FnMut() -> Result<(), JobError>,
) -> Result<LeafProjection, JobError> {
    let layout = layout(
        (triangles.to_vec(), vec![[0; 3]; triangles.len()]),
        resources,
        profile,
        drop,
        check,
    )?;
    let mut base = 0u64;
    let mut chain = 0u64;
    let mut geometry = 0u64;
    // Fixed admitted schemas: at most four channel metric objects, normal,
    // MASK and guard objects per level. Reserve 256 bytes per field/container
    // for at most 128 entries, plus page/binding/material owners separately.
    // This is an allocation lease, not a serialized byte-budget estimate.
    let max_levels = if profile == Profile::Lossless {
        1
    } else if profile == Profile::Fast {
        12
    } else {
        13
    };
    // One page per binding also bounds any later byte-budget density retry.
    let mut receipt_owners = (layout.bindings.len() as u64)
        .checked_mul((64 << 10) + max_levels * (32 << 10))
        .ok_or_else(resource)?;
    for page in &layout.pages {
        let (mut w, mut h) = (u64::from(page.width), u64::from(page.height));
        base = base
            .checked_add(w.checked_mul(h).ok_or_else(resource)?)
            .ok_or_else(resource)?;
        loop {
            chain = chain
                .checked_add(w.checked_mul(h).ok_or_else(resource)?)
                .ok_or_else(resource)?;
            if profile == Profile::Lossless || (w == 1 && h == 1) {
                break;
            }
            w = (w / 2).max(1);
            h = (h / 2).max(1);
        }
    }
    for primitive in &layout.primitives {
        receipt_owners = receipt_owners.checked_add(64 << 10).ok_or_else(resource)?;
        let flags = primitive.triangles[0].flags;
        let stride = (if matches!(profile, Profile::Fast | Profile::Balanced) {
            8
        } else {
            12
        }) + 12 * u64::from(flags & super::super::attributes::NORMAL != 0)
            + 16 * u64::from(flags & super::super::attributes::TANGENT != 0)
            + 16 * u64::from(flags & COLOR != 0)
            + 8 * primitive.uvs.len() as u64
            + 8;
        let count = primitive.triangles.len() as u64;
        let samples = count
            .checked_mul(primitive.uvs.len() as u64)
            .and_then(|n| n.checked_mul(22 * std::mem::size_of::<SamplerSample>() as u64))
            .ok_or_else(resource)?;
        receipt_owners = receipt_owners.checked_add(samples).ok_or_else(resource)?;
        let bytes = count
            .checked_mul(3 * stride + 58)
            .and_then(|n| n.checked_add(24))
            .ok_or_else(resource)?;
        geometry = geometry.checked_add(bytes).ok_or_else(resource)?;
    }
    let rows = (triangles.len() as u64)
        .checked_mul(std::mem::size_of::<FullTriangle>() as u64 + 3)
        .ok_or_else(resource)?;
    let uv = layout
        .primitives
        .iter()
        .try_fold(0u64, |n, p| {
            n.checked_add((p.triangles.len() as u64).checked_mul(24 * p.uvs.len() as u64)?)
        })
        .ok_or_else(resource)?;
    let metadata = (layout.bindings.len() as u64)
        .checked_mul(std::mem::size_of::<Binding>() as u64)
        .and_then(|n| {
            n.checked_add(
                layout.primitives.len() as u64 * std::mem::size_of::<PreparedPrimitive>() as u64,
            )
        })
        .and_then(|n| {
            n.checked_add(layout.pages.len() as u64 * std::mem::size_of::<PagePlan>() as u64)
        })
        .ok_or_else(resource)?;
    let owned = rows
        .checked_add(uv)
        .and_then(|n| n.checked_add(metadata))
        .and_then(|n| n.checked_add(receipt_owners))
        .and_then(|n| n.checked_add((layout.bindings.len() as u64).checked_mul(16 << 10)?))
        .ok_or_else(resource)?;
    Ok(LeafProjection {
        pages: layout.pages.len() as u64,
        base_texels: base,
        chain_texels: chain,
        geometry_bytes: geometry,
        owned_bytes: owned,
    })
}
fn layout(
    geometry: (Vec<FullTriangle>, Vec<[u8; 3]>),
    resources: AtlasResources<'_>,
    profile: Profile,
    drop: u32,
    check: &mut dyn FnMut() -> Result<(), JobError>,
) -> Result<Layout, JobError> {
    let mut primitives = triangles_by_primitive(geometry.0, geometry.1)?;
    if drop > 12 || (profile == Profile::Lossless && drop != 0) {
        return Err(error(
            JobErrorKind::InvalidRequest,
            "invalid leaf density reduction",
        ));
    }
    let max_edge = match profile {
        Profile::Fast => 2048,
        Profile::Lossless => 8192,
        _ => 4096,
    };
    let mut bindings = Vec::new();
    let mut masked = vec![false; primitives.len()];
    for (pi, p) in primitives.iter_mut().enumerate() {
        check()?;
        let id = p.triangles[0].material;
        if id == u32::MAX {
            continue;
        }
        let material = resources.materials.get(id as usize).ok_or_else(invariant)?;
        masked[pi] = material.mask().is_some();
        for (output_uv, b) in material.bindings().enumerate() {
            let texture = resources.textures.get(b.texture).ok_or_else(invariant)?;
            let image = texture["source"]
                .as_u64()
                .and_then(|v| usize::try_from(v).ok())
                .ok_or_else(invariant)?;
            let sampler_id = texture
                .get("sampler")
                .and_then(Value::as_u64)
                .and_then(|v| usize::try_from(v).ok());
            let sampler = crops::resolve(
                sampler_id
                    .map(|i| resources.samplers.get(i).ok_or_else(invariant))
                    .transpose()?,
            )?;
            let info = resources.image_info.get(image).ok_or_else(invariant)?;
            let size = [(info.width >> drop).max(1), (info.height >> drop).max(1)];
            let uvs: Vec<_> = p
                .triangles
                .iter()
                .flat_map(|t| t.corners.iter().map(|c| c.texcoords[b.texcoord]))
                .collect();
            let source_mips =
                profile != Profile::Lossless && matches!(sampler.min_filter, 9984..=9987);
            let crop = if source_mips {
                crops::plan_mip_aligned(&uvs, size[0], size[1], max_edge)?
            } else {
                crops::plan(&uvs, size[0], size[1], max_edge, 2)?
            };
            p.uvs.push(uvs);
            bindings.push(Binding {
                primitive: pi,
                slot: b.slot,
                source_uv: b.texcoord,
                output_uv,
                image,
                sampler,
                source_mips,
                crop,
                source_size: size,
                offset: [0; 2],
                page: 0,
            });
        }
    }
    let plans = pack(&mut bindings, max_edge, &masked)?;
    for b in &bindings {
        let page = &plans[b.page];
        for uv in &mut primitives[b.primitive].uvs[b.output_uv] {
            *uv = crops::remap_uv(
                *uv,
                b.source_size[0],
                b.source_size[1],
                &b.crop,
                b.offset,
                [page.width, page.height],
            )?;
        }
    }
    Ok(Layout {
        primitives,
        bindings,
        masked,
        pages: plans,
    })
}

#[derive(Debug)]
pub(super) enum PrepareError {
    Budget(JobError),
    Failure(JobError),
}
impl From<JobError> for PrepareError {
    fn from(error: JobError) -> Self {
        Self::Failure(error)
    }
}
/// A budget overrun permits the caller's deterministic density schedule.
/// Fidelity failures retain their cause and never trigger a density fallback.
pub(super) fn prepare(
    geometry: (Vec<FullTriangle>, Vec<[u8; 3]>),
    captured: &FullCaptured,
    profile: Profile,
    depth: u8,
    drop: u32,
    memory: u64,
    check: &mut dyn FnMut() -> Result<(), JobError>,
) -> Result<(PreparedAtlas, Vec<Value>), PrepareError> {
    let resources = AtlasResources {
        materials: &captured.materials,
        textures: &captured.textures,
        samplers: &captured.samplers,
        image_info: &captured.image_info,
    };
    let Layout {
        primitives,
        bindings,
        masked,
        pages: plans,
    } = layout(geometry, resources, profile, drop, check)?;
    let mut atlas = PreparedAtlas {
        primitives,
        pages: Vec::new(),
        textures: Vec::new(),
        samplers: Vec::new(),
    };
    let mut receipts = Vec::new();
    let mut encoded_bytes = 0u64;
    for (page_id, page) in plans.iter().enumerate() {
        check()?;
        let delivered = if page.source_mips {
            if page.bindings.len() != 1 {
                return Err(invariant().into());
            }
            let b = &bindings[page.bindings[0]];
            if b.offset != [0, 0] {
                return Err(invariant().into());
            }
            let (info, source) = source_image(captured, b.image, check)?;
            let original_size = [info.width, info.height];
            let class = match b.slot {
                TextureSlot::BaseColor | TextureSlot::Emissive => Class::Colour,
                TextureSlot::Normal => Class::Normal,
                _ => Class::Data,
            };
            let chain = source_sampler_mips::build(
                (&source, original_size),
                &b.crop,
                drop,
                class,
                b.sampler,
                memory,
            )?;
            let source_samples = sampler_samples(
                &atlas.primitives[b.primitive],
                b,
                &captured.lattice,
                depth,
                original_size,
                [page.width, page.height],
            )?;
            let material = atlas.primitives[b.primitive].triangles[0].material;
            let mask_context = if b.slot == TextureSlot::BaseColor && masked[b.primitive] {
                Some(
                    captured.materials[material as usize]
                        .mask()
                        .ok_or_else(invariant)?,
                )
            } else {
                None
            };
            let surface = if let Some((factor, cutoff)) = mask_context {
                samples(
                    &atlas.primitives[b.primitive],
                    b,
                    &captured.lattice,
                    depth,
                    &source,
                    original_size,
                    (factor, cutoff),
                )?
            } else {
                Vec::new()
            };
            std::mem::drop(source);
            let mask = mask_context.map(|(factor, cutoff)| MaskDelivery {
                samples: &surface,
                factor,
                cutoff,
                linear: b.sampler.mag_filter == 9729,
                min_linear: matches!(b.sampler.min_filter, 9729 | 9985 | 9987),
            });
            let context = SamplerCheck {
                samples: &source_samples,
                sampler: b.sampler,
                slot: b.slot,
                profile,
                density_drop: drop,
                mask: mask.is_some(),
            };
            let planned = source_sampler_mips::check_authored(&chain, &context)?;
            receipts.push(json!({"page":page_id,"primitive":b.primitive,"slot":role(b.slot),"sourceImage":b.image,"sourceUV":b.source_uv,"outputUV":b.output_uv,"densityDrop":drop,"cropOrigin":b.crop.origin,"cropSize":[b.crop.width,b.crop.height],"pageOffset":b.offset,"sourceSampler":{"wrapS":b.sampler.wrap_s,"wrapT":b.sampler.wrap_t,"minFilter":b.sampler.min_filter,"magFilter":b.sampler.mag_filter},"mipConstruction":"dedicated original-source class levels","samplerCheckBeforeEncode":planned,"samplerCheckScope":"22 locations per nondegenerate decoded triangle; finite isotropic surface-footprint integer/quarter LODs plus magnification; MASK alpha separate"}));
            delivery::encode_authored(&chain.page, b.slot, profile, memory, mask, |decoded| {
                let result = source_sampler_mips::check_samples(&chain, decoded, &context)?;
                Ok(
                    json!({"sourceSamplerCheckAfterEncode":result,"sourceChainChargedBytes":chain.charged_bytes,"sourceLODClampedSeparately":true}),
                )
            })?
        } else {
            let pixels = (page.width as usize)
                .checked_mul(page.height as usize)
                .ok_or_else(resource)?;
            if (pixels as u64).checked_mul(8).ok_or_else(resource)? > memory {
                return Err(resource().into());
            }
            let mut rgba = Vec::new();
            rgba.try_reserve_exact(pixels * 4).map_err(|_| resource())?;
            rgba.resize(pixels * 4, 0);
            let mut support = Vec::new();
            support.try_reserve_exact(pixels).map_err(|_| resource())?;
            support.resize(pixels, 0u32);
            let mut surface = Vec::new();
            let mut mask_context = None;
            for &bi in &page.bindings {
                check()?;
                let b = &bindings[bi];
                let (size, source) = density_image(captured, b.image, b.slot, drop, memory, check)?;
                if size != b.source_size {
                    return Err(invariant().into());
                }
                let crop = crops::copy_crop(&source, size[0], size[1], &b.crop, b.sampler, memory)?;
                for y in 0..b.crop.height {
                    let dst = ((b.offset[1] + y) * page.width + b.offset[0]) as usize * 4;
                    let src = y as usize * b.crop.width as usize * 4;
                    rgba[dst..dst + b.crop.width as usize * 4]
                        .copy_from_slice(&crop[src..src + b.crop.width as usize * 4]);
                }
                for y in b.crop.gutter..b.crop.gutter + b.crop.interior_height {
                    for x in b.crop.gutter..b.crop.gutter + b.crop.interior_width {
                        support[((b.offset[1] + y) * page.width + b.offset[0] + x) as usize] = 1;
                    }
                }
                if b.slot == TextureSlot::BaseColor && masked[b.primitive] {
                    let id = atlas.primitives[b.primitive].triangles[0].material;
                    let (factor, cutoff) = captured.materials[id as usize]
                        .mask()
                        .ok_or_else(invariant)?;
                    let (original_size, original) = if drop == 0 {
                        (size, source)
                    } else {
                        let (info, raw) = source_image(captured, b.image, check)?;
                        ([info.width, info.height], raw)
                    };
                    surface.extend(samples(
                        &atlas.primitives[b.primitive],
                        b,
                        &captured.lattice,
                        depth,
                        &original,
                        original_size,
                        (factor, cutoff),
                    )?);
                    mask_context = Some((factor, cutoff, b.sampler.mag_filter == 9729));
                }
                receipts.push(json!({"page":page_id,"primitive":b.primitive,"slot":role(b.slot),"sourceImage":b.image,"sourceUV":b.source_uv,"outputUV":b.output_uv,"densityDrop":drop,"cropOrigin":b.crop.origin,"cropSize":[b.crop.width,b.crop.height],"pageOffset":b.offset,"sourceSampler":{"wrapS":b.sampler.wrap_s,"wrapT":b.sampler.wrap_t,"minFilter":b.sampler.min_filter,"magFilter":b.sampler.mag_filter}}));
            }
            let min_linear = matches!(page.sampler.min_filter, 9729 | 9985 | 9987);
            let mask = mask_context.map(|(factor, cutoff, linear)| MaskDelivery {
                samples: &surface,
                factor,
                cutoff,
                linear,
                min_linear,
            });
            delivery::encode(
                [page.width, page.height],
                &rgba,
                &support,
                page.slot,
                profile,
                memory,
                mask,
            )?
        };
        encoded_bytes = encoded_bytes
            .checked_add(delivered.bytes.len() as u64)
            .ok_or_else(resource)?;
        if encoded_bytes > TILE_BYTES {
            return Err(PrepareError::Budget(error(
                JobErrorKind::Unsupported,
                "encoded leaf pages exceed 96 MiB member budget",
            )));
        }
        receipts.push(json!({"page":page_id,"width":page.width,"height":page.height,"mime":delivered.mime,"levels":delivered.mip_count,"bytes":delivered.bytes.len(),"quality":delivered.receipts}));
        atlas.pages.push(PreparedPage {
            bytes: delivered.bytes,
            mime: delivered.mime,
        });
        atlas.samplers.push(json!({"wrapS":33071,"wrapT":33071,"minFilter":page.sampler.min_filter,"magFilter":page.sampler.mag_filter}));
        atlas.textures.push(if delivered.mime == "image/ktx2" {
            json!({"sampler":page_id,"extensions":{"KHR_texture_basisu":{"source":page_id}}})
        } else {
            json!({"sampler":page_id,"source":page_id})
        });
    }
    for (pi, p) in atlas.primitives.iter_mut().enumerate() {
        let id = p.triangles[0].material;
        if id == u32::MAX {
            continue;
        }
        let patches: Vec<_> = bindings
            .iter()
            .filter(|b| b.primitive == pi)
            .map(|b| (b.slot, b.page, b.output_uv))
            .collect();
        p.material = Some(captured.materials[id as usize].remap_slots(&patches)?);
    }
    Ok((atlas, receipts))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn surface_samples_cover_triangle_without_uv_inverse() {
        let weights = barycentrics();
        assert_eq!(weights.len(), 16);
        for w in &weights {
            assert!((w.iter().sum::<f64>() - 1.).abs() < 1e-14);
            assert!(w.iter().all(|x| *x > 0.));
        }
        for axis in 0..3 {
            assert!((weights.iter().map(|w| w[axis]).sum::<f64>() / 16. - 1. / 3.).abs() < 1e-14);
        }
    }
}
