//! ENU → ECEF placement matching TinyOwl `modeltiles.eastNorthUpMatrix` for
//! lon/lat/height (heading about up in Go is EN-plane clockwise-from-north).
//! `--rotation-degrees` uses Cesium HPR (−heading, pitch, roll) and is **not**
//! the Go heading contract — do not swap the worker onto this until heading-only
//! 4×4s match.

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

/// Heading, pitch, roll in degrees (Cesium HPR: heading about up / −Z in ENU).
#[derive(Clone, Copy, Debug, Default)]
pub struct RotationDegrees {
    pub heading: f64,
    pub pitch: f64,
    pub roll: f64,
}

/// Column-major 4×4 tile `transform` (ENU→ECEF, optional HPR).
pub fn root_transform(pos: Cartographic, rot: Option<RotationDegrees>) -> [f64; 16] {
    let enu = east_north_up(pos);
    match rot {
        None => enu,
        Some(r) if r.heading == 0.0 && r.pitch == 0.0 && r.roll == 0.0 => enu,
        Some(r) => mul4(enu, hpr_matrix(r)),
    }
}

/// WGS84 east-north-up to ECEF, column-major. Same ellipsoid as TinyOwl Go.
pub fn east_north_up(pos: Cartographic) -> [f64; 16] {
    let lon = pos.lon_deg.to_radians();
    let lat = pos.lat_deg.to_radians();
    let height = pos.height_m;
    let a = 6378137.0;
    let e2 = 6.69437999014e-3;
    let sin_lat = lat.sin();
    let cos_lat = lat.cos();
    let sin_lon = lon.sin();
    let cos_lon = lon.cos();
    let n = a / (1.0 - e2 * sin_lat * sin_lat).sqrt();
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

/// Local HPR in ENU: Cesium heading about −Z (up), pitch about −Y, roll about +X.
fn hpr_matrix(r: RotationDegrees) -> [f64; 16] {
    let h = -r.heading.to_radians();
    let p = -r.pitch.to_radians();
    let roll = r.roll.to_radians();
    let (sh, ch) = (h.sin(), h.cos());
    let (sp, cp) = (p.sin(), p.cos());
    let (sr, cr) = (roll.sin(), roll.cos());

    // Rz(h) * Ry(p) * Rx(roll), column-major 3×3 embedded in 4×4.
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

fn mul4(a: [f64; 16], b: [f64; 16]) -> [f64; 16] {
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
    fn sydney_enu_translation_is_earth_radius_scale() {
        let t = east_north_up(Cartographic::new(151.2, -33.9, 0.0));
        let x = t[12];
        let y = t[13];
        let z = t[14];
        let r = (x * x + y * y + z * z).sqrt();
        assert!((r - 6.37e6).abs() < 5e4, "r={r}");
    }
}
