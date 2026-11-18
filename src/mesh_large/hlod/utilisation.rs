//! Worker use of the coarse walk, by cell depth (#343): how much of the walk's worker
//! time built cells at each depth, how much sat idle and while what, and how much of each
//! depth's wall time its slowest cell took.
//!
//! The walk is not depth-synchronous: a parent starts as soon as its children are ready,
//! so cells of several depths can build at once. Busy time is therefore charged to each
//! cell's own depth, and idle worker time to the coarsest depth (nearest the root) that was
//! building at that instant: the work every idle worker was waiting behind. Idle time with
//! no cell building is the walk's own serial work (reading leaves, writing rows, spilling,
//! waiting for admission). The per-depth idle times and that serial idle time add up to
//! the walk's whole idle time. Measurement only: nothing here changes scheduling or output.
use serde::Serialize;
use std::time::Instant;

/// One built cell's interval on its worker.
#[derive(Clone, Copy, Debug)]
pub(super) struct CellSpan {
    pub depth: u32,
    pub start: Instant,
    pub end: Instant,
}

/// The walk's worker use (`HlodDiagnostics::walk_utilisation`).
#[derive(Clone, Debug, Default, Serialize)]
pub struct WalkUtilisation {
    /// The walk's admitted workers.
    pub workers: u64,
    /// The walk's wall seconds (its worker pool's scope).
    pub wall_seconds: f64,
    /// Summed cell build seconds, all depths.
    pub busy_worker_seconds: f64,
    /// `workers * wall_seconds - busy_worker_seconds`.
    pub idle_worker_seconds: f64,
    /// Of the idle worker seconds, those with no cell building (the walk's serial work).
    pub serial_idle_worker_seconds: f64,
    /// One entry per depth that built a cell, root (0) first.
    pub by_depth: Vec<DepthUtilisation>,
}

#[derive(Clone, Debug, Default, Serialize)]
pub struct DepthUtilisation {
    pub depth: u32,
    /// Cells built at this depth (omitted cells build nothing and are not counted).
    pub cells: u64,
    /// Summed build seconds of this depth's cells.
    pub busy_worker_seconds: f64,
    /// Idle worker seconds while this was the coarsest depth building.
    pub idle_worker_seconds: f64,
    /// From the first of this depth's cells starting to the last finishing.
    pub wall_seconds: f64,
    /// The slowest cell's build seconds, and its share of `wall_seconds` (1 when one cell
    /// spans the depth's whole wall time).
    pub slowest_cell_seconds: f64,
    pub slowest_cell_share: f64,
}

/// Seconds between two instants, zero if `end` precedes `start`.
fn seconds(start: Instant, end: Instant) -> f64 {
    end.saturating_duration_since(start).as_secs_f64()
}

