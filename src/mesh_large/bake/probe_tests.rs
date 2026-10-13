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
                donor_ranges: std::iter::once(node.first..node.end).collect(),
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
    let mut query_face = f.faces[0].clone();
    query_face.source_triangle = 1;
    let w = source
        .nearest([0.25, 0.25, 0.], &query_face, [0., 0., 1.], &mut || Ok(()))
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
            .nearest([0., 0., 0.], &f.faces[0], [0., 0., 1.], &mut || Ok(()))
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
    let layout = charts::build(
        &f,
        &raster_faces(&f, &source).unwrap(),
        l,
        0.125,
        &mut || Ok(()),
    )
    .unwrap();
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
            for (pixel, w) in page.rgba.as_chunks::<4>().0.iter().zip(&page.support) {
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
        let layout = charts::build(
            &f,
            &raster_faces(&f, &source).unwrap(),
            l,
            0.125,
            &mut || Ok(()),
        )
        .unwrap();
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
        .nearest([0.5, 0.25, 0.], face, [0., 0., 1.], &mut || Ok(()))
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

#[test]
fn nonuniform_alias_bindings_keep_uv_sampler_and_class_identity() {
    let (_work, c) = capture(1, |input| {
        let mut doc: serde_json::Value = serde_json::from_slice(&fs::read(input).unwrap()).unwrap();
        let off = doc["bufferViews"][4]["byteOffset"].as_u64().unwrap() as usize;
        let path = input.parent().unwrap().join("attributes.bin");
        let mut bin = fs::read(&path).unwrap();
        // UV1 is half UV0 in U. At the same surface point these select a
        // literal red texel versus the red/blue LINEAR half sample.
        bin[off + 8..off + 12].copy_from_slice(&0.5f32.to_le_bytes());
        fs::write(path, bin).unwrap();
        image::RgbaImage::from_raw(2, 1, vec![255, 0, 0, 64, 0, 0, 255, 192])
            .unwrap()
            .save(input.parent().unwrap().join("pixel.png"))
            .unwrap();
        doc["samplers"] = serde_json::json!([
            {"wrapS":33071,"wrapT":33071,"magFilter":9729,"minFilter":9987},
            {"wrapS":10497,"wrapT":10497,"magFilter":9728,"minFilter":9984}]);
        doc["textures"] = serde_json::json!([{"source":0,"sampler":0},{"source":0,"sampler":1}]);
        doc["materials"][0]["pbrMetallicRoughness"]["metallicRoughnessTexture"] =
            serde_json::json!({"index":0,"texCoord":0});
        doc["materials"][0]["normalTexture"] =
            serde_json::json!({"index":0,"texCoord":0,"scale":2.});
        doc["materials"][0]["occlusionTexture"] =
            serde_json::json!({"index":1,"texCoord":0,"strength":0.4});
        doc["materials"][0]["emissiveTexture"] = serde_json::json!({"index":0,"texCoord":0});
        doc["materials"][0]["emissiveFactor"] = serde_json::json!([0.2, 0.3, 0.4]);
        fs::write(input, serde_json::to_vec(&doc).unwrap()).unwrap();
    });
    let f = packet(&c);
    let mut source = CapturedSource::new(&c, 128 << 20, &mut || Ok(())).unwrap();
    source.begin(f.node, 10000).unwrap();
    let w = source
        .nearest([0.5, 0.25, 0.], &f.faces[0], [0., 0., 1.], &mut || Ok(()))
        .unwrap();
    let mut sample = |slot| source.signal(w, slot, 0., &mut || Ok(())).unwrap().unwrap();
    let midpoint = [0.5, 0., 0.5, 128. / 255.];
    assert_eq!(sample(TextureSlot::MetallicRoughness), (midpoint, 0));
    assert_eq!(sample(TextureSlot::Normal), (midpoint, 0));
    assert_eq!(
        sample(TextureSlot::Occlusion),
        ([0., 0., 1., 192. / 255.], 0)
    );
    let (emissive, uv) = sample(TextureSlot::Emissive);
    assert_eq!(uv, 0);
    let linear_half = 1.055 * 0.5f64.powf(1. / 2.4) - 0.055;
    assert!((emissive[0] - linear_half).abs() < 1e-12);
    assert!((emissive[2] - linear_half).abs() < 1e-12);
    assert_eq!((emissive[1], emissive[3]), (0., 128. / 255.));
    assert_eq!(
        sample(TextureSlot::BaseColor),
        ([1., 0., 0., 64. / 255.], 1)
    );
    let material = source.material(f.faces[0].material).unwrap();
    assert_eq!(material["normalTexture"]["scale"], 2.);
    assert_eq!(material["occlusionTexture"]["strength"], 0.4);
    assert_eq!(
        material["emissiveFactor"],
        serde_json::json!([0.2, 0.3, 0.4])
    );
    assert_eq!(material["pbrMetallicRoughness"]["metallicFactor"], 0.25);
}

#[test]
fn entry_contract_errors_and_cancellation_never_reach_assembler() {
    let (_work, c) = capture(1, |_| {});
    let valid = packet(&c);
    let mut source = CapturedSource::new(&c, 128 << 20, &mut || Ok(())).unwrap();
    // The analytic packet carries an empty GLB: these are contract-error probes,
    // never H17 admission or assembled-candidate acceptance.
    let mut cases = Vec::new();
    let mut f = packet(&c);
    f.node.children = 1;
    cases.push(f);
    let mut f = packet(&c);
    f.original_sha256[0] ^= 1;
    cases.push(f);
    let mut f = packet(&c);
    f.faces[0].donor_ranges.clear();
    cases.push(f);
    let mut f = packet(&c);
    f.faces[0].transform.scale = [1f32.to_bits(), 2f32.to_bits(), 1f32.to_bits()];
    cases.push(f);
    let mut f = packet(&c);
    f.faces[0].transform.scale = [(-1f32).to_bits(); 3];
    cases.push(f);
    let mut f = packet(&c);
    f.faces[0].transform.translation[0] = f32::INFINITY.to_bits();
    cases.push(f);
    for f in cases {
        let mut calls = 0;
        let error = bake_node(
            &f,
            &mut source,
            &[],
            limits(),
            &mut |_| {
                calls += 1;
                Err(invalid("unexpected callback"))
            },
            &mut || Ok(()),
        )
        .err()
        .unwrap();
        assert_eq!(error.kind(), JobErrorKind::InvalidState);
        assert_eq!(calls, 0);
    }
    let mut calls = 0;
    let error = bake_node(
        &valid,
        &mut source,
        &[],
        limits(),
        &mut |_| {
            calls += 1;
            Err(invalid("unexpected callback"))
        },
        &mut || Err(JobError::new(JobErrorKind::Cancelled, "entry control")),
    )
    .err()
    .unwrap();
    assert_eq!(error.kind(), JobErrorKind::Cancelled);
    assert_eq!(calls, 0);
    let tiny = BakeLimits {
        memory_bytes: 1,
        ..limits()
    };
    let error = bake_node(
        &valid,
        &mut source,
        &[],
        tiny,
        &mut |_| {
            calls += 1;
            Err(invalid("unexpected callback"))
        },
        &mut || Ok(()),
    )
    .err()
    .unwrap();
    assert_eq!(error.kind(), JobErrorKind::ResourceLimit);
    assert_eq!(calls, 0);
}

#[test]
fn entry_preparation_propagates_typed_failure_and_only_physical_budget_retries() {
    let (_work, c) = capture(1, |input| {
        let mut doc: serde_json::Value = serde_json::from_slice(&fs::read(input).unwrap()).unwrap();
        doc["materials"][0]["pbrMetallicRoughness"]
            .as_object_mut()
            .unwrap()
            .remove("baseColorTexture");
        fs::write(input, serde_json::to_vec(&doc).unwrap()).unwrap();
    });
    let f = packet(&c);
    let mut source = CapturedSource::new(&c, 128 << 20, &mut || Ok(())).unwrap();
    // Empty original GLB is permitted solely for stage-preparation/error protocol
    // probes. The callback NEVER claims Fit, certification, or successful output.
    let inspect = |p: &PreparedNode| {
        assert_eq!(p.corners.len(), 3);
        assert_eq!(p.face_identities, vec![f.faces[0].identity()]);
        assert!(p.pages.is_empty());
        assert!(p.no_raster_faces.is_empty());
        assert!(p.assembly_memory_bytes > 0 && p.assembly_memory_bytes < limits().memory_bytes);
        for c in &p.corners {
            assert_eq!(c.position, f.faces[0].words[usize::from(c.corner)]);
            assert_eq!(c.transform, f.faces[0].transform);
            assert_eq!(c.color, [1., 0.25, 0.5, 1.]);
            assert!(c
                .normal
                .iter()
                .chain(c.tangent.iter())
                .all(|v| v.is_finite()));
        }
    };
    let mut calls = 0;
    let error = bake_node(
        &f,
        &mut source,
        &[],
        limits(),
        &mut |p| {
            calls += 1;
            inspect(p);
            Err(JobError::new(
                JobErrorKind::Unsupported,
                "typed assembler control",
            ))
        },
        &mut || Ok(()),
    )
    .err()
    .unwrap();
    assert_eq!(error.kind(), JobErrorKind::Unsupported);
    assert_eq!(calls, 1);
    let policy = BakeLimits {
        max_density_drops: 1,
        ..limits()
    };
    calls = 0;
    let error = bake_node(
        &f,
        &mut source,
        &[],
        policy,
        &mut |p| {
            calls += 1;
            inspect(p);
            Ok(Assembly::PhysicalBudget {
                actual_bytes: policy.member_bytes + 1,
            })
        },
        &mut || Ok(()),
    )
    .err()
    .unwrap();
    assert_eq!(error.kind(), JobErrorKind::ResourceLimit);
    assert_eq!(calls, 2);
    calls = 0;
    let error = bake_node(
        &f,
        &mut source,
        &[],
        policy,
        &mut |_| {
            calls += 1;
            Ok(Assembly::PhysicalBudget {
                actual_bytes: policy.member_bytes,
            })
        },
        &mut || Ok(()),
    )
    .err()
    .unwrap();
    assert_eq!(error.kind(), JobErrorKind::InvalidState);
    assert_eq!(calls, 1);
}

#[test]
fn physical_source_sampler_convex_bounds_preserve_constants_and_normal_guards() {
    use crate::mesh_large::leaves::mips::{Class, Level};
    let levels = [
        Level {
            width: 2,
            height: 2,
            rgba: vec![255; 16],
        },
        Level {
            width: 1,
            height: 1,
            rgba: vec![255; 4],
        },
    ];
    let sampler = crops::SourceSampler {
        wrap_s: 33071,
        wrap_t: 33071,
        mag_filter: 9729,
        min_filter: 9987,
    };
    for class in [Class::Data, Class::Colour] {
        for x in 0..129 {
            for y in 0..129 {
                let rgba = source::sample(
                    &levels,
                    class,
                    sampler,
                    [x as f64 / 127., y as f64 / 127.],
                    0.37,
                )
                .unwrap();
                assert_eq!(
                    rgba, [1.; 4],
                    "constant physical taps must remain bounded and exact"
                );
            }
        }
    }
    let identity = frames::Frame {
        normal: [0., 0., 1.],
        tangent: [1., 0., 0.],
        bitangent: [0., 1., 0.],
    };
    for rgb in [
        [f64::NAN, 0.5, 1.],
        [1. + f64::EPSILON, 0.5, 1.],
        [-f64::EPSILON, 0.5, 1.],
    ] {
        assert!(frames::reexpress(rgb, 1., identity, identity).is_err());
    }
    assert!(source::sample(&levels, Class::Data, sampler, [f64::INFINITY, 0.], 0.).is_err());
    assert!(source::sample(&levels, Class::Data, sampler, [0., 0.], f64::NAN).is_err());
}

#[test]
fn positive_actual_child_density_is_not_coarsened_by_source_heuristic() {
    let (_work, c) = capture(2, |input| {
        let mut doc: serde_json::Value = serde_json::from_slice(&fs::read(input).unwrap()).unwrap();
        doc["materials"][0]["pbrMetallicRoughness"]
            .as_object_mut()
            .unwrap()
            .remove("baseColorTexture");
        fs::write(input, serde_json::to_vec(&doc).unwrap()).unwrap();
    });
    let mut f = packet(&c);
    assert_eq!(f.node.identity(), (0, 0, 0, 0));
    f.node.kind = crate::mesh_large::records::NodeKind::Internal;
    f.node.children = 2;
    f.child_texel_size = 0.25;
    // These analytic immediate child records partition the captured source pool
    // for a preparation-only protocol probe; they are not H17 sink certificates.
    let children: Vec<_> = (0..2)
        .map(|i| {
            let mut node = f.node;
            node.kind = crate::mesh_large::records::NodeKind::Leaf;
            node.cell_depth = 1;
            node.cell = i;
            node.last_octant = i as u8;
            node.children = 0;
            node.first = f.node.first + i;
            node.end = node.first + 1;
            PreencodeChild {
                node,
                texel_size: 0.25,
                faces: Vec::new(),
                pages: Vec::new(),
            }
        })
        .collect();
    let mut source = CapturedSource::new(&c, 128 << 20, &mut || Ok(())).unwrap();
    source.begin(f.node, limits().query_work).unwrap();
    assert!(source.density_floor(&mut || Ok(())).unwrap() > f.child_texel_size);
    let mut calls = 0;
    let error = bake_node(
        &f,
        &mut source,
        &children,
        limits(),
        &mut |p| {
            calls += 1;
            assert_eq!(p.receipts["density"]["planned"], 0.25);
            assert_eq!(p.receipts["density"]["schedule"], 0.25);
            Err(JobError::new(
                JobErrorKind::Unsupported,
                "density preparation only",
            ))
        },
        &mut || Ok(()),
    )
    .err()
    .unwrap();
    assert_eq!(error.kind(), JobErrorKind::Unsupported);
    assert_eq!(calls, 1);
    f.child_texel_size = 0.5;
    let error = bake_node(
        &f,
        &mut source,
        &children,
        limits(),
        &mut |_| {
            calls += 1;
            Err(invalid("unexpected callback"))
        },
        &mut || Ok(()),
    )
    .err()
    .unwrap();
    assert_eq!(error.kind(), JobErrorKind::InvalidState);
    assert_eq!(calls, 1);
}

#[test]
fn disparate_scale_collinear_faces_retain_corners_without_raster_density() {
    for count in [1usize, 2] {
        let (_work, c) = capture(count, |input| {
            let mut doc: serde_json::Value =
                serde_json::from_slice(&fs::read(input).unwrap()).unwrap();
            let path = input.parent().unwrap().join("attributes.bin");
            let mut bin = fs::read(&path).unwrap();
            let points = [[0f32, 0., 0.], [1e-20, 0., 0.], [1., 0., 0.]];
            for (i, v) in points.iter().flatten().enumerate() {
                bin[i * 4..i * 4 + 4].copy_from_slice(&v.to_le_bytes());
            }
            fs::write(path, bin).unwrap();
            if count == 1 {
                doc["accessors"][0]["max"] = serde_json::json!([1, 0, 0]);
            }
            doc["materials"][0]["normalTexture"] = serde_json::json!({"index":0});
            fs::write(input, serde_json::to_vec(&doc).unwrap()).unwrap();
        });
        let f = packet(&c);
        let mut source = CapturedSource::new(&c, 128 << 20, &mut || Ok(())).unwrap();
        let l = BakeLimits {
            profile: Profile::Lossless,
            ..limits()
        };
        let mut calls = 0;
        // Error/preparation only: analytic packet with empty originalGLB never
        // obtains Fit. Real source capture and scalar/RGBA provenance are used.
        let error = bake_node(
            &f,
            &mut source,
            &[],
            l,
            &mut |p| {
                calls += 1;
                let nonraster: Vec<_> = f
                    .faces
                    .iter()
                    .enumerate()
                    .filter_map(|(i, face)| {
                        math::face_normal(face.positions.map(|p| p.map(f64::from)))
                            .is_none()
                            .then_some(i)
                    })
                    .collect();
                assert_eq!(p.no_raster_faces, nonraster);
                assert_eq!(nonraster.len(), 1);
                let fi = nonraster[0];
                assert!(p.face_pages[fi].iter().all(Option::is_none));
                for corner in &p.corners[fi * 3..fi * 3 + 3] {
                    assert_eq!(
                        corner.position,
                        f.faces[fi].words[usize::from(corner.corner)]
                    );
                    assert_eq!(corner.color, [1., 0.25, 0.5, 1.]);
                    assert_eq!(corner.uv, [0., 0.]);
                }
                if count == 1 {
                    assert!(p.pages.is_empty());
                } else {
                    assert_eq!(p.pages.len(), 2);
                }
                Err(JobError::new(
                    JobErrorKind::Unsupported,
                    "nonraster preparation only",
                ))
            },
            &mut || Ok(()),
        )
        .err()
        .unwrap();
        assert_eq!(error.kind(), JobErrorKind::Unsupported);
        assert_eq!(calls, 1);
    }
}

#[test]
fn skinny_scalar_colour_only_capture_has_normal_fans_without_atlas_obligation() {
    let (_work, c) = capture(1, |input| {
        let mut doc: serde_json::Value = serde_json::from_slice(&fs::read(input).unwrap()).unwrap();
        let path = input.parent().unwrap().join("attributes.bin");
        let mut bin = fs::read(&path).unwrap();
        let points = [[0f32, 0., 0.], [1e-20, 0., 0.], [0., 1., 0.]];
        for (i, v) in points.iter().flatten().enumerate() {
            bin[i * 4..i * 4 + 4].copy_from_slice(&v.to_le_bytes());
        }
        fs::write(path, bin).unwrap();
        doc["accessors"][0]["max"] = serde_json::json!([1e-20, 1, 0]);
        doc["materials"][0]["pbrMetallicRoughness"]
            .as_object_mut()
            .unwrap()
            .remove("baseColorTexture");
        fs::write(input, serde_json::to_vec(&doc).unwrap()).unwrap();
    });
    let f = packet(&c);
    assert!(math::face_normal(f.faces[0].positions.map(|p| p.map(f64::from))).is_some());
    let mut source = CapturedSource::new(&c, 128 << 20, &mut || Ok(())).unwrap();
    assert_eq!(raster_faces(&f, &source).unwrap(), vec![false]);
    let mut calls = 0;
    // Nondegenerate scalar-only preparation with real captured COLOR. The
    // callback refuses: empty original GLB is not an accepted finishing packet.
    let error = bake_node(
        &f,
        &mut source,
        &[],
        limits(),
        &mut |p| {
            calls += 1;
            assert!(p.pages.is_empty());
            assert!(p.no_raster_faces.is_empty());
            assert_eq!(p.corners.len(), 3);
            assert_eq!(p.face_identities, vec![f.faces[0].identity()]);
            assert!(p.receipts["charts"].as_array().unwrap().is_empty());
            assert!(p.receipts["density"]["planned"].as_f64().unwrap() < 1e-19);
            for corner in &p.corners {
                assert_eq!(
                    corner.position,
                    f.faces[0].words[usize::from(corner.corner)]
                );
                assert_eq!(corner.color, [1., 0.25, 0.5, 1.]);
                assert_eq!(corner.uv, [0., 0.]);
                assert_eq!(corner.normal, [0., 0., 1.]);
                assert!(corner.tangent.iter().all(|v| v.is_finite()));
            }
            Err(JobError::new(
                JobErrorKind::Unsupported,
                "scalar preparation only",
            ))
        },
        &mut || Ok(()),
    )
    .err()
    .unwrap();
    assert_eq!(error.kind(), JobErrorKind::Unsupported);
    assert_eq!(calls, 1);
}
