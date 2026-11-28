//! `mesh-inspect` on small analytic fixtures: exact topology counts, the default inspection
//! against the capture-based one (`--forecast`) field for field, determinism across `--jobs`
//! and partitions, the forecast's certain sources against a real bake's audit, request
//! refusals and cancellation.
use super::{forecast::Risk, inspect_mesh, InspectRequest, InspectResult};
use crate::{
    mesh_large::{hlod, TilerJobs, TilerRequest},
    JobErrorKind, MeshPlacement, Observer, OutputPolicy, RunControl, RunEvent,
};
use serde_json::{json, Value};
use std::{
    collections::BTreeSet,
    fs,
    num::NonZeroUsize,
    path::{Path, PathBuf},
    sync::{Arc, OnceLock},
};

struct Mesh {
    positions: Vec<[f32; 3]>,
    indices: Vec<u32>,
}

/// One node, mesh and material per primitive (so primitives never share vertices).
fn write_gltf(dir: &Path, meshes: &[Mesh]) -> PathBuf {
    fs::create_dir_all(dir).unwrap();
    let mut bin = Vec::new();
    let (mut views, mut accessors, mut gltf_meshes, mut nodes, mut materials) =
        (vec![], vec![], vec![], vec![], vec![]);
    for (i, m) in meshes.iter().enumerate() {
        let start = bin.len();
        let mut lo = [f32::INFINITY; 3];
        let mut hi = [f32::NEG_INFINITY; 3];
        for p in &m.positions {
            for a in 0..3 {
                bin.extend(p[a].to_le_bytes());
                lo[a] = lo[a].min(p[a]);
                hi[a] = hi[a].max(p[a]);
            }
        }
        views.push(json!({"buffer":0,"byteOffset":start,"byteLength":bin.len()-start}));
        accessors.push(json!({"bufferView":views.len()-1,"componentType":5126,"count":m.positions.len(),"type":"VEC3","min":lo,"max":hi}));
        let start = bin.len();
        for v in &m.indices {
            bin.extend(v.to_le_bytes());
        }
        views.push(json!({"buffer":0,"byteOffset":start,"byteLength":bin.len()-start}));
        accessors.push(json!({"bufferView":views.len()-1,"componentType":5125,"count":m.indices.len(),"type":"SCALAR"}));
        gltf_meshes.push(json!({"primitives":[{"attributes":{"POSITION":accessors.len()-2},"indices":accessors.len()-1,"material":i}]}));
        nodes.push(json!({"mesh":i}));
        materials.push(json!({"pbrMetallicRoughness":{"baseColorFactor":[0.5,0.5,0.5,1.0],"metallicFactor":0.0}}));
    }
    fs::write(dir.join("scene.bin"), &bin).unwrap();
    let doc = json!({"asset":{"version":"2.0"},"scene":0,"scenes":[{"nodes":(0..meshes.len()).collect::<Vec<_>>()}],
        "nodes":nodes,"meshes":gltf_meshes,"materials":materials,
        "buffers":[{"uri":"scene.bin","byteLength":bin.len()}],"bufferViews":views,"accessors":accessors});
    let path = dir.join("scene.gltf");
    fs::write(&path, serde_json::to_vec(&doc).unwrap()).unwrap();
    path
}

/// `nx × nz` unit quads from `(x0, z0)` at height `y(x, z)`, skipping the quads `hole` keeps.
fn grid(
    (nx, nz): (usize, usize),
    (x0, z0): (f32, f32),
    y: &dyn Fn(f32, f32) -> f32,
    hole: &dyn Fn(usize, usize) -> bool,
) -> Mesh {
    let mut m = Mesh {
        positions: vec![],
        indices: vec![],
    };
    for i in 0..=nx {
        for j in 0..=nz {
            let (x, z) = (x0 + i as f32, z0 + j as f32);
            m.positions.push([x, y(x, z), z]);
        }
    }
    let w = nz + 1;
    for i in 0..nx {
        for j in 0..nz {
            if hole(i, j) {
                continue;
            }
            let v = |a: usize, c: usize| (a * w + c) as u32;
            m.indices.extend([v(i, j), v(i, j + 1), v(i + 1, j)]);
            m.indices
                .extend([v(i + 1, j), v(i, j + 1), v(i + 1, j + 1)]);
        }
    }
    m
}

