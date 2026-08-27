//! Pack a tileset directory (or `tileset.json` + siblings) into `.3tz`.
//!
//! Layout matches Cesium `3d-tiles-tools` `TilesetTarget3tz`:
//! uncompressed ZIP (`store`), then `"@3dtilesIndex1@"` as the last member.
//! Offsets in the index assume a 30-byte local header and no extra field
//! (`IndexBuilder` in 3d-tiles-tools@0.5.4).
//!
//! A standard ZIP is enough for TinyOwl [`tileset.ExtractZip`]; the index is
//! for Cesium 3TZ random access.

use std::fs::{self, File};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

use walkdir::WalkDir;

use crate::error::Error;

/// 3TZ index member name (Cesium / Maxar 3TZ spec).
pub const TZ_INDEX_NAME: &str = "@3dtilesIndex1@";

const ZIP_LOCAL_FILE_HEADER_SIZE: u64 = 30;
const MAX_STORED_FILE: u64 = u32::MAX as u64;

#[derive(Clone, Debug, Default)]
pub struct PackOptions {
    pub force: bool,
}

pub fn convert_to_3tz(input: &Path, output: &Path, opts: &PackOptions) -> Result<(), Error> {
    let root = tileset_root(input)?;
    if !root.join("tileset.json").is_file() {
        return Err(Error::MissingTilesetJson);
    }
    let mut files = collect_files(&root)?;
    files.sort_by(|a, b| match (a.name.as_str(), b.name.as_str()) {
        ("tileset.json", _) => std::cmp::Ordering::Less,
        (_, "tileset.json") => std::cmp::Ordering::Greater,
        _ => a.name.cmp(&b.name),
    });
    pack_zip_files(&files, output, opts)
}

/// Pack named files into a 3TZ (first should be `tileset.json`). Used by `glb-to-3tz`
/// so the source GLB is read in place instead of copied.
pub fn pack_named_files(
    files: &[(String, PathBuf)],
    output: &Path,
    opts: &PackOptions,
) -> Result<(), Error> {
    if !files.iter().any(|(n, _)| n == "tileset.json") {
        return Err(Error::MissingTilesetJson);
    }
    let mut zipped = Vec::with_capacity(files.len());
    for (name, path) in files {
        zipped.push(zip_file_from_path(name.clone(), path)?);
    }
    pack_zip_files(&zipped, output, opts)
}

fn pack_zip_files(files: &[ZipFile], output: &Path, opts: &PackOptions) -> Result<(), Error> {
    if output.exists() && !opts.force {
        return Err(Error::OutputExists(output.to_path_buf()));
    }
    if let Some(parent) = output.parent() {
        if !parent.as_os_str().is_empty() {
            fs::create_dir_all(parent)?;
        }
    }
    let index = build_tz_index(files)?;
    write_stored_zip(output, files, &index)
}

struct ZipFile {
    name: String,
    path: PathBuf,
    size: u64,
    crc: u32,
}

fn collect_files(root: &Path) -> Result<Vec<ZipFile>, Error> {
    let mut out = Vec::new();
    for entry in WalkDir::new(root).into_iter().filter_map(|e| e.ok()) {
        if !entry.file_type().is_file() {
            continue;
        }
        let abs = entry.path();
        let rel = abs
            .strip_prefix(root)
            .unwrap_or(abs)
            .to_string_lossy()
            .replace('\\', "/");
        if rel.is_empty() || rel == TZ_INDEX_NAME {
            continue;
        }
        out.push(zip_file_from_path(rel, abs)?);
    }
    if out.is_empty() {
        return Err(Error::MissingTilesetJson);
    }
    Ok(out)
}

fn zip_file_from_path(name: String, path: &Path) -> Result<ZipFile, Error> {
    if !path.is_file() {
        return Err(Error::InputNotFound(path.to_path_buf()));
    }
    let size = fs::metadata(path)?.len();
    if size > MAX_STORED_FILE {
        return Err(Error::msg(format!(
            "{} is {size} bytes; 3TZ local headers cap each file at 4 GiB",
            path.display()
        )));
    }
    Ok(ZipFile {
        crc: crc32_file(path)?,
        name,
        path: path.to_path_buf(),
        size,
    })
}

