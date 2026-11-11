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
use std::{
    collections::VecDeque,
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc, Arc,
    },
};

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

/// Decoded workspace of one worker job: its decoded views and its surface are each capped at
/// this, below the document limit (review P2-6). A job that needs more fails on the worker and
/// the pass replays it sequentially under the full limit, with the same result.
pub(crate) const JOB_DECODE_BYTES: usize = 8 << 20;

/// The replay workers of one delivery stage, started at the first parallel pass. A pool that
/// cannot start leaves every replay sequential, which gives the same result.
pub(super) struct Pool {
    threads: usize,
    pool: std::sync::OnceLock<Option<rayon::ThreadPool>>,
    /// Placements in flight at most.
    items: usize,
    /// In-flight bytes at most: each job is charged its stored bytes plus twice
    /// [`JOB_DECODE_BYTES`] from dispatch until the pass consumes it. A job alone always runs.
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
                    // A panicking job only drops its sender: the pass replays that placement
                    // sequentially, where the same panic unwinds normally (review P2-4).
                    .panic_handler(|_| {})
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
    charge: u64,
    outcome: Option<mpsc::Receiver<Outcome>>,
}

/// A placement read ahead whose job does not fit the window yet.
struct Held {
    rank: i64,
    frame: Vec<u8>,
    located: Option<(u64, Fetch)>,
}

/// The lookahead of one pass over `rows`, a second execution of the pass's query.
pub(super) struct Lookahead<'p, 'r, 's> {
    pool: &'p Pool,
    rows: Option<&'r mut rusqlite::Rows<'s>>,
    /// Columns of the rank and frame in `rows`.
    columns: (usize, usize),
    queue: VecDeque<Pending>,
    held: Option<Held>,
    in_flight: u64,
    cap: usize,
    budget: super::Budget,
    /// Whether this pass compares coarse NORMALs (#371): the coarse pass only.
    normals: bool,
    /// Set when the pass ends, fails or is cancelled: jobs not yet started do nothing.
    stop: Arc<AtomicBool>,
}

