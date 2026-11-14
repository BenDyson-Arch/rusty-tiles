//! Independent tasks on scoped worker threads while the calling thread keeps checking.
use crate::{JobError, JobErrorKind};
use std::sync::{
    atomic::{AtomicBool, AtomicUsize, Ordering::Relaxed},
    Mutex,
};

/// How often the calling thread runs its check while tasks run.
const POLL: std::time::Duration = std::time::Duration::from_millis(5);

/// What [`run`] returns when every task ran.
pub(crate) enum Ran<T> {
    /// Each task's result, in task order.
    All(Vec<T>),
    /// A task failed; the tasks after it may not have run. The caller reports the failure its
    /// own way (capture reruns its serial pass, which reports the first failure in order).
    Failed,
}

/// Runs `task(i, stop)` for every `i < count` on up to `workers` threads. The calling thread
/// runs `poll` every few milliseconds until they finish, and a `poll` error stops the
/// workers and is returned. A task should return early (with any error) once `stop` is set.
pub(crate) fn run<T: Send>(
    count: usize,
    workers: usize,
    poll: &mut dyn FnMut() -> Result<(), JobError>,
    task: impl Fn(usize, &AtomicBool) -> Result<T, JobError> + Sync,
) -> Result<Ran<T>, JobError> {
    let results: Vec<Mutex<Option<T>>> = (0..count).map(|_| Mutex::new(None)).collect();
    let (next, stop, failed) = (
        AtomicUsize::new(0),
        AtomicBool::new(false),
        AtomicBool::new(false),
    );
    // Sets the stop flag when a worker unwinds, so the others stop and the caller joins.
    struct Unwind<'a>(&'a AtomicBool);
    impl Drop for Unwind<'_> {
        fn drop(&mut self) {
            if std::thread::panicking() {
                self.0.store(true, Relaxed);
            }
        }
    }
    let stopped = std::thread::scope(|scope| {
        let worker = || {
            let _unwind = Unwind(&stop);
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
                    Err(_) => {
                        failed.store(true, Relaxed);
                        stop.store(true, Relaxed);
                    }
                }
            }
        };
        let mut threads = Vec::new();
        let mut stopped = None;
        for _ in 0..workers.clamp(1, count.max(1)) {
            match std::thread::Builder::new()
                .name("rusty-tiles-capture-task".into())
                .spawn_scoped(scope, worker)
            {
                Ok(thread) => threads.push(thread),
                Err(error) => {
                    stop.store(true, Relaxed);
                    stopped = Some(JobError::new(
                        JobErrorKind::ResourceLimit,
                        format!("start capture worker: {error}"),
                    ));
                    break;
                }
            }
        }
        while stopped.is_none() && !threads.iter().all(|t| t.is_finished()) {
            if let Err(error) = poll() {
                stop.store(true, Relaxed);
                stopped = Some(error);
                break;
            }
            std::thread::sleep(POLL);
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
    if failed.load(Relaxed) {
        return Ok(Ran::Failed);
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
                Ran::Failed => panic!("no task fails"),
            }
            let failing = run(100, workers, &mut || Ok(()), |i, _| {
                if i == 70 {
                    Err(JobError::new(JobErrorKind::InvalidInput, "bad"))
                } else {
                    Ok(i)
                }
            })
            .unwrap();
            assert!(matches!(failing, Ran::Failed));
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
    }
}
