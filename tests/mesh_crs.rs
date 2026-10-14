//! General mesh placement is audited from decoded leaf attributes and complete
//! world transforms; expected positions do not call the converter's CRS path.
use rusty_tiles::{
    georef::{Cartographic, SourceAxes},
    tile::{mesh_to_3tz, MeshTo3tzOptions},
    SourceOffset,
};
use serde_json::{json, Value};
use std::{fs, io::Read, path::Path};

const POINTS: [[f32; 3]; 4] = [
    [0., 0., 0.],
    [12.25, -3.5, 8.],
    [25.5, 1., 4.],
    [3., 10., 1.],
];
// Frozen PROJ 9.8.1 cs2cs references: UTM32 WGS84 metres -> WGS84 geocentric,
// with source E=500000.123456789,N=1000000.87654321,A=42.125 and height offset7.5.
// These records include sub-float32 offsets that would be lost by shifting in f32.
#[allow(clippy::excessive_precision)] // Retain the independently printed reference records.
const EXPECTED: [[f64; 3]; 4] = [
    [
        6221813.105511038564,
        985438.511566915433,
        996257.072980326018,
    ],
    [
        6221819.535388029180,
        985451.937732487218,
        996254.872998385574,
    ],
    [
        6221812.860996612348,
        985464.301248833188,
        996258.689885473112,
    ],
    [
        6221812.057762536220,
        985441.384254812961,
        996267.109855561284,
    ],
];

fn fixture(points: &[[f32; 3]], normals: &[[f32; 3]]) -> Vec<u8> {
    let mut bytes = Vec::new();
    for attributes in [points, normals] {
        for point in attributes {
            for component in point {
                bytes.extend(component.to_le_bytes());
            }
        }
    }
    let min: [f32; 3] =
        std::array::from_fn(|i| points.iter().map(|p| p[i]).fold(f32::INFINITY, f32::min));
    let max: [f32; 3] = std::array::from_fn(|i| {
        points
            .iter()
            .map(|p| p[i])
            .fold(f32::NEG_INFINITY, f32::max)
    });
    let offset = bytes.len();
    for index in [0_u16, 1, 2, 0, 2, 3] {
        bytes.extend(index.to_le_bytes());
    }
    let document = json!({
        "asset":{"version":"2.0"}, "scene":0,"scenes":[{"nodes":[0]}],
        "nodes":[{"mesh":0}],"meshes":[{"primitives":[{"attributes":{"POSITION":0,"NORMAL":1},"indices":2}]}],
        "accessors":[
            {"bufferView":0,"componentType":5126,"count":4,"type":"VEC3","min":min,"max":max},
            {"bufferView":1,"componentType":5126,"count":4,"type":"VEC3"},
            {"bufferView":2,"componentType":5123,"count":6,"type":"SCALAR"}],
        "bufferViews":[{"buffer":0,"byteOffset":0,"byteLength":48,"target":34962},
            {"buffer":0,"byteOffset":48,"byteLength":48,"target":34962},
            {"buffer":0,"byteOffset":offset,"byteLength":12,"target":34963}],
        "buffers":[{"byteLength":bytes.len()}]
    });
    encode(&document, bytes)
}
fn encode(document: &Value, mut bytes: Vec<u8>) -> Vec<u8> {
    let mut encoded = serde_json::to_vec(document).unwrap();
    while !encoded.len().is_multiple_of(4) {
        encoded.push(b' ');
    }
    while !bytes.len().is_multiple_of(4) {
        bytes.push(0);
    }
    let mut output = b"glTF".to_vec();
    for header in [
        2_u32,
        (28 + encoded.len() + bytes.len()) as u32,
        encoded.len() as u32,
        0x4e4f534a,
    ] {
        output.extend(header.to_le_bytes());
    }
    output.extend(encoded);
    output.extend((bytes.len() as u32).to_le_bytes());
    output.extend(0x004e4942_u32.to_le_bytes());
    output.extend(bytes);
    output
}

