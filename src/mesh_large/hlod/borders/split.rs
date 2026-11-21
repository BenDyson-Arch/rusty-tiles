//! D (keep a stuck border vertex): a border vertex a cell cannot remove onto its depth-d
//! chord is kept instead, splitting the one stored segment there. Every owner of the
//! segment then publishes the same pieces, so seams stay exact.
//!
//! A kept vertex is a source arc vertex strictly between the segment's endpoints and
//! retained one level finer (a child contract point), so retained sets stay nested and no
//! coarser chord moves. Each piece is certified over its own subarc as `curve::shortcuts`
//! certifies a chord, and is admitted only within the tolerance of the segment's ladder
//! step (the bound the validators and the seam budget assume of every chord). Splits are
//! values (`Overlay`) the walk owns and passes to contract and mapping lookups; the stored
//! border pass never changes.
use super::{
    curve::{self, state},
    disk::NONE,
    Borders,
};
use crate::{
    mesh_large::hlod::{interfaces::Key, store::TileKey},
    JobError,
};
use std::collections::{BTreeMap, BTreeSet};

/// Why a stuck vertex cannot be kept (`Borders::locate`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) enum Refused {
    /// Not a source vertex, or retained at this depth already.
    NotOnChord,
    /// No segment of this cell's contract holds it strictly inside its arc.
    OffArc,
    /// Not retained one level finer (it would not nest).
    Unretained,
    /// The segment has an owner in a pass-through chain, whose publication depth the
    /// border pass fixed before the walk.
    Chain,
}

/// A stuck vertex located on one of the cell's segments: the segment and the vertex's
/// ordinal on its chain.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct Site {
    pub segment: u64,
    pub ordinal: u64,
}

/// Where a stuck vertex can be kept, or why not (`Borders::locate`).
pub(crate) type Located = Result<Site, Refused>;

/// One split segment: its pieces as segment rows (as stored segments are), the kept
/// vertices' keys, and the depth-level map of every arc vertex strictly inside it.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Split {
    pieces: Vec<[u64; 12]>,
    maps: Vec<(u64, [u64; 3])>,
    pub kept: Vec<Key>,
    /// The largest piece bound (raises coarser relieved chords' finer displacement).
    bound: f64,
}

/// Every accepted split, by depth and segment, and the finer displacement each coarser
/// relieved chord over a split arc must cover. Deterministic: a function of the kept
/// sites alone (`Borders::split`).
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct Overlay {
    splits: BTreeMap<(u32, u64), Split>,
    maps: BTreeMap<(u32, u64), [u64; 3]>,
    /// (depth, lesser endpoint, greater endpoint) of a coarser chord: the largest bound of
    /// a finer split on its arc.
    finer: BTreeMap<(u32, u64, u64), f64>,
}

impl Overlay {
    pub(crate) fn is_empty(&self) -> bool {
        self.splits.is_empty()
    }
    /// Kept vertices, over every split, in (depth, segment) order.
    pub(crate) fn kept(&self) -> impl Iterator<Item = (u32, Key)> + '_ {
        self.splits
            .iter()
            .flat_map(|(&(d, _), s)| s.kept.iter().map(move |&k| (d, k)))
    }
    /// The split of one segment at `depth`, if any.
    pub(crate) fn split_of(&self, depth: u32, segment: u64) -> Option<&Split> {
        self.splits.get(&(depth, segment))
    }
    /// The splits at `depth`, by segment.
    pub(crate) fn splits_at(&self, depth: u32) -> Vec<(u64, &Split)> {
        self.splits
            .range((depth, 0)..=(depth, u64::MAX))
            .map(|(&(_, s), split)| (s, split))
            .collect()
    }
    pub(super) fn pieces(&self, depth: u32, segment: u64) -> Option<&[[u64; 12]]> {
        self.splits.get(&(depth, segment)).map(|s| &s.pieces[..])
    }
    pub(super) fn map(&self, depth: u32, vertex: u64) -> Option<[u64; 3]> {
        self.maps.get(&(depth, vertex)).copied()
    }
    pub(super) fn finer(&self, depth: u32, a: u64, b: u64) -> f64 {
        self.finer
            .get(&(depth, a.min(b), a.max(b)))
            .copied()
            .unwrap_or(0.)
    }
    /// Add `split` for `segment` at `depth` (replacing an earlier split of it).
    fn insert(&mut self, depth: u32, segment: u64, split: Split, coarser: Vec<(u32, u64, u64)>) {
        if let Some(old) = self.splits.remove(&(depth, segment)) {
            for (v, _) in old.maps {
                self.maps.remove(&(depth, v));
            }
        }
        for &(v, m) in &split.maps {
            self.maps.insert((depth, v), m);
        }
        for c in coarser {
            let f = self.finer.entry(c).or_insert(0.);
            *f = f.max(split.bound);
        }
        self.splits.insert((depth, segment), split);
    }
    /// The union of two overlays (`other` wins on a segment both split).
    pub(crate) fn merge(&mut self, other: &Overlay) {
        for (&(d, s), split) in &other.splits {
            self.splits.insert((d, s), split.clone());
        }
        for (&k, &m) in &other.maps {
            self.maps.insert(k, m);
        }
        for (&k, &f) in &other.finer {
            let e = self.finer.entry(k).or_insert(0.);
            *e = e.max(f);
        }
    }
}

