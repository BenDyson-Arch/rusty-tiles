//! Order-preserving parallel geometry replay (#342).
//!
//! A replay pass reads placements from one ordered query and replays each payload's surface.
//! With more than one worker, a second copy of the same query runs ahead of it, uncharged (its
//! SQLite operations are not work, [`super::ticks::Ticks::uncharged`]): each placement it
//! yields is fetched and replayed on a worker thread, a bounded window ahead. The pass itself
//! is unchanged: it consumes the placements of its own query in order and makes every charge
//! the sequential replay makes, at the same point:
//!
//! - the replay's own unit and the read's admission (member cap, `totalBytesRead`), in order;
//! - a worker's surface is used only when the sequential replay would have succeeded, which is
//!   exactly when its work units fit the budget from the units already charged (charges are
//!   nonnegative, so the last one is the largest); its units are then charged at once, as the
//!   sequential replay charges them at its end;
//! - any other outcome (a worker error, a lookahead that disagrees with the pass's own row, an
//!   unavailable lookahead) replays sequentially there, so the error is the sequential one.
//!
//! Work counts, refusals and reports are therefore those of one worker for any worker count.
use super::{replay_surface, replay_surface_free, sql, Effort, Error, Store, Surface};
#[cfg(test)]
use std::sync::Arc;
use std::{collections::VecDeque, sync::mpsc};

/// Bytes of one member, read and CRC-checked on a worker.
pub(crate) type Fetch = Box<dyn FnOnce() -> Result<Vec<u8>, Error> + Send>;

/// The payload reads of a replay: in-order admission, sequential fetch, and lookahead.
pub(crate) trait Payloads {
    /// Every check and charge the read makes before it touches bytes, in order.
    fn admit(&mut self, rank: u64) -> Result<(), Error>;
    /// The admitted member's bytes.
    fn fetch(&mut self, rank: u64) -> Result<Vec<u8>, Error>;
    /// A worker fetch of `rank` and its stored size, for lookahead only: no charge, and any
    /// failure is `None` (the pass then reads sequentially and meets it in order).
    fn locate(&mut self, rank: u64) -> Option<(u64, Fetch)>;
}

/// A plain read closure as [`Payloads`] (unit tests).
#[cfg(test)]
pub(super) struct Plain<'a>(pub(super) &'a mut dyn FnMut(u64) -> Result<Vec<u8>, Error>);
#[cfg(test)]
impl Payloads for Plain<'_> {
    fn admit(&mut self, _: u64) -> Result<(), Error> {
        Ok(())
    }
    fn fetch(&mut self, rank: u64) -> Result<Vec<u8>, Error> {
        (self.0)(rank)
    }
    fn locate(&mut self, _: u64) -> Option<(u64, Fetch)> {
        None
    }
}

/// The replay workers of one delivery stage, started at the first parallel pass. A pool that
/// cannot start leaves every replay sequential, which gives the same result.
pub(super) struct Pool {
    threads: usize,
    pool: std::sync::OnceLock<Option<rayon::ThreadPool>>,
    /// Placements in flight at most, and their stored bytes at most (at least one).
    items: usize,
    window_bytes: u64,
}

impl Pool {
    /// `workers` replay threads besides the pass's own; none for one worker.
    pub(super) fn new(workers: usize, window_bytes: u64) -> Self {
        let threads = workers.saturating_sub(1);
        Self {
            threads,
            pool: std::sync::OnceLock::new(),
            items: threads.saturating_mul(4),
            window_bytes,
        }
    }

    fn threads(&self) -> Option<&rayon::ThreadPool> {
        if self.threads == 0 {
            return None;
        }
        self.pool
            .get_or_init(|| {
                rayon::ThreadPoolBuilder::new()
                    .num_threads(self.threads)
                    .thread_name(|i| format!("rt-validate-{i}"))
                    .build()
                    .ok()
            })
            .as_ref()
    }

    pub(super) fn parallel(&self) -> bool {
        self.threads().is_some()
    }
}

type Outcome = Result<(Surface, u64), Error>;

struct Pending {
    rank: i64,
    frame: Vec<u8>,
    bytes: u64,
    outcome: Option<mpsc::Receiver<Outcome>>,
}

/// The lookahead of one pass over `rows`, a second execution of the pass's query.
pub(super) struct Lookahead<'p, 'r, 's> {
    pool: &'p Pool,
    rows: Option<&'r mut rusqlite::Rows<'s>>,
    /// Columns of the rank and frame in `rows`.
    columns: (usize, usize),
    queue: VecDeque<Pending>,
    in_flight: u64,
    cap: usize,
    budget: super::Budget,
    /// Whether this pass compares coarse NORMALs (#371): the coarse pass only.
    normals: bool,
}