fn bytes(path: &Path, name: &str) -> Vec<u8> {
    let mut archive = zip::ZipArchive::new(fs::File::open(path).unwrap()).unwrap();
    let mut bytes = Vec::new();
    archive
        .by_name(name)
        .unwrap()
        .read_to_end(&mut bytes)
        .unwrap();
    bytes
}
fn identity() -> [f64; 16] {
    [
        1., 0., 0., 0., 0., 1., 0., 0., 0., 0., 1., 0., 0., 0., 0., 1.,
    ]
}
fn multiply(a: [f64; 16], b: [f64; 16]) -> [f64; 16] {
    std::array::from_fn(|i| (0..4).map(|k| a[k * 4 + i % 4] * b[(i / 4) * 4 + k]).sum())
}
fn transform(m: [f64; 16], point: [f64; 3], w: f64) -> [f64; 3] {
    std::array::from_fn(|i| {
        m[i] * point[0] + m[4 + i] * point[1] + m[8 + i] * point[2] + m[12 + i] * w
    })
}
#[derive(Debug)]
struct Leaf {
    triangles: Vec<[[f64; 3]; 3]>,
    normals: Vec<([f64; 3], [f64; 3])>,
}
fn leaves(path: &Path) -> Leaf {
    fn visit(
        path: &Path,
        tile: &Value,
        parent: [f64; 16],
        ancestors: &[([f64; 16], [f64; 12])],
        result: &mut Leaf,
    ) {
        let m = tile["transform"]
            .as_array()
            .map(|a| std::array::from_fn(|i| a[i].as_f64().unwrap()))
            .unwrap_or(identity());
        let world = multiply(parent, m);
        let bounds = tile["boundingVolume"]["box"].as_array().unwrap();
        let bounds: [f64; 12] = std::array::from_fn(|i| bounds[i].as_f64().unwrap());
        for i in [4, 5, 6, 8, 9, 10] {
            assert_eq!(bounds[i], 0.);
        }
        let mut ancestors = ancestors.to_vec();
        ancestors.push((world, bounds));
        if let Some(children) = tile["children"].as_array().filter(|a| !a.is_empty()) {
            for child in children {
                visit(path, child, world, &ancestors, result);
            }
            return;
        }
        let source = bytes(path, tile["content"]["uri"].as_str().unwrap());
        let glb = gltf::Gltf::from_slice(&source).unwrap();
        let buffer = glb.blob.as_deref().unwrap();
        for node in glb.nodes().filter(|n| n.mesh().is_some()) {
            let matrix = node.transform().matrix();
            let local: [f64; 16] = std::array::from_fn(|i| f64::from(matrix[i / 4][i % 4]));
            for primitive in node.mesh().unwrap().primitives() {
                let reader = primitive.reader(|_| Some(buffer));
                let positions: Vec<_> = reader
                    .read_positions()
                    .unwrap()
                    .map(|p| {
                        let p = transform(local, p.map(f64::from), 1.);
                        let point=transform(world, [p[0], -p[2], p[1]], 1.);
                        for (matrix,bounds) in &ancestors {
                            let relative: [f64;3]=std::array::from_fn(|i| point[i]-matrix[12+i]);
                            for axis in 0..3 {
                                // All emitted tile transforms are rotations/translations;
                                // transpose the basis independently to recover local axes.
                                let local=(0..3).map(|i| matrix[axis*4+i]*relative[i]).sum::<f64>();
                                assert!((local-bounds[axis]).abs()<=bounds[3+axis*4]+0.00002,
                                    "decoded world vertex {point:?} outside ancestor bounds {bounds:?}");
                            }
                        }
                        point
                    })
                    .collect();
                let indices: Vec<_> = reader.read_indices().unwrap().into_u32().collect();
                for tri in indices.as_chunks::<3>().0 {
                    result
                        .triangles
                        .push(std::array::from_fn(|i| positions[tri[i] as usize]));
                }
                if let Some(normals) = reader.read_normals() {
                    for (p, n) in positions.into_iter().zip(normals) {
                        let n = transform(local, n.map(f64::from), 0.);
                        result
                            .normals
                            .push((p, transform(world, [n[0], -n[2], n[1]], 0.)));
                    }
                }
            }
        }
    }
    let manifest: Value = serde_json::from_slice(&bytes(path, "tileset.json")).unwrap();
    let expanded =
        rusty_tiles::implicit::expand_tileset(&manifest, |name| Ok(bytes(path, name))).unwrap();
    let mut result = Leaf {
        triangles: Vec::new(),
        normals: Vec::new(),
    };
    visit(path, &expanded["root"], identity(), &[], &mut result);
    result
}
fn close(actual: [f64; 3], expected: [f64; 3], tolerance: f64) {
    for (a, e) in actual.into_iter().zip(expected) {
        assert!((a - e).abs() < tolerance, "{actual:?} != {expected:?}");
    }
}
fn options(definition: &str, axes: SourceAxes) -> MeshTo3tzOptions {
    let mut options = MeshTo3tzOptions {
        source_axes: Some(axes),
        height_offset: Some(7.5),
        source_offset: Some(SourceOffset {
            easting: 500000.123456789,
            northing: 1000000.87654321,
            height: 42.125,
        }),
        max_triangles: 1,
        max_bytes: 0,
        tile_size: 64,
        meshopt: false,
        ..Default::default()
    };
    options.set_source_crs(definition).unwrap();
    options
}
#[test]
fn general_projected_xyz_y_up_and_horizontal_feet_keep_oriented_leaf_geometry() {
    let work = tempfile::tempdir().unwrap();
    for (definition, factor) in [
        ("EPSG:32632", 1.),
        ("+proj=utm +zone=32 +datum=WGS84 +units=ft", 0.3048),
        (
            "+proj=utm +zone=32 +datum=WGS84 +units=us-ft",
            1200. / 3937.,
        ),
    ] {
        for axes in [SourceAxes::Xyz, SourceAxes::YUp] {
            for explicit in [false, true] {
                let points: Vec<_> = POINTS
                    .iter()
                    .map(|p| {
                        let xyz = [p[0] / factor as f32, p[1] / factor as f32, p[2]];
                        match axes {
                            SourceAxes::Xyz => xyz,
                            SourceAxes::YUp => [xyz[0], xyz[2], -xyz[1]],
                        }
                    })
                    .collect();
                let normal = match axes {
                    SourceAxes::Xyz => [0., 0., 1.],
                    SourceAxes::YUp => [0., 1., 0.],
                };
                let source = fixture(&points, &[normal; 4]);
                let input = work.path().join("source.glb");
                let output = work.path().join("output.3tz");
                fs::write(&input, &source).unwrap();
                let mut options = options(definition, axes);
                let offset = options.source_offset.as_mut().unwrap();
                offset.easting /= factor;
                offset.northing /= factor;
                options.explicit = explicit;
                options.force = true;
                mesh_to_3tz(&input, &output, &options).unwrap();
                rusty_tiles::validate_3tz(&output).unwrap();
                assert_eq!(fs::read(&input).unwrap(), source);
                let actual = leaves(&output);
                assert_eq!(actual.triangles.len(), 2);
                let mut seen = [false; 2];
                for tri in actual.triangles {
                    let nearest: Vec<_> = tri
                        .iter()
                        .map(|point| {
                            let index = EXPECTED
                                .iter()
                                .enumerate()
                                .min_by(|(_, a), (_, b)| {
                                    let distance = |p: &[f64; 3]| {
                                        (0..3).map(|i| (point[i] - p[i]).powi(2)).sum::<f64>()
                                    };
                                    distance(a).total_cmp(&distance(b))
                                })
                                .unwrap()
                                .0;
                            close(*point, EXPECTED[index], 0.00002);
                            index
                        })
                        .collect();
                    let index = match nearest.as_slice() {
                        [0, 1, 2] | [1, 2, 0] | [2, 0, 1] => 0,
                        [0, 2, 3] | [2, 3, 0] | [3, 0, 2] => 1,
                        _ => panic!("membership/winding changed: {nearest:?}"),
                    };
                    assert!(!seen[index]);
                    seen[index] = true;
                }
                assert_eq!(seen, [true, true]);
                for (position, normal) in actual.normals {
                    // Ellipsoidal up, independently from the ECEF ellipsoid's gradient.
                    let a = 6378137_f64;
                    let b = a * (1. - 1. / 298.257223563);
                    let gradient = [
                        position[0] / (a * a),
                        position[1] / (a * a),
                        position[2] / (b * b),
                    ];
                    let length = gradient.iter().map(|v| v * v).sum::<f64>().sqrt();
                    close(normal, gradient.map(|v| v / length), 0.000002);
                }
            }
        }
    }
}

