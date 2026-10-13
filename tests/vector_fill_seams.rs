//! Quantized fill fragments in different tiles must meet without cracks.
//!
//! Each published content is decoded as a client does (meshopt vertex
//! stream, normalized u16 or float32, node translation and scale,
//! implicit-root translations) into the tileset root frame. Wherever two contents that can
//! render together (neither is the other's ancestor) put fill vertices within
//! one lattice step of each other, they must be the same point: a vertex
//! shared across a seam decodes identically on both sides. Deriving the
//! quantization range per payload (the defect) fails this.
use serde_json::{json, Value};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    io::Read,
    path::Path,
    process::Command,
};

fn convert(input: &Path, output: &Path, arguments: &[&str]) {
    let result = Command::new(env!("CARGO_BIN_EXE_rusty-tiles"))
        .args(["vector", "-i"])
        .arg(input)
        .arg("-o")
        .arg(output)
        .args(["--json", "--reproducible"])
        .args(arguments)
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{} {}",
        String::from_utf8_lossy(&result.stdout),
        String::from_utf8_lossy(&result.stderr)
    );
}

fn member(zip: &mut zip::ZipArchive<fs::File>, name: &str) -> Vec<u8> {
    let mut bytes = Vec::new();
    zip.by_name(name).unwrap().read_to_end(&mut bytes).unwrap();
    bytes
}

/// Implicit root id -> (depth of the implicit root, its z-up translation
/// relative to the tileset root). The root's own frame transform is common
/// to every content, so positions are compared before it.
fn implicit_roots(
    node: &Value,
    depth: usize,
    offset: [f64; 3],
    out: &mut BTreeMap<String, (usize, [f64; 3])>,
) {
    let mut offset = offset;
    if depth > 0 {
        if let Some(t) = node["transform"].as_array() {
            let m: Vec<f64> = t.iter().map(|v| v.as_f64().unwrap()).collect();
            assert_eq!(
                [m[0], m[5], m[10], m[1], m[2], m[4], m[6], m[8], m[9]],
                [1., 1., 1., 0., 0., 0., 0., 0., 0.],
                "below the root, tile transforms are translations"
            );
            offset = std::array::from_fn(|i| offset[i] + m[12 + i]);
        }
    }
    if node.get("implicitTiling").is_some() {
        let uri = node["content"]["uri"].as_str().unwrap();
        let id = uri
            .strip_prefix("implicit-content/")
            .and_then(|u| u.split('-').next())
            .unwrap();
        out.insert(id.to_string(), (depth, offset));
    }
    for child in node["children"].as_array().into_iter().flatten() {
        implicit_roots(child, depth + 1, offset, out);
    }
}

struct Content {
    name: String,
    /// Implicit root, level, x, y (quadtree).
    tile: (String, usize, u64, u64),
    depth: usize,
    /// The lattice step of u16 contents; float32 contents have none.
    step: Option<[f64; 3]>,
    fills: Vec<[f64; 3]>,
    lines: Vec<[f64; 3]>,
}

