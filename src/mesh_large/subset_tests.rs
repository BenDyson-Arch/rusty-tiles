//! Subset bakes: the region rules, the selection's invariance (a subset capture is the full
//! capture's records, filtered), and the published archive against a full bake.
use super::*;
use crate::{
    mesh_large::{
        config::Jobs,
        full::{self, tests as full_tests},
        TilerJobs, TilerRequest,
    },
    spill::Record,
    tile_view::{TileRole, TileSet},
    MeshSourceCrs, RunControl,
};
use serde_json::json;
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    io::Read,
    num::NonZeroUsize,
    path::PathBuf,
};

fn geojson(value: Value) -> Result<MeshSubset> {
    MeshSubset::polygon_geojson(&serde_json::to_vec(&value).unwrap())
}

fn rectangle([x0, y0, x1, y1]: [f64; 4]) -> Value {
    json!([[x0, y0], [x1, y0], [x1, y1], [x0, y1], [x0, y0]])
}

/// The rectangle, ring reversed.
fn reversed([x0, y0, x1, y1]: [f64; 4]) -> Value {
    json!([[x0, y0], [x0, y1], [x1, y1], [x1, y0], [x0, y0]])
}

#[test]
fn bbox_boundary_is_half_open_and_equals_its_rectangle_polygon() {
    let bbox = MeshSubset::bbox(&[0., 0., 3., 3.]).unwrap();
    for (point, inside) in [
        ([0., 0.], true),
        ([3., 0.], false),
        ([0., 3.], false),
        ([3., 3.], false),
        ([2.999_999, 2.999_999], true),
        ([-1e-300, 1.], false),
        ([1., -0.], true),
        ([f64::NAN, 1.], false),
    ] {
        assert_eq!(bbox.contains(point), inside, "{point:?}");
    }
    // The rectangle polygon, either winding, decides every point as the bbox does: on its
    // corners, edges, inside and outside.
    let coordinates = [-1., 0., 0.5, 1., 1.5, 2.999_999_999, 3., 4.];
    for ring in [rectangle([0., 0., 3., 3.]), reversed([0., 0., 3., 3.])] {
        let polygon = geojson(json!({"type":"Polygon","coordinates":[ring]})).unwrap();
        for x in coordinates {
            for y in coordinates {
                assert_eq!(polygon.contains([x, y]), bbox.contains([x, y]), "{x} {y}");
            }
        }
    }
}

#[test]
fn polygons_sharing_an_edge_partition_the_plane_and_holes_are_half_open() {
    // Two quadrilaterals sharing the slanted edge (2, 0)-(1, 3), wound opposite ways.
    let left = json!([[0., 0.], [2., 0.], [1., 3.], [0., 3.], [0., 0.]]);
    let right = json!([[1., 3.], [2., 0.], [4., 0.], [4., 3.], [1., 3.]]);
    let left = geojson(json!({"type":"Polygon","coordinates":[left]})).unwrap();
    let right = geojson(json!({"type":"Polygon","coordinates":[right]})).unwrap();
    let union = MeshSubset::bbox(&[0., 0., 4., 3.]).unwrap();
    let mut on_edge = 0;
    for k in 0..=4096 {
        // Points on the shared edge (each computed as a centroid would be) and beside it.
        let t = f64::from(k) / 4096.;
        let edge = [2. - t, 3. * t];
        for point in [edge, [edge[0] + 1e-9, edge[1]], [edge[0] - 1e-9, edge[1]]] {
            let (l, r) = (left.contains(point), right.contains(point));
            assert!(!(l && r), "{point:?} in both");
            assert_eq!(l || r, union.contains(point), "{point:?}");
        }
        on_edge += usize::from(edge[1] < 3.);
    }
    assert!(on_edge > 4000);
    // A hole is half-open like a bbox: its min edges leave the polygon, its max edges stay.
    let holed = geojson(json!({"type":"Polygon","coordinates":[
        rectangle([0., 0., 10., 10.]), reversed([3., 3., 7., 7.])]}))
    .unwrap();
    for (point, inside) in [
        ([1., 1.], true),
        ([5., 5.], false),
        ([3., 5.], false),
        ([7., 5.], true),
        ([5., 3.], false),
        ([5., 7.], true),
        ([9.5, 9.5], true),
        ([10., 5.], false),
    ] {
        assert_eq!(holed.contains(point), inside, "{point:?}");
    }
    // A MultiPolygon is the union of its members, even where they overlap.
    let overlapping = geojson(json!({"type":"MultiPolygon","coordinates":[
        [rectangle([0., 0., 2., 2.])], [rectangle([1., 1., 3., 3.])]]}))
    .unwrap();
    for (point, inside) in [([0.5, 0.5], true), ([1.5, 1.5], true), ([2.5, 2.5], true)] {
        assert_eq!(overlapping.contains(point), inside, "{point:?}");
    }
}

