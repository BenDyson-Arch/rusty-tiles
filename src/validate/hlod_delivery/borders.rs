//! D11 scalable borders, from the archive alone.
//!
//! A coarse tile's border is the set of exact leaf edges and points it shares with
//! leaves outside it (the `requirements` rows). D11 lets an ancestor contract that
//! border: a delivered chord joins two retained source vertices and stands for the
//! source arc between them, parameterized by original leaf edge ordinal. Junctions
//! (border degree other than two) and point-only contacts stay exact. Every arc
//! between retained vertices must be delivered as exactly one chord with witnessed
//! directions; nothing is matched by distance and no chord is inferred.
//!
//! Displacement `d` of a contracted vertex is its distance from the chord at its
//! ordinal, computed in f64 and rounded outward. Each tile needs `d <= G/16` (the
//! archive analogue of `epsilon_d <= E_d/16 <= G/16`). Logical depth is not in the
//! archive (spliced chains skip levels), so pairwise depth rules use only what the
//! delivered tree proves: siblings share one logical depth and must agree exactly
//! (same chord and ordinal, or both exact); a tile whose cell parent is an ancestor
//! of a disjoint tile is no deeper than it, so a leaf there is a terminal lock (the
//! deeper tile is exact) and a coarse tile needs `d_X + d_Y <= G_X/8`. Same-depth
//! cousins and source-arc certificates need the source oracle and audit sidecar.
use super::super::Error;
use super::{
    bad, read_edge, read_point, sql, CanonicalEdge, CanonicalPoint, Effort, Store, Surface,
};
use rusqlite::params;
use std::collections::{BTreeMap, BTreeSet, HashMap};

/// Outward rounding of every f64 distance sum below (well above ten roundings).
const OUTWARD: f64 = 1. + 1. / (1u64 << 48) as f64;
/// Charged bytes per resident border edge, vertex image or adjacency entry.
const ITEM_BYTES: usize = 160;

fn key(a: CanonicalPoint, b: CanonicalPoint) -> CanonicalEdge {
    if a < b {
        (a, b)
    } else {
        (b, a)
    }
}
/// A direction mask (bit 0: lower key to higher) seen walking from `from` to `to`
/// (bit 0: along the walk).
fn along(mask: i64, from: CanonicalPoint, to: CanonicalPoint) -> i64 {
    if from < to {
        mask & 3
    } else {
        ((mask & 1) << 1) | ((mask >> 1) & 1)
    }
}
fn coordinate(bits: u32) -> f64 {
    f64::from(f32::from_bits(bits))
}
/// Upper bound of |v - chord(i/m)|, the chord from `a` (ordinal 0) to `b` (ordinal m).
/// The residual is formed from differences, (v - a) - (b - a) i/m, so its rounding
/// allowance scales with those differences: translation-invariant, and zero on an
/// axis where all three coordinates are equal.
fn displacement(a: CanonicalPoint, b: CanonicalPoint, v: CanonicalPoint, i: u64, m: u64) -> f64 {
    const U: f64 = f64::EPSILON / 2.;
    let t = i as f64 / m as f64;
    let mut sum = 0.;
    for k in 0..3 {
        let (a, b, v) = (coordinate(a[k]), coordinate(b[k]), coordinate(v[k]));
        let (dv, db) = (v - a, b - a);
        // Two differences, a product with the rounded ordinal and the subtraction:
        // at most 2u|dv| + 4u|db| to first order; 8u covers the second order.
        let error = 8. * U * (dv.abs() + db.abs());
        let d = (dv - db * t).abs() + error + if error > 0. { f64::MIN_POSITIVE } else { 0. };
        sum += d * d;
    }
    sum.sqrt() * OUTWARD
}

