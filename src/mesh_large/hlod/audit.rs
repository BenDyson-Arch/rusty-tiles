//! Opt-in durable audit evidence, outside the client archive. One streaming JSONL file
//! avoids source-sized filename/hash lists; every byte is admitted before the first write.
use super::{
    borders::Contract,
    scratch::ComponentLease,
    store::{HlodRow, TileKey},
};
use crate::{
    spill::{inspect_volume, preflight, PreflightStage, SpaceRole},
    JobError,
};
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::{
    fs::{self, File},
    io::{self, BufWriter, Write},
    path::{Path, PathBuf},
    sync::Mutex,
};

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FileReceipt {
    pub path: PathBuf,
    pub bytes: u64,
    pub sha256: String,
}
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Summary {
    pub file: &'static str,
    pub bytes: u64,
    pub sha256: String,
}
impl From<&FileReceipt> for Summary {
    fn from(r: &FileReceipt) -> Self {
        Self {
            file: "borders.jsonl",
            bytes: r.bytes,
            sha256: r.sha256.clone(),
        }
    }
}
#[derive(Clone, Copy, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Index {
    pub offset: u64,
    pub bytes: u64,
}

/// Decimal JSON's worst case: a source arc record has at most 24 u32 coordinates,
/// three u64 ordinals, one finite f64 and fixed keys. Point contacts are smaller.
pub(super) fn bound(refs: u64, nodes: u64) -> Option<u64> {
    refs.checked_mul(768)?
        .checked_add(nodes.checked_mul(4096)?)?
        .checked_add(4096)
}
pub(super) fn existing_parent(path: &Path) -> &Path {
    let mut at = path;
    while !at.exists() {
        at = at.parent().unwrap_or(Path::new("."));
    }
    at
}
pub(super) fn memory_backed(path: &Path) -> Result<bool, JobError> {
    Ok(inspect_volume(existing_parent(path))?.memory_backed)
}
struct Hashed<'a> {
    file: File,
    bytes: u64,
    hash: Sha256,
    lease: &'a ComponentLease<'a, 'a>,
}
impl Write for Hashed<'_> {
    fn write(&mut self, data: &[u8]) -> io::Result<usize> {
        self.lease
            .charge(self.bytes.saturating_add(data.len() as u64))
            .map_err(io::Error::other)?;
        let n = self.file.write(data)?;
        self.bytes += n as u64;
        self.hash.update(&data[..n]);
        Ok(n)
    }
    fn flush(&mut self) -> io::Result<()> {
        self.file.flush()
    }
}
pub(super) struct Writer<'a> {
    path: PathBuf,
    stream: Mutex<BufWriter<Hashed<'a>>>,
}
impl<'a> Writer<'a> {
    pub(super) fn new(
        dir: &Path,
        bound: u64,
        lease: &'a ComponentLease<'a, 'a>,
    ) -> Result<Self, JobError> {
        preflight(
            existing_parent(dir),
            bound,
            SpaceRole::Output,
            PreflightStage::Lod,
        )?;
        // Exclusive directory creation makes evidence immutable, including on reruns.
        fs::create_dir(dir).map_err(|e| JobError::io("create HLOD audit directory", dir, e))?;
        let path = dir.join("borders.jsonl");
        let file =
            File::create_new(&path).map_err(|e| JobError::io("create HLOD audit", &path, e))?;
        Ok(Self {
            path,
            stream: Mutex::new(BufWriter::with_capacity(
                64 << 10,
                Hashed {
                    file,
                    bytes: 0,
                    hash: Sha256::new(),
                    lease,
                },
            )),
        })
    }
    fn append(&self, record: &impl Serialize) -> Result<Index, JobError> {
        let mut s = self.stream.lock().unwrap_or_else(|e| e.into_inner());
        let offset = s.get_ref().bytes + s.buffer().len() as u64;
        serde_json::to_writer(&mut *s, record).map_err(|e| super::state(e.to_string()))?;
        s.write_all(b"\n")
            .map_err(|e| JobError::io("write HLOD audit", &self.path, e))?;
        Ok(Index {
            offset,
            bytes: s.get_ref().bytes + s.buffer().len() as u64 - offset,
        })
    }
    pub(super) fn tile(&self, row: &HlodRow, contract: &Contract) -> Result<Index, JobError> {
        #[derive(Serialize)]
        #[serde(rename_all = "camelCase")]
        struct Tile<'a> {
            schema_version: u32,
            kind: &'static str,
            tile: &'a HlodRow,
            contract: &'a Contract,
        }
        self.append(&Tile {
            schema_version: 1,
            kind: "tile",
            tile: row,
            contract,
        })
    }
    pub(super) fn omitted(&self, key: TileKey, source: bool) -> Result<(), JobError> {
        #[derive(Serialize)]
        #[serde(rename_all = "camelCase")]
        struct Omitted {
            schema_version: u32,
            kind: &'static str,
            key: TileKey,
            source: bool,
        }
        self.append(&Omitted {
            schema_version: 1,
            kind: "omitted",
            key,
            source,
        })
        .map(|_| ())
    }
    pub(super) fn finish(&self) -> Result<FileReceipt, JobError> {
        let mut s = self.stream.lock().unwrap_or_else(|e| e.into_inner());
        s.flush()
            .and_then(|_| s.get_ref().file.sync_all())
            .map_err(|e| JobError::io("finish HLOD audit", &self.path, e))?;
        Ok(FileReceipt {
            path: self.path.clone(),
            bytes: s.get_ref().bytes,
            sha256: format!("{:x}", s.get_ref().hash.clone().finalize()),
        })
    }
}
