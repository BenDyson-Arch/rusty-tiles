use rusty_tiles::{
    glb_write::{write_glb, TilePrimitive},
    mesh_to_3tz,
    pack::{pack_named_files, PackOptions},
    validate_3tz, MeshTo3tzOptions,
};
use serde_json::{json, Value};
use std::{fs, io::Read, path::Path};

fn entry(path: &Path, name: &str) -> Vec<u8> {
    let mut z = zip::ZipArchive::new(fs::File::open(path).unwrap()).unwrap();
    let mut out = Vec::new();
    z.by_name(name).unwrap().read_to_end(&mut out).unwrap();
    out
}
fn expand(bytes: &[u8]) -> Vec<u8> {
    let glb = gltf::Glb::from_slice(bytes).unwrap();
    let mut d: Value = serde_json::from_slice(&glb.json).unwrap();
    let raw = glb.bin.unwrap();
    let mut bin = vec![0u8; d["buffers"][1]["byteLength"].as_u64().unwrap() as usize];
    for view in d["bufferViews"].as_array_mut().unwrap() {
        if let Some(e) = view["extensions"].get("EXT_meshopt_compression") {
            let start = e["byteOffset"].as_u64().unwrap() as usize;
            let n = e["byteLength"].as_u64().unwrap() as usize;
            let count = e["count"].as_u64().unwrap() as usize;
            let stride = e["byteStride"].as_u64().unwrap() as usize;
            let dst = view["byteOffset"].as_u64().unwrap_or(0) as usize;
            let mut decoded = vec![0u32; (count * stride).div_ceil(4)];
            let code = unsafe {
                if e["mode"] == "TRIANGLES" {
                    meshopt::ffi::meshopt_decodeIndexBuffer(
                        decoded.as_mut_ptr().cast(),
                        count,
                        stride,
                        raw[start..start + n].as_ptr(),
                        n,
                    )
                } else {
                    meshopt::ffi::meshopt_decodeVertexBuffer(
                        decoded.as_mut_ptr().cast(),
                        count,
                        stride,
                        raw[start..start + n].as_ptr(),
                        n,
                    )
                }
            };
            assert_eq!(code, 0);
            bin[dst..dst + count * stride]
                .copy_from_slice(&bytemuck::cast_slice(&decoded)[..count * stride]);
        } else {
            let start = view["byteOffset"].as_u64().unwrap_or(0) as usize;
            let n = view["byteLength"].as_u64().unwrap() as usize;
            view["byteOffset"] = json!(bin.len());
            bin.extend_from_slice(&raw[start..start + n]);
        }
        view["buffer"] = json!(0);
        view.as_object_mut().unwrap().remove("extensions");
    }
    d["buffers"] = json!([{"byteLength":bin.len()}]);
    d.as_object_mut().unwrap().remove("extensionsUsed");
    d.as_object_mut().unwrap().remove("extensionsRequired");
    let json = serde_json::to_vec(&d).unwrap();
    let mut out = Vec::new();
    gltf::Glb {
        header: gltf::binary::Header {
            magic: *b"glTF",
            version: 2,
            length: 0,
        },
        json: std::borrow::Cow::Owned(json),
        bin: Some(std::borrow::Cow::Owned(bin)),
    }
    .to_writer(&mut out)
    .unwrap();
    out
}
fn leaves(tile: &Value, out: &mut Vec<String>) {
    if let Some(c) = tile["children"].as_array() {
        for child in c {
            leaves(child, out);
        }
    } else {
        out.push(tile["content"]["uri"].as_str().unwrap().into());
    }
}

