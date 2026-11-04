//! Stage timing for the large-mesh leaf producer. Diagnostics include
//! summed per-thread wall nanoseconds, call counts and texel counters are
//! accumulated in process-wide atomics. RUSTY_TILES_STAGE_TIMING additionally
//! writes a JSON document at completion. Nothing here feeds output bytes.
use serde_json::{json, Map, Value};
use std::{
    sync::{
        atomic::{AtomicU64, Ordering},
        OnceLock,
    },
    time::Instant,
};

#[derive(Clone, Copy, Debug)]
pub(crate) enum Stage {
    SourceReconstruct,
    SourceDecode,
    SourceCacheHit,
    Layout,
    PageCut,
    DensityMips,
    SparseMips,
    SamplerCheckDecoded,
    Mask,
    BasisEncode,
    CodecVerify,
    DecodeForCheck,
    FidelityCheck,
    PngEncode,
    PngVerify,
    GlbWrite,
    MemberWrite,
    Capture,
    Lod,
}

const STAGES: [(Stage, &str); 19] = [
    (Stage::SourceReconstruct, "source_reconstruct"),
    (Stage::SourceDecode, "source_decode"),
    (Stage::SourceCacheHit, "source_cache_hit"),
    (Stage::Layout, "layout"),
    (Stage::PageCut, "page_cut"),
    (Stage::DensityMips, "density_mips"),
    (Stage::SparseMips, "sparse_mips"),
    (Stage::SamplerCheckDecoded, "sampler_check_decoded"),
    (Stage::Mask, "mask"),
    (Stage::BasisEncode, "basis_encode"),
    (Stage::CodecVerify, "codec_verify"),
    (Stage::DecodeForCheck, "decode_for_check"),
    (Stage::FidelityCheck, "fidelity_check"),
    (Stage::PngEncode, "png_encode"),
    (Stage::PngVerify, "png_verify"),
    (Stage::GlbWrite, "glb_write"),
    (Stage::MemberWrite, "member_write"),
    (Stage::Capture, "capture"),
    (Stage::Lod, "lod"),
];

#[derive(Clone, Copy, Debug)]
pub(crate) enum Counter {
    /// Texels of every distinct source image referenced by any leaf.
    SourceTexels,
    /// Crop interior texels (real source signal) summed over all bindings.
    CropInteriorTexels,
    /// Allocated page base-level texels, including padding.
    PageBaseTexels,
    /// Encoded chain texels (all levels).
    EncodedChainTexels,
    Pages,
    Leaves,
    EncodedBytes,
}

const COUNTERS: [(Counter, &str); 7] = [
    (Counter::SourceTexels, "source_texels"),
    (Counter::CropInteriorTexels, "crop_interior_texels"),
    (Counter::PageBaseTexels, "page_base_texels"),
    (Counter::EncodedChainTexels, "encoded_chain_texels"),
    (Counter::Pages, "pages"),
    (Counter::Leaves, "leaves"),
    (Counter::EncodedBytes, "encoded_bytes"),
];

/// Basis encode time bucketed by log2 of the base-level texel count.
const ENCODE_BUCKETS: usize = 32;
/// Per-mip-level work of the delivery chain: codec decode, codec floors, delivery gates.
pub(crate) const LEVEL_DECODE: usize = 0;
pub(crate) const LEVEL_FLOORS: usize = 1;
pub(crate) const LEVEL_GATES: usize = 2;
const LEVEL_KINDS: [&str; 3] = ["codec_decode", "codec_floors", "delivery_gates"];
const LEVELS: usize = 16;

struct Totals {
    cpu_nanos: [AtomicU64; STAGES.len()],
    nanos: [AtomicU64; STAGES.len()],
    calls: [AtomicU64; STAGES.len()],
    counters: [AtomicU64; COUNTERS.len()],
    encode_nanos: [AtomicU64; ENCODE_BUCKETS],
    encode_texels: [AtomicU64; ENCODE_BUCKETS],
    encode_calls: [AtomicU64; ENCODE_BUCKETS],
    level_cpu_nanos: [[AtomicU64; LEVELS]; 3],
    level_texels: [[AtomicU64; LEVELS]; 3],
}

fn totals() -> Option<&'static Totals> {
    static TOTALS: OnceLock<Option<Totals>> = OnceLock::new();
    TOTALS
        .get_or_init(|| {
            Some(Totals {
                cpu_nanos: std::array::from_fn(|_| AtomicU64::new(0)),
                nanos: std::array::from_fn(|_| AtomicU64::new(0)),
                calls: std::array::from_fn(|_| AtomicU64::new(0)),
                counters: std::array::from_fn(|_| AtomicU64::new(0)),
                encode_nanos: std::array::from_fn(|_| AtomicU64::new(0)),
                encode_texels: std::array::from_fn(|_| AtomicU64::new(0)),
                encode_calls: std::array::from_fn(|_| AtomicU64::new(0)),
                level_cpu_nanos: std::array::from_fn(|_| {
                    std::array::from_fn(|_| AtomicU64::new(0))
                }),
                level_texels: std::array::from_fn(|_| std::array::from_fn(|_| AtomicU64::new(0))),
            })
        })
        .as_ref()
}

