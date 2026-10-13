//! The report's payload rows at archive scale (VS-A).
//!
//! Each first-inspected payload adds one fixed-width row while the traversal runs. At report
//! time the rows are put in URI byte order by a most-significant-chunk external sort: pass 0
//! orders the first 64 name bytes; each later pass orders the next 64 bytes of only those
//! rows whose names tied so far. Every pass is linear in the rows it handles, so the total is
//! linear in the payload name bytes. Rows are then read back one at a time, with names from
//! the catalog, so neither the report nor its serialization holds them all.
use super::{PayloadReport, ValidationFailure};
use crate::archive3tz::{CatalogConfig, Names};
use crate::spill::{
    ExternalSorter, RandomRecords, Record, Scratch, Sequence, SequenceReader, SequenceWriter,
    SortConfig, SpillError,
};
use std::{fmt, sync::Arc};

const CHUNK: usize = 64;

/// One row while its order is being decided: its group (the first final position of the
/// rows it tied with), the name chunk at the current depth, and the row itself.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Pending {
    group: u64,
    chunk: [u8; CHUNK],
    /// Bytes of the name in `chunk` (the rest is zero).
    length: u8,
    /// The name continues past `chunk`.
    more: bool,
    row: Placed,
}

impl Pending {
    fn order(&self) -> (u64, &[u8; CHUNK], u8, bool) {
        (self.group, &self.chunk, self.length, self.more)
    }
}

impl Ord for Pending {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.order()
            .cmp(&other.order())
            .then(self.row.name_ref.cmp(&other.row.name_ref))
    }
}
impl PartialOrd for Pending {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Record for Pending {
    const SIZE: usize = 8 + CHUNK + 2 + Placed::SIZE;
    fn encode(&self, out: &mut [u8]) {
        out[..8].copy_from_slice(&self.group.to_le_bytes());
        out[8..8 + CHUNK].copy_from_slice(&self.chunk);
        out[8 + CHUNK] = self.length;
        out[9 + CHUNK] = u8::from(self.more);
        self.row.encode(&mut out[10 + CHUNK..]);
    }
    fn decode(bytes: &[u8]) -> Self {
        Self {
            group: u64::from_le_bytes(bytes[..8].try_into().unwrap()),
            chunk: bytes[8..8 + CHUNK].try_into().unwrap(),
            length: bytes[8 + CHUNK],
            more: bytes[9 + CHUNK] != 0,
            row: Placed::decode(&bytes[10 + CHUNK..]),
        }
    }
}

/// One payload row: where its name lies in the catalog log, and its counts. `position` is
/// its final place in URI order once decided.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Placed {
    position: u64,
    name_ref: u64,
    name_len: u16,
    accessors: u64,
    primitives: u64,
    vertices: u64,
}

impl Ord for Placed {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        (self.position, self.name_ref).cmp(&(other.position, other.name_ref))
    }
}
impl PartialOrd for Placed {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Record for Placed {
    const SIZE: usize = 42;
    fn encode(&self, out: &mut [u8]) {
        out[..8].copy_from_slice(&self.position.to_le_bytes());
        out[8..16].copy_from_slice(&self.name_ref.to_le_bytes());
        out[16..18].copy_from_slice(&self.name_len.to_le_bytes());
        out[18..26].copy_from_slice(&self.accessors.to_le_bytes());
        out[26..34].copy_from_slice(&self.primitives.to_le_bytes());
        out[34..42].copy_from_slice(&self.vertices.to_le_bytes());
    }
    fn decode(bytes: &[u8]) -> Self {
        let u64_at = |at: usize| u64::from_le_bytes(bytes[at..at + 8].try_into().unwrap());
        Self {
            position: u64_at(0),
            name_ref: u64_at(8),
            name_len: u16::from_le_bytes([bytes[16], bytes[17]]),
            accessors: u64_at(18),
            primitives: u64_at(26),
            vertices: u64_at(34),
        }
    }
}

fn scratch_error(error: SpillError) -> ValidationFailure {
    super::spill_failure(error)
}

/// The chunk of `row`'s name at `depth` (bytes `64·depth..64·depth + 64`), as a pending
/// row of `group`. Only those bytes are read, so a row of name length ℓ costs O(ℓ) over
/// all its passes.
fn chunk_of(names: &Names, row: Placed, depth: usize, group: u64) -> Result<Pending, SpillError> {
    let len = usize::from(row.name_len);
    let start = (depth * CHUNK).min(len);
    let length = (len - start).min(CHUNK);
    let mut chunk = [0; CHUNK];
    names.read_part(row.name_ref, start, &mut chunk[..length])?;
    Ok(Pending {
        group,
        chunk,
        length: length as u8,
        more: len - start > CHUNK,
        row,
    })
}

/// Collects first-inspection payload rows during traversal: 42-byte rows, held in name-log
/// order so that the name chunks are later read sequentially.
pub(super) struct RowSink<'s> {
    sorter: ExternalSorter<'s, Placed>,
    scratch: &'s Scratch,
    /// The first ordering pass runs after the walk, with the catalog's row resources.
    config: SortConfig,
    /// Later passes and the placed rows, live beside the first, use the side resources.
    side: SortConfig,
}