#[test]
fn leaves_preserve_triangles_float_attributes_rgba_and_materials() {
    let dir = tempfile::tempdir().unwrap();
    let input = dir.path().join("source.glb");
    let output = dir.path().join("result.3tz");
    let source = image::RgbaImage::from_fn(64, 64, |x, y| {
        image::Rgba([x as u8 * 3, y as u8 * 3, (x ^ y) as u8, ((x + y) * 2) as u8])
    });
    let mut png = Vec::new();
    image::DynamicImage::ImageRgba8(source.clone())
        .write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
        .unwrap();
    let pos = vec![
        [0.12345679, 0.0, 0.0],
        [1.2345679, 0.0, 0.0],
        [0.12345679, 1.2345679, 0.0],
        [1.2345679, 1.2345679, 0.0],
    ];
    let uv = vec![
        [0.125, 0.125],
        [0.875, 0.125],
        [0.125, 0.875],
        [0.875, 0.875],
    ];
    fs::write(
        &input,
        write_glb(&[TilePrimitive {
            positions: pos.clone(),
            normals: vec![[0.0, 0.0, 1.0]; 4],
            uvs: uv.clone(),
            indices: vec![0, 1, 2, 1, 3, 2],
            jpeg: Some(png),
        }])
        .unwrap(),
    )
    .unwrap();
    mesh_to_3tz(
        &input,
        &output,
        &MeshTo3tzOptions {
            max_triangles: 1,
            max_bytes: 0,
            tile_size: 128,
            ..Default::default()
        },
    )
    .unwrap();
    validate_3tz(&output).unwrap();
    if let Ok(path) = std::env::var("RUSTY_TILES_VIEWER_FIXTURE") {
        fs::create_dir_all(&path).unwrap();
        fs::copy(&input, Path::new(&path).join("source.glb")).unwrap();
        fs::copy(&output, Path::new(&path).join("lossless.3tz")).unwrap();
    }
    let ts: Value = serde_json::from_slice(&entry(&output, "tileset.json")).unwrap();
    let mut uris = Vec::new();
    leaves(&ts["root"], &mut uris);
    let mut count = 0;
    let mut triangles = Vec::new();
    for uri in uris {
        let expanded = expand(&entry(&output, &uri));
        let (doc, buffers, images) = gltf::import_slice(&expanded).unwrap();
        for prim in doc.meshes().flat_map(|m| m.primitives()) {
            assert!(prim.material().double_sided());
            assert_eq!(
                prim.material().pbr_metallic_roughness().metallic_factor(),
                0.0
            );
            let r = prim.reader(|b| Some(&buffers[b.index()]));
            let p: Vec<_> = r.read_positions().unwrap().collect();
            let n: Vec<_> = r.read_normals().unwrap().collect();
            let t: Vec<_> = r.read_tex_coords(0).unwrap().into_f32().collect();
            let indices: Vec<_> = r.read_indices().unwrap().into_u32().collect();
            count += indices.len() / 3;
            for tri in indices.as_chunks::<3>().0 {
                let mut source_ids: Vec<_> = tri
                    .iter()
                    .map(|&i| {
                        pos.iter()
                            .position(|v| *v == p[i as usize])
                            .expect("position changed")
                    })
                    .collect();
                let first = source_ids
                    .iter()
                    .enumerate()
                    .min_by_key(|(_, v)| **v)
                    .unwrap()
                    .0;
                source_ids.rotate_left(first);
                triangles.push(source_ids);
            }
            let img = &images[prim
                .material()
                .pbr_metallic_roughness()
                .base_color_texture()
                .unwrap()
                .texture()
                .source()
                .index()];
            assert_eq!(img.format, gltf::image::Format::R8G8B8A8);
            for (i, v) in p.iter().enumerate() {
                let original = pos.iter().position(|p| p == v).unwrap();
                assert_eq!(n[i], [0.0, 0.0, 1.0]);
                // Compare texels around every remapped vertex, including alpha.
                let sx = (uv[original][0] * 64.0).round() as i32;
                let sy = (uv[original][1] * 64.0).round() as i32;
                let dx = (t[i][0] * img.width as f32).round() as i32;
                let dy = (t[i][1] * img.height as f32).round() as i32;
                for oy in -1..=1 {
                    for ox in -1..=1 {
                        let dst = ((dy + oy) as u32 * img.width + (dx + ox) as u32) as usize * 4;
                        assert_eq!(
                            &img.pixels[dst..dst + 4],
                            &source.get_pixel((sx + ox) as u32, (sy + oy) as u32).0
                        );
                    }
                }
            }
        }
    }
    triangles.sort();
    assert_eq!(triangles, vec![vec![0, 1, 2], vec![1, 3, 2]]);
    assert_eq!(count, 2);
}
#[test]
fn invalid_input_and_failed_pack_leave_existing_output_intact() {
    let dir = tempfile::tempdir().unwrap();
    let input = dir.path().join("bad.glb");
    let output = dir.path().join("keep.3tz");
    fs::write(&input, b"glTF").unwrap();
    fs::write(&output, b"existing").unwrap();
    assert!(mesh_to_3tz(
        &input,
        &output,
        &MeshTo3tzOptions {
            force: true,
            ..Default::default()
        }
    )
    .is_err());
    assert_eq!(fs::read(&output).unwrap(), b"existing");
    assert!(pack_named_files(
        &[
            ("tileset.json".into(), input.clone()),
            ("../escape".into(), input)
        ],
        &output,
        &PackOptions { force: true }
    )
    .is_err());
    assert_eq!(fs::read(output).unwrap(), b"existing");
}
#[test]
fn zero_budgets_and_invalid_texture_sizes_are_errors() {
    let dir = tempfile::tempdir().unwrap();
    for opts in [
        MeshTo3tzOptions {
            max_triangles: 0,
            ..Default::default()
        },
        MeshTo3tzOptions {
            tile_size: 0,
            ..Default::default()
        },
        MeshTo3tzOptions {
            tile_size: 1000,
            ..Default::default()
        },
        MeshTo3tzOptions {
            max_texel_density: f64::NAN,
            ..Default::default()
        },
    ] {
        assert!(mesh_to_3tz(
            &dir.path().join("missing.glb"),
            &dir.path().join("out.3tz"),
            &opts
        )
        .is_err());
    }
}

