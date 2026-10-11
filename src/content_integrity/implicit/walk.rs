//! §3 traversal and §4 effective values: a depth-first pre-order walk of one
//! root. A tile, its emitted contents in slot order, then its available
//! children in ascending child index. Child subtrees are admitted only when
//! the walk descends into them and released when their last tile finishes.
use super::subtree::{self, Subtree};
use super::{
    invalid, power_of_two, try_push, unsupported, Cell, EffectiveError, EffectiveVolume,
    FormatError, ImplicitError, ImplicitLimits, ImplicitRoot, Never, Resolver, VisitedTile,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum State {
    Fresh,
    Running,
    Done,
    Failed,
}

/// A live subtree on the current path.
#[derive(Debug)]
struct Live {
    subtree: Subtree,
    visited: u64,
    emitted: u64,
}

/// A tile on the current path.
#[derive(Clone, Copy, Debug)]
struct Frame {
    subtree: usize,
    local: u32,
    morton: u64,
    level: u32,
    coords: [u32; 3],
    next: u64,
}

/// The walk of one admitted root. Obtain it from [`ImplicitRoot::walk`].
#[derive(Debug)]
pub(crate) struct Walk<'r> {
    root: &'r ImplicitRoot<'r>,
    path: Vec<Live>,
    frames: Vec<Frame>,
    /// Σ live bytes of the subtrees on the current path.
    live: u64,
    emitted: u64,
    state: State,
}

fn child(coords: [u32; 3], c: u64) -> [u32; 3] {
    std::array::from_fn(|k| 2 * coords[k] + ((c >> k) & 1) as u32)
}

/// `e_R·2^−level`: exact when `e_R = 0` or the result is normal (§4, deeper
/// tiles). Level 0 is `e_R` itself, with nothing derived.
fn derived_error(root: f64, level: u32) -> Result<f64, FormatError> {
    if level == 0 {
        return Ok(root);
    }
    let error = root * power_of_two(level);
    if error != 0. && !error.is_normal() {
        return Err(unsupported(format!(
            "derived geometric error underflow at level {level}"
        )));
    }
    Ok(error)
}

impl<'r> Walk<'r> {
    pub(super) fn new(root: &'r ImplicitRoot<'r>) -> Self {
        Self {
            root,
            path: Vec::new(),
            frames: Vec::new(),
            live: 0,
            emitted: 0,
            state: State::Fresh,
        }
    }

    /// The next available tile in pre-order, or `None` when the walk is done.
    pub(crate) fn next<E>(
        &mut self,
        limits: &mut ImplicitLimits,
        read: &mut Resolver<'_, E>,
    ) -> Result<Option<VisitedTile>, ImplicitError<E>> {
        match self.state {
            State::Failed => return Err(ImplicitError::InvalidState("walk resumed after failure")),
            State::Done => return Ok(None),
            State::Fresh | State::Running => {}
        }
        let result = self.step(limits, read);
        self.state = match &result {
            Err(_) => State::Failed,
            Ok(None) => State::Done,
            Ok(Some(_)) => State::Running,
        };
        result
    }

    /// The post-walk content rule (`:299`).
    pub(crate) fn finish(self) -> Result<(), ImplicitError<Never>> {
        if self.state != State::Done {
            return Err(ImplicitError::InvalidState(
                "walk finished before completion",
            ));
        }
        if !self.root.contents.is_empty() && self.emitted == 0 {
            return Err(invalid(
                "implicit root has content templates but no tile has available content",
            )
            .into());
        }
        Ok(())
    }

    fn step<E>(
        &mut self,
        limits: &mut ImplicitLimits,
        read: &mut Resolver<'_, E>,
    ) -> Result<Option<VisitedTile>, ImplicitError<E>> {
        if self.state == State::Fresh {
            self.enter(0, [0; 3], limits, read)?;
            return self.visit().map(Some);
        }
        let n = self.root.scheme.branches();
        loop {
            let Some(frame) = self.frames.last_mut() else {
                return Ok(None);
            };
            if frame.next == n {
                let frame = *frame;
                self.frames.pop();
                if frame.local == 0 {
                    self.leave(frame.subtree)?;
                }
                continue;
            }
            let c = frame.next;
            frame.next += 1;
            let frame = *frame;
            let (level, coords) = (frame.level + 1, child(frame.coords, c));
            let live = &self.path[frame.subtree];
            let availability = &live.subtree.availability;
            let morton = u128::from(frame.morton) * u128::from(n) + u128::from(c);
            if frame.local + 1 < availability.shape().levels {
                // Within T, so within u64.
                let i = u128::from(availability.first_bit(frame.local + 1)) + morton;
                if availability.tile(&live.subtree.storage, i as u64) {
                    try_push(
                        &mut self.frames,
                        Frame {
                            subtree: frame.subtree,
                            local: frame.local + 1,
                            morton: morton as u64,
                            level,
                            coords,
                            next: 0,
                        },
                    )?;
                    return self.visit().map(Some);
                }
            } else if availability.child(&live.subtree.storage, morton) {
                self.enter(level, coords, limits, read)?;
                return self.visit().map(Some);
            }
        }
    }