impl<'s> RowSink<'s> {
    pub(super) fn new(
        scratch: &'s Scratch,
        config: &CatalogConfig,
    ) -> Result<Self, ValidationFailure> {
        Ok(Self {
            sorter: ExternalSorter::new(scratch, config.rows_sort).map_err(scratch_error)?,
            scratch,
            config: config.sort,
            side: config.side_sort,
        })
    }

    /// One first-inspected payload, named `name` at `name_ref` in the catalog log.
    pub(super) fn push(
        &mut self,
        name: &str,
        name_ref: u64,
        report: &PayloadReport,
    ) -> Result<(), ValidationFailure> {
        let row = Placed {
            position: 0,
            name_ref,
            name_len: name.len() as u16,
            accessors: report.accessors_checked,
            primitives: report.primitives_checked,
            vertices: report.vertices,
        };
        self.sorter.push(row).map_err(scratch_error)
    }
}

/// Runs the passes: returns every row with its final position, unordered.
fn order<'s>(
    rows: ExternalSorter<'s, Placed>,
    scratch: &'s Scratch,
    config: SortConfig,
    side: SortConfig,
    names: &Names,
) -> Result<ExternalSorter<'s, Placed>, SpillError> {
    let mut pass = ExternalSorter::new(scratch, config)?;
    for row in rows.finish()? {
        pass.push(chunk_of(names, row?, 0, 0)?)?;
    }
    let mut placed = ExternalSorter::new(scratch, side)?;
    for depth in 1.. {
        let mut next = ExternalSorter::<Pending>::new(scratch, side)?;
        let mut tied = 0u64;
        let mut rows = pass.finish()?.peekable();
        // Within one group the rows take the group's positions in sorted order.
        let mut group = None;
        let mut offset = 0u64;
        let mut previous: Option<Pending> = None;
        let mut run_start = 0u64;
        while let Some(row) = rows.next() {
            let row = row?;
            if group != Some(row.group) {
                group = Some(row.group);
                offset = 0;
            }
            let position = row.group + offset;
            let same = |other: &Pending| other.order() == row.order();
            let joins_previous = previous.as_ref().is_some_and(same);
            if !joins_previous {
                run_start = position;
            }
            let joins_next = match rows.peek() {
                Some(Ok(next)) => same(next),
                Some(Err(_)) => return Err(rows.next().unwrap().unwrap_err()),
                None => false,
            };
            if row.more && (joins_previous || joins_next) {
                next.push(chunk_of(names, row.row, depth, run_start)?)?;
                tied += 1;
            } else {
                placed.push(Placed {
                    position,
                    ..row.row
                })?;
            }
            offset += 1;
            previous = Some(row);
        }
        if tied == 0 {
            break;
        }
        pass = next;
    }
    Ok(placed)
}