fn flat(_: f32, _: f32) -> f32 {
    0.
}
fn solid(_: usize, _: usize) -> bool {
    false
}

fn sort() -> crate::spill::SortConfig {
    crate::spill::SortConfig {
        run_bytes: 1 << 16,
        fan_in: 4,
        buffer_bytes: 1 << 12,
        jobs: 1,
        writers: 0,
    }
}

/// The test seams the HLOD tests use: 256-triangle leaves, small spilling sorts.
fn request(input: &Path, settings: hlod::Settings, jobs: usize) -> InspectRequest {
    InspectRequest::new(input)
        .with_jobs(TilerJobs::Maximum(NonZeroUsize::new(jobs).unwrap()))
        .with_hlod_settings(settings)
        .with_test_leaf_triangles(256)
        .with_test_spill(sort(), 1 << 14)
}

fn inspect(request: InspectRequest) -> InspectResult {
    inspect_mesh(request, &RunControl::default()).unwrap_or_else(|f| panic!("{}", f.error))
}

/// The topology and weld sections, serialized: what both inspections must agree on, byte
/// for byte.
fn topology_bytes(result: &InspectResult) -> String {
    serde_json::to_string(&(&result.report.source, &result.report.weld)).unwrap()
}

/// The default inspection gives the capture-based topology, at one and several partitions,
/// with corners from position tables or read in triangle order.
fn assert_same_topology(request: impl Fn() -> InspectRequest) -> InspectResult {
    let captured = inspect(request().with_forecast(true));
    for (parts, tables) in [(1, None), (7, None), (5, Some(0))] {
        let mut r = request().with_test_partitions(parts);
        if let Some(bytes) = tables {
            r = r.with_test_tables(bytes);
        }
        let fast = inspect(r);
        assert!(fast.report.forecast.is_none() && fast.report.tree.is_none());
        assert_eq!(
            topology_bytes(&fast),
            topology_bytes(&captured),
            "{parts} partitions, tables {tables:?}"
        );
    }
    captured
}