#[test]
fn many_edge_polygons_band_their_edges_and_agree_with_the_plain_crossing_test() {
    // A wavy circle of 5,000 short edges and a star of 5,000 long ones: the banded index
    // answers as a loop over every edge does; short edges get many bands.
    let n = 5000;
    let ring = |radius: &dyn Fn(u32) -> f64| -> Vec<[f64; 2]> {
        (0..=n)
            .map(|k| {
                let angle = std::f64::consts::TAU * f64::from(k % n) / f64::from(n);
                [radius(k % n) * angle.cos(), radius(k % n) * angle.sin()]
            })
            .collect()
    };
    let wavy = ring(&|k| 10. + 0.5 * (f64::from(k) * 0.37).sin());
    let star = ring(&|k| if k % 2 == 0 { 10. } else { 4. });
    for (name, ring, min_bands) in [("wavy", wavy, 1000), ("star", star, 1)] {
        let polygon = geojson(json!({"type":"Polygon","coordinates":[ring]})).unwrap();
        let Region::Polygon(index) = &polygon.region else {
            panic!("a polygon")
        };
        assert!(
            index.starts.len() >= min_bands,
            "{name}: {} bands",
            index.starts.len()
        );
        let plain = |p: [f64; 2]| index.edges.iter().filter(|e| e.crosses(p)).count() % 2 == 1;
        let mut state = 0x2545_f491_4f6c_dd1du64;
        for _ in 0..20_000 {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            let x = (state % 24_000) as f64 / 1000. - 12.;
            let y = ((state >> 20) % 24_000) as f64 / 1000. - 12.;
            assert_eq!(polygon.contains([x, y]), plain([x, y]), "{name}: {x} {y}");
        }
    }
}

#[test]
fn invalid_region_requests_are_refused_clearly() {
    for (values, message) in [
        (vec![0., 0., 1.], "exactly four"),
        (vec![0., 0., f64::INFINITY, 1.], "finite"),
        (vec![1., 0., 1., 2.], "minx < maxx"),
        (vec![0., 2., 1., 2.], "minx < maxx and miny < maxy"),
    ] {
        let error = MeshSubset::bbox(&values).unwrap_err();
        assert_eq!(error.kind(), JobErrorKind::InvalidRequest);
        assert!(error.to_string().contains(message), "{error}");
    }
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("region.geojson");
    fs::write(
        &file,
        serde_json::to_vec(&json!({"type":"Polygon","coordinates":[rectangle([0., 0., 1., 1.])]}))
            .unwrap(),
    )
    .unwrap();
    let both = MeshSubset::from_parameters(Some(&[0., 0., 1., 1.]), Some(&file)).unwrap_err();
    assert!(both.to_string().contains("only one"), "{both}");
    assert!(MeshSubset::from_parameters(None, None).unwrap().is_none());
    let missing = MeshSubset::polygon_file(&dir.path().join("missing.geojson")).unwrap_err();
    assert_eq!(missing.kind(), JobErrorKind::Io);
    // With a source CRS the region is geographic.
    for values in [[-181., 0., 0., 1.], [0., -91., 1., 1.], [0., 0., 1., 90.5]] {
        let error = MeshSubset::bbox(&values)
            .unwrap()
            .check_geographic()
            .unwrap_err();
        assert!(
            error.to_string().contains("longitude in [-180, 180]"),
            "{error}"
        );
    }
    MeshSubset::bbox(&[-180., -90., 180., 90.])
        .unwrap()
        .check_geographic()
        .unwrap();
}

