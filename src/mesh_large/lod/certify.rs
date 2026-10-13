//! Direct certificate terms against the captured baked source (never the leaves alone).
//!
//! - source -> proxy streams the node's rank range once against the resident proxy. If the
//!   work budget ends first, every UNREAD triangle is bounded by the node's actual source box
//!   against one real proxy point (the farthest box corner, outward), so the bound stays
//!   sound.
//! - proxy -> source keeps, per proxy face, the least whole-face witness over successive
//!   bounded source chunks: each chunk's witness is a valid bound for the whole source, so
//!   stopping after any complete chunk is sound. With no complete chunk the term is unused.
use super::{
    geom::{Arith, Bvh, Tri},
    incidence::LodSource,
};
use crate::{spill::Sequence, JobError};

/// Source triangles per proxy -> source chunk.
pub(crate) const CHUNK: usize = 1 << 16;
/// Target evaluations per direct query.
const CAP: u32 = 64;
/// Source triangles per parallel source -> proxy block.
const BLOCK: u64 = 1 << 18;

pub(crate) fn source_tri(s: &LodSource) -> Tri {
    s.positions.map(|p| p.map(f64::from))
}

#[derive(Clone, Copy, Debug, Default, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct DirectReport {
    /// Not attempted: the range is too large for the work budget to finish.
    pub skipped: bool,
    pub source_to_proxy: f64,
    pub source_to_proxy_exhausted: bool,
    /// `None` when not even one chunk completed.
    pub proxy_to_source: Option<f64>,
    pub proxy_to_source_chunks: u64,
    pub proxy_to_source_exhausted: bool,
    pub evals: u64,
}

/// Both direct terms for a node's rank range `[first, end)` with source box `lo..hi`.
#[allow(clippy::too_many_arguments)]
pub(crate) fn direct(
    ar: Arith,
    source: &Sequence<LodSource>,
    first: u64,
    end: u64,
    lo: [f64; 3],
    hi: [f64; 3],
    proxy: &[Tri],
    proxy_bvh: &Bvh,
    work: u64,
    attempt: bool,
    skip: &(dyn Fn(&LodSource) -> bool + Sync),
    check: &(dyn Fn() -> Result<(), JobError> + Sync),
) -> Result<DirectReport, JobError> {
    let mut report = DirectReport::default();
    if !attempt {
        report.skipped = true;
        report.source_to_proxy = f64::INFINITY;
        return Ok(report);
    }
    // Source -> proxy: one capped query per source triangle, so the budget always covers
    // the range (the composed term would otherwise add up level by level).
    let range = end - first;
    let work_from = work.max(range.saturating_mul(u64::from(CAP) + 4));
    let anchor = proxy[0][0];
    // Fixed blocks scanned in parallel; the maximum and the totals do not depend on order.
    let blocks: Vec<(u64, u64)> = (first..end)
        .step_by(BLOCK as usize)
        .map(|b| (b, (b + BLOCK).min(end)))
        .collect();
    let per_block = work_from.div_ceil(blocks.len().max(1) as u64);
    let results: Vec<Result<(f64, u64, bool), JobError>> = {
        use rayon::prelude::*;
        blocks
            .par_iter()
            .map(|&(b, e)| {
                let mut evals = 0u64;
                let mut bound = 0f64;
                let mut exhausted = false;
                let mut reader = source.range(b, e, 1 << 18)?;
                let mut read = 0u64;
                while let Some(s) = reader.next().transpose()? {
                    read += 1;
                    if read.is_multiple_of(1 << 16) {
                        check()?;
                    }
                    if skip(&s) {
                        continue;
                    }
                    let t = source_tri(&s);
                    let w = match proxy_bvh.best_target(&t, CAP, &mut evals) {
                        Some((k, _)) => ar.patch(&t, proxy_bvh.tri(k), 0.),
                        None => f64::INFINITY,
                    };
                    bound = bound.max(w);
                    if evals >= per_block && read < e - b {
                        // Every unread triangle lies in the node's source box.
                        exhausted = true;
                        bound = bound.max(ar.box_point(lo, hi, anchor));
                        break;
                    }
                }
                Ok((bound, evals, exhausted))
            })
            .collect()
    };
    let mut bound = 0f64;
    let mut evals = 0u64;
    for r in results {
        let (b, e, x) = r?;
        bound = bound.max(b);
        evals += e;
        report.source_to_proxy_exhausted |= x;
    }
    report.source_to_proxy = bound;
    report.evals = evals;
    // Proxy -> source, chunked: a whole-face witness against small source triangles is
    // loose, so it is attempted only for ranges of at most four chunks.
    if range > 4 * CHUNK as u64 {
        return Ok(report);
    }
    let mut evals = 0u64;
    let mut faces = vec![f64::INFINITY; proxy.len()];
    let mut reader = source.range(first, end, 1 << 18)?;
    let mut chunk: Vec<Tri> = Vec::with_capacity(CHUNK.min((end - first) as usize));
    let mut remaining = end - first;
    while remaining > 0 {
        check()?;
        chunk.clear();
        while chunk.len() < CHUNK {
            match reader.next().transpose()? {
                Some(s) => {
                    remaining -= 1;
                    chunk.push(source_tri(&s));
                }
                None => {
                    remaining = 0;
                    break;
                }
            }
        }
        if chunk.is_empty() {
            break;
        }
        let bvh = Bvh::new(&chunk);
        for (face, best) in proxy.iter().zip(faces.iter_mut()) {
            if let Some((k, _)) = bvh.best_target(face, CAP, &mut evals) {
                *best = best.min(ar.patch(face, bvh.tri(k), 0.));
            }
        }
        report.proxy_to_source_chunks += 1;
        if evals >= work && remaining > 0 {
            report.proxy_to_source_exhausted = true;
            break;
        }
    }
    if report.proxy_to_source_chunks > 0 {
        report.proxy_to_source = Some(faces.iter().copied().fold(0f64, f64::max));
    }
    report.evals += evals;
    Ok(report)
}
