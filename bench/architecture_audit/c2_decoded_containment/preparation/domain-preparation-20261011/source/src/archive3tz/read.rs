//! Finite stored ZIP/ZIP64 admission before directory/index allocation.
//! Caller owns the selected source identity, known byte length and resource policy.
use sha2::{Digest, Sha256};
use std::io::{self, Read, Seek, SeekFrom};

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
}

/// Raw central-directory identity and the admitted stored range.
pub(crate) struct StoredMember {
    pub name: String,
    pub size: u64,
    pub local_offset: u64,
    pub data_offset: u64,
    pub crc32: u32,
}

/// One selected end record, one admitted catalog, one owned source reader.
pub(crate) struct StoredArchive<R> {
    reader: R,
    members: Vec<StoredMember>,
    by_name: Vec<usize>,
    index_read_bytes: u64,
}

impl<R: Read + Seek> StoredArchive<R> {
    pub(crate) fn new(
        mut reader: R,
        known_length: u64,
        limits: &ReadLimits,
    ) -> Result<Self, ReadError> {
        let (members, by_name) = scan_directory(&mut reader, known_length, limits)?;
        let mut archive = Self {
            reader,
            members,
            by_name,
            index_read_bytes: 0,
        };
        archive.validate_index()?;
        Ok(archive)
    }

    pub(crate) fn len(&self) -> usize {
        self.members.len()
    }

    pub(crate) fn member_at(&self, index: usize) -> Result<&StoredMember, ReadError> {
        self.members
            .get(index)
            .ok_or_else(|| ReadError::MissingMember(format!("ordinal {index}")))
    }

