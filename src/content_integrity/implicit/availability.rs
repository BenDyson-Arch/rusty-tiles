//! §2 and §2a: availability sources, deduplicated by `(view, bits)`, evaluated
//! with one memoized pass per distinct source after the `Scan` charge, then
//! the visit and content-entry reservations, the rank table and the sparse
//! content index. Constant sources are decided in closed form and never read.
use super::subtree::Storage;
use super::{invalid, limit, refused, uint, FormatError, ImplicitLimits};
use serde_json::{Map, Value};
use std::collections::HashMap;

/// The OD-10 message: the scan shares the member-read work counter.
const SCAN_LIMIT: &str = "availability scan exceeds total_bytes_read work limit";

#[cfg(test)]
thread_local! {
    /// Rank-table allocation attempts (the R5b probe).
    pub(super) static RANK_ALLOCATIONS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    /// Distinct-source bit passes of step 3 (the memoization probe).
    pub(super) static SOURCE_PASSES: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

/// One availability object's resolved source.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum Source {
    Const0,
    Const1,
    /// An index into the distinct view sources.
    View(usize),
}

#[derive(Debug)]
struct Distinct {
    view: usize,
    bits: u64,
    popcount: u64,
}

/// The fixed shape of one subtree.
#[derive(Clone, Copy, Debug)]
pub(super) struct Shape {
    /// N: 4 or 8.
    pub(super) branches: u64,
    /// L.
    pub(super) levels: u32,
    /// A.
    pub(super) available: u32,
    /// g: the subtree root's global level.
    pub(super) global: u32,
    pub(super) slots: usize,
}

impl Shape {
    /// `(N^level − 1)/(N − 1)`, the first bit of a local level.
    fn offset(self, level: u32) -> u128 {
        (u128::from(self.branches).pow(level) - 1) / u128::from(self.branches - 1)
    }
}

/// The typed availability facts of one admitted subtree (§1 "typed facts").
#[derive(Debug)]
pub(super) struct Availability {
    shape: Shape,
    /// T, bounded by a view length or by the visit reservation.
    tiles: u64,
    tile: Source,
    child: Source,
    distinct: Vec<Distinct>,
    slot_popcounts: Vec<u64>,
    /// `P_t`, reserved from `hierarchy_visits`.
    pub(super) tile_popcount: u64,
    /// `E`, reserved from `references`.
    pub(super) entries: u64,
    /// Popcount of each preceding 64-bit word (View tile source only).
    rank: Vec<u32>,
    /// Content index: entries of the tile of rank `r` are
    /// `slots[offsets[r]..offsets[r + 1]]`, ascending by slot.
    offsets: Vec<u32>,
    slots: Vec<u16>,
}

fn bit(bytes: &[u8], i: u64) -> bool {
    bytes[(i / 8) as usize] >> (i % 8) & 1 == 1
}

/// Calls `f` for every set bit in `[start, end)`, skipping zero bytes.
fn for_each_set(
    bytes: &[u8],
    start: u64,
    end: u64,
    mut f: impl FnMut(u64) -> Result<(), FormatError>,
) -> Result<(), FormatError> {
    if start >= end {
        return Ok(());
    }
    for byte in start / 8..=(end - 1) / 8 {
        let mut value = bytes[byte as usize];
        while value != 0 {
            let i = byte * 8 + u64::from(value.trailing_zeros());
            value &= value - 1;
            if (start..end).contains(&i) {
                f(i)?;
            }
        }
    }
    Ok(())
}

/// Whether every bit in `[start, end)` is set, byte-wise where aligned.
fn all_set(bytes: &[u8], start: u64, end: u64) -> bool {
    let mut i = start;
    while i < end {
        if i.is_multiple_of(8) && end - i >= 8 {
            if bytes[(i / 8) as usize] != u8::MAX {
                return false;
            }
            i += 8;
        } else {
            if !bit(bytes, i) {
                return false;
            }
            i += 1;
        }
    }
    true
}

