//! Shared delivery services for parallel leaf encoding: a byte-budget memory
//! gate that every page acquires before it allocates its workspace, and a
//! immutable image table backed by resident rasters or a bounded texel-tile cache. Neither changes
//! output bytes: work order and thread count only affect timing.
use crate::{JobError, JobErrorKind};
use std::sync::{Arc, Condvar, Mutex, MutexGuard, OnceLock};

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// Counting byte semaphore. A request larger than the whole budget is refused
/// up front; otherwise it waits until enough bytes are free.
pub(super) struct MemoryGate {
    total: u64,
    state: Mutex<GateState>,
    freed: Condvar,
}
struct GateState {
    free: u64,
    peak: u64,
}
pub(super) struct Permit<'a> {
    gate: &'a MemoryGate,
    bytes: u64,
}
impl MemoryGate {
    pub(super) fn new(total: u64) -> Self {
        Self {
            total,
            state: Mutex::new(GateState {
                free: total,
                peak: 0,
            }),
            freed: Condvar::new(),
        }
    }
    /// Highest number of simultaneously leased bytes.
    pub(super) fn peak(&self) -> u64 {
        lock(&self.state).peak
    }
    pub(super) fn acquire(&self, bytes: u64) -> Result<Permit<'_>, JobError> {
        if bytes > self.total {
            return Err(JobError::new(
                JobErrorKind::ResourceLimit,
                "one leaf page workspace cannot fit available memory",
            ));
        }
        let mut state = lock(&self.state);
        while state.free < bytes {
            state = self
                .freed
                .wait(state)
                .unwrap_or_else(|poisoned| poisoned.into_inner());
        }
        state.free -= bytes;
        state.peak = state.peak.max(self.total - state.free);
        Ok(Permit { gate: self, bytes })
    }
}
impl Drop for Permit<'_> {
    fn drop(&mut self) {
        let mut state = lock(&self.gate.state);
        state.free += self.bytes;
        drop(state);
        self.gate.freed.notify_all();
    }
}

