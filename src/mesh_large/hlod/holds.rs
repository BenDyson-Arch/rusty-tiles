//! D's hold plan. A built cell holds its inputs until every co-owner of its chords has
//! built, in case a co-owner's kept vertex splits a shared chord and the cell must build
//! again. Co-owners can be far apart in the walk, so holds are planned before it starts,
//! within a budget of holds, and the walk enforces the plan:
//!
//! - **Interval.** A cell's hold spans its postorder rank to `R`, the latest rank its
//!   release can wait on: a co-owner's build, which waits for that co-owner's children to
//!   be released, which waits for their co-owners' builds, and so on (bottom-up by depth:
//!   `S(c)` = max(rank(c), `R` of its children); `R(c)` = max(`S` of c and its co-owners)).
//! - **Admission.** Cells are admitted in postorder while the number of admitted intervals
//!   through every rank of theirs stays within the budget. A chord with an owner that is
//!   not admitted is sealed: no owner keeps a vertex on it, so that owner's first build is
//!   final and it holds nothing.
//! - **Gate.** An admitted cell spawns only after every admitted cell whose interval ends
//!   before its rank has been released.
//!
//! Then the cells holding at any moment all span the rank of the latest of them to spawn:
//! that one waited for every admitted cell ending before it, so each other holder (of
//! lower rank, not released) ends at or after it. So holds never exceed the budget, for
//! any number of workers. A gate waits for releases that depend (through co-owners,
//! children and their gates) only on ranks below the waiting cell's, so the walk cannot
//! wait on itself. The plan reads only the tree and the borders, so it is the same for
//! any number of workers.
use super::{borders::Borders, JobError};
use std::collections::{HashMap, HashSet};

/// One cell's planned hold: its rank and the end of its interval.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Interval {
    pub rank: usize,
    pub end: usize,
}

#[derive(Debug, Default)]
pub(crate) struct Plan {
    /// Admitted cells with co-owners, by border node id.
    admitted: HashMap<u64, Interval>,
    /// Segments with an owner that is not admitted.
    sealed: HashSet<u64>,
    pub budget: u64,
    /// Cells with co-owners not admitted.
    pub sealed_cells: u64,
    /// The most intervals through one rank with every cell admitted: the least budget
    /// that seals nothing.
    pub peak: u64,
}

