//! Independently authored semantic witnesses and immutable handoff fixture.
//! The embedded GLB is generated/audited by the stdlib-only evidence generator;
//! it has analytic zero source/proxy correspondence, not executed H17 acceptance.
use super::{
    frames::{self, Frame},
    source,
    types::*,
};
use crate::mesh_large::{
    leaves::{
        crops::SourceSampler,
        id_texture,
        mips::{Class, Level},
        sparse_mips,
    },
    records::{Bounds, NodeKind, NodeRecord},
};
use base64::{engine::general_purpose::STANDARD, Engine};
use sha2::{Digest, Sha256};

fn close(actual: [f64; 3], expected: [f64; 3]) {
    for (a, e) in actual.into_iter().zip(expected) {
        assert!(
            (a - e).abs() < 1e-12,
            "actual={actual:?} expected={expected:?}"
        );
    }
}

fn identity() -> Frame {
    Frame {
        tangent: [1., 0., 0.],
        bitangent: [0., 1., 0.],
        normal: [0., 0., 1.],
    }
}

fn rotated_sixty() -> Frame {
    Frame {
        tangent: [0.5, 0., -3f64.sqrt() / 2.],
        bitangent: [0., 1., 0.],
        normal: [3f64.sqrt() / 2., 0., 0.5],
    }
}

#[test]
fn representative_chart_aggregates_original_support_at_each_footprint() {
    let samples = [
        NormalSample {
            uv: [0.1, 0.1],
            frame: identity(),
            chart: 7,
            weight: 3.,
        },
        NormalSample {
            uv: [0.4, 0.1],
            frame: identity(),
            chart: 7,
            weight: 3.,
        },
        NormalSample {
            uv: [0.8, 0.1],
            frame: rotated_sixty(),
            chart: 2,
            weight: 5.,
        },
    ];
    let mut work = 0;
    let (fine, mixed) =
        super::mips::dominant_frames(&samples, 4, 1, 1 << 20, &mut work, 100, &mut || Ok(()))
            .unwrap();
    assert_eq!(mixed, 0);
    close(fine[3].unwrap().normal, rotated_sixty().normal);
    let (coarse, mixed) =
        super::mips::dominant_frames(&samples, 1, 1, 1 << 20, &mut work, 100, &mut || Ok(()))
            .unwrap();
    // Chart7 has6, versus chart2's5: largest individual sample and chart-ID
    // ordering both elect the wrong chart. Original contributions survive coarsening.
    close(coarse[0].unwrap().normal, [0., 0., 1.]);
    assert_eq!(mixed, 1);
    let same_chart: Vec<_> = samples
        .iter()
        .map(|s| NormalSample {
            chart: 7,
            frame: identity(),
            ..*s
        })
        .collect();
    let (_, mixed) =
        super::mips::dominant_frames(&same_chart, 1, 1, 1 << 20, &mut work, 100, &mut || Ok(()))
            .unwrap();
    assert_eq!(
        mixed, 0,
        "three contributors in one chart are not chart mixing"
    );
}

fn committed_normal_page(second_frame: Frame) -> SignalPage {
    let terminal = TerminalSample {
        source_rank: 0,
        barycentric: [1., 0., 0.],
        centre: [0.; 3],
        support_min: [0.; 3],
        support_max: [0.; 3],
        weight: 1,
        centre_is_centroid: false,
    };
    let levels: Vec<_> = [4, 2, 1]
        .into_iter()
        .map(|edge| {
            let count = edge as usize * edge as usize;
            SignalLevel {
                width: edge,
                height: edge,
                rgba: [128, 128, 255, 255].repeat(count),
                support: vec![1; count],
                frames: vec![identity(); count],
                samples: vec![terminal; count],
                world_normals: vec![[0., 0., 1.]; count],
            }
        })
        .collect();
    SignalPage {
        width: 4,
        height: 4,
        rgba: levels[0].rgba.clone(),
        support: levels[0].support.clone(),
        slot: crate::gltf_source::source::TextureSlot::Normal,
        samples: levels[0].samples.clone(),
        levels,
        sampler: serde_json::json!({"magFilter":9728,"minFilter":9984}),
        material: 0,
        receipt: serde_json::json!({}),
        normal_scale: 1.,
        normal_samples: vec![
            NormalSample {
                uv: [0.25, 0.5],
                frame: identity(),
                chart: 0,
                weight: 1.,
            },
            NormalSample {
                uv: [0.75, 0.5],
                frame: second_frame,
                chart: 1,
                weight: 1.,
            },
        ],
    }
}

#[test]
fn terminal_two_frame_normal_floors_keep_committed_reference_and_report_bleed() {
    let page = committed_normal_page(rotated_sixty());
    let pixels: Vec<_> = page.levels.iter().map(|l| l.rgba.as_slice()).collect();
    let receipt = super::mips::normal_floor(&page, &pixels, 100, &mut || Ok(())).unwrap();
    let levels = receipt["rendererFrameFloors"].as_array().unwrap();
    assert_eq!(levels.len(), 3, "terminal1x1 must be evaluated");
    for level in levels {
        // Identical committed tangent pixels viewed through two actual emitted
        // frames differ from common-world Z by0/60degrees, mean30. This is
        // permitted shared-texel bleed, not a codec/reference fidelity failure.
        assert_eq!(level["committedMixtureReference"], true);
        assert!(level["maxAngle"].as_f64().unwrap() < 0.5);
        let bleed = &level["worldMixtureBleed"];
        assert!((bleed["mean"].as_f64().unwrap() - 30.).abs() < 1e-6);
        assert!((bleed["max"].as_f64().unwrap() - 60.).abs() < 1e-6);
        let upper = bleed["p99Upper"].as_f64().unwrap();
        assert!((60. ..=60.01).contains(&upper));
    }
    let wrong: Vec<_> = page
        .levels
        .iter()
        .map(|l| [255, 128, 128, 255].repeat(l.support.len()))
        .collect();
    let wrong_pixels: Vec<_> = wrong.iter().map(Vec::as_slice).collect();
    let err = super::mips::normal_floor(&page, &wrong_pixels, 100, &mut || Ok(())).unwrap_err();
    assert_eq!(err.kind(), crate::JobErrorKind::Unsupported);
    let mut wrong_representation = committed_normal_page(rotated_sixty());
    for level in &mut wrong_representation.levels {
        level.frames.fill(rotated_sixty());
    }
    // Changing representative frame without regenerating committed RGB must
    // fail; a self-reference built by decoding those same pixels would pass.
    assert!(
        super::mips::normal_floor(&wrong_representation, &pixels, 100, &mut || Ok(()),).is_err()
    );
}

