//! ENU → ECEF `root.transform` matching Cesium `3d-tiles-tools` `createTilesetJson`
//! (`Transforms.eastNorthUpToFixedFrame` / `headingPitchRollQuaternion`).
//!

/// WGS84 as Cesium `Ellipsoid.WGS84`.
const WGS84_A: f64 = 6_378_137.0;
const WGS84_F: f64 = 1.0 / 298.257_223_563;

/// Longitude/latitude in degrees, height in metres (WGS84).
#[derive(Clone, Copy, Debug)]
pub struct Cartographic {
    pub lon_deg: f64,
    pub lat_deg: f64,
    pub height_m: f64,
}

impl Cartographic {
    pub fn new(lon_deg: f64, lat_deg: f64, height_m: f64) -> Self {
        Self {
            lon_deg,
            lat_deg,
            height_m,
        }
    }
}

/// Heading, pitch, roll in degrees (`3d-tiles-tools --rotationDegrees`).
#[derive(Clone, Copy, Debug, Default)]
pub struct RotationDegrees {
    pub heading: f64,
    pub pitch: f64,
    pub roll: f64,
}

/// Column-major 4×4 tile `transform` (ENU→ECEF, optional Cesium HPR).
pub fn root_transform(pos: Cartographic, rot: Option<RotationDegrees>) -> [f64; 16] {
    let enu = east_north_up(pos);
    match rot {
        None => enu,
        Some(r) if r.heading == 0.0 && r.pitch == 0.0 && r.roll == 0.0 => enu,
        Some(r) => mul4(enu, hpr_matrix(r)),
    }
}

/// WGS84 geodetic → ECEF metres; also the origin of [`east_north_up`].
pub fn geodetic_to_ecef(pos: Cartographic) -> [f64; 3] {
    let lon = pos.lon_deg.to_radians();
    let lat = pos.lat_deg.to_radians();
    let e2 = WGS84_F * (2.0 - WGS84_F);
    let sin_lat = lat.sin();
    let cos_lat = lat.cos();
    let n = WGS84_A / (1.0 - e2 * sin_lat * sin_lat).sqrt();
    [
        (n + pos.height_m) * cos_lat * lon.cos(),
        (n + pos.height_m) * cos_lat * lon.sin(),
        (n * (1.0 - e2) + pos.height_m) * sin_lat,
    ]
}

/// WGS84 ECEF metres to longitude/latitude in degrees and ellipsoidal metres.
/// This fixed-datum inverse is also used to orient portable vector frames;
/// it does not select or perform a source datum transformation.
/// Longitude is conventionally zero at a pole, where it is indeterminate.
pub fn ecef_to_cartographic(point: [f64; 3]) -> Result<Cartographic, crate::Error> {
    if !point.iter().all(|value| value.is_finite()) || point.iter().all(|value| value.abs() < 1e-12)
    {
        return Err(crate::Error::Data(
            "cannot orient a nonfinite or Earth-centre ECEF position".into(),
        ));
    }
    let source = proj4rs::Proj::from_proj_string("+proj=geocent +datum=WGS84 +units=m")
        .map_err(|error| crate::Error::Environment(error.to_string()))?;
    let target = proj4rs::Proj::from_proj_string("+proj=longlat +datum=WGS84")
        .map_err(|error| crate::Error::Environment(error.to_string()))?;
    let mut position = (point[0], point[1], point[2]);
    proj4rs::transform::transform(&source, &target, &mut position)
        .map_err(|error| crate::Error::Data(format!("cannot orient ECEF position: {error}")))?;
    if ![position.0, position.1, position.2]
        .iter()
        .all(|value| value.is_finite())
    {
        return Err(crate::Error::Data(
            "ECEF inverse produced nonfinite coordinates".into(),
        ));
    }
    Ok(Cartographic::new(
        position.0.to_degrees(),
        position.1.to_degrees(),
        position.2,
    ))
}

/// ECEF origin plus ENU basis, reused when baking many vertices.
#[derive(Clone, Copy, Debug)]
pub struct EnuFrame {
    origin_ecef: [f64; 3],
    east: [f64; 3],
    north: [f64; 3],
    up: [f64; 3],
}

