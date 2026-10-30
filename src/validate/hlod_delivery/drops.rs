//! D11 coarse drops (owner decision 2026-10-28). A coarse payload may lose whole
//! components of its immediate children's payloads, and declares how many in
//! `asset.extras.rustyTiles.droppedComponents`. They are recounted here from decoded
//! canonical keys alone: a child component (faces joined by shared keys) is dropped
//! when the parent holds none of its keys, since every coarse vertex is a source key.
//! A drop at or above the children's largest G is a coarse drop: it needs cap pressure
//! (the children's faces exceed the cap) and goes smallest first (no border-free child
//! component smaller than it stays). Every dropped key, and S2's fixed sample points on
//! every dropped face, must lie within the parent's geometricError of its surface (the
//! face's child G added off its vertices): lower bounds on the dropped surface's loss.
//! A drop that touches the border loses shared keys, which the retention checks
//! refuse; nothing here relaxes them.
use super::{bad, point_bytes, read_point, sql, CanonicalPoint, Effort, Error, Store};
use rustc_hash::FxHashMap as HashMap;
use std::collections::HashSet;

/// A parent's geometricError covers a dropped key up to this relative rounding.
const TOLERANCE: f64 = 1e-6;
/// The distance grid has at most this many cells per axis.
const GRID: f64 = 32.;

struct Content {
    id: i64,
    last: i64,
    leaf: bool,
    depth: usize,
    error: f64,
    declared: u64,
    digests: Vec<Vec<u8>>,
    /// Child tiles: id, leaf, geometricError.
    children: Vec<(i64, bool, f64)>,
}

/// Library tests bake at a 300-triangle cap (`hlod::tests::test_settings`); validation of
/// their archives applies cap pressure at the same cap. This holds for the whole lib test
/// binary, so the few tests baking at 8,192 get a laxer pressure check (never a false
/// refusal); `at` in the unit tests below sets it per case.
#[cfg(test)]
pub(crate) const TEST_DEFAULT_CAP: u64 = 300;
#[cfg(test)]
thread_local! {
    pub(crate) static TEST_CAP: std::cell::Cell<u64> = const { std::cell::Cell::new(TEST_DEFAULT_CAP) };
}
fn cap() -> u64 {
    #[cfg(test)]
    return TEST_CAP.with(|c| c.get());
    #[cfg(not(test))]
    super::TRIANGLE_CAP
}

struct Frame {
    id: i64,
    placeholder: bool,
    depth: usize,
    content: Option<Content>,
}