#[test]
fn same_emitted_frame_has_no_world_mixture_bleed_at_terminal_level() {
    let page = committed_normal_page(identity());
    let pixels: Vec<_> = page.levels.iter().map(|l| l.rgba.as_slice()).collect();
    let receipt = super::mips::normal_floor(&page, &pixels, 100, &mut || Ok(())).unwrap();
    for level in receipt["rendererFrameFloors"].as_array().unwrap() {
        assert!(level["worldMixtureBleed"]["max"].as_f64().unwrap() < 1e-5);
        assert!(level["maxAngle"].as_f64().unwrap() < 0.5);
    }
}

#[test]
fn child_mip_footprint_uses_decoded_uv_jacobian_not_nominal_density() {
    let positions = [[0., 0., 0.], [2., 0., 0.], [0., 1., 0.]];
    // Projection sigma.5 compensated by2pixels/metre in X,1in Y.
    let uvpixels = [[0., 0.], [4., 0.], [0., 1.]];
    let stretch = super::math::pixel_gradient(positions, uvpixels).unwrap();
    assert!((stretch - 2.).abs() < 1e-12);
    assert!((stretch.log2() - 1.).abs() < 1e-12);
    assert!((super::math::pixel_size(positions, uvpixels).unwrap() - 1.).abs() < 1e-12);
    // Decode literal normalized-u16 words exactly as final f32 UVs, on a
    // non-square512x256page. Independent diagonal derivatives need no SVD.
    let uv = [[4u16, 8u16], [1028, 8], [4, 520]]
        .map(|p| p.map(|word| f64::from(f32::from(word) / 65535.)));
    let pixels = uv.map(|p| [p[0] * 512., p[1] * 256.]);
    let dx = (pixels[1][0] - pixels[0][0]) / 2.;
    let dy = pixels[2][1] - pixels[0][1];
    let actual = super::math::pixel_gradient(positions, pixels).unwrap();
    assert!((actual - dx.max(dy)).abs() < 1e-12);
    let metres = super::math::pixel_size(positions, pixels).unwrap();
    assert!((metres - (1. / dx).max(1. / dy)).abs() < 1e-12);
    assert!(actual > 4., "u16/f32 decoded distortion is measured");
    assert!(metres > 0.49 && metres < 0.51);
    assert!((actual.log2() - 2.).abs() < 0.001);
}

#[test]
fn authored_tangent_skew_is_not_silently_orthogonalized() {
    let donor = Frame::donor([[0., 0., 1.]; 3], [[0.8, 0., 0.6, 1.]; 3], [0.2, 0.3, 0.5]).unwrap();
    let normal = frames::reexpress([1., 0.5, 1.], 1., donor, identity()).unwrap();
    close(normal, [1. / 5f64.sqrt(), 0., 2. / 5f64.sqrt()]);
    // A Gram-Schmidt donor gives 45 degrees, versus the required26.565degrees.
    assert!((normal[0] - 1. / 2f64.sqrt()).abs() > 0.2);
}

#[test]
fn decoded_proxy_skew_requires_the_full_inverse_not_its_transpose() {
    let proxy = Frame {
        tangent: [0.8, 0., 0.6],
        bitangent: [0., 1., 0.],
        normal: [0., 0., 1.],
    };
    close(proxy.inverse_direction([0., 0., 1.]).unwrap(), [0., 0., 1.]);
    let wrong_transpose = [0.6 / (1.36f64).sqrt(), 0., 1. / (1.36f64).sqrt()];
    assert!(
        wrong_transpose[0] > 0.5,
        "witness must discriminate a transpose shortcut"
    );
    let result = proxy
        .transform(proxy.inverse_direction([0.3, 0.4, 0.5]).unwrap())
        .unwrap();
    close(
        result,
        [
            0.3 / 0.5f64.sqrt(),
            0.4 / 0.5f64.sqrt(),
            0.5 / 0.5f64.sqrt(),
        ],
    );
}

#[test]
fn authored_vertex_axes_and_fragment_interpolation_each_normalize() {
    let donor = Frame::donor(
        [[0., 0., 2.], [0., 3., 0.], [0., 0., 4.]],
        [[1.6, 0., 1.2, 1.], [3., 0., 0., 1.], [2.4, 0., 1.8, 1.]],
        [0.25, 0.5, 0.25],
    )
    .unwrap();
    // Each vertex is normalized first, then each interpolated axis separately.
    close(donor.normal, [0., 1. / 2f64.sqrt(), 1. / 2f64.sqrt()]);
    close(donor.tangent, [3. / 10f64.sqrt(), 0., 1. / 10f64.sqrt()]);
    close(donor.bitangent, [0., 1. / 2f64.sqrt(), -1. / 2f64.sqrt()]);
}