impl RowSink<'_> {
    /// Decides URI order and stores the rows for reading back. `scratch` is the directory
    /// the rows may live in; the returned rows keep it.
    pub(super) fn finish(
        self,
        names: Names,
        scratch: Arc<Scratch>,
    ) -> Result<PayloadRows, ValidationFailure> {
        let RowSink {
            sorter,
            scratch: borrowed,
            config,
            side,
        } = self;
        let placed = order(sorter, borrowed, config, side, &names).map_err(scratch_error)?;
        let store = Store::new(placed, borrowed, side).map_err(scratch_error)?;
        Ok(PayloadRows {
            inner: Arc::new(Inner {
                store,
                names,
                _scratch: Some(scratch),
            }),
        })
    }
}

enum Store {
    Memory(Vec<Placed>),
    Disk {
        // Declared before `sequence` so the handle closes before the file is removed.
        random: RandomRecords<Placed>,
        sequence: Sequence<Placed>,
    },
}

impl Store {
    fn new(
        placed: ExternalSorter<'_, Placed>,
        scratch: &Scratch,
        config: SortConfig,
    ) -> Result<Self, SpillError> {
        match placed.finish()?.into_sorted_vec() {
            Ok(rows) => Ok(Self::Memory(rows)),
            Err(stream) => {
                let mut writer = SequenceWriter::new(scratch, "payload-rows", config.buffer_bytes)?;
                for row in stream {
                    writer.push(&row?)?;
                }
                let sequence = writer.finish()?;
                Ok(Self::Disk {
                    random: sequence.random()?,
                    sequence,
                })
            }
        }
    }

    fn len(&self) -> u64 {
        match self {
            Self::Memory(rows) => rows.len() as u64,
            Self::Disk { sequence, .. } => sequence.len(),
        }
    }
}

struct Inner {
    store: Store,
    names: Names,
    // Declared last: the store's files are removed before the directory.
    _scratch: Option<Arc<Scratch>>,
}

/// The report's payload rows in URI byte order, read on demand.
///
/// Small reports keep their rows in memory; large ones read them back from the
/// validation's scratch, which lives as long as any clone of the rows.
#[derive(Clone)]
pub struct PayloadRows {
    inner: Arc<Inner>,
}

impl PayloadRows {
    pub fn len(&self) -> usize {
        self.inner.store.len() as usize
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// The row at `index` in URI order, if any.
    pub fn get(&self, index: usize) -> Option<Result<PayloadReport, ValidationFailure>> {
        let row = match &self.inner.store {
            Store::Memory(rows) => Ok(*rows.get(index)?),
            Store::Disk { random, sequence } => {
                if index as u64 >= sequence.len() {
                    return None;
                }
                let mut out = Vec::with_capacity(1);
                random
                    .read(index as u64, 1, &mut Vec::new(), &mut out)
                    .map(|()| out[0])
            }
        };
        Some(
            row.map_err(scratch_error)
                .and_then(|row| self.inner.report(row)),
        )
    }

    /// Every row in URI order. An item is an error only if the validation's own scratch
    /// storage fails.
    pub fn iter(&self) -> PayloadRowsIter<'_> {
        let source = match &self.inner.store {
            Store::Memory(rows) => Source::Memory(rows.iter()),
            Store::Disk { sequence, .. } => match sequence.reader() {
                Ok(reader) => Source::Disk(reader),
                Err(error) => Source::Failed(Some(error)),
            },
        };
        PayloadRowsIter {
            inner: &self.inner,
            source,
        }
    }
}

impl Inner {
    fn report(&self, row: Placed) -> Result<PayloadReport, ValidationFailure> {
        let mut name = Vec::new();
        self.names
            .read(row.name_ref, row.name_len, &mut name)
            .map_err(scratch_error)?;
        Ok(PayloadReport {
            uri: String::from_utf8(name).map_err(|_| {
                ValidationFailure::InvalidState("payload row name is not UTF-8".into())
            })?,
            accessors_checked: row.accessors,
            primitives_checked: row.primitives,
            vertices: row.vertices,
        })
    }
}

