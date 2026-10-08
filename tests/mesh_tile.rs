use std::fs;
use std::io::Read;
use std::path::Path;

use image::{Rgba, RgbaImage};
use rusty_tiles::bbox::bounding_box_from_gltf_path;
use rusty_tiles::fixtures::{geographic_glb, triangle_glb};
use rusty_tiles::glb_write::{write_glb, TilePrimitive};
use rusty_tiles::mesh;
use rusty_tiles::pack::list_zip_names;
use rusty_tiles::tile::{mesh_to_3tz, MeshTo3tzOptions};
use rusty_tiles::tileset::{glb_to_3tz, CreateTilesetOptions};
use rusty_tiles::{validate_3tz, Cartographic, SourceCrs, SourceOffset};
use serde_json::Value;

fn zip_bytes(tz: &Path, name: &str) -> Vec<u8> {
    let mut z = zip::ZipArchive::new(fs::File::open(tz).unwrap()).unwrap();
    let mut f = z.by_name(name).unwrap();
    let mut buf = Vec::new();
    f.read_to_end(&mut buf).unwrap();
    buf
}

fn tileset_json(tz: &Path) -> Value {
    let manifest = serde_json::from_slice(&zip_bytes(tz, "tileset.json")).unwrap();
    rusty_tiles::implicit::expand_tileset(&manifest, |name| Ok(zip_bytes(tz, name))).unwrap()
}

fn collect_uris(tile: &Value, out: &mut Vec<String>) {
    if let Some(uri) = tile.pointer("/content/uri").and_then(|v| v.as_str()) {
        out.push(uri.to_string());
    }
    if let Some(kids) = tile["children"].as_array() {
        for k in kids {
            collect_uris(k, out);
        }
    }
}

fn collect_leaf_ge(tile: &Value, out: &mut Vec<f64>) {
    let kids = tile["children"].as_array();
    let has_kids = kids.map(|a| !a.is_empty()).unwrap_or(false);
    if !has_kids {
        out.push(tile["geometricError"].as_f64().unwrap());
        return;
    }
    for k in kids.unwrap() {
        collect_leaf_ge(k, out);
    }
}

fn count_transforms(tile: &Value) -> usize {
    let n = usize::from(tile.get("transform").is_some());
    tile["children"]
        .as_array()
        .map(|kids| n + kids.iter().map(count_transforms).sum::<usize>())
        .unwrap_or(n)
}

fn assert_monotonic_ge(tile: &Value) {
    let ge = tile["geometricError"].as_f64().unwrap();
    let Some(kids) = tile["children"].as_array() else {
        return;
    };
    if kids.is_empty() {
        return;
    }
    for k in kids {
        let cg = k["geometricError"].as_f64().unwrap();
        assert!(ge + 1e-12 >= cg, "parent GE {ge} < child GE {cg}");
        assert_monotonic_ge(k);
    }
}

fn tile_obb(tile: &Value) -> rusty_tiles::bbox::Obb {
    let b = tile["boundingVolume"]["box"].as_array().unwrap();
    let boxv: [f64; 12] = std::array::from_fn(|i| b[i].as_f64().unwrap());
    rusty_tiles::bbox::Obb::from_box(boxv)
}

/// Every child box must sit inside its parent's box (k-d cells nest;
/// content that pokes out is absorbed into both).
fn assert_child_boxes_nested(tile: &Value) {
    let Some(kids) = tile["children"].as_array() else {
        return;
    };
    if kids.is_empty() {
        return;
    }
    let parent = tile_obb(tile);
    for k in kids {
        let child = tile_obb(k);
        for c in child.corners() {
            assert!(
                parent.contains(c, 1e-6),
                "child BV corner {c:?} outside parent {parent:?}"
            );
        }
    }
    for k in kids {
        assert_child_boxes_nested(k);
    }
}

fn glb_json(bytes: &[u8]) -> Value {
    let json_len = u32::from_le_bytes(bytes[12..16].try_into().unwrap()) as usize;
    serde_json::from_slice(&bytes[20..20 + json_len]).unwrap()
}