impl Plan {
    pub(crate) fn new(borders: &Borders, budget: u64) -> Result<Self, JobError> {
        // Coarse cells in node order: by first source rank, shallower first (preorder).
        let cells = borders.coarse_cells()?;
        let index: HashMap<u64, usize> = cells
            .iter()
            .enumerate()
            .map(|(i, (n, _))| (*n, i))
            .collect();
        // Postorder: by end, deeper first among cells sharing it (nested ones).
        let mut order: Vec<usize> = (0..cells.len()).collect();
        order.sort_unstable_by_key(|&i| (cells[i].1.end, std::cmp::Reverse(cells[i].1.depth)));
        let mut rank = vec![0usize; cells.len()];
        for (r, &i) in order.iter().enumerate() {
            rank[i] = r;
        }
        // Each cell's coarse parent: the nearest enclosing cell in preorder.
        let mut parent = vec![None; cells.len()];
        let mut stack: Vec<usize> = Vec::new();
        for (i, (_, key)) in cells.iter().enumerate() {
            while stack.last().is_some_and(|&t| cells[t].1.end <= key.first) {
                stack.pop();
            }
            parent[i] = stack.last().copied();
            stack.push(i);
        }
        let mut co_owners = Vec::with_capacity(cells.len());
        for (_, key) in &cells {
            let co: Vec<usize> = borders
                .co_owners(*key)?
                .iter()
                .map(|n| index.get(n).copied())
                .collect::<Option<_>>()
                .ok_or_else(|| super::state("a co-owner is not a coarse cell"))?;
            co_owners.push(co);
        }
        // `S` and `R`, deepest cells first (children are deeper than their parent).
        let mut by_depth: Vec<usize> = (0..cells.len()).collect();
        by_depth.sort_unstable_by_key(|&i| std::cmp::Reverse(cells[i].1.depth));
        let (mut spawn, mut release) = (rank.clone(), vec![0usize; cells.len()]);
        let mut children = vec![0usize; cells.len()];
        for level in by_depth.chunk_by(|&a, &b| cells[a].1.depth == cells[b].1.depth) {
            for &i in level {
                spawn[i] = rank[i].max(children[i]);
            }
            for &i in level {
                release[i] = co_owners[i].iter().fold(spawn[i], |a, &o| a.max(spawn[o]));
                if let Some(p) = parent[i] {
                    children[p] = children[p].max(release[i]);
                }
            }
        }
        let holding: Vec<usize> = order
            .iter()
            .copied()
            .filter(|&i| !co_owners[i].is_empty())
            .collect();
        let spans: Vec<Interval> = holding
            .iter()
            .map(|&i| Interval {
                rank: rank[i],
                end: release[i],
            })
            .collect();
        let admit = admit(&spans, cells.len(), budget);
        let mut plan = Plan {
            budget,
            peak: peak(&spans, cells.len()),
            ..Default::default()
        };
        for ((&i, &interval), admitted) in holding.iter().zip(&spans).zip(admit) {
            let (node, key) = cells[i];
            if admitted {
                plan.admitted.insert(node, interval);
            } else {
                plan.sealed_cells += 1;
                plan.sealed.extend(borders.own_segments(key)?);
            }
        }
        Ok(plan)
    }

    /// A plan over given intervals (cells by node id), admitted within `budget`.
    #[cfg(test)]
    pub(crate) fn from_intervals(cells: &[(u64, Interval)], ranks: usize, budget: u64) -> Self {
        let spans: Vec<Interval> = cells.iter().map(|&(_, i)| i).collect();
        let mut plan = Plan {
            budget,
            peak: peak(&spans, ranks),
            ..Default::default()
        };
        for (&(node, interval), admitted) in cells.iter().zip(admit(&spans, ranks, budget)) {
            if admitted {
                plan.admitted.insert(node, interval);
            } else {
                plan.sealed_cells += 1;
            }
        }
        plan
    }

    /// The admitted interval of a cell that may hold, or `None` for a cell that holds
    /// nothing (no co-owners, or not admitted).
    pub(crate) fn holds(&self, node: u64) -> Option<Interval> {
        self.admitted.get(&node).copied()
    }

    /// Every admitted cell, with its interval.
    pub(crate) fn admitted(&self) -> impl Iterator<Item = (u64, Interval)> + '_ {
        self.admitted.iter().map(|(&n, &i)| (n, i))
    }

    pub(crate) fn sealed(&self, segment: u64) -> bool {
        self.sealed.contains(&segment)
    }
}

/// Greedy admission in rank order: an interval is admitted while every rank it spans has
/// fewer than `budget` admitted intervals through it.
fn admit(spans: &[Interval], ranks: usize, budget: u64) -> Vec<bool> {
    let mut tree = MaxAdd::new(ranks.max(1));
    spans
        .iter()
        .map(|s| {
            let ok = tree.max(s.rank, s.end) < budget;
            if ok {
                tree.add(s.rank, s.end);
            }
            ok
        })
        .collect()
}

/// The most intervals through one rank.
fn peak(spans: &[Interval], ranks: usize) -> u64 {
    let (mut starts, mut ends) = (vec![0u64; ranks], vec![0u64; ranks]);
    for s in spans {
        starts[s.rank] += 1;
        ends[s.end] += 1;
    }
    let (mut run, mut most) = (0u64, 0u64);
    for (s, e) in starts.iter().zip(&ends) {
        run += s;
        most = most.max(run);
        run -= e;
    }
    most
}