impl Store {
    pub(super) fn coarse_drops(
        &self,
        dropped_by_depth: &mut Vec<u64>,
        effort: &Effort,
    ) -> Result<(), Error> {
        self.connection
            .execute_batch("CREATE INDEX IF NOT EXISTS leaf_faces_tile ON leaf_faces(tile)")
            .map_err(sql)?;
        let mut query = self
            .connection
            .prepare(
                "SELECT t.id,t.parent,t.placeholder,t.leaf,t.error,t.last,
                 (SELECT group_concat(hex(p.digest)) FROM (SELECT DISTINCT p.digest FROM refs r
                    JOIN payloads p ON p.rank=r.rank WHERE r.tile=t.id ORDER BY p.digest) p),
                 (SELECT sum(p.dropped) FROM refs r JOIN payloads p ON p.rank=r.rank WHERE r.tile=t.id)
                 FROM tiles t ORDER BY t.id",
            )
            .map_err(sql)?;
        let mut rows = query.query([]).map_err(sql)?;
        let mut stack: Vec<Frame> = Vec::new();
        let mut close = |frame: Frame| -> Result<(), Error> {
            match frame.content {
                Some(c) if !c.leaf => self.recount(&c, dropped_by_depth, effort),
                Some(c) if c.declared != 0 => Err(bad(
                    "coarseDrops",
                    &format!("leaf tile {} declares dropped components", c.id),
                )),
                _ => Ok(()),
            }
        };
        while let Some(row) = rows.next().map_err(sql)? {
            effort.charge(1)?;
            let id: i64 = row.get(0).map_err(sql)?;
            let parent: Option<i64> = row.get(1).map_err(sql)?;
            while stack.last().map(|f| f.id) != parent {
                let frame = stack.pop().ok_or_else(|| {
                    Error::InvalidState("HLOD hierarchy is not in preorder".into())
                })?;
                close(frame)?;
            }
            let placeholder: bool = row.get(2).map_err(sql)?;
            let depth = stack
                .last()
                .map_or(0, |p| p.depth + usize::from(!p.placeholder));
            let digests: Option<String> = row.get(6).map_err(sql)?;
            let content = match digests {
                None => None,
                Some(digests) => {
                    let digests: Vec<Vec<u8>> =
                        digests.split(',').map(|d| d.as_bytes().to_vec()).collect();
                    let declared: i64 = row.get(7).map_err(sql)?;
                    let ancestor = stack.iter_mut().rev().find_map(|f| f.content.as_mut());
                    match ancestor {
                        // A copy down a single-child chain stands for the same cell
                        // (single cover); its descendants are the original's children.
                        Some(a) if a.digests == digests => None,
                        ancestor => {
                            let leaf: bool = row.get(3).map_err(sql)?;
                            let error = row.get::<_, Option<f64>>(4).map_err(sql)?.unwrap_or(0.);
                            if let Some(a) = ancestor {
                                a.children.push((id, leaf, error));
                            }
                            Some(Content {
                                id,
                                last: row.get::<_, Option<i64>>(5).map_err(sql)?.unwrap_or(id),
                                leaf,
                                depth,
                                error,
                                declared: u64::try_from(declared)
                                    .map_err(|_| bad("coarseDrops", "negative declaration"))?,
                                digests,
                                children: Vec::new(),
                            })
                        }
                    }
                }
            };
            stack.push(Frame {
                id,
                placeholder,
                depth,
                content,
            });
        }
        while let Some(frame) = stack.pop() {
            close(frame)?;
        }
        Ok(())
    }

    /// A delivered tile's faces by canonical key and multiplicity, from the leaf and
    /// coarse face tables already filled by the delivery replays: the recount decodes
    /// nothing again.
    fn faces(&self, tile: i64, leaf: bool) -> Result<Vec<([CanonicalPoint; 3], u64)>, Error> {
        let mut query = self
            .connection
            .prepare_cached(if leaf {
                "SELECT corners,n FROM leaf_faces WHERE tile=?"
            } else {
                "SELECT corners,1 FROM coarse_faces WHERE tile=?"
            })
            .map_err(sql)?;
        let mut rows = query.query([tile]).map_err(sql)?;
        let mut faces = Vec::new();
        while let Some(row) = rows.next().map_err(sql)? {
            let bytes: Vec<u8> = row.get(0).map_err(sql)?;
            if bytes.len() != 36 {
                return Err(Error::InvalidState("corrupt canonical face scratch".into()));
            }
            let n: i64 = row.get(1).map_err(sql)?;
            faces.push((
                [
                    read_point(&bytes[..12])?,
                    read_point(&bytes[12..24])?,
                    read_point(&bytes[24..])?,
                ],
                u64::try_from(n).map_err(|_| Error::InvalidState("negative face count".into()))?,
            ));
        }
        Ok(faces)
    }

    /// Whether a key is held by a leaf outside the preorder range `first..=last`: a
    /// border point of that tile. One seek on each side of the range; the caller charges
    /// the first, the second is charged here.
    fn outside_leaf(
        &self,
        key: CanonicalPoint,
        first: i64,
        last: i64,
        effort: &Effort,
    ) -> Result<bool, Error> {
        let key = point_bytes(key);
        let mut before = self
            .connection
            .prepare_cached("SELECT EXISTS(SELECT 1 FROM leaf_points WHERE key=? AND tile<?)")
            .map_err(sql)?;
        if before
            .query_row(rusqlite::params![key, first], |r| r.get(0))
            .map_err(sql)?
        {
            return Ok(true);
        }
        effort.charge(super::SEEK_UNITS)?;
        self.connection
            .prepare_cached("SELECT EXISTS(SELECT 1 FROM leaf_points WHERE key=? AND tile>?)")
            .map_err(sql)?
            .query_row(rusqlite::params![key, last], |r| r.get(0))
            .map_err(sql)
    }