/// Two 2×2 sheets meeting along an unwelded seam (separate primitives), a lone triangle,
/// and a quad whose diagonal is unwelded inside its primitive: every count is known.
#[test]
fn topology_counts_are_exact_on_an_analytic_fixture() {
    let work = tempfile::tempdir().unwrap();
    let quad = Mesh {
        positions: vec![
            [20., 0., 0.],
            [21., 0., 0.],
            [20., 0., 1.],
            [21., 0., 1.],
            [21., 0., 0.],
            [20., 0., 1.],
        ],
        indices: vec![0, 2, 1, 4, 5, 3],
    };
    let lone = Mesh {
        positions: vec![[10., 0., 0.], [11., 0., 0.], [10., 0., 1.]],
        indices: vec![0, 2, 1],
    };
    let input = write_gltf(
        &work.path().join("src"),
        &[
            grid((2, 2), (0., 0.), &flat, &solid),
            grid((2, 2), (2., 0.), &flat, &solid),
            lone,
            quad,
        ],
    );
    let captured = assert_same_topology(|| request(&input, hlod::Settings::production(), 1));
    let result = inspect(request(&input, hlod::Settings::production(), 1));
    let s = serde_json::to_value(&result.report.source).unwrap();
    let r2 = 2f64.sqrt();
    let close = |v: &Value, want: f64| {
        let got = v.as_f64().unwrap();
        assert!((got - want).abs() < 1e-9, "{got} != {want}");
    };
    assert_eq!(s["triangles"], 19);
    assert_eq!(s["degenerate_triangles"], 0);
    assert_eq!(s["positions"], 15 + 3 + 4);
    assert_eq!(s["source_vertices"], 9 + 9 + 3 + 6);
    assert_eq!(s["primitive_instances"], 4);
    assert_eq!(s["components"], 3);
    assert_eq!(s["nonmanifold_edges"], 0);
    close(&s["surface_area_m2"], 4. + 4. + 0.5 + 1.);
    let b = &s["primitive_boundaries"];
    assert_eq!(b["edges"]["edges"], 8 + 8 + 3 + 6);
    assert_eq!(b["coincide_with_other_primitive"]["edges"], 4);
    close(&b["coincide_with_other_primitive"]["length_m"], 4.);
    assert_eq!(b["unwelded_within_primitive"]["edges"], 2);
    close(&b["unwelded_within_primitive"]["length_m"], 2. * r2);
    assert_eq!(b["open"]["edges"], 12 + 3 + 4);
    let rp = &s["repeated_positions"];
    assert_eq!(rp["positions"], 5);
    assert_eq!(rp["extra_vertices"], 5);
    assert_eq!(rp["closes_open_edges"]["edges"], 3);
    close(&rp["closes_open_edges"]["length_m"], 2. + r2);
    let open = &s["open"];
    assert_eq!(open["edges"], 19);
    assert_eq!(open["loops"], 3);
    close(&open["length_m"], 12. + 2. + r2 + 4.);
    let loops: Vec<u64> = open["by_size_class"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| c["loops"].as_u64().unwrap())
        .collect();
    assert_eq!(loops, [0, 2, 1, 0], "{open}");
    let seams = &s["seams"];
    assert_eq!(seams["edges"], 3);
    assert_eq!(seams["loops"], 2);
    close(&seams["length_m"], 2. + r2);
    // One leaf: no coarse cell, nothing to forecast.
    let r = &captured.report;
    assert_eq!(r.tree.as_ref().unwrap().coarse_cells, 0);
    assert_eq!(r.forecast.as_ref().unwrap().root, "built");
    assert_eq!(r.hlod_view.as_ref().unwrap().open_edges, 12 + 3 + 4);
}

/// With a weld (and between primitives of one material), the default inspection reports
/// the welded source and the weld's statement as the capture-based one does.
#[test]
fn a_weld_preview_is_the_capture_based_one() {
    let work = tempfile::tempdir().unwrap();
    // Two sheets 1 mm apart along their shared column, and a sliver that collapses.
    let mut meshes = holed_sheet();
    for p in &mut meshes[1].positions {
        p[0] += 0.001;
    }
    meshes.push(Mesh {
        positions: vec![[0., 0., 0.], [0.0005, 0., 0.], [0., 0.0005, 0.]],
        indices: vec![0, 1, 2],
    });
    let input = write_gltf(&work.path().join("src"), &meshes);
    for weld in [
        crate::MeshWeld::new(0.002).unwrap(),
        crate::MeshWeld::new(0.002).unwrap().joining_same_material(),
    ] {
        let captured = assert_same_topology(|| {
            request(&input, hlod::Settings::production(), 2).with_weld(weld)
        });
        let w = captured.report.weld.as_ref().unwrap();
        assert!(w.merged_vertices > 0, "{w:?}");
    }
}

/// A failing source gets capture's verdict (`--forecast` captures) at every job count: the
/// first failing triangle in input order, not the first failure in time. Two invalid
/// positions lie in different scan and decode chunks, in both orders; a bad index in the
/// second primitive comes after a bad position in the first.
#[test]
fn the_first_failure_is_captures_at_any_job_count() {
    let work = tempfile::tempdir().unwrap();
    // 301 × 301 vertices: more than one 2^16-vertex chunk and 2^16-triangle task.
    let sheet = || grid((300, 300), (0., 0.), &flat, &solid);
    let failure = |input: &Path, jobs: usize, forecast: bool| -> String {
        let r = request(input, hlod::Settings::production(), jobs).with_forecast(forecast);
        inspect_mesh(r, &RunControl::default())
            .map(|_| ())
            .unwrap_err()
            .error
            .to_string()
    };
    let mut verdicts = Vec::new();
    for (name, early, late) in [
        ("nan-first", f32::NAN, 2.0e6),
        ("far-first", 2.0e6, f32::NAN),
    ] {
        let mut m = sheet();
        // The end of the first chunk and the start of the second: a parallel pass meets
        // the later one first.
        m.positions[65_000][0] = early;
        m.positions[65_600][0] = late;
        let input = write_gltf(&work.path().join(name), &[m]);
        let captured = failure(&input, 1, true);
        for jobs in [1, 4] {
            assert_eq!(
                failure(&input, jobs, false),
                captured,
                "{name}, {jobs} jobs"
            );
        }
        verdicts.push(captured);
    }
    assert_ne!(verdicts[0], verdicts[1], "the order decides: {verdicts:?}");
    let mut first = sheet();
    first.positions[50_000][1] = f32::NAN;
    let mut second = grid((4, 4), (400., 0.), &flat, &solid);
    second.indices[5] = 10_000;
    let input = write_gltf(&work.path().join("index"), &[first, second]);
    let captured = failure(&input, 1, true);
    assert!(captured.contains("nonfinite"), "{captured}");
    for jobs in [1, 4] {
        assert_eq!(failure(&input, jobs, false), captured, "{jobs} jobs");
    }
}

