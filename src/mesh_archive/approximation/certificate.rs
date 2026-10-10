//! Complete decoded-surface bounds using exact source proof patches.
//! Projection only proposes target witnesses; intervals certify their distances.
use crate::{JobError, JobErrorKind};

pub(super) type Face = [[f32; 3]; 3];
type Result<T> = std::result::Result<T, JobError>;
type Weights = [u32; 3];
const DENOMINATOR: u32 = 1 << 24;
const WEIGHT_SCALE: f64 = 1.0 / DENOMINATOR as f64;
const MAX_PATCH_FACE_TESTS: u64 = 16_777_216;
const MAX_DEPTH: u8 = 24;
const MAX_PENDING_PATCHES: usize = 1 + 3 * MAX_DEPTH as usize;

pub(in crate::mesh_archive) struct Certificate {
    pub error_metres: f64,
    pub patch_face_tests: u64,
    pub accepted_patches: u64,
    pub max_depth: u8,
}

fn unsupported(message: &str) -> JobError {
    JobError::new(JobErrorKind::Unsupported, message)
}

fn invalid(message: &str) -> JobError {
    JobError::new(JobErrorKind::InvalidState, message)
}

#[derive(Clone, Copy)]
struct Patch {
    corners: [Weights; 3],
    depth: u8,
}

impl Patch {
    const ROOT: Self = Self {
        corners: [
            [DENOMINATOR, 0, 0],
            [0, DENOMINATOR, 0],
            [0, 0, DENOMINATOR],
        ],
        depth: 0,
    };

    fn children(self) -> Result<[Self; 4]> {
        if self.depth >= MAX_DEPTH {
            return Err(invalid(
                "certificate split exceeds the exact patch address depth",
            ));
        }
        let midpoint = |a: Weights, b: Weights| -> Result<Weights> {
            let mut result = [0; 3];
            for axis in 0..3 {
                let sum = a[axis]
                    .checked_add(b[axis])
                    .filter(|v| v.is_multiple_of(2))
                    .ok_or_else(|| {
                        invalid("certificate patch midpoint is not an exact dyadic address")
                    })?;
                result[axis] = sum / 2;
            }
            Ok(result)
        };
        let [a, b, c] = self.corners;
        let (ab, bc, ca) = (midpoint(a, b)?, midpoint(b, c)?, midpoint(c, a)?);
        let depth = self.depth + 1;
        Ok([
            Self {
                corners: [a, ab, ca],
                depth,
            },
            Self {
                corners: [ab, b, bc],
                depth,
            },
            Self {
                corners: [ca, bc, c],
                depth,
            },
            Self {
                corners: [ab, bc, ca],
                depth,
            },
        ])
    }
}

#[derive(Clone, Copy)]
struct Interval {
    low: f64,
    high: f64,
}

impl Interval {
    fn exact(value: f64) -> Self {
        Self {
            low: value,
            high: value,
        }
    }
}

struct Corner {
    coordinates: [Interval; 3],
    proposal: [f64; 3],
    exact_point: Option<[f32; 3]>,
}

fn validate_weights(weights: Weights) -> Result<()> {
    if weights.into_iter().map(u64::from).sum::<u64>() != u64::from(DENOMINATOR) {
        return Err(invalid(
            "certificate barycentric address is outside the simplex",
        ));
    }
    Ok(())
}

/// Equality is exact only when every contributing authored coordinate agrees.
/// In particular, a one-hot address preserves the decoded f32 vertex exactly.
fn reconstruct(face: &Face, weights: Weights) -> Result<[Interval; 3]> {
    validate_weights(weights)?;
    let mut result = [Interval::exact(0.); 3];
    for (axis, interval) in result.iter_mut().enumerate() {
        let mut exact_coordinate = None;
        let mut all_equal = true;
        let mut low = 0.0;
        let mut high = 0.0;
        for (vertex, weight) in face.iter().zip(weights) {
            if weight == 0 {
                continue;
            }
            if let Some(previous) = exact_coordinate {
                all_equal &= previous == vertex[axis];
            } else {
                exact_coordinate = Some(vertex[axis]);
            }
            let product = f64::from(vertex[axis]) * (f64::from(weight) * WEIGHT_SCALE);
            low = (low + product.next_down()).next_down();
            high = (high + product.next_up()).next_up();
        }
        *interval = if all_equal {
            Interval::exact(f64::from(
                exact_coordinate.ok_or_else(|| invalid("empty certificate address"))?,
            ))
        } else {
            Interval { low, high }
        };
        if !interval.low.is_finite() || !interval.high.is_finite() || interval.low > interval.high {
            return Err(invalid(
                "certificate coordinate interval is nonfinite or inverted",
            ));
        }
    }
    Ok(result)
}