    fn recount(
        &self,
        parent: &Content,
        dropped_by_depth: &mut Vec<u64>,
        effort: &Effort,
    ) -> Result<(), Error> {
        let faces: Vec<[CanonicalPoint; 3]> = self
            .faces(parent.id, false)?
            .into_iter()
            .map(|(f, _)| f)
            .collect();
        let kept: HashSet<CanonicalPoint> = faces.iter().flatten().copied().collect();
        // Union-find over the children's faces by canonical key.
        let mut index: HashMap<CanonicalPoint, usize> = HashMap::default();
        let mut root: Vec<usize> = Vec::new();
        fn find(root: &mut [usize], mut v: usize) -> usize {
            while root[v] != v {
                root[v] = root[root[v]];
                v = root[v];
            }
            v
        }
        let mut keys: Vec<CanonicalPoint> = Vec::new();
        // Each child face with its child's G, and the children's face count.
        let mut child_faces: Vec<([usize; 3], f64)> = Vec::new();
        let (mut count_faces, mut child_g) = (0u64, 0f64);
        for &(child, leaf, g) in &parent.children {
            child_g = child_g.max(g);
            for (face, n) in self.faces(child, leaf)? {
                effort.charge(1)?;
                count_faces = count_faces.saturating_add(n);
                let ids = face.map(|k| {
                    *index.entry(k).or_insert_with(|| {
                        root.push(root.len());
                        keys.push(k);
                        root.len() - 1
                    })
                });
                for i in 1..3 {
                    let (a, b) = (find(&mut root, ids[0]), find(&mut root, ids[i]));
                    root[a.max(b)] = a.min(b);
                }
                child_faces.push((ids, g));
            }
        }
        drop(index);
        let mut present = vec![false; keys.len()];
        for (v, k) in keys.iter().enumerate() {
            if kept.contains(k) {
                let r = find(&mut root, v);
                present[r] = true;
            }
        }
        // Every component's members and bounding-box diagonal.
        let mut components: HashMap<usize, Vec<usize>> = HashMap::default();
        for v in 0..keys.len() {
            let r = find(&mut root, v);
            components.entry(r).or_default().push(v);
        }
        let diagonal = |members: &[usize]| {
            let (mut lo, mut hi) = ([f64::MAX; 3], [f64::MIN; 3]);
            for &v in members {
                for (a, x) in point(keys[v]).into_iter().enumerate() {
                    lo[a] = lo[a].min(x);
                    hi[a] = hi[a].max(x);
                }
            }
            (0..3).map(|a| (hi[a] - lo[a]).powi(2)).sum::<f64>().sqrt()
        };
        let count = components.keys().filter(|r| !present[**r]).count() as u64;
        if count != parent.declared {
            return Err(bad(
                "coarseDrops",
                &format!(
                    "tile {} drops {count} child components but declares {}",
                    parent.id, parent.declared
                ),
            ));
        }
        if count == 0 {
            return Ok(());
        }
        // Coarse drops: cap pressure, then smallest first against every kept border-free
        // component (border contact is a key held by a leaf outside this tile).
        let largest = components
            .iter()
            .filter(|(r, _)| !present[**r])
            .map(|(_, m)| diagonal(m))
            .fold(f64::MIN, f64::max);
        if largest >= child_g {
            if count_faces <= cap() {
                return Err(bad(
                    "coarseDrops",
                    &format!(
                        "tile {} drops a component of diagonal {largest} at or above its \
                         children's G {child_g} though its children's {count_faces} faces fit \
                         the {} cap",
                        parent.id,
                        cap()
                    ),
                ));
            }
            for (r, members) in &components {
                if !present[*r] || diagonal(members) >= largest {
                    continue;
                }
                let mut touches = false;
                for &v in members {
                    effort.charge(super::SEEK_UNITS)?;
                    if self.outside_leaf(keys[v], parent.id, parent.last, effort)? {
                        touches = true;
                        break;
                    }
                }
                if !touches {
                    return Err(bad(
                        "coarseDrops",
                        &format!(
                            "tile {} keeps a border-free child component of diagonal {} \
                             but drops one of {largest}: drops go smallest first",
                            parent.id,
                            diagonal(members)
                        ),
                    ));
                }
            }
        }
        let grid = Grid::new(&faces, parent.error, child_g, effort)?;
        let far = |p: [f64; 3], slack: f64| -> Result<bool, Error> {
            effort.charge(1)?;
            Ok(!grid.within(p, slack, &faces, effort)?)
        };
        let refuse = |p: [f64; 3]| {
            bad(
                "coarseDrops",
                &format!(
                    "tile {} drops surface at {p:?} farther than its geometricError {} from \
                     its surface",
                    parent.id, parent.error
                ),
            )
        };
        for (r, members) in &components {
            if present[*r] {
                continue;
            }
            for &v in members {
                if far(point(keys[v]), 0.)? {
                    return Err(refuse(point(keys[v])));
                }
            }
        }
        // S2's fixed controls on every dropped face: its quarter grid and centroid. Off a
        // face's vertices its child may lie up to that child's G from the source.
        for (ids, g) in &child_faces {
            if present[find(&mut root, ids[0])] {
                continue;
            }
            let t = ids.map(|v| point(keys[v]));
            for p in samples(t) {
                if far(p, *g)? {
                    return Err(refuse(p));
                }
            }
        }
        if dropped_by_depth.len() <= parent.depth {
            dropped_by_depth.resize(parent.depth + 1, 0);
        }
        dropped_by_depth[parent.depth] += count;
        Ok(())
    }
}