#[test]
fn general_mesh_refusals_preserve_prior_output() {
    let work = tempfile::tempdir().unwrap();
    let input = work.path().join("source.glb");
    let output = work.path().join("output.3tz");
    fs::write(&input, fixture(&POINTS, &[[0., 0., 1.]; 4])).unwrap();
    fs::write(&output, b"KEEP").unwrap();
    let base = options("EPSG:32632", SourceAxes::Xyz);
    let cases = [
        (
            MeshTo3tzOptions {
                source_axes: None,
                ..base.clone()
            },
            "source-axes",
        ),
        (
            MeshTo3tzOptions {
                height_offset: None,
                ..base.clone()
            },
            "height-offset",
        ),
        (
            MeshTo3tzOptions {
                height_offset: Some(f64::NAN),
                ..base.clone()
            },
            "finite",
        ),
        (
            MeshTo3tzOptions {
                source_offset: Some(SourceOffset {
                    easting: f64::INFINITY,
                    ..Default::default()
                }),
                ..base.clone()
            },
            "finite",
        ),
        (
            MeshTo3tzOptions {
                cartographic: Some(Cartographic::new(9., 0., 0.)),
                ..base.clone()
            },
            "manual",
        ),
        (
            MeshTo3tzOptions {
                source_crs_definition: Some("EPSG:4978".into()),
                ..base.clone()
            },
            "horizontal",
        ),
        (
            MeshTo3tzOptions {
                source_crs_definition: Some("EPSG:7415".into()),
                ..base.clone()
            },
            if cfg!(feature = "native-geospatial") {
                "horizontal"
            } else {
                "native-geospatial"
            },
        ),
    ];
    for (mut options, reason) in cases {
        options.force = true;
        let error = mesh_to_3tz(&input, &output, &options).unwrap_err();
        assert!(error.to_string().contains(reason), "{error}");
        assert_eq!(fs::read(&output).unwrap(), b"KEEP");
    }
    let mut options = options("EPSG:4326", SourceAxes::Xyz);
    options.force = true;
    options.source_offset = Some(SourceOffset {
        easting: 190.,
        ..Default::default()
    });
    assert!(mesh_to_3tz(&input, &output, &options).is_err());
    assert_eq!(fs::read(&output).unwrap(), b"KEEP");
}

