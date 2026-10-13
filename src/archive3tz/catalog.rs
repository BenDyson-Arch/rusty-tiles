//! The admitted catalog at archive scale (VS-A).
//!
//! Every admitted member becomes one fixed-width [`Row`], sorted by its [`Key`]: the
//! `@3dtilesIndex1@` order of the name's MD5 (two little-endian `u64` halves), then a
//! secondary hash of the name. A row's position in that order is its **rank**. Names and the
//! central-directory-order facts the hash pass needs go to an append-only log.
//!
//! Small catalogs stay in memory. A catalog whose estimate exceeds
//! [`CatalogConfig::resident_bytes`] keeps its rows and log on deferred scratch: rows are
//! found through an in-memory fence table (one key per page of `k` rows, at most
//! [`CatalogConfig::max_fences`] keys) and a direct-mapped page cache of at most
//! [`CatalogConfig::cache_bytes`]. Resident memory is then a configuration constant.
use crate::spill::{
    ByteFile, ByteWriter, ExternalSorter, RandomRecords, Record, Scratch, Sequence, SequenceWriter,
    SortConfig, SortedStream, SpillError,
};
use sha2::{Digest, Sha256};
use std::{io::Read, sync::Arc};

/// Resources of one catalog. Only the forced-spill profile differs from the default.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct CatalogConfig {
    /// External sorts of rows, extents and index rows.
    pub sort: SortConfig,
    /// External sorts of the physical extents (live beside the row sort) and of a failing
    /// index's rows and pairs.
    pub side_sort: SortConfig,
    /// External sorts of the report's payload rows (live during traversal).
    pub rows_sort: SortConfig,
    /// Rows and log stay in memory when `N·(64 + 22) + centralDirectoryBytes` fits.
    pub resident_bytes: u64,
    /// Fence keys of an on-disk catalog (24 B each).
    pub max_fences: u64,
    /// Page cache of an on-disk catalog. At least one page is always cached.
    pub cache_bytes: u64,
    /// Buffer of each scratch log or sequence writer and reader.
    pub buffer_bytes: usize,
    /// The validator's per-URI facts (131 B per entry) stay in memory when `N·131` fits
    /// (VS-B-5); otherwise they go to a rank-indexed scratch file.
    pub facts_resident_bytes: u64,
}

impl CatalogConfig {
    /// Design §1.3/§1.4: `R` = 64 MiB, `F` = 32, `b` = 1 MiB. A catalog stays resident up
    /// to 96 MiB, which covers every archive within the default entry (2^19) and
    /// central-directory (32 MiB) limits: at most 2^19·86 B + 32 MiB ≈ 77 MiB. Larger
    /// catalogs use 2^19 fences (12 MiB) and a 16 MiB page cache. Payload rows collect in a
    /// 32 MiB run (2^19 rows of 48 B) before they spill. Extents and index-diagnosis rows
    /// sort in 24 MiB runs, so the build holds at most `1.5·64 + 1.5·24` MiB of runs.
    pub(crate) const DEFAULT: Self = Self {
        sort: SortConfig {
            run_bytes: 64 << 20,
            fan_in: 32,
            buffer_bytes: 1 << 20,
            jobs: 1,
        },
        side_sort: SortConfig {
            run_bytes: 24 << 20,
            fan_in: 32,
            buffer_bytes: 256 << 10,
            jobs: 1,
        },
        rows_sort: SortConfig {
            run_bytes: 32 << 20,
            fan_in: 16,
            buffer_bytes: 256 << 10,
            jobs: 1,
        },
        resident_bytes: 96 << 20,
        max_fences: 1 << 19,
        cache_bytes: 16 << 20,
        buffer_bytes: 256 << 10,
        // 2^19 entries (the default entry limit) need 65.5 MiB.
        facts_resident_bytes: 72 << 20,
    };