/// Every content's fill (mode 4) and line (mode 3) vertices in the root
/// frame (glTF y-up).
fn contents(archive: &Path) -> Vec<Content> {
    let mut zip = zip::ZipArchive::new(fs::File::open(archive).unwrap()).unwrap();
    let manifest: Value = serde_json::from_slice(&member(&mut zip, "tileset.json")).unwrap();
    let mut roots = BTreeMap::new();
    implicit_roots(&manifest["root"], 0, [0.; 3], &mut roots);
    let names: Vec<String> = zip
        .file_names()
        .filter(|n| n.starts_with("implicit-content/") && n.ends_with(".glb"))
        .map(str::to_string)
        .collect();
    let mut out = Vec::new();
    for name in names {
        let bytes = member(&mut zip, &name);
        let fields: Vec<&str> = name
            .trim_start_matches("implicit-content/")
            .trim_end_matches(".glb")
            .split('-')
            .collect();
        let (root_depth, offset) = roots[fields[0]];
        let level: usize = fields[1].parse().unwrap();
        let length = u32::from_le_bytes(bytes[12..16].try_into().unwrap()) as usize;
        let doc: Value = serde_json::from_slice(&bytes[20..20 + length]).unwrap();
        let binary = &bytes[28 + length..];
        assert_eq!(doc["nodes"].as_array().unwrap().len(), 1);
        let node = &doc["nodes"][0];
        let vector = |key: &str, default: f64| -> [f64; 3] {
            std::array::from_fn(|i| node[key][i].as_f64().unwrap_or(default))
        };
        let (translation, scale) = (vector("translation", 0.), vector("scale", 1.));
        // z-up tile offset as the glTF y-up translation it composes with.
        let offset = [offset[0], offset[2], -offset[1]];
        let mut fills = Vec::new();
        let mut lines = Vec::new();
        let mut float = None;
        for primitive in doc["meshes"][0]["primitives"].as_array().unwrap() {
            let mode = primitive["mode"].as_u64().unwrap_or(4);
            if mode != 4 && mode != 3 {
                continue;
            }
            let accessor =
                &doc["accessors"][primitive["attributes"]["POSITION"].as_u64().unwrap() as usize];
            // Normalized u16 codes, or float32 lattice values (a tile
            // coarser than a tile it shares a vertex with).
            let is_float = match accessor["componentType"].as_u64() {
                Some(5123) => {
                    assert_eq!(accessor["normalized"], true, "{name}");
                    false
                }
                Some(5126) => true,
                other => panic!("{name}: position component type {other:?}"),
            };
            assert_eq!(
                *float.get_or_insert(is_float),
                is_float,
                "{name}: one storage"
            );
            let count = accessor["count"].as_u64().unwrap() as usize;
            let view = &doc["bufferViews"][accessor["bufferView"].as_u64().unwrap() as usize];
            let stride = if is_float { 12 } else { 8 };
            let raw: Vec<u8> = if let Some(e) = view["extensions"].get("EXT_meshopt_compression") {
                assert_eq!(e["mode"], "ATTRIBUTES");
                let start = e["byteOffset"].as_u64().unwrap_or(0) as usize;
                let stream = &binary[start..start + e["byteLength"].as_u64().unwrap() as usize];
                if is_float {
                    meshopt::encoding::decode_vertex_buffer::<[u8; 12]>(stream, count)
                        .unwrap()
                        .concat()
                } else {
                    meshopt::encoding::decode_vertex_buffer::<[u8; 8]>(stream, count)
                        .unwrap()
                        .concat()
                }
            } else {
                let start = view["byteOffset"].as_u64().unwrap_or(0) as usize
                    + accessor["byteOffset"].as_u64().unwrap_or(0) as usize;
                binary[start..start + count * stride].to_vec()
            };
            let target = if mode == 4 { &mut fills } else { &mut lines };
            target.extend(raw.chunks_exact(stride).map(|v| -> [f64; 3] {
                std::array::from_fn(|i| {
                    let value = if is_float {
                        f64::from(f32::from_le_bytes(v[i * 4..i * 4 + 4].try_into().unwrap()))
                    } else {
                        f64::from(u16::from_le_bytes([v[i * 2], v[i * 2 + 1]])) / 65535.
                    };
                    offset[i] + (translation[i] + value * scale[i])
                })
            }));
        }
        if !fills.is_empty() || !lines.is_empty() {
            out.push(Content {
                tile: (
                    fields[0].to_string(),
                    level,
                    fields[2].parse().unwrap(),
                    fields[3].parse().unwrap(),
                ),
                name,
                depth: root_depth + level,
                step: (float == Some(false)).then(|| scale.map(|s| s / 65535.)),
                fills,
                lines,
            });
        }
    }
    out
}

/// Whether one content's tile is an ancestor of the other's (REPLACE
/// refinement never draws both, so their copies need not agree).
fn nested(a: &Content, b: &Content) -> bool {
    let above = |p: &Content, c: &Content| {
        let ((rp, lp, xp, yp), (rc, lc, xc, yc)) = (&p.tile, &c.tile);
        rp == rc && lp < lc && xc >> (lc - lp) == *xp && yc >> (lc - lp) == *yp
    };
    above(a, b) || above(b, a)
}

/// The largest u16 lattice step of any content, per axis.
fn largest_step(contents: &[Content]) -> [f64; 3] {
    std::array::from_fn(|i| {
        contents
            .iter()
            .filter_map(|c| c.step)
            .map(|s| s[i])
            .fold(0., f64::max)
    })
}