#[test]
fn explicit_axes_choose_general_path_and_legacy_selection_remains_compatible() {
    let mut legacy = MeshTo3tzOptions::default();
    for value in ["auto", "geographic", "epsg:3857", "mercator"] {
        legacy.set_source_crs(value).unwrap();
        assert!(legacy.source_crs_definition.is_none());
    }
    legacy.source_axes = Some(SourceAxes::Xyz);
    legacy.height_offset = Some(0.);
    legacy.set_source_crs("epsg:3857").unwrap();
    assert_eq!(legacy.source_crs_definition.as_deref(), Some("EPSG:3857"));
    assert!(legacy.set_source_crs("auto").is_err());
}

#[test]
fn general_axes_are_applied_after_node_rotation_and_translation() {
    let source = fixture(&POINTS, &[[0., 0., 1.]; 4]);
    let json_length = u32::from_le_bytes(source[12..16].try_into().unwrap()) as usize;
    let mut document: Value = serde_json::from_slice(&source[20..20 + json_length]).unwrap();
    // XYZ source attributes become Y-up after this exact node rotation+shift.
    document["nodes"][0]["matrix"] = json!([1, 0, 0, 0, 0, 0, -1, 0, 0, 1, 0, 0, 7, 11, 13, 1]);
    let source = encode(&document, source[28 + json_length..].to_vec());
    let work = tempfile::tempdir().unwrap();
    let input = work.path().join("node.glb");
    let output = work.path().join("node.3tz");
    fs::write(&input, &source).unwrap();
    let mut options = options("EPSG:32632", SourceAxes::YUp);
    let shift = options.source_offset.as_mut().unwrap();
    shift.easting -= 7.;
    shift.northing += 13.;
    shift.height -= 11.;
    mesh_to_3tz(&input, &output, &options).unwrap();
    assert_eq!(fs::read(input).unwrap(), source);
    let actual = leaves(&output);
    assert_eq!(actual.triangles.len(), 2);
    for point in actual.triangles.into_iter().flatten() {
        assert!(
            EXPECTED
                .iter()
                .any(|reference| (0..3).all(|i| (point[i] - reference[i]).abs() < 0.00002)),
            "{point:?}"
        );
    }
}

