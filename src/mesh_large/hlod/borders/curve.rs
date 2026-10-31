//! Linear-work shortcut selection on ordered source arcs. The parameter is the original
//! edge ordinal, not nearest-curve distance. An accepted chord stays inside every source
//! vertex's three coordinate corridors; convexity covers each intervening source edge.
//! Final certificates independently visit every source breakpoint once more. No recursive
//! Douglas–Peucker rescans, owner-pair enumeration or child-relative error estimates.
use crate::{JobError, JobErrorKind};

pub(super) fn state(message: &str) -> JobError {
    JobError::new(JobErrorKind::InvalidState, message)
}

/// A predetermined, globally shared ladder. Depth increases towards the leaves.
#[derive(Clone, Copy, Debug)]
pub(super) struct Ladder {
    root: f64,
}
impl Ladder {
    pub(super) fn new(extent: f64) -> Result<Self, JobError> {
        if !extent.is_finite() || extent < 0. {
            return Err(state("invalid shared-border scale"));
        }
        Ok(Self {
            root: extent / 1024.,
        })
    }
    pub(super) fn error(self, depth: u32) -> f64 {
        self.root * 2f64.powi(-(depth as i32))
    }
    pub(super) fn epsilon(self, depth: u32) -> f64 {
        self.error(depth) / 16.
    }
}

/// Rounding allowance for f64 affine arithmetic over f32 coordinates and u64 ordinal
/// parameters (including ordinal conversion and division). Retained source edges need
/// no allowance: their complete piecewise-linear parameterization is identical.
fn rounding(a: [f32; 3], b: [f32; 3]) -> f64 {
    a.into_iter()
        .chain(b)
        .map(|x| f64::from(x).abs())
        .fold(1., f64::max)
        * (128. * f64::EPSILON)
}

