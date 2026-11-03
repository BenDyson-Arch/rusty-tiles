//! Bounded per-leaf preparation: positive source-pixel maps and owned local closure.
use super::super::{
    attributes::{FullTriangle, COLOR},
    config::{Profile, TILE_BYTES},
    full::FullCaptured,
};
use super::schedule::{Decoded, MemoryGate, SourceCache, Stored};
use super::{
    crops::{self, Block, Crop, SourceSampler, CLAMP},
    delivery::{self, MaskDelivery},
    geometry::LatticePlan,
    mask::SurfaceSample,
    mips::{self, Class},
    prepared::{PreparedAtlas, PreparedPage, PreparedPrimitive},
    sparse_mips::LEAF_ISLAND_ISOLATED_LEVELS,
};
use crate::mesh_large::mask_pack::{self, Mask};
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
    admit: &mut dyn FnMut(u64) -> Result<(), JobError>,
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
                    b.block = crops::plan_islands_bounded(
                        uvs, size[0], size[1], max_edge, gutter, align, mipmapped, admit,
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

/// Packs every island of one bucket's bindings directly into a single page, when they
/// fit: each island by its mask ([`crops::Island::mask`]), bottom-left, so islands
/// interlock and never share a cell. The page is whole cells and whole 4x4 codec blocks
/// (`build_islands` and `build_base` take any multiple of four), at the least-area width
/// among a few around the square root of the masked area. Bindings then sit at page
/// offset zero and each island carries its page position.
fn pack_islands(
    bindings: &mut [Binding],
    ids: &[usize],
    max_edge: u32,
    page_index: usize,
) -> Result<Option<PagePlan>, JobError> {
    let mut items = Vec::new();
    for &b in ids {
        items.extend((0..bindings[b].block.islands.len()).map(|k| (b, k)));
    }
    let masks: Option<Vec<(u32, &Mask)>> = items
        .iter()
        .map(|&(b, k)| {
            bindings[b].block.islands[k]
                .mask
                .as_ref()
                .map(|(c, m)| (*c, m))
        })
        .collect();
    let Some(masks) = masks else {
        return Ok(None);
    };
    let cell = masks.first().map_or(4, |m| m.0);
    if masks.iter().any(|m| m.0 != cell) || !(1..=64).contains(&cell) {
        return Ok(None);
    }
    let masks: Vec<&Mask> = masks.into_iter().map(|m| m.1).collect();
    let mut order: Vec<usize> = (0..items.len()).collect();
    order.sort_by_key(|&i| {
        let c = masks[i].cells;
        (std::cmp::Reverse(c[1]), std::cmp::Reverse(c[0]), items[i])
    });
    let unit = cell.max(4);
    let round = |v: u64| v.div_ceil(u64::from(unit)) * u64::from(unit);
    let area: u64 = masks.iter().map(|m| m.count()).sum::<u64>() * u64::from(cell * cell);
    let widest = round(u64::from(
        masks.iter().map(|m| m.cells[0]).max().unwrap_or(1) * cell,
    ));
    let mut widths: Vec<u64> = [1., 1.15, 1.35, 1.7]
        .iter()
        .map(|f| round(((area as f64).sqrt() * f).ceil() as u64).max(widest))
        .chain([widest])
        .filter(|&w| w <= u64::from(max_edge))
        .collect();
    widths.sort_unstable();
    widths.dedup();
    // (page area, longer side, width, height, island origins in cells)
    type Fit = (u64, u32, u32, u32, Vec<[u32; 2]>);
    let mut best: Option<Fit> = None;
    for w in widths {
        let Some((at, used, h)) = mask_pack::pack(&masks, &order, w as u32 / cell, max_edge / cell)
        else {
            continue;
        };
        let (pw, ph) = (round(u64::from(used * cell)), round(u64::from(h * cell)));
        if ph > u64::from(max_edge) {
            continue;
        }
        let key = (pw * ph, pw.max(ph) as u32);
        if best.as_ref().is_none_or(|b| key < (b.0, b.1)) {
            best = Some((key.0, key.1, pw as u32, ph as u32, at));
        }
    }
    let Some((_, _, width, height, at)) = best else {
        return Ok(None);
    };
    for (&(b, k), origin) in items.iter().zip(at) {
        bindings[b].block.islands[k].offset = origin.map(|v| v * cell);
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

/// A bucket's islands share one mask-packed page when they fit ([`pack_islands`]);
/// otherwise bindings take stable shelves ordered by descending crop dimensions and
/// canonical binding order, spanning the maximum page edge, on power-of-two pages.
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
) -> Result<Arc<Stored>, JobError> {
    services
        .cache
        .get(&services.captured.scratch, (index, 0), || {
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
) -> Result<Arc<Stored>, JobError> {
    if drop == 0 {
        return source_image(services, index, check);
    }
    let original = source_image(services, index, check)?;
    let drop = drop.min(31 - original.size[0].max(original.size[1]).leading_zeros());
    // The density level depends on the image's class; key it by slot class.
    let class_key = match class(slot) {
        Class::Colour => 0,
        Class::Normal => 1,
        Class::Data => 2,
        Class::Id => 3,
    };
    if drop == 0 {
        return Ok(original);
    }
    services
        .cache
        .density(&services.captured.scratch, (index, class_key), drop, || {
            let memory = usize::try_from(services.memory).map_err(|_| resource())?;
            timing::time(Stage::DensityMips, || {
                mips::build(
                    original.size[0],
                    original.size[1],
                    original.rgba.materialize()?,
                    class(slot),
                    None,
                    memory,
                )
            })
            .map(|chain| chain.levels)
        })
}

fn source_alpha(
    reader: &mut crate::mesh_large::image_tiles::Reader<'_>,
    size: [u32; 2],
    uv: [f64; 2],
    sampler: SourceSampler,
    linear: bool,
) -> Result<f64, JobError> {
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
    let mut tap = |x: i64, y: i64| -> Result<f64, JobError> {
        Ok(f64::from(
            reader.pixel(
                wrap(x, size[0], sampler.wrap_s),
                wrap(y, size[1], sampler.wrap_t),
            )?[3],
        ) / 255.)
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
    Ok(
        (tap(ix, iy)? * (1. - fx) + tap(ix + 1, iy)? * fx) * (1. - fy)
            + (tap(ix, iy + 1)? * (1. - fx) + tap(ix + 1, iy + 1)? * fx) * fy,
    )
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
    source: &crate::mesh_large::image_tiles::Raster,
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
    let mut reader = source.reader();
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
                source_pass: source_alpha(&mut reader, source_size, uv, b.sampler, linear)?
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
    planning_bytes: u64,
}
/// Shares the exact physical crop/packing representation with final preparation.
/// Counts planning owners and raw streams; it does not predict compressed file size.
#[cfg(test)]
pub(super) fn project(
    triangles: &[FullTriangle],
    resources: AtlasResources<'_>,
    profile: Profile,
    drop: u32,
    check: &mut dyn FnMut() -> Result<(), JobError>,
) -> Result<LeafProjection, JobError> {
    project_bounded(triangles, resources, profile, drop, None, check)
}
pub(super) fn project_bounded(
    triangles: &[FullTriangle],
    resources: AtlasResources<'_>,
    profile: Profile,
    drop: u32,
    ceiling: Option<u64>,
    check: &mut dyn FnMut() -> Result<(), JobError>,
) -> Result<LeafProjection, JobError> {
    let layout = layout(
        (triangles.to_vec(), vec![[0; 3]; triangles.len()]),
        resources,
        profile,
        drop,
        ceiling,
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
        .and_then(|n| n.checked_add(layout.planning_bytes))
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
    ceiling: Option<u64>,
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
    let (mut retained, mut planning_bytes) = (0u64, 0u64);
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
            let block = match crops::plan_islands_bounded(
                &uvs,
                size[0],
                size[1],
                max_edge,
                gutter,
                align,
                mipmapped,
                &mut |bytes| {
                    let peak = retained.checked_add(bytes).ok_or_else(resource)?;
                    if ceiling.is_some_and(|limit| peak > limit) {
                        return Err(resource());
                    }
                    planning_bytes = planning_bytes.max(peak);
                    Ok(())
                },
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
            retained = retained
                .checked_add(block.heap_bytes())
                .ok_or_else(resource)?;
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
    let items = bindings.iter().map(|b| b.block.islands.len()).sum();
    planning_bytes = planning_bytes.max(
        retained
            .checked_add(if items == 0 {
                0
            } else {
                mask_pack::workspace_bytes(max_edge, items)
            })
            .ok_or_else(resource)?,
    );
    if ceiling.is_some_and(|limit| planning_bytes > limit) {
        return Err(resource());
    }
    let dedicated = period_pages(
        &mut bindings,
        &primitives,
        (profile, max_edge, &mask_classes),
        &mut |bytes| {
            let peak = retained.checked_add(bytes).ok_or_else(resource)?;
            if ceiling.is_some_and(|limit| peak > limit) {
                return Err(resource());
            }
            planning_bytes = planning_bytes.max(peak);
            Ok(())
        },
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
        planning_bytes,
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
    // RGBA/support, labels/reduced chain, and the span Vec: at most one run
    // per texel, 12 bytes each with up to twice the capacity during collection.
    let mipmapped = profile != Profile::Lossless && matches!(page.sampler.min_filter, 9984..=9987);
    let assembly = pixels
        .checked_mul(if mipmapped { 42 } else { 32 })
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
        tier: delivery::Tier::Leaf,
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
                    let copied = crops::copy_crop_tiled(&source.rgba, crop, b.sampler, memory)?;
                    // The island's spans of crop texels: its mask cells (other islands may
                    // interlock into the rest of its rectangle), or the whole crop.
                    let spans: Vec<(u32, u32, u32)> = match &island.mask {
                        Some((cell, mask)) => mask
                            .runs()
                            .flat_map(|(cy, c0, c1)| {
                                (cy * cell..(cy + 1) * cell)
                                    .map(move |y| (y, c0 * cell, (c1 * cell).min(crop.width)))
                            })
                            .filter(|&(y, ..)| y < crop.height)
                            .collect(),
                        None => (0..crop.height).map(|y| (y, 0, crop.width)).collect(),
                    };
                    // Mipmapped islands: every carried texel is real source under the
                    // sampler's wrap (copy_crop), so every level through K is the
                    // area-weighted source average over its footprint, gutters included.
                    // Otherwise only the interior is signal.
                    let interior = if mipmapped {
                        [0, crop.width, 0, crop.height]
                    } else {
                        let g = crop.gutter;
                        [g, g + crop.interior_width, g, g + crop.interior_height]
                    };
                    next_label += 1;
                    for (y, x0, x1) in spans {
                        let at = |x: u32| ((oy + y) * page.width + ox + x) as usize;
                        let src = (y * crop.width + x0) as usize * 4;
                        rgba[at(x0) * 4..at(x1) * 4]
                            .copy_from_slice(&copied[src..src + (x1 - x0) as usize * 4]);
                        let (s0, s1) = (x0.max(interior[0]), x1.min(interior[1]));
                        if (interior[2]..interior[3]).contains(&y) && s0 < s1 {
                            support[at(s0)..at(s1)].fill(1);
                        }
                        if mipmapped {
                            labels[at(x0)..at(x1)].fill(next_label);
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
        layout(geometry, resources, profile, drop, None, &mut || {
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
    fn lossless_8192_mask_capacity_is_projected_and_refused_before_allocation() {
        use crate::gltf_source::source::{Document, ImageInfo};
        use crate::mesh_large::{
            attributes::{CornerAttributes, UV0},
            records::{Provenance, TriangleRecord},
        };
        let doc = json!({"asset":{"version":"2.0"},"scene":0,"scenes":[{"nodes":[0]}],"nodes":[{"mesh":0}],
            "meshes":[{"primitives":[{"attributes":{"POSITION":0,"TEXCOORD_0":1},"material":0}]}],
            "buffers":[{"uri":"never-read.bin","byteLength":60}],"bufferViews":[{"buffer":0,"byteLength":36},{"buffer":0,"byteOffset":36,"byteLength":24}],
            "accessors":[{"bufferView":0,"componentType":5126,"count":3,"type":"VEC3","min":[0,0,0],"max":[1,1,0]},
                {"bufferView":1,"componentType":5126,"count":3,"type":"VEC2"}],
            "images":[{"uri":"never-read.png"}],"textures":[{"source":0,"sampler":0}],
            "samplers":[{"minFilter":9729,"wrapS":33071,"wrapT":33071}],
            "materials":[{"pbrMetallicRoughness":{"baseColorTexture":{"index":0}}}]});
        let document = Document::parse(&serde_json::to_vec(&doc).unwrap(), || Ok(())).unwrap();
        let info = [ImageInfo {
            mime_type: "image/png",
            extension: "png",
            width: 8186,
            height: 8186,
        }];
        let classes = crate::mesh_large::batching::MaterialClasses::default();
        let resources = AtlasResources {
            materials: document.materials(),
            classes: &classes,
            textures: document.textures(),
            samplers: document.samplers(),
            image_info: &info,
        };
        let uv = [[0., 0.], [1., 0.], [0., 1.]];
        let row = FullTriangle {
            triangle: TriangleRecord {
                key: 0,
                source: Provenance::default(),
                vertices: [0, 1, 2],
                positions: [[0.; 3], [1., 0., 0.], [0., 1., 0.]],
                bytes: 56,
                degenerate: false,
            },
            expanded_ordinal: 0,
            material: 0,
            flags: UV0,
            corners: std::array::from_fn(|i| CornerAttributes {
                texcoords: [uv[i], [0.; 2]],
                ..Default::default()
            }),
        };
        let block = crops::plan_islands_aligned(&uv, 8186, 8186, 8192, 2, 1, false).unwrap();
        let bits: u64 = block
            .islands
            .iter()
            .map(|i| i.mask.as_ref().unwrap().1.bits.capacity() as u64 * 8)
            .sum();
        assert_eq!(bits, 8_388_608);
        let projected = project(&[row], resources, Profile::Lossless, 0, &mut || Ok(())).unwrap();
        assert!(projected.owned_bytes >= block.heap_bytes());
        assert!(projected.owned_bytes >= bits + mask_pack::workspace_bytes(8192, 1));
        println!(
            "8192² bits={bits}, containers={}, projected owned={}",
            block.heap_bytes() - bits,
            projected.owned_bytes
        );
        let err = project_bounded(
            &[row],
            resources,
            Profile::Lossless,
            0,
            Some(bits - 1),
            &mut || Ok(()),
        )
        .err()
        .unwrap();
        assert_eq!(err.kind(), JobErrorKind::ResourceLimit);
    }
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
    /// L-shaped footprints (a row and a column of small triangles) make islands whose
    /// rectangles are mostly empty. On one page no cell is shared, every island stays on its
    /// alignment grid inside the page, and the page is no larger than the islands'
    /// rectangles laid end to end, and smaller than the previous power-of-two shelf page.
    #[test]
    fn island_pages_interlock_masks_on_whole_blocks() {
        let n = 1024u32;
        let sampler = crops::resolve(None).unwrap();
        let mut bindings = Vec::new();
        for k in 0..12 {
            let (u0, v0) = (0.05 + 0.07 * f64::from(k), 0.1 + 0.06 * f64::from(k % 5));
            let mut uvs = Vec::new();
            for i in 0..12 {
                let step = 4. * f64::from(i) / f64::from(n);
                let d = 2. / f64::from(n);
                for (u, v) in [(u0 + step, v0), (u0, v0 + step)] {
                    uvs.extend([[u, v], [u + d, v], [u, v + d]].map(|q| q.map(|x| x as f32)));
                }
            }
            let block = crops::plan_islands_aligned(&uvs, n, n, 4096, 8, 8, true).unwrap();
            bindings.push(Binding {
                primitive: 0,
                slot: TextureSlot::BaseColor,
                source_uv: 0,
                output_uv: 0,
                image: 0,
                sampler,
                block,
                source_size: [n, n],
                offset: [0, 0],
                page: 0,
                period: [false; 2],
            });
        }
        let rects: u64 = bindings
            .iter()
            .flat_map(|b| &b.block.islands)
            .map(|i| u64::from(i.crop.width) * u64::from(i.crop.height))
            .sum();
        let ids: Vec<usize> = (0..bindings.len()).collect();
        let page = pack_islands(&mut bindings, &ids, 4096, 0).unwrap().unwrap();
        assert_eq!((page.width % 8, page.height % 8), (0, 0));
        // The previous layout: shelves by descending height at power-of-two widths, on a
        // power-of-two page. Masks and block-sized pages are never larger.
        let mut items: Vec<[u32; 2]> = bindings
            .iter()
            .flat_map(|b| &b.block.islands)
            .map(|i| [i.crop.width, i.crop.height])
            .collect();
        items.sort_by_key(|d| (std::cmp::Reverse(d[1]), std::cmp::Reverse(d[0])));
        let shelf = (2..=12)
            .filter_map(|k| {
                let limit = 1u32 << k;
                let (mut x, mut y, mut row, mut w) = (0, 0, 0, 0);
                for d in &items {
                    if d[0] > limit {
                        return None;
                    }
                    if x + d[0] > limit {
                        (x, y, row) = (0, y + row, 0);
                    }
                    x += d[0];
                    row = row.max(d[1]);
                    w = w.max(x);
                }
                Some(u64::from(w.next_power_of_two()) * u64::from((y + row).next_power_of_two()))
            })
            .min()
            .unwrap();
        let area = u64::from(page.width) * u64::from(page.height);
        assert!(area <= rects && area < shelf, "{area} vs {rects} / {shelf}");
        let mut owner = std::collections::HashMap::new();
        for (bi, b) in bindings.iter().enumerate() {
            for (k, island) in b.block.islands.iter().enumerate() {
                let (cell, mask) = island.mask.as_ref().unwrap();
                assert_eq!((island.offset[0] % cell, island.offset[1] % cell), (0, 0));
                assert!(island.offset[0] + island.crop.width < page.width + cell);
                for (y, x0, x1) in mask.runs() {
                    for x in x0..x1 {
                        let at = (island.offset[0] / cell + x, island.offset[1] / cell + y);
                        assert!(at.0 * cell < page.width && at.1 * cell < page.height);
                        assert!(owner.insert(at, (bi, k)).is_none(), "cell {at:?} shared");
                    }
                }
            }
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
