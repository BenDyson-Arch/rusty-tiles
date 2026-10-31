//! Finite-grid MASK scale on a fixed semantic level. Run only AFTER the complete
//! unadjusted chain exists: adjusted alpha must never feed another reduction.
//! Immutable source_pass targets refer to original source mip zero. A delivery
//! guard is admissible only if preserves_classifications proves every positive
//! weighted finite surface sample keeps its label; this is not a continuous
//! surface guarantee. Guards never change the fixed unguarded optimum gap.
//! PNG literal source-pixel lossless bypasses this module. Delivered coverage
//! must be checked against an INDEPENDENT semantic optimum plus profile floor.
use crate::{JobError, JobErrorKind};
const Q: u64 = 1 << 32;
const MAX: u64 = 256 * Q;
#[derive(Clone, Copy, Debug)]
pub(in crate::mesh_large) struct SurfaceSample {
    pub uv: [f64; 2],
    /// Original interpolated vertex COLOR alpha; material factor is separate.
    pub alpha: f64,
    pub weight: f64,
    pub source_pass: bool,
}
#[derive(Debug)]
pub(in crate::mesh_large) struct MaskReceipt {
    pub target: f64,
    pub optimum_gap: f64,
    pub scale: f64,
    /// Complete streamed sample evaluations, including literal/candidate probes.
    pub scans: u32,
}
fn invalid(message: &str) -> JobError {
    JobError::new(JobErrorKind::InvalidRequest, message.to_owned())
}
fn validate(
    bytes: &[u8],
    w: u32,
    h: u32,
    samples: &[SurfaceSample],
    factor: f64,
    cutoff: f64,
) -> Result<f64, JobError> {
    if w == 0 || h == 0 {
        return Err(invalid("MASK level dimensions must be nonzero"));
    }
    let n = (w as usize)
        .checked_mul(h as usize)
        .and_then(|x| x.checked_mul(4))
        .ok_or_else(|| {
            JobError::new(
                JobErrorKind::ResourceLimit,
                "MASK shape arithmetic overflow",
            )
        })?;
    if bytes.len() != n {
        return Err(invalid("MASK RGBA shape mismatch"));
    }
    if !factor.is_finite() || !cutoff.is_finite() || !(0.0..=1.0).contains(&factor) || cutoff < 0.0
    {
        return Err(invalid("MASK material factor/cutoff invalid"));
    }
    let mut total = 0.0;
    for s in samples {
        if s.uv.iter().any(|v| !v.is_finite())
            || !s.alpha.is_finite()
            || !(0.0..=1.0).contains(&s.alpha)
            || !s.weight.is_finite()
            || s.weight < 0.0
        {
            return Err(invalid("MASK surface sample invalid"));
        }
        total += s.weight;
    }
    if !total.is_finite() {
        return Err(invalid("MASK samples require finite total weight"));
    }
    Ok(total)
}
fn scaled(a: u8, k: u64) -> u8 {
    (f64::from(a) * (k as f64 / Q as f64) + 0.5)
        .floor()
        .clamp(0.0, 255.0) as u8
}
/// How a delivered page is sampled: LINEAR or NEAREST taps and the published wrap per
/// axis. A plain `bool` is a clamped page; a period page repeats or mirrors its whole axes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::mesh_large) struct Taps {
    pub linear: bool,
    pub wrap: [u32; 2],
}
impl From<bool> for Taps {
    fn from(linear: bool) -> Self {
        Self {
            linear,
            wrap: [33071; 2],
        }
    }
}
/// Texel index `i` of an `n`-texel axis under a glTF wrap (as the sampler addresses it).
fn wrapped(i: i64, n: u32, wrap: u32) -> usize {
    let n = i64::from(n);
    (match wrap {
        10497 => i.rem_euclid(n),
        33648 => {
            let p = i.rem_euclid(2 * n);
            if p < n {
                p
            } else {
                2 * n - 1 - p
            }
        }
        _ => i.clamp(0, n - 1),
    }) as usize
}
fn sample_alpha(bytes: &[u8], w: u32, h: u32, uv: [f64; 2], taps: Taps, k: u64) -> f64 {
    let clamp = |c: f64, wrap: u32| if wrap == 33071 { c.clamp(0.0, 1.0) } else { c };
    let (u, v) = (clamp(uv[0], taps.wrap[0]), clamp(uv[1], taps.wrap[1]));
    let tap = |x: i64, y: i64| {
        let (x, y) = (wrapped(x, w, taps.wrap[0]), wrapped(y, h, taps.wrap[1]));
        f64::from(scaled(bytes[(y * w as usize + x) * 4 + 3], k)) / 255.0
    };
    if !taps.linear {
        return tap(
            (u * f64::from(w)).floor() as i64,
            (v * f64::from(h)).floor() as i64,
        );
    }
    let (x, y) = (u * f64::from(w) - 0.5, v * f64::from(h) - 0.5);
    let (ix, iy) = (x.floor() as i64, y.floor() as i64);
    let (fx, fy) = (x - x.floor(), y - y.floor());
    let top = tap(ix, iy) * (1.0 - fx) + tap(ix + 1, iy) * fx;
    let bottom = tap(ix, iy + 1) * (1.0 - fx) + tap(ix + 1, iy + 1) * fx;
    top * (1.0 - fy) + bottom * fy
}
struct Evaluator<'a> {
    bytes: &'a [u8],
    w: u32,
    h: u32,
    samples: &'a [SurfaceSample],
    factor: f64,
    cutoff: f64,
    taps: Taps,
    total: f64,
    scans: u32,
}
impl Evaluator<'_> {
    fn value(&mut self, k: u64) -> f64 {
        self.scans += 1;
        let mut passing = 0.0;
        for s in self.samples {
            if sample_alpha(self.bytes, self.w, self.h, s.uv, self.taps, k) * self.factor * s.alpha
                >= self.cutoff
            {
                passing += s.weight;
            }
        }
        passing / self.total
    }
    fn first(&mut self, threshold: f64, strict: bool) -> u64 {
        let (mut low, mut high) = (0, MAX + 1);
        while low < high {
            let mid = low + (high - low) / 2;
            let yes = if mid > MAX {
                false
            } else {
                let value = self.value(mid);
                if strict {
                    value > threshold
                } else {
                    value >= threshold
                }
            };
            if yes {
                high = mid;
            } else {
                low = mid + 1;
            }
        }
        low
    }
}

