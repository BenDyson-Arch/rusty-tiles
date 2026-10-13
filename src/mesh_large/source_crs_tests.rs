//! Source CRS capture is audited against an independent oracle: the EPSG:3857 inverse and
//! WGS84 geodetic-to-ECEF equations written here, with Jacobians from the oracle's own small
//! central differences and normals from mapped tangent-plane edges. Expected values never
//! call the resolver, `georef` or the production Jacobian. Sensitivity controls show that a
//! skipped normal transform, an f32 offset and a wrong ENU origin each fail the same checks.
use super::*;
use crate::{
    mesh_large::{
        config::{Jobs, Profile, TreeConfig},
        full, TilerJobs, TilerRequest,
    },
    RunControl,
};
use serde_json::{json, Value};
use std::{
    fs,
    io::Read,
    num::NonZeroUsize,
    path::{Path, PathBuf},
};

const A: f64 = 6_378_137.0;
const FLATTENING: f64 = 1.0 / 298.257_223_563;
// Metashape-style shift near the mgal site, with fractions that f32 cannot hold.
const OFFSET: [f64; 3] = [14_812_000.4375, -1_384_000.3125, 100.25];
const HEIGHT_OFFSET: f64 = 0.5;
const NODE: [f64; 3] = [3., 0.5, -2.];
const GRID: usize = 5;

type V3 = [f64; 3];

fn sub(a: V3, b: V3) -> V3 {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}
fn cross(a: V3, b: V3) -> V3 {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}
fn dot(a: V3, b: V3) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}
fn unit(a: V3) -> V3 {
    let length = dot(a, a).sqrt();
    a.map(|v| v / length)
}
fn distance(a: V3, b: V3) -> f64 {
    let d = sub(a, b);
    dot(d, d).sqrt()
}
fn angle(a: V3, b: V3) -> f64 {
    dot(unit(a), unit(b)).clamp(-1., 1.).acos()
}

/// Independent WGS84 geodetic (radians, metres) to ECEF.
fn geodetic(lon: f64, lat: f64, height: f64) -> V3 {
    let e2 = FLATTENING * (2. - FLATTENING);
    let n = A / (1. - e2 * lat.sin().powi(2)).sqrt();
    [
        (n + height) * lat.cos() * lon.cos(),
        (n + height) * lat.cos() * lon.sin(),
        (n * (1. - e2) + height) * lat.sin(),
    ]
}
/// Post-node Y-up source position to geodetic through the EPSG:3857 inverse equations.
fn oracle_geodetic(world: V3, offset: V3, height_offset: f64) -> V3 {
    let easting = world[0] + offset[0];
    let northing = offset[1] - world[2];
    let height = world[1] + offset[2] + height_offset;
    [easting / A, (northing / A).sinh().atan(), height]
}
fn oracle(world: V3) -> V3 {
    let [lon, lat, h] = oracle_geodetic(world, OFFSET, HEIGHT_OFFSET);
    geodetic(lon, lat, h)
}
/// ECEF Jacobian columns of the oracle map by a 1 cm central difference.
fn oracle_jacobian(world: V3) -> [V3; 3] {
    std::array::from_fn(|axis| {
        let mut a = world;
        let mut b = world;
        a[axis] -= 0.01;
        b[axis] += 0.01;
        sub(oracle(b), oracle(a)).map(|v| v / 0.02)
    })
}
fn apply(columns: &[V3; 3], v: V3) -> V3 {
    std::array::from_fn(|i| (0..3).map(|j| columns[j][i] * v[j]).sum())
}
/// The mapped tangent plane's normal: two source edges orthogonal to `n`, mapped and crossed.
fn oracle_normal(world: V3, n: V3) -> V3 {
    let columns = oracle_jacobian(world);
    let helper = if n[0].abs() < 0.9 {
        [1., 0., 0.]
    } else {
        [0., 1., 0.]
    };
    let u = unit(cross(helper, n));
    let v = cross(n, u);
    let mapped = cross(apply(&columns, u), apply(&columns, v));
    let orientation = dot(cross(columns[0], columns[1]), columns[2]).signum();
    mapped.map(|x| x * orientation)
}
/// Independent ENU-to-ECEF at a geodetic origin, as a column-major 4x4.
fn oracle_root([lon, lat, h]: V3) -> [f64; 16] {
    let east = [-lon.sin(), lon.cos(), 0.];
    let north = [-lat.sin() * lon.cos(), -lat.sin() * lon.sin(), lat.cos()];
    let up = [lat.cos() * lon.cos(), lat.cos() * lon.sin(), lat.sin()];
    let origin = geodetic(lon, lat, h);
    let mut m = [0.; 16];
    for (column, v) in [east, north, up, origin].iter().enumerate() {
        m[column * 4..column * 4 + 3].copy_from_slice(v);
    }
    m[15] = 1.;
    m
}
/// Local glTF Y-up position or direction to ECEF through a root transform.
fn place(root: &[f64; 16], local: [f32; 3], point: bool) -> V3 {
    let tile = [
        f64::from(local[0]),
        -f64::from(local[2]),
        f64::from(local[1]),
    ];
    std::array::from_fn(|i| {
        (0..3).map(|j| root[j * 4 + i] * tile[j]).sum::<f64>()
            + if point { root[12 + i] } else { 0. }
    })
}