fn crc32_file(path: &Path) -> Result<u32, Error> {
    let mut hasher = crc32fast::Hasher::new();
    let mut f = File::open(path)?;
    let mut buf = [0u8; 1024 * 64];
    loop {
        let n = f.read(&mut buf)?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Ok(hasher.finalize())
}

/// MD5(path) ‖ u64le local-header offset, sorted by MD5 (3TZ spec / IndexBuilder).
fn build_tz_index(files: &[ZipFile]) -> Result<Vec<u8>, Error> {
    let mut offset = 0u64;
    let mut entries: Vec<([u8; 16], u64)> = Vec::with_capacity(files.len());
    for f in files {
        if offset > MAX_STORED_FILE {
            return Err(Error::msg(
                "3TZ archive exceeds 4 GiB ZIP offset (ZIP64 not written)",
            ));
        }
        entries.push((md5::compute(f.name.as_bytes()).0, offset));
        offset += ZIP_LOCAL_FILE_HEADER_SIZE + f.name.len() as u64 + f.size;
    }
    entries.sort_by(|a, b| {
        let lo = |h: &[u8; 16]| u64::from_le_bytes(h[0..8].try_into().unwrap());
        let hi = |h: &[u8; 16]| u64::from_le_bytes(h[8..16].try_into().unwrap());
        lo(&a.0)
            .cmp(&lo(&b.0))
            .then_with(|| hi(&a.0).cmp(&hi(&b.0)))
    });
    let mut buf = Vec::with_capacity(entries.len() * 24);
    for (hash, off) in entries {
        buf.extend_from_slice(&hash);
        buf.extend_from_slice(&off.to_le_bytes());
    }
    Ok(buf)
}

fn write_stored_zip(output: &Path, files: &[ZipFile], index: &[u8]) -> Result<(), Error> {
    let mut w = File::create(output)?;
    let mut central = Vec::new();
    let mut local_off = 0u32;
    let n = files.len() + 1;

    for f in files {
        write_local_and_data(
            &mut w,
            &mut central,
            local_off,
            &f.name,
            f.crc,
            f.size as u32,
            FilePayload::Path(&f.path),
        )?;
        local_off += 30 + f.name.len() as u32 + f.size as u32;
    }
    let index_crc = crc32fast::hash(index);
    write_local_and_data(
        &mut w,
        &mut central,
        local_off,
        TZ_INDEX_NAME,
        index_crc,
        index.len() as u32,
        FilePayload::Bytes(index),
    )?;

    let cd_off = local_off + 30 + TZ_INDEX_NAME.len() as u32 + index.len() as u32;
    w.write_all(&central)?;
    // EOCD
    w.write_all(&0x0605_4b50u32.to_le_bytes())?;
    w.write_all(&0u16.to_le_bytes())?;
    w.write_all(&0u16.to_le_bytes())?;
    w.write_all(&(n as u16).to_le_bytes())?;
    w.write_all(&(n as u16).to_le_bytes())?;
    w.write_all(&(central.len() as u32).to_le_bytes())?;
    w.write_all(&cd_off.to_le_bytes())?;
    w.write_all(&0u16.to_le_bytes())?;
    w.flush()?;
    Ok(())
}

enum FilePayload<'a> {
    Path(&'a Path),
    Bytes(&'a [u8]),
}

fn write_local_and_data(
    w: &mut File,
    central: &mut Vec<u8>,
    local_off: u32,
    name: &str,
    crc: u32,
    size: u32,
    payload: FilePayload<'_>,
) -> Result<(), Error> {
    let name_b = name.as_bytes();
    // Local file header
    w.write_all(&0x0403_4b50u32.to_le_bytes())?;
    w.write_all(&20u16.to_le_bytes())?;
    w.write_all(&0u16.to_le_bytes())?;
    w.write_all(&0u16.to_le_bytes())?; // stored
    w.write_all(&0u16.to_le_bytes())?;
    w.write_all(&0u16.to_le_bytes())?;
    w.write_all(&crc.to_le_bytes())?;
    w.write_all(&size.to_le_bytes())?;
    w.write_all(&size.to_le_bytes())?;
    w.write_all(&(name_b.len() as u16).to_le_bytes())?;
    w.write_all(&0u16.to_le_bytes())?;
    w.write_all(name_b)?;
    match payload {
        FilePayload::Path(p) => {
            let mut f = File::open(p)?;
            std::io::copy(&mut f, w)?;
        }
        FilePayload::Bytes(b) => w.write_all(b)?,
    }

    // Central directory header
    central.extend_from_slice(&0x0201_4b50u32.to_le_bytes());
    central.extend_from_slice(&20u16.to_le_bytes());
    central.extend_from_slice(&20u16.to_le_bytes());
    central.extend_from_slice(&0u16.to_le_bytes());
    central.extend_from_slice(&0u16.to_le_bytes());
    central.extend_from_slice(&0u16.to_le_bytes());
    central.extend_from_slice(&0u16.to_le_bytes());
    central.extend_from_slice(&crc.to_le_bytes());
    central.extend_from_slice(&size.to_le_bytes());
    central.extend_from_slice(&size.to_le_bytes());
    central.extend_from_slice(&(name_b.len() as u16).to_le_bytes());
    central.extend_from_slice(&0u16.to_le_bytes());
    central.extend_from_slice(&0u16.to_le_bytes());
    central.extend_from_slice(&0u16.to_le_bytes());
    central.extend_from_slice(&0u16.to_le_bytes());
    central.extend_from_slice(&0u32.to_le_bytes());
    central.extend_from_slice(&local_off.to_le_bytes());
    central.extend_from_slice(name_b);
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
    let mut index = Vec::new();
    zip.by_name(TZ_INDEX_NAME)?.read_to_end(&mut index)?;
    drop(zip);

    if index.len() % 24 != 0 {
        return Err(Error::msg("3TZ index length is not a multiple of 24"));
    }
    let mut f = File::open(path)?;
    for rec in index.chunks_exact(24) {
        let off = u64::from_le_bytes(rec[16..24].try_into().unwrap());
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
        let extra_len = u16::from_le_bytes(hdr[28..30].try_into().unwrap()) as usize;
        if extra_len != 0 {
            return Err(Error::msg(
                "3TZ local header extra field must be empty (index assumes 30-byte headers)",
            ));
        }
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