/// Exact sum of f64 terms as a nonoverlapping expansion (Shewchuk's grow-expansion
/// with zero elimination); its sign is the sign of its largest component.
fn exact_sign(terms: &[f64]) -> std::cmp::Ordering {
    let mut expansion: Vec<f64> = Vec::new();
    for &b in terms {
        let mut q = b;
        let mut next = Vec::with_capacity(expansion.len() + 1);
        for &e in &expansion {
            let s = q + e;
            let bv = s - q;
            let h = (q - (s - bv)) + (e - bv);
            if h != 0. {
                next.push(h);
            }
            q = s;
        }
        if q != 0. {
            next.push(q);
        }
        expansion = next;
    }
    expansion
        .last()
        .map_or(std::cmp::Ordering::Equal, |x| x.total_cmp(&0.))
}
/// Exact products of two term lists whose entries are f32 values or their exact
/// pairwise products (each product of two f32 is exact in f64; a further product
/// splits into its rounded value and exact fused remainder).
fn product(x: &[f64], y: &[f64], out: &mut Vec<f64>, sign: f64) {
    for &a in x {
        for &b in y {
            let p = a * b;
            out.push(sign * p);
            let e = a.mul_add(b, -p);
            if e != 0. {
                out.push(sign * e);
            }
        }
    }
}
/// Sign of |d|^2 (u.w) - (u.d)(w.d) for d = b - a, u = c - a, w = e - a: the
/// wings (perpendicular to the edge) towards c and e form an acute wedge (+), a
/// right angle (0) or an obtuse one (-). Exact over f32 coordinates; with c = e it
/// is |d x u|^2, zero only for a collinear face.
fn wedge(a: [f32; 3], b: [f32; 3], c: [f32; 3], e: [f32; 3]) -> std::cmp::Ordering {
    let difference = |p: [f32; 3], k: usize| [f64::from(p[k]), -f64::from(a[k])];
    let dot = |p: [f32; 3], q: [f32; 3]| {
        let mut terms = Vec::new();
        for k in 0..3 {
            product(&difference(p, k), &difference(q, k), &mut terms, 1.);
        }
        terms
    };
    let mut terms = Vec::new();
    product(&dot(b, b), &dot(c, e), &mut terms, 1.);
    product(&dot(c, b), &dot(e, b), &mut terms, -1.);
    exact_sign(&terms)
}
/// Whether the edge's same-direction faces of `direction` (0: lower to higher key)
/// lie on distinct sides: every pair forms a right or obtuse wedge, and no face is
/// collinear with the edge. A book's perpendicular pages and a collapsed plate's
/// opposite sheets pass; a duplicate, a fold or a leaning flap does not.
fn distinct_sides(edge: &CanonicalEdge, direction: usize, surface: &Surface) -> bool {
    let point = |p: CanonicalPoint| p.map(f32::from_bits);
    let (a, b) = (point(edge.0), point(edge.1));
    let mut thirds = Vec::new();
    for f in &surface.faces {
        for j in 0..3 {
            let (x, y) = (f[j], f[(j + 1) % 3]);
            let along = if direction == 0 { (x, y) } else { (y, x) };
            if along == *edge {
                thirds.push(point(f[(j + 2) % 3]));
            }
        }
    }
    thirds.iter().enumerate().all(|(i, c)| {
        wedge(a, b, *c, *c) == std::cmp::Ordering::Greater
            && thirds[i + 1..]
                .iter()
                .all(|e| wedge(a, b, *c, *e) != std::cmp::Ordering::Greater)
    })
}
/// Where one tile delivers a border vertex: retained exactly (empty), or on a chord
/// (`lower`, `upper`) at ordinal `i` of `m` counted from `lower`.
fn image(chord: CanonicalEdge, i: u64, m: u64) -> Vec<u8> {
    let mut bytes = super::edge_bytes(chord);
    bytes.extend(i.to_le_bytes());
    bytes.extend(m.to_le_bytes());
    bytes
}

/// One coarse tile's border: required leaf edges (allowed inside directions) and
/// points, with adjacency, resident under the decoded document budget.
struct Border {
    edges: BTreeMap<CanonicalEdge, i64>,
    points: BTreeSet<CanonicalPoint>,
    adjacent: BTreeMap<CanonicalPoint, Vec<CanonicalPoint>>,
}

impl Store {
    fn load_border(&self, tile: i64, effort: &Effort, used: usize) -> Result<Border, Error> {
        let mut border = Border {
            edges: BTreeMap::new(),
            points: BTreeSet::new(),
            adjacent: BTreeMap::new(),
        };
        let mut query = self
            .connection
            .prepare_cached("SELECT kind,key,directions FROM requirements WHERE tile=?")
            .map_err(sql)?;
        let mut rows = query.query([tile]).map_err(sql)?;
        let mut items = 0usize;
        while let Some(row) = rows.next().map_err(sql)? {
            effort.charge(1)?;
            items += 3;
            if items.saturating_mul(ITEM_BYTES).saturating_add(used) > self.decoded_limit {
                return Err(Error::limit(
                    "interfaceSegments: one tile's border exceeds the decoded document budget",
                ));
            }
            let kind: i64 = row.get(0).map_err(sql)?;
            let bytes: Vec<u8> = row.get(1).map_err(sql)?;
            if kind == 0 {
                border.points.insert(read_point(&bytes)?);
            } else {
                let e = read_edge(&bytes)?;
                border.edges.insert(e, row.get(2).map_err(sql)?);
                border.adjacent.entry(e.0).or_default().push(e.1);
                border.adjacent.entry(e.1).or_default().push(e.0);
            }
        }
        Ok(border)
    }