#[test]
fn zip64_member_counts_have_complete_random_access_index() {
    let dir = tempfile::tempdir().unwrap();
    let source = dir.path().join("tileset.json");
    fs::write(&source, b"{}").unwrap();
    let mut files = vec![("tileset.json".into(), source.clone())];
    files.extend((0..65535).map(|i| (format!("t/{i}.json"), source.clone())));
    let output = dir.path().join("large-index.3tz");
    pack_named_files(&files, &output, &PackOptions::default()).unwrap();
    validate_3tz(&output).unwrap();
    let zip = zip::ZipArchive::new(fs::File::open(output).unwrap()).unwrap();
    assert_eq!(zip.len(), 65537);
}

fn rewrite_json(bytes: &[u8], edit: impl FnOnce(&mut Value)) -> Vec<u8> {
    let glb = gltf::Glb::from_slice(bytes).unwrap();
    let mut doc: Value = serde_json::from_slice(&glb.json).unwrap();
    edit(&mut doc);
    let mut out = Vec::new();
    gltf::Glb {
        header: glb.header,
        json: std::borrow::Cow::Owned(serde_json::to_vec(&doc).unwrap()),
        bin: glb.bin,
    }
    .to_writer(&mut out)
    .unwrap();
    out
}
#[test]
fn active_scene_and_mirrored_nonuniform_normal_transform() {
    let dir = tempfile::tempdir().unwrap();
    let input = dir.path().join("scene.glb");
    let n = std::f32::consts::FRAC_1_SQRT_2;
    let bytes = write_glb(&[TilePrimitive {
        positions: vec![[0., 0., 0.], [1., 0., 0.], [0., 1., 0.]],
        normals: vec![[n, n, 0.]; 3],
        indices: vec![0, 1, 2],
        ..Default::default()
    }])
    .unwrap();
    let bytes = rewrite_json(&bytes, |d| {
        d["nodes"] = json!([{"mesh":0,"translation":[1000,0,0]},{"mesh":0,"scale":[-2,1,1]}]);
        d["scenes"] = json!([{"nodes":[0]},{"nodes":[1]}]);
        d["scene"] = json!(1);
    });
    fs::write(&input, bytes).unwrap();
    let scene = rusty_tiles::mesh::load(&input).unwrap();
    assert_eq!(scene.triangles.len(), 1);
    assert_eq!(scene.triangles[0].verts, [0, 2, 1]);
    assert!((scene.vertices[0].nrm[0] + 1.0f32 / 5.0f32.sqrt()).abs() < 1e-6);
    assert!((scene.vertices[0].nrm[1] - 2.0f32 / 5.0f32.sqrt()).abs() < 1e-6);
    let (min, max) = rusty_tiles::bbox::box_to_aabb(
        rusty_tiles::bbox::bounding_box_from_gltf_path(&input).unwrap(),
    );
    assert_eq!(min[0], -2.0);
    assert_eq!(max[0], 0.0);
}
#[test]
fn unsupported_attributes_fail_without_publishing_partial_geometry() {
    let dir = tempfile::tempdir().unwrap();
    let input = dir.path().join("colours.glb");
    let output = dir.path().join("out.3tz");
    let bytes = write_glb(&[TilePrimitive {
        positions: vec![[0., 0., 0.], [1., 0., 0.], [0., 1., 0.]],
        indices: vec![0, 1, 2],
        ..Default::default()
    }])
    .unwrap();
    let bytes = rewrite_json(&bytes, |d| {
        let pos = d["meshes"][0]["primitives"][0]["attributes"]["POSITION"].clone();
        d["meshes"][0]["primitives"][0]["attributes"]["COLOR_0"] = pos;
    });
    fs::write(&input, bytes).unwrap();
    assert!(mesh_to_3tz(
        &input,
        &output,
        &MeshTo3tzOptions {
            max_bytes: 0,
            ..Default::default()
        }
    )
    .is_err());
    assert!(!output.exists());
}

