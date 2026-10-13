//! Finite stored ZIP/ZIP64 admission before directory/index allocation.
//! Caller owns the selected source identity, known byte length and resource policy.
//!
//! The admitted catalog is held by [`super::catalog`]: in memory when small, otherwise on
//! deferred scratch, so resident memory does not grow with the entry count (VS-A).
use super::catalog::{
    self, md5_key, Catalog, CatalogConfig, Directory, IndexRow, Key, LogWriter, Member, Names, Row,
    RowsWriter,
};
use crate::spill::{Record, Scratch, SpillError};
use sha2::{Digest, Sha256};
use std::{
    io::{self, Read, Seek, SeekFrom},
    path::Path,
    sync::Arc,
};

/// Index rows per streamed chunk: a 65,520-byte buffer.
const INDEX_CHUNK_ROWS: usize = 2730;
/// Central-directory bytes read at once.
const DIRECTORY_CHUNK: u64 = 64 * 1024;

#[derive(Clone, Copy, Debug)]
pub(crate) struct ReadLimits {
    pub source_archive_bytes: u64,
    pub central_directory_bytes: u64,
    pub archive_entries: u64,
    pub member_bytes: u64,
    pub archive_stored_bytes: u64,
}

#[derive(Debug, thiserror::Error)]
pub(crate) enum ReadError {
    #[error("{0}")]
    InvalidInput(String),
    #[error("{0}")]
    Unsupported(String),
    #[error("{0}")]
    ResourceLimit(String),
    #[error("missing archive member: {0}")]
    MissingMember(String),
    #[error(transparent)]
    Io(#[from] io::Error),
    /// The catalog's own scratch storage failed; never a verdict on the archive.
    #[error("archive catalog scratch: {0}")]
    Scratch(#[from] SpillError),
}

/// The last central-directory entry, which a 3TZ requires to be its index.
#[derive(Clone, Copy, Debug)]
struct Last {
    is_index: bool,
    size: u64,
    data_offset: u64,
    local_offset: u64,
    crc32: u32,
}

/// One selected end record, one admitted catalog, one owned source reader.
pub(crate) struct StoredArchive<R> {
    reader: R,
    catalog: Catalog,
    last: Last,
    /// S, fixed before traversal: the basis of VS-C's scaled budgets.
    #[allow(dead_code)]
    stored_bytes: u64,
    index_read_bytes: u64,
    config: CatalogConfig,
    // Declared last: every scratch file above is removed before the directory.
    scratch: Arc<Scratch>,
}

impl<R: Read + Seek> StoredArchive<R> {
    /// Admits the archive with this build's catalog profile and default scratch parent.
    pub(crate) fn new(
        reader: R,
        known_length: u64,
        limits: &ReadLimits,
    ) -> Result<Self, ReadError> {
        Self::new_with(
            reader,
            known_length,
            limits,
            CatalogConfig::selected(),
            None,
        )
    }

    /// Admits the archive; a catalog over `config.resident_bytes` spills under
    /// `scratch_parent` (default: the system temporary directory), created on first use.
    pub(crate) fn new_with(
        mut reader: R,
        known_length: u64,
        limits: &ReadLimits,
        config: CatalogConfig,
        scratch_parent: Option<&Path>,
    ) -> Result<Self, ReadError> {
        let scanned = scan_directory(&mut reader, known_length, limits, &config, scratch_parent)?;
        let mut archive = Self {
            reader,
            catalog: scanned.catalog,
            last: scanned.last,
            stored_bytes: scanned.stored_bytes,
            index_read_bytes: 0,
            config,
            scratch: scanned.scratch,
        };
        archive.validate_index()?;
        Ok(archive)
    }

    /// Admitted member count N, index included.
    pub(crate) fn len(&self) -> u64 {
        self.catalog.len()
    }

    /// S: the sum of admitted stored member sizes.
    #[allow(dead_code)]
    pub(crate) fn stored_bytes(&self) -> u64 {
        self.stored_bytes
    }

    /// The member named exactly `name`, if any.
    pub(crate) fn find(&mut self, name: &str) -> Result<Option<Member>, ReadError> {
        Ok(self.catalog.find(name)?)
    }

    pub(crate) fn member(&mut self, name: &str) -> Result<Member, ReadError> {
        self.find(name)?
            .ok_or_else(|| ReadError::MissingMember(name.into()))
    }

    /// The name of the member at `rank` (catalog order).
    pub(crate) fn name(&mut self, rank: u64) -> Result<String, ReadError> {
        Ok(self.catalog.name(rank)?)
    }

    /// Shared access to the member names, for consumers that outlive this reader.
    pub(crate) fn names(&self) -> Names {
        self.catalog.names()
    }

    /// The members in central-directory order.
    pub(crate) fn directory(&self) -> Result<Directory, ReadError> {
        Ok(self.catalog.directory()?)
    }

    /// The catalog's scratch, for consumers that spill under the same directory.
    pub(crate) fn scratch(&self) -> Arc<Scratch> {
        self.scratch.clone()
    }

    pub(crate) fn config(&self) -> &CatalogConfig {
        &self.config
    }

    /// Constructor work that C1 charges under its existing aggregate read cap.
    pub(crate) fn index_read_bytes(&self) -> u64 {
        self.index_read_bytes
    }

    pub(crate) fn read_member(&mut self, name: &str, max: u64) -> Result<Vec<u8>, ReadError> {
        let member = self.member(name)?;
        let (size, offset, crc) = (member.size, member.data_offset, member.crc32);
        if size > max {
            return Err(ReadError::limit("archive member exceeds read limit"));
        }
        let length = usize::try_from(size)
            .map_err(|_| ReadError::limit("archive member exceeds host range"))?;
        let mut bytes = bounded_bytes(length)?;
        read(&mut self.reader, offset, &mut bytes)?;
        if crc32fast::hash(&bytes) != crc {
            return Err(malformed("stored member CRC mismatch"));
        }
        Ok(bytes)
    }

    /// Streams one admitted stored range through CRC-32 and SHA-256.
    pub(crate) fn hash_range(
        &mut self,
        size: u64,
        offset: u64,
        expected_crc: u32,
    ) -> Result<[u8; 32], ReadError> {
        let mut remaining = size;
        self.reader.seek(SeekFrom::Start(offset))?;
        let mut buffer = [0; 64 * 1024];
        let mut crc = crc32fast::Hasher::new();
        let mut sha = Sha256::new();
        while remaining != 0 {
            let count = remaining.min(buffer.len() as u64) as usize;
            read_exact(&mut self.reader, &mut buffer[..count])?;
            crc.update(&buffer[..count]);
            sha.update(&buffer[..count]);
            remaining -= count as u64;
        }
        if crc.finalize() != expected_crc {
            return Err(malformed("stored member CRC mismatch"));
        }
        Ok(sha.finalize().into())
    }

    fn validate_index(&mut self) -> Result<(), ReadError> {
        let n = self.len();
        if n < 2 {
            return Err(malformed("3TZ must contain tileset.json and an index"));
        }
        self.member("tileset.json")?;
        let length = (n - 1)
            .checked_mul(24)
            .ok_or_else(|| malformed("3TZ index length overflows"))?;
        if !self.last.is_index {
            return Err(malformed("last ZIP member must be @3dtilesIndex1@"));
        }
        if self.last.size != length {
            return Err(malformed(
                "3TZ index must contain every member exactly once",
            ));
        }
        // Cardinality is admitted before any index read. The catalog supplies every
        // association; no second directory interpretation. The index is streamed in
        // whole-row chunks against the catalog in rank order, so neither is held whole.
        self.index_read_bytes = length;
        let index_offset = self.last.local_offset;
        let mut pending = CatalogCursor {
            rows: self.catalog.rows().iter()?,
            index_offset,
        };
        let mut previous: Option<[u64; 2]> = None;
        let mut unordered = None;
        let mut matched = true;
        // A stored CRC mismatch outranks every row failure, as when the index was checked
        // whole before parsing: row failures are decided after the CRC.
        let crc_ok = index_pass(
            &mut self.reader,
            &self.last,
            length,
            |position, key, offset| {
                if unordered.is_none() && previous.is_some_and(|previous| previous > key) {
                    unordered = Some(position);
                }
                previous = Some(key);
                if matched {
                    matched = pending
                        .next()?
                        .is_some_and(|(row, local)| row.md5 == key && local == offset);
                }
                Ok(())
            },
        )?;
        drop(pending);
        if !crc_ok {
            return Err(malformed("stored member CRC mismatch"));
        }
        if matched {
            return Ok(());
        }
        // Exact diagnosis (only for a failing index or an MD5-colliding group in another
        // order): the first row, in index order, that the sequential check rejects.
        let first_mismatch = self.first_mismatch(length, index_offset)?;
        match (unordered, first_mismatch) {
            (Some(unordered), first) if first.is_none_or(|first| unordered <= first) => {
                Err(malformed("3TZ index hashes are not ordered"))
            }
            (None, None) => Ok(()),
            _ => Err(malformed("3TZ index differs from admitted directory")),
        }
    }

    /// The first index position whose row names no admitted non-index member, or repeats
    /// an earlier row's member (its MD5 and offset). Rows and catalog pairs are sorted by
    /// `(md5, offset)` on scratch and merged; each pair's earliest row is the one the
    /// sequential check accepts.
    fn first_mismatch(&mut self, length: u64, index_offset: u64) -> Result<Option<u64>, ReadError> {
        let scratch = self.scratch.clone();
        let mut rows = catalog::sorter::<IndexRow>(&scratch, self.config.side_sort)?;
        let crc_ok = index_pass(
            &mut self.reader,
            &self.last,
            length,
            |position, md5, offset| {
                Ok(rows.push(IndexRow {
                    md5,
                    offset,
                    position,
                })?)
            },
        )?;
        if !crc_ok {
            return Err(malformed("stored member CRC mismatch"));
        }
        let mut members = catalog::sorter::<IndexRow>(&scratch, self.config.side_sort)?;
        self.catalog.rows().for_each(|row| {
            if row.local_offset != index_offset {
                members.push(IndexRow {
                    md5: row.key.md5,
                    offset: row.local_offset,
                    position: 0,
                })?;
            }
            Ok(())
        })?;
        Ok(first_failing_row(rows.finish()?, members.finish()?)?)
    }
}

/// The first index position the sequential check rejects, from the index rows and the
/// non-index catalog pairs, each sorted by `(md5, offset)` (rows then by position).
///
/// Before the first failure every row passes, so the first failure is the least of: each
/// pair absent from the catalog, at its earliest row; each present pair repeated, at its
/// second row. Equal MD5s (collisions) need no special case: pairs are exact.
fn first_failing_row(
    rows: impl Iterator<Item = Result<IndexRow, SpillError>>,
    members: impl Iterator<Item = Result<IndexRow, SpillError>>,
) -> Result<Option<u64>, SpillError> {
    let mut members = members.peekable();
    type Group = Option<(([u64; 2], u64), u64, bool)>;
    // A pair's rows all fail when no member has it; otherwise only its later rows do.
    fn close(group: Group, second: Option<u64>, first: &mut Option<u64>) {
        if let Some((_, lowest, present)) = group {
            let failing = if present { second } else { Some(lowest) };
            if let Some(failing) = failing {
                *first = Some(first.map_or(failing, |first| first.min(failing)));
            }
        }
    }
    let mut first: Option<u64> = None;
    let mut group: Group = None;
    let mut second = None;
    for row in rows {
        let row = row?;
        let pair = (row.md5, row.offset);
        if group.is_some_and(|(held, _, _)| held == pair) {
            second.get_or_insert(row.position);
            continue;
        }
        close(group, second.take(), &mut first);
        let present = loop {
            match members.peek() {
                Some(Ok(member)) if (member.md5, member.offset) < pair => {
                    members.next();
                }
                Some(Ok(member)) => break (member.md5, member.offset) == pair,
                Some(Err(_)) => return Err(members.next().unwrap().unwrap_err()),
                None => break false,
            }
        };
        group = Some((pair, row.position, present));
    }
    close(group, second, &mut first);
    Ok(first)
}

/// Reads the index once in whole-row chunks, giving each row's position, MD5 key and
/// offset to `row`; returns whether the stored CRC matches.
fn index_pass(
    reader: &mut (impl Read + Seek),
    last: &Last,
    length: u64,
    mut row: impl FnMut(u64, [u64; 2], u64) -> Result<(), ReadError>,
) -> Result<bool, ReadError> {
    let mut hasher = crc32fast::Hasher::new();
    let mut buffer = [0; INDEX_CHUNK_ROWS * 24];
    let mut remaining = length;
    let mut position = 0u64;
    reader.seek(SeekFrom::Start(last.data_offset))?;
    while remaining != 0 {
        let count = remaining.min(buffer.len() as u64) as usize;
        let chunk = &mut buffer[..count];
        read_exact(reader, chunk)?;
        remaining -= count as u64;
        hasher.update(chunk);
        for bytes in chunk.as_chunks::<24>().0 {
            row(position, md5_key(&bytes[..16]), u64_at(bytes, 16))?;
            position += 1;
        }
    }
    Ok(hasher.finalize() == last.crc32)
}

/// Non-index catalog rows in rank order, read lazily.
struct CatalogCursor<'a> {
    rows: Box<dyn Iterator<Item = Result<Row, SpillError>> + 'a>,
    index_offset: u64,
}

impl CatalogCursor<'_> {
    fn next(&mut self) -> Result<Option<(Key, u64)>, ReadError> {
        for row in self.rows.by_ref() {
            let row = row?;
            if row.local_offset != self.index_offset {
                return Ok(Some((row.key, row.local_offset)));
            }
        }
        Ok(None)
    }
}