struct Fixture {
    input: PathBuf,
    /// Source accessor positions (before the node translation), normals and tangents.
    positions: Vec<V3>,
    normals: Vec<V3>,
    tangents: Vec<[f64; 4]>,
}
impl Fixture {
    fn world(&self, vertex: usize) -> V3 {
        std::array::from_fn(|i| self.positions[vertex][i] + NODE[i])
    }
    /// The geodetic origin the frame must choose: the centre of the used post-node bounds.
    fn origin(&self) -> V3 {
        let world: Vec<_> = (0..self.positions.len()).map(|v| self.world(v)).collect();
        let centre: V3 = std::array::from_fn(|i| {
            let min = world.iter().map(|p| p[i]).fold(f64::INFINITY, f64::min);
            let max = world.iter().map(|p| p[i]).fold(f64::NEG_INFINITY, f64::max);
            (min + max) * 0.5
        });
        oracle_geodetic(centre, OFFSET, HEIGHT_OFFSET)
    }
}

/// A tilted 4x4-cell Y-up surface (40 m) with authored normals and tangents, under a
/// translated node. Every value is exactly representable in f32.
fn fixture(parent: &Path) -> Fixture {
    fs::create_dir_all(parent).unwrap();
    let mut positions = Vec::new();
    let mut normals = Vec::new();
    let mut tangents = Vec::new();
    for j in 0..GRID {
        for i in 0..GRID {
            let (x, z) = (i as f64 * 10., -(j as f64) * 10.);
            positions.push([x, 2. + 0.25 * x - 0.125 * z + ((i * j) % 3) as f64, z]);
            let n = unit([-0.25 + 0.0625 * i as f64, 1., 0.125 - 0.0625 * j as f64]);
            normals.push(n.map(|v| f64::from(v as f32)));
            let t = unit(cross([0., 0.25 * j as f64, 1.], n));
            tangents.push([
                f64::from(t[0] as f32),
                f64::from(t[1] as f32),
                f64::from(t[2] as f32),
                1.,
            ]);
        }
    }
    let mut indices = Vec::new();
    for j in 0..GRID - 1 {
        for i in 0..GRID - 1 {
            let a = (j * GRID + i) as u16;
            let b = a + 1;
            let c = a + GRID as u16;
            let d = c + 1;
            indices.extend([a, c, b, b, c, d]);
        }
    }
    let mut bytes = Vec::new();
    let mut views = Vec::new();
    for values in [
        positions.iter().flatten().copied().collect::<Vec<_>>(),
        normals.iter().flatten().copied().collect(),
        tangents.iter().flatten().copied().collect(),
    ] {
        let start = bytes.len();
        for value in values {
            bytes.extend((value as f32).to_le_bytes());
        }
        views.push(json!({"buffer":0,"byteOffset":start,"byteLength":bytes.len()-start}));
    }
    let start = bytes.len();
    for index in &indices {
        bytes.extend(index.to_le_bytes());
    }
    views.push(json!({"buffer":0,"byteOffset":start,"byteLength":bytes.len()-start}));
    let min: V3 =
        std::array::from_fn(|i| positions.iter().map(|p| p[i]).fold(f64::INFINITY, f64::min));
    let max: V3 = std::array::from_fn(|i| {
        positions
            .iter()
            .map(|p| p[i])
            .fold(f64::NEG_INFINITY, f64::max)
    });
    let count = positions.len();
    let doc = json!({"asset":{"version":"2.0"},"scene":0,"scenes":[{"nodes":[0]}],
        "nodes":[{"mesh":0,"translation":NODE}],
        "meshes":[{"primitives":[{"attributes":{"POSITION":0,"NORMAL":1,"TANGENT":2},"indices":3}]}],
        "buffers":[{"uri":"data.bin","byteLength":bytes.len()}],"bufferViews":views,
        "accessors":[
            {"bufferView":0,"componentType":5126,"count":count,"type":"VEC3","min":min,"max":max},
            {"bufferView":1,"componentType":5126,"count":count,"type":"VEC3"},
            {"bufferView":2,"componentType":5126,"count":count,"type":"VEC4"},
            {"bufferView":3,"componentType":5123,"count":indices.len(),"type":"SCALAR"}]});
    fs::write(parent.join("data.bin"), &bytes).unwrap();
    let input = parent.join("scene.gltf");
    fs::write(&input, serde_json::to_vec(&doc).unwrap()).unwrap();
    Fixture {
        input,
        positions,
        normals,
        tangents,
    }
}

