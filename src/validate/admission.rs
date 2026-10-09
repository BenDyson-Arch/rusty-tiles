//! Admission before ZIP directory allocation, hashing or index materialization.
use super::{ValidationFailure as Failure, ValidationLimits};
use std::{
    fs::File,
    io::{Read, Seek, SeekFrom},
};

fn malformed(message: &str) -> Failure {
    Failure::invalid(message)
}
fn read(file: &mut File, offset: u64, bytes: &mut [u8]) -> Result<(), Failure> {
    file.seek(SeekFrom::Start(offset))?;
    file.read_exact(bytes).map_err(|e| {
        if e.kind() == std::io::ErrorKind::UnexpectedEof {
            malformed("truncated ZIP directory")
        } else {
            Failure::Io(e)
        }
    })
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

fn zip64_sizes<const N: usize>(mut values: [u64; N], extra: &[u8]) -> Result<[u64; N], Failure> {
    if values.contains(&u64::from(u32::MAX)) {
        let mut cursor = 0;
        let mut record = None;
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
            cursor = end;
        }
        let record = record.ok_or_else(|| malformed("missing ZIP64 sizes/offset"))?;
        let mut cursor = 0;
        for value in &mut values {
            if *value == u64::from(u32::MAX) {
                if record.len() - cursor < 8 {
                    return Err(malformed("truncated ZIP64 sizes/offset"));
                }
                *value = u64_at(record, cursor);
                cursor += 8;
            }
        }
    }
    Ok(values)
}

fn descriptor(
    file: &mut File,
    offset: u64,
    boundary: u64,
    crc: u64,
    size: u64,
    zip64: bool,
) -> Result<(), Failure> {
    // A signature is optional. Trying both layouts also handles a CRC whose
    // value equals the signature without assigning it the wrong interpretation.
    let mut bytes = [0; 24];
    let length = (boundary - offset).min(bytes.len() as u64) as usize;
    read(file, offset, &mut bytes[..length])?;
    let width = if zip64 { 8 } else { 4 };
    let matches = [0, 4].into_iter().any(|start| {
        if start + 4 + width * 2 > length
            || (start == 4 && u32_at(&bytes, 0) != 0x08074b50)
            || u32_at(&bytes, start) != crc
        {
            return false;
        }
        let value = |at| {
            if zip64 {
                u64_at(&bytes, at)
            } else {
                u32_at(&bytes, at)
            }
        };
        value(start + 4) == size && value(start + 4 + width) == size
    });
    if !matches {
        return Err(malformed(
            "ZIP data descriptor differs from central directory",
        ));
    }
    Ok(())
}

