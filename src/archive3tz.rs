//! Stored ZIP/ZIP64 and Maxar 3TZ index codec. No conversion-job dependency.
use std::{
    fs::File,
    io::{Read, Seek, SeekFrom, Write},
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc,
    },
};
pub const TZ_INDEX_NAME: &str = "@3dtilesIndex1@";

#[derive(Debug, thiserror::Error)]
pub(crate) enum CodecError {
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error("source I/O at {path:?}: {source}")]
    SourceIo {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error(transparent)]
    Zip(#[from] zip::result::ZipError),
    #[error("{0}")]
    Invalid(String),
    #[error("package missing tileset.json")]
    MissingManifest,
    #[error("source changed while packing: {0}")]
    SourceChanged(PathBuf),
}
impl CodecError {
    fn invalid(message: impl Into<String>) -> Self {
        Self::Invalid(message.into())
    }
}

pub(crate) struct Member<'a> {
    pub name: &'a str,
    pub source: &'a Path,
    pub size: u64,
    pub modified: Option<std::time::SystemTime>,
}
#[derive(Debug)]
pub(crate) enum WriteFailure<E> {
    Codec(CodecError),
    Checkpoint(E),
}
impl<E> From<std::io::Error> for WriteFailure<E> {
    fn from(error: std::io::Error) -> Self {
        Self::Codec(CodecError::Io(error))
    }
}
impl<E> From<zip::result::ZipError> for WriteFailure<E> {
    fn from(error: zip::result::ZipError) -> Self {
        Self::Codec(CodecError::Zip(error))
    }
}
struct Tracked<W> {
    writer: W,
    position: Arc<AtomicU64>,
}
impl<W: Write> Write for Tracked<W> {
    fn write(&mut self, b: &[u8]) -> std::io::Result<usize> {
        let n = self.writer.write(b)?;
        self.position.fetch_add(n as u64, Ordering::Relaxed);
        Ok(n)
    }
    fn flush(&mut self) -> std::io::Result<()> {
        self.writer.flush()
    }
}
impl<W: Seek> Seek for Tracked<W> {
    fn seek(&mut self, s: SeekFrom) -> std::io::Result<u64> {
        let p = self.writer.seek(s)?;
        self.position.store(p, Ordering::Relaxed);
        Ok(p)
    }
}

