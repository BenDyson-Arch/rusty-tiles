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

/// Decimal JSON's worst case per reference includes three interface rows (27 u32s),
/// the certificate (15 u32s, two u64s, f64), and two terminal vertices (six u32s).
/// 1 KiB covers these numbers and fixed keys; 8 KiB covers the fixed tile/omission
/// row, topology and measurement summaries, including full-width counters.
pub(super) fn bound(refs: u64, nodes: u64) -> Option<u64> {
    refs.checked_mul(1024)?
        .checked_add(nodes.checked_mul(8192)?)?
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
    pub(super) fn omitted(
        &self,
        key: TileKey,
        reason: super::OmissionReason,
        frontier: &[TileKey],
        topology: super::geometry::TopologyReceipt,
    ) -> Result<(), JobError> {
        #[derive(Serialize)]
        #[serde(rename_all = "camelCase")]
        struct Omitted<'a> {
            schema_version: u32,
            kind: &'static str,
            key: TileKey,
            reason: super::OmissionReason,
            source: bool,
            frontier: &'a [TileKey],
            attempted_topology: super::geometry::TopologyReceipt,
        }
        self.append(&Omitted {
            schema_version: 1,
            kind: "omitted",
            key,
            reason,
            source: reason.source(),
            frontier,
            attempted_topology: topology,
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{mesh_large::hlod::scratch::ScratchLedger, spill::Scratch};

    #[test]
    fn terminal_contact_json_fits_the_precharged_reference_bound() {
        use crate::mesh_large::hlod::{borders::Certificate, interfaces::InterfaceRow};
        let a = [u32::MAX; 3];
        let b = [u32::MAX - 1; 3];
        let contract = Contract {
            depth: 31,
            interface: vec![
                InterfaceRow::new(u32::MAX, a, a, u32::MAX, u32::MAX),
                InterfaceRow::new(u32::MAX, b, b, u32::MAX, u32::MAX),
                InterfaceRow::new(u32::MAX, b, a, u32::MAX, u32::MAX),
            ],
            certificates: vec![Certificate {
                endpoints: [a, b],
                chain: [a, b],
                origin: a,
                first: u64::MAX - 1,
                end: u64::MAX,
                epsilon: f64::MAX,
                terminal_lock: false,
            }],
            terminal_vertices: vec![a, b],
            scale: f64::MAX,
        };
        let n = serde_json::to_vec(&contract).unwrap().len() as u64;
        // Includes fixed contract keys too, so this is stricter than the per-ref claim.
        assert!(n <= bound(1, 0).unwrap() - bound(0, 0).unwrap(), "{n}");
    }

    #[test]
    fn sidecar_ranges_hash_and_exclusive_creation_are_independent_of_stage_lifetime() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("out.hlod-audit");
        let receipt;
        {
            let scratch = Scratch::create(Some(dir.path()), 0).unwrap();
            let ledger = ScratchLedger::new(&scratch, 0, &|_| Ok(()));
            let lease = ledger.admit_external("audit", 4096).unwrap();
            let writer = Writer::new(&path, 4096, &lease).unwrap();
            let one = writer
                .append(&serde_json::json!({"kind":"control", "id": 1}))
                .unwrap();
            let two = writer
                .append(&serde_json::json!({"kind":"control", "id": 2}))
                .unwrap();
            receipt = writer.finish().unwrap();
            let bytes = fs::read(&receipt.path).unwrap();
            assert_eq!(receipt.bytes as usize, bytes.len());
            assert_eq!(receipt.sha256, format!("{:x}", Sha256::digest(&bytes)));
            assert_eq!(one.offset, 0);
            assert_eq!(two.offset, one.bytes);
            assert_eq!(two.offset + two.bytes, receipt.bytes);
            for (index, id) in [(one, 1), (two, 2)] {
                let at = index.offset as usize;
                let row: serde_json::Value =
                    serde_json::from_slice(&bytes[at..at + index.bytes as usize]).unwrap();
                assert_eq!(row["id"], id);
            }
            assert!(Writer::new(&path, 4096, &lease).is_err());
            assert_eq!(fs::read(&receipt.path).unwrap(), bytes);
        }
        assert_eq!(fs::metadata(receipt.path).unwrap().len(), receipt.bytes);
    }

    #[test]
    fn sidecar_overrun_is_refused_before_any_file_bytes_are_written() {
        let dir = tempfile::tempdir().unwrap();
        let scratch = Scratch::create(Some(dir.path()), 0).unwrap();
        let ledger = ScratchLedger::new(&scratch, 0, &|_| Ok(()));
        let lease = ledger.admit_external("audit", 1).unwrap();
        let writer = Writer::new(&dir.path().join("audit"), 1, &lease).unwrap();
        writer
            .append(&serde_json::json!({"tooLarge":true}))
            .unwrap();
        assert!(writer.finish().is_err());
        assert_eq!(fs::metadata(&writer.path).unwrap().len(), 0);
    }
}
