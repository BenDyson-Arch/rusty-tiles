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
///
/// The quantized-mesh terrain encoder keeps its own formula (e² from the
/// semi-minor axis, a different evaluation order): its published `.terrain`
/// bytes differ from this one in the last ulp.
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

    /// Transform an authored normal by the inverse transpose of the CRS
    /// projection's local Jacobian. Difference in f64 before casting to f32;
    /// recomputing normals from triangles would erase authored hard edges.
    pub fn normal_to_enu_yup(&self, pos: [f32; 3], normal: [f32; 3], kind: CrsKind) -> [f32; 3] {
        self.normal_to_enu_yup_offset(pos, normal, kind, SourceOffset::default())
    }

    pub fn normal_to_enu_yup_offset(
        &self,
        pos: [f32; 3],
        normal: [f32; 3],
        kind: CrsKind,
        offset: SourceOffset,
    ) -> [f32; 3] {
        if normal == [0.0; 3] {
            return normal;
        }
        let project = |p: [f64; 3]| {
            let (lon, lat) = match kind {
                CrsKind::Geographic => (p[0], -p[2]),
                CrsKind::WebMercator => {
                    mercator_to_geodetic(p[0] + offset.easting, offset.northing - p[2])
                }
            };
            let [e, n, u] = self.to_enu(geodetic_to_ecef(Cartographic::new(
                lon,
                lat,
                p[1] + offset.height,
            )));
            [e, u, -n]
        };
        let columns: [[f64; 3]; 3] = std::array::from_fn(|axis| {
            let step = if matches!(kind, CrsKind::Geographic) && axis != 1 {
                1e-5
            } else {
                0.1
            };
            let mut a = pos.map(f64::from);
            let mut b = a;
            a[axis] -= step;
            b[axis] += step;
            let a = project(a);
            let b = project(b);
            std::array::from_fn(|i| (b[i] - a[i]) / (2.0 * step))
        });
        let cross = |a: [f64; 3], b: [f64; 3]| {
            [
                a[1] * b[2] - a[2] * b[1],
                a[2] * b[0] - a[0] * b[2],
                a[0] * b[1] - a[1] * b[0],
            ]
        };
        let co = [
            cross(columns[1], columns[2]),
            cross(columns[2], columns[0]),
            cross(columns[0], columns[1]),
        ];
        let det = (0..3).map(|i| columns[0][i] * co[0][i]).sum::<f64>();
        let n: [f64; 3] = std::array::from_fn(|i| {
            (0..3).map(|j| co[j][i] * normal[j] as f64).sum::<f64>() * det.signum()
        });
        let len = n.iter().map(|v| v * v).sum::<f64>().sqrt();
        if len > 0.0 {
            n.map(|v| (v / len) as f32)
        } else {
            normal
        }
    }

    /// Metashape geographic Y-up (lon°, height m, −lat°) → ENU Y-up.
    pub fn geog_yup_to_enu_yup(&self, pos: [f32; 3]) -> [f32; 3] {
        let geog = Cartographic::new(pos[0] as f64, -(pos[2] as f64), pos[1] as f64);
        let [e, n, u] = self.to_enu(geodetic_to_ecef(geog));
        [e as f32, u as f32, (-n) as f32]
    }

    /// Metashape Web Mercator Y-up (easting m, height m, −northing m) → ENU Y-up.
    pub fn mercator_yup_to_enu_yup(&self, pos: [f32; 3]) -> [f32; 3] {
        self.mercator_enuh_to_enu_yup(pos[0] as f64, -(pos[2] as f64), pos[1] as f64)
    }

    /// Local Metashape Y-up plus [`SourceOffset`] (f64), then ENU Y-up.
    /// easting = X+E, height = Y+A, northing = N−Z. Never add the shift in f32.
    pub fn mercator_yup_offset_to_enu_yup(&self, pos: [f32; 3], off: SourceOffset) -> [f32; 3] {
        let (easting, height, northing) = apply_mercator_offset(pos, off);
        self.mercator_enuh_to_enu_yup(easting, northing, height)
    }

    fn mercator_enuh_to_enu_yup(&self, easting: f64, northing: f64, height: f64) -> [f32; 3] {
        let (lon, lat) = mercator_to_geodetic(easting, northing);
        let [e, n, u] = self.to_enu(geodetic_to_ecef(Cartographic::new(lon, lat, height)));
        [e as f32, u as f32, (-n) as f32]
    }
}