    /// D11 border check of one coarse tile's decoded surface. Records each border
    /// vertex's image for the pairwise depth rules and returns the contracted arcs
    /// by chord (for the equal-direction multiplicity check).
    pub(super) fn border(
        &self,
        tile: i64,
        surface: &Surface,
        effort: &Effort,
    ) -> Result<BTreeMap<CanonicalEdge, Vec<CanonicalPoint>>, Error> {
        let g: Option<f64> = self
            .connection
            .query_row("SELECT error FROM tiles WHERE id=?", [tile], |r| r.get(0))
            .map_err(sql)?;
        // G bounds contraction only; an exact border needs none.
        let g = g.filter(|g| g.is_finite() && *g >= 0.);
        let used = surface
            .points
            .len()
            .saturating_add(surface.edges.len())
            .saturating_mul(ITEM_BYTES);
        let border = self.load_border(tile, effort, used)?;
        let log = |n: usize| n.max(2).ilog2() as usize + 1;
        let retained = |p: &CanonicalPoint| surface.points.contains(p);
        // Junctions and point-only contacts are exact at every depth.
        for p in border.points.iter().chain(border.adjacent.keys()) {
            effort.charge(2 * log(surface.points.len()))?;
            let degree = border.adjacent.get(p).map_or(0, Vec::len);
            if degree != 2 && !retained(p) {
                return Err(bad(
                    "interfacePoints",
                    &format!("decoded ancestor lost an exact border junction or point contact: tile={tile}"),
                ));
            }
        }
        let mut visited = BTreeSet::<CanonicalEdge>::new();
        let mut chords = BTreeSet::<CanonicalEdge>::new();
        let mut contracted = BTreeMap::<CanonicalEdge, Vec<CanonicalPoint>>::new();
        let mut images = BTreeMap::<CanonicalPoint, (Vec<u8>, f64)>::new();
        for (&start, around) in &border.adjacent {
            if !retained(&start) {
                continue;
            }
            for &first in around {
                effort.charge(log(visited.len()))?;
                if visited.contains(&key(start, first)) {
                    continue;
                }
                // Walk the arc to the next retained vertex. Unretained vertices have
                // degree two (checked above), so the walk is unique.
                let mut path = vec![start];
                let (mut at, mut next) = (start, first);
                loop {
                    effort.charge(4 * log(border.edges.len()))?;
                    if !visited.insert(key(at, next)) {
                        return Err(Error::InvalidState("border arc revisits an edge".into()));
                    }
                    path.push(next);
                    if retained(&next) {
                        break;
                    }
                    let around = &border.adjacent[&next];
                    let other = if around[0] == at {
                        around[1]
                    } else {
                        around[0]
                    };
                    (at, next) = (next, other);
                }
                if (visited.len() + images.len() + contracted.len() + chords.len())
                    .saturating_mul(ITEM_BYTES)
                    .saturating_add(used)
                    .saturating_add(border.edges.len().saturating_mul(3 * ITEM_BYTES))
                    > self.decoded_limit
                {
                    return Err(Error::limit(
                        "interfaceSegments: one tile's border exceeds the decoded document budget",
                    ));
                }
                self.arc(tile, g, surface, &border, &path, &mut chords, &mut images)?;
                if path.len() > 2 {
                    let (a, b) = (path[0], path[path.len() - 1]);
                    contracted.insert(key(a, b), path);
                }
            }
        }
        if visited.len() != border.edges.len() {
            return Err(bad(
                "interfaceSegments",
                &format!(
                    "decoded ancestor retains no vertex of a closed shared leaf arc: tile={tile}"
                ),
            ));
        }
        for p in border.points.iter().chain(border.adjacent.keys()) {
            if retained(p) {
                images.insert(*p, (Vec::new(), 0.));
            }
        }
        let mut insert = self
            .connection
            .prepare_cached("INSERT INTO border_map VALUES(?,?,?,?)")
            .map_err(sql)?;
        for (p, (image, d)) in &images {
            effort.charge(1)?;
            insert
                .execute(params![super::point_bytes(*p), tile, image, d])
                .map_err(sql)?;
        }
        Ok(contracted)
    }

    /// One arc between retained vertices: its chord, directions and displacement.
    #[allow(clippy::too_many_arguments)]
    fn arc(
        &self,
        tile: i64,
        g: Option<f64>,
        surface: &Surface,
        border: &Border,
        path: &[CanonicalPoint],
        chords: &mut BTreeSet<CanonicalEdge>,
        images: &mut BTreeMap<CanonicalPoint, (Vec<u8>, f64)>,
    ) -> Result<(), Error> {
        let m = path.len() - 1;
        let (a, b) = (path[0], path[m]);
        if a == b {
            return Err(bad(
                "interfaceSegments",
                &format!("a closed shared leaf arc keeps one decoded vertex: tile={tile}"),
            ));
        }
        let chord = key(a, b);
        if !chords.insert(chord) {
            return Err(bad(
                "interfaceSegments",
                &format!("two shared leaf arcs share one decoded chord: tile={tile}"),
            ));
        }
        let Some(incidence) = surface.edges.get(&chord) else {
            return Err(bad(
                "interfaceSegments",
                &format!(
                    "decoded ancestor lacks the chord of an exact shared leaf arc: tile={tile}"
                ),
            ));
        };
        // Every delivered chord direction needs an inside source witness on every
        // edge of its arc.
        let allowed = path.windows(2).fold(3, |mask, w| {
            mask & along(border.edges[&key(w[0], w[1])], w[0], w[1])
        });
        let actual = along(incidence.directions(), a, b);
        if actual == 0 || actual & !allowed != 0 {
            return Err(bad(
                "interfaceWinding",
                &format!(
                    "decoded chord direction is not witnessed along its source arc: tile={tile}"
                ),
            ));
        }
        let m64 = m as u64;
        if m > 1 && g.is_none() {
            return Err(bad(
                "borderDisplacement",
                &format!("contracted border of tile {tile} without a finite geometricError"),
            ));
        }
        let g = g.unwrap_or(0.);
        for (i, &v) in path.iter().enumerate().take(m).skip(1) {
            let d = displacement(a, b, v, i as u64, m64);
            if d > g / 16. {
                return Err(bad(
                    "borderDisplacement",
                    &format!("contracted border vertex is {d} from its chord, above G/16 = {}: tile={tile}", g / 16.),
                ));
            }
            let from_lower = if a == chord.0 {
                i as u64
            } else {
                m64 - i as u64
            };
            images.insert(v, (image(chord, from_lower, m64), d));
        }
        Ok(())
    }

