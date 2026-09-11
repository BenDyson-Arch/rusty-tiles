//! Dense uniform grid over a triangle soup for nearest-triangle queries.
//!
//! Replaces `HashMap<(i32,i32,i32), Vec<u32>>` bins: cells are a flat CSR
//! array, so a 27-cell neighbourhood is a few adds, not 27 SipHash lookups.

#[derive(Clone, Copy, Debug)]
pub struct Hit {
    pub tri: u32,
    pub d2: f32,
    pub bary: [f32; 3],
}

pub struct TriGrid {
    tris: Vec<[[f32; 3]; 3]>,
    /// Bounding sphere per triangle (centre, radius): a 3-flop reject
    /// before the full closest-point test.
    spheres: Vec<[f32; 4]>,
    min: [f32; 3],
    cell: f32,
    dims: [usize; 3],
    offsets: Vec<u32>,
    items: Vec<u32>,
}

/// Soft cap on cell count: 4 B per cell, so ~32 MB per grid (one per
/// parent being built, i.e. one per thread).
const MAX_CELLS: f32 = 8.0e6;

impl TriGrid {
    pub fn new(tris: Vec<[[f32; 3]; 3]>) -> Self {
        let mut min = [f32::INFINITY; 3];
        let mut max = [f32::NEG_INFINITY; 3];
        for t in &tris {
            for p in t {
                for i in 0..3 {
                    min[i] = min[i].min(p[i]);
                    max[i] = max[i].max(p[i]);
                }
            }
        }
        if tris.is_empty() {
            min = [0.0; 3];
            max = [1.0; 3];
        }
        let ext = [
            (max[0] - min[0]).max(1e-6),
            (max[1] - min[1]).max(1e-6),
            (max[2] - min[2]).max(1e-6),
        ];
        let longest = ext[0].max(ext[1]).max(ext[2]);
        // Triangles lie on surfaces, not in volumes: a cell of edge c holds
        // ~c²/mean_area of them. Aim for ~2 per occupied cell, bounded by
        // MAX_CELLS (a hollow cave wall would otherwise put 20+ per cell).
        let area: f32 = tris
            .iter()
            .map(|t| face_normal_area(t[0], t[1], t[2]).1)
            .sum();
        let mean_area = area / tris.len().max(1) as f32;
        let by_area = (2.0 * mean_area).sqrt();
        let by_vol = (ext[0] * ext[1] * ext[2] / MAX_CELLS).cbrt();
        let cell = by_area.max(by_vol).max(longest / 2048.0).max(1e-6);
        let dims = [
            ((ext[0] / cell).floor() as usize + 1).max(1),
            ((ext[1] / cell).floor() as usize + 1).max(1),
            ((ext[2] / cell).floor() as usize + 1).max(1),
        ];
        let ncell = dims[0] * dims[1] * dims[2];
        let mut counts = vec![0u32; ncell + 1];
        let mut spans = Vec::with_capacity(tris.len());
        for t in &tris {
            let (lo, hi) = Self::span_static(t, min, cell, dims);
            for z in lo[2]..=hi[2] {
                for y in lo[1]..=hi[1] {
                    for x in lo[0]..=hi[0] {
                        counts[Self::idx_static(dims, x, y, z) + 1] += 1;
                    }
                }
            }
            spans.push((lo, hi));
        }
        for i in 1..=ncell {
            counts[i] += counts[i - 1];
        }
        let mut fill = counts.clone();
        let mut items = vec![0u32; counts[ncell] as usize];
        for (ti, (lo, hi)) in spans.iter().enumerate() {
            for z in lo[2]..=hi[2] {
                for y in lo[1]..=hi[1] {
                    for x in lo[0]..=hi[0] {
                        let c = Self::idx_static(dims, x, y, z);
                        items[fill[c] as usize] = ti as u32;
                        fill[c] += 1;
                    }
                }
            }
        }
        let spheres = tris
            .iter()
            .map(|t| {
                let c = [
                    (t[0][0] + t[1][0] + t[2][0]) / 3.0,
                    (t[0][1] + t[1][1] + t[2][1]) / 3.0,
                    (t[0][2] + t[1][2] + t[2][2]) / 3.0,
                ];
                let r2 = t.iter().map(|p| dist2(*p, c)).fold(0.0f32, f32::max);
                [c[0], c[1], c[2], r2.sqrt()]
            })
            .collect();
        Self {
            tris,
            spheres,
            min,
            cell,
            dims,
            offsets: counts,
            items,
        }
    }

    pub fn tri(&self, i: u32) -> &[[f32; 3]; 3] {
        &self.tris[i as usize]
    }

    pub fn len(&self) -> usize {
        self.tris.len()
    }

    pub fn is_empty(&self) -> bool {
        self.tris.is_empty()
    }

    pub fn cell_size(&self) -> f32 {
        self.cell
    }

    fn idx_static(dims: [usize; 3], x: usize, y: usize, z: usize) -> usize {
        (z * dims[1] + y) * dims[0] + x
    }