// Independent WGS84 ellipsoid equations, not the CRS resolver or ENU bake.
fn geographic_reference(longitude: f64, latitude: f64, height: f64) -> ([f64; 3], [f64; 3]) {
    let lon = longitude.to_radians();
    let lat = latitude.to_radians();
    let a = 6378137.;
    let f = 1. / 298.257223563;
    let e2 = f * (2. - f);
    let radius = a / (1. - e2 * lat.sin().powi(2)).sqrt();
    (
        [
            (radius + height) * lat.cos() * lon.cos(),
            (radius + height) * lat.cos() * lon.sin(),
            (radius * (1. - e2) + height) * lat.sin(),
        ],
        [lat.cos() * lon.cos(), lat.cos() * lon.sin(), lat.sin()],
    )
}

#[test]
fn geographic_domain_edges_keep_decoded_world_positions_and_authored_normals() {
    const GRADS: &str = r#"GEOGCS["WGS 84",DATUM["WGS_1984",SPHEROID["WGS 84",6378137,298.257223563]],PRIMEM["Greenwich",0],UNIT["grad",0.015707963267948967]]"#;
    let work = tempfile::tempdir().unwrap();
    for (definition, factor, longitude_limit, latitude_limit) in [
        ("EPSG:4326", 1., 180_f32, 90_f32),
        (GRADS, 0.9, 200_f32, 100_f32),
    ] {
        let inside_latitude = f32::from_bits(latitude_limit.to_bits() - 1);
        for sign in [-1_f32, 1.] {
            for points in [
                [
                    [sign * longitude_limit, 0., 0.],
                    [sign * (longitude_limit - 0.001), 0., 0.],
                    [sign * longitude_limit, 0.001, 0.],
                    [sign * (longitude_limit - 0.001), 0.001, 0.],
                ],
                [
                    [12., sign * inside_latitude, 0.],
                    [12.001, sign * inside_latitude, 0.],
                    [12., sign * (inside_latitude - 0.001), 0.],
                    [12.001, sign * (inside_latitude - 0.001), 0.],
                ],
            ] {
                for axes in [SourceAxes::Xyz, SourceAxes::YUp] {
                    let positions: Vec<_> = points
                        .iter()
                        .map(|p| match axes {
                            SourceAxes::Xyz => *p,
                            SourceAxes::YUp => [p[0], p[2], -p[1]],
                        })
                        .collect();
                    let normal = match axes {
                        SourceAxes::Xyz => [0., 0., 1.],
                        SourceAxes::YUp => [0., 1., 0.],
                    };
                    for explicit in [false, true] {
                        let input = work.path().join("boundary.glb");
                        let output = work.path().join("boundary.3tz");
                        let source = fixture(&positions, &[normal; 4]);
                        fs::write(&input, &source).unwrap();
                        let mut options = MeshTo3tzOptions {
                            source_axes: Some(axes),
                            height_offset: Some(0.),
                            max_triangles: 1,
                            tile_size: 64,
                            meshopt: false,
                            explicit,
                            force: true,
                            ..Default::default()
                        };
                        options.set_source_crs(definition).unwrap();
                        mesh_to_3tz(&input, &output, &options).unwrap();
                        rusty_tiles::validate_3tz(&output).unwrap();
                        assert_eq!(fs::read(&input).unwrap(), source);
                        let actual = leaves(&output);
                        assert_eq!(actual.triangles.len(), 2);
                        assert!(!actual.normals.is_empty());
                        let references: Vec<_> = points
                            .iter()
                            .map(|p| {
                                geographic_reference(
                                    f64::from(p[0]) * factor,
                                    f64::from(p[1]) * factor,
                                    0.,
                                )
                            })
                            .collect();
                        for (position, normal) in actual.normals {
                            let (expected, expected_normal) = references
                                .iter()
                                .min_by(|a, b| {
                                    let distance = |p: &[f64; 3]| {
                                        (0..3).map(|i| (position[i] - p[i]).powi(2)).sum::<f64>()
                                    };
                                    distance(&a.0).total_cmp(&distance(&b.0))
                                })
                                .unwrap();
                            close(position, *expected, 0.00003);
                            close(normal, *expected_normal, 0.0000003);
                        }
                    }
                }
            }
        }
    }
}

