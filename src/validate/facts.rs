//! Per-URI facts at archive scale (VS-B-5).
//!
//! The first inspection of a payload writes one fixed-width record, keyed by the member's
//! catalog rank: its C2 facts, its vertex count and its C1 replay receipt (H23). Every later
//! reference reads it back. The store is chosen once from the entry count, like the catalog:
//! rank-indexed pages in memory when every record fits [`CatalogConfig::facts_resident_bytes`],
//! otherwise a rank-indexed scratch file read and written one record at a time.
//!
//! The receipt is trusted only because this validation wrote it after a successful full
//! admission and reads it back only for a rank whose inspected bit it set; the store lives
//! for one validation. Receipts never come from archive bytes, reports or metadata.
use super::containment::{Bound, Pair, ReferenceFacts, UriBounds};
use super::{refused, spill_failure, Bits, Error, UriFacts};
use crate::archive3tz::CatalogConfig;
use crate::content_integrity::payload::ReplayAdmission;
use crate::spill::{Scratch, SlotFile};

/// Encoded record width: 131 bytes of facts, the 64-byte replay receipt and a CRC-32 of both.
pub(super) const WIDTH: usize = 199;
/// Bytes covered by the record's CRC-32.
const BODY: usize = WIDTH - 4;
/// Records per resident page.
const PAGE: u64 = 64;

/// One payload URI's facts as every later reference needs them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Stored {
    pub facts: UriFacts,
    /// The C1 vertex count of the payload, for the per-tile budgets.
    pub vertices: u64,
    /// The first admission's receipt, for the phase-2 replay (H23).
    pub replay: ReplayAdmission,
}

fn encode(stored: &Stored, out: &mut [u8; WIDTH]) {
    let f = &stored.facts;
    let mut flags = 0u8;
    let mut at = 1;
    let mut put = |out: &mut [u8; WIDTH], bytes: &[u8]| {
        out[at..at + bytes.len()].copy_from_slice(bytes);
        at += bytes.len();
    };
    match &f.bounds {
        Some(bounds) => {
            flags |= 1;
            for (i, bound) in bounds.pair0.iter().enumerate() {
                let Pair(lo, hi) = match bound {
                    Bound::Zero => {
                        flags |= 4 << i;
                        Pair(0, 0)
                    }
                    Bound::P(pair) => *pair,
                };
                put(out, &lo.to_le_bytes());
                put(out, &hi.to_le_bytes());
            }
            put(out, &bounds.s_inst.to_le_bytes());
            put(out, &bounds.s_pos.to_le_bytes());
        }
        None => put(out, &[0; 40]),
    }
    if f.nodes.is_some() {
        flags |= 2;
    }
    let r = &f.per_reference;
    for word in [
        f.t,
        f.nodes.unwrap_or(0),
        f.idx,
        f.e,
        r.b,
        r.dec,
        r.json_nodes,
        r.external_resources,
        r.instance_primitives,
        f.max_depth,
    ] {
        put(out, &word.to_le_bytes());
    }
    put(out, &f.exclusions.to_le_bytes());
    put(out, &stored.vertices.to_le_bytes());
    put(out, &stored.replay.to_stored_bytes());
    debug_assert_eq!(at, BODY);
    out[0] = flags;
    let crc = crc32fast::hash(&out[..BODY]);
    out[BODY..].copy_from_slice(&crc.to_le_bytes());
}

/// A record whose CRC-32 does not match is corrupt scratch, never a verdict.
fn checked(bytes: &[u8; WIDTH], rank: u64) -> Result<Stored, Error> {
    let crc = u32::from_le_bytes(bytes[BODY..].try_into().unwrap());
    if crc32fast::hash(&bytes[..BODY]) != crc {
        return Err(Error::InvalidState(format!(
            "validation scratch: payload facts record {rank} is corrupt"
        )));
    }
    Ok(decode(bytes))
}

fn decode(bytes: &[u8; WIDTH]) -> Stored {
    let flags = bytes[0];
    let i32_at = |at: usize| i32::from_le_bytes(bytes[at..at + 4].try_into().unwrap());
    let u32_at = |at: usize| u32::from_le_bytes(bytes[at..at + 4].try_into().unwrap());
    let u64_at = |at: usize| u64::from_le_bytes(bytes[at..at + 8].try_into().unwrap());
    let bounds = (flags & 1 != 0).then(|| UriBounds {
        pair0: std::array::from_fn(|i| {
            if flags & (4 << i) != 0 {
                Bound::Zero
            } else {
                Bound::P(Pair(i32_at(1 + 8 * i), i32_at(5 + 8 * i)))
            }
        }),
        s_inst: u32_at(33),
        s_pos: u32_at(37),
    });
    let word = |k: usize| u64_at(41 + 8 * k);
    Stored {
        facts: UriFacts {
            bounds,
            t: word(0),
            nodes: (flags & 2 != 0).then(|| word(1)),
            idx: word(2),
            e: word(3),
            per_reference: ReferenceFacts {
                b: word(4),
                dec: word(5),
                json_nodes: word(6),
                external_resources: word(7),
                instance_primitives: word(8),
            },
            max_depth: word(9),
            exclusions: u16::from_le_bytes([bytes[121], bytes[122]]),
        },
        vertices: u64_at(123),
        // Trusted: this validation's own record, guarded by the inspected bit.
        replay: ReplayAdmission::from_stored_bytes(bytes[131..BODY].try_into().unwrap()),
    }
}