fn spec(offset: V3) -> MeshSourceCrs {
    MeshSourceCrs::from_parameters(
        Some("EPSG:3857"),
        Some(SourceAxes::YUp),
        Some(&offset),
        Some(HEIGHT_OFFSET),
    )
    .unwrap()
    .unwrap()
}

#[test]
fn parameters_are_explicit_and_local_admits_no_crs_parameter() {
    let kind = |result: Result<Option<MeshSourceCrs>>| result.unwrap_err().kind();
    assert_eq!(
        MeshSourceCrs::from_parameters(None, None, None, None).unwrap(),
        None
    );
    assert_eq!(
        MeshSourceCrs::from_parameters(Some(" Local "), None, None, None).unwrap(),
        None
    );
    for (axes, offset, height) in [
        (Some(SourceAxes::Xyz), None, None),
        (None, Some(&[1., 2.][..]), None),
        (None, None, Some(0.)),
    ] {
        assert_eq!(
            kind(MeshSourceCrs::from_parameters(
                Some("local"),
                axes,
                offset,
                height
            )),
            JobErrorKind::InvalidRequest
        );
    }
    for legacy in ["auto", "geographic", "WGS84", "3857", "Mercator"] {
        assert_eq!(
            kind(MeshSourceCrs::from_parameters(
                Some(legacy),
                Some(SourceAxes::Xyz),
                None,
                Some(0.)
            )),
            JobErrorKind::InvalidRequest,
            "{legacy}"
        );
    }
    let crs = Some("EPSG:32632");
    assert_eq!(
        kind(MeshSourceCrs::from_parameters(crs, None, None, Some(0.))),
        JobErrorKind::InvalidRequest
    );
    assert_eq!(
        kind(MeshSourceCrs::from_parameters(
            crs,
            Some(SourceAxes::Xyz),
            None,
            None
        )),
        JobErrorKind::InvalidRequest
    );
    for offset in [&[1.][..], &[1., 2., 3., 4.], &[f64::NAN, 0.]] {
        assert_eq!(
            kind(MeshSourceCrs::from_parameters(
                crs,
                Some(SourceAxes::Xyz),
                Some(offset),
                Some(0.)
            )),
            JobErrorKind::InvalidRequest
        );
    }
    let two =
        MeshSourceCrs::from_parameters(crs, Some(SourceAxes::Xyz), Some(&[5e5, 1e6]), Some(-3.))
            .unwrap()
            .unwrap();
    assert_eq!(
        two.offset,
        SourceOffset {
            easting: 5e5,
            northing: 1e6,
            height: 0.
        }
    );
    two.check().unwrap();
    // Syntax errors are request errors; grid-requiring CRS need native PROJ.
    let bad =
        MeshSourceCrs::from_parameters(Some("EPSG:abc"), Some(SourceAxes::Xyz), None, Some(0.))
            .unwrap()
            .unwrap();
    assert_eq!(
        bad.check().unwrap_err().kind(),
        JobErrorKind::InvalidRequest
    );
    #[cfg(not(feature = "native-geospatial"))]
    {
        let grid = MeshSourceCrs::from_parameters(
            Some("EPSG:27700"),
            Some(SourceAxes::Xyz),
            None,
            Some(0.),
        )
        .unwrap()
        .unwrap();
        assert_eq!(grid.check().unwrap_err().kind(), JobErrorKind::Unsupported);
    }
}

