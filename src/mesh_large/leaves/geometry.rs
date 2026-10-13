//! Frozen dataset-wide leaf/proxy position encoding; no triangle corner is removed.
//! The encoding origin is ZERO across all implicit forest roots. Quantized glTF
//! nodes use only diagonal `scale` and exact aligned `translation`, with identity
//! local ancestors. `u * scale + translation` is the canonical local f32 decode;
//! FMA and flattening identity parents produce the same correctly rounded sum.
//! Geographic placement / viewer RTC matrices are outside this local proof.
use super::super::{config::Profile, records::Bounds};
use crate::{JobError, JobErrorKind};
use serde::Serialize;

const ALIGNMENT: i64 = 1 << 15;
const GLOBAL_LIMIT: i64 = (1 << 38) - 65536;
const SNAPPED_LIMIT: i64 = 1 << 38;
const LOCAL_LIMIT: f32 = 1_000_000.0;

#[derive(Clone, Debug, Serialize)]
pub(crate) struct LatticePlan {
    version: u32,
    profile: Profile,
    /// Fixed zero origin, never a separately rounded nonzero parent translation.
    common_origin: [f32; 3],
    steps: Vec<f32>,
    depth_extents: Vec<f64>,
    source_bounds: Bounds,
}
#[derive(Clone, Debug)]
pub(crate) enum PositionStorage {
    Quantized {
        codes: Vec<[u16; 3]>,
        translation: [f32; 3],
        scale: f32,
    },
    Float {
        positions: Vec<[f32; 3]>,
    },
}
#[derive(Clone, Debug)]
pub(crate) struct EncodedPositions {
    pub storage: PositionStorage,
    pub common_origin: [f32; 3],
    pub decoded: Vec<[f32; 3]>,
    pub bounds: Bounds,
    /// Outward max vertex correspondence displacement. Barycentric pairing
    /// bounds both triangle-surface directions, including point/segment supports.
    pub correspondence_error: f64,
}
fn failure(kind: JobErrorKind, message: &str) -> JobError {
    JobError::new(kind, message.to_owned())
}
fn request(message: &str) -> JobError {
    failure(JobErrorKind::InvalidRequest, message)
}
fn state(message: &str) -> JobError {
    failure(JobErrorKind::InvalidState, message)
}
fn resource(message: &str) -> JobError {
    failure(JobErrorKind::ResourceLimit, message)
}
fn up(x: f64) -> f64 {
    if x == f64::INFINITY {
        x
    } else if x == 0.0 {
        f64::from_bits(1)
    } else if x > 0.0 {
        f64::from_bits(x.to_bits() + 1)
    } else {
        f64::from_bits(x.to_bits() - 1)
    }
}
fn canonical_zero(x: f32) -> f32 {
    if x == 0.0 {
        0.0
    } else {
        x
    }
}
fn normal_power_two(step: f32) -> bool {
    step.is_normal() && step > 0.0 && step.to_bits() & 0x007f_ffff == 0
}
fn power_step(required: f64) -> Result<f32, JobError> {
    if !required.is_finite() || required < 0.0 {
        return Err(resource(
            "position lattice required-step arithmetic exhausted",
        ));
    }
    // Compare exact f32 power-of-two values instead of approximating log2.
    let mut step = f32::MIN_POSITIVE;
    while f64::from(step) < required {
        step *= 2.0;
        if !step.is_finite() {
            return Err(resource("position lattice step overflow"));
        }
    }
    Ok(step)
}