/// Fill vertices, and line vertices, of different contents within one
/// lattice step on every axis must coincide. Returns (fill seam vertex
/// pairs, pairs at mixed depth).
fn assert_seams_agree(archive: &Path) -> (usize, usize) {
    let contents = contents(archive);
    assert!(
        contents.len() > 1,
        "{archive:?}: fragments in several contents"
    );
    let (seams, mixed) = assert_kind_agrees(archive, &contents, |c| &c.fills);
    // Boundary runs meet across tiles too.
    let (line_seams, _) = assert_kind_agrees(archive, &contents, |c| &c.lines);
    assert!(line_seams > 0, "line seam vertices exercised");
    // Both storages are exercised: u16 contents, and float32 contents that
    // are coarser than a content they share a vertex with.
    let floats = contents.iter().filter(|c| c.step.is_none()).count();
    assert!(
        floats > 0 && floats < contents.len(),
        "{archive:?}: {floats} of {} contents float32",
        contents.len()
    );
    assert!(
        contents
            .iter()
            .any(|c| c.step.is_none() && !c.lines.is_empty()),
        "{archive:?}: float32 contents with line strips"
    );
    (seams, mixed)
}

fn assert_kind_agrees(
    archive: &Path,
    contents: &[Content],
    kind: fn(&Content) -> &Vec<[f64; 3]>,
) -> (usize, usize) {
    let step = largest_step(contents);
    // Points of one grid are equal or at least one step apart on an axis.
    let cell =
        |p: &[f64; 3]| -> [i64; 3] { std::array::from_fn(|i| (p[i] / step[i]).floor() as i64) };
    let mut buckets: BTreeMap<[i64; 3], Vec<(usize, [f64; 3])>> = BTreeMap::new();
    for (c, content) in contents.iter().enumerate() {
        let unique: BTreeSet<[u64; 3]> =
            kind(content).iter().map(|p| p.map(f64::to_bits)).collect();
        for p in unique {
            let p = p.map(f64::from_bits);
            buckets.entry(cell(&p)).or_default().push((c, p));
        }
    }
    let (mut seams, mut mixed) = (0, 0);
    let mut failures = Vec::new();
    for (key, points) in &buckets {
        for dx in -1..=1 {
            for dy in -1..=1 {
                for dz in -1..=1 {
                    let other = [key[0] + dx, key[1] + dy, key[2] + dz];
                    if other < *key {
                        continue;
                    }
                    let Some(others) = buckets.get(&other) else {
                        continue;
                    };
                    for (a, p) in points {
                        for (b, q) in others {
                            if a == b
                                || (other == *key && a > b)
                                || nested(&contents[*a], &contents[*b])
                            {
                                continue;
                            }
                            let near = (0..3).all(|i| (p[i] - q[i]).abs() < step[i] * 0.999);
                            if !near {
                                continue;
                            }
                            let gap = (0..3).map(|i| (p[i] - q[i]).abs()).fold(0., f64::max);
                            let scale = p.iter().map(|v| v.abs()).fold(1., f64::max);
                            if gap > scale * 1e-12 {
                                failures.push(format!(
                                    "{} vs {}: {p:?} {q:?} gap {gap}",
                                    contents[*a].name, contents[*b].name
                                ));
                            } else {
                                seams += 1;
                                if contents[*a].depth != contents[*b].depth {
                                    mixed += 1;
                                }
                            }
                        }
                    }
                }
            }
        }
    }
    assert!(
        failures.is_empty(),
        "{archive:?}: {} cracked seam vertices, e.g. {:?}",
        failures.len(),
        &failures[..failures.len().min(4)]
    );
    (seams, mixed)
}

#[test]
fn country_fragments_share_decoded_seam_vertices_across_pole_and_antimeridian() {
    // Sudan and Antarctica (pole, antimeridian, negative frame coordinates).
    let temp = tempfile::tempdir().unwrap();
    let input =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/countries-source.geojson");
    for meshopt in [false, true] {
        let archive = temp.path().join(format!("countries-{meshopt}.3tz"));
        let mut arguments = vec!["--maxVertices", "256", "--quantize"];
        if meshopt {
            arguments.push("--meshopt");
        }
        convert(&input, &archive, &arguments);
        let (seams, mixed) = assert_seams_agree(&archive);
        assert!(seams > 100, "seam vertices exercised: {seams}");
        assert!(mixed > 0, "mixed-depth seams exercised");
    }
}

#[test]
fn neighbouring_features_and_fragments_share_seams_at_negative_coordinates() {
    // Every polygon split, then only five of nine (split next to unsplit).
    block(false);
    block(true);
}