impl Drop for Lookahead<'_, '_, '_> {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
    }
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
            held: None,
            in_flight: 0,
            cap: store.decoded_limit,
            budget: effort.budget,
            normals,
            stop: Arc::new(AtomicBool::new(false)),
        }
    }

    /// The next placement of the lookahead query, uncharged; `None` at its end or on a
    /// lookahead error (the pass meets that row in order itself).
    fn next_row(&mut self, effort: &Effort, read: &mut dyn Payloads) -> Option<Held> {
        if let Some(held) = self.held.take() {
            return Some(held);
        }
        let rows = self.rows.as_mut()?;
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
            self.rows = None;
            return None;
        };
        #[cfg(test)]
        if SKIP_AHEAD.with(|skip| skip.get() == Some(rank) && skip.take().is_some()) {
            return self.next_row(effort, read);
        }
        let located = u64::try_from(rank).ok().and_then(|r| read.locate(r));
        Some(Held {
            rank,
            frame,
            located,
        })
    }

    /// Runs ahead until the window or the item count is full or the lookahead query ends.
    fn fill(&mut self, effort: &Effort, read: &mut dyn Payloads) {
        let pools: &'p Pool = self.pool;
        let Some(pool) = pools.threads() else {
            return;
        };
        while self.queue.len() < self.pool.items.max(1) {
            let Some(next) = self.next_row(effort, read) else {
                return;
            };
            let decode = 2 * JOB_DECODE_BYTES as u64;
            let charge = next
                .located
                .as_ref()
                .map_or(0, |(stored, _)| stored.saturating_add(decode));
            if !self.queue.is_empty()
                && self.in_flight.saturating_add(charge) > self.pool.window_bytes
            {
                self.held = Some(next);
                return;
            }
            let Held {
                rank,
                frame,
                located,
            } = next;
            let outcome = match (located, super::decode::<16>(&frame)) {
                // A member larger than the whole window replays sequentially.
                (Some((stored, fetch)), Ok(matrix)) if stored <= self.pool.window_bytes => {
                    let (sender, receiver) = mpsc::sync_channel(1);
                    let (cap, budget, normals) =
                        (self.cap.min(JOB_DECODE_BYTES), self.budget, self.normals);
                    let stop = self.stop.clone();
                    pool.spawn(move || {
                        if stop.load(Ordering::Relaxed) {
                            return;
                        }
                        let outcome = fetch().and_then(|payload| {
                            if stop.load(Ordering::Relaxed) {
                                return Err(Error::Cancelled);
                            }
                            replay_surface_free(&payload, matrix, cap, budget, normals)
                        });
                        let _ = sender.send(outcome);
                    });
                    Some(receiver)
                }
                _ => None,
            };
            let charge = if outcome.is_some() { charge } else { 0 };
            self.in_flight = self.in_flight.saturating_add(charge);
            self.queue.push_back(Pending {
                rank,
                frame,
                charge,
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
        #[cfg(test)]
        BOUNDARIES.with(|b| {
            b.borrow_mut()
                .push(effort.base + effort.used.load(Ordering::Relaxed))
        });
        self.fill(effort, read);
        let pending = self.queue.pop_front();
        if let Some(p) = &pending {
            self.in_flight = self.in_flight.saturating_sub(p.charge);
        }
        let outcome = match pending {
            Some(p) if p.rank == rank && p.frame == frame => p.outcome,
            Some(_) => {
                // The two executions disagree: no further lookahead in this pass.
                self.desync();
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
                .saturating_add(effort.used.load(Ordering::Relaxed));
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

    fn desync(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        self.queue.clear();
        self.held = None;
        self.rows = None;
        self.in_flight = 0;
    }
}

#[cfg(test)]
thread_local! {
    /// Test seam: the lookahead skips its row of this rank once, so the two executions
    /// disagree from there.
    pub(super) static SKIP_AHEAD: std::cell::Cell<Option<i64>> =
        const { std::cell::Cell::new(None) };
    /// Work units charged when each replayed placement starts, in pass order.
    pub(super) static BOUNDARIES: std::cell::RefCell<Vec<u64>> =
        const { std::cell::RefCell::new(Vec::new()) };
}

/// A shared payload list as [`Payloads`] with lookahead (unit tests of the parallel path).
/// Worker fetches of `panic` ranks panic and of `corrupt` ranks fail; `refuse` makes the
/// in-order admission of that rank a budget refusal.
#[cfg(test)]
#[derive(Default)]
pub(super) struct Shared {
    pub(super) payloads: Arc<Vec<Vec<u8>>>,
    pub(super) panic: Vec<u64>,
    pub(super) corrupt: Vec<u64>,
    pub(super) refuse: Option<u64>,
}
#[cfg(test)]
impl Payloads for Shared {
    fn admit(&mut self, rank: u64) -> Result<(), Error> {
        if self.refuse == Some(rank) {
            return Err(Error::limit("member reads exceed work limit"));
        }
        Ok(())
    }
    fn fetch(&mut self, rank: u64) -> Result<Vec<u8>, Error> {
        Ok(self.payloads[rank as usize].clone())
    }
    fn locate(&mut self, rank: u64) -> Option<(u64, Fetch)> {
        let payloads = self.payloads.clone();
        let size = payloads[rank as usize].len() as u64;
        let (panic, corrupt) = (self.panic.contains(&rank), self.corrupt.contains(&rank));
        Some((
            size,
            Box::new(move || {
                if panic {
                    panic!("injected worker panic at rank {rank}");
                }
                if corrupt {
                    return Err(Error::invalid(
                        "malformed archive bytes: stored member CRC mismatch",
                    ));
                }
                Ok(payloads[rank as usize].clone())
            }),
        ))
    }
}