#[test]
fn rigid_placement_and_source_crs_are_exclusive_before_any_output() {
    let work = tempfile::tempdir().unwrap();
    let fixture = fixture(&work.path().join("src"));
    let output = work.path().join("out.3tz");
    let failure = crate::mesh_to_tileset(
        TilerRequest::new(&fixture.input, &output)
            .with_placement(MeshPlacement::Wgs84 {
                anchor_degrees_metres: [133., -12., 0.],
                orientation_xyzw: [0., 0., 0., 1.],
                scene_offset_metres: [0.; 3],
            })
            .with_source_crs(spec(OFFSET)),
        &RunControl::default(),
    )
    .unwrap_err();
    assert_eq!(failure.error.kind(), JobErrorKind::InvalidRequest);
    assert!(!output.exists());
}

struct Captured {
    root: [f64; 16],
    report: MeshSourceCrsReport,
    /// (source vertex, local position, normal, tangent) for every captured corner.
    corners: Vec<(usize, [f32; 3], [f32; 3], [f32; 4])>,
}

fn capture(input: &Path, scratch: &Path, offset: V3) -> Captured {
    let mut config = TreeConfig::new(Profile::Balanced);
    config.jobs = Jobs::Fixed(NonZeroUsize::new(1).unwrap());
    config.scratch_dir = Some(scratch.to_owned());
    config.source_crs = Some(spec(offset));
    let captured = full::capture(input, &config, &mut || Ok(())).unwrap();
    let mut corners = Vec::new();
    for row in captured.triangles.reader().unwrap() {
        let row = row.unwrap();
        for corner in 0..3 {
            corners.push((
                row.triangle.vertices[corner] as usize,
                row.triangle.positions[corner],
                row.corners[corner].normal,
                row.corners[corner].tangent,
            ));
        }
    }
    let root = captured.placement.as_ref().unwrap().transform();
    let report = captured.report.source_crs.clone().unwrap();
    captured.close().unwrap();
    Captured {
        root,
        report,
        corners,
    }
}

/// Largest position (metres), normal and tangent (radians) errors against the oracle.
fn errors(
    fixture: &Fixture,
    root: &[f64; 16],
    corners: &[(usize, [f32; 3], [f32; 3], [f32; 4])],
) -> [f64; 3] {
    let mut worst = [0_f64; 3];
    for &(vertex, position, normal, tangent) in corners {
        let world = fixture.world(vertex);
        worst[0] = worst[0].max(distance(place(root, position, true), oracle(world)));
        let expected = oracle_normal(world, fixture.normals[vertex]);
        worst[1] = worst[1].max(angle(place(root, normal, false), expected));
        let t = fixture.tangents[vertex];
        let expected = apply(&oracle_jacobian(world), [t[0], t[1], t[2]]);
        worst[2] = worst[2].max(angle(
            place(root, [tangent[0], tangent[1], tangent[2]], false),
            expected,
        ));
        assert_eq!(
            f64::from(tangent[3]),
            t[3],
            "positive Jacobian keeps handedness"
        );
    }
    worst
}

