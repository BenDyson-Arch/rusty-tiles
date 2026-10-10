//! Explicit rigid placement of admitted local metre/Y-up mesh geometry.
//! Numerical interpretation only: no paths, source I/O or publication policy.
use crate::{JobError, JobErrorKind};
use serde::Serialize;

const WGS84_A: f64 = 6_378_137.0;
const WGS84_F: f64 = 1.0 / 298.257_223_563;
// Greatest f64 <= 2^26 - WGS84_A - sqrt(3)*LOCAL_COMPONENT_LIMIT.
// Independent Decimal80 boundary reference lives in tests/f1c1_oracle.py.
const AVAILABLE_PLACEMENT_MAGNITUDE: f64 = 58_998_676.192_431_12;
const QUATERNION_NORM_TOLERANCE: f64 = 1e-12;
const IDENTITY: [f64; 16] = [
    1., 0., 0., 0., 0., 1., 0., 0., 0., 0., 1., 0., 0., 0., 0., 1.,
];

/// Output placement of an explicitly local metre/Y-up source.
/// Numeric parameters are validated by the mesh operation before source I/O.
#[derive(Clone, Debug, Default, PartialEq)]
pub enum MeshPlacement {
    /// Keep the output in the local frame without inferring Earth coordinates.
    #[default]
    Local,
    /// WGS84 ellipsoidal anchor and a right-handed active ENU rotation.
    Wgs84 {
        /// Longitude/latitude in degrees, ellipsoidal height in metres.
        anchor_degrees_metres: [f64; 3],
        /// ENU quaternion XYZW, near unit length; W is the scalar.
        orientation_xyzw: [f64; 4],
        /// Post-node, pre-orientation translation in source Y-up metres.
        scene_offset_metres: [f64; 3],
    },
}

impl MeshPlacement {
    /// Shared adapter grammar. Orientation and offset require an anchor.
    /// With an anchor their omissions mean identity rotation and zero offset.
    pub fn from_parameters(
        anchor: Option<[f64; 3]>,
        orientation: Option<[f64; 4]>,
        offset: Option<[f64; 3]>,
    ) -> Result<Self, JobError> {
        match anchor {
            Some(anchor_degrees_metres) => Ok(Self::Wgs84 {
                anchor_degrees_metres,
                orientation_xyzw: orientation.unwrap_or([0., 0., 0., 1.]),
                scene_offset_metres: offset.unwrap_or([0.; 3]),
            }),
            None if orientation.is_none() && offset.is_none() => Ok(Self::Local),
            None => Err(invalid(
                "mesh orientation and scene offset require an anchor",
            )),
        }
    }
}

/// The actual placement parameters used to produce the root transform.
#[derive(Clone, Debug, Serialize, PartialEq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum MeshPlacementReport {
    Local,
    Wgs84 {
        anchor_degrees_metres: [f64; 3],
        orientation_xyzw: [f64; 4],
        scene_offset_metres: [f64; 3],
    },
}

#[derive(Clone, Debug)]
pub(super) struct ResolvedPlacement {
    transform: [f64; 16],
    report: MeshPlacementReport,
}

fn invalid(message: &'static str) -> JobError {
    JobError::new(JobErrorKind::InvalidRequest, message)
}

/// Runtime glTF Y-up to tile Z-up: +X east, +Y up, -Z north.
pub(super) fn to_tile([x, y, z]: [f64; 3]) -> [f64; 3] {
    [x, -z, y]
}

/// Exact sign of the four-component sum minus the conservative budget.
/// General TwoSum retains each residual in a bounded, nonoverlapping expansion.
/// The individual guard ensures finite intermediates; five terms need five slots.
fn within_magnitude_budget(height: f64, offset: [f64; 3]) -> bool {
    let components = [
        height.abs(),
        offset[0].abs(),
        offset[1].abs(),
        offset[2].abs(),
    ];
    if components
        .iter()
        .any(|value| *value > AVAILABLE_PLACEMENT_MAGNITUDE)
    {
        return false;
    }
    let mut expansion = [0.; 5];
    let mut count = 0;
    for value in components
        .into_iter()
        .chain([-AVAILABLE_PLACEMENT_MAGNITUDE])
    {
        let mut next = [0.; 5];
        let mut next_count = 0;
        let mut sum = value;
        for component in &expansion[..count] {
            let combined = sum + component;
            let virtual_component = combined - sum;
            let residual = (sum - (combined - virtual_component)) + (component - virtual_component);
            if residual != 0. {
                next[next_count] = residual;
                next_count += 1;
            }
            sum = combined;
        }
        if sum != 0. {
            next[next_count] = sum;
            next_count += 1;
        }
        expansion = next;
        count = next_count;
    }
    count == 0 || expansion[count - 1] < 0.
}