impl Borders {
    /// The cell's own segments (stored references), as segment ids.
    pub(crate) fn own_segments(&self, key: TileKey) -> Result<Vec<u64>, JobError> {
        let n = self.source.nodes.get(self.source.node(key)?)?;
        let mut out = Vec::with_capacity(usize::try_from(n[7]).unwrap_or(0));
        let mut at = n[6];
        while at != NONE {
            let link = self.refs.get(at)?;
            out.push(link[1]);
            at = link[0];
        }
        Ok(out)
    }

    /// The other coarse owners, at `key`'s depth, of `key`'s own chords (segments with two
    /// endpoints), by node id in ascending order: the cells whose kept vertices can split a
    /// chord `key` publishes, and whose own `key` can split. Leaves are exact (no chord of
    /// theirs has an interior point to keep).
    pub(crate) fn co_owners(&self, key: TileKey) -> Result<Vec<u64>, JobError> {
        let me = self.source.node(key)?;
        let mut out = BTreeSet::new();
        for id in self.own_segments(key)? {
            self.chord_owners(id, key.depth, me, &mut out)?;
        }
        Ok(out.into_iter().collect())
    }

    /// Whether `segment`, one of `key`'s own, has a coarse owner under another parent cell
    /// that comes before `key` in the walk. D keeps no vertex on such a chord: that owner may
    /// already be final, and waiting for it to build again would stall the walk.
    pub(crate) fn has_earlier_owner_elsewhere(
        &self,
        key: TileKey,
        segment: u64,
    ) -> Result<bool, JobError> {
        let me = self.source.node(key)?;
        let mut owners = BTreeSet::new();
        self.chord_owners(segment, key.depth, me, &mut owners)?;
        let parent = self.parent(me)?;
        for o in owners.range(..me) {
            if self.parent(*o)? != parent {
                return Ok(true);
            }
        }
        Ok(false)
    }

    /// `key`'s chord co-owners (`co_owners`) split by where D waits for them: its siblings
    /// (same parent cell), whose final splits it settles with before its parent builds; and
    /// owners under another parent that come before it in the walk, which must be final
    /// before it builds. Owners under another parent that come later wait for it instead.
    pub(crate) fn co_owner_groups(&self, key: TileKey) -> Result<(Vec<u64>, Vec<u64>), JobError> {
        let me = self.source.node(key)?;
        let parent = self.parent(me)?;
        let (mut siblings, mut earlier) = (Vec::new(), Vec::new());
        for o in self.co_owners(key)? {
            if self.parent(o)? == parent {
                siblings.push(o);
            } else if o < me {
                earlier.push(o);
            }
        }
        Ok((siblings, earlier))
    }

    /// A node's parent cell (the root is its own).
    fn parent(&self, node: u64) -> Result<u64, JobError> {
        let row = self.source.nodes.get(node)?;
        let depth = usize::try_from(row[2]).map_err(|_| state("a node depth exceeds usize"))?;
        if depth == 0 {
            return Ok(node);
        }
        Ok(self.source.leaves.get(row[3])?[depth])
    }

    /// The coarse owners at `depth` other than `me` of segment `id`, when it is a chord.
    fn chord_owners(
        &self,
        id: u64,
        depth: u32,
        me: u64,
        out: &mut BTreeSet<u64>,
    ) -> Result<(), JobError> {
        let s = self.segments.get(id)?;
        if s[0] == s[1] || s[5] == s[4] + 1 {
            return Ok(());
        }
        let mut at = s[8];
        while at < s[9] {
            let owner = self.source.owner(at, depth)?;
            let n = self.source.nodes.get(owner)?;
            if owner != me && n[5] & 1 == 0 {
                out.insert(owner);
            }
            at = super::graph::next_owner(&self.source.owners, at, s[9], n[4])?;
        }
        Ok(())
    }