pub(in crate::mesh_large) fn coverage(
    bytes: &[u8],
    w: u32,
    h: u32,
    samples: &[SurfaceSample],
    factor: f64,
    cutoff: f64,
    taps: impl Into<Taps>,
) -> Result<f64, JobError> {
    let total = validate(bytes, w, h, samples, factor, cutoff)?;
    if total == 0.0 {
        return Ok(0.0);
    }
    Ok(Evaluator {
        bytes,
        w,
        h,
        samples,
        factor,
        cutoff,
        taps: taps.into(),
        total,
        scans: 0,
    }
    .value(Q))
}

/// Finite geometry-sample proof for a proposed delivery mutation. Coverage
/// equality alone is insufficient: pass labels can swap while totals stay equal.
/// Validates both full byte shapes; ignores only zero-weight sample labels.
pub(in crate::mesh_large) fn preserves_classifications(
    before: &[u8],
    after: &[u8],
    dimensions: [u32; 2],
    samples: &[SurfaceSample],
    factor: f64,
    cutoff: f64,
    taps: impl Into<Taps>,
) -> Result<bool, JobError> {
    let [w, h] = dimensions;
    let taps = taps.into();
    validate(before, w, h, samples, factor, cutoff)?;
    validate(after, w, h, samples, factor, cutoff)?;
    for sample in samples {
        if sample.weight == 0.0 {
            continue;
        }
        let passes = |bytes: &[u8]| {
            sample_alpha(bytes, w, h, sample.uv, taps, Q) * factor * sample.alpha >= cutoff
        };
        if passes(before) != passes(after) {
            return Ok(false);
        }
    }
    Ok(true)
}

