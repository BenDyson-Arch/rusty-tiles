//! Bounded per-leaf preparation: positive source-pixel maps and owned local closure.
use super::super::{
    attributes::{FullTriangle, COLOR},
    config::{Profile, TILE_BYTES},
    full::FullCaptured,
};
use super::schedule::{Decoded, MemoryGate, SourceCache};
use super::{
    crops::{self, Block, Crop, SourceSampler, CLAMP},
    delivery::{self, MaskDelivery},
    geometry::LatticePlan,
    mask::SurfaceSample,
    mips::{self, Class},
    prepared::{PreparedAtlas, PreparedPage, PreparedPrimitive},
    sparse_mips::LEAF_ISLAND_ISOLATED_LEVELS,
};
use crate::stage_timing::{self as timing, Counter, Stage};
use crate::{
    gltf_source::source::{self, TextureSlot},
    JobError, JobErrorKind,
};
use rayon::prelude::*;
use serde_json::{json, Value};
use std::{collections::BTreeMap, sync::Arc};

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
    block: Block,
    source_size: [u32; 2],
    offset: [u32; 2],
    page: usize,
    /// Axes the binding's UVs span a whole period of ([`period_axes`]).
    period: [bool; 2],
}
struct PagePlan {
    bindings: Vec<usize>,
    slot: TextureSlot,
    sampler: SourceSampler,
    width: u32,
    height: u32,
    /// Published wrap: CLAMP_TO_EDGE, or the source's on a period page's whole axes.
    wrap: [u32; 2],
}

/// Per axis, whether `uvs` span at least one whole period of a repeating source wrap
/// (REPEAT, MIRRORED_REPEAT). Such a binding needs every source texel on that axis
/// (D6), so a page holding the axis whole, sampled with the source wrap at the source
/// UV, replaces copies unrolled period by period.
fn period_axes(uvs: &[[f32; 2]], sampler: SourceSampler) -> [bool; 2] {
    let wraps = [sampler.wrap_s, sampler.wrap_t];
    std::array::from_fn(|a| {
        let (mut lo, mut hi) = (f64::INFINITY, f64::NEG_INFINITY);
        for uv in uvs {
            let v = f64::from(uv[a]);
            if !v.is_finite() || v.abs() > 1e6 {
                return false;
            }
            (lo, hi) = (lo.min(v), hi.max(v));
        }
        matches!(wraps[a], 10497 | 33648) && hi - lo >= 1.
    })
}

