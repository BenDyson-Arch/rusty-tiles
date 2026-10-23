//! SIGINT/SIGTERM become the run's ordinary cancellation, so Drop guards and tempdir
//! removal clean spill and scratch directories instead of the process dying with them
//! on disk. Unix only, behind the default `cli` feature; elsewhere the platform default applies.
//!
//! The first signal cancels the registered run and the process then exits through the
//! normal error path with the conventional `128 + signal` status. A second signal, or
//! any signal while no run is registered, exits at once.
use std::sync::atomic::{AtomicI32, Ordering};
use std::sync::Mutex;

use rusty_tiles::{CancellationHandle, Observer, RunControl};

static RUN: Mutex<Option<CancellationHandle>> = Mutex::new(None);
static SIGNAL: AtomicI32 = AtomicI32::new(0);

/// Start the signal watcher. Call once, before any work is started.
pub fn install() {
    #[cfg(all(unix, feature = "cli"))]
    {
        use signal_hook::consts::{SIGINT, SIGTERM};
        let Ok(mut signals) = signal_hook::iterator::Signals::new([SIGINT, SIGTERM]) else {
            return;
        };
        let spawned = std::thread::Builder::new()
            .name("signals".into())
            .spawn(move || {
                for signal in signals.forever() {
                    on_signal(signal);
                }
            });
        drop(spawned);
    }
}

#[cfg(all(unix, feature = "cli"))]
fn on_signal(signal: i32) {
    if SIGNAL
        .compare_exchange(0, signal, Ordering::SeqCst, Ordering::SeqCst)
        .is_err()
    {
        // The second signal forces the exit; cleanup is abandoned.
        std::process::exit(128 + signal);
    }
    let registered = RUN.lock().unwrap_or_else(|e| e.into_inner()).clone();
    match registered {
        Some(handle) if handle.cancel() => {
            eprintln!("interrupted: cancelling and cleaning up (signal again to force exit)");
        }
        // Past the point where cancellation is possible, the run publishes its result.
        Some(_) => eprintln!("publishing; signal again to force exit"),
        // Nothing to unwind yet, or a command with no cancellation point.
        None => std::process::exit(128 + signal),
    }
}

/// The signal that interrupted this process, if any.
pub fn signal() -> Option<i32> {
    Some(SIGNAL.load(Ordering::SeqCst)).filter(|signal| *signal != 0)
}

/// The conventional shell status for a process stopped by `signal`.
pub fn exit_status(signal: i32) -> u8 {
    u8::try_from(128 + signal).unwrap_or(1)
}

/// A run control the signal watcher can cancel.
pub fn run_control(observer: Option<std::sync::Arc<dyn Observer>>) -> RunControl {
    let control = RunControl::new(observer);
    let handle = control.cancellation_handle();
    *RUN.lock().unwrap_or_else(|e| e.into_inner()) = Some(handle.clone());
    // A signal that landed before the run existed still cancels it.
    if signal().is_some() {
        handle.cancel();
    }
    control
}
