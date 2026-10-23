//! Unaccepted experiments. Flags only work with `fast-lod-experiments` enabled.
//! The inverse projection is deliberately not an exact nearest-surface query.
use super::{
    math::{self, V3},
    source::{CapturedSource, Witness},
    types::FrozenFace,
};
use crate::JobError;
pub(super) struct Options {
    pub centroid: bool,
    pub inverse: bool,
    pub plain: bool,
}
pub(super) fn options() -> &'static Options {
    static OPTIONS: std::sync::OnceLock<Options> = std::sync::OnceLock::new();
    OPTIONS.get_or_init(|| {
        let flag =
            |name| cfg!(feature = "fast-lod-experiments") && std::env::var_os(name).is_some();
        let inverse = flag("RT_EXP_INVERSE");
        Options {
            centroid: inverse || flag("RT_EXP_CENTROID"),
            inverse,
            plain: flag("RT_EXP_PLAIN_CHILD"),
        }
    })
}
pub(super) fn placeholder(rank: u64) -> super::types::TerminalSample {
    super::types::TerminalSample {
        source_rank: rank,
        barycentric: [1., 0., 0.],
        centre: [0.; 3],
        support_min: [0.; 3],
        support_max: [0.; 3],
        weight: 1,
        centre_is_centroid: true,
    }
}
fn orient(a: [f64; 2], b: [f64; 2], c: [f64; 2]) -> f64 {
    (b[0] - a[0]) * (c[1] - a[1]) - (b[1] - a[1]) * (c[0] - a[0])
}
fn bary(q: [f64; 2], t: [[f64; 2]; 3]) -> V3 {
    let d = orient(t[0], t[1], t[2]);
    let a = orient(t[1], t[2], q) / d;
    let b = orient(t[2], t[0], q) / d;
    [a, b, 1. - a - b]
}
pub(super) struct Draw {
    lo: [u32; 2],
    width: u32,
    winners: Vec<Option<Witness>>,
}
impl Draw {
    pub(super) fn get(&self, x: u32, y: u32) -> Option<Witness> {
        self.winners[((y - self.lo[1]) * self.width + x - self.lo[0]) as usize]
    }
}
/// Orthographic source triangle -> proxy UV rasterisation. For each covered
/// proxy pixel, choose the nearest projected facing source surface. Fills and
/// projection holes use the established lookup path in the caller.
pub(super) fn draw(
    source: &CapturedSource<'_>,
    face: &FrozenFace,
    uv: [[f64; 2]; 3],
    lo: [u32; 2],
    hi: [u32; 2],
    pad: f64,
    check: &mut dyn FnMut() -> Result<(), JobError>,
) -> Result<Draw, JobError> {
    let width = hi[0].saturating_sub(lo[0]);
    let height = hi[1].saturating_sub(lo[1]);
    let p = face.positions.map(|p| p.map(f64::from));
    let e1 = math::sub(p[1], p[0]);
    let e2 = math::sub(p[2], p[0]);
    let a = math::dot(e1, e1);
    let b = math::dot(e1, e2);
    let c = math::dot(e2, e2);
    let det = a * c - b * b;
    let project = |q: V3| {
        let v = math::sub(q, p[0]);
        let d = math::dot(v, e1);
        let e = math::dot(v, e2);
        let y = (c * d - b * e) / det;
        let z = (a * e - b * d) / det;
        std::array::from_fn::<_, 2, _>(|k| uv[0][k] * (1. - y - z) + uv[1][k] * y + uv[2][k] * z)
    };
    let n = (width as usize) * (height as usize);
    let mut winners: Vec<Option<Witness>> = vec![None; n];
    let mut queries = vec![None; n];
    for y in lo[1]..hi[1] {
        for x in lo[0]..hi[0] {
            let (area, q) = super::raster::coverage(uv, x, y);
            if area > 0. {
                queries[((y - lo[1]) * width + x - lo[0]) as usize] =
                    Some((q, math::bary(p, bary(q, uv))));
            }
        }
    }
    if det > 0. {
        for w in source.projection_candidates(face, pad, check)? {
            let sp = w.triangle.triangle.positions.map(|p| p.map(f64::from));
            let t = sp.map(project);
            if orient(t[0], t[1], t[2]).abs() < 1e-16 {
                continue;
            }
            let l: [u32; 2] = std::array::from_fn(|k| {
                (t.iter()
                    .map(|p| p[k])
                    .fold(f64::INFINITY, f64::min)
                    .floor()
                    .max(lo[k] as f64) as u32)
                    .min(hi[k])
            });
            let h: [u32; 2] = std::array::from_fn(|k| {
                (t.iter()
                    .map(|p| p[k])
                    .fold(f64::NEG_INFINITY, f64::max)
                    .ceil()
                    .max(lo[k] as f64) as u32)
                    .min(hi[k])
            });
            for y in l[1]..h[1] {
                for x in l[0]..h[0] {
                    let i = ((y - lo[1]) * width + x - lo[0]) as usize;
                    let Some((q, world)) = queries[i] else {
                        continue;
                    };
                    let bs = bary(q, t);
                    if bs.iter().any(|&v| !(0. ..=1.).contains(&v)) {
                        continue;
                    }
                    let d = math::distance2(world, math::bary(sp, bs));
                    if winners[i].is_none_or(|old| (d, w.rank) < (old.distance2, old.rank)) {
                        winners[i] = Some(Witness {
                            bary: bs,
                            distance2: d,
                            ..w
                        });
                    }
                }
            }
        }
    }
    Ok(Draw { lo, width, winners })
}