impl Availability {
    /// §2a steps 1–11 for one subtree, in the normative order (the step-10
    /// index allocation follows the step-11 rank table, which it uses).
    pub(super) fn admit(
        subtree: &Map<String, Value>,
        storage: &Storage,
        shape: Shape,
        limits: &mut ImplicitLimits,
    ) -> Result<Availability, FormatError> {
        let n = shape.branches;
        let tiles = shape.offset(shape.levels);
        let children = u128::from(n).pow(shape.levels);
        // 1. O(1) per availability object.
        let mut distinct: Vec<Distinct> = Vec::new();
        let mut keys: HashMap<(usize, u64), usize> = HashMap::new();
        let mut object = |value: Option<&Value>,
                          bits: u128,
                          label: &str|
         -> Result<(Source, Option<u64>), FormatError> {
            let value = value
                .and_then(Value::as_object)
                .ok_or_else(|| invalid(format!("{label} must be an object")))?;
            let count = match value.get("availableCount") {
                Some(count) => Some(uint(Some(count), "availableCount")?),
                None => None,
            };
            let source = match (value.get("bitstream"), value.get("constant")) {
                (Some(_), Some(_)) | (None, None) => {
                    return Err(invalid(format!(
                        "{label} needs exactly one of bitstream and constant"
                    )))
                }
                (None, Some(constant)) => match uint(Some(constant), "availability constant")? {
                    0 => Source::Const0,
                    1 => Source::Const1,
                    _ => return Err(invalid("availability constant must be 0 or 1")),
                },
                (Some(bitstream), None) => {
                    let view = storage.view_index(bitstream, "bitstream")?;
                    if storage.view(view).len() as u128 != bits.div_ceil(8) {
                        return Err(invalid(format!(
                            "{label} bitstream byteLength must be ceil({bits}/8)"
                        )));
                    }
                    // Bounded by the view length now.
                    let bits = bits as u64;
                    let next = distinct.len();
                    keys.try_reserve(1).map_err(|_| refused())?;
                    let index = *keys.entry((view, bits)).or_insert(next);
                    if index == next {
                        super::try_push(
                            &mut distinct,
                            Distinct {
                                view,
                                bits,
                                popcount: 0,
                            },
                        )?;
                    }
                    Source::View(index)
                }
            };
            Ok((source, count))
        };
        let tile = object(subtree.get("tileAvailability"), tiles, "tileAvailability")?;
        if tile.0 == Source::Const0 {
            return Err(invalid("tileAvailability.constant 0 is disallowed"));
        }
        let mut contents = Vec::new();
        match subtree.get("contentAvailability") {
            None if shape.slots == 0 => {}
            Some(Value::Array(layers)) if layers.len() == shape.slots => {
                contents
                    .try_reserve_exact(layers.len())
                    .map_err(|_| refused())?;
                for layer in layers {
                    contents.push(object(Some(layer), tiles, "contentAvailability")?);
                }
            }
            _ => return Err(invalid("content availability count differs from templates")),
        }
        let child = object(
            subtree.get("childSubtreeAvailability"),
            children,
            "childSubtreeAvailability",
        )?;
        // 2. The work charge, before any bit pass.
        let scan = distinct
            .iter()
            .map(|d| storage.view(d.view).len() as u128)
            .sum::<u128>();
        if scan > u128::from(limits.scan_remaining) {
            return Err(FormatError::ResourceLimit(SCAN_LIMIT.into()));
        }
        limits.scan_remaining -= scan as u64;
        // 3. One pass per distinct view source: trailing zeros and popcount.
        for source in &mut distinct {
            #[cfg(test)]
            SOURCE_PASSES.with(|n| n.set(n.get() + 1));
            let bytes = storage.view(source.view);
            let tail = (source.bits % 8) as u32;
            if tail != 0 && bytes[bytes.len() - 1] >> tail != 0 {
                return Err(invalid("nonzero trailing availability bits"));
            }
            source.popcount = bytes.iter().map(|b| u64::from(b.count_ones())).sum();
        }
        let popcount = |source: Source, bits: u128| match source {
            Source::Const0 => 0,
            Source::Const1 => bits,
            Source::View(d) => u128::from(distinct[d].popcount),
        };
        // 4. availableCount per object, against its memoized source.
        for ((source, count), bits) in std::iter::once((tile, tiles))
            .chain(contents.iter().map(|c| (*c, tiles)))
            .chain(std::iter::once((child, children)))
        {
            if count.is_some_and(|count| u128::from(count) != popcount(source, bits)) {
                return Err(invalid(
                    "availableCount differs from the availability popcount",
                ));
            }
        }
        let (tile, child) = (tile.0, child.0);
        let view_bytes = |source: Source| match source {
            Source::View(d) => Some(storage.view(distinct[d].view)),
            _ => None,
        };
        let offset = |level: u32| shape.offset(level) as u64;
        // 5. Root bit and parent rule (closed form for Const1).
        if let Some(bytes) = view_bytes(tile) {
            if !bit(bytes, 0) {
                return Err(invalid("root tile of the subtree is unavailable"));
            }
            for level in 1..shape.levels {
                let start = offset(level);
                let parent = offset(level - 1);
                for_each_set(bytes, start, offset(level + 1), |i| {
                    if bit(bytes, parent + (i - start) / n) {
                        Ok(())
                    } else {
                        Err(invalid("tile has an unavailable parent"))
                    }
                })?;
            }
        }
        // 6. Levels: nothing available at global level ≥ A.
        let beyond = shape.available.saturating_sub(shape.global);
        if beyond < shape.levels {
            match view_bytes(tile) {
                None => return Err(invalid("available tile beyond availableLevels")),
                Some(bytes) => for_each_set(bytes, offset(beyond), tiles as u64, |_| {
                    Err(invalid("available tile beyond availableLevels"))
                })?,
            }
        }
        if popcount(child, children) > 0
            && u64::from(shape.global) + u64::from(shape.levels) >= u64::from(shape.available)
        {
            return Err(invalid("child subtree beyond availableLevels"));
        }
        // 7. content ⇒ tile, once per distinct content source.
        let tile_popcount = popcount(tile, tiles);
        let mut checked = Vec::new();
        checked
            .try_reserve_exact(distinct.len())
            .map_err(|_| refused())?;
        checked.resize(distinct.len(), false);
        let mut checked_const1 = false;
        for &(source, _) in &contents {
            let seen = match source {
                Source::Const0 => continue,
                Source::Const1 => &mut checked_const1,
                Source::View(d) => &mut checked[d],
            };
            if std::mem::replace(seen, true) {
                continue;
            }
            let holds = match (source, view_bytes(tile)) {
                (_, None) => true,
                (Source::Const1, Some(_)) => tile_popcount == tiles,
                (Source::View(_), Some(_)) if source == tile => true,
                (Source::View(d), Some(tile_bytes)) => storage
                    .view(distinct[d].view)
                    .iter()
                    .zip(tile_bytes)
                    .all(|(c, t)| c & !t == 0),
                (Source::Const0, Some(_)) => true,
            };
            if !holds {
                return Err(invalid("content belongs to an unavailable tile"));
            }
        }
        drop(checked);
        // 8. child ⇒ tile.
        if let Some(tile_bytes) = view_bytes(tile) {
            let last = offset(shape.levels - 1);
            match child {
                Source::Const0 => {}
                Source::Const1 => {
                    if !all_set(tile_bytes, last, tiles as u64) {
                        return Err(invalid("child subtree has an unavailable parent"));
                    }
                }
                Source::View(d) => {
                    let bits = distinct[d].bits;
                    for_each_set(storage.view(distinct[d].view), 0, bits, |m| {
                        if bit(tile_bytes, last + m / n) {
                            Ok(())
                        } else {
                            Err(invalid("child subtree has an unavailable parent"))
                        }
                    })?;
                }
            }
        }
        // 9. Visit reservation.
        if tile_popcount > u128::from(limits.visits_remaining) {
            return Err(limit(
                "subtree tile availability exceeds remaining hierarchy visits",
            ));
        }
        limits.visits_remaining -= tile_popcount as u64;
        let tile_popcount = tile_popcount as u64;
        // T ≤ P_t for Const1, else bounded by the view length.
        let tiles = tiles as u64;
        // 10. Content-entry reservation (DC-3), before any index allocation.
        let mut slot_popcounts = Vec::new();
        slot_popcounts
            .try_reserve_exact(contents.len())
            .map_err(|_| refused())?;
        slot_popcounts.extend(
            contents
                .iter()
                .map(|(source, _)| popcount(*source, u128::from(tiles)) as u64),
        );
        let entries = slot_popcounts.iter().map(|p| u128::from(*p)).sum::<u128>();
        if entries > u128::from(limits.refs_remaining) {
            return Err(limit("subtree content entries exceed remaining references"));
        }
        limits.refs_remaining -= entries as u64;
        let entries = entries as u64;
        let tile_view = view_bytes(tile);
        if entries > u64::from(u32::MAX)
            || (tiles > u64::from(u32::MAX) && (entries > 0 || tile_view.is_some()))
        {
            return Err(limit("subtree exceeds host index range"));
        }
        let mut availability = Availability {
            shape,
            tiles,
            tile,
            child,
            distinct,
            slot_popcounts,
            tile_popcount,
            entries,
            rank: Vec::new(),
            offsets: Vec::new(),
            slots: Vec::new(),
        };
        // 11. Rank table, View tile source only.
        if let Some(bytes) = tile_view {
            #[cfg(test)]
            RANK_ALLOCATIONS.with(|n| n.set(n.get() + 1));
            let words = tiles.div_ceil(64) as usize;
            availability
                .rank
                .try_reserve_exact(words)
                .map_err(|_| refused())?;
            let mut total = 0u32;
            for word in bytes.chunks(8) {
                availability.rank.push(total);
                total += word.iter().map(|b| b.count_ones()).sum::<u32>();
            }
        }
        // 10 (allocation). The sparse index: each distinct source's set bits
        // are extracted once and shared by its aliasing slots.
        if entries > 0 {
            availability.index(&contents, storage)?;
        }
        Ok(availability)
    }