enum Source<'a> {
    Memory(std::slice::Iter<'a, Placed>),
    Disk(SequenceReader<'a, Placed>),
    Failed(Option<SpillError>),
}

pub struct PayloadRowsIter<'a> {
    inner: &'a Inner,
    source: Source<'a>,
}

impl Iterator for PayloadRowsIter<'_> {
    type Item = Result<PayloadReport, ValidationFailure>;

    fn next(&mut self) -> Option<Self::Item> {
        let row = match &mut self.source {
            Source::Memory(rows) => Ok(*rows.next()?),
            Source::Disk(reader) => reader.next()?,
            Source::Failed(error) => Err(error.take()?),
        };
        Some(
            row.map_err(scratch_error)
                .and_then(|row| self.inner.report(row)),
        )
    }
}

impl<'a> IntoIterator for &'a PayloadRows {
    type Item = Result<PayloadReport, ValidationFailure>;
    type IntoIter = PayloadRowsIter<'a>;
    fn into_iter(self) -> Self::IntoIter {
        self.iter()
    }
}

impl fmt::Debug for PayloadRows {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PayloadRows")
            .field("len", &self.len())
            .finish()
    }
}

/// Serializes as the JSON array of rows, streamed one row at a time.
impl serde::Serialize for PayloadRows {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        use serde::ser::{Error, SerializeSeq};
        let mut seq = serializer.serialize_seq(Some(self.len()))?;
        for row in self {
            seq.serialize_element(&row.map_err(S::Error::custom)?)?;
        }
        seq.end()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::spill::Placement;

    /// Orders rows whose log (central-directory) order differs from URI order, with ties
    /// at depths 1 and 2 and names that end on a chunk boundary, and returns the URIs.
    fn ordered(names: &[Vec<u8>], config: CatalogConfig) -> Vec<String> {
        let parent = tempfile::tempdir().unwrap();
        let scratch = Arc::new(Scratch::deferred(Placement::In(parent.path().into()), 0));
        let slices: Vec<&[u8]> = names.iter().map(Vec::as_slice).collect();
        let (log, refs) = Names::of(&slices);
        let mut sink = RowSink::new(&scratch, &config).unwrap();
        for (name, &name_ref) in names.iter().zip(&refs) {
            let report = PayloadReport {
                uri: String::from_utf8(name.clone()).unwrap(),
                accessors_checked: name.len() as u64,
                primitives_checked: 1,
                vertices: 3,
            };
            sink.push(&report.uri, name_ref, &report).unwrap();
        }
        let rows = sink.finish(log, scratch.clone()).unwrap();
        rows.iter()
            .map(|row| {
                let row = row.unwrap();
                assert_eq!(row.accessors_checked, row.uri.len() as u64);
                row.uri
            })
            .collect()
    }

    #[test]
    fn deeper_passes_order_rows_whose_log_order_is_not_uri_order() {
        let one = "d".repeat(100);
        let two = format!("{}/{}", "e".repeat(64), "f".repeat(70));
        let mut names: Vec<Vec<u8>> = Vec::new();
        // A deterministic shuffle: 37 is coprime to 50.
        for i in (0..50).map(|i| (i * 37) % 50) {
            names.push(format!("{one}/{i:02}.glb").into_bytes());
            names.push(format!("{two}/{}.glb", 49 - i).into_bytes());
        }
        names.push("e".repeat(64).into_bytes());
        names.push("e".repeat(128).into_bytes());
        names.push(one.clone().into_bytes());
        names.push(b"a.glb".to_vec());
        names.reverse();
        let mut expected: Vec<String> = names
            .iter()
            .map(|name| String::from_utf8(name.clone()).unwrap())
            .collect();
        expected.sort();
        for config in [CatalogConfig::DEFAULT, CatalogConfig::FORCED_SPILL] {
            assert_eq!(ordered(&names, config), expected, "{config:?}");
        }
    }
}