    /// The source vertices of a stored segment's arc, from its first to its last ordinal,
    /// walked from the chain's origin along its first edge through vertices of live degree
    /// two at its depth (every chain-interior vertex). With each vertex, the edge into it.
    fn arc(&self, s: &[u64; 12]) -> Result<Vec<u64>, JobError> {
        let depth = narrow(s[7])?;
        let (first, last) = (s[4], s[5]);
        let mut out = Vec::with_capacity(usize::try_from(last - first + 1).unwrap_or(0));
        let (mut at, mut edge) = (s[3], s[2]);
        if first == 0 {
            out.push(at);
        }
        for i in 1..=last {
            let e = self.source.edges.get(edge)?;
            let next = if e[0] == at { e[1] } else { e[0] };
            if i >= first {
                out.push(next);
            }
            if i == last {
                break;
            }
            let mut found = NONE;
            let mut f = self.source.vertices.get(next)?[5];
            while f != NONE {
                let ef = self.source.edges.get(f)?;
                if f != edge && self.source.external(ef[2], ef[3], depth)? {
                    if found != NONE {
                        return Err(state(
                            "a chain-interior vertex has more than two live edges",
                        ));
                    }
                    found = f;
                }
                f = if ef[0] == next { ef[4] } else { ef[5] };
            }
            if found == NONE {
                return Err(state("a chain ends before its segment"));
            }
            (at, edge) = (next, found);
        }
        if out.first() != Some(&s[0]) || out.last() != Some(&s[1]) {
            return Err(state("a segment's arc does not join its endpoints"));
        }
        Ok(out)
    }

    /// Whether any owner of the segment is in a pass-through chain: it publishes at
    /// another depth, or the next deeper cell publishes at it.
    fn in_chain(&self, s: &[u64; 12]) -> Result<bool, JobError> {
        let depth = narrow(s[7])?;
        let mut at = s[8];
        while at < s[9] {
            let owner = self.source.owner(at, depth)?;
            let n = self.source.nodes.get(owner)?;
            if n[5] & 1 == 0 {
                let key = TileKey {
                    first: n[0],
                    end: n[1],
                    depth,
                    leaf_begin: 0,
                    leaf_end: 0,
                };
                if self.publication_depth(key)? != depth
                    || (depth < self.source.depth
                        && self
                            .source
                            .node(TileKey {
                                depth: depth + 1,
                                ..key
                            })
                            .is_ok()
                        && self.can_collapse(n[0], n[1], depth)?)
                {
                    return Ok(true);
                }
            }
            at = super::graph::next_owner(&self.source.owners, at, s[9], n[4])?;
        }
        Ok(false)
    }

    /// Locate each stuck key on one of `key`'s own segments: the segment holding it strictly
    /// inside its arc, and its chain ordinal; or why it cannot be kept.
    pub(crate) fn locate(
        &self,
        key: TileKey,
        stuck: &[Key],
    ) -> Result<Vec<(Key, Located)>, JobError> {
        let depth = key.depth;
        let n = self.source.vertices.len;
        let own = self.own_segments(key)?;
        let mut out = Vec::with_capacity(stuck.len());
        for &k in stuck {
            let Some(v) = self.source.vertex(k)? else {
                out.push((k, Err(Refused::NotOnChord)));
                continue;
            };
            let m = self.maps.get(u64::from(depth) * n + v)?;
            if m[0] == m[1] {
                out.push((k, Err(Refused::NotOnChord)));
                continue;
            }
            if depth < self.source.depth {
                let finer = self.maps.get(u64::from(depth + 1) * n + v)?;
                if finer[0] != finer[1] {
                    out.push((k, Err(Refused::Unretained)));
                    continue;
                }
            }
            let mut found = Err(Refused::OffArc);
            for &id in &own {
                let s = self.segments.get(id)?;
                if s[0] == s[1] || (s[0], s[1]) != (m[0], m[1]) && (s[1], s[0]) != (m[0], m[1]) {
                    continue;
                }
                let arc = self.arc(&s)?;
                if let Some(i) = arc[1..arc.len() - 1].iter().position(|&w| w == v) {
                    found = if self.in_chain(&s)? {
                        Err(Refused::Chain)
                    } else {
                        Ok(Site {
                            segment: id,
                            ordinal: s[4] + 1 + i as u64,
                        })
                    };
                    break;
                }
            }
            out.push((k, found));
        }
        Ok(out)
    }