/// S2's fixed controls on a triangle (`geometry::measurement::samples`): the quarter grid,
/// corners and edge points included, and the centroid.
fn samples(t: [[f64; 3]; 3]) -> impl Iterator<Item = [f64; 3]> {
    (0..=4)
        .flat_map(move |a| {
            (0..=4 - a).map(move |b| {
                let w = [a as f64 / 4., b as f64 / 4., (4 - a - b) as f64 / 4.];
                std::array::from_fn(|k| w[0] * t[0][k] + w[1] * t[1][k] + w[2] * t[2][k])
            })
        })
        .chain(std::iter::once(std::array::from_fn(|k| {
            (t[0][k] + t[1][k] + t[2][k]) / 3.
        })))
}

fn point(k: CanonicalPoint) -> [f64; 3] {
    k.map(|b| f64::from(f32::from_bits(b)))
}

/// The parent's faces binned by their box grown by the widest reach: a point within
/// that distance of a face lies in a cell the face was entered in. Every insertion is
/// charged to the validation budget.
struct Grid {
    lo: [f64; 3],
    size: f64,
    reach: f64,
    cells: HashMap<[i64; 3], Vec<u32>>,
}

impl Grid {
    fn new(
        faces: &[[CanonicalPoint; 3]],
        error: f64,
        slack: f64,
        effort: &Effort,
    ) -> Result<Self, Error> {
        let reach = error * (1. + TOLERANCE) + TOLERANCE;
        let widest = reach + slack;
        let mut lo = [f64::MAX; 3];
        let mut hi = [f64::MIN; 3];
        for f in faces {
            for k in f {
                let p = point(*k);
                for a in 0..3 {
                    lo[a] = lo[a].min(p[a] - widest);
                    hi[a] = hi[a].max(p[a] + widest);
                }
            }
        }
        let extent = (0..3).map(|a| hi[a] - lo[a]).fold(0., f64::max);
        let size = (extent / GRID).max(widest).max(f64::MIN_POSITIVE);
        let mut grid = Self {
            lo,
            size,
            reach,
            cells: HashMap::default(),
        };
        for (i, f) in faces.iter().enumerate() {
            let ps = f.map(point);
            let (mut a, mut b) = ([i64::MAX; 3], [i64::MIN; 3]);
            for p in ps {
                let (c, d) = (
                    grid.cell(p.map(|x| x - widest)),
                    grid.cell(p.map(|x| x + widest)),
                );
                for k in 0..3 {
                    a[k] = a[k].min(c[k]);
                    b[k] = b[k].max(d[k]);
                }
            }
            let cells = (0..3).map(|k| (b[k] - a[k] + 1) as u64).product::<u64>();
            effort.charge(cells as usize)?;
            for x in a[0]..=b[0] {
                for y in a[1]..=b[1] {
                    for z in a[2]..=b[2] {
                        grid.cells.entry([x, y, z]).or_default().push(i as u32);
                    }
                }
            }
        }
        Ok(grid)
    }
    fn cell(&self, p: [f64; 3]) -> [i64; 3] {
        std::array::from_fn(|a| ((p[a] - self.lo[a]) / self.size).floor() as i64)
    }
    /// Whether `p` lies within the parent's reach plus `slack` of a face.
    fn within(
        &self,
        p: [f64; 3],
        slack: f64,
        faces: &[[CanonicalPoint; 3]],
        effort: &Effort,
    ) -> Result<bool, Error> {
        let Some(cell) = self.cells.get(&self.cell(p)) else {
            return Ok(false);
        };
        let reach = self.reach + slack;
        for &i in cell {
            effort.charge(1)?;
            if distance2(p, faces[i as usize].map(point)) <= reach * reach {
                return Ok(true);
            }
        }
        Ok(false)
    }
}