impl ResolvedPlacement {
    pub(super) fn resolve(placement: &MeshPlacement) -> Result<Self, JobError> {
        let MeshPlacement::Wgs84 {
            anchor_degrees_metres,
            orientation_xyzw,
            scene_offset_metres,
        } = placement
        else {
            return Ok(Self {
                transform: IDENTITY,
                report: MeshPlacementReport::Local,
            });
        };
        if anchor_degrees_metres
            .iter()
            .chain(orientation_xyzw)
            .chain(scene_offset_metres)
            .any(|value| !value.is_finite())
        {
            return Err(invalid("mesh placement parameters must be finite"));
        }
        let [longitude, latitude, height] = *anchor_degrees_metres;
        if !(-180.0..=180.0).contains(&longitude) || !(-90.0..=90.0).contains(&latitude) {
            return Err(invalid(
                "mesh anchor longitude must be in [-180,180] and latitude in [-90,90] degrees",
            ));
        }
        let norm = orientation_xyzw
            .iter()
            .fold(0.0_f64, |norm, component| norm.hypot(*component));
        if !norm.is_finite() || (norm - 1.0).abs() > QUATERNION_NORM_TOLERANCE {
            return Err(invalid(
                "mesh ENU orientation quaternion must have unit norm within 1e-12",
            ));
        }
        if !within_magnitude_budget(height, *scene_offset_metres) {
            return Err(JobError::new(
                JobErrorKind::Unsupported,
                "mesh placement exceeds the 2^26 metre forward-magnitude precision budget",
            ));
        }
        let quaternion = orientation_xyzw.map(|component| component / norm);
        let [x, y, z, w] = quaternion;
        let rotation = [
            [
                1. - 2. * (y * y + z * z),
                2. * (x * y - z * w),
                2. * (x * z + y * w),
            ],
            [
                2. * (x * y + z * w),
                1. - 2. * (x * x + z * z),
                2. * (y * z - x * w),
            ],
            [
                2. * (x * z - y * w),
                2. * (y * z + x * w),
                1. - 2. * (x * x + y * y),
            ],
        ];
        let (sin_lon, cos_lon) = longitude.to_radians().sin_cos();
        let (sin_lat, cos_lat) = latitude.to_radians().sin_cos();
        // This frame uses declared cartographic latitude/longitude, including
        // elevated anchors and pole meridians; no inverse or repair is needed.
        let enu = [
            [-sin_lon, -sin_lat * cos_lon, cos_lat * cos_lon],
            [cos_lon, -sin_lat * sin_lon, cos_lat * sin_lon],
            [0., cos_lat, sin_lat],
        ];
        let eccentricity_squared = WGS84_F * (2. - WGS84_F);
        let radius = WGS84_A / (1. - eccentricity_squared * sin_lat * sin_lat).sqrt();
        let origin = [
            (radius + height) * cos_lat * cos_lon,
            (radius + height) * cos_lat * sin_lon,
            (radius * (1. - eccentricity_squared) + height) * sin_lat,
        ];
        let linear: [[f64; 3]; 3] = std::array::from_fn(|row| {
            std::array::from_fn(|column| {
                (0..3)
                    .map(|axis| enu[row][axis] * rotation[axis][column])
                    .sum()
            })
        });
        let offset = to_tile(*scene_offset_metres);
        let translation: [f64; 3] = std::array::from_fn(|row| {
            origin[row]
                + (0..3)
                    .map(|axis| linear[row][axis] * offset[axis])
                    .sum::<f64>()
        });
        let mut transform = IDENTITY;
        for column in 0..3 {
            for row in 0..3 {
                transform[column * 4 + row] = linear[row][column];
            }
            transform[12 + column] = translation[column];
        }
        if transform.iter().any(|value| !value.is_finite()) {
            return Err(JobError::new(
                JobErrorKind::InvalidState,
                "validated mesh placement produced a nonfinite root transform",
            ));
        }
        Ok(Self {
            transform,
            report: MeshPlacementReport::Wgs84 {
                anchor_degrees_metres: *anchor_degrees_metres,
                orientation_xyzw: quaternion,
                scene_offset_metres: *scene_offset_metres,
            },
        })
    }

    pub(super) fn transform(&self) -> [f64; 16] {
        self.transform
    }

    pub(super) fn report(&self) -> MeshPlacementReport {
        self.report.clone()
    }

