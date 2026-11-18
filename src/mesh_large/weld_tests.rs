//! The weld: its request rules, the sweep against brute force, captures of near and exact
//! seams (positions only, every worker count and vertex store), and the published errors.
use super::*;
use crate::{
    mesh_large::{
        assembly::AssemblyPlan,
        attributes::FullTriangle,
        config::Jobs,
        full::{self, tests as full_tests},
        TilerJobs, TilerRequest,
    },
    tile_view::TileSet,
    RunControl,
};
use serde_json::json;
use std::{
    collections::BTreeMap,
    fs,
    io::Read,
    num::NonZeroUsize,
    path::{Path, PathBuf},
};

#[test]
fn weld_requests_are_checked() {
    assert_eq!(MeshWeld::from_parameters(None, None).unwrap(), None);
    let weld = MeshWeld::from_parameters(Some(0.01), None)
        .unwrap()
        .unwrap();
    assert_eq!((weld.epsilon(), weld.joins_same_material()), (0.01, false));
    // Joining alone welds exact repeats.
    let join = MeshWeld::from_parameters(None, Some("same-material"))
        .unwrap()
        .unwrap();
    assert_eq!((join.epsilon(), join.joins_same_material()), (0., true));
    assert_eq!(
        MeshWeld::new(-0.).unwrap().epsilon().to_bits(),
        0f64.to_bits()
    );
    for bad in [-1e-9, f64::NAN, f64::INFINITY] {
        let error = MeshWeld::from_parameters(Some(bad), None).unwrap_err();
        assert_eq!(error.kind(), JobErrorKind::InvalidRequest, "{bad}");
        assert!(error.to_string().contains("finite"), "{error}");
    }
    let error = MeshWeld::from_parameters(Some(0.), Some("all")).unwrap_err();
    assert_eq!(error.kind(), JobErrorKind::InvalidRequest);
    assert!(error.to_string().contains("same-material"), "{error}");
}

fn next(state: &mut u64) -> u64 {
    *state ^= *state << 13;
    *state ^= *state >> 7;
    *state ^= *state << 17;
    *state
}
fn unit(state: &mut u64) -> f64 {
    (next(state) >> 11) as f64 / (1u64 << 53) as f64
}

/// The sweep's classes are those of every pair within ε in one scope (brute force), for
/// exact repeats at ε = 0, jittered near-repeats, chains, mixed scopes and coordinates on
/// cell boundaries; every vertex has one home record and at most four records.
#[test]
fn the_sweep_finds_the_brute_force_classes() {
    for (epsilon, spread, groups) in [
        (0., 2., 3u32),
        (0.01, 0.3, 2),
        (0.25, 3., 4),
        (1e-7, 1., 1),
        // Cells at their floor (2^-20 m): coordinates on many cell boundaries.
        (1e-9, 1e-5, 2),
    ] {
        let weld = MeshWeld::new(epsilon).unwrap();
        let mut state = 0x9e37_79b9_7f4a_7c15u64 ^ epsilon.to_bits();
        let n = 3000usize;
        let mut vertices: Vec<(u32, [f32; 3])> = Vec::with_capacity(n);
        for k in 0..n {
            let group = (next(&mut state) % u64::from(groups)) as u32;
            let position = if k > 0 && next(&mut state).is_multiple_of(3) {
                // A repeat or near-repeat of an earlier vertex, up to 1.5 ε away per axis.
                let (_, base) = vertices[(next(&mut state) % k as u64) as usize];
                base.map(|v| {
                    let jitter = if next(&mut state).is_multiple_of(2) {
                        0.
                    } else {
                        (unit(&mut state) * 3. - 1.5) * epsilon
                    };
                    (f64::from(v) + jitter) as f32
                })
            } else {
                std::array::from_fn(|_| (unit(&mut state) * spread) as f32)
            };
            vertices.push((group, canonical(position)));
        }
        let mut records = Vec::new();
        for (id, (group, position)) in vertices.iter().enumerate() {
            let before = records.len();
            points(&weld, *group, id as u32, *position, &mut records).unwrap();
            let mine = &records[before..];
            assert!((1..=4).contains(&mine.len()), "{epsilon}: {}", mine.len());
            assert_eq!(mine.iter().filter(|p| p.id & HOME != 0).count(), 1);
        }
        records.sort();
        let mut classes = Classes::new(n).unwrap();
        let paired: Vec<AtomicU64> = (0..n.div_ceil(64)).map(|_| AtomicU64::new(0)).collect();
        let mut homes = Vec::new();
        sweep(
            records.into_iter().map(Ok),
            epsilon,
            &mut classes,
            &paired,
            &mut |home| {
                homes.push(home.id);
                Ok(())
            },
        )
        .unwrap();
        homes.sort_unstable();
        assert_eq!(homes, (0..n as u32).collect::<Vec<_>>());
        let mut truth = Classes::new(n).unwrap();
        let mut pairs = 0;
        for a in 0..n {
            for b in a + 1..n {
                let ((ga, pa), (gb, pb)) = (vertices[a], vertices[b]);
                if ga == gb && within(bits(pa), bits(pb), epsilon) {
                    truth.union(a as u32, b as u32);
                    pairs += 1;
                }
            }
        }
        assert!(pairs > 100, "{epsilon}: {pairs} pairs");
        let roots: Vec<u32> = (0..n as u32).map(|v| truth.find(v)).collect();
        let mut sizes = vec![0u32; n];
        for root in &roots {
            sizes[*root as usize] += 1;
        }
        let mut merged = 0;
        for v in 0..n as u32 {
            let root = roots[v as usize];
            assert_eq!(classes.find(v), root, "{epsilon}: vertex {v}");
            // Roots are the lowest index of their class, and paired exactly when in one.
            assert!(root <= v);
            let in_class = sizes[root as usize] > 1;
            assert_eq!(bit(&paired, v as usize), in_class, "{epsilon}: vertex {v}");
            merged += u64::from(root != v);
        }
        assert!(merged > 100, "{epsilon}: {merged}");
    }
}

