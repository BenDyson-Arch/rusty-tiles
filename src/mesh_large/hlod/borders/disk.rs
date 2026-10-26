//! Fixed-record, positioned graph storage. Only small records are resident; source-sized
//! owner runs, adjacency, level maps and contract lists belong to admitted scratch.
use crate::{
    spill::{Record, Scratch, SlotFile},
    JobError,
};

pub(super) const NONE: u64 = u64::MAX;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(super) struct Words<const N: usize>(pub [u64; N]);
impl<const N: usize> Record for Words<N> {
    const SIZE: usize = N * 8;
    fn encode(&self, bytes: &mut [u8]) {
        for (v, b) in self.0.iter().zip(bytes.as_chunks_mut::<8>().0) {
            b.copy_from_slice(&v.to_le_bytes());
        }
    }
    fn decode(bytes: &[u8]) -> Self {
        Self(std::array::from_fn(|i| {
            u64::from_le_bytes(bytes[i * 8..i * 8 + 8].try_into().unwrap())
        }))
    }
}

const PAGE_BYTES: usize = 16 << 10;
const PAGES: usize = 64;
const WAYS: usize = 4;
/// Every store has at most 64 pages plus fixed metadata. Twelve stores overlap in the
/// graph build; eight survive for the walk. This never grows with source size.
pub(super) const CACHE_BYTES: u64 = (PAGE_BYTES * PAGES + 16_384) as u64;
struct Page {
    id: u64,
    used: u64,
    valid: usize,
    dirty: bool,
    bytes: Vec<u8>,
}
struct Storage {
    slots: SlotFile,
    pages: Vec<Option<Page>>,
    clock: u64,
}
impl Storage {
    fn page(&mut self, id: u64, width: usize) -> Result<&mut Page, JobError> {
        self.clock = self.clock.wrapping_add(1);
        let mut hash = id.wrapping_add(0x9e3779b97f4a7c15);
        hash = (hash ^ (hash >> 30)).wrapping_mul(0xbf58476d1ce4e5b9);
        hash = (hash ^ (hash >> 27)).wrapping_mul(0x94d049bb133111eb);
        hash ^= hash >> 31;
        let start = (hash as usize % (PAGES / WAYS)) * WAYS;
        let slots_per_page = PAGE_BYTES / width;
        let index = (start..start + WAYS)
            .find(|&i| self.pages[i].as_ref().is_some_and(|p| p.id == id))
            .unwrap_or_else(|| {
                (start..start + WAYS)
                    .min_by_key(|&i| self.pages[i].as_ref().map_or(0, |p| p.used))
                    .unwrap()
            });
        if !self.pages[index].as_ref().is_some_and(|p| p.id == id) {
            let mut bytes = if let Some(page) = self.pages[index].take() {
                if page.dirty {
                    self.slots
                        .write_many(page.id * slots_per_page as u64, &page.bytes[..page.valid])?;
                }
                page.bytes
            } else {
                vec![0; slots_per_page * width]
            };
            bytes.fill(0);
            let first = id * slots_per_page as u64;
            let count = self
                .slots
                .len()
                .saturating_sub(first)
                .min(slots_per_page as u64) as usize;
            let valid = count * width;
            if valid > 0 {
                self.slots.read_many(first, &mut bytes[..valid])?;
            }
            self.pages[index] = Some(Page {
                id,
                used: 0,
                valid,
                dirty: false,
                bytes,
            });
        }
        let page = self.pages[index].as_mut().unwrap();
        page.used = self.clock;
        Ok(page)
    }
    fn flush(&mut self, width: usize) -> Result<(), JobError> {
        for page in self.pages.iter_mut().flatten() {
            if page.dirty {
                self.slots.write_many(
                    page.id * (PAGE_BYTES / width) as u64,
                    &page.bytes[..page.valid],
                )?;
                page.dirty = false;
            }
        }
        Ok(())
    }
}
pub(super) struct Disk<const N: usize> {
    storage: std::sync::Mutex<Storage>,
    pub(super) len: u64,
}
impl<const N: usize> Disk<N> {
    pub(super) fn new(scratch: &Scratch, name: &str) -> Result<Self, JobError> {
        assert!(N > 0 && N <= 33);
        Ok(Self {
            storage: std::sync::Mutex::new(Storage {
                slots: SlotFile::new(scratch, name, N * 8)?,
                pages: (0..PAGES).map(|_| None).collect(),
                clock: 0,
            }),
            len: 0,
        })
    }
    pub(super) fn get(&self, index: u64) -> Result<[u64; N], JobError> {
        if index >= self.len {
            return Err(super::curve::state("graph slot is out of range"));
        }
        let width = N * 8;
        let count = (PAGE_BYTES / width) as u64;
        let mut storage = self.storage.lock().unwrap_or_else(|e| e.into_inner());
        let page = storage.page(index / count, width)?;
        let at = (index % count) as usize * width;
        Ok(Words::<N>::decode(&page.bytes[at..at + width]).0)
    }
    pub(super) fn set(&mut self, index: u64, record: [u64; N]) -> Result<(), JobError> {
        let width = N * 8;
        let count = (PAGE_BYTES / width) as u64;
        let page = self
            .storage
            .get_mut()
            .unwrap_or_else(|e| e.into_inner())
            .page(index / count, width)?;
        let at = (index % count) as usize * width;
        Words(record).encode(&mut page.bytes[at..at + width]);
        page.valid = page.valid.max(at + width);
        page.dirty = true;
        self.len = self.len.max(index + 1);
        Ok(())
    }
    pub(super) fn push(&mut self, record: [u64; N]) -> Result<u64, JobError> {
        let i = self.len;
        self.set(i, record)?;
        Ok(i)
    }
    pub(super) fn flush(&self) -> Result<(), JobError> {
        self.storage
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .flush(N * 8)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn bounded_pages_round_trip_updates_evictions_and_sparse_records() {
        let work = tempfile::tempdir().unwrap();
        let scratch = Scratch::create(Some(work.path()), 0).unwrap();
        let mut disk = Disk::<3>::new(&scratch, "pages").unwrap();
        let count = 100_000u64;
        for i in 0..count {
            disk.push([i, i * 7, i * 13]).unwrap();
        }
        for i in (0..count).step_by(17) {
            disk.set(i, [i, 99, 101]).unwrap();
        }
        disk.flush().unwrap();
        for i in (0..count).rev() {
            assert_eq!(
                disk.get(i).unwrap(),
                if i % 17 == 0 {
                    [i, 99, 101]
                } else {
                    [i, i * 7, i * 13]
                }
            );
        }
        disk.set(count + 1000, [1, 2, 3]).unwrap();
        disk.flush().unwrap();
        assert_eq!(disk.get(count + 1).unwrap(), [0; 3]);
        assert!(disk.get(count + 1001).is_err());
        let storage = disk.storage.lock().unwrap();
        assert!(
            storage
                .pages
                .iter()
                .flatten()
                .map(|p| p.bytes.capacity())
                .sum::<usize>() as u64
                <= CACHE_BYTES
        );
    }
}
