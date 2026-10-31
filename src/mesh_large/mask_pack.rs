//! Bottom-left packing of cell masks: shapes on a common square-cell grid (coarse charts on
//! their isolation grid, leaf islands on their crop alignment) that collide only where they
//! share a cell, so concave or sparse shapes interlock instead of each taking its bounding
//! rectangle. Exact and deterministic: the placement depends only on the masks and order.
use std::collections::BTreeSet;

/// Lowest bottom-left candidates kept while packing (by row, then column). A scale-gate
/// tuning constant: 1,024 already packs Walem's coarse tiles identically; 256 leaves 410 of
/// its 799 coarse tiles below their requested spacing.
pub(crate) const CANDIDATES: usize = 4096;

/// A mask of `cells[0] x cells[1]` cells: rows of `cells[0].div_ceil(64)` words, bit `x % 64`
/// of word `x / 64` for column `x`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct Mask {
    pub cells: [u32; 2],
    pub bits: Vec<u64>,
}

impl Mask {
    pub(crate) fn new(cells: [u32; 2]) -> Self {
        Self {
            cells,
            bits: vec![0; cells[0].div_ceil(64) as usize * cells[1] as usize],
        }
    }
    fn words(&self) -> usize {
        self.cells[0].div_ceil(64) as usize
    }
    pub(crate) fn set(&mut self, x: u32, y: u32) {
        let w = self.words();
        self.bits[y as usize * w + x as usize / 64] |= 1 << (x % 64);
    }
    pub(crate) fn get(&self, x: u32, y: u32) -> bool {
        self.bits[y as usize * self.words() + x as usize / 64] >> (x % 64) & 1 == 1
    }
    /// Set cells, the packed area.
    pub(crate) fn count(&self) -> u64 {
        self.bits.iter().map(|w| u64::from(w.count_ones())).sum()
    }
    /// Maximal runs of set cells per row: `(row, first column, end column)`.
    pub(crate) fn runs(&self) -> impl Iterator<Item = (u32, u32, u32)> + '_ {
        (0..self.cells[1]).flat_map(move |y| {
            let mut x = 0;
            std::iter::from_fn(move || {
                while x < self.cells[0] && !self.get(x, y) {
                    x += 1;
                }
                if x == self.cells[0] {
                    return None;
                }
                let first = x;
                while x < self.cells[0] && self.get(x, y) {
                    x += 1;
                }
                Some((y, first, x))
            })
        })
    }
    fn row(&self, r: u32) -> &[u64] {
        let w = self.words();
        &self.bits[r as usize * w..(r as usize + 1) * w]
    }
}