/// The classes do not depend on the order pairs are joined in: the root is the lowest index.
#[test]
fn union_find_roots_are_the_lowest_index_in_any_order() {
    let pairs = [(5u32, 9u32), (9, 2), (7, 8), (2, 6), (8, 3)];
    let mut orders = vec![pairs.to_vec()];
    let mut reversed = pairs.to_vec();
    reversed.reverse();
    orders.push(reversed);
    orders.push(pairs.iter().map(|&(a, b)| (b, a)).collect());
    for order in orders {
        let mut classes = Classes::new(10).unwrap();
        for (a, b) in order {
            classes.union(a, b);
        }
        let roots: Vec<u32> = (0..10).map(|v| classes.find(v)).collect();
        assert_eq!(roots, [0, 1, 2, 3, 4, 2, 2, 3, 3, 2]);
    }
}

/// Four 8 × 8 blocks of 1 m cells side by side in x (block `b` spans x 8b to 8b + 8), heights
/// varying, Y up. Each has normals and UVs of its own, so the vertices where two blocks meet
/// differ in both. Blocks 0 and 1 are primitive 0, block 2 primitive 1 (the same material),
/// block 3 primitive 2 (another material). A block's first column, where it meets the block
/// before, is moved `delta` in x.
fn blocks(parent: &Path, delta: f32) -> PathBuf {
    fs::create_dir_all(parent).unwrap();
    let (n, row) = (8u32, 9u32);
    let height = |x: u32, j: u32| ((x * 7 + j * 3) % 5) as f32 * 0.25;
    let mut bin: Vec<u8> = Vec::new();
    let mut views = Vec::new();
    let mut accessors = Vec::new();
    let mut view = |bin: &mut Vec<u8>, bytes: &[u8]| {
        let at = bin.len();
        bin.extend_from_slice(bytes);
        views.push(json!({"buffer":0,"byteOffset":at,"byteLength":bytes.len()}));
        views.len() - 1
    };
    let mut primitives = Vec::new();
    for (members, material) in [(&[0u32, 1][..], 0), (&[2][..], 0), (&[3][..], 1)] {
        let (mut positions, mut normals, mut uvs, mut indices) =
            (Vec::new(), Vec::new(), Vec::new(), Vec::new());
        let (mut low, mut high) = ([f32::MAX; 3], [f32::MIN; 3]);
        for (k, &b) in members.iter().enumerate() {
            let base = k as u32 * row * row;
            for j in 0..row {
                for i in 0..row {
                    let x = 8 * b + i;
                    let p = [
                        x as f32 + if i == 0 && b > 0 { delta } else { 0. },
                        height(x, j),
                        -(j as f32),
                    ];
                    for a in 0..3 {
                        low[a] = low[a].min(p[a]);
                        high[a] = high[a].max(p[a]);
                    }
                    positions.extend(p.iter().flat_map(|v| v.to_le_bytes()));
                    let tilt = 0.1 * b as f32;
                    let length = (1. + tilt * tilt).sqrt();
                    normals.extend(
                        [tilt / length, 1. / length, 0.]
                            .iter()
                            .flat_map(|v| v.to_le_bytes()),
                    );
                    uvs.extend(
                        [i as f32 / n as f32, j as f32 / n as f32]
                            .iter()
                            .flat_map(|v| v.to_le_bytes()),
                    );
                }
            }
            for j in 0..n {
                for i in 0..n {
                    let a = base + j * row + i;
                    let (b, c, d) = (a + 1, a + row, a + row + 1);
                    indices.extend([a, c, b, b, c, d].iter().flat_map(|v| v.to_le_bytes()));
                }
            }
        }
        let count = members.len() as u32 * row * row;
        let first = accessors.len();
        accessors.push(
            json!({"bufferView":view(&mut bin, &positions),"componentType":5126,
            "count":count,"type":"VEC3","min":low,"max":high}),
        );
        accessors.push(
            json!({"bufferView":view(&mut bin, &normals),"componentType":5126,
            "count":count,"type":"VEC3"}),
        );
        accessors.push(
            json!({"bufferView":view(&mut bin, &uvs),"componentType":5126,
            "count":count,"type":"VEC2"}),
        );
        accessors.push(
            json!({"bufferView":view(&mut bin, &indices),"componentType":5125,
            "count":members.len() as u32 * n * n * 6,"type":"SCALAR"}),
        );
        primitives.push(json!({"attributes":{"POSITION":first,"NORMAL":first + 1,
            "TEXCOORD_0":first + 2},"indices":first + 3,"material":material}));
    }
    fs::write(parent.join("blocks.bin"), &bin).unwrap();
    let doc = json!({
        "asset":{"version":"2.0"},"scene":0,"scenes":[{"nodes":[0]}],"nodes":[{"mesh":0}],
        "meshes":[{"primitives":primitives}],
        "materials":[{"name":"a"},{"name":"b"}],
        "buffers":[{"uri":"blocks.bin","byteLength":bin.len()}],
        "bufferViews":views,"accessors":accessors
    });
    let input = parent.join("blocks.gltf");
    fs::write(&input, serde_json::to_vec(&doc).unwrap()).unwrap();
    input
}

