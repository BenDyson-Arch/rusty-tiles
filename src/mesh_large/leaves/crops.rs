//! Checked source-aligned, translation-only crops. No class/value conversion.
use crate::{JobError, JobErrorKind};
use serde_json::Value;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct SourceSampler {
    pub wrap_s: u32,
    pub wrap_t: u32,
    pub min_filter: u32,
    pub mag_filter: u32,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Crop {
    /// Interior source texel origin; allocation includes gutter before this.
    pub origin: [i64; 2],
    pub width: u32,
    pub height: u32,
    pub gutter: u32,
    pub interior_width: u32,
    pub interior_height: u32,
}
fn error(kind: JobErrorKind, message: &str) -> JobError {
    JobError::new(kind, message.to_owned())
}
fn invalid(message: &str) -> JobError {
    error(JobErrorKind::InvalidRequest, message)
}
fn resource() -> JobError {
    error(
        JobErrorKind::ResourceLimit,
        "crop arithmetic/allocation budget",
    )
}
fn unsupported() -> JobError {
    error(
        JobErrorKind::Unsupported,
        "source UV crop exceeds physical representation",
    )
}

pub(crate) fn resolve(value: Option<&Value>) -> Result<SourceSampler, JobError> {
    if value.is_some_and(|v| !v.is_object()) {
        return Err(invalid("sampler must be an object"));
    }
    let field = |name: &str, default: u32, allowed: &[u32]| -> Result<u32, JobError> {
        let n = match value.and_then(|v| v.get(name)) {
            None => default,
            Some(v) => v
                .as_u64()
                .and_then(|n| u32::try_from(n).ok())
                .ok_or_else(|| invalid("invalid sampler enumerant"))?,
        };
        if !allowed.contains(&n) {
            return Err(invalid("unsupported sampler enumerant"));
        }
        Ok(n)
    };
    Ok(SourceSampler {
        wrap_s: field("wrapS", 10497, &[10497, 33648, 33071])?,
        wrap_t: field("wrapT", 10497, &[10497, 33648, 33071])?,
        // Resolve an unspecified minification policy to authored mip sampling,
        // rather than pinning a particular consumer's non-mip default.
        min_filter: field("minFilter", 9987, &[9728, 9729, 9984, 9985, 9986, 9987])?,
        mag_filter: field("magFilter", 9729, &[9728, 9729])?,
    })
}
pub(crate) fn plan(
    uvs: &[[f32; 2]],
    image_width: u32,
    image_height: u32,
    max_edge: u32,
    gutter: u32,
) -> Result<Crop, JobError> {
    if uvs.is_empty() || image_width == 0 || image_height == 0 || max_edge == 0 {
        return Err(invalid("empty crop input or dimensions"));
    }
    let mut lo = [f64::INFINITY; 2];
    let mut hi = [f64::NEG_INFINITY; 2];
    for uv in uvs {
        for axis in 0..2 {
            let v = f64::from(uv[axis]);
            if !v.is_finite() {
                return Err(invalid("nonfinite source UV"));
            }
            if v.abs() > 1e6 {
                return Err(unsupported());
            }
            lo[axis] = lo[axis].min(v);
            hi[axis] = hi[axis].max(v);
        }
    }
    let mut origin = [0; 2];
    let mut interior = [0; 2];
    let mut extent = [0; 2];
    for axis in 0..2 {
        let size = f64::from(if axis == 0 { image_width } else { image_height });
        // UV admission and u32 dimensions bound each product below 2^52.
        // Real source signal halo preserves both half-texel LINEAR taps.
        // Outer gutters remain separate zero-support dilation space.
        origin[axis] = ((lo[axis] * size).floor() as i64)
            .checked_sub(1)
            .ok_or_else(resource)?;
        let end = ((hi[axis] * size).ceil() as i64)
            .checked_add(1)
            .ok_or_else(resource)?;
        let span = end.checked_sub(origin[axis]).ok_or_else(resource)?.max(1);
        interior[axis] = u32::try_from(span).map_err(|_| unsupported())?;
        extent[axis] = interior[axis]
            .checked_add(gutter.checked_mul(2).ok_or_else(resource)?)
            .ok_or_else(resource)?;
        if extent[axis] > max_edge {
            return Err(unsupported());
        }
        origin[axis]
            .checked_sub(i64::from(gutter))
            .ok_or_else(resource)?;
        origin[axis]
            .checked_add(i64::from(interior[axis]))
            .and_then(|v| v.checked_add(i64::from(gutter)))
            .ok_or_else(resource)?;
    }
    Ok(Crop {
        origin,
        width: extent[0],
        height: extent[1],
        gutter,
        interior_width: interior[0],
        interior_height: interior[1],
    })
}
/// Dedicated mipmapped binding page. Both physical edges carry at least one
/// complete source-axis period of padding, and the page origin is aligned to
/// the next-POT source axis. For POT sources this fixes every mip's grid phase.
/// NPOT source/page correspondence still requires the finite sampler gate.
pub(crate) fn plan_mip_aligned(
    uvs: &[[f32; 2]],
    image_width: u32,
    image_height: u32,
    max_edge: u32,
) -> Result<Crop, JobError> {
    if uvs.is_empty() || image_width == 0 || image_height == 0 || max_edge < 4 {
        return Err(invalid("empty mip crop or invalid dimensions"));
    }
    let mut lo = [f64::INFINITY; 2];
    let mut hi = [f64::NEG_INFINITY; 2];
    for uv in uvs {
        for axis in 0..2 {
            let v = f64::from(uv[axis]);
            if !v.is_finite() {
                return Err(invalid("nonfinite mip crop UV"));
            }
            if v.abs() > 1e6 {
                return Err(unsupported());
            }
            lo[axis] = lo[axis].min(v);
            hi[axis] = hi[axis].max(v);
        }
    }
    let mut origin = [0i64; 2];
    let mut extent = [0u32; 2];
    for (axis, size) in [image_width, image_height].into_iter().enumerate() {
        let alignment = size.checked_next_power_of_two().ok_or_else(resource)?;
        let margin = i64::from(size);
        let first = (lo[axis] * f64::from(size)).floor() as i64;
        let last = (hi[axis] * f64::from(size)).ceil() as i64;
        let padded_first = first.checked_sub(margin).ok_or_else(resource)?;
        origin[axis] = padded_first
            .div_euclid(i64::from(alignment))
            .checked_mul(i64::from(alignment))
            .ok_or_else(resource)?;
        let padded_end = last.checked_add(margin).ok_or_else(resource)?;
        let span = padded_end
            .checked_sub(origin[axis])
            .ok_or_else(resource)?
            .max(4);
        extent[axis] = u32::try_from(span)
            .map_err(|_| unsupported())?
            .checked_next_power_of_two()
            .ok_or_else(resource)?;
        if extent[axis] > max_edge {
            return Err(unsupported());
        }
    }
    Ok(Crop {
        origin,
        width: extent[0],
        height: extent[1],
        gutter: 0,
        interior_width: extent[0],
        interior_height: extent[1],
    })
}

/// One translation-only island crop placed at `offset` inside its binding block.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Island {
    pub crop: Crop,
    pub offset: [u32; 2],
}
/// A binding's physical representation: one or more island crops packed into
/// a `width` x `height` block, and each triangle's island. A single island at
/// offset zero is the classic whole-bounding-box crop.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Block {
    pub width: u32,
    pub height: u32,
    pub islands: Vec<Island>,
    pub triangle_island: Vec<u32>,
}
impl Block {
    pub(crate) fn single(crop: Crop, triangles: usize) -> Self {
        Self {
            width: crop.width,
            height: crop.height,
            islands: vec![Island {
                crop,
                offset: [0, 0],
            }],
            triangle_island: vec![0; triangles],
        }
    }
    /// Real source-signal texels (crop interiors) carried by the block.
    pub(crate) fn interior_texels(&self) -> u64 {
        self.islands
            .iter()
            .map(|i| u64::from(i.crop.interior_width) * u64::from(i.crop.interior_height))
            .sum()
    }
}
/// Union-find with path halving; deterministic for a fixed union order.
fn find(parent: &mut [u32], mut i: u32) -> u32 {
    while parent[i as usize] != i {
        parent[i as usize] = parent[parent[i as usize] as usize];
        i = parent[i as usize];
    }
    i
}
/// Grid cells visited while clustering triangle footprints, before the cell
/// edge doubles. Bounds clustering work for arbitrarily large UV footprints.
const ISLAND_CELL_BUDGET: u64 = 1 << 22;
/// Initial clustering cell edge in source texels. Footprints whose padded
/// allocations come within one cell of each other share an island.
const ISLAND_CELL: i64 = 8;
/// Island crops: triangles (three UV corners each) are clustered by the
/// overlap of their padded texel footprints on a coarse grid; each cluster
/// becomes one crop with the same real-source halo and gutter as [`plan`],
/// its allocation rounded up to `align` texels, and the crops are shelf-packed
/// into one block. Whenever the packed block is not smaller than the classic
/// whole-bounding-box crop, that crop is returned instead, so islands never
/// increase a binding's texel area.
pub(crate) fn plan_islands(
    uvs: &[[f32; 2]],
    image_width: u32,
    image_height: u32,
    max_edge: u32,
    gutter: u32,
    align: u32,
) -> Result<Block, JobError> {
    if uvs.is_empty() || !uvs.len().is_multiple_of(3) || align == 0 || !align.is_power_of_two() {
        return Err(invalid(
            "island crop requires whole triangles and POT alignment",
        ));
    }
    let triangles = uvs.len() / 3;
    let size = [f64::from(image_width), f64::from(image_height)];
    // Per-triangle interior spans [lo, hi) with plan()'s one-texel halo.
    let mut spans = Vec::new();
    spans.try_reserve_exact(triangles).map_err(|_| resource())?;
    for corners in uvs.as_chunks::<3>().0 {
        let mut lo = [i64::MAX; 2];
        let mut hi = [i64::MIN; 2];
        for uv in corners {
            for axis in 0..2 {
                let v = f64::from(uv[axis]);
                if !v.is_finite() {
                    return Err(invalid("nonfinite source UV"));
                }
                if v.abs() > 1e6 {
                    return Err(unsupported());
                }
                lo[axis] = lo[axis].min((v * size[axis]).floor() as i64 - 1);
                hi[axis] = hi[axis].max((v * size[axis]).ceil() as i64 + 1);
            }
        }
        for axis in 0..2 {
            hi[axis] = hi[axis].max(lo[axis] + 1);
        }
        spans.push((lo, hi));
    }
    // Footprints are padded by both gutters plus alignment slack, so islands
    // whose allocations would nearly touch are merged rather than packed apart.
    let pad = i64::from(gutter) * 2 + i64::from(align);
    let mut cell = ISLAND_CELL;
    loop {
        let mut cells = 0u64;
        for (lo, hi) in &spans {
            let n = |axis: usize| {
                ((hi[axis] + pad).div_euclid(cell) - (lo[axis] - pad).div_euclid(cell) + 1) as u64
            };
            cells = cells.saturating_add(n(0).saturating_mul(n(1)));
        }
        if cells <= ISLAND_CELL_BUDGET {
            break;
        }
        cell = cell.checked_mul(2).ok_or_else(resource)?;
    }
    let mut parent: Vec<u32> = (0..triangles as u32).collect();
    let mut owner = std::collections::HashMap::<(i64, i64), u32>::new();
    for (t, (lo, hi)) in spans.iter().enumerate() {
        for cy in (lo[1] - pad).div_euclid(cell)..=(hi[1] + pad).div_euclid(cell) {
            for cx in (lo[0] - pad).div_euclid(cell)..=(hi[0] + pad).div_euclid(cell) {
                match owner.entry((cx, cy)) {
                    std::collections::hash_map::Entry::Occupied(entry) => {
                        let a = find(&mut parent, *entry.get());
                        let b = find(&mut parent, t as u32);
                        if a != b {
                            // The smaller root wins: islands are keyed by
                            // their first triangle independent of grid order.
                            let (keep, drop) = (a.min(b), a.max(b));
                            parent[drop as usize] = keep;
                        }
                    }
                    std::collections::hash_map::Entry::Vacant(entry) => {
                        entry.insert(t as u32);
                    }
                }
            }
        }
    }
    drop(owner);
    // Islands in first-triangle order with their union interior spans.
    let mut index_of = vec![u32::MAX; triangles];
    let mut bounds: Vec<([i64; 2], [i64; 2])> = Vec::new();
    let mut triangle_island = Vec::new();
    triangle_island
        .try_reserve_exact(triangles)
        .map_err(|_| resource())?;
    for (t, (lo, hi)) in spans.iter().enumerate() {
        let root = find(&mut parent, t as u32) as usize;
        if index_of[root] == u32::MAX {
            index_of[root] = bounds.len() as u32;
            bounds.push((*lo, *hi));
        }
        let island = index_of[root];
        let entry = &mut bounds[island as usize];
        for axis in 0..2 {
            entry.0[axis] = entry.0[axis].min(lo[axis]);
            entry.1[axis] = entry.1[axis].max(hi[axis]);
        }
        triangle_island.push(island);
    }
    let round = |v: u64| v.div_ceil(u64::from(align)) * u64::from(align);
    // One island over the union of every footprint: plan()'s classic crop,
    // with the allocation rounded to the alignment.
    let island = |lo: [i64; 2], hi: [i64; 2]| -> Result<Option<Crop>, JobError> {
        let mut interior = [0u32; 2];
        let mut extent = [0u32; 2];
        for axis in 0..2 {
            let span = u64::try_from(hi[axis] - lo[axis]).map_err(|_| resource())?;
            let full = round(span + 2 * u64::from(gutter));
            if full > u64::from(max_edge) {
                return Ok(None);
            }
            extent[axis] = full as u32;
            interior[axis] = (full - 2 * u64::from(gutter)) as u32;
        }
        Ok(Some(Crop {
            origin: lo,
            width: extent[0],
            height: extent[1],
            gutter,
            interior_width: interior[0],
            interior_height: interior[1],
        }))
    };
    let union = bounds
        .iter()
        .fold(([i64::MAX; 2], [i64::MIN; 2]), |(lo, hi), (a, b)| {
            (
                [lo[0].min(a[0]), lo[1].min(a[1])],
                [hi[0].max(b[0]), hi[1].max(b[1])],
            )
        });
    let whole = island(union.0, union.1)?;
    let fallback = |whole: Option<Crop>| {
        whole
            .map(|crop| Block::single(crop, triangles))
            .ok_or_else(unsupported)
    };
    let mut islands = Vec::new();
    islands
        .try_reserve_exact(bounds.len())
        .map_err(|_| resource())?;
    let mut area = 0u64;
    let mut widest = 0u64;
    for (lo, hi) in &bounds {
        let Some(crop) = island(*lo, *hi)? else {
            return fallback(whole);
        };
        area += u64::from(crop.width) * u64::from(crop.height);
        widest = widest.max(u64::from(crop.width));
        islands.push(Island {
            crop,
            offset: [0, 0],
        });
    }
    if islands.len() == 1 {
        return fallback(whole);
    }
    // Shelf packing by descending height, then width, then island order, in
    // a block about as wide as the square root of the total area.
    let mut order: Vec<usize> = (0..islands.len()).collect();
    order.sort_by_key(|&i| {
        (
            std::cmp::Reverse(islands[i].crop.height),
            std::cmp::Reverse(islands[i].crop.width),
            i,
        )
    });
    let square = round((area as f64).sqrt().ceil() as u64);
    let mut packed = None;
    for limit in [square.max(widest), u64::from(max_edge)] {
        let limit = limit.min(u64::from(max_edge));
        let (mut x, mut y, mut row, mut width) = (0u64, 0u64, 0u64, 0u64);
        let mut offsets = vec![[0u32; 2]; islands.len()];
        for &i in &order {
            let (w, h) = (
                u64::from(islands[i].crop.width),
                u64::from(islands[i].crop.height),
            );
            if x + w > limit {
                x = 0;
                y += row;
                row = 0;
            }
            offsets[i] = [x as u32, y as u32];
            x += w;
            row = row.max(h);
            width = width.max(x);
        }
        let height = y + row;
        if height <= u64::from(max_edge) {
            packed = Some((width as u32, height as u32, offsets));
            break;
        }
    }
    let Some((width, height, offsets)) = packed else {
        return fallback(whole);
    };
    if let Some(crop) = &whole {
        if u64::from(crop.width) * u64::from(crop.height) <= u64::from(width) * u64::from(height) {
            return fallback(whole);
        }
    }
    for (island, offset) in islands.iter_mut().zip(offsets) {
        island.offset = offset;
    }
    Ok(Block {
        width,
        height,
        islands,
        triangle_island,
    })
}