#[test]
fn mirrored_uvs_and_negative_literal_normal_scale_keep_their_signs() {
    let mirrored = Frame::from_triangle(
        [[0., 0., 0.], [2., 0., 0.], [0., 1., 0.]],
        [[0., 0.], [0., 1.], [1., 0.]],
        [0., 0., 1.],
    )
    .unwrap();
    close(mirrored.tangent, [0., 1., 0.]);
    close(mirrored.bitangent, [1., 0., 0.]);
    close(
        mirrored.transform([0., 1., 1.]).unwrap(),
        [1. / 2f64.sqrt(), 0., 1. / 2f64.sqrt()],
    );
    close(
        frames::reexpress([1., 0.5, 1.], -2., identity(), identity()).unwrap(),
        [-2. / 5f64.sqrt(), 0., 1. / 5f64.sqrt()],
    );
    let huge = frames::reexpress([1., 0.5, 1.], f64::MAX, identity(), identity()).unwrap();
    assert!(huge.iter().all(|v| v.is_finite()));
    assert!(huge[0] > 0.999);
    assert!(
        frames::reexpress([1., 0.5, 0.5], 0., identity(), identity()).is_err(),
        "zero effective normal cannot become a neutral repair"
    );
}

fn sampler() -> SourceSampler {
    SourceSampler {
        wrap_s: 33071,
        wrap_t: 33071,
        min_filter: 9987,
        mag_filter: 9729,
    }
}

#[test]
fn source_colour_and_material_data_use_distinct_transfer_with_linear_alpha() {
    let levels = [Level {
        width: 2,
        height: 1,
        rgba: vec![255, 0, 0, 0, 0, 0, 255, 255],
    }];
    let colour = source::sample(&levels, Class::Colour, sampler(), [0.5, 0.5], 0.).unwrap();
    let data = source::sample(&levels, Class::Data, sampler(), [0.5, 0.5], 0.).unwrap();
    assert_eq!(colour.map(|v| (v * 255.).round() as u8), [188, 0, 188, 128]);
    assert_eq!(data.map(|v| (v * 255.).round() as u8), [128, 0, 128, 128]);
    // RGB remains unpremultiplied even though the red source texel has alpha0.
    assert!(colour[0] > 0.7);
}

#[test]
fn wrap_axes_endpoints_and_minification_filters_follow_the_literal_sampler() {
    let levels = [Level {
        width: 2,
        height: 2,
        rgba: vec![10, 0, 0, 255, 20, 0, 0, 255, 30, 0, 0, 255, 40, 0, 0, 255],
    }];
    for (wrap_s, u, expected) in [
        (10497, 1., 30),
        (33648, 1., 40),
        (33071, 1., 40),
        (10497, -0.25, 40),
        (33648, -0.25, 30),
    ] {
        let s = SourceSampler {
            wrap_s,
            mag_filter: 9728,
            ..sampler()
        };
        let pixel = source::sample(&levels, Class::Data, s, [u, 1.], 0.).unwrap();
        assert!((pixel[0] * 255. - f64::from(expected)).abs() < 1e-12);
    }
    let levels = [
        Level {
            width: 2,
            height: 1,
            rgba: vec![0; 8],
        },
        Level {
            width: 1,
            height: 1,
            rgba: vec![255; 4],
        },
    ];
    for (filter, expected) in [
        (9984, 1.),
        (9985, 1.),
        (9986, 0.75),
        (9987, 0.75),
        (9729, 0.),
    ] {
        let s = SourceSampler {
            min_filter: filter,
            ..sampler()
        };
        let pixel = source::sample(&levels, Class::Data, s, [0.5, 0.5], 0.75).unwrap();
        assert!((pixel[0] - expected).abs() < 1e-12, "filter={filter}");
    }
}

fn categorical_pixels() -> (Vec<u8>, Vec<u32>) {
    let tuples = [[1, 9, 1, 64], [1, 9, 1, 64], [2, 8, 2, 128], [3, 8, 3, 128]];
    let pixels = (0..4)
        .flat_map(|y| (0..4).flat_map(move |x| tuples[(y % 2) * 2 + x % 2]))
        .collect();
    let support = (0..4)
        .flat_map(|y| (0..4).map(move |x| [2, 2, 3, 3][(y % 2) * 2 + x % 2]))
        .collect();
    (pixels, support)
}

#[test]
fn categorical_mips_choose_the_complete_tuple_and_never_premultiply_ids() {
    let (pixels, support) = categorical_pixels();
    let chain = sparse_mips::build(4, 4, &pixels, &support, Class::Id, 1 << 20).unwrap();
    assert_eq!(chain.levels.len(), 3);
    for level in &chain.levels[1..] {
        for pixel in level.rgba.chunks_exact(4) {
            assert_eq!(pixel, [1, 9, 1, 64]);
            assert_ne!(
                pixel,
                [1, 8, 1, 64],
                "per-channel modes invent a nonexistent tuple"
            );
        }
    }
    assert_eq!(chain.levels.last().unwrap().rgba, [1, 9, 1, 64]);
}