    /// Every directed incidence is counted, not reduced to a direction mask. A
    /// coarse edge used n >= 2 times in one direction needs n descendant source faces
    /// that way on every source edge it stands for (a nonmanifold spine retained).
    /// Otherwise the edge must keep the other direction and those faces must lie on
    /// distinct sides (`distinct_sides`: no acute wedge, exactly): a simplified book
    /// spine's perpendicular pages or a collapsed plate's opposite sheets. A fold
    /// re-flipped onto the border, a duplicate or a leaning flap is refused, balanced
    /// or not. Coincident faces (n on one corner set, either orientation) need, at
    /// every corner, a descendant coincident source group of at least n faces.
    pub(super) fn equal_directions(
        &self,
        tile: i64,
        surface: &Surface,
        contracted: &BTreeMap<CanonicalEdge, Vec<CanonicalPoint>>,
        effort: &Effort,
    ) -> Result<(), Error> {
        for (edge, e) in &surface.edges {
            effort.charge(1)?;
            if e.forward < 2 && e.reverse < 2 {
                continue;
            }
            let arc: Vec<(CanonicalPoint, CanonicalPoint)> = match contracted.get(edge) {
                Some(path) => path.windows(2).map(|w| (w[0], w[1])).collect(),
                None => vec![*edge],
            };
            // Descendant faces on every source edge, read in the coarse edge's
            // lower-to-higher direction and back; the arc may run either way.
            let lower_first = arc[0].0 == edge.0;
            let mut support = [i64::MAX; 2];
            for (x, y) in arc {
                effort.charge(1)?;
                let (nf, nr) = self.leaf_incidence(tile, key(x, y))?;
                let (with, against) = if x < y { (nf, nr) } else { (nr, nf) };
                let (forward, reverse) = if lower_first {
                    (with, against)
                } else {
                    (against, with)
                };
                support = [support[0].min(forward), support[1].min(reverse)];
            }
            for (direction, n, other) in [(0, e.forward, e.reverse), (1, e.reverse, e.forward)] {
                if n < 2 || support[direction] >= n as i64 {
                    continue;
                }
                effort.charge(n.saturating_mul(n) as usize)?;
                // A lone reversed face (2/0 across the edge) keeps no other direction.
                if other == 0 || !distinct_sides(edge, direction, surface) {
                    return Err(bad(
                        "interfaceWinding",
                        &format!(
                            "coarse edge used {}/{} times per direction, overlapping without that descendant multiplicity: tile={tile}",
                            e.forward, e.reverse
                        ),
                    ));
                }
            }
        }
        let mut corners = BTreeMap::<[CanonicalPoint; 3], i64>::new();
        for f in &surface.faces {
            effort.charge(1)?;
            let mut k = *f;
            k.sort_unstable();
            *corners.entry(k).or_default() += 1;
        }
        let mut licensed = self
            .connection
            .prepare_cached(
                "SELECT EXISTS(SELECT 1 FROM twin_points p,tiles t WHERE t.id=?2 AND p.key=?1 AND
                 (SELECT coalesce(sum(f.n),0) FROM leaf_faces f WHERE f.corners=p.corners AND f.tile>t.id AND f.tile<=t.last)>=?3)",
            )
            .map_err(sql)?;
        for (c, &n) in corners.iter().filter(|(_, n)| **n > 1) {
            for v in c {
                effort.charge(1)?;
                let ok: bool = licensed
                    .query_row(params![super::point_bytes(*v), tile, n], |r| r.get(0))
                    .map_err(sql)?;
                if !ok {
                    return Err(bad(
                        "interfaceWinding",
                        &format!("{n} coincident coarse faces without a coincident source group of that multiplicity at every corner: tile={tile}"),
                    ));
                }
            }
        }
        Ok(())
    }

    /// Descendant leaf faces on a canonical edge, (lower to higher key, back).
    fn leaf_incidence(&self, tile: i64, edge: CanonicalEdge) -> Result<(i64, i64), Error> {
        self.connection
            .prepare_cached("SELECT coalesce(sum(s.nf),0),coalesce(sum(s.nr),0) FROM leaf_segments s,tiles t WHERE t.id=?2 AND s.key=?1 AND s.tile>t.id AND s.tile<=t.last")
            .and_then(|mut q| {
                q.query_row(params![super::edge_bytes(edge), tile], |r| {
                    Ok((r.get(0)?, r.get(1)?))
                })
            })
            .map_err(sql)
    }

    /// Pairwise rules at every shared border vertex, grouped by key.
    pub(super) fn border_pairs(&self, effort: &Effort) -> Result<(), Error> {
        // Leaves deliver every shared point exactly.
        self.connection
            .execute_batch("INSERT INTO border_map SELECT l.key,l.tile,x'',0 FROM leaf_points l JOIN shared_point_keys s ON s.key=l.key JOIN tiles t ON t.id=l.tile WHERE t.leaf=1")
            .map_err(sql)?;
        let mut query = self
            .connection
            .prepare("SELECT b.key,b.tile,t.last,t.cell,c.last,t.error,t.leaf,b.image,b.d FROM border_map b JOIN tiles t ON t.id=b.tile LEFT JOIN tiles c ON c.id=t.cell ORDER BY b.key,b.tile")
            .map_err(sql)?;
        let mut rows = query.query([]).map_err(sql)?;
        let mut group: Vec<Owner> = Vec::new();
        let mut current: Option<Vec<u8>> = None;
        loop {
            let row = rows.next().map_err(sql)?;
            let key: Option<Vec<u8>> = row.map(|r| r.get(0)).transpose().map_err(sql)?;
            if key != current {
                vertex_pairs(&group, effort)?;
                group.clear();
                current.clone_from(&key);
            }
            let Some(row) = row else { break };
            effort.charge(1)?;
            if group.len().saturating_add(1).saturating_mul(ITEM_BYTES) > self.decoded_limit {
                return Err(Error::limit(
                    "interfacePoints: one shared border vertex's owners exceed the decoded document budget",
                ));
            }
            let cell: Option<i64> = row.get(3).map_err(sql)?;
            let cell_last: Option<i64> = row.get(4).map_err(sql)?;
            group.push(Owner {
                tile: row.get(1).map_err(sql)?,
                last: row.get(2).map_err(sql)?,
                cell: cell.zip(cell_last),
                g: row.get::<_, Option<f64>>(5).map_err(sql)?.unwrap_or(0.),
                leaf: row.get(6).map_err(sql)?,
                image: row.get(7).map_err(sql)?,
                d: row.get(8).map_err(sql)?,
            });
        }
        Ok(())
    }
}