    /// Every store on scratch, runs of a few records, two fences and one cached page: the
    /// parity profile that exercises every spilled path on small archives.
    #[cfg_attr(not(any(test, feature = "validate-forced-spill")), allow(dead_code))]
    pub(crate) const FORCED_SPILL: Self = Self {
        sort: SortConfig {
            run_bytes: 4 * size_of::<Row>(),
            fan_in: 2,
            buffer_bytes: 64,
            jobs: 1,
        },
        side_sort: SortConfig {
            run_bytes: 4 * 32,
            fan_in: 2,
            buffer_bytes: 64,
            jobs: 1,
        },
        rows_sort: SortConfig {
            run_bytes: 4 * 112,
            fan_in: 2,
            buffer_bytes: 64,
            jobs: 1,
        },
        resident_bytes: 0,
        max_fences: 2,
        cache_bytes: 0,
        buffer_bytes: 64,
        facts_resident_bytes: 0,
    };

    /// The profile of this build. Test builds and `validate-forced-spill` builds select
    /// [`CatalogConfig::FORCED_SPILL`] when `RUSTY_TILES_FORCED_SPILL=1` (or, in unit
    /// tests, through the thread override); production builds always use the default.
    pub(crate) fn selected() -> Self {
        #[cfg(any(test, feature = "validate-forced-spill"))]
        {
            #[cfg(test)]
            if let Some(forced) = FORCED.with(std::cell::Cell::get) {
                return if forced {
                    Self::FORCED_SPILL
                } else {
                    Self::DEFAULT
                };
            }
            if std::env::var_os("RUSTY_TILES_FORCED_SPILL").is_some_and(|v| v == "1") {
                return Self::FORCED_SPILL;
            }
        }
        Self::DEFAULT
    }

    /// Scratch estimate from the end record (design §1.3, labelled an estimate): two
    /// generations of row runs and the log, plus two of extents and of index rows.
    pub(crate) fn scratch_estimate(entries: u64, directory_bytes: u64) -> u64 {
        let rows = entries.saturating_mul(Row::SIZE as u64 + LOG_HEADER as u64);
        let sides = entries.saturating_mul(Extent32::SIZE as u64 + IndexRow::SIZE as u64);
        rows.saturating_add(directory_bytes)
            .saturating_mul(2)
            .saturating_add(sides.saturating_mul(2))
    }
}

#[cfg(test)]
thread_local! {
    /// Unit-test override of the catalog profile on this thread.
    pub(crate) static FORCED: std::cell::Cell<Option<bool>> = const { std::cell::Cell::new(None) };
}

/// Size stand-in for the extent record in the scratch estimate.
struct Extent32;
impl Extent32 {
    const SIZE: usize = 32;
}

/// Sort key of a name: the index order of its MD5, then 64 bits of its SHA-256. Equal keys
/// are compared by name, so the secondary hash only keeps MD5-colliding groups small.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(crate) struct Key {
    pub md5: [u64; 2],
    pub h2: u64,
}

impl Key {
    pub(crate) fn of(name: &[u8]) -> Self {
        let digest = md5::compute(name).0;
        let h2 = Sha256::digest(name);
        Self {
            md5: md5_key(&digest),
            h2: u64::from_le_bytes(h2[..8].try_into().unwrap()),
        }
    }
}

/// The `@3dtilesIndex1@` sort key of an MD5: two little-endian `u64` halves.
pub(crate) fn md5_key(digest: &[u8]) -> [u64; 2] {
    [
        u64::from_le_bytes(digest[..8].try_into().unwrap()),
        u64::from_le_bytes(digest[8..16].try_into().unwrap()),
    ]
}

/// One admitted member, as stored in the catalog.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Row {
    pub key: Key,
    /// Offset of the name bytes in the log.
    pub name_ref: u64,
    pub name_len: u16,
    pub size: u64,
    pub data_offset: u64,
    pub local_offset: u64,
    pub crc32: u32,
}

