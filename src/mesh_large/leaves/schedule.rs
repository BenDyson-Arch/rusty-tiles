//! Shared delivery services for parallel leaf encoding: a byte-budget memory
//! gate that every page acquires before it allocates its workspace, and a
//! bounded least-recently-used cache of decoded source images. Neither changes
//! output bytes: work order and thread count only affect timing.
use crate::{JobError, JobErrorKind};
use std::{
    collections::HashMap,
    sync::{Arc, Condvar, Mutex, MutexGuard, OnceLock},
};

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
    pub(super) fn total(&self) -> u64 {
        self.total
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
type Cell = Arc<OnceLock<Result<Arc<Decoded>, JobError>>>;
struct Entry {
    cell: Cell,
    last_use: u64,
    bytes: u64,
}
#[derive(Default)]
struct CacheState {
    entries: HashMap<(usize, u32), Entry>,
    clock: u64,
    resident: u64,
    peak: u64,
    hits: u64,
    misses: u64,
}
/// Decoded sources keyed by (image, density drop). Concurrent requests for one
/// key share a single decode. Entries beyond `budget` bytes are evicted
/// least-recently-used first; callers keep evicted images alive only while
/// they still hold them.
pub(super) struct SourceCache {
    budget: u64,
    state: Mutex<CacheState>,
}
#[derive(Clone, Copy, Debug, serde::Serialize)]
pub(super) struct CacheStats {
    pub budget_bytes: u64,
    pub peak_resident_bytes: u64,
    pub hits: u64,
    pub misses: u64,
}
impl SourceCache {
    pub(super) fn new(budget: u64) -> Self {
        Self {
            budget,
            state: Mutex::new(CacheState::default()),
        }
    }
    pub(super) fn stats(&self) -> CacheStats {
        let state = lock(&self.state);
        CacheStats {
            budget_bytes: self.budget,
            peak_resident_bytes: state.peak,
            hits: state.hits,
            misses: state.misses,
        }
    }
    pub(super) fn get(
        &self,
        key: (usize, u32),
        load: impl FnOnce() -> Result<Decoded, JobError>,
    ) -> Result<Arc<Decoded>, JobError> {
        let cell = {
            let mut state = lock(&self.state);
            state.clock += 1;
            let clock = state.clock;
            let mut hit = true;
            let cell = state
                .entries
                .entry(key)
                .or_insert_with(|| {
                    hit = false;
                    Entry {
                        cell: Arc::new(OnceLock::new()),
                        last_use: clock,
                        bytes: 0,
                    }
                })
                .cell
                .clone();
            if let Some(entry) = state.entries.get_mut(&key) {
                entry.last_use = clock;
            }
            if hit {
                state.hits += 1;
            } else {
                state.misses += 1;
            }
            cell
        };
        let result = cell.get_or_init(|| load().map(Arc::new)).clone();
        let mut state = lock(&self.state);
        match &result {
            Err(_) => {
                // Never retain a failure; a later request decodes again.
                if state
                    .entries
                    .get(&key)
                    .is_some_and(|entry| Arc::ptr_eq(&entry.cell, &cell))
                {
                    let removed = state.entries.remove(&key).map_or(0, |e| e.bytes);
                    state.resident -= removed;
                }
            }
            Ok(decoded) => {
                let bytes = decoded.rgba.len() as u64;
                if let Some(entry) = state.entries.get_mut(&key) {
                    if entry.bytes == 0 && Arc::ptr_eq(&entry.cell, &cell) {
                        entry.bytes = bytes;
                        state.resident += bytes;
                    }
                }
                state.peak = state.peak.max(state.resident);
                while state.resident > self.budget {
                    let Some(oldest) = state
                        .entries
                        .iter()
                        .filter(|(k, e)| **k != key && e.bytes > 0)
                        .min_by_key(|(k, e)| (e.last_use, **k))
                        .map(|(k, _)| *k)
                    else {
                        break;
                    };
                    let removed = state.entries.remove(&oldest).map_or(0, |e| e.bytes);
                    state.resident -= removed;
                }
            }
        }
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    #[test]
    fn cache_shares_one_decode_and_evicts_least_recent() {
        let cache = SourceCache::new(8);
        let loads = AtomicUsize::new(0);
        let load = |n: u8| {
            loads.fetch_add(1, Ordering::SeqCst);
            Ok(Decoded {
                size: [1, 1],
                rgba: vec![n; 4],
            })
        };
        std::thread::scope(|scope| {
            for _ in 0..8 {
                scope.spawn(|| cache.get((0, 0), || load(1)).unwrap());
            }
        });
        assert_eq!(loads.load(Ordering::SeqCst), 1);
        cache.get((1, 0), || load(2)).unwrap();
        cache.get((0, 0), || load(9)).unwrap();
        // Budget 8 holds two 4-byte images: (1,0) is now least recent.
        cache.get((2, 0), || load(3)).unwrap();
        assert_eq!(cache.get((0, 0), || load(9)).unwrap().rgba, [1; 4]);
        assert_eq!(cache.get((1, 0), || load(7)).unwrap().rgba, [7; 4]);
        assert!(cache.stats().peak_resident_bytes <= 12);
        let failed = cache.get((5, 0), || {
            Err(JobError::new(JobErrorKind::Cancelled, "cancelled"))
        });
        assert!(failed.is_err());
        assert_eq!(cache.get((5, 0), || load(5)).unwrap().rgba, [5; 4]);
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