#[test]
fn three_packed_id_sets_png_readback_null_gutters_and_nearest_sampler_are_exact() {
    let (pixels, mut support) = categorical_pixels();
    support[15] = 0;
    let mut ids = Vec::new();
    for pixel in pixels.chunks_exact(4) {
        ids.extend([
            u32::from(pixel[0]),
            u32::from(pixel[1]),
            u32::from(pixel[2]) | (u32::from(pixel[3]) << 8),
        ]);
    }
    let sets = [
        id_texture::IdSet { bytes: 1 },
        id_texture::IdSet { bytes: 1 },
        id_texture::IdSet { bytes: 2 },
    ];
    // Retained PNG delivery declares a conservative codec workspace even for
    // this tiny image. Include that bound plus the fixture's chain/readback.
    let memory_limit = crate::mesh_large::leaves::delivery::workspace_charge(
        [4, 4],
        crate::mesh_large::config::Profile::Lossless,
    )
    .unwrap()
        + 32 * 1024;
    let texture = id_texture::bake(&id_texture::Request {
        width: 4,
        height: 4,
        sets: &sets,
        ids: &ids,
        support: &support,
        memory_limit,
    })
    .unwrap();
    assert_eq!(texture.layouts.len(), 3);
    assert_eq!(texture.pages.len(), 1);
    assert_eq!(texture.pages[0].levels.len(), 3);
    let mut expected = pixels;
    expected[60..64].copy_from_slice(&[255; 4]);
    let base = image::load_from_memory(&texture.pages[0].levels[0])
        .unwrap()
        .to_rgba8();
    assert_eq!(base.as_raw(), &expected);
    let last = image::load_from_memory(texture.pages[0].levels.last().unwrap())
        .unwrap()
        .to_rgba8();
    assert_eq!(last.as_raw(), &[1, 9, 1, 64]);
    id_texture::check_sampler(&id_texture::sampler()).unwrap();
    let mut blended = id_texture::sampler();
    blended["magFilter"] = serde_json::json!(9729);
    assert!(id_texture::check_sampler(&blended).is_err());
}

/// Portable equivalent of controls/immutable-three-sets.glb in the evidence corpus.
/// Original GLB bytes are immutable; a real assembler must attach the requested
/// page/material/corner packet and then audit geometry plus every metadata table.
pub(super) fn immutable_fixture() -> FrozenNode {
    let original_glb = STANDARD.decode(IMMUTABLE_BASE64).unwrap();
    let float_words = [
        PositionWord::Float([-0.0f32, 0., 0.25].map(f32::to_bits)),
        PositionWord::Float([2.0f32, 0., 0.25].map(f32::to_bits)),
        PositionWord::Float([0.0f32, 1., 0.25].map(f32::to_bits)),
    ];
    let float_transform = PositionTransform {
        translation: [10f32, -2., 0.].map(f32::to_bits),
        scale: [0.5f32, 2., 1.].map(f32::to_bits),
    };
    let quant_words = [
        PositionWord::U16([16, 0, 0]),
        PositionWord::U16([0, 8, 0]),
        PositionWord::U16([0, 0, 0]),
    ];
    let quant_transform = PositionTransform {
        translation: [-3f32, 5., -1.].map(f32::to_bits),
        scale: [0.125f32, 0.25, 0.5].map(f32::to_bits),
    };
    let faces = [
        (float_words, float_transform),
        (quant_words, quant_transform),
    ]
    .into_iter()
    .enumerate()
    .map(|(row, (words, transform))| {
        let positions = words.map(|word| transform.decode(word));
        FrozenFace {
            ordinal: row as u64,
            source: SourceKey {
                node: 5,
                mesh: 11,
                primitive: 7 + 2 * row as u32,
                instance: 12345 + row as u64,
            },
            source_triangle: row as u64,
            material: u32::MAX,
            positions,
            words,
            transform,
            welds: positions.map(|p| p.map(|v| if v == 0. { 0 } else { v.to_bits() })),
            features: [0, 3, 7]
                .into_iter()
                .enumerate()
                .map(|(entry, set)| FeatureIdentity {
                    set,
                    table: Some(entry as u32),
                    value: row as u64,
                })
                .collect(),
            fill_bound: None,
            donor_ranges: vec![row as u64..row as u64 + 1],
        }
    })
    .collect();
    FrozenNode {
        node: NodeRecord {
            cell_depth: 0,
            cell: 0,
            path_len: 0,
            path: 0,
            last_octant: 0,
            kind: NodeKind::Internal,
            children: 2,
            first: 0,
            end: 2,
            bytes: original_glb.len() as u64,
            bounds: Bounds {
                min: [-3., -2., -1.],
                max: [11., 7., 0.25],
            },
        },
        original_sha256: Sha256::digest(&original_glb).into(),
        original_glb,
        geometric_error: 0.,
        child_texel_size: 0.25,
        faces,
    }
}

#[test]
fn immutable_float_and_u16_fixture_preserves_exact_words_trs_and_three_sets() {
    let node = immutable_fixture();
    assert_eq!(node.original_glb.len(), 5444);
    assert_eq!(
        node.original_sha256,
        [
            0xb3, 0x13, 0x64, 0x51, 0x62, 0x75, 0xef, 0x5d, 0xcd, 0x58, 0xfb, 0xb5, 0xf2, 0x15,
            0x69, 0x66, 0x60, 0x86, 0xec, 0x11, 0x5c, 0xd3, 0x3c, 0xeb, 0xb5, 0xa4, 0xb4, 0xf8,
            0x9f, 0x2d, 0x24, 0x96,
        ]
    );
    assert_eq!(
        node.faces[0].words[0],
        PositionWord::Float([0x80000000, 0, 0x3e800000])
    );
    assert_eq!(
        node.faces[0].positions,
        [[10., -2., 0.25], [11., -2., 0.25], [10., 0., 0.25]]
    );
    assert_eq!(
        node.faces[1].positions,
        [[-1., 5., -1.], [-3., 7., -1.], [-3., 5., -1.]]
    );
    for face in &node.faces {
        assert_eq!(face.features.len(), 3);
        assert_eq!(
            face.features.iter().map(|f| f.set).collect::<Vec<_>>(),
            [0, 3, 7]
        );
        for corner in 0..3 {
            assert_eq!(
                face.transform.decode(face.words[corner]).map(f32::to_bits),
                face.positions[corner].map(f32::to_bits)
            );
        }
    }
    let n = u32::from_le_bytes(node.original_glb[12..16].try_into().unwrap()) as usize;
    let doc: serde_json::Value = serde_json::from_slice(&node.original_glb[20..20 + n]).unwrap();
    let tables = doc["extensions"]["EXT_structural_metadata"]["propertyTables"]
        .as_array()
        .unwrap();
    assert_eq!(
        tables.len(),
        4,
        "unreferenced V/F metadata must also survive assembly"
    );
    for primitive in doc["meshes"]
        .as_array()
        .unwrap()
        .iter()
        .map(|m| &m["primitives"][0])
    {
        let sets = primitive["extensions"]["EXT_mesh_features"]["featureIds"]
            .as_array()
            .unwrap();
        assert_eq!(sets.len(), 3);
        assert_eq!(
            sets.iter()
                .map(|set| set["attribute"].as_u64().unwrap())
                .collect::<Vec<_>>(),
            [0, 3, 7]
        );
    }
    audit_original_glb(&node);
}