impl Ord for Row {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        (self.key, self.local_offset, self.name_ref).cmp(&(
            other.key,
            other.local_offset,
            other.name_ref,
        ))
    }
}
impl PartialOrd for Row {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

fn put(out: &mut [u8], at: &mut usize, bytes: &[u8]) {
    out[*at..*at + bytes.len()].copy_from_slice(bytes);
    *at += bytes.len();
}
fn take<const N: usize>(bytes: &[u8], at: &mut usize) -> [u8; N] {
    let value = bytes[*at..*at + N].try_into().unwrap();
    *at += N;
    value
}
pub(crate) fn take_u64(bytes: &[u8], at: &mut usize) -> u64 {
    u64::from_le_bytes(take(bytes, at))
}

impl Record for Row {
    const SIZE: usize = 62;
    fn encode(&self, out: &mut [u8]) {
        let at = &mut 0;
        put(out, at, &self.key.md5[0].to_le_bytes());
        put(out, at, &self.key.md5[1].to_le_bytes());
        put(out, at, &self.key.h2.to_le_bytes());
        put(out, at, &self.name_ref.to_le_bytes());
        put(out, at, &self.name_len.to_le_bytes());
        put(out, at, &self.size.to_le_bytes());
        put(out, at, &self.data_offset.to_le_bytes());
        put(out, at, &self.local_offset.to_le_bytes());
        put(out, at, &self.crc32.to_le_bytes());
    }
    fn decode(bytes: &[u8]) -> Self {
        let at = &mut 0;
        Self {
            key: Key {
                md5: [take_u64(bytes, at), take_u64(bytes, at)],
                h2: take_u64(bytes, at),
            },
            name_ref: take_u64(bytes, at),
            name_len: u16::from_le_bytes(take(bytes, at)),
            size: take_u64(bytes, at),
            data_offset: take_u64(bytes, at),
            local_offset: take_u64(bytes, at),
            crc32: u32::from_le_bytes(take(bytes, at)),
        }
    }
}

/// One `@3dtilesIndex1@` row at its index position, for the exact diagnosis of a failing
/// index: sorted by `(md5, offset, position)`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct IndexRow {
    pub md5: [u64; 2],
    pub offset: u64,
    pub position: u64,
}

impl Record for IndexRow {
    const SIZE: usize = 32;
    fn encode(&self, out: &mut [u8]) {
        let at = &mut 0;
        put(out, at, &self.md5[0].to_le_bytes());
        put(out, at, &self.md5[1].to_le_bytes());
        put(out, at, &self.offset.to_le_bytes());
        put(out, at, &self.position.to_le_bytes());
    }
    fn decode(bytes: &[u8]) -> Self {
        let at = &mut 0;
        Self {
            md5: [take_u64(bytes, at), take_u64(bytes, at)],
            offset: take_u64(bytes, at),
            position: take_u64(bytes, at),
        }
    }
}

/// What a lookup returns: the member's rank and its admitted stored range.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Member {
    pub rank: u64,
    pub size: u64,
    pub data_offset: u64,
    pub crc32: u32,
    /// Where the name lies in [`Names`].
    pub name_ref: u64,
    pub name_len: u16,
}

/// Bytes of one log entry before its name: size, data offset, CRC and name length.
pub(crate) const LOG_HEADER: usize = 22;

/// The name/directory log: one entry per member in central-directory order.
pub(crate) enum Log {
    Memory(Vec<u8>),
    Disk(ByteFile),
}

pub(crate) enum LogWriter {
    Memory(Vec<u8>),
    Disk(ByteWriter),
}

impl LogWriter {
    pub(crate) fn new(
        scratch: &Scratch,
        memory: bool,
        config: &CatalogConfig,
    ) -> Result<Self, SpillError> {
        Ok(if memory {
            Self::Memory(Vec::new())
        } else {
            Self::Disk(ByteWriter::new(
                scratch,
                "catalog-log",
                config.buffer_bytes,
            )?)
        })
    }

    fn len(&self) -> u64 {
        match self {
            Self::Memory(bytes) => bytes.len() as u64,
            Self::Disk(writer) => writer.len(),
        }
    }