impl<'p, 'r, 's> Lookahead<'p, 'r, 's> {
    /// `rows` is `None` without workers: every replay is then sequential.
    pub(super) fn new(
        store: &Store,
        pool: &'p Pool,
        rows: Option<&'r mut rusqlite::Rows<'s>>,
        columns: (usize, usize),
        effort: &Effort,
        normals: bool,
    ) -> Self {
        Self {
            pool,
            rows: rows.filter(|_| pool.parallel()),
            columns,
            queue: VecDeque::new(),
            in_flight: 0,
            cap: store.decoded_limit,
            budget: effort.budget,
            normals,
        }
    }

    /// Runs ahead, uncharged, until the window is full or the lookahead query ends.
    fn fill(&mut self, effort: &Effort, read: &mut dyn Payloads) {
        let pools: &'p Pool = self.pool;
        let Some(pool) = pools.threads() else {
            return;
        };
        while self.queue.len() < self.pool.items.max(1)
            && (self.queue.is_empty() || self.in_flight < self.pool.window_bytes)
        {
            let Some(rows) = self.rows.as_mut() else {
                return;
            };
            let (rank_column, frame_column) = self.columns;
            let next = effort.ticks.uncharged(|| {
                let row = rows.next().map_err(sql)?;
                row.map(|r| {
                    Ok::<_, Error>((
                        r.get::<_, i64>(rank_column).map_err(sql)?,
                        r.get::<_, Vec<u8>>(frame_column).map_err(sql)?,
                    ))
                })
                .transpose()
            });
            let Ok(Some((rank, frame))) = next else {
                // The end, or a lookahead error that the pass meets in order itself.
                self.rows = None;
                return;
            };
            let located = u64::try_from(rank).ok().and_then(|r| read.locate(r));
            let (bytes, outcome) = match (located, super::decode::<16>(&frame)) {
                (Some((bytes, fetch)), Ok(matrix)) => {
                    let (sender, receiver) = mpsc::sync_channel(1);
                    let (cap, budget, normals) = (self.cap, self.budget, self.normals);
                    pool.spawn(move || {
                        let outcome = fetch().and_then(|payload| {
                            replay_surface_free(&payload, matrix, cap, budget, normals)
                        });
                        let _ = sender.send(outcome);
                    });
                    (bytes, Some(receiver))
                }
                _ => (0, None),
            };
            self.in_flight = self.in_flight.saturating_add(bytes);
            self.queue.push_back(Pending {
                rank,
                frame,
                bytes,
                outcome,
            });
        }
    }

    /// The surface of the pass's next placement, with the sequential replay's charges.
    pub(super) fn surface(
        &mut self,
        rank: i64,
        frame: &[u8],
        effort: &Effort,
        read: &mut dyn Payloads,
    ) -> Result<Surface, Error> {
        self.fill(effort, read);
        let pending = self.queue.pop_front();
        if let Some(p) = &pending {
            self.in_flight = self.in_flight.saturating_sub(p.bytes);
        }
        let outcome = match pending {
            Some(p) if p.rank == rank && p.frame == frame => p.outcome,
            Some(_) => {
                // The two executions disagree: no further lookahead in this pass.
                self.queue.clear();
                self.rows = None;
                None
            }
            None => None,
        };
        effort.charge(1)?;
        let rank_u = u64::try_from(rank).map_err(|_| super::super::refused())?;
        read.admit(rank_u)?;
        if let Some(Ok((surface, units))) = outcome.and_then(|r| r.recv().ok()) {
            let base = effort
                .base
                .saturating_add(effort.used.load(std::sync::atomic::Ordering::Relaxed));
            if base
                .checked_add(units)
                .is_some_and(|total| total <= self.budget.value)
            {
                effort.charge(usize::try_from(units).map_err(|_| super::super::refused())?)?;
                return Ok(surface);
            }
        }
        let bytes = read.fetch(rank_u)?;
        replay_surface(
            &bytes,
            super::decode::<16>(frame)?,
            self.cap,
            effort,
            self.normals,
        )
    }
}

/// A shared payload list as [`Payloads`] with lookahead (unit tests of the parallel path).
#[cfg(test)]
pub(super) struct Shared(pub(super) Arc<Vec<Vec<u8>>>);
#[cfg(test)]
impl Payloads for Shared {
    fn admit(&mut self, _: u64) -> Result<(), Error> {
        Ok(())
    }
    fn fetch(&mut self, rank: u64) -> Result<Vec<u8>, Error> {
        Ok(self.0[rank as usize].clone())
    }
    fn locate(&mut self, rank: u64) -> Option<(u64, Fetch)> {
        let payloads = self.0.clone();
        let size = payloads[rank as usize].len() as u64;
        Some((size, Box::new(move || Ok(payloads[rank as usize].clone()))))
    }
}
