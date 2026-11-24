//! Bounded root recovery from a complete built-descendant frontier.
use super::{state, store::TileKey, Job, TileGeometry};
use crate::{mesh_large::records::NodeKind, JobError};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

pub(crate) const INPUTS: usize = 8;
pub(crate) const OMITTED: usize = INPUTS * 32;
/// Root recovery v2: the most frontier tiles a recovered root delivers (its children).
/// Provisional: it bounds the root's children and its relation metadata (about 100 bytes
/// of JSON per frontier tile, within the 64 KiB declaration).
pub(crate) const FRONTIER: usize = 384;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Input {
    pub(crate) key: TileKey,
    pub(crate) triangles: u32,
}

/// Ownership is separate from the recovered root's explicit refinement graph.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct Recovery {
    pub(crate) version: u32,
    pub(crate) root: TileKey,
    pub(crate) encoded_depth: u32,
    pub(crate) frontier: Vec<Input>,
    pub(crate) omitted: Vec<TileKey>,
}

impl Recovery {
    /// Resolve the producer's relation before delivery (version 2; validators derive their
    /// own checks): the frontier covers the root exactly, in order, with at most `FRONTIER`
    /// tiles of at most the cap; every omitted cell lies strictly between the root and the
    /// frontier and is covered exactly by the frontier tiles inside it.
    pub(crate) fn check(&self) -> Result<(), JobError> {
        if self.version != 2
            || self.root.depth != 0
            || self.root.first != 0
            || self.root.leaf_begin != 0
            || self.encoded_depth > 30
            || self.omitted.is_empty()
            || self.omitted.len() > OMITTED
            || self.frontier.is_empty()
            || self.frontier.len() > FRONTIER
            || !covers(self.frontier.iter().map(|c| c.key), self.root)
            || self
                .frontier
                .iter()
                .any(|c| c.triangles == 0 || c.triangles as usize > super::TRIANGLE_CAP)
        {
            return Err(state("invalid root recovery v2 declaration"));
        }
        for (i, k) in self.omitted.iter().enumerate() {
            if k.depth <= self.encoded_depth
                || k.depth > 30
                || self.omitted[..i].contains(k)
                || !covers(
                    self.frontier
                        .iter()
                        .filter(|c| k.first <= c.key.first && c.key.end <= k.end)
                        .map(|c| c.key),
                    *k,
                )
            {
                return Err(state("invalid root recovery v2 ownership scaffold"));
            }
        }
        Ok(())
    }
}

/// Whether `tiles`, in order, cover `key`'s rank and leaf ranges exactly.
fn covers(tiles: impl IntoIterator<Item = TileKey>, key: TileKey) -> bool {
    let (mut first, mut leaf, mut count) = (key.first, key.leaf_begin, 0);
    for child in tiles {
        count += 1;
        if child.first != first
            || child.leaf_begin != leaf
            || child.end <= child.first
            || child.leaf_end <= child.leaf_begin
        {
            return false;
        }
        (first, leaf) = (child.end, child.leaf_end);
    }
    count > 0 && first == key.end && leaf == key.leaf_end
}

/// The validator's bound on a recovery declaration's JSON (`recoveryBounds`).
pub(crate) const MAX_JSON: usize = 64 << 10;

/// Root recovery v2: the relation over the published frontier `frontier` (in rank order),
/// or `None` when it exceeds a bound the validator applies (more than `OMITTED` scaffold
/// cells, or a declaration past `MAX_JSON` bytes): the root then stays omitted.
pub(super) fn declare_v2(
    job: &Job<'_, '_>,
    key: TileKey,
    frontier: Vec<Input>,
) -> Result<Option<Recovery>, JobError> {
    if frontier.len() > FRONTIER || !covers(frontier.iter().map(|c| c.key), key) {
        return Err(state("root recovery v2 frontier does not cover the root"));
    }
    let Some(relation) = scaffold(job, key, 2, frontier)? else {
        return Ok(None);
    };
    let bytes = serde_json::to_vec(&relation)
        .map_err(|e| state(format!("recovery declaration: {e}")))?
        .len();
    Ok((bytes <= MAX_JSON).then_some(relation))
}