/// EPSG:3857 (spherical) easting/northing → lon/lat degrees.
pub fn mercator_to_geodetic(easting: f64, northing: f64) -> (f64, f64) {
    let lon = easting * 180.0 / (std::f64::consts::PI * WGS84_A);
    let lat = (northing / WGS84_A).sinh().atan().to_degrees();
    (lon, lat)
}

/// Metashape geographic glTF is Y-up: X=longitude°, Y=height m, Z=−latitude°.
pub fn looks_geographic_yup(min: [f64; 3], max: [f64; 3]) -> bool {
    let dx = max[0] - min[0];
    let dy = max[1] - min[1];
    let dz = max[2] - min[2];
    let cx = (min[0] + max[0]) * 0.5;
    let cy = (min[1] + max[1]) * 0.5;
    let cz = (min[2] + max[2]) * 0.5;
    if cx.abs() > 180.0 || cz.abs() > 90.0 {
        return false;
    }
    if dx >= 1.0 || dz >= 1.0 {
        return false;
    }
    if cx.abs() < 2.0 || cz.abs() < 1.0 {
        return false;
    }
    if cy.abs() > 20_000.0 || dy > 20_000.0 {
        return false;
    }
    if dy < 2.0 && cy.abs() < 20.0 {
        return false;
    }
    dx < 0.5 && dz < 0.5
}

/// Metashape Web Mercator glTF is Y-up: X=easting m, Y=height m, Z=−northing m.
pub fn looks_web_mercator_yup(min: [f64; 3], max: [f64; 3]) -> bool {
    let dx = max[0] - min[0];
    let dy = max[1] - min[1];
    let dz = max[2] - min[2];
    let cx = (min[0] + max[0]) * 0.5;
    let cy = (min[1] + max[1]) * 0.5;
    let cz = (min[2] + max[2]) * 0.5;
    const LIMIT: f64 = 20_037_508.0;
    if cx.abs() < 100_000.0 || cx.abs() > LIMIT {
        return false;
    }
    if cz.abs() < 100_000.0 || cz.abs() > LIMIT {
        return false;
    }
    if cy.abs() > 20_000.0 || dy > 20_000.0 {
        return false;
    }
    if dx < 2.0 || dz < 2.0 {
        return false;
    }
    dx <= 50_000.0 && dz <= 50_000.0
}

pub fn geographic_origin_yup(min: [f64; 3], max: [f64; 3]) -> Cartographic {
    Cartographic::new(
        (min[0] + max[0]) * 0.5,
        -(min[2] + max[2]) * 0.5,
        (min[1] + max[1]) * 0.5,
    )
}

pub fn geographic_bbox_wgs84(min: [f64; 3], max: [f64; 3]) -> [f64; 4] {
    let west = min[0];
    let east = max[0];
    let mut south = -max[2];
    let mut north = -min[2];
    if south > north {
        std::mem::swap(&mut south, &mut north);
    }
    [west, south, east, north]
}

pub fn mercator_origin_yup(min: [f64; 3], max: [f64; 3]) -> Cartographic {
    let easting = (min[0] + max[0]) * 0.5;
    let northing = -(min[2] + max[2]) * 0.5;
    let (lon, lat) = mercator_to_geodetic(easting, northing);
    Cartographic::new(lon, lat, (min[1] + max[1]) * 0.5)
}

/// ENU origin at the Metashape Shift (E, N, A), matching unmoved local vertices.
pub fn mercator_shift_origin(off: SourceOffset) -> Cartographic {
    let (lon, lat) = mercator_to_geodetic(off.easting, off.northing);
    Cartographic::new(lon, lat, off.height)
}

pub fn mercator_bbox_wgs84(min: [f64; 3], max: [f64; 3]) -> [f64; 4] {
    let (west, _) = mercator_to_geodetic(min[0], 0.0);
    let (east, _) = mercator_to_geodetic(max[0], 0.0);
    let (_, mut south) = mercator_to_geodetic(0.0, -max[2]);
    let (_, mut north) = mercator_to_geodetic(0.0, -min[2]);
    if south > north {
        std::mem::swap(&mut south, &mut north);
    }
    [west, south, east, north]
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CrsKind {
    Geographic,
    WebMercator,
}

/// How to interpret glTF POSITION before the ENU bake.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum SourceCrs {
    #[default]
    Auto,
    Geographic,
    WebMercator,
}