fn index(stage: Stage) -> usize {
    STAGES
        .iter()
        .position(|(s, _)| std::mem::discriminant(s) == std::mem::discriminant(&stage))
        .expect("every stage is listed")
}

// The encoder is single-threaded per call. On Linux, thread CPU excludes idle
// scheduling delays and other workers, unlike the summed worker wall clock.
#[cfg(target_os = "linux")]
fn thread_cpu_nanos() -> u64 {
    let mut time = libc::timespec {
        tv_sec: 0,
        tv_nsec: 0,
    };
    // SAFETY: time is a writable timespec; the clock has no external state.
    if unsafe { libc::clock_gettime(libc::CLOCK_THREAD_CPUTIME_ID, &mut time) } == 0 {
        time.tv_sec as u64 * 1_000_000_000 + time.tv_nsec as u64
    } else {
        0
    }
}
#[cfg(not(target_os = "linux"))]
fn thread_cpu_nanos() -> u64 {
    0
}

/// Runs `f`, charging worker wall time and (on Linux) thread CPU when enabled.
pub(crate) fn time<T>(stage: Stage, f: impl FnOnce() -> T) -> T {
    let Some(totals) = totals() else {
        return f();
    };
    let start = Instant::now();
    let cpu_start = thread_cpu_nanos();
    let out = f();
    let i = index(stage);
    totals.nanos[i].fetch_add(start.elapsed().as_nanos() as u64, Ordering::Relaxed);
    totals.calls[i].fetch_add(1, Ordering::Relaxed);
    totals.cpu_nanos[i].fetch_add(
        thread_cpu_nanos().saturating_sub(cpu_start),
        Ordering::Relaxed,
    );
    out
}

/// Runs `f`, the `kind` work of mip `level` over `texels`, charging thread CPU when enabled.
pub(crate) fn level<T>(kind: usize, level: usize, texels: u64, f: impl FnOnce() -> T) -> T {
    let Some(totals) = totals() else {
        return f();
    };
    let cpu_start = thread_cpu_nanos();
    let out = f();
    let l = level.min(LEVELS - 1);
    totals.level_cpu_nanos[kind][l].fetch_add(
        thread_cpu_nanos().saturating_sub(cpu_start),
        Ordering::Relaxed,
    );
    totals.level_texels[kind][l].fetch_add(texels, Ordering::Relaxed);
    out
}

pub(crate) fn count(counter: Counter, n: u64) {
    if let Some(totals) = totals() {
        let i = COUNTERS
            .iter()
            .position(|(c, _)| std::mem::discriminant(c) == std::mem::discriminant(&counter))
            .expect("every counter is listed");
        totals.counters[i].fetch_add(n, Ordering::Relaxed);
    }
}

/// Basis encode call of `chain_texels` with base level `base_texels`.
pub(crate) fn encode<T>(base_texels: u64, chain_texels: u64, f: impl FnOnce() -> T) -> T {
    let Some(totals) = totals() else {
        return f();
    };
    let start = Instant::now();
    let cpu_start = thread_cpu_nanos();
    let out = f();
    let nanos = start.elapsed().as_nanos() as u64;
    let bucket = (64 - base_texels.max(1).leading_zeros() as usize - 1).min(ENCODE_BUCKETS - 1);
    totals.encode_nanos[bucket].fetch_add(nanos, Ordering::Relaxed);
    totals.encode_texels[bucket].fetch_add(chain_texels, Ordering::Relaxed);
    totals.encode_calls[bucket].fetch_add(1, Ordering::Relaxed);
    let i = index(Stage::BasisEncode);
    totals.nanos[i].fetch_add(nanos, Ordering::Relaxed);
    totals.calls[i].fetch_add(1, Ordering::Relaxed);
    totals.cpu_nanos[i].fetch_add(
        thread_cpu_nanos().saturating_sub(cpu_start),
        Ordering::Relaxed,
    );
    out
}

/// The process totals at one instant. Totals are process-wide and never reset, so a
/// job reports [`since`] a mark taken at its start: sequential jobs in one process
/// (Python API, tests) each see only their own work.
#[derive(Clone)]
pub(crate) struct Mark(Raw);

/// The current totals, to pass to [`since`].
pub(crate) fn mark() -> Option<Mark> {
    totals().map(|t| Mark(Raw::load(t)))
}

/// The document (stages, counters, encode buckets and per-level work) of everything
/// accumulated after `mark`.
pub(crate) fn since(mark: &Option<Mark>) -> Option<Value> {
    let now = Raw::load(totals()?);
    let delta = match mark {
        Some(Mark(start)) => now.minus(start),
        None => now,
    };
    Some(delta.document(Value::Null))
}