/// A position the scan admits but the source CRS cannot map fails while streaming; that
/// failure is rerun serially for capture's verdict (`--forecast`), at every job count. On a
/// small longitude/latitude patch at 10 E, two vertices, at the end of the first 2^16-vertex
/// chunk and the start of the second, fail differently: one at 190 E is outside the
/// longitude range, one at 170 W maps outside the local domain (the bounds' centre stays
/// on the patch). The first in vertex order decides, in both orders.
#[test]
fn a_source_crs_failure_while_streaming_is_captures() {
    let work = tempfile::tempdir().unwrap();
    let crs = || {
        crate::MeshSourceCrs::from_parameters(
            Some("EPSG:4326"),
            Some(crate::SourceAxes::Xyz),
            None,
            None,
            None,
        )
        .unwrap()
        .unwrap()
    };
    let mut verdicts = Vec::new();
    for (name, early, late) in [("range-first", 190., -170.), ("far-first", -170., 190.)] {
        let mut m = grid((300, 300), (0., 0.), &flat, &solid);
        // East = x, north = y, up = z: a 0.003-degree patch at (10 E, 45 N), height 0.
        for p in &mut m.positions {
            *p = [10. + p[0] * 1e-5, 45. + p[2] * 1e-5, 0.];
        }
        m.positions[65_000][0] = early;
        m.positions[65_600][0] = late;
        let input = write_gltf(&work.path().join(name), &[m]);
        let failure = |jobs: usize, forecast: bool| -> String {
            let r = request(&input, hlod::Settings::production(), jobs)
                .with_source_crs(crs())
                .with_forecast(forecast);
            inspect_mesh(r, &RunControl::default())
                .map(|_| ())
                .unwrap_err()
                .error
                .to_string()
        };
        let captured = failure(1, true);
        for jobs in [1, 4] {
            assert_eq!(failure(jobs, false), captured, "{name}, {jobs} jobs");
        }
        verdicts.push(captured);
    }
    assert_ne!(verdicts[0], verdicts[1], "the order decides: {verdicts:?}");
}

/// A holed, two-primitive sheet large enough for coarse cells.
fn holed_sheet() -> Vec<Mesh> {
    let hole = |i: usize, j: usize| (10..14).contains(&i) && (20..23).contains(&j);
    let bump = |x: f32, z: f32| {
        let (u, v) = ((x - 30.) / 20., (z - 30.) / 20.);
        3. / (1. + u * u + v * v)
    };
    vec![
        grid((30, 60), (0., 0.), &bump, &hole),
        grid((30, 60), (30., 0.), &bump, &|i, j| {
            (5..9).contains(&i) && (40..44).contains(&j)
        }),
    ]
}

