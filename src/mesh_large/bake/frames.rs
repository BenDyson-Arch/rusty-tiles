//! The pinned authored Cesium TBN path preserves source tangent/normal skew.
use super::{
    charts::Layout,
    math::{self, V3},
    resource,
    types::{FrozenNode, SourceKey},
    unsupported,
};
use crate::JobError;

#[derive(Clone, Copy, Debug)]
pub(crate) struct Frame {
    pub tangent: V3,
    pub bitangent: V3,
    pub normal: V3,
}
impl Frame {
    pub(crate) fn from_triangle(
        p: [V3; 3],
        uv: [[f64; 2]; 3],
        normal: V3,
    ) -> Result<Self, JobError> {
        let n = math::unit(normal).ok_or_else(|| unsupported("proxy normal has no direction"))?;
        let e1 = math::sub(p[1], p[0]);
        let e2 = math::sub(p[2], p[0]);
        let a = [uv[1][0] - uv[0][0], uv[1][1] - uv[0][1]];
        let b = [uv[2][0] - uv[0][0], uv[2][1] - uv[0][1]];
        let det = a[0] * b[1] - a[1] * b[0];
        if det == 0.0 || !det.is_finite() {
            return Err(unsupported(
                "nondegenerate proxy has singular decoded UV frame",
            ));
        }
        // Common division magnitude is irrelevant after direction normalization.
        let t = math::mul(
            math::sub(math::mul(e1, b[1]), math::mul(e2, a[1])),
            det.signum(),
        );
        let raw_b = math::mul(
            math::sub(math::mul(e2, a[0]), math::mul(e1, b[0])),
            det.signum(),
        );
        let t = math::unit(math::sub(t, math::mul(n, math::dot(n, t))))
            .ok_or_else(|| unsupported("proxy tangent projection has no direction"))?;
        let cross = math::cross(n, t);
        let w = if math::dot(cross, raw_b) < 0.0 {
            -1.0
        } else {
            1.0
        };
        Ok(Self {
            tangent: t,
            bitangent: math::mul(cross, w),
            normal: n,
        })
    }
    /// GeometryStageVS and GeometryStageFS normalize each axis separately.
    /// Neither orthogonalizes authored T/N; MaterialStageFS then multiplies TBN.
    pub(crate) fn donor(
        normals: [V3; 3],
        tangents: [[f64; 4]; 3],
        w: V3,
    ) -> Result<Self, JobError> {
        let mut ns = [[0.0; 3]; 3];
        let mut ts = ns;
        let mut bs = ns;
        for i in 0..3 {
            ns[i] = math::unit(normals[i])
                .ok_or_else(|| unsupported("source normal has no direction"))?;
            ts[i] = math::unit([tangents[i][0], tangents[i][1], tangents[i][2]])
                .ok_or_else(|| unsupported("source tangent has no direction"))?;
            bs[i] = math::unit(math::mul(math::cross(ns[i], ts[i]), tangents[i][3]))
                .ok_or_else(|| unsupported("source authored TBN is singular"))?;
        }
        Self::interpolate(ns, ts, bs, w)
    }
    pub(crate) fn interpolate(
        ns: [V3; 3],
        ts: [V3; 3],
        bs: [V3; 3],
        w: V3,
    ) -> Result<Self, JobError> {
        Ok(Self {
            normal: math::unit(math::bary(ns, w))
                .ok_or_else(|| unsupported("interpolated normal cancelled"))?,
            tangent: math::unit(math::bary(ts, w))
                .ok_or_else(|| unsupported("interpolated tangent cancelled"))?,
            bitangent: math::unit(math::bary(bs, w))
                .ok_or_else(|| unsupported("interpolated bitangent cancelled"))?,
        })
    }
    pub(crate) fn transform(self, v: V3) -> Result<V3, JobError> {
        math::unit(math::add(
            math::add(
                math::mul(self.tangent, v[0]),
                math::mul(self.bitangent, v[1]),
            ),
            math::mul(self.normal, v[2]),
        ))
        .ok_or_else(|| unsupported("effective source shading normal cancelled"))
    }
    /// Full cofactor inverse, preserving skew and determinant sign.
    /// Normalize before division to avoid overflow for nearly singular frames.
    pub(crate) fn inverse_direction(self, v: V3) -> Result<V3, JobError> {
        let bc = math::cross(self.bitangent, self.normal);
        let det = math::dot(self.tangent, bc);
        if det == 0.0 || !det.is_finite() {
            return Err(unsupported("decoded proxy TBN is singular"));
        }
        let q = [
            math::dot(bc, v),
            math::dot(math::cross(self.normal, self.tangent), v),
            math::dot(math::cross(self.tangent, self.bitangent), v),
        ];
        math::unit(math::mul(q, det.signum()))
            .ok_or_else(|| unsupported("normal frame inversion has no direction"))
    }
}
pub(crate) fn reexpress(rgb: V3, scale: f64, donor: Frame, proxy: Frame) -> Result<V3, JobError> {
    if !scale.is_finite()
        || rgb
            .iter()
            .any(|v| !v.is_finite() || !(0.0..=1.0).contains(v))
    {
        return Err(unsupported("normal sample or literal scale is not finite"));
    }
    let v = rgb.map(|v| 2.0 * v - 1.0);
    proxy.inverse_direction(donor.transform(effective_direction(v, scale)?)?)
}
pub(super) fn effective_direction(v: V3, scale: f64) -> Result<V3, JobError> {
    if !scale.is_finite() || v.iter().any(|v| !v.is_finite()) {
        return Err(unsupported("normal signal/literal scale is not finite"));
    }
    let k = scale.abs();
    let effective = if k > 1.0 {
        [v[0] * scale.signum(), v[1] * scale.signum(), v[2] / k]
    } else {
        [v[0] * scale, v[1] * scale, v[2]]
    };
    math::unit(effective).ok_or_else(|| unsupported("source effective normal is zero"))
}