fn glb_index_count(bytes: &[u8]) -> usize {
    let j = glb_json(bytes);
    j["meshes"][0]["primitives"]
        .as_array()
        .unwrap()
        .iter()
        .map(|p| {
            let acc = p["indices"].as_u64().unwrap() as usize;
            j["accessors"][acc]["count"].as_u64().unwrap() as usize
        })
        .sum()
}

fn glb_jpeg_len(bytes: &[u8]) -> usize {
    let j = glb_json(bytes);
    let view = j["images"][0]["bufferView"].as_u64().unwrap() as usize;
    j["bufferViews"][view]["byteLength"].as_u64().unwrap() as usize
}

fn mat_id() -> [f64; 16] {
    let mut m = [0.0; 16];
    m[0] = 1.0;
    m[5] = 1.0;
    m[10] = 1.0;
    m[15] = 1.0;
    m
}

fn mat_mul(a: [f64; 16], b: [f64; 16]) -> [f64; 16] {
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

fn xform_pt(m: [f64; 16], p: [f64; 3]) -> [f64; 3] {
    [
        m[0] * p[0] + m[4] * p[1] + m[8] * p[2] + m[12],
        m[1] * p[0] + m[5] * p[1] + m[9] * p[2] + m[13],
        m[2] * p[0] + m[6] * p[1] + m[10] * p[2] + m[14],
    ]
}

fn union_world_boxes(tile: &Value, parent: [f64; 16], min: &mut [f64; 3], max: &mut [f64; 3]) {
    let xf = match tile.get("transform").and_then(|v| v.as_array()) {
        Some(arr) if arr.len() == 16 => {
            let mut t = [0.0; 16];
            for i in 0..16 {
                t[i] = arr[i].as_f64().unwrap();
            }
            mat_mul(parent, t)
        }
        _ => parent,
    };
    let kids = tile["children"]
        .as_array()
        .map(|a| !a.is_empty())
        .unwrap_or(false);
    if !kids {
        let b = tile["boundingVolume"]["box"].as_array().unwrap();
        let boxv: [f64; 12] = std::array::from_fn(|i| b[i].as_f64().unwrap());
        let (cx, cy, cz) = (boxv[0], boxv[1], boxv[2]);
        let hx = [boxv[3], boxv[4], boxv[5]];
        let hy = [boxv[6], boxv[7], boxv[8]];
        let hz = [boxv[9], boxv[10], boxv[11]];
        for sx in [-1.0, 1.0] {
            for sy in [-1.0, 1.0] {
                for sz in [-1.0, 1.0] {
                    let p = [
                        cx + sx * hx[0] + sy * hy[0] + sz * hz[0],
                        cy + sx * hx[1] + sy * hy[1] + sz * hz[1],
                        cz + sx * hx[2] + sy * hy[2] + sz * hz[2],
                    ];
                    let w = xform_pt(xf, p);
                    for i in 0..3 {
                        min[i] = min[i].min(w[i]);
                        max[i] = max[i].max(w[i]);
                    }
                }
            }
        }
        return;
    }
    for k in tile["children"].as_array().unwrap() {
        union_world_boxes(k, xf, min, max);
    }
}

fn encode_jpeg(img: &RgbaImage) -> Vec<u8> {
    let rgb = image::DynamicImage::ImageRgba8(img.clone()).to_rgb8();
    let mut buf = Vec::new();
    let mut enc = image::codecs::jpeg::JpegEncoder::new_with_quality(&mut buf, 90);
    enc.encode(
        rgb.as_raw(),
        rgb.width(),
        rgb.height(),
        image::ExtendedColorType::Rgb8,
    )
    .unwrap();
    buf
}

fn grid_glb(nx: u32, ny: u32) -> Vec<u8> {
    write_glb(&[grid_prim(nx, ny, |_, _| 0.0)]).unwrap()
}

#[test]
fn under_budget_matches_wrap() {
    let tmp = tempfile::tempdir().unwrap();
    let glb = tmp.path().join("triangle.glb");
    fs::write(&glb, triangle_glb()).unwrap();

    let wrap = tmp.path().join("wrap.3tz");
    glb_to_3tz(&glb, &wrap, &CreateTilesetOptions::default()).unwrap();

    let tiled = tmp.path().join("tiled.3tz");
    mesh_to_3tz(
        &glb,
        &tiled,
        &MeshTo3tzOptions {
            explicit: true,
            ..Default::default()
        },
    )
    .unwrap();

    validate_3tz(&tiled).unwrap();
    let mut wrap_names = list_zip_names(&wrap).unwrap();
    let mut tile_names = list_zip_names(&tiled).unwrap();
    wrap_names.sort();
    tile_names.sort();
    assert_eq!(wrap_names, tile_names);
    assert_eq!(
        zip_bytes(&wrap, "triangle.glb"),
        zip_bytes(&tiled, "triangle.glb")
    );
}

#[test]
fn eighty_k_grid_splits_under_budget() {
    let tmp = tempfile::tempdir().unwrap();
    let glb = tmp.path().join("grid.glb");
    fs::write(&glb, grid_glb(200, 200)).unwrap();
    let scene = mesh::load(&glb).unwrap();
    assert_eq!(scene.triangle_count(), 80_000);

    let tz = tmp.path().join("grid.3tz");
    mesh_to_3tz(&glb, &tz, &MeshTo3tzOptions::default()).unwrap();
    validate_3tz(&tz).unwrap();

    let ts = tileset_json(&tz);
    assert_eq!(ts["asset"]["version"], "1.1");
    assert_eq!(ts["root"]["refine"], "REPLACE");
    assert_eq!(
        count_transforms(&ts["root"]),
        0,
        "shared local frame: no per-tile transform"
    );
    assert_monotonic_ge(&ts["root"]);
    assert_child_boxes_nested(&ts["root"]);
    let root_ge = ts["root"]["geometricError"].as_f64().unwrap();
    // Tileset GE sits above the root so the root always renders when visible.
    let ts_ge = ts["geometricError"].as_f64().unwrap();
    assert!(
        ts_ge >= root_ge && root_ge > 0.0,
        "tileset GE {ts_ge} root GE {root_ge}"
    );
    let src_box = bounding_box_from_gltf_path(&glb).unwrap();
    let (smin, smax) = rusty_tiles::bbox::box_to_aabb(src_box);
    // A flat grid simplifies with ~zero surface error: GE must be the numeric
    // guard, not a spatial floor that would force refinement everywhere.
    assert!(
        root_ge < 0.05,
        "flat grid root GE {root_ge} should be near zero (no extent-based floor)"
    );

    let mut uris = Vec::new();
    collect_uris(&ts["root"], &mut uris);
    assert!(uris.len() > 1, "expected a split tree, got {uris:?}");
    let root_uri = ts["root"]["content"]["uri"].as_str().unwrap();
    assert!(ts["root"]["children"]
        .as_array()
        .is_some_and(|c| !c.is_empty()));

    let mut leaf_ge = Vec::new();
    collect_leaf_ge(&ts["root"], &mut leaf_ge);
    assert!(!leaf_ge.is_empty());
    assert!(leaf_ge.iter().all(|&g| g == 0.0));

    let mut union_min = [f64::INFINITY; 3];
    let mut union_max = [f64::NEG_INFINITY; 3];
    union_world_boxes(&ts["root"], mat_id(), &mut union_min, &mut union_max);
    for i in 0..3 {
        assert!(union_min[i] <= smin[i] + 1e-3);
        assert!(union_max[i] >= smax[i] - 1e-3);
    }

    for uri in &uris {
        let bytes = zip_bytes(&tz, uri);
        let j = glb_json(&bytes);
        let used = j["extensionsUsed"].as_array().unwrap();
        assert!(!used.iter().any(|v| v == "KHR_mesh_quantization"));
        assert!(used.iter().any(|v| v == "EXT_meshopt_compression"));
        if uri != root_uri {
            let tris = glb_index_count(&bytes) / 3;
            // Clip tessellation of the cut plane can add a handful of tris.
            assert!(tris <= 22_000, "{uri} has {tris} tris (leaf budget 20000)");
        }
    }
}

#[test]
fn texture_crop_shrinks_shared_atlas() {
    let tmp = tempfile::tempdir().unwrap();
    let mut img = RgbaImage::new(256, 256);
    for y in 0..256 {
        for x in 0..256 {
            let left = x < 128;
            img.put_pixel(
                x,
                y,
                Rgba([
                    if left { 220 } else { 20 },
                    ((x.wrapping_mul(13) + y.wrapping_mul(7)) % 256) as u8,
                    if left { 20 } else { 220 },
                    255,
                ]),
            );
        }
    }
    let jpeg = encode_jpeg(&img);

    // Two quads far apart on X, sharing one atlas (left vs right UVs).
    let glb_bytes = write_glb(&[TilePrimitive {
        positions: vec![
            [0.0, 0.0, 0.0],
            [1.0, 0.0, 0.0],
            [1.0, 1.0, 0.0],
            [0.0, 1.0, 0.0],
            [50.0, 0.0, 0.0],
            [51.0, 0.0, 0.0],
            [51.0, 1.0, 0.0],
            [50.0, 1.0, 0.0],
        ],
        normals: Vec::new(),
        uvs: vec![
            [0.0, 0.0],
            [0.5, 0.0],
            [0.5, 1.0],
            [0.0, 1.0],
            [0.5, 0.0],
            [1.0, 0.0],
            [1.0, 1.0],
            [0.5, 1.0],
        ],
        indices: vec![0, 1, 2, 0, 2, 3, 4, 5, 6, 4, 6, 7],
        jpeg: Some(jpeg),
    }])
    .unwrap();

    let glb = tmp.path().join("atlas.glb");
    fs::write(&glb, &glb_bytes).unwrap();
    assert!(glb_bytes.len() as u64 > 1024);

    let tz = tmp.path().join("atlas.3tz");
    let opts = MeshTo3tzOptions {
        max_triangles: 2,
        tile_size: 256,
        ..MeshTo3tzOptions::default()
    };
    mesh_to_3tz(&glb, &tz, &opts).unwrap();
    validate_3tz(&tz).unwrap();

    let ts = tileset_json(&tz);
    assert_eq!(ts["root"]["refine"], "REPLACE");
    let atlas_root_ge = ts["root"]["geometricError"].as_f64().unwrap();
    // Disconnected source charts remain separate in the parent. The gap
    // between them no longer consumes texture resolution; texel GE stays positive.
    assert!(
        atlas_root_ge > 0.1,
        "textured parent GE {atlas_root_ge} too small to refine in Cesium"
    );
    let mut uris = Vec::new();
    collect_uris(&ts["root"], &mut uris);
    assert!(uris.len() >= 2, "expected split, got {uris:?}");

    for uri in &uris {
        if uri == ts["root"]["content"]["uri"].as_str().unwrap() {
            // Packed parent atlas (one image).
            let bytes = zip_bytes(&tz, uri);
            let imgs = glb_json(&bytes)["images"]
                .as_array()
                .cloned()
                .unwrap_or_default();
            assert_eq!(imgs.len(), 1, "parent should have one packed atlas");
            continue;
        }
        let bytes = zip_bytes(&tz, uri);
        let j = glb_json(&bytes);
        assert!(!j["images"].as_array().unwrap().is_empty());
        let mime = j["images"][0]["mimeType"].as_str().unwrap_or("");
        assert_eq!(mime, "image/png", "{uri} mime {mime}");
        let jpeg_len = glb_jpeg_len(&bytes);
        assert!(jpeg_len > 32, "leaf image too small: {jpeg_len}");
    }
}

#[test]
fn mesh_to_3tz_is_byte_identical_across_runs() {
    let tmp = tempfile::tempdir().unwrap();
    let glb = tmp.path().join("grid.glb");
    fs::write(&glb, grid_glb(40, 40)).unwrap();
    let a = tmp.path().join("a.3tz");
    let b = tmp.path().join("b.3tz");
    let opts = MeshTo3tzOptions {
        max_triangles: 200,
        ..MeshTo3tzOptions::default()
    };
    mesh_to_3tz(&glb, &a, &opts).unwrap();
    mesh_to_3tz(&glb, &b, &opts).unwrap();
    assert_eq!(
        fs::read(&a).unwrap(),
        fs::read(&b).unwrap(),
        "mesh-to-3tz must be byte-identical across runs"
    );
}

#[test]
fn no_meshopt_writes_float32_tiles() {
    let tmp = tempfile::tempdir().unwrap();
    let glb = tmp.path().join("grid.glb");
    fs::write(&glb, grid_glb(40, 40)).unwrap();
    let tz = tmp.path().join("grid.3tz");
    mesh_to_3tz(
        &glb,
        &tz,
        &MeshTo3tzOptions {
            max_triangles: 200,
            meshopt: false,
            ..MeshTo3tzOptions::default()
        },
    )
    .unwrap();
    validate_3tz(&tz).unwrap();
    let ts = tileset_json(&tz);
    assert_eq!(count_transforms(&ts["root"]), 0);
    let mut uris = Vec::new();
    collect_uris(&ts["root"], &mut uris);
    assert!(uris.len() > 1);
    for uri in &uris {
        let j = glb_json(&zip_bytes(&tz, uri));
        assert!(
            j.get("extensionsRequired").is_none(),
            "{uri} has {}",
            j["extensionsRequired"]
        );
    }
}

#[test]
fn parent_ge_tracks_bump_height_not_diagonal() {
    let tmp = tempfile::tempdir().unwrap();
    let glb = tmp.path().join("bump.glb");
    fs::write(
        &glb,
        write_glb(std::slice::from_ref(&grid_prim(40, 40, |x, y| {
            if (x / 2 + y / 2) % 2 == 0 {
                4.0
            } else {
                0.0
            }
        })))
        .unwrap(),
    )
    .unwrap();

    let tz = tmp.path().join("bump.3tz");
    mesh_to_3tz(
        &glb,
        &tz,
        &MeshTo3tzOptions {
            max_triangles: 32,
            ..MeshTo3tzOptions::default()
        },
    )
    .unwrap();
    validate_3tz(&tz).unwrap();

    let ts = tileset_json(&tz);
    assert_monotonic_ge(&ts["root"]);
    let ge = ts["root"]["geometricError"].as_f64().unwrap();
    let b = ts["root"]["boundingVolume"]["box"].as_array().unwrap();
    let hx = b[3].as_f64().unwrap();
    let hy = b[7].as_f64().unwrap();
    let hz = b[11].as_f64().unwrap();
    let diag = (hx * hx + hy * hy + hz * hz).sqrt() * 2.0;
    assert!(
        ge > 0.5,
        "checkerboard height 4 should leave metres of error, got {ge}"
    );
    assert!(
        ge < diag * 0.35,
        "GE {ge} looks like bbox diagonal {diag}, not surface error"
    );
}

fn grid_prim(nx: u32, ny: u32, z_at: impl Fn(u32, u32) -> f32) -> TilePrimitive {
    let mut positions = Vec::new();
    for y in 0..=ny {
        for x in 0..=nx {
            positions.push([x as f32, y as f32, z_at(x, y)]);
        }
    }
    let mut indices = Vec::new();
    let w = nx + 1;
    for y in 0..ny {
        for x in 0..nx {
            let i = y * w + x;
            indices.extend_from_slice(&[i, i + 1, i + w + 1, i, i + w + 1, i + w]);
        }
    }
    TilePrimitive {
        positions,
        normals: Vec::new(),
        uvs: Vec::new(),
        indices,
        jpeg: None,
    }
}

fn geog_verts() -> Vec<[f32; 3]> {
    vec![
        [30.0000, 120.0, 20.0010],
        [30.0010, 140.0, 20.0000],
        [30.0005, 130.0, 20.0005],
    ]
}

#[test]
fn bake_geographic_skips_local_metres() {
    let tmp = tempfile::tempdir().unwrap();
    let glb = tmp.path().join("triangle.glb");
    fs::write(&glb, triangle_glb()).unwrap();
    let mut scene = mesh::load(&glb).unwrap();
    assert!(mesh::bake_geographic(&mut scene, None).is_none());
}

#[test]
fn bake_geographic_enu_metres_and_origin() {
    let tmp = tempfile::tempdir().unwrap();
    let glb = tmp.path().join("geog.glb");
    let src = geographic_glb(&geog_verts());
    fs::write(&glb, &src).unwrap();

    let mut scene = mesh::load(&glb).unwrap();
    let baked = mesh::bake_geographic(&mut scene, None).expect("expected geographic bake");
    assert!((baked.origin.lon_deg - 30.0005).abs() < 0.001);
    assert!((baked.origin.lat_deg - (-20.0005)).abs() < 0.001);
    assert!((baked.origin.height_m - 130.0).abs() < 1.0);
    let bb = baked.bbox_wgs84;
    assert!(
        bb[0] <= 30.000 + 1e-3 && bb[2] >= 30.001 - 1e-3,
        "bbox west/east {bb:?}"
    );
    assert!(
        bb[1] <= -20.001 + 1e-3 && bb[3] >= -20.000 - 1e-3,
        "bbox south/north {bb:?}"
    );

    let p = scene.vertices[0].pos;
    assert!(
        p[0].abs() < 200.0 && p[1].abs() < 50.0 && p[2].abs() < 200.0,
        "baked vertex still not ENU metres: {p:?}"
    );
    let span = (p[0] * p[0] + p[1] * p[1] + p[2] * p[2]).sqrt();
    assert!(
        span > 5.0,
        "expected ~10–100 m offset from centroid, got {span}"
    );

    assert_eq!(
        fs::read(&glb).unwrap(),
        src,
        "source GLB must stay geographic"
    );
}

#[test]
fn mesh_to_3tz_bakes_geographic_and_places_on_globe() {
    let tmp = tempfile::tempdir().unwrap();
    let glb = tmp.path().join("geog.glb");
    let src = geographic_glb(&geog_verts());
    fs::write(&glb, &src).unwrap();

    let tz = tmp.path().join("geog.3tz");
    mesh_to_3tz(&glb, &tz, &MeshTo3tzOptions::default()).unwrap();
    validate_3tz(&tz).unwrap();
    assert_eq!(fs::read(&glb).unwrap(), src);

    let ts = tileset_json(&tz);
    assert_eq!(ts["root"]["refine"], "REPLACE");
    let xf = ts["root"]["transform"]
        .as_array()
        .expect("geog bake sets ENU→ECEF root.transform");
    assert_eq!(
        count_transforms(&ts["root"]),
        1,
        "only root.transform, not per-tile"
    );
    let tx = xf[12].as_f64().unwrap();
    let ty = xf[13].as_f64().unwrap();
    let tz_ecef = xf[14].as_f64().unwrap();
    let r = (tx * tx + ty * ty + tz_ecef * tz_ecef).sqrt();
    assert!(
        (6.0e6..6.5e6).contains(&r),
        "root translation should be ECEF metres, got {r}"
    );

    let b = ts["root"]["boundingVolume"]["box"].as_array().unwrap();
    let hx = b[3].as_f64().unwrap().abs();
    let hy = b[7].as_f64().unwrap().abs();
    let hz = b[11].as_f64().unwrap().abs();
    let half = hx.max(hy).max(hz);
    assert!(
        half > 5.0 && half < 200.0,
        "local box should be tens of metres after bake, got {half}"
    );

    let names = list_zip_names(&tz).unwrap();
    assert!(
        !names.iter().any(|n| n.ends_with("geog.glb")),
        "must not wrap the geographic source: {names:?}"
    );
}

#[test]
fn bake_geographic_prefer_pin() {
    let tmp = tempfile::tempdir().unwrap();
    let glb = tmp.path().join("geog.glb");
    fs::write(&glb, geographic_glb(&geog_verts())).unwrap();
    let mut scene = mesh::load(&glb).unwrap();
    let pin = Cartographic::new(30.01, -20.01, 200.0);
    let baked = mesh::bake_geographic(&mut scene, Some(pin)).unwrap();
    assert!((baked.origin.lon_deg - 30.01).abs() < 1e-12);
    assert!((baked.origin.lat_deg - (-20.01)).abs() < 1e-12);
    assert!((baked.origin.height_m - 200.0).abs() < 1e-12);
}

#[test]
fn bake_mercator_offset_keeps_local_precision() {
    let tmp = tempfile::tempdir().unwrap();
    let glb = tmp.path().join("offset.glb");
    let verts = vec![
        [200.0, 10.0, -80.0],
        [300.0, 40.0, -20.0],
        [250.0, 25.0, -50.0],
    ];
    fs::write(&glb, geographic_glb(&verts)).unwrap();
    let mut scene = mesh::load(&glb).unwrap();
    assert!(mesh::bake_geographic(&mut scene, None).is_none());

    let off = SourceOffset {
        easting: 3_000_000.0,
        northing: -2_000_000.0,
        height: 100.0,
    };
    let baked = mesh::bake_to_enu(
        &mut scene,
        &mesh::BakeToEnu {
            prefer: None,
            crs: SourceCrs::WebMercator,
            offset: Some(off),
        },
    )
    .expect("offset mercator bake");
    let pin = rusty_tiles::georef::mercator_shift_origin(off);
    assert!((baked.origin.lon_deg - pin.lon_deg).abs() < 1e-12);
    assert!((baked.origin.lat_deg - pin.lat_deg).abs() < 1e-12);
    assert!((baked.origin.height_m - 100.0).abs() < 1e-12);
    let p = scene.vertices[2].pos;
    assert!(
        (p[0] - 238.27213).abs() < 0.001
            && (p[1] - 24.995375).abs() < 0.001
            && (p[2] + 47.36318).abs() < 0.001,
        "shifted Web Mercator must match independent PROJ ECEF/ENU reference, got {p:?}"
    );

    let tz = tmp.path().join("offset.3tz");
    mesh_to_3tz(
        &glb,
        &tz,
        &MeshTo3tzOptions {
            source_crs: SourceCrs::WebMercator,
            source_offset: Some(off),
            meshopt: false,
            ..MeshTo3tzOptions::default()
        },
    )
    .unwrap();
    validate_3tz(&tz).unwrap();
    let ts = tileset_json(&tz);
    let xf = ts["root"]["transform"].as_array().unwrap();
    let r = {
        let tx = xf[12].as_f64().unwrap();
        let ty = xf[13].as_f64().unwrap();
        let tz = xf[14].as_f64().unwrap();
        (tx * tx + ty * ty + tz * tz).sqrt()
    };
    assert!((6.0e6..6.5e6).contains(&r), "ECEF r {r}");
    assert_eq!(count_transforms(&ts["root"]), 1);

    let mut uris = Vec::new();
    collect_uris(&ts["root"], &mut uris);
    for uri in &uris {
        let j = glb_json(&zip_bytes(&tz, uri));
        assert!(
            j.get("extensionsRequired").is_none(),
            "{uri} should be float32 without meshopt"
        );
        let mins = j["accessors"][0]["min"].as_array().unwrap();
        let x = mins[0].as_f64().unwrap();
        assert!(
            (180.0..300.0).contains(&x),
            "GLB stays in local metres, POSITION.min.x={x}"
        );
    }
}