pub(super) fn admit(file: &mut File, limits: &ValidationLimits) -> Result<(), Failure> {
    let metadata = file.metadata()?;
    if !metadata.is_file() {
        return Err(Failure::unsupported(
            "validation requires a regular .3tz file",
        ));
    }
    let size = metadata.len();
    if size > limits.source_archive_bytes {
        return Err(Failure::limit("source archive exceeds 1 GiB"));
    }
    if size < 22 {
        return Err(malformed("truncated ZIP end record"));
    }
    let mut magic = [0; 4];
    read(file, 0, &mut magic)?;
    if !matches!(&magic, b"PK\x03\x04" | b"PK\x05\x06") {
        return Err(malformed("validate currently checks .3tz archives only; raster and terrain output directories are not validated yet"));
    }
    let n = size.min(22 + u64::from(u16::MAX));
    let start = size - n;
    let mut tail = vec![0; n as usize];
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
        return Err(Failure::unsupported(
            "multi-disk ZIP archives are unsupported",
        ));
    }
    let end_offset = start + end as u64;
    let (entries, bytes, offset, boundary) = if u16_at(e, 10) == u16::MAX
        || u32_at(e, 12) == u64::from(u32::MAX)
        || u32_at(e, 16) == u64::from(u32::MAX)
    {
        let location = end_offset
            .checked_sub(20)
            .ok_or_else(|| malformed("missing ZIP64 locator"))?;
        let mut locator = [0; 20];
        read(file, location, &mut locator)?;
        if &locator[..4] != b"PK\x06\x07" || u32_at(&locator, 4) != 0 || u32_at(&locator, 16) != 1 {
            return Err(malformed("invalid ZIP64 locator"));
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
        if u32_at(&record, 16) != 0
            || u32_at(&record, 20) != 0
            || u64_at(&record, 24) != u64_at(&record, 32)
        {
            return Err(Failure::unsupported(
                "multi-disk ZIP64 archives are unsupported",
            ));
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
        return Err(Failure::limit("archive exceeds 65,536 entries"));
    }
    if bytes > limits.central_directory_bytes {
        return Err(Failure::limit("ZIP central directory exceeds 16 MiB"));
    }
    let stop = offset
        .checked_add(bytes)
        .filter(|&n| n <= boundary)
        .ok_or_else(|| malformed("ZIP central directory outside source"))?;
    let mut cursor = offset;
    let mut total = 0u64;
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
        if u16_at(&h, 8) & 1 != 0 {
            return Err(Failure::unsupported(
                "encrypted ZIP members are unsupported",
            ));
        }
        let name_size = usize::from(u16_at(&h, 28));
        let extra_size = usize::from(u16_at(&h, 30));
        let next = cursor
            .checked_add(46 + name_size as u64 + extra_size as u64 + u64::from(u16_at(&h, 32)))
            .filter(|&n| n <= stop)
            .ok_or_else(|| malformed("ZIP entry fields exceed central directory"))?;
        let mut name = vec![0; name_size];
        read(file, cursor + 46, &mut name)?;
        let mut extra = vec![0; extra_size];
        read(file, cursor + 46 + name_size as u64, &mut extra)?;
        let [member, compressed, local_offset] =
            zip64_sizes([u32_at(&h, 24), u32_at(&h, 20), u32_at(&h, 42)], &extra)?;
        if member > limits.member_bytes {
            return Err(Failure::limit("archive member exceeds 64 MiB"));
        }
        if compressed != member {
            return Err(malformed("stored member lengths differ"));
        }
        if u16_at(&h, 34) != 0 {
            return Err(Failure::unsupported(
                "multi-disk member references are unsupported",
            ));
        }
        // Admit the local header and payload range before any seek requested by
        // the ZIP library. Malformed offsets are input failures, including MAX.
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
        let mut actual_name = vec![0; local_name];
        read(file, local_offset + 30, &mut actual_name)?;
        if actual_name != name {
            return Err(malformed("local ZIP name differs from central directory"));
        }
        let mut local_extra = vec![0; usize::from(u16_at(&local, 28))];
        read(
            file,
            local_offset + 30 + local_name as u64,
            &mut local_extra,
        )?;
        let [local_member, local_compressed] =
            zip64_sizes([u32_at(&local, 22), u32_at(&local, 18)], &local_extra)?;
        let crc = u32_at(&h, 16);
        if u16_at(&local, 6) & 8 != 0 {
            let zip64 = [
                u32_at(&local, 18),
                u32_at(&local, 22),
                u32_at(&h, 20),
                u32_at(&h, 24),
            ]
            .contains(&u64::from(u32::MAX));
            descriptor(file, data_start + member, offset, crc, member, zip64)?;
        } else if u32_at(&local, 14) != crc
            || local_member != member
            || local_compressed != compressed
        {
            return Err(malformed(
                "local ZIP CRC/sizes differ from central directory",
            ));
        }
        total = total
            .checked_add(member)
            .filter(|&n| n <= limits.archive_stored_bytes)
            .ok_or_else(|| Failure::limit("stored archive payload exceeds 1 GiB"))?;
        cursor = next;
    }
    if cursor != stop {
        return Err(malformed("ZIP directory length/count mismatch"));
    }
    Ok(())
}
