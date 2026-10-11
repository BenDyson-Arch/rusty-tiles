//! Finite R2 corner cover and fixed WebMercatorQuad grid.
//!
//! The outward forward transform owns coverage. Report geographic bounds are
//! descriptive only. Native execution, source authority and publication live
//! with their respective owners. See the frozen R2 B4 math contract.

use std::fmt;

const H: f64 = f64::from_bits(0x4173_1bf8_457c_1093);
const R: f64 = 6_378_137.0;
const MAX_SOURCE_DIMENSION: u32 = 65_536;
const PI: Pair = Pair {
    lo: f64::from_bits(0x4009_21fb_5444_2d18),
    hi: f64::from_bits(0x4009_21fb_5444_2d19),
};
const LN2: Pair = Pair {
    lo: f64::from_bits(0x3fe6_2e42_fefa_39ef),
    hi: f64::from_bits(0x3fe6_2e42_fefa_39f0),
};
// Upward binary64 roundings of (3/2)^27/27! and
// 2*(1/2)^65/(65*(1-1/4)); exact rational proofs are retained in B4.
const SIN_TAIL: f64 = f64::from_bits(0x3b19_3b3b_1fcb_ce84);
const LOG_TAIL: f64 = f64::from_bits(0x3b95_0150_1501_5016);

#[repr(u8)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum StaticCrs {
    Geographic4326 = 1,
    WebMercator3857 = 2,
}

#[derive(Clone, Copy, Debug)]
pub(super) struct ZoomRange {
    min: u8,
    max: u8,
}

impl ZoomRange {
    /// Called once by the core request boundary; children consume this value.
    pub(super) fn new(min: u8, max: u8) -> Result<Self, GridError> {
        if min > max || max > 24 {
            return Err(GridError::InvalidInput(
                "zoom range must satisfy min <= max <= 24",
            ));
        }
        Ok(Self { min, max })
    }

    pub(super) fn min(self) -> u8 {
        self.min
    }

    pub(super) fn max(self) -> u8 {
        self.max
    }
}

/// Projection of the root's already validated limits, not another validator.
#[derive(Clone, Copy, Debug)]
pub(super) struct GridCaps {
    pub(super) max_total_tiles: u64,
    pub(super) max_finest_rgba_bytes: u64,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct ZoomRect {
    pub(super) x0: u32,
    pub(super) y0: u32,
    pub(super) x1: u32,
    pub(super) y1: u32,
    pub(super) z: u8,
    reserved: [u8; 3],
}

impl ZoomRect {
    const EMPTY: Self = Self {
        x0: 0,
        y0: 0,
        x1: 0,
        y1: 0,
        z: 0,
        reserved: [0; 3],
    };
}

#[derive(Debug)]
pub(super) struct TargetGrid {
    pub(super) width: u32,
    pub(super) height: u32,
    pub(super) geotransform: [f64; 6],
}

#[derive(Debug)]
pub(super) struct GridPlan {
    rectangles: [ZoomRect; 25],
    active: u8,
    target: TargetGrid,
    total_tiles: u64,
    finest_rgba_bytes: u64,
    geographic_bounds: [f64; 4],
}

impl GridPlan {
    /// Finest first, then requested integer ancestors; no per-address owner.
    pub(super) fn rectangles(&self) -> &[ZoomRect] {
        &self.rectangles[..usize::from(self.active)]
    }

    pub(super) fn finest_rect(&self) -> ZoomRect {
        self.rectangles[0]
    }

    pub(super) fn target(&self) -> &TargetGrid {
        &self.target
    }

    pub(super) fn total_tiles(&self) -> u64 {
        self.total_tiles
    }

    pub(super) fn finest_rgba_bytes(&self) -> u64 {
        self.finest_rgba_bytes
    }

    /// Approximate descriptive W,S,E,N for TileJSON, never cover authority.
    pub(super) fn geographic_bounds(&self) -> [f64; 4] {
        self.geographic_bounds
    }
}

#[derive(Debug, Eq, PartialEq)]
pub(super) enum GridError {
    InvalidInput(&'static str),
    Unsupported(&'static str),
    ResourceLimit(&'static str),
}

impl fmt::Display for GridError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let (Self::InvalidInput(reason) | Self::Unsupported(reason) | Self::ResourceLimit(reason)) =
            self;
        f.write_str(reason)
    }
}

impl std::error::Error for GridError {}

#[derive(Clone, Copy, Debug, PartialEq)]
struct Pair {
    lo: f64,
    hi: f64,
}

impl Pair {
    const fn point(value: f64) -> Self {
        Self {
            lo: value,
            hi: value,
        }
    }

    fn neg(self) -> Self {
        Self {
            lo: -self.hi,
            hi: -self.lo,
        }
    }

    fn outward(lo: f64, hi: f64) -> Result<Self, GridError> {
        let result = Self {
            lo: next_down(lo),
            hi: next_up(hi),
        };
        if !result.lo.is_finite() || !result.hi.is_finite() {
            return Err(GridError::Unsupported(
                "numeric enclosure exceeds finite binary64",
            ));
        }
        Ok(result)
    }

