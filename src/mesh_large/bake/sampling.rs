//! Pre-encode child correspondence and terminal source ledgers. Codec bytes
//! never enter this owner. The resident child BVH is bounded by the node lease.
use super::{
    frames::Frame,
    invalid,
    math::{self, V3},
    resource,
    source::{CapturedSource, Witness},
    types::{slot_index, FrozenFace, PreencodeChild, TerminalSample},
};
use crate::{gltf_source::source::TextureSlot, mesh_large::leaves::crops, JobError};
#[derive(Clone, Copy)]
struct Entry {
    child: usize,
    face: usize,
    lo: V3,
    hi: V3,
    centre: V3,
}
struct Branch {
    lo: V3,
    hi: V3,
    range: std::ops::Range<usize>,
    children: Option<[usize; 2]>,
}
pub(super) struct ChildQuery<'a> {
    children: &'a [PreencodeChild],
    entries: Vec<Entry>,
    tree: Vec<Branch>,
    work: u64,
    limit: u64,
}
pub(super) struct Signal {
    pub rgba: [f64; 4],
    pub terminal: TerminalSample,
    pub direct: bool,
    pub fill_fallback: bool,
    pub displacement: f64,
    pub drift: f64,
    pub witness_distance: f64,
    pub support_distance: f64,
}
fn linear(v: f64) -> f64 {
    if v <= 0.04045 {
        v / 12.92
    } else {
        ((v + 0.055) / 1.055).powf(2.4)
    }
}
fn srgb(v: f64) -> f64 {
    if v <= 0.0031308 {
        v * 12.92
    } else {
        1.055 * v.powf(1. / 2.4) - 0.055
    }
}
fn colour(slot: TextureSlot) -> bool {
    matches!(slot, TextureSlot::BaseColor | TextureSlot::Emissive)
}
impl<'a> ChildQuery<'a> {
    pub(super) fn new(
        children: &'a [PreencodeChild],
        memory: u64,
        limit: u64,
    ) -> Result<Self, JobError> {
        if children.len() > 8 {
            return Err(invalid("proxy has more than eight preencode children"));
        }
        let count = children
            .iter()
            .try_fold(0usize, |a, c| a.checked_add(c.faces.len()))
            .ok_or_else(|| resource("child query faces"))?;
        if (count as u64).checked_mul(256).is_none_or(|b| b > memory) {
            return Err(resource("child query BVH memory"));
        }
        let mut entries = Vec::new();
        entries
            .try_reserve_exact(count)
            .map_err(|_| resource("child query entries"))?;
        for (ci, c) in children.iter().enumerate() {
            for (fi, f) in c.faces.iter().enumerate() {
                let p = f.positions.map(|p| p.map(f64::from));
                let lo =
                    std::array::from_fn(|a| p.iter().map(|p| p[a]).fold(f64::INFINITY, f64::min));
                let hi = std::array::from_fn(|a| {
                    p.iter().map(|p| p[a]).fold(f64::NEG_INFINITY, f64::max)
                });
                entries.push(Entry {
                    child: ci,
                    face: fi,
                    lo,
                    hi,
                    centre: std::array::from_fn(|a| lo[a] * 0.5 + hi[a] * 0.5),
                });
            }
        }
        let mut out = Self {
            children,
            entries,
            tree: Vec::new(),
            work: 0,
            limit,
        };
        out.tree
            .try_reserve_exact(count.saturating_mul(2))
            .map_err(|_| resource("child query branches"))?;
        if count > 0 {
            out.branch(0..count)?;
        }
        Ok(out)
    }
    fn branch(&mut self, range: std::ops::Range<usize>) -> Result<usize, JobError> {
        let lo = std::array::from_fn(|a| {
            self.entries[range.clone()]
                .iter()
                .map(|e| e.lo[a])
                .fold(f64::INFINITY, f64::min)
        });
        let hi = std::array::from_fn(|a| {
            self.entries[range.clone()]
                .iter()
                .map(|e| e.hi[a])
                .fold(f64::NEG_INFINITY, f64::max)
        });
        let i = self.tree.len();
        self.tree.push(Branch {
            lo,
            hi,
            range: range.clone(),
            children: None,
        });
        if range.len() > 8 {
            let axis = (0..3)
                .max_by(|a, b| (hi[*a] - lo[*a]).total_cmp(&(hi[*b] - lo[*b])))
                .unwrap();
            self.entries[range.clone()].sort_by(|a, b| {
                a.centre[axis]
                    .total_cmp(&b.centre[axis])
                    .then((a.child, a.face).cmp(&(b.child, b.face)))
            });
            let mid = range.start + range.len() / 2;
            let a = self.branch(range.start..mid)?;
            let b = self.branch(mid..range.end)?;
            self.tree[i].children = Some([a, b]);
        }
        Ok(i)
    }
    fn nearest(
        &mut self,
        q: V3,
        f: &FrozenFace,
        n: V3,
        check: &mut dyn FnMut() -> Result<(), JobError>,
    ) -> Result<Option<(Entry, V3)>, JobError> {
        if self.tree.is_empty() {
            return Ok(None);
        }
        let mut stack = vec![0usize];
        let mut best: Option<(f64, Entry, V3)> = None;
        while let Some(i) = stack.pop() {
            check()?;
            self.work = self
                .work
                .checked_add(1)
                .ok_or_else(|| resource("child query work"))?;
            if self.work > self.limit {
                return Err(resource("child query work budget"));
            }
            let b = &self.tree[i];
            if best.is_some_and(|(d, _, _)| math::box_distance_lower(q, b.lo, b.hi) > d) {
                continue;
            }
            if let Some([a, b]) = b.children {
                stack.push(b);
                stack.push(a);
                continue;
            }
            for &e in &self.entries[b.range.clone()] {
                self.work += 1;
                if self.work > self.limit {
                    return Err(resource("child candidate work budget"));
                }
                let c = &self.children[e.child].faces[e.face];
                if c.source != f.source || c.material != f.material {
                    continue;
                }
                let p = c.positions.map(|p| p.map(f64::from));
                if !math::face_normal(p).is_some_and(|cn| math::dot(cn, n) > 0.) {
                    continue;
                }
                let bary = math::closest(q, p);
                let d = math::distance2(q, math::bary(p, bary));
                let tie = |old: Entry| {
                    let oldf = &self.children[old.child].faces[old.face];
                    (
                        c.source_triangle != f.source_triangle,
                        c.source_triangle,
                        e.child,
                        e.face,
                    ) < (
                        oldf.source_triangle != f.source_triangle,
                        oldf.source_triangle,
                        old.child,
                        old.face,
                    )
                };
                if best.is_none_or(|(old, entry, _)| d < old || (d == old && tie(entry))) {
                    best = Some((d, e, bary));
                }
            }
        }
        Ok(best.map(|(_, e, b)| (e, b)))
    }
    pub(super) fn signal(
        &mut self,
        source: &mut CapturedSource<'_>,
        face: &FrozenFace,
        p: V3,
        n: V3,
        frame: Frame,
        slot: TextureSlot,
        texel: f64,
        error: f64,
        depth: u32,
        check: &mut dyn FnMut() -> Result<(), JobError>,
    ) -> Result<Option<Signal>, JobError> {
        let direct = |source: &mut CapturedSource<'_>,
                      check: &mut dyn FnMut() -> Result<(), JobError>|
         -> Result<Option<Signal>, JobError> {
            let w = source.nearest(
                p,
                face.source,
                face.source_triangle,
                n,
                face.fill_bound,
                &face.donor_ranges,
                check,
            )?;
            direct_signal(source, w, p, frame, slot, texel, check)
        };
        // Accepted fills always use their immutable same-primitive eligibility.
        if face.fill_bound.is_some() {
            return direct(source, check);
        }
        let Some((e, bary)) = self.nearest(p, face, n, check)? else {
            return direct(source, check);
        };
        let child = &self.children[e.child];
        let donor = &child.faces[e.face];
        let Some(page) = donor.pages[slot_index(slot)].and_then(|i| child.pages.get(i)) else {
            return direct(source, check);
        };
        if page.slot != slot || page.material != face.material {
            return Err(invalid("child semantic binding closure"));
        }
        let uv = std::array::from_fn(|a| (0..3).map(|i| f64::from(donor.uv[i][a]) * bary[i]).sum());
        let sampler = crops::resolve(Some(&page.sampler))?;
        let pixel_uv = donor.uv.map(|uv| {
            [
                f64::from(uv[0]) * f64::from(page.width),
                f64::from(uv[1]) * f64::from(page.height),
            ]
        });
        let gradient = math::pixel_gradient(donor.positions.map(|p| p.map(f64::from)), pixel_uv)
            .ok_or_else(|| invalid("decoded child UV/geometry footprint"))?;
        let lod = (texel * gradient).log2();
        let taps = taps(&page.levels, sampler, uv, lod)?;
        let mut sum = [0.; 4];
        let mut centre = [0.; 3];
        let mut lo = [f64::INFINITY; 3];
        let mut hi = [f64::NEG_INFINITY; 3];
        let mut best = None;
        let mut total = 0.;
        for (level, i, weight) in taps {
            if weight == 0. {
                continue;
            }
            let l = &page.levels[level];
            let t = *l
                .samples
                .get(i)
                .ok_or_else(|| invalid("child terminal ledger shape"))?;
            let raw = std::array::from_fn::<_, 4, _>(|a| f64::from(l.rgba[i * 4 + a]) / 255.);
            if t.source_rank == u64::MAX {
                return direct(source, check);
            }
            let rgb = if slot == TextureSlot::Normal {
                [2. * raw[0] - 1., 2. * raw[1] - 1., 2. * raw[2] - 1.]
            } else {
                std::array::from_fn(|a| if colour(slot) { linear(raw[a]) } else { raw[a] })
            };
            for a in 0..3 {
                sum[a] += rgb[a] * weight;
                centre[a] += t.centre[a] * weight;
                lo[a] = lo[a].min(t.support_min[a]);
                hi[a] = hi[a].max(t.support_max[a]);
            }
            sum[3] += raw[3] * weight;
            total += weight;
            if best.is_none_or(|(old, _, _)| weight > old) {
                best = Some((weight, t, rgb));
            }
        }
        if total == 0. {
            return direct(source, check);
        }
        let (_, mut terminal, _) = best.unwrap();
        terminal.centre = centre.map(|v| v / total);
        terminal.support_min = lo;
        terminal.support_max = hi;
        terminal.weight = 1;
        terminal.centre_is_centroid = true;
        let displacement = math::distance_upper(p, terminal.centre);
        if displacement > error {
            return direct(source, check);
        }
        let rgb = if slot == TextureSlot::Normal {
            // A real child fragment samples RGB first, then applies THAT
            // fragment's emitted TBN, never a mip texel's representative frame.
            let donor_frame = Frame::interpolate(
                donor.frames.map(|f| f.normal),
                donor.frames.map(|f| f.tangent),
                donor.frames.map(|f| f.bitangent),
                bary,
            )?;
            let world = donor_frame.transform(super::frames::effective_direction(
                [sum[0] / total, sum[1] / total, sum[2] / total],
                page.normal_scale,
            )?)?;
            frame.inverse_direction(world)?.map(|v| v * 0.5 + 0.5)
        } else {
            std::array::from_fn(|a| {
                let v = sum[a] / total;
                if colour(slot) {
                    srgb(v)
                } else {
                    v
                }
            })
        };
        let rgba = [rgb[0], rgb[1], rgb[2], sum[3] / total];
        // Deep recursive drift is measured against the independent captured
        // source path, never used as an unannounced direct-only replacement.
        let mut drift = 0.;
        if depth >= 6 {
            if let Some(reference) = direct(source, check)? {
                drift = (0..4)
                    .map(|a| (rgba[a] - reference.rgba[a]).abs() * 255.)
                    .fold(0., f64::max);
            }
        }
        Ok(Some(Signal {
            rgba,
            terminal,
            direct: false,
            fill_fallback: false,
            displacement,
            drift,
            witness_distance: math::distance_upper(
                p,
                math::bary(
                    source
                        .record(terminal.source_rank, terminal.barycentric)?
                        .triangle
                        .triangle
                        .positions
                        .map(|p| p.map(f64::from)),
                    terminal.barycentric,
                ),
            ),
            support_distance: math::support_distance_upper(
                p,
                terminal.support_min,
                terminal.support_max,
            ),
        }))
    }
}
fn direct_signal(
    source: &mut CapturedSource<'_>,
    w: Witness,
    p: V3,
    frame: Frame,
    slot: TextureSlot,
    texel: f64,
    check: &mut dyn FnMut() -> Result<(), JobError>,
) -> Result<Option<Signal>, JobError> {
    let lod = source.lod(w, slot, texel)?;
    let Some((raw, _uv)) = source.signal(w, slot, lod, check)? else {
        return Ok(None);
    };
    let rgba = if slot == TextureSlot::Normal {
        let material = source.material(w.triangle.material)?;
        let scale = material
            .pointer("/normalTexture/scale")
            .and_then(serde_json::Value::as_f64)
            .unwrap_or(1.);
        let v = super::frames::reexpress(
            [raw[0], raw[1], raw[2]],
            scale,
            CapturedSource::frame(w)?,
            frame,
        )?;
        [v[0] * 0.5 + 0.5, v[1] * 0.5 + 0.5, v[2] * 0.5 + 0.5, raw[3]]
    } else {
        raw
    };
    let q = math::bary(
        w.triangle.triangle.positions.map(|p| p.map(f64::from)),
        w.bary,
    );
    let (support_min, support_max) = source.footprint(w, slot, lod)?;
    let terminal = TerminalSample {
        source_rank: w.rank,
        barycentric: w.bary,
        centre: q,
        support_min,
        support_max,
        weight: 1,
        centre_is_centroid: false,
    };
    Ok(Some(Signal {
        rgba,
        terminal,
        direct: true,
        fill_fallback: w.fill_fallback,
        displacement: math::distance_upper(p, q),
        drift: 0.,
        witness_distance: math::distance_upper(p, q),
        support_distance: math::support_distance_upper(p, support_min, support_max),
    }))
}
fn taps(
    levels: &[super::types::SignalLevel],
    s: crops::SourceSampler,
    uv: [f64; 2],
    lod: f64,
) -> Result<Vec<(usize, usize, f64)>, JobError> {
    if levels.is_empty() || !lod.is_finite() || uv.iter().any(|v| !v.is_finite()) {
        return Err(invalid("child page sample coordinates"));
    }
    let filter = if lod <= 0. {
        s.mag_filter
    } else {
        s.min_filter
    };
    let l = if matches!(filter, 9984..=9987) {
        lod.max(0.).min((levels.len() - 1) as f64)
    } else {
        0.
    };
    let blend = matches!(filter, 9986 | 9987);
    let a = if blend {
        l.floor() as usize
    } else {
        l.round() as usize
    };
    let b = if blend {
        (a + 1).min(levels.len() - 1)
    } else {
        a
    };
    let f = if blend { l - a as f64 } else { 0. };
    let mut out = Vec::with_capacity(8);
    for (m, weight) in [(a, 1. - f), (b, f)] {
        if weight == 0. {
            continue;
        }
        let level = &levels[m];
        let xy = [
            uv[0].clamp(0., 1.) * f64::from(level.width) - 0.5,
            uv[1].clamp(0., 1.) * f64::from(level.height) - 0.5,
        ];
        let index = |x: i64, y: i64| {
            y.clamp(0, i64::from(level.height) - 1) as usize * level.width as usize
                + x.clamp(0, i64::from(level.width) - 1) as usize
        };
        if matches!(filter, 9728 | 9984 | 9986) {
            out.push((
                m,
                index((xy[0] + 0.5).floor() as i64, (xy[1] + 0.5).floor() as i64),
                weight,
            ));
            continue;
        }
        let x = xy[0].floor() as i64;
        let y = xy[1].floor() as i64;
        let fx = xy[0] - x as f64;
        let fy = xy[1] - y as f64;
        for (dx, dy, w) in [
            (0, 0, (1. - fx) * (1. - fy)),
            (1, 0, fx * (1. - fy)),
            (0, 1, (1. - fx) * fy),
            (1, 1, fx * fy),
        ] {
            out.push((m, index(x + dx, y + dy), weight * w));
        }
    }
    Ok(out)
}