    /// The overlay of `sites` (any cells', any depths): per segment, kept ordinals are
    /// added in ascending order, each only if every piece it makes is certified within the
    /// segment's step tolerance; one that is not is dropped (its cell then fails). Returns
    /// the overlay and the sites dropped.
    pub(crate) fn split(
        &self,
        sites: &BTreeMap<u32, BTreeSet<Site>>,
        mut overlay: Overlay,
    ) -> Result<(Overlay, Vec<(u32, Site)>), JobError> {
        let n = self.source.vertices.len;
        let mut dropped = Vec::new();
        for (&depth, set) in sites {
            let mut by_segment: BTreeMap<u64, Vec<u64>> = BTreeMap::new();
            for site in set {
                by_segment
                    .entry(site.segment)
                    .or_default()
                    .push(site.ordinal);
            }
            for (segment, ordinals) in by_segment {
                let s = self.segments.get(segment)?;
                let arc = self.arc(&s)?;
                let vertex = |i: u64| -> Result<u64, JobError> {
                    usize::try_from(i - s[4])
                        .ok()
                        .and_then(|k| arc.get(k).copied())
                        .ok_or_else(|| state("a split ordinal is off its arc"))
                };
                let point = |i: u64| self.source.point(vertex(i)?);
                let tolerance = self.ladder.relief(depth, narrow(s[11])?);
                let mut cuts = vec![s[4], s[5]];
                for ordinal in ordinals {
                    let at = cuts.partition_point(|&c| c < ordinal);
                    if cuts.get(at) == Some(&ordinal) {
                        continue;
                    }
                    let (a, b) = (cuts[at - 1], cuts[at]);
                    let fits = curve::certify(a, ordinal, &mut |i| point(i))? <= tolerance
                        && curve::certify(ordinal, b, &mut |i| point(i))? <= tolerance;
                    if fits {
                        cuts.insert(at, ordinal);
                    } else {
                        dropped.push((depth, Site { segment, ordinal }));
                    }
                }
                if cuts.len() == 2 {
                    continue;
                }
                let mut pieces = Vec::with_capacity(cuts.len() - 1);
                let mut bound = 0f64;
                let mut maps = Vec::new();
                for w in cuts.windows(2) {
                    let (a, b) = (w[0], w[1]);
                    let piece_bound = curve::certify(a, b, &mut |i| point(i))?;
                    bound = bound.max(piece_bound);
                    let mut row = s;
                    row[0] = vertex(a)?;
                    row[1] = vertex(b)?;
                    row[4] = a;
                    row[5] = b;
                    row[6] = piece_bound.to_bits();
                    pieces.push(row);
                    for i in a..=b {
                        if i == s[4] || i == s[5] {
                            continue;
                        }
                        let v = vertex(i)?;
                        let map = if i == a || i == b {
                            [v, v, 0]
                        } else {
                            [
                                vertex(a)?,
                                vertex(b)?,
                                ((i - a) as f64 / (b - a) as f64).to_bits(),
                            ]
                        };
                        maps.push((v, map));
                    }
                }
                maps.sort_unstable();
                maps.dedup();
                let kept: Vec<Key> = cuts[1..cuts.len() - 1]
                    .iter()
                    .map(|&i| self.source.key(vertex(i)?))
                    .collect::<Result<_, _>>()?;
                // Coarser chords over this arc: each kept vertex lies inside one at every
                // coarser depth (it is not retained there).
                let mut coarser = Vec::new();
                for &i in &cuts[1..cuts.len() - 1] {
                    for d in 0..depth {
                        let m = self.maps.get(u64::from(d) * n + vertex(i)?)?;
                        if m[0] != m[1] {
                            coarser.push((d, m[0].min(m[1]), m[0].max(m[1])));
                        }
                    }
                }
                coarser.sort_unstable();
                coarser.dedup();
                overlay.insert(
                    depth,
                    segment,
                    Split {
                        pieces,
                        maps,
                        kept,
                        bound,
                    },
                    coarser,
                );
            }
        }
        Ok((overlay, dropped))
    }
}

/// A border store value that is a `u32` by construction.
fn narrow(v: u64) -> Result<u32, JobError> {
    u32::try_from(v).map_err(|_| state("a border value exceeds u32"))
}