/// The report and the GeoJSON are the same bytes at 1 and 3 jobs; the GeoJSON is a
/// FeatureCollection whose features match the report.
#[test]
fn report_and_geojson_do_not_depend_on_jobs() {
    let work = tempfile::tempdir().unwrap();
    let input = write_gltf(&work.path().join("src"), &holed_sheet());
    let run = |jobs: usize, name: &str, forecast: bool| {
        let path = work.path().join(name);
        let result = inspect(
            request(&input, hlod::Settings::production(), jobs)
                .with_test_partitions(if forecast { 1 } else { 2 * jobs as u64 })
                .with_forecast(forecast)
                .with_geojson(&path)
                .with_geojson_local(true)
                .with_min_feature_length(1.),
        );
        // The result names the resolved output path (on Windows a verbatim `\\?\` path).
        let resolved = crate::output_path::resolve(&path).unwrap();
        assert_eq!(result.geojson.as_deref(), Some(resolved.as_path()));
        let report = serde_json::to_value(&result.report).unwrap();
        (report, fs::read(path).unwrap(), result)
    };
    let (one, one_geo, fast) = run(1, "one.geojson", false);
    let (three, three_geo, _) = run(3, "three.geojson", false);
    assert_eq!(one, three);
    assert!(one_geo == three_geo, "GeoJSON differs between job counts");
    let (with_cells, cells_geo, result) = run(1, "cells.geojson", true);
    assert_eq!(topology_bytes(&fast), topology_bytes(&result));
    assert_eq!(
        one["geojson"]["open_loops"],
        with_cells["geojson"]["open_loops"]
    );
    assert_eq!(one["geojson"]["seams"], with_cells["geojson"]["seams"]);
    assert_eq!(fast.report.geojson.as_ref().unwrap().cells, 0);
    // Without the forecast, the GeoJSON is the forecast's without its cells.
    let lines = |bytes: &[u8]| -> Vec<Value> {
        let doc: Value = serde_json::from_slice(bytes).unwrap();
        doc["features"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|f| f["properties"]["kind"] != "cell")
            .cloned()
            .collect()
    };
    assert_eq!(lines(&one_geo), lines(&cells_geo));
    let one_geo = cells_geo;
    let report = &result.report;
    let tree = report.tree.as_ref().unwrap();
    assert!(tree.coarse_cells > 3, "{tree:?}");
    assert_eq!(report.source.components, 1);
    // Two holes and the sheet's outer border.
    assert_eq!(report.source.open.loops, 3);
    let g = report.geojson.as_ref().unwrap();
    assert_eq!(g.coordinates, "local_east_north_metres");
    let doc: Value = serde_json::from_slice(&one_geo).unwrap();
    assert_eq!(doc["type"], "FeatureCollection");
    // Local metres, said plainly: RFC 7946 coordinates would be WGS84.
    assert_eq!(doc["rustyTilesFrame"]["wgs84"], false);
    assert_eq!(doc["rustyTilesFrame"]["y"], "north (glTF -Z)");
    let features = doc["features"].as_array().unwrap();
    let kind = |k: &str| {
        features
            .iter()
            .filter(|f| f["properties"]["kind"] == k)
            .count() as u64
    };
    assert_eq!(kind("open_loop"), g.open_loops);
    assert_eq!(kind("seam"), g.seams);
    assert_eq!(kind("cell"), g.cells);
    assert_eq!(g.open_loops, 3);
    assert_eq!(
        g.seams, 1,
        "the primitives' shared column is one seam chain"
    );
    for f in features {
        assert_eq!(f["type"], "Feature");
        let geometry = &f["geometry"];
        let lines = geometry["coordinates"].as_array().unwrap();
        // Every loop is one closed ring; the seam column is one open polyline.
        match f["properties"]["kind"].as_str().unwrap() {
            "open_loop" => {
                assert_eq!(lines.len(), 1, "{f}");
                let ring = lines[0].as_array().unwrap();
                assert_eq!(ring.first(), ring.last());
                assert_eq!(
                    ring.len() as u64,
                    f["properties"]["edges"].as_u64().unwrap() + 1
                );
            }
            "seam" => {
                assert_eq!(lines.len(), 1, "{f}");
                assert_eq!(lines[0].as_array().unwrap().len(), 61);
            }
            _ => {}
        }
        match geometry["type"].as_str().unwrap() {
            "MultiLineString" => assert!(!geometry["coordinates"].as_array().unwrap().is_empty()),
            "Polygon" => assert_eq!(geometry["coordinates"][0].as_array().unwrap().len(), 5),
            other => panic!("{other}"),
        }
    }
    // Every coarse cell is forecast once, by depth.
    let cells: u64 = report
        .forecast
        .as_ref()
        .unwrap()
        .by_depth
        .iter()
        .map(|d| d.cells)
        .sum();
    assert_eq!(cells, tree.coarse_cells);
}