#[test]
fn crs_projection_retains_authored_normal_direction() {
    use rusty_tiles::georef::{Cartographic, CrsKind, EnuFrame};
    let f = EnuFrame::new(Cartographic::new(151.0, -34.0, 120.0));
    for n in [[1., 0., 0.], [0., 1., 0.], [0., 0., 1.]] {
        let actual = f.normal_to_enu_yup([151., 120., 34.], n, CrsKind::Geographic);
        for i in 0..3 {
            assert!((actual[i] - n[i]).abs() < 1e-6);
        }
    }
    let n = f.normal_to_enu_yup([151., 120., 34.], [1., 1., 0.], CrsKind::Geographic);
    assert!(n[1] > 0.9999 && n[0] > 0.0 && n[0] < 0.0001);
    assert_eq!(
        f.normal_to_enu_yup([151., 120., 34.], [0.; 3], CrsKind::Geographic),
        [0.; 3]
    );
}

/// Audit a real, already-generated local-coordinate model without decoding its
/// textures. The triangle multiset includes cyclic winding, not just vertices.
#[test]
#[ignore = "set RUSTY_TILES_FIDELITY_SOURCE and RUSTY_TILES_FIDELITY_ARCHIVE to matching local-coordinate files"]
fn full_model_leaf_triangle_audit() {
    use std::hash::{Hash, Hasher};
    let signature = |points: [[f32; 3]; 3]| {
        let mut bits = points.map(|p| p.map(f32::to_bits));
        let start = (0..3).min_by_key(|&i| bits[i]).unwrap();
        bits.rotate_left(start);
        let mut hash = std::collections::hash_map::DefaultHasher::new();
        bits.hash(&mut hash);
        hash.finish()
    };
    let source = std::env::var("RUSTY_TILES_FIDELITY_SOURCE").expect("source path required");
    let archive = std::env::var("RUSTY_TILES_FIDELITY_ARCHIVE").expect("archive path required");
    let mut scene = rusty_tiles::mesh::load(Path::new(&source)).unwrap();
    if let Ok(offset) = std::env::var("RUSTY_TILES_FIDELITY_OFFSET") {
        let offset =
            rusty_tiles::georef::parse_metashape_offset(&fs::read_to_string(offset).unwrap())
                .unwrap();
        rusty_tiles::mesh::bake_to_enu(
            &mut scene,
            &rusty_tiles::mesh::BakeToEnu {
                crs: rusty_tiles::georef::SourceCrs::WebMercator,
                offset: Some(offset),
                prefer: None,
            },
        )
        .expect("projected source");
    }
    let source_has_normals = scene.vertices.iter().any(|v| v.nrm != [0.; 3]);
    let mut expected: Vec<_> = scene
        .triangles
        .iter()
        .map(|t| signature(t.verts.map(|v| scene.vertices[v as usize].pos)))
        .collect();
    drop(scene);
    let mut z = zip::ZipArchive::new(fs::File::open(archive).unwrap()).unwrap();
    let manifest: Value = serde_json::from_reader(z.by_name("tileset.json").unwrap()).unwrap();
    let mut uris = Vec::new();
    leaves(&manifest["root"], &mut uris);
    let mut actual = Vec::with_capacity(expected.len());
    for uri in &uris {
        let mut bytes = Vec::new();
        z.by_name(uri).unwrap().read_to_end(&mut bytes).unwrap();
        let expanded = expand(&bytes);
        let gltf = gltf::Gltf::from_slice_without_validation(&expanded).unwrap();
        for primitive in gltf.document.meshes().flat_map(|m| m.primitives()) {
            let reader = primitive.reader(|buffer| {
                assert_eq!(buffer.index(), 0);
                gltf.blob.as_deref()
            });
            if !source_has_normals {
                assert!(
                    reader.read_normals().is_none(),
                    "serializer introduced normals absent from the source"
                );
            }
            let positions: Vec<_> = reader.read_positions().unwrap().collect();
            let indices: Vec<_> = reader.read_indices().unwrap().into_u32().collect();
            for t in indices.as_chunks::<3>().0 {
                actual.push(signature([
                    positions[t[0] as usize],
                    positions[t[1] as usize],
                    positions[t[2] as usize],
                ]));
            }
        }
    }
    expected.sort_unstable();
    actual.sort_unstable();
    assert_eq!(actual.len(), expected.len(), "triangle count changed");
    assert!(
        actual == expected,
        "leaf positions or cyclic triangle winding changed"
    );
    eprintln!(
        "Audited {} triangles across {} leaves: positions and winding retained",
        actual.len(),
        uris.len()
    );
}