/// The skipped scaffold between `key` and `frontier`: one streamed ownership pass.
fn scaffold(
    job: &Job<'_, '_>,
    key: TileKey,
    version: u32,
    frontier: Vec<Input>,
) -> Result<Option<Recovery>, JobError> {
    let mut omitted = Vec::new();
    // One streamed ownership pass; at most eight root-to-frontier paths survive.
    // Nodes below a built frontier tile are never part of the skipped scaffold.
    for node in job.input.captured.tree.nodes.reader()? {
        (job.check)()?;
        let node = node?;
        if node.kind != NodeKind::Internal || node.depth() <= key.depth {
            continue;
        }
        let inside: Vec<_> = frontier
            .iter()
            .filter(|c| {
                node.depth() < c.key.depth && node.first <= c.key.first && c.key.end <= node.end
            })
            .collect();
        let (Some(first), Some(last)) = (inside.first(), inside.last()) else {
            continue;
        };
        if first.key.first != node.first || last.key.end != node.end {
            return Err(state("root recovery ownership scaffold is incomplete"));
        }
        if omitted.len() >= job.input.settings.mutation.scaffold_bound() {
            return Ok(None);
        }
        omitted.push(TileKey {
            first: node.first,
            end: node.end,
            depth: node.depth(),
            leaf_begin: first.key.leaf_begin,
            leaf_end: last.key.leaf_end,
        });
    }
    if omitted.is_empty() {
        return Err(state("root recovery has no omitted ownership cell"));
    }
    Ok(Some(Recovery {
        version,
        root: TileKey { depth: 0, ..key },
        encoded_depth: key.depth,
        frontier,
        omitted,
    }))
}

/// Root recovery v2: a frontier component's label, `(tile first rank) << 16 | index`, unique
/// per tile-local component of a frontier tile (a tile holds at most 2^16 triangles).
pub(crate) type Label = u64;
/// A canonical point as the validator reads it (signed zero folded).
pub(crate) type Canon = [u32; 3];

pub(crate) fn canon(p: [f32; 3]) -> Canon {
    p.map(|v| if v == 0. { 0 } else { v.to_bits() })
}

/// What a v2 job reports about the frontier (`label_job`): the labels it created (its
/// frontier tiles' components), the points where labels must join (shared border points of
/// its frontier tiles, and points where two labels meet in its inputs), and its stand-in's
/// surviving labels by point.
#[derive(Clone, Debug, Default)]
pub(crate) struct Labels {
    pub created: Vec<Label>,
    pub joins: Vec<(Canon, Label)>,
    pub stand_in: Vec<(Canon, Label)>,
}

/// The labels of `children` by point: a stand-in child brings its own (`stand_ins`, by its
/// first rank); a frontier tile's components (by canonical point) get new labels. Returns
/// the point map with the report (`stand_in` empty).
pub(crate) fn label_inputs(
    children: &[TileGeometry],
    stand_ins: &[(TileKey, Vec<(Canon, Label)>)],
) -> (std::collections::BTreeMap<Canon, Vec<Label>>, Labels) {
    let mut at: std::collections::BTreeMap<Canon, Vec<Label>> = std::collections::BTreeMap::new();
    let mut report = Labels::default();
    let add = |at: &mut std::collections::BTreeMap<Canon, Vec<Label>>, k: Canon, l: Label| {
        let v = at.entry(k).or_default();
        if !v.contains(&l) {
            v.push(l);
        }
    };
    for c in children {
        if let Some((_, list)) = stand_ins.iter().find(|(k, _)| *k == c.key) {
            for &(k, l) in list {
                add(&mut at, k, l);
            }
            continue;
        }
        // Tile-local components by canonical point.
        let mut index: HashMap<Canon, usize> = HashMap::new();
        let ids: Vec<usize> = c
            .positions
            .iter()
            .map(|&p| {
                let n = index.len();
                *index.entry(canon(p)).or_insert(n)
            })
            .collect();
        let mut parent: Vec<usize> = (0..index.len()).collect();
        fn find(parent: &mut [usize], mut a: usize) -> usize {
            while parent[a] != a {
                parent[a] = parent[parent[a]];
                a = parent[a];
            }
            a
        }
        for t in &c.triangles {
            let [a, b, d] = t.map(|v| ids[v as usize]);
            for (x, y) in [(a, b), (b, d)] {
                let (x, y) = (find(&mut parent, x), find(&mut parent, y));
                parent[x.max(y)] = x.min(y);
            }
        }
        // Only points a face references belong to a component.
        let mut used = vec![false; index.len()];
        for t in &c.triangles {
            for &v in t {
                used[ids[v as usize]] = true;
            }
        }
        let mut dense: HashMap<usize, Label> = HashMap::new();
        let shared: std::collections::HashSet<Canon> = c
            .interface
            .iter()
            .flat_map(|r| [r.a, r.b])
            .map(|k| k.map(|b| if f32::from_bits(b) == 0. { 0 } else { b }))
            .collect();
        // In vertex order (deterministic), each point once.
        let mut seen = vec![false; index.len()];
        for (&p, &i) in c.positions.iter().zip(&ids) {
            if !used[i] || std::mem::replace(&mut seen[i], true) {
                continue;
            }
            let k = &canon(p);
            let r = find(&mut parent, i);
            let next = (c.key.first << 16) | dense.len() as u64;
            let l = *dense.entry(r).or_insert_with(|| {
                report.created.push(next);
                next
            });
            add(&mut at, *k, l);
            if shared.contains(k) {
                report.joins.push((*k, l));
            }
        }
    }
    for (k, v) in &at {
        if v.len() > 1 {
            report.joins.extend(v.iter().map(|&l| (*k, l)));
        }
    }
    report.joins.sort_unstable();
    (at, report)
}