/// Range add of one, range maximum (segment tree with lazy adds).
struct MaxAdd {
    n: usize,
    max: Vec<u64>,
    add: Vec<u64>,
}

impl MaxAdd {
    fn new(n: usize) -> Self {
        Self {
            n,
            max: vec![0; 4 * n],
            add: vec![0; 4 * n],
        }
    }
    fn max(&self, lo: usize, hi: usize) -> u64 {
        self.query(1, 0, self.n - 1, lo, hi)
    }
    fn add(&mut self, lo: usize, hi: usize) {
        self.update(1, 0, self.n - 1, lo, hi);
    }
    fn query(&self, at: usize, l: usize, r: usize, lo: usize, hi: usize) -> u64 {
        if hi < l || r < lo {
            return 0;
        }
        if lo <= l && r <= hi {
            return self.max[at];
        }
        let m = (l + r) / 2;
        self.add[at]
            + self
                .query(2 * at, l, m, lo, hi)
                .max(self.query(2 * at + 1, m + 1, r, lo, hi))
    }
    fn update(&mut self, at: usize, l: usize, r: usize, lo: usize, hi: usize) {
        if hi < l || r < lo {
            return;
        }
        if lo <= l && r <= hi {
            self.max[at] += 1;
            self.add[at] += 1;
            return;
        }
        let m = (l + r) / 2;
        self.update(2 * at, l, m, lo, hi);
        self.update(2 * at + 1, m + 1, r, lo, hi);
        self.max[at] = self.add[at] + self.max[2 * at].max(self.max[2 * at + 1]);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spans(v: &[(usize, usize)]) -> Vec<Interval> {
        v.iter()
            .map(|&(rank, end)| Interval { rank, end })
            .collect()
    }

    /// The most admitted intervals through one rank, by brute force.
    fn load(spans: &[Interval], admitted: &[bool], ranks: usize) -> u64 {
        (0..ranks)
            .map(|r| {
                spans
                    .iter()
                    .zip(admitted)
                    .filter(|(s, &a)| a && s.rank <= r && r <= s.end)
                    .count() as u64
            })
            .max()
            .unwrap_or(0)
    }

    #[test]
    fn admission_keeps_every_rank_within_the_budget() {
        let s = spans(&[(0, 5), (1, 2), (2, 9), (3, 3), (6, 8), (7, 7)]);
        assert_eq!(peak(&s, 10), 3);
        assert_eq!(admit(&s, 10, 3), vec![true; 6]);
        // Budget 2: (2, 9) would make three through rank 2; (3, 3) through rank 3 (with
        // (0, 5)) fits once (2, 9) is out.
        assert_eq!(admit(&s, 10, 2), [true, true, false, true, true, true]);
        assert_eq!(admit(&s, 10, 0), vec![false; 6]);
        // Random intervals: within the budget, and greedy (no refused one would fit).
        let mut x = 0x2545f4914f6cdd1du64;
        let mut next = |m: u64| {
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            x % m
        };
        for _ in 0..200 {
            let ranks = 1 + next(60) as usize;
            let mut s: Vec<Interval> = (0..next(40))
                .map(|_| {
                    let rank = next(ranks as u64) as usize;
                    let end = rank + next((ranks - rank) as u64) as usize;
                    Interval { rank, end }
                })
                .collect();
            s.sort_by_key(|i| i.rank);
            let budget = next(6);
            let a = admit(&s, ranks, budget);
            assert!(load(&s, &a, ranks) <= budget);
            assert_eq!(peak(&s, ranks), load(&s, &vec![true; s.len()], ranks));
            for (i, admitted) in a.iter().enumerate() {
                if !admitted {
                    let mut more = a.clone();
                    more[i] = true;
                    let before: Vec<bool> = (0..s.len()).map(|j| j <= i && more[j]).collect();
                    assert!(load(&s, &before, ranks) > budget);
                }
            }
        }
    }
}