#[test]
fn opaque_parent_keeps_colour_even_when_source_alpha_is_zero() {
    let dir = tempfile::tempdir().unwrap();
    let input = dir.path().join("source.glb");
    let output = dir.path().join("result.3tz");
    let mut png = Vec::new();
    image::DynamicImage::ImageRgba8(image::RgbaImage::from_pixel(
        32,
        32,
        image::Rgba([200, 50, 20, 0]),
    ))
    .write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
    .unwrap();
    fs::write(
        &input,
        write_glb(&[TilePrimitive {
            positions: vec![[0., 0., 0.], [1., 0., 0.], [0., 1., 0.], [1., 1., 0.]],
            normals: vec![],
            uvs: vec![[0., 0.], [1., 0.], [0., 1.], [1., 1.]],
            indices: vec![0, 1, 2, 1, 3, 2],
            jpeg: Some(png),
        }])
        .unwrap(),
    )
    .unwrap();
    mesh_to_3tz(
        &input,
        &output,
        &MeshTo3tzOptions {
            max_triangles: 1,
            max_bytes: 0,
            tile_size: 128,
            ..Default::default()
        },
    )
    .unwrap();
    let expanded = expand(&entry(&output, "t/0.glb"));
    let (_, _, images) = gltf::import_slice(&expanded).unwrap();
    assert!(!images.is_empty());
    for image in images {
        for p in image.pixels.as_chunks::<4>().0 {
            assert_eq!(*p, [200, 50, 20, 255]);
        }
    }
}

#[test]
fn missing_normals_remain_missing_through_spatial_conversion() {
    let dir = tempfile::tempdir().unwrap();
    let input = dir.path().join("source.glb");
    let output = dir.path().join("tiles.3tz");
    let source = write_glb(&[TilePrimitive {
        positions: vec![[0., 0., 0.], [1., 0., 0.], [0., 1., 0.], [1., 1., 0.]],
        indices: vec![0, 1, 2, 1, 3, 2],
        ..Default::default()
    }])
    .unwrap();
    let gltf = gltf::Gltf::from_slice(&source).unwrap();
    assert!(gltf
        .meshes()
        .flat_map(|m| m.primitives())
        .all(|p| p.get(&gltf::Semantic::Normals).is_none()));
    fs::write(&input, source).unwrap();
    mesh_to_3tz(
        &input,
        &output,
        &MeshTo3tzOptions {
            max_triangles: 1,
            max_bytes: 0,
            ..Default::default()
        },
    )
    .unwrap();
    let mut zip = zip::ZipArchive::new(fs::File::open(output).unwrap()).unwrap();
    for i in 0..zip.len() {
        let mut file = zip.by_index(i).unwrap();
        if !file.name().ends_with(".glb") {
            continue;
        }
        let mut bytes = Vec::new();
        file.read_to_end(&mut bytes).unwrap();
        let doc = gltf::Gltf::from_slice_without_validation(&bytes).unwrap();
        assert!(doc
            .meshes()
            .flat_map(|m| m.primitives())
            .all(|p| p.get(&gltf::Semantic::Normals).is_none()));
    }
}
