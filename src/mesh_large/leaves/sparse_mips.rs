//! Lossy leaf page signal mips. Real crop interiors have positive support;
//! padding/gutters have zero support and never feed a later reduction. Normal
//! values are raw unit tangent vectors; material normalScale remains literal.
//! PNG source-pixel lossless bypasses this module. MASK/guards are later delivery
//! operations and must never feed this semantic pyramid.
//! `Class::Id` pages are never averaged: each level takes the support-weighted
//! mode of its 2x2 children (whole RGBA tuple, ties to the first row-major
//! contributor), and every zero-support texel is the no-feature value
//! [`ID_NO_FEATURE`] rather than a dilated neighbour.
use super::mips::Class;
use crate::{JobError, JobErrorKind};
use std::mem::size_of;

#[derive(Debug)]
pub(in crate::mesh_large) struct Level {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
    pub support: Vec<u32>,
}
/// Gutter/padding texel of an ID page: every channel all-ones, so each packed
/// feature-ID set reads its own `nullFeatureId` (2^(8*bytes) - 1).
pub(in crate::mesh_large) const ID_NO_FEATURE: [u8; 4] = [255; 4];
#[derive(Debug)]
pub(in crate::mesh_large) struct Chain {
    pub class: Class,
    pub levels: Vec<Level>,
    /// Owned chain plus reusable gutter scratch bound; borrowed inputs and
    /// native encoder workspace/output must be charged by the caller.
    pub charged_bytes: u64,
}
#[derive(Clone, Copy)]
struct Nearest {
    distance: u64,
    owner: usize,
}
fn error(kind: JobErrorKind, message: &str) -> JobError {
    JobError::new(kind, message.to_owned())
}
fn resource() -> JobError {
    error(
        JobErrorKind::ResourceLimit,
        "sparse mip size, support or allocation exceeds resource limit",
    )
}
fn quant(x: f64) -> u8 {
    (x.clamp(0.0, 1.0) * 255.0).round() as u8
}
fn decode_colour_exact(x: u8) -> f64 {
    let v = f64::from(x) / 255.0;
    if v <= 0.04045 {
        v / 12.92
    } else {
        ((v + 0.055) / 1.055).powf(2.4)
    }
}
/// The same values from a table built with the same expression.
fn decode_colour(x: u8) -> f64 {
    static TABLE: std::sync::OnceLock<[f64; 256]> = std::sync::OnceLock::new();
    TABLE.get_or_init(|| std::array::from_fn(|i| decode_colour_exact(i as u8)))[usize::from(x)]
}
fn encode_colour(v: f64) -> u8 {
    quant(if v <= 0.0031308 {
        12.92 * v
    } else {
        1.055 * v.powf(1.0 / 2.4) - 0.055
    })
}
fn normal(p: &[u8]) -> [f64; 3] {
    std::array::from_fn(|a| f64::from(p[a]) / 127.5 - 1.0)
}
fn unit(v: [f64; 3]) -> Result<[f64; 3], JobError> {
    let length = v.iter().map(|x| x * x).sum::<f64>();
    if !length.is_finite() || length <= 1e-24 {
        return Err(error(
            JobErrorKind::Unsupported,
            "sparse mip unrepresentable zero/nonfinite normal",
        ));
    }
    Ok(v.map(|x| x / length.sqrt()))
}
fn empty_pixel(class: Class) -> [u8; 4] {
    match class {
        Class::Normal => [128, 128, 255, 0],
        Class::Id => ID_NO_FEATURE,
        _ => [0; 4],
    }
}
fn reserve<T>(n: usize) -> Result<Vec<T>, JobError> {
    let mut v = Vec::new();
    v.try_reserve_exact(n).map_err(|_| resource())?;
    Ok(v)
}
fn relax(nearest: &mut [Nearest], labels: Option<&[u32]>, to: usize, from: usize) {
    let src = nearest[from];
    if src.owner == usize::MAX {
        return;
    }
    // A labelled gutter texel takes only its own island's signal.
    if labels.is_some_and(|l| l[to] != 0 && l[src.owner] != l[to]) {
        return;
    }
    let candidate = Nearest {
        distance: src.distance + 1,
        owner: src.owner,
    };
    if (candidate.distance, candidate.owner) < (nearest[to].distance, nearest[to].owner) {
        nearest[to] = candidate;
    }
}
/// Exact Manhattan nearest signal texel; ties original row-major identity.
/// Two-pass taxicab distance transform, not a quadratic search or Euclidean claim.
fn fill_gutters(
    level: &mut Level,
    class: Class,
    nearest: &mut Vec<Nearest>,
    labels: Option<&[u32]>,
) {
    if class == Class::Id {
        for (p, &s) in level
            .rgba
            .as_chunks_mut::<4>()
            .0
            .iter_mut()
            .zip(&level.support)
        {
            if s == 0 {
                *p = ID_NO_FEATURE;
            }
        }
        return;
    }
    let n = level.support.len();
    let w = level.width as usize;
    nearest.clear();
    nearest.extend(level.support.iter().enumerate().map(|(i, &s)| Nearest {
        distance: if s > 0 { 0 } else { u64::MAX },
        owner: if s > 0 { i } else { usize::MAX },
    }));
    for i in 0..n {
        if i % w > 0 {
            relax(nearest, labels, i, i - 1);
        }
        if i >= w {
            relax(nearest, labels, i, i - w);
        }
    }
    for i in (0..n).rev() {
        if i % w + 1 < w {
            relax(nearest, labels, i, i + 1);
        }
        if i + w < n {
            relax(nearest, labels, i, i + w);
        }
    }
    for (i, &s) in level.support.iter().enumerate() {
        if s > 0 {
            continue;
        }
        let owner = nearest[i].owner;
        let value = if owner == usize::MAX {
            empty_pixel(class)
        } else {
            level.rgba[owner * 4..owner * 4 + 4]
                .try_into()
                .expect("validated RGBA shape")
        };
        level.rgba[i * 4..i * 4 + 4].copy_from_slice(&value);
    }
}