    fn enter<E>(
        &mut self,
        level: u32,
        coords: [u32; 3],
        limits: &mut ImplicitLimits,
        read: &mut Resolver<'_, E>,
    ) -> Result<(), ImplicitError<E>> {
        let subtree = subtree::admit(self.root, level, coords, limits, &mut self.live, read)?;
        try_push(
            &mut self.path,
            Live {
                subtree,
                visited: 0,
                emitted: 0,
            },
        )?;
        try_push(
            &mut self.frames,
            Frame {
                subtree: self.path.len() - 1,
                local: 0,
                morton: 0,
                level,
                coords,
                next: 0,
            },
        )?;
        Ok(())
    }

    /// Release the finished subtree at the top of the path.
    fn leave<E>(&mut self, index: usize) -> Result<(), ImplicitError<E>> {
        if index + 1 != self.path.len() {
            return Err(ImplicitError::InvalidState("subtree released out of order"));
        }
        let live = self.path.pop().unwrap();
        let availability = &live.subtree.availability;
        if live.visited != availability.tile_popcount {
            return Err(ImplicitError::InvalidState(
                "subtree visits differ from the reservation",
            ));
        }
        if live.emitted != availability.entries {
            return Err(ImplicitError::InvalidState(
                "emitted contents differ from the reserved entries",
            ));
        }
        self.live -= live.subtree.live_bytes;
        Ok(())
    }