/// One dedicated page per source image, slot role, sampler and MASK class of the
/// bindings with a period axis: whole on every axis the members span a period of
/// (published with the source wrap; UVs unchanged there), one crop of their union on
/// the other (context folded into the crop, so it is exact real source with
/// `crop_padding`'s alignment; CLAMP_TO_EDGE). A group returns to island crops when such a
/// page would hold more texels than its members' crops, or cannot be one (a KTX2 edge not
/// a multiple of four, or over `max_edge`).
fn period_pages(
    bindings: &mut [Binding],
    primitives: &[PreparedPrimitive],
    (profile, max_edge, mask_classes): (Profile, u32, &[Option<u32>]),
) -> Result<Vec<PagePlan>, JobError> {
    let mut groups = BTreeMap::<_, Vec<usize>>::new();
    for (i, b) in bindings.iter().enumerate() {
        if b.period.contains(&true) {
            let mask = (b.slot == TextureSlot::BaseColor).then(|| mask_classes[b.primitive]);
            let s = b.sampler;
            let key = (
                b.image,
                role(b.slot),
                [s.wrap_s, s.wrap_t, s.min_filter, s.mag_filter],
            );
            groups.entry((key, mask)).or_default().push(i);
        }
    }
    let mut pages = Vec::new();
    for ids in groups.into_values() {
        let first = &bindings[ids[0]];
        let (size, sampler, slot) = (first.source_size, first.sampler, first.slot);
        let wraps = [sampler.wrap_s, sampler.wrap_t];
        let (gutter, align) = crop_padding(class(slot), profile, sampler);
        let (g, al) = (i64::from(gutter), i64::from(align));
        let mipmapped = profile != Profile::Lossless && matches!(sampler.min_filter, 9984..=9987);
        let uvs = |b: &Binding| &primitives[b.primitive].uvs[b.output_uv];
        let mut members = Vec::new();
        for &i in &ids {
            members.extend_from_slice(uvs(&bindings[i]));
        }
        let whole = period_axes(&members, sampler);
        let (mut origin, mut extent, mut fits) = ([0i64; 2], [0u32; 2], true);
        for a in 0..2 {
            let n = f64::from(size[a]);
            if whole[a] {
                extent[a] = size[a];
                fits &= profile == Profile::Lossless || size[a].is_multiple_of(4);
            } else {
                let lo = members
                    .iter()
                    .map(|uv| f64::from(uv[a]))
                    .fold(f64::INFINITY, f64::min);
                let hi = members
                    .iter()
                    .map(|uv| f64::from(uv[a]))
                    .fold(f64::NEG_INFINITY, f64::max);
                // plan()'s one-texel real-source halo plus the gutter, as real source.
                let mut start = (lo * n).floor() as i64 - 1 - g;
                if mipmapped {
                    start = start.div_euclid(al) * al;
                }
                let span = ((hi * n).ceil() as i64 + 1 + g - start).max(1);
                (origin[a], extent[a]) = (
                    start,
                    u32::try_from((span + al - 1) / al * al).unwrap_or(u32::MAX),
                );
            }
            fits &= extent[a] <= max_edge;
        }
        // The period page only where it is no larger than the island crops the members'
        // actual footprints need (or those crops cannot be represented at all): a bounding
        // range of sparse triangles spans periods without needing them.
        let crops = ids.iter().try_fold(0u64, |n, &i| {
            let b = &bindings[i].block;
            (b.width > 0).then(|| n + u64::from(b.width) * u64::from(b.height))
        });
        let texels = u64::from(extent[0]) * u64::from(extent[1]);
        if !fits || crops.is_some_and(|c| texels > c) {
            for &i in &ids {
                let b = &mut bindings[i];
                b.period = [false; 2];
                if b.block.width == 0 {
                    let uvs = &primitives[b.primitive].uvs[b.output_uv];
                    b.block = crops::plan_islands_aligned(
                        uvs, size[0], size[1], max_edge, gutter, align, mipmapped,
                    )?;
                }
            }
            continue;
        }
        let crop = Crop {
            origin,
            width: extent[0],
            height: extent[1],
            gutter: 0,
            interior_width: extent[0],
            interior_height: extent[1],
        };
        for &i in &ids {
            let triangles = uvs(&bindings[i]).len() / 3;
            let b = &mut bindings[i];
            (b.block, b.offset) = (Block::single(crop.clone(), triangles), [0, 0]);
        }
        pages.push(PagePlan {
            bindings: ids,
            slot,
            sampler,
            width: extent[0],
            height: extent[1],
            wrap: std::array::from_fn(|a| if whole[a] { wraps[a] } else { CLAMP }),
        });
    }
    Ok(pages)
}

/// Shelf height/width of one bucket's blocks laid out with shelf width `limit`.
fn shelf_extent(bindings: &[Binding], ids: &[usize], limit: u32) -> Option<(u32, u32)> {
    let (mut x, mut y, mut row, mut width) = (0u64, 0u64, 0u64, 0u64);
    for &i in ids {
        let b = &bindings[i];
        if u64::from(b.block.width) > u64::from(limit) {
            return None;
        }
        if x + u64::from(b.block.width) > u64::from(limit) {
            x = 0;
            y += row;
            row = 0;
        }
        x += u64::from(b.block.width);
        row = row.max(u64::from(b.block.height));
        width = width.max(x);
    }
    Some((width as u32, u32::try_from(y + row).ok()?))
}