fn source_corner(face: &Face, weights: Weights) -> Result<Corner> {
    let coordinates = reconstruct(face, weights)?;
    let first = weights
        .iter()
        .position(|&w| w != 0)
        .ok_or_else(|| invalid("empty source patch corner"))?;
    let exact_point = face
        .iter()
        .zip(weights)
        .all(|(p, w)| w == 0 || *p == face[first])
        .then_some(face[first]);
    // The rounded coordinate is untrusted and used only to propose witnesses.
    let proposal = std::array::from_fn(|axis| {
        face.iter()
            .zip(weights)
            .map(|(p, w)| f64::from(p[axis]) * (f64::from(w) * WEIGHT_SCALE))
            .sum()
    });
    Ok(Corner {
        coordinates,
        proposal,
        exact_point,
    })
}

fn witness_squared_bound(source: &Corner, target: &Face, weights: Weights) -> Result<f64> {
    let target = reconstruct(target, weights)?;
    let mut sum = 0.0_f64;
    for (source, target) in source.coordinates.into_iter().zip(target) {
        if source.low == source.high && source.low == target.low && target.low == target.high {
            continue; // Exact equal singleton coordinates have zero residual.
        }
        let low = (source.low - target.high).next_down();
        let high = (source.high - target.low).next_up();
        let residual = low.abs().max(high.abs());
        let squared = (residual * residual).next_up();
        sum = (sum + squared).next_up();
    }
    if !sum.is_finite() {
        return Err(invalid(
            "certificate distance interval arithmetic is nonfinite",
        ));
    }
    Ok(sum)
}

fn sub(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    std::array::from_fn(|i| a[i] - b[i])
}

fn dot(a: [f64; 3], b: [f64; 3]) -> f64 {
    (0..3).map(|i| a[i] * b[i]).sum()
}

fn dyadic(u: f64, v: f64) -> Option<Weights> {
    if !u.is_finite() || !v.is_finite() {
        return None;
    }
    let u = (u.clamp(0., 1.) * f64::from(DENOMINATOR)).round() as u32;
    let v = ((v.clamp(0., 1.) * f64::from(DENOMINATOR)).round() as u32).min(DENOMINATOR - u);
    Some([DENOMINATOR - u - v, u, v])
}

/// Vertex/edge/interior projections merely propose integer target witnesses.
/// Every proposed distance is reconstructed against both exact supports.
fn point_squared_bound(source: &Corner, target: &Face) -> Result<f64> {
    if source.exact_point.is_some_and(|p| target.contains(&p)) {
        return Ok(0.);
    }
    let p = source.proposal;
    let [a, b, c] = target.map(|v| v.map(f64::from));
    let mut best = f64::INFINITY;
    for weights in Patch::ROOT.corners {
        best = best.min(witness_squared_bound(source, target, weights)?);
    }
    for (start, end) in [(0, 1), (1, 2), (2, 0)] {
        let vertices = [a, b, c];
        let edge = sub(vertices[end], vertices[start]);
        let length_squared = dot(edge, edge);
        if length_squared > 0. {
            let t = dot(sub(p, vertices[start]), edge) / length_squared;
            if t.is_finite() {
                let amount = (t.clamp(0., 1.) * f64::from(DENOMINATOR)).round() as u32;
                let mut weights = [0; 3];
                weights[start] = DENOMINATOR - amount;
                weights[end] = amount;
                best = best.min(witness_squared_bound(source, target, weights)?);
            }
        }
    }
    let (e, f, r) = (sub(b, a), sub(c, a), sub(p, a));
    let (ee, ef, ff) = (dot(e, e), dot(e, f), dot(f, f));
    let determinant = ee * ff - ef * ef;
    if determinant > 0. {
        let (re, rf) = (dot(r, e), dot(r, f));
        if let Some(weights) = dyadic(
            (re * ff - rf * ef) / determinant,
            (rf * ee - re * ef) / determinant,
        ) {
            best = best.min(witness_squared_bound(source, target, weights)?);
        }
    }
    if !best.is_finite() {
        return Err(invalid("no finite explicit certificate witness"));
    }
    Ok(best)
}

