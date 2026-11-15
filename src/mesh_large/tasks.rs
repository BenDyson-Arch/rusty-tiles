//! Independent tasks on scoped worker threads while the calling thread keeps checking.
use crate::{JobError, JobErrorKind};
use std::sync::{
    atomic::{AtomicBool, AtomicUsize, Ordering::Relaxed},
    Condvar, Mutex,
};

/// How often the calling thread runs its check while tasks run.
const POLL: std::time::Duration = std::time::Duration::from_millis(5);

/// What [`run`] returns when every task ran or one failed.
pub(crate) enum Ran<T> {
    /// Each task's result, in task order.
    All(Vec<T>),
    /// A task failed with this error, the first to fail (tasks that stopped because of it
    /// fail later); the tasks after it may not have run. Capture's scan and validation rerun
    /// their serial pass, which reports the first failure in input order; other callers
    /// report this error.
    Failed(JobError),
}

#[cfg(test)]
thread_local! {
    /// Test seam: worker threads [`run`] started from this thread.
    pub(crate) static SPAWNED: std::cell::Cell<u64> = const { std::cell::Cell::new(0) };
}

/// Runs `task(i, stop)` for every `i < count` on up to `workers` threads. The calling thread
/// runs `poll` while they work (at least every few milliseconds, and once more after they
/// finish); a `poll` error stops the workers and is returned. A task should return early
/// (with any error) once `stop` is set. With one task or one worker, the tasks run on the
/// calling thread, `poll` between them, and no thread is started.
pub(crate) fn run<T: Send>(
    count: usize,
    workers: usize,
    poll: &mut dyn FnMut() -> Result<(), JobError>,
    task: impl Fn(usize, &AtomicBool) -> Result<T, JobError> + Sync,
) -> Result<Ran<T>, JobError> {
    if count <= 1 || workers <= 1 {
        let stop = AtomicBool::new(false);
        let mut results = Vec::with_capacity(count);
        for i in 0..count {
            poll()?;
            crate::test_hooks::pace("mesh.capture-task");
            match task(i, &stop) {
                Ok(value) => results.push(value),
                Err(error) => return Ok(Ran::Failed(error)),
            }
        }
        poll()?;
        return Ok(Ran::All(results));
    }
    let results: Vec<Mutex<Option<T>>> = (0..count).map(|_| Mutex::new(None)).collect();
    let (next, stop, running) = (
        AtomicUsize::new(0),
        AtomicBool::new(false),
        AtomicUsize::new(0),
    );
    let first_error: Mutex<Option<JobError>> = Mutex::new(None);
    // Workers wake the calling thread when they finish a task or leave.
    let (gate, done) = (Mutex::new(()), Condvar::new());
    // Sets the stop flag when a worker unwinds, so the others stop and the caller joins;
    // counts the worker out either way.
    struct Leave<'a>(&'a AtomicBool, &'a AtomicUsize, &'a Condvar);
    impl Drop for Leave<'_> {
        fn drop(&mut self) {
            if std::thread::panicking() {
                self.0.store(true, Relaxed);
            }
            self.1.fetch_sub(1, Relaxed);
            self.2.notify_all();
        }
    }
    let stopped = std::thread::scope(|scope| {
        let worker = || {
            let _leave = Leave(&stop, &running, &done);
            loop {
                let i = next.fetch_add(1, Relaxed);
                if i >= count || stop.load(Relaxed) {
                    break;
                }
                crate::test_hooks::pace("mesh.capture-task");
                if stop.load(Relaxed) {
                    break;
                }
                match task(i, &stop) {
                    Ok(value) => {
                        *results[i].lock().unwrap_or_else(|e| e.into_inner()) = Some(value)
                    }
                    Err(error) => {
                        // The first failure in time; failures caused by the stop come later.
                        let mut first = first_error.lock().unwrap_or_else(|e| e.into_inner());
                        if first.is_none() {
                            *first = Some(error);
                        }
                        stop.store(true, Relaxed);
                    }
                }
                done.notify_all();
            }
        };
        let mut threads = Vec::new();
        let mut stopped = None;
        for _ in 0..workers.min(count) {
            running.fetch_add(1, Relaxed);
            match std::thread::Builder::new()
                .name("rusty-tiles-capture-task".into())
                .spawn_scoped(scope, worker)
            {
                Ok(thread) => {
                    #[cfg(test)]
                    SPAWNED.with(|n| n.set(n.get() + 1));
                    threads.push(thread);
                }
                Err(error) => {
                    running.fetch_sub(1, Relaxed);
                    stop.store(true, Relaxed);
                    stopped = Some(JobError::new(
                        JobErrorKind::ResourceLimit,
                        format!("start capture worker: {error}"),
                    ));
                    break;
                }
            }
        }
        while stopped.is_none() && running.load(Relaxed) > 0 {
            if let Err(error) = poll() {
                stop.store(true, Relaxed);
                stopped = Some(error);
                break;
            }
            let guard = gate.lock().unwrap_or_else(|e| e.into_inner());
            if running.load(Relaxed) > 0 {
                drop(done.wait_timeout(guard, POLL));
            }
        }
        for thread in threads {
            if let Err(panic) = thread.join() {
                std::panic::resume_unwind(panic);
            }
        }
        stopped
    });
    if let Some(error) = stopped {
        return Err(error);
    }
    // A stop that arrived while the last tasks finished.
    poll()?;
    if let Some(error) = first_error.into_inner().unwrap_or_else(|e| e.into_inner()) {
        return Ok(Ran::Failed(error));
    }
    results
        .into_iter()
        .map(|r| r.into_inner().unwrap_or_else(|e| e.into_inner()))
        .collect::<Option<Vec<T>>>()
        .map(Ran::All)
        .ok_or_else(|| JobError::new(JobErrorKind::InvalidState, "capture task did not run"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn results_keep_task_order_and_a_failure_is_reported_as_such() {
        for workers in [1, 3, 8] {
            match run(100, workers, &mut || Ok(()), |i, _| Ok(i * 2)).unwrap() {
                Ran::All(values) => assert_eq!(values, (0..100).map(|i| i * 2).collect::<Vec<_>>()),
                Ran::Failed(error) => panic!("no task fails: {error}"),
            }
            // The failing task's own error, not the stops it caused.
            let failing = run(100, workers, &mut || Ok(()), |i, stop| {
                if i == 70 {
                    Err(JobError::new(JobErrorKind::ResourceLimit, "bad"))
                } else if stop.load(Relaxed) {
                    Err(JobError::new(JobErrorKind::Cancelled, "stopped"))
                } else {
                    Ok(i)
                }
            })
            .unwrap();
            match failing {
                Ran::Failed(error) => {
                    assert_eq!(error.kind(), JobErrorKind::ResourceLimit, "{error}");
                }
                Ran::All(_) => panic!("task 70 fails"),
            }
        }
        // The caller's check stops the workers and is the error.
        let mut polls = 0;
        let error = run(
            4,
            2,
            &mut || {
                polls += 1;
                Err(JobError::new(JobErrorKind::Cancelled, "stop"))
            },
            |_, stop| {
                while !stop.load(Relaxed) {
                    std::thread::sleep(std::time::Duration::from_millis(1));
                }
                Err::<(), _>(JobError::new(JobErrorKind::Cancelled, "task stopped"))
            },
        )
        .err()
        .unwrap();
        assert_eq!(
            error.to_string(),
            JobError::new(JobErrorKind::Cancelled, "stop").to_string()
        );
        assert_eq!(polls, 1);
        // A stop that arrives once the tasks are done is still seen.
        let mut polls = 0;
        let late = run(
            3,
            2,
            &mut || {
                polls += 1;
                if polls > 1 {
                    Err(JobError::new(JobErrorKind::Cancelled, "late"))
                } else {
                    Ok(())
                }
            },
            |i, _| Ok(i),
        );
        assert!(late.is_err());
    }

    /// One task or one worker runs inline: no thread is started.
    #[test]
    fn single_tasks_and_single_workers_run_inline() {
        SPAWNED.with(|n| n.set(0));
        for _ in 0..1000 {
            run(1, 8, &mut || Ok(()), |i, _| Ok(i)).unwrap();
            run(5, 1, &mut || Ok(()), |i, _| Ok(i)).unwrap();
        }
        assert_eq!(SPAWNED.with(|n| n.get()), 0);
        run(5, 3, &mut || Ok(()), |i, _| Ok(i)).unwrap();
        assert_eq!(SPAWNED.with(|n| n.get()), 3);
    }

    /// A panicking task stops the others and the panic reaches the caller.
    #[test]
    fn a_panicking_task_stops_the_others() {
        let started = AtomicUsize::new(0);
        let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            run(64, 4, &mut || Ok(()), |i, stop| {
                started.fetch_add(1, Relaxed);
                if i == 0 {
                    panic!("injected panic");
                }
                while !stop.load(Relaxed) {
                    std::thread::sleep(std::time::Duration::from_millis(1));
                }
                Ok(())
            })
        }));
        assert!(outcome.is_err());
        assert!(started.load(Relaxed) < 64, "{}", started.load(Relaxed));
    }
}