impl ReadError {
    fn unsupported(message: &str) -> Self {
        Self::Unsupported(message.into())
    }
    fn limit(message: &str) -> Self {
        Self::ResourceLimit(message.into())
    }
}

fn malformed(message: &str) -> ReadError {
    ReadError::InvalidInput(message.into())
}
fn read(file: &mut (impl Read + Seek), offset: u64, bytes: &mut [u8]) -> Result<(), ReadError> {
    file.seek(SeekFrom::Start(offset))?;
    read_exact(file, bytes)
}
fn read_exact(reader: &mut impl Read, mut bytes: &mut [u8]) -> Result<(), ReadError> {
    // Observe exhaustion ourselves: Read::read_exact also manufactures an
    // UnexpectedEof, which cannot be distinguished from a source's real error.
    while !bytes.is_empty() {
        match reader.read(bytes) {
            Ok(0) => return Err(malformed("truncated stored archive record")),
            Ok(count) => bytes = &mut bytes[count..],
            Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
            Err(error) => return Err(ReadError::Io(error)),
        }
    }
    Ok(())
}
fn u16_at(b: &[u8], n: usize) -> u16 {
    u16::from_le_bytes(b[n..n + 2].try_into().unwrap())
}
fn u32_at(b: &[u8], n: usize) -> u64 {
    u32::from_le_bytes(b[n..n + 4].try_into().unwrap()).into()
}
fn u64_at(b: &[u8], n: usize) -> u64 {
    u64::from_le_bytes(b[n..n + 8].try_into().unwrap())
}

// Every TLV is checked, even without a ZIP64 sentinel. Only sentinel-selected
// ZIP64 fields are consumed, in uncompressed/compressed/offset/disk order.
fn zip64_sizes<const N: usize>(
    mut values: [u64; N],
    extra: &[u8],
    disk: Option<u16>,
) -> Result<([u64; N], u32, bool), ReadError> {
    let mut cursor = 0;
    let mut record = None;
    let mut unicode_path = false;
    let mut aes = false;
    while cursor < extra.len() {
        if extra.len() - cursor < 4 {
            return Err(malformed("truncated ZIP extra field"));
        }
        let length = usize::from(u16_at(extra, cursor + 2));
        let end = cursor
            .checked_add(4 + length)
            .filter(|&n| n <= extra.len())
            .ok_or_else(|| malformed("ZIP extra field exceeds entry"))?;
        if u16_at(extra, cursor) == 1 {
            if record.is_some() {
                return Err(malformed("duplicate ZIP64 extra field"));
            }
            record = Some(&extra[cursor + 4..end]);
        }
        unicode_path |= u16_at(extra, cursor) == 0x7075;
        aes |= u16_at(extra, cursor) == 0x9901;
        cursor = end;
    }
    // ZipArchive interprets Unicode Path as an alternate decoded identity.
    // The finite reader admits exactly the raw UTF-8 name, with no override.
    if unicode_path {
        return Err(ReadError::unsupported(
            "Unicode Path name overrides are unsupported",
        ));
    }
    if aes {
        return Err(ReadError::unsupported(
            "AES ZIP extra fields are unsupported",
        ));
    }
    let required = values
        .iter()
        .filter(|&&value| value == u64::from(u32::MAX))
        .count()
        * 8
        + usize::from(disk == Some(u16::MAX)) * 4;
    let mut disk_number = u32::from(disk.unwrap_or(0));
    if values.contains(&u64::from(u32::MAX)) || disk == Some(u16::MAX) {
        let record = record.ok_or_else(|| malformed("missing ZIP64 sizes/offset"))?;
        cursor = 0;
        for value in &mut values {
            if *value == u64::from(u32::MAX) {
                if record.len() - cursor < 8 {
                    return Err(malformed("truncated ZIP64 sizes/offset"));
                }
                *value = u64_at(record, cursor);
                cursor += 8;
            }
        }
        if disk == Some(u16::MAX) {
            if record.len() - cursor < 4 {
                return Err(malformed("truncated ZIP64 disk number"));
            }
            disk_number = u32_at(record, cursor) as u32;
        }
    }
    if let Some(record) = record {
        // The discarded Zip 2.4.2 parser consumed all three u64 fields for a
        // central payload >=24 bytes even without their sentinels. This native
        // owner consumes sentinel-selected values only and admits exactly their
        // central field length. Without local sentinels, the finite profile
        // admits an empty tag or the optional two-size descriptor declaration.
        let supported = if N == 2 && required == 0 {
            matches!(record.len(), 0 | 16)
        } else {
            record.len() == required
        };
        if !supported {
            return Err(ReadError::unsupported(
                "surplus ZIP64 values are outside the admitted profile",
            ));
        }
    }
    Ok((values, disk_number, record.is_some()))
}

fn bounded_bytes(size: usize) -> Result<Vec<u8>, ReadError> {
    let mut bytes = Vec::new();
    bytes
        .try_reserve_exact(size)
        .map_err(|_| ReadError::limit("bounded archive allocation unavailable"))?;
    bytes.resize(size, 0);
    Ok(bytes)
}

#[derive(Clone, Copy, Debug)]
struct Ends {
    values: [u64; 2],
    count: usize,
}
impl Ends {
    fn one(end: u64) -> Self {
        Self {
            values: [end, 0],
            count: 1,
        }
    }
    fn select(self, boundary: u64) -> Result<u64, ReadError> {
        let mut fitting = self.values[..self.count]
            .iter()
            .copied()
            .filter(|&end| end <= boundary);
        let selected = fitting
            .next()
            .ok_or_else(|| ReadError::unsupported("stored ZIP member records overlap"))?;
        if fitting.any(|end| end != selected) {
            return Err(ReadError::unsupported(
                "ZIP descriptor has multiple fitting extents",
            ));
        }
        Ok(selected)
    }
}

fn descriptor(
    file: &mut (impl Read + Seek),
    offset: u64,
    boundary: u64,
    crc: u64,
    size: u64,
    zip64: bool,
) -> Result<Ends, ReadError> {
    // A signature is optional. Trying both layouts also handles a CRC whose
    // value equals the signature without assigning it the wrong interpretation.
    let mut bytes = [0; 24];
    let length = boundary
        .checked_sub(offset)
        .ok_or_else(|| malformed("ZIP descriptor outside payload"))?
        .min(bytes.len() as u64) as usize;
    read(file, offset, &mut bytes[..length])?;
    let width = if zip64 { 8 } else { 4 };
    let mut ends = Ends {
        values: [0; 2],
        count: 0,
    };
    for start in [0, 4] {
        if start + 4 + width * 2 > length
            || (start == 4 && u32_at(&bytes, 0) != 0x08074b50)
            || u32_at(&bytes, start) != crc
        {
            continue;
        }
        let value = |at| {
            if zip64 {
                u64_at(&bytes, at)
            } else {
                u32_at(&bytes, at)
            }
        };
        if value(start + 4) == size && value(start + 4 + width) == size {
            ends.values[ends.count] = offset + (start + 4 + width * 2) as u64;
            ends.count += 1;
        }
    }
    if ends.count == 0 {
        return Err(malformed(
            "ZIP data descriptor differs from central directory",
        ));
    }
    Ok(ends)
}

/// The admitted directory: its catalog, last entry and stored total S.
struct Scanned {
    catalog: Catalog,
    last: Last,
    stored_bytes: u64,
    scratch: Arc<Scratch>,
}

/// Central-directory bytes, read in chunks instead of one read per field.
struct DirectoryBuffer {
    start: u64,
    bytes: Vec<u8>,
    stop: u64,
}

impl DirectoryBuffer {
    /// `len` bytes at `at`; the caller has checked `at + len <= stop`.
    fn get(
        &mut self,
        file: &mut (impl Read + Seek),
        at: u64,
        len: usize,
    ) -> Result<&[u8], ReadError> {
        let end = at + len as u64;
        if at < self.start || end > self.start + self.bytes.len() as u64 {
            let want = (len as u64).max(DIRECTORY_CHUNK).min(self.stop - at);
            self.bytes.clear();
            self.bytes
                .try_reserve_exact(want as usize)
                .map_err(|_| ReadError::limit("bounded archive allocation unavailable"))?;
            self.bytes.resize(want as usize, 0);
            read(file, at, &mut self.bytes)?;
            self.start = at;
        }
        let from = (at - self.start) as usize;
        Ok(&self.bytes[from..from + len])
    }
}

/// One record's physical extent, sorted by start for the disjointness check.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct Extent {
    start: u64,
    ends: [u64; 2],
    count: u8,
}

impl Record for Extent {
    const SIZE: usize = 25;
    fn encode(&self, out: &mut [u8]) {
        out[..8].copy_from_slice(&self.start.to_le_bytes());
        out[8..16].copy_from_slice(&self.ends[0].to_le_bytes());
        out[16..24].copy_from_slice(&self.ends[1].to_le_bytes());
        out[24] = self.count;
    }
    fn decode(bytes: &[u8]) -> Self {
        Self {
            start: u64_at(bytes, 0),
            ends: [u64_at(bytes, 8), u64_at(bytes, 16)],
            count: bytes[24],
        }
    }
}

impl Extent {
    fn ends(self) -> Ends {
        Ends {
            values: self.ends,
            count: usize::from(self.count),
        }
    }
}