/// Squared distance from `p` to a closed triangle (Ericson, closest point by region).
fn distance2(p: [f64; 3], t: [[f64; 3]; 3]) -> f64 {
    let sub = |a: [f64; 3], b: [f64; 3]| [a[0] - b[0], a[1] - b[1], a[2] - b[2]];
    let dot = |a: [f64; 3], b: [f64; 3]| a[0] * b[0] + a[1] * b[1] + a[2] * b[2];
    let at = |o: [f64; 3], d: [f64; 3], s: f64| [o[0] + s * d[0], o[1] + s * d[1], o[2] + s * d[2]];
    let [a, b, c] = t;
    let (ab, ac, ap) = (sub(b, a), sub(c, a), sub(p, a));
    let (d1, d2) = (dot(ab, ap), dot(ac, ap));
    let closest = if d1 <= 0. && d2 <= 0. {
        a
    } else {
        let bp = sub(p, b);
        let (d3, d4) = (dot(ab, bp), dot(ac, bp));
        let cp = sub(p, c);
        let (d5, d6) = (dot(ab, cp), dot(ac, cp));
        let vc = d1 * d4 - d3 * d2;
        let vb = d5 * d2 - d1 * d6;
        let va = d3 * d6 - d5 * d4;
        if d3 >= 0. && d4 <= d3 {
            b
        } else if d6 >= 0. && d5 <= d6 {
            c
        } else if vc <= 0. && d1 >= 0. && d3 <= 0. {
            at(a, ab, d1 / (d1 - d3))
        } else if vb <= 0. && d2 >= 0. && d6 <= 0. {
            at(a, ac, d2 / (d2 - d6))
        } else if va <= 0. && d4 - d3 >= 0. && d5 - d6 >= 0. {
            at(b, sub(c, b), (d4 - d3) / ((d4 - d3) + (d5 - d6)))
        } else {
            let denom = va + vb + vc;
            if denom == 0. {
                // Degenerate face: its closest point lies on an edge.
                return [(a, b), (b, c), (c, a)]
                    .into_iter()
                    .map(|(s, e)| {
                        let d = sub(e, s);
                        let dd = dot(d, d);
                        let t = if dd == 0. {
                            0.
                        } else {
                            (dot(sub(p, s), d) / dd).clamp(0., 1.)
                        };
                        let q = sub(p, at(s, d, t));
                        dot(q, q)
                    })
                    .fold(f64::MAX, f64::min);
            }
            let (v, w) = (vb / denom, vc / denom);
            [
                a[0] + ab[0] * v + ac[0] * w,
                a[1] + ab[1] * v + ac[1] * w,
                a[2] + ab[2] * v + ac[2] * w,
            ]
        }
    };
    let q = sub(p, closest);
    dot(q, q)
}

