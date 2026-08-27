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
use tinyowl_tiles::validate_3tz;

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
    if tile.get("content").is_some() {
        out.push(tile["geometricError"].as_f64().unwrap());
    }
    if let Some(kids) = tile["children"].as_array() {
        for k in kids {
            collect_leaf_ge(k, out);
        }
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
    let mut positions = Vec::new();
    let mut indices = Vec::new();
    for y in 0..ny {
        for x in 0..nx {
            let i = positions.len() as u32;
            let fx = x as f32;
            let fy = y as f32;
            positions.push([fx, fy, 0.0]);
            positions.push([fx + 1.0, fy, 0.0]);
            positions.push([fx + 1.0, fy + 1.0, 0.0]);
            positions.push([fx, fy + 1.0, 0.0]);
            indices.extend_from_slice(&[i, i + 1, i + 2, i, i + 2, i + 3]);
        }
    }
    write_glb(&[TilePrimitive {
        positions,
        normals: Vec::new(),
        uvs: Vec::new(),
        indices,
        jpeg: None,
    }])
    .unwrap()
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

    let mut uris = Vec::new();
    collect_uris(&ts["root"], &mut uris);
    assert!(uris.len() > 1, "expected a split tree, got {uris:?}");

    let mut leaf_ge = Vec::new();
    collect_leaf_ge(&ts["root"], &mut leaf_ge);
    assert!(leaf_ge.iter().all(|&g| g == 0.0));

    let src_box = bounding_box_from_gltf_path(&glb).unwrap();
    let mut union_min = [f64::INFINITY; 3];
    let mut union_max = [f64::NEG_INFINITY; 3];
    for uri in &uris {
        let bytes = zip_bytes(&tz, uri);
        let leaf_path = tmp.path().join(uri.replace('/', "_"));
        fs::write(&leaf_path, &bytes).unwrap();
        let leaf = mesh::load(&leaf_path).unwrap();
        assert!(
            leaf.triangle_count() <= 20_000,
            "{} has {} tris",
            uri,
            leaf.triangle_count()
        );
        let b = bounding_box_from_gltf_path(&leaf_path).unwrap();
        let (mn, mx) = tinyowl_tiles::bbox::box_to_aabb(b);
        let (u0, u1) = tinyowl_tiles::bbox::union_aabb(union_min, union_max, mn, mx);
        union_min = u0;
        union_max = u1;
    }
    let (smin, smax) = tinyowl_tiles::bbox::box_to_aabb(src_box);
    for i in 0..3 {
        assert!(union_min[i] <= smin[i] + 1e-3);
        assert!(union_max[i] >= smax[i] - 1e-3);
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
        max_bytes: 1024,
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
        let leaf_path = tmp.path().join(uri.replace('/', "_"));
        fs::write(&leaf_path, &bytes).unwrap();
        let leaf = mesh::load(&leaf_path).unwrap();
        assert!(!leaf.images.is_empty(), "{uri} missing embedded image");
        assert!(
            leaf.images[0].bytes.len() < source_jpeg_len,
            "leaf jpeg {} >= source {}",
            leaf.images[0].bytes.len(),
            source_jpeg_len
        );
        for v in &leaf.vertices {
            assert!(v.uv[0] >= 0.0 && v.uv[0] <= 1.0);
            assert!(v.uv[1] >= 0.0 && v.uv[1] <= 1.0);
        }
    }
}
