//! Opt-in durable audit evidence, outside the client archive. One streaming JSONL file
//! avoids source-sized filename/hash lists; every byte is admitted before the first write.
use super::{
    borders::Contract,
    geometry::{locations::Locations, Built},
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
/// 1 KiB covers these numbers and fixed keys; 8 KiB covers each fixed tile/omission.
/// Location banks keep at most eight faces and eight edges apiece. Three shared banks
/// plus receipt/framing fit 16 KiB per proposal. Per input face, 2 KiB covers a one-face
/// dropped-component bank and every possible cycle-rank operation descriptor. Parent
/// inputs at any depth partition the source faces; retries revisit them at most 72 times
/// (two caps plus seven remesh rounds, two hole policies, RETRIES+1 each). One additional
/// construction record locates cleanup drops on final failure. That counts one build: the
/// last-resort build's pinned retries, coarse-drop bisection and panels, and the staged
/// build's accepted proposal, can exceed it; the generous per-record allowance absorbs
/// this in practice, and an overshoot refuses the opt-in audit (ResourceLimit).
/// This is a logical maximum: disk space is checked on actual writes. Memory-backed
/// destinations use the explicit byte budget below in both planning and writing.
const MAX_LOCATION_RECORDS: u64 = (2 + 7) * 2 * (super::geometry::RETRIES as u64 + 1) + 1;
const LOCATION_FIXED_BYTES: u64 = 16 << 10;
const LOCATION_INPUT_BYTES: u64 = 2 << 10;
pub(super) fn bound(refs: u64, nodes: u64, source_triangle_depths: u64) -> Option<u64> {
    refs.checked_mul(1024)?
        .checked_add(nodes.checked_mul(8192)?)?
        .checked_add(
            nodes
                .checked_mul(LOCATION_FIXED_BYTES)?
                .checked_add(source_triangle_depths.checked_mul(LOCATION_INPUT_BYTES)?)?
                .checked_mul(MAX_LOCATION_RECORDS)?,
        )?
        .checked_add(4096)
}
/// Audit evidence is optional but complete: exceeding this internal tmpfs budget
/// refuses the run; it never truncates a record or skips operations. Ordinary disks
/// retain the logical maximum and preflight only actual growth.
const MEMORY_BACKED_BYTE_BUDGET: u64 = 64 << 20;
pub(super) fn admitted_bound(logical: u64, memory_backed: bool) -> u64 {
    if memory_backed {
        logical.min(MEMORY_BACKED_BYTE_BUDGET)
    } else {
        logical
    }
}

/// Measure a record without a record-sized allocation, so a resource refusal precedes
/// every byte of that record, including bytes that would otherwise enter BufWriter.
#[derive(Default)]
struct Count(u64);
impl Write for Count {
    fn write(&mut self, data: &[u8]) -> io::Result<usize> {
        self.0 = self
            .0
            .checked_add(data.len() as u64)
            .ok_or_else(|| io::Error::other("HLOD audit record size overflows"))?;
        Ok(data.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
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
    path: PathBuf,
    bound: u64,
    bytes: u64,
    hash: Sha256,
    lease: &'a ComponentLease<'a, 'a>,
}
impl Write for Hashed<'_> {
    fn write(&mut self, data: &[u8]) -> io::Result<usize> {
        let next = self
            .bytes
            .checked_add(data.len() as u64)
            .ok_or_else(|| io::Error::other("HLOD audit byte count overflows"))?;
        if next > self.bound {
            return Err(io::Error::other(
                "HLOD audit exceeds its admitted byte budget",
            ));
        }
        self.lease.charge(next).map_err(io::Error::other)?;
        // The lease owns a conservative maximum. Ordinary disks only need room for
        // actual growth; bytes already written are reflected in the volume's free space.
        preflight(
            existing_parent(&self.path),
            data.len() as u64,
            SpaceRole::Output,
            PreflightStage::Lod,
        )
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
            0,
            SpaceRole::Output,
            PreflightStage::Lod,
        )?;
        // Exclusive directory creation makes evidence immutable, including on reruns.
        fs::create_dir(dir).map_err(|e| JobError::io("create HLOD audit directory", dir, e))?;
        let path = dir.join("borders.jsonl");
        let file =
            File::create_new(&path).map_err(|e| JobError::io("create HLOD audit", &path, e))?;
        Ok(Self {
            path: path.clone(),
            stream: Mutex::new(BufWriter::with_capacity(
                64 << 10,
                Hashed {
                    file,
                    path: path.clone(),
                    bound,
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
        let mut count = Count::default();
        serde_json::to_writer(&mut count, record).map_err(|e| super::state(e.to_string()))?;
        let next = offset
            .checked_add(count.0)
            .and_then(|n| n.checked_add(1))
            .ok_or_else(|| super::resource("HLOD audit byte count overflows"))?;
        if next > s.get_ref().bound {
            return Err(super::resource(format!(
                "HLOD audit needs {next} bytes, past its admitted byte budget {}",
                s.get_ref().bound,
            )));
        }
        s.get_ref().lease.charge(next)?;
        serde_json::to_writer(&mut *s, record).map_err(|e| super::state(e.to_string()))?;
        s.write_all(b"\n")
            .map_err(|e| JobError::io("write HLOD audit", &self.path, e))?;
        Ok(Index {
            offset,
            bytes: s.get_ref().bytes + s.buffer().len() as u64 - offset,
        })
    }
    /// Stream a measured proposal while its bounded geometry is still resident. Attempt
    /// ordinals are local to a candidate key; worker scheduling may reorder these lines.
    /// The staged build streams only its accepted proposal (`Built::staged`); the row's
    /// attempt count still includes its rejected ones.
    pub(super) fn topology_attempt(
        &self,
        built: &Built,
        attempt: u32,
        accepted: bool,
    ) -> Result<Index, JobError> {
        #[derive(Serialize)]
        #[serde(rename_all = "camelCase")]
        struct Attempt {
            schema_version: u32,
            kind: &'static str,
            key: TileKey,
            attempt: u32,
            accepted: bool,
            hint_only: bool,
            topology: super::geometry::TopologyReceipt,
            locations: Locations,
        }
        let mut topology = built.change.receipt;
        if !accepted {
            topology.rejected_hole_closures += topology.closed_holes;
            topology.rejected_hole_openings += topology.opened_holes;
        }
        self.append(&Attempt {
            schema_version: 1,
            kind: "topologyAttempt",
            key: built.geometry.key,
            attempt,
            accepted,
            hint_only: true,
            topology,
            locations: Locations::capture(built)?,
        })
    }
    /// Intermediate cleanup drops that survive in an omitted job's attempted receipt.
    /// A published candidate already carries these changes in its reference comparison.
    pub(super) fn construction(
        &self,
        key: TileKey,
        dropped: &[[[f32; 3]; 3]],
    ) -> Result<Index, JobError> {
        #[derive(Serialize)]
        #[serde(rename_all = "camelCase")]
        struct Construction {
            schema_version: u32,
            kind: &'static str,
            key: TileKey,
            hint_only: bool,
            dropped_components: u64,
            locations: Locations,
        }
        let locations = Locations::construction(key, dropped);
        self.append(&Construction {
            schema_version: 1,
            kind: "topologyConstruction",
            key,
            hint_only: true,
            dropped_components: locations.dropped_count(),
            locations,
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
            topology_attempts: u32,
            published_topology_attempt: Option<Index>,
        }
        self.append(&Tile {
            schema_version: 1,
            kind: "tile",
            tile: row,
            contract,
            topology_attempts: row.topology_attempts,
            published_topology_attempt: row.published_topology_attempt,
        })
    }
    pub(super) fn omitted(
        &self,
        key: TileKey,
        reason: super::OmissionReason,
        frontier: &[TileKey],
        topology: super::geometry::TopologyReceipt,
        topology_attempts: u32,
        construction_topology: Option<Index>,
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
            topology_attempts: u32,
            construction_topology: Option<Index>,
        }
        self.append(&Omitted {
            schema_version: 1,
            kind: "omitted",
            key,
            reason,
            source: reason.source(),
            frontier,
            attempted_topology: topology,
            topology_attempts,
            construction_topology,
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

    fn located_proposal() -> Built {
        use crate::mesh_large::hlod::{
            geometry::TopologyReceipt,
            store::{StoppingBound, TileGeometry},
        };
        let positions = vec![[0., 0., 0.], [1., 0., 0.], [0., 1., 0.]];
        let mut built = Built {
            geometry: TileGeometry {
                key: TileKey {
                    first: 10,
                    end: 20,
                    depth: 2,
                    leaf_begin: 3,
                    leaf_end: 5,
                },
                positions: positions.clone(),
                ells: vec![0; 3],
                triangles: vec![[0, 1, 2]],
                primitives: Vec::new(),
                representative: Vec::new(),
                interface: Vec::new(),
                geometric_error: 0.,
                lease: None,
            },
            locked: vec![false; 3],
            stopping: StoppingBound::Input,
            simplifier_error: 0.,
            children_error: 0.,
            input_triangles: 3,
            protected_faces: 0,
            retries: 0,
            last_resort: false,
            staged: false,
            interface_vertices: 0,
            interface_segments: 0,
            reoriented: 0,
            topology: TopologyReceipt::default(),
            change: Default::default(),
            controls: Default::default(),
        };
        built.change.receipt.closed_holes = 1;
        built.change.receipt.opened_holes = 1;
        built.change.receipt.dropped_components = 1;
        built.change.patch_faces = vec![0];
        built.change.boundary = vec![[positions[0], positions[1]]];
        built.change.dropped = vec![[[4., 0., 0.], [5., 0., 0.], [4., 1., 0.]]];
        built.change.opening_reference = vec![([positions[0], positions[1], positions[2]], true)];
        built.change.opening_boundary = vec![[positions[1], positions[2]]];
        built.change.opening_faces = vec![true];
        built
    }

    #[test]
    fn measured_proposals_locate_every_counted_change_and_link_the_exact_attempt() {
        let dir = tempfile::tempdir().unwrap();
        let scratch = Scratch::create(Some(dir.path()), 0).unwrap();
        let ledger = ScratchLedger::new(&scratch, 0, &|_| Ok(()));
        let lease = ledger.admit_external("audit", 64 << 10).unwrap();
        let writer = Writer::new(&dir.path().join("audit"), 64 << 10, &lease).unwrap();
        let built = located_proposal();
        let rejected = writer.topology_attempt(&built, 1, false).unwrap();
        let accepted = writer.topology_attempt(&built, 2, true).unwrap();
        let receipt = writer.finish().unwrap();
        let bytes = fs::read(receipt.path).unwrap();
        for (index, attempt, published) in [(rejected, 1, false), (accepted, 2, true)] {
            let at = index.offset as usize;
            let row: serde_json::Value =
                serde_json::from_slice(&bytes[at..at + index.bytes as usize]).unwrap();
            assert_eq!(row["schemaVersion"], 1);
            assert_eq!(row["kind"], "topologyAttempt");
            assert_eq!(row["hintOnly"], true);
            assert_eq!(row["attempt"], attempt);
            assert_eq!(row["accepted"], published);
            assert_eq!(row["key"]["leafBegin"], 3);
            let loc = &row["locations"];
            assert_eq!(loc["sourceHint"], row["key"]);
            assert_eq!(loc["operations"].as_array().unwrap().len(), 3);
            assert_eq!(
                loc["closure"]["faceAnchors"][0][1],
                serde_json::json!([1., 0., 0.])
            );
            assert_eq!(loc["closure"]["edgeCount"], 1);
            assert_eq!(loc["dropped"][0]["min"], serde_json::json!([4., 0., 0.]));
            assert_eq!(loc["dropped"][0]["max"], serde_json::json!([5., 1., 0.]));
            assert_eq!(loc["opening"]["rimAndLostRegion"]["faceCount"], 1);
            assert_eq!(loc["opening"]["rimAndLostRegion"]["edgeCount"], 1);
            assert_eq!(loc["opening"]["incidentFaces"]["faceCount"], 1);
            assert_eq!(
                row["topology"]["rejectedHoleClosures"],
                if published { 0 } else { 1 }
            );
        }
    }

    #[test]
    fn omitted_construction_drops_keep_a_location_link_without_measured_proposals() {
        let dir = tempfile::tempdir().unwrap();
        let scratch = Scratch::create(Some(dir.path()), 0).unwrap();
        let ledger = ScratchLedger::new(&scratch, 0, &|_| Ok(()));
        let limit = bound(0, 1, 3).unwrap();
        let lease = ledger.admit_external("audit", limit).unwrap();
        let writer = Writer::new(&dir.path().join("audit"), limit, &lease).unwrap();
        let built = located_proposal();
        let key = built.geometry.key;
        let index = writer.construction(key, &built.change.dropped).unwrap();
        let topology = super::super::geometry::TopologyReceipt {
            dropped_components: 1,
            ..Default::default()
        };
        writer
            .omitted(
                key,
                super::super::OmissionReason::RemeshStalled,
                &[],
                topology,
                0,
                Some(index),
            )
            .unwrap();
        let receipt = writer.finish().unwrap();
        let bytes = fs::read(receipt.path).unwrap();
        let records: Vec<serde_json::Value> = bytes
            .split(|&b| b == b'\n')
            .filter(|s| !s.is_empty())
            .map(|s| serde_json::from_slice(s).unwrap())
            .collect();
        assert_eq!(records[0]["kind"], "topologyConstruction");
        assert_eq!(records[0]["droppedComponents"], 1);
        assert_eq!(records[0]["locations"]["operations"][0]["kind"], "drop");
        assert_eq!(records[0]["locations"]["dropped"][0]["faceCount"], 1);
        assert_eq!(records[1]["kind"], "omitted");
        assert_eq!(records[1]["topologyAttempts"], 0);
        assert_eq!(records[1]["constructionTopology"]["offset"], index.offset);
        assert_eq!(records[1]["constructionTopology"]["bytes"], index.bytes);
        assert_eq!(records[1]["attemptedTopology"]["droppedComponents"], 1);
    }

    #[test]
    fn location_bound_covers_real_serialization_and_rejects_overflow() {
        let mut built = located_proposal();
        let p = [-f32::MAX, f32::MIN_POSITIVE, -f32::from_bits(1)];
        built.geometry.positions = vec![p; 3];
        built.change.patch_faces = vec![0; 100];
        built.change.boundary = vec![[p; 2]; 100];
        built.change.opening_reference = vec![([p; 3], true); 100];
        built.change.opening_boundary = vec![[p; 2]; 100];
        let dir = tempfile::tempdir().unwrap();
        let scratch = Scratch::create(Some(dir.path()), 0).unwrap();
        let ledger = ScratchLedger::new(&scratch, 0, &|_| Ok(()));
        let one_attempt = LOCATION_FIXED_BYTES + 3 * LOCATION_INPUT_BYTES;
        let lease = ledger.admit_external("audit", one_attempt).unwrap();
        let writer = Writer::new(&dir.path().join("audit"), one_attempt, &lease).unwrap();
        let index = writer.topology_attempt(&built, u32::MAX, false).unwrap();
        writer.finish().unwrap();
        assert!(index.bytes <= one_attempt);
        assert!(bound(0, 1, 3).unwrap() >= one_attempt * MAX_LOCATION_RECORDS);
        assert!(bound(u64::MAX, 1, 1).is_none());
        assert!(bound(1, u64::MAX, 1).is_none());
        assert!(bound(1, 1, u64::MAX).is_none());
    }

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
        assert!(
            n <= bound(1, 0, 0).unwrap() - bound(0, 0, 0).unwrap(),
            "{n}"
        );
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
    fn memory_backed_budget_selection_preserves_small_bounds_and_caps_large_ones() {
        let cap = MEMORY_BACKED_BYTE_BUDGET;
        for logical in [0, cap - 1, cap, cap + 1, u64::MAX] {
            assert_eq!(admitted_bound(logical, false), logical);
            assert_eq!(admitted_bound(logical, true), logical.min(64 << 20));
        }
    }

    #[test]
    fn selected_budget_accepts_exact_bytes_then_refuses_a_record_without_partial_evidence() {
        let dir = tempfile::tempdir().unwrap();
        let scratch = Scratch::create(Some(dir.path()), 0).unwrap();
        let ledger = ScratchLedger::new(&scratch, 0, &|_| Ok(()));
        let record = serde_json::json!({"kind":"control", "id": 7});
        let expected = serde_json::to_vec(&record).unwrap();
        let selected = admitted_bound(expected.len() as u64 + 1, true);
        let lease = ledger.admit_external("audit", selected).unwrap();
        let writer = Writer::new(&dir.path().join("audit"), selected, &lease).unwrap();
        let accepted = writer.append(&record).unwrap();
        assert_eq!(accepted.bytes, selected);
        let before = writer.finish().unwrap();
        let bytes = fs::read(&before.path).unwrap();
        assert_eq!(bytes.len() as u64, selected);
        assert_eq!(&bytes[..expected.len()], expected.as_slice());
        assert_eq!(bytes[expected.len()], b'\n');
        let error = writer.append(&record).unwrap_err();
        assert_eq!(error.kind(), crate::JobErrorKind::ResourceLimit);
        let after = writer.finish().unwrap();
        assert_eq!(after.bytes, before.bytes);
        assert_eq!(after.sha256, before.sha256);
        assert_eq!(fs::read(&after.path).unwrap(), bytes);
    }

    #[test]
    fn sidecar_overrun_is_refused_before_any_file_bytes_are_written() {
        let dir = tempfile::tempdir().unwrap();
        let scratch = Scratch::create(Some(dir.path()), 0).unwrap();
        let ledger = ScratchLedger::new(&scratch, 0, &|_| Ok(()));
        let lease = ledger.admit_external("audit", 1).unwrap();
        let writer = Writer::new(&dir.path().join("audit"), 1, &lease).unwrap();
        let error = writer
            .append(&serde_json::json!({"tooLarge":true}))
            .unwrap_err();
        assert_eq!(error.kind(), crate::JobErrorKind::ResourceLimit);
        assert_eq!(writer.finish().unwrap().bytes, 0);
        assert_eq!(fs::metadata(&writer.path).unwrap().len(), 0);
    }
}