/// Source taps of destination index `d` along one axis (`src` texels reduced to `dst`):
/// exact halving takes texels `2d, 2d+1` at weight 1 (unit 1); a floor-halved odd axis
/// takes every source texel the destination footprint `[d·src/dst, (d+1)·src/dst)`
/// overlaps, weighted by the overlap in units of `1/dst` (unit `dst`), so no trailing
/// row or column is ever dropped. At most three taps.
fn taps(d: usize, src: usize, dst: usize) -> ([(usize, u64); 3], usize, u64) {
    let mut out = [(0usize, 0u64); 3];
    if src == dst {
        out[0] = (d, 1);
        return (out, 1, 1);
    }
    if src == 2 * dst {
        out[0] = (2 * d, 1);
        out[1] = (2 * d + 1, 1);
        return (out, 2, 1);
    }
    let (lo, hi) = (d * src, (d + 1) * src);
    let mut n = 0;
    for t in lo / dst..hi.div_ceil(dst) {
        let w = hi.min((t + 1) * dst) - lo.max(t * dst);
        if w > 0 && n < 3 {
            out[n] = (t, w as u64);
            n += 1;
        }
    }
    (out, n, dst as u64)
}

/// Refill zero-support gutters after a caller re-expresses normal values.
/// The caller already owns the level; memory_limit charges only this helper's
/// transient nearest-owner scratch plus the returned owner map, with build's tie rules.
/// Empty texels map to usize::MAX; positive support always owns itself.
pub(in crate::mesh_large) fn refill_gutters(
    level: &mut Level,
    class: Class,
    memory_limit: u64,
) -> Result<Vec<usize>, JobError> {
    let pixels = u64::from(level.width)
        .checked_mul(u64::from(level.height))
        .ok_or_else(resource)?;
    let count = usize::try_from(pixels).map_err(|_| resource())?;
    if count == 0
        || level.rgba.len() != count.checked_mul(4).ok_or_else(resource)?
        || level.support.len() != count
    {
        return Err(error(
            JobErrorKind::InvalidRequest,
            "gutter RGBA/support shape mismatch",
        ));
    }
    let bytes = pixels
        .checked_mul((size_of::<Nearest>() + size_of::<usize>()) as u64)
        .ok_or_else(resource)?;
    if bytes > memory_limit {
        return Err(resource());
    }
    let mut nearest = reserve(count)?;
    fill_gutters(level, class, &mut nearest, None);
    let mut owners = reserve(count)?;
    owners.extend((0..count).map(|i| {
        if class == Class::Id {
            if level.support[i] == 0 {
                usize::MAX
            } else {
                i
            }
        } else {
            nearest[i].owner
        }
    }));
    Ok(owners)
}

/// Support-weighted value of one level-0 texel of an averaged class: linear colour or
/// data (or the unit normal) and linear alpha, each times the texel's support.
fn weighted(class: Class, p: &[u8], s: u32) -> Result<[f64; 4], JobError> {
    let rgb = match class {
        Class::Colour => std::array::from_fn(|a| decode_colour(p[a])),
        Class::Data => std::array::from_fn(|a| f64::from(p[a]) / 255.0),
        Class::Normal => unit(normal(p))?,
        Class::Id => unreachable!("ID levels take the mode"),
    };
    let s = f64::from(s);
    Ok([
        rgb[0] * s,
        rgb[1] * s,
        rgb[2] * s,
        f64::from(p[3]) / 255.0 * s,
    ])
}

/// One destination level of [`direct_levels`] and its two accumulating rows.
struct Direct {
    level: Level,
    /// Island label of each texel (that of its footprint's first level-0 texel) while
    /// the level is isolated; such a texel integrates only its own island.
    labels: Option<Vec<u32>>,
    sum: [Vec<[f64; 4]>; 2],
    area: [Vec<u128>; 2],
}