/// Bottom-left pack `masks` in `order` into `width` cells: per mask its origin in cells, and
/// the (used width, height), or `None` when a mask is wider or the height exceeds
/// `max_height`. Candidates are the origin, the page top and every placed mask's right and
/// top edges; the first that collides with no set cell is taken.
pub(crate) fn pack(
    masks: &[&Mask],
    order: &[usize],
    width: u32,
    max_height: u32,
) -> Option<(Vec<[u32; 2]>, u32, u32)> {
    let words = width.div_ceil(64) as usize + 1;
    let mut page: Vec<u64> = Vec::new();
    let mut candidates = BTreeSet::from([(0u32, 0u32)]);
    let mut origins = vec![[0u32; 2]; masks.len()];
    let (mut height, mut used) = (0u32, 0u32);
    for &i in order {
        let m = masks[i];
        if m.cells[0] > width {
            return None;
        }
        candidates.insert((height, 0));
        while candidates.len() > CANDIDATES {
            candidates.pop_last();
        }
        let cw = m.words();
        let fits = |x: u32, y: u32| {
            let (ws, bs) = ((x / 64) as usize, x % 64);
            (0..m.cells[1].min(height.saturating_sub(y))).all(|r| {
                let base = (y + r) as usize * words + ws;
                let row = &page[base..base + cw + 1];
                m.row(r).iter().enumerate().all(|(k, &v)| {
                    row[k] & v << bs == 0 && (bs == 0 || row[k + 1] & v >> (64 - bs) == 0)
                })
            })
        };
        let (y, x) = candidates
            .iter()
            .copied()
            .find(|&(y, x)| x + m.cells[0] <= width && fits(x, y))?;
        if y + m.cells[1] > max_height {
            return None;
        }
        page.resize(page.len().max((y + m.cells[1]) as usize * words), 0);
        let (ws, bs) = ((x / 64) as usize, x % 64);
        for r in 0..m.cells[1] {
            let base = (y + r) as usize * words + ws;
            for (k, &v) in m.row(r).iter().enumerate() {
                page[base + k] |= v << bs;
                if bs > 0 {
                    page[base + k + 1] |= v >> (64 - bs);
                }
            }
        }
        height = height.max(y + m.cells[1]);
        used = used.max(x + m.cells[0]);
        candidates.remove(&(y, x));
        candidates.extend([(y, x + m.cells[0]), (y + m.cells[1], x)]);
        origins[i] = [x, y];
    }
    Some((origins, used, height))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mask(rows: &[&str]) -> Mask {
        let mut m = Mask::new([rows[0].len() as u32, rows.len() as u32]);
        for (y, r) in rows.iter().enumerate() {
            for (x, c) in r.chars().enumerate() {
                if c == '#' {
                    m.set(x as u32, y as u32);
                }
            }
        }
        m
    }

    /// Every set cell of every placed mask, checked unshared; the cells per mask.
    fn owners(masks: &[&Mask], origins: &[[u32; 2]], width: u32) -> Vec<u64> {
        let mut owner = std::collections::HashMap::new();
        let mut counts = vec![0u64; masks.len()];
        for (i, m) in masks.iter().enumerate() {
            for (y, x0, x1) in m.runs() {
                for x in x0..x1 {
                    let at = (origins[i][0] + x, origins[i][1] + y);
                    assert!(at.0 < width);
                    assert!(owner.insert(at, i).is_none(), "cell {at:?} shared");
                    counts[i] += 1;
                }
            }
        }
        counts
    }

    #[test]
    fn masks_interlock_where_rectangles_would_collide() {
        // A 2x1 bar, a 1x3 post right of it, then a 3x2 block whose third column is
        // empty: it sits on the bar with its rectangle over the post, which a rectangle
        // packer could not do in a 3-wide page.
        let bar = mask(&["##"]);
        let post = mask(&["#", "#", "#"]);
        let notched = mask(&["##.", "##."]);
        let masks = [&bar, &post, &notched];
        let (origins, w, h) = pack(&masks, &[0, 1, 2], 3, 16).unwrap();
        assert_eq!(origins, vec![[0, 0], [2, 0], [0, 1]]);
        assert_eq!((w, h), (3, 3));
        owners(&masks, &origins, 3);
        // Too narrow or too low refuses.
        assert!(pack(&[&post, &notched], &[0, 1], 2, 16).is_none());
        assert!(pack(&masks, &[0, 1, 2], 3, 2).is_none());
    }

    #[test]
    fn random_masks_never_share_a_cell_and_keep_every_cell() {
        let mut seed = 0x9e37_79b9_7f4a_7c15u64;
        let mut next = move || {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            seed
        };
        for _ in 0..40 {
            let owned: Vec<Mask> = (0..30)
                .map(|_| {
                    let cells = [1 + (next() % 9) as u32, 1 + (next() % 9) as u32];
                    let mut m = Mask::new(cells);
                    for y in 0..cells[1] {
                        for x in 0..cells[0] {
                            if next() % 3 != 0 {
                                m.set(x, y);
                            }
                        }
                    }
                    m
                })
                .collect();
            let masks: Vec<&Mask> = owned.iter().collect();
            let order: Vec<usize> = (0..masks.len()).collect();
            let (origins, used, height) = pack(&masks, &order, 70, 10_000).unwrap();
            let counts = owners(&masks, &origins, 70);
            for (m, n) in masks.iter().zip(counts) {
                assert_eq!(m.count(), n);
            }
            for (m, o) in masks.iter().zip(&origins) {
                assert!(o[0] + m.cells[0] <= used && o[1] + m.cells[1] <= height);
            }
        }
    }

    #[test]
    fn runs_and_counts_follow_the_bits_across_words() {
        let mut m = Mask::new([130, 2]);
        for x in [0, 1, 63, 64, 65, 129] {
            m.set(x, 1);
        }
        assert_eq!(m.count(), 6);
        assert_eq!(
            m.runs().collect::<Vec<_>>(),
            vec![(1, 0, 2), (1, 63, 66), (1, 129, 130)]
        );
        assert!(m.get(64, 1) && !m.get(64, 0));
    }
}