#[cfg(test)]
mod tests {
    use super::super::tests::{budget, replay_fixture, store};
    use super::super::{integer, params, sql, Error, Point, IDENTITY};

    type Face = [Point; 3];
    const NEAR: Face = [[0., 0., 0.], [2., 0., 0.], [0., 2., 0.]];
    /// A second component, its farthest key 4 m from `NEAR`.
    const FAR: Face = [[5., 0., 0.], [6., 0., 0.], [5., 1., 0.]];

    /// An omitted root over a coarse tile (G `error`, declaring `declared` drops) above
    /// one leaf, beside a second leaf.
    fn case(
        coarse: &[Face],
        leaf: &[Face],
        sibling: &[Face],
        error: f64,
        declared: u64,
    ) -> Result<Vec<u64>, Error> {
        ordered(coarse, leaf, sibling, error, declared, false)
    }
    /// `case`, with the sibling leaf before the coarse tile in preorder when `first`.
    fn ordered(
        coarse: &[Face],
        leaf: &[Face],
        sibling: &[Face],
        error: f64,
        declared: u64,
        first: bool,
    ) -> Result<Vec<u64>, Error> {
        let payloads = [
            replay_fixture(coarse),
            replay_fixture(leaf),
            replay_fixture(sibling),
        ];
        let mut s = store();
        s.decoded_limit = 1 << 24;
        // Tile ids of the coarse tile, its leaf and the sibling leaf.
        let [c, l, b] = if first { [2, 3, 1] } else { [1, 2, 3] };
        s.tile(0, None, IDENTITY)?;
        for id in 1..4 {
            s.tile(id, Some(if id == l { c } else { 0 }), IDENTITY)?;
        }
        s.kind(0, None, false, 1e3, Some(1.))?;
        s.kind(c, None, false, error, Some(1.))?;
        for (tile, rank) in [(c, 0), (l, 1), (b, 2)] {
            s.reference(tile, rank)?;
            s.payload(rank, true, rank != 0)?;
        }
        s.materialize()?;
        s.connection
            .execute(
                "UPDATE payloads SET dropped=? WHERE rank=0",
                params![integer(declared)?],
            )
            .map_err(sql)?;
        s.finish(budget(), 0, &mut |rank| Ok(payloads[rank as usize].clone()))?;
        Ok(s.census().unwrap().dropped_by_depth)
    }
    fn refused(result: Result<Vec<u64>, Error>, label: &str) {
        let e = result.unwrap_err().to_string();
        assert!(e.contains(label), "{e}");
    }

    /// Run `case` at `cap` (the children here hold a few faces).
    fn at(cap: u64, run: impl FnOnce() -> Result<Vec<u64>, Error>) -> Result<Vec<u64>, Error> {
        super::TEST_CAP.with(|c| c.set(cap));
        let result = run();
        super::TEST_CAP.with(|c| c.set(super::TEST_DEFAULT_CAP));
        result
    }
    /// A third component, larger than `FAR`, 14 m beyond it.
    const BIG: Face = [[20., 0., 0.], [22., 0., 0.], [20., 2., 0.]];

