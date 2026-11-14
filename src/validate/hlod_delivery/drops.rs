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
//!
//! D1/D11 amendment A2: a tile declaring last-resort stage 3 (N3) may drop border-free
//! components at any size, measured into G. It is exempt from cap pressure and size order;
//! the declaration count and the distance bound still hold. Its declaration has been
//! verified by the time the recount runs (`lastResort`: shape, fold recount, leaf refusal).
//! N3 certifies every dropped triangle continuously at G, without child-G slack.
//! Stages 0-2 and undeclared tiles keep the sampled rule described above.
mod certificate;

use super::{bad, point_bytes, read_point, sql, CanonicalPoint, Effort, Error, Store};
use std::collections::{HashMap, HashSet};

/// A parent's geometricError covers a dropped key up to this relative rounding.
const TOLERANCE: f64 = 1e-6;
/// The distance grid has at most this many cells per axis.
const GRID: f64 = 32.;
/// Units per grid cell insert and per distance query or face test: each class's worst
/// measured CPU per insert or test over τ, rounded up on both cores (PR #309 review 3:
/// inserts 18.4 ns P-core and 53 ns E-core at load 24, tests 18.4 and 23.8 ns, against τ 11.53
/// and 15.3 ns). A few stored faces can drive any number of either, so neither is amortized
/// against row charges.
const CELL_UNITS: usize = 4;
const TEST_UNITS: usize = 2;
/// Units per child face the recount reads: its share of the `leaf_faces_tile` build and the
/// recount's pass over it (union-find, components), which no other charge covers. Up to
/// 0.96 µs and 0.52 µs per face on an E-core at 1–64 M faces, 0.66 µs and 0.33 µs on a
/// P-core (PR #309 review 3).
const FACE_READ_UNITS: usize = 97;
/// The stage-3 continuous certificate (#313, filtered by #317), per item over τ, rounded up on
/// the slower core (PR #309 review 4): a region's setup (centroid, radius, cell lookup; 84 ns
/// P-core, 151 ns E-core); a distance test's fast path (box reject or f64 filter; up to 99 ns
/// on both cores, zero-width corners included, review 5); a test the filter leaves undecided,
/// which runs the interval distance on top (372 ns and 741 ns since review 5 kept subnormals
/// out of the interval arithmetic; 795 units, set at the former 12.3 µs, stays as an upper
/// bound); and a failed region's subdivision (three midpoints and four stacked regions, fewer
/// interval operations than a setup, so charged as one).
const CERT_REGION_UNITS: usize = 10;
const CERT_FAST_UNITS: usize = 9;
const CERT_SLOW_UNITS: usize = 795;
const CERT_SPLIT_UNITS: usize = CERT_REGION_UNITS;
/// The recount's leaf faces by tile: `leaf_faces_tile` covers the query (the table's key
/// holds the corners), so a tile's faces are one index range read in order.
pub(super) const LEAF_FACES_TILE: &str =
    "CREATE INDEX IF NOT EXISTS leaf_faces_tile ON leaf_faces(tile,n)";
pub(super) const LEAF_FACES_OF: &str = "SELECT corners,n FROM leaf_faces WHERE tile=?";

