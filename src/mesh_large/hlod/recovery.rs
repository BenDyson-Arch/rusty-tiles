//! Bounded root recovery from a complete built-descendant frontier.
use super::{state, store::TileKey, Job, TileGeometry};
use crate::{mesh_large::records::NodeKind, JobError};
use serde::{Deserialize, Serialize};

pub(crate) const INPUTS: usize = 8;
pub(crate) const OMITTED: usize = INPUTS * 32;

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
    /// Resolve the producer's relation before delivery; validators derive their own checks.
    pub(crate) fn check(&self) -> Result<(), JobError> {
        if self.version != 1
            || self.root.depth != 0
            || self.root.first != 0
            || self.root.leaf_begin != 0
            || self.encoded_depth > 30
            || self.omitted.is_empty()
            || self.omitted.len() > OMITTED
            || !bounded(
                self.frontier
                    .iter()
                    .map(|c| (c.key, u64::from(c.triangles))),
                self.root,
                super::TRIANGLE_CAP,
            )
        {
            return Err(state("invalid bounded root recovery declaration"));
        }
        for (i, k) in self.omitted.iter().enumerate() {
            if k.depth <= self.encoded_depth
                || k.depth > 30
                || self.omitted[..i].contains(k)
                || !bounded(
                    self.frontier
                        .iter()
                        .filter(|c| k.first <= c.key.first && c.key.end <= k.end)
                        .map(|c| (c.key, u64::from(c.triangles))),
                    *k,
                    super::TRIANGLE_CAP,
                )
            {
                return Err(state("invalid recovery ownership scaffold"));
            }
        }
        Ok(())
    }
}

/// A failed cell keeps either this complete capped cover or no reusable input.
pub(super) fn bounded(
    tiles: impl IntoIterator<Item = (TileKey, u64)>,
    key: TileKey,
    cap: usize,
) -> bool {
    let (mut first, mut leaf, mut count, mut triangles) = (key.first, key.leaf_begin, 0, 0);
    for (child, n) in tiles {
        count += 1;
        triangles = u64::saturating_add(triangles, n);
        if count > INPUTS
            || n == 0
            || n > cap as u64
            || triangles > INPUTS as u64 * cap as u64
            || child.first != first
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

pub(super) fn declare(
    job: &Job<'_, '_>,
    key: TileKey,
    children: &[TileGeometry],
) -> Result<Recovery, JobError> {
    if !bounded(
        children.iter().map(|c| (c.key, c.triangles.len() as u64)),
        key,
        job.input.settings.cap,
    ) {
        return Err(state(
            "root recovery input is not a complete bounded frontier",
        ));
    }
    let frontier = children
        .iter()
        .map(|c| {
            Ok(Input {
                key: TileKey {
                    depth: job.borders.publication_depth(c.key)?,
                    ..c.key
                },
                triangles: c.triangles.len() as u32,
            })
        })
        .collect::<Result<Vec<_>, JobError>>()?;
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
        if first.key.first != node.first || last.key.end != node.end || omitted.len() >= OMITTED {
            return Err(state(
                "root recovery ownership scaffold is incomplete or exceeds its bound",
            ));
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
    Ok(Recovery {
        version: 1,
        root: TileKey { depth: 0, ..key },
        encoded_depth: key.depth,
        frontier,
        omitted,
    })
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
    fn a_frontier_is_complete_or_unavailable_never_truncated() {
        let root = key(0, 8);
        let inputs: Vec<_> = (0..8).map(|i| (key(i, i + 1), 8192)).collect();
        assert!(bounded(inputs.iter().copied(), root, 8192));
        assert!(!bounded(inputs.iter().copied().skip(1), root, 8192));
        assert!(!bounded(inputs.iter().copied().take(7), root, 8192));
        let mut overlap = inputs.clone();
        overlap[1].0.first = 0;
        assert!(!bounded(overlap, root, 8192));
        let mut big = inputs.clone();
        big[0].1 = u64::MAX;
        assert!(!bounded(big, root, 8192));
        assert!(!bounded(
            (0..9).map(|i| (key(i, i + 1), 1)),
            key(0, 9),
            8192
        ));
        assert!(!bounded(std::iter::empty(), root, 8192));
    }
}