/// Omitted cells in a bake's audit sidecar: `(first, depth) -> reason`.
fn audited_omissions(dir: &Path) -> std::collections::BTreeMap<(u64, u32), String> {
    fs::read_to_string(dir.join("borders.jsonl"))
        .unwrap()
        .lines()
        .map(|l| serde_json::from_str::<Value>(l).unwrap())
        .filter(|v| v["kind"] == "omitted")
        .map(|v| {
            (
                (
                    v["key"]["first"].as_u64().unwrap(),
                    v["key"]["depth"].as_u64().unwrap() as u32,
                ),
                v["reason"].as_str().unwrap().to_owned(),
            )
        })
        .collect()
}

/// With a 12-triangle cap, cells whose border contract alone needs more triangles are
/// omitted by the bake without a build. The forecast marks exactly those cells `certain`,
/// and the bake's omitted cells by depth agree with the forecast's propagation of them.
#[test]
fn certain_sources_are_the_bakes_contract_lower_bound_omissions() {
    let work = tempfile::tempdir().unwrap();
    let input = write_gltf(&work.path().join("src"), &holed_sheet());
    let settings = || hlod::Settings {
        mutation: hlod::Mutation::NoReliefNoRecovery,
        ..hlod::test_settings(12)
    };
    let audit = work.path().join("audit");
    let bake = TilerRequest::new(&input, work.path().join("bake.3tz"))
        .with_jobs(TilerJobs::Maximum(NonZeroUsize::new(2).unwrap()))
        .with_test_leaf_triangles(256)
        .with_hlod_settings(hlod::Settings {
            audit_dir: Some(Arc::new(audit.clone())),
            ..settings()
        })
        .with_test_spill(sort(), 1 << 14);
    let baked = crate::mesh_large::mesh_to_tileset(bake, &RunControl::default())
        .unwrap_or_else(|f| panic!("{}", f.error));
    let omitted = audited_omissions(&audit);
    let lower_bound: BTreeSet<(u64, u32)> = omitted
        .iter()
        .filter(|(_, r)| *r == "contract_lower_bound")
        .map(|(k, _)| *k)
        .collect();
    assert!(
        !lower_bound.is_empty(),
        "the fixture must exercise the lower bound"
    );
    let geojson = work.path().join("forecast.geojson");
    let result = inspect(
        request(&input, settings(), 2)
            .with_forecast(true)
            .with_geojson(&geojson)
            .with_geojson_local(true),
    );
    let forecast = result.report.forecast.as_ref().unwrap();
    let certain: BTreeSet<(u64, u32)> = forecast
        .top_cells
        .iter()
        .filter(|c| c.0.risk == Risk::Certain && c.0.outcome == super::Outcome::Source)
        .map(|c| (c.0.first, c.0.depth))
        .collect();
    assert_eq!(certain.len() as u64, forecast.certain_sources);
    assert_eq!(certain, lower_bound);
    assert_eq!(
        forecast.root == "omitted",
        baked.diagnostics.hlod.omitted_by_depth[0] == 1
    );
    // Here every source is certain, so the replayed propagation gives the bake's omitted
    // cells exactly: by depth, and cell for cell (every omitted cell is a GeoJSON feature).
    let predicted: Vec<u64> = forecast
        .by_depth
        .iter()
        .map(|d| d.predicted_omitted)
        .collect();
    assert_eq!(predicted, baked.diagnostics.hlod.omitted_by_depth);
    let doc: Value = serde_json::from_slice(&fs::read(&geojson).unwrap()).unwrap();
    let predicted: BTreeSet<(u64, u32)> = doc["features"]
        .as_array()
        .unwrap()
        .iter()
        .map(|f| &f["properties"])
        .filter(|p| {
            p["kind"] == "cell" && matches!(p["outcome"].as_str(), Some("source" | "descendant"))
        })
        .map(|p| {
            (
                p["first"].as_u64().unwrap(),
                p["depth"].as_u64().unwrap() as u32,
            )
        })
        .collect();
    assert_eq!(predicted, omitted.keys().copied().collect::<BTreeSet<_>>());
}