/// Separately read the actual stored bytes and table values. This intentionally
/// supports the small uncompressed fixture only, not arbitrary glTF admission.
pub(super) fn audit_original_glb(node: &FrozenNode) {
    let glb = &node.original_glb;
    let json_bytes = u32::from_le_bytes(glb[12..16].try_into().unwrap()) as usize;
    let doc: serde_json::Value = serde_json::from_slice(&glb[20..20 + json_bytes]).unwrap();
    let binary = &glb[28 + json_bytes..];
    let accessor = |index: usize, element: usize| {
        let a = &doc["accessors"][index];
        let v = &doc["bufferViews"][a["bufferView"].as_u64().unwrap() as usize];
        let size = if a["componentType"] == 5123 { 2 } else { 4 };
        let components = if a["type"] == "VEC3" { 3 } else { 1 };
        let start = v["byteOffset"].as_u64().unwrap_or(0) as usize
            + a["byteOffset"].as_u64().unwrap_or(0) as usize
            + element
                * v["byteStride"]
                    .as_u64()
                    .map_or(size * components, |n| n as usize);
        &binary[start..start + size * components]
    };
    for (row, wire_node) in doc["nodes"].as_array().unwrap().iter().enumerate() {
        let expected = &node.faces[row];
        let transform = PositionTransform {
            translation: std::array::from_fn(|axis| {
                (wire_node["translation"][axis].as_f64().unwrap() as f32).to_bits()
            }),
            scale: std::array::from_fn(|axis| {
                (wire_node["scale"][axis].as_f64().unwrap() as f32).to_bits()
            }),
        };
        assert_eq!(transform, expected.transform);
        let p = &doc["meshes"][wire_node["mesh"].as_u64().unwrap() as usize]["primitives"][0];
        let position = p["attributes"]["POSITION"].as_u64().unwrap() as usize;
        let indices = p["indices"].as_u64().unwrap() as usize;
        for corner in 0..3 {
            let vertex = u16::from_le_bytes(accessor(indices, corner).try_into().unwrap()) as usize;
            let raw = accessor(position, vertex);
            let word = if doc["accessors"][position]["componentType"] == 5123 {
                PositionWord::U16(std::array::from_fn(|axis| {
                    u16::from_le_bytes(raw[axis * 2..axis * 2 + 2].try_into().unwrap())
                }))
            } else {
                PositionWord::Float(std::array::from_fn(|axis| {
                    u32::from_le_bytes(raw[axis * 4..axis * 4 + 4].try_into().unwrap())
                }))
            };
            assert_eq!(word, expected.words[corner]);
            for (entry, feature) in p["extensions"]["EXT_mesh_features"]["featureIds"]
                .as_array()
                .unwrap()
                .iter()
                .enumerate()
            {
                let set = feature["attribute"].as_u64().unwrap() as u32;
                let attribute = p["attributes"][format!("_FEATURE_ID_{set}")]
                    .as_u64()
                    .unwrap() as usize;
                let value =
                    f32::from_le_bytes(accessor(attribute, vertex).try_into().unwrap()) as u64;
                assert_eq!(
                    expected.features[entry],
                    FeatureIdentity {
                        set,
                        table: Some(feature["propertyTable"].as_u64().unwrap() as u32),
                        value
                    }
                );
            }
        }
    }
    let business = &doc["extensions"]["EXT_structural_metadata"]["propertyTables"][2]["properties"]
        ["business_id"];
    let view = &doc["bufferViews"][business["values"].as_u64().unwrap() as usize];
    let offset = view["byteOffset"].as_u64().unwrap() as usize;
    let ids = [
        u64::from_le_bytes(binary[offset..offset + 8].try_into().unwrap()),
        u64::from_le_bytes(binary[offset + 8..offset + 16].try_into().unwrap()),
    ];
    assert_eq!(ids, [(1 << 53) + 7, (1 << 63) + 11]);
}