/// Packs every island of one bucket's bindings directly into a single page,
/// when they fit, choosing the power-of-two shelf width that minimises the
/// page area. Bindings then sit at page offset zero and each island carries
/// its page position.
fn pack_islands(
    bindings: &mut [Binding],
    ids: &[usize],
    max_edge: u32,
    page_index: usize,
) -> Result<Option<PagePlan>, JobError> {
    let mut items = Vec::new();
    for &b in ids {
        for (k, island) in bindings[b].block.islands.iter().enumerate() {
            items.push((b, k, island.crop.width, island.crop.height));
        }
    }
    items.sort_by_key(|&(b, k, w, h)| (std::cmp::Reverse(h), std::cmp::Reverse(w), b, k));
    let shelf = |limit: u32| -> Option<(u32, u32, Vec<[u32; 2]>)> {
        let (mut x, mut y, mut row, mut width) = (0u64, 0u64, 0u64, 0u64);
        let mut offsets = Vec::with_capacity(items.len());
        for &(_, _, w, h) in &items {
            let (w, h) = (u64::from(w), u64::from(h));
            if w > u64::from(limit) {
                return None;
            }
            if x + w > u64::from(limit) {
                x = 0;
                y += row;
                row = 0;
            }
            offsets.push([x as u32, y as u32]);
            x += w;
            row = row.max(h);
            width = width.max(x);
        }
        let height = y + row;
        (height <= u64::from(max_edge)).then_some((width as u32, height as u32, offsets))
    };
    let mut best: Option<(u64, u32, u32, Vec<[u32; 2]>)> = None;
    let mut limit = 4u32;
    while limit <= max_edge {
        if let Some((w, h, offsets)) = shelf(limit) {
            let (pw, ph) = (w.max(4).next_power_of_two(), h.max(4).next_power_of_two());
            let area = u64::from(pw) * u64::from(ph);
            if best.as_ref().is_none_or(|b| area < b.0) {
                best = Some((area, pw, ph, offsets));
            }
        }
        limit = match limit.checked_mul(2) {
            Some(next) => next,
            None => break,
        };
    }
    let Some((_, width, height, offsets)) = best else {
        return Ok(None);
    };
    for (&(b, k, _, _), offset) in items.iter().zip(offsets) {
        bindings[b].block.islands[k].offset = offset;
    }
    let first = &bindings[ids[0]];
    let page = PagePlan {
        bindings: ids.to_vec(),
        slot: first.slot,
        sampler: first.sampler,
        width,
        height,
        wrap: [CLAMP; 2],
    };
    for &b in ids {
        let binding = &mut bindings[b];
        binding.page = page_index;
        binding.offset = [0, 0];
        binding.block.width = width;
        binding.block.height = height;
    }
    Ok(Some(page))
}