enum Slots {
    /// `⌈N/64⌉` page slots, each allocated at its first record.
    Resident(Vec<Option<Box<[u8]>>>),
    Spilled(SlotFile),
}

/// The inspected bit per rank and the records behind it.
pub(super) struct FactStore {
    inspected: Bits,
    slots: Option<Slots>,
    entries: u64,
    resident: bool,
}

impl FactStore {
    /// The store for `entries` members; nothing is allocated or created before the first
    /// record.
    pub(super) fn new(entries: u64, config: &CatalogConfig) -> Result<Self, Error> {
        Ok(Self {
            inspected: Bits::new(entries)?,
            slots: None,
            entries,
            resident: entries
                .checked_mul(WIDTH as u64)
                .is_some_and(|bytes| bytes <= config.facts_resident_bytes),
        })
    }

    pub(super) fn inspected(&self, rank: u64) -> bool {
        self.inspected.get(rank)
    }

    /// Records the first inspection of the member at `rank`.
    pub(super) fn insert(
        &mut self,
        rank: u64,
        stored: &Stored,
        scratch: &Scratch,
    ) -> Result<(), Error> {
        if self.inspected.get(rank) {
            return Err(Error::InvalidState(
                "payload facts recorded twice for one member".into(),
            ));
        }
        let mut record = [0u8; WIDTH];
        encode(stored, &mut record);
        let slots = match &mut self.slots {
            Some(slots) => slots,
            None => {
                let slots = if self.resident {
                    let pages = usize::try_from(self.entries.div_ceil(PAGE))
                        .map_err(|_| Error::limit("archive entry count exceeds host range"))?;
                    let mut table = Vec::new();
                    table.try_reserve_exact(pages).map_err(|_| refused())?;
                    table.resize_with(pages, || None);
                    Slots::Resident(table)
                } else {
                    Slots::Spilled(
                        SlotFile::new(scratch, "uri-facts", WIDTH).map_err(spill_failure)?,
                    )
                };
                self.slots.insert(slots)
            }
        };
        match slots {
            Slots::Resident(pages) => {
                let page = &mut pages[(rank / PAGE) as usize];
                if page.is_none() {
                    let mut bytes = Vec::new();
                    bytes
                        .try_reserve_exact(PAGE as usize * WIDTH)
                        .map_err(|_| refused())?;
                    bytes.resize(PAGE as usize * WIDTH, 0);
                    *page = Some(bytes.into_boxed_slice());
                }
                let at = (rank % PAGE) as usize * WIDTH;
                page.as_mut().unwrap()[at..at + WIDTH].copy_from_slice(&record);
            }
            Slots::Spilled(file) => file.write(rank, &record).map_err(spill_failure)?,
        }
        self.inspected.set(rank);
        Ok(())
    }