// F0 container profile: stored payload, UTF-8 names, no data descriptors,
// actual 32-bit local sizes/CRC; archive-level ZIP64 offsets/counts as needed.
// Field layouts follow PKWARE APPNOTE 6.3.9 sections 4.3.7, 4.3.12-16 and 4.5.3.
fn put16(bytes: &mut [u8], offset: usize, value: u16) {
    bytes[offset..offset + 2].copy_from_slice(&value.to_le_bytes());
}
fn put32(bytes: &mut [u8], offset: usize, value: u32) {
    bytes[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
}
fn put64(bytes: &mut [u8], offset: usize, value: u64) {
    bytes[offset..offset + 8].copy_from_slice(&value.to_le_bytes());
}

struct Entry<'a> {
    name: &'a str,
    size: u32,
    crc: u32,
    offset: u64,
}
fn local_header(writer: &mut (impl Write + Seek), name: &str, size: u32) -> std::io::Result<u64> {
    let offset = writer.stream_position()?;
    let name_len = u16::try_from(name.len()).map_err(|_| {
        std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "ZIP name exceeds 16-bit field",
        )
    })?;
    let mut header = [0u8; 30];
    put32(&mut header, 0, 0x04034b50);
    put16(
        &mut header,
        4,
        if offset >= u64::from(u32::MAX) {
            45
        } else {
            20
        },
    );
    put16(&mut header, 6, 0x0800); // UTF-8, no streaming descriptor.
    put16(&mut header, 12, 0x0021); // 1980-01-01, midnight.
    put32(&mut header, 18, size);
    put32(&mut header, 22, size);
    put16(&mut header, 26, name_len);
    writer.write_all(&header)?;
    writer.write_all(name.as_bytes())?;
    Ok(offset)
}
fn complete_entry(writer: &mut (impl Write + Seek), offset: u64, crc: u32) -> std::io::Result<()> {
    let end = writer.stream_position()?;
    writer.seek(SeekFrom::Start(offset + 14))?;
    writer.write_all(&crc.to_le_bytes())?;
    writer.seek(SeekFrom::Start(end))?;
    Ok(())
}
fn central_directory<E>(
    writer: &mut (impl Write + Seek),
    entries: &[Entry<'_>],
    mut checkpoint: impl FnMut() -> Result<(), E>,
) -> Result<(), WriteFailure<E>> {
    let start = writer.stream_position()?;
    for entry in entries {
        checkpoint().map_err(WriteFailure::Checkpoint)?;
        let zip64_offset = entry.offset >= u64::from(u32::MAX);
        let version = if zip64_offset { 45 } else { 20 };
        let mut header = [0u8; 46];
        put32(&mut header, 0, 0x02014b50);
        put16(&mut header, 4, (3 << 8) | version);
        put16(&mut header, 6, version);
        put16(&mut header, 8, 0x0800);
        put16(&mut header, 14, 0x0021);
        put32(&mut header, 16, entry.crc);
        put32(&mut header, 20, entry.size);
        put32(&mut header, 24, entry.size);
        put16(
            &mut header,
            28,
            u16::try_from(entry.name.len()).map_err(|_| {
                std::io::Error::new(
                    std::io::ErrorKind::InvalidInput,
                    "ZIP name exceeds 16-bit field",
                )
            })?,
        );
        put16(&mut header, 30, if zip64_offset { 12 } else { 0 });
        put32(&mut header, 38, 0o100644 << 16);
        put32(
            &mut header,
            42,
            u32::try_from(entry.offset).unwrap_or(u32::MAX),
        );
        writer.write_all(&header)?;
        writer.write_all(entry.name.as_bytes())?;
        if zip64_offset {
            let mut extra = [0u8; 12];
            put16(&mut extra, 0, 1);
            put16(&mut extra, 2, 8);
            put64(&mut extra, 4, entry.offset);
            writer.write_all(&extra)?;
        }
    }
    let end = writer.stream_position()?;
    let size = end - start;
    let count = entries.len() as u64;
    if count >= u64::from(u16::MAX) || size >= u64::from(u32::MAX) || start >= u64::from(u32::MAX) {
        let mut record = [0u8; 56];
        put32(&mut record, 0, 0x06064b50);
        put64(&mut record, 4, 44);
        put16(&mut record, 12, (3 << 8) | 45);
        put16(&mut record, 14, 45);
        put64(&mut record, 24, count);
        put64(&mut record, 32, count);
        put64(&mut record, 40, size);
        put64(&mut record, 48, start);
        writer.write_all(&record)?;
        let mut locator = [0u8; 20];
        put32(&mut locator, 0, 0x07064b50);
        put64(&mut locator, 8, end);
        put32(&mut locator, 16, 1);
        writer.write_all(&locator)?;
    }
    let mut end_record = [0u8; 22];
    put32(&mut end_record, 0, 0x06054b50);
    put16(&mut end_record, 8, u16::try_from(count).unwrap_or(u16::MAX));
    put16(
        &mut end_record,
        10,
        u16::try_from(count).unwrap_or(u16::MAX),
    );
    put32(&mut end_record, 12, u32::try_from(size).unwrap_or(u32::MAX));
    put32(
        &mut end_record,
        16,
        u32::try_from(start).unwrap_or(u32::MAX),
    );
    writer.write_all(&end_record)?;
    checkpoint().map_err(WriteFailure::Checkpoint)?;
    writer.flush()?;
    Ok(())
}

/// Serialize a prepared inventory. Payload memory is one 64KiB buffer; metadata/index
/// memory scales with selected member count. The caller owns cooperative checkpoints.
/// There is no finalization in Drop: failed encoding never retries writes or emits stderr.
pub(crate) fn serialize<E, W: Write + Seek>(
    members: &[Member<'_>],
    writer: &mut W,
    mut checkpoint: impl FnMut(u64) -> Result<(), E>,
) -> Result<(), WriteFailure<E>> {
    let index_size = (members.len() as u64)
        .checked_mul(24)
        .filter(|size| *size < u64::from(u32::MAX))
        .ok_or_else(|| {
            WriteFailure::Codec(CodecError::invalid("3TZ index size exceeds local header"))
        })? as u32;
    let mut ordered: Vec<_> = members.iter().collect();
    ordered.sort_by(|a, b| {
        (a.name != "tileset.json", a.name).cmp(&(b.name != "tileset.json", b.name))
    });
    let mut entries = Vec::with_capacity(members.len() + 1);
    let mut index = Vec::with_capacity(members.len());
    let mut buffer = [0u8; 64 * 1024];
    let mut total = 0u64;
    for member in ordered {
        checkpoint(total).map_err(WriteFailure::Checkpoint)?;
        let changed = || WriteFailure::Codec(CodecError::SourceChanged(member.source.into()));
        let size = u32::try_from(member.size)
            .ok()
            .filter(|size| *size < u32::MAX)
            .ok_or_else(|| {
                WriteFailure::Codec(CodecError::invalid("3TZ member size exceeds local header"))
            })?;
        let source_io = |source| {
            WriteFailure::Codec(CodecError::SourceIo {
                path: member.source.into(),
                source,
            })
        };
        let path_metadata = std::fs::symlink_metadata(member.source).map_err(source_io)?;
        if !path_metadata.is_file()
            || path_metadata.len() != member.size
            || path_metadata.modified().ok() != member.modified
        {
            return Err(changed());
        }
        let mut source = File::open(member.source).map_err(source_io)?;
        let metadata = source.metadata().map_err(source_io)?;
        if !metadata.is_file()
            || metadata.len() != member.size
            || metadata.modified().ok() != member.modified
        {
            return Err(changed());
        }
        let offset = local_header(writer, member.name, size)?;
        let mut copied = 0u64;
        let mut crc = crc32fast::Hasher::new();
        loop {
            checkpoint(total).map_err(WriteFailure::Checkpoint)?;
            let n = source.read(&mut buffer).map_err(source_io)?;
            if n == 0 {
                break;
            }
            copied = copied.checked_add(n as u64).ok_or_else(changed)?;
            if copied > member.size {
                return Err(changed());
            }
            writer.write_all(&buffer[..n])?;
            crc.update(&buffer[..n]);
            total = total.checked_add(n as u64).ok_or_else(changed)?;
        }
        let after = source.metadata().map_err(source_io)?;
        if copied != member.size
            || after.len() != member.size
            || after.modified().ok() != member.modified
        {
            return Err(changed());
        }
        let crc = crc.finalize();
        complete_entry(writer, offset, crc)?;
        entries.push(Entry {
            name: member.name,
            size,
            crc,
            offset,
        });
        index.push((md5::compute(member.name.as_bytes()).0, offset));
    }
    index.sort_by_key(|(hash, _)| {
        (
            u64::from_le_bytes(hash[..8].try_into().unwrap()),
            u64::from_le_bytes(hash[8..].try_into().unwrap()),
        )
    });
    let offset = local_header(writer, TZ_INDEX_NAME, index_size)?;
    let mut crc = crc32fast::Hasher::new();
    for (hash, offset) in index {
        checkpoint(total).map_err(WriteFailure::Checkpoint)?;
        let offset = offset.to_le_bytes();
        writer.write_all(&hash)?;
        writer.write_all(&offset)?;
        crc.update(&hash);
        crc.update(&offset);
    }
    let crc = crc.finalize();
    complete_entry(writer, offset, crc)?;
    entries.push(Entry {
        name: TZ_INDEX_NAME,
        size: index_size,
        crc,
        offset,
    });
    checkpoint(total).map_err(WriteFailure::Checkpoint)?;
    central_directory(writer, &entries, || checkpoint(total))?;
    checkpoint(total).map_err(WriteFailure::Checkpoint)?;
    Ok(())
}

// Legacy converters are not migrated by F0. This historical writer remains
// separately unproven, including ZIP64 member sizes and ZipWriter Drop diagnostics.
/// Write a stored ZIP with `tileset.json` first and the 3TZ index last.
pub(crate) fn write_archive(
    files: &[(String, PathBuf)],
    file: &mut File,
) -> Result<(), CodecError> {
    let position = Arc::new(AtomicU64::new(0));
    let mut zip = zip::ZipWriter::new(Tracked {
        writer: file,
        position: position.clone(),
    });
    let mut index = Vec::with_capacity(files.len());
    let mut ordered: Vec<_> = files.iter().collect();
    ordered.sort_by(|a, b| (a.0 != "tileset.json", &a.0).cmp(&(b.0 != "tileset.json", &b.0)));
    for (name, path) in ordered {
        let mut source = File::open(path)?;
        let size = source.metadata()?.len();
        let offset = position.load(Ordering::Relaxed);
        zip.start_file(
            name,
            zip::write::SimpleFileOptions::default()
                .compression_method(zip::CompressionMethod::Stored)
                .last_modified_time(zip::DateTime::default())
                .unix_permissions(0o644)
                .large_file(size >= u32::MAX as u64),
        )?;
        let copied = std::io::copy(&mut source, &mut zip)?;
        if copied != size {
            return Err(CodecError::invalid(format!(
                "source changed while packing: {}",
                path.display()
            )));
        }
        index.push((md5::compute(name.as_bytes()).0, offset));
    }
    index.sort_by_key(|(h, _)| {
        (
            u64::from_le_bytes(h[..8].try_into().unwrap()),
            u64::from_le_bytes(h[8..].try_into().unwrap()),
        )
    });
    zip.start_file(
        TZ_INDEX_NAME,
        zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Stored)
            .last_modified_time(zip::DateTime::default())
            .unix_permissions(0o644),
    )?;
    for (hash, offset) in index {
        zip.write_all(&hash)?;
        zip.write_all(&offset.to_le_bytes())?;
    }
    zip.finish()?.flush()?;
    Ok(())
}
/// List archive member names (for tests / golden compare).
pub fn list_zip_names(path: &Path) -> Result<Vec<String>, CodecError> {
    let file = File::open(path)?;
    let mut zip = zip::ZipArchive::new(file)?;
    let mut names: Vec<String> = (0..zip.len())
        .filter_map(|i| zip.by_index(i).ok().map(|z| z.name().to_string()))
        .collect();
    names.sort();
    Ok(names)
}

/// Check Maxar 3TZ v1.4 layout: root `tileset.json`, last member `@3dtilesIndex1@`
/// stored, index offsets point at local file headers whose names match the MD5.
pub fn validate_3tz(path: &Path) -> Result<(), CodecError> {
    validate_open_3tz(File::open(path)?)
}

/// Validate the selected open file, without resolving its pathname again.
pub(crate) fn validate_open_3tz(mut file: File) -> Result<(), CodecError> {
    let mut zip = zip::ZipArchive::new(file.try_clone()?)?;
    let n = zip.len();
    if n < 2 {
        return Err(CodecError::invalid(
            "3TZ must contain tileset.json and an index",
        ));
    }
    if zip.by_name("tileset.json").is_err() {
        return Err(CodecError::MissingManifest);
    }
    {
        let last = zip.by_index(n - 1)?;
        if last.name() != TZ_INDEX_NAME {
            return Err(CodecError::Invalid(format!(
                "last ZIP member must be {TZ_INDEX_NAME}, got {}",
                last.name()
            )));
        }
        if last.compression() != zip::CompressionMethod::Stored {
            return Err(CodecError::invalid(
                "3TZ index must be stored (uncompressed)",
            ));
        }
    }
    let mut expected = std::collections::HashMap::new();
    for i in 0..n - 1 {
        let member = zip.by_index(i)?;
        if member.compression() != zip::CompressionMethod::Stored {
            return Err(CodecError::invalid("3TZ members must be stored"));
        }
        expected.insert(
            member.header_start(),
            md5::compute(member.name().as_bytes()).0,
        );
    }
    let mut index = Vec::new();
    zip.by_name(TZ_INDEX_NAME)?.read_to_end(&mut index)?;
    drop(zip);

    if index.len() % 24 != 0 {
        return Err(CodecError::invalid(
            "3TZ index length is not a multiple of 24",
        ));
    }
    if index.len() / 24 != n - 1 {
        return Err(CodecError::invalid(
            "3TZ index must contain every member exactly once",
        ));
    }
    let mut previous = None;
    let f = &mut file;
    for rec in index.as_chunks::<24>().0 {
        let key = (
            u64::from_le_bytes(rec[..8].try_into().unwrap()),
            u64::from_le_bytes(rec[8..16].try_into().unwrap()),
        );
        if previous.is_some_and(|p| p > key) {
            return Err(CodecError::invalid("3TZ index hashes are not ordered"));
        }
        previous = Some(key);
        let off = u64::from_le_bytes(rec[16..24].try_into().unwrap());
        if expected.remove(&off).as_ref().map(|v| v.as_slice()) != Some(&rec[..16]) {
            return Err(CodecError::invalid("3TZ index differs from ZIP directory"));
        }
        f.seek(SeekFrom::Start(off))?;
        let mut hdr = [0u8; 30];
        f.read_exact(&mut hdr)?;
        let sig = u32::from_le_bytes(hdr[0..4].try_into().unwrap());
        if sig != 0x0403_4b50 {
            return Err(CodecError::Invalid(format!(
                "3TZ index offset {off} is not a ZIP local header"
            )));
        }
        let name_len = u16::from_le_bytes(hdr[26..28].try_into().unwrap()) as usize;
        let mut name = vec![0u8; name_len];
        f.read_exact(&mut name)?;
        let digest = md5::compute(&name).0;
        if digest != rec[..16] {
            return Err(CodecError::Invalid(format!(
                "3TZ index MD5 mismatch at offset {off}"
            )));
        }
    }
    Ok(())
}

#[cfg(test)]
pub(crate) mod test_faults {
    use std::io::{self, Seek, SeekFrom, Write};
    #[derive(Clone, Copy, Debug)]
    pub(crate) enum Fault {
        Payload,
        Finalize,
        Flush,
    }
    pub(crate) struct FaultWriter<W> {
        inner: W,
        fault: Fault,
        pub(crate) fired: bool,
    }
    impl<W> FaultWriter<W> {
        pub(crate) fn new(inner: W, fault: Fault) -> Self {
            Self {
                inner,
                fault,
                fired: false,
            }
        }
        fn error(&mut self) -> io::Error {
            self.fired = true;
            io::Error::other("injected codec storage fault")
        }
    }
    impl<W: Write> Write for FaultWriter<W> {
        fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
            let target = match self.fault {
                Fault::Payload => bytes.len() >= 64 && bytes.iter().all(|b| *b == 0xaa),
                Fault::Finalize => bytes.starts_with(b"PK\x01\x02"),
                Fault::Flush => false,
            };
            if self.fired || target {
                return Err(self.error());
            }
            self.inner.write(bytes)
        }
        fn flush(&mut self) -> io::Result<()> {
            if self.fired || matches!(self.fault, Fault::Flush) {
                return Err(self.error());
            }
            self.inner.flush()
        }
    }
    impl<W: Seek> Seek for FaultWriter<W> {
        fn seek(&mut self, position: SeekFrom) -> io::Result<u64> {
            self.inner.seek(position)
        }
    }
}