#[test]
fn invalid_geojson_is_refused_with_a_clear_message() {
    let square = rectangle([0., 0., 1., 1.]);
    let cases: Vec<(Vec<u8>, &str)> = vec![
        (b"{\"type\": \"Polygon\", ".to_vec(), "not valid GeoJSON"),
        (b"[1, 2]".to_vec(), "expected an object"),
        (
            serde_json::to_vec(&json!({"coordinates":[square]})).unwrap(),
            "no \"type\"",
        ),
        (
            serde_json::to_vec(&json!({"type":"LineString","coordinates":[[0,0],[1,1]]}))
                .unwrap(),
            "type LineString is not supported",
        ),
        (
            serde_json::to_vec(&json!({"type":"Point","coordinates":[0,0]})).unwrap(),
            "type Point is not supported",
        ),
        (
            serde_json::to_vec(&json!({"type":"GeometryCollection","geometries":[
                {"type":"Polygon","coordinates":[square]}]}))
            .unwrap(),
            "type GeometryCollection is not supported",
        ),
        (
            serde_json::to_vec(&json!({"type":"Polygon","coordinates":[[[0,0],[1,0],[1,1],[0,1]]]}))
                .unwrap(),
            "must be closed",
        ),
        (
            serde_json::to_vec(&json!({"type":"Polygon","coordinates":[[[0,0],[1,0],[0,0]]]}))
                .unwrap(),
            "at least four positions",
        ),
        (
            serde_json::to_vec(&json!({"type":"Polygon","coordinates":[[[0,0],[1,"a"],[1,1],[0,0]]]}))
                .unwrap(),
            "finite numbers",
        ),
        (
            serde_json::to_vec(&json!({"type":"Polygon","coordinates":[[[0],[1,0],[1,1],[0]]]}))
                .unwrap(),
            "two or three numbers",
        ),
        (
            serde_json::to_vec(&json!({"type":"Polygon","coordinates":[]})).unwrap(),
            "no ring",
        ),
        (
            serde_json::to_vec(&json!({"type":"MultiPolygon","coordinates":[]})).unwrap(),
            "MultiPolygon holds no polygon",
        ),
        (
            serde_json::to_vec(&json!({"type":"Polygon","coordinates":[square],
                "crs":{"type":"name","properties":{"name":"EPSG:3857"}}}))
            .unwrap(),
            "\"crs\" members are not supported",
        ),
        (
            serde_json::to_vec(&json!({"type":"Feature","geometry":null,"properties":{}}))
                .unwrap(),
            "Feature has no geometry",
        ),
        (
            serde_json::to_vec(&json!({"type":"Feature","properties":{},
                "geometry":{"type":"Feature","geometry":{"type":"Polygon","coordinates":[square]}}}))
            .unwrap(),
            "type Feature is not supported",
        ),
        (
            serde_json::to_vec(&json!({"type":"FeatureCollection","features":[
                {"type":"Polygon","coordinates":[square]}]}))
            .unwrap(),
            "members must be Features",
        ),
        (
            serde_json::to_vec(&json!({"type":"FeatureCollection","features":[]})).unwrap(),
            "holds no polygon",
        ),
    ];
    for (bytes, message) in cases {
        let error = MeshSubset::polygon_geojson(&bytes).unwrap_err();
        assert_eq!(error.kind(), JobErrorKind::InvalidRequest, "{error}");
        assert!(error.to_string().contains(message), "{message}: {error}");
    }
    // A Feature and a FeatureCollection of Polygon and MultiPolygon Features are accepted.
    geojson(json!({"type":"FeatureCollection","features":[
        {"type":"Feature","properties":null,"geometry":{"type":"Polygon","coordinates":[square]}},
        {"type":"Feature","properties":null,"geometry":{"type":"MultiPolygon",
            "coordinates":[[rectangle([2., 2., 3., 3.])]]}}]}))
    .unwrap();
}

#[test]
fn ecef_lon_lat_inverts_geodetic_positions() {
    use crate::georef::{geodetic_to_ecef, Cartographic};
    for (lon, lat, height) in [
        (153.02, -27.47, 25.),
        (-0.001, 51.48, -40.),
        (179.999, 89.9999, 3000.),
        (-179.5, -89.99, 0.),
        (12.5, 0., 8848.),
    ] {
        let ecef = geodetic_to_ecef(Cartographic {
            lon_deg: lon,
            lat_deg: lat,
            height_m: height,
        });
        let [got_lon, got_lat] = super::super::source_crs::ecef_lon_lat(ecef);
        assert!((got_lon - lon).abs() < 1e-11, "{lon}: {got_lon}");
        assert!((got_lat - lat).abs() < 1e-11, "{lat}: {got_lat}");
    }
}

/// The records a capture retains, encoded, and its assembly report.
fn capture_rows(
    input: &Path,
    parent: &Path,
    jobs: usize,
    subset: Option<&MeshSubset>,
    plan: Option<super::super::assembly::AssemblyPlan>,
) -> (
    Vec<Vec<u8>>,
    super::super::assembly::AssemblyReport,
    Option<MeshSubsetReport>,
) {
    fs::create_dir_all(parent).unwrap();
    let mut config = full_tests::config(parent);
    config.sort.buffer_bytes = 64;
    config.jobs = Jobs::Fixed(NonZeroUsize::new(jobs).unwrap());
    config.test_assembly = plan;
    config.subset = subset.cloned();
    let output = full::capture(input, &config, &mut || Ok(())).unwrap();
    let rows = output
        .triangles
        .reader()
        .unwrap()
        .map(|t| {
            let mut bytes = vec![0; super::super::attributes::FullTriangle::SIZE];
            t.unwrap().encode(&mut bytes);
            bytes
        })
        .collect();
    let report = output.report.assembly.clone();
    let subset = output.report.subset.clone();
    output.close().unwrap();
    assert_eq!(fs::read_dir(parent).unwrap().count(), 0);
    (rows, report, subset)
}