/// One tile delivering a shared border vertex: its preorder range, cell parent's
/// range (nearest non-placeholder ancestor), G, image and displacement.
struct Owner {
    tile: i64,
    last: i64,
    cell: Option<(i64, i64)>,
    g: f64,
    leaf: bool,
    image: Vec<u8>,
    d: f64,
}

/// Disjoint tiles below one cell parent and outside a tile are logically no
/// shallower than it: siblings are the same depth, everything else is deeper.
fn vertex_pairs(group: &[Owner], effort: &Effort) -> Result<(), Error> {
    let n = group.len();
    if n < 2 {
        return Ok(());
    }
    let levels = n.ilog2() as usize + 1;
    effort.charge(n.saturating_mul(levels + 2))?;
    // Siblings: one logical depth, identical images (zero gap).
    let mut first = HashMap::<i64, usize>::new();
    for (i, r) in group.iter().enumerate() {
        let Some((cell, _)) = r.cell else { continue };
        match first.get(&cell) {
            Some(&j) if group[j].image != r.image => {
                return Err(bad(
                    "sameDepthBorderGap",
                    &format!(
                        "sibling tiles {} and {} deliver one border vertex differently",
                        group[j].tile, r.tile
                    ),
                ));
            }
            Some(_) => {}
            None => {
                first.insert(cell, i);
            }
        }
    }
    // Range maxima (sparse table) of displacement, and contraction counts, over
    // the group's tiles in preorder.
    let mut table = vec![group.iter().map(|r| r.d).collect::<Vec<f64>>()];
    for level in 1..levels {
        let previous = &table[level - 1];
        let width = 1 << (level - 1);
        let next: Vec<f64> = (0..=n - 2 * width)
            .map(|i| previous[i].max(previous[i + width]))
            .collect();
        table.push(next);
    }
    let maximum = |lo: usize, hi: usize| -> f64 {
        if lo >= hi {
            return 0.;
        }
        let level = (hi - lo).ilog2() as usize;
        table[level][lo].max(table[level][hi - (1 << level)])
    };
    let mut moved = vec![0usize; n + 1];
    for (i, r) in group.iter().enumerate() {
        moved[i + 1] = moved[i] + usize::from(!r.image.is_empty());
    }
    let index = |id: i64| group.partition_point(|r| r.tile < id);
    for x in group {
        let Some((cell, cell_last)) = x.cell else {
            continue;
        };
        let ranges = [
            (index(cell + 1), index(x.tile)),
            (index(x.last + 1), index(cell_last + 1)),
        ];
        if x.leaf {
            if ranges.iter().any(|&(lo, hi)| moved[hi] > moved[lo]) {
                return Err(bad(
                    "terminalBorderLock",
                    &format!(
                        "a tile no shallower than leaf {} contracts their shared border vertex",
                        x.tile
                    ),
                ));
            }
        } else {
            let deeper = ranges
                .iter()
                .map(|&(lo, hi)| maximum(lo, hi))
                .fold(0., f64::max);
            if (x.d + deeper) * OUTWARD > x.g / 8. {
                return Err(bad(
                    "mixedDepthBorderGap",
                    &format!(
                        "border gap bound {} beside tile {} exceeds its G/8 = {}",
                        x.d + deeper,
                        x.tile,
                        x.g / 8.
                    ),
                ));
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    //! Native D11 controls on a flat two-cell fixture. Cell A is x in [0, 4], cell B
    //! x in [4, 8], each split into two leaves at z = 2. Their shared source arc
    //! P0..P4 runs along x = 4 with P2 moved 0.1 off the chord P0-P4, so a
    //! contraction to that chord displaces P2 by 0.1 and nothing else.
    use super::super::tests::{budget, replay_fixture, store};
    use super::super::{Point, IDENTITY};
    use super::*;

    const P: [Point; 5] = [
        [4., 0., 0.],
        [4., 0., 1.],
        [4.1, 0., 2.],
        [4., 0., 3.],
        [4., 0., 4.],
    ];
    /// Triangles fanned from a loop's first corner (loops are counterclockwise
    /// seen from -y, so every border edge has one inside direction).
    fn fan(lp: &[[f64; 2]]) -> Vec<[Point; 3]> {
        let p = |q: [f64; 2]| [q[0], 0., q[1]];
        (1..lp.len() - 1)
            .map(|i| [p(lp[0]), p(lp[i]), p(lp[i + 1])])
            .collect()
    }
    fn xz(p: Point) -> [f64; 2] {
        [p[0], p[2]]
    }
    fn a_leaves() -> [Vec<[Point; 3]>; 2] {
        [
            fan(&[[0., 0.], xz(P[0]), xz(P[1]), xz(P[2]), [0., 2.]]),
            fan(&[[0., 2.], xz(P[2]), xz(P[3]), xz(P[4]), [0., 4.]]),
        ]
    }
    fn b_leaves() -> [Vec<[Point; 3]>; 2] {
        [
            fan(&[xz(P[0]), [8., 0.], [8., 2.], xz(P[2]), xz(P[1])]),
            fan(&[xz(P[2]), [8., 2.], [8., 4.], xz(P[4]), xz(P[3])]),
        ]
    }
    /// Coarse A keeping the given border vertices (indices into P).
    fn a_coarse(keep: &[usize]) -> Vec<[Point; 3]> {
        let mut lp = vec![[0., 0.]];
        lp.extend(keep.iter().map(|&i| xz(P[i])));
        lp.push([0., 4.]);
        fan(&lp)
    }
    fn b_coarse(keep: &[usize]) -> Vec<[Point; 3]> {
        let mut lp = vec![[8., 4.], xz(P[4])];
        lp.extend(
            keep.iter()
                .rev()
                .map(|&i| xz(P[i]))
                .filter(|q| *q != xz(P[4])),
        );
        lp.push([8., 0.]);
        fan(&lp)
    }
    /// Tiles in preorder: (parent, G, payload). The root is an omitted cell.
    /// One tile: parent, G and payload (none for an omitted cell).
    type Node = (Option<u64>, f64, Option<Vec<[Point; 3]>>);
    fn tree(tiles: &[Node]) -> Result<(), Error> {
        let mut s = store();
        s.decoded_limit = 1 << 24;
        let mut payloads = Vec::new();
        let leaf = |i: usize| !tiles.iter().any(|t| t.0 == Some(i as u64));
        for (id, (parent, g, faces)) in tiles.iter().enumerate() {
            s.tile(id as u64, *parent, IDENTITY)?;
            match faces {
                None => s.kind(id as u64, None, false, 16., Some(1.))?,
                Some(faces) => {
                    s.kind(id as u64, None, false, *g, None)?;
                    s.reference(id as u64, payloads.len() as u64)?;
                    s.payload(payloads.len() as u64, true, leaf(id))?;
                    payloads.push(replay_fixture(faces));
                }
            }
        }
        s.finish(budget(), 0, &mut |rank| Ok(payloads[rank as usize].clone()))
            .map(|_| ())
    }
    /// root -> A(G_a) -> A0, A1; B(G_b) -> B0, B1.
    fn siblings(a: Vec<[Point; 3]>, g_a: f64, b: Vec<[Point; 3]>, g_b: f64) -> Result<(), Error> {
        let [a0, a1] = a_leaves();
        let [b0, b1] = b_leaves();
        tree(&[
            (None, 16., None),
            (Some(0), g_a, Some(a)),
            (Some(1), 0., Some(a0)),
            (Some(1), 0., Some(a1)),
            (Some(0), g_b, Some(b)),
            (Some(4), 0., Some(b0)),
            (Some(4), 0., Some(b1)),
        ])
    }
    fn refused(result: Result<(), Error>, check: &str) {
        let message = result.expect_err(check).to_string();
        assert!(message.contains(check), "{check}: {message}");
    }
    const ALL: [usize; 5] = [0, 1, 2, 3, 4];

    #[test]
    fn displacement_is_exact_on_the_chord_and_rounded_outward() {
        let (a, b) = ([0u32; 3], [1f32.to_bits(), 0, 0]);
        let mid = [0.5f32.to_bits(), 0, 0];
        // Only the outward rounding allowance remains on the chord.
        assert!(displacement(a, b, mid, 1, 2) < 1e-14);
        let off = [0.5f32.to_bits(), 0.25f32.to_bits(), 0];
        let d = displacement(a, b, off, 1, 2);
        assert!((0.25..0.25 * (1. + 1e-12)).contains(&d));
    }
    #[test]
    fn wedge_sign_is_exact_and_displacement_translation_invariant() {
        use std::cmp::Ordering::*;
        let (a, b) = ([0f32, 0., 0.], [0f32, 0., 1.]);
        // Perpendicular pages, an acute pair, an obtuse pair and a collinear face.
        assert_eq!(wedge(a, b, [1., 0., 0.5], [0., 1., 0.25]), Equal);
        assert_eq!(wedge(a, b, [1., 0., 0.5], [1., 1e-7, 0.25]), Greater);
        assert_eq!(wedge(a, b, [1., 0., 0.5], [-1., 1e-7, 0.25]), Less);
        assert_eq!(wedge(a, b, [0., 0., 3.], [0., 0., 3.]), Equal);
        // Exact even where every coordinate is far from the origin.
        let far = 1e12f32;
        let shift = |p: [f32; 3]| [p[0], p[1] + far, p[2]];
        assert_eq!(
            wedge(
                shift(a),
                shift(b),
                shift([1., 0., 0.5]),
                shift([0., 4096., 0.25])
            ),
            Equal
        );
        // A rigid translation leaves a contraction's displacement unchanged
        // (review r1: an absolute-coordinate allowance refused it at y = 1e12).
        let bits = |p: [f32; 3]| p.map(|v| if v == 0. { 0 } else { v.to_bits() });
        let (p0, p2, p4) = ([4f32, 0., 0.], [4.0001f32, 0., 2.], [4f32, 0., 4.]);
        let near = displacement(bits(p0), bits(p4), bits(p2), 2, 4);
        let moved = displacement(bits(shift(p0)), bits(shift(p4)), bits(shift(p2)), 2, 4);
        assert!(
            near < 0.000101 && (moved - near).abs() < 1e-12,
            "{near} {moved}"
        );
        // On the chord only the difference-scaled allowance remains, not 1e12 ulps.
        let on = displacement(
            bits(shift(p0)),
            bits(shift(p4)),
            bits(shift([4., 0., 1.])),
            1,
            4,
        );
        assert!(on < 1e-13, "{on}");
    }
    #[test]
    fn coincident_coarse_faces_need_a_coincident_source_group_at_every_corner() {
        // A tiny back-to-back source plate in A0 does not license a new coarse twin
        // elsewhere (review r1 finding 1); it licenses one on its own corners.
        let plate = [[1., 0.1, 1.], [1.001, 0.1, 1.], [1., 0.1, 1.001]];
        let twin = [plate[0], plate[2], plate[1]];
        let case = |extra: Vec<[Point; 3]>| {
            let [mut a0, a1] = a_leaves();
            a0.extend([plate, twin]);
            let [b0, b1] = b_leaves();
            let mut a = a_coarse(&[0, 4]);
            a.extend([plate, twin]);
            a.extend(extra);
            tree(&[
                (None, 16., None),
                (Some(0), 2., Some(a)),
                (Some(1), 0., Some(a0)),
                (Some(1), 0., Some(a1)),
                (Some(0), 2., Some(b_coarse(&[0, 4]))),
                (Some(4), 0., Some(b0)),
                (Some(4), 0., Some(b1)),
            ])
        };
        case(Vec::new()).unwrap();
        let t = [[1.1, 0., 1.1], [1.2, 0., 1.1], [1.1, 0., 1.2]];
        let r = [t[0], t[2], t[1]];
        refused(case(vec![t, r, t, r]), "interfaceWinding");
        refused(case(vec![t, r]), "interfaceWinding");
        // Three coarse copies on the plate's corners exceed its two source faces.
        refused(case(vec![plate]), "interfaceWinding");
    }
    #[test]
    fn identical_sibling_contraction_within_g_over_16_passes() {
        // d = 0.1 <= 2/16; A + B gap bound 0.2 <= 2/8.
        siblings(a_coarse(&[0, 4]), 2., b_coarse(&[0, 4]), 2.).unwrap();
        // Exact borders need no G at all.
        siblings(a_coarse(&ALL), 0., b_coarse(&ALL), 0.).unwrap();
        // Keeping P2 moves P1 and P3 by 0.05 off the chords through it.
        siblings(a_coarse(&[0, 2, 4]), 1., b_coarse(&[0, 2, 4]), 1.).unwrap();
    }
    #[test]
    fn contraction_beyond_g_over_16_is_refused() {
        refused(
            siblings(a_coarse(&[0, 4]), 1.5, b_coarse(&[0, 4]), 1.5),
            "borderDisplacement",
        );
        // A contraction with no published G is never within it.
        refused(
            siblings(a_coarse(&[0, 2, 4]), 0., b_coarse(&[0, 2, 4]), 0.),
            "borderDisplacement",
        );
    }
    #[test]
    fn siblings_must_deliver_identical_borders() {
        refused(
            siblings(a_coarse(&[0, 4]), 2., b_coarse(&ALL), 2.),
            "sameDepthBorderGap",
        );
        refused(
            siblings(a_coarse(&[0, 2, 4]), 2., b_coarse(&[0, 4]), 2.),
            "sameDepthBorderGap",
        );
    }
    #[test]
    fn border_arcs_need_one_witnessed_chord_and_exact_ends() {
        // P0 ends the shared arc (border degree one): never contracted.
        refused(
            siblings(a_coarse(&[1, 2, 3, 4]), 2., b_coarse(&[1, 2, 3, 4]), 2.),
            "interfacePoints",
        );
        // A vertex off the source arc leaves P0-P4 without its chord.
        let moved = fan(&[[0., 0.], xz(P[0]), [4.05, 2.], xz(P[4]), [0., 4.]]);
        refused(
            siblings(moved, 2., b_coarse(&[0, 4]), 2.),
            "interfaceSegments",
        );
        // A reversed coarse A runs its chord against every inside source direction.
        let reversed: Vec<[Point; 3]> = a_coarse(&[0, 4])
            .into_iter()
            .map(|[x, y, z]| [x, z, y])
            .collect();
        refused(
            siblings(reversed, 2., b_coarse(&[0, 4]), 2.),
            "interfaceWinding",
        );
    }
    #[test]
    fn folds_beside_contracted_and_exact_borders_are_refused() {
        // A second face on the same side of chord P0 -> P4 (review: contraction
        // folds re-flipped onto the border) has one source face per arc edge.
        let mut folded = a_coarse(&[0, 4]);
        folded.push([[2., 0., 1.], P[0], P[4]]);
        assert!(siblings(a_coarse(&[0, 4]), 2., b_coarse(&[0, 4]), 2.).is_ok());
        refused(
            siblings(folded, 2., b_coarse(&[0, 4]), 2.),
            "interfaceWinding",
        );
        // An overlapping duplicate on an exact border, and three faces one way.
        let mut doubled = a_coarse(&ALL);
        doubled.push(doubled[1]);
        refused(
            siblings(doubled, 0., b_coarse(&ALL), 0.),
            "interfaceWinding",
        );
        // A same-side duplicate where the other direction is present (2/1).
        let mut overlapped = a_coarse(&[0, 4]);
        overlapped.push(overlapped[0]);
        refused(
            siblings(overlapped, 2., b_coarse(&[0, 4]), 2.),
            "interfaceWinding",
        );
        // A coincident reversed twin: each side single, but no source has it.
        let mut twinned = a_coarse(&[0, 4]);
        let [x, y, z] = twinned[0];
        twinned.push([x, z, y]);
        refused(
            siblings(twinned, 2., b_coarse(&[0, 4]), 2.),
            "interfaceWinding",
        );
        let mut tripled = a_coarse(&[0, 4]);
        tripled.extend([[[2., 0., 1.], P[0], P[4]], [[1., 0., 1.], P[0], P[4]]]);
        refused(
            siblings(tripled, 2., b_coarse(&[0, 4]), 2.),
            "interfaceWinding",
        );
    }
    /// root -> A(G_a) -> A0, A1; B(2) -> M(2, keeping `deep`) -> B0, B1.
    fn mixed(g_a: f64, a: &[usize], b: &[usize], deep: &[usize]) -> Result<(), Error> {
        let [a0, a1] = a_leaves();
        let [b0, b1] = b_leaves();
        tree(&[
            (None, 16., None),
            (Some(0), g_a, Some(a_coarse(a))),
            (Some(1), 0., Some(a0)),
            (Some(1), 0., Some(a1)),
            (Some(0), 2., Some(b_coarse(b))),
            (Some(4), 2., Some(b_coarse(deep))),
            (Some(5), 0., Some(b0)),
            (Some(5), 0., Some(b1)),
        ])
    }
    #[test]
    fn a_deeper_neighbour_stays_within_the_coarser_g_over_8() {
        // M contracts P2 by 0.1 (<= its 2/16) beside an exact A.
        mixed(1., &ALL, &ALL, &[0, 4]).unwrap();
        refused(mixed(0.5, &ALL, &ALL, &[0, 4]), "mixedDepthBorderGap");
    }
    #[test]
    fn a_shallower_leaf_locks_every_deeper_neighbour_exactly() {
        // root -> L (one leaf over all of A); B(2) -> M(2) -> B0, B1.
        let [a0, a1] = a_leaves();
        let l: Vec<[Point; 3]> = a0.into_iter().chain(a1).collect();
        let [b0, b1] = b_leaves();
        let case = |deep: &[usize]| {
            tree(&[
                (None, 16., None),
                (Some(0), 0., Some(l.clone())),
                (Some(0), 2., Some(b_coarse(&ALL))),
                (Some(2), 2., Some(b_coarse(deep))),
                (Some(3), 0., Some(b0.clone())),
                (Some(3), 0., Some(b1.clone())),
            ])
        };
        case(&ALL).unwrap();
        refused(case(&[0, 4]), "terminalBorderLock");
    }
}