/// Levels `1..` of an averaged class, each integrated DIRECTLY over level 0 (decision
/// (a)): texel `(x, y)` of a `w x h` level is the support-weighted average of level 0
/// (`W x H`) over its footprint `[x·W/w, (x+1)·W/w) x [y·H/h, (y+1)·H/h)`, a fractional
/// edge texel weighted by its overlap, so odd reductions never blur through earlier
/// levels. One streaming pass over level-0 rows: each row's prefix sums (one row of a
/// summed-area table) give every level's horizontal footprint integrals in O(1) per
/// destination texel, accumulated into at most two destination rows per level with
/// their vertical overlap. Total O(N) time; scratch O(W) besides the levels. Exact
/// integer support (overlaps are integers in units of `1/w`, `1/h`), f64 value sums.
/// Through `isolated` levels with `labels`, each destination texel integrates only
/// level-0 texels of its own island, so islands stay isolated; deeper levels use the
/// page-wide integral. A normal footprint whose unit vectors cancel takes its
/// largest-weight contributor (first row-major on ties), by a direct footprint scan.
fn direct_levels(
    base: &Level,
    labels: Option<&[u32]>,
    isolated: u32,
    class: Class,
    dimensions: &[(u32, u32)],
) -> Result<Vec<Direct>, JobError> {
    let (w0, h0) = (base.width as usize, base.height as usize);
    let block = 1usize << isolated.min(16);
    let mut levels: Vec<Direct> = reserve(dimensions.len())?;
    for (index, &(dw, dh)) in dimensions.iter().enumerate() {
        let (dw, dh) = (dw as usize, dh as usize);
        let n = dw.checked_mul(dh).ok_or_else(resource)?;
        let mut level = Level {
            width: dw as u32,
            height: dh as u32,
            rgba: reserve(n.checked_mul(4).ok_or_else(resource)?)?,
            support: reserve(n)?,
        };
        level.rgba.resize(n * 4, 0);
        level.support.resize(n, 0);
        let labels = match labels {
            Some(l) if (index as u32) < isolated => {
                let mut out = reserve(n)?;
                out.extend((0..n).map(|i| l[(i / dw * h0 / dh) * w0 + i % dw * w0 / dw]));
                Some(out)
            }
            _ => None,
        };
        let row = || -> Result<_, JobError> {
            let mut s = reserve(dw)?;
            s.resize(dw, [0.0; 4]);
            let mut a = reserve(dw)?;
            a.resize(dw, 0u128);
            Ok((s, a))
        };
        let ((s0, a0), (s1, a1)) = (row()?, row()?);
        levels.push(Direct {
            level,
            labels,
            sum: [s0, s1],
            area: [a0, a1],
        });
    }
    let mut value: Vec<[f64; 4]> = reserve(w0)?;
    value.resize(w0, [0.0; 4]);
    let mut prefix: Vec<[f64; 4]> = reserve(w0 + 1)?;
    prefix.resize(w0 + 1, [0.0; 4]);
    let mut support: Vec<u64> = reserve(w0 + 1)?;
    support.resize(w0 + 1, 0);
    let mut cuts: Vec<([f64; 4], u128)> = reserve(w0 + 1)?;
    for y0 in 0..h0 {
        let row = &base.support[y0 * w0..(y0 + 1) * w0];
        for (t, &s) in row.iter().enumerate() {
            value[t] = if s > 0 {
                weighted(
                    class,
                    &base.rgba[(y0 * w0 + t) * 4..(y0 * w0 + t) * 4 + 4],
                    s,
                )?
            } else {
                [0.0; 4]
            };
            prefix[t + 1] = std::array::from_fn(|a| prefix[t][a] + value[t][a]);
            support[t + 1] = support[t] + u64::from(s);
        }
        // Integral of this row over level-0 columns [0, c/u), c in units of 1/u.
        let integral = |c: usize, u: usize| -> ([f64; 4], u128) {
            let (t, f) = (c / u, c % u);
            let mut v = prefix[t].map(|x| x * u as f64);
            let mut s = u128::from(support[t]) * u as u128;
            if f > 0 {
                for (a, x) in v.iter_mut().enumerate() {
                    *x += value[t][a] * f as f64;
                }
                s += u128::from(row[t]) * f as u128;
            }
            (v, s)
        };
        for d in levels.iter_mut() {
            let (dw, dh) = (d.level.width as usize, d.level.height as usize);
            // This row covers [y0, y0 + 1) = [lo, hi) in units of 1/dh.
            let (lo, hi) = (y0 * dh, (y0 + 1) * dh);
            if d.labels.is_none() {
                cuts.clear();
                cuts.extend((0..=dw).map(|x| integral(x * w0, dw)));
            }
            for y in lo / h0..=(hi - 1) / h0 {
                let wy = hi.min((y + 1) * h0) - lo.max(y * h0);
                let (sum, area) = (&mut d.sum[y & 1], &mut d.area[y & 1]);
                for x in 0..dw {
                    let (v, s) = match &d.labels {
                        None => {
                            let (a, b) = (cuts[x], cuts[x + 1]);
                            (std::array::from_fn(|c| b.0[c] - a.0[c]), b.1 - a.1)
                        }
                        Some(own) => {
                            let island = own[y * dw + x];
                            let (a, b) = (x * w0, (x + 1) * w0);
                            let (mut v, mut s) = ([0.0; 4], 0u128);
                            for k in a / dw / block..=(b - 1) / dw / block {
                                if labels.expect("isolated levels have labels")[y0 * w0 + k * block]
                                    != island
                                {
                                    continue;
                                }
                                let from = integral(a.max(k * block * dw), dw);
                                let to = integral(b.min(((k + 1) * block).min(w0) * dw), dw);
                                for (c, x) in v.iter_mut().enumerate() {
                                    *x += to.0[c] - from.0[c];
                                }
                                s += to.1 - from.1;
                            }
                            (v, s)
                        }
                    };
                    for (c, x) in sum[x].iter_mut().enumerate() {
                        *x += v[c] * wy as f64;
                    }
                    area[x] += s * wy as u128;
                }
                if (y + 1) * h0 <= hi {
                    finish_row(d, y, base, labels, class)?;
                }
            }
        }
    }
    Ok(levels)
}

/// Writes destination row `y` of `d` from its accumulators and clears them for row `y+2`.
fn finish_row(
    d: &mut Direct,
    y: usize,
    base: &Level,
    labels: Option<&[u32]>,
    class: Class,
) -> Result<(), JobError> {
    let (dw, dh) = (d.level.width as usize, d.level.height as usize);
    let unit_area = (dw * dh) as u128;
    for x in 0..dw {
        let i = y * dw + x;
        let (sum, area) = (d.sum[y & 1][x], d.area[y & 1][x]);
        d.sum[y & 1][x] = [0.0; 4];
        d.area[y & 1][x] = 0;
        d.level.support[i] = u32::try_from(area.div_ceil(unit_area)).map_err(|_| resource())?;
        if area == 0 {
            d.level.rgba[i * 4..i * 4 + 4].copy_from_slice(&empty_pixel(class));
            continue;
        }
        let weight = area as f64;
        let rgb = if class == Class::Normal {
            let length = sum[..3].iter().map(|v| v * v).sum::<f64>().sqrt();
            if length <= 1e-12 * weight {
                let own = d
                    .labels
                    .as_ref()
                    .map(|l| (labels.expect("labelled level"), l[i]));
                dominant_normal(base, own, x, y, dw, dh)?
            } else {
                unit([sum[0], sum[1], sum[2]])?
            }
        } else {
            std::array::from_fn(|a| sum[a] / weight)
        };
        for (a, &v) in rgb.iter().enumerate() {
            d.level.rgba[i * 4 + a] = match class {
                Class::Colour => encode_colour(v),
                Class::Data => quant(v),
                Class::Normal => quant((v + 1.0) * 0.5),
                Class::Id => unreachable!("ID levels take the mode"),
            };
        }
        d.level.rgba[i * 4 + 3] = quant(sum[3] / weight);
    }
    Ok(())
}

