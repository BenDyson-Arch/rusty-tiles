//! ENU → ECEF `root.transform` matching Cesium `3d-tiles-tools` `createTilesetJson`
//! (`Transforms.eastNorthUpToFixedFrame` / `headingPitchRollQuaternion`).
//!
//! This is **not** TinyOwl Go `eastNorthUpMatrix` heading (EN-plane clockwise
//! from north with a truncated Z). `model-worker` still applies Go placement
//! after convert; do not pass `--rotationDegrees` from the worker until that
//! contract is dropped.

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

/// WGS84 east-north-up to ECEF, column-major. Cesium `eastNorthUpToFixedFrame`.
pub fn east_north_up(pos: Cartographic) -> [f64; 16] {
    let lon = pos.lon_deg.to_radians();
    let lat = pos.lat_deg.to_radians();
    let height = pos.height_m;
    let e2 = WGS84_F * (2.0 - WGS84_F);
    let sin_lat = lat.sin();
    let cos_lat = lat.cos();
    let sin_lon = lon.sin();
    let cos_lon = lon.cos();
    let n = WGS84_A / (1.0 - e2 * sin_lat * sin_lat).sqrt();
    let x = (n + height) * cos_lat * cos_lon;
    let y = (n + height) * cos_lat * sin_lon;
    let z = (n * (1.0 - e2) + height) * sin_lat;

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

/// Column-major translation (3D Tiles `tile.transform`).
pub fn translation(tx: f64, ty: f64, tz: f64) -> [f64; 16] {
    [
        1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, tx, ty, tz, 1.0,
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