#[cfg(test)]
mod codec_fault_tests {
    use super::test_faults::{Fault, FaultWriter};
    use super::*;
    fn u64le(bytes: &[u8]) -> u64 {
        u64::from_le_bytes(bytes.try_into().unwrap())
    }

    #[test]
    fn zip64_count_sentinel_is_read_independently() {
        let work = tempfile::tempdir().unwrap();
        let source = work.path().join("empty");
        std::fs::write(&source, []).unwrap();
        let modified = std::fs::metadata(&source).unwrap().modified().ok();
        // Exactly 65,535 total entries needs ZIP64: 0xffff is a sentinel.
        let names: Vec<_> = (0..65_534)
            .map(|n| {
                if n == 0 {
                    "tileset.json".to_owned()
                } else {
                    format!("empty/{n}")
                }
            })
            .collect();
        let members: Vec<_> = names
            .iter()
            .map(|name| Member {
                name,
                source: &source,
                size: 0,
                modified,
            })
            .collect();
        let mut writer = std::io::Cursor::new(Vec::new());
        serialize(&members, &mut writer, |_| {
            Ok::<_, std::convert::Infallible>(())
        })
        .unwrap();
        let bytes = writer.into_inner();
        let eocd = bytes.len() - 22;
        assert_eq!(&bytes[eocd..eocd + 4], b"PK\x05\x06");
        assert_eq!(&bytes[eocd + 8..eocd + 12], &[0xff; 4]);
        let locator = eocd - 20;
        assert_eq!(&bytes[locator..locator + 4], b"PK\x06\x07");
        let record = u64le(&bytes[locator + 8..locator + 16]) as usize;
        assert_eq!(&bytes[record..record + 4], b"PK\x06\x06");
        assert_eq!(u64le(&bytes[record + 24..record + 32]), 65_535);
        assert_eq!(u64le(&bytes[record + 32..record + 40]), 65_535);
        let mut zip = zip::ZipArchive::new(std::io::Cursor::new(bytes)).unwrap();
        assert_eq!(zip.len(), 65_535);
        assert_eq!(zip.by_name("tileset.json").unwrap().size(), 0);
        let mut index = Vec::new();
        zip.by_name(TZ_INDEX_NAME)
            .unwrap()
            .read_to_end(&mut index)
            .unwrap();
        assert_eq!(index.len(), members.len() * 24);
    }