    fn span_static(
        t: &[[f32; 3]; 3],
        min: [f32; 3],
        cell: f32,
        dims: [usize; 3],
    ) -> ([usize; 3], [usize; 3]) {
        let mut lo = [usize::MAX; 3];
        let mut hi = [0usize; 3];
        for p in t {
            for i in 0..3 {
                let c = (((p[i] - min[i]) / cell).floor().max(0.0) as usize).min(dims[i] - 1);
                lo[i] = lo[i].min(c);
                hi[i] = hi[i].max(c);
            }
        }
        (lo, hi)
    }

    fn cell_of(&self, p: [f32; 3]) -> [i64; 3] {
        let mut c = [0i64; 3];
        for i in 0..3 {
            c[i] = ((p[i] - self.min[i]) / self.cell).floor() as i64;
        }
        c
    }

    /// Visit candidate triangles in the cube of radius `r` cells around `c`,
    /// skipping the inner cube of radius `r - 1` (already visited).
    fn visit_ring(&self, c: [i64; 3], r: i64, f: &mut impl FnMut(u32)) {
        for dz in -r..=r {
            let z = c[2] + dz;
            if z < 0 || z >= self.dims[2] as i64 {
                continue;
            }
            for dy in -r..=r {
                let y = c[1] + dy;
                if y < 0 || y >= self.dims[1] as i64 {
                    continue;
                }
                let edge_zy = dz.abs() == r || dy.abs() == r;
                for dx in -r..=r {
                    if !edge_zy && dx.abs() != r {
                        continue;
                    }
                    let x = c[0] + dx;
                    if x < 0 || x >= self.dims[0] as i64 {
                        continue;
                    }
                    let ci = Self::idx_static(self.dims, x as usize, y as usize, z as usize);
                    let (a, b) = (self.offsets[ci] as usize, self.offsets[ci + 1] as usize);
                    for &ti in &self.items[a..b] {
                        f(ti);
                    }
                }
            }
        }
    }

    /// Nearest triangle by `score` (lower is better; `None` rejects; a score
    /// must never be below the squared distance, which lets the search stop
    /// as soon as the best score is within the distance to the searched
    /// cube's boundary), or until the caller's ring limit is reached.
    pub fn nearest_by(
        &self,
        p: [f32; 3],
        max_ring: i64,
        score: impl FnMut(u32, f32, [f32; 3]) -> Option<f32>,
    ) -> Option<Hit> {
        self.nearest_by_seeded(p, max_ring, None, score)
    }

    /// `nearest_by` with an optional starting candidate (e.g. the previous
    /// texel's hit): an early upper bound lets the sphere reject and the
    /// ring cut-off fire from the first cell.
    pub fn nearest_by_seeded(
        &self,
        p: [f32; 3],
        max_ring: i64,
        seed: Option<u32>,
        mut score: impl FnMut(u32, f32, [f32; 3]) -> Option<f32>,
    ) -> Option<Hit> {
        let c = self.cell_of(p);
        let mut best: Option<(f32, Hit)> = None;
        if let Some(ti) = seed.filter(|&ti| (ti as usize) < self.tris.len()) {
            let t = &self.tris[ti as usize];
            let (d2, bary) = point_tri_dist2_bary(p, t[0], t[1], t[2]);
            if let Some(s) = score(ti, d2, bary) {
                best = Some((s, Hit { tri: ti, d2, bary }));
            }
        }
        // Ring radius that reaches every cell from `c`, even if `p` is outside.
        let far = (0..3)
            .map(|i| c[i].abs().max((self.dims[i] as i64 - 1 - c[i]).abs()))
            .max()
            .unwrap_or(1);
        let limit = max_ring.min(far);
        for r in 0..=limit {
            self.visit_ring(c, r, &mut |ti| {
                if let Some((bs, _)) = best {
                    // Score ≥ d² ≥ (|p − centre| − radius)²: cannot win.
                    let s = &self.spheres[ti as usize];
                    let lower = dist2(p, [s[0], s[1], s[2]]).sqrt() - s[3];
                    if lower > 0.0 && lower * lower >= bs {
                        return;
                    }
                }
                let t = &self.tris[ti as usize];
                let (d2, bary) = point_tri_dist2_bary(p, t[0], t[1], t[2]);
                if let Some(s) = score(ti, d2, bary) {
                    if best.map(|(bs, _)| s < bs).unwrap_or(true) {
                        best = Some((s, Hit { tri: ti, d2, bary }));
                    }
                }
            });
            if let Some((bs, _)) = best {
                // Every unvisited triangle is at least this far away.
                let wall = self.cube_boundary_dist(p, c, r);
                if bs <= wall * wall {
                    break;
                }
            }
        }
        best.map(|(_, h)| h)
    }

    /// Distance from `p` to the boundary of the (2r+1)³-cell cube around `c`.
    fn cube_boundary_dist(&self, p: [f32; 3], c: [i64; 3], r: i64) -> f32 {
        let mut d = f32::INFINITY;
        for i in 0..3 {
            let lo = self.min[i] + (c[i] - r) as f32 * self.cell;
            let hi = self.min[i] + (c[i] + r + 1) as f32 * self.cell;
            d = d.min(p[i] - lo).min(hi - p[i]);
        }
        d.max(0.0)
    }