    pub(crate) fn member(&self, name: &str) -> Result<&StoredMember, ReadError> {
        let at = self
            .by_name
            .binary_search_by(|&i| self.members[i].name.as_str().cmp(name))
            .map_err(|_| ReadError::MissingMember(name.into()))?;
        Ok(&self.members[self.by_name[at]])
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

    pub(crate) fn hash_member(&mut self, index: usize) -> Result<[u8; 32], ReadError> {
        let member = self.member_at(index)?;
        let (mut remaining, offset, expected_crc) = (member.size, member.data_offset, member.crc32);
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
        let length = u64::try_from(n - 1)
            .ok()
            .and_then(|n| n.checked_mul(24))
            .ok_or_else(|| malformed("3TZ index length overflows"))?;
        let last = &self.members[n - 1];
        if last.name != super::TZ_INDEX_NAME {
            return Err(malformed("last ZIP member must be @3dtilesIndex1@"));
        }
        if last.size != length {
            return Err(malformed(
                "3TZ index must contain every member exactly once",
            ));
        }
        // Cardinality is admitted before any index allocation. The catalog
        // supplies every association; no second directory interpretation.
        let bytes = self.read_member(super::TZ_INDEX_NAME, length)?;
        self.index_read_bytes = length;
        let mut offsets = Vec::new();
        offsets
            .try_reserve_exact(n - 1)
            .map_err(|_| ReadError::limit("bounded index offset table unavailable"))?;
        offsets.extend(0..n - 1);
        offsets.sort_unstable_by_key(|&i| self.members[i].local_offset);
        let mut seen = bounded_bytes(n - 1)?;
        let mut previous = None;
        for row in bytes.as_chunks::<24>().0 {
            let key = (u64_at(row, 0), u64_at(row, 8));
            if previous.is_some_and(|previous| previous > key) {
                return Err(malformed("3TZ index hashes are not ordered"));
            }
            previous = Some(key);
            let offset = u64_at(row, 16);
            let position = offsets
                .binary_search_by_key(&offset, |&i| self.members[i].local_offset)
                .map_err(|_| malformed("3TZ index differs from admitted directory"))?;
            let i = offsets[position];
            if seen[i] != 0 || md5::compute(self.members[i].name.as_bytes()).0 != row[..16] {
                return Err(malformed("3TZ index differs from admitted directory"));
            }
            seen[i] = 1;
        }
        Ok(())
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
#[derive(Clone, Copy, Debug)]
struct Extent {
    start: u64,
    ends: Ends,
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

fn scan_directory(
    file: &mut (impl Read + Seek),
    size: u64,
    limits: &ReadLimits,
) -> Result<(Vec<StoredMember>, Vec<usize>), ReadError> {
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
    let count = usize::try_from(entries)
        .map_err(|_| ReadError::limit("archive entry count exceeds host range"))?;
    let mut extents = Vec::new();
    extents
        .try_reserve_exact(count)
        .map_err(|_| ReadError::limit("bounded extent table allocation unavailable"))?;
    let mut members = Vec::new();
    members
        .try_reserve_exact(count)
        .map_err(|_| ReadError::limit("bounded catalog allocation unavailable"))?;
    let mut by_name = Vec::new();
    by_name
        .try_reserve_exact(count)
        .map_err(|_| ReadError::limit("bounded name ordinal table unavailable"))?;
    let mut cursor = offset;
    let mut total = 0u64;
    let mut name_bytes = 0u64;
    for _ in 0..entries {
        if cursor.checked_add(46).is_none_or(|n| n > stop) {
            return Err(malformed("truncated ZIP central entry"));
        }
        let mut h = [0; 46];
        read(file, cursor, &mut h)?;
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
        let mut name = bounded_bytes(name_size)?;
        read(file, cursor + 46, &mut name)?;
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
        let mut extra = bounded_bytes(extra_size)?;
        read(file, cursor + 46 + name_size as u64, &mut extra)?;
        let ([member, compressed, local_offset], disk, central_zip64) = zip64_sizes(
            [u32_at(&h, 24), u32_at(&h, 20), u32_at(&h, 42)],
            &extra,
            Some(u16_at(&h, 34)),
        )?;
        if member > limits.member_bytes {
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
        let mut local = [0; 30];
        read(file, local_offset, &mut local)?;
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
        let mut actual_name = bounded_bytes(local_name)?;
        read(file, local_offset + 30, &mut actual_name)?;
        if actual_name != name {
            return Err(malformed("local ZIP name differs from central directory"));
        }
        let mut local_extra = bounded_bytes(usize::from(u16_at(&local, 28)))?;
        read(
            file,
            local_offset + 30 + local_name as u64,
            &mut local_extra,
        )?;
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
            ends,
        });
        by_name.push(members.len());
        members.push(StoredMember {
            name: String::from_utf8(name)
                .map_err(|_| malformed("invalid UTF-8 ZIP member name"))?,
            size: member,
            local_offset,
            data_offset: data_start,
            crc32: crc as u32,
        });
        total = total
            .checked_add(member)
            .filter(|&n| n <= limits.archive_stored_bytes)
            .ok_or_else(|| ReadError::limit("stored archive payload exceeds byte limit"))?;
        cursor = next;
    }
    if cursor != stop {
        return Err(malformed("ZIP directory length/count mismatch"));
    }
    by_name.sort_unstable_by(|&a, &b| members[a].name.cmp(&members[b].name));
    if by_name
        .windows(2)
        .any(|pair| members[pair[0]].name == members[pair[1]].name)
    {
        return Err(malformed("duplicate ZIP member name"));
    }
    extents.sort_unstable_by_key(|extent| extent.start);
    for (index, extent) in extents.iter().enumerate() {
        let boundary = extents.get(index + 1).map_or(offset, |next| next.start);
        extent.ends.select(boundary)?;
    }
    Ok((members, by_name))
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use std::io::Cursor;

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
        scan_directory(&mut Cursor::new(bytes), bytes.len() as u64, &limits()).map(|_| ())
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
        scan_directory(&mut Cursor::new(&bytes), bytes.len() as u64, &exact).unwrap();
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
                scan_directory(&mut Cursor::new(&bytes), bytes.len() as u64, &bounded),
                Err(ReadError::ResourceLimit(_))
            ));
        }
    }

    #[test]
    fn archive_zip64_counts_do_not_change_member_descriptor_width() {
        let mut bytes = archive(
            &[Record::stored(b"a", b"data").described(false, true)],
            &[0],
            0,
        );
        let classic = bytes.split_off(bytes.len() - 22);
        let count = u16_at(&classic, 10);
        let directory_size = u32_at(&classic, 12);
        let directory = u32_at(&classic, 16);
        let record_offset = bytes.len() as u64;
        let mut record = [0; 56];
        record[..4].copy_from_slice(b"PK\x06\x06");
        record[4..12].copy_from_slice(&44u64.to_le_bytes());
        put16(&mut record, 12, 45);
        put16(&mut record, 14, 45);
        record[24..32].copy_from_slice(&u64::from(count).to_le_bytes());
        record[32..40].copy_from_slice(&u64::from(count).to_le_bytes());
        record[40..48].copy_from_slice(&directory_size.to_le_bytes());
        record[48..56].copy_from_slice(&directory.to_le_bytes());
        bytes.extend(record);
        let mut locator = [0; 20];
        locator[..4].copy_from_slice(b"PK\x06\x07");
        locator[8..16].copy_from_slice(&record_offset.to_le_bytes());
        put32(&mut locator, 16, 1);
        bytes.extend(locator);
        let mut classic = classic;
        put16(&mut classic, 8, u16::MAX);
        put16(&mut classic, 10, u16::MAX);
        bytes.extend(classic);
        admit(&bytes).unwrap();
        let record_offset = record_offset as usize;
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
            scan_directory(&mut reader, 4097, &limits()),
            Err(ReadError::ResourceLimit(_))
        ));
        assert_eq!(reader.reads, 0);
        assert!(
            matches!(scan_directory(&mut reader, 22, &limits()), Err(ReadError::Io(e)) if e.kind() == io::ErrorKind::InvalidData)
        );
        reader.seek = true;
        assert!(
            matches!(scan_directory(&mut reader, 22, &limits()), Err(ReadError::Io(e)) if e.kind() == io::ErrorKind::PermissionDenied)
        );
        assert!(matches!(
            scan_directory(&mut Cursor::new([]), 22, &limits()),
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
        for index in 0..cursor.len() {
            let a = cursor.member_at(index).unwrap();
            let b = held.member_at(index).unwrap();
            assert_eq!(
                (&a.name, a.local_offset, a.data_offset, a.size, a.crc32),
                (&b.name, b.local_offset, b.data_offset, b.size, b.crc32)
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
            cursor.member_at(3),
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
        // Separate actual reads/seeks of the fixed header, raw name, and a
        // nonempty framed extra must all retain their underlying I/O cause.
        for offset in [
            directory,
            directory + 46,
            directory + 46 + b"tileset.json".len() as u64,
        ] {
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
}