/// Greedy coordinate corridors. Each source point is read a constant number of times;
/// `emit` receives inclusive endpoint ordinals and an outward displacement certificate.
/// Splitting into pinned subarcs (junctions, terminal locks, cycle anchors) is the caller's
/// responsibility. Reads may be positioned disk reads; only constant state is resident.
pub(super) fn shortcuts(
    first: u64,
    last: u64,
    epsilon: f64,
    point: &mut impl FnMut(u64) -> Result<[f32; 3], JobError>,
    retained: &mut impl FnMut(u64) -> Result<bool, JobError>,
    emit: &mut impl FnMut(u64, u64, f64) -> Result<(), JobError>,
) -> Result<(), JobError> {
    if first >= last || !epsilon.is_finite() || epsilon < 0. {
        return Err(state("invalid source arc or shortcut tolerance"));
    }
    // One common arithmetic allowance for the whole arc. An endpoint with larger
    // absolute coordinates must not retroactively invalidate earlier corridors.
    let mut guard = 0f64;
    for i in first..=last {
        let p = point(i)?;
        guard = guard.max(rounding(p, p));
    }
    let mut start = first;
    while start < last {
        let a = point(start)?;
        let mut lower = [f64::NEG_INFINITY; 3];
        let mut upper = [f64::INFINITY; 3];
        let mut end = start;
        let mut i = start + 1;
        while i <= last {
            let b = point(i)?;
            let radius = (epsilon - 2. * guard).max(0.) / 3f64.sqrt();
            let n = (i - start) as f64;
            let mut fits = epsilon >= 2. * guard;
            for axis in 0..3 {
                let delta = f64::from(b[axis]) - f64::from(a[axis]);
                lower[axis] = lower[axis].max(((delta - radius) / n).next_up());
                upper[axis] = upper[axis].min(((delta + radius) / n).next_down());
                let slope = delta / n;
                fits &= lower[axis] <= slope && slope <= upper[axis];
            }
            // An original source edge always fits exactly. Failed extension finalizes the
            // last accepted endpoint, then starts the next chord there.
            if retained(i)? {
                if end != start && !fits {
                    break;
                }
                end = i;
            }
            i += 1;
        }
        if end == start {
            return Err(state("source arc has no retained endpoint"));
        }
        let b = point(end)?;
        let mut distance_max = 0f64;
        let mut local_guard = rounding(a, b);
        for i in start + 1..end {
            let p = point(i)?;
            let t = (i - start) as f64 / (end - start) as f64;
            let distance = (0..3)
                .map(|axis| {
                    let q = f64::from(a[axis]) * (1. - t) + f64::from(b[axis]) * t;
                    (q - f64::from(p[axis])).powi(2)
                })
                .sum::<f64>()
                .sqrt();
            distance_max = distance_max.max(distance);
            local_guard = local_guard.max(rounding(p, p));
        }
        // The first retained chord may come from a finer level. A distant large-coordinate
        // part of a newly joined chain must not invalidate that already certified chord.
        // Selection uses the conservative whole-chain guard; certification uses this arc.
        let bound = if end == start + 1 {
            0.
        } else {
            (distance_max + local_guard).next_up()
        };
        if bound > epsilon {
            return Err(state("shortcut exceeds its source-arc displacement budget"));
        }
        emit(start, end, bound)?;
        start = end;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn contract(points: &[[f32; 3]], epsilon: f64) -> (Vec<(u64, u64, f64)>, usize) {
        let mut result = Vec::new();
        let mut reads = 0;
        shortcuts(
            0,
            points.len() as u64 - 1,
            epsilon,
            &mut |i| {
                reads += 1;
                Ok(points[i as usize])
            },
            &mut |_| Ok(true),
            &mut |a, b, e| {
                result.push((a, b, e));
                Ok(())
            },
        )
        .unwrap();
        (result, reads)
    }

    #[test]
    fn a_joined_chain_cannot_invalidate_its_finer_retained_chord() {
        let points = [[0., 0., 0.], [1., 0., 0.], [2., 0., 0.], [1e20, 0., 0.]];
        let mut chords = Vec::new();
        shortcuts(
            0,
            3,
            1e-5,
            &mut |i| Ok(points[i as usize]),
            &mut |i| Ok(i != 1),
            &mut |a, b, e| {
                chords.push((a, b, e));
                Ok(())
            },
        )
        .unwrap();
        assert_eq!(chords.len(), 2);
        assert_eq!((chords[0].0, chords[0].1), (0, 2));
        assert!(chords[0].2 <= 1e-5);
        assert_eq!(chords[1], (2, 3, 0.));
    }
    #[test]
    fn straight_sheet_borders_do_not_retain_source_density() {
        for count in [64, 1024, 32768] {
            let points: Vec<_> = (0..=count).map(|i| [i as f32, 2., 0.]).collect();
            let (chords, reads) = contract(&points, 1e-5);
            assert_eq!(chords.len(), 1);
            assert!(reads <= 4 * points.len());
            assert!(chords[0].2 <= 1e-5);
        }
    }

    #[test]
    fn large_source_ordinals_keep_linear_point_reads() {
        // Exercise production ordinal subtraction and certification without materializing
        // a billion-element arc. Admission at those source sizes is tested in borders.
        for first in [500_000_000u64, 1_000_000_000] {
            let last = first + 1024;
            let mut reads = 0;
            let mut chords = Vec::new();
            shortcuts(
                first,
                last,
                1e-5,
                &mut |i| {
                    reads += 1;
                    Ok([(i - first) as f32, 2., 0.])
                },
                &mut |_| Ok(true),
                &mut |a, b, bound| {
                    chords.push((a, b, bound));
                    Ok(())
                },
            )
            .unwrap();
            assert_eq!(chords.len(), 1);
            assert_eq!((chords[0].0, chords[0].1), (first, last));
            assert!(chords[0].2 <= 1e-5);
            assert!(reads <= 4 * (last - first + 1));
        }
    }

    #[test]
    fn oscillations_and_reversed_arcs_have_complete_certified_coverage() {
        let points: Vec<_> = (0..2048).map(|i| [i as f32, (i % 2) as f32, 0.]).collect();
        for reverse in [false, true] {
            let mut p = points.clone();
            if reverse {
                p.reverse();
            }
            for epsilon in [0., 0.01, 2.] {
                let (chords, reads) = contract(&p, epsilon);
                assert!(reads <= 7 * p.len());
                let mut next = 0;
                for (a, b, bound) in chords {
                    assert_eq!(a, next);
                    assert!(a < b && bound <= epsilon);
                    for i in a..=b {
                        let t = (i - a) as f64 / (b - a) as f64;
                        let distance = (0..3)
                            .map(|k| {
                                let q = f64::from(p[a as usize][k]) * (1. - t)
                                    + f64::from(p[b as usize][k]) * t;
                                (q - f64::from(p[i as usize][k])).powi(2)
                            })
                            .sum::<f64>()
                            .sqrt();
                        assert!(distance <= bound || b == a + 1);
                    }
                    next = b;
                }
                assert_eq!(next, p.len() as u64 - 1);
            }
        }
    }

    #[test]
    fn every_depth_difference_is_bounded_despite_unequal_neighbor_errors() {
        let ladder = Ladder::new(1000.).unwrap();
        for coarse in 0..31 {
            for fine in coarse + 1..32 {
                let coarse_g = ladder.error(coarse);
                // The neighboring sampled G can be arbitrarily greater or smaller.
                for neighbor_g in [0., 1e6] {
                    let fine_g = ladder.error(fine).max(neighbor_g);
                    assert!(fine_g >= ladder.error(fine));
                    assert!(ladder.epsilon(coarse) + ladder.epsilon(fine) <= coarse_g / 8.);
                }
            }
        }
    }
}