    fn index(
        &mut self,
        contents: &[(Source, Option<u64>)],
        storage: &Storage,
    ) -> Result<(), FormatError> {
        let mut used = Vec::new();
        used.try_reserve_exact(self.distinct.len())
            .map_err(|_| refused())?;
        used.resize(self.distinct.len(), false);
        for (source, _) in contents {
            if let Source::View(d) = source {
                used[*d] = true;
            }
        }
        let mut positions: Vec<Vec<u32>> = Vec::new();
        positions
            .try_reserve_exact(self.distinct.len())
            .map_err(|_| refused())?;
        for (source, used) in self.distinct.iter().zip(used) {
            let mut set = Vec::new();
            if used {
                set.try_reserve_exact(source.popcount as usize)
                    .map_err(|_| refused())?;
                for_each_set(storage.view(source.view), 0, source.bits, |i| {
                    set.push(i as u32);
                    Ok(())
                })?;
            }
            positions.push(set);
        }
        let ranks = self.tile_popcount as usize;
        self.offsets
            .try_reserve_exact(ranks + 1)
            .map_err(|_| refused())?;
        self.offsets.resize(ranks + 1, 0);
        self.slots
            .try_reserve_exact(self.entries as usize)
            .map_err(|_| refused())?;
        self.slots.resize(self.entries as usize, 0);
        let tile_bytes = match self.tile {
            Source::View(d) => Some(storage.view(self.distinct[d].view)),
            _ => None,
        };
        let rank = |i: u64| rank(&self.rank, tile_bytes, i) as usize;
        let tiles = self.tiles;
        let each = |source: Source, f: &mut dyn FnMut(u64)| match source {
            Source::Const0 => {}
            Source::Const1 => (0..tiles).for_each(f),
            Source::View(d) => positions[d].iter().for_each(|i| f(u64::from(*i))),
        };
        // Count per tile rank, then inclusive prefix sums, then fill slots in
        // descending order from each tile's end: entries ascend by slot.
        let offsets = &mut self.offsets;
        for (source, _) in contents {
            each(*source, &mut |i| offsets[rank(i)] += 1);
        }
        let mut total = 0u32;
        for offset in offsets[..ranks].iter_mut() {
            total += *offset;
            *offset = total;
        }
        offsets[ranks] = total;
        let slots = &mut self.slots;
        for (slot, (source, _)) in contents.iter().enumerate().rev() {
            each(*source, &mut |i| {
                let r = rank(i);
                offsets[r] -= 1;
                slots[offsets[r] as usize] = slot as u16;
            });
        }
        Ok(())
    }