fn validate_crop(c: &Crop) -> Result<(), JobError> {
    let g = c.gutter.checked_mul(2).ok_or_else(resource)?;
    if c.interior_width == 0
        || c.interior_height == 0
        || c.interior_width.checked_add(g) != Some(c.width)
        || c.interior_height.checked_add(g) != Some(c.height)
    {
        return Err(invalid("inconsistent crop"));
    }
    for (axis, size) in [c.width, c.height].into_iter().enumerate() {
        c.origin[axis]
            .checked_sub(i64::from(c.gutter))
            .and_then(|v| v.checked_add(i64::from(size)))
            .ok_or_else(resource)?;
    }
    Ok(())
}
fn tap(i: i64, n: u32, wrap: u32) -> Result<u32, JobError> {
    let n = i64::from(n);
    if n == 0 {
        return Err(invalid("zero source size"));
    }
    let v = match wrap {
        10497 => i.rem_euclid(n),
        33648 => {
            let p = i.rem_euclid(2 * n);
            if p < n {
                p
            } else {
                2 * n - 1 - p
            }
        }
        33071 => i.clamp(0, n - 1),
        _ => return Err(invalid("invalid wrap")),
    };
    Ok(v as u32)
}
pub(crate) fn copy_crop(
    source_rgba: &[u8],
    source_width: u32,
    source_height: u32,
    crop: &Crop,
    sampler: SourceSampler,
    memory_limit: u64,
) -> Result<Vec<u8>, JobError> {
    validate_crop(crop)?;
    if source_width == 0 || source_height == 0 {
        return Err(invalid("zero source size"));
    }
    let bytes = |w: u32, h: u32| {
        u64::from(w)
            .checked_mul(u64::from(h))
            .and_then(|n| n.checked_mul(4))
            .ok_or_else(resource)
    };
    let source_bytes = bytes(source_width, source_height)?;
    if u64::try_from(source_rgba.len()).map_err(|_| resource())? != source_bytes {
        return Err(invalid("source RGBA shape"));
    }
    let count = bytes(crop.width, crop.height)?;
    if count > memory_limit {
        return Err(resource());
    }
    // Validate both wrap modes before allocating, including zero-use axes.
    tap(0, source_width, sampler.wrap_s)?;
    tap(0, source_height, sampler.wrap_t)?;
    let count = usize::try_from(count).map_err(|_| resource())?;
    let mut out = Vec::new();
    out.try_reserve_exact(count).map_err(|_| resource())?;
    out.resize(count, 0);
    for y in 0..crop.height {
        let sy = tap(
            crop.origin[1] + i64::from(y) - i64::from(crop.gutter),
            source_height,
            sampler.wrap_t,
        )?;
        for x in 0..crop.width {
            let sx = tap(
                crop.origin[0] + i64::from(x) - i64::from(crop.gutter),
                source_width,
                sampler.wrap_s,
            )?;
            let src =
                usize::try_from((u64::from(sy) * u64::from(source_width) + u64::from(sx)) * 4)
                    .map_err(|_| resource())?;
            let dst = usize::try_from((u64::from(y) * u64::from(crop.width) + u64::from(x)) * 4)
                .map_err(|_| resource())?;
            out[dst..dst + 4].copy_from_slice(&source_rgba[src..src + 4]);
        }
    }
    Ok(out)
}
pub(crate) fn remap_uv(
    uv: [f32; 2],
    source_width: u32,
    source_height: u32,
    crop: &Crop,
    page_offset: [u32; 2],
    page_dimensions: [u32; 2],
) -> Result<[f32; 2], JobError> {
    validate_crop(crop)?;
    if source_width == 0 || source_height == 0 {
        return Err(invalid("zero source size"));
    }
    let mut out = [0.; 2];
    for axis in 0..2 {
        let size = if axis == 0 { crop.width } else { crop.height };
        if page_dimensions[axis] == 0
            || page_offset[axis]
                .checked_add(size)
                .is_none_or(|v| v > page_dimensions[axis])
        {
            return Err(invalid("crop outside page"));
        }
        if !uv[axis].is_finite() {
            return Err(invalid("nonfinite UV"));
        }
        if uv[axis].abs() > 1e6 {
            return Err(unsupported());
        }
        let source = if axis == 0 {
            source_width
        } else {
            source_height
        };
        let v = (f64::from(uv[axis]) * f64::from(source) - crop.origin[axis] as f64
            + f64::from(crop.gutter)
            + f64::from(page_offset[axis]))
            / f64::from(page_dimensions[axis]);
        let stored = v as f32;
        if !stored.is_finite() {
            return Err(resource());
        }
        out[axis] = stored;
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    // Independent normalized-coordinate source reference: wrap before selecting
    // taps, rather than using production integer wrapping.
    fn reference(u: f64, pixels: &[f64], wrap: u32, linear: bool) -> f64 {
        let coordinate = match wrap {
            10497 => u - u.floor(),
            33648 => {
                let cycle = u.rem_euclid(2.);
                if cycle <= 1. {
                    cycle
                } else {
                    2. - cycle
                }
            }
            33071 => u.clamp(0., 1.),
            _ => panic!(),
        };
        let fetch = |i: i64| {
            let n = pixels.len() as i64;
            let k = match wrap {
                10497 => ((i % n) + n) % n,
                _ => i.clamp(0, n - 1),
            };
            pixels[k as usize]
        };
        if !linear {
            // Reflected half-open cells reverse the exact-boundary tie.
            let x = coordinate * pixels.len() as f64;
            let i = if wrap == 33648 && u.rem_euclid(2.) > 1. {
                x.ceil() as i64 - 1
            } else {
                x.floor() as i64
            };
            return fetch(i);
        }
        let x = coordinate * pixels.len() as f64 - 0.5;
        let i = x.floor() as i64;
        let t = x - i as f64;
        fetch(i) * (1. - t) + fetch(i + 1) * t
    }
    #[test]
    fn sampler_defaults_and_literal_filters() {
        let s = resolve(None).unwrap();
        assert_eq!((s.min_filter, s.mag_filter, s.wrap_s), (9987, 9729, 10497));
        assert_eq!(resolve(Some(&serde_json::json!({}))).unwrap(), s);
        assert_eq!(
            resolve(Some(&serde_json::json!({"minFilter":9729})))
                .unwrap()
                .min_filter,
            9729
        );
        let s = resolve(Some(
            &serde_json::json!({"minFilter":9987,"magFilter":9728,"wrapS":33648}),
        ))
        .unwrap();
        assert_eq!((s.min_filter, s.mag_filter, s.wrap_s), (9987, 9728, 33648));
        assert!(resolve(Some(&serde_json::json!({"minFilter":true}))).is_err());
    }
    #[test]
    fn literal_copy_and_independent_filter_correspondence() {
        let pixels = [7., 61., 149., 233.];
        let source: Vec<u8> = pixels.iter().flat_map(|p| [*p as u8, 19, 203, 0]).collect();
        let crop = plan(&[[-1., 0.], [2., 1.]], 4, 1, 32, 2).unwrap();
        for wrap in [10497, 33648, 33071] {
            let s = SourceSampler {
                wrap_s: wrap,
                ..resolve(None).unwrap()
            };
            let out = copy_crop(&source, 4, 1, &crop, s, 4096).unwrap();
            for u in [
                -1., -0.875, -0.5, -0.125, 0., 0.125, 0.25, 0.5, 0.875, 1., 1.125, 1.5, 2.,
            ] {
                let mapped = remap_uv([u as f32, 0.5], 4, 1, &crop, [3, 2], [32, 16]).unwrap();
                let x = f64::from(mapped[0]) * 32. - 3. - 0.5;
                for linear in [false, true] {
                    let actual = if linear {
                        let i = x.floor() as usize;
                        let t = x - i as f64;
                        f64::from(out[i * 4]) * (1. - t) + f64::from(out[(i + 1) * 4]) * t
                    } else {
                        f64::from(out[((x + 0.5).floor() as usize) * 4])
                    };
                    assert!(
                        (actual - reference(u, &pixels, wrap, linear)).abs() < 1e-6,
                        "wrap {wrap} u {u} linear {linear}"
                    );
                }
            }
            // Every copied texel keeps otherwise visually unused RGBA channels.
            for rgba in out.as_chunks::<4>().0 {
                assert_eq!(&rgba[1..], &[19, 203, 0]);
            }
        }
    }
    #[test]
    fn zero_uv_and_checked_physical_refusals() {
        let crop = plan(&[[0., 0.]; 3], 4, 4, 8, 2).unwrap();
        assert_eq!((crop.interior_width, crop.interior_height), (2, 2));
        for uv in [-1e6, 1e6] {
            // Domain endpoint remains admitted; only physical span is refused.
            assert!(plan(&[[uv, uv]; 3], 4, 4, 8, 2).is_ok());
            assert_eq!(
                plan(&[[0., 0.], [uv, uv]], 4, 4, 8, 2).unwrap_err().kind(),
                JobErrorKind::Unsupported
            );
        }
        assert_eq!(
            plan(&[[f32::NAN, 0.]], 4, 4, 8, 2).unwrap_err().kind(),
            JobErrorKind::InvalidRequest
        );
        assert_eq!(
            copy_crop(&[0; 64], 4, 4, &crop, resolve(None).unwrap(), 99)
                .unwrap_err()
                .kind(),
            JobErrorKind::ResourceLimit
        );
        assert!(remap_uv([0., 0.], 4, 4, &crop, [4, 4], [8, 8]).is_err());
    }
    #[test]
    fn source_tap_halo_survives_sparse_gutter_fill() {
        use super::super::{mips::Class, sparse_mips};
        for class in [Class::Colour, Class::Data, Class::Normal] {
            let values = if class == Class::Normal {
                [128., 128., 128., 128.]
            } else {
                [7., 61., 149., 233.]
            };
            let source: Vec<u8> = values
                .iter()
                .flat_map(|p| {
                    if class == Class::Normal {
                        [*p as u8, 128, 255, 255]
                    } else {
                        [*p as u8, 19, 203, 255]
                    }
                })
                .collect();
            for wrap in [10497, 33648, 33071] {
                for zero_area in [false, true] {
                    let uvs = if zero_area {
                        vec![[0., 0.]; 3]
                    } else {
                        vec![[-0.5, 0.], [1., 1.]]
                    };
                    let crop = plan(&uvs, 4, 1, 32, 2).unwrap();
                    let copied = copy_crop(
                        &source,
                        4,
                        1,
                        &crop,
                        SourceSampler {
                            wrap_s: wrap,
                            ..resolve(None).unwrap()
                        },
                        4096,
                    )
                    .unwrap();
                    let mut pixels = vec![0; 32 * 16 * 4];
                    let mut support = vec![0; 32 * 16];
                    for y in 0..crop.height {
                        for x in 0..crop.width {
                            let destination = ((y + 2) * 32 + x + 3) as usize;
                            let src = (y * crop.width + x) as usize;
                            pixels[destination * 4..destination * 4 + 4]
                                .copy_from_slice(&copied[src * 4..src * 4 + 4]);
                            if x >= crop.gutter
                                && x < crop.gutter + crop.interior_width
                                && y >= crop.gutter
                                && y < crop.gutter + crop.interior_height
                            {
                                support[destination] = 1;
                            }
                        }
                    }
                    let chain =
                        sparse_mips::build(32, 16, &pixels, &support, class, 1 << 20).unwrap();
                    let points = if zero_area {
                        vec![0.]
                    } else {
                        vec![-0.5, -0.125, 0., 0.125, 0.875, 1.]
                    };
                    for u in points {
                        let uv = remap_uv([u as f32, 0.], 4, 1, &crop, [3, 2], [32, 16]).unwrap();
                        let x = f64::from(uv[0]) * 32. - 0.5;
                        let y = f64::from(uv[1]) * 16. - 0.5;
                        let (ix, iy) = (x.floor() as usize, y.floor() as usize);
                        let (fx, fy) = (x - x.floor(), y - y.floor());
                        let tap =
                            |x: usize, y: usize| f64::from(chain.levels[0].rgba[(y * 32 + x) * 4]);
                        let actual = (tap(ix, iy) * (1. - fx) + tap(ix + 1, iy) * fx) * (1. - fy)
                            + (tap(ix, iy + 1) * (1. - fx) + tap(ix + 1, iy + 1) * fx) * fy;
                        assert!(
                            (actual - reference(u, &values, wrap, true)).abs() < 1e-6,
                            "{class:?} wrap{wrap} u{u}"
                        );
                        let nearest = tap((x + 0.5).floor() as usize, (y + 0.5).floor() as usize);
                        assert!(
                            (nearest - reference(u, &values, wrap, false)).abs() < 1e-6,
                            "nearest {class:?} wrap{wrap} u{u}"
                        );
                    }
                }
            }
        }
    }
    /// Deterministic scattered small triangles over a `size` texture.
    fn scattered(count: usize, size: f32, seed: u64) -> Vec<[f32; 2]> {
        let mut state = seed;
        let mut next = || {
            state = state
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            ((state >> 33) as f32) / (1u64 << 31) as f32
        };
        let mut uvs = Vec::new();
        for _ in 0..count {
            let (u, v) = (next() * 0.9 + 0.05, next() * 0.9 + 0.05);
            let d = 3. / size;
            uvs.extend([[u, v], [u + d, v], [u, v + d]]);
        }
        uvs
    }
    #[test]
    fn islands_cover_every_triangle_and_never_exceed_the_whole_crop() {
        for (count, align, gutter) in [(1, 4, 2), (40, 4, 2), (300, 1, 2), (40, 8, 8)] {
            let uvs = scattered(count, 1024., count as u64);
            let block = plan_islands(&uvs, 1024, 1024, 4096, gutter, align).unwrap();
            let whole = plan(&uvs, 1024, 1024, 4096, gutter).unwrap();
            assert!(
                u64::from(block.width) * u64::from(block.height)
                    <= u64::from(whole.width.next_multiple_of(align))
                        * u64::from(whole.height.next_multiple_of(align))
            );
            assert_eq!(block.triangle_island.len(), count);
            for island in &block.islands {
                assert_eq!(island.crop.gutter, gutter);
                assert_eq!(island.crop.width % align, 0);
                assert_eq!(island.crop.height % align, 0);
                assert_eq!(island.offset[0] % align, 0);
                assert_eq!(island.offset[1] % align, 0);
                assert!(island.offset[0] + island.crop.width <= block.width);
                assert!(island.offset[1] + island.crop.height <= block.height);
            }
            // Islands never overlap inside the block.
            for (i, a) in block.islands.iter().enumerate() {
                for b in &block.islands[i + 1..] {
                    let apart = a.offset[0] + a.crop.width <= b.offset[0]
                        || b.offset[0] + b.crop.width <= a.offset[0]
                        || a.offset[1] + a.crop.height <= b.offset[1]
                        || b.offset[1] + b.crop.height <= a.offset[1];
                    assert!(apart);
                }
            }
            // Every corner, with both LINEAR taps, lands in its island interior.
            for (t, corners) in uvs.as_chunks::<3>().0.iter().enumerate() {
                let island = &block.islands[block.triangle_island[t] as usize];
                for uv in corners {
                    let mapped = remap_uv(
                        *uv,
                        1024,
                        1024,
                        &island.crop,
                        island.offset,
                        [block.width, block.height],
                    )
                    .unwrap();
                    for (axis, coordinate) in mapped.into_iter().enumerate() {
                        let edge = if axis == 0 { block.width } else { block.height };
                        let x = f64::from(coordinate) * f64::from(edge) - 0.5;
                        let lo = f64::from(island.offset[axis] + gutter);
                        let interior = if axis == 0 {
                            island.crop.interior_width
                        } else {
                            island.crop.interior_height
                        };
                        assert!(x.floor() >= lo && x.floor() + 1. < lo + f64::from(interior));
                    }
                }
            }
            if count == 1 {
                assert_eq!(block.islands.len(), 1);
            }
            if count == 40 {
                // Scattered footprints: islands are far smaller than the hull.
                assert!(block.islands.len() > 1);
                assert!(
                    u64::from(block.width) * u64::from(block.height) * 8
                        < u64::from(whole.width) * u64::from(whole.height)
                );
            }
            // Deterministic for identical input.
            assert_eq!(
                block,
                plan_islands(&uvs, 1024, 1024, 4096, gutter, align).unwrap()
            );
        }
    }
    #[test]
    fn single_island_matches_classic_crop_and_refusals_are_kept() {
        let uvs = [[0.1, 0.2], [0.4, 0.2], [0.1, 0.7]];
        let block = plan_islands(&uvs, 64, 32, 4096, 2, 1).unwrap();
        assert_eq!(block.islands.len(), 1);
        assert_eq!(block.islands[0].crop, plan(&uvs, 64, 32, 4096, 2).unwrap());
        assert_eq!(
            plan_islands(&[[0., 0.], [1e6, 0.], [0., 1.]], 4, 4, 8, 2, 4)
                .unwrap_err()
                .kind(),
            JobErrorKind::Unsupported
        );
        assert_eq!(
            plan_islands(&[[f32::NAN, 0.], [0., 0.], [0., 0.]], 4, 4, 8, 2, 4)
                .unwrap_err()
                .kind(),
            JobErrorKind::InvalidRequest
        );
        assert!(plan_islands(&uvs[..2], 64, 32, 4096, 2, 4).is_err());
        // A hull wider than a page still fits as islands.
        let mut spread = scattered(8, 4096., 7);
        spread.extend([[0.0, 0.0], [0.001, 0.0], [0.0, 0.001]]);
        spread.extend([[0.999, 0.999], [0.998, 0.999], [0.999, 0.998]]);
        assert!(plan(&spread, 4096, 4096, 1024, 2).is_err());
        let block = plan_islands(&spread, 4096, 4096, 1024, 2, 4).unwrap();
        assert!(block.width <= 1024 && block.height <= 1024);
    }
    #[test]
    fn mip_aligned_page_both_edge_period_margins_and_physical_limits() {
        for (width, height) in [(4, 1), (3, 5), (8, 4)] {
            let uvs = [[-0.25, 0.0], [1.0, 1.0]];
            let crop = plan_mip_aligned(&uvs, width, height, 64).unwrap();
            assert_eq!(crop.gutter, 0);
            assert_eq!(crop.width, crop.interior_width);
            assert_eq!(crop.height, crop.interior_height);
            for (axis, size) in [width, height].into_iter().enumerate() {
                assert_eq!(
                    crop.origin[axis].rem_euclid(i64::from(size.next_power_of_two())),
                    0
                );
                let physical_end =
                    crop.origin[axis] + i64::from(if axis == 0 { crop.width } else { crop.height });
                let lo = f64::from(uvs[0][axis]) * f64::from(size);
                let hi = f64::from(uvs[1][axis]) * f64::from(size);
                assert!(lo - crop.origin[axis] as f64 >= f64::from(size));
                assert!(physical_end as f64 - hi >= f64::from(size));
            }
        }
        assert_eq!(
            plan_mip_aligned(&[[0.0, 0.0], [1.0, 1.0]], 4096, 4096, 4096)
                .unwrap_err()
                .kind(),
            JobErrorKind::Unsupported
        );
        assert_eq!(
            plan_mip_aligned(&[[f32::NAN, 0.0]], 4, 4, 64)
                .unwrap_err()
                .kind(),
            JobErrorKind::InvalidRequest
        );
    }
}