    /// The tile of the top frame.
    fn visit<E>(&mut self) -> Result<VisitedTile, ImplicitError<E>> {
        let frame = *self.frames.last().unwrap();
        let root = self.root;
        let live = &mut self.path[frame.subtree];
        let availability = &live.subtree.availability;
        live.visited += 1;
        if live.visited > availability.tile_popcount {
            return Err(ImplicitError::InvalidState("visit beyond the reservation"));
        }
        let storage = &live.subtree.storage;
        let i = availability.first_bit(frame.local) + frame.morton;
        let rank = availability.rank(storage, i);
        if rank >= availability.tile_popcount {
            return Err(ImplicitError::InvalidState(
                "tile rank beyond the row count",
            ));
        }
        let row = |column: super::metadata::Column, lanes: usize| {
            let size = column.float.size();
            let at = rank as usize * lanes * size;
            let values = &storage.view(column.view)[at..at + lanes * size];
            move |lane: usize| column.float.read(&values[lane * size..])
        };
        let volume = match live.subtree.columns.bounding_box {
            Some(column) => {
                let lane = row(column, 12);
                EffectiveVolume::Row(std::array::from_fn(lane))
            }
            None if frame.level == 0 => EffectiveVolume::DerivedRoot,
            None => EffectiveVolume::Derived(Cell {
                root: root.root_box,
                level: frame.level,
                coords: frame.coords,
                scheme: root.scheme,
            }),
        };
        let error = match live.subtree.columns.geometric_error {
            Some(column) => EffectiveError::Row(row(column, 1)(0)),
            None => EffectiveError::Derived(derived_error(root.root_error, frame.level)?),
        };
        let slots = availability.entries(rank);
        let mut contents = Vec::new();
        contents
            .try_reserve_exact(slots.len())
            .map_err(|_| super::refused())?;
        for &slot in slots {
            let name = root.contents[usize::from(slot)].substitute(frame.level, frame.coords)?;
            contents.push((slot, name));
        }
        live.emitted += slots.len() as u64;
        self.emitted += slots.len() as u64;
        Ok(VisitedTile {
            level: frame.level,
            coords: frame.coords,
            volume,
            error,
            contents,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::super::test_support::*;
    use super::super::{admit_root, EffectiveError, ImplicitError, Resolver};
    use serde_json::{json, Value};

    fn leaf(tile: Av) -> Sub {
        Sub::new().tile(tile).child(Av::Const(0))
    }

    #[test]
    fn preorder_across_subtrees_with_morton_x_lsb() {
        // OCTREE L = 1, A = 3. Child index bit 0 is x, bit 1 is y, bit 2 is z.
        let tile = root_tile("OCTREE", 1, 3, &["c/{level}/{x}/{y}/{z}.glb"]);
        let inner = |child: usize| {
            Sub::new()
                .tile(Av::Const(1))
                .child(Av::Bits(bits(8, &[child])))
                .content(&[Av::Const(1)])
                .build()
        };
        let mut archive = Archive::default()
            .with(
                "s/0/0/0/0.subtree",
                Sub::new()
                    .tile(Av::Const(1))
                    .child(Av::Bits(bits(8, &[1, 4])))
                    .content(&[Av::Const(0)])
                    .build(),
            )
            .with("s/1/1/0/0.subtree", inner(2))
            .with("s/1/0/0/1.subtree", inner(2))
            .with(
                "s/2/2/1/0.subtree",
                leaf(Av::Const(1)).content(&[Av::Const(1)]).build(),
            )
            .with(
                "s/2/0/1/2.subtree",
                leaf(Av::Const(1)).content(&[Av::Const(1)]).build(),
            );
        let tiles = walk(&tile, &mut archive).unwrap();
        assert_eq!(
            coords(&tiles),
            vec![
                (0, 0, 0, 0),
                (1, 1, 0, 0),
                (2, 2, 1, 0),
                (1, 0, 0, 1),
                (2, 0, 1, 2)
            ]
        );
        assert!(tiles[0].contents.is_empty());
        assert_eq!(tiles[2].contents, vec![(0, "c/2/2/1/0.glb".to_owned())]);
        // Child subtrees are read only when the walk descends into them.
        let reads: Vec<&str> = archive.reads.iter().map(|(n, _)| n.as_str()).collect();
        assert_eq!(
            reads,
            [
                "s/0/0/0/0.subtree",
                "s/1/1/0/0.subtree",
                "s/2/2/1/0.subtree",
                "s/1/0/0/1.subtree",
                "s/2/0/1/2.subtree"
            ]
        );
    }

    #[test]
    fn preorder_within_a_subtree() {
        // QUADTREE L = 3: bits 1 (1,0,0), 3 (1,0,1), 5 (2,0,0), 8 (2,1,1),
        // 13 (2,0,2) and 14 (2,1,2).
        let tile = root_tile("QUADTREE", 3, 3, &[]);
        let mut archive = Archive::default().with(
            "s/0/0/0.subtree",
            leaf(Av::Bits(bits(21, &[0, 1, 3, 5, 8, 13, 14]))).build(),
        );
        assert_eq!(
            coords(&walk(&tile, &mut archive).unwrap()),
            vec![
                (0, 0, 0, 0),
                (1, 0, 0, 0),
                (2, 0, 0, 0),
                (2, 1, 1, 0),
                (1, 0, 1, 0),
                (2, 0, 2, 0),
                (2, 1, 2, 0)
            ]
        );
    }

    #[test]
    fn missing_child_subtree_is_the_resolvers_failure() {
        // A-10.
        let tile = root_tile("QUADTREE", 1, 2, &[]);
        let mut archive = Archive::default().with(
            "s/0/0/0.subtree",
            Sub::new()
                .tile(Av::Const(1))
                .child(Av::Bits(bits(4, &[3])))
                .build(),
        );
        let result = walk(&tile, &mut archive);
        assert!(matches!(
            result,
            Err(ImplicitError::Resolver(m)) if m == "missing archive entry: s/1/1/1.subtree"
        ));
        // A sparse child bitstream reads only its set bits (C is never enumerated).
        let wide = root_tile("QUADTREE", 8, 9, &[]);
        let mut archive = Archive::default()
            .with(
                "s/0/0/0.subtree",
                Sub::new()
                    .tile(Av::Const(1))
                    .child(Av::Bits(bits(65_536, &[65_535])))
                    .build(),
            )
            .with(
                "s/8/255/255.subtree",
                leaf(Av::Bits(bits(21_845, &[0]))).build(),
            );
        assert_eq!(walk(&wide, &mut archive).unwrap().len(), 21_845 + 1);
        assert_eq!(archive.reads.len(), 2);
    }

    #[test]
    fn derived_error_is_exact_or_unsupported() {
        let mut tile = root_tile("QUADTREE", 2, 2, &[]);
        let member = leaf(Av::Const(1)).build();
        let run = |tile: &Value| {
            let mut archive = Archive::default().with("s/0/0/0.subtree", member.clone());
            format(walk(tile, &mut archive))
        };
        let tiles = run(&tile).unwrap();
        assert_eq!(tiles[1].error, EffectiveError::Derived(32.));
        // B-10: a nonzero subnormal derived error.
        tile["geometricError"] = json!(f64::MIN_POSITIVE);
        assert_unsupported(run(&tile), "derived geometric error underflow at level 1");
        tile["geometricError"] = json!(0.0);
        assert!(run(&tile)
            .unwrap()
            .iter()
            .all(|t| t.error == EffectiveError::Derived(0.)));
        // Level 0 is e_R itself, even when subnormal.
        let mut single = root_tile("QUADTREE", 2, 1, &[]);
        single["geometricError"] = json!(f64::MIN_POSITIVE / 2.);
        let mut archive =
            Archive::default().with("s/0/0/0.subtree", leaf(Av::Bits(bits(5, &[0]))).build());
        assert_eq!(
            walk(&single, &mut archive).unwrap()[0].error,
            EffectiveError::Derived(f64::MIN_POSITIVE / 2.)
        );
    }

    /// R2 at small scale: QUADTREE L = 1, A = 4, branching 2 per level and no
    /// payloads; every subtree padded with an unused 64-byte view.
    fn chain() -> (Value, Archive, Vec<u64>) {
        let tile = root_tile("QUADTREE", 1, 4, &[]);
        let mut archive = Archive::default();
        let mut sizes = Vec::new();
        for level in 0..4u32 {
            let child = if level < 3 {
                Av::Bits(bits(4, &[0, 3]))
            } else {
                Av::Const(0)
            };
            let mut sub = Sub::new().tile(Av::Const(1)).child(child);
            sub.view(&[7; 64]);
            let member = sub.build();
            sizes.push(member.len() as u64);
            for x in 0..1u32 << level {
                for y in 0..1u32 << level {
                    archive
                        .members
                        .insert(format!("s/{level}/{x}/{y}.subtree"), member.clone());
                }
            }
        }
        (tile, archive, sizes)
    }

    #[test]
    fn live_path_bytes_are_released_with_their_subtrees() {
        let (tile, mut archive, sizes) = chain();
        let root = admit_root(&tile, "tileset.json", None).unwrap();
        let path: u64 = sizes.iter().sum();
        let cumulative: u64 = (0..4).map(|l| sizes[l] << l).sum();
        assert!(cumulative > 3 * path);
        // Exactly the maximum live sum is admitted (R2b: far below cumulative).
        let mut limits = limits();
        limits.live_subtree_bytes = path;
        assert_eq!(
            walk_with(&root, &mut archive, &mut limits).unwrap().len(),
            15
        );
        // The resolver is always offered `cap − live`.
        let maxima: Vec<u64> = archive.reads.iter().map(|(_, m)| *m).collect();
        assert_eq!(
            &maxima[..4],
            &[path, path - sizes[0], path - sizes[0] - sizes[1], sizes[3]]
        );
        // The sibling at the deepest level sees the released bytes again.
        assert_eq!(maxima[4], sizes[3]);
        assert_eq!(maxima[5], path - sizes[0] - sizes[1]);
        // One byte less refuses the deepest read (R2).
        let mut archive = chain().1;
        let mut limits = super::super::test_support::limits();
        limits.live_subtree_bytes = path - 1;
        assert!(matches!(
            walk_with(&root, &mut archive, &mut limits),
            Err(ImplicitError::Resolver(m)) if m.starts_with("member exceeds admitted byte limit: s/3/")
        ));
    }

    #[test]
    fn repeated_subtree_members_are_unsupported() {
        // A-11 across roots in one validation: the limits carry the set.
        let tile = root_tile("QUADTREE", 1, 1, &[]);
        let mut archive = Archive::default().with("s/0/0/0.subtree", leaf(Av::Const(1)).build());
        let root = admit_root(&tile, "tileset.json", None).unwrap();
        let mut limits = limits();
        assert!(walk_with(&root, &mut archive, &mut limits).is_ok());
        assert_unsupported(
            format(walk_with(&root, &mut archive, &mut limits)),
            "repeated subtree member s/0/0/0.subtree",
        );
        // Within one root: `{x}{y}` names (1, 10) and (11, 0) alike at level 4.
        let mut tile = root_tile("QUADTREE", 2, 6, &[]);
        tile["implicitTiling"]["subtrees"]["uri"] = json!("s/{level}/{x}{y}.subtree");
        let mut archive = Archive::default();
        for level in [0u32, 2, 4] {
            let child = Av::Const(u64::from(level < 4));
            for x in 0..1u32 << level {
                for y in 0..1u32 << level {
                    archive.members.insert(
                        format!("s/{level}/{x}{y}.subtree"),
                        Sub::new().tile(Av::Const(1)).child(child.clone()).build(),
                    );
                }
            }
        }
        assert_unsupported(
            format(walk(&tile, &mut archive)),
            "repeated subtree member s/4/",
        );
    }

    #[test]
    fn post_walk_content_rule_and_walk_state() {
        // C-14: content templates, but no tile has available content.
        let tile = root_tile("QUADTREE", 1, 1, &["c/{level}/{x}/{y}.glb"]);
        let empty = leaf(Av::Const(1)).content(&[Av::Const(0)]).build();
        let mut archive = Archive::default().with("s/0/0/0.subtree", empty);
        assert_invalid(
            format(walk(&tile, &mut archive)),
            "no tile has available content",
        );
        let full = leaf(Av::Const(1)).content(&[Av::Const(1)]).build();
        let mut archive = Archive::default().with("s/0/0/0.subtree", full);
        assert_eq!(walk(&tile, &mut archive).unwrap()[0].contents.len(), 1);
        // Content-free roots have no content rule.
        let bare = root_tile("QUADTREE", 1, 1, &[]);
        let mut archive = Archive::default().with("s/0/0/0.subtree", leaf(Av::Const(1)).build());
        assert!(walk(&bare, &mut archive).is_ok());
        // finish() before the end, next() after a failure, next() after the end.
        let root = admit_root(&bare, "tileset.json", None).unwrap();
        let mut limits = limits();
        let read: &mut Resolver<'_, String> = &mut |n, b, m| archive.read(n, b, m);
        let mut early = root.walk();
        assert!(early.next(&mut limits, read).unwrap().is_some());
        assert!(matches!(
            early.finish(),
            Err(ImplicitError::InvalidState(_))
        ));
        let mut walk = root.walk();
        assert!(walk.next(&mut limits, read).is_err());
        assert!(matches!(
            walk.next(&mut limits, read),
            Err(ImplicitError::InvalidState(_))
        ));
        let mut empty = Archive::default().with("s/0/0/0.subtree", leaf(Av::Const(1)).build());
        let read: &mut Resolver<'_, String> = &mut |n, b, m| empty.read(n, b, m);
        let mut limits = super::super::test_support::limits();
        let mut walk = root.walk();
        assert!(walk.next(&mut limits, read).unwrap().is_some());
        assert!(walk.next(&mut limits, read).unwrap().is_none());
        assert!(walk.next(&mut limits, read).unwrap().is_none());
        assert!(walk.finish().is_ok());
    }

    #[test]
    fn charges_are_mirrored_exactly() {
        // Visits by P_t, references by reads + E, bytes by reads + Scan.
        let tile = root_tile(
            "QUADTREE",
            2,
            3,
            &["a/{level}/{x}/{y}", "b/{level}/{x}/{y}"],
        );
        let root_member = Sub::new()
            .tile(Av::Bits(bits(5, &[0, 1, 2])))
            .child(Av::Bits(bits(16, &[0])))
            .content(&[Av::Const(0), Av::Bits(bits(5, &[0, 2]))])
            .build();
        // Root P_t 3, E 2, Scan 1 + 2 + 1; child P_t 1 (only level 2 < A),
        // E 1 and Scan 1 (its content layer aliases the tile view).
        let child_member = leaf(Av::Bits(bits(5, &[0])))
            .content(&[Av::View(0), Av::Const(0)])
            .build();
        let mut archive = Archive::default()
            .with("s/0/0/0.subtree", root_member.clone())
            .with("s/2/0/0.subtree", child_member.clone());
        let root = admit_root(&tile, "tileset.json", None).unwrap();
        let mut limits = limits();
        let before = (
            limits.visits_remaining,
            limits.refs_remaining,
            limits.scan_remaining,
        );
        let tiles = walk_with(&root, &mut archive, &mut limits).unwrap();
        assert_eq!(tiles.len(), 4);
        assert_eq!(tiles.iter().map(|t| t.contents.len()).sum::<usize>(), 3);
        assert_eq!(limits.visits_remaining, before.0 - 4);
        assert_eq!(limits.refs_remaining, before.1 - 2 - 3);
        let reads = (root_member.len() + child_member.len()) as u64;
        assert_eq!(limits.scan_remaining, before.2 - reads - (1 + 2 + 1) - 1);
    }
}