fn scan_directory(
    file: &mut (impl Read + Seek),
    size: u64,
    limits: &ReadLimits,
    config: &CatalogConfig,
    scratch_parent: Option<&Path>,
) -> Result<Scanned, ReadError> {
    if size > limits.source_archive_bytes {
        return Err(ReadError::limit("source archive exceeds byte limit"));
    }
    if size < 22 {
        return Err(malformed("truncated ZIP end record"));
    }
    let mut magic = [0; 4];
    read(file, 0, &mut magic)?;
    if !matches!(&magic, b"PK\x03\x04" | b"PK\x05\x06") {
        return Err(malformed(
            "stored archive must begin with a ZIP local or end record",
        ));
    }
    let n = size.min(22 + u64::from(u16::MAX));
    let start = size - n;
    let mut tail = bounded_bytes(n as usize)?;
    read(file, start, &mut tail)?;
    let end = (0..=tail.len() - 22)
        .rev()
        .find(|&i| {
            &tail[i..i + 4] == b"PK\x05\x06"
                && i + 22 + usize::from(u16_at(&tail, i + 20)) == tail.len()
        })
        .ok_or_else(|| malformed("missing ZIP end record"))?;
    let e = &tail[end..];
    if u16_at(e, 4) != 0 || u16_at(e, 6) != 0 {
        return Err(ReadError::unsupported(
            "multi-disk ZIP archives are unsupported",
        ));
    }
    let end_offset = start + end as u64;
    let (entries, bytes, offset, boundary) = if u16_at(e, 8) == u16::MAX
        || u16_at(e, 10) == u16::MAX
        || u32_at(e, 12) == u64::from(u32::MAX)
        || u32_at(e, 16) == u64::from(u32::MAX)
    {
        let location = end_offset
            .checked_sub(20)
            .ok_or_else(|| malformed("missing ZIP64 locator"))?;
        let mut locator = [0; 20];
        read(file, location, &mut locator)?;
        if &locator[..4] != b"PK\x06\x07" {
            return Err(malformed("invalid ZIP64 locator"));
        }
        if u32_at(&locator, 4) != 0 || u32_at(&locator, 16) != 1 {
            return Err(ReadError::unsupported(
                "multi-disk ZIP64 archives are unsupported",
            ));
        }
        let record_offset = u64_at(&locator, 8);
        if record_offset.checked_add(56).is_none_or(|n| n > location) {
            return Err(malformed("ZIP64 record outside source"));
        }
        let mut record = [0; 56];
        read(file, record_offset, &mut record)?;
        let length = u64_at(&record, 4);
        if &record[..4] != b"PK\x06\x06"
            || length < 44
            || record_offset
                .checked_add(12)
                .and_then(|n| n.checked_add(length))
                != Some(location)
        {
            return Err(malformed("invalid ZIP64 end record"));
        }
        if u32_at(&record, 16) != 0 || u32_at(&record, 20) != 0 {
            return Err(ReadError::unsupported(
                "multi-disk ZIP64 archives are unsupported",
            ));
        }
        if u64_at(&record, 24) != u64_at(&record, 32)
            || (u16_at(e, 8) != u16::MAX && u64::from(u16_at(e, 8)) != u64_at(&record, 24))
            || (u16_at(e, 10) != u16::MAX && u64::from(u16_at(e, 10)) != u64_at(&record, 32))
            || (u32_at(e, 12) != u64::from(u32::MAX) && u32_at(e, 12) != u64_at(&record, 40))
            || (u32_at(e, 16) != u64::from(u32::MAX) && u32_at(e, 16) != u64_at(&record, 48))
        {
            return Err(malformed("ZIP64 end fields differ from classic end record"));
        }
        (
            u64_at(&record, 32),
            u64_at(&record, 40),
            u64_at(&record, 48),
            record_offset,
        )
    } else {
        if u16_at(e, 8) != u16_at(e, 10) {
            return Err(malformed("ZIP entry counts differ"));
        }
        (
            u64::from(u16_at(e, 10)),
            u32_at(e, 12),
            u32_at(e, 16),
            end_offset,
        )
    };
    // A format rule, not a resource policy: it precedes every engineering
    // limit and any catalog allocation, matching the producer's refusal.
    if entries > super::MEMBER_CEILING {
        return Err(ReadError::Unsupported(format!(
            "3TZ member ceiling: {entries} members exceed {}",
            super::MEMBER_CEILING
        )));
    }
    if entries > limits.archive_entries {
        return Err(ReadError::limit("archive exceeds entry limit"));
    }
    if bytes > limits.central_directory_bytes {
        return Err(ReadError::limit("ZIP central directory exceeds byte limit"));
    }
    let stop = offset
        .checked_add(bytes)
        .filter(|&n| n <= boundary)
        .ok_or_else(|| malformed("ZIP central directory outside source"))?;
    if entries
        .checked_mul(46)
        .is_none_or(|minimum| minimum > bytes)
    {
        return Err(malformed("ZIP entry count exceeds directory length"));
    }
    // Catalog storage is chosen from the admitted count and directory length, before any
    // record is read. Scratch, if any, is preflighted and created at its first file.
    let memory = entries
        .checked_mul(64 + catalog::LOG_HEADER as u64)
        .and_then(|n| n.checked_add(bytes))
        .is_some_and(|n| n <= config.resident_bytes);
    let scratch = Arc::new(Scratch::deferred(
        scratch_parent,
        CatalogConfig::scratch_estimate(entries, bytes),
    ));
    let mut log = LogWriter::new(&scratch, memory, config)?;
    let mut rows = catalog::sorter::<Row>(&scratch, config.sort)?;
    let mut extents = catalog::sorter::<Extent>(&scratch, config.side_sort)?;
    let mut last = None;
    let mut directory = DirectoryBuffer {
        start: offset,
        bytes: Vec::new(),
        stop,
    };
    let mut cursor = offset;
    let mut total = 0u64;
    let mut name_bytes = 0u64;
    for ordinal in 0..entries {
        if cursor.checked_add(46).is_none_or(|n| n > stop) {
            return Err(malformed("truncated ZIP central entry"));
        }
        let mut h = [0; 46];
        h.copy_from_slice(directory.get(file, cursor, 46)?);
        if &h[..4] != b"PK\x01\x02" {
            return Err(malformed("invalid ZIP central entry"));
        }
        if u16_at(&h, 10) != 0 {
            return Err(malformed("3TZ members must be stored"));
        }
        if u16_at(&h, 8) & (1 | 64 | 8192) != 0 {
            return Err(ReadError::unsupported(
                "encrypted ZIP members are unsupported",
            ));
        }
        let name_size = usize::from(u16_at(&h, 28));
        name_bytes = name_bytes
            .checked_add(name_size as u64)
            .filter(|&n| n <= bytes)
            .ok_or_else(|| malformed("ZIP names exceed admitted directory bytes"))?;
        let extra_size = usize::from(u16_at(&h, 30));
        let next = cursor
            .checked_add(46 + name_size as u64 + extra_size as u64 + u64::from(u16_at(&h, 32)))
            .filter(|&n| n <= stop)
            .ok_or_else(|| malformed("ZIP entry fields exceed central directory"))?;
        let name = directory.get(file, cursor + 46, name_size)?.to_vec();
        if std::str::from_utf8(&name).is_err() {
            if u16_at(&h, 8) & 0x800 != 0 {
                return Err(malformed("invalid UTF-8 ZIP member name"));
            }
            return Err(ReadError::unsupported(
                "non-UTF8 ZIP member names are unsupported",
            ));
        }
        if !name.is_ascii() && u16_at(&h, 8) & 0x800 == 0 {
            return Err(ReadError::unsupported(
                "non-ASCII ZIP names require the UTF-8 flag",
            ));
        }
        let extra = directory
            .get(file, cursor + 46 + name_size as u64, extra_size)?
            .to_vec();
        let ([member, compressed, local_offset], disk, central_zip64) = zip64_sizes(
            [u32_at(&h, 24), u32_at(&h, 20), u32_at(&h, 42)],
            &extra,
            Some(u16_at(&h, 34)),
        )?;
        // The last-entry index is structural, not payload: validate_index
        // admits exactly 24 * (entries - 1) bytes, a length the checked
        // central-directory count fixes and the member ceiling bounds, and
        // streams it. Every other member keeps the per-member limit.
        let index = ordinal + 1 == entries && name == super::TZ_INDEX_NAME.as_bytes();
        if !index && member > limits.member_bytes {
            return Err(ReadError::limit("archive member exceeds byte limit"));
        }
        if compressed != member {
            return Err(malformed("stored member lengths differ"));
        }
        if disk != 0 {
            return Err(ReadError::unsupported(
                "multi-disk member references are unsupported",
            ));
        }
        // Malformed offsets are input failures, including MAX. The catalog
        // records exactly these checked raw ranges for all subsequent reads.
        if local_offset.checked_add(30).is_none_or(|n| n > offset) {
            return Err(malformed("ZIP local header outside source payload"));
        }
        // The fixed local header and its name are read at once, never past the directory.
        let span = (offset - local_offset).min(30 + name_size as u64) as usize;
        let mut local_record = bounded_bytes(span)?;
        read(file, local_offset, &mut local_record)?;
        let mut local = [0; 30];
        local.copy_from_slice(&local_record[..30]);
        if &local[..4] != b"PK\x03\x04"
            || u16_at(&local, 8) != 0
            || u16_at(&local, 6) != u16_at(&h, 8)
        {
            return Err(malformed("local ZIP header differs from central directory"));
        }
        let local_name = usize::from(u16_at(&local, 26));
        if local_name != name_size {
            return Err(malformed("local ZIP name differs from central directory"));
        }
        let data_start = local_offset
            .checked_add(30 + local_name as u64 + u64::from(u16_at(&local, 28)))
            .ok_or_else(|| malformed("local ZIP fields overflow"))?;
        if data_start.checked_add(member).is_none_or(|n| n > offset) {
            return Err(malformed("stored ZIP payload outside source"));
        }
        // `data_start + member <= offset` bounds the name within the bytes read above.
        if local_record[30..30 + local_name] != name[..] {
            return Err(malformed("local ZIP name differs from central directory"));
        }
        let mut local_extra = bounded_bytes(usize::from(u16_at(&local, 28)))?;
        if !local_extra.is_empty() {
            read(
                file,
                local_offset + 30 + local_name as u64,
                &mut local_extra,
            )?;
        }
        let ([local_member, local_compressed], _, local_zip64) =
            zip64_sizes([u32_at(&local, 22), u32_at(&local, 18)], &local_extra, None)?;
        let crc = u32_at(&h, 16);
        let ends = if u16_at(&local, 6) & 8 != 0 {
            if central_zip64 && !local_zip64 {
                return Err(ReadError::unsupported(
                    "central-only ZIP64 descriptor width is outside the admitted profile",
                ));
            }
            let invalid_sizes = [
                (u32_at(&local, 22), local_member, member),
                (u32_at(&local, 18), local_compressed, compressed),
            ]
            .into_iter()
            .any(|(stored, expanded, expected)| {
                if stored == u64::from(u32::MAX) {
                    expanded != 0 && expanded != expected
                } else {
                    stored != 0
                }
            });
            if u32_at(&local, 14) != 0 || invalid_sizes {
                return Err(malformed("invalid local ZIP data-descriptor fields"));
            }
            // A local ZIP64 tag selects 64-bit descriptor widths even for tiny
            // members with no size sentinel. Central-only tags are refused
            // above; do not try another width to make a boundary fit.
            descriptor(file, data_start + member, offset, crc, member, local_zip64)?
        } else {
            if u32_at(&local, 14) != crc || local_member != member || local_compressed != compressed
            {
                return Err(malformed(
                    "local ZIP CRC/sizes differ from central directory",
                ));
            }
            Ends::one(data_start + member)
        };
        extents.push(Extent {
            start: local_offset,
            ends: ends.values,
            count: ends.count as u8,
        })?;
        let name_ref = log.append(&name, member, data_start, crc as u32)?;
        rows.push(Row {
            key: Key::of(&name),
            name_ref,
            name_len: name_size as u16,
            size: member,
            data_offset: data_start,
            local_offset,
            crc32: crc as u32,
        })?;
        if ordinal + 1 == entries {
            last = Some(Last {
                is_index: index,
                size: member,
                data_offset: data_start,
                local_offset,
                crc32: crc as u32,
            });
        }
        total = total
            .checked_add(member)
            .filter(|&n| n <= limits.archive_stored_bytes)
            .ok_or_else(|| ReadError::limit("stored archive payload exceeds byte limit"))?;
        cursor = next;
    }
    if cursor != stop {
        return Err(malformed("ZIP directory length/count mismatch"));
    }
    let log = log.finish()?;
    let names = catalog::Names::from_log(log);
    // Equal names have equal keys, so duplicates are adjacent within an equal-key run.
    let mut run: Vec<Row> = Vec::new();
    let (mut a, mut b) = (Vec::new(), Vec::new());
    let mut duplicate = false;
    let writer = RowsWriter::new(&scratch, memory, entries, config)?;
    let sorted = catalog::store_sorted(rows.finish()?, writer, |row| {
        if run.last().is_some_and(|held| held.key != row.key) {
            run.clear();
        }
        if !duplicate {
            names.read(row.name_ref, row.name_len, &mut a)?;
            for held in &run {
                names.read(held.name_ref, held.name_len, &mut b)?;
                duplicate |= a == b;
            }
        }
        run.push(*row);
        Ok(())
    })?;
    if duplicate {
        return Err(malformed("duplicate ZIP member name"));
    }
    // Physical disjointness: each extent against the next start (or the directory).
    let mut extents = extents.finish()?.peekable();
    while let Some(extent) = extents.next() {
        let extent = extent?;
        let boundary = match extents.peek() {
            Some(Ok(next)) => next.start,
            Some(Err(_)) => return Err(extents.next().unwrap().unwrap_err().into()),
            None => offset,
        };
        extent.ends().select(boundary)?;
    }
    Ok(Scanned {
        catalog: Catalog::new(sorted, names, config),
        last: last.ok_or_else(|| malformed("3TZ must contain tileset.json and an index"))?,
        stored_bytes: total,
        scratch,
    })
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use std::io::Cursor;

    /// The scan with this build's catalog profile and default scratch.
    fn scan(
        file: &mut (impl Read + Seek),
        size: u64,
        limits: &ReadLimits,
    ) -> Result<Scanned, ReadError> {
        scan_directory(file, size, limits, &CatalogConfig::selected(), None)
    }

    impl<R: Read + Seek> StoredArchive<R> {
        /// Central-directory-order entries.
        fn entries(&self) -> Vec<catalog::DirectoryEntry> {
            let mut directory = self.directory().unwrap();
            std::iter::from_fn(|| directory.next_entry().unwrap()).collect()
        }
        /// Hashes the member at central-directory position `index`.
        fn hash_member(&mut self, index: usize) -> Result<[u8; 32], ReadError> {
            let entry = self
                .entries()
                .into_iter()
                .nth(index)
                .ok_or_else(|| ReadError::MissingMember(format!("ordinal {index}")))?;
            self.hash_range(entry.size, entry.data_offset, entry.crc32)
        }
    }

    fn put16(bytes: &mut [u8], at: usize, value: u16) {
        bytes[at..at + 2].copy_from_slice(&value.to_le_bytes());
    }
    fn put32(bytes: &mut [u8], at: usize, value: u32) {
        bytes[at..at + 4].copy_from_slice(&value.to_le_bytes());
    }
    fn extra(tag: u16, payload: &[u8]) -> Vec<u8> {
        let mut bytes = Vec::new();
        bytes.extend(tag.to_le_bytes());
        bytes.extend((payload.len() as u16).to_le_bytes());
        bytes.extend(payload);
        bytes
    }

    #[derive(Clone)]
    struct Record<'a> {
        name: &'a [u8],
        data: &'a [u8],
        flags: u16,
        local_extra: Vec<u8>,
        central_extra: Vec<u8>,
        descriptor: Vec<u8>,
    }
    impl<'a> Record<'a> {
        fn stored(name: &'a [u8], data: &'a [u8]) -> Self {
            Self {
                name,
                data,
                flags: 0x800,
                local_extra: Vec::new(),
                central_extra: Vec::new(),
                descriptor: Vec::new(),
            }
        }
        fn described(mut self, wide: bool, signed: bool) -> Self {
            self.flags |= 8;
            if wide {
                self.local_extra = extra(1, &[]);
            }
            if signed {
                self.descriptor.extend(0x08074b50u32.to_le_bytes());
            }
            self.descriptor
                .extend(crc32fast::hash(self.data).to_le_bytes());
            for _ in 0..2 {
                if wide {
                    self.descriptor
                        .extend((self.data.len() as u64).to_le_bytes());
                } else {
                    self.descriptor
                        .extend((self.data.len() as u32).to_le_bytes());
                }
            }
            self
        }
    }

    // Independently authored APPNOTE fixed fields; never invokes our writer.
    fn local(record: &Record<'_>) -> Vec<u8> {
        let mut bytes = vec![0; 30];
        bytes[..4].copy_from_slice(b"PK\x03\x04");
        put16(&mut bytes, 4, 20);
        put16(&mut bytes, 6, record.flags);
        if record.flags & 8 == 0 {
            put32(&mut bytes, 14, crc32fast::hash(record.data));
            put32(&mut bytes, 18, record.data.len() as u32);
            put32(&mut bytes, 22, record.data.len() as u32);
        }
        put16(&mut bytes, 26, record.name.len() as u16);
        put16(&mut bytes, 28, record.local_extra.len() as u16);
        bytes.extend(record.name);
        bytes.extend(&record.local_extra);
        bytes.extend(record.data);
        bytes.extend(&record.descriptor);
        bytes
    }
    fn central(record: &Record<'_>, offset: u32) -> Vec<u8> {
        let mut bytes = vec![0; 46];
        bytes[..4].copy_from_slice(b"PK\x01\x02");
        put16(&mut bytes, 4, 20);
        put16(&mut bytes, 6, 20);
        put16(&mut bytes, 8, record.flags);
        put32(&mut bytes, 16, crc32fast::hash(record.data));
        put32(&mut bytes, 20, record.data.len() as u32);
        put32(&mut bytes, 24, record.data.len() as u32);
        put16(&mut bytes, 28, record.name.len() as u16);
        put16(&mut bytes, 30, record.central_extra.len() as u16);
        put32(&mut bytes, 42, offset);
        bytes.extend(record.name);
        bytes.extend(&record.central_extra);
        bytes
    }
    fn end(bytes: &mut Vec<u8>, directory: usize, count: usize) {
        let directory_size = bytes.len() - directory;
        let mut e = [0; 22];
        e[..4].copy_from_slice(b"PK\x05\x06");
        put16(&mut e, 8, count as u16);
        put16(&mut e, 10, count as u16);
        put32(&mut e, 12, directory_size as u32);
        put32(&mut e, 16, directory as u32);
        bytes.extend(e);
    }
    fn archive(records: &[Record<'_>], order: &[usize], padding: usize) -> Vec<u8> {
        let mut bytes = Vec::new();
        let mut offsets = Vec::new();
        for record in records {
            offsets.push(bytes.len() as u32);
            bytes.extend(local(record));
            bytes.resize(bytes.len() + padding, 0);
        }
        let directory = bytes.len();
        for &i in order {
            bytes.extend(central(&records[i], offsets[i]));
        }
        end(&mut bytes, directory, order.len());
        bytes
    }
    pub(crate) fn stored_zip(records: &[(&[u8], &[u8])]) -> Vec<u8> {
        let records: Vec<_> = records.iter().map(|&(n, d)| Record::stored(n, d)).collect();
        archive(&records, &(0..records.len()).collect::<Vec<_>>(), 0)
    }
    fn indexed(records: &[Record<'_>]) -> Vec<u8> {
        indexed_rows(records, |_| {})
    }
    /// Index rows in sorted order; `edit` defects them before the CRC is set.
    fn indexed_rows(
        records: &[Record<'_>],
        edit: impl FnOnce(&mut Vec<([u8; 16], u64)>),
    ) -> Vec<u8> {
        let mut offset = 0u64;
        let mut rows: Vec<_> = records
            .iter()
            .map(|record| {
                let row = (md5::compute(record.name).0, offset);
                offset += local(record).len() as u64;
                row
            })
            .collect();
        rows.sort_by_key(|(hash, _)| (u64_at(hash, 0), u64_at(hash, 8)));
        edit(&mut rows);
        let mut index = Vec::new();
        for (hash, offset) in rows {
            index.extend(hash);
            index.extend(offset.to_le_bytes());
        }
        let mut records = records.to_vec();
        records.push(Record::stored(
            super::super::TZ_INDEX_NAME.as_bytes(),
            &index,
        ));
        archive(&records, &(0..records.len()).collect::<Vec<_>>(), 0)
    }
    fn limits() -> ReadLimits {
        ReadLimits {
            source_archive_bytes: 4096,
            central_directory_bytes: 2048,
            archive_entries: 16,
            member_bytes: 1024,
            archive_stored_bytes: 2048,
        }
    }
    fn admit(bytes: &[u8]) -> Result<(), ReadError> {
        scan(&mut Cursor::new(bytes), bytes.len() as u64, &limits()).map(|_| ())
    }
    fn directory(bytes: &[u8]) -> usize {
        u32_at(bytes, bytes.len() - 6) as usize
    }

    #[test]
    fn disjoint_adjacent_reordered_and_padded_records_are_admitted() {
        let records = [Record::stored(b"a", b""), Record::stored(b"b", b"data")];
        for padding in [0, 7] {
            for order in [[0, 1], [1, 0]] {
                admit(&archive(&records, &order, padding)).unwrap();
            }
        }
    }

    #[test]
    fn opaque_nested_zip_is_not_another_member_but_indexed_nested_record_overlaps() {
        let inner = Record::stored(b"inner", b"payload");
        let nested = local(&inner);
        let outer = Record::stored(b"outer", &nested);
        admit(&archive(std::slice::from_ref(&outer), &[0], 0)).unwrap();
        let mut bytes = local(&outer);
        let directory = bytes.len();
        bytes.extend(central(&outer, 0));
        bytes.extend(central(&inner, 30 + outer.name.len() as u32));
        end(&mut bytes, directory, 2);
        assert!(matches!(admit(&bytes), Err(ReadError::Unsupported(_))));
    }

    #[test]
    fn repeated_logical_name_is_invalid_before_downstream_zip_deduplication() {
        let record = Record::stored(b"same", b"data");
        let bytes = archive(&[record], &[0, 0], 0);
        assert!(matches!(admit(&bytes), Err(ReadError::InvalidInput(_))));
    }

    #[test]
    fn malformed_local_fields_and_ranges_are_invalid() {
        let original = stored_zip(&[(b"a", b"data")]);
        for at in [14, 18, 22, 30] {
            let mut bytes = original.clone();
            bytes[at] ^= 1;
            assert!(matches!(admit(&bytes), Err(ReadError::InvalidInput(_))));
        }
        let mut bytes = original;
        let cd = directory(&bytes);
        put32(&mut bytes, cd + 42, u32::MAX - 1);
        assert!(matches!(admit(&bytes), Err(ReadError::InvalidInput(_))));
    }

    #[test]
    fn every_extra_tlv_is_checked_without_sentinels() {
        for bad in [
            vec![2],
            vec![2, 0, 4, 0, 0],
            [extra(1, &[]), extra(1, &[])].concat(),
        ] {
            let mut record = Record::stored(b"a", b"data");
            record.local_extra = bad.clone();
            assert!(matches!(
                admit(&archive(&[record.clone()], &[0], 0)),
                Err(ReadError::InvalidInput(_))
            ));
            record.local_extra.clear();
            record.central_extra = bad;
            assert!(matches!(
                admit(&archive(&[record], &[0], 0)),
                Err(ReadError::InvalidInput(_))
            ));
        }
        let mut record = Record::stored(b"a", b"data");
        record.local_extra = extra(0x9999, b"opaque");
        record.central_extra = extra(0x8888, b"opaque");
        admit(&archive(&[record], &[0], 0)).unwrap();
    }

    #[test]
    fn unicode_path_override_is_unsupported_even_if_redundant() {
        let mut record = Record::stored(b"a", b"data");
        let mut payload = vec![1];
        payload.extend(crc32fast::hash(record.name).to_le_bytes());
        payload.extend(record.name);
        for local in [false, true] {
            if local {
                record.local_extra = extra(0x7075, &payload);
            } else {
                record.central_extra = extra(0x7075, &payload);
            }
            assert!(matches!(
                admit(&archive(&[record.clone()], &[0], 0)),
                Err(ReadError::Unsupported(_))
            ));
            record.local_extra.clear();
            record.central_extra.clear();
        }
        let bad = [extra(0x7075, &payload), vec![2]].concat();
        assert!(matches!(
            zip64_sizes([0], &bad, None),
            Err(ReadError::InvalidInput(_))
        ));
    }

    #[test]
    fn surplus_zip64_and_aes_cannot_override_admitted_fields() {
        let ordinary = [4, 4, 0];
        let surplus = [4u64.to_le_bytes(), 4u64.to_le_bytes(), 0u64.to_le_bytes()].concat();
        assert!(matches!(
            zip64_sizes(ordinary, &extra(1, &surplus), Some(0)),
            Err(ReadError::Unsupported(_))
        ));
        assert!(matches!(
            zip64_sizes([0, 0], &extra(1, &surplus), None),
            Err(ReadError::Unsupported(_))
        ));
        assert!(matches!(
            zip64_sizes(ordinary, &extra(1, &[0; 8]), Some(0)),
            Err(ReadError::Unsupported(_))
        ));
        zip64_sizes(ordinary, &extra(1, &[]), Some(0)).unwrap();
        let mut record = Record::stored(b"a", b"data").described(true, false);
        record.local_extra = extra(1, &[4u64.to_le_bytes(), 4u64.to_le_bytes()].concat());
        admit(&archive(&[record], &[0], 0)).unwrap();
        assert!(matches!(
            zip64_sizes(ordinary, &extra(0x9901, &[0; 7]), Some(0)),
            Err(ReadError::Unsupported(_))
        ));
    }

    #[test]
    fn zip64_sentinels_consume_only_required_fields_in_order() {
        let sentinel = u64::from(u32::MAX);
        let payload = [
            7u64.to_le_bytes().as_slice(),
            19u64.to_le_bytes().as_slice(),
            2u32.to_le_bytes().as_slice(),
        ]
        .concat();
        let (values, disk, present) =
            zip64_sizes([sentinel, 7, sentinel], &extra(1, &payload), Some(u16::MAX)).unwrap();
        assert_eq!(values, [7, 7, 19]);
        assert_eq!(disk, 2);
        assert!(present);
        assert!(matches!(
            zip64_sizes([sentinel], &extra(1, &[0; 7]), None),
            Err(ReadError::InvalidInput(_))
        ));
        assert!(matches!(
            zip64_sizes([sentinel], &[], None),
            Err(ReadError::InvalidInput(_))
        ));
    }

    #[test]
    fn signed_and_unsigned_32_and_64_bit_descriptors_are_admitted() {
        for wide in [false, true] {
            for signed in [false, true] {
                let described = Record::stored(b"a", b"data").described(wide, signed);
                let records = [described, Record::stored(b"b", b"")];
                admit(&archive(&records, &[1, 0], 0)).unwrap();
            }
        }
    }

    #[test]
    fn descriptor_width_never_falls_back_to_a_fitting_layout() {
        let mut record = Record::stored(b"a", b"data").described(false, false);
        record.local_extra = extra(1, &[]); // This selects 64 bits, not the supplied 32.
        assert!(matches!(
            admit(&archive(&[record], &[0], 0)),
            Err(ReadError::InvalidInput(_))
        ));
        let mut record = Record::stored(b"a", b"data").described(false, false);
        record.central_extra = extra(1, &[]);
        assert!(matches!(
            admit(&archive(&[record], &[0], 0)),
            Err(ReadError::Unsupported(_))
        ));
    }

    #[test]
    fn central_offset_only_zip64_without_descriptor_is_admitted() {
        let mut record = Record::stored(b"a", b"data");
        record.central_extra = extra(1, &0u64.to_le_bytes());
        let mut bytes = archive(&[record], &[0], 0);
        let cd = directory(&bytes);
        put32(&mut bytes, cd + 42, u32::MAX);
        admit(&bytes).unwrap();
    }

    #[test]
    fn descriptor_crc_signature_ambiguity_is_decided_by_physical_boundary() {
        let word = 0x08074b50u32;
        let bytes = [word.to_le_bytes(); 4].concat();
        let ends = descriptor(
            &mut Cursor::new(&bytes),
            0,
            16,
            u64::from(word),
            u64::from(word),
            false,
        )
        .unwrap();
        assert_eq!(ends.select(12).unwrap(), 12);
        assert!(matches!(ends.select(16), Err(ReadError::Unsupported(_))));
        assert!(matches!(ends.select(11), Err(ReadError::Unsupported(_))));
        assert!(matches!(
            descriptor(
                &mut Cursor::new(&bytes),
                0,
                11,
                u64::from(word),
                u64::from(word),
                false
            ),
            Err(ReadError::InvalidInput(_))
        ));
    }

    #[test]
    fn source_directory_count_member_and_total_limits_are_independent() {
        let bytes = stored_zip(&[(b"a", b"data"), (b"b", b"more")]);
        let cd_bytes = u32_at(&bytes, bytes.len() - 10);
        let exact = ReadLimits {
            source_archive_bytes: bytes.len() as u64,
            central_directory_bytes: cd_bytes,
            archive_entries: 2,
            member_bytes: 4,
            archive_stored_bytes: 8,
        };
        scan(&mut Cursor::new(&bytes), bytes.len() as u64, &exact).unwrap();
        for field in 0..5 {
            let mut bounded = exact;
            match field {
                0 => bounded.source_archive_bytes -= 1,
                1 => bounded.central_directory_bytes -= 1,
                2 => bounded.archive_entries -= 1,
                3 => bounded.member_bytes -= 1,
                _ => bounded.archive_stored_bytes -= 1,
            }
            assert!(matches!(
                scan(&mut Cursor::new(&bytes), bytes.len() as u64, &bounded),
                Err(ReadError::ResourceLimit(_))
            ));
        }
    }

    /// Replaces a classic end record with a ZIP64 end declaring `count`
    /// entries on both count fields. Returns the ZIP64 record offset.
    fn zip64_end(mut bytes: Vec<u8>, count: u64) -> (Vec<u8>, usize) {
        let mut classic = bytes.split_off(bytes.len() - 22);
        let directory_size = u32_at(&classic, 12);
        let directory = u32_at(&classic, 16);
        let record_offset = bytes.len();
        let mut record = [0; 56];
        record[..4].copy_from_slice(b"PK\x06\x06");
        record[4..12].copy_from_slice(&44u64.to_le_bytes());
        put16(&mut record, 12, 45);
        put16(&mut record, 14, 45);
        record[24..32].copy_from_slice(&count.to_le_bytes());
        record[32..40].copy_from_slice(&count.to_le_bytes());
        record[40..48].copy_from_slice(&directory_size.to_le_bytes());
        record[48..56].copy_from_slice(&directory.to_le_bytes());
        bytes.extend(record);
        let mut locator = [0; 20];
        locator[..4].copy_from_slice(b"PK\x06\x07");
        locator[8..16].copy_from_slice(&(record_offset as u64).to_le_bytes());
        put32(&mut locator, 16, 1);
        bytes.extend(locator);
        put16(&mut classic, 8, u16::MAX);
        put16(&mut classic, 10, u16::MAX);
        bytes.extend(classic);
        (bytes, record_offset)
    }

    #[test]
    fn archive_zip64_counts_do_not_change_member_descriptor_width() {
        let bytes = archive(
            &[Record::stored(b"a", b"data").described(false, true)],
            &[0],
            0,
        );
        let count = u64::from(u16_at(&bytes, bytes.len() - 12));
        let (mut bytes, record_offset) = zip64_end(bytes, count);
        admit(&bytes).unwrap();
        bytes[record_offset + 32] = 2;
        assert!(matches!(admit(&bytes), Err(ReadError::InvalidInput(_))));
    }

    #[test]
    fn stored_encoding_and_encryption_profile_is_explicit() {
        let mut record = Record::stored(b"ascii", b"");
        record.flags = 0;
        admit(&archive(&[record.clone()], &[0], 0)).unwrap();
        record.name = "café".as_bytes();
        assert!(matches!(
            admit(&archive(&[record.clone()], &[0], 0)),
            Err(ReadError::Unsupported(_))
        ));
        record.flags = 0x800;
        admit(&archive(&[record.clone()], &[0], 0)).unwrap();
        record.name = &[0xff];
        assert!(matches!(
            admit(&archive(&[record.clone()], &[0], 0)),
            Err(ReadError::InvalidInput(_))
        ));
        record.flags = 0;
        assert!(matches!(
            admit(&archive(&[record.clone()], &[0], 0)),
            Err(ReadError::Unsupported(_))
        ));
        record.name = b"ascii";
        record.flags = 1;
        assert!(matches!(
            admit(&archive(&[record], &[0], 0)),
            Err(ReadError::Unsupported(_))
        ));
        let mut bytes = stored_zip(&[(b"a", b"")]);
        let cd = directory(&bytes);
        put16(&mut bytes, cd + 10, 8);
        assert!(matches!(admit(&bytes), Err(ReadError::InvalidInput(_))));
    }

    struct FailingReader {
        seek: bool,
        reads: usize,
    }
    impl Read for FailingReader {
        fn read(&mut self, _: &mut [u8]) -> io::Result<usize> {
            self.reads += 1;
            Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "injected read cause",
            ))
        }
    }
    impl Seek for FailingReader {
        fn seek(&mut self, _: SeekFrom) -> io::Result<u64> {
            if self.seek {
                Err(io::Error::new(
                    io::ErrorKind::PermissionDenied,
                    "injected seek cause",
                ))
            } else {
                Ok(0)
            }
        }
    }
    #[test]
    fn limit_before_io_and_underlying_read_seek_causes_are_preserved() {
        let mut reader = FailingReader {
            seek: false,
            reads: 0,
        };
        assert!(matches!(
            scan(&mut reader, 4097, &limits()),
            Err(ReadError::ResourceLimit(_))
        ));
        assert_eq!(reader.reads, 0);
        assert!(
            matches!(scan(&mut reader, 22, &limits()), Err(ReadError::Io(e)) if e.kind() == io::ErrorKind::InvalidData)
        );
        reader.seek = true;
        assert!(
            matches!(scan(&mut reader, 22, &limits()), Err(ReadError::Io(e)) if e.kind() == io::ErrorKind::PermissionDenied)
        );
        assert!(matches!(
            scan(&mut Cursor::new([]), 22, &limits()),
            Err(ReadError::InvalidInput(_))
        ));
    }

    fn natural_limits(length: usize) -> ReadLimits {
        let length = length as u64;
        ReadLimits {
            source_archive_bytes: length,
            central_directory_bytes: length,
            archive_entries: length / 46,
            member_bytes: length,
            archive_stored_bytes: length,
        }
    }

    #[test]
    fn cursor_and_held_file_share_catalog_index_and_payload_meaning() {
        let bytes = indexed(&[
            Record::stored(b"tileset.json", b"{}"),
            Record::stored(b"data", b"payload"),
        ]);
        let limits = natural_limits(bytes.len());
        let mut cursor =
            StoredArchive::new(Cursor::new(&bytes), bytes.len() as u64, &limits).unwrap();
        let mut file = tempfile::tempfile().unwrap();
        std::io::Write::write_all(&mut file, &bytes).unwrap();
        let mut held = StoredArchive::new(file, bytes.len() as u64, &limits).unwrap();
        assert_eq!(cursor.len(), 3);
        assert_eq!(cursor.index_read_bytes(), 48);
        let (a, b) = (cursor.entries(), held.entries());
        assert_eq!(a.len(), 3);
        for index in 0..a.len() {
            let (a, b) = (&a[index], &b[index]);
            assert_eq!(
                (&a.name, a.data_offset, a.size, a.crc32),
                (&b.name, b.data_offset, b.size, b.crc32)
            );
            assert_eq!(
                cursor.hash_member(index).unwrap(),
                held.hash_member(index).unwrap()
            );
        }
        assert_eq!(cursor.read_member("data", 7).unwrap(), b"payload");
        assert!(matches!(
            cursor.read_member("data", 6),
            Err(ReadError::ResourceLimit(_))
        ));
        assert!(matches!(
            cursor.member("missing"),
            Err(ReadError::MissingMember(_))
        ));
        assert!(matches!(
            cursor.hash_member(3),
            Err(ReadError::MissingMember(_))
        ));
    }

    #[test]
    fn empty_member_crc_is_verified_by_both_read_and_hash() {
        let manifest = Record::stored(b"tileset.json", b"{}");
        let empty = Record::stored(b"empty", b"");
        let empty_offset = local(&manifest).len();
        let mut bytes = indexed(&[manifest.clone(), empty]);
        let cd = directory(&bytes);
        let empty_cd = cd + central(&manifest, 0).len();
        put32(&mut bytes, empty_offset + 14, 1);
        put32(&mut bytes, empty_cd + 16, 1);
        let mut owner = StoredArchive::new(
            Cursor::new(&bytes),
            bytes.len() as u64,
            &natural_limits(bytes.len()),
        )
        .unwrap();
        assert!(matches!(
            owner.read_member("empty", 0),
            Err(ReadError::InvalidInput(_))
        ));
        assert!(matches!(
            owner.hash_member(1),
            Err(ReadError::InvalidInput(_))
        ));
    }

    #[test]
    fn payload_corruption_is_not_claimed_read_by_catalog_or_index_admission() {
        let manifest = Record::stored(b"tileset.json", b"{}");
        let mut bytes = indexed(&[manifest.clone(), Record::stored(b"data", b"payload")]);
        let start = local(&manifest).len() + 30 + b"data".len();
        bytes[start] ^= 1;
        let mut owner = StoredArchive::new(
            Cursor::new(&bytes),
            bytes.len() as u64,
            &natural_limits(bytes.len()),
        )
        .unwrap();
        assert!(matches!(
            owner.read_member("data", 7),
            Err(ReadError::InvalidInput(_))
        ));
        assert!(matches!(
            owner.hash_member(1),
            Err(ReadError::InvalidInput(_))
        ));
    }

    struct RangeReader {
        inner: Cursor<Vec<u8>>,
        guard: Option<(u64, u64)>,
        consumed: u64,
        max_request: usize,
        fault: Option<io::ErrorKind>,
    }
    impl Read for RangeReader {
        fn read(&mut self, bytes: &mut [u8]) -> io::Result<usize> {
            if let Some((start, end)) = self.guard {
                let position = self.inner.position();
                if position < start || position >= end {
                    return Err(io::Error::other("read outside admitted member"));
                }
                if let Some(kind) = self.fault {
                    return Err(io::Error::new(kind, "injected member read cause"));
                }
                assert!(position + bytes.len() as u64 <= end);
                self.max_request = self.max_request.max(bytes.len());
            }
            let count = self.inner.read(bytes)?;
            self.consumed += count as u64;
            Ok(count)
        }
    }
    impl Seek for RangeReader {
        fn seek(&mut self, from: SeekFrom) -> io::Result<u64> {
            self.inner.seek(from)
        }
    }

    #[test]
    fn hash_streams_at_most_64k_and_stops_at_exact_admitted_size() {
        let data = vec![0x5a; 65537];
        let bytes = indexed(&[
            Record::stored(b"tileset.json", b"{}"),
            Record::stored(b"data", &data),
        ]);
        let length = bytes.len();
        let reader = RangeReader {
            inner: Cursor::new(bytes),
            guard: None,
            consumed: 0,
            max_request: 0,
            fault: None,
        };
        let mut owner = StoredArchive::new(reader, length as u64, &natural_limits(length)).unwrap();
        let member = owner.member("data").unwrap();
        owner.reader.guard = Some((member.data_offset, member.data_offset + member.size));
        owner.reader.consumed = 0;
        assert_eq!(
            owner.hash_member(1).unwrap().as_slice(),
            Sha256::digest(&data).as_slice()
        );
        assert_eq!(owner.reader.consumed, 65537);
        assert_eq!(owner.reader.max_request, 65536);
        owner.reader.fault = Some(io::ErrorKind::InvalidData);
        assert!(
            matches!(owner.hash_member(1), Err(ReadError::Io(e)) if e.kind() == io::ErrorKind::InvalidData)
        );
        assert!(
            matches!(owner.read_member("data", 65537), Err(ReadError::Io(e)) if e.kind() == io::ErrorKind::InvalidData)
        );
        owner.reader.fault = Some(io::ErrorKind::UnexpectedEof);
        assert!(
            matches!(owner.hash_member(1), Err(ReadError::Io(e)) if e.kind() == io::ErrorKind::UnexpectedEof)
        );
        assert!(
            matches!(owner.read_member("data", 65537), Err(ReadError::Io(e)) if e.kind() == io::ErrorKind::UnexpectedEof)
        );
        owner.reader.fault = None;
        let data_start = owner.reader.guard.unwrap().0 as usize;
        owner.reader.inner.get_mut().truncate(data_start);
        assert!(matches!(
            owner.hash_member(1),
            Err(ReadError::InvalidInput(_))
        ));
        assert!(matches!(
            owner.read_member("data", 65537),
            Err(ReadError::InvalidInput(_))
        ));
    }

    struct DirectoryFault {
        inner: Cursor<Vec<u8>>,
        offset: u64,
        seek_fault: bool,
    }
    impl Read for DirectoryFault {
        fn read(&mut self, bytes: &mut [u8]) -> io::Result<usize> {
            if self.inner.position() == self.offset {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "actual directory read cause",
                ));
            }
            self.inner.read(bytes)
        }
    }
    impl Seek for DirectoryFault {
        fn seek(&mut self, from: SeekFrom) -> io::Result<u64> {
            if self.seek_fault && matches!(from, SeekFrom::Start(n) if n == self.offset) {
                return Err(io::Error::new(
                    io::ErrorKind::PermissionDenied,
                    "actual directory seek cause",
                ));
            }
            self.inner.seek(from)
        }
    }
    #[test]
    fn selected_directory_read_and_seek_faults_are_not_retried_or_reclassified() {
        let mut manifest = Record::stored(b"tileset.json", b"{}");
        manifest.central_extra = extra(0x9999, b"opaque");
        let bytes = indexed(&[manifest]);
        let length = bytes.len();
        let directory = directory(&bytes) as u64;
        // The directory is read in chunks from its first record; the actual read or seek
        // of a chunk must retain its underlying I/O cause, as must the local header read.
        let local = 0u64;
        for offset in [directory, local] {
            for seek_fault in [false, true] {
                let source = DirectoryFault {
                    inner: Cursor::new(bytes.clone()),
                    offset,
                    seek_fault,
                };
                let (expected_kind, expected_cause) = if seek_fault {
                    (
                        io::ErrorKind::PermissionDenied,
                        "actual directory seek cause",
                    )
                } else {
                    (io::ErrorKind::InvalidData, "actual directory read cause")
                };
                assert!(
                    matches!(StoredArchive::new(source, length as u64, &natural_limits(length)), Err(ReadError::Io(e)) if e.kind() == expected_kind && e.to_string() == expected_cause)
                );
            }
        }
    }

    enum DirectoryRead {
        Error(io::ErrorKind),
        Exhausted,
        InterruptedOnce,
    }
    struct DirectoryReadSource {
        inner: Cursor<Vec<u8>>,
        offset: u64,
        behavior: DirectoryRead,
        interrupted: bool,
    }
    impl Read for DirectoryReadSource {
        fn read(&mut self, bytes: &mut [u8]) -> io::Result<usize> {
            if self.inner.position() == self.offset {
                match self.behavior {
                    DirectoryRead::Error(kind) => {
                        return Err(io::Error::new(kind, "underlying directory error identity"));
                    }
                    DirectoryRead::Exhausted => return Ok(0),
                    DirectoryRead::InterruptedOnce if !self.interrupted => {
                        self.interrupted = true;
                        return Err(io::Error::from(io::ErrorKind::Interrupted));
                    }
                    DirectoryRead::InterruptedOnce => {}
                }
            }
            self.inner.read(bytes)
        }
    }
    impl Seek for DirectoryReadSource {
        fn seek(&mut self, from: SeekFrom) -> io::Result<u64> {
            self.inner.seek(from)
        }
    }
    #[test]
    fn actual_directory_unexpected_eof_is_io_but_observed_exhaustion_is_malformed() {
        let bytes = indexed(&[Record::stored(b"tileset.json", b"{}")]);
        let length = bytes.len();
        let offset = directory(&bytes) as u64;
        let source = |behavior| DirectoryReadSource {
            inner: Cursor::new(bytes.clone()),
            offset,
            behavior,
            interrupted: false,
        };
        assert!(
            matches!(StoredArchive::new(source(DirectoryRead::Error(io::ErrorKind::UnexpectedEof)), length as u64, &natural_limits(length)),
            Err(ReadError::Io(error)) if error.kind() == io::ErrorKind::UnexpectedEof && error.to_string() == "underlying directory error identity")
        );
        assert!(
            matches!(StoredArchive::new(source(DirectoryRead::Exhausted), length as u64, &natural_limits(length)),
            Err(ReadError::InvalidInput(message)) if message == "truncated stored archive record")
        );
        let owner = StoredArchive::new(
            source(DirectoryRead::InterruptedOnce),
            length as u64,
            &natural_limits(length),
        )
        .unwrap();
        assert!(owner.reader.interrupted);
        assert_eq!(owner.len(), 2);
    }

    #[test]
    fn impossible_declared_count_fails_before_any_directory_read_or_catalog_allocation() {
        let mut bytes = indexed(&[Record::stored(b"tileset.json", b"{}")]);
        let length = bytes.len();
        let end = length - 22;
        put16(&mut bytes, end + 8, 1000);
        put16(&mut bytes, end + 10, 1000);
        let source = DirectoryFault {
            offset: directory(&bytes) as u64,
            inner: Cursor::new(bytes),
            seek_fault: false,
        };
        let mut generous = natural_limits(length);
        generous.archive_entries = 1000;
        assert!(
            matches!(StoredArchive::new(source, length as u64, &generous), Err(ReadError::InvalidInput(message)) if message.contains("count exceeds directory length"))
        );
    }

    #[test]
    fn selected_outer_directory_does_not_fall_back_to_valid_embedded_end_record() {
        let nested = indexed(&[Record::stored(b"tileset.json", b"{}")]);
        let mut outer_manifest = Record::stored(b"tileset.json", b"outer");
        // Fully framed opaque timestamp bytes previously triggered lower-reader
        // parse/retry behavior. The selected catalog does not interpret them.
        outer_manifest.central_extra = extra(0x5455, &[0x99]);
        let outer = indexed(&[outer_manifest, Record::stored(b"nested.zip", &nested)]);
        let mut owner = StoredArchive::new(
            Cursor::new(&outer),
            outer.len() as u64,
            &natural_limits(outer.len()),
        )
        .unwrap();
        assert_eq!(owner.len(), 3);
        assert_eq!(owner.read_member("tileset.json", 5).unwrap(), b"outer");
        assert_eq!(
            owner
                .read_member("nested.zip", nested.len() as u64)
                .unwrap(),
            nested
        );
        let absent = indexed(&[Record::stored(b"other", &nested)]);
        assert!(
            matches!(StoredArchive::new(Cursor::new(&absent), absent.len() as u64, &natural_limits(absent.len())), Err(ReadError::MissingMember(name)) if name == "tileset.json")
        );
    }

    fn ceiling_refusal(count: u64) -> String {
        format!("3TZ member ceiling: {count} members exceed 178956970")
    }

    #[test]
    fn member_ceiling_is_a_typed_refusal_before_limits_and_allocation() {
        let base = indexed(&[Record::stored(b"tileset.json", b"{}")]);
        let ceiling = super::super::MEMBER_CEILING;
        assert_eq!(ceiling, 178_956_970);
        let generous = ReadLimits {
            archive_entries: u64::MAX,
            central_directory_bytes: u64::MAX,
            ..limits()
        };
        for count in [ceiling + 1, ceiling + 2, u64::MAX] {
            let (bytes, _) = zip64_end(base.clone(), count);
            // Refused before the 16-entry limit, and before the declared
            // directory length could refute the count.
            for limits in [limits(), generous] {
                assert!(
                    matches!(scan(&mut Cursor::new(&bytes), bytes.len() as u64, &limits),
                        Err(ReadError::Unsupported(message)) if message == ceiling_refusal(count))
                );
            }
        }
        // Exactly the ceiling is admitted by the format rule; the engineering
        // limit or the directory length then decides.
        let (bytes, _) = zip64_end(base, ceiling);
        assert!(
            matches!(admit(&bytes), Err(ReadError::ResourceLimit(message)) if message == "archive exceeds entry limit")
        );
        assert!(
            matches!(scan(&mut Cursor::new(&bytes), bytes.len() as u64, &generous),
                Err(ReadError::InvalidInput(message)) if message == "ZIP entry count exceeds directory length")
        );
    }

    #[test]
    fn validate_reports_the_member_ceiling_as_unsupported() {
        let count = super::super::MEMBER_CEILING + 1;
        let (bytes, _) = zip64_end(indexed(&[Record::stored(b"tileset.json", b"{}")]), count);
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("ceiling.3tz");
        std::fs::write(&path, bytes).unwrap();
        let failure =
            crate::validate::inspect(crate::validate::ValidationRequest::new(&path)).unwrap_err();
        assert!(
            matches!(&failure, crate::validate::ValidationFailure::Unsupported(message) if *message == ceiling_refusal(count)),
            "{failure:?}"
        );
    }

    /// `tileset.json` plus `payloads` empty members, so the index exceeds one
    /// streamed chunk when `payloads >= INDEX_CHUNK_ROWS`.
    fn empty_member_names(payloads: usize) -> Vec<String> {
        (0..payloads).map(|i| format!("m{i:05}")).collect()
    }
    fn index_archive(names: &[String], edit: impl FnOnce(&mut Vec<([u8; 16], u64)>)) -> Vec<u8> {
        let mut records = vec![Record::stored(b"tileset.json", b"{}")];
        records.extend(
            names
                .iter()
                .map(|name| Record::stored(name.as_bytes(), b"")),
        );
        indexed_rows(&records, edit)
    }
    /// Natural limits with a per-member limit far below the index length.
    fn tight_member_limits(length: usize) -> ReadLimits {
        ReadLimits {
            member_bytes: 2,
            ..natural_limits(length)
        }
    }
    fn open<'a>(
        bytes: &'a [u8],
        limits: &ReadLimits,
    ) -> Result<StoredArchive<Cursor<&'a [u8]>>, ReadError> {
        StoredArchive::new(Cursor::new(bytes), bytes.len() as u64, limits)
    }
    fn refused(bytes: &[u8], expected: &str) -> bool {
        matches!(open(bytes, &tight_member_limits(bytes.len())),
            Err(ReadError::InvalidInput(message)) if message == expected)
    }

    #[test]
    fn index_is_bounded_by_member_count_not_member_limit() {
        let names = empty_member_names(INDEX_CHUNK_ROWS + 270);
        let rows = names.len() + 1;
        let bytes = index_archive(&names, |_| {});
        let limits = tight_member_limits(bytes.len());
        let owner = open(&bytes, &limits).unwrap();
        assert_eq!(owner.len(), rows as u64 + 1);
        assert_eq!(owner.index_read_bytes(), 24 * rows as u64);
        assert!(owner.index_read_bytes() > INDEX_CHUNK_ROWS as u64 * 24);
        assert!(owner.index_read_bytes() > limits.member_bytes);
        // Payload members keep the per-member limit.
        let bounded = ReadLimits {
            member_bytes: 1,
            ..limits
        };
        assert!(
            matches!(open(&bytes, &bounded), Err(ReadError::ResourceLimit(message)) if message == "archive member exceeds byte limit")
        );
        // Only the last central entry is the index: an index-named payload
        // elsewhere is an ordinary member under the per-member limit.
        let index = [7; 48];
        let misplaced = archive(
            &[
                Record::stored(b"tileset.json", b"{}"),
                Record::stored(super::super::TZ_INDEX_NAME.as_bytes(), &index),
                Record::stored(b"z", b""),
            ],
            &[0, 1, 2],
            0,
        );
        assert!(
            matches!(open(&misplaced, &tight_member_limits(misplaced.len())),
                Err(ReadError::ResourceLimit(message)) if message == "archive member exceeds byte limit")
        );
    }

    #[test]
    fn index_rows_disagreeing_with_the_directory_count_are_refused() {
        let names = empty_member_names(INDEX_CHUNK_ROWS + 270);
        let exact = "3TZ index must contain every member exactly once";
        for edit in [
            (|rows: &mut Vec<([u8; 16], u64)>| rows.push(*rows.last().unwrap()))
                as fn(&mut Vec<([u8; 16], u64)>),
            |rows| {
                rows.pop();
            },
            |rows| {
                let last = *rows.last().unwrap();
                rows.extend(std::iter::repeat_n(last, 4096));
            },
            |rows| rows.truncate(1),
        ] {
            assert!(refused(&index_archive(&names, edit), exact));
        }
    }

    #[test]
    fn streamed_row_failures_across_chunks_keep_crc_precedence() {
        let names = empty_member_names(INDEX_CHUNK_ROWS + 270);
        let boundary = INDEX_CHUNK_ROWS;
        let ordered = "3TZ index hashes are not ordered";
        let differs = "3TZ index differs from admitted directory";
        // Defects straddling and following the first chunk boundary.
        assert!(refused(
            &index_archive(&names, |rows| rows.swap(boundary - 1, boundary)),
            ordered
        ));
        assert!(refused(
            &index_archive(&names, |rows| rows[boundary] = rows[boundary - 1]),
            differs
        ));
        assert!(refused(
            &index_archive(&names, |rows| rows.last_mut().unwrap().1 += 1),
            differs
        ));
        assert!(refused(
            &index_archive(&names, |rows| rows.last_mut().unwrap().0[15] ^= 1),
            differs
        ));
        // A row failure in the first chunk is held until the CRC over the
        // whole index, corrupted in its last chunk, has been checked.
        let mut bytes = index_archive(&names, |rows| rows.swap(0, 1));
        assert!(refused(&bytes, ordered));
        let end = directory(&bytes);
        bytes[end - 1] ^= 1;
        assert!(refused(&bytes, "stored member CRC mismatch"));
    }

    /// Admission at more than 5,592,406 index rows, beyond what a 128 MiB
    /// per-member limit allowed. Writes about 640 MB under
    /// `RUSTY_TILES_SCALE_DIR` (default: the system temporary directory).
    #[test]
    #[ignore = "writes a ~640 MB synthetic archive"]
    fn index_above_default_member_limit_is_admitted_at_scale() {
        use std::io::{BufWriter, Write};
        const MEMBERS: u64 = 5_600_001;
        let member_bytes = crate::validate::ValidationLimits::default().member_bytes;
        let rows = MEMBERS - 1;
        assert!(24 * rows > member_bytes);
        let dir = std::env::var_os("RUSTY_TILES_SCALE_DIR")
            .map(std::path::PathBuf::from)
            .unwrap_or_else(std::env::temp_dir);
        let file = tempfile::NamedTempFile::new_in(dir).unwrap();
        let name = |i: u64| {
            if i == 0 {
                b"tileset.json".to_vec()
            } else {
                format!("{i:07x}").into_bytes()
            }
        };
        let data = |i: u64| if i == 0 { &b"{}"[..] } else { &b""[..] };
        let local_len = |i: u64| 30 + name(i).len() as u64 + data(i).len() as u64;
        let local_header = |name: &[u8], data: &[u8]| {
            let mut record = Record::stored(name, data);
            record.flags = 0;
            local(&record)
        };
        let central_header = |name: &[u8], data: &[u8], offset: u64| {
            let mut record = Record::stored(name, data);
            record.flags = 0;
            central(&record, u32::try_from(offset).unwrap())
        };
        {
            let mut out = BufWriter::new(file.as_file());
            let mut index = Vec::with_capacity(rows as usize);
            let mut offset = 0u64;
            for i in 0..rows {
                let (name, data) = (name(i), data(i));
                index.push((md5::compute(&name).0, offset));
                out.write_all(&local_header(&name, data)).unwrap();
                offset += local_len(i);
            }
            index.sort_unstable_by_key(|(hash, _)| (u64_at(hash, 0), u64_at(hash, 8)));
            let mut crc = crc32fast::Hasher::new();
            for (hash, at) in &index {
                crc.update(hash);
                crc.update(&at.to_le_bytes());
            }
            let index_offset = offset;
            let index_name = super::super::TZ_INDEX_NAME.as_bytes();
            let mut header = local_header(index_name, b"");
            put32(&mut header, 14, crc.clone().finalize());
            put32(&mut header, 18, u32::try_from(24 * rows).unwrap());
            put32(&mut header, 22, u32::try_from(24 * rows).unwrap());
            out.write_all(&header).unwrap();
            for (hash, at) in &index {
                out.write_all(hash).unwrap();
                out.write_all(&at.to_le_bytes()).unwrap();
            }
            drop(index);
            let directory = index_offset + header.len() as u64 + 24 * rows;
            let mut offset = 0u64;
            for i in 0..rows {
                out.write_all(&central_header(&name(i), data(i), offset))
                    .unwrap();
                offset += local_len(i);
            }
            let mut central = central_header(index_name, b"", index_offset);
            put32(&mut central, 16, crc.finalize());
            put32(&mut central, 20, u32::try_from(24 * rows).unwrap());
            put32(&mut central, 24, u32::try_from(24 * rows).unwrap());
            out.write_all(&central).unwrap();
            let end = out.stream_position().unwrap();
            // ZIP64 end: the count exceeds the classic 16-bit field.
            let mut record = [0; 56];
            record[..4].copy_from_slice(b"PK\x06\x06");
            record[4..12].copy_from_slice(&44u64.to_le_bytes());
            put16(&mut record, 12, 45);
            put16(&mut record, 14, 45);
            record[24..32].copy_from_slice(&MEMBERS.to_le_bytes());
            record[32..40].copy_from_slice(&MEMBERS.to_le_bytes());
            record[40..48].copy_from_slice(&(end - directory).to_le_bytes());
            record[48..56].copy_from_slice(&directory.to_le_bytes());
            out.write_all(&record).unwrap();
            let mut locator = [0; 20];
            locator[..4].copy_from_slice(b"PK\x06\x07");
            locator[8..16].copy_from_slice(&end.to_le_bytes());
            put32(&mut locator, 16, 1);
            out.write_all(&locator).unwrap();
            let mut classic = [0; 22];
            classic[..4].copy_from_slice(b"PK\x05\x06");
            put16(&mut classic, 8, u16::MAX);
            put16(&mut classic, 10, u16::MAX);
            put32(&mut classic, 12, u32::MAX);
            put32(&mut classic, 16, u32::MAX);
            out.write_all(&classic).unwrap();
            out.flush().unwrap();
        }
        let length = file.as_file().metadata().unwrap().len();
        let limits = ReadLimits {
            source_archive_bytes: length,
            central_directory_bytes: length,
            archive_entries: MEMBERS,
            member_bytes,
            archive_stored_bytes: length,
        };
        let owner = StoredArchive::new(file.reopen().unwrap(), length, &limits).unwrap();
        assert_eq!(owner.len() as u64, MEMBERS);
        assert_eq!(owner.index_read_bytes(), 24 * rows);
    }

    // ---- VS-A catalog controls -------------------------------------------------------

    /// The pre-VS-A index check, kept as the independent reference: non-index members by
    /// offset, a binary search per row and a seen table, first failure in index order.
    fn reference_index(
        rows: &[([u8; 16], u64)],
        members: &[([u8; 16], u64)],
    ) -> Result<(), &'static str> {
        let mut offsets: Vec<usize> = (0..members.len()).collect();
        offsets.sort_by_key(|&i| members[i].1);
        let mut seen = vec![false; members.len()];
        let mut previous = None;
        for (hash, offset) in rows {
            let key = (u64_at(hash, 0), u64_at(hash, 8));
            if previous.is_some_and(|previous| previous > key) {
                return Err("3TZ index hashes are not ordered");
            }
            previous = Some(key);
            let Ok(position) = offsets.binary_search_by_key(offset, |&i| members[i].1) else {
                return Err("3TZ index differs from admitted directory");
            };
            let i = offsets[position];
            if seen[i] || members[i].0 != *hash {
                return Err("3TZ index differs from admitted directory");
            }
            seen[i] = true;
        }
        Ok(())
    }

    struct XorShift(u64);
    impl XorShift {
        fn next(&mut self) -> u64 {
            self.0 ^= self.0 << 13;
            self.0 ^= self.0 >> 7;
            self.0 ^= self.0 << 17;
            self.0
        }
        fn below(&mut self, n: usize) -> usize {
            (self.next() % n as u64) as usize
        }
    }

    fn admit_with<'a>(
        bytes: &'a [u8],
        config: CatalogConfig,
        parent: &Path,
    ) -> Result<StoredArchive<Cursor<&'a [u8]>>, ReadError> {
        StoredArchive::new_with(
            Cursor::new(bytes),
            bytes.len() as u64,
            &natural_limits(bytes.len()),
            config,
            Some(parent),
        )
    }

    fn outcome<R>(result: Result<R, ReadError>) -> Result<(), String> {
        result.map(|_| ()).map_err(|error| format!("{error:?}"))
    }

    /// Randomly defected indexes give the reference check's verdict under both catalog
    /// profiles: the merge-join and its exact diagnosis decide every first failure.
    #[test]
    fn index_merge_join_matches_the_sequential_reference() {
        let parent = tempfile::tempdir().unwrap();
        let names: Vec<Vec<u8>> = (0..40)
            .map(|i| format!("m/{i:03}.glb").into_bytes())
            .collect();
        let mut random = XorShift(0x9e37_79b9_7f4a_7c15);
        let (mut verdicts, mut trials) = (std::collections::BTreeMap::new(), 0);
        for trial in 0..600 {
            let count = 1 + random.below(names.len());
            let mut records = vec![Record::stored(b"tileset.json", b"{}")];
            records.extend(names[..count].iter().map(|name| Record::stored(name, b"x")));
            let mut offset = 0u64;
            let members: Vec<([u8; 16], u64)> = records
                .iter()
                .map(|record| {
                    let row = (md5::compute(record.name).0, offset);
                    offset += local(record).len() as u64;
                    row
                })
                .collect();
            let index_member = (md5::compute(TZ_INDEX).0, offset);
            let mut edited = Vec::new();
            let bytes = indexed_rows(&records, |rows| {
                for _ in 0..random.below(4) {
                    let n = rows.len();
                    let (a, b) = (random.below(n), random.below(n));
                    match random.below(8) {
                        0 => rows.swap(a, b),
                        1 => rows[a].0 = members[random.below(members.len())].0,
                        2 => rows[a].1 = members[random.below(members.len())].1,
                        3 => rows[a].1 = 7,
                        4 => rows[a] = rows[b],
                        5 => rows[a] = index_member,
                        6 => rows[a..].reverse(),
                        _ => rows[a].0[15] ^= 1,
                    }
                }
                edited = rows.clone();
            });
            let expected = reference_index(&edited, &members).map_err(str::to_owned);
            for config in [CatalogConfig::DEFAULT, CatalogConfig::FORCED_SPILL] {
                let actual = admit_with(&bytes, config, parent.path())
                    .map(|_| ())
                    .map_err(|error| match error {
                        ReadError::InvalidInput(message) => message,
                        other => panic!("trial {trial}: {other:?}"),
                    });
                assert_eq!(actual, expected, "trial {trial}, {config:?}");
            }
            *verdicts.entry(format!("{expected:?}")).or_insert(0) += 1;
            trials += 1;
        }
        // Sensitivity: every verdict occurs often.
        assert_eq!(verdicts.len(), 3, "{verdicts:?}");
        assert!(
            verdicts.values().all(|&n| n >= 50),
            "{verdicts:?} of {trials}"
        );
        assert_eq!(std::fs::read_dir(parent.path()).unwrap().count(), 0);
    }

    const TZ_INDEX: &[u8] = super::super::TZ_INDEX_NAME.as_bytes();

    /// MD5-colliding members cannot be written as valid UTF-8 names, so the diagnosis is
    /// checked on synthetic keys: equal MD5s with distinct offsets, in either index order.
    #[test]
    fn diagnosis_resolves_md5_collisions_by_offset() {
        let row = |md5: u64, offset, position| IndexRow {
            md5: [md5, 0],
            offset,
            position,
        };
        let diagnose = |index: &[(u64, u64)], members: &[(u64, u64)]| {
            let mut rows: Vec<_> = index
                .iter()
                .enumerate()
                .map(|(i, &(m, o))| row(m, o, i as u64))
                .collect();
            rows.sort();
            let mut pairs: Vec<_> = members.iter().map(|&(m, o)| row(m, o, 0)).collect();
            pairs.sort();
            first_failing_row(rows.into_iter().map(Ok), pairs.into_iter().map(Ok)).unwrap()
        };
        let members = [(1, 10), (5, 30), (5, 20), (9, 40)];
        // A colliding group in either order is admitted.
        assert_eq!(
            diagnose(&[(1, 10), (5, 20), (5, 30), (9, 40)], &members),
            None
        );
        assert_eq!(
            diagnose(&[(1, 10), (5, 30), (5, 20), (9, 40)], &members),
            None
        );
        // A repeated pair fails at its second row; an absent pair at its first.
        assert_eq!(
            diagnose(&[(1, 10), (5, 30), (5, 30), (9, 40)], &members),
            Some(2)
        );
        assert_eq!(
            diagnose(&[(1, 10), (5, 30), (5, 40), (9, 40)], &members),
            Some(2)
        );
        assert_eq!(
            diagnose(&[(5, 10), (5, 20), (5, 30), (9, 40)], &members),
            Some(0)
        );
        assert_eq!(
            diagnose(&[(1, 10), (5, 20), (5, 30), (9, 40), (9, 40)], &members),
            Some(4)
        );
        // Collisions in the catalog are matched by offset as well.
        let mut random = XorShift(7);
        for _ in 0..2000 {
            let members: Vec<(u64, u64)> =
                (0..8).map(|i| (random.below(3) as u64, 10 * i)).collect();
            let mut index = members.clone();
            index.sort_by_key(|&(m, _)| m);
            for _ in 0..random.below(3) {
                let (a, b) = (random.below(8), random.below(8));
                match random.below(3) {
                    0 => index[a].1 = index[b].1,
                    1 => index[a].0 = random.below(3) as u64,
                    _ => index.swap(a, b),
                }
            }
            index.sort_by_key(|&(m, _)| m);
            let digest = |m: u64| {
                let mut d = [0u8; 16];
                d[..8].copy_from_slice(&m.to_le_bytes());
                d
            };
            let as_rows = |list: &[(u64, u64)]| {
                list.iter()
                    .map(|&(m, o)| (digest(m), o))
                    .collect::<Vec<_>>()
            };
            let expected = reference_index(&as_rows(&index), &as_rows(&members)).is_err();
            assert_eq!(
                diagnose(&index, &members).is_some(),
                expected,
                "{index:?} {members:?}"
            );
        }
    }

    /// The forced-spill profile really spills (scratch appears during admission and is
    /// removed with the archive); the default keeps a small catalog in memory.
    #[test]
    fn forced_spill_profile_uses_scratch_and_default_does_not() {
        let parent = tempfile::tempdir().unwrap();
        let names: Vec<Vec<u8>> = (0..9).map(|i| format!("{i}.bin").into_bytes()).collect();
        let mut records = vec![Record::stored(b"tileset.json", b"{}")];
        records.extend(names.iter().map(|name| Record::stored(name, b"x")));
        let bytes = indexed(&records);
        let mut default = admit_with(&bytes, CatalogConfig::DEFAULT, parent.path()).unwrap();
        assert!(!default.scratch().is_created());
        let mut forced = admit_with(&bytes, CatalogConfig::FORCED_SPILL, parent.path()).unwrap();
        assert!(forced.scratch().is_created());
        assert_eq!(std::fs::read_dir(parent.path()).unwrap().count(), 1);
        for archive in [&mut default, &mut forced] {
            assert_eq!(archive.len(), 11);
            for name in names.iter().chain([&b"tileset.json".to_vec()]) {
                let name = std::str::from_utf8(name).unwrap();
                let member = archive.member(name).unwrap();
                assert_eq!(archive.name(member.rank).unwrap(), name);
            }
            assert!(archive.find("absent").unwrap().is_none());
            assert!(archive.find("0.bi").unwrap().is_none());
        }
        let ranks = |archive: &mut StoredArchive<_>| {
            (0..archive.len())
                .map(|rank| archive.name(rank).unwrap())
                .collect::<Vec<_>>()
        };
        assert_eq!(ranks(&mut default), ranks(&mut forced));
        assert_eq!(
            outcome(admit_with(&bytes, CatalogConfig::DEFAULT, parent.path())),
            outcome(admit_with(
                &bytes,
                CatalogConfig::FORCED_SPILL,
                parent.path()
            ))
        );
        drop((default, forced));
        assert_eq!(std::fs::read_dir(parent.path()).unwrap().count(), 0);
    }

    /// Duplicate names and overlapping records keep their messages and precedence on the
    /// spilled path.
    #[test]
    fn duplicate_and_overlap_failures_agree_across_profiles() {
        let parent = tempfile::tempdir().unwrap();
        let cases: Vec<Vec<u8>> = vec![
            indexed(&[
                Record::stored(b"tileset.json", b"{}"),
                Record::stored(b"a", b"1"),
                Record::stored(b"b", b"2"),
                Record::stored(b"a", b"3"),
            ]),
            {
                let a = Record::stored(b"tileset.json", b"{}");
                let b = Record::stored(b"b", b"2");
                let index = Record::stored(TZ_INDEX, b"");
                archive(&[a, b, index], &[0, 1, 2], 0)
            },
        ];
        for bytes in &cases {
            assert_eq!(
                outcome(admit_with(bytes, CatalogConfig::DEFAULT, parent.path())),
                outcome(admit_with(
                    bytes,
                    CatalogConfig::FORCED_SPILL,
                    parent.path()
                ))
            );
        }
        assert!(matches!(
            admit_with(&cases[0], CatalogConfig::FORCED_SPILL, parent.path()),
            Err(ReadError::InvalidInput(message)) if message == "duplicate ZIP member name"
        ));
    }
}