impl EnuFrame {
    pub fn new(origin: Cartographic) -> Self {
        let lon = origin.lon_deg.to_radians();
        let lat = origin.lat_deg.to_radians();
        let sin_lat = lat.sin();
        let cos_lat = lat.cos();
        let sin_lon = lon.sin();
        let cos_lon = lon.cos();
        Self {
            origin_ecef: geodetic_to_ecef(origin),
            east: [-sin_lon, cos_lon, 0.0],
            north: [-sin_lat * cos_lon, -sin_lat * sin_lon, cos_lat],
            up: [cos_lat * cos_lon, cos_lat * sin_lon, sin_lat],
        }
    }

    pub fn to_enu(&self, ecef: [f64; 3]) -> [f64; 3] {
        let dx = ecef[0] - self.origin_ecef[0];
        let dy = ecef[1] - self.origin_ecef[1];
        let dz = ecef[2] - self.origin_ecef[2];
        [
            self.east[0] * dx + self.east[1] * dy + self.east[2] * dz,
            self.north[0] * dx + self.north[1] * dy + self.north[2] * dz,
            self.up[0] * dx + self.up[1] * dy + self.up[2] * dz,
        ]
    }
}

/// Axes of general mesh POSITION after glTF node transforms.
#[derive(Clone, Copy, Debug, Eq, PartialEq, clap::ValueEnum)]
pub enum SourceAxes {
    /// X=easting/longitude, Y=northing/latitude, Z=height in metres.
    Xyz,
    /// X=easting/longitude, Y=height in metres, Z=negative northing/latitude.
    YUp,
}

impl SourceAxes {
    pub(crate) fn coordinates(self, point: [f64; 3], offset: SourceOffset) -> [f64; 3] {
        let [east, north, height] = match self {
            Self::Xyz => point,
            Self::YUp => [point[0], -point[2], point[1]],
        };
        [
            east + offset.easting,
            north + offset.northing,
            height + offset.height,
        ]
    }
}

/// Source-coordinate shift: E/N use the declared horizontal CRS units, A is
/// metres.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct SourceOffset {
    pub easting: f64,
    pub northing: f64,
    pub height: f64,
}

/// WGS84 east-north-up to ECEF, column-major. Cesium `eastNorthUpToFixedFrame`.
pub fn east_north_up(pos: Cartographic) -> [f64; 16] {
    let lon = pos.lon_deg.to_radians();
    let lat = pos.lat_deg.to_radians();
    let sin_lat = lat.sin();
    let cos_lat = lat.cos();
    let sin_lon = lon.sin();
    let cos_lon = lon.cos();
    let [x, y, z] = geodetic_to_ecef(pos);

    let east = [-sin_lon, cos_lon, 0.0];
    let north = [-sin_lat * cos_lon, -sin_lat * sin_lon, cos_lat];
    let up = [cos_lat * cos_lon, cos_lat * sin_lon, sin_lat];

    [
        east[0], east[1], east[2], 0.0, north[0], north[1], north[2], 0.0, up[0], up[1], up[2],
        0.0, x, y, z, 1.0,
    ]
}

/// Cesium `Matrix3.fromHeadingPitchRoll` as a 4×4 (heading about −Z, pitch −Y, roll +X).
fn hpr_matrix(r: RotationDegrees) -> [f64; 16] {
    let h = -r.heading.to_radians();
    let p = -r.pitch.to_radians();
    let roll = r.roll.to_radians();
    let (sh, ch) = (h.sin(), h.cos());
    let (sp, cp) = (p.sin(), p.cos());
    let (sr, cr) = (roll.sin(), roll.cos());

    let r00 = ch * cp;
    let r01 = ch * sp * sr - sh * cr;
    let r02 = ch * sp * cr + sh * sr;
    let r10 = sh * cp;
    let r11 = sh * sp * sr + ch * cr;
    let r12 = sh * sp * cr - ch * sr;
    let r20 = -sp;
    let r21 = cp * sr;
    let r22 = cp * cr;

    [
        r00, r10, r20, 0.0, r01, r11, r21, 0.0, r02, r12, r22, 0.0, 0.0, 0.0, 0.0, 1.0,
    ]
}