pub(in crate::mesh_large) fn adjust(
    bytes: &mut [u8],
    w: u32,
    h: u32,
    samples: &[SurfaceSample],
    factor: f64,
    cutoff: f64,
    taps: impl Into<Taps>,
) -> Result<MaskReceipt, JobError> {
    let total = validate(bytes, w, h, samples, factor, cutoff)?;
    if total == 0.0 {
        return Ok(MaskReceipt {
            target: 0.0,
            optimum_gap: 0.0,
            scale: 1.0,
            scans: 0,
        });
    }
    let target = samples
        .iter()
        .filter(|s| s.source_pass)
        .map(|s| s.weight)
        .sum::<f64>()
        / total;
    let (key, scans) = {
        let mut eval = Evaluator {
            bytes,
            w,
            h,
            samples,
            factor,
            cutoff,
            taps: taps.into(),
            total,
            scans: 0,
        };
        let crossing = eval.first(target, false);
        let candidates = [
            0,
            Q,
            MAX,
            crossing.min(MAX),
            crossing.saturating_sub(1).min(MAX),
        ];
        let values = candidates.map(|k| eval.value(k));
        let minimum = values
            .iter()
            .map(|&v| (v - target).abs())
            .fold(f64::INFINITY, f64::min);
        let at_one = values[1];
        let mut best = (f64::INFINITY, u64::MAX, u64::MAX);
        for (i, &value) in values.iter().enumerate() {
            if (value - target).abs() != minimum || values[..i].contains(&value) {
                continue;
            }
            let k = if at_one == value {
                Q
            } else if at_one < value {
                eval.first(value, false)
            } else {
                eval.first(value, true) - 1
            };
            let key = ((value - target).abs(), k.abs_diff(Q), k);
            if key < best {
                best = key;
            }
        }
        (best, eval.scans)
    };
    for pixel in bytes.as_chunks_mut::<4>().0 {
        pixel[3] = scaled(pixel[3], key.2);
    }
    Ok(MaskReceipt {
        target,
        optimum_gap: key.0,
        scale: key.2 as f64 / Q as f64,
        scans,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    fn pixels(alpha: &[u8]) -> Vec<u8> {
        alpha.iter().flat_map(|&a| [11, 22, 33, a]).collect()
    }
    fn samples(n: usize) -> Vec<SurfaceSample> {
        (0..n)
            .map(|i| SurfaceSample {
                uv: [(i as f64 + 0.5) / n as f64, 0.5],
                alpha: 1.0,
                weight: 1.0,
                source_pass: i < n / 2,
            })
            .collect()
    }
    // Independent finite event reference: integer rounding and centre NEAREST,
    // not production quantization/sampler/search helpers.
    fn event_reference(
        alpha: &[u8],
        surface: &[SurfaceSample],
        factor: f64,
        cutoff: f64,
    ) -> (f64, u64) {
        let target = surface
            .iter()
            .filter(|s| s.source_pass)
            .map(|s| s.weight)
            .sum::<f64>()
            / surface.iter().map(|s| s.weight).sum::<f64>();
        let mut events = vec![0, MAX + 1];
        for &a in alpha {
            if a > 0 {
                for byte in 1..=255u64 {
                    let numerator = (2 * byte - 1) * (Q / 2);
                    let transition = numerator.div_ceil(u64::from(a));
                    if transition <= MAX {
                        events.push(transition);
                    }
                }
            }
        }
        events.sort_unstable();
        events.dedup();
        let mut best = (f64::INFINITY, u64::MAX, u64::MAX);
        for interval in events.windows(2) {
            let k = Q.clamp(interval[0], interval[1] - 1);
            let mut pass = 0.0;
            let mut total = 0.0;
            for s in surface {
                let i = (s.uv[0] * alpha.len() as f64)
                    .floor()
                    .clamp(0.0, (alpha.len() - 1) as f64) as usize;
                let byte = ((u64::from(alpha[i]) * k + Q / 2) / Q).min(255);
                if byte as f64 / 255.0 * factor * s.alpha >= cutoff {
                    pass += s.weight;
                }
                total += s.weight;
            }
            let candidate = ((pass / total - target).abs(), k.abs_diff(Q), k);
            if candidate < best {
                best = candidate;
            }
        }
        (best.0, best.2)
    }
    #[test]
    fn finite_events_nearest_ties_and_varying_vertex_alpha() {
        for values in [vec![191, 64, 191, 64], vec![128; 4], vec![100, 100, 90, 10]] {
            for varied in [false, true] {
                let mut surface = samples(4);
                if varied {
                    surface[1].alpha = 0.5;
                    surface[3].alpha = 0.25;
                }
                let reference = event_reference(&values, &surface, 0.75, 0.5);
                let mut rgba = pixels(&values);
                let receipt = adjust(&mut rgba, 4, 1, &surface, 0.75, 0.5, false).unwrap();
                assert_eq!(receipt.optimum_gap, reference.0);
                assert_eq!(receipt.scale, reference.1 as f64 / Q as f64);
                assert!(receipt.scans <= 129);
                assert_eq!(receipt.target, 0.5);
                for p in rgba.as_chunks::<4>().0 {
                    assert_eq!(&p[..3], &[11, 22, 33]);
                }
            }
        }
    }
    #[test]
    fn attainable_larger_and_unavoidable_terminal_gap() {
        let surface = samples(16);
        let mut larger = pixels(
            &(0..16)
                .map(|i| if i % 2 == 0 { 191 } else { 64 })
                .collect::<Vec<_>>(),
        );
        let receipt = adjust(&mut larger, 16, 1, &surface, 1.0, 0.8, false).unwrap();
        assert_eq!(receipt.optimum_gap, 0.0);
        assert_eq!(
            coverage(&larger, 16, 1, &surface, 1.0, 0.8, false).unwrap(),
            0.5
        );
        let all_same: Vec<_> = surface
            .iter()
            .map(|s| SurfaceSample {
                uv: [0.5, 0.5],
                ..*s
            })
            .collect();
        let mut terminal = pixels(&[128]);
        let receipt = adjust(&mut terminal, 1, 1, &all_same, 1.0, 0.8, false).unwrap();
        assert_eq!(receipt.optimum_gap, 0.5);
        assert_eq!(receipt.scale, 1.0);
        assert_eq!(terminal[3], 128);
        // Terminal allowance derives from semantic gap; 16 samples are not16
        // independently attainable alpha outcomes. Source target stays0.5.
        assert!(receipt.optimum_gap > 1.0 / 16.0);
    }
    #[test]
    fn independent_linear_half_texel_and_physical_clamp() {
        let rgba = pixels(&[0, 255]);
        let surface = [
            SurfaceSample {
                uv: [0.5, 0.5],
                alpha: 0.5,
                weight: 3.0,
                source_pass: false,
            },
            SurfaceSample {
                uv: [2.0, -1.0],
                alpha: 1.0,
                weight: 1.0,
                source_pass: true,
            },
        ];
        // Centre interpolation .5 * C0 .5 fails .3; clamped endpoint1 passes.
        assert_eq!(
            coverage(&rgba, 2, 1, &surface, 1.0, 0.3, true).unwrap(),
            0.25
        );
        let mut mutable = rgba.clone();
        let receipt = adjust(&mut mutable, 2, 1, &surface, 1.0, 0.3, true).unwrap();
        assert_eq!(receipt.optimum_gap, 0.0);
        assert_eq!(receipt.scale, 1.0);
        assert_eq!(mutable, rgba); // no guard
        let missing_c0: Vec<_> = surface
            .iter()
            .map(|s| SurfaceSample { alpha: 1.0, ..*s })
            .collect();
        assert_eq!(
            coverage(&rgba, 2, 1, &missing_c0, 1.0, 0.3, true).unwrap(),
            1.0
        );
    }
    #[test]
    fn omitted_lower_neighbor_and_candidate_gap_sensitive_controls() {
        let values = [100, 100, 100, 100, 90, 90, 90, 10, 10, 10];
        let mut surface = samples(10);
        for (i, s) in surface.iter_mut().enumerate() {
            s.source_pass = i < 5;
        }
        let reference = event_reference(&values, &surface, 1.0, 0.8);
        let mut rgba = pixels(&values);
        let receipt = adjust(&mut rgba, 10, 1, &surface, 1.0, 0.8, false).unwrap();
        assert_eq!(receipt.optimum_gap, reference.0);
        assert!(receipt.optimum_gap < 0.2);
        let source = samples(4);
        let original = [191, 64, 191, 64];
        let semantic_gap = event_reference(&original, &source, 1.0, 0.8).0;
        let candidate_gap = event_reference(&[0; 4], &source, 1.0, 0.8).0;
        assert_eq!(semantic_gap, 0.0);
        assert_eq!(candidate_gap, 0.5);
        // The delivery layer must use originalsemanticgap, nevercandidategap.
    }
    #[test]
    fn malformed_requests_and_direct_zero_factor_cutoff_semantics() {
        let surface = samples(2);
        let rgba = pixels(&[0, 255]);
        assert_eq!(
            coverage(&rgba, 0, 1, &surface, 1.0, 0.5, false)
                .unwrap_err()
                .kind(),
            JobErrorKind::InvalidRequest
        );
        assert_eq!(
            coverage(&rgba, 2, 1, &surface, f64::NAN, 0.5, false)
                .unwrap_err()
                .kind(),
            JobErrorKind::InvalidRequest
        );
        assert_eq!(coverage(&rgba, 2, 1, &[], 1.0, 0.5, false).unwrap(), 0.0);
        let mut unchanged = rgba.clone();
        let no_surface = adjust(&mut unchanged, 2, 1, &[], 1.0, 0.5, false).unwrap();
        assert_eq!(unchanged, rgba);
        assert_eq!(no_surface.target, 0.0);
        assert_eq!(no_surface.optimum_gap, 0.0);
        assert_eq!(no_surface.scale, 1.0);
        assert_eq!(no_surface.scans, 0);
        let no_weight: Vec<_> = surface
            .iter()
            .map(|s| SurfaceSample { weight: 0.0, ..*s })
            .collect();
        assert_eq!(
            coverage(&rgba, 2, 1, &no_weight, 1.0, 0.5, false).unwrap(),
            0.0
        );
        assert_eq!(
            coverage(&rgba, 2, 1, &surface, 1.0, 2.0, false).unwrap(),
            0.0
        );
        assert_eq!(
            coverage(&rgba, 2, 1, &surface, 0.0, 0.0, false).unwrap(),
            1.0
        );
        assert_eq!(
            coverage(&rgba, 2, 1, &surface, 0.0, 0.1, false).unwrap(),
            0.0
        );
    }
    #[test]
    fn guard_proof_preserves_every_context_label_not_only_coverage() {
        let before = pixels(&[204]);
        let after = pixels(&[205]);
        let constant = [
            SurfaceSample {
                uv: [0.5, 0.5],
                alpha: 1.0,
                weight: 1.0,
                source_pass: true,
            },
            SurfaceSample {
                uv: [0.5, 0.5],
                alpha: 0.5,
                weight: 1.0,
                source_pass: false,
            },
        ];
        assert!(
            preserves_classifications(&before, &after, [1, 1], &constant, 1.0, 0.8, false).unwrap()
        );
        let coupled = [
            constant[0],
            SurfaceSample {
                alpha: 0.999,
                ..constant[1]
            },
        ];
        assert!(
            !preserves_classifications(&before, &after, [1, 1], &coupled, 1.0, 0.8, false).unwrap()
        );
        // Source204*C0.999 fails; guarded205*C0.999 passes at cutoff.8.
        // The predicate result above executes this crossing.

        let labels = samples(2);
        let swapped_before = pixels(&[255, 0]);
        let swapped_after = pixels(&[0, 255]);
        assert_eq!(
            coverage(&swapped_before, 2, 1, &labels, 1.0, 0.5, false).unwrap(),
            coverage(&swapped_after, 2, 1, &labels, 1.0, 0.5, false).unwrap()
        );
        assert!(!preserves_classifications(
            &swapped_before,
            &swapped_after,
            [2, 1],
            &labels,
            1.0,
            0.5,
            false
        )
        .unwrap());
    }
    #[test]
    fn guard_proof_linear_crossing_empty_and_direct_edges() {
        let before = pixels(&[204, 0]);
        let after = pixels(&[205, 0]);
        let centre = [SurfaceSample {
            uv: [0.5, 0.5],
            alpha: 1.0,
            weight: 1.0,
            source_pass: false,
        }];

        assert!(
            !preserves_classifications(&before, &after, [2, 1], &centre, 1.0, 0.401, true).unwrap()
        );
        assert!(
            preserves_classifications(&before, &after, [2, 1], &centre, 1.0, 2.0, true).unwrap()
        );
        assert!(
            preserves_classifications(&before, &after, [2, 1], &centre, 0.0, 0.0, true).unwrap()
        );
        assert!(
            preserves_classifications(&before, &after, [2, 1], &centre, 0.0, 0.5, true).unwrap()
        );
        assert!(preserves_classifications(&before, &after, [2, 1], &[], 1.0, 0.5, true).unwrap());
        let zero = [SurfaceSample {
            weight: 0.0,
            ..centre[0]
        }];
        assert!(
            preserves_classifications(&before, &after, [2, 1], &zero, 1.0, 0.401, true).unwrap()
        );
        assert_eq!(
            preserves_classifications(&before, &after[..7], [2, 1], &centre, 1.0, 0.5, true)
                .unwrap_err()
                .kind(),
            JobErrorKind::InvalidRequest
        );
        let malformed = [SurfaceSample {
            alpha: f64::NAN,
            ..centre[0]
        }];
        assert_eq!(
            preserves_classifications(&before, &after, [2, 1], &malformed, 1.0, 0.5, true)
                .unwrap_err()
                .kind(),
            JobErrorKind::InvalidRequest
        );
    }
}