/// Largest-weight level-0 unit normal of the footprint of `(x, y)` (first row-major on
/// ties), restricted to island `own` when given.
fn dominant_normal(
    base: &Level,
    own: Option<(&[u32], u32)>,
    x: usize,
    y: usize,
    dw: usize,
    dh: usize,
) -> Result<[f64; 3], JobError> {
    let (w0, h0) = (base.width as usize, base.height as usize);
    let overlap = |t: usize, lo: usize, hi: usize, u: usize| hi.min((t + 1) * u) - lo.max(t * u);
    let (xa, xb, ya, yb) = (x * w0, (x + 1) * w0, y * h0, (y + 1) * h0);
    let mut best = ([0.0, 0.0, 1.0], 0u128);
    for ty in ya / dh..yb.div_ceil(dh) {
        for tx in xa / dw..xb.div_ceil(dw) {
            let j = ty * w0 + tx;
            if own.is_some_and(|(l, island)| l[j] != island) {
                continue;
            }
            let w = u128::from(base.support[j])
                * overlap(tx, xa, xb, dw) as u128
                * overlap(ty, ya, yb, dh) as u128;
            if w > best.1 {
                best = (unit(normal(&base.rgba[j * 4..j * 4 + 4]))?, w);
            }
        }
    }
    Ok(best.0)
}

pub(in crate::mesh_large) fn build(
    width: u32,
    height: u32,
    rgba: &[u8],
    support: &[u32],
    class: Class,
    memory_limit: u64,
) -> Result<Chain, JobError> {
    if !width.is_power_of_two() || !height.is_power_of_two() {
        return Err(error(
            JobErrorKind::InvalidRequest,
            "sparse mip page requires power-of-two dimensions >=4",
        ));
    }
    build_islands(width, height, rgba, support, None, 0, class, memory_limit)
}

/// Mip levels through which leaf island pages keep every island isolated: island crops
/// sit on the `2^K` source grid with gutters of `2^K` texels, so levels `0..=K` never mix
/// two islands and match the source's own reduction. Leaves keep source-density texels,
/// which can be minified far before the HLOD parent replaces the leaf, so K is deeper
/// than for coarse charts (`hlod::charts::COARSE_ISLAND_ISOLATED_LEVELS`). A scale-gate
/// tuning constant.
pub(crate) const LEAF_ISLAND_ISOLATED_LEVELS: u32 = 3;

