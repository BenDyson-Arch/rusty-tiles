//! Adaptive sibling spilling. The walk holds a bounded prefix of keys; payloads can leave
//! the resident ledger before another leaf or parent is admitted. Parent jobs reload at
//! most eight capped children inside a separately charged read allowance.
use super::{SourcePrimitive, TileGeometry, TileKey};
use crate::{
    mesh_large::hlod::{
        interfaces::InterfaceRow,
        scratch::{ComponentLease, ScratchLedger},
    },
    spill::{ByteFile, ByteWriter, Record, Scratch},
    JobError,
};
use std::{
    collections::HashMap,
    io::Read,
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc,
    },
};

const BUFFER: usize = 64 << 10;
#[derive(Clone, Copy, Debug, Default, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Stats {
    pub tiles: u64,
    pub bytes: u64,
    pub peak_bytes: u64,
}

pub(crate) fn bound(
    source_triangles: u64,
    nodes: u64,
    leaf_cap: u64,
    cap: u64,
    height: u32,
    allocation: u64,
) -> Option<u64> {
    let limit = limit(height) as u64 + 1;
    let all = source_triangles.checked_add(nodes.checked_mul(cap)?)?;
    let triangles = all.min(limit.checked_mul(leaf_cap.max(cap))?);
    triangles
        .checked_mul(super::HELD_BYTES_PER_TRIANGLE)?
        .checked_add(limit.checked_mul(256 + allocation.checked_mul(2)?)?)
}
pub(crate) fn limit(height: u32) -> usize {
    8 * (height as usize + 1) + 32
}

pub(crate) enum Held {
    Resident(TileGeometry),
    Disk(Spilled),
}
impl Held {
    pub(crate) fn key(&self) -> TileKey {
        match self {
            Self::Resident(t) => t.key,
            Self::Disk(t) => t.key,
        }
    }
    pub(crate) fn triangles(&self) -> u64 {
        match self {
            Self::Resident(t) => t.triangles.len() as u64,
            Self::Disk(t) => t.triangles,
        }
    }
    pub(crate) fn load(self) -> Result<TileGeometry, JobError> {
        match self {
            Self::Resident(t) => Ok(t),
            Self::Disk(t) => t.load(),
        }
    }
}
pub(crate) struct Spilled {
    file: ByteFile,
    key: TileKey,
    triangles: u64,
    live: Arc<AtomicU64>,
}
impl Drop for Spilled {
    fn drop(&mut self) {
        self.live.fetch_sub(self.file.len(), Ordering::Relaxed);
    }
}
impl Spilled {
    fn load(&self) -> Result<TileGeometry, JobError> {
        let mut reader = self.file.reader(BUFFER)?;
        let mut read = |n: usize| -> Result<[u8; 36], JobError> {
            let mut b = [0; 36];
            reader
                .read_exact(&mut b[..n])
                .map_err(|e| JobError::io("read held tile", self.file.path(), e))?;
            Ok(b)
        };
        let header = read(24)?;
        let sizes: [usize; 4] = std::array::from_fn(|i| {
            u32::from_le_bytes(header[i * 4..i * 4 + 4].try_into().unwrap()) as usize
        });
        let [vertices, triangles, primitives, interfaces] = sizes;
        if triangles as u64 != self.triangles
            || vertices > 3 * triangles
            || primitives > triangles
            || interfaces > 6 * triangles
        {
            return Err(super::super::state("invalid held-tile lengths"));
        }
        let mut tile = TileGeometry {
            key: self.key,
            ells: Vec::with_capacity(vertices),
            positions: Vec::with_capacity(vertices),
            triangles: Vec::with_capacity(triangles),
            representative: Vec::with_capacity(triangles),
            primitives: Vec::with_capacity(primitives),
            interface: Vec::with_capacity(interfaces),
            geometric_error: f64::from_le_bytes(header[16..24].try_into().unwrap()),
            lease: None,
        };
        for _ in 0..vertices {
            let b = read(13)?;
            tile.ells.push(b[0]);
            tile.positions.push(std::array::from_fn(|i| {
                f32::from_le_bytes(b[1 + i * 4..5 + i * 4].try_into().unwrap())
            }));
        }
        for _ in 0..triangles {
            let b = read(16)?;
            tile.triangles.push(std::array::from_fn(|i| {
                u32::from_le_bytes(b[i * 4..i * 4 + 4].try_into().unwrap())
            }));
            tile.representative
                .push(u32::from_le_bytes(b[12..16].try_into().unwrap()));
        }
        for _ in 0..primitives {
            let b = read(24)?;
            let word = |i| u32::from_le_bytes(b[i..i + 4].try_into().unwrap());
            tile.primitives.push(SourcePrimitive {
                node: word(0),
                mesh: word(4),
                primitive: word(8),
                expanded_ordinal: u64::from_le_bytes(b[12..20].try_into().unwrap()),
                material: word(20),
            });
        }
        for _ in 0..interfaces {
            tile.interface.push(InterfaceRow::decode(&read(36)?));
        }
        if tile
            .triangles
            .iter()
            .flatten()
            .any(|v| *v as usize >= vertices)
            || tile
                .representative
                .iter()
                .any(|v| *v as usize >= primitives)
        {
            return Err(super::super::state("invalid held-tile indices"));
        }
        tile.check_charge()?;
        Ok(tile)
    }
}