#[test]
fn normal_probe_adjustments_never_accept_invalid_geographic_vertices_or_pole_normals() {
    let work = tempfile::tempdir().unwrap();
    let input = work.path().join("invalid.glb");
    let output = work.path().join("keep.3tz");
    for (longitude, latitude, reason) in [
        (180.01, 0., "outside longitude/latitude"),
        (-180.01, 0., "outside longitude/latitude"),
        (0., 90.01, "outside longitude/latitude"),
        (0., -90.01, "outside longitude/latitude"),
        (0., 90., "singular at a geographic pole"),
        (0., -90., "singular at a geographic pole"),
    ] {
        let points = [[longitude, latitude, 0.]; 4];
        fs::write(&input, fixture(&points, &[[0., 0., 1.]; 4])).unwrap();
        fs::write(&output, b"KEEP").unwrap();
        let mut options = MeshTo3tzOptions {
            source_axes: Some(SourceAxes::Xyz),
            height_offset: Some(0.),
            force: true,
            ..Default::default()
        };
        options.set_source_crs("EPSG:4326").unwrap();
        let error = mesh_to_3tz(&input, &output, &options).unwrap_err();
        assert!(error.to_string().contains(reason), "{error}");
        assert_eq!(fs::read(&output).unwrap(), b"KEEP");
        assert_eq!(fs::read_dir(work.path()).unwrap().count(), 2);
    }
}

#[test]
fn cli_general_mesh_axes_height_and_offset_file_reach_world_reference() {
    let work = tempfile::tempdir().unwrap();
    let input = work.path().join("cli.glb");
    let output = work.path().join("cli.3tz");
    let offset = work.path().join("offset.txt");
    fs::write(&input, fixture(&POINTS, &[[0., 0., 1.]; 4])).unwrap();
    fs::write(
        &offset,
        "E: 500000.123456789\nN: 1000000.87654321\nA: 42.125\n",
    )
    .unwrap();
    let run = |height: bool| {
        let mut command = std::process::Command::new(env!("CARGO_BIN_EXE_rusty-tiles"));
        command
            .args(["mesh-to-3tz", "-i"])
            .arg(&input)
            .arg("-o")
            .arg(&output)
            .args([
                "--sourceCrs",
                "EPSG:32632",
                "--source-axes",
                "xyz",
                "--sourceOffsetFile",
            ])
            .arg(&offset)
            .args([
                "--max-triangles",
                "1",
                "--maxBytes",
                "0",
                "--tile-size",
                "64",
                "--noMeshopt",
                "--texture-format",
                "lossless",
                "--force",
            ]);
        if height {
            command.args(["--height-offset", "7.5"]);
        }
        command.output().unwrap()
    };
    let result = run(true);
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    let result = leaves(&output);
    assert_eq!(result.triangles.len(), 2);
    for point in result.triangles.into_iter().flatten() {
        assert!(
            EXPECTED
                .iter()
                .any(|reference| (0..3).all(|i| (point[i] - reference[i]).abs() < 0.00002)),
            "{point:?}"
        );
    }
    let original = fs::read(&output).unwrap();
    let result = run(false);
    assert_eq!(result.status.code(), Some(3));
    assert!(String::from_utf8_lossy(&result.stderr).contains("--height-offset"));
    assert_eq!(fs::read(output).unwrap(), original);
}