    fn add(self, other: Self) -> Result<Self, GridError> {
        if self == Self::point(0.0) {
            return Ok(other);
        }
        if other == Self::point(0.0) {
            return Ok(self);
        }
        if self.lo == self.hi && other.lo == other.hi {
            let (x, y) = (self.lo, other.lo);
            // Point subtraction under Sterbenz, or a <=53-bit integer sum.
            if (x * y < 0.0 && 0.5 * x.abs() <= y.abs() && y.abs() <= 2.0 * x.abs())
                || (x.fract() == 0.0
                    && y.fract() == 0.0
                    && x.abs() <= 4_503_599_627_370_496.0
                    && y.abs() <= 4_503_599_627_370_496.0)
            {
                return Ok(Self::point(x + y));
            }
        }
        Self::outward(self.lo + other.lo, self.hi + other.hi)
    }

    fn sub(self, other: Self) -> Result<Self, GridError> {
        self.add(other.neg())
    }

    /// Normal exact power-of-two endpoint shifts only; otherwise decline.
    fn shift(self, scalar: f64) -> Option<Self> {
        power_exponent(scalar)?;
        let x = self.lo * scalar;
        let y = self.hi * scalar;
        if !x.is_finite()
            || !y.is_finite()
            || (self.lo != 0.0 && x.abs() < f64::MIN_POSITIVE)
            || (self.hi != 0.0 && y.abs() < f64::MIN_POSITIVE)
        {
            return None;
        }
        Some(Self {
            lo: x.min(y),
            hi: x.max(y),
        })
    }

    fn mul(self, other: Self) -> Result<Self, GridError> {
        if self == Self::point(0.0) || other == Self::point(0.0) {
            return Ok(Self::point(0.0));
        }
        if other.lo == other.hi {
            if let Some(exact) = self.shift(other.lo) {
                return Ok(exact);
            }
        }
        if self.lo == self.hi {
            if let Some(exact) = other.shift(self.lo) {
                return Ok(exact);
            }
        }
        endpoints([
            self.lo * other.lo,
            self.lo * other.hi,
            self.hi * other.lo,
            self.hi * other.hi,
        ])
    }