/// Bad requests are refused before the source is read; an existing GeoJSON is a conflict
/// unless replacement is requested.
#[test]
fn requests_are_checked_before_any_work() {
    let work = tempfile::tempdir().unwrap();
    let input = write_gltf(
        &work.path().join("src"),
        &[grid((2, 2), (0., 0.), &flat, &solid)],
    );
    let fail = |r: InspectRequest| inspect_mesh(r, &RunControl::default()).unwrap_err().error;
    assert_eq!(
        fail(InspectRequest::new(&input).with_geojson(work.path().join("x.json"))).kind(),
        JobErrorKind::InvalidRequest
    );
    assert_eq!(
        fail(InspectRequest::new(&input).with_min_feature_length(-1.)).kind(),
        JobErrorKind::InvalidRequest
    );
    assert_eq!(
        fail(InspectRequest::new(&input).with_min_feature_length(f64::NAN)).kind(),
        JobErrorKind::InvalidRequest
    );
    assert_eq!(
        fail(InspectRequest::new(&input).with_memory_budget(0)).kind(),
        JobErrorKind::InvalidRequest
    );
    assert_eq!(
        fail(InspectRequest::new(work.path().join("missing.glb"))).kind(),
        JobErrorKind::Io
    );
    // An unplaced source has no WGS84 coordinates: refused unless local metres are asked for.
    let unplaced = fail(InspectRequest::new(&input).with_geojson(work.path().join("u.geojson")));
    assert_eq!(unplaced.kind(), JobErrorKind::InvalidRequest);
    assert!(
        unplaced.to_string().contains("--geojson-local"),
        "{unplaced}"
    );
    assert!(!work.path().join("u.geojson").exists());
    let existing = work.path().join("old.geojson");
    fs::write(&existing, b"keep").unwrap();
    assert_eq!(
        fail(
            InspectRequest::new(&input)
                .with_geojson(&existing)
                .with_geojson_local(true)
        )
        .kind(),
        JobErrorKind::Conflict
    );
    assert_eq!(fs::read(&existing).unwrap(), b"keep");
    let replaced = inspect(
        InspectRequest::new(&input)
            .with_geojson(&existing)
            .with_geojson_local(true)
            .with_output_policy(OutputPolicy::Replace),
    );
    assert!(replaced.report.geojson.is_some());
    let doc: Value = serde_json::from_slice(&fs::read(&existing).unwrap()).unwrap();
    assert_eq!(doc["type"], "FeatureCollection");
}

/// Cancelled during the topology pass: a typed cancellation, no GeoJSON, scratch removed.
#[test]
fn cancellation_inside_the_topology_pass_leaves_nothing() {
    struct CancelAt(
        OnceLock<crate::CancellationHandle>,
        std::sync::Mutex<Vec<String>>,
    );
    impl Observer for CancelAt {
        fn observe(&self, event: &RunEvent<'_>) -> Result<(), crate::JobError> {
            if let RunEvent::Progress { phase, .. } = event {
                self.1.lock().unwrap().push((*phase).to_owned());
                if *phase == "inspect_edges" {
                    self.0.get().unwrap().cancel();
                }
            }
            Ok(())
        }
    }
    let work = tempfile::tempdir().unwrap();
    let input = write_gltf(&work.path().join("src"), &holed_sheet());
    let scratch = work.path().join("scratch");
    fs::create_dir(&scratch).unwrap();
    let observer = Arc::new(CancelAt(OnceLock::new(), Default::default()));
    let control = RunControl::new(Some(observer.clone()));
    observer.0.set(control.cancellation_handle()).ok().unwrap();
    let geojson = work.path().join("out.geojson");
    let failure = inspect_mesh(
        request(&input, hlod::Settings::production(), 1)
            .with_geojson(&geojson)
            .with_geojson_local(true)
            .with_scratch_dir(&scratch),
        &control,
    )
    .unwrap_err();
    assert_eq!(
        failure.error.kind(),
        JobErrorKind::Cancelled,
        "{}",
        failure.error
    );
    assert!(!geojson.exists());
    assert_eq!(fs::read_dir(&scratch).unwrap().count(), 0);
    let phases = observer.1.lock().unwrap();
    assert!(phases.iter().any(|p| p == "inspect_stream"), "{phases:?}");
    assert!(phases.iter().any(|p| p == "inspect_vertices"), "{phases:?}");
    assert!(
        !phases.iter().any(|p| p.starts_with("capture")),
        "{phases:?}"
    );
    assert!(
        work.path().read_dir().unwrap().all(|e| !e
            .unwrap()
            .file_name()
            .to_string_lossy()
            .starts_with(".tiles-stage-")),
        "the staged GeoJSON is removed"
    );
}