impl LatticePlan {
    pub(crate) fn steps(&self) -> &[f32] {
        &self.steps
    }
    /// Extents are maximum INDIVIDUAL actual-node extents at each accepted LOD
    /// depth, including prospective proxies, packed leaves and synthetic runs.
    /// Supply zero for empty depths. Aggregate ALL forest roots in this one plan.
    /// Snap growth and aligned-translation slack are frozen BEFORE leaf encoding.
    pub(crate) fn new(
        profile: Profile,
        source_bounds: Bounds,
        depth_extents: &[f64],
    ) -> Result<Self, JobError> {
        if depth_extents.is_empty() || depth_extents.len() > 32 {
            return Err(request("position lattice requires 1..=32 depths"));
        }
        if (0..3).any(|a| {
            !source_bounds.min[a].is_finite()
                || !source_bounds.max[a].is_finite()
                || source_bounds.min[a] > source_bounds.max[a]
                || source_bounds.min[a].abs() > LOCAL_LIMIT
                || source_bounds.max[a].abs() > LOCAL_LIMIT
        }) || depth_extents
            .iter()
            .any(|x| !x.is_finite() || *x < 0.0 || *x > 2.0 * f64::from(LOCAL_LIMIT))
        {
            return Err(request(
                "position lattice invalid admitted local bounds/extents",
            ));
        }
        let mut plan = Self {
            version: 1,
            profile,
            common_origin: [0.0; 3],
            depth_extents: depth_extents.to_vec(),
            steps: vec![1.0; depth_extents.len()],
            source_bounds,
        };
        if matches!(profile, Profile::Quality | Profile::Lossless) {
            return Ok(plan);
        }
        let radius = source_bounds
            .min
            .iter()
            .chain(&source_bounds.max)
            .map(|p| f64::from(p.abs()))
            .fold(0.0_f64, f64::max);
        // The global ceiling is a slack policy, not an exact-sum requirement.
        // It admits signed q well beyond 2^24 and does not depend on tree depth.
        let minimum = power_step(up(radius / GLOBAL_LIMIT as f64))?;
        for (step, extent) in plan.steps.iter_mut().zip(depth_extents) {
            *step = power_step(up(*extent / ALIGNMENT as f64))?.max(minimum);
        }
        for _ in 0..512 {
            for d in (0..plan.steps.len().saturating_sub(1)).rev() {
                plan.steps[d] = plan.steps[d].max(plan.steps[d + 1]);
            }
            let growth = f64::from(plan.steps[0]);
            let old = plan.steps.clone();
            for (step, extent) in plan.steps.iter_mut().zip(depth_extents) {
                // Both endpoints move <=s0/2 on each axis. Reserve <=32768
                // codes for span, <=32767 for aligned floor-translation slack.
                *step = (*step).max(power_step(up(up(*extent + growth) / ALIGNMENT as f64))?);
            }
            if old == plan.steps {
                return Ok(plan);
            }
        }
        Err(resource("position lattice fit arithmetic did not converge"))
    }