impl SourceCrs {
    pub fn parse_cli(s: &str) -> Result<Self, crate::error::Error> {
        match s.trim().to_ascii_lowercase().as_str() {
            "auto" => Ok(Self::Auto),
            "geographic" | "wgs84" | "epsg:4326" | "4326" => Ok(Self::Geographic),
            "webmercator" | "mercator" | "epsg:3857" | "3857" | "pseudo-mercator"
            | "pseudomercator" => Ok(Self::WebMercator),
            other => Err(crate::error::Error::msg(format!(
                "unknown --sourceCrs {other:?} (auto|geographic|epsg:3857)"
            ))),
        }
    }
}

/// Metashape Shift / offset.txt: world = local + (E, N, A).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct SourceOffset {
    pub easting: f64,
    pub northing: f64,
    pub height: f64,
}

/// easting = X+E, height = Y+A, northing = N−Z (Metashape Y-up, Z = −northing).
pub fn apply_mercator_offset(pos: [f32; 3], off: SourceOffset) -> (f64, f64, f64) {
    (
        pos[0] as f64 + off.easting,
        pos[1] as f64 + off.height,
        off.northing - pos[2] as f64,
    )
}

pub fn mercator_origin_yup_offset(min: [f64; 3], max: [f64; 3], off: SourceOffset) -> Cartographic {
    let easting = (min[0] + max[0]) * 0.5 + off.easting;
    let height = (min[1] + max[1]) * 0.5 + off.height;
    let northing = off.northing - (min[2] + max[2]) * 0.5;
    let (lon, lat) = mercator_to_geodetic(easting, northing);
    Cartographic::new(lon, lat, height)
}

pub fn mercator_bbox_wgs84_offset(min: [f64; 3], max: [f64; 3], off: SourceOffset) -> [f64; 4] {
    let (west, _) = mercator_to_geodetic(min[0] + off.easting, 0.0);
    let (east, _) = mercator_to_geodetic(max[0] + off.easting, 0.0);
    let (_, mut south) = mercator_to_geodetic(0.0, off.northing - max[2]);
    let (_, mut north) = mercator_to_geodetic(0.0, off.northing - min[2]);
    if south > north {
        std::mem::swap(&mut south, &mut north);
    }
    [west, south, east, north]
}

/// Metashape `offset.txt`: `E: …` / `N: …` / `A: …`.
pub fn parse_metashape_offset(text: &str) -> Result<SourceOffset, crate::error::Error> {
    let mut easting = None;
    let mut northing = None;
    let mut height = 0.0;
    for raw in text.lines() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let (k, v) = if let Some(i) = line.find(':') {
            (&line[..i], &line[i + 1..])
        } else if let Some(i) = line.find('=') {
            (&line[..i], &line[i + 1..])
        } else {
            return Err(crate::error::Error::msg(format!(
                "offset line must be K: value, got {line:?}"
            )));
        };
        let key = k.trim().to_ascii_uppercase();
        let val: f64 = v.trim().parse().map_err(|_| {
            crate::error::Error::msg(format!("offset {key} is not a number: {}", v.trim()))
        })?;
        match key.as_str() {
            "E" | "EASTING" | "X" => easting = Some(val),
            "N" | "NORTHING" => northing = Some(val),
            "A" | "ALTITUDE" | "H" | "HEIGHT" | "Z" => height = val,
            _ => {
                return Err(crate::error::Error::msg(format!(
                    "unknown offset key {key:?} (expected E, N, A)"
                )));
            }
        }
    }
    Ok(SourceOffset {
        easting: easting.ok_or_else(|| crate::error::Error::msg("offset missing E (easting)"))?,
        northing: northing
            .ok_or_else(|| crate::error::Error::msg("offset missing N (northing)"))?,
        height,
    })
}