#[test]
fn capture_maps_positions_normals_and_tangents_onto_the_independent_oracle() {
    let work = tempfile::tempdir().unwrap();
    let fixture = fixture(&work.path().join("src"));
    let scratch = work.path().join("scratch");
    fs::create_dir(&scratch).unwrap();
    let captured = capture(&fixture.input, &scratch, OFFSET);
    assert_eq!(captured.corners.len(), (GRID - 1) * (GRID - 1) * 6);

    // Root: ENU at the mapped centre of the used post-node source bounds.
    let origin = fixture.origin();
    let expected = oracle_root(origin);
    for (got, want) in captured.root.iter().zip(expected) {
        assert!(
            (got - want).abs() < 1e-6,
            "{:?} != {expected:?}",
            captured.root
        );
    }
    let [lon, lat, h] = captured.report.origin_degrees_metres;
    assert!((lon.to_radians() - origin[0]).abs() < 1e-12);
    assert!((lat.to_radians() - origin[1]).abs() < 1e-12);
    assert!((h - origin[2]).abs() < 1e-6);
    assert_eq!(captured.report.operation, "portable");
    assert_eq!(captured.report.axes, "y-up");
    assert_eq!(captured.report.source_offset, OFFSET);

    let [position, normal, tangent] = errors(&fixture, &captured.root, &captured.corners);
    eprintln!(
        "capture errors: position {position:e} m, normal {normal:e} rad, tangent {tangent:e} rad"
    );
    assert!(position < 1e-4, "position error {position}");
    assert!(normal < 2e-6, "normal error {normal}");
    assert!(tangent < 2e-6, "tangent error {tangent}");

    // Control: the same capture with the normal transform skipped (the axis map only).
    let skipped: Vec<_> = captured
        .corners
        .iter()
        .map(|&(vertex, position, _, tangent)| {
            (
                vertex,
                position,
                fixture.normals[vertex].map(|v| v as f32),
                tangent,
            )
        })
        .collect();
    let [_, skipped_normal, _] = errors(&fixture, &captured.root, &skipped);
    eprintln!("control: skipped normal transform {skipped_normal:e} rad");
    assert!(
        skipped_normal > 1e-3,
        "skipped-normal control {skipped_normal}"
    );

    // Control: the offset rounded to f32 before it is added.
    let rounded = OFFSET.map(|v| f64::from(v as f32));
    let f32_offset = capture(&fixture.input, &scratch, rounded);
    let [shifted, _, _] = errors(&fixture, &f32_offset.root, &f32_offset.corners);
    eprintln!("control: f32 offset {shifted:e} m");
    assert!(shifted > 0.1, "f32-offset control {shifted}");

    // Control: the right local coordinates under an ENU origin 1e-6 degree away.
    let wrong = oracle_root([origin[0] + 1e-6_f64.to_radians(), origin[1], origin[2]]);
    let [misplaced, _, _] = errors(&fixture, &wrong, &captured.corners);
    eprintln!("control: wrong ENU origin {misplaced:e} m");
    assert!(misplaced > 0.05, "wrong-origin control {misplaced}");
}

fn members(path: &Path) -> std::collections::BTreeMap<String, Vec<u8>> {
    let mut zip = zip::ZipArchive::new(fs::File::open(path).unwrap()).unwrap();
    (0..zip.len())
        .map(|i| {
            let mut entry = zip.by_index(i).unwrap();
            let mut bytes = Vec::new();
            entry.read_to_end(&mut bytes).unwrap();
            (entry.name().to_owned(), bytes)
        })
        .collect()
}