/// A capture's retained records and its weld statement.
fn capture(
    input: &Path,
    parent: &Path,
    jobs: usize,
    weld: Option<MeshWeld>,
    plan: Option<AssemblyPlan>,
) -> (Vec<FullTriangle>, Option<MeshWeldReport>) {
    fs::create_dir_all(parent).unwrap();
    let mut config = full_tests::config(parent);
    config.sort.buffer_bytes = 64;
    config.jobs = Jobs::Fixed(NonZeroUsize::new(jobs).unwrap());
    config.test_assembly = plan;
    config.weld = weld;
    let output = full::capture(input, &config, &mut || Ok(())).unwrap();
    let rows = output
        .triangles
        .reader()
        .unwrap()
        .collect::<std::result::Result<Vec<_>, _>>()
        .unwrap();
    let report = output.report.weld.clone();
    assert_eq!(output.report.memory.weld_bytes > 0, weld.is_some());
    output.close().unwrap();
    assert_eq!(fs::read_dir(parent).unwrap().count(), 0);
    (rows, report)
}

fn encoded(rows: &[FullTriangle]) -> Vec<Vec<u8>> {
    rows.iter()
        .map(|t| {
            let mut bytes = vec![0; FullTriangle::SIZE];
            t.encode(&mut bytes);
            bytes
        })
        .collect()
}