/// Stable shelves ordered by descending crop dimensions and canonical binding order.
/// When a bucket fits one page, the shelf width is the power of two that
/// minimises the page area; otherwise shelves span the maximum page edge.
///
/// Mipmapped bindings share pages like any other: their islands carry aligned
/// source context, never a whole-image dedicated page (D6). MASK
/// base-colour bindings share pages per material class (H28): one class has one alpha
/// factor and cutoff, so its coverage guard is the same for every binding on the page.
fn pack(
    bindings: &mut [Binding],
    max_edge: u32,
    mask_classes: &[Option<u32>],
) -> Result<Vec<PagePlan>, JobError> {
    let mut buckets = BTreeMap::<(u8, u32, u32, Option<u32>), Vec<usize>>::new();
    for (i, b) in bindings
        .iter()
        .enumerate()
        .filter(|(_, b)| !b.period.contains(&true))
    {
        let mask = if b.slot == TextureSlot::BaseColor {
            mask_classes[b.primitive]
        } else {
            None
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
        if let Some(page) = pack_islands(bindings, &ids, max_edge, pages.len())? {
            pages.push(page);
            continue;
        }
        ids.sort_by_key(|&i| {
            (
                std::cmp::Reverse(bindings[i].block.height),
                std::cmp::Reverse(bindings[i].block.width),
                i,
            )
        });
        let mut limit = max_edge;
        let mut best = u64::MAX;
        let mut candidate = 4u32;
        while candidate <= max_edge {
            if let Some((w, h)) = shelf_extent(bindings, &ids, candidate) {
                if h <= max_edge {
                    let area = u64::from(w.max(4).next_power_of_two())
                        * u64::from(h.max(4).next_power_of_two());
                    if area < best {
                        best = area;
                        limit = candidate;
                    }
                }
            }
            candidate = match candidate.checked_mul(2) {
                Some(next) => next,
                None => break,
            };
        }
        let first = &bindings[ids[0]];
        let mut page = PagePlan {
            bindings: Vec::new(),
            slot: first.slot,
            sampler: first.sampler,
            width: 0,
            height: 0,
            wrap: [CLAMP; 2],
        };
        let (mut x, mut y, mut row) = (0u32, 0u32, 0u32);
        for i in ids {
            let b = &mut bindings[i];
            if x.checked_add(b.block.width).ok_or_else(resource)? > limit {
                x = 0;
                y = y.checked_add(row).ok_or_else(resource)?;
                row = 0;
            }
            if y.checked_add(b.block.height).ok_or_else(resource)? > max_edge {
                page.width = page.width.max(4).next_power_of_two();
                page.height = page.height.max(4).next_power_of_two();
                pages.push(page);
                page = PagePlan {
                    bindings: Vec::new(),
                    slot: b.slot,
                    sampler: b.sampler,
                    width: 0,
                    height: 0,
                    wrap: [CLAMP; 2],
                };
                x = 0;
                y = 0;
                row = 0;
            }
            b.page = pages.len();
            b.offset = [x, y];
            page.bindings.push(i);
            x = x.checked_add(b.block.width).ok_or_else(resource)?;
            row = row.max(b.block.height);
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

/// Gutter and allocation alignment of island crops. Lossy pages align islands
/// to the 4x4 codec block so no block mixes two islands. Lossy mipmapped bindings
/// (complete authored KTX2 chains) put every island's real-source interior on the
/// `2^K` source grid with gutters of `2^K` ([`LEAF_ISLAND_ISOLATED_LEVELS`]): islands are
/// isolated through level K and match the source's own reduction there; deeper levels
/// are a page-wide box filter (bleed accepted, as standard; the HLOD parent replaces the
/// leaf before deep levels dominate). PNG pages filtered with viewer-generated mips carry
/// eight texels of real source context around every island, aligned to eight texels.
/// ID pages (lossless PNG, NEAREST, no runtime mips) keep a two-texel no-feature gutter
/// and align islands to eight texels, so the first three authored mode levels used by
/// coarse proxies never merge two islands.
fn crop_padding(class: Class, profile: Profile, sampler: SourceSampler) -> (u32, u32) {
    if class == Class::Id {
        return (2, 8);
    }
    match profile {
        Profile::Lossless if matches!(sampler.min_filter, 9984..=9987) => (8, 8),
        _ if matches!(sampler.min_filter, 9984..=9987) => {
            let a = 1 << LEAF_ISLAND_ISOLATED_LEVELS;
            (a, a)
        }
        Profile::Lossless => (2, 1),
        _ => (2, 4),
    }
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
            feature_id_textures: Vec::new(),
        })
        .collect())
}

fn source_image(
    services: &Services<'_>,
    index: usize,
    check: &mut dyn FnMut() -> Result<(), JobError>,
) -> Result<Arc<Decoded>, JobError> {
    services.cache.get((index, 0), || {
        let snapshot = services.captured.images.get(index).ok_or_else(invariant)?;
        let encoded = timing::time(Stage::SourceReconstruct, || snapshot.reconstruct(check))?;
        let (info, rgba) = timing::time(Stage::SourceDecode, || {
            source::decode_image_admitted(
                &encoded,
                snapshot.identity.mime.as_deref(),
                source::DecodeAdmission::LARGE_MESH,
                &mut || check(),
            )
        })?;
        Ok(Decoded {
            size: [info.width, info.height],
            rgba,
        })
    })
}

fn density_image(
    services: &Services<'_>,
    index: usize,
    slot: TextureSlot,
    drop: u32,
    check: &mut dyn FnMut() -> Result<(), JobError>,
) -> Result<Arc<Decoded>, JobError> {
    if drop == 0 {
        return source_image(services, index, check);
    }
    let original = source_image(services, index, check)?;
    // The density level depends on the image's class; key it by slot class.
    let class_key = match class(slot) {
        Class::Colour => 0,
        Class::Normal => 1,
        Class::Data => 2,
        Class::Id => 3,
    };
    services.cache.get((index, drop * 4 + class_key), || {
        let memory = usize::try_from(services.memory).map_err(|_| resource())?;
        let mut chain = timing::time(Stage::DensityMips, || {
            mips::build(
                original.size[0],
                original.size[1],
                original.rgba.clone(),
                class(slot),
                None,
                memory,
            )
        })?;
        let index = (drop as usize).min(chain.levels.len() - 1);
        let level = chain.levels.swap_remove(index);
        Ok(Decoded {
            size: [level.width, level.height],
            rgba: level.rgba,
        })
    })
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

#[derive(Clone, Copy)]
pub(in crate::mesh_large) struct AtlasResources<'a> {
    pub materials: &'a [source::Material],
    /// H28 material classes of `materials`.
    pub classes: &'a crate::mesh_large::batching::MaterialClasses,
    pub textures: &'a [Value],
    pub samplers: &'a [Value],
    pub image_info: &'a [source::ImageInfo],
}
pub(super) struct LeafProjection {
    pub pages: u64,
    /// The largest page workspace lease of this leaf ([`page_charge`]) at its density
    /// drop or any later byte-budget retry (smaller pages; decode charged as a retry).
    pub page_bytes: u64,
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
    let mut page_bytes = 0u64;
    for page in &layout.pages {
        // A later byte-budget retry only shrinks pages, but a density drop charges the
        // larger decode transient: lease as if dropped.
        page_bytes = page_bytes.max(page_charge(
            page,
            &layout.bindings,
            resources.image_info,
            profile,
            drop.max(1),
        )?);
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
        page_bytes,
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
    let mut mask_classes = vec![None; primitives.len()];
    for (pi, p) in primitives.iter_mut().enumerate() {
        check()?;
        let id = p.triangles[0].material;
        if id == u32::MAX {
            continue;
        }
        let material = resources.materials.get(id as usize).ok_or_else(invariant)?;
        masked[pi] = material.mask().is_some();
        mask_classes[pi] = masked[pi].then(|| resources.classes.of(id));
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
            let (gutter, align) = crop_padding(class(b.slot), profile, sampler);
            // Mipmapped lossy bindings: interiors on the alignment grid (D6).
            let mipmapped =
                profile != Profile::Lossless && matches!(sampler.min_filter, 9984..=9987);
            let period = period_axes(&uvs, sampler);
            let block = match crops::plan_islands_aligned(
                &uvs, size[0], size[1], max_edge, gutter, align, mipmapped,
            ) {
                Ok(block) => block,
                // Unrepresentable as crops: `period_pages` may still place it whole.
                Err(e) if period.contains(&true) && e.kind() == JobErrorKind::Unsupported => {
                    Block {
                        width: 0,
                        height: 0,
                        islands: Vec::new(),
                        triangle_island: Vec::new(),
                    }
                }
                Err(e) => return Err(e),
            };
            p.uvs.push(uvs);
            bindings.push(Binding {
                primitive: pi,
                slot: b.slot,
                source_uv: b.texcoord,
                output_uv,
                image,
                sampler,
                block,
                source_size: size,
                offset: [0; 2],
                page: 0,
                period,
            });
        }
    }
    let dedicated = period_pages(
        &mut bindings,
        &primitives,
        (profile, max_edge, &mask_classes),
    )?;
    let mut plans = pack(&mut bindings, max_edge, &mask_classes)?;
    for page in dedicated {
        for &b in &page.bindings {
            bindings[b].page = plans.len();
        }
        plans.push(page);
    }
    for b in &bindings {
        let page = &plans[b.page];
        for (corner, uv) in primitives[b.primitive].uvs[b.output_uv]
            .iter_mut()
            .enumerate()
        {
            let island = b
                .block
                .triangle_island
                .get(corner / 3)
                .and_then(|&i| b.block.islands.get(i as usize))
                .ok_or_else(invariant)?;
            let offset = [
                b.offset[0]
                    .checked_add(island.offset[0])
                    .ok_or_else(resource)?,
                b.offset[1]
                    .checked_add(island.offset[1])
                    .ok_or_else(resource)?,
            ];
            *uv = crops::remap_uv(
                *uv,
                b.source_size[0],
                b.source_size[1],
                &island.crop,
                offset,
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
/// Shared, thread-safe delivery services for one producer run.
pub(super) struct Services<'a> {
    pub captured: &'a FullCaptured,
    pub cache: &'a SourceCache,
    pub gate: &'a MemoryGate,
    /// Declared per-page workspace limit for internal allocation checks.
    pub memory: u64,
    pub check: &'a (dyn Fn() -> Result<(), JobError> + Sync),
}

/// Bytes leased from the memory gate for one page: page assembly, the
/// delivery codec workspace and, for each distinct source image it reads, the
/// decode transient (encoded snapshot, decoder output and RGBA conversion).
/// Decoded images themselves are owned by the separately budgeted cache.
fn page_charge(
    page: &PagePlan,
    bindings: &[Binding],
    image_info: &[source::ImageInfo],
    profile: Profile,
    drop: u32,
) -> Result<u64, JobError> {
    let pixels = u64::from(page.width)
        .checked_mul(u64::from(page.height))
        .ok_or_else(resource)?;
    let codec = delivery::workspace_charge([page.width, page.height], profile)?;
    // RGBA and support, plus island labels and their reduced chain on mipmapped pages.
    let mipmapped = profile != Profile::Lossless && matches!(page.sampler.min_filter, 9984..=9987);
    let assembly = pixels
        .checked_mul(if mipmapped { 18 } else { 8 })
        .ok_or_else(resource)?;
    let mut decode = 0u64;
    for &bi in &page.bindings {
        let info = image_info.get(bindings[bi].image).ok_or_else(invariant)?;
        let texels = u64::from(info.width) * u64::from(info.height);
        // Encoded bytes (<=4/texel), decoder RGB(A) output and RGBA8 copy;
        // a density drop also builds the full-resolution class chain.
        let per_texel = if drop > 0 { 32 } else { 12 };
        decode = decode.max(texels.checked_mul(per_texel).ok_or_else(resource)?);
    }
    codec
        .checked_add(assembly)
        .and_then(|n| n.checked_add(decode))
        .ok_or_else(resource)
}

/// Every island's source-to-page mapping: interior source origin, interior
/// size and the page texel where that interior starts. The allocation adds
/// `islandGutter` texels of context on each side.
fn island_receipts(b: &Binding) -> Value {
    Value::Array(
        b.block
            .islands
            .iter()
            .map(|island| {
                let c = &island.crop;
                json!([
                    c.origin[0],
                    c.origin[1],
                    c.interior_width,
                    c.interior_height,
                    b.offset[0] + island.offset[0] + c.gutter,
                    b.offset[1] + island.offset[1] + c.gutter
                ])
            })
            .collect(),
    )
}

struct EncodedPage {
    delivered: delivery::Delivery,
    binding_receipts: Vec<Value>,
}

/// glTF material property of a texture slot, for diagnostics.
pub(super) fn slot_property(slot: TextureSlot) -> &'static str {
    match slot {
        TextureSlot::BaseColor => "baseColorTexture",
        TextureSlot::MetallicRoughness => "metallicRoughnessTexture",
        TextureSlot::Normal => "normalTexture",
        TextureSlot::Occlusion => "occlusionTexture",
        TextureSlot::Emissive => "emissiveTexture",
    }
}
/// A page that left its profile's encoder setting, with what it was cut from.
#[derive(Clone, Debug)]
pub(super) struct PageEscalation {
    pub page: usize,
    pub slot: TextureSlot,
    /// Distinct (source image, source material) pairs on the page, in binding order.
    pub sources: Vec<(usize, u32)>,
    pub escalation: delivery::Escalation,
    /// Delivered as exact level-0 PNG under a mipmap sampler: the viewer generates its
    /// coarser levels, outside the decoded guarantee.
    pub viewer_mips: bool,
}
/// A leaf's appearance receipts and its escalated pages, in page order.
#[derive(Debug, Default)]
pub(super) struct Appearance {
    pub receipts: Vec<Value>,
    pub escalations: Vec<PageEscalation>,
}
fn page_sources(layout: &Layout, page: &PagePlan) -> Vec<(usize, u32)> {
    let mut sources = Vec::new();
    for &bi in &page.bindings {
        let b = &layout.bindings[bi];
        let material = layout.primitives[b.primitive].triangles[0].material;
        if !sources.contains(&(b.image, material)) {
            sources.push((b.image, material));
        }
    }
    sources
}
/// Names the page, slot and sources in any page failure except cancellation.
fn page_error(
    error: JobError,
    page_id: usize,
    slot: TextureSlot,
    sources: &[(usize, u32)],
) -> JobError {
    if error.kind() == JobErrorKind::Cancelled {
        return error;
    }
    let sources: Vec<String> = sources
        .iter()
        .map(|(image, material)| format!("image {image} material {material}"))
        .collect();
    JobError::with_cause(
        error.kind(),
        format!(
            "page {page_id} {} (source {}): {error}",
            slot_property(slot),
            sources.join(", ")
        ),
        error,
    )
}

#[allow(clippy::too_many_arguments)]
fn encode_page(
    services: &Services<'_>,
    layout: &Layout,
    page: &PagePlan,
    page_id: usize,
    profile: Profile,
    depth: u8,
    drop: u32,
) -> Result<EncodedPage, JobError> {
    let captured = services.captured;
    let memory = services.memory;
    let check = &mut || (services.check)();
    check()?;
    let ladder = delivery::Ladder {
        check: services.check,
    };
    let _permit = services.gate.acquire(page_charge(
        page,
        &layout.bindings,
        &captured.image_info,
        profile,
        drop,
    )?)?;
    let bindings = &layout.bindings;
    let primitives = &layout.primitives;
    let masked = &layout.masked;
    let mut receipts = Vec::new();
    let delivered = {
        let pixels = (page.width as usize)
            .checked_mul(page.height as usize)
            .ok_or_else(resource)?;
        if (pixels as u64).checked_mul(8).ok_or_else(resource)? > memory {
            return Err(resource());
        }
        let mut rgba = Vec::new();
        rgba.try_reserve_exact(pixels * 4).map_err(|_| resource())?;
        rgba.resize(pixels * 4, 0);
        let mut support = Vec::new();
        support.try_reserve_exact(pixels).map_err(|_| resource())?;
        support.resize(pixels, 0u32);
        // Mipmapped lossy pages keep their islands apart in every reduced level.
        let mipmapped =
            profile != Profile::Lossless && matches!(page.sampler.min_filter, 9984..=9987);
        let mut labels = if mipmapped {
            vec![0u32; pixels]
        } else {
            Vec::new()
        };
        let mut next_label = 0u32;
        let mut surface = Vec::new();
        let mut mask_context = None;
        for &bi in &page.bindings {
            check()?;
            let b = &bindings[bi];
            let source = density_image(services, b.image, b.slot, drop, check)?;
            let size = source.size;
            if size != b.source_size {
                return Err(invariant());
            }
            // A period page's members share one crop: cut it once.
            let cut = page.wrap == [CLAMP; 2] || bi == page.bindings[0];
            timing::time(Stage::PageCut, || -> Result<(), JobError> {
                for island in b.block.islands.iter().filter(|_| cut) {
                    let crop = &island.crop;
                    let [ox, oy] = [
                        b.offset[0] + island.offset[0],
                        b.offset[1] + island.offset[1],
                    ];
                    let copied =
                        crops::copy_crop(&source.rgba, size[0], size[1], crop, b.sampler, memory)?;
                    let row = crop.width as usize * 4;
                    for y in 0..crop.height {
                        let dst = ((oy + y) * page.width + ox) as usize * 4;
                        let src = y as usize * row;
                        rgba[dst..dst + row].copy_from_slice(&copied[src..src + row]);
                    }
                    // Mipmapped islands: the whole allocation is real source under the
                    // sampler's wrap (copy_crop), so every level through K is the
                    // area-weighted source average over its footprint, gutters included.
                    let (from, rows, cols) = if mipmapped {
                        (0, crop.height, crop.width)
                    } else {
                        (crop.gutter, crop.interior_height, crop.interior_width)
                    };
                    for y in from..from + rows {
                        let first = ((oy + y) * page.width + ox + from) as usize;
                        support[first..first + cols as usize].fill(1);
                    }
                    if mipmapped {
                        next_label += 1;
                        for y in 0..crop.height {
                            let first = ((oy + y) * page.width + ox) as usize;
                            labels[first..first + crop.width as usize].fill(next_label);
                        }
                    }
                }
                Ok(())
            })?;
            timing::count(Counter::CropInteriorTexels, b.block.interior_texels());
            if b.slot == TextureSlot::BaseColor && masked[b.primitive] {
                let id = primitives[b.primitive].triangles[0].material;
                let (factor, cutoff) = captured.materials[id as usize]
                    .mask()
                    .ok_or_else(invariant)?;
                let original = if drop == 0 {
                    source
                } else {
                    source_image(services, b.image, check)?
                };
                surface.extend(samples(
                    &primitives[b.primitive],
                    b,
                    &captured.lattice,
                    depth,
                    &original.rgba,
                    original.size,
                    (factor, cutoff),
                )?);
                mask_context = Some((factor, cutoff, b.sampler.mag_filter == 9729));
            }
            receipts.push(json!({"page":page_id,"primitive":b.primitive,"slot":role(b.slot),"sourceImage":b.image,"sourceUV":b.source_uv,"outputUV":b.output_uv,"densityDrop":drop,"islands":island_receipts(b),"islandGutter":b.block.islands[0].crop.gutter,"islandInteriorTexels":b.block.interior_texels(),"pageOffset":b.offset,"sourceSampler":{"wrapS":b.sampler.wrap_s,"wrapT":b.sampler.wrap_t,"minFilter":b.sampler.min_filter,"magFilter":b.sampler.mag_filter}}));
        }
        let min_linear = matches!(page.sampler.min_filter, 9729 | 9985 | 9987);
        let mask = mask_context.map(|(factor, cutoff, linear)| MaskDelivery {
            samples: &surface,
            factor,
            cutoff,
            linear,
            min_linear,
            wrap: page.wrap,
        });
        // The published sampler is the source's: a chain only where it mipmaps.
        if mipmapped {
            delivery::encode_islands(
                [page.width, page.height],
                &rgba,
                &support,
                Some(&labels),
                LEAF_ISLAND_ISOLATED_LEVELS,
                page.slot,
                profile,
                memory,
                mask,
                ladder,
            )?
        } else {
            delivery::encode_base(
                [page.width, page.height],
                &rgba,
                &support,
                page.slot,
                profile,
                memory,
                mask,
                ladder,
            )?
        }
    };
    timing::count(Counter::Pages, 1);
    timing::count(
        Counter::PageBaseTexels,
        u64::from(page.width) * u64::from(page.height),
    );
    timing::count(Counter::EncodedBytes, delivered.bytes.len() as u64);
    Ok(EncodedPage {
        delivered,
        binding_receipts: receipts,
    })
}

/// A budget overrun permits the caller's deterministic density schedule.
/// Fidelity failures retain their cause and never trigger a density fallback.
/// Pages encode in parallel; results, receipts and the first failure are
/// taken in page order, so output and diagnostics do not depend on jobs.
pub(super) fn prepare(
    geometry: (Vec<FullTriangle>, Vec<[u8; 3]>),
    services: &Services<'_>,
    profile: Profile,
    depth: u8,
    drop: u32,
) -> Result<(PreparedAtlas, Appearance), PrepareError> {
    let captured = services.captured;
    let resources = AtlasResources {
        materials: &captured.materials,
        classes: &captured.material_classes,
        textures: &captured.textures,
        samplers: &captured.samplers,
        image_info: &captured.image_info,
    };
    let layout = timing::time(Stage::Layout, || {
        layout(geometry, resources, profile, drop, &mut || {
            (services.check)()
        })
    })?;
    let sources: Vec<_> = layout
        .pages
        .iter()
        .map(|page| page_sources(&layout, page))
        .collect();
    let encoded: Vec<Result<EncodedPage, JobError>> = layout
        .pages
        .par_iter()
        .enumerate()
        .map(|(page_id, page)| {
            encode_page(services, &layout, page, page_id, profile, depth, drop)
                .map_err(|e| page_error(e, page_id, page.slot, &sources[page_id]))
        })
        .collect();
    let Layout {
        primitives,
        bindings,
        pages: plans,
        ..
    } = layout;
    let mut atlas = PreparedAtlas {
        primitives,
        pages: Vec::new(),
        textures: Vec::new(),
        samplers: Vec::new(),
    };
    let mut receipts = Vec::new();
    let mut escalations = Vec::new();
    let mut encoded_bytes = 0u64;
    for (page_id, ((page, result), sources)) in plans.iter().zip(encoded).zip(sources).enumerate() {
        let EncodedPage {
            delivered,
            binding_receipts,
        } = result?;
        receipts.extend(binding_receipts);
        let escalation = delivered.escalation.as_ref().map(|e| {
            json!({"step":e.step.name(),"misses":e.misses.iter().map(delivery::GateMiss::to_json).collect::<Vec<_>>()})
        });
        if let Some(escalation) = delivered.escalation.clone() {
            let viewer_mips = escalation.step == delivery::Step::LosslessPng
                && matches!(page.sampler.min_filter, 9984..=9987);
            escalations.push(PageEscalation {
                page: page_id,
                slot: page.slot,
                sources,
                escalation,
                viewer_mips,
            });
        }
        encoded_bytes = encoded_bytes
            .checked_add(delivered.bytes.len() as u64)
            .ok_or_else(resource)?;
        if encoded_bytes > TILE_BYTES {
            return Err(PrepareError::Budget(error(
                JobErrorKind::Unsupported,
                "encoded leaf pages exceed 96 MiB member budget",
            )));
        }
        receipts.push(json!({"page":page_id,"width":page.width,"height":page.height,"mime":delivered.mime,"levels":delivered.mip_count,"bytes":delivered.bytes.len(),"quality":delivered.receipts,"escalation":escalation}));
        atlas.pages.push(PreparedPage {
            bytes: delivered.bytes,
            mime: delivered.mime,
        });
        atlas.samplers.push(json!({"wrapS":page.wrap[0],"wrapT":page.wrap[1],"minFilter":page.sampler.min_filter,"magFilter":page.sampler.mag_filter}));
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
    Ok((
        atlas,
        Appearance {
            receipts,
            escalations,
        },
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn whole_period_axes_keep_source_uvs_bit_for_bit() {
        let crop = Crop {
            origin: [0, -12],
            width: 1000,
            height: 40,
            gutter: 0,
            interior_width: 1000,
            interior_height: 40,
        };
        let mut x = 0x2545_f491u32;
        for _ in 0..10_000 {
            x ^= x << 13;
            x ^= x >> 17;
            x ^= x << 5;
            let u = (x as f32 / u32::MAX as f32 - 0.5) * 100.;
            assert_eq!(
                crops::remap_uv([u, 0.], 1000, 64, &crop, [0, 0], [1000, 40]).unwrap()[0].to_bits(),
                u.to_bits()
            );
        }
    }
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