struct Content {
    id: i64,
    last: i64,
    leaf: bool,
    depth: usize,
    error: f64,
    declared: u64,
    /// Declared last-resort stage 3 (N3).
    n3: bool,
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
            .execute_batch(LEAF_FACES_TILE)
            .map_err(sql)?;
        let mut query = self
            .connection
            .prepare(
                "SELECT t.id,t.parent,t.placeholder,t.leaf,t.error,t.last,
                 (SELECT group_concat(hex(p.digest)) FROM (SELECT DISTINCT p.digest FROM refs r
                    JOIN payloads p ON p.rank=r.rank WHERE r.tile=t.id ORDER BY p.digest) p),
                 (SELECT sum(p.dropped) FROM refs r JOIN payloads p ON p.rank=r.rank WHERE r.tile=t.id),
                 (SELECT min(p.last_stage) FROM refs r JOIN payloads p ON p.rank=r.rank
                    WHERE r.tile=t.id AND p.last_stage>0)
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
            // The tile's digests, drops and last-resort stage: seeks by tile.
            effort.charge(super::SEEK_UNITS)?;
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
                                // `last_stage` stores stage + 1 (0: undeclared).
                                n3: row.get::<_, Option<i64>>(8).map_err(sql)? == Some(4),
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
                LEAF_FACES_OF
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
        // Union-find over the children's faces by canonical key. The key map is keyed per run
        // (SipHash, random key), so crafted coordinates cannot collide it; it only assigns ids
        // in read order and is never iterated.
        let mut index: HashMap<CanonicalPoint, usize> = HashMap::new();
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
                effort.charge(FACE_READ_UNITS)?;
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
        // Every component's members (by root, in root order) and bounding-box diagonal.
        let mut members = vec![Vec::new(); keys.len()];
        for v in 0..keys.len() {
            let r = find(&mut root, v);
            members[r].push(v);
        }
        let components: Vec<(usize, Vec<usize>)> = members
            .into_iter()
            .enumerate()
            .filter(|(_, m)| !m.is_empty())
            .collect();
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
        let count = components.iter().filter(|(r, _)| !present[*r]).count() as u64;
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
        // component (border contact is a key held by a leaf outside this tile). N3 is
        // exempt (A2); the distance bound below still applies.
        let largest = components
            .iter()
            .filter(|(r, _)| !present[*r])
            .map(|(_, m)| diagonal(m))
            .fold(f64::MIN, f64::max);
        if largest >= child_g && !parent.n3 {
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
            effort.charge(TEST_UNITS)?;
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
            if parent.n3 {
                if !grid.covers(t, &faces, effort)? {
                    return Err(bad(
                        "coarseDrops",
                        &format!(
                            "tile {}: cannot certify dropped surface within geometricError {}",
                            parent.id, parent.error
                        ),
                    ));
                }
                continue;
            }
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
/// charged to the validation budget, `CELL_UNITS` each. Each axis has at most `GRID` cells
/// of its own size (never under the widest reach), so a long thin parent is not binned into a
/// few crowded cells along its short axes.
struct Grid {
    lo: [f64; 3],
    size: [f64; 3],
    reach: f64,
    /// Cells per axis, and every cell's faces in one dense array (x fastest): no hashing,
    /// so no input can crowd a bucket.
    dims: [usize; 3],
    cells: Vec<Vec<u32>>,
    /// The face that answered the last point query; `within` tries it first. Queries come in
    /// runs from one dropped face or component, so it usually answers the next one too.
    last: std::cell::Cell<Option<u32>>,
    /// The face that certified last, with its decoded corners. The certificate reuses them only
    /// at that face's own cell-list position, so its charged work never depends on the cache
    /// (#317).
    certified: std::cell::Cell<Option<(u32, [[f64; 3]; 3])>>,
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
        let size =
            std::array::from_fn(|a| ((hi[a] - lo[a]) / GRID).max(widest).max(f64::MIN_POSITIVE));
        let mut grid = Self {
            lo,
            size,
            reach,
            dims: [0; 3],
            cells: Vec::new(),
            last: std::cell::Cell::new(None),
            certified: std::cell::Cell::new(None),
        };
        // Every face's grown box ends at or below `hi`, so its cells lie within `cell(hi)`.
        let top = grid.cell(hi);
        grid.dims = std::array::from_fn(|a| usize::try_from(top[a]).unwrap_or(0) + 1);
        let count = grid.dims.iter().product::<usize>();
        effort.charge(count)?;
        grid.cells = vec![Vec::new(); count];
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
            effort.charge((cells as usize).saturating_mul(CELL_UNITS))?;
            for x in a[0]..=b[0] {
                for y in a[1]..=b[1] {
                    for z in a[2]..=b[2] {
                        let slot = grid.slot([x, y, z]).ok_or_else(|| {
                            Error::InvalidState("distance grid cell out of range".into())
                        })?;
                        grid.cells[slot].push(i as u32);
                    }
                }
            }
        }
        Ok(grid)
    }
    fn cell(&self, p: [f64; 3]) -> [i64; 3] {
        std::array::from_fn(|a| ((p[a] - self.lo[a]) / self.size[a]).floor() as i64)
    }
    /// A cell's index in `cells`, or none outside the grid (where no face was entered).
    fn slot(&self, c: [i64; 3]) -> Option<usize> {
        let mut slot = 0;
        for a in (0..3).rev() {
            let k = usize::try_from(c[a]).ok().filter(|&k| k < self.dims[a])?;
            slot = slot * self.dims[a] + k;
        }
        Some(slot)
    }
    fn faces_at(&self, p: [f64; 3]) -> Option<&Vec<u32>> {
        self.slot(self.cell(p)).map(|s| &self.cells[s])
    }
    /// Face `i`'s corners: the cached decode if `i` is the cached face, else decoded. The cache
    /// is read once per scan (`cached`): it changes only when the scan ends.
    fn face(
        cached: Option<(u32, [[f64; 3]; 3])>,
        i: u32,
        faces: &[[CanonicalPoint; 3]],
    ) -> [[f64; 3]; 3] {
        match cached {
            Some((last, face)) if last == i => face,
            _ => faces[i as usize].map(point),
        }
    }
    /// Continuous certificate at G, with no child-G slack. Distance to a closed
    /// surface is 1-Lipschitz; the centroid's farthest corner bounds its covering
    /// radius. Also, distance to ONE convex retained triangle is convex, so its
    /// three corner distances bound the whole region (useful at equality).
    /// Four-way subdivision uses a depth-first stack of at most 1 + 3*32 regions.
    /// Each region's setup, each distance test (fast path, and the interval path on top when the
    /// filter leaves it undecided) and each subdivision is charged at its measured cost
    /// (`CERT_*_UNITS`). Verdicts depend on the archive and budget, never on the coordinate
    /// cache. Deterministic pruning may lower the budget required for an accept.
    fn covers(
        &self,
        triangle: [[f64; 3]; 3],
        faces: &[[CanonicalPoint; 3]],
        effort: &Effort,
    ) -> Result<bool, Error> {
        use certificate::{ball_fits, filtered_upper, radius2, within, Interval};
        let mut stack = vec![(triangle.map(|p| p.map(Interval::point)), 0)];
        while let Some((t, depth)) = stack.pop() {
            effort.charge(CERT_REGION_UNITS)?;
            let center =
                std::array::from_fn(|k| t[0][k].add(t[1][k]).add(t[2][k]).div(Interval::point(3.)));
            let radius = radius2(t, center);
            let ball = (self.reach - radius.sqrt().next_up()).next_down();
            // Each test is charged its fast path first; one that falls to the slow path is
            // charged that path's extra when the slow counter moves (one charge's overshoot).
            let charge_distance = || -> Result<(), Error> {
                effort.charge(CERT_FAST_UNITS)?;
                effort
                    .certificate_fast
                    .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                Ok(())
            };
            let distance = |p, face, ball| -> Result<f64, Error> {
                use std::sync::atomic::Ordering::Relaxed;
                charge_distance()?;
                let slow = effort.certificate_slow.load(Relaxed);
                let d = filtered_upper(p, face, self.reach, ball, &effort.certificate_slow);
                if effort.certificate_slow.load(Relaxed) != slow {
                    effort.charge(CERT_SLOW_UNITS)?;
                }
                Ok(d)
            };
            let mut certified = false;
            let mut center_near = false;
            if let Some(cell) = self.faces_at(center.map(Interval::middle)) {
                let cached = self.certified.get();
                for &i in cell {
                    let face = Self::face(cached, i, faces);
                    // A separated centroid rules out BOTH certificates: if all
                    // corners were within reach of this convex face, their
                    // centroid would be too. Charge the executed box query;
                    // omitted corner queries consume no work. This deterministic
                    // reduction may admit a smaller budget, never refuse an old
                    // accept. Cache state changes neither queries nor charges.
                    if certificate::outside_box(center, face, self.reach) {
                        charge_distance()?;
                        continue;
                    }
                    let d = distance(center, face, ball)?;
                    center_near |= within(d, self.reach);
                    if ball_fits(d, radius, self.reach) {
                        self.certified.set(Some((i, face)));
                        certified = true;
                        break;
                    }
                    let mut corners_near = true;
                    for p in t {
                        if !within(distance(p, face, self.reach)?, self.reach) {
                            corners_near = false;
                            break;
                        }
                    }
                    if corners_near {
                        self.certified.set(Some((i, face)));
                        certified = true;
                        break;
                    }
                }
            }
            if certified {
                continue;
            }
            if !center_near || depth == 32 {
                return Ok(false);
            }
            effort.charge(CERT_SPLIT_UNITS)?;
            let [a, b, c] = t;
            let mid = |p: certificate::Point, q: certificate::Point| {
                std::array::from_fn(|k| p[k].add(q[k]).div(Interval::point(2.)))
            };
            let (ab, bc, ca) = (mid(a, b), mid(b, c), mid(c, a));
            for sub in [[a, ab, ca], [ab, b, bc], [ca, bc, c], [ab, bc, ca]] {
                stack.push((sub, depth + 1));
            }
        }
        Ok(true)
    }

    /// Whether `p` lies within the parent's reach plus `slack` of a face. The last answering face
    /// is tried first (charged as a test): any face within reach answers, and one within reach
    /// of `p` was entered in `p`'s cell, so the answer is the cell scan's. The charged work then
    /// depends on the cache, which is a pure function of the archive (the queries run in a fixed
    /// order), so the verdict stays deterministic and every test is charged.
    fn within(
        &self,
        p: [f64; 3],
        slack: f64,
        faces: &[[CanonicalPoint; 3]],
        effort: &Effort,
    ) -> Result<bool, Error> {
        let reach = self.reach + slack;
        let near = |i: u32| distance2(p, faces[i as usize].map(point)) <= reach * reach;
        if let Some(i) = self.last.get() {
            effort.charge(TEST_UNITS)?;
            if near(i) {
                return Ok(true);
            }
        }
        let Some(cell) = self.faces_at(p) else {
            return Ok(false);
        };
        for &i in cell {
            effort.charge(TEST_UNITS)?;
            if near(i) {
                self.last.set(Some(i));
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
        s.finish(budget(), 0, None, &mut |rank| {
            Ok(payloads[rank as usize].clone())
        })?;
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
    /// Grid inserts and distance tests are charged at their measured weights (PR #309 review
    /// 3): `CELL_UNITS` per cell a face is entered in, `TEST_UNITS` per query and per face
    /// tested.
    #[test]
    fn grid_inserts_and_distance_tests_are_charged_at_their_weights() {
        use super::super::{Budget, Effort};
        use std::sync::{atomic::AtomicU64, atomic::Ordering, Arc};
        let key = |p: Point| p.map(|x| (x as f32).to_bits());
        let faces: Vec<_> = (0..40)
            .map(|i| {
                let t = 0.1 + 0.3 * f64::from(i) / 40.;
                [[0., t, 0.], [1., 1. - t, 1.], [t, 1., 0.5]].map(key)
            })
            .collect();
        let effort = Effort {
            budget: Budget {
                value: 1 << 40,
                ..budget()
            },
            base: 0,
            used: Arc::new(AtomicU64::new(0)),
            ticks: Default::default(),
            certificate_fast: AtomicU64::new(0),
            certificate_slow: AtomicU64::new(0),
        };
        let used = || effort.used.load(Ordering::Relaxed) as usize;
        let grid = super::Grid::new(&faces, 1e-3, 0.01, &effort).unwrap();
        let cells: usize = grid.cells.iter().map(Vec::len).sum();
        assert!(cells > 40 * 1000, "{cells}");
        assert_eq!(used(), super::CELL_UNITS * cells + grid.cells.len());
        let p = super::point(faces[39][0]);
        let reach = grid.reach + 0.01;
        let tested = grid
            .faces_at(p)
            .unwrap()
            .iter()
            .position(|&i| {
                super::distance2(p, faces[i as usize].map(super::point)) <= reach * reach
            })
            .unwrap()
            + 1;
        let before = used();
        assert!(grid.within(p, 0.01, &faces, &effort).unwrap());
        assert_eq!(used() - before, super::TEST_UNITS * tested);
        // The last answer is tried first: one test.
        let before = used();
        assert!(grid.within(p, 0.01, &faces, &effort).unwrap());
        assert_eq!(used() - before, super::TEST_UNITS);
    }

    #[test]
    fn continuous_certificate_subdivides_and_charges_work() {
        use super::{Effort, Grid};
        use std::sync::{atomic::AtomicU64, Arc};
        let effort = Effort {
            certificate_fast: AtomicU64::new(0),
            certificate_slow: AtomicU64::new(0),
            budget: budget(),
            base: 0,
            used: Arc::new(AtomicU64::new(0)),
            ticks: Default::default(),
        };
        let a = [0., 0., 0.];
        let b = [2., 0., 0.];
        let c = [0., 2., 0.];
        let faces = [
            [a, [1., 0., 0.], [0., 1., 0.]],
            [[1., 0., 0.], b, [1., 1., 0.]],
            [[0., 1., 0.], [1., 1., 0.], c],
            [[1., 0., 0.], [1., 1., 0.], [0., 1., 0.]],
        ]
        .map(|t| t.map(|p| p.map(f32::to_bits)));
        let grid = Grid::new(&faces, 0., 0., &effort).unwrap();
        // No single retained face covers the whole triangle; one subdivision does.
        assert!(grid
            .covers([a, b, c].map(|p| p.map(f64::from)), &faces, &effort)
            .unwrap());
        let mut tiny = budget();
        tiny.value = 1;
        let exhausted = Effort {
            certificate_fast: AtomicU64::new(0),
            certificate_slow: AtomicU64::new(0),
            budget: tiny,
            base: 0,
            used: Arc::new(AtomicU64::new(0)),
            ticks: Default::default(),
        };
        assert!(grid
            .covers([a, b, c].map(|p| p.map(f64::from)), &faces, &exhausted)
            .is_err());
        assert!(!grid
            .covers([[0., 0., 1.], [2., 0., 1.], [0., 2., 1.]], &faces, &effort)
            .unwrap());
    }

    #[test]
    fn cached_faces_keep_certificate_work_and_honest_point_queries() {
        use super::super::tests::effort_for_tests;
        use super::{distance2, point, Grid};
        use std::sync::atomic::Ordering::Relaxed;

        let triangle = [[0.1, 0.1, 0.], [0.2, 0.1, 0.], [0.1, 0.2, 0.]];
        let p = [0.125, 0.125, 0.];
        // The first face certifies the centroid ball in one distance test.
        // The second is outside the cell, in-cell but too far, or certifying
        // only through three corner tests. Prepending it changes charged work
        // even in the last case, where both faces certify the region.
        for (other, in_cell) in [
            (FAR, false),
            (NEAR.map(|v| [v[0], v[1], 0.25]), true),
            (NEAR.map(|v| [v[0], v[1], 0.125]), true),
        ] {
            let faces = [NEAR, other].map(|t| t.map(|p| p.map(|v| (v as f32).to_bits())));
            let grid = Grid::new(&faces, 0.18, 0., &effort_for_tests()).unwrap();
            assert_eq!(grid.faces_at(p).unwrap().contains(&1), in_cell);
            let reach2 = grid.reach * grid.reach;
            let near = |i: u32| distance2(p, faces[i as usize].map(point)) <= reach2;
            let cell = grid.faces_at(p).unwrap().clone();
            for remaining in 0..=super::CERT_REGION_UNITS as u64 + 32 {
                // The certificate: the same verdict and work whatever the cache holds (#317).
                let mut cold = None;
                for cached in [None, Some(0), Some(1)] {
                    grid.certified
                        .set(cached.map(|i| (i, faces[i as usize].map(point))));
                    let mut effort = effort_for_tests();
                    effort.base = effort.budget.value - remaining;
                    let result = grid.covers(triangle, &faces, &effort);
                    let needed = (super::CERT_REGION_UNITS + super::CERT_FAST_UNITS) as u64;
                    assert_eq!(result.as_ref().ok(), (remaining >= needed).then_some(&true));
                    let observed = (
                        result.map_err(|e| e.to_string()),
                        effort.used.load(Relaxed),
                        effort.certificate_fast.load(Relaxed),
                        effort.certificate_slow.load(Relaxed),
                    );
                    match &cold {
                        Some(expected) => assert_eq!(&observed, expected),
                        None => cold = Some(observed),
                    }
                }
                // A point query: its work depends on the cache (tried first), so it is a pure
                // function of the cache state, and every distance test it runs is charged.
                for cached in [None, Some(0), Some(1)] {
                    let tests = match cached {
                        Some(i) if near(i) => 1,
                        Some(_) => 1 + cell.iter().position(|&i| near(i)).unwrap() + 1,
                        None => cell.iter().position(|&i| near(i)).unwrap() + 1,
                    } as u64;
                    let needed = super::TEST_UNITS as u64 * tests;
                    let mut runs = Vec::new();
                    for _ in 0..2 {
                        grid.last.set(cached);
                        let mut effort = effort_for_tests();
                        effort.base = effort.budget.value - remaining;
                        let result = grid.within(p, 0., &faces, &effort);
                        assert_eq!(result.as_ref().ok(), (remaining >= needed).then_some(&true));
                        if remaining >= needed {
                            assert_eq!(effort.used.load(Relaxed), needed);
                        }
                        runs.push((result.map_err(|e| e.to_string()), effort.used.load(Relaxed)));
                    }
                    assert_eq!(runs[0], runs[1]);
                }
            }
        }
    }

    #[test]
    fn pruning_saves_work_with_cache_independent_budget_verdicts() {
        use super::super::tests::effort_for_tests;
        use super::{point, Grid};
        use std::sync::atomic::Ordering::Relaxed;
        let triangle = [[0.1, 0.1, 0.], [0.2, 0.1, 0.], [0.1, 0.2, 0.]];
        let faces = [NEAR.map(|v| [v[0], v[1], 0.25]), NEAR]
            .map(|t| t.map(|p| p.map(|v| (v as f32).to_bits())));
        let grid = Grid::new(&faces, 0.18, 0., &effort_for_tests()).unwrap();
        assert_eq!(grid.faces_at([0.125, 0.125, 0.]).unwrap(), &vec![0, 1]);
        let needed = (super::CERT_REGION_UNITS + 2 * super::CERT_FAST_UNITS) as u64;
        // Region + separated centroid + certifying centroid: one region and two fast
        // tests, not three. The old scan also queried a corner of the impossible first face.
        // This budget improvement is intentional: executed work is charged,
        // and the archive plus budget determines the verdict, not cache state.
        for remaining in 0..=needed + 2 {
            let mut cold = None;
            for cached in [None, Some(0), Some(1)] {
                grid.certified
                    .set(cached.map(|i| (i, faces[i as usize].map(point))));
                let mut effort = effort_for_tests();
                effort.base = effort.budget.value - remaining;
                let result = grid.covers(triangle, &faces, &effort);
                assert_eq!(result.as_ref().ok(), (remaining >= needed).then_some(&true));
                if remaining >= needed {
                    assert_eq!(effort.used.load(Relaxed), needed);
                    assert_eq!(effort.certificate_fast.load(Relaxed), 2);
                    assert_eq!(effort.certificate_slow.load(Relaxed), 0);
                }
                let observed = (result.map_err(|e| e.to_string()), effort.used.load(Relaxed));
                if let Some(expected) = &cold {
                    assert_eq!(&observed, expected);
                } else {
                    cold = Some(observed);
                }
            }
        }
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
