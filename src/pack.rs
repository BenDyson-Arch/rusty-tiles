//! One-pass stored ZIP/ZIP64 with a 3TZ index and atomic publication.
use crate::{error::Error, output::Job, report::ConversionResult};
use std::{
    collections::HashSet,
    fs::File,
    io::{Read, Seek, SeekFrom, Write},
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc,
    },
};
pub const TZ_INDEX_NAME: &str = "@3dtilesIndex1@";
#[derive(Clone, Debug, Default)]
pub struct PackOptions {
    pub force: bool,
}

pub fn convert_to_3tz(input: &Path, output: &Path, opts: &PackOptions) -> Result<(), Error> {
    convert_to_3tz_reported(input, output, opts).map(drop)
}

/// [`convert_to_3tz`] returning the published result.
pub fn convert_to_3tz_reported(
    input: &Path,
    output: &Path,
    opts: &PackOptions,
) -> Result<ConversionResult, Error> {
    let root = tileset_root(input)?;
    if !root.join("tileset.json").is_file() {
        return Err(Error::MissingTilesetJson);
    }
    Job::begin(output, opts.force)?.publish_tree_3tz(&root, None)
}

/// Every file below `root` as `(archive name, path)`, `tileset.json` first,
/// skipping the archive being written and any stale 3TZ index.
pub(crate) fn tree_members(root: &Path, output: &Path) -> Result<Vec<(String, PathBuf)>, Error> {
    let absolute_output = std::path::absolute(output)?;
    let mut files = Vec::new();
    for entry in walkdir::WalkDir::new(root) {
        let entry = entry.map_err(|e| Error::msg(e.to_string()))?;
        if !entry.file_type().is_file() {
            continue;
        }
        if std::path::absolute(entry.path())? == absolute_output {
            continue;
        }
        let name = entry
            .path()
            .strip_prefix(root)
            .unwrap()
            .to_string_lossy()
            .replace('\\', "/");
        if name != TZ_INDEX_NAME {
            files.push((name, entry.into_path()));
        }
    }
    files.sort_by(|a, b| (a.0 != "tileset.json", &a.0).cmp(&(b.0 != "tileset.json", &b.0)));
    Ok(files)
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
pub fn pack_named_files(
    files: &[(String, PathBuf)],
    output: &Path,
    opts: &PackOptions,
) -> Result<(), Error> {
    Job::begin(output, opts.force)?
        .publish_3tz(files, None)
        .map(drop)
}

/// Reject archives without a root manifest and unsafe, duplicate or missing
/// member paths before any bytes are written.
pub(crate) fn check_members(files: &[(String, PathBuf)], output: &Path) -> Result<(), Error> {
    if !files.iter().any(|(n, _)| n == "tileset.json") {
        return Err(Error::MissingTilesetJson);
    }
    let absolute_output = std::path::absolute(output)?;
    let mut seen = HashSet::new();
    for (name, path) in files {
        if name.is_empty()
            || name == TZ_INDEX_NAME
            || name.contains('\\')
            || name.starts_with('/')
            || name
                .split('/')
                .any(|s| s == ".." || s == "." || s.is_empty())
            || !seen.insert(name)
        {
            return Err(Error::msg(format!(
                "invalid or duplicate archive path: {name}"
            )));
        }
        if !path.is_file() {
            return Err(Error::InputNotFound(path.clone()));
        }
        if std::path::absolute(path)? == absolute_output {
            return Err(Error::msg("archive output cannot be an input member"));
        }
    }
    Ok(())
}

/// Write a stored ZIP with `tileset.json` first and the 3TZ index last.
pub(crate) fn write_archive(files: &[(String, PathBuf)], file: &mut File) -> Result<(), Error> {
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
            return Err(Error::msg(format!(
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
fn tileset_root(input: &Path) -> Result<PathBuf, Error> {
    if !input.exists() {
        return Err(Error::InputNotFound(input.to_path_buf()));
    }
    if input.is_dir() {
        return Ok(input.to_path_buf());
    }
    if input
        .file_name()
        .and_then(|n| n.to_str())
        .is_some_and(|n| n.eq_ignore_ascii_case("tileset.json"))
    {
        return Ok(input.parent().unwrap_or(input).to_path_buf());
    }
    Err(Error::msg(format!(
        "convert expects a directory or tileset.json, got {}",
        input.display()
    )))
}

/// List archive member names (for tests / golden compare).
pub fn list_zip_names(path: &Path) -> Result<Vec<String>, Error> {
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
pub fn validate_3tz(path: &Path) -> Result<(), Error> {
    let mut zip = zip::ZipArchive::new(File::open(path)?)?;
    let n = zip.len();
    if n < 2 {
        return Err(Error::msg("3TZ must contain tileset.json and an index"));
    }
    if zip.by_name("tileset.json").is_err() {
        return Err(Error::MissingTilesetJson);
    }
    {
        let last = zip.by_index(n - 1)?;
        if last.name() != TZ_INDEX_NAME {
            return Err(Error::msg(format!(
                "last ZIP member must be {TZ_INDEX_NAME}, got {}",
                last.name()
            )));
        }
        if last.compression() != zip::CompressionMethod::Stored {
            return Err(Error::msg("3TZ index must be stored (uncompressed)"));
        }
    }
    let mut expected = std::collections::HashMap::new();
    for i in 0..n - 1 {
        let member = zip.by_index(i)?;
        if member.compression() != zip::CompressionMethod::Stored {
            return Err(Error::msg("3TZ members must be stored"));
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
        return Err(Error::msg("3TZ index length is not a multiple of 24"));
    }
    if index.len() / 24 != n - 1 {
        return Err(Error::msg(
            "3TZ index must contain every member exactly once",
        ));
    }
    let mut previous = None;
    let mut f = File::open(path)?;
    for rec in index.as_chunks::<24>().0 {
        let key = (
            u64::from_le_bytes(rec[..8].try_into().unwrap()),
            u64::from_le_bytes(rec[8..16].try_into().unwrap()),
        );
        if previous.is_some_and(|p| p >= key) {
            return Err(Error::msg("3TZ index hashes are not strictly ordered"));
        }
        previous = Some(key);
        let off = u64::from_le_bytes(rec[16..24].try_into().unwrap());
        if expected.remove(&off).as_ref().map(|v| v.as_slice()) != Some(&rec[..16]) {
            return Err(Error::msg("3TZ index differs from ZIP directory"));
        }
        f.seek(SeekFrom::Start(off))?;
        let mut hdr = [0u8; 30];
        f.read_exact(&mut hdr)?;
        let sig = u32::from_le_bytes(hdr[0..4].try_into().unwrap());
        if sig != 0x0403_4b50 {
            return Err(Error::msg(format!(
                "3TZ index offset {off} is not a ZIP local header"
            )));
        }
        let name_len = u16::from_le_bytes(hdr[26..28].try_into().unwrap()) as usize;
        let mut name = vec![0u8; name_len];
        f.read_exact(&mut name)?;
        let digest = md5::compute(&name).0;
        if digest != rec[..16] {
            return Err(Error::msg(format!(
                "3TZ index MD5 mismatch at offset {off}"
            )));
        }
    }
    Ok(())
}

#[cfg(test)]
mod reproducibility_tests {
    use super::*;
    use std::fs;
    #[test]
    fn packing_is_independent_of_caller_entry_order() {
        let work = tempfile::tempdir().unwrap();
        let manifest = work.path().join("tileset.json");
        let content = work.path().join("tile.glb");
        fs::write(&manifest, b"{}").unwrap();
        fs::write(&content, b"same payload").unwrap();
        let mut files = vec![
            ("tile.glb".into(), content),
            ("tileset.json".into(), manifest),
        ];
        let a = work.path().join("a.3tz");
        let b = work.path().join("b.3tz");
        pack_named_files(&files, &a, &PackOptions::default()).unwrap();
        files.reverse();
        pack_named_files(&files, &b, &PackOptions::default()).unwrap();
        assert_eq!(fs::read(&a).unwrap(), fs::read(&b).unwrap());
        validate_3tz(&a).unwrap();
    }
}