fn patch_squared_bound(
    original: &Face,
    patch: &Patch,
    corners: &[Corner; 3],
    target: &Face,
) -> Result<f64> {
    // Only exact original corner supports authorize this complete-face shortcut.
    if patch.depth == 0
        && original.iter().all(|p| target.contains(p))
        && target.iter().all(|p| original.contains(p))
    {
        return Ok(0.);
    }
    let mut squared = 0.0_f64;
    for corner in corners {
        squared = squared.max(point_squared_bound(corner, target)?);
    }
    Ok(squared)
}

fn admit(
    original: &[Vec<Face>],
    proxy: &[Vec<Face>],
    limit: u64,
    check: &mut impl FnMut() -> Result<()>,
) -> Result<u64> {
    if original.is_empty() || original.len() != proxy.len() {
        return Err(invalid(
            "certificate region supports are empty or inconsistent",
        ));
    }
    let mut base = 0_u64;
    for (a, b) in original.iter().zip(proxy) {
        check()?;
        if a.is_empty() || b.is_empty() {
            return Err(invalid("certificate has empty region support"));
        }
        base = (a.len() as u64)
            .checked_mul(b.len() as u64)
            .and_then(|n| n.checked_mul(2))
            .and_then(|n| base.checked_add(n))
            .filter(|&n| n <= limit)
            .ok_or_else(|| {
                unsupported(&format!(
                    "root proxy certificate exceeds the admitted base face-pair work limit of {limit}"
                ))
            })?;
        for (index, face) in a.iter().chain(b).enumerate() {
            if index.is_multiple_of(1024) {
                check()?;
            }
            if face
                .iter()
                .flatten()
                .any(|p| !p.is_finite() || p.abs() > 1_000_000.)
            {
                return Err(invalid(
                    "certificate position is outside the admitted finite domain",
                ));
            }
        }
    }
    Ok(base)
}

struct Work {
    result: Certificate,
    limit: u64,
}

impl Work {
    fn new(limit: u64) -> Self {
        Self {
            result: Certificate {
                error_metres: 0.,
                patch_face_tests: 0,
                accepted_patches: 0,
                max_depth: 0,
            },
            limit,
        }
    }

    fn consume(&mut self, check: &mut impl FnMut() -> Result<()>) -> Result<()> {
        if self.result.patch_face_tests.is_multiple_of(64) {
            check()?;
        }
        self.result.patch_face_tests = self.result.patch_face_tests.checked_add(1)
            .filter(|&n| n <= self.limit)
            .ok_or_else(|| unsupported("root proxy could not certify the requested maximum within the patch-face work limit"))?;
        Ok(())
    }

    fn accept(&mut self, bound: f64, depth: u8) -> Result<()> {
        self.result.accepted_patches = self
            .result
            .accepted_patches
            .checked_add(1)
            .ok_or_else(|| invalid("certificate accepted-patch count overflowed"))?;
        self.result.error_metres = self.result.error_metres.max(bound);
        self.result.max_depth = self.result.max_depth.max(depth);
        Ok(())
    }
}

fn directed(
    source: &[Face],
    target: &[Face],
    budget: f64,
    work: &mut Work,
    depth_limit: u8,
    check: &mut impl FnMut() -> Result<()>,
) -> Result<()> {
    let mut pending = Vec::with_capacity(MAX_PENDING_PATCHES);
    for original in source {
        pending.push(Patch::ROOT);
        while let Some(patch) = pending.pop() {
            check()?;
            let corners = [
                source_corner(original, patch.corners[0])?,
                source_corner(original, patch.corners[1])?,
                source_corner(original, patch.corners[2])?,
            ];
            let mut best = f64::INFINITY;
            for face in target {
                work.consume(check)?;
                best = best.min(patch_squared_bound(original, &patch, &corners, face)?);
            }
            let bound = if best == 0. {
                0.
            } else {
                best.sqrt().next_up()
            };
            if !bound.is_finite() {
                return Err(invalid("complete-patch certificate is nonfinite"));
            }
            // Never square an arbitrary positive budget: it may under/overflow.
            if bound <= budget {
                work.accept(bound, patch.depth)?;
            } else if patch.depth >= depth_limit {
                return Err(unsupported("root proxy could not certify the requested maximum within the proof depth limit"));
            } else {
                pending.extend(patch.children()?.into_iter().rev());
                debug_assert!(pending.len() <= MAX_PENDING_PATCHES);
            }
        }
    }
    Ok(())
}