    /// Appends one directory entry; returns the offset of its name bytes.
    pub(crate) fn append(
        &mut self,
        name: &[u8],
        size: u64,
        data_offset: u64,
        crc32: u32,
    ) -> Result<u64, SpillError> {
        let mut header = [0u8; LOG_HEADER];
        let at = &mut 0;
        put(&mut header, at, &size.to_le_bytes());
        put(&mut header, at, &data_offset.to_le_bytes());
        put(&mut header, at, &crc32.to_le_bytes());
        put(&mut header, at, &(name.len() as u16).to_le_bytes());
        let name_ref = self
            .len()
            .checked_add(LOG_HEADER as u64)
            .ok_or(SpillError::Overflow)?;
        match self {
            Self::Memory(bytes) => {
                bytes
                    .try_reserve(LOG_HEADER + name.len())
                    .map_err(|_| SpillError::Overflow)?;
                bytes.extend_from_slice(&header);
                bytes.extend_from_slice(name);
            }
            Self::Disk(writer) => {
                writer.append(&header)?;
                writer.append(name)?;
            }
        }
        Ok(name_ref)
    }

    pub(crate) fn finish(self) -> Result<Log, SpillError> {
        Ok(match self {
            Self::Memory(mut bytes) => {
                bytes.shrink_to_fit();
                Log::Memory(bytes)
            }
            Self::Disk(writer) => Log::Disk(writer.finish()?),
        })
    }
}

impl Log {
    fn read(&self, offset: u64, out: &mut [u8]) -> Result<(), SpillError> {
        match self {
            Self::Memory(bytes) => {
                let range = usize::try_from(offset)
                    .ok()
                    .and_then(|start| Some(start..start.checked_add(out.len())?))
                    .filter(|range| range.end <= bytes.len())
                    .ok_or_else(|| SpillError::CorruptRun {
                        path: Default::default(),
                        detail: format!("name at {offset} outside the catalog log"),
                    })?;
                out.copy_from_slice(&bytes[range]);
                Ok(())
            }
            Self::Disk(file) => file.read_at(offset, out),
        }
    }
}

/// Shared read access to the member names of one catalog (the report's payload rows
/// outlive the archive reader).
#[derive(Clone)]
pub(crate) struct Names(Arc<Log>);

impl Names {
    pub(crate) fn from_log(log: Log) -> Self {
        Self(Arc::new(log))
    }

    /// The name at `name_ref`, into `out`.
    pub(crate) fn read(
        &self,
        name_ref: u64,
        name_len: u16,
        out: &mut Vec<u8>,
    ) -> Result<(), SpillError> {
        out.resize(usize::from(name_len), 0);
        self.0.read(name_ref, out)
    }

    /// `out.len()` bytes of the name at `name_ref`, starting `skip` bytes in; the range
    /// must lie within the name.
    pub(crate) fn read_part(
        &self,
        name_ref: u64,
        skip: usize,
        out: &mut [u8],
    ) -> Result<(), SpillError> {
        self.0.read(name_ref + skip as u64, out)
    }

    /// A resident log of `names` in the given order, and each name's offset.
    #[cfg(test)]
    pub(crate) fn of(names: &[&[u8]]) -> (Self, Vec<u64>) {
        let mut log = LogWriter::Memory(Vec::new());
        let refs = names
            .iter()
            .map(|name| log.append(name, 0, 0, 0).unwrap())
            .collect();
        (Self::from_log(log.finish().unwrap()), refs)
    }

    /// Central-directory-order entries, read sequentially.
    pub(crate) fn directory(
        &self,
        entries: u64,
        buffer_bytes: usize,
    ) -> Result<Directory, SpillError> {
        let source = match &*self.0 {
            Log::Memory(_) => DirectorySource::Memory(self.0.clone(), 0),
            Log::Disk(file) => DirectorySource::Disk(file.reader(buffer_bytes)?),
        };
        Ok(Directory {
            source,
            remaining: entries,
        })
    }

    fn corrupt(&self, detail: String) -> SpillError {
        SpillError::CorruptRun {
            path: match &*self.0 {
                Log::Memory(_) => Default::default(),
                Log::Disk(file) => file.path().to_owned(),
            },
            detail,
        }
    }
}

