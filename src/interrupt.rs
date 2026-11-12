//! SIGINT/SIGTERM (Unix) and Ctrl+C/Ctrl+Break (Windows console) become the run's ordinary
//! cancellation, so Drop guards and tempdir removal clean spill and scratch directories
//! instead of the process dying with them on disk. Unix handling is behind the default
//! `cli` feature; elsewhere the platform default applies.
//!
//! The first signal cancels the registered run and the process then exits through the
//! normal error path with the conventional `128 + signal` status (130 for Ctrl+C and
//! Ctrl+Break). Every long phase checks for the stop within a few seconds; a native call
//! that cannot be interrupted (one KTX2 page encode can take minutes) is covered by a
//! backstop: if the run has not finished unwinding [`BACKSTOP`] after the signal, the CLI
//! removes the run's registered temporary paths itself and exits with the same status. A
//! second signal, or any signal while no run is registered, terminates the process at once:
//! by that signal on Unix, by the console's default handler on Windows.
use std::sync::atomic::{AtomicI32, Ordering};
use std::sync::{Mutex, Once};
use std::time::Duration;

use rusty_tiles::{CancellationHandle, Observer, RunControl};

static RUN: Mutex<Option<CancellationHandle>> = Mutex::new(None);
static SIGNAL: AtomicI32 = AtomicI32::new(0);
static INSTALL: Once = Once::new();

/// How long an accepted stop may take before the backstop exits: far longer than any
/// checkpointed phase needs, far below systemd's 90 s stop timeout.
const BACKSTOP: Duration = Duration::from_secs(20);

/// What the watcher does after one signal.
#[cfg_attr(not(any(all(unix, feature = "cli"), windows)), allow(dead_code))]
enum Action {
    /// The run is unwinding (or publishing); keep running.
    Continue,
    /// Terminate as the platform would without a handler.
    Default,
}

/// Start the signal watcher before any work is started. Later calls do nothing, so a
/// handler is never registered twice.
pub fn install() {
    INSTALL.call_once(install_once);
}

fn install_once() {
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
                    if let Action::Default = on_signal(signal) {
                        die_by(signal);
                    }
                }
            });
        drop(spawned);
    }
    #[cfg(windows)]
    console::install();
}

#[cfg_attr(not(any(all(unix, feature = "cli"), windows)), allow(dead_code))]
fn on_signal(signal: i32) -> Action {
    if SIGNAL
        .compare_exchange(0, signal, Ordering::SeqCst, Ordering::SeqCst)
        .is_err()
    {
        // The second signal forces the exit; cleanup is abandoned.
        return Action::Default;
    }
    let registered = RUN.lock().unwrap_or_else(|e| e.into_inner()).clone();
    match registered {
        Some(handle) if handle.cancel() => {
            errln!("interrupted: cancelling and cleaning up (signal again to force exit)");
            arm_backstop(signal);
            Action::Continue
        }
        // Past the point where cancellation is possible, the run publishes its result.
        Some(_) => {
            errln!("publishing; signal again to force exit");
            Action::Continue
        }
        // Nothing to unwind yet, or a command with no cancellation point (such as
        // `preview`): terminate by the signal itself, as without a handler.
        None => Action::Default,
    }
}

/// Terminate by `signal` with its default disposition, so a parent sees the process
/// killed by the signal rather than an ordinary exit status.
#[cfg(all(unix, feature = "cli"))]
fn die_by(signal: i32) -> ! {
    let _ = signal_hook::low_level::emulate_default_handler(signal);
    std::process::exit(128 + signal)
}

/// Windows console control events. The system runs the handler on a thread of its own,
/// so it can take the run lock like the Unix watcher thread does.
#[cfg(windows)]
mod console {
    use windows_sys::Win32::System::Console::{
        SetConsoleCtrlHandler, CTRL_BREAK_EVENT, CTRL_C_EVENT,
    };

    /// Ctrl+C and Ctrl+Break both report the SIGINT status (130).
    const SIGINT: i32 = 2;

    pub(super) fn install() {
        // SAFETY: the handler is a plain function that lives for the whole process.
        let _ = unsafe { SetConsoleCtrlHandler(Some(handler), 1) };
    }

    /// TRUE (handled) keeps the process running while the run unwinds; FALSE passes the
    /// event to the next handler, ending in the default `ExitProcess`. Close, logoff and
    /// shutdown events always take the default.
    unsafe extern "system" fn handler(event: u32) -> i32 {
        if event != CTRL_C_EVENT && event != CTRL_BREAK_EVENT {
            return 0;
        }
        match super::on_signal(SIGINT) {
            super::Action::Continue => 1,
            super::Action::Default => 0,
        }
    }
}

/// After [`BACKSTOP`], remove the run's registered temporary paths and exit with the
/// signal's status, in case the run is blocked in a call that cannot see the stop. A run
/// that was cancelled never publishes, so nothing else needs to be undone.
#[cfg_attr(not(any(all(unix, feature = "cli"), windows)), allow(dead_code))]
fn arm_backstop(signal: i32) {
    let mut wait = BACKSTOP;
    if cfg!(debug_assertions) {
        if let Some(ms) = std::env::var("RUSTY_TILES_TEST_BACKSTOP_MS")
            .ok()
            .and_then(|ms| ms.parse().ok())
        {
            wait = Duration::from_millis(ms);
        }
    }
    let spawned = std::thread::Builder::new()
        .name("stop-backstop".into())
        .spawn(move || {
            std::thread::sleep(wait);
            errln!(
                "interrupted: the run did not stop within {} s; removing its temporary files and exiting",
                wait.as_secs()
            );
            for path in rusty_tiles::cleanup::remove_registered_now() {
                errln!("could not remove {}", path.display());
            }
            std::process::exit(i32::from(exit_status(signal)));
        });
    drop(spawned);
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