    pub(super) fn coordinates(&self) -> &'static str {
        match self.report {
            MeshPlacementReport::Local => "local-gltf",
            MeshPlacementReport::Wgs84 { .. } => "wgs84-ecef",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request(anchor: [f64; 3], quaternion: [f64; 4], offset: [f64; 3]) -> MeshPlacement {
        MeshPlacement::Wgs84 {
            anchor_degrees_metres: anchor,
            orientation_xyzw: quaternion,
            scene_offset_metres: offset,
        }
    }

    #[test]
    fn magnitude_boundary_cannot_hide_a_positive_rounding_residual() {
        let boundary = 58_998_676.192_431_12;
        assert!(within_magnitude_budget(boundary, [0.; 3]));
        assert!(within_magnitude_budget(0., [boundary, 0., 0.]));
        assert!(!within_magnitude_budget(boundary.next_up(), [0.; 3]));
        assert!(!within_magnitude_budget(
            boundary,
            [f64::from_bits(1), 0., 0.]
        ));
        assert!(within_magnitude_budget(
            boundary.next_down(),
            [1e-10, -1e-10, 1e-10]
        ));
        assert!(!within_magnitude_budget(
            boundary.next_down(),
            [1e-8, 0., 0.]
        ));
    }

    #[test]
    fn omitted_anchor_cannot_hide_orientation_or_offset() {
        assert_eq!(
            MeshPlacement::from_parameters(None, None, None).unwrap(),
            MeshPlacement::Local
        );
        for (orientation, offset) in [(Some([0., 0., 0., 1.]), None), (None, Some([0.; 3]))] {
            assert_eq!(
                MeshPlacement::from_parameters(None, orientation, offset)
                    .unwrap_err()
                    .kind(),
                JobErrorKind::InvalidRequest,
            );
        }
        assert_eq!(
            MeshPlacement::from_parameters(Some([0.; 3]), None, None).unwrap(),
            request([0.; 3], [0., 0., 0., 1.], [0.; 3]),
        );
    }

    #[test]
    fn numeric_validation_precedes_magnitude_eligibility() {
        let identity = [0., 0., 0., 1.];
        for placement in [
            request([f64::NAN, 0., 0.], identity, [0.; 3]),
            request([0., 0., f64::INFINITY], identity, [0.; 3]),
            request([0.; 3], [0., 0., f64::NEG_INFINITY, 1.], [0.; 3]),
            request([0.; 3], identity, [0., f64::NAN, 0.]),
            request([180.0001, 0., 0.], identity, [0.; 3]),
            request([0., -90.0001, 0.], identity, [0.; 3]),
            request([0.; 3], [0.; 4], [0.; 3]),
            request([0.; 3], [0., 0., 0., 1. + 2e-12], [f64::MAX; 3]),
        ] {
            assert_eq!(
                ResolvedPlacement::resolve(&placement).unwrap_err().kind(),
                JobErrorKind::InvalidRequest
            );
        }
        assert_eq!(
            ResolvedPlacement::resolve(&request([0.; 3], identity, [f64::MAX; 3]))
                .unwrap_err()
                .kind(),
            JobErrorKind::Unsupported,
        );
    }

    #[test]
    fn drift_is_normalized_and_signed_height_uses_one_budget() {
        let identity = [0., 0., 0., 1.];
        let resolved = ResolvedPlacement::resolve(&request(
            [180., 90., 0.],
            [0., 0., 0., 1. + 0.5e-12],
            [0.; 3],
        ))
        .unwrap();
        assert_eq!(
            resolved.report(),
            MeshPlacementReport::Wgs84 {
                anchor_degrees_metres: [180., 90., 0.],
                orientation_xyzw: identity,
                scene_offset_metres: [0.; 3],
            }
        );
        let height_limit = AVAILABLE_PLACEMENT_MAGNITUDE;
        for sign in [-1., 1.] {
            assert!(ResolvedPlacement::resolve(&request(
                [0., 0., sign * (height_limit - 1.)],
                identity,
                [0.; 3]
            ))
            .is_ok());
            assert_eq!(
                ResolvedPlacement::resolve(&request(
                    [0., 0., sign * (height_limit + 1.)],
                    identity,
                    [0.; 3]
                ))
                .unwrap_err()
                .kind(),
                JobErrorKind::Unsupported
            );
        }
        let local = ResolvedPlacement::resolve(&MeshPlacement::Local).unwrap();
        assert_eq!(local.transform(), IDENTITY);
        assert_eq!(local.coordinates(), "local-gltf");
        assert_eq!(local.report(), MeshPlacementReport::Local);
    }
}