    #[test]
    fn a_declared_drop_within_geometric_error_is_counted() {
        let ok = at(1, || case(&[NEAR], &[NEAR, FAR], &[], 6., 1));
        assert_eq!(ok.unwrap(), vec![0, 1]);
        assert_eq!(
            case(&[NEAR], &[NEAR], &[], 0., 0).unwrap(),
            Vec::<u64>::new()
        );
    }

    #[test]
    fn uncounted_drops_and_understated_error_are_refused() {
        refused(
            at(1, || case(&[NEAR], &[NEAR, FAR], &[], 6., 0)),
            "coarseDrops",
        );
        refused(
            at(1, || case(&[NEAR], &[NEAR, FAR], &[], 6., 2)),
            "coarseDrops",
        );
        refused(
            at(1, || case(&[NEAR], &[NEAR, FAR], &[], 3.9, 1)),
            "coarseDrops",
        );
        refused(
            at(1, || case(&[NEAR, FAR], &[NEAR], &[], 6., 1)),
            "coarseDrops",
        );
    }

    /// Review r1 B-P1-1: children that fit the cap give no license for a coarse drop.
    #[test]
    fn a_drop_without_cap_pressure_is_refused() {
        refused(case(&[NEAR], &[NEAR, FAR], &[], 6., 1), "cap");
        assert_eq!(at(2, || case(&[NEAR], &[NEAR, FAR], &[], 6., 1)).ok(), None);
        assert!(at(1, || case(&[NEAR], &[NEAR, FAR], &[], 6., 1)).is_ok());
    }

    /// Review r1 B-P2-1: corners within G of the parent, edge midpoints about 4.3 m away
    /// (the kept faces are larger, so the drop is smallest first).
    #[test]
    fn interior_loss_beyond_geometric_error_is_refused() {
        let a: Face = [[0., 0., 0.], [1., 0., 0.], [0., 12., 0.]];
        let b: Face = [[10., 0., 0.], [11., 0., 0.], [10., 12., 0.]];
        let d: Face = [[0.25, 0.25, 0.25], [10.25, 0.25, 0.25], [10.25, 0.5, 0.25]];
        refused(
            at(1, || case(&[a, b], &[a, b, d], &[], 0.5, 1)),
            "geometricError",
        );
        assert!(at(1, || case(&[a, b], &[a, b, d], &[], 5., 1)).is_ok());
    }

    /// Drops go smallest first: no border-free component smaller than a dropped one stays.
    #[test]
    fn a_larger_drop_beside_a_kept_smaller_component_is_refused() {
        assert!(at(1, || case(&[NEAR, BIG], &[NEAR, FAR, BIG], &[], 20., 1)).is_ok());
        refused(
            at(1, || case(&[NEAR, FAR], &[NEAR, FAR, BIG], &[], 20., 1)),
            "smallest first",
        );
    }

    #[test]
    fn a_kept_component_touching_the_border_on_either_side_may_stay() {
        // FAR shares an edge with the sibling leaf, before or after the coarse tile in
        // preorder: it is a border component, so it may stay beside the larger dropped BIG.
        let touching = [[6., 0., 0.], [5., 0., 0.], [5., -1., 0.]];
        for first in [false, true] {
            let kept = at(1, || {
                ordered(&[NEAR, FAR], &[NEAR, FAR, BIG], &[touching], 20., 1, first)
            });
            assert_eq!(kept.unwrap(), vec![0, 1], "sibling first: {first}");
        }
    }

    #[test]
    fn a_dropped_component_touching_the_border_is_refused() {
        // The sibling leaf shares FAR's edge: FAR's keys are interface points the coarse
        // tile must keep, whatever it declares.
        let touching = [[5., 0., 0.], [6., 0., 0.], [5., -1., 0.]];
        let result = at(1, || case(&[NEAR], &[NEAR, FAR], &[touching], 6., 1));
        let e = result.unwrap_err().to_string();
        assert!(e.contains("interface"), "{e}");
    }
}
