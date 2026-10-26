//! The streamed 3TZ writer: stored members copied one at a time in a fixed order,
//! `tileset.json` first and the index last, with the per-member index rows external-sorted
//! in scratch and the central directory spooled to a scratch file. Resident memory is one copy
//! buffer, the sorter's configured run and the spool buffer, whatever the member count. The
//! ZIP records are the shared `archive3tz` codec's.
use crate::{
    archive3tz::{
        central_header, complete_entry, end_records, index_key, local_header, TZ_INDEX_NAME,
    },
    spill::{ExternalSorter, Record, Scratch, SortConfig},
    JobError, JobErrorKind,
};
use std::{
    fs::File,
    io::{BufRead, BufReader, BufWriter, Read, Seek, Write},
    path::Path,
};

/// One 3TZ index row: the member name's MD5 and its local header offset.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct IndexRow {
    hash: [u8; 16],
    offset: u64,
}

impl Ord for IndexRow {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        (index_key(&self.hash), self.offset).cmp(&(index_key(&other.hash), other.offset))
    }
}
impl PartialOrd for IndexRow {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}
impl Record for IndexRow {
    const SIZE: usize = 24;
    fn encode(&self, out: &mut [u8]) {
        out[..16].copy_from_slice(&self.hash);
        out[16..].copy_from_slice(&self.offset.to_le_bytes());
    }
    fn decode(bytes: &[u8]) -> Self {
        Self {
            hash: bytes[..16].try_into().unwrap(),
            offset: u64::from_le_bytes(bytes[16..].try_into().unwrap()),
        }
    }
}

/// The sorter configuration: a 16 MiB run, 64-way merges, 256 KiB stream buffers.
const SORT: SortConfig = SortConfig {
    run_bytes: 16 << 20,
    fan_in: 64,
    buffer_bytes: 256 << 10,
    jobs: 1,
};

/// Resident bytes of the archive writer: the index sorter, the central-directory spool,
/// the output writer and the copy buffer.
pub(crate) fn resident_bound() -> u64 {
    SORT.memory_bound::<IndexRow>() + (1 << 20) + (1 << 20) + (1 << 16)
}

fn io<'p>(operation: &'static str, path: &'p Path) -> impl Fn(std::io::Error) -> JobError + 'p {
    move |error| JobError::io(operation, path, error)
}

/// Scratch bytes the archive needs besides its output: the sort runs and the spooled
/// central directory (at most 46 + 256 + 12 bytes per member).
pub(crate) fn scratch_need(members: u64) -> Option<u64> {
    let sort = SortConfig::scratch_need::<IndexRow>(members).ok()?;
    sort.checked_add(members.checked_add(1)?.checked_mul(46 + 256 + 12)?)
}

/// Write the archive of `count` members named by `names` (one per line, `tileset.json` not
/// among them) from `staging` into `out`: `tileset.json`, then `names` in order, then the
/// index. Returns the archive length. With `limit` (the memory charged to an archive staged
/// on memory-backed storage, D8) the archive is refused before a member or the central
/// directory would take it past `limit`.
pub(crate) fn write(
    staging: &Path,
    names: &Path,
    count: u64,
    scratch: &Scratch,
    out: &mut File,
    check: &mut dyn FnMut() -> Result<(), JobError>,
    limit: Option<u64>,
) -> Result<u64, JobError> {
    write_with(staging, names, count, scratch, out, check, SORT, limit)
}

/// The refusal of an archive past the memory charged to its memory-backed staging.
fn over_charge(needed: u64, limit: u64) -> JobError {
    JobError::new(
        JobErrorKind::ResourceLimit,
        format!(
            "the archive staged on memory-backed storage needs at least {needed} bytes, past \
             the {limit} charged to the memory budget; stage the output on disk"
        ),
    )
}