/// Geographic Y-up vertex → glTF Y-up ENU (X east, Y up, Z −north).
pub fn geog_yup_to_enu_yup(pos: [f32; 3], origin: Cartographic) -> [f32; 3] {
    EnuFrame::new(origin).geog_yup_to_enu_yup(pos)
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
                    let expected = root_transform(original, None);
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

    #[test]
    fn local_metre_aabb_is_not_projected() {
        assert!(!looks_geographic_yup([0.0, 0.0, 0.0], [1.0, 1.0, 0.0]));
        assert!(!looks_web_mercator_yup(
            [-50.0, 0.0, -50.0],
            [50.0, 10.0, 50.0]
        ));
    }

    #[test]
    fn metashape_synthetic_aabb_is_geographic() {
        let min = [30.0000, 120.0, 20.0000];
        let max = [30.0010, 140.0, 20.0010];
        assert!(looks_geographic_yup(min, max));
        assert!(!looks_web_mercator_yup(min, max));
        let o = geographic_origin_yup(min, max);
        assert!((o.lon_deg - 30.0005).abs() < 1e-9);
        assert!((o.lat_deg - (-20.0005)).abs() < 1e-9);
    }

    #[test]
    fn geog_vertex_bakes_to_enu_metres() {
        let origin = Cartographic::new(30.0005, -20.0005, 130.0);
        let p = geog_yup_to_enu_yup([30.0000, 120.0, 20.0010], origin);
        assert!(
            p[0].abs() < 200.0 && p[1].abs() < 50.0 && p[2].abs() < 200.0,
            "baked vertex still not ENU metres: {p:?}"
        );
        let span = (p[0] * p[0] + p[1] * p[1] + p[2] * p[2]).sqrt();
        assert!(
            span > 5.0,
            "expected ~10–100 m offset from centroid, got {span}"
        );
    }

    #[test]
    fn synthetic_web_mercator_aabb_inverts_to_wgs84() {
        let min = [3_000_200.0, 110.0, 1_999_920.0];
        let max = [3_000_300.0, 140.0, 1_999_980.0];
        assert!(looks_web_mercator_yup(min, max));
        assert!(!looks_geographic_yup(min, max));
        let o = mercator_origin_yup(min, max);
        assert!(
            (o.lon_deg - 26.9517043118).abs() < 1e-4,
            "lon {}",
            o.lon_deg
        );
        assert!(
            (o.lat_deg - (-17.6784862924)).abs() < 1e-4,
            "lat {}",
            o.lat_deg
        );
        let p = EnuFrame::new(o).mercator_yup_to_enu_yup([min[0] as f32, 120.0, max[2] as f32]);
        assert!(
            p[0].abs() < 200.0 && p[2].abs() < 200.0,
            "mercator corner not ENU metres: {p:?}"
        );
    }

    #[test]
    fn parse_metashape_offset_txt() {
        let o = parse_metashape_offset("E: 3000000\nN: -2000000\nA: 100\n").unwrap();
        assert_eq!(o.easting, 3_000_000.0);
        assert_eq!(o.northing, -2_000_000.0);
        assert_eq!(o.height, 100.0);
    }

    #[test]
    fn offset_local_aabb_matches_world_mercator_origin() {
        let off = SourceOffset {
            easting: 3_000_000.0,
            northing: -2_000_000.0,
            height: 100.0,
        };
        let min = [200.0, 10.0, -80.0];
        let max = [300.0, 40.0, -20.0];
        assert!(!looks_web_mercator_yup(min, max));
        let o = mercator_origin_yup_offset(min, max, off);
        assert!(
            (o.lon_deg - 26.9517043118).abs() < 1e-4,
            "lon {}",
            o.lon_deg
        );
        assert!(
            (o.lat_deg - (-17.6784862924)).abs() < 1e-4,
            "lat {}",
            o.lat_deg
        );
        assert!((o.height_m - 125.0).abs() < 0.01, "h {}", o.height_m);
        let mid = [
            ((min[0] + max[0]) * 0.5) as f32,
            ((min[1] + max[1]) * 0.5) as f32,
            ((min[2] + max[2]) * 0.5) as f32,
        ];
        let p = EnuFrame::new(o).mercator_yup_offset_to_enu_yup(mid, off);
        assert!(
            p[0].abs() < 2.0 && p[1].abs() < 2.0 && p[2].abs() < 2.0,
            "centroid should bake near ENU origin, got {p:?}"
        );
    }

    #[test]
    fn mercator_shift_origin_pins_enu_at_e_n_a() {
        let off = SourceOffset {
            easting: 3_000_000.0,
            northing: -2_000_000.0,
            height: 100.0,
        };
        let o = mercator_shift_origin(off);
        let (lon, lat) = mercator_to_geodetic(off.easting, off.northing);
        assert!((o.lon_deg - lon).abs() < 1e-12);
        assert!((o.lat_deg - lat).abs() < 1e-12);
        assert!((o.height_m - 100.0).abs() < 1e-12);
        assert!(
            (o.lon_deg - 26.9494585236).abs() < 0.01,
            "lon {}",
            o.lon_deg
        );
        assert!(
            (o.lat_deg - (-17.6789142383)).abs() < 0.02,
            "lat {}",
            o.lat_deg
        );
    }
}
