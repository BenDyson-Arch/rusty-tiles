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