/// One member in central-directory order, for the hash pass.
pub(crate) struct DirectoryEntry {
    pub name: String,
    pub size: u64,
    pub data_offset: u64,
    pub crc32: u32,
}

enum DirectorySource {
    Memory(Arc<Log>, usize),
    Disk(std::io::BufReader<Box<dyn Read + Send>>),
}

/// Sequential reader of the log in central-directory order.
pub(crate) struct Directory {
    source: DirectorySource,
    remaining: u64,
}

impl Directory {
    pub(crate) fn next_entry(&mut self) -> Result<Option<DirectoryEntry>, SpillError> {
        if self.remaining == 0 {
            return Ok(None);
        }
        let mut header = [0u8; LOG_HEADER];
        let name = match &mut self.source {
            DirectorySource::Memory(log, at) => {
                let Log::Memory(bytes) = &**log else {
                    unreachable!("memory directory over a memory log")
                };
                let end = *at + LOG_HEADER;
                header.copy_from_slice(bytes.get(*at..end).ok_or_else(truncated_log)?);
                let len = usize::from(u16::from_le_bytes([header[20], header[21]]));
                let name = bytes
                    .get(end..end + len)
                    .ok_or_else(truncated_log)?
                    .to_vec();
                *at = end + len;
                name
            }
            DirectorySource::Disk(reader) => {
                reader
                    .read_exact(&mut header)
                    .map_err(|e| SpillError::io("read catalog log", "", e))?;
                let len = usize::from(u16::from_le_bytes([header[20], header[21]]));
                let mut name = vec![0; len];
                reader
                    .read_exact(&mut name)
                    .map_err(|e| SpillError::io("read catalog log", "", e))?;
                name
            }
        };
        self.remaining -= 1;
        let at = &mut 0;
        let size = take_u64(&header, at);
        let data_offset = take_u64(&header, at);
        let crc32 = u32::from_le_bytes(take(&header, at));
        let name = String::from_utf8(name).map_err(|_| SpillError::CorruptRun {
            path: Default::default(),
            detail: "catalog log name is not UTF-8".into(),
        })?;
        Ok(Some(DirectoryEntry {
            name,
            size,
            data_offset,
            crc32,
        }))
    }
}

fn truncated_log() -> SpillError {
    SpillError::CorruptRun {
        path: Default::default(),
        detail: "catalog log ends inside an entry".into(),
    }
}

/// The sorted rows: a vector, or a scratch sequence with fences and a page cache.
pub(crate) enum Rows {
    Memory(Vec<Row>),
    Disk(DiskRows),
}

pub(crate) struct DiskRows {
    // Declared before `sequence` so the handle closes before the file is removed.
    random: RandomRecords<Row>,
    sequence: Sequence<Row>,
    /// Key of the first row of each page.
    fences: Vec<Key>,
    /// Rows per page.
    page: u64,
    len: u64,
    /// Direct-mapped cache: slot `p % slots` holds page `p`.
    slots: Vec<Option<(u64, Vec<Row>)>>,
    encoded: Vec<u8>,
    buffer_bytes: usize,
}

/// Builds [`Rows`] from rows given in sorted order.
pub(crate) enum RowsWriter {
    Memory(Vec<Row>),
    Disk {
        writer: SequenceWriter<Row>,
        fences: Vec<Key>,
        page: u64,
        cache_bytes: u64,
        buffer_bytes: usize,
    },
}

impl RowsWriter {
    pub(crate) fn new(
        scratch: &Scratch,
        memory: bool,
        entries: u64,
        config: &CatalogConfig,
    ) -> Result<Self, SpillError> {
        if memory {
            return Ok(Self::Memory(Vec::new()));
        }
        let page = entries.div_ceil(config.max_fences.max(1)).max(1);
        Ok(Self::Disk {
            writer: SequenceWriter::new(scratch, "catalog-rows", config.buffer_bytes)?,
            fences: Vec::new(),
            page,
            cache_bytes: config.cache_bytes,
            buffer_bytes: config.buffer_bytes,
        })
    }