pub(crate) fn mul4(a: [f64; 16], b: [f64; 16]) -> [f64; 16] {
    let mut c = [0.0; 16];
    for col in 0..4 {
        for row in 0..4 {
            c[col * 4 + row] = a[row] * b[col * 4]
                + a[4 + row] * b[col * 4 + 1]
                + a[8 + row] * b[col * 4 + 2]
                + a[12 + row] * b[col * 4 + 3];
        }
    }
    c
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fixed_wgs84_inverse_orients_surface_poles_and_high_altitude_frames() {
        for lon in [-180., 0., 153., 180.] {
            for lat in [-90., -89.99, -27., 0., 75., 90.] {
                for height in [-500., 0., 10000., 35_786_000.] {
                    let original = Cartographic::new(lon, lat, height);
                    let xyz = geodetic_to_ecef(original);
                    let inverse = ecef_to_cartographic(xyz).unwrap();
                    assert!((inverse.lat_deg - lat).abs() < 1e-9);
                    assert!((inverse.height_m - height).abs() < 0.001);
                    let frame = root_transform(inverse, None);
                    let expected = root_transform(
                        if lat.abs() == 90. {
                            Cartographic::new(0., lat, height)
                        } else {
                            original
                        },
                        None,
                    );
                    for i in [0, 1, 2, 4, 5, 6, 8, 9, 10] {
                        assert!((frame[i] - expected[i]).abs() < 1e-11);
                    }
                }
            }
        }
        for point in [[0.; 3], [f64::NAN, 1., 2.], [1., f64::INFINITY, 2.]] {
            assert!(ecef_to_cartographic(point).is_err());
        }
    }

    fn assert_mat(got: [f64; 16], want: [f64; 16], trans_eps: f64) {
        for i in 0..16 {
            let eps = if i == 12 || i == 13 || i == 14 {
                trans_eps
            } else {
                1e-8
            };
            assert!(
                (got[i] - want[i]).abs() < eps,
                "m[{i}] got={} want={}",
                got[i],
                want[i]
            );
        }
    }

    #[test]
    fn east_north_up_origin_is_bitwise_geodetic_to_ecef() {
        // The pre-refactor inline origin, kept to pin published transforms.
        let inline = |pos: Cartographic| {
            let (lon, lat) = (pos.lon_deg.to_radians(), pos.lat_deg.to_radians());
            let e2 = WGS84_F * (2.0 - WGS84_F);
            let (sin_lat, cos_lat) = (lat.sin(), lat.cos());
            let n = WGS84_A / (1.0 - e2 * sin_lat * sin_lat).sqrt();
            [
                (n + pos.height_m) * cos_lat * lon.cos(),
                (n + pos.height_m) * cos_lat * lon.sin(),
                (n * (1.0 - e2) + pos.height_m) * sin_lat,
            ]
        };
        for lon in (-180..=180).step_by(7) {
            for lat in (-90..=90).step_by(3) {
                let pos = Cartographic::new(lon as f64 + 0.123, lat as f64 * 0.999, 17.25);
                let t = east_north_up(pos);
                assert_eq!(
                    [t[12], t[13], t[14]].map(f64::to_bits),
                    inline(pos).map(f64::to_bits)
                );
            }
        }
    }

    #[test]
    fn sydney_enu_matches_cesium_east_north_up_to_fixed_frame() {
        let t = east_north_up(Cartographic::new(151.2, -33.9, 10.0));
        // Dumped from cesium@ used by 3d-tiles-tools@0.5.4.
        let want = [
            -0.481_753_674_101_715_55,
            -0.876_306_680_043_863_5,
            0.0,
            0.0,
            -0.488_755_768_314_889_65,
            0.268_695_757_417_162_45,
            0.830_012_282_369_929_5,
            0.0,
            -0.727_345_307_559_222_6,
            0.399_861_466_581_264_1,
            -0.557_745_113_035_57,
            0.0,
            -4_643_953.300_870_437,
            2_553_034.931_719_495_4,
            -3_537_250.925_356_345_7,
            1.0,
        ];
        assert_mat(t, want, 1e-4);
    }

    #[test]
    fn rotation_degrees_90_matches_cesium_heading_pitch_roll() {
        let t = root_transform(
            Cartographic::new(151.2, -33.9, 10.0),
            Some(RotationDegrees {
                heading: 90.0,
                pitch: 0.0,
                roll: 0.0,
            }),
        );
        let want = [
            0.488_755_768_314_889_6,
            -0.268_695_757_417_162_7,
            -0.830_012_282_369_929_3,
            0.0,
            -0.481_753_674_101_715_66,
            -0.876_306_680_043_863_4,
            0.0,
            0.0,
            -0.727_345_307_559_222_6,
            0.399_861_466_581_264_1,
            -0.557_745_113_035_570_1,
            0.0,
            -4_643_953.300_870_437,
            2_553_034.931_719_495_4,
            -3_537_250.925_356_345_7,
            1.0,
        ];
        assert_mat(t, want, 1e-4);
    }
}