/// A subset capture holds exactly the full capture's records whose centroid lies in the
/// region, byte for byte and in the same order (so keyed by the same root cube), for
/// vertices held in memory, spilled behind block caches and routed through the corner
/// sorts, on one and three workers. A primitive with no kept triangle is not decoded.
#[test]
fn subset_capture_is_the_full_capture_filtered() {
    use super::super::attributes::FullTriangle;
    let dir = tempfile::tempdir().unwrap();
    // x in [0, 40], [100, 130] and [200, 220]; z in [0, 0.5].
    let input = full_tests::indexed_grids(
        &dir.path().join("source"),
        &[(40, true, true), (30, false, false), (20, true, false)],
    );
    let (all, report, none) = capture_rows(&input, &dir.path().join("full"), 1, None, None);
    assert!(none.is_none());
    assert_eq!(report.excluded_primitives, 0);
    let subset = MeshSubset::bbox(&[20., -1., 110.5, 1.]).unwrap();
    let expected: Vec<Vec<u8>> = all
        .iter()
        .filter(|bytes| {
            let p = FullTriangle::decode(bytes).triangle.positions;
            let centroid = |axis: usize| {
                (f64::from(p[0][axis]) + f64::from(p[1][axis]) + f64::from(p[2][axis])) / 3.
            };
            subset.contains([centroid(0), -centroid(2)])
        })
        .cloned()
        .collect();
    assert!(expected.len() > 1000 && expected.len() < all.len() / 2);
    for jobs in [1, 3] {
        for (plan, spilled, routed) in [
            (None, 0, false),
            (Some(full_tests::spilled(1 << 20)), 2, false),
            (Some(full_tests::spilled(1)), 2, true),
        ] {
            let parent = dir.path().join(format!("subset-{jobs}-{spilled}-{routed}"));
            let (rows, report, stated) = capture_rows(&input, &parent, jobs, Some(&subset), plan);
            assert_eq!(report.excluded_primitives, 1, "{report:?}");
            assert_eq!(report.spilled_primitives, spilled, "{report:?}");
            assert_eq!(report.routed_primitives > 0, routed, "{report:?}");
            assert!(rows == expected, "{jobs} jobs, {report:?}");
            let stated = stated.unwrap();
            assert_eq!(stated.kept_triangles, expected.len() as u64);
            assert_eq!(stated.source_triangles, all.len() as u64);
        }
    }
}

/// An indexed Y-up grid of `side` × `side` cells of 3 m in the X-Z plane (glTF -Z north),
/// heights varying, and a second instance of it `far` metres east. Every centroid is exact
/// in f64: cell (i, j)'s two triangles have region centroids (3i+1, 3j+1) and (3i+2, 3j+2).
fn plane(parent: &Path, side: u32, far: f64) -> PathBuf {
    fs::create_dir_all(parent).unwrap();
    let row = side + 1;
    let mut bytes = Vec::new();
    for j in 0..row {
        for i in 0..row {
            let height = ((i * 7 + j * 3) % 5) as f32 * 0.25;
            for v in [(3 * i) as f32, height, -((3 * j) as f32)] {
                bytes.extend(v.to_le_bytes());
            }
        }
    }
    let positions = bytes.len();
    for j in 0..side {
        for i in 0..side {
            let (a, b) = (j * row + i, j * row + i + 1);
            let (c, d) = (a + row, b + row);
            for v in [a, c, b, b, c, d] {
                bytes.extend(v.to_le_bytes());
            }
        }
    }
    fs::write(parent.join("plane.bin"), &bytes).unwrap();
    let max = f64::from(3 * side);
    let doc = json!({
        "asset":{"version":"2.0"},"scene":0,"scenes":[{"nodes":[0, 1]}],
        "nodes":[{"mesh":0},{"mesh":0,"translation":[far,0,0]}],
        "meshes":[{"primitives":[{"attributes":{"POSITION":0},"indices":1}]}],
        "buffers":[{"uri":"plane.bin","byteLength":bytes.len()}],
        "bufferViews":[{"buffer":0,"byteLength":positions},
            {"buffer":0,"byteOffset":positions,"byteLength":bytes.len()-positions}],
        "accessors":[{"bufferView":0,"componentType":5126,"count":row*row,"type":"VEC3",
                "min":[0.,0.,-max],"max":[max,1.,0.]},
            {"bufferView":1,"componentType":5125,"count":side*side*6,"type":"SCALAR"}]
    });
    let input = parent.join("plane.gltf");
    fs::write(&input, serde_json::to_vec(&doc).unwrap()).unwrap();
    input
}

/// The plane's region centroids (both instances).
fn centroids(side: u32, far: f64) -> Vec<[f64; 2]> {
    let mut out = Vec::new();
    for shift in [0., far] {
        for j in 0..side {
            for i in 0..side {
                let (i, j) = (f64::from(i), f64::from(j));
                out.push([3. * i + 1. + shift, 3. * j + 1.]);
                out.push([3. * i + 2. + shift, 3. * j + 2.]);
            }
        }
    }
    out
}