/// The walk's union of frontier labels (`Labels::created` and `joins` of every v2 job).
#[derive(Clone, Debug, Default)]
pub(crate) struct LabelUnion {
    parent: HashMap<Label, Label>,
    at: HashMap<Canon, Label>,
}

impl LabelUnion {
    fn find(&mut self, mut l: Label) -> Label {
        while let Some(&p) = self.parent.get(&l) {
            if p == l {
                break;
            }
            let g = self.parent.get(&p).copied().unwrap_or(p);
            self.parent.insert(l, g);
            l = g;
        }
        l
    }

    pub(crate) fn add(&mut self, labels: &Labels) {
        for &l in &labels.created {
            self.parent.entry(l).or_insert(l);
        }
        for &(k, l) in &labels.joins {
            self.parent.entry(l).or_insert(l);
            match self.at.get(&k).copied() {
                Some(other) => {
                    let (a, b) = (self.find(l), self.find(other));
                    if a != b {
                        self.parent.insert(a.max(b), a.min(b));
                    }
                }
                None => {
                    self.at.insert(k, l);
                }
            }
        }
    }

    /// Frontier components (label classes) none of whose labels survives at `kept`
    /// (the root's points, looked up in its point map `at`): the validator's recount.
    pub(crate) fn dropped(
        &mut self,
        at: &std::collections::BTreeMap<Canon, Vec<Label>>,
        kept: impl IntoIterator<Item = Canon>,
    ) -> u64 {
        let mut present = std::collections::HashSet::new();
        for k in kept {
            for &l in at.get(&k).into_iter().flatten() {
                let r = self.find(l);
                present.insert(r);
            }
        }
        let labels: Vec<Label> = self.parent.keys().copied().collect();
        let mut classes = std::collections::HashSet::new();
        for l in labels {
            let r = self.find(l);
            classes.insert(r);
        }
        classes.difference(&present).count() as u64
    }

    /// Bytes held (for the memory report).
    pub(crate) fn bytes(&self) -> u64 {
        ((self.parent.len() * 16 + self.at.len() * 20) * 16 / 7) as u64
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn key(first: u64, end: u64) -> TileKey {
        TileKey {
            first,
            end,
            depth: 1,
            leaf_begin: first as u32,
            leaf_end: end as u32,
        }
    }
    #[test]
    fn a_frontier_covers_its_root_exactly_or_not_at_all() {
        let root = key(0, 8);
        let tiles: Vec<_> = (0..8).map(|i| key(i, i + 1)).collect();
        assert!(covers(tiles.iter().copied(), root));
        assert!(!covers(tiles.iter().copied().skip(1), root));
        assert!(!covers(tiles.iter().copied().take(7), root));
        let mut overlap = tiles.clone();
        overlap[1].first = 0;
        assert!(!covers(overlap, root));
        assert!(covers((0..9).map(|i| key(i, i + 1)), key(0, 9)));
        assert!(!covers(std::iter::empty(), root));
    }
}