    /// Whether local tile bit `i < T` is available.
    pub(super) fn tile(&self, storage: &Storage, i: u64) -> bool {
        match self.tile {
            Source::Const0 => false,
            Source::Const1 => i < self.tiles,
            Source::View(d) => i < self.tiles && bit(storage.view(self.distinct[d].view), i),
        }
    }

    /// Whether child subtree bit `m` is available. Never enumerates `C`.
    pub(super) fn child(&self, storage: &Storage, m: u128) -> bool {
        match self.child {
            Source::Const0 => false,
            Source::Const1 => true,
            Source::View(d) => {
                let source = &self.distinct[d];
                m < u128::from(source.bits) && bit(storage.view(source.view), m as u64)
            }
        }
    }

    /// `rank(i) = popcount(tile[0..i))`, strict, O(1).
    pub(super) fn rank(&self, storage: &Storage, i: u64) -> u64 {
        let bytes = match self.tile {
            Source::View(d) => Some(storage.view(self.distinct[d].view)),
            _ => None,
        };
        rank(&self.rank, bytes, i)
    }

    /// The emitted slots of the tile of rank `r`, ascending.
    pub(super) fn entries(&self, r: u64) -> &[u16] {
        if self.slots.is_empty() {
            return &[];
        }
        let r = r as usize;
        &self.slots[self.offsets[r] as usize..self.offsets[r + 1] as usize]
    }

    pub(super) fn slot_popcount(&self, slot: usize) -> u64 {
        self.slot_popcounts[slot]
    }

    pub(super) fn shape(&self) -> Shape {
        self.shape
    }

    pub(super) fn first_bit(&self, level: u32) -> u64 {
        self.shape.offset(level) as u64
    }
}

fn rank(table: &[u32], bytes: Option<&[u8]>, i: u64) -> u64 {
    let Some(bytes) = bytes else {
        return i;
    };
    let word = (i / 64) as usize;
    let mut rank = u64::from(table[word]);
    let first = word * 8;
    let byte = (i / 8) as usize;
    for b in &bytes[first..byte] {
        rank += u64::from(b.count_ones());
    }
    let partial = (i % 8) as u32;
    if partial > 0 {
        rank += u64::from((bytes[byte] & ((1u8 << partial) - 1)).count_ones());
    }
    rank
}