    #[cfg(unix)]
    #[test]
    fn zip64_offset_sentinel_uses_offset_extra_and_actual_local_sizes() {
        let work = tempfile::tempdir().unwrap();
        let source = work.path().join("tileset.json");
        std::fs::write(&source, b"opaque").unwrap();
        let metadata = std::fs::metadata(&source).unwrap();
        let members = [Member {
            name: "tileset.json",
            source: &source,
            size: metadata.len(),
            modified: metadata.modified().ok(),
        }];
        let path = work.path().join("sparse.3tz");
        let mut writer = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create_new(true)
            .open(&path)
            .unwrap();
        let offset = u64::from(u32::MAX);
        // Seek creates a sparse preamble rather than allocating/reading 4 GiB.
        writer.seek(SeekFrom::Start(offset)).unwrap();
        serialize(&members, &mut writer, |_| {
            Ok::<_, std::convert::Infallible>(())
        })
        .unwrap();
        let mut local = [0u8; 30];
        writer.seek(SeekFrom::Start(offset)).unwrap();
        writer.read_exact(&mut local).unwrap();
        assert_eq!(&local[..4], b"PK\x03\x04");
        assert_eq!(
            u32::from_le_bytes(local[14..18].try_into().unwrap()),
            crc32fast::hash(b"opaque")
        );
        assert_eq!(u32::from_le_bytes(local[18..22].try_into().unwrap()), 6);
        assert_eq!(u32::from_le_bytes(local[22..26].try_into().unwrap()), 6);
        assert_eq!(&local[28..30], &[0, 0]);
        writer.seek(SeekFrom::End(-42)).unwrap();
        let mut tail = [0u8; 42];
        writer.read_exact(&mut tail).unwrap();
        assert_eq!(&tail[..4], b"PK\x06\x07");
        let record = u64le(&tail[8..16]);
        writer.seek(SeekFrom::Start(record)).unwrap();
        let mut zip64 = [0u8; 56];
        writer.read_exact(&mut zip64).unwrap();
        assert_eq!(&zip64[..4], b"PK\x06\x06");
        let central = u64le(&zip64[48..56]);
        writer.seek(SeekFrom::Start(central)).unwrap();
        let mut header = [0u8; 46];
        writer.read_exact(&mut header).unwrap();
        assert_eq!(&header[..4], b"PK\x01\x02");
        assert_eq!(&header[42..46], &[0xff; 4]);
        assert_eq!(u16::from_le_bytes(header[30..32].try_into().unwrap()), 12);
        let name_len = u16::from_le_bytes(header[28..30].try_into().unwrap());
        writer.seek(SeekFrom::Current(i64::from(name_len))).unwrap();
        let mut extra = [0u8; 12];
        writer.read_exact(&mut extra).unwrap();
        assert_eq!(&extra[..4], &[1, 0, 8, 0]);
        assert_eq!(u64le(&extra[4..]), offset);
        let mut zip = zip::ZipArchive::new(writer).unwrap();
        assert_eq!(zip.len(), 2);
        let mut content = Vec::new();
        let mut entry = zip.by_name("tileset.json").unwrap();
        assert_eq!(entry.header_start(), offset);
        entry.read_to_end(&mut content).unwrap();
        assert_eq!(content, b"opaque");
        drop(entry);
        let mut index = Vec::new();
        zip.by_name(TZ_INDEX_NAME)
            .unwrap()
            .read_to_end(&mut index)
            .unwrap();
        assert_eq!(index.len(), 24);
        assert_eq!(u64le(&index[16..]), offset);
    }

    #[test]
    fn payload_central_directory_and_flush_faults_are_returned() {
        let work = tempfile::tempdir().unwrap();
        let source = work.path().join("tileset.json");
        std::fs::write(&source, vec![0xaa; 8192]).unwrap();
        let metadata = std::fs::metadata(&source).unwrap();
        let members = [Member {
            name: "tileset.json",
            source: &source,
            size: metadata.len(),
            modified: metadata.modified().ok(),
        }];
        for fault in [Fault::Payload, Fault::Finalize, Fault::Flush] {
            let mut writer = FaultWriter::new(std::io::Cursor::new(Vec::new()), fault);
            let result = serialize(&members, &mut writer, |_| {
                Ok::<_, std::convert::Infallible>(())
            });
            assert!(
                writer.fired,
                "{fault:?} injection did not reach its intended boundary"
            );
            assert!(
                matches!(
                    result,
                    Err(WriteFailure::Codec(CodecError::Io(_)))
                        | Err(WriteFailure::Codec(CodecError::Zip(
                            zip::result::ZipError::Io(_)
                        )))
                ),
                "{fault:?}: {result:?}"
            );
        }
    }
}
