//! Concrete captured-source queries. The hierarchy is streamed once into an
//! immutable postorder spill; a query owns only its bounded traversal stack.
use super::{
    frames::Frame,
    invalid,
    math::{self, V3},
    resource,
    types::{FrozenFace, SourceKey},
    unsupported,
};
use crate::{
    gltf_source::source::{self, TextureSlot},
    mesh_large::{
        attributes::{FullTriangle, COLOR, NORMAL, TANGENT},
        full::FullCaptured,
        leaves::{crops, mips},
        records::{NodeKind, NodeRecord},
    },
    spill::{Record, Sequence, SequenceWriter},
    JobError,
};
use std::{cmp::Ordering, collections::BTreeMap};

#[derive(Clone, Copy)]
struct QueryNode {
    first: u64,
    end: u64,
    lo: [u32; 3],
    hi: [u32; 3],
    children: [u64; 8],
    count: u8,
}
impl PartialEq for QueryNode {
    fn eq(&self, b: &Self) -> bool {
        (self.first, self.end) == (b.first, b.end)
    }
}
impl Eq for QueryNode {}
impl Ord for QueryNode {
    fn cmp(&self, b: &Self) -> Ordering {
        (self.first, self.end).cmp(&(b.first, b.end))
    }
}
impl PartialOrd for QueryNode {
    fn partial_cmp(&self, b: &Self) -> Option<Ordering> {
        Some(self.cmp(b))
    }
}
impl Record for QueryNode {
    const SIZE: usize = 105;
    fn encode(&self, b: &mut [u8]) {
        b[..8].copy_from_slice(&self.first.to_le_bytes());
        b[8..16].copy_from_slice(&self.end.to_le_bytes());
        for (i, x) in self.lo.iter().chain(self.hi.iter()).enumerate() {
            b[16 + i * 4..20 + i * 4].copy_from_slice(&x.to_le_bytes());
        }
        for (i, x) in self.children.iter().enumerate() {
            b[40 + i * 8..48 + i * 8].copy_from_slice(&x.to_le_bytes());
        }
        b[104] = self.count;
    }
    fn decode(b: &[u8]) -> Self {
        Self {
            first: u64::from_le_bytes(b[..8].try_into().unwrap()),
            end: u64::from_le_bytes(b[8..16].try_into().unwrap()),
            lo: std::array::from_fn(|i| {
                u32::from_le_bytes(b[16 + i * 4..20 + i * 4].try_into().unwrap())
            }),
            hi: std::array::from_fn(|i| {
                u32::from_le_bytes(b[28 + i * 4..32 + i * 4].try_into().unwrap())
            }),
            children: std::array::from_fn(|i| {
                u64::from_le_bytes(b[40 + i * 8..48 + i * 8].try_into().unwrap())
            }),
            count: b[104],
        }
    }
}
struct Pending {
    node: NodeRecord,
    children: [u64; 8],
    count: u8,
}
fn finish(p: Pending, w: &mut SequenceWriter<QueryNode>) -> Result<u64, JobError> {
    if (p.node.kind == NodeKind::Internal && p.count != p.node.children)
        || (p.node.kind == NodeKind::Leaf && p.count != 0)
    {
        return Err(invalid("source hierarchy child closure"));
    }
    let i = w.len();
    w.push(&QueryNode {
        first: p.node.first,
        end: p.node.end,
        lo: p.node.bounds.min.map(f32::to_bits),
        hi: p.node.bounds.max.map(f32::to_bits),
        children: p.children,
        count: p.count,
    })?;
    Ok(i)
}
pub(super) fn key(t: &FullTriangle) -> SourceKey {
    SourceKey {
        node: t.triangle.source.node,
        mesh: t.triangle.source.mesh,
        primitive: t.triangle.source.primitive,
        instance: t.expanded_ordinal,
    }
}
#[derive(Clone, Copy, Debug)]
pub(super) struct Witness {
    pub rank: u64,
    pub triangle: FullTriangle,
    pub bary: V3,
    pub distance2: f64,
    pub fill_fallback: bool,
}
struct Cache {
    image: usize,
    class: mips::Class,
    chain: mips::Chain,
    sampler: crops::SourceSampler,
}
pub(crate) struct CapturedSource<'a> {
    pub(super) captured: &'a FullCaptured,
    index: Sequence<QueryNode>,
    root: u64,
    first: u64,
    end: u64,
    work: u64,
    limit: u64,
    cache_limit: u64,
    cache: Option<Cache>,
}
impl<'a> CapturedSource<'a> {
    pub(crate) fn new(
        captured: &'a FullCaptured,
        cache_limit: u64,
        check: &mut dyn FnMut() -> Result<(), JobError>,
    ) -> Result<Self, JobError> {
        let mut writer = SequenceWriter::new(&captured.scratch, "proxy-source-index", 4096)?;
        let mut reader = captured.tree.nodes.reader()?;
        let mut stack: Vec<Pending> = Vec::new();
        stack
            .try_reserve_exact(65)
            .map_err(|_| resource("source index stack"))?;
        let mut root = None;
        for next in &mut reader {
            check()?;
            let node = next?;
            while stack.last().is_some_and(|p| p.node.depth() >= node.depth()) {
                let i = finish(stack.pop().unwrap(), &mut writer)?;
                if let Some(p) = stack.last_mut() {
                    if p.count >= 8 {
                        return Err(invalid("source index fanout"));
                    }
                    p.children[p.count as usize] = i;
                    p.count += 1;
                } else {
                    if root.replace(i).is_some() {
                        return Err(invalid("multiple source roots"));
                    }
                }
            }
            if stack.len() >= 65 {
                return Err(resource("source hierarchy depth"));
            }
            if let Some(p) = stack.last() {
                if node.parent() != Some(p.node.identity())
                    || node.first < p.node.first
                    || node.end > p.node.end
                {
                    return Err(invalid("source index preorder"));
                }
            }
            stack.push(Pending {
                node,
                children: [0; 8],
                count: 0,
            });
        }
        while let Some(p) = stack.pop() {
            let i = finish(p, &mut writer)?;
            if let Some(p) = stack.last_mut() {
                if p.count >= 8 {
                    return Err(invalid("source index fanout"));
                }
                p.children[p.count as usize] = i;
                p.count += 1;
            } else {
                if root.replace(i).is_some() {
                    return Err(invalid("multiple source roots"));
                }
            }
        }
        Ok(Self {
            captured,
            index: writer.finish()?,
            root: root.ok_or_else(|| invalid("empty captured hierarchy"))?,
            first: 0,
            end: captured.triangles.len(),
            work: 0,
            limit: 0,
            cache_limit,
            cache: None,
        })
    }
    pub(super) fn begin(&mut self, node: NodeRecord, limit: u64) -> Result<(), JobError> {
        if node.first >= node.end || node.end > self.captured.triangles.len() || limit == 0 {
            return Err(invalid("proxy source query range/budget"));
        }
        self.first = node.first;
        self.end = node.end;
        self.work = 0;
        self.limit = limit;
        Ok(())
    }
    fn charge(&mut self) -> Result<(), JobError> {
        self.work = self
            .work
            .checked_add(1)
            .ok_or_else(|| resource("source query work overflow"))?;
        if self.work > self.limit {
            return Err(resource("source correspondence work budget"));
        }
        Ok(())
    }
    /// Stable nearest actual surface witness. Distance never participates in H17
    /// acceptance: source appearance may move arbitrarily on a certified proxy.
    pub(super) fn nearest(
        &mut self,
        q: V3,
        face: &FrozenFace,
        normal: V3,
        check: &mut dyn FnMut() -> Result<(), JobError>,
    ) -> Result<Witness, JobError> {
        let k = face.source;
        let lineage = face.source_triangle;
        let fill = face.fill_bound;
        let ranges = &face.donor_ranges;
        let captured = self.captured;
        let mut best: Option<Witness> = None;
        let mut unfiltered: Option<Witness> = None;
        let mut stack = Vec::new();
        stack
            .try_reserve_exact(8 * 65)
            .map_err(|_| resource("source query stack"))?;
        stack.push(self.root);
        while let Some(i) = stack.pop() {
            check()?;
            self.charge()?;
            let n = self
                .index
                .range(i, i + 1, 4096)?
                .next()
                .transpose()?
                .ok_or_else(|| invalid("missing query node"))?;
            if n.end <= self.first
                || n.first >= self.end
                || !ranges.iter().any(|r| n.first < r.end && n.end > r.start)
            {
                continue;
            }
            let lower = math::box_distance_lower(
                q,
                n.lo.map(|v| f64::from(f32::from_bits(v))),
                n.hi.map(|v| f64::from(f32::from_bits(v))),
            );
            if best.is_some_and(|b| lower > b.distance2)
                && (fill.is_none() || unfiltered.is_some_and(|b| lower > b.distance2))
            {
                continue;
            }
            if n.count > 0 {
                for &child in n.children[..n.count as usize].iter().rev() {
                    if stack.len() >= 8 * 65 {
                        return Err(resource("source query stack bound"));
                    }
                    stack.push(child);
                }
                continue;
            }
            let first = n.first.max(self.first);
            let end = n.end.min(self.end);
            let mut r = captured.triangles.range(first, end, 4096)?;
            let mut rank = first;
            while let Some(t) = r.next().transpose()? {
                self.charge()?;
                if ranges.iter().any(|r| r.contains(&rank)) && key(&t) == k {
                    let p = t.triangle.positions.map(|p| p.map(f64::from));
                    let bary = math::closest(q, p);
                    let d = math::distance2(q, math::bary(p, bary));
                    if !d.is_finite() {
                        return Err(invalid("nonfinite source witness distance"));
                    }
                    let witness = Witness {
                        rank,
                        triangle: t,
                        bary,
                        distance2: d,
                        fill_fallback: false,
                    };
                    let earlier = |b: Witness| {
                        d < b.distance2
                            || (d == b.distance2
                                && (
                                    t.triangle.source.triangle != lineage,
                                    t.triangle.source.triangle,
                                    rank,
                                ) < (
                                    b.triangle.triangle.source.triangle != lineage,
                                    b.triangle.triangle.source.triangle,
                                    b.rank,
                                ))
                    };
                    if fill.is_some() && unfiltered.is_none_or(earlier) {
                        unfiltered = Some(witness);
                    }
                    if math::face_normal(p).is_some_and(|n| math::dot(n, normal) > 0.)
                        && best.is_none_or(earlier)
                    {
                        best = Some(witness);
                    }
                }
                rank += 1;
                check()?;
            }
        }
        if let Some(bound) = fill {
            if best.is_none_or(|b| {
                (if b.distance2 == 0. {
                    0.
                } else {
                    b.distance2.sqrt().next_up()
                }) > bound
            }) {
                let mut w = unfiltered
                    .ok_or_else(|| invalid("accepted fill has no same-primitive donor"))?;
                // Allow finite arithmetic outward error, never a new appearance bound.
                if w.distance2.sqrt() > bound.next_up() {
                    return Err(invalid(
                        "accepted fill donor violates immutable eligibility bound",
                    ));
                }
                w.fill_fallback = true;
                return Ok(w);
            }
        }
        best.ok_or_else(|| {
            unsupported("proxy has no facing same-primitive source appearance donor")
        })
    }
    /// The weld key, not the replacement face's Origin, selects canonical colour.
    pub(super) fn color(
        &mut self,
        k: SourceKey,
        own_triangle: u64,
        weld: [u32; 3],
        check: &mut dyn FnMut() -> Result<(), JobError>,
    ) -> Result<[f32; 4], JobError> {
        let captured = self.captured;
        let q = weld.map(|v| f64::from(f32::from_bits(v)));
        let mut stack = vec![self.root];
        let mut best: Option<((bool, u64, u8), [f32; 4])> = None;
        while let Some(i) = stack.pop() {
            self.charge()?;
            check()?;
            let n = self
                .index
                .range(i, i + 1, 4096)?
                .next()
                .transpose()?
                .ok_or_else(|| invalid("missing colour query node"))?;
            if n.end <= self.first
                || n.first >= self.end
                || math::box_distance_lower(
                    q,
                    n.lo.map(|v| f64::from(f32::from_bits(v))),
                    n.hi.map(|v| f64::from(f32::from_bits(v))),
                ) > 0.
            {
                continue;
            }
            if n.count > 0 {
                for &c in n.children[..n.count as usize].iter().rev() {
                    if stack.len() >= 8 * 65 {
                        return Err(resource("colour query stack"));
                    }
                    stack.push(c);
                }
                continue;
            }
            let mut r =
                captured
                    .triangles
                    .range(n.first.max(self.first), n.end.min(self.end), 4096)?;
            while let Some(t) = r.next().transpose()? {
                self.charge()?;
                check()?;
                if key(&t) != k {
                    continue;
                }
                for c in 0..3 {
                    let bits =
                        t.triangle.positions[c].map(|v| if v == 0. { 0 } else { v.to_bits() });
                    if bits != weld {
                        continue;
                    }
                    let order = (
                        t.triangle.source.triangle != own_triangle,
                        t.triangle.source.triangle,
                        c as u8,
                    );
                    if best.as_ref().is_none_or(|(b, _)| order < *b) {
                        best = Some((
                            order,
                            if t.flags & COLOR == 0 {
                                [1.; 4]
                            } else {
                                t.corners[c].color
                            },
                        ));
                    }
                }
            }
        }
        best.map(|(_, c)| c)
            .ok_or_else(|| invalid("proxy welded colour key has no source corner"))
    }
    pub(super) fn material(&self, index: u32) -> Result<serde_json::Value, JobError> {
        if index == u32::MAX {
            return Ok(serde_json::json!({}));
        }
        let m = self
            .captured
            .materials
            .get(index as usize)
            .ok_or_else(|| invalid("proxy material index"))?;
        m.remap(
            &m.bindings()
                .map(|b| (b.texture, b.texture))
                .collect::<BTreeMap<_, _>>(),
        )
    }
    pub(super) fn record(&self, rank: u64, bary: V3) -> Result<Witness, JobError> {
        if rank >= self.captured.triangles.len()
            || bary.iter().any(|v| !v.is_finite() || *v < 0. || *v > 1.)
            || (bary.iter().sum::<f64>() - 1.).abs() > 1e-8
        {
            return Err(invalid("terminal source witness"));
        }
        let triangle = self
            .captured
            .triangles
            .range(rank, rank + 1, 4096)?
            .next()
            .transpose()?
            .ok_or_else(|| invalid("terminal source rank"))?;
        Ok(Witness {
            rank,
            triangle,
            bary,
            distance2: 0.,
            fill_fallback: false,
        })
    }
    pub(super) fn lod(&self, w: Witness, slot: TextureSlot, texel: f64) -> Result<f64, JobError> {
        if w.triangle.material == u32::MAX {
            return Ok(0.);
        }
        let Some(binding) = self.captured.materials[w.triangle.material as usize]
            .bindings()
            .find(|b| b.slot == slot)
        else {
            return Ok(0.);
        };
        let image = self.captured.textures[binding.texture]["source"]
            .as_u64()
            .ok_or_else(|| invalid("footprint image"))? as usize;
        let info = self
            .captured
            .image_info
            .get(image)
            .ok_or_else(|| invalid("footprint image metadata"))?;
        let p = w.triangle.triangle.positions.map(|p| p.map(f64::from));
        let uv = w.triangle.corners.map(|c| {
            [
                f64::from(c.texcoords[binding.texcoord][0]) * f64::from(info.width),
                f64::from(c.texcoords[binding.texcoord][1]) * f64::from(info.height),
            ]
        });
        let sigma = math::pixel_gradient(p, uv).unwrap_or(0.);
        let rho = texel * sigma;
        if !rho.is_finite() {
            return Err(resource("source footprint overflow"));
        }
        Ok(if rho > 0. { rho.log2() } else { 0. })
    }
    /// Conservative source UV-plane box of the actual finite isotropic filter.
    /// It may include points outside the face; the separately stored barycentric
    /// centre remains the actual source-surface witness.
    pub(super) fn footprint(
        &self,
        w: Witness,
        slot: TextureSlot,
        lod: f64,
    ) -> Result<(V3, V3), JobError> {
        let p = w.triangle.triangle.positions.map(|p| p.map(f64::from));
        let q = math::bary(p, w.bary);
        if w.triangle.material == u32::MAX {
            return Ok((q, q));
        }
        let Some(binding) = self.captured.materials[w.triangle.material as usize]
            .bindings()
            .find(|b| b.slot == slot)
        else {
            return Ok((q, q));
        };
        let texture = &self.captured.textures[binding.texture];
        let image = texture["source"]
            .as_u64()
            .ok_or_else(|| invalid("support image"))? as usize;
        let info = &self.captured.image_info[image];
        let s = crops::resolve(
            texture
                .get("sampler")
                .and_then(serde_json::Value::as_u64)
                .and_then(|n| self.captured.samplers.get(n as usize)),
        )?;
        let filter = if lod <= 0. {
            s.mag_filter
        } else {
            s.min_filter
        };
        let maxlevel = (u32::BITS - info.width.max(info.height).leading_zeros() - 1) as f64;
        let mip = if matches!(filter, 9984..=9987) {
            if matches!(filter, 9986 | 9987) {
                lod.max(0.).ceil()
            } else {
                lod.max(0.).round()
            }
        } else {
            0.
        }
        .min(maxlevel) as u32;
        let size = [(info.width >> mip).max(1), (info.height >> mip).max(1)];
        let radius = if matches!(filter, 9728 | 9984 | 9986) {
            0.5
        } else {
            1.5
        };
        let uv = w
            .triangle
            .corners
            .map(|c| c.texcoords[binding.texcoord].map(f64::from));
        let a = [uv[1][0] - uv[0][0], uv[1][1] - uv[0][1]];
        let b = [uv[2][0] - uv[0][0], uv[2][1] - uv[0][1]];
        let det = a[0] * b[1] - a[1] * b[0];
        if det == 0. {
            return Ok((q, q));
        }
        let e = math::sub(p[1], p[0]);
        let f = math::sub(p[2], p[0]);
        let du = math::mul(
            math::sub(math::mul(e, b[1]), math::mul(f, a[1])),
            radius / (det * f64::from(size[0])),
        );
        let dv = math::mul(
            math::sub(math::mul(f, a[0]), math::mul(e, b[0])),
            radius / (det * f64::from(size[1])),
        );
        let r: V3 = std::array::from_fn(|a| du[a].abs() + dv[a].abs());
        if r.iter().any(|v| !v.is_finite()) {
            return Err(resource("source support extent overflow"));
        }
        Ok((
            std::array::from_fn(|a| (q[a] - r[a]).next_down()),
            std::array::from_fn(|a| (q[a] + r[a]).next_up()),
        ))
    }
    /// Original normal-map input admission requires authored NORMAL+TANGENT.
    /// A missing flag is a broken captured handoff, never a new derivative mode.
    pub(super) fn frame(w: Witness) -> Result<Frame, JobError> {
        let t = w.triangle;
        if t.flags & (NORMAL | TANGENT) != (NORMAL | TANGENT) {
            return Err(invalid(
                "admitted source normal binding lacks authored NORMAL/TANGENT",
            ));
        }
        Frame::donor(
            t.corners.map(|c| c.normal.map(f64::from)),
            t.corners.map(|c| c.tangent.map(f64::from)),
            w.bary,
        )
    }
    pub(super) fn signal(
        &mut self,
        w: Witness,
        slot: TextureSlot,
        lod: f64,
        check: &mut dyn FnMut() -> Result<(), JobError>,
    ) -> Result<Option<([f64; 4], usize)>, JobError> {
        if w.triangle.material == u32::MAX {
            return Ok(None);
        }
        let m = self
            .captured
            .materials
            .get(w.triangle.material as usize)
            .ok_or_else(|| invalid("source material"))?;
        let Some(binding) = m.bindings().find(|b| b.slot == slot).copied() else {
            return Ok(None);
        };
        let tex = self
            .captured
            .textures
            .get(binding.texture)
            .ok_or_else(|| invalid("source texture"))?;
        let image = tex
            .get("source")
            .and_then(serde_json::Value::as_u64)
            .and_then(|n| usize::try_from(n).ok())
            .ok_or_else(|| invalid("source texture image"))?;
        let sampler = crops::resolve(
            tex.get("sampler")
                .and_then(serde_json::Value::as_u64)
                .and_then(|n| self.captured.samplers.get(n as usize)),
        )?;
        let class = match slot {
            TextureSlot::BaseColor | TextureSlot::Emissive => mips::Class::Colour,
            // Original normal images are raw LINEAR texture components: the
            // viewer filters unequal-length vectors BEFORE normalScale/TBN and
            // normalization. Normalizing source base pixels does not commute.
            // Source minification uses raw linear component reduction too;
            // proxy-authored normals separately own common-frame unit mips.
            TextureSlot::Normal => mips::Class::Data,
            _ => mips::Class::Data,
        };
        if self
            .cache
            .as_ref()
            .is_none_or(|c| c.image != image || c.class != class)
        {
            self.cache = None;
            let snapshot = self
                .captured
                .images
                .get(image)
                .ok_or_else(|| invalid("source image snapshot"))?;
            let info = self
                .captured
                .image_info
                .get(image)
                .ok_or_else(|| invalid("source image metadata"))?;
            let pixels = u64::from(info.width)
                .checked_mul(u64::from(info.height))
                .ok_or_else(|| resource("source decode pixels"))?;
            let peak = pixels
                .checked_mul(24)
                .and_then(|n| n.checked_add(snapshot.identity.encoded_bytes))
                .and_then(|n| n.checked_add(64 << 20))
                .ok_or_else(|| resource("source decode charge"))?;
            if peak > self.cache_limit {
                return Err(resource("source image decode/cache budget"));
            }
            let encoded = snapshot.reconstruct(check)?;
            let (_, rgba) = source::decode_image_admitted(
                &encoded,
                snapshot.identity.mime.as_deref(),
                source::DecodeAdmission::LARGE_MESH,
                &mut || check(),
            )?;
            drop(encoded);
            let chain = mips::build(
                info.width,
                info.height,
                rgba,
                class,
                None,
                usize::try_from(self.cache_limit).map_err(|_| resource("source cache budget"))?,
            )?;
            self.cache = Some(Cache {
                image,
                class,
                chain,
                sampler,
            });
        }
        let c = self.cache.as_mut().unwrap();
        c.sampler = sampler;
        let uv: [f64; 2] = std::array::from_fn(|a| {
            (0..3)
                .map(|i| {
                    f64::from(w.triangle.corners[i].texcoords[binding.texcoord][a]) * w.bary[i]
                })
                .sum()
        });
        Ok(Some((
            sample(&c.chain.levels, c.class, sampler, uv, lod)?,
            binding.texcoord,
        )))
    }
    pub(super) fn work(&self) -> u64 {
        self.work
    }
    pub(super) fn cache_limit(&self) -> u64 {
        self.cache_limit
    }
    pub(super) fn density_floor(
        &mut self,
        check: &mut dyn FnMut() -> Result<(), JobError>,
    ) -> Result<f64, JobError> {
        let captured = self.captured;
        let mut reader = captured.triangles.range(self.first, self.end, 4096)?;
        let mut minimum = f64::INFINITY;
        let edge = self
            .captured
            .image_info
            .iter()
            .map(|i| i.width.max(i.height))
            .max()
            .unwrap_or(1)
            .max(1);
        while let Some(t) = reader.next().transpose()? {
            check()?;
            self.charge()?;
            let p = t.triangle.positions.map(|p| p.map(f64::from));
            // This heuristic is only for a zero prescribed raster density;
            // retained no-raster geometry supplies no sampling density.
            if math::face_normal(p).is_none() {
                continue;
            }
            for e in 0..3 {
                let d = math::distance2(p[e], p[(e + 1) % 3]).sqrt();
                if d > 0. {
                    minimum = minimum.min(d / f64::from(edge));
                }
            }
        }
        // Point-only geometry has no raster area. Its finite fallback does not
        // manufacture texture coverage or geometric error.
        Ok(if minimum.is_finite() && minimum > 0. {
            minimum
        } else {
            f64::from(f32::MIN_POSITIVE)
        })
    }
}
fn wrap(i: i64, n: u32, mode: u32) -> usize {
    let n = i64::from(n);
    match mode {
        33071 => i.clamp(0, n - 1) as usize,
        33648 => {
            let k = i.rem_euclid(2 * n);
            if k < n {
                k as usize
            } else {
                (2 * n - 1 - k) as usize
            }
        }
        _ => i.rem_euclid(n) as usize,
    }
}
fn linear(v: f64) -> f64 {
    if v <= 0.04045 {
        v / 12.92
    } else {
        ((v + 0.055) / 1.055).powf(2.4)
    }
}
fn srgb(v: f64) -> f64 {
    if v == 0. || v == 1. {
        return v;
    }
    if v <= 0.0031308 {
        v * 12.92
    } else {
        1.055 * v.powf(1. / 2.4) - 0.055
    }
}
/// The admitted taps and fraction are finite and bounded. Nested interpolation
/// keeps constants exact; endpoint bounds contain only floating point rounding
/// of the mathematical convex combination, never an external normal input.
fn convex(a: f64, b: f64, t: f64) -> f64 {
    if t == 0. {
        a
    } else if t == 1. {
        b
    } else {
        (a + (b - a) * t).clamp(a.min(b), a.max(b))
    }
}
pub(super) fn sample(
    levels: &[mips::Level],
    class: mips::Class,
    s: crops::SourceSampler,
    uv: [f64; 2],
    lod: f64,
) -> Result<[f64; 4], JobError> {
    if levels.is_empty() || uv.iter().any(|v| !v.is_finite()) || !lod.is_finite() {
        return Err(invalid("source sampling coordinates"));
    }
    let filter = if lod <= 0. {
        s.mag_filter
    } else {
        s.min_filter
    };
    let mip = matches!(filter, 9984..=9987);
    let blend = matches!(filter, 9986 | 9987);
    let l = if mip {
        lod.max(0.).min((levels.len() - 1) as f64)
    } else {
        0.
    };
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
    let fetch = |level: &mips::Level| -> Result<[f64; 4], JobError> {
        // Reduce UV before integer conversion, including huge finite repeats.
        let uv = std::array::from_fn::<_, 2, _>(|i| match [s.wrap_s, s.wrap_t][i] {
            33071 => uv[i].clamp(0., 1.),
            33648 => {
                let q = uv[i].rem_euclid(2.);
                if q <= 1. {
                    q
                } else {
                    2. - q
                }
            }
            _ => uv[i].rem_euclid(1.),
        });
        let xy = [
            uv[0] * f64::from(level.width) - 0.5,
            uv[1] * f64::from(level.height) - 0.5,
        ];
        let get = |x: i64, y: i64| {
            let i = (wrap(y, level.height, s.wrap_t) * level.width as usize
                + wrap(x, level.width, s.wrap_s))
                * 4;
            std::array::from_fn::<_, 4, _>(|a| {
                let v = f64::from(level.rgba[i + a]) / 255.;
                if class == mips::Class::Colour && a < 3 {
                    linear(v)
                } else {
                    v
                }
            })
        };
        if matches!(filter, 9728 | 9984 | 9986) {
            return Ok(get(
                (xy[0] + 0.5).floor() as i64,
                (xy[1] + 0.5).floor() as i64,
            ));
        }
        let x = xy[0].floor() as i64;
        let y = xy[1].floor() as i64;
        let f = [xy[0] - x as f64, xy[1] - y as f64];
        let p = [get(x, y), get(x + 1, y), get(x, y + 1), get(x + 1, y + 1)];
        Ok(std::array::from_fn(|a| {
            convex(
                convex(p[0][a], p[1][a], f[0]),
                convex(p[2][a], p[3][a], f[0]),
                f[1],
            )
        }))
    };
    let p = fetch(&levels[a])?;
    let q = fetch(&levels[b])?;
    Ok(std::array::from_fn(|i| {
        let v = convex(p[i], q[i], f);
        if class == mips::Class::Colour && i < 3 {
            srgb(v)
        } else {
            v
        }
    }))
}