/// [`build`] for packed island pages: a COMPLETE chain to 1x1 (a mipmapped consumer needs
/// complete chains), every level the support-weighted area average of level 0 over its
/// footprint ([`direct_levels`]; ID pages take the mode of the previous level instead).
/// The guarantee: islands are isolated through level `isolated` (the caller's layout puts
/// each island allocation of `labels`, 0 outside, on the `2^isolated` grid, which is
/// verified here; through that level each texel integrates and each gutter is filled
/// only from its own island); deeper levels integrate the whole page, where neighbouring
/// islands may blend (bleed accepted, as standard; the HLOD parent replaces the leaf
/// before deep levels dominate). Dimensions may be any multiple of four (whole KTX2
/// blocks).
#[allow(clippy::too_many_arguments)]
pub(in crate::mesh_large) fn build_islands(
    width: u32,
    height: u32,
    rgba: &[u8],
    support: &[u32],
    labels: Option<&[u32]>,
    isolated: u32,
    class: Class,
    memory_limit: u64,
) -> Result<Chain, JobError> {
    if width < 4 || height < 4 || !width.is_multiple_of(4) || !height.is_multiple_of(4) {
        return Err(error(
            JobErrorKind::InvalidRequest,
            "sparse mip page requires block-aligned dimensions >=4",
        ));
    }
    if labels.is_some_and(|l| l.len() as u64 != u64::from(width) * u64::from(height)) {
        return Err(error(
            JobErrorKind::InvalidRequest,
            "sparse mip label/page shape mismatch",
        ));
    }
    // The layout's promise: every aligned 2^isolated block holds one island allocation.
    if let Some(l) = labels {
        let a = 1usize << isolated.min(16);
        let (w, h) = (width as usize, height as usize);
        for by in (0..h).step_by(a) {
            for bx in (0..w).step_by(a) {
                let first = l[by * w + bx];
                for y in by..(by + a).min(h) {
                    if l[y * w + bx..y * w + (bx + a).min(w)]
                        .iter()
                        .any(|&v| v != first)
                    {
                        return Err(error(
                            JobErrorKind::InvalidState,
                            "island page block mixes two islands below its isolated levels",
                        ));
                    }
                }
            }
        }
    }
    let pixels = u64::from(width)
        .checked_mul(u64::from(height))
        .ok_or_else(resource)?;
    let base_len = usize::try_from(pixels).map_err(|_| resource())?;
    let rgba_len = base_len.checked_mul(4).ok_or_else(resource)?;
    if rgba.len() != rgba_len || support.len() != base_len {
        return Err(error(
            JobErrorKind::InvalidRequest,
            "sparse mip RGBA/support shape mismatch",
        ));
    }
    support
        .iter()
        .try_fold(0u32, |sum, &s| sum.checked_add(s).ok_or_else(resource))?;
    let mut dimensions = [(0u32, 0u32); 33];
    let mut count = 0usize;
    let (mut w, mut h) = (width, height);
    let mut charged = 0u64;
    loop {
        dimensions[count] = (w, h);
        count += 1;
        let n = u64::from(w) * u64::from(h);
        let label_bytes = if labels.is_some() {
            size_of::<u32>() as u64
        } else {
            0
        };
        charged = charged
            .checked_add(
                n.checked_mul(4 + size_of::<u32>() as u64 + label_bytes)
                    .ok_or_else(resource)?,
            )
            .ok_or_else(resource)?;
        // Two accumulating rows of the direct integration (`Direct`).
        charged = charged
            .checked_add(
                u64::from(w)
                    .checked_mul(2 * (size_of::<[f64; 4]>() + size_of::<u128>()) as u64)
                    .ok_or_else(resource)?,
            )
            .ok_or_else(resource)?;
        if w == 1 && h == 1 {
            break;
        }
        w = (w / 2).max(1);
        h = (h / 2).max(1);
    }
    charged = charged
        .checked_add(
            pixels
                .checked_mul(size_of::<Nearest>() as u64)
                .ok_or_else(resource)?,
        )
        .ok_or_else(resource)?;
    // One level-0 row of values, prefix sums and footprint cuts (`direct_levels`).
    charged = charged
        .checked_add(
            (u64::from(width) + 1)
                .checked_mul(
                    (2 * size_of::<[f64; 4]>() + size_of::<u64>() + size_of::<([f64; 4], u128)>())
                        as u64,
                )
                .ok_or_else(resource)?,
        )
        .ok_or_else(resource)?;
    charged = charged
        .checked_add(
            (count as u64)
                .checked_mul(size_of::<Level>() as u64)
                .ok_or_else(resource)?,
        )
        .ok_or_else(resource)?;
    if charged > memory_limit {
        return Err(resource());
    }
    let mut chain = Chain {
        class,
        levels: reserve(count)?,
        charged_bytes: charged,
    };
    let mut nearest = reserve(base_len)?;
    let mut base = Level {
        width,
        height,
        rgba: reserve(rgba_len)?,
        support: reserve(base_len)?,
    };
    base.rgba.extend_from_slice(rgba);
    base.support.extend_from_slice(support);
    if class == Class::Normal {
        for (p, &s) in base.rgba.as_chunks_mut::<4>().0.iter_mut().zip(support) {
            if s > 0 {
                let v = unit(normal(p))?;
                for (a, &component) in v.iter().enumerate() {
                    p[a] = quant((component + 1.0) * 0.5);
                }
            }
        }
    }
    fill_gutters(&mut base, class, &mut nearest, labels);
    chain.levels.push(base);
    if class != Class::Id {
        for mut d in direct_levels(
            &chain.levels[0],
            labels,
            isolated,
            class,
            &dimensions[1..count],
        )? {
            fill_gutters(&mut d.level, class, &mut nearest, d.labels.as_deref());
            chain.levels.push(d.level);
        }
        return Ok(chain);
    }
    // ID pages are never averaged: each level takes the mode of its previous level's
    // footprint taps (decision (a)'s area average does not apply to identifiers). Through
    // the isolated levels a texel counts only taps of its own island (the island of its
    // level-0 footprint's first texel, as for averaged classes), so a page whose sides are
    // not multiples of `2^isolated` (where a footprint straddles two islands' blocks) never
    // takes a neighbour island's ID.
    let (w0, h0) = (width as usize, height as usize);
    let island =
        |w: usize, h: usize, x: usize, y: usize| labels.map(|l| l[(y * h0 / h) * w0 + x * w0 / w]);
    for (index, &(dw, dh)) in dimensions[1..count].iter().enumerate() {
        let isolating = labels.is_some() && (index as u32) < isolated;
        let src = chain.levels.last().expect("base exists");
        let (sw, sh) = (src.width as usize, src.height as usize);
        let n = (dw as usize)
            .checked_mul(dh as usize)
            .ok_or_else(resource)?;
        let mut dst = Level {
            width: dw,
            height: dh,
            rgba: reserve(n.checked_mul(4).ok_or_else(resource)?)?,
            support: reserve(n)?,
        };
        dst.rgba.resize(n * 4, 0);
        dst.support.resize(n, 0);
        for y in 0..dh as usize {
            let (ty, ny, uy) = taps(y, src.height as usize, dh as usize);
            for x in 0..dw as usize {
                let (tx, nx, ux) = taps(x, sw, dw as usize);
                let i = y * dw as usize + x;
                let own = island(dw as usize, dh as usize, x, y).filter(|_| isolating);
                let mut len = 0;
                let mut tally = [([0u8; 4], 0u64); 9];
                let mut area = 0u64;
                for &(sy, wy) in &ty[..ny] {
                    for &(sx, wx) in &tx[..nx] {
                        let j = sy * sw + sx;
                        let s = src.support[j];
                        if s == 0 || own.is_some_and(|o| island(sw, sh, sx, sy) != Some(o)) {
                            continue;
                        }
                        let p: [u8; 4] = src.rgba[j * 4..j * 4 + 4].try_into().expect("RGBA8");
                        let w = u64::from(s) * wx * wy;
                        match tally[..len].iter_mut().find(|(v, _)| *v == p) {
                            Some((_, t)) => *t += w,
                            None => {
                                tally[len] = (p, w);
                                len += 1;
                            }
                        }
                        area += w;
                    }
                }
                dst.support[i] = u32::try_from(area.div_ceil(ux * uy)).map_err(|_| resource())?;
                // Strictly greater keeps the first row-major value on ties.
                let mut best = (empty_pixel(class), 0u64);
                for &(v, w) in &tally[..len] {
                    if w > best.1 {
                        best = (v, w);
                    }
                }
                dst.rgba[i * 4..i * 4 + 4].copy_from_slice(&best.0);
            }
        }
        fill_gutters(&mut dst, class, &mut nearest, None);
        chain.levels.push(dst);
    }
    Ok(chain)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn page(class: Class, pixels: &[u8], weights: &[u32]) -> Chain {
        build(4, 4, pixels, weights, class, 1 << 20).unwrap()
    }
    #[test]
    fn gamma_data_and_gutter_contamination_sensitive_control() {
        let mut pixels = vec![17; 64];
        let mut weights = vec![0; 16];
        pixels[0..4].copy_from_slice(&[0, 0, 0, 0]);
        weights[0] = 1;
        pixels[4..8].copy_from_slice(&[255, 255, 255, 255]);
        weights[1] = 1;
        let colour = page(Class::Colour, &pixels, &weights);
        let data = page(Class::Data, &pixels, &weights);
        for level in &colour.levels[1..] {
            assert_eq!(level.rgba[0], 188);
            assert_eq!(level.rgba[3], 128);
            assert_eq!(level.support.iter().sum::<u32>(), 2);
        }
        for level in &data.levels[1..] {
            assert_eq!(level.rgba[0], 128);
        }
        // A black-padding/gutter-as-support mutant averages four values, not two.
        assert!((188_i32 - 137).abs() > 1);
        assert_eq!(colour.levels[0].support, weights);
    }
    #[test]
    fn independent_integer_weighted_linear_reference_every_level() {
        let mut pixels = vec![0; 64];
        let weights: Vec<u32> = (0..16)
            .map(|i| if i % 3 == 0 { 0 } else { (i % 4 + 1) as u32 })
            .collect();
        for (i, p) in pixels.as_chunks_mut::<4>().0.iter_mut().enumerate() {
            p.copy_from_slice(&[(i * 13) as u8, (255 - i * 11) as u8, (i * 7) as u8, 255]);
        }
        let chain = page(Class::Data, &pixels, &weights);
        // Independent reference: integer rational weighted byte means of each level's
        // whole level-0 footprint (explicit index sets, no production filtering helper).
        for (level, actual) in chain.levels.iter().enumerate().skip(1) {
            let (side, width) = (1usize << level, 4usize >> level);
            for y in 0..width {
                for x in 0..width {
                    let ids: Vec<usize> = (0..side * side)
                        .map(|k| (y * side + k / side) * 4 + x * side + k % side)
                        .collect();
                    let weight: u32 = ids.iter().map(|&i| weights[i]).sum();
                    let i = y * width + x;
                    assert_eq!(actual.support[i], weight);
                    if weight == 0 {
                        continue;
                    }
                    for a in 0..4 {
                        let numerator: u64 = ids
                            .iter()
                            .map(|&j| u64::from(pixels[j * 4 + a]) * u64::from(weights[j]))
                            .sum();
                        let want = (2 * numerator + u64::from(weight)) / (2 * u64::from(weight));
                        assert_eq!(
                            u64::from(actual.rgba[i * 4 + a]),
                            want,
                            "level {level} texel {i}"
                        );
                    }
                }
            }
        }
    }
    /// Independent reference of decision (a): the support-weighted mean of level 0 over
    /// the footprint of texel `(x, y)` of a `w x h` level, by explicit rational overlaps
    /// (units `1/w`, `1/h`), restricted to texels `keep` accepts.
    fn footprint_mean(
        pixels: &[u8],
        support: &[u32],
        (width, height): (usize, usize),
        (w, h): (usize, usize),
        (x, y): (usize, usize),
        keep: &dyn Fn(usize) -> bool,
    ) -> Option<f64> {
        let (mut sum, mut weight) = (0f64, 0u128);
        for ty in y * height / h..((y + 1) * height).div_ceil(h) {
            let oy = ((y + 1) * height)
                .min((ty + 1) * h)
                .saturating_sub((y * height).max(ty * h));
            for tx in x * width / w..((x + 1) * width).div_ceil(w) {
                let ox = ((x + 1) * width)
                    .min((tx + 1) * w)
                    .saturating_sub((x * width).max(tx * w));
                let j = ty * width + tx;
                if ox == 0 || oy == 0 || !keep(j) {
                    continue;
                }
                let o = u128::from(support[j]) * ox as u128 * oy as u128;
                sum += f64::from(pixels[j * 4]) * o as f64;
                weight += o;
            }
        }
        (weight > 0).then(|| sum / weight as f64)
    }
    /// Review round 4 defect 1: every level of every page size (odd reductions included)
    /// is the direct level-0 footprint average, within RGBA8 rounding. Recursive reduction
    /// gave 26 for 0 at mip 5 of an 88x24 step, 28 at mip 4 of 72x40 and 30 at mip 4 of
    /// 104x216.
    #[test]
    fn every_level_is_the_direct_level0_footprint_average() {
        for (w, h) in [
            (88usize, 24usize),
            (104, 216),
            (72, 40),
            (132, 128),
            (20, 28),
            (100, 60),
            (4, 4),
        ] {
            for pattern in 0..3 {
                let value = |x: usize, y: usize| -> u8 {
                    match pattern {
                        0 => {
                            if x < w / 2 {
                                0
                            } else {
                                255
                            }
                        }
                        1 => {
                            if x + 8 >= w {
                                255
                            } else {
                                0
                            }
                        }
                        _ => {
                            if (x / 3 + y / 5).is_multiple_of(2) {
                                255
                            } else {
                                (x * 7 + y * 3) as u8
                            }
                        }
                    }
                };
                let pixels: Vec<u8> = (0..w * h).flat_map(|i| [value(i % w, i / w); 4]).collect();
                let support: Vec<u32> = (0..w * h)
                    .map(|i| {
                        if pattern < 2 {
                            1
                        } else {
                            u32::from(i % w + 16 >= w || i / w + 4 >= h) * (1 + (i % 3) as u32)
                        }
                    })
                    .collect();
                let one = vec![1; w * h];
                for labels in [None, Some(&one[..])] {
                    let chain = build_islands(
                        w as u32,
                        h as u32,
                        &pixels,
                        &support,
                        labels,
                        3,
                        Class::Data,
                        1 << 26,
                    )
                    .unwrap();
                    assert_eq!(
                        (
                            chain.levels.last().unwrap().width,
                            chain.levels.last().unwrap().height
                        ),
                        (1, 1)
                    );
                    for (k, l) in chain.levels.iter().enumerate() {
                        let (lw, lh) = (l.width as usize, l.height as usize);
                        for y in 0..lh {
                            for x in 0..lw {
                                let Some(want) = footprint_mean(
                                    &pixels,
                                    &support,
                                    (w, h),
                                    (lw, lh),
                                    (x, y),
                                    &|_| true,
                                ) else {
                                    assert_eq!(l.support[y * lw + x], 0);
                                    continue;
                                };
                                let got = f64::from(l.rgba[(y * lw + x) * 4]);
                                assert!((got - want).abs() <= 0.5 + 1e-9, "{w}x{h} pattern {pattern} level {k} ({x},{y}): {got} vs {want}");
                            }
                        }
                    }
                }
            }
        }
        let step = |w: usize, h: usize, level: usize, x: usize| {
            let pixels: Vec<u8> = (0..w * h)
                .flat_map(|i| [if i % w < w / 2 { 0 } else { 255 }; 4])
                .collect();
            let chain = build_islands(
                w as u32,
                h as u32,
                &pixels,
                &vec![1; w * h],
                None,
                0,
                Class::Data,
                1 << 26,
            )
            .unwrap();
            chain.levels[level].rgba[x * 4]
        };
        assert_eq!(step(88, 24, 5, 0), 0);
        assert_eq!(step(72, 40, 4, 1), 0);
        assert_eq!(step(104, 216, 4, 2), 0);
    }
    /// Within the isolated levels a texel integrates only its own island, also where a
    /// non-power-of-two page makes a level's footprint straddle two islands' blocks
    /// (100 -> 50 -> 25 -> 12 columns: level 3 texel 5 covers columns 41.7..50).
    #[test]
    fn isolated_levels_integrate_only_their_own_island() {
        let (w, h) = (100usize, 24usize);
        let labels: Vec<u32> = (0..w * h).map(|i| if i % w < 48 { 1 } else { 2 }).collect();
        let pixels: Vec<u8> = (0..w * h)
            .flat_map(|i| [if labels[i] == 1 { 255 } else { 0 }; 4])
            .collect();
        let support = vec![1; w * h];
        for class in [Class::Data, Class::Colour] {
            let chain = build_islands(
                w as u32,
                h as u32,
                &pixels,
                &support,
                Some(&labels),
                3,
                class,
                1 << 24,
            )
            .unwrap();
            for (k, l) in chain.levels.iter().enumerate().take(4) {
                let (lw, lh) = (l.width as usize, l.height as usize);
                for y in 0..lh {
                    for x in 0..lw {
                        let island = labels[(y * h / lh) * w + x * w / lw];
                        let want =
                            footprint_mean(&pixels, &support, (w, h), (lw, lh), (x, y), &|j| {
                                labels[j] == island
                            })
                            .unwrap();
                        assert_eq!(
                            f64::from(l.rgba[(y * lw + x) * 4]),
                            want,
                            "{class:?} level {k} ({x},{y})"
                        );
                    }
                }
            }
            // Below the isolated levels the page-wide integral blends them.
            let level = &chain.levels[4];
            assert!(level
                .rgba
                .as_chunks::<4>()
                .0
                .iter()
                .any(|p| p[0] != 0 && p[0] != 255));
        }
    }
    #[test]
    fn independent_bruteforce_gutter_nearest_and_ties() {
        for mask in [1u16, 0x8001, 0x0440, 0x1357, 0x9009] {
            let mut pixels = vec![0; 64];
            let weights: Vec<u32> = (0..16).map(|i| u32::from(mask & (1 << i) != 0)).collect();
            for (i, p) in pixels.as_chunks_mut::<4>().0.iter_mut().enumerate() {
                *p = [i as u8, 0, 0, 255];
            }
            let chain = page(Class::Data, &pixels, &weights);
            let base = &chain.levels[0];
            for i in 0usize..16 {
                let owner = (0usize..16)
                    .filter(|&j| weights[j] > 0)
                    .min_by_key(|&j| ((i % 4).abs_diff(j % 4) + (i / 4).abs_diff(j / 4), j))
                    .unwrap();
                assert_eq!(base.rgba[i * 4], owner as u8);
                assert_eq!(base.support[i], weights[i]);
            }
        }
    }
    #[test]
    fn normal_unit_every_level_and_row_major_cancellation_donor() {
        let mut pixels = vec![0; 64];
        let mut weights = vec![0; 16];
        pixels[..4].copy_from_slice(&[255, 255, 255, 255]);
        pixels[4..8].copy_from_slice(&[0, 0, 0, 255]);
        weights[0] = 1;
        weights[1] = 1;
        let chain = page(Class::Normal, &pixels, &weights);
        assert_eq!(&chain.levels[1].rgba[..3], &chain.levels[0].rgba[..3]);
        for l in &chain.levels {
            for p in l.rgba.as_chunks::<4>().0 {
                let v = p[..3]
                    .iter()
                    .map(|&b| 2.0 * f64::from(b) / 255.0 - 1.0)
                    .collect::<Vec<_>>();
                assert!((v.iter().map(|x| x * x).sum::<f64>().sqrt() - 1.0).abs() < 0.01);
            }
        }
        let oblique: Vec<u8> = (0..16)
            .flat_map(|i| {
                if i % 2 == 0 {
                    [218, 128, 218, 255]
                } else {
                    [37, 128, 218, 255]
                }
            })
            .collect();
        let oblique_chain = page(Class::Normal, &oblique, &[1; 16]);
        for l in &oblique_chain.levels {
            for p in l.rgba.as_chunks::<4>().0 {
                let squared = p[..3]
                    .iter()
                    .map(|&b| {
                        let x = 2.0 * f64::from(b) / 255.0 - 1.0;
                        x * x
                    })
                    .sum::<f64>();
                assert!((squared.sqrt() - 1.0).abs() < 0.01);
            }
        }
        // Oblique unnormalized mean loses length despite a valid angle.
        let length = (181.0_f64 / 255.0).abs();
        assert!((length - 1.0) > 0.01 || (1.0 - length) > 0.01);
    }
    #[test]
    fn empty_pages_and_rectangular_complete_chain() {
        for class in [Class::Colour, Class::Data, Class::Normal, Class::Id] {
            let chain = build(8, 4, &[255; 128], &[0; 32], class, 1 << 20).unwrap();
            assert_eq!(
                chain
                    .levels
                    .iter()
                    .map(|l| (l.width, l.height))
                    .collect::<Vec<_>>(),
                vec![(8, 4), (4, 2), (2, 1), (1, 1)]
            );
            for l in chain.levels {
                assert!(l.support.iter().all(|&s| s == 0));
                for p in l.rgba.as_chunks::<4>().0 {
                    assert_eq!(*p, empty_pixel(class));
                }
            }
        }
    }
    #[test]
    fn typed_shape_memory_and_support_overflow() {
        assert_eq!(
            build(3, 4, &[], &[], Class::Data, u64::MAX)
                .unwrap_err()
                .kind(),
            JobErrorKind::InvalidRequest
        );
        assert_eq!(
            build(4, 4, &[0; 63], &[0; 16], Class::Data, u64::MAX)
                .unwrap_err()
                .kind(),
            JobErrorKind::InvalidRequest
        );
        assert_eq!(
            build(4, 4, &[0; 64], &[0; 16], Class::Data, 1)
                .unwrap_err()
                .kind(),
            JobErrorKind::ResourceLimit
        );
        assert_eq!(
            build(4, 4, &[0; 64], &[u32::MAX; 16], Class::Data, u64::MAX)
                .unwrap_err()
                .kind(),
            JobErrorKind::ResourceLimit
        );
    }

    /// Review round 5 P3-1: an 84x44 ID page (width not a multiple of 8) whose isolated
    /// level-3 footprints straddle two islands' 8-blocks. Island 1 (columns 0..40) has
    /// support only left of column 34, island 2 (40..84) everywhere, so a page-wide mode
    /// would hand island 1's straddling texels island 2's ID. Each texel of the isolated
    /// levels keeps its own island's ID (or no feature).
    #[test]
    fn isolated_id_levels_never_take_a_neighbour_islands_id() {
        let (w, h) = (84usize, 44usize);
        let labels: Vec<u32> = (0..w * h).map(|i| if i % w < 40 { 1 } else { 2 }).collect();
        let support: Vec<u32> = (0..w * h)
            .map(|i| u32::from(i % w < 34 || i % w >= 40))
            .collect();
        let pixels: Vec<u8> = (0..w * h)
            .flat_map(|i| [labels[i] as u8, 0, 0, 0])
            .collect();
        let chain = build_islands(
            w as u32,
            h as u32,
            &pixels,
            &support,
            Some(&labels),
            3,
            Class::Id,
            1 << 24,
        )
        .unwrap();
        let mut straddling = 0;
        for (k, l) in chain.levels.iter().enumerate().take(4).skip(1) {
            let (lw, lh) = (l.width as usize, l.height as usize);
            for y in 0..lh {
                for x in 0..lw {
                    let island = labels[(y * h / lh) * w + x * w / lw] as u8;
                    let p = &l.rgba[(y * lw + x) * 4..(y * lw + x) * 4 + 4];
                    assert!(
                        p == [island, 0, 0, 0] || p == ID_NO_FEATURE,
                        "level {k} ({x},{y}) {p:?} in island {island}"
                    );
                    straddling += usize::from(island == 1 && ((x + 1) * w).div_ceil(lw) > 40);
                }
            }
        }
        assert!(
            straddling > 0,
            "the page has footprints straddling the islands"
        );
    }
    /// Review round 3 probe 1: an 88x24 page whose only island (one white 8x8 signal
    /// block at x 72..80) sits at the right edge, grid 8, gutters 8. Floor-halved odd
    /// levels (11 -> 5 -> 2 -> 1 columns) reduce by area overlap, so the island never
    /// vanishes: every level keeps support and its white signal.
    #[test]
    fn odd_reductions_never_drop_an_island() {
        let (w, h) = (88u32, 24u32);
        let mut pixels = vec![0; (w * h * 4) as usize];
        let mut support = vec![0; (w * h) as usize];
        let mut labels = vec![0; (w * h) as usize];
        for y in 0..h {
            for x in 64..w {
                labels[(y * w + x) as usize] = 1;
            }
        }
        for y in 8..16 {
            for x in 72..80 {
                let i = (y * w + x) as usize;
                support[i] = 1;
                pixels[i * 4..i * 4 + 4].copy_from_slice(&[255; 4]);
            }
        }
        let c = build_islands(
            w,
            h,
            &pixels,
            &support,
            Some(&labels),
            3,
            Class::Data,
            1 << 24,
        )
        .unwrap();
        let last = c.levels.last().unwrap();
        assert_eq!([last.width, last.height], [1, 1]);
        for (k, l) in c.levels.iter().enumerate() {
            let best = (0..l.support.len())
                .filter(|&i| l.support[i] > 0)
                .map(|i| l.rgba[i * 4])
                .max();
            assert_eq!(
                best,
                Some(255),
                "level {k} ({}x{}) lost the island",
                l.width,
                l.height
            );
        }
        // Odd taps cover the whole footprint: three columns 2:2:1 for 5 -> 2.
        assert_eq!(taps(1, 5, 2).1, 3);
        assert_eq!(taps(1, 5, 2).0[..3].iter().map(|t| t.1).sum::<u64>(), 5);
    }
}