/// Every root-node POSITION of a GLB as a client decodes it: meshopt (validator decoder),
/// then `u * scale + translation` in f32 for quantized positions.
fn glb_positions(bytes: &[u8]) -> Vec<[f32; 3]> {
    use crate::content_integrity::meshopt::{decode, Mode};
    let u32_at = |i: usize| u32::from_le_bytes(bytes[i..i + 4].try_into().unwrap()) as usize;
    let json_length = u32_at(12);
    let doc: Value = serde_json::from_slice(&bytes[20..20 + json_length]).unwrap();
    let bin = &bytes[20 + json_length + 8..];
    let mut out = Vec::new();
    for root in doc["scenes"][0]["nodes"].as_array().unwrap() {
        let node = &doc["nodes"][root.as_u64().unwrap() as usize];
        let vector = |key: &str, default: f32| -> [f32; 3] {
            std::array::from_fn(|i| {
                node.get(key)
                    .map_or(default, |v| v[i].as_f64().unwrap() as f32)
            })
        };
        let (translation, scale) = (vector("translation", 0.), vector("scale", 1.));
        let Some(mesh) = node["mesh"].as_u64() else {
            continue;
        };
        for primitive in doc["meshes"][mesh as usize]["primitives"]
            .as_array()
            .unwrap()
        {
            let accessor =
                &doc["accessors"][primitive["attributes"]["POSITION"].as_u64().unwrap() as usize];
            let view = &doc["bufferViews"][accessor["bufferView"].as_u64().unwrap() as usize];
            let count = accessor["count"].as_u64().unwrap() as usize;
            let component = accessor["componentType"].as_u64().unwrap();
            let (data, stride) = match view["extensions"].get("EXT_meshopt_compression") {
                Some(ext) => {
                    let start = ext["byteOffset"].as_u64().unwrap_or(0) as usize;
                    let stride = ext["byteStride"].as_u64().unwrap() as usize;
                    let compressed =
                        &bin[start..start + ext["byteLength"].as_u64().unwrap() as usize];
                    (
                        decode(
                            compressed,
                            ext["count"].as_u64().unwrap() as usize,
                            stride,
                            Mode::Attributes,
                        )
                        .unwrap(),
                        stride,
                    )
                }
                None => {
                    let start = view["byteOffset"].as_u64().unwrap_or(0) as usize;
                    let length = view["byteLength"].as_u64().unwrap() as usize;
                    let size = if component == 5123 { 2 } else { 4 };
                    (
                        bin[start..start + length].to_vec(),
                        view["byteStride"].as_u64().map_or(3 * size, |s| s as usize),
                    )
                }
            };
            let offset = accessor["byteOffset"].as_u64().unwrap_or(0) as usize;
            for i in 0..count {
                let e = &data[offset + i * stride..];
                out.push(std::array::from_fn(|a| match component {
                    5123 => {
                        f32::from(u16::from_le_bytes([e[2 * a], e[2 * a + 1]])) * scale[a]
                            + translation[a]
                    }
                    5126 => {
                        f32::from_le_bytes(e[4 * a..4 * a + 4].try_into().unwrap()) * scale[a]
                            + translation[a]
                    }
                    other => panic!("position component {other}"),
                }));
            }
        }
    }
    out
}

#[test]
fn published_archive_validates_on_the_oracle_and_is_identical_across_jobs() {
    let work = tempfile::tempdir().unwrap();
    let fixture = fixture(&work.path().join("src"));
    let mut previous: Option<Vec<u8>> = None;
    for jobs in [1, 3] {
        let scratch = work.path().join(format!("scratch-{jobs}"));
        fs::create_dir(&scratch).unwrap();
        let output = work.path().join(format!("crs-{jobs}.3tz"));
        let result = crate::mesh_to_tileset(
            TilerRequest::new(&fixture.input, &output)
                .with_jobs(TilerJobs::Maximum(NonZeroUsize::new(jobs).unwrap()))
                .with_scratch_dir(&scratch)
                .with_source_crs(spec(OFFSET)),
            &RunControl::default(),
        )
        .unwrap();
        let report = &result.report;
        assert_eq!(report.source_coordinates, "source-crs");
        assert_eq!(report.coordinates, "wgs84-ecef");
        let root = report.root_transform.unwrap();
        for (got, want) in root.iter().zip(oracle_root(fixture.origin())) {
            assert!((got - want).abs() < 1e-6);
        }
        let archive = members(&output);
        let tileset: Value = serde_json::from_slice(&archive["tileset.json"]).unwrap();
        let published: Vec<f64> =
            serde_json::from_value(tileset["root"]["transform"].clone()).unwrap();
        assert_eq!(published, root);
        let conversion: Value = serde_json::from_slice(&archive["conversion.json"]).unwrap();
        assert_eq!(conversion["source_crs"]["definition"], "EPSG:3857");
        // Every decoded vertex of every payload is a quantized source vertex on the oracle.
        let source: Vec<V3> = (0..fixture.positions.len())
            .map(|v| oracle(fixture.world(v)))
            .collect();
        let mut worst = 0_f64;
        let mut decoded = 0;
        for (name, bytes) in &archive {
            if !name.ends_with(".glb") {
                continue;
            }
            for p in glb_positions(bytes) {
                {
                    let ecef = place(&root, p, true);
                    let nearest = source
                        .iter()
                        .map(|s| distance(*s, ecef))
                        .fold(f64::INFINITY, f64::min);
                    worst = worst.max(nearest);
                    decoded += 1;
                }
            }
        }
        assert!(decoded >= (GRID - 1) * (GRID - 1) * 6);
        eprintln!("jobs {jobs}: published vertex error {worst:e} m over {decoded} corners");
        assert!(worst < 5e-3, "published vertex error {worst}");
        crate::validate::inspect(crate::validate::ValidationRequest::new(&output)).unwrap();
        let bytes = fs::read(&output).unwrap();
        if let Some(prior) = &previous {
            assert_eq!(
                prior, &bytes,
                "workers and scratch do not change the archive"
            );
        }
        previous = Some(bytes);
    }
}