/// Generated proxy attributes are corner-owned. Normal fans follow original
/// weld edges and primitive/material crease boundaries; tangent fans additionally
/// split at decoded chart and handedness seams. Geometry/provenance is unchanged.
pub(super) struct Generated {
    pub frames: Vec<[Frame; 3]>,
    pub normals: Vec<[[f32; 3]; 3]>,
    pub tangents: Vec<[[f32; 4]; 3]>,
}
fn root(parents: &mut [usize], mut i: usize) -> usize {
    while parents[i] != i {
        parents[i] = parents[parents[i]];
        i = parents[i];
    }
    i
}
#[derive(PartialEq, Eq, PartialOrd, Ord)]
struct Edge {
    source: SourceKey,
    material: u32,
    a: [u32; 3],
    b: [u32; 3],
    face: usize,
    corners: [usize; 2],
}
pub(super) fn generate(
    f: &FrozenNode,
    layout: &Layout,
    memory: u64,
    work_limit: u64,
    check: &mut dyn FnMut() -> Result<(), JobError>,
) -> Result<Generated, JobError> {
    // Includes sorted edge keys, union-find, fan maps and retained attributes;
    // reject before any node-size allocation, including the nonmanifold case.
    if layout.raster_faces.len() != f.faces.len() {
        return Err(super::invalid("resolved frame participation shape"));
    }
    let charge = (f.faces.len() as u64)
        .checked_mul(2048)
        .ok_or_else(|| resource("corner fan lease overflow"))?;
    if charge > memory {
        return Err(resource("corner fan lease"));
    }
    let mut work = 0u64;
    let mut tick = || -> Result<(), JobError> {
        check()?;
        work = work
            .checked_add(1)
            .ok_or_else(|| resource("corner fan work overflow"))?;
        if work > work_limit {
            return Err(resource("corner fan work"));
        }
        Ok(())
    };
    let mut chart = vec![usize::MAX; f.faces.len()];
    for (ci, c) in layout.charts.iter().enumerate() {
        for &fi in &c.faces {
            chart[fi] = ci;
        }
    }
    let mut parents: Vec<_> = (0..f.faces.len() * 3).collect();
    let mut areas = Vec::with_capacity(f.faces.len());
    let mut normals = Vec::with_capacity(f.faces.len());
    let mut raw_t = Vec::with_capacity(f.faces.len());
    let mut raw_b = Vec::with_capacity(f.faces.len());
    let mut edges = Vec::with_capacity(f.faces.len() * 3);
    for (fi, face) in f.faces.iter().enumerate() {
        tick()?;
        let p = face.positions.map(|p| p.map(f64::from));
        let e1 = math::sub(p[1], p[0]);
        let e2 = math::sub(p[2], p[0]);
        let cross = math::cross(e1, e2);
        let n = math::unit(cross);
        let area = math::dot(cross, cross).sqrt() * 0.5;
        areas.push(area);
        normals.push(n);
        if n.is_none() {
            raw_t.push([1., 0., 0.]);
            raw_b.push([0., 1., 0.]);
            continue;
        }
        if layout.raster_faces[fi] {
            if chart[fi] == usize::MAX {
                return Err(super::invalid("raster face missing decoded UV chart"));
            }
            let uv = layout.uv[fi].map(|p| p.map(f64::from));
            let a = [uv[1][0] - uv[0][0], uv[1][1] - uv[0][1]];
            let b = [uv[2][0] - uv[0][0], uv[2][1] - uv[0][1]];
            let det = a[0] * b[1] - a[1] * b[0];
            if det == 0. || !det.is_finite() {
                return Err(unsupported("corner fan singular decoded UV"));
            }
            let t = math::mul(
                math::sub(math::mul(e1, b[1]), math::mul(e2, a[1])),
                1. / det,
            );
            let b = math::mul(
                math::sub(math::mul(e2, a[0]), math::mul(e1, b[0])),
                1. / det,
            );
            if t.iter().chain(b.iter()).any(|v| !v.is_finite()) {
                return Err(unsupported("corner fan tangent overflow"));
            }
            raw_t.push(t);
            raw_b.push(b);
        } else {
            // No normal-map/ID UV exists. Only the normal fan has rendering
            // meaning; final per-corner canonical tangents are unused by shading.
            raw_t.push([1., 0., 0.]);
            raw_b.push([0., 1., 0.]);
        }
        for i in 0..3 {
            let j = (i + 1) % 3;
            let (a, b, corners) = if face.welds[i] <= face.welds[j] {
                (face.welds[i], face.welds[j], [i, j])
            } else {
                (face.welds[j], face.welds[i], [j, i])
            };
            if a != b {
                edges.push(Edge {
                    source: face.source,
                    material: face.material,
                    a,
                    b,
                    face: fi,
                    corners,
                });
            }
        }
    }
    edges.sort_unstable();
    let mut start = 0;
    while start < edges.len() {
        let mut end = start + 1;
        while end < edges.len()
            && (
                edges[end].source,
                edges[end].material,
                edges[end].a,
                edges[end].b,
            ) == (
                edges[start].source,
                edges[start].material,
                edges[start].a,
                edges[start].b,
            )
        {
            end += 1;
        }
        for i in start..end {
            for j in i + 1..end {
                tick()?;
                let (a, b) = (&edges[i], &edges[j]);
                if math::dot(normals[a.face].unwrap(), normals[b.face].unwrap()) < 0.5 {
                    continue;
                }
                for k in 0..2 {
                    let x = root(&mut parents, a.face * 3 + a.corners[k]);
                    let y = root(&mut parents, b.face * 3 + b.corners[k]);
                    parents[x.max(y)] = x.min(y);
                }
            }
        }
        start = end;
    }
    let mut order: Vec<_> = (0..f.faces.len()).collect();
    order.sort_unstable_by_key(|&i| f.faces[i].ordinal);
    let mut sums = vec![[0.; 3]; parents.len()];
    for &fi in &order {
        if let Some(n) = normals[fi] {
            for ci in 0..3 {
                let r = root(&mut parents, fi * 3 + ci);
                sums[r] = math::add(sums[r], math::mul(n, areas[fi]));
            }
        }
    }
    let mut ns = vec![[[0.; 3]; 3]; f.faces.len()];
    let mut groups = vec![[(0, 0, false); 3]; f.faces.len()];
    let mut tangent_sums = std::collections::BTreeMap::new();
    for &fi in &order {
        let n = normals[fi];
        tick()?;
        let mut out = [[0.; 3]; 3];
        let mut keys = [(0, 0, false); 3];
        for (ci, value) in out.iter_mut().enumerate() {
            let r = root(&mut parents, fi * 3 + ci);
            let (n, r) = match math::unit(sums[r]) {
                Some(n) => (n, r),
                None => (n.unwrap_or([0., 0., 1.]), fi * 3 + ci),
            };
            *value = n.map(|v| v as f32);
            let published_n = math::unit(value.map(f64::from)).unwrap();
            let negative = math::dot(math::cross(published_n, raw_t[fi]), raw_b[fi]) < 0.;
            let key = (r, chart[fi], negative);
            keys[ci] = key;
            let sum = tangent_sums.entry(key).or_insert([0.; 3]);
            *sum = math::add(*sum, math::mul(raw_t[fi], areas[fi]));
        }
        ns[fi] = out;
        groups[fi] = keys;
    }
    let mut ts = Vec::with_capacity(f.faces.len());
    let mut frames = Vec::with_capacity(f.faces.len());
    for fi in 0..f.faces.len() {
        tick()?;
        let mut out = [[0.; 4]; 3];
        let mut fs = [Frame {
            normal: [0., 0., 1.],
            tangent: [1., 0., 0.],
            bitangent: [0., 1., 0.],
        }; 3];
        for (ci, (attribute, frame)) in out.iter_mut().zip(fs.iter_mut()).enumerate() {
            let n = math::unit(ns[fi][ci].map(f64::from)).unwrap();
            if !layout.raster_faces[fi] {
                let axis = (0..3)
                    .min_by(|a, b| n[*a].abs().total_cmp(&n[*b].abs()))
                    .unwrap();
                let mut canonical = [0.; 3];
                canonical[axis] = 1.;
                let t = math::unit(math::cross(n, canonical))
                    .unwrap()
                    .map(|v| v as f32);
                *attribute = [t[0], t[1], t[2], 1.];
                *frame = Frame::donor(
                    [ns[fi][ci].map(f64::from); 3],
                    [attribute.map(f64::from); 3],
                    [1., 0., 0.],
                )?;
                continue;
            }
            let project = |t| math::unit(math::sub(t, math::mul(n, math::dot(n, t))));
            let candidate = project(tangent_sums[&groups[fi][ci]])
                .or_else(|| project(raw_t[fi]))
                .ok_or_else(|| unsupported("corner fan tangent projection cancelled"))?;
            let stored = |t: V3| t.map(|v| v as f32);
            let handedness = |t: [f32; 3]| -> Result<f32, JobError> {
                let decoded = math::unit(t.map(f64::from))
                    .ok_or_else(|| unsupported("stored proxy tangent cancelled"))?;
                let orientation = math::dot(math::cross(n, decoded), raw_b[fi]);
                if orientation == 0. || !orientation.is_finite() {
                    return Err(unsupported("stored proxy tangent handedness ambiguous"));
                }
                Ok(if orientation < 0. { -1. } else { 1. })
            };
            let mut t = stored(candidate);
            let mut sign = handedness(t)?;
            let grouped_sign = if groups[fi][ci].2 { -1. } else { 1. };
            if sign != grouped_sign {
                // A summed tangent can cross a face's handedness boundary.
                // Split that render corner to its raw per-face UV tangent;
                // never reuse the sign selected before accumulation/rounding.
                t = stored(
                    project(raw_t[fi])
                        .ok_or_else(|| unsupported("split corner tangent cancelled"))?,
                );
                sign = handedness(t)?;
            }
            *attribute = [t[0], t[1], t[2], sign];
            *frame = Frame::donor(
                [ns[fi][ci].map(f64::from); 3],
                [attribute.map(f64::from); 3],
                [1., 0., 0.],
            )?;
        }
        ts.push(out);
        frames.push(fs);
    }
    Ok(Generated {
        frames,
        normals: ns,
        tangents: ts,
    })
}
