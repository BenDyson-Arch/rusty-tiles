//! Small actual captured-source probes. These do not pretend to be an H17
//! finishing packet or a complete immutable-GLB assembly acceptance fixture.
use super::*;
use crate::gltf_source::source::TextureSlot;
use crate::mesh_large::{attributes::FullTriangle, config::TreeConfig, full, leaves::crops};
use std::fs;
fn capture(
    count: usize,
    edit: impl FnOnce(&std::path::Path),
) -> (tempfile::TempDir, full::FullCaptured) {
    let work = tempfile::tempdir().unwrap();
    let input = full::tests::fixture(work.path(), count);
    edit(&input);
    let mut config = TreeConfig::new(Profile::Balanced);
    config.jobs = crate::mesh_large::config::Jobs::Fixed(std::num::NonZeroUsize::new(1).unwrap());
    config.sort.run_bytes = 8192;
    config.sort.fan_in = 2;
    config.sort.buffer_bytes = 128;
    config.scratch_dir = Some(work.path().join("scratch"));
    fs::create_dir_all(config.scratch_dir.as_ref().unwrap()).unwrap();
    let captured = full::capture(&input, &config, &mut || Ok(())).unwrap();
    (work, captured)
}
fn packet(c: &full::FullCaptured) -> FrozenNode {
    let node = c
        .tree
        .nodes
        .reader()
        .unwrap()
        .next()
        .transpose()
        .unwrap()
        .unwrap();
    let faces = c
        .triangles
        .reader()
        .unwrap()
        .enumerate()
        .map(|(i, t)| {
            let t = t.unwrap();
            FrozenFace {
                ordinal: i as u64,
                source: source::key(&t),
                source_triangle: t.triangle.source.triangle,
                material: t.material,
                positions: t.triangle.positions,
                words: t
                    .triangle
                    .positions
                    .map(|p| PositionWord::Float(p.map(f32::to_bits))),
                transform: PositionTransform::default(),
                welds: t
                    .triangle
                    .positions
                    .map(|p| p.map(|v| if v == 0. { 0 } else { v.to_bits() })),
                features: Vec::new(),
                fill_bound: None,
                donor_ranges: vec![node.first..node.end],
            }
        })
        .collect();
    FrozenNode {
        node,
        original_glb: Vec::new(),
        original_sha256: Sha256::digest([]).into(),
        geometric_error: 0.,
        child_texel_size: 0.,
        faces,
    }
}
fn limits() -> BakeLimits {
    BakeLimits {
        profile: Profile::Balanced,
        memory_bytes: 512 << 20,
        member_bytes: 96 << 20,
        source_cache_bytes: 128 << 20,
        query_work: 1_000_000,
        max_page_edge: 2048,
        max_density_drops: 4,
    }
}
#[test]
fn source_query_spill_weld_colour_and_lineage_ties_use_actual_capture() {
    let (_work, c) = capture(2, |input| {
        let doc: serde_json::Value = serde_json::from_slice(&fs::read(input).unwrap()).unwrap();
        let offset = doc["bufferViews"][5]["byteOffset"].as_u64().unwrap() as usize;
        let path = input.parent().unwrap().join("attributes.bin");
        let mut bytes = fs::read(&path).unwrap();
        for tri in 0..2 {
            for corner in 0..3 {
                for a in 0..4 {
                    let value = if a == 3 || a == tri { 1f32 } else { 0f32 };
                    let i = offset + (tri * 12 + corner * 4 + a) * 4;
                    bytes[i..i + 4].copy_from_slice(&value.to_le_bytes());
                }
            }
        }
        fs::write(path, bytes).unwrap();
    });
    let f = packet(&c);
    let t: FullTriangle = c
        .triangles
        .reader()
        .unwrap()
        .next()
        .transpose()
        .unwrap()
        .unwrap();
    let mut source = CapturedSource::new(&c, 128 << 20, &mut || Ok(())).unwrap();
    source.begin(f.node, 10000).unwrap();
    let k = source::key(&t);
    let w = source
        .nearest(
            [0.25, 0.25, 0.],
            k,
            1,
            [0., 0., 1.],
            None,
            &[f.node.first..f.node.end],
            &mut || Ok(()),
        )
        .unwrap();
    assert_eq!(
        w.triangle.triangle.source.triangle, 1,
        "equal-distance source sheets prefer exact face lineage"
    );
    let weld = t.triangle.positions[0].map(|v| if v == 0. { 0 } else { v.to_bits() });
    assert_eq!(
        source.color(k, 0, weld, &mut || Ok(())).unwrap(),
        [1., 0., 0., 1.]
    );
    assert_eq!(
        source.color(k, 1, weld, &mut || Ok(())).unwrap(),
        [0., 1., 0., 1.]
    );
    source.begin(f.node, 1).unwrap();
    assert_eq!(
        source
            .nearest(
                [0., 0., 0.],
                k,
                0,
                [0., 0., 1.],
                None,
                &[f.node.first..f.node.end],
                &mut || Ok(())
            )
            .unwrap_err()
            .kind(),
        JobErrorKind::ResourceLimit
    );
}
#[test]
fn five_aliased_slots_raster_and_normal_mips_use_original_captured_samples() {
    let (_work, c) = capture(1, |input| {
        let mut doc: serde_json::Value = serde_json::from_slice(&fs::read(input).unwrap()).unwrap();
        doc["materials"][0]["pbrMetallicRoughness"]["metallicRoughnessTexture"] =
            serde_json::json!({"index":0,"texCoord":0});
        doc["materials"][0]["normalTexture"] =
            serde_json::json!({"index":0,"texCoord":1,"scale":2.});
        doc["materials"][0]["occlusionTexture"] =
            serde_json::json!({"index":0,"texCoord":0,"strength":0.4});
        doc["materials"][0]["emissiveTexture"] = serde_json::json!({"index":0,"texCoord":1});
        doc["materials"][0]["emissiveFactor"] = serde_json::json!([0.2, 0.3, 0.4]);
        fs::write(input, serde_json::to_vec(&doc).unwrap()).unwrap();
    });
    let f = packet(&c);
    let l = limits();
    let mut source = CapturedSource::new(&c, l.source_cache_bytes, &mut || Ok(())).unwrap();
    source.begin(f.node, l.query_work).unwrap();
    let layout = charts::build(&f, l, 0.125, &mut || Ok(())).unwrap();
    let mut child = sampling::ChildQuery::new(&[], l.memory_bytes, l.query_work).unwrap();
    let mut raster =
        raster::build(&f, &layout, &mut source, &mut child, l, &mut || Ok(())).unwrap();
    assert_eq!(raster.pages.len(), 5);
    for page in &mut raster.pages {
        let chain = mips::build(page, l.memory_bytes, l.query_work, &mut || Ok(())).unwrap();
        assert_eq!(
            (
                chain.levels.last().unwrap().width,
                chain.levels.last().unwrap().height
            ),
            (1, 1)
        );
        assert!(page.support.iter().any(|w| *w > 0));
        if page.slot != TextureSlot::Normal {
            for (pixel, w) in page.rgba.chunks_exact(4).zip(&page.support) {
                if *w > 0 {
                    assert_eq!(pixel, &[120, 80, 40, 255]);
                }
            }
        } else {
            let refs: Vec<&[u8]> = chain.levels.iter().map(|l| l.rgba.as_slice()).collect();
            mips::normal_floor(page, &refs, l.query_work, &mut || Ok(())).unwrap();
            let expected = frames::effective_direction(
                [120. / 127.5 - 1., 80. / 127.5 - 1., 40. / 127.5 - 1.],
                2.,
            )
            .unwrap();
            for (i, w) in page.support.iter().enumerate() {
                if *w > 0 {
                    let raw = std::array::from_fn(|a| f64::from(page.rgba[i * 4 + a]) / 127.5 - 1.);
                    let actual = page.levels[0].frames[i].transform(raw).unwrap();
                    assert!(
                        math::dot(actual, expected)
                            .clamp(-1., 1.)
                            .acos()
                            .to_degrees()
                            < 0.5
                    );
                }
            }
        }
    }
    assert!(raster.face_pages[0].iter().all(Option::is_some));
    assert_eq!(
        crops::resolve(Some(&raster.pages[0].sampler))
            .unwrap()
            .wrap_s,
        33071
    );
}
#[test]
fn all_and_mixed_degenerate_textured_capture_keeps_colour_and_geometry_without_empty_delivery() {
    for count in [1usize, 2] {
        let (_work, c) = capture(count, |input| {
            let mut doc: serde_json::Value =
                serde_json::from_slice(&fs::read(input).unwrap()).unwrap();
            let path = input.parent().unwrap().join("attributes.bin");
            let mut bytes = fs::read(&path).unwrap();
            bytes[..36].fill(0);
            fs::write(path, bytes).unwrap();
            if count == 1 {
                doc["accessors"][0]["max"] = serde_json::json!([0, 0, 0]);
            }
            doc["materials"][0]["normalTexture"] = serde_json::json!({"index":0});
            fs::write(input, serde_json::to_vec(&doc).unwrap()).unwrap();
        });
        let f = packet(&c);
        let l = limits();
        let mut source = CapturedSource::new(&c, l.source_cache_bytes, &mut || Ok(())).unwrap();
        source.begin(f.node, l.query_work).unwrap();
        let layout = charts::build(&f, l, 0.125, &mut || Ok(())).unwrap();
        let mut children = sampling::ChildQuery::new(&[], l.memory_bytes, l.query_work).unwrap();
        let raster =
            raster::build(&f, &layout, &mut source, &mut children, l, &mut || Ok(())).unwrap();
        assert_eq!(raster.frames.len(), count);
        assert_eq!(raster.colors.len(), count);
        assert_eq!(raster.colors[0], [[1., 0.25, 0.5, 1.]; 3]);
        assert!(raster.face_pages[0].iter().all(Option::is_none));
        assert_eq!(f.faces[0].positions, [[0.; 3]; 3]);
        if count == 1 {
            assert!(raster.pages.is_empty());
        } else {
            assert!(raster.face_pages[1][0].is_some());
            assert!(raster.face_pages[1][2].is_some());
            assert!(raster
                .pages
                .iter()
                .all(|p| p.support.iter().any(|w| *w > 0)));
        }
    }
}
#[test]
fn unequal_length_original_normal_pixels_filter_raw_then_apply_literal_scale() {
    let (_work, c) = capture(1, |input| {
        let mut doc: serde_json::Value = serde_json::from_slice(&fs::read(input).unwrap()).unwrap();
        doc["materials"][0]["normalTexture"] = serde_json::json!({"index":0,"scale":2.});
        fs::write(input, serde_json::to_vec(&doc).unwrap()).unwrap();
        let pixels =
            image::RgbaImage::from_raw(2, 1, vec![255, 128, 128, 255, 128, 128, 159, 255]).unwrap();
        pixels
            .save(input.parent().unwrap().join("pixel.png"))
            .unwrap();
    });
    let f = packet(&c);
    let mut source = CapturedSource::new(&c, 128 << 20, &mut || Ok(())).unwrap();
    source.begin(f.node, 10000).unwrap();
    let face = &f.faces[0];
    let w = source
        .nearest(
            [0.5, 0.25, 0.],
            face.source,
            face.source_triangle,
            [0., 0., 1.],
            None,
            &face.donor_ranges,
            &mut || Ok(()),
        )
        .unwrap();
    let (raw, uv) = source
        .signal(w, TextureSlot::Normal, 0., &mut || Ok(()))
        .unwrap()
        .unwrap();
    assert_eq!(uv, 0);
    assert_eq!(raw, [191.5 / 255., 128. / 255., 143.5 / 255., 1.]);
    let donor = CapturedSource::frame(w).unwrap();
    let actual = frames::reexpress(
        [raw[0], raw[1], raw[2]],
        2.,
        donor,
        frames::Frame {
            tangent: [1., 0., 0.],
            bitangent: [0., 1., 0.],
            normal: [0., 0., 1.],
        },
    )
    .unwrap();
    let expected = math::unit([
        2. * (191.5 / 127.5 - 1.),
        2. * (128. / 127.5 - 1.),
        143.5 / 127.5 - 1.,
    ])
    .unwrap();
    assert!(math::distance2(actual, expected) < 1e-24);
    let a = math::unit([1., 128. / 127.5 - 1., 128. / 127.5 - 1.]).unwrap();
    let b = math::unit([128. / 127.5 - 1., 128. / 127.5 - 1., 159. / 127.5 - 1.]).unwrap();
    let wrong =
        frames::effective_direction(std::array::from_fn(|i| (a[i] + b[i]) * 0.5), 2.).unwrap();
    assert!(
        math::dot(actual, wrong).clamp(-1., 1.).acos().to_degrees() > 15.,
        "control must expose source-base normalization before LINEAR filtering"
    );
    let (coarse, _) = source
        .signal(w, TextureSlot::Normal, 1., &mut || Ok(()))
        .unwrap()
        .unwrap();
    assert_eq!(
        coarse,
        [192. / 255., 128. / 255., 144. / 255., 1.],
        "original minification retains raw linear component policy"
    );
}
#[test]
fn mirrored_normal_texture_without_authored_tangent_keeps_admission_unsupported_and_clean() {
    let work = tempfile::tempdir().unwrap();
    let input = full::tests::fixture(work.path(), 1);
    let mut doc: serde_json::Value = serde_json::from_slice(&fs::read(&input).unwrap()).unwrap();
    doc["meshes"][0]["primitives"][0]["attributes"]
        .as_object_mut()
        .unwrap()
        .remove("TANGENT");
    doc["materials"][0]["normalTexture"] = serde_json::json!({"index":0,"texCoord":1});
    let uv_offset = doc["bufferViews"][4]["byteOffset"].as_u64().unwrap() as usize;
    let bin = work.path().join("attributes.bin");
    let mut bytes = fs::read(&bin).unwrap();
    for c in 0..3 {
        let i = uv_offset + (c * 2 + 1) * 4;
        let v = -f32::from_le_bytes(bytes[i..i + 4].try_into().unwrap());
        bytes[i..i + 4].copy_from_slice(&v.to_le_bytes());
    }
    fs::write(bin, bytes).unwrap();
    fs::write(&input, serde_json::to_vec(&doc).unwrap()).unwrap();
    let mut config = TreeConfig::new(Profile::Balanced);
    config.jobs = crate::mesh_large::config::Jobs::Fixed(std::num::NonZeroUsize::new(1).unwrap());
    config.sort.run_bytes = 8192;
    config.sort.fan_in = 2;
    config.sort.buffer_bytes = 128;
    let scratch = work.path().join("scratch");
    fs::create_dir_all(&scratch).unwrap();
    config.scratch_dir = Some(scratch.clone());
    let failure = full::capture(&input, &config, &mut || Ok(()))
        .err()
        .expect("owning normal-map admission remains unchanged");
    assert_eq!(failure.error.kind(), JobErrorKind::Unsupported);
    assert!(failure
        .error
        .to_string()
        .contains("normalTexture requires authored NORMAL and TANGENT"));
    assert_eq!(
        fs::read_dir(scratch).unwrap().count(),
        0,
        "failed admission must leave no owned scratch"
    );
}
