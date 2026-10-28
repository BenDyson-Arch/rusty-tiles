//! D11 coarse drops (owner decision 2026-10-28). A coarse payload may lose whole
//! components of its immediate children's payloads, and declares how many in
//! `asset.extras.rustyTiles.droppedComponents`. They are recounted here from decoded
//! canonical keys alone: a child component (faces joined by shared keys) is dropped
//! when the parent holds none of its keys, since every coarse vertex is a source key.
//! Each dropped key must lie within the parent's geometricError of its surface, a lower
//! bound on the dropped surface's loss. A drop that touches the border loses shared
//! keys, which the retention checks refuse; nothing here relaxes them.
use super::{bad, read_point, sql, CanonicalPoint, Effort, Error, Store};
use rustc_hash::FxHashMap as HashMap;
use std::collections::HashSet;

/// A parent's geometricError covers a dropped key up to this relative rounding.
const TOLERANCE: f64 = 1e-6;
/// The distance grid has at most this many cells per axis.
const GRID: f64 = 32.;

struct Content {
    id: i64,
    leaf: bool,
    depth: usize,
    error: f64,
    declared: u64,
    digests: Vec<Vec<u8>>,
    children: Vec<(i64, bool)>,
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
                "SELECT t.id,t.parent,t.placeholder,t.leaf,t.error,
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
            let digests: Option<String> = row.get(5).map_err(sql)?;
            let content = match digests {
                None => None,
                Some(digests) => {
                    let digests: Vec<Vec<u8>> =
                        digests.split(',').map(|d| d.as_bytes().to_vec()).collect();
                    let declared: i64 = row.get(6).map_err(sql)?;
                    let ancestor = stack.iter_mut().rev().find_map(|f| f.content.as_mut());
                    match ancestor {
                        // A copy down a single-child chain stands for the same cell
                        // (single cover); its descendants are the original's children.
                        Some(a) if a.digests == digests => None,
                        ancestor => {
                            let leaf: bool = row.get(3).map_err(sql)?;
                            if let Some(a) = ancestor {
                                a.children.push((id, leaf));
                            }
                            Some(Content {
                                id,
                                leaf,
                                depth,
                                error: row.get::<_, Option<f64>>(4).map_err(sql)?.unwrap_or(0.),
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

    /// A delivered tile's faces by canonical key, from the leaf and coarse face tables
    /// already filled by the delivery replays: the recount decodes nothing again.
    fn faces(&self, tile: i64, leaf: bool) -> Result<Vec<[CanonicalPoint; 3]>, Error> {
        let mut query = self
            .connection
            .prepare_cached(if leaf {
                "SELECT corners FROM leaf_faces WHERE tile=?"
            } else {
                "SELECT corners FROM coarse_faces WHERE tile=?"
            })
            .map_err(sql)?;
        let mut rows = query.query([tile]).map_err(sql)?;
        let mut faces = Vec::new();
        while let Some(row) = rows.next().map_err(sql)? {
            let bytes: Vec<u8> = row.get(0).map_err(sql)?;
            if bytes.len() != 36 {
                return Err(Error::InvalidState("corrupt canonical face scratch".into()));
            }
            faces.push([
                read_point(&bytes[..12])?,
                read_point(&bytes[12..24])?,
                read_point(&bytes[24..])?,
            ]);
        }
        Ok(faces)
    }

    fn recount(
        &self,
        parent: &Content,
        dropped_by_depth: &mut Vec<u64>,
        effort: &Effort,
    ) -> Result<(), Error> {
        let faces = self.faces(parent.id, false)?;
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
        for &(child, leaf) in &parent.children {
            for face in self.faces(child, leaf)? {
                effort.charge(1)?;
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
            }
        }
        let mut present = vec![false; keys.len()];
        for (v, k) in keys.iter().enumerate() {
            if kept.contains(k) {
                let r = find(&mut root, v);
                present[r] = true;
            }
        }
        let mut dropped: HashMap<usize, Vec<CanonicalPoint>> = HashMap::default();
        for (v, k) in keys.iter().enumerate() {
            let r = find(&mut root, v);
            if !present[r] {
                dropped.entry(r).or_default().push(*k);
            }
        }
        let count = dropped.len() as u64;
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
        let grid = Grid::new(&faces, parent.error);
        for points in dropped.values() {
            for k in points {
                effort.charge(1)?;
                let p = point(*k);
                if !grid.within(p, &faces, effort)? {
                    return Err(bad(
                        "coarseDrops",
                        &format!(
                            "tile {} drops a component point {p:?} farther than its \
                             geometricError {} from its surface",
                            parent.id, parent.error
                        ),
                    ));
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

fn point(k: CanonicalPoint) -> [f64; 3] {
    k.map(|b| f64::from(f32::from_bits(b)))
}

/// The parent's faces binned by their box grown by its geometricError: a point within
/// that distance of a face lies in a cell the face was entered in.
struct Grid {
    lo: [f64; 3],
    size: f64,
    reach: f64,
    cells: HashMap<[i64; 3], Vec<u32>>,
}

impl Grid {
    fn new(faces: &[[CanonicalPoint; 3]], error: f64) -> Self {
        let reach = error * (1. + TOLERANCE) + TOLERANCE;
        let mut lo = [f64::MAX; 3];
        let mut hi = [f64::MIN; 3];
        for f in faces {
            for k in f {
                let p = point(*k);
                for a in 0..3 {
                    lo[a] = lo[a].min(p[a] - reach);
                    hi[a] = hi[a].max(p[a] + reach);
                }
            }
        }
        let extent = (0..3).map(|a| hi[a] - lo[a]).fold(0., f64::max);
        let size = (extent / GRID).max(reach).max(f64::MIN_POSITIVE);
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
                    grid.cell(p.map(|x| x - reach)),
                    grid.cell(p.map(|x| x + reach)),
                );
                for k in 0..3 {
                    a[k] = a[k].min(c[k]);
                    b[k] = b[k].max(d[k]);
                }
            }
            for x in a[0]..=b[0] {
                for y in a[1]..=b[1] {
                    for z in a[2]..=b[2] {
                        grid.cells.entry([x, y, z]).or_default().push(i as u32);
                    }
                }
            }
        }
        grid
    }
    fn cell(&self, p: [f64; 3]) -> [i64; 3] {
        std::array::from_fn(|a| ((p[a] - self.lo[a]) / self.size).floor() as i64)
    }
    fn within(
        &self,
        p: [f64; 3],
        faces: &[[CanonicalPoint; 3]],
        effort: &Effort,
    ) -> Result<bool, Error> {
        let Some(cell) = self.cells.get(&self.cell(p)) else {
            return Ok(false);
        };
        for &i in cell {
            effort.charge(1)?;
            if distance2(p, faces[i as usize].map(point)) <= self.reach * self.reach {
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
        let payloads = [
            replay_fixture(coarse),
            replay_fixture(leaf),
            replay_fixture(sibling),
        ];
        let mut s = store();
        s.decoded_limit = 1 << 24;
        for (id, parent) in [(0, None), (1, Some(0)), (2, Some(1)), (3, Some(0))] {
            s.tile(id, parent, IDENTITY)?;
        }
        s.kind(0, None, false, 1e3, Some(1.))?;
        s.kind(1, None, false, error, Some(1.))?;
        for (tile, rank) in [(1, 0), (2, 1), (3, 2)] {
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

    #[test]
    fn a_declared_drop_within_geometric_error_is_counted() {
        assert_eq!(case(&[NEAR], &[NEAR, FAR], &[], 6., 1).unwrap(), vec![0, 1]);
        assert_eq!(
            case(&[NEAR], &[NEAR], &[], 0., 0).unwrap(),
            Vec::<u64>::new()
        );
    }

    #[test]
    fn uncounted_drops_and_understated_error_are_refused() {
        refused(case(&[NEAR], &[NEAR, FAR], &[], 6., 0), "coarseDrops");
        refused(case(&[NEAR], &[NEAR, FAR], &[], 6., 2), "coarseDrops");
        refused(case(&[NEAR], &[NEAR, FAR], &[], 3.9, 1), "coarseDrops");
        refused(case(&[NEAR, FAR], &[NEAR], &[], 6., 1), "coarseDrops");
    }

    #[test]
    fn a_dropped_component_touching_the_border_is_refused() {
        // The sibling leaf shares FAR's edge: FAR's keys are interface points the coarse
        // tile must keep, whatever it declares.
        let touching = [[5., 0., 0.], [6., 0., 0.], [5., -1., 0.]];
        let result = case(&[NEAR], &[NEAR, FAR], &[touching], 6., 1);
        let e = result.unwrap_err().to_string();
        assert!(e.contains("interface"), "{e}");
    }
}
