//! Debug-build hooks for the stop-latency tests (`tests/stop_requests.rs`); release builds
//! compile them to nothing.
//!
//! - `RUSTY_TILES_TEST_PACE=<site>` slows the named site's work unit by
//!   `RUSTY_TILES_TEST_PACE_US` microseconds (default 1000) per call, so a small fixture
//!   spends seconds in that phase. The first call prints `rusty-tiles test: in <site>` on
//!   stderr, so a test can signal the run once it is inside the phase.
//! - `RUSTY_TILES_TEST_STALL=<site>` makes the named site block until the process exits,
//!   standing in for a native call that cannot be interrupted (a KTX2 page encode).
//! - `RUSTY_TILES_TEST_BACKSTOP_MS` shortens the CLI's stop backstop.

#[cfg(debug_assertions)]
mod enabled {
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::OnceLock;
    use std::time::Duration;

    struct Hooks {
        pace: Option<String>,
        pace_by: Duration,
        stall: Option<String>,
    }

    fn hooks() -> &'static Hooks {
        static HOOKS: OnceLock<Hooks> = OnceLock::new();
        HOOKS.get_or_init(|| Hooks {
            pace: std::env::var("RUSTY_TILES_TEST_PACE").ok(),
            pace_by: Duration::from_micros(
                std::env::var("RUSTY_TILES_TEST_PACE_US")
                    .ok()
                    .and_then(|us| us.parse().ok())
                    .unwrap_or(1000),
            ),
            stall: std::env::var("RUSTY_TILES_TEST_STALL").ok(),
        })
    }

    fn announce(site: &str) {
        static ANNOUNCED: AtomicBool = AtomicBool::new(false);
        if !ANNOUNCED.swap(true, Ordering::SeqCst) {
            eprintln!("rusty-tiles test: in {site}");
        }
    }

    pub(crate) fn pace(site: &str) {
        let hooks = hooks();
        if hooks.pace.as_deref() == Some(site) {
            announce(site);
            std::thread::sleep(hooks.pace_by);
        }
    }

    pub(crate) fn stall(site: &str) {
        if hooks().stall.as_deref() == Some(site) {
            announce(site);
            loop {
                std::thread::sleep(Duration::from_secs(1));
            }
        }
    }
}

#[cfg(debug_assertions)]
pub(crate) use enabled::{pace, stall};

#[cfg(not(debug_assertions))]
#[inline(always)]
pub(crate) fn pace(_site: &str) {}

#[cfg(not(debug_assertions))]
#[inline(always)]
pub(crate) fn stall(_site: &str) {}