/// A 3x3 block of polygons with wiggly shared borders in negative local
/// coordinates, fragmented across tiles of different depths. With `holes`,
/// only the five polygons with a hole exceed the vertex budget and split;
/// their unsplit neighbours share the borders.
fn block(holes: bool) {
    let temp = tempfile::tempdir().unwrap();
    let xs = [-30000.25, -19876.5, -10123.75, -0.125];
    let ys = [-20000.5, -9000.25, 1000.75, 10000.5];
    let n = 24;
    // Border vertices just past a rounding tie of the 0.5 m frame lattice
    // (the first vertex is the local frame's anchor): snapping anything but
    // the exact source position (e.g. a float32 tile-local value) splits them.
    let tie = |x: f64| xs[0] + ((x - xs[0]) / 0.5).floor() * 0.5 + 0.25 + 2f64.powi(-28);
    let vertical = |x: f64, y0: f64, y1: f64| -> Vec<[f64; 3]> {
        (0..=n)
            .map(|k| {
                let y = y0 + (y1 - y0) * k as f64 / n as f64;
                let w = if k == 0 || k == n {
                    0.
                } else {
                    313.7 * (y * 0.0021).sin()
                };
                [if w == 0. { x } else { tie(x + w) }, y, 0.]
            })
            .collect()
    };
    let horizontal = |y: f64, x0: f64, x1: f64| -> Vec<[f64; 3]> {
        (0..=n)
            .map(|k| {
                let x = x0 + (x1 - x0) * k as f64 / n as f64;
                let w = if k == 0 || k == n {
                    0.
                } else {
                    271.3 * (x * 0.0017).cos()
                };
                [if w == 0. { x } else { tie(x) }, y + w, 0.]
            })
            .collect()
    };
    let mut features = Vec::new();
    // Vertices of each polygon (bit keys), and whether that polygon splits.
    let mut polygons: Vec<(BTreeSet<[u64; 3]>, bool)> = Vec::new();
    for i in 0..3 {
        for j in 0..3 {
            let (x0, x1, y0, y1) = (xs[i], xs[i + 1], ys[j], ys[j + 1]);
            // Counter-clockwise: bottom, right, top (reversed), left (reversed).
            let mut ring = horizontal(y0, x0, x1);
            ring.extend(vertical(x1, y0, y1).into_iter().skip(1));
            ring.extend(horizontal(y1, x0, x1).into_iter().rev().skip(1));
            ring.extend(vertical(x0, y0, y1).into_iter().rev().skip(1));
            let mut rings = vec![ring];
            let split = !holes || (i + j) % 2 == 0;
            if holes && split {
                let (cx, cy) = ((x0 + x1) / 2., (y0 + y1) / 2.);
                let hole: Vec<[f64; 3]> = [(-1., -1.), (-1., 1.), (1., 1.), (1., -1.), (-1., -1.)]
                    .iter()
                    .map(|(dx, dy)| [cx + 500. * dx, cy + 500. * dy, 0.])
                    .collect();
                rings.push(hole);
            }
            let keys = rings
                .iter()
                .flatten()
                .map(|p| p.map(f64::to_bits))
                .collect();
            polygons.push((keys, split));
            features.push(json!({"type":"Feature","id":format!("{i}{j}"),"properties":{"name":format!("{i}{j}")},
                "geometry":{"type":"Polygon","coordinates":rings}}));
        }
    }
    // Spooled in more than one row: a polygon's own fragments and boundary,
    // or the border of two polygons.
    let mut all = BTreeSet::new();
    let mut shared = BTreeSet::new();
    for (keys, split) in &polygons {
        for k in keys {
            if *split
                || polygons
                    .iter()
                    .filter(|(other, _)| other.contains(k))
                    .count()
                    > 1
            {
                shared.insert(*k);
            }
            all.insert(*k);
        }
    }
    let all: Vec<([f64; 3], bool)> = all
        .into_iter()
        .map(|k| (k.map(f64::from_bits), shared.contains(&k)))
        .collect();
    let input = temp.path().join("block.geojson");
    fs::write(
        &input,
        json!({"type":"FeatureCollection","features":features}).to_string(),
    )
    .unwrap();
    for meshopt in [false, true] {
        let archive = temp.path().join(format!("block-{meshopt}.3tz"));
        let budget = if holes { "98" } else { "48" };
        let mut arguments = vec![
            "--source-crs",
            "local",
            "--maxVertices",
            budget,
            "--maxFeatures",
            "2",
            "--quantize",
        ];
        if meshopt {
            arguments.push("--meshopt");
        }
        convert(&input, &archive, &arguments);
        let mut zip = zip::ZipArchive::new(fs::File::open(&archive).unwrap()).unwrap();
        let report: Value = serde_json::from_slice(&member(&mut zip, "conversion.json")).unwrap();
        assert_eq!(
            report["fragmentedPolygons"],
            if holes { 5 } else { 9 },
            "{report}"
        );
        let (seams, mixed) = assert_seams_agree(&archive);
        assert!(seams > 100, "seam vertices exercised: {seams}");
        assert!(mixed > 0, "mixed-depth seams exercised");
        assert_source_vertices_snapped(&archive, &all);
    }
}