/// The legacy general bake (`mesh-to-3tz --source-crs EPSG:3857 --source-axes y-up`) and
/// its Web Mercator adapter (`--source-crs epsg:3857 --source-offset ...`) place the same
/// vertices where the new capture frame does.
#[test]
fn legacy_general_and_web_mercator_bakes_agree_with_the_capture_frame() {
    use crate::georef::{root_transform, Cartographic, SourceCrs};
    use crate::mesh::{BakeToEnu, Scene, Triangle, Vertex};
    let work = tempfile::tempdir().unwrap();
    let fixture = fixture(&work.path().join("src"));
    let world: Vec<V3> = (0..fixture.positions.len())
        .map(|v| fixture.world(v))
        .collect();
    let scene = || Scene {
        vertices: world
            .iter()
            .zip(&fixture.normals)
            .map(|(p, n)| Vertex {
                pos: p.map(|v| v as f32),
                nrm: n.map(|v| v as f32),
                uv: [0.; 2],
            })
            .collect(),
        triangles: vec![Triangle {
            verts: [0, 1, 2],
            image: None,
            material: None,
        }],
        images: vec![],
        source_bytes: 0,
        materials: vec![],
        triangle_nodes: vec![],
        node_names: Default::default(),
    };
    let ecef = |root: [f64; 16], scene: &Scene| -> Vec<V3> {
        scene
            .vertices
            .iter()
            .map(|v| place(&root, v.pos, true))
            .collect()
    };
    let offset = SourceOffset {
        easting: OFFSET[0],
        northing: OFFSET[1],
        height: OFFSET[2],
    };
    // The legacy adapter has no height offset: compare it at zero.
    for height_offset in [HEIGHT_OFFSET, 0.] {
        let spec = MeshSourceCrs::from_parameters(
            Some("EPSG:3857"),
            Some(SourceAxes::YUp),
            Some(&OFFSET),
            Some(height_offset),
        )
        .unwrap()
        .unwrap();
        let min: V3 =
            std::array::from_fn(|i| world.iter().map(|p| p[i]).fold(f64::INFINITY, f64::min));
        let max: V3 =
            std::array::from_fn(|i| world.iter().map(|p| p[i]).fold(f64::NEG_INFINITY, f64::max));
        let mut frame = SourceFrame::resolve(&spec, [min, max]).unwrap();
        let root = frame.placement().unwrap().0.transform();
        let new: Vec<V3> = world
            .iter()
            .map(|p| place(&root, frame.position(*p).unwrap(), true))
            .collect();

        let mut options = crate::tile::MeshTo3tzOptions {
            source_axes: Some(SourceAxes::YUp),
            height_offset: Some(height_offset),
            source_offset: Some(offset),
            ..Default::default()
        };
        options.set_source_crs("EPSG:3857").unwrap();
        let mut general = scene();
        let origin = crate::mesh_crs::bake(&mut general, "EPSG:3857", &options).unwrap();
        let general = ecef(root_transform(origin, None), &general);
        let worst = new
            .iter()
            .zip(&general)
            .map(|(a, b)| distance(*a, *b))
            .fold(0., f64::max);
        eprintln!("height offset {height_offset}: legacy general bake {worst:e} m");
        assert!(worst < 1e-4, "legacy general bake differs by {worst}");

        if height_offset == 0. {
            let mut adapter = scene();
            let bake = crate::mesh::bake_to_enu(
                &mut adapter,
                &BakeToEnu {
                    prefer: None::<Cartographic>,
                    crs: SourceCrs::WebMercator,
                    offset: Some(offset),
                },
            )
            .unwrap();
            let adapter = ecef(root_transform(bake.origin, None), &adapter);
            let worst = new
                .iter()
                .zip(&adapter)
                .map(|(a, b)| distance(*a, *b))
                .fold(0., f64::max);
            eprintln!("legacy Web Mercator adapter {worst:e} m");
            assert!(
                worst < 1e-4,
                "legacy Web Mercator adapter differs by {worst}"
            );
        }
    }
}