// Limits are private seams for meaningful boundary controls. Production selects
// one fixed profile through `certify`; callers cannot request another search.
fn certify_with_limits(
    original: &[Vec<Face>],
    proxy: &[Vec<Face>],
    budget: f64,
    limit: u64,
    depth_limit: u8,
    check: &mut impl FnMut() -> Result<()>,
) -> Result<Certificate> {
    check()?;
    debug_assert!(budget.is_finite() && budget > 0.);
    if depth_limit > MAX_DEPTH {
        return Err(invalid(
            "certificate profile exceeds exact patch address depth",
        ));
    }
    let base = admit(original, proxy, limit, check)?;
    let mut work = Work::new(limit);
    for (a, b) in original.iter().zip(proxy) {
        directed(a, b, budget, &mut work, depth_limit, check)?;
        directed(b, a, budget, &mut work, depth_limit, check)?;
    }
    check()?;
    debug_assert!(work.result.patch_face_tests >= base);
    Ok(work.result)
}

pub(super) fn certify(
    original: &[Vec<Face>],
    proxy: &[Vec<Face>],
    budget: f64,
    mut check: impl FnMut() -> Result<()>,
) -> Result<Certificate> {
    certify_with_limits(
        original,
        proxy,
        budget,
        MAX_PATCH_FACE_TESTS,
        MAX_DEPTH,
        &mut check,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn large() -> Face {
        [[0., 0., 0.], [8., 0., 0.], [0., 8., 0.]]
    }

    fn quarters() -> Vec<Face> {
        vec![
            [[0., 0., 0.], [4., 0., 0.], [0., 4., 0.]],
            [[4., 0., 0.], [8., 0., 0.], [4., 4., 0.]],
            [[0., 4., 0.], [4., 4., 0.], [0., 8., 0.]],
            [[4., 0., 0.], [4., 4., 0.], [0., 4., 0.]],
        ]
    }

    fn test_certificate(
        a: Vec<Face>,
        b: Vec<Face>,
        budget: f64,
        limit: u64,
        depth: u8,
    ) -> Result<Certificate> {
        certify_with_limits(&[a], &[b], budget, limit, depth, &mut || Ok(()))
    }

    fn signed_area(corners: [Weights; 3]) -> i64 {
        let [a, b, c] = corners.map(|p| p.map(i64::from));
        (b[1] - a[1]) * (c[2] - a[2]) - (b[2] - a[2]) * (c[1] - a[1])
    }

    #[test]
    fn canonical_children_preserve_exact_addresses_through_depth_24() {
        let root_area = signed_area(Patch::ROOT.corners);
        let children = Patch::ROOT.children().unwrap();
        assert_eq!(
            children.iter().map(|p| signed_area(p.corners)).sum::<i64>(),
            root_area
        );
        assert!(children
            .iter()
            .all(|p| signed_area(p.corners) == root_area / 4));
        // Canonical midpoint rows, not rounded positions, establish four regions.
        assert_eq!(
            children[0].corners,
            [
                [DENOMINATOR, 0, 0],
                [DENOMINATOR / 2, DENOMINATOR / 2, 0],
                [DENOMINATOR / 2, 0, DENOMINATOR / 2]
            ]
        );
        assert_eq!(
            children[3].corners,
            [
                [DENOMINATOR / 2, DENOMINATOR / 2, 0],
                [0, DENOMINATOR / 2, DENOMINATOR / 2],
                [DENOMINATOR / 2, 0, DENOMINATOR / 2]
            ]
        );
        for first in 0..4 {
            let mut patch = Patch::ROOT;
            for depth in 0..=MAX_DEPTH {
                for row in patch.corners {
                    assert_eq!(row.into_iter().sum::<u32>(), DENOMINATOR);
                    assert!(row
                        .into_iter()
                        .all(|n| n.is_multiple_of(1 << (MAX_DEPTH - depth))));
                }
                assert_eq!(signed_area(patch.corners), root_area >> (2 * depth));
                if depth < MAX_DEPTH {
                    patch = patch.children().unwrap()[(first + depth as usize) % 4];
                }
            }
            assert!(patch.children().is_err());
        }
    }

    #[test]
    fn subdivision_tightens_equal_surface_and_counts_all_target_scans() {
        let proof = test_certificate(vec![large()], quarters(), 1e-10, 24, 24).unwrap();
        assert!(proof.error_metres <= 1e-10);
        assert_eq!(proof.patch_face_tests, 24); // 5*4 forward, 4*1 reverse.
        assert_eq!(proof.accepted_patches, 8);
        assert_eq!(proof.max_depth, 1);
        let error = test_certificate(vec![large()], quarters(), 1e-10, 23, 24)
            .err()
            .unwrap();
        assert_eq!(error.kind(), JobErrorKind::Unsupported);
        let error = test_certificate(vec![large()], quarters(), 1e-10, 24, 0)
            .err()
            .unwrap();
        assert_eq!(error.kind(), JobErrorKind::Unsupported);
    }

    #[test]
    fn offset_spike_hole_and_degenerate_supports_have_truthful_limits() {
        let offset: Vec<_> = quarters()
            .into_iter()
            .map(|f| f.map(|[x, y, _]| [x, y, 0.125]))
            .collect();
        let proof = test_certificate(vec![large()], offset.clone(), 0.126, 1000, 24).unwrap();
        assert!(proof.error_metres >= 0.125 && proof.error_metres < 0.126);
        assert_eq!(
            test_certificate(vec![large()], offset, 0.1, 1000, 2)
                .err()
                .unwrap()
                .kind(),
            JobErrorKind::Unsupported
        );
        let point = [[[0.; 3]; 3]];
        let spike = [[[0., 0., 0.], [1., 0., 100.], [0., 1., 100.]]];
        let proof = test_certificate(spike.to_vec(), point.to_vec(), 101., 2, 0).unwrap();
        assert!(proof.error_metres >= 100.);
        let boundary = vec![
            [[0., 0., 0.], [8., 0., 0.], [0., 0., 0.]],
            [[8., 0., 0.], [0., 8., 0.], [8., 0., 0.]],
            [[0., 8., 0.], [0., 0., 0.], [0., 8., 0.]],
        ];
        assert_eq!(
            test_certificate(vec![large()], boundary, 0.01, 1000, 4)
                .err()
                .unwrap()
                .kind(),
            JobErrorKind::Unsupported
        );
        let proof = test_certificate(vec![[[3., 4., 0.]; 3]], point.to_vec(), 5.01, 2, 0).unwrap();
        assert!(proof.error_metres >= 5. && proof.error_metres < 5.01);
        // Equal point/line corner sets retain exact zero, even for tiny budgets.
        let line = [[0., 0., 0.], [8., 0., 0.], [0., 0., 0.]];
        let proof = test_certificate(
            vec![line],
            vec![[line[1], line[0], line[1]]],
            f64::MIN_POSITIVE,
            2,
            0,
        )
        .unwrap();
        assert_eq!(proof.error_metres, 0.);
    }

    #[test]
    fn mixed_subnormal_source_midpoint_is_an_interval_not_rounded_geometry() {
        let tiny = f32::from_bits(1);
        let face = [[1_000_000., 0., 0.], [tiny, 1., 0.], [0., 0., 0.]];
        let corner = source_corner(&face, [DENOMINATOR / 2, DENOMINATOR / 2, 0]).unwrap();
        let x = corner.coordinates[0];
        assert_eq!(corner.proposal[0], 500_000.);
        assert!(x.low < 500_000. && x.high > 500_000.);
        assert_eq!(corner.exact_point, None);
        let target = [[500_000., 0.5, 0.]; 3];
        let squared = point_squared_bound(&corner, &target).unwrap();
        assert!(squared > 0. && squared < 1e-12);
        let proof = test_certificate(
            vec![face],
            vec![[face[2], face[1], face[0]]],
            f64::from_bits(1),
            2,
            24,
        )
        .unwrap();
        assert_eq!(proof.error_metres, 0.);
    }

    #[test]
    fn explicit_lattice_witness_does_not_claim_zero_true_distance() {
        let point = [[1., 0., 0.]; 3];
        let source = source_corner(&point, Patch::ROOT.corners[0]).unwrap();
        let line = [[0., 0., 0.], [1_000_000., 0., 0.], [0., 0., 0.]];
        let squared = point_squared_bound(&source, &line).unwrap();
        let ideal_lattice_residual = 3481. / 262144.;
        assert!(squared >= ideal_lattice_residual * ideal_lattice_residual);
        assert!(squared.sqrt() < 0.013279);
        assert!(dyadic(f64::NAN, 0.).is_none());
        assert!(reconstruct(&line, [1, 2, 3]).is_err());
    }

    #[test]
    fn base_dynamic_and_global_work_boundaries_are_checked_before_work() {
        let face = large();
        let regions = vec![vec![face], vec![face]];
        let mut work = 0;
        assert_eq!(admit(&regions, &regions, 4, &mut || Ok(())).unwrap(), 4);
        assert_eq!(
            admit(&regions, &regions, 3, &mut || Ok(()))
                .err()
                .unwrap()
                .kind(),
            JobErrorKind::Unsupported
        );
        let proof = certify_with_limits(&regions, &regions, 1., 4, 24, &mut || Ok(())).unwrap();
        assert_eq!(proof.patch_face_tests, 4);
        let mut counter = Work::new(3);
        for _ in 0..3 {
            counter
                .consume(&mut || {
                    work += 1;
                    Ok(())
                })
                .unwrap();
        }
        assert!(counter.consume(&mut || Ok(())).is_err());
        assert_eq!(counter.result.patch_face_tests, 3);
        assert_eq!(work, 1);
        let many = vec![vec![face; 4096]];
        let other = vec![vec![face; 2048]];
        assert_eq!(
            admit(&many, &other, MAX_PATCH_FACE_TESTS, &mut || Ok(())).unwrap(),
            MAX_PATCH_FACE_TESTS
        );
        let over = vec![vec![face; 2049]];
        assert_eq!(
            admit(&many, &over, MAX_PATCH_FACE_TESTS, &mut || Ok(()))
                .err()
                .unwrap()
                .kind(),
            JobErrorKind::Unsupported
        );
        assert_eq!(
            certify(&[], &[], 1., || Ok(())).err().unwrap().kind(),
            JobErrorKind::InvalidState
        );
        assert_eq!(
            certify(&[vec![face]], &[vec![[[f32::NAN; 3]; 3]]], 1., || Ok(()))
                .err()
                .unwrap()
                .kind(),
            JobErrorKind::InvalidState
        );
        for invalid_face in [[[f32::INFINITY; 3]; 3], [[1_000_001.; 3]; 3]] {
            // Equal invalid supports must not reach the exact-zero shortcut.
            assert_eq!(
                certify(&[vec![invalid_face]], &[vec![invalid_face]], 1., || Ok(()))
                    .err()
                    .unwrap()
                    .kind(),
                JobErrorKind::InvalidState
            );
        }
    }

    #[test]
    fn cancellation_interrupts_long_scans_and_after_the_last_test() {
        let face = large();
        let mut work = Work::new(1000);
        let mut checkpoints = 0;
        let control = crate::RunControl::default();
        let cancellation = control.cancellation_handle();
        let attempt = control.begin().unwrap();
        let error = directed(&[face], &vec![face; 130], 1., &mut work, 24, &mut || {
            checkpoints += 1;
            if checkpoints == 3 {
                assert!(cancellation.cancel());
            }
            attempt.check()
        })
        .err()
        .unwrap();
        assert_eq!(error.kind(), JobErrorKind::Cancelled);
        assert_eq!(work.result.patch_face_tests, 64);
        let failure = attempt.fail(error);
        assert_eq!(failure.error.kind(), JobErrorKind::Cancelled);
        assert!(failure.secondary.is_empty());
        assert!(failure.retained_paths.is_empty());
        // First establish the complete control's checkpoint count, then abort
        // exactly at its final checkpoint to ensure completed math cannot win.
        let mut calls = 0;
        certify(&[vec![face]], &[vec![face]], 1., || {
            calls += 1;
            Ok(())
        })
        .unwrap();
        let mut seen = 0;
        let error = certify(&[vec![face]], &[vec![face]], 1., || {
            seen += 1;
            if seen == calls {
                Err(invalid("final checkpoint abort"))
            } else {
                Ok(())
            }
        })
        .err()
        .unwrap();
        assert_eq!(error.kind(), JobErrorKind::InvalidState);
        assert_eq!(seen, calls);
    }
}
