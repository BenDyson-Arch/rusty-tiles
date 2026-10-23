//! Pre-encode child correspondence and terminal source ledgers. Codec bytes
//! never enter this owner. The resident child BVH is bounded by the node lease, and
//! child texels are read through a cache of fixed size ([`TexelCache`]).
use super::{
    frames::Frame,
    invalid,
    math::{self, V3},
    resource,
    retained::{ChildLevel, ChildView, TexelCache},
    source::{CapturedSource, Witness},
    types::{slot_index, FrozenFace, TerminalSample},
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
    children: &'a [ChildView],
    texels: TexelCache,
    entries: Vec<Entry>,
    tree: Vec<Branch>,
    /// Reused depth-first stack of [`ChildQuery::nearest`]: branch and lower bound.
    stack: Vec<(usize, f64)>,
    work: u64,
    limit: u64,
}
/// Actual decoded proxy coverage point and its emitted fragment frame.
pub(super) struct ProxySample<'a> {
    pub face: &'a FrozenFace,
    pub position: V3,
    pub geometric_normal: V3,
    pub frame: Frame,
    pub texel_size: f64,
    pub geometric_error: f64,
    pub depth: u32,
}
pub(super) struct Signal {
    pub rgba: [f64; 4],
    pub terminal: TerminalSample,
    pub direct: bool,
    pub fill_fallback: bool,
    /// A direct sample whose witness faces away from the proxy: no same-primitive
    /// source triangle faces it (see `CapturedSource::nearest_result`).
    pub facing_fallback: bool,
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
    /// `memory` bounds the child BVH; `texels` bounds the child texel cache.
    pub(super) fn new(
        children: &'a [ChildView],
        memory: u64,
        texels: u64,
        limit: u64,
    ) -> Result<Self, JobError> {
        if children.len() > 8 {
            return Err(invalid("proxy has more than eight preencode children"));
        }
        let count = children
            .iter()
            .try_fold(0usize, |a, c| a.checked_add(c.faces.len()))
            .ok_or_else(|| resource("child query faces"))?;
        if (count as u64).checked_mul(512).is_none_or(|b| b > memory) {
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
            texels: TexelCache::new(children, texels)?,
            entries,
            tree: Vec::new(),
            stack: Vec::new(),
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
    /// Child tiles read from packets so far (diagnostics).
    pub(super) fn tile_reads(&self) -> u64 {
        self.texels.reads
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
        // Nearest-first: the nearer branch is searched first so the bound tightens
        // early. A branch is pruned only when its lower bound exceeds the best distance
        // by more than the arithmetic slack of a computed distance, and candidates are
        // compared by a strict total order, so the answer is the exhaustive one.
        let slack = 1e-12 * (1. + math::dot(q, q));
        let mut stack = std::mem::take(&mut self.stack);
        stack.clear();
        let mut best: Option<(f64, Entry, V3)> = None;
        stack.push((
            0,
            math::box_distance_lower(q, self.tree[0].lo, self.tree[0].hi),
        ));
        while let Some((i, lower)) = stack.pop() {
            check()?;
            self.work = self
                .work
                .checked_add(1)
                .ok_or_else(|| resource("child query work"))?;
            if self.work > self.limit {
                return Err(resource("child query work budget"));
            }
            if best.is_some_and(|(d, _, _)| lower > d + slack) {
                continue;
            }
            let b = &self.tree[i];
            if let Some([a, b]) = b.children {
                let la = math::box_distance_lower(q, self.tree[a].lo, self.tree[a].hi);
                let lb = math::box_distance_lower(q, self.tree[b].lo, self.tree[b].hi);
                if la <= lb {
                    stack.push((b, lb));
                    stack.push((a, la));
                } else {
                    stack.push((a, la));
                    stack.push((b, lb));
                }
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
                // total_cmp: a NaN distance orders after every finite one, so it never
                // sticks as the best candidate.
                if best.is_none_or(|(old, entry, _)| match d.total_cmp(&old) {
                    std::cmp::Ordering::Less => true,
                    std::cmp::Ordering::Equal => tie(entry),
                    std::cmp::Ordering::Greater => false,
                }) {
                    best = Some((d, e, bary));
                }
            }
        }
        self.stack = stack;
        Ok(best.map(|(_, e, b)| (e, b)))
    }
    pub(super) fn signal(
        &mut self,
        source: &mut CapturedSource<'_>,
        sample: ProxySample<'_>,
        slot: TextureSlot,
        check: &mut dyn FnMut() -> Result<(), JobError>,
    ) -> Result<Option<Signal>, JobError> {
        let ProxySample {
            face,
            position: p,
            geometric_normal: n,
            frame,
            texel_size: texel,
            geometric_error: error,
            depth,
        } = sample;
        let direct = |source: &mut CapturedSource<'_>,
                      check: &mut dyn FnMut() -> Result<(), JobError>|
         -> Result<Option<Signal>, JobError> {
            let w = source.nearest(p, face, n, check)?;
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
        let Some(binding) = donor.bindings[slot_index(slot)] else {
            return direct(source, check);
        };
        let page = child
            .pages
            .get(binding.page)
            .ok_or_else(|| invalid("child slot binding page closure"))?;
        if page.slot != slot
            || (page.material != source.class(face.material)
                && !(super::experiment::options().plain && page.material == face.material))
        {
            return Err(invalid("child semantic binding closure"));
        }
        let uv =
            std::array::from_fn(|a| (0..3).map(|i| f64::from(binding.uv[i][a]) * bary[i]).sum());
        let sampler = crops::resolve(Some(&page.sampler))?;
        let pixel_uv = binding.uv.map(|uv| {
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
        for (level, [x, y], weight) in taps {
            if weight == 0. {
                continue;
            }
            let (rgba, t) = self
                .texels
                .texel(self.children, e.child, binding.page, level, x, y)?;
            let raw = std::array::from_fn::<_, 4, _>(|a| f64::from(rgba[a]) / 255.);
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
        if super::experiment::options().plain {
            terminal.centre = p;
            terminal.support_min = p;
            terminal.support_max = p;
        }
        let displacement = math::distance_upper(p, terminal.centre);
        if !super::experiment::options().plain && displacement > error {
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
        if !super::experiment::options().plain && depth >= 6 {
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
            facing_fallback: false,
            displacement,
            drift,
            witness_distance: if super::experiment::options().plain {
                0.
            } else {
                math::distance_upper(
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
                )
            },
            support_distance: math::support_distance_upper(
                p,
                terminal.support_min,
                terminal.support_max,
            ),
        }))
    }
}
pub(super) fn direct_signal(
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
        facing_fallback: w.facing_fallback,
        displacement: math::distance_upper(p, q),
        drift: 0.,
        witness_distance: math::distance_upper(p, q),
        support_distance: math::support_distance_upper(p, support_min, support_max),
    }))
}
/// The level and clamped texel `[x, y]` of each tap of a child page sample, with its
/// weight.
fn taps(
    levels: &[ChildLevel],
    s: crops::SourceSampler,
    uv: [f64; 2],
    lod: f64,
) -> Result<Vec<(usize, [u32; 2], f64)>, JobError> {
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
            [
                x.clamp(0, i64::from(level.width) - 1) as u32,
                y.clamp(0, i64::from(level.height) - 1) as u32,
            ]
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mesh_large::{
        bake::types::{ChildFace, PositionWord, PreencodeChild, SourceKey},
        records::{Bounds, NodeKind, NodeRecord},
    };

    /// Distance, ordering key (own lineage, lineage, child, face) and barycentrics.
    type Expected = (f64, (bool, u64, usize, usize), V3);

    #[test]
    fn child_nearest_matches_exhaustive_search_including_ties() {
        let mut state = 0x853c_49e6_748f_ea9bu64;
        let mut rng = move || {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            (state >> 11) as f64 / (1u64 << 53) as f64
        };
        let frame = Frame {
            tangent: [1., 0., 0.],
            bitangent: [0., 1., 0.],
            normal: [0., 0., 1.],
        };
        let source = |primitive| SourceKey {
            node: 0,
            mesh: 0,
            primitive,
            instance: 0,
        };
        // Four children of 90 faces over two sources and two materials; every fifth face
        // repeats the previous one exactly, so distances tie.
        let children: Vec<PreencodeChild> = (0..4u64)
            .map(|c| {
                let mut faces: Vec<ChildFace> = Vec::new();
                for i in 0..90u64 {
                    let positions = if i % 5 == 4 {
                        faces.last().unwrap().positions
                    } else {
                        std::array::from_fn(|_| std::array::from_fn(|_| (rng() * 3.) as f32))
                    };
                    faces.push(ChildFace {
                        source: source((i % 2) as u32),
                        source_triangle: (c * 1000 + i) % 7,
                        material: (i % 3 == 0) as u32,
                        positions,
                        frames: [frame; 3],
                        bindings: [None; 5],
                    });
                }
                PreencodeChild {
                    node: NodeRecord {
                        cell_depth: 1,
                        cell: c,
                        path_len: 0,
                        path: 0,
                        last_octant: 0,
                        kind: NodeKind::Leaf,
                        children: 0,
                        first: c * 90,
                        end: c * 90 + 90,
                        bytes: 0,
                        bounds: Bounds::EMPTY,
                    },
                    texel_size: 0.,
                    faces,
                    pages: Vec::new(),
                }
            })
            .collect();
        let children: Vec<ChildView> = children.into_iter().map(ChildView::resident).collect();
        let mut query = ChildQuery::new(&children, 1 << 30, 0, u64::MAX / 4).unwrap();
        assert!(query.tree.len() > 16);
        let ok = &mut || Ok(());
        let mut checked = 0;
        for i in 0..800usize {
            let face = FrozenFace {
                ordinal: 0,
                source: source((i % 2) as u32),
                source_triangle: (i % 7) as u64,
                material: (i % 4 == 0) as u32,
                positions: [[0.; 3]; 3],
                words: [PositionWord::Float([0; 3]); 3],
                transform: Default::default(),
                welds: [[0; 3]; 3],
                features: Vec::new(),
                fill_bound: None,
                donor_ranges: Vec::new(),
            };
            let q: V3 = if i % 2 == 0 {
                let c = &children[i % 4].faces[(i * 3) % 90];
                c.positions[i % 3].map(f64::from)
            } else {
                std::array::from_fn(|_| rng() * 4. - 0.5)
            };
            let n = math::unit(std::array::from_fn(|_| rng() - 0.5)).unwrap();
            // Exhaustive: same filters, then (distance, own-lineage, lineage, child, face).
            let mut expected: Option<Expected> = None;
            for (ci, child) in children.iter().enumerate() {
                for (fi, c) in child.faces.iter().enumerate() {
                    if c.source != face.source || c.material != face.material {
                        continue;
                    }
                    let p = c.positions.map(|p| p.map(f64::from));
                    if !math::face_normal(p).is_some_and(|cn| math::dot(cn, n) > 0.) {
                        continue;
                    }
                    let bary = math::closest(q, p);
                    let d = math::distance2(q, math::bary(p, bary));
                    let order = (
                        c.source_triangle != face.source_triangle,
                        c.source_triangle,
                        ci,
                        fi,
                    );
                    if expected.is_none_or(|(od, oo, _)| d < od || (d == od && order < oo)) {
                        expected = Some((d, order, bary));
                    }
                }
            }
            let actual = query.nearest(q, &face, n, ok).unwrap();
            match (actual, expected) {
                (None, None) => {}
                (Some((e, bary)), Some((_, (_, _, ci, fi), eb))) => {
                    assert_eq!((e.child, e.face), (ci, fi), "query {i}");
                    assert_eq!(bary.map(f64::to_bits), eb.map(f64::to_bits));
                    checked += 1;
                }
                other => panic!("query {i}: {:?}", other.0.map(|(e, _)| (e.child, e.face))),
            }
        }
        assert!(checked > 400);
    }
}