/// A placed source writes WGS84 coordinates: the anchor's origin maps to its longitude and
/// latitude.
#[test]
fn placed_geojson_is_wgs84_and_the_origin_maps_to_the_anchor() {
    let anchor = crate::Cartographic {
        lon_deg: 133.0584,
        lat_deg: -12.3362,
        height_m: 100.,
    };
    let frame = super::Frame::Wgs84(crate::georef::root_transform(anchor, None));
    let [lon, lat] = frame.point([0., 0., 0.]);
    assert!((lon - anchor.lon_deg).abs() < 1e-9 && (lat - anchor.lat_deg).abs() < 1e-9);
    // 100 m north (glTF -Z) is about 0.0009 degrees of latitude.
    let [_, north] = frame.point([0., 0., -100.]);
    assert!(
        (north - lat - 100. / 110_574.).abs() < 2e-6,
        "{}",
        north - lat
    );
    let work = tempfile::tempdir().unwrap();
    let input = write_gltf(&work.path().join("src"), &holed_sheet());
    let path = work.path().join("placed.geojson");
    let result = inspect(
        request(&input, hlod::Settings::production(), 1)
            .with_placement(
                MeshPlacement::from_parameters(Some([133.0584, -12.3362, 100.]), None, None)
                    .unwrap(),
            )
            .with_geojson(&path),
    );
    assert_eq!(result.report.geojson.as_ref().unwrap().coordinates, "wgs84");
    let doc: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    assert!(
        doc.get("rustyTilesFrame").is_none(),
        "RFC 7946 output needs no frame member"
    );
    let first = &doc["features"][0]["geometry"]["coordinates"][0][0];
    let (x, y) = (first[0].as_f64().unwrap(), first[1].as_f64().unwrap());
    assert!(
        (x - 133.0584).abs() < 0.01 && (y + 12.3362).abs() < 0.01,
        "{first}"
    );
}

/// Segments join into ordered polylines: an open chain from its end, a branch split at the
/// branch point, and a ring closed on its smallest point, whatever the segment order.
#[test]
fn polylines_follow_ends_branches_and_rings() {
    let p = |x: u32, y: u32| [x, y, 0];
    // A chain 0-1-2, a branch 1-3, and a ring 10-11-12-10.
    let mut segments = vec![
        (p(1, 0), p(2, 0)),
        (p(10, 0), p(11, 0)),
        (p(0, 0), p(1, 0)),
        (p(12, 0), p(10, 0)),
        (p(1, 0), p(3, 0)),
        (p(11, 0), p(12, 0)),
    ];
    let lines = super::polylines(&segments);
    segments.reverse();
    assert_eq!(super::polylines(&segments).len(), lines.len());
    assert_eq!(
        lines,
        vec![
            vec![p(0, 0), p(1, 0)],
            vec![p(1, 0), p(2, 0)],
            vec![p(1, 0), p(3, 0)],
            vec![p(10, 0), p(11, 0), p(12, 0), p(10, 0)],
        ]
    );
}