pub(crate) struct Manager<'l, 's> {
    scratch: &'s Scratch,
    ledger: &'l ScratchLedger<'s>,
    lease: Option<ComponentLease<'l, 's>>,
    bound: u64,
    live: Arc<AtomicU64>,
    stats: Stats,
}
impl<'l, 's> Manager<'l, 's> {
    pub(crate) fn new(scratch: &'s Scratch, ledger: &'l ScratchLedger<'s>, bound: u64) -> Self {
        Self {
            scratch,
            ledger,
            lease: None,
            bound,
            live: Arc::new(AtomicU64::new(0)),
            stats: Stats::default(),
        }
    }
    pub(crate) fn stats(&self) -> Stats {
        self.stats
    }
    fn spill(&mut self, tile: TileGeometry) -> Result<Spilled, JobError> {
        if self.lease.is_none() {
            self.lease = Some(self.ledger.admit("s1b-held-tiles", self.bound)?);
        }
        let size = 24
            + 13 * tile.positions.len() as u64
            + 16 * tile.triangles.len() as u64
            + 24 * tile.primitives.len() as u64
            + 36 * tile.interface.len() as u64;
        let next = self
            .live
            .load(Ordering::Relaxed)
            .checked_add(size)
            .ok_or_else(|| super::super::resource("held spill bytes overflow"))?;
        self.lease.as_ref().unwrap().charge(next)?;
        let mut writer = ByteWriter::new(self.scratch, "hlod-held-tile", BUFFER)?;
        for n in [
            tile.positions.len(),
            tile.triangles.len(),
            tile.primitives.len(),
            tile.interface.len(),
        ] {
            writer.append(&(n as u32).to_le_bytes())?;
        }
        writer.append(&tile.geometric_error.to_le_bytes())?;
        for (p, ell) in tile.positions.iter().zip(&tile.ells) {
            writer.append(&[*ell])?;
            for c in p {
                writer.append(&c.to_le_bytes())?;
            }
        }
        for (t, r) in tile.triangles.iter().zip(&tile.representative) {
            for v in t {
                writer.append(&v.to_le_bytes())?;
            }
            writer.append(&r.to_le_bytes())?;
        }
        for p in &tile.primitives {
            for v in [p.node, p.mesh, p.primitive] {
                writer.append(&v.to_le_bytes())?;
            }
            writer.append(&p.expanded_ordinal.to_le_bytes())?;
            writer.append(&p.material.to_le_bytes())?;
        }
        for r in &tile.interface {
            let mut b = [0; 36];
            r.encode(&mut b);
            writer.append(&b)?;
        }
        let file = writer.finish()?;
        assert_eq!(file.len(), size);
        self.live.store(next, Ordering::Relaxed);
        self.stats.tiles += 1;
        self.stats.bytes += size;
        self.stats.peak_bytes = self.stats.peak_bytes.max(next);
        self.lease.as_ref().unwrap().charge(next)?;
        Ok(Spilled {
            file,
            key: tile.key,
            triangles: tile.triangles.len() as u64,
            live: self.live.clone(),
        })
    }
    /// Oldest resident sibling first, independent of HashMap iteration and worker timing.
    pub(crate) fn spill_one(
        &mut self,
        ready: &mut HashMap<u64, Option<Held>>,
    ) -> Result<bool, JobError> {
        let id = ready
            .iter()
            .filter(|(_, t)| matches!(t, Some(Held::Resident(_))))
            .map(|(id, _)| *id)
            .min();
        let Some(id) = id else {
            return Ok(false);
        };
        let Some(Held::Resident(tile)) = ready.remove(&id).unwrap() else {
            unreachable!()
        };
        ready.insert(id, Some(Held::Disk(self.spill(tile)?)));
        Ok(true)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mesh_large::hlod::interfaces::key;

    #[test]
    fn held_tiles_round_trip_and_release_their_admitted_bytes() {
        let dir = tempfile::tempdir().unwrap();
        let scratch = Scratch::create(Some(dir.path()), 0).unwrap();
        let calls = std::sync::Mutex::new(Vec::new());
        let admit = |n| {
            calls.lock().unwrap().push(n);
            Ok(())
        };
        let ledger = ScratchLedger::new(&scratch, 0, &admit);
        let mut manager = Manager::new(&scratch, &ledger, 8192);
        let p = [[0., 0., 0.], [1., 0., 0.], [0., 1., 0.]];
        let tile = TileGeometry {
            key: TileKey {
                first: 3,
                end: 9,
                depth: 2,
                leaf_begin: 1,
                leaf_end: 3,
            },
            positions: p.to_vec(),
            ells: vec![2; 3],
            triangles: vec![[0, 1, 2]],
            representative: vec![0],
            primitives: vec![SourcePrimitive {
                node: 7,
                mesh: 2,
                primitive: 1,
                expanded_ordinal: u64::MAX - 1,
                material: 4,
            }],
            interface: vec![InterfaceRow::new(1, key(p[0]), key(p[1]), 1, 3)],
            geometric_error: 0.125,
            lease: None,
        };
        let disk = manager.spill(tile).unwrap();
        assert_eq!(*calls.lock().unwrap(), [8192]);
        let loaded = disk.load().unwrap();
        assert_eq!(loaded.positions, p);
        assert_eq!(loaded.triangles, [[0, 1, 2]]);
        assert_eq!(loaded.primitives[0].expanded_ordinal, u64::MAX - 1);
        assert_eq!(loaded.interface[0].a, key(p[0]));
        assert_eq!(loaded.geometric_error, 0.125);
        assert_eq!(manager.stats.tiles, 1);
        assert!(manager.stats.peak_bytes > 0);
        drop(disk);
        assert_eq!(manager.live.load(Ordering::Relaxed), 0);
    }

    #[test]
    fn billion_scale_sibling_storage_depends_on_height_and_cap() {
        let at = |n| bound(n, n / 8192, 8192, 8192, 32, 4096).unwrap();
        assert_eq!(at(500_000_000), at(1_000_000_000));
        assert!(at(1_000_000_000) < 1 << 30);
        assert_eq!(limit(32), 296);
    }
}