    pub(crate) fn push(&mut self, row: Row) -> Result<(), SpillError> {
        match self {
            Self::Memory(rows) => {
                rows.try_reserve(1).map_err(|_| SpillError::Overflow)?;
                rows.push(row);
            }
            Self::Disk {
                writer,
                fences,
                page,
                ..
            } => {
                if writer.len() % *page == 0 {
                    fences.try_reserve(1).map_err(|_| SpillError::Overflow)?;
                    fences.push(row.key);
                }
                writer.push(&row)?;
            }
        }
        Ok(())
    }

    pub(crate) fn finish(self) -> Result<Rows, SpillError> {
        Ok(match self {
            Self::Memory(rows) => Rows::Memory(rows),
            Self::Disk {
                writer,
                fences,
                page,
                cache_bytes,
                buffer_bytes,
            } => {
                let sequence = writer.finish()?;
                let page_bytes = page.saturating_mul(Row::SIZE as u64);
                let slots = usize::try_from((cache_bytes / page_bytes).max(1)).unwrap_or(1);
                Rows::Disk(DiskRows {
                    random: sequence.random()?,
                    len: sequence.len(),
                    sequence,
                    fences,
                    page,
                    slots: (0..slots).map(|_| None).collect(),
                    encoded: Vec::new(),
                    buffer_bytes,
                })
            }
        })
    }
}

/// Sorts rows on deferred scratch, then stores them for lookup.
pub(crate) fn store_sorted(
    sorted: SortedStream<'_, Row>,
    writer: RowsWriter,
    mut visit: impl FnMut(&Row) -> Result<(), SpillError>,
) -> Result<Rows, SpillError> {
    match writer {
        RowsWriter::Memory(_) => {
            let rows = match sorted.into_sorted_vec() {
                Ok(rows) => rows,
                Err(stream) => stream.collect::<Result<Vec<_>, _>>()?,
            };
            rows.iter().try_for_each(&mut visit)?;
            Ok(Rows::Memory(rows))
        }
        mut writer => {
            for row in sorted {
                let row = row?;
                visit(&row)?;
                writer.push(row)?;
            }
            writer.finish()
        }
    }
}

impl Rows {
    pub(crate) fn len(&self) -> u64 {
        match self {
            Self::Memory(rows) => rows.len() as u64,
            Self::Disk(disk) => disk.len,
        }
    }

    pub(crate) fn get(&mut self, rank: u64) -> Result<Row, SpillError> {
        match self {
            Self::Memory(rows) => usize::try_from(rank)
                .ok()
                .and_then(|rank| rows.get(rank).copied())
                .ok_or(SpillError::InvalidConfig("catalog rank out of range")),
            Self::Disk(disk) => {
                let (page, within) = (rank / disk.page, (rank % disk.page) as usize);
                Ok(disk.load(page)?[within])
            }
        }
    }

    /// The first rank whose key is not below `key` (`len` if none).
    pub(crate) fn lower_bound(&mut self, key: &Key) -> Result<u64, SpillError> {
        match self {
            Self::Memory(rows) => Ok(rows.partition_point(|row| row.key < *key) as u64),
            Self::Disk(disk) => {
                // Fences below `key`; the bound lies in the last such page or starts the next.
                let below = disk.fences.partition_point(|fence| fence < key) as u64;
                let Some(page) = below.checked_sub(1) else {
                    return Ok(0);
                };
                let rows = disk.load(page)?;
                let within = rows.partition_point(|row| row.key < *key) as u64;
                Ok(page * disk.page + within)
            }
        }
    }

