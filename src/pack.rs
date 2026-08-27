//! Pack a tileset directory (or tileset.json + siblings) into `.3tz` (ZIP).
//!
//! A standard ZIP with `tileset.json` at the archive root is enough for
//! TinyOwl [`tileset.ExtractZip`]. This is the `convert` subset of 3d-tiles-tools.

use std::fs::{self, File};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

use walkdir::WalkDir;
use zip::write::SimpleFileOptions;
use zip::CompressionMethod;
use zip::ZipWriter;

use crate::error::Error;

#[derive(Clone, Debug, Default)]
pub struct PackOptions {
    pub force: bool,
}

pub fn convert_to_3tz(input: &Path, output: &Path, opts: &PackOptions) -> Result<(), Error> {
    if output.exists() && !opts.force {
        return Err(Error::OutputExists(output.to_path_buf()));
    }
    let root = tileset_root(input)?;
    if !root.join("tileset.json").is_file() {
        return Err(Error::MissingTilesetJson);
    }
    if let Some(parent) = output.parent() {
        if !parent.as_os_str().is_empty() {
            fs::create_dir_all(parent)?;
        }
    }

    let file = File::create(output)?;
    let mut zip = ZipWriter::new(file);
    let options = SimpleFileOptions::default()
        .compression_method(CompressionMethod::Deflated)
        .unix_permissions(0o644);

    let mut buf = Vec::new();
    for entry in WalkDir::new(&root).into_iter().filter_map(|e| e.ok()) {
        if !entry.file_type().is_file() {
            continue;
        }
        let abs = entry.path();
        let rel = abs
            .strip_prefix(&root)
            .unwrap_or(abs)
            .to_string_lossy()
            .replace('\\', "/");
        if rel.is_empty() {
            continue;
        }
        buf.clear();
        File::open(abs)?.read_to_end(&mut buf)?;
        zip.start_file(&rel, options)?;
        zip.write_all(&buf)?;
    }
    zip.finish()?;
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