fn bake(
    input: &Path,
    work: &Path,
    name: &str,
    configure: impl Fn(TilerRequest) -> TilerRequest,
) -> crate::mesh_large::TilerResult {
    let scratch = work.join(format!("scratch-{name}"));
    fs::create_dir_all(&scratch).unwrap();
    let request = TilerRequest::new(input, work.join(format!("{name}.3tz")))
        .with_jobs(TilerJobs::Maximum(NonZeroUsize::new(2).unwrap()))
        .with_scratch_dir(&scratch)
        .with_test_leaf_triangles(64);
    let result = crate::mesh_to_tileset(configure(request), &RunControl::default()).unwrap();
    assert_eq!(fs::read_dir(&scratch).unwrap().count(), 0);
    result
}

/// Centroids exactly on a bbox's edges follow the half-open rule through a whole bake, and
/// two abutting bboxes keep every triangle of their union exactly once.
#[test]
fn boundary_centroids_follow_the_half_open_rule_through_a_bake() {
    let work = tempfile::tempdir().unwrap();
    let (side, far) = (12, 5000.);
    let input = plane(&work.path().join("src"), side, far);
    let all = centroids(side, far);
    let count = |bbox: [f64; 4]| {
        let region = MeshSubset::bbox(&bbox).unwrap();
        all.iter().filter(|c| region.contains(**c)).count() as u64
    };
    // Every edge of these boxes passes through centroids.
    let (left, right, union) = ([1., 1., 13., 22.], [13., 1., 31., 22.], [1., 1., 31., 22.]);
    let mut kept = Vec::new();
    for (name, bbox) in [("left", left), ("right", right), ("union", union)] {
        let result = bake(&input, work.path(), name, |r| {
            r.with_subset(MeshSubset::bbox(&bbox).unwrap())
        });
        let subset = result.report.subset.clone().unwrap();
        assert_eq!(subset.kept_triangles, count(bbox), "{name}");
        assert_eq!(result.report.triangles, subset.kept_triangles);
        assert_eq!(subset.source_triangles, all.len() as u64);
        assert_eq!((subset.region, subset.coordinates), ("bbox", "local"));
        assert_eq!(subset.axes, ["x", "-z"]);
        // The kept corners reach one cell past the centroids, never further.
        let [x0, y0, x1, y1] = subset.extent;
        assert!(
            x0 >= bbox[0] - 3. && y0 >= bbox[1] - 3. && x1 <= bbox[2] + 3. && y1 <= bbox[3] + 3.
        );
        kept.push(subset.kept_triangles);
    }
    // Centroid x = 13 lies on the shared edge: in `right`, not `left`.
    assert_eq!(kept[0] + kept[1], kept[2]);
    assert_eq!(count([13., 1., 13.5, 22.]), 7);
}

#[test]
fn an_empty_selection_is_a_clear_request_error_and_publishes_nothing() {
    let work = tempfile::tempdir().unwrap();
    let input = plane(&work.path().join("src"), 4, 100.);
    let scratch = work.path().join("scratch");
    fs::create_dir(&scratch).unwrap();
    let output = work.path().join("empty.3tz");
    let failure = crate::mesh_to_tileset(
        TilerRequest::new(&input, &output)
            .with_scratch_dir(&scratch)
            .with_subset(MeshSubset::bbox(&[1000., 1000., 1001., 1001.]).unwrap()),
        &RunControl::default(),
    )
    .unwrap_err();
    assert_eq!(failure.error.kind(), JobErrorKind::InvalidRequest);
    let message = failure.error.to_string();
    assert!(message.contains("keeps no source triangle"), "{message}");
    assert!(message.contains("x 1 to 111 and y 1 to 11"), "{message}");
    assert!(!output.exists());
    assert_eq!(fs::read_dir(&scratch).unwrap().count(), 0);
    assert_eq!(
        fs::read_dir(work.path())
            .unwrap()
            .filter(|e| e
                .as_ref()
                .unwrap()
                .file_name()
                .to_string_lossy()
                .starts_with(".tiles"))
            .count(),
        0
    );
}

#[test]
fn a_polygon_hole_excludes_its_centroids() {
    let work = tempfile::tempdir().unwrap();
    let (side, far) = (12, 5000.);
    let input = plane(&work.path().join("src"), side, far);
    let file = work.path().join("holed.geojson");
    let outer = [0.5, 0.5, 30.5, 30.5];
    let hole = [7., 7., 19., 19.];
    fs::write(
        &file,
        serde_json::to_vec(&json!({"type":"Feature","properties":{},"geometry":{
            "type":"Polygon","coordinates":[rectangle(outer), reversed(hole)]}}))
        .unwrap(),
    )
    .unwrap();
    let inside =
        |[x, y]: [f64; 2], [x0, y0, x1, y1]: [f64; 4]| x0 <= x && x < x1 && y0 <= y && y < y1;
    let all = centroids(side, far);
    let expected = all
        .iter()
        .filter(|c| inside(**c, outer) && !inside(**c, hole))
        .count() as u64;
    let in_hole = all.iter().filter(|c| inside(**c, hole)).count();
    assert!(in_hole > 20);
    let result = bake(&input, work.path(), "holed", |r| {
        r.with_subset(MeshSubset::polygon_file(&file).unwrap())
    });
    let subset = result.report.subset.unwrap();
    assert_eq!(subset.kept_triangles, expected);
    assert_eq!(
        (subset.region, subset.polygons, subset.rings),
        ("polygon", Some(1), Some(2))
    );
    assert_eq!(subset.bounds, outer);
}