#[derive(Clone)]
struct Raw {
    cpu_nanos: [u64; STAGES.len()],
    nanos: [u64; STAGES.len()],
    calls: [u64; STAGES.len()],
    counters: [u64; COUNTERS.len()],
    encode_nanos: [u64; ENCODE_BUCKETS],
    encode_texels: [u64; ENCODE_BUCKETS],
    encode_calls: [u64; ENCODE_BUCKETS],
    level_cpu_nanos: [[u64; LEVELS]; 3],
    level_texels: [[u64; LEVELS]; 3],
}
fn load<const N: usize>(a: &[AtomicU64; N]) -> [u64; N] {
    std::array::from_fn(|i| a[i].load(Ordering::Relaxed))
}
fn minus<const N: usize>(a: &[u64; N], b: &[u64; N]) -> [u64; N] {
    std::array::from_fn(|i| a[i].saturating_sub(b[i]))
}
impl Raw {
    fn load(t: &Totals) -> Self {
        Self {
            cpu_nanos: load(&t.cpu_nanos),
            nanos: load(&t.nanos),
            calls: load(&t.calls),
            counters: load(&t.counters),
            encode_nanos: load(&t.encode_nanos),
            encode_texels: load(&t.encode_texels),
            encode_calls: load(&t.encode_calls),
            level_cpu_nanos: std::array::from_fn(|k| load(&t.level_cpu_nanos[k])),
            level_texels: std::array::from_fn(|k| load(&t.level_texels[k])),
        }
    }
    fn minus(&self, o: &Self) -> Self {
        Self {
            cpu_nanos: minus(&self.cpu_nanos, &o.cpu_nanos),
            nanos: minus(&self.nanos, &o.nanos),
            calls: minus(&self.calls, &o.calls),
            counters: minus(&self.counters, &o.counters),
            encode_nanos: minus(&self.encode_nanos, &o.encode_nanos),
            encode_texels: minus(&self.encode_texels, &o.encode_texels),
            encode_calls: minus(&self.encode_calls, &o.encode_calls),
            level_cpu_nanos: std::array::from_fn(|k| {
                minus(&self.level_cpu_nanos[k], &o.level_cpu_nanos[k])
            }),
            level_texels: std::array::from_fn(|k| minus(&self.level_texels[k], &o.level_texels[k])),
        }
    }
}

/// Writes the accumulated document to the configured path, when enabled.
pub(crate) fn finish(extra: Value) {
    let (Some(totals), Some(path)) = (totals(), std::env::var_os("RUSTY_TILES_STAGE_TIMING"))
    else {
        return;
    };
    let document = Raw::load(totals).document(extra);
    let _ = std::fs::write(
        path,
        serde_json::to_vec_pretty(&document).unwrap_or_default(),
    );
}

impl Raw {
    fn document(&self, extra: Value) -> Value {
        let totals = self;
        let mut stages = Map::new();
        for (i, (_, name)) in STAGES.iter().enumerate() {
            let calls = totals.calls[i];
            if calls == 0 {
                continue;
            }
            stages.insert(
            (*name).into(),
            json!({"seconds": totals.nanos[i] as f64 / 1e9, "calls": calls, "cpu_seconds": cfg!(target_os = "linux").then(|| totals.cpu_nanos[i] as f64 / 1e9)}),
        );
        }
        let mut counters = Map::new();
        for (i, (_, name)) in COUNTERS.iter().enumerate() {
            counters.insert((*name).into(), json!(totals.counters[i]));
        }
        let mut encode = Vec::new();
        for bucket in 0..ENCODE_BUCKETS {
            let calls = totals.encode_calls[bucket];
            if calls == 0 {
                continue;
            }
            let seconds = totals.encode_nanos[bucket] as f64 / 1e9;
            let texels = totals.encode_texels[bucket];
            encode.push(json!({"base_texels_log2": bucket, "calls": calls, "seconds": seconds, "chain_texels": texels, "seconds_per_megapixel": seconds / (texels as f64 / 1e6)}));
        }
        let mut levels = Map::new();
        for (kind, name) in LEVEL_KINDS.iter().enumerate() {
            let rows: Vec<Value> = (0..LEVELS)
            .filter(|&l| totals.level_texels[kind][l] > 0)
            .map(|l| {
                json!({"level": l, "cpu_seconds": totals.level_cpu_nanos[kind][l] as f64 / 1e9, "texels": totals.level_texels[kind][l]})
            })
            .collect();
            levels.insert((*name).into(), json!(rows));
        }
        json!({"schema": "rusty_tiles_stage_timing_v1", "note": "summed per-thread wall time; parallel stages overlap", "stages": stages, "counters": counters, "basis_encode_by_base_size": encode, "levels": levels, "run": extra})
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    /// A job's report excludes work timed before its mark (other tests may time
    /// concurrently, so only the excluded work is pinned).
    #[test]
    fn a_mark_excludes_earlier_work() {
        let calls = |doc: Option<Value>| {
            doc.unwrap()["stages"]["png_verify"]["calls"]
                .as_u64()
                .unwrap_or(0)
        };
        let first = mark();
        time(Stage::PngVerify, || ());
        let second = mark();
        time(Stage::PngVerify, || ());
        let (from_first, from_second) = (calls(since(&first)), calls(since(&second)));
        assert!(from_first >= 2);
        assert!(from_second >= 1 && from_second < from_first);
    }
}
