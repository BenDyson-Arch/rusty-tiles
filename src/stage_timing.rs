//! Opt-in stage timing for the large-mesh leaf producer. Disabled unless the
//! `RUSTY_TILES_STAGE_TIMING` environment variable names an output file; then
//! summed per-thread wall nanoseconds, call counts and texel counters are
//! accumulated in process-wide atomics and written as one JSON document when
//! the producer finishes. Diagnostics only: nothing here feeds output bytes.
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
    SamplerChainBuild,
    SamplerCheckPlanned,
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

const STAGES: [(Stage, &str); 21] = [
    (Stage::SourceReconstruct, "source_reconstruct"),
    (Stage::SourceDecode, "source_decode"),
    (Stage::SourceCacheHit, "source_cache_hit"),
    (Stage::Layout, "layout"),
    (Stage::PageCut, "page_cut"),
    (Stage::DensityMips, "density_mips"),
    (Stage::SparseMips, "sparse_mips"),
    (Stage::SamplerChainBuild, "sampler_chain_build"),
    (Stage::SamplerCheckPlanned, "sampler_check_planned"),
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

struct Totals {
    cpu_nanos: [AtomicU64; STAGES.len()],
    nanos: [AtomicU64; STAGES.len()],
    calls: [AtomicU64; STAGES.len()],
    counters: [AtomicU64; COUNTERS.len()],
    encode_nanos: [AtomicU64; ENCODE_BUCKETS],
    encode_texels: [AtomicU64; ENCODE_BUCKETS],
    encode_calls: [AtomicU64; ENCODE_BUCKETS],
}

fn totals() -> Option<&'static Totals> {
    static TOTALS: OnceLock<Option<Totals>> = OnceLock::new();
    TOTALS
        .get_or_init(|| {
            std::env::var_os("RUSTY_TILES_STAGE_TIMING").map(|_| Totals {
                cpu_nanos: std::array::from_fn(|_| AtomicU64::new(0)),
                nanos: std::array::from_fn(|_| AtomicU64::new(0)),
                calls: std::array::from_fn(|_| AtomicU64::new(0)),
                counters: std::array::from_fn(|_| AtomicU64::new(0)),
                encode_nanos: std::array::from_fn(|_| AtomicU64::new(0)),
                encode_texels: std::array::from_fn(|_| AtomicU64::new(0)),
                encode_calls: std::array::from_fn(|_| AtomicU64::new(0)),
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

/// Writes the accumulated document to the configured path, when enabled.
pub(crate) fn finish(extra: Value) {
    let (Some(totals), Some(path)) = (totals(), std::env::var_os("RUSTY_TILES_STAGE_TIMING"))
    else {
        return;
    };
    let mut stages = Map::new();
    for (i, (_, name)) in STAGES.iter().enumerate() {
        let calls = totals.calls[i].load(Ordering::Relaxed);
        if calls == 0 {
            continue;
        }
        stages.insert(
            (*name).into(),
            json!({"seconds": totals.nanos[i].load(Ordering::Relaxed) as f64 / 1e9, "calls": calls, "cpu_seconds": cfg!(target_os = "linux").then(|| totals.cpu_nanos[i].load(Ordering::Relaxed) as f64 / 1e9)}),
        );
    }
    let mut counters = Map::new();
    for (i, (_, name)) in COUNTERS.iter().enumerate() {
        counters.insert(
            (*name).into(),
            json!(totals.counters[i].load(Ordering::Relaxed)),
        );
    }
    let mut encode = Vec::new();
    for bucket in 0..ENCODE_BUCKETS {
        let calls = totals.encode_calls[bucket].load(Ordering::Relaxed);
        if calls == 0 {
            continue;
        }
        let seconds = totals.encode_nanos[bucket].load(Ordering::Relaxed) as f64 / 1e9;
        let texels = totals.encode_texels[bucket].load(Ordering::Relaxed);
        encode.push(json!({"base_texels_log2": bucket, "calls": calls, "seconds": seconds, "chain_texels": texels, "seconds_per_megapixel": seconds / (texels as f64 / 1e6)}));
    }
    let document = json!({"schema": "rusty_tiles_stage_timing_v1", "note": "summed per-thread wall time; parallel stages overlap", "stages": stages, "counters": counters, "basis_encode_by_base_size": encode, "run": extra});
    let _ = std::fs::write(
        path,
        serde_json::to_vec_pretty(&document).unwrap_or_default(),
    );
}