    /// points contains complete triangle corners in original order. ell is the
    /// EXACT global minimum incident leaf depth, including all forest roots.
    /// Future proxies may have d<ell: they quantize on their own depth lattice.
    pub(crate) fn encode(
        &self,
        points: &[[f32; 3]],
        minimum_leaf_depths: &[u8],
        depth: u32,
    ) -> Result<EncodedPositions, JobError> {
        let d = usize::try_from(depth).map_err(|_| request("position depth overflow"))?;
        if points.is_empty()
            || !points.len().is_multiple_of(3)
            || points.len() != minimum_leaf_depths.len()
            || d >= self.steps.len()
            || minimum_leaf_depths
                .iter()
                .any(|ell| usize::from(*ell) >= self.steps.len())
        {
            return Err(request("position triangle/depth correspondence invalid"));
        }
        if self.common_origin != [0.0; 3]
            || self.steps.windows(2).any(|s| s[0] < s[1])
            || self.steps.iter().any(|s| !normal_power_two(*s))
        {
            return Err(state("position frozen lattice/origin was modified"));
        }
        if points.iter().any(|p| {
            (0..3).any(|a| {
                !p[a].is_finite()
                    || p[a] < self.source_bounds.min[a]
                    || p[a] > self.source_bounds.max[a]
            })
        }) {
            return Err(request("positions outside frozen source domain"));
        }
        let (storage, decoded) = if matches!(self.profile, Profile::Quality | Profile::Lossless) {
            // H1 normalizes zero; normalize defensively at this boundary too.
            let positions: Vec<[f32; 3]> = points.iter().map(|p| p.map(canonical_zero)).collect();
            let decoded = positions.clone();
            (PositionStorage::Float { positions }, decoded)
        } else {
            let scale = self.steps[d];
            let mut global = Vec::with_capacity(points.len());
            for (p, ell) in points.iter().zip(minimum_leaf_depths) {
                let snap = self.steps[d.min(usize::from(*ell))];
                let mut q = [0_i64; 3];
                for a in 0..3 {
                    q[a] = snapped_code(p[a], snap, scale)?;
                }
                global.push(q);
            }
            let qmin: [i64; 3] =
                std::array::from_fn(|a| global.iter().map(|q| q[a]).min().unwrap());
            let qmax: [i64; 3] =
                std::array::from_fn(|a| global.iter().map(|q| q[a]).max().unwrap());
            if (0..3).any(|a| qmax[a] - qmin[a] > ALIGNMENT) {
                return Err(state("position frozen plan did not fit source/snap span"));
            }
            let offset: [i64; 3] = qmin.map(|q| q.div_euclid(ALIGNMENT) * ALIGNMENT);
            let translation: [f32; 3] = std::array::from_fn(|a| offset[a] as f32 * scale);
            // |t/A|<=2^23 and A is a power of two: t itself is exact f32.
            if offset
                .iter()
                .any(|t| t.abs() > SNAPPED_LIMIT || *t as f32 as i64 != *t)
                || translation.iter().any(|t| !t.is_finite())
            {
                return Err(state("position frozen aligned translation lost exactness"));
            }
            let mut codes = Vec::with_capacity(points.len());
            let mut decoded = Vec::with_capacity(points.len());
            for q in global {
                let mut u = [0_u16; 3];
                let mut p = [0_f32; 3];
                for a in 0..3 {
                    u[a] = u16::try_from(q[a] - offset[a])
                        .map_err(|_| state("position frozen aligned range exceeds u16"))?;
                    // u*s and t*s are EXACT, while q*s need not be exact f32.
                    // RN(exact u*s + exact t*s) is independent of chosen t.
                    p[a] = canonical_zero(f32::from(u[a]) * scale + translation[a]);
                    let canonical = canonical_zero((q[a] as f64 * f64::from(scale)) as f32);
                    let fused = canonical_zero(f32::from(u[a]).mul_add(scale, translation[a]));
                    if !p[a].is_finite()
                        || p[a].to_bits() != canonical.to_bits()
                        || fused.to_bits() != canonical.to_bits()
                    {
                        return Err(state("position client-order rounded decode differs from canonical lattice point"));
                    }
                }
                codes.push(u);
                decoded.push(p);
            }
            (
                PositionStorage::Quantized {
                    codes,
                    translation,
                    scale,
                },
                decoded,
            )
        };
        let mut bounds = Bounds::EMPTY;
        let mut error = 0.0_f64;
        for (source, final_point) in points.iter().zip(&decoded) {
            bounds.include(*final_point);
            let mut squared = 0.0;
            for a in 0..3 {
                let difference = (f64::from(source[a]) - f64::from(final_point[a])).abs();
                if difference != 0.0 {
                    let delta = up(difference);
                    squared = up(squared + up(delta * delta));
                }
            }
            if squared != 0.0 {
                error = error.max(up(squared.sqrt()));
            }
        }
        Ok(EncodedPositions {
            storage,
            common_origin: [0.0; 3],
            decoded,
            bounds,
            correspondence_error: error,
        })
    }
}

