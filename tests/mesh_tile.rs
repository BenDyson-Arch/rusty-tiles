use std::fs;
use std::io::Read;
use std::path::Path;

use image::{Rgba, RgbaImage};
use serde_json::Value;
use tinyowl_tiles::bbox::bounding_box_from_gltf_path;
use tinyowl_tiles::fixtures::triangle_glb;
use tinyowl_tiles::glb_write::{write_glb, TilePrimitive};
use tinyowl_tiles::mesh;
use tinyowl_tiles::pack::list_zip_names;
use tinyowl_tiles::tile::{mesh_to_3tz, MeshTo3tzOptions};
use tinyowl_tiles::tileset::{glb_to_3tz, CreateTilesetOptions};
use tinyowl_tiles::{validate_3tz, write_glb_compressed};

fn zip_bytes(tz: &Path, name: &str) -> Vec<u8> {
    let mut z = zip::ZipArchive::new(fs::File::open(tz).unwrap()).unwrap();
    let mut f = z.by_name(name).unwrap();
    let mut buf = Vec::new();
    f.read_to_end(&mut buf).unwrap();
    buf
}

fn tileset_json(tz: &Path) -> Value {
    serde_json::from_slice(&zip_bytes(tz, "tileset.json")).unwrap()
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

fn assert_monotonic_ge(tile: &Value) {
    let ge = tile["geometricError"].as_f64().unwrap();
    let Some(kids) = tile["children"].as_array() else {
        return;
    };
    if kids.is_empty() {
        return;
    }
    assert!(
        tile.pointer("/content/uri")
            .and_then(|v| v.as_str())
            .is_some(),
        "internal tile missing content.uri: {tile}"
    );
    for k in kids {
        let cg = k["geometricError"].as_f64().unwrap();
        assert!(ge + 1e-12 >= cg, "parent GE {ge} < child GE {cg}");
        assert_monotonic_ge(k);
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
    mesh_to_3tz(&glb, &tiled, &MeshTo3tzOptions::default()).unwrap();

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
    assert_monotonic_ge(&ts["root"]);
    let root_ge = ts["root"]["geometricError"].as_f64().unwrap();
    assert_eq!(ts["geometricError"].as_f64(), Some(root_ge));
    assert!(
        root_ge < 1.0,
        "flat grid parent GE {root_ge} should be ~0 m, not bbox diagonal"
    );

    let mut uris = Vec::new();
    collect_uris(&ts["root"], &mut uris);
    assert!(uris.len() > 1, "expected a split tree, got {uris:?}");
    assert!(
        uris.iter().any(|u| u.contains("/p")),
        "expected parent tiles, got {uris:?}"
    );

    let mut leaf_ge = Vec::new();
    collect_leaf_ge(&ts["root"], &mut leaf_ge);
    assert!(!leaf_ge.is_empty());
    assert!(leaf_ge.iter().all(|&g| g == 0.0));

    let src_box = bounding_box_from_gltf_path(&glb).unwrap();
    let mut union_min = [f64::INFINITY; 3];
    let mut union_max = [f64::NEG_INFINITY; 3];
    union_world_boxes(&ts["root"], mat_id(), &mut union_min, &mut union_max);
    let (smin, smax) = tinyowl_tiles::bbox::box_to_aabb(src_box);
    for i in 0..3 {
        assert!(union_min[i] <= smin[i] + 1e-3);
        assert!(union_max[i] >= smax[i] - 1e-3);
    }

    for uri in &uris {
        let bytes = zip_bytes(&tz, uri);
        let j = glb_json(&bytes);
        let used = j["extensionsUsed"].as_array().unwrap();
        assert!(used.iter().any(|v| v == "KHR_mesh_quantization"));
        assert!(used.iter().any(|v| v == "EXT_meshopt_compression"));
        if !uri.contains("/p") {
            let tris = glb_index_count(&bytes) / 3;
            assert!(tris <= 20_000, "{uri} has {tris} tris");
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
    let source_jpeg_len = jpeg.len();

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
    let mut uris = Vec::new();
    collect_uris(&ts["root"], &mut uris);
    assert!(uris.len() >= 2, "expected split, got {uris:?}");

    for uri in &uris {
        let bytes = zip_bytes(&tz, uri);
        assert!(!glb_json(&bytes)["images"].as_array().unwrap().is_empty());
        let jpeg_len = glb_jpeg_len(&bytes);
        assert!(
            jpeg_len < source_jpeg_len,
            "leaf jpeg {jpeg_len} >= source {source_jpeg_len}"
        );
    }
}

#[test]
fn compressed_glb_smaller_than_uncompressed() {
    let prim = grid_prim(80, 80, |_, _| 0.0);
    let raw = write_glb(std::slice::from_ref(&prim)).unwrap();
    let packed = write_glb_compressed(std::slice::from_ref(&prim)).unwrap();
    assert!(
        packed.len() < raw.len(),
        "compressed {} >= uncompressed {}",
        packed.len(),
        raw.len()
    );
    let j = glb_json(&packed);
    assert!(j["extensionsRequired"]
        .as_array()
        .unwrap()
        .iter()
        .any(|v| v == "EXT_meshopt_compression"));
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