    /// Squared distance to the nearest triangle (searching outward).
    pub fn nearest_dist2(&self, p: [f32; 3]) -> Option<f32> {
        self.nearest_by(p, i64::MAX, |_, d2, _| Some(d2))
            .map(|h| h.d2)
    }
}

/// Closest point on a triangle: squared distance + barycentrics (Ericson).
pub fn point_tri_dist2_bary(p: [f32; 3], a: [f32; 3], b: [f32; 3], c: [f32; 3]) -> (f32, [f32; 3]) {
    let ab = sub(b, a);
    let ac = sub(c, a);
    let ap = sub(p, a);
    let d1 = dot(ab, ap);
    let d2 = dot(ac, ap);
    if d1 <= 0.0 && d2 <= 0.0 {
        return (dot(ap, ap), [1.0, 0.0, 0.0]);
    }
    let bp = sub(p, b);
    let d3 = dot(ab, bp);
    let d4 = dot(ac, bp);
    if d3 >= 0.0 && d4 <= d3 {
        return (dot(bp, bp), [0.0, 1.0, 0.0]);
    }
    let vc = d1 * d4 - d3 * d2;
    if vc <= 0.0 && d1 >= 0.0 && d3 <= 0.0 {
        let v = d1 / (d1 - d3);
        let q = add(a, scl(ab, v));
        return (dist2(p, q), [1.0 - v, v, 0.0]);
    }
    let cp = sub(p, c);
    let d5 = dot(ab, cp);
    let d6 = dot(ac, cp);
    if d6 >= 0.0 && d5 <= d6 {
        return (dot(cp, cp), [0.0, 0.0, 1.0]);
    }
    let vb = d5 * d2 - d1 * d6;
    if vb <= 0.0 && d2 >= 0.0 && d6 <= 0.0 {
        let w = d2 / (d2 - d6);
        let q = add(a, scl(ac, w));
        return (dist2(p, q), [1.0 - w, 0.0, w]);
    }
    let va = d3 * d6 - d5 * d4;
    if va <= 0.0 && (d4 - d3) >= 0.0 && (d5 - d6) >= 0.0 {
        let w = (d4 - d3) / ((d4 - d3) + (d5 - d6));
        let q = add(b, scl(sub(c, b), w));
        return (dist2(p, q), [0.0, 1.0 - w, w]);
    }
    let denom = va + vb + vc;
    if denom.abs() < 1e-20 {
        return (dot(ap, ap), [1.0, 0.0, 0.0]);
    }
    let v = vb / denom;
    let w = vc / denom;
    let q = add(a, add(scl(ab, v), scl(ac, w)));
    (dist2(p, q), [1.0 - v - w, v, w])
}

pub fn sub(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

pub fn add(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}

pub fn scl(a: [f32; 3], s: f32) -> [f32; 3] {
    [a[0] * s, a[1] * s, a[2] * s]
}

pub fn dot(a: [f32; 3], b: [f32; 3]) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

pub fn dist2(a: [f32; 3], b: [f32; 3]) -> f32 {
    let d = sub(a, b);
    dot(d, d)
}

pub fn cross(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}

pub fn normalize(n: [f32; 3]) -> [f32; 3] {
    let len = dot(n, n).sqrt();
    if len < 1e-20 {
        [0.0, 1.0, 0.0]
    } else {
        [n[0] / len, n[1] / len, n[2] / len]
    }
}

/// Unit normal and area of a triangle.
pub fn face_normal_area(a: [f32; 3], b: [f32; 3], c: [f32; 3]) -> ([f32; 3], f32) {
    let n = cross(sub(b, a), sub(c, a));
    let len = dot(n, n).sqrt();
    if len < 1e-20 {
        ([0.0, 1.0, 0.0], 0.0)
    } else {
        ([n[0] / len, n[1] / len, n[2] / len], len * 0.5)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nearest_finds_far_triangle_across_empty_cells() {
        let tris = vec![
            [[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]],
            [[10.0, 0.0, 0.0], [11.0, 0.0, 0.0], [10.0, 1.0, 0.0]],
        ];
        let g = TriGrid::new(tris);
        let d2 = g.nearest_dist2([5.0, 0.2, 0.0]).unwrap();
        let want = (16.0f32 + 0.04).sqrt();
        assert!((d2.sqrt() - want).abs() < 1e-4, "got {}", d2.sqrt());
        let h = g
            .nearest_by([10.5, 0.2, 3.0], i64::MAX, |_, d2, _| Some(d2))
            .unwrap();
        assert_eq!(h.tri, 1);
        assert!((h.d2 - 9.0).abs() < 1e-4);
    }

    #[test]
    fn empty_grid_returns_none() {
        let g = TriGrid::new(Vec::new());
        assert!(g.nearest_dist2([0.0, 0.0, 0.0]).is_none());
    }
}