    fn div(self, other: Self) -> Result<Self, GridError> {
        if other.lo <= 0.0 && other.hi >= 0.0 {
            return Err(GridError::Unsupported(
                "numeric divisor enclosure includes zero",
            ));
        }
        if self == Self::point(0.0) {
            return Ok(self);
        }
        if other.lo == other.hi {
            if let Some(exponent) = power_exponent(other.lo) {
                if let Some(reciprocal) = power_of_two(-exponent) {
                    if let Some(exact) = self.shift(reciprocal.copysign(other.lo)) {
                        return Ok(exact);
                    }
                }
            }
            if self.lo == self.hi {
                let q = self.lo / other.lo;
                if other.shift(q) == Some(self) {
                    return Ok(Self::point(q));
                }
            }
        }
        endpoints([
            self.lo / other.lo,
            self.lo / other.hi,
            self.hi / other.lo,
            self.hi / other.hi,
        ])
    }
}

fn next_up(x: f64) -> f64 {
    if x.is_nan() || x == f64::INFINITY {
        return x;
    }
    if x == 0.0 {
        return f64::from_bits(1);
    }
    f64::from_bits(if x > 0.0 {
        x.to_bits() + 1
    } else {
        x.to_bits() - 1
    })
}

fn next_down(x: f64) -> f64 {
    -next_up(-x)
}

fn endpoints(values: [f64; 4]) -> Result<Pair, GridError> {
    if values.iter().any(|x| !x.is_finite()) {
        return Err(GridError::Unsupported(
            "numeric primitive exceeds finite binary64",
        ));
    }
    let lo = values.into_iter().fold(f64::INFINITY, f64::min);
    let hi = values.into_iter().fold(f64::NEG_INFINITY, f64::max);
    Pair::outward(lo, hi)
}

fn power_exponent(x: f64) -> Option<i32> {
    let bits = x.to_bits() & 0x7fff_ffff_ffff_ffff;
    let exponent = ((bits >> 52) & 0x7ff) as i32;
    let fraction = bits & 0x000f_ffff_ffff_ffff;
    if exponent == 0 && fraction.is_power_of_two() {
        Some(fraction.trailing_zeros() as i32 - 1074)
    } else if exponent > 0 && exponent < 2047 && fraction == 0 {
        Some(exponent - 1023)
    } else {
        None
    }
}

fn power_of_two(exponent: i32) -> Option<f64> {
    match exponent {
        -1074..=-1023 => Some(f64::from_bits(1 << (exponent + 1074))),
        -1022..=1023 => Some(f64::from_bits(((exponent + 1023) as u64) << 52)),
        _ => None,
    }
}

fn sin_bound(x: Pair) -> Result<Pair, GridError> {
    if x.lo.abs().max(x.hi.abs()) > 1.5 {
        return Err(GridError::Unsupported(
            "latitude polynomial domain exceeded",
        ));
    }
    if x == Pair::point(0.0) {
        return Ok(x);
    }
    let square = x.mul(x)?;
    let (mut term, mut total) = (x, x);
    for k in 1..13 {
        term = term
            .mul(square)?
            .neg()
            .div(Pair::point(f64::from((2 * k) * (2 * k + 1))))?;
        total = total.add(term)?;
    }
    let total = total.add(Pair {
        lo: -SIN_TAIL,
        hi: SIN_TAIL,
    })?;
    Ok(Pair {
        lo: total.lo.max(-1.0),
        hi: total.hi.min(1.0),
    })
}

fn log_point(x: f64) -> Result<Pair, GridError> {
    if !x.is_finite() || x <= 0.0 {
        return Err(GridError::Unsupported(
            "logarithm requires a positive finite endpoint",
        ));
    }
    // Exact mantissa/exponent reduction including subnormal endpoints.
    let (normal, correction) = if x < f64::MIN_POSITIVE {
        (x * 4_503_599_627_370_496.0, -52)
    } else {
        (x, 0)
    };
    let bits = normal.to_bits();
    let exponent = ((bits >> 52) & 0x7ff) as i32 - 1023 + correction;
    let m = f64::from_bits((bits & 0x000f_ffff_ffff_ffff) | 0x3ff0_0000_0000_0000);
    if m == 1.0 {
        return Pair::point(f64::from(exponent)).mul(LN2);
    }
    let mut z = Pair::point(m)
        .sub(Pair::point(1.0))?
        .div(Pair::point(m).add(Pair::point(1.0))?)?;
    z.lo = z.lo.max(0.0);
    if !(0.0 <= z.lo && z.lo <= z.hi && z.hi <= 0.5) {
        return Err(GridError::Unsupported(
            "logarithm range reduction is ambiguous",
        ));
    }
    let mut square = z.mul(z)?;
    square.lo = square.lo.max(0.0);
    let (mut term, mut total) = (z, Pair::point(0.0));
    for k in 0..32 {
        total = total.add(term.div(Pair::point(f64::from(2 * k + 1)))?)?;
        term = term.mul(square)?;
        term.lo = term.lo.max(0.0);
    }
    let reduced = Pair::point(2.0).mul(total)?.add(Pair {
        lo: 0.0,
        hi: LOG_TAIL,
    })?;
    reduced.add(Pair::point(f64::from(exponent)).mul(LN2)?)
}

fn log_bound(x: Pair) -> Result<Pair, GridError> {
    Ok(Pair {
        lo: log_point(x.lo)?.lo,
        hi: log_point(x.hi)?.hi,
    })
}

fn corners(affine: [f64; 6], dimensions: [u32; 2]) -> Result<[[Pair; 2]; 4], GridError> {
    let mut result = [[Pair::point(0.0); 2]; 4];
    let [w, h] = dimensions;
    for (index, (u, v)) in [(0, 0), (w, 0), (w, h), (0, h)].into_iter().enumerate() {
        for (axis, coordinate) in result[index].iter_mut().enumerate() {
            let base = 3 * axis;
            *coordinate = Pair::point(affine[base])
                .add(Pair::point(f64::from(u)).mul(Pair::point(affine[base + 1]))?)?
                .add(Pair::point(f64::from(v)).mul(Pair::point(affine[base + 2]))?)?;
        }
    }
    Ok(result)
}

fn project(corner: [Pair; 2], crs: StaticCrs) -> Result<[Pair; 2], GridError> {
    let [x, y] = corner;
    match crs {
        StaticCrs::WebMercator3857 => {
            if x.lo.abs().max(x.hi.abs()).max(y.lo.abs()).max(y.hi.abs()) > H {
                return Err(GridError::Unsupported(
                    "source corner lies outside represented Mercator square",
                ));
            }
            Ok(corner)
        }
        StaticCrs::Geographic4326 => {
            if x.lo < -180.0 || x.hi > 180.0 || y.lo < -85.0 || y.hi > 85.0 {
                return Err(GridError::Unsupported(
                    "source corner lies outside longitude/latitude profile",
                ));
            }
            let radians = y.mul(PI)?.div(Pair::point(180.0))?;
            let sine = sin_bound(radians)?;
            let ratio = Pair::point(1.0)
                .add(sine)?
                .div(Pair::point(1.0).sub(sine)?)?;
            Ok([
                Pair::point(R).mul(PI)?.mul(x)?.div(Pair::point(180.0))?,
                Pair::point(R / 2.0).mul(log_bound(ratio)?)?,
            ])
        }
    }
}

fn tile_coords(world: [Pair; 2], z: u8) -> Result<[Pair; 2], GridError> {
    let half = Pair::point(power_of_two(i32::from(z) - 1).expect("validated zoom power"));
    Ok([
        half.add(half.mul(world[0].div(Pair::point(H))?)?)?,
        half.sub(half.mul(world[1].div(Pair::point(H))?)?)?,
    ])
}

fn finest_cover(corners: &[[Pair; 2]; 4], crs: StaticCrs, z: u8) -> Result<ZoomRect, GridError> {
    let (mut min_x, mut min_y, mut max_x, mut max_y) = (
        f64::INFINITY,
        f64::INFINITY,
        f64::NEG_INFINITY,
        f64::NEG_INFINITY,
    );
    for corner in corners {
        let [x, y] = tile_coords(project(*corner, crs)?, z)?;
        min_x = min_x.min(x.lo);
        min_y = min_y.min(y.lo);
        max_x = max_x.max(x.hi);
        max_y = max_y.max(y.hi);
    }
    let last = f64::from((1u32 << z) - 1);
    // Certified finite tile-coordinate enclosure; exact upper edges half-open.
    let x0 = min_x.floor().max(0.0).min(last) as u32;
    let y0 = min_y.floor().max(0.0).min(last) as u32;
    let x1 = (max_x.ceil() - 1.0).min(last);
    let y1 = (max_y.ceil() - 1.0).min(last);
    if x1 < f64::from(x0) || y1 < f64::from(y0) {
        return Err(GridError::Unsupported(
            "source has an empty conservative tile cover",
        ));
    }
    Ok(ZoomRect {
        x0,
        y0,
        x1: x1 as u32,
        y1: y1 as u32,
        z,
        reserved: [0; 3],
    })
}

fn descriptive_bounds(corners: &[[Pair; 2]; 4], crs: StaticCrs) -> [f64; 4] {
    let mut result = [
        f64::INFINITY,
        f64::INFINITY,
        f64::NEG_INFINITY,
        f64::NEG_INFINITY,
    ];
    for [x, y] in corners {
        let x = x.lo + (x.hi - x.lo) / 2.0;
        let y = y.lo + (y.hi - y.lo) / 2.0;
        let (longitude, latitude) = match crs {
            StaticCrs::Geographic4326 => (x, y),
            // libm inverse is deliberately descriptive; it never influences
            // cover, target, counts, source admission or native policy.
            StaticCrs::WebMercator3857 => {
                (
                    // H0 rounds slightly beyond real pi*R; clamp descriptive
                    // longitude to the geographic display domain only.
                    (x / R).to_degrees().clamp(-180.0, 180.0),
                    (y / R).sinh().atan().to_degrees(),
                )
            }
        };
        result[0] = result[0].min(longitude);
        result[1] = result[1].min(latitude);
        result[2] = result[2].max(longitude);
        result[3] = result[3].max(latitude);
    }
    result
}

pub(super) fn plan(
    crs: StaticCrs,
    affine: [f64; 6],
    source_dimensions: [u32; 2],
    zooms: ZoomRange,
    caps: GridCaps,
) -> Result<GridPlan, GridError> {
    if source_dimensions
        .iter()
        .any(|&n| n == 0 || n > MAX_SOURCE_DIMENSION)
    {
        return Err(GridError::InvalidInput(
            "source dimensions must lie in 1..=65536",
        ));
    }
    if affine.iter().any(|x| !x.is_finite()) {
        return Err(GridError::InvalidInput("source affine must be finite"));
    }
    let determinant = Pair::point(affine[1])
        .mul(Pair::point(affine[5]))?
        .sub(Pair::point(affine[2]).mul(Pair::point(affine[4]))?)?;
    if determinant.lo <= 0.0 && determinant.hi >= 0.0 {
        return Err(GridError::Unsupported(
            "source affine is singular or numerically ambiguous",
        ));
    }
    let corners = corners(affine, source_dimensions)?;
    let finest = finest_cover(&corners, crs, zooms.max())?;
    let geographic_bounds = descriptive_bounds(&corners, crs);
    let mut rectangles = [ZoomRect::EMPTY; 25];
    let mut active = 0u8;
    let mut total_tiles = 0u64;
    for z in (zooms.min()..=zooms.max()).rev() {
        let k = 1u32 << (zooms.max() - z);
        let rect = ZoomRect {
            x0: finest.x0 / k,
            y0: finest.y0 / k,
            x1: finest.x1 / k,
            y1: finest.y1 / k,
            z,
            reserved: [0; 3],
        };
        let columns = u64::from(rect.x1 - rect.x0) + 1;
        let rows = u64::from(rect.y1 - rect.y0) + 1;
        let tiles = columns
            .checked_mul(rows)
            .ok_or(GridError::ResourceLimit("tile count overflow"))?;
        total_tiles = total_tiles
            .checked_add(tiles)
            .ok_or(GridError::ResourceLimit("total tile count overflow"))?;
        if total_tiles > caps.max_total_tiles {
            return Err(GridError::ResourceLimit(
                "requested tile count exceeds limit",
            ));
        }
        rectangles[usize::from(active)] = rect;
        active += 1;
    }
    let dimension = |first: u32, last: u32| {
        (u64::from(last - first) + 1)
            .checked_mul(256)
            .filter(|&n| n <= i32::MAX as u64)
            .map(|n| n as u32)
            .ok_or(GridError::ResourceLimit(
                "finest target dimension exceeds native i32",
            ))
    };
    let width = dimension(finest.x0, finest.x1)?;
    let height = dimension(finest.y0, finest.y1)?;
    let finest_pixels = u64::from(width)
        .checked_mul(u64::from(height))
        .ok_or(GridError::ResourceLimit("finest pixel count overflow"))?;
    let finest_rgba_bytes = finest_pixels
        .checked_mul(4)
        .ok_or(GridError::ResourceLimit("finest RGBA byte count overflow"))?;
    if finest_rgba_bytes > caps.max_finest_rgba_bytes {
        return Err(GridError::ResourceLimit("finest RGBA bytes exceed limit"));
    }
    let resolution =
        power_of_two(-7 - i32::from(zooms.max())).expect("validated zoom resolution") * H;
    // Deliberately match selected tile source evaluation order, with no FMA.
    let x = -H + f64::from(finest.x0) * resolution * 256.0;
    let y = H - f64::from(finest.y0) * resolution * 256.0;
    let target = TargetGrid {
        width,
        height,
        geotransform: [x, resolution, 0.0, y, 0.0, -resolution],
    };
    Ok(GridPlan {
        rectangles,
        active,
        target,
        total_tiles,
        finest_rgba_bytes,
        geographic_bounds,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn caps() -> GridCaps {
        GridCaps {
            max_total_tiles: 100_000,
            max_finest_rgba_bytes: 2 * 1024 * 1024 * 1024,
        }
    }

    // Exact bits from frozen B4 epoch2, stdoutSHA76cd6ae3ab96365ae82a443a4cd529b842d605b6aa135a65be1ebed0fd85665a.
    // Geographic world references came independently from Decimal96 tan-form
    // Mercator/exact affine Fractions, not this module's log-ratio recurrence.
    // Finite rounded-reference containment is not a universal libm theorem.
    struct Fixture {
        name: &'static str,
        bits: [u64; 6],
        crs: StaticCrs,
        z: u8,
        expected: [u32; 4],
        world: [[u64; 2]; 4],
    }
    const FIXTURES: [Fixture; 10] = [
        Fixture {
            name: "aligned-z2-tile1",
            bits: [
                0xc1631bf8457c1093,
                0x41331bf8457c1093,
                0x0000000000000000,
                0x41631bf8457c1093,
                0x0000000000000000,
                0xc1331bf8457c1093,
            ],
            crs: StaticCrs::WebMercator3857,
            z: 2,
            expected: [1, 1, 1, 1],
            world: [
                [0xc1631bf8457c1093, 0x41631bf8457c1093],
                [0x0000000000000000, 0x41631bf8457c1093],
                [0x0000000000000000, 0x0000000000000000],
                [0xc1631bf8457c1093, 0x0000000000000000],
            ],
        },
        Fixture {
            name: "aligned-z2-tile0",
            bits: [
                0xc1731bf8457c1093,
                0x41331bf8457c1093,
                0x0000000000000000,
                0x41731bf8457c1093,
                0x0000000000000000,
                0xc1331bf8457c1093,
            ],
            crs: StaticCrs::WebMercator3857,
            z: 2,
            expected: [0, 0, 0, 0],
            world: [
                [0xc1731bf8457c1093, 0x41731bf8457c1093],
                [0xc1631bf8457c1093, 0x41731bf8457c1093],
                [0xc1631bf8457c1093, 0x41631bf8457c1093],
                [0xc1731bf8457c1093, 0x41631bf8457c1093],
            ],
        },
        Fixture {
            name: "near-edge-inside",
            bits: [
                0xc1631bf8457c1092,
                0x41331bf8457c1093,
                0x0000000000000000,
                0x41631bf8457c1093,
                0x0000000000000000,
                0xc1331bf8457c1093,
            ],
            crs: StaticCrs::WebMercator3857,
            z: 2,
            expected: [0, 1, 2, 1],
            world: [
                [0xc1631bf8457c1092, 0x41631bf8457c1093],
                [0x3e20000000000000, 0x41631bf8457c1093],
                [0x3e20000000000000, 0x0000000000000000],
                [0xc1631bf8457c1092, 0x0000000000000000],
            ],
        },
        Fixture {
            name: "near-edge-outside",
            bits: [
                0xc1631bf8457c1094,
                0x41331bf8457c1093,
                0x0000000000000000,
                0x41631bf8457c1093,
                0x0000000000000000,
                0xc1331bf8457c1093,
            ],
            crs: StaticCrs::WebMercator3857,
            z: 2,
            expected: [0, 1, 1, 1],
            world: [
                [0xc1631bf8457c1094, 0x41631bf8457c1093],
                [0xbe20000000000000, 0x41631bf8457c1093],
                [0xbe20000000000000, 0x0000000000000000],
                [0xc1631bf8457c1094, 0x0000000000000000],
            ],
        },
        Fixture {
            name: "rotated-corner-box",
            bits: [
                0xc1531bf8457c1093,
                0x41131bf8457c1093,
                0x41031bf8457c1093,
                0x41531bf8457c1093,
                0x41031bf8457c1093,
                0xc1131bf8457c1093,
            ],
            crs: StaticCrs::WebMercator3857,
            z: 4,
            expected: [6, 5, 7, 6],
            world: [
                [0xc1531bf8457c1093, 0x41531bf8457c1093],
                [0xc1431bf8457c1093, 0x4157e2f656db14b8],
                [0xc1331bf8457c1093, 0x414ca9f4683a18dc],
                [0xc14ca9f4683a18dc, 0x41431bf8457c1093],
            ],
        },
        Fixture {
            name: "geographic-zero-edges",
            bits: [
                0xbff0000000000000,
                0x3fc0000000000000,
                0x0000000000000000,
                0x3ff0000000000000,
                0x0000000000000000,
                0xbfc0000000000000,
            ],
            crs: StaticCrs::Geographic4326,
            z: 8,
            expected: [127, 127, 127, 127],
            world: [
                [0xc0fb2d77da4a0c31, 0x40fb2dd2492e433b],
                [0x0000000000000000, 0x40fb2dd2492e433b],
                [0x0000000000000000, 0x0000000000000000],
                [0xc0fb2d77da4a0c31, 0x0000000000000000],
            ],
        },
        Fixture {
            name: "geographic-rotated",
            bits: [
                0xc000000000000000,
                0x3fd0000000000000,
                0x3fc0000000000000,
                0x4046800000000000,
                0x3fc0000000000000,
                0xbfd0000000000000,
            ],
            crs: StaticCrs::Geographic4326,
            z: 8,
            expected: [126, 91, 128, 94],
            world: [
                [0xc10b2d77da4a0c31, 0x415571c45f1dc555],
                [0x0000000000000000, 0x41560cdf4e18ae1b],
                [0x40fb2d77da4a0c31, 0x4154d9588bbb8f5a],
                [0xc0fb2d77da4a0c31, 0x41544378f6345ac8],
            ],
        },
        Fixture {
            name: "geographic-highlat",
            bits: [
                0x4024000000000000,
                0x3fc0000000000000,
                0x0000000000000000,
                0x4055000000000000,
                0x0000000000000000,
                0xbfc0000000000000,
            ],
            crs: StaticCrs::Geographic4326,
            z: 10,
            expected: [540, 31, 543, 56],
            world: [
                [0x4130fc6ae86e479f, 0x4171ef9ae18c4752],
                [0x4132af426612e862, 0x4171ef9ae18c4752],
                [0x4132af426612e862, 0x4170ff0d7f59da7f],
                [0x4130fc6ae86e479f, 0x4170ff0d7f59da7f],
            ],
        },
        Fixture {
            name: "native-phase-source8x8",
            bits: [
                0x0000000000000000,
                0x40c31bf8457c1093,
                0x0000000000000000,
                0x0000000000000000,
                0x0000000000000000,
                0xc0c31bf8457c1093,
            ],
            crs: StaticCrs::WebMercator3857,
            z: 4,
            expected: [8, 8, 8, 8],
            world: [
                [0x0000000000000000, 0x0000000000000000],
                [0x40f31bf8457c1093, 0x0000000000000000],
                [0x40f31bf8457c1093, 0xc0f31bf8457c1093],
                [0x0000000000000000, 0xc0f31bf8457c1093],
            ],
        },
        Fixture {
            name: "native-two-worker-source8x8",
            bits: [
                0x0000000000000000,
                0x41331bf8457c1093,
                0x0000000000000000,
                0x0000000000000000,
                0x0000000000000000,
                0xc1331bf8457c1093,
            ],
            crs: StaticCrs::WebMercator3857,
            z: 4,
            expected: [8, 8, 11, 11],
            world: [
                [0x0000000000000000, 0x0000000000000000],
                [0x41631bf8457c1093, 0x0000000000000000],
                [0x41631bf8457c1093, 0xc1631bf8457c1093],
                [0x0000000000000000, 0xc1631bf8457c1093],
            ],
        },
    ];

    #[test]
    fn frozen_independent_corner_and_cover_cases() {
        for fixture in &FIXTURES {
            let affine = fixture.bits.map(f64::from_bits);
            let p = plan(
                fixture.crs,
                affine,
                [8, 8],
                ZoomRange::new(fixture.z, fixture.z).unwrap(),
                caps(),
            )
            .unwrap();
            let r = p.finest_rect();
            assert_eq!(
                [r.x0, r.y0, r.x1, r.y1],
                fixture.expected,
                "{}",
                fixture.name
            );
            let c = corners(affine, [8, 8]).unwrap();
            for (corner, reference) in c.into_iter().zip(fixture.world) {
                let projected = project(corner, fixture.crs).unwrap();
                for (enclosure, value) in projected.into_iter().zip(reference.map(f64::from_bits)) {
                    assert!(
                        enclosure.lo <= value && value <= enclosure.hi,
                        "{} {:?} {}",
                        fixture.name,
                        enclosure,
                        value
                    );
                }
            }
        }
    }

    #[test]
    fn ancestors_and_exact_admission_boundaries() {
        let f = &FIXTURES[9];
        let p = plan(
            f.crs,
            f.bits.map(f64::from_bits),
            [8, 8],
            ZoomRange::new(2, 4).unwrap(),
            caps(),
        )
        .unwrap();
        assert_eq!(
            p.rectangles()
                .iter()
                .map(|r| [r.x0, r.y0, r.x1, r.y1])
                .collect::<Vec<_>>(),
            vec![[8, 8, 11, 11], [4, 4, 5, 5], [2, 2, 2, 2]]
        );
        assert_eq!(p.total_tiles(), 21);
        assert_eq!(
            u64::from(p.target().width) * u64::from(p.target().height),
            1024 * 1024
        );
        assert_eq!(p.finest_rgba_bytes(), 4 * 1024 * 1024);
        let exact = GridCaps {
            max_total_tiles: 21,
            max_finest_rgba_bytes: 4 * 1024 * 1024,
        };
        assert!(plan(
            f.crs,
            f.bits.map(f64::from_bits),
            [8, 8],
            ZoomRange::new(2, 4).unwrap(),
            exact
        )
        .is_ok());
        assert_eq!(
            plan(
                f.crs,
                f.bits.map(f64::from_bits),
                [8, 8],
                ZoomRange::new(2, 4).unwrap(),
                GridCaps {
                    max_total_tiles: 20,
                    ..exact
                }
            )
            .unwrap_err(),
            GridError::ResourceLimit("requested tile count exceeds limit")
        );
        assert_eq!(
            plan(
                f.crs,
                f.bits.map(f64::from_bits),
                [8, 8],
                ZoomRange::new(2, 4).unwrap(),
                GridCaps {
                    max_finest_rgba_bytes: 4 * 1024 * 1024 - 1,
                    ..exact
                }
            )
            .unwrap_err(),
            GridError::ResourceLimit("finest RGBA bytes exceed limit")
        );
    }

    #[test]
    fn native_target_and_report_metadata_have_separate_authority() {
        let f = &FIXTURES[8];
        let p = plan(
            f.crs,
            f.bits.map(f64::from_bits),
            [8, 8],
            ZoomRange::new(2, 4).unwrap(),
            caps(),
        )
        .unwrap();
        let t = p.target();
        assert_eq!([t.width, t.height], [256, 256]);
        assert_eq!(
            t.geotransform.map(f64::to_bits),
            [
                0,
                H.to_bits() - 11 * (1u64 << 52),
                0,
                0,
                0,
                (-H / 2048.0).to_bits()
            ]
        );
        let [x, resolution, _, y, _, step_y] = t.geotransform;
        let east = x + f64::from(t.width) * resolution;
        let south = y + f64::from(t.height) * step_y;
        assert_eq!([x, south, east, y], [0., -H / 8., H / 8., 0.]);
        // Independent exact-Fraction per-primitive rounding for frozen
        // geographic-highlat cover [540,31,543,56], Z10. Reconstructing
        // resolution from rounded extent loses three binary64 ulps here.
        let h = &FIXTURES[7];
        let high = plan(
            h.crs,
            h.bits.map(f64::from_bits),
            [8, 8],
            ZoomRange::new(10, 10).unwrap(),
            caps(),
        )
        .unwrap();
        let target = high.target();
        assert_eq!([target.width, target.height], [1024, 6656]);
        assert_eq!(
            target.geotransform.map(f64::to_bits),
            [
                0x4130_b879_3ccc_8e80,
                0x4063_1bf8_457c_1093,
                0,
                0x4171_f3c6_bd47_0d92,
                0,
                0xc063_1bf8_457c_1093,
            ]
        );
        let [x, resolution, _, _, _, _] = target.geotransform;
        let east = x + f64::from(target.width) * resolution;
        let recomputed = (east - x) / f64::from(target.width);
        assert_eq!(recomputed.to_bits(), 0x4063_1bf8_457c_1090);
        assert_ne!(recomputed, target.geotransform[1]);
        let g = &FIXTURES[5];
        let p = plan(
            g.crs,
            g.bits.map(f64::from_bits),
            [8, 8],
            ZoomRange::new(8, 8).unwrap(),
            caps(),
        )
        .unwrap();
        assert_eq!(p.geographic_bounds(), [-1., 0., 0., 1.]);
        // Approximate report facts cannot change any certified cover field.
        let mut p = p;
        let rect = p.finest_rect();
        let gt = p.target().geotransform;
        p.geographic_bounds = [-179., -84., 179., 84.];
        assert_eq!(p.finest_rect(), rect);
        assert_eq!(p.target().geotransform, gt);
    }

    #[test]
    fn source_only_dimension_cap_and_native_dimension_refusal() {
        // A wide allowed target exceeds65536 while its actual source is1x1.
        let p = plan(
            StaticCrs::WebMercator3857,
            [-H, 300.0 * H / 512.0, 0., 0., 0., -H / 512.0],
            [1, 1],
            ZoomRange::new(10, 10).unwrap(),
            caps(),
        )
        .unwrap();
        assert!(p.target().width > 65_536);
        assert!(p.finest_rgba_bytes() <= caps().max_finest_rgba_bytes);
        assert!(matches!(
            plan(
                StaticCrs::WebMercator3857,
                [0., 1., 0., 0., 0., -1.],
                [65_537, 1],
                ZoomRange::new(0, 0).unwrap(),
                caps()
            ),
            Err(GridError::InvalidInput(_))
        ));
        let unlimited = GridCaps {
            max_total_tiles: u64::MAX,
            max_finest_rgba_bytes: u64::MAX,
        };
        assert_eq!(
            plan(
                StaticCrs::WebMercator3857,
                [-H, 2. * H, 0., H, 0., -2. * H],
                [1, 1],
                ZoomRange::new(24, 24).unwrap(),
                unlimited
            )
            .unwrap_err(),
            GridError::ResourceLimit("finest target dimension exceeds native i32")
        );
    }

    #[test]
    fn finite_invalid_and_unsupported_domain_errors() {
        assert!(ZoomRange::new(5, 4).is_err());
        assert!(ZoomRange::new(0, 25).is_err());
        let z = ZoomRange::new(4, 4).unwrap();
        assert!(matches!(
            plan(
                StaticCrs::WebMercator3857,
                [0., 1., 0., 0., 0., -1.],
                [0, 8],
                z,
                caps()
            ),
            Err(GridError::InvalidInput(_))
        ));
        assert!(matches!(
            plan(
                StaticCrs::WebMercator3857,
                [f64::NAN, 1., 0., 0., 0., -1.],
                [8, 8],
                z,
                caps()
            ),
            Err(GridError::InvalidInput(_))
        ));
        assert!(matches!(
            plan(
                StaticCrs::WebMercator3857,
                [0., 1., 1., 0., 1., 1.],
                [8, 8],
                z,
                caps()
            ),
            Err(GridError::Unsupported(_))
        ));
        assert!(matches!(
            plan(
                StaticCrs::Geographic4326,
                [0., 0.1, 0., 86., 0., -0.1],
                [8, 8],
                z,
                caps()
            ),
            Err(GridError::Unsupported(_))
        ));
        assert!(matches!(
            plan(
                StaticCrs::WebMercator3857,
                [H, 1., 0., 0., 0., -1.],
                [8, 8],
                z,
                caps()
            ),
            Err(GridError::Unsupported(_))
        ));
        assert!(Pair::point(f64::MAX).mul(Pair::point(2.)).is_err());
        assert!(Pair::point(1.).div(Pair { lo: -1., hi: 1. }).is_err());
    }

    #[test]
    fn exact_float_steps_and_shortcuts() {
        assert_eq!(next_up(0.).to_bits(), 1);
        assert_eq!(next_down(0.).to_bits(), 0x8000_0000_0000_0001);
        assert_eq!(next_up(-0.).to_bits(), 1);
        assert_eq!(next_down(-0.).to_bits(), 0x8000_0000_0000_0001);
        assert_eq!(next_up(-1.), -next_down(1.));
        assert_eq!(Pair::point(H).div(Pair::point(H)).unwrap(), Pair::point(1.));
        assert_eq!(
            Pair::point(H).mul(Pair::point(0.5)).unwrap(),
            Pair::point(H / 2.)
        );
        assert!(Pair::point(f64::MIN_POSITIVE).shift(0.5).is_none());
        assert_eq!(power_of_two(-1074).unwrap().to_bits(), 1);
        assert_eq!(power_exponent(f64::from_bits(1)), Some(-1074));
        assert_eq!(power_exponent(-0.5), Some(-1));
        assert_eq!(power_exponent(3.), None);
    }

    // Exactu128 crossproducts with denominator power-of-two cancellation.
    // This independently proves minimal upward tail rounding, not the series
    // implementation itself; the analytic tails are the frozen B4 obligations.
    fn at_least_fraction(x: f64, numerator: u128, denominator: u128) -> bool {
        let bits = x.to_bits();
        let mantissa = u128::from((bits & 0x000f_ffff_ffff_ffff) | (1u64 << 52));
        let exponent = ((bits >> 52) & 0x7ff) as i32 - 1023 - 52;
        let shift = exponent + denominator.trailing_zeros() as i32;
        let odd = denominator >> denominator.trailing_zeros();
        if shift >= 0 {
            (mantissa * odd) << shift >= numerator
        } else {
            mantissa * odd >= numerator << (-shift)
        }
    }

    #[test]
    fn literal_constants_and_rigorous_tail_roundings() {
        assert_eq!(PI.lo.to_bits(), 0x4009_21fb_5444_2d18);
        assert_eq!(PI.hi.to_bits(), 0x4009_21fb_5444_2d19);
        assert_eq!(LN2.lo.to_bits(), 0x3fe6_2e42_fefa_39ef);
        assert_eq!(LN2.hi.to_bits(), 0x3fe6_2e42_fefa_39f0);
        let factorial = (1u128..=27).product::<u128>();
        let numerator = 3u128.pow(27);
        let denominator = (1u128 << 27) * factorial;
        assert!(at_least_fraction(SIN_TAIL, numerator, denominator));
        assert!(!at_least_fraction(
            next_down(SIN_TAIL),
            numerator,
            denominator
        ));
        assert!(at_least_fraction(LOG_TAIL, 1, (1u128 << 62) * 195));
        assert!(!at_least_fraction(
            next_down(LOG_TAIL),
            1,
            (1u128 << 62) * 195
        ));
        assert_eq!(sin_bound(Pair::point(0.)).unwrap(), Pair::point(0.));
        assert_eq!(log_point(1.).unwrap(), Pair::point(0.));
    }

    #[test]
    fn global_nearest_tie_and_recursive_phase_sensitive_control() {
        // Independent exact integer formula; production native realization is
        // owned by tile.rs, no unused public sampling API is introduced.
        let finest_center = |z: u8, tile: u32, pixel: u32| {
            let k = 1u64 << (4 - z);
            k * (2 * (256 * u64::from(tile) + u64::from(pixel)) + 1) / 2
        };
        assert_eq!(finest_center(2, 0, 0), 2);
        assert_eq!(finest_center(2, 2, 0), 2050);
        assert_eq!(finest_center(3, 4, 0), 2049);
        assert_eq!(finest_center(2, 2, 255), 3070);
        let recursive = 3;
        assert_ne!(finest_center(2, 0, 0), recursive);
        assert_ne!(finest_center(2, 0, 0), 1); // lesser-index tie is wrong
    }

    #[test]
    fn fixed_layout_and_active_storage() {
        assert_eq!(std::mem::size_of::<StaticCrs>(), 1);
        assert_eq!(std::mem::size_of::<ZoomRect>(), 20);
        println!(
            "R2 GridPlan={} TargetGrid={} ZoomRange={} GridCaps={} Pair={} GridError={}",
            std::mem::size_of::<GridPlan>(),
            std::mem::size_of::<TargetGrid>(),
            std::mem::size_of::<ZoomRange>(),
            std::mem::size_of::<GridCaps>(),
            std::mem::size_of::<Pair>(),
            std::mem::size_of::<GridError>()
        );
        let step = H / 2_147_483_648.;
        let p = plan(
            StaticCrs::WebMercator3857,
            [0., step, 0., 0., 0., -step],
            [8, 8],
            ZoomRange::new(0, 24).unwrap(),
            caps(),
        )
        .unwrap();
        assert_eq!(p.rectangles().len(), 25);
        assert!(p.rectangles().iter().all(|r| r.reserved == [0; 3]));
    }
}