/// The members of a `.3tz`.
fn members(path: &Path) -> BTreeMap<String, Vec<u8>> {
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

/// A subset archive against a full archive of the same source. Tile names and bounding
/// volumes are not compared: names count triangle ranks, and volumes fit their content.
#[derive(Debug, Default, Serialize)]
pub(crate) struct Alignment {
    /// The root transforms (both absent for local output) agree bit for bit.
    pub root_transform_identical: bool,
    pub subset_leaves: u64,
    /// Subset leaves holding exactly the source triangles of a full leaf, and how many of
    /// those have identical member bytes.
    pub shared_leaves: u64,
    pub identical_shared_leaves: u64,
    pub subset_triangles: u64,
    /// Subset leaf triangles found (by source provenance) in a full leaf.
    pub matched_triangles: u64,
    /// Over matched triangles' corners, in the shared tile frame: bit-identical corners and
    /// the largest component difference (metres).
    pub corners: u64,
    pub identical_corners: u64,
    pub max_corner_difference: f64,
}

/// A leaf triangle's source provenance: node, mesh, primitive and triangle.
type Key = [u64; 4];
/// Leaf triangles by key, with their corners in the tile frame.
type Triangles = BTreeMap<Key, [[f64; 3]; 3]>;
/// A leaf's URI.
type Leaf = String;

/// Every leaf triangle of `archive`, keeping only `wanted` triangles when given; and each
/// leaf by the SHA-256 of its sorted triangle keys.
fn leaf_triangles(
    archive: &Path,
    wanted: Option<&BTreeSet<Key>>,
) -> (TileSet, Triangles, BTreeMap<[u8; 32], Leaf>) {
    use sha2::Digest;
    let mut set = TileSet::open(archive).unwrap();
    let leaves: Vec<Leaf> = set
        .tiles()
        .iter()
        .filter(|t| t.role == TileRole::Leaf)
        .map(|t| t.uri.clone())
        .collect();
    let (mut triangles, mut by_content) = (BTreeMap::new(), BTreeMap::new());
    for leaf in leaves {
        let tile = set.arrays(&leaf).unwrap();
        let table = tile
            .property_tables
            .iter()
            .find(|t| t.class == "source_triangle")
            .unwrap();
        let column = |name: &str| -> Vec<u64> {
            let column = table.columns.iter().find(|c| c.name == name).unwrap();
            match column.component_type.as_str() {
                "UINT32" => column
                    .bytes
                    .as_chunks::<4>()
                    .0
                    .iter()
                    .map(|b| u64::from(u32::from_le_bytes(*b)))
                    .collect(),
                "UINT64" => column
                    .bytes
                    .as_chunks::<8>()
                    .0
                    .iter()
                    .map(|b| u64::from_le_bytes(*b))
                    .collect(),
                other => panic!("{name}: {other}"),
            }
        };
        let rows: Vec<Key> = {
            let [n, m, p, t] = [
                "node_index",
                "mesh_index",
                "primitive_index",
                "triangle_index",
            ]
            .map(column);
            (0..n.len()).map(|r| [n[r], m[r], p[r], t[r]]).collect()
        };
        let mut keys = Vec::new();
        for primitive in &tile.primitives {
            let ids = primitive
                .feature_ids
                .iter()
                .find(|f| f.label.as_deref() == Some("source_triangle"))
                .unwrap();
            for corners in &primitive.indices {
                let key = rows[ids.values[corners[0] as usize] as usize];
                keys.push(key);
                if wanted.is_none_or(|w| w.contains(&key)) {
                    triangles.insert(key, corners.map(|v| primitive.positions[v as usize]));
                }
            }
        }
        keys.sort_unstable();
        let mut digest = sha2::Sha256::new();
        for key in keys {
            for part in key {
                digest.update(part.to_le_bytes());
            }
        }
        by_content.insert(digest.finalize().into(), leaf);
    }
    (set, triangles, by_content)
}

/// Compares a subset bake with a full bake of the same source; see [`Alignment`].
pub(crate) fn compare(full: &Path, subset: &Path) -> Alignment {
    let (part, part_triangles, part_leaves) = leaf_triangles(subset, None);
    let wanted: BTreeSet<Key> = part_triangles.keys().copied().collect();
    let (whole, full_triangles, full_leaves) = leaf_triangles(full, Some(&wanted));
    let transform = |path: &Path| -> Value {
        let tileset: Value = serde_json::from_slice(&members(path)["tileset.json"]).unwrap();
        tileset["root"]["transform"].clone()
    };
    let mut out = Alignment {
        root_transform_identical: whole.root_transform().map(f64::to_bits)
            == part.root_transform().map(f64::to_bits)
            && transform(full) == transform(subset),
        subset_leaves: part_leaves.len() as u64,
        subset_triangles: part_triangles.len() as u64,
        ..Alignment::default()
    };
    let (full_members, part_members) = (members(full), members(subset));
    for (content, uri) in &part_leaves {
        let Some(full_uri) = full_leaves.get(content) else {
            continue;
        };
        out.shared_leaves += 1;
        out.identical_shared_leaves += u64::from(part_members[uri] == full_members[full_uri]);
    }
    for (key, corners) in &part_triangles {
        let Some(truth) = full_triangles.get(key) else {
            continue;
        };
        out.matched_triangles += 1;
        for (a, b) in corners.iter().zip(truth) {
            out.corners += 1;
            out.identical_corners += u64::from(a.map(f64::to_bits) == b.map(f64::to_bits));
            let difference = (0..3).map(|i| (a[i] - b[i]).abs()).fold(0., f64::max);
            out.max_corner_difference = out.max_corner_difference.max(difference);
        }
    }
    out
}

/// A subset bake and a full bake of the same source share the frame: an identical root
/// transform, and every kept triangle found in a full leaf. Unquantized (`lossless`) corners
/// are bit-identical; quantized (`balanced`) ones agree to within the coarser per-depth step
/// (here the steps coincide, so they are bit-identical too). Some leaves hold the same
/// triangles as a full bake's leaf and are byte-identical.
#[test]
fn subset_tiles_share_the_full_bakes_frame_and_grid() {
    let work = tempfile::tempdir().unwrap();
    let (side, far) = (24, 300.);
    let input = plane(&work.path().join("src"), side, far);
    for (profile, placed) in [
        (crate::mesh_large::TilerProfile::Balanced, true),
        (crate::mesh_large::TilerProfile::Lossless, false),
    ] {
        let configure = |r: TilerRequest| {
            let r = r.with_profile(profile);
            if placed {
                r.with_placement(crate::MeshPlacement::Wgs84 {
                    anchor_degrees_metres: [153.02, -27.47, 25.],
                    orientation_xyzw: [0., 0., 0., 1.],
                    scene_offset_metres: [10., 2., -5.],
                })
            } else {
                r
            }
        };
        let full = bake(&input, work.path(), &format!("full-{placed}"), configure);
        let part = bake(&input, work.path(), &format!("part-{placed}"), |r| {
            configure(r).with_subset(MeshSubset::bbox(&[10., 7., 50., 61.]).unwrap())
        });
        assert_eq!(full.report.root_transform, part.report.root_transform);
        let alignment = compare(
            &work.path().join(format!("full-{placed}.3tz")),
            &work.path().join(format!("part-{placed}.3tz")),
        );
        eprintln!("{profile:?}: {alignment:?}");
        assert!(alignment.root_transform_identical);
        assert_eq!(alignment.subset_triangles, part.report.triangles);
        assert_eq!(alignment.matched_triangles, alignment.subset_triangles);
        if placed {
            assert!(alignment.max_corner_difference < 0.01, "{alignment:?}");
        } else {
            assert_eq!(alignment.identical_corners, alignment.corners);
        }
        assert!(alignment.identical_shared_leaves > 0, "{alignment:?}");
        crate::validate::inspect(crate::validate::ValidationRequest::new(
            work.path().join(format!("part-{placed}.3tz")),
        ))
        .unwrap();
    }
}

/// EPSG:3857 spherical-Mercator inverse, independent of the crate's CRS code.
fn mercator_lon_lat([e, n]: [f64; 2]) -> [f64; 2] {
    const R: f64 = 6_378_137.;
    [
        (e / R).to_degrees(),
        (2. * (n / R).exp().atan() - std::f64::consts::FRAC_PI_2).to_degrees(),
    ]
}

/// A source CRS takes a WGS84 bbox: the kept triangles are those whose source centroid maps
/// inside it (an independent Mercator inverse), and the root transform is the full bake's.
#[test]
fn a_crs_source_takes_a_wgs84_bbox_and_keeps_the_full_frame() {
    let work = tempfile::tempdir().unwrap();
    let (side, far) = (16, 400.);
    let input = plane(&work.path().join("src"), side, far);
    let offset = [17_034_000., -3_160_000., 12.];
    let crs = || {
        MeshSourceCrs::from_parameters(Some("EPSG:3857"), None, Some(&offset[..]), None, None)
            .unwrap()
            .unwrap()
    };
    // Region x/y here are the source E/N: (x, -z) plus the offset.
    let geographic: Vec<[f64; 2]> = centroids(side, far)
        .iter()
        .map(|[x, y]| mercator_lon_lat([x + offset[0], y + offset[1]]))
        .collect();
    let sorted = |axis: usize| {
        let mut values: Vec<f64> = geographic.iter().map(|p| p[axis]).collect();
        values.sort_by(f64::total_cmp);
        values.dedup();
        values
    };
    let (lons, lats) = (sorted(0), sorted(1));
    // Bounds midway between centroid values, so no centroid is within rounding of an edge.
    let between = |v: &[f64], at: usize| (v[at] + v[at + 1]) / 2.;
    let bbox = [
        between(&lons, 5),
        between(&lats, 3),
        between(&lons, 20),
        between(&lats, 12),
    ];
    let region = MeshSubset::bbox(&bbox).unwrap();
    let expected = geographic.iter().filter(|p| region.contains(**p)).count() as u64;
    assert!(expected > 50);
    let full = bake(&input, work.path(), "crs-full", |r| {
        r.with_source_crs(crs())
    });
    let part = bake(&input, work.path(), "crs-part", |r| {
        r.with_source_crs(crs()).with_subset(region.clone())
    });
    let subset = part.report.subset.clone().unwrap();
    assert_eq!(subset.kept_triangles, expected);
    assert_eq!(
        (subset.coordinates, subset.axes),
        ("wgs84", ["longitude", "latitude"])
    );
    let [x0, y0, x1, y1] = subset.extent;
    assert!(
        x0 <= bbox[0] && y0 <= bbox[1] && x1 >= bbox[2] && y1 >= bbox[3],
        "{subset:?}"
    );
    assert_eq!(
        full.report.root_transform.unwrap().map(f64::to_bits),
        part.report.root_transform.unwrap().map(f64::to_bits)
    );
    assert_eq!(full.report.source_crs, part.report.source_crs);
    let alignment = compare(
        &work.path().join("crs-full.3tz"),
        &work.path().join("crs-part.3tz"),
    );
    eprintln!("crs: {alignment:?}");
    assert!(alignment.root_transform_identical);
    assert_eq!(alignment.matched_triangles, alignment.subset_triangles);
    assert!(alignment.max_corner_difference < 0.01, "{alignment:?}");
    // A geographic region outside [-180, 180] x [-90, 90] is refused before the source is read.
    let failure = crate::mesh_to_tileset(
        TilerRequest::new(work.path().join("missing.gltf"), work.path().join("x.3tz"))
            .with_source_crs(crs())
            .with_subset(MeshSubset::bbox(&[150., -30., 200., -20.]).unwrap()),
        &RunControl::default(),
    )
    .unwrap_err();
    assert_eq!(failure.error.kind(), JobErrorKind::InvalidRequest);
}

/// The selection, and so the archive, does not depend on the worker count.
#[test]
fn subset_archives_are_identical_across_jobs() {
    let work = tempfile::tempdir().unwrap();
    let input = plane(&work.path().join("src"), 40, 200.);
    let subset = MeshSubset::polygon_geojson(
        &serde_json::to_vec(&json!({"type":"MultiPolygon","coordinates":[
            [[[5., 5.], [80., 9.], [60., 70.], [5., 5.]]],
            [[[150., 100.], [119., 100.], [119., 119.], [150., 100.]]]]}))
        .unwrap(),
    )
    .unwrap();
    let mut archives = Vec::new();
    for jobs in [1, 3, 4] {
        let scratch = work.path().join(format!("scratch-{jobs}"));
        fs::create_dir(&scratch).unwrap();
        let output = work.path().join(format!("jobs-{jobs}.3tz"));
        crate::mesh_to_tileset(
            TilerRequest::new(&input, &output)
                .with_jobs(TilerJobs::Maximum(NonZeroUsize::new(jobs).unwrap()))
                .with_scratch_dir(&scratch)
                .with_test_leaf_triangles(64)
                .with_subset(subset.clone()),
            &RunControl::default(),
        )
        .unwrap();
        archives.push(fs::read(&output).unwrap());
    }
    assert!(archives.windows(2).all(|w| w[0] == w[1]));
}

/// Compares two archives named by `RT_SUBSET_FULL` and `RT_SUBSET_PART` and prints the
/// [`Alignment`] as JSON (evidence on private sources).
#[test]
#[ignore = "driver: set RT_SUBSET_FULL and RT_SUBSET_PART"]
fn subset_alignment_driver() {
    let full = PathBuf::from(std::env::var_os("RT_SUBSET_FULL").unwrap());
    let part = PathBuf::from(std::env::var_os("RT_SUBSET_PART").unwrap());
    println!(
        "{}",
        serde_json::to_string_pretty(&compare(&full, &part)).unwrap()
    );
}