/// The walk's worker use, from its cell spans and the pool scope's `start` and `end`.
pub(super) fn measure(
    workers: u32,
    start: Instant,
    end: Instant,
    spans: &[CellSpan],
) -> WalkUtilisation {
    let depths = spans
        .iter()
        .map(|s| s.depth)
        .max()
        .map_or(0, |d| d.saturating_add(1));
    let slots = usize::try_from(depths).unwrap_or(usize::MAX);
    let mut by_depth: Vec<DepthUtilisation> = (0..depths)
        .map(|depth| DepthUtilisation {
            depth,
            ..Default::default()
        })
        .collect();
    let mut first_start: Vec<Option<Instant>> = vec![None; slots];
    let mut last_end: Vec<Option<Instant>> = vec![None; slots];
    let mut events: Vec<(Instant, bool, usize)> = Vec::with_capacity(spans.len() * 2);
    let mut busy = 0.;
    for span in spans {
        let Ok(d) = usize::try_from(span.depth) else {
            continue;
        };
        let cell = seconds(span.start, span.end);
        let entry = &mut by_depth[d];
        entry.cells += 1;
        entry.busy_worker_seconds += cell;
        entry.slowest_cell_seconds = entry.slowest_cell_seconds.max(cell);
        busy += cell;
        first_start[d] = Some(first_start[d].map_or(span.start, |t| t.min(span.start)));
        last_end[d] = Some(last_end[d].map_or(span.end, |t| t.max(span.end)));
        events.push((span.start, true, d));
        events.push((span.end, false, d));
    }
    // Ends sort before starts at the same instant, so a worker handing over is not
    // counted twice.
    events.sort_by_key(|&(at, starts, _)| (at, starts));
    let mut running = vec![0u32; slots];
    let mut building = 0u32;
    let mut serial = 0.;
    let mut at = start;
    let mut idle_between = |from: Instant, to: Instant, running: &[u32], building: u32| {
        let idle = f64::from(workers.saturating_sub(building)) * seconds(from, to);
        match running.iter().position(|&n| n > 0) {
            Some(d) => by_depth[d].idle_worker_seconds += idle,
            None => serial += idle,
        }
    };
    for &(time, starts, d) in &events {
        let time = time.clamp(start, end.max(start));
        idle_between(at, time, &running, building);
        at = at.max(time);
        if starts {
            running[d] += 1;
            building += 1;
        } else {
            running[d] = running[d].saturating_sub(1);
            building = building.saturating_sub(1);
        }
    }
    idle_between(at, end, &running, building);
    for (d, entry) in by_depth.iter_mut().enumerate() {
        if let (Some(first), Some(last)) = (first_start[d], last_end[d]) {
            entry.wall_seconds = seconds(first, last);
            entry.slowest_cell_share = if entry.wall_seconds > 0. {
                (entry.slowest_cell_seconds / entry.wall_seconds).min(1.)
            } else {
                1.
            };
        }
    }
    by_depth.retain(|entry| entry.cells > 0);
    let wall = seconds(start, end);
    WalkUtilisation {
        workers: u64::from(workers),
        wall_seconds: wall,
        busy_worker_seconds: busy,
        idle_worker_seconds: (f64::from(workers) * wall - busy).max(0.),
        serial_idle_worker_seconds: serial,
        by_depth,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    /// `base` plus whole milliseconds.
    fn after(base: Instant, millis: u64) -> Instant {
        base + Duration::from_millis(millis)
    }

    fn close(a: f64, b: f64) -> bool {
        (a - b).abs() < 1e-9
    }

    /// Two workers over a 10 ms walk. Depth 2: two cells 0-4 ms and 1-3 ms. Depth 1: one
    /// cell 4-8 ms. Nothing builds 8-10 ms. Busy 4+2+4 = 10 worker-ms of 20.
    #[test]
    fn idle_time_is_charged_to_the_coarsest_depth_building_or_to_serial_work() {
        let t = Instant::now();
        let span = |depth, a, b| CellSpan {
            depth,
            start: after(t, a),
            end: after(t, b),
        };
        let spans = [span(2, 0, 4), span(2, 1, 3), span(1, 4, 8)];
        let u = measure(2, t, after(t, 10), &spans);
        assert_eq!(u.workers, 2);
        assert!(close(u.wall_seconds, 0.010));
        assert!(close(u.busy_worker_seconds, 0.010));
        assert!(close(u.idle_worker_seconds, 0.010));
        // 8-10 ms: two idle workers with nothing building.
        assert!(close(u.serial_idle_worker_seconds, 0.004));
        let depths: Vec<u32> = u.by_depth.iter().map(|d| d.depth).collect();
        assert_eq!(depths, [1, 2]);
        let (d1, d2) = (&u.by_depth[0], &u.by_depth[1]);
        // Depth 2 idles one worker 0-1 and 3-4 ms; depth 1 idles one worker 4-8 ms.
        assert!(close(d2.idle_worker_seconds, 0.002));
        assert!(close(d1.idle_worker_seconds, 0.004));
        assert!(close(
            d1.idle_worker_seconds + d2.idle_worker_seconds + u.serial_idle_worker_seconds,
            u.idle_worker_seconds
        ));
        assert_eq!((d2.cells, d1.cells), (2, 1));
        assert!(close(d2.wall_seconds, 0.004));
        assert!(close(d2.slowest_cell_seconds, 0.004));
        assert!(close(d2.slowest_cell_share, 1.));
        assert!(close(d1.busy_worker_seconds, 0.004));
    }

    /// While a coarse and a fine cell build together, idle time goes to the coarse depth.
    #[test]
    fn overlapping_depths_charge_idle_time_to_the_coarser() {
        let t = Instant::now();
        let spans = [
            CellSpan {
                depth: 3,
                start: t,
                end: after(t, 6),
            },
            CellSpan {
                depth: 1,
                start: after(t, 2),
                end: after(t, 4),
            },
        ];
        let u = measure(4, t, after(t, 6), &spans);
        let d1 = u.by_depth.iter().find(|d| d.depth == 1).unwrap();
        let d3 = u.by_depth.iter().find(|d| d.depth == 3).unwrap();
        // 2-4 ms: two idle workers behind depth 1; 0-2 and 4-6 ms: three behind depth 3.
        assert!(close(d1.idle_worker_seconds, 0.004));
        assert!(close(d3.idle_worker_seconds, 0.012));
        assert!(close(u.serial_idle_worker_seconds, 0.));
        assert!(close(d1.slowest_cell_share, 1.));
        // An empty walk is all serial.
        let empty = measure(3, t, after(t, 5), &[]);
        assert!(empty.by_depth.is_empty());
        assert!(close(empty.serial_idle_worker_seconds, 0.015));
        assert!(close(empty.idle_worker_seconds, 0.015));
    }
}