// Bytes of the independent handoff fixture, not a production writer result.
const IMMUTABLE_BASE64: &str = concat!(
    "Z2xURgIAAABEFQAA2BMAAEpTT057ImFzc2V0Ijp7InZlcnNpb24iOiIyLjAiLCJnZW5lcmF0b3IiOiJIMjcgaW5kZXBlbmRlbnQgaW1tdXRhYmxl",
    "IGZpeHR1cmUifSwic2NlbmUiOjAsInNjZW5lcyI6W3sibm9kZXMiOlswLDFdfV0sIm5vZGVzIjpbeyJtZXNoIjowLCJ0cmFuc2xhdGlvbiI6WzEw",
    "LjAsLTIuMCwwLjBdLCJzY2FsZSI6WzAuNSwyLjAsMS4wXX0seyJtZXNoIjoxLCJ0cmFuc2xhdGlvbiI6Wy0zLjAsNS4wLC0xLjBdLCJzY2FsZSI6",
    "WzAuMTI1LDAuMjUsMC41XX1dLCJtZXNoZXMiOlt7InByaW1pdGl2ZXMiOlt7ImF0dHJpYnV0ZXMiOnsiUE9TSVRJT04iOjAsIl9GRUFUVVJFX0lE",
    "XzAiOjEsIl9GRUFUVVJFX0lEXzMiOjIsIl9GRUFUVVJFX0lEXzciOjN9LCJpbmRpY2VzIjo0LCJleHRlbnNpb25zIjp7IkVYVF9tZXNoX2ZlYXR1",
    "cmVzIjp7ImZlYXR1cmVJZHMiOlt7ImZlYXR1cmVDb3VudCI6MiwiYXR0cmlidXRlIjowLCJwcm9wZXJ0eVRhYmxlIjowLCJsYWJlbCI6InNvdXJj",
    "ZV9wcmltaXRpdmUifSx7ImZlYXR1cmVDb3VudCI6MiwiYXR0cmlidXRlIjozLCJwcm9wZXJ0eVRhYmxlIjoxLCJsYWJlbCI6InNvdXJjZV90cmlh",
    "bmdsZSJ9LHsiZmVhdHVyZUNvdW50IjoyLCJhdHRyaWJ1dGUiOjcsInByb3BlcnR5VGFibGUiOjIsImxhYmVsIjoiaW5kZXBlbmRlbnRfY2xhc3Mi",
    "fV19fX1dfSx7InByaW1pdGl2ZXMiOlt7ImF0dHJpYnV0ZXMiOnsiUE9TSVRJT04iOjUsIl9GRUFUVVJFX0lEXzAiOjYsIl9GRUFUVVJFX0lEXzMi",
    "OjcsIl9GRUFUVVJFX0lEXzciOjh9LCJpbmRpY2VzIjo5LCJleHRlbnNpb25zIjp7IkVYVF9tZXNoX2ZlYXR1cmVzIjp7ImZlYXR1cmVJZHMiOlt7",
    "ImZlYXR1cmVDb3VudCI6MiwiYXR0cmlidXRlIjowLCJwcm9wZXJ0eVRhYmxlIjowLCJsYWJlbCI6InNvdXJjZV9wcmltaXRpdmUifSx7ImZlYXR1",
    "cmVDb3VudCI6MiwiYXR0cmlidXRlIjozLCJwcm9wZXJ0eVRhYmxlIjoxLCJsYWJlbCI6InNvdXJjZV90cmlhbmdsZSJ9LHsiZmVhdHVyZUNvdW50",
    "IjoyLCJhdHRyaWJ1dGUiOjcsInByb3BlcnR5VGFibGUiOjIsImxhYmVsIjoiaW5kZXBlbmRlbnRfY2xhc3MifV19fX1dfV0sImFjY2Vzc29ycyI6",
    "W3siYnVmZmVyVmlldyI6MCwiY29tcG9uZW50VHlwZSI6NTEyNiwidHlwZSI6IlZFQzMiLCJjb3VudCI6MywibWluIjpbLTAuMCwwLjAsMC4yNV0s",
    "Im1heCI6WzIuMCwxLjAsMC4yNV19LHsiYnVmZmVyVmlldyI6MSwiY29tcG9uZW50VHlwZSI6NTEyNiwidHlwZSI6IlNDQUxBUiIsImNvdW50Ijoz",
    "fSx7ImJ1ZmZlclZpZXciOjIsImNvbXBvbmVudFR5cGUiOjUxMjYsInR5cGUiOiJTQ0FMQVIiLCJjb3VudCI6M30seyJidWZmZXJWaWV3IjozLCJj",
    "b21wb25lbnRUeXBlIjo1MTI2LCJ0eXBlIjoiU0NBTEFSIiwiY291bnQiOjN9LHsiYnVmZmVyVmlldyI6NCwiY29tcG9uZW50VHlwZSI6NTEyMywi",
    "dHlwZSI6IlNDQUxBUiIsImNvdW50IjozfSx7ImJ1ZmZlclZpZXciOjUsImNvbXBvbmVudFR5cGUiOjUxMjMsInR5cGUiOiJWRUMzIiwiY291bnQi",
    "OjMsIm1pbiI6WzAsMCwwXSwibWF4IjpbMTYsOCwwXX0seyJidWZmZXJWaWV3Ijo2LCJjb21wb25lbnRUeXBlIjo1MTI2LCJ0eXBlIjoiU0NBTEFS",
    "IiwiY291bnQiOjN9LHsiYnVmZmVyVmlldyI6NywiY29tcG9uZW50VHlwZSI6NTEyNiwidHlwZSI6IlNDQUxBUiIsImNvdW50IjozfSx7ImJ1ZmZl",
    "clZpZXciOjgsImNvbXBvbmVudFR5cGUiOjUxMjYsInR5cGUiOiJTQ0FMQVIiLCJjb3VudCI6M30seyJidWZmZXJWaWV3Ijo5LCJjb21wb25lbnRU",
    "eXBlIjo1MTIzLCJ0eXBlIjoiU0NBTEFSIiwiY291bnQiOjN9XSwiYnVmZmVyVmlld3MiOlt7ImJ1ZmZlciI6MCwiYnl0ZU9mZnNldCI6MCwiYnl0",
    "ZUxlbmd0aCI6MzYsImJ5dGVTdHJpZGUiOjEyLCJ0YXJnZXQiOjM0OTYyfSx7ImJ1ZmZlciI6MCwiYnl0ZU9mZnNldCI6NDAsImJ5dGVMZW5ndGgi",
    "OjEyLCJ0YXJnZXQiOjM0OTYyfSx7ImJ1ZmZlciI6MCwiYnl0ZU9mZnNldCI6NTYsImJ5dGVMZW5ndGgiOjEyLCJ0YXJnZXQiOjM0OTYyfSx7ImJ1",
    "ZmZlciI6MCwiYnl0ZU9mZnNldCI6NzIsImJ5dGVMZW5ndGgiOjEyLCJ0YXJnZXQiOjM0OTYyfSx7ImJ1ZmZlciI6MCwiYnl0ZU9mZnNldCI6ODgs",
    "ImJ5dGVMZW5ndGgiOjYsInRhcmdldCI6MzQ5NjN9LHsiYnVmZmVyIjowLCJieXRlT2Zmc2V0Ijo5NiwiYnl0ZUxlbmd0aCI6MjQsImJ5dGVTdHJp",
    "ZGUiOjgsInRhcmdldCI6MzQ5NjJ9LHsiYnVmZmVyIjowLCJieXRlT2Zmc2V0IjoxMjAsImJ5dGVMZW5ndGgiOjEyLCJ0YXJnZXQiOjM0OTYyfSx7",
    "ImJ1ZmZlciI6MCwiYnl0ZU9mZnNldCI6MTM2LCJieXRlTGVuZ3RoIjoxMiwidGFyZ2V0IjozNDk2Mn0seyJidWZmZXIiOjAsImJ5dGVPZmZzZXQi",
    "OjE1MiwiYnl0ZUxlbmd0aCI6MTIsInRhcmdldCI6MzQ5NjJ9LHsiYnVmZmVyIjowLCJieXRlT2Zmc2V0IjoxNjgsImJ5dGVMZW5ndGgiOjYsInRh",
    "cmdldCI6MzQ5NjN9LHsiYnVmZmVyIjowLCJieXRlT2Zmc2V0IjoxNzYsImJ5dGVMZW5ndGgiOjh9LHsiYnVmZmVyIjowLCJieXRlT2Zmc2V0Ijox",
    "ODQsImJ5dGVMZW5ndGgiOjh9LHsiYnVmZmVyIjowLCJieXRlT2Zmc2V0IjoxOTIsImJ5dGVMZW5ndGgiOjh9LHsiYnVmZmVyIjowLCJieXRlT2Zm",
    "c2V0IjoyMDAsImJ5dGVMZW5ndGgiOjE2fSx7ImJ1ZmZlciI6MCwiYnl0ZU9mZnNldCI6MjE2LCJieXRlTGVuZ3RoIjoxNn0seyJidWZmZXIiOjAs",
    "ImJ5dGVPZmZzZXQiOjIzMiwiYnl0ZUxlbmd0aCI6OH0seyJidWZmZXIiOjAsImJ5dGVPZmZzZXQiOjI0MCwiYnl0ZUxlbmd0aCI6OH0seyJidWZm",
    "ZXIiOjAsImJ5dGVPZmZzZXQiOjI0OCwiYnl0ZUxlbmd0aCI6MTZ9LHsiYnVmZmVyIjowLCJieXRlT2Zmc2V0IjoyNjQsImJ5dGVMZW5ndGgiOjh9",
    "LHsiYnVmZmVyIjowLCJieXRlT2Zmc2V0IjoyNzIsImJ5dGVMZW5ndGgiOjE2fSx7ImJ1ZmZlciI6MCwiYnl0ZU9mZnNldCI6Mjg4LCJieXRlTGVu",
    "Z3RoIjoxNn0seyJidWZmZXIiOjAsImJ5dGVPZmZzZXQiOjMwNCwiYnl0ZUxlbmd0aCI6OH0seyJidWZmZXIiOjAsImJ5dGVPZmZzZXQiOjMxMiwi",
    "Ynl0ZUxlbmd0aCI6OH0seyJidWZmZXIiOjAsImJ5dGVPZmZzZXQiOjMyMCwiYnl0ZUxlbmd0aCI6MTZ9XSwiYnVmZmVycyI6W3siYnl0ZUxlbmd0",
    "aCI6MzM2fV0sImV4dGVuc2lvbnNVc2VkIjpbIktIUl9tZXNoX3F1YW50aXphdGlvbiIsIkVYVF9tZXNoX2ZlYXR1cmVzIiwiRVhUX3N0cnVjdHVy",
    "YWxfbWV0YWRhdGEiXSwiZXh0ZW5zaW9uc1JlcXVpcmVkIjpbIktIUl9tZXNoX3F1YW50aXphdGlvbiJdLCJleHRlbnNpb25zIjp7IkVYVF9zdHJ1",
    "Y3R1cmFsX21ldGFkYXRhIjp7InNjaGVtYSI6eyJpZCI6ImgyN19pbmRlcGVuZGVudF9hbGxfdGFibGVzX3YxIiwiY2xhc3NlcyI6eyJzb3VyY2Vf",
    "cHJpbWl0aXZlIjp7InByb3BlcnRpZXMiOnsibm9kZV9pbmRleCI6eyJ0eXBlIjoiU0NBTEFSIiwiY29tcG9uZW50VHlwZSI6IlVJTlQzMiIsInJl",
    "cXVpcmVkIjp0cnVlfSwibWVzaF9pbmRleCI6eyJ0eXBlIjoiU0NBTEFSIiwiY29tcG9uZW50VHlwZSI6IlVJTlQzMiIsInJlcXVpcmVkIjp0cnVl",
    "fSwicHJpbWl0aXZlX2luZGV4Ijp7InR5cGUiOiJTQ0FMQVIiLCJjb21wb25lbnRUeXBlIjoiVUlOVDMyIiwicmVxdWlyZWQiOnRydWV9LCJleHBh",
    "bmRlZF9wcmltaXRpdmUiOnsidHlwZSI6IlNDQUxBUiIsImNvbXBvbmVudFR5cGUiOiJVSU5UNjQiLCJyZXF1aXJlZCI6dHJ1ZX0sImVfQSI6eyJ0",
    "eXBlIjoiU0NBTEFSIiwiY29tcG9uZW50VHlwZSI6IkZMT0FUNjQiLCJyZXF1aXJlZCI6dHJ1ZX0sIlYiOnsidHlwZSI6IlNDQUxBUiIsImNvbXBv",
    "bmVudFR5cGUiOiJVSU5UMzIiLCJyZXF1aXJlZCI6dHJ1ZX0sIkYiOnsidHlwZSI6IlNDQUxBUiIsImNvbXBvbmVudFR5cGUiOiJVSU5UMzIiLCJy",
    "ZXF1aXJlZCI6dHJ1ZX19fSwic291cmNlX3RyaWFuZ2xlIjp7InByb3BlcnRpZXMiOnsidHJpYW5nbGVfaW5kZXgiOnsidHlwZSI6IlNDQUxBUiIs",
    "ImNvbXBvbmVudFR5cGUiOiJVSU5UNjQiLCJyZXF1aXJlZCI6dHJ1ZX0sInByaW1pdGl2ZV90YWJsZV9yb3ciOnsidHlwZSI6IlNDQUxBUiIsImNv",
    "bXBvbmVudFR5cGUiOiJVSU5UMzIiLCJyZXF1aXJlZCI6dHJ1ZX19fSwiaW5kZXBlbmRlbnRfY2xhc3MiOnsicHJvcGVydGllcyI6eyJidXNpbmVz",
    "c19pZCI6eyJ0eXBlIjoiU0NBTEFSIiwiY29tcG9uZW50VHlwZSI6IlVJTlQ2NCIsInJlcXVpcmVkIjp0cnVlfSwibGl0ZXJhbF9mYWN0b3IiOnsi",
    "dHlwZSI6IlNDQUxBUiIsImNvbXBvbmVudFR5cGUiOiJGTE9BVDY0IiwicmVxdWlyZWQiOnRydWV9fX0sInByb3h5X3JlZ2lvbnMiOnsicHJvcGVy",
    "dGllcyI6eyJWIjp7InR5cGUiOiJTQ0FMQVIiLCJjb21wb25lbnRUeXBlIjoiVUlOVDMyIiwicmVxdWlyZWQiOnRydWV9LCJGIjp7InR5cGUiOiJT",
    "Q0FMQVIiLCJjb21wb25lbnRUeXBlIjoiVUlOVDMyIiwicmVxdWlyZWQiOnRydWV9LCJzb3VyY2VfcmVnaW9uIjp7InR5cGUiOiJTQ0FMQVIiLCJj",
    "b21wb25lbnRUeXBlIjoiVUlOVDY0IiwicmVxdWlyZWQiOnRydWV9fX19fSwicHJvcGVydHlUYWJsZXMiOlt7ImNsYXNzIjoic291cmNlX3ByaW1p",
    "dGl2ZSIsImNvdW50IjoyLCJwcm9wZXJ0aWVzIjp7Im5vZGVfaW5kZXgiOnsidmFsdWVzIjoxMH0sIm1lc2hfaW5kZXgiOnsidmFsdWVzIjoxMX0s",
    "InByaW1pdGl2ZV9pbmRleCI6eyJ2YWx1ZXMiOjEyfSwiZXhwYW5kZWRfcHJpbWl0aXZlIjp7InZhbHVlcyI6MTN9LCJlX0EiOnsidmFsdWVzIjox",
    "NH0sIlYiOnsidmFsdWVzIjoxNX0sIkYiOnsidmFsdWVzIjoxNn19fSx7ImNsYXNzIjoic291cmNlX3RyaWFuZ2xlIiwiY291bnQiOjIsInByb3Bl",
    "cnRpZXMiOnsidHJpYW5nbGVfaW5kZXgiOnsidmFsdWVzIjoxN30sInByaW1pdGl2ZV90YWJsZV9yb3ciOnsidmFsdWVzIjoxOH19fSx7ImNsYXNz",
    "IjoiaW5kZXBlbmRlbnRfY2xhc3MiLCJjb3VudCI6MiwicHJvcGVydGllcyI6eyJidXNpbmVzc19pZCI6eyJ2YWx1ZXMiOjE5fSwibGl0ZXJhbF9m",
    "YWN0b3IiOnsidmFsdWVzIjoyMH19fSx7ImNsYXNzIjoicHJveHlfcmVnaW9ucyIsImNvdW50IjoyLCJwcm9wZXJ0aWVzIjp7IlYiOnsidmFsdWVz",
    "IjoyMX0sIkYiOnsidmFsdWVzIjoyMn0sInNvdXJjZV9yZWdpb24iOnsidmFsdWVzIjoyM319fV19fX0gUAEAAEJJTgAAAACAAAAAAAAAgD4AAABA",
    "AAAAAAAAgD4AAAAAAACAPwAAgD4AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAABAAIAAAAAAAAA",
    "AAAAABAAAAAAAAAAAAAIAAAAAAAAAIA/AACAPwAAgD8AAAAAAACAPwAAgD8AAIA/AAAAAAAAgD8AAIA/AACAPwAAAAABAAIAAAAAAAUAAAAFAAAA",
    "CwAAAAsAAAAHAAAACQAAADkwAAAAAAAAOjAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAQAAAAAAAAAAAAAA",
    "AQAAAAcAAAAAACAACwAAAAAAAIAAAAAAAADQPwAAAAAAAOg/AAAAAAAAAAAAAAAAAAAAABEAAAAAAAAAEwAAAAAAAAA=",
);