/// No arbitrary-width arithmetic is needed in this bounded domain: converting a
/// f32 to f64 is exact; division by normal 2^k changes only its binary exponent.
/// The quotient is exactly representable in f64 (<=24 significant bits, exponent
/// range within f64), and |quotient|<=2^38-65536. Nearest-even produces an exact
/// <=39-bit integer. Multiplying it by nested power-of-two snap/scale is exact
/// once the resulting integer is checked <=2^38. This check precedes i64 casts.
fn snapped_code(position: f32, snap: f32, scale: f32) -> Result<i64, JobError> {
    let quotient = f64::from(position) / f64::from(snap);
    if !quotient.is_finite() || quotient.abs() > GLOBAL_LIMIT as f64 {
        return Err(state(
            "position frozen zero-origin code exceeded source allowance",
        ));
    }
    let rounded = quotient.round_ties_even();
    let ratio = f64::from(snap) / f64::from(scale);
    let code = rounded * ratio;
    if !code.is_finite() {
        return Err(resource("position dyadic code arithmetic overflow"));
    }
    if ratio < 1.0 || code.abs() > SNAPPED_LIMIT as f64 || code.fract() != 0.0 {
        return Err(state(
            "position frozen nested snapped code exceeded alignment allowance",
        ));
    }
    Ok(code as i64)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn domain() -> Bounds {
        Bounds {
            min: [-1_000_000.0; 3],
            max: [1_000_000.0; 3],
        }
    }
    fn bits(points: &[[f32; 3]]) -> Vec<[u32; 3]> {
        points.iter().map(|p| p.map(f32::to_bits)).collect()
    }
    fn local_triangle() -> [[f32; 3]; 3] {
        [
            [-999_999.75, -1_000_000.0 + 0.125, -999_999.5],
            [-999_999.25, -999_999.75, -999_999.0],
            [-999_999.5, -999_999.25, -999_999.75],
        ]
    }
    #[test]
    fn depth12_negative_noninteger_global_codes_beyond_old_limit() {
        let mut extents = vec![1.0; 32];
        extents[0] = 2_000_000.0;
        let plan = LatticePlan::new(Profile::Balanced, domain(), &extents).unwrap();
        let source = local_triangle();
        let code = snapped_code(source[0][0], plan.steps[12], plan.steps[12]).unwrap();
        assert!(code < -(1 << 24)); // Old-domain refusal is a sensitive negative control.
        let a = plan.encode(&source, &[12; 3], 12).unwrap();
        let b = plan.encode(&source, &[12; 3], 31).unwrap();
        assert_eq!(bits(&a.decoded), bits(&b.decoded));
        assert_eq!(a.common_origin.map(f32::to_bits), [0; 3]);
    }
    #[test]
    fn unequal_lattices_mixed_corner_and_proxy_before_leaf() {
        let mut extents = vec![0.001; 32];
        extents[0] = 2_000_000.0;
        extents[1] = 100.0;
        let plan = LatticePlan::new(Profile::Fast, domain(), &extents).unwrap();
        assert!(plan.steps[1] > plan.steps[31]);
        let source = [[-999_999.75, -999_999.5, -999_999.25]; 3];
        let a = plan.encode(&source, &[1; 3], 1).unwrap();
        let b = plan.encode(&source, &[1; 3], 31).unwrap();
        assert_eq!(bits(&a.decoded), bits(&b.decoded));
        // d<ell is valid for future proxies; it uses depth d, not leaf snap.
        let proxy = plan.encode(&source, &[31; 3], 1).unwrap();
        assert_eq!(bits(&proxy.decoded), bits(&a.decoded));
        assert_eq!(b.decoded.len(), 3); // Degenerate support/corner identity retained.
    }
    #[test]
    fn off_lattice_shared_corners_use_shallowest_incidence_in_decoded_bytes() {
        let bounds = Bounds {
            min: [-1.0; 3],
            max: [1.0; 3],
        };
        let mut extents = vec![0.001; 32];
        extents[0] = 2.0;
        extents[3] = 0.25;
        extents[9] = 0.01;
        let points = [[-0.00137, 0.00271, -0.00319]; 3];
        for profile in [Profile::Fast, Profile::Balanced] {
            let plan = LatticePlan::new(profile, bounds, &extents).unwrap();
            let shallow = plan.encode(&points, &[3; 3], 3).unwrap();
            // Mixed-depth, packed, and synthetic-depth consumers share the same
            // off-lattice source corner and its shallowest incident depth.
            for depth in [9, 21, 31] {
                assert!(plan.steps[3] > plan.steps[depth as usize]);
                let shared = plan.encode(&points, &[3; 3], depth).unwrap();
                let local = plan.encode(&points, &[depth as u8; 3], depth).unwrap();
                let decoded_bytes = |encoded: &EncodedPositions| match &encoded.storage {
                    PositionStorage::Quantized {
                        codes,
                        translation,
                        scale,
                    } => codes
                        .iter()
                        .flat_map(|corner| {
                            (0..3).flat_map(move |axis| {
                                let raw = corner[axis].to_le_bytes();
                                let value = u16::from_le_bytes(raw);
                                let decoded = (f64::from(value) * f64::from(*scale)
                                    + f64::from(translation[axis]))
                                    as f32;
                                decoded.to_le_bytes()
                            })
                        })
                        .collect::<Vec<_>>(),
                    _ => panic!("quantized profile required"),
                };
                assert_eq!(decoded_bytes(&shared), decoded_bytes(&shallow));
                assert_ne!(decoded_bytes(&shared), decoded_bytes(&local));
                // A proxy shallower than all incident leaves must snap on its
                // own coarser lattice, rather than on the leaf's lattice.
                let proxy = plan.encode(&points, &[depth as u8; 3], 3).unwrap();
                assert_eq!(decoded_bytes(&proxy), decoded_bytes(&shallow));
                assert_ne!(decoded_bytes(&proxy), decoded_bytes(&local));
            }
        }
    }
    #[test]
    fn alternate_exact_translations_same_rounded_sum_and_fma() {
        for q in [(1_i64 << 30) + 1, -(1_i64 << 30) + 3, (1_i64 << 38) - 40000] {
            let scale = 2_f32.powi(-20);
            let t = q.div_euclid(ALIGNMENT) * ALIGNMENT;
            let expected = (q as f64 * f64::from(scale)) as f32;
            for translation_code in [t, t - ALIGNMENT] {
                let u = u16::try_from(q - translation_code).unwrap();
                let translation = translation_code as f32 * scale;
                assert_eq!(translation_code as f32 as i64, translation_code);
                assert_eq!(
                    (f32::from(u) * scale + translation).to_bits(),
                    expected.to_bits()
                );
                assert_eq!(
                    f32::from(u).mul_add(scale, translation).to_bits(),
                    expected.to_bits()
                );
                // Flattened local affine evaluated in f64 then stored f32.
                assert_eq!(
                    (f64::from(u) * f64::from(scale) + f64::from(translation)) as f32,
                    expected
                );
            }
        }
    }
    #[test]
    fn packed_actual_extent_synthetic_and_cross_root_shared_plan() {
        let mut extents = vec![1.0; 32];
        extents[0] = 2_000_000.0;
        extents[12] = 200.0;
        extents[31] = 200.0;
        let plan = LatticePlan::new(Profile::Balanced, domain(), &extents).unwrap();
        let points = local_triangle();
        let root_a = plan.encode(&points, &[12; 3], 12).unwrap();
        let root_b = plan.clone().encode(&points, &[12; 3], 31).unwrap();
        assert_eq!(bits(&root_a.decoded), bits(&root_b.decoded));
        assert!(plan.steps.windows(2).all(|s| s[0] >= s[1]));
        for (step, extent) in plan.steps.iter().zip(&extents) {
            assert!(f64::from(*step) * ALIGNMENT as f64 >= *extent + f64::from(plan.steps[0]));
        }
    }
    #[test]
    fn negative_ties_and_stable_raw_float_profiles() {
        assert_eq!(snapped_code(-2.5, 1.0, 1.0).unwrap(), -2);
        assert_eq!(snapped_code(-3.5, 1.0, 1.0).unwrap(), -4);
        for profile in [Profile::Quality, Profile::Lossless] {
            let plan = LatticePlan::new(profile, domain(), &[1.0; 32]).unwrap();
            let source = local_triangle();
            let a = plan.encode(&source, &[31; 3], 0).unwrap();
            let b = plan.encode(&source, &[31; 3], 31).unwrap();
            assert_eq!(bits(&source), bits(&a.decoded));
            assert_eq!(bits(&a.decoded), bits(&b.decoded));
            assert_eq!(a.correspondence_error, 0.0);
        }
    }
    #[test]
    fn decoded_client_order_and_outward_correspondence() {
        let plan = LatticePlan::new(Profile::Balanced, domain(), &[2_000_000.0, 1.0]).unwrap();
        let source = local_triangle();
        let out = plan.encode(&source, &[0, 1, 1], 1).unwrap();
        // Coarse source[0] snap may span <=s0, already in the frozen envelope.
        if let PositionStorage::Quantized {
            codes,
            translation,
            scale,
        } = &out.storage
        {
            for (i, u) in codes.iter().enumerate() {
                let p: [f32; 3] = std::array::from_fn(|a| {
                    canonical_zero(f32::from(u[a]) * scale + translation[a])
                });
                assert_eq!(p.map(f32::to_bits), out.decoded[i].map(f32::to_bits));
                let distance = (0..3)
                    .map(|a| (f64::from(source[i][a]) - f64::from(p[a])).powi(2))
                    .sum::<f64>()
                    .sqrt();
                assert!(out.correspondence_error >= distance);
                for (a, value) in p.iter().enumerate() {
                    assert!(*value >= out.bounds.min[a] && *value <= out.bounds.max[a]);
                }
            }
        } else {
            panic!("expected quantized storage");
        }
    }
    #[test]
    fn malformed_requests_and_frozen_fit_defect_are_distinct() {
        let mut extents = vec![1.0; 32];
        extents[0] = 2_000_000.0;
        let plan = LatticePlan::new(Profile::Fast, domain(), &extents).unwrap();
        assert_eq!(
            plan.encode(&local_triangle()[..2], &[0; 2], 0)
                .unwrap_err()
                .kind(),
            JobErrorKind::InvalidRequest
        );
        assert!(plan.encode(&local_triangle(), &[32; 3], 0).is_err());
        assert!(LatticePlan::new(Profile::Fast, domain(), &[1.0; 33]).is_err());
        let too_wide = [[-900_000.0; 3], [0.0; 3], [900_000.0; 3]];
        assert_eq!(
            plan.encode(&too_wide, &[31; 3], 31).unwrap_err().kind(),
            JobErrorKind::InvalidState
        );
    }
}