/// Records by source triangle.
fn by_source(rows: &[FullTriangle]) -> BTreeMap<(u32, u64), FullTriangle> {
    rows.iter()
        .map(|t| {
            (
                (t.triangle.source.primitive, t.triangle.source.triangle),
                *t,
            )
        })
        .collect()
}

const DELTA: f32 = 1. / 4096.;
const EPSILON: f64 = 1. / 1024.;

/// A near seam inside a primitive closes at ε; with `same-material`, also the seam between
/// two primitives of one material, never the one between materials. Only positions move,
/// each onto the lowest-index vertex's; normals, UVs and provenance stay each vertex's own.
/// The records are the same for one and three workers and for every vertex store.
#[test]
fn a_capture_welds_near_seams_by_scope_and_moves_positions_only() {
    let dir = tempfile::tempdir().unwrap();
    let input = blocks(&dir.path().join("source"), DELTA);
    let (plain, none) = capture(&input, &dir.path().join("plain"), 1, None, None);
    assert!(none.is_none());
    let plain_by_source = by_source(&plain);
    for (join, merged, moved_columns) in [(false, 9, &[8.][..]), (true, 18, &[8., 16.][..])] {
        let weld = MeshWeld::new(EPSILON).unwrap();
        let weld = if join {
            weld.joining_same_material()
        } else {
            weld
        };
        let (rows, report) = capture(&input, &dir.path().join("weld"), 1, Some(weld), None);
        let report = report.unwrap();
        assert_eq!(report.vertices, 4 * 81);
        assert_eq!(
            (
                report.merged_vertices,
                report.moved_vertices,
                report.classes
            ),
            (merged, merged, merged)
        );
        assert_eq!(report.largest_class, 2);
        assert_eq!(report.max_displacement_m, f64::from(DELTA));
        assert_eq!(report.geometric_error_term_m, f64::from(DELTA).next_up());
        assert_eq!(
            report.join_primitives,
            if join { "same-material" } else { "none" }
        );
        let welded = by_source(&rows);
        assert_eq!(welded.len(), plain_by_source.len());
        let mut moved_corners = 0;
        for (key, t) in &welded {
            let p = &plain_by_source[key];
            assert_eq!(t.corners, p.corners, "attributes stay each vertex's own");
            assert_eq!(t.triangle.vertices, p.triangle.vertices);
            assert_eq!(t.material, p.material);
            for (a, b) in t.triangle.positions.iter().zip(&p.triangle.positions) {
                let column = f64::from(b[0]) - f64::from(DELTA);
                if b[0].fract() != 0. && moved_columns.contains(&column) {
                    assert_eq!(*a, [column as f32, b[1], b[2]]);
                    moved_corners += 1;
                } else {
                    assert_eq!(a, b);
                }
            }
        }
        assert!(moved_corners > 0);
        // Every worker count and vertex store captures the same records.
        for jobs in [1, 3] {
            for plan in [
                None,
                Some(full_tests::spilled(1 << 20)),
                Some(full_tests::spilled(1)),
            ] {
                let parent = dir
                    .path()
                    .join(format!("weld-{join}-{jobs}-{}", plan.is_some()));
                let (again, stated) = capture(&input, &parent, jobs, Some(weld), plan);
                assert!(encoded(&again) == encoded(&rows), "{jobs} jobs, {plan:?}");
                assert_eq!(stated.as_ref(), Some(&report));
            }
        }
    }
    // A tolerance below the gap welds nothing.
    let weld = MeshWeld::new(f64::from(DELTA) / 2.).unwrap();
    let (rows, report) = capture(&input, &dir.path().join("tight"), 1, Some(weld), None);
    assert_eq!(report.unwrap().merged_vertices, 0);
    assert!(encoded(&rows) == encoded(&plain));
}

/// At ε = 0, exact repeats are merged (and reported) but no position moves, so the captured
/// records are the unwelded capture's, byte for byte.
#[test]
fn exact_repeats_weld_without_changing_a_captured_record() {
    let dir = tempfile::tempdir().unwrap();
    let input = blocks(&dir.path().join("source"), 0.);
    let (plain, _) = capture(&input, &dir.path().join("plain"), 1, None, None);
    for (weld, merged) in [
        (MeshWeld::new(0.).unwrap(), 9),
        (MeshWeld::new(0.).unwrap().joining_same_material(), 18),
    ] {
        let (rows, report) = capture(&input, &dir.path().join("weld"), 2, Some(weld), None);
        let report = report.unwrap();
        assert_eq!((report.merged_vertices, report.moved_vertices), (merged, 0));
        assert_eq!(
            (report.max_displacement_m, report.geometric_error_term_m),
            (0., 0.)
        );
        assert!(encoded(&rows) == encoded(&plain));
    }
}

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