    /// The record of an inspected member.
    pub(super) fn get(&self, rank: u64) -> Result<Stored, Error> {
        if !self.inspected.get(rank) {
            return Err(Error::InvalidState(
                "payload facts read before the first inspection".into(),
            ));
        }
        let mut record = [0u8; WIDTH];
        match &self.slots {
            Some(Slots::Resident(pages)) => {
                let page = pages[(rank / PAGE) as usize]
                    .as_ref()
                    .ok_or_else(|| Error::InvalidState("payload facts page missing".into()))?;
                let at = (rank % PAGE) as usize * WIDTH;
                record.copy_from_slice(&page[at..at + WIDTH]);
            }
            Some(Slots::Spilled(file)) => file.read(rank, &mut record).map_err(spill_failure)?,
            None => {
                return Err(Error::InvalidState("payload facts store is empty".into()));
            }
        }
        checked(&record, rank)
    }
    /// Test seam: the scratch file of a spilled store, for corruption controls.
    #[cfg(test)]
    fn slot_file(&mut self) -> Option<&mut SlotFile> {
        match &mut self.slots {
            Some(Slots::Spilled(file)) => Some(file),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample(seed: u64) -> Stored {
        let bound = |k: u64| {
            if (seed >> k) & 1 == 0 {
                Bound::Zero
            } else {
                Bound::P(Pair(-(seed as i32) - k as i32, (seed as i32) ^ 0x5a5a))
            }
        };
        Stored {
            facts: UriFacts {
                bounds: (!seed.is_multiple_of(3)).then(|| UriBounds {
                    pair0: std::array::from_fn(|i| bound(i as u64)),
                    s_inst: seed as u32 ^ 7,
                    s_pos: u32::MAX - seed as u32,
                }),
                t: seed.wrapping_mul(0x9e37_79b9_7f4a_7c15),
                nodes: (!seed.is_multiple_of(5)).then_some(seed << 3),
                idx: !seed,
                e: seed + 1,
                per_reference: ReferenceFacts {
                    b: seed + 2,
                    dec: seed + 3,
                    json_nodes: seed + 4,
                    external_resources: seed + 5,
                    instance_primitives: seed + 6,
                },
                max_depth: u64::MAX - seed,
                exclusions: (seed as u16).rotate_left(3),
            },
            vertices: seed ^ u64::MAX,
            replay: ReplayAdmission::from_stored_bytes(std::array::from_fn(|i| {
                (seed as u8).wrapping_mul(31).wrapping_add(i as u8)
            })),
        }
    }

    #[test]
    fn records_round_trip_every_field() {
        for seed in 0..200u64 {
            let stored = sample(seed);
            let mut bytes = [0u8; WIDTH];
            encode(&stored, &mut bytes);
            assert_eq!(decode(&bytes), stored, "seed {seed}");
        }
    }

    #[test]
    fn resident_and_spilled_stores_agree() {
        let scratch = Scratch::deferred(
            crate::spill::Placement::In(
                std::env::var_os("TMPDIR")
                    .map_or_else(std::env::temp_dir, std::path::PathBuf::from),
            ),
            0,
        );
        let entries = 1000;
        let mut stores = [
            FactStore::new(entries, &CatalogConfig::DEFAULT).unwrap(),
            FactStore::new(entries, &CatalogConfig::FORCED_SPILL).unwrap(),
        ];
        assert!(stores[0].resident && !stores[1].resident);
        // Strided ranks plus a run of adjacent ones.
        let mut ranks: Vec<u64> = (0..entries)
            .map(|i| (i * 617) % entries)
            .step_by(3)
            .collect();
        ranks.extend(
            (500..520)
                .filter(|r| !ranks.contains(r))
                .collect::<Vec<_>>(),
        );
        for store in &mut stores {
            for &rank in &ranks {
                store.insert(rank, &sample(rank), &scratch).unwrap();
            }
            assert!(store.insert(ranks[0], &sample(0), &scratch).is_err());
            for rank in 0..entries {
                assert_eq!(store.inspected(rank), ranks.contains(&rank));
                if store.inspected(rank) {
                    assert_eq!(store.get(rank).unwrap(), sample(rank));
                } else {
                    assert!(matches!(store.get(rank), Err(Error::InvalidState(_))));
                }
            }
        }
        assert!(scratch.is_created(), "the forced profile spills");
        // Scratch accounting: the spilled file's logical extent is one record past the
        // highest rank written.
        let top = ranks.iter().max().unwrap() + 1;
        assert_eq!(scratch.peak_bytes(), top * WIDTH as u64);
        assert_eq!(scratch.live_bytes(), top * WIDTH as u64);
    }

    #[test]
    fn a_corrupt_record_is_a_typed_scratch_failure() {
        let mut bytes = [0u8; WIDTH];
        encode(&sample(7), &mut bytes);
        assert_eq!(checked(&bytes, 7).unwrap(), sample(7));
        for at in [0, 60, 130, 131, 194, WIDTH - 1] {
            let mut flipped = bytes;
            flipped[at] ^= 0x10;
            assert!(
                matches!(checked(&flipped, 7), Err(Error::InvalidState(m)) if m.contains("record 7 is corrupt")),
                "byte {at}"
            );
        }
        // A flipped byte in the spilled file itself.
        let dir = tempfile::tempdir().unwrap();
        let scratch = Scratch::deferred(crate::spill::Placement::In(dir.path().into()), 0);
        let mut store = FactStore::new(16, &CatalogConfig::FORCED_SPILL).unwrap();
        store.insert(5, &sample(5), &scratch).unwrap();
        let file = store.slot_file().unwrap();
        let mut record = [0u8; WIDTH];
        file.read(5, &mut record).unwrap();
        record[100] ^= 1;
        file.write(5, &record).unwrap();
        assert!(matches!(store.get(5), Err(Error::InvalidState(_))));
    }

    #[test]
    fn the_scratch_estimate_counts_the_facts_file() {
        assert_eq!(crate::archive3tz::FACTS_RECORD_BYTES, WIDTH as u64);
        let base = CatalogConfig::scratch_estimate(0, 0);
        assert!(CatalogConfig::scratch_estimate(1000, 0) - base >= 1000 * WIDTH as u64);
    }

    #[test]
    fn default_profile_keeps_the_default_entry_limit_resident() {
        let limit = crate::validate::ValidationLimits::default().archive_entries;
        assert!(
            FactStore::new(limit, &CatalogConfig::DEFAULT)
                .unwrap()
                .resident
        );
        let over = CatalogConfig::DEFAULT.facts_resident_bytes / WIDTH as u64 + 1;
        assert!(
            !FactStore::new(over, &CatalogConfig::DEFAULT)
                .unwrap()
                .resident
        );
    }
}
