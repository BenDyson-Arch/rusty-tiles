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
        for (v, b) in self.0.iter().zip(bytes.chunks_exact_mut(8)) {
            b.copy_from_slice(&v.to_le_bytes());
        }
    }
    fn decode(bytes: &[u8]) -> Self {
        Self(std::array::from_fn(|i| {
            u64::from_le_bytes(bytes[i * 8..i * 8 + 8].try_into().unwrap())
        }))
    }
}

pub(super) struct Disk<const N: usize> {
    slots: SlotFile,
    pub(super) len: u64,
}
impl<const N: usize> Disk<N> {
    pub(super) fn new(scratch: &Scratch, name: &str) -> Result<Self, JobError> {
        assert!(N <= 33);
        Ok(Self {
            slots: SlotFile::new(scratch, name, N * 8)?,
            len: 0,
        })
    }
    pub(super) fn get(&self, index: u64) -> Result<[u64; N], JobError> {
        let mut bytes = [0u8; 33 * 8];
        self.slots.read(index, &mut bytes[..N * 8])?;
        Ok(Words::<N>::decode(&bytes[..N * 8]).0)
    }
    pub(super) fn set(&mut self, index: u64, record: [u64; N]) -> Result<(), JobError> {
        let mut bytes = [0u8; 33 * 8];
        Words(record).encode(&mut bytes[..N * 8]);
        self.slots.write(index, &bytes[..N * 8])?;
        self.len = self.len.max(index + 1);
        Ok(())
    }
    pub(super) fn push(&mut self, record: [u64; N]) -> Result<u64, JobError> {
        let index = self.len;
        self.set(index, record)?;
        Ok(index)
    }
}