fn bake(
    input: &Path,
    work: &Path,
    name: &str,
    jobs: usize,
    weld: Option<MeshWeld>,
) -> (PathBuf, crate::mesh_large::TilerResult) {
    let scratch = work.join(format!("scratch-{name}"));
    fs::create_dir_all(&scratch).unwrap();
    let output = work.join(format!("{name}.3tz"));
    let mut request = TilerRequest::new(input, &output)
        .with_jobs(TilerJobs::Maximum(NonZeroUsize::new(jobs).unwrap()))
        .with_scratch_dir(&scratch)
        .with_test_leaf_triangles(32);
    if let Some(weld) = weld {
        request = request.with_weld(weld);
    }
    let result = crate::mesh_to_tileset(request, &RunControl::default()).unwrap();
    assert_eq!(fs::read_dir(&scratch).unwrap().count(), 0);
    (output, result)
}

/// A weld that moves nothing publishes the unwelded archive: every member but the
/// conversion report (which states the weld) is byte-identical.
#[test]
fn a_weld_that_moves_nothing_publishes_the_same_tiles() {
    let work = tempfile::tempdir().unwrap();
    let input = blocks(&work.path().join("src"), 0.);
    let (plain, plain_result) = bake(&input, work.path(), "plain", 2, None);
    let weld = MeshWeld::new(0.).unwrap().joining_same_material();
    let (welded, result) = bake(&input, work.path(), "welded", 2, Some(weld));
    assert!(plain_result.report.weld.is_none());
    assert_eq!(result.report.weld.as_ref().unwrap().merged_vertices, 18);
    let (a, b) = (members(&plain), members(&welded));
    assert_eq!(a.keys().collect::<Vec<_>>(), b.keys().collect::<Vec<_>>());
    let differing: Vec<&String> = a.keys().filter(|k| a[*k] != b[*k]).collect();
    assert_eq!(differing, ["conversion.json"]);
    assert!(result.diagnostics.memory.capture_weld_bytes > 0);
    assert_eq!(plain_result.diagnostics.memory.capture_weld_bytes, 0);
}

/// A weld that moves vertices adds its largest displacement to every published error, leaf
/// and coarse, keeps the tile frame, validates, and is the same archive for every worker
/// count.
#[test]
fn published_errors_carry_the_weld_displacement() {
    let work = tempfile::tempdir().unwrap();
    // Near-seams 2 cm wide: a displacement that dwarfs the leaves' own error.
    let input = blocks(&work.path().join("src"), 0.02);
    let weld = MeshWeld::new(0.05).unwrap().joining_same_material();
    let (plain, _) = bake(&input, work.path(), "plain", 2, None);
    let mut archives = Vec::new();
    let mut term = 0.;
    for jobs in [1, 3] {
        let (welded, result) = bake(
            &input,
            work.path(),
            &format!("welded-{jobs}"),
            jobs,
            Some(weld),
        );
        let stated = result.report.weld.clone().unwrap();
        assert_eq!(stated.merged_vertices, 18);
        assert!(
            (stated.max_displacement_m - 0.02).abs() < 1e-6,
            "{stated:?}"
        );
        term = stated.geometric_error_term_m;
        crate::validate::inspect(crate::validate::ValidationRequest::new(&welded)).unwrap();
        archives.push(fs::read(&welded).unwrap());
        let set = TileSet::open(&welded).unwrap();
        let errors: Vec<f64> = set.tiles().iter().map(|t| t.geometric_error).collect();
        assert!(errors.iter().all(|g| *g >= term), "{errors:?}");
        let plain_set = TileSet::open(&plain).unwrap();
        assert_eq!(
            set.root_transform().map(f64::to_bits),
            plain_set.root_transform().map(f64::to_bits)
        );
    }
    assert!(term > 0.02);
    assert!(archives.windows(2).all(|w| w[0] == w[1]));
}