/// One decoded RGBA8 source image (or a density-reduced level of it).
pub(super) struct Decoded {
    pub size: [u32; 2],
    pub rgba: Vec<u8>,
}
pub(super) struct Stored {
    pub size: [u32; 2],
    pub rgba: crate::mesh_large::image_tiles::Raster,
}
type Cell = Arc<OnceLock<Result<Arc<Stored>, JobError>>>;
/// A density chain's reduced levels, or `None` when it could not be stored (memory-
/// backed scratch without room): each request then rebuilds it, as develop did after
/// an eviction (a mip build from the stored source, never a decode).
type ChainCell = Arc<OnceLock<Result<Option<Vec<Arc<Stored>>>, JobError>>>;
#[derive(Default)]
struct CacheState {
    entries: std::collections::BTreeMap<(usize, u32), Cell>,
    chains: std::collections::BTreeMap<(usize, u32), ChainCell>,
    hits: u64,
    misses: u64,
    decodes: u64,
}
pub(super) struct SourceCache {
    budget: u64,
    store: crate::mesh_large::image_tiles::Store,
    state: Mutex<CacheState>,
}
#[derive(Clone, Copy, Debug, serde::Serialize)]
pub(super) struct CacheStats {
    pub budget_bytes: u64,
    pub peak_resident_bytes: u64,
    pub hits: u64,
    pub misses: u64,
    pub scratch_bytes: u64,
    pub decode_count: u64,
}
impl SourceCache {
    /// `primary` bounds the source images, which take residency before density
    /// chains; `bound` covers both.
    pub(super) fn new(budget: u64, bound: u64, primary: u64, spill_density: bool) -> Self {
        let store = crate::mesh_large::image_tiles::Store::with_primary(budget, bound, primary);
        Self {
            budget,
            store: if spill_density {
                store
            } else {
                store.without_secondary_spill()
            },
            state: Mutex::new(CacheState::default()),
        }
    }
    /// Bytes the image store may spill to scratch (zero when every image fits).
    pub(super) fn spill_bound(&self) -> u64 {
        self.store.spill_bound()
    }
    pub(super) fn stats(&self) -> CacheStats {
        let s = lock(&self.state);
        CacheStats {
            budget_bytes: self.budget,
            peak_resident_bytes: self.store.peak(),
            hits: s.hits,
            misses: s.misses,
            scratch_bytes: self.store.scratch_bytes(),
            decode_count: s.decodes,
        }
    }
    pub(super) fn get(
        &self,
        scratch: &crate::spill::Scratch,
        key: (usize, u32),
        load: impl FnOnce() -> Result<Decoded, JobError>,
    ) -> Result<Arc<Stored>, JobError> {
        let cell = {
            let mut s = lock(&self.state);
            if let Some(cell) = s.entries.get(&key).cloned() {
                s.hits += 1;
                cell
            } else {
                s.misses += 1;
                if key.1 == 0 {
                    s.decodes += 1;
                }
                let cell = Arc::new(OnceLock::new());
                s.entries.insert(key, cell.clone());
                cell
            }
        };
        cell.get_or_init(|| {
            let d = load()?;
            let level = super::mips::Level {
                width: d.size[0],
                height: d.size[1],
                rgba: d.rgba,
            };
            let rgba = self.store.put(scratch, vec![level])?.pop();
            Ok(Arc::new(Stored {
                size: d.size,
                rgba: rgba
                    .ok_or_else(|| JobError::new(JobErrorKind::InvalidState, "stored image"))?,
            }))
        })
        .clone()
    }
    /// Density variants have different semantic filters from HLOD. Build each
    /// complete class chain once, retaining every reduced level for later requests.
    pub(super) fn density(
        &self,
        scratch: &crate::spill::Scratch,
        key: (usize, u32),
        drop: u32,
        load: impl FnOnce() -> Result<Vec<super::mips::Level>, JobError>,
    ) -> Result<Arc<Stored>, JobError> {
        let cell = {
            let mut s = lock(&self.state);
            if let Some(cell) = s.chains.get(&key).cloned() {
                s.hits += 1;
                cell
            } else {
                s.misses += 1;
                let cell = Arc::new(OnceLock::new());
                s.chains.insert(key, cell.clone());
                cell
            }
        };
        let outside = || JobError::new(JobErrorKind::InvalidState, "density level outside chain");
        let mut load = Some(load);
        // Level 0 is the source image, already stored once under (image, 0).
        let mut reduced = || -> Result<Vec<super::mips::Level>, JobError> {
            Ok((load.take().ok_or_else(outside)?)()?
                .into_iter()
                .skip(1)
                .collect())
        };
        let mut handed = None;
        let entry = cell.get_or_init(|| {
            Ok(match self.store.put_secondary(scratch, reduced()?)? {
                Ok(rasters) => Some(
                    rasters
                        .into_iter()
                        .map(|rgba| {
                            Arc::new(Stored {
                                size: [rgba.width, rgba.height],
                                rgba,
                            })
                        })
                        .collect(),
                ),
                Err(levels) => {
                    handed = Some(levels);
                    None
                }
            })
        });
        let index = drop.saturating_sub(1) as usize;
        match entry.as_ref().map_err(Clone::clone)? {
            Some(levels) => levels.get(index).cloned().ok_or_else(outside),
            None => {
                let levels = match handed {
                    Some(levels) => levels,
                    None => reduced()?,
                };
                let level = levels.into_iter().nth(index).ok_or_else(outside)?;
                Ok(Arc::new(Stored {
                    size: [level.width, level.height],
                    rgba: crate::mesh_large::image_tiles::Raster::transient(level),
                }))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    #[test]
    fn source_cache_scale_decode_count_is_independent_of_access_order() {
        let scratch = crate::spill::Scratch::create(None, 0).unwrap();
        let cache = SourceCache::new(256, 32 * 256, 32 * 256, true);
        let decodes = AtomicUsize::new(0);
        for round in 0..8 {
            for image in 0..32 {
                let decoded = cache
                    .get(&scratch, (image, 0), || {
                        decodes.fetch_add(1, Ordering::SeqCst);
                        Ok(Decoded {
                            size: [8, 8],
                            rgba: vec![image as u8; 256],
                        })
                    })
                    .unwrap();
                assert_eq!(decoded.rgba.materialize().unwrap(), vec![image as u8; 256]);
            }
            assert!(
                decodes.load(Ordering::SeqCst) <= 32 * 2,
                "round {round}: {} decodes for 32 images",
                decodes.load(Ordering::SeqCst)
            );
        }
    }

    #[test]
    fn cache_shares_one_decode_and_retains_failures() {
        let scratch = crate::spill::Scratch::create(None, 0).unwrap();
        let cache = SourceCache::new(8, 12, 12, true);
        let loads = AtomicUsize::new(0);
        std::thread::scope(|scope| {
            for _ in 0..8 {
                scope.spawn(|| {
                    cache
                        .get(&scratch, (0, 0), || {
                            loads.fetch_add(1, Ordering::SeqCst);
                            Ok(Decoded {
                                size: [1, 1],
                                rgba: vec![1; 4],
                            })
                        })
                        .unwrap()
                });
            }
        });
        assert_eq!(loads.load(Ordering::SeqCst), 1);
        for i in 1..3 {
            cache
                .get(&scratch, (i, 0), || {
                    Ok(Decoded {
                        size: [1, 1],
                        rgba: vec![i as u8; 4],
                    })
                })
                .unwrap();
        }
        assert_eq!(
            cache
                .get(&scratch, (0, 0), || panic!("redecode"))
                .unwrap()
                .rgba
                .materialize()
                .unwrap(),
            [1; 4]
        );
        assert!(cache
            .get(&scratch, (5, 0), || Err(JobError::new(
                JobErrorKind::Cancelled,
                "cancelled"
            )))
            .is_err());
        assert!(cache
            .get(&scratch, (5, 0), || panic!(
                "failed identities must not restart"
            ))
            .is_err());
    }
    #[test]
    fn density_requests_build_a_class_chain_once() {
        let scratch = crate::spill::Scratch::create(None, 0).unwrap();
        let cache = SourceCache::new(16, 84, 0, true);
        let builds = AtomicUsize::new(0);
        for drop in [3, 1, 2, 1, 3, 2] {
            let level = cache
                .density(&scratch, (0, 0), drop, || {
                    builds.fetch_add(1, Ordering::SeqCst);
                    Ok((0..4)
                        .map(|i| super::super::mips::Level {
                            width: 8 >> i,
                            height: 8 >> i,
                            rgba: vec![i as u8; (8usize >> i) * (8usize >> i) * 4],
                        })
                        .collect())
                })
                .unwrap();
            assert_eq!(
                level.rgba.materialize().unwrap(),
                vec![drop as u8; (8usize >> drop) * (8usize >> drop) * 4]
            );
        }
        assert_eq!(builds.load(Ordering::SeqCst), 1);
    }

    /// Memory-backed scratch without room: density chains are rebuilt per request
    /// (never spilled) and serve the same levels.
    #[test]
    fn unstored_density_chains_are_rebuilt_not_spilled() {
        let scratch = crate::spill::Scratch::create(None, 0).unwrap();
        let cache = SourceCache::new(16, 84, 0, false);
        let builds = AtomicUsize::new(0);
        for drop in [3, 1, 2, 1] {
            let level = cache
                .density(&scratch, (0, 0), drop, || {
                    builds.fetch_add(1, Ordering::SeqCst);
                    Ok((0..4)
                        .map(|i| super::super::mips::Level {
                            width: 8 >> i,
                            height: 8 >> i,
                            rgba: vec![i as u8; (8usize >> i) * (8usize >> i) * 4],
                        })
                        .collect())
                })
                .unwrap();
            assert_eq!(
                level.rgba.materialize().unwrap(),
                vec![drop as u8; (8usize >> drop) * (8usize >> drop) * 4]
            );
        }
        assert_eq!(builds.load(Ordering::SeqCst), 4);
        assert_eq!(cache.stats().scratch_bytes, 0);
        assert_eq!(std::fs::read_dir(scratch.path()).unwrap().count(), 0);
    }

    #[test]
    fn gate_refuses_oversized_and_blocks_until_release() {
        let gate = MemoryGate::new(10);
        assert_eq!(
            gate.acquire(11).err().unwrap().kind(),
            JobErrorKind::ResourceLimit
        );
        let first = gate.acquire(6).unwrap();
        std::thread::scope(|scope| {
            let waiter = scope.spawn(|| {
                let _second = gate.acquire(6).unwrap();
            });
            std::thread::sleep(std::time::Duration::from_millis(20));
            assert!(!waiter.is_finished());
            drop(first);
            waiter.join().unwrap();
        });
        assert_eq!(gate.peak(), 6);
    }
}