/// Source-vertex oracle (local frame only): every fill vertex of every
/// content is keyed by its nearest source vertex. All copies of a vertex
/// spooled in more than one row, in contents that can render together,
/// decode to exactly one point within a lattice step of the source.
fn assert_source_vertices_snapped(archive: &Path, sources: &[([f64; 3], bool)]) {
    let mut zip = zip::ZipArchive::new(fs::File::open(archive).unwrap()).unwrap();
    let state: Value = serde_json::from_slice(&member(&mut zip, "vector-build.json")).unwrap();
    let manifest: Value = serde_json::from_slice(&member(&mut zip, "tileset.json")).unwrap();
    let anchor: Vec<f64> = (0..3)
        .map(|i| state["anchor"][i].as_f64().unwrap())
        .collect();
    assert_eq!(
        state["frame"],
        json!([[1., 0., 0.], [0., 1., 0.], [0., 0., 1.]])
    );
    // Root translation = anchor + root center (z-up); positions are relative
    // to that center in glTF y-up.
    let t: Vec<f64> = (12..15)
        .map(|i| manifest["root"]["transform"][i].as_f64().unwrap())
        .collect();
    let center = [t[0] - anchor[0], t[2] - anchor[2], -(t[1] - anchor[1])];
    let frame = |p: &[f64; 3]| -> [f64; 3] {
        let d = [p[0] - anchor[0], p[1] - anchor[1], p[2] - anchor[2]];
        [d[0], d[2], -d[1]]
    };
    let framed: Vec<[f64; 3]> = sources.iter().map(|(p, _)| frame(p)).collect();
    let ties = sources
        .iter()
        .filter(|(p, shared)| *shared && ((frame(p)[0] / 0.5).fract().abs() - 0.5).abs() < 1e-6)
        .count();
    assert!(ties > 50, "near-tie shared source vertices: {ties}");
    let contents = contents(archive);
    // Source index -> (content, decoded copy).
    let mut copies: BTreeMap<usize, Vec<(usize, [f64; 3])>> = BTreeMap::new();
    // Precision: a u16 content keeps its own step, so every vertex,
    // shared or not, is within half that step of its source on each axis.
    // Grouping that snaps shared vertices at a coarser member's step (one
    // coarse step for all) fails here. A float32 content's shared vertices
    // snap at its u16 partners' steps.
    let largest = largest_step(&contents);
    let mut steps = BTreeSet::new();
    for (c, content) in contents.iter().enumerate() {
        let step = content.step.unwrap_or(largest);
        steps.insert(content.step.map(|s| s.map(f64::to_bits)));
        for p in &content.fills {
            let p: [f64; 3] = std::array::from_fn(|i| p[i] + center[i]);
            let gap = |s: &[f64; 3]| (0..3).map(|i| (s[i] - p[i]).abs()).fold(0., f64::max);
            let (index, source) = framed
                .iter()
                .enumerate()
                .min_by(|a, b| gap(a.1).total_cmp(&gap(b.1)))
                .unwrap();
            for i in 0..3 {
                assert!(
                    (source[i] - p[i]).abs() <= step[i] / 2. + 1e-9 * p[i].abs().max(1.),
                    "{}: {p:?} more than half a step {step:?} from {source:?}",
                    content.name
                );
            }
            copies.entry(index).or_default().push((c, p));
        }
    }
    assert!(steps.len() > 2, "u16 steps exercised: {steps:?}");
    let mut compared = 0;
    for (index, held) in &copies {
        if !sources[*index].1 {
            continue;
        }
        for (i, (a, p)) in held.iter().enumerate() {
            for (b, q) in &held[i + 1..] {
                if a == b || nested(&contents[*a], &contents[*b]) {
                    continue;
                }
                let gap = (0..3).map(|k| (p[k] - q[k]).abs()).fold(0., f64::max);
                assert!(
                    gap < 1e-6,
                    "{} vs {}: shared vertex {:?} decodes to {p:?} and {q:?}",
                    contents[*a].name,
                    contents[*b].name,
                    framed[*index]
                );
                compared += 1;
            }
        }
    }
    assert!(compared > 200, "shared copies compared: {compared}");
}