#[allow(clippy::too_many_arguments)]
fn write_with(
    staging: &Path,
    names: &Path,
    count: u64,
    scratch: &Scratch,
    out: &mut File,
    check: &mut dyn FnMut() -> Result<(), JobError>,
    sort: SortConfig,
    limit: Option<u64>,
) -> Result<u64, JobError> {
    let admit = |needed: u64| match limit {
        Some(limit) if needed > limit => Err(over_charge(needed, limit)),
        _ => Ok(()),
    };
    let index_bytes = count
        .checked_mul(24)
        .filter(|bytes| *bytes < u64::from(u32::MAX))
        .ok_or_else(|| JobError::new(JobErrorKind::Unsupported, "3TZ index exceeds 4 GiB"))?;
    let spool_path = scratch.path().join("central-directory");
    let mut spool = BufWriter::with_capacity(
        1 << 20,
        File::create(&spool_path).map_err(io("create central-directory spool", &spool_path))?,
    );
    let mut sorter = ExternalSorter::<IndexRow>::new(scratch, sort)?;
    let mut writer = BufWriter::with_capacity(1 << 20, &mut *out);
    let mut buffer = vec![0u8; 1 << 16];
    let mut written = 0u64;
    let list = File::open(names).map_err(io("open member list", names))?;
    let mut lines = BufReader::new(list).lines();
    let mut next = Some("tileset.json".to_owned());
    while let Some(name) = next.take() {
        check()?;
        let path = staging.join(&name);
        let mut source = File::open(&path).map_err(io("open staged member", &path))?;
        let length = source
            .metadata()
            .map_err(io("inspect staged member", &path))?
            .len();
        let size = u32::try_from(length)
            .ok()
            .filter(|size| *size < u32::MAX)
            .ok_or_else(|| {
                JobError::new(
                    JobErrorKind::Unsupported,
                    format!("3TZ member {name} exceeds 4 GiB"),
                )
            })?;
        let offset =
            local_header(&mut writer, &name, size).map_err(io("write archive member", &path))?;
        admit(offset.saturating_add(length))?;
        let mut crc = crc32fast::Hasher::new();
        let mut copied = 0u64;
        loop {
            let n = source
                .read(&mut buffer)
                .map_err(io("read staged member", &path))?;
            if n == 0 {
                break;
            }
            copied += n as u64;
            crc.update(&buffer[..n]);
            writer
                .write_all(&buffer[..n])
                .map_err(io("write archive member", &path))?;
        }
        if copied != length {
            return Err(JobError::new(
                JobErrorKind::InvalidState,
                format!("staged member {name} changed while it was archived"),
            ));
        }
        let crc = crc.finalize();
        complete_entry(&mut writer, offset, crc).map_err(io("write archive member", &path))?;
        spool
            .write_all(
                &central_header(&name, size, crc, offset)
                    .map_err(io("spool central directory", &spool_path))?,
            )
            .map_err(io("spool central directory", &spool_path))?;
        sorter.push(IndexRow {
            hash: md5::compute(name.as_bytes()).0,
            offset,
        })?;
        written += 1;
        next = lines
            .next()
            .transpose()
            .map_err(io("read member list", names))?;
    }
    if written != count {
        return Err(JobError::new(
            JobErrorKind::InvalidState,
            format!("the member list holds {written} members; the plan counted {count}"),
        ));
    }
    let index_path = Path::new(TZ_INDEX_NAME);
    let offset = local_header(&mut writer, TZ_INDEX_NAME, index_bytes as u32)
        .map_err(io("write archive index", index_path))?;
    admit(offset.saturating_add(index_bytes))?;
    let mut crc = crc32fast::Hasher::new();
    let mut rows = 0u64;
    for row in sorter.finish()? {
        let row = row?;
        rows += 1;
        if rows.is_multiple_of(65_536) {
            check()?;
        }
        let mut bytes = [0u8; 24];
        row.encode(&mut bytes);
        crc.update(&bytes);
        writer
            .write_all(&bytes)
            .map_err(io("write archive index", index_path))?;
    }
    if rows != count {
        return Err(JobError::new(
            JobErrorKind::InvalidState,
            "the index row count differs from the member count",
        ));
    }
    let crc = crc.finalize();
    complete_entry(&mut writer, offset, crc).map_err(io("write archive index", index_path))?;
    spool
        .write_all(
            &central_header(TZ_INDEX_NAME, index_bytes as u32, crc, offset)
                .map_err(io("spool central directory", &spool_path))?,
        )
        .map_err(io("spool central directory", &spool_path))?;
    spool
        .flush()
        .map_err(io("spool central directory", &spool_path))?;
    drop(spool);
    check()?;
    let start = writer
        .stream_position()
        .map_err(io("write central directory", &spool_path))?;
    let mut directory =
        File::open(&spool_path).map_err(io("read central-directory spool", &spool_path))?;
    admit(
        start.saturating_add(
            directory
                .metadata()
                .map_err(io("read central-directory spool", &spool_path))?
                .len(),
        ),
    )?;
    std::io::copy(&mut directory, &mut writer)
        .map_err(io("write central directory", &spool_path))?;
    let end = writer
        .stream_position()
        .map_err(io("write central directory", &spool_path))?;
    end_records(&mut writer, start, end, count + 1)
        .map_err(io("write central directory", &spool_path))?;
    writer
        .flush()
        .map_err(io("write central directory", &spool_path))?;
    let length = writer
        .stream_position()
        .map_err(io("write central directory", &spool_path))?;
    admit(length)?;
    Ok(length)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::archive3tz::{serialize, Member};

    /// Over 65,535 members (ZIP64 end records) and an index sorted through many spilled runs:
    /// the streamed writer gives exactly the bytes of the audited `archive3tz::serialize` for
    /// the same member order, and the 3TZ reader admits them.
    #[test]
    fn streamed_archive_equals_the_audited_writer_beyond_zip64_counts() {
        let work = tempfile::tempdir().unwrap();
        let staging = work.path().join("staging");
        std::fs::create_dir_all(staging.join("m")).unwrap();
        std::fs::write(
            staging.join("tileset.json"),
            b"{\"asset\":{\"version\":\"1.1\"}}",
        )
        .unwrap();
        let count = 70_000usize;
        let mut names = String::new();
        let mut members = vec!["tileset.json".to_owned()];
        for i in 0..count {
            let name = format!("m/{i:06}.bin");
            std::fs::write(
                staging.join(&name),
                (i as u64).to_le_bytes().repeat(1 + i % 3),
            )
            .unwrap();
            names.push_str(&name);
            names.push('\n');
            members.push(name);
        }
        let list = work.path().join("members.txt");
        std::fs::write(&list, names).unwrap();
        let scratch = Scratch::create(Some(work.path()), 1 << 30).unwrap();
        let streamed = work.path().join("streamed.3tz");
        let mut file = File::create(&streamed).unwrap();
        let tiny = SortConfig {
            run_bytes: 24 * 1000,
            fan_in: 4,
            buffer_bytes: 4096,
            jobs: 1,
        };
        let length = write_with(
            &staging,
            &list,
            count as u64 + 1,
            &scratch,
            &mut file,
            &mut || Ok(()),
            tiny,
            None,
        )
        .unwrap();
        drop(file);
        let streamed = std::fs::read(&streamed).unwrap();
        assert_eq!(streamed.len() as u64, length);
        let sources: Vec<_> = members.iter().map(|m| staging.join(m)).collect();
        let inventory: Vec<Member<'_>> = members
            .iter()
            .zip(&sources)
            .map(|(name, source)| {
                let metadata = std::fs::metadata(source).unwrap();
                Member {
                    name,
                    source,
                    size: metadata.len(),
                    modified: metadata.modified().ok(),
                }
            })
            .collect();
        let mut audited = std::io::Cursor::new(Vec::new());
        serialize::<(), _>(&inventory, &mut audited, |_| Ok(())).unwrap();
        assert!(
            streamed == audited.into_inner(),
            "streamed and audited archives differ"
        );
        crate::archive3tz::validate_reader_3tz(std::io::Cursor::new(streamed)).unwrap();
    }

    /// D8: an archive staged on memory-backed storage is admitted up to its exact length and
    /// refused one byte below it, at every stage of the stream (members, index, directory).
    #[test]
    fn memory_backed_archive_limit_admits_its_length_and_refuses_one_below() {
        let work = tempfile::tempdir().unwrap();
        let staging = work.path().join("staging");
        std::fs::create_dir_all(&staging).unwrap();
        std::fs::write(
            staging.join("tileset.json"),
            b"{\"asset\":{\"version\":\"1.1\"}}",
        )
        .unwrap();
        std::fs::write(staging.join("a.glb"), vec![7u8; 5000]).unwrap();
        std::fs::write(staging.join("b.glb"), vec![9u8; 3000]).unwrap();
        let list = work.path().join("members.txt");
        std::fs::write(&list, "a.glb\nb.glb\n").unwrap();
        let scratch = Scratch::create(Some(work.path()), 1 << 20).unwrap();
        let run = |name: &str, limit| {
            let mut file = File::create(work.path().join(name)).unwrap();
            write(
                &staging,
                &list,
                3,
                &scratch,
                &mut file,
                &mut || Ok(()),
                limit,
            )
        };
        let length = run("free.3tz", None).unwrap();
        assert_eq!(run("exact.3tz", Some(length)).unwrap(), length);
        assert_eq!(
            std::fs::read(work.path().join("free.3tz")).unwrap(),
            std::fs::read(work.path().join("exact.3tz")).unwrap()
        );
        for limit in [length - 1, 4000, 100] {
            let error = run("over.3tz", Some(limit)).unwrap_err();
            assert_eq!(error.kind(), JobErrorKind::ResourceLimit, "{error}");
            assert!(error.to_string().contains("memory-backed"), "{error}");
            // Refused before the stream ran far past the charge.
            let written = std::fs::metadata(work.path().join("over.3tz"))
                .unwrap()
                .len();
            assert!(written <= limit.max(5000) + (1 << 16), "{limit}: {written}");
        }
    }
}
