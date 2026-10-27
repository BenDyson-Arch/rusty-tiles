//! Internal inclusive thread-CPU timers. Disabled unless RT_HLOD_CPU_PROFILE is set;
//! output goes to stderr and never enters a payload, manifest or public diagnostic.
use std::sync::{
    atomic::{AtomicU64, Ordering},
    OnceLock,
};

#[derive(Clone, Copy)]
pub(super) enum Stage {
    Merge,
    Contract,
    Attempt,
    Simplify,
    Source,
    Boundary,
    Faces,
    Incidence,
    Fans,
    ExactFans,
    Decode,
    Contacts,
    Topology,
    Measurement,
    FineLoad,
}
const NAMES: [&str; 15] = [
    "merge",
    "contract",
    "attempt",
    "simplify",
    "source",
    "boundary",
    "faces",
    "incidence",
    "fans",
    "exact_fans",
    "decode",
    "contacts",
    "topology",
    "measurement",
    "fine_load",
];
static ENABLED: OnceLock<bool> = OnceLock::new();
static CPU: [AtomicU64; 15] = [const { AtomicU64::new(0) }; 15];
static CALLS: [AtomicU64; 15] = [const { AtomicU64::new(0) }; 15];
static WORK: [AtomicU64; 15] = [const { AtomicU64::new(0) }; 15];

pub(super) struct Timer {
    stage: Stage,
    start: Option<f64>,
    work: usize,
}
pub(super) fn start(stage: Stage, work: usize) -> Timer {
    let enabled = *ENABLED.get_or_init(|| std::env::var_os("RT_HLOD_CPU_PROFILE").is_some());
    Timer {
        stage,
        start: if enabled {
            super::error::thread_cpu()
        } else {
            None
        },
        work,
    }
}
impl Drop for Timer {
    fn drop(&mut self) {
        if let Some(elapsed) = self
            .start
            .and_then(|a| super::error::thread_cpu().map(|b| b - a))
        {
            let i = self.stage as usize;
            CPU[i].fetch_add((elapsed.max(0.) * 1e9) as u64, Ordering::Relaxed);
            CALLS[i].fetch_add(1, Ordering::Relaxed);
            WORK[i].fetch_add(self.work as u64, Ordering::Relaxed);
        }
    }
}
pub(super) fn dump() {
    if ENABLED.get() != Some(&true) {
        return;
    }
    let rows: Vec<_> = NAMES.iter().enumerate().map(|(i, name)| serde_json::json!({
        "stage": name, "inclusive_thread_cpu_s": CPU[i].swap(0, Ordering::Relaxed) as f64 / 1e9,
        "calls": CALLS[i].swap(0, Ordering::Relaxed), "items": WORK[i].swap(0, Ordering::Relaxed),
    })).collect();
    eprintln!(
        "HLOD_CPU {}",
        serde_json::json!({"version": 1, "stages": rows})
    );
}