    /// Every row in rank order, read sequentially.
    pub(crate) fn iter(
        &self,
    ) -> Result<Box<dyn Iterator<Item = Result<Row, SpillError>> + '_>, SpillError> {
        Ok(match self {
            Self::Memory(rows) => Box::new(rows.iter().copied().map(Ok)),
            Self::Disk(disk) => Box::new(disk.sequence.reader()?),
        })
    }

    /// Every row in rank order.
    pub(crate) fn for_each(
        &self,
        mut visit: impl FnMut(&Row) -> Result<(), SpillError>,
    ) -> Result<(), SpillError> {
        match self {
            Self::Memory(rows) => rows.iter().try_for_each(visit),
            Self::Disk(disk) => {
                let _ = disk.buffer_bytes;
                for row in disk.sequence.reader()? {
                    visit(&row?)?;
                }
                Ok(())
            }
        }
    }
}

impl DiskRows {
    fn load(&mut self, page: u64) -> Result<&[Row], SpillError> {
        let slot = (page % self.slots.len() as u64) as usize;
        if self.slots[slot]
            .as_ref()
            .is_none_or(|(held, _)| *held != page)
        {
            let first = page * self.page;
            let count = (self.len - first).min(self.page) as usize;
            let mut rows = self.slots[slot]
                .take()
                .map(|(_, rows)| rows)
                .unwrap_or_default();
            self.random
                .read(first, count, &mut self.encoded, &mut rows)?;
            self.slots[slot] = Some((page, rows));
        }
        Ok(&self.slots[slot].as_ref().expect("page just loaded").1)
    }
}

/// The sorted rows and the name log, with a one-entry lookup memo.
pub(crate) struct Catalog {
    rows: Rows,
    names: Names,
    memo: Option<(String, Option<Member>)>,
    buffer: Vec<u8>,
    buffer_bytes: usize,
}

impl Catalog {
    pub(crate) fn new(rows: Rows, names: Names, config: &CatalogConfig) -> Self {
        Self {
            rows,
            names,
            memo: None,
            buffer: Vec::new(),
            buffer_bytes: config.buffer_bytes,
        }
    }

    pub(crate) fn len(&self) -> u64 {
        self.rows.len()
    }

    pub(crate) fn names(&self) -> Names {
        self.names.clone()
    }

    pub(crate) fn rows(&self) -> &Rows {
        &self.rows
    }

    pub(crate) fn directory(&self) -> Result<Directory, SpillError> {
        self.names.directory(self.len(), self.buffer_bytes)
    }

    /// The member named exactly `name`, by key then by exact name bytes.
    pub(crate) fn find(&mut self, name: &str) -> Result<Option<Member>, SpillError> {
        if let Some((held, member)) = &self.memo {
            if held == name {
                return Ok(*member);
            }
        }
        let key = Key::of(name.as_bytes());
        let mut rank = self.rows.lower_bound(&key)?;
        let mut found = None;
        while rank < self.rows.len() {
            let row = self.rows.get(rank)?;
            if row.key != key {
                break;
            }
            if usize::from(row.name_len) == name.len() {
                self.names
                    .read(row.name_ref, row.name_len, &mut self.buffer)?;
                if self.buffer == name.as_bytes() {
                    found = Some(Member {
                        rank,
                        size: row.size,
                        data_offset: row.data_offset,
                        crc32: row.crc32,
                        name_ref: row.name_ref,
                        name_len: row.name_len,
                    });
                    break;
                }
            }
            rank += 1;
        }
        let memo = self.memo.get_or_insert_with(|| (String::new(), None));
        memo.0.clear();
        memo.0.push_str(name);
        memo.1 = found;
        Ok(found)
    }

    /// The name of the member at `rank`.
    pub(crate) fn name(&mut self, rank: u64) -> Result<String, SpillError> {
        let row = self.rows.get(rank)?;
        let mut bytes = Vec::new();
        self.names.read(row.name_ref, row.name_len, &mut bytes)?;
        String::from_utf8(bytes).map_err(|_| self.names.corrupt("catalog name is not UTF-8".into()))
    }
}

/// An external sorter of `T` on `scratch` with sort resources `config`.
pub(crate) fn sorter<T: Record>(
    scratch: &Scratch,
    config: SortConfig,
) -> Result<ExternalSorter<'_, T>, SpillError> {
    ExternalSorter::new(scratch, config)
}
