//! Shared inputs and recipes for tests that run the built `rusty-tiles`
//! binary once per converter (`cli_contract.rs`, `output_digests.rs`).
//!
//! Every input is generated deterministically from code, so the same recipe
//! list can also be exported for `scripts/compare_outputs.sh`.
#![allow(dead_code)]

use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    fs,
    io::Read,
    path::{Path, PathBuf},
    process::{Command, Output},
};

pub fn bin() -> &'static str {
    env!("CARGO_BIN_EXE_rusty-tiles")
}

pub fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

/// One CLI invocation. `{in}` in an argument is replaced by the input
/// directory; `-o <run dir>/<output>` is appended.
pub struct Recipe {
    pub name: &'static str,
    pub command: &'static str,
    /// Output file or directory name inside the run directory.
    pub output: &'static str,
    pub args: Vec<&'static str>,
    /// Needs a `native-geospatial` build.
    pub native: bool,
    /// Writes vector encoder fingerprints that change with the vector source.
    pub vector: bool,
}

const MODEL_ANCHOR: [&str; 4] = ["--anchor", "133.0", "-12.0", "10"];

fn recipe(
    name: &'static str,
    command: &'static str,
    output: &'static str,
    args: &[&[&'static str]],
    native: bool,
) -> Recipe {
    Recipe {
        name,
        command,
        output,
        args: args.concat(),
        native,
        vector: command == "vector",
    }
}

/// Small, fixed recipes: at least one per converter subcommand.
pub fn recipes() -> Vec<Recipe> {
    let vector = [
        "-i",
        "{in}/vector.geojson",
        "--maxFeatures",
        "2",
        "--jobs",
        "2",
        "--reproducible",
    ];
    let cloud = ["-i", "{in}/cloud.las", "--maxPoints", "500"];
    vec![
        // Lossless keeps the digests independent of the KTX2 encoder's CPU path.
        recipe(
            "mesh",
            "mesh",
            "mesh.3tz",
            &[
                &["-i", "{in}/mesh.glb"],
                &MODEL_ANCHOR,
                &["--profile", "lossless", "--jobs", "1"],
            ],
            false,
        ),
        // Pin the default lossy profile too; omitting --profile exercises the
        // CLI default. Every platform checks repeatability, and the recorded
        // platform additionally pins the encoder's concrete output digests.
        recipe(
            "mesh-balanced",
            "mesh",
            "mesh-balanced.3tz",
            &[
                &["-i", "{in}/mesh-balanced.glb"],
                &MODEL_ANCHOR,
                &["--jobs", "1"],
            ],
            false,
        ),
        recipe(
            "glb-to-3tz",
            "glb-to-3tz",
            "glb.3tz",
            &[&["-i", "{in}/mesh.glb"], &MODEL_ANCHOR],
            false,
        ),
        recipe(
            "createTilesetJson",
            "createTilesetJson",
            "tileset.json",
            &[
                &["-i", "{in}/mesh.glb"],
                &MODEL_ANCHOR,
                &[
                    "--orientation-xyzw",
                    "0",
                    "0",
                    "0.08715574274765817",
                    "0.9961946980917455",
                ],
            ],
            false,
        ),
        recipe(
            "convert",
            "convert",
            "convert.3tz",
            &[&["-i", "{in}/tileset"]],
            false,
        ),
        recipe("vector", "vector", "vector.3tz", &[&vector], true),
        recipe(
            "vector-meshopt-quantize",
            "vector",
            "vector-mq.3tz",
            &[&vector, &["--meshopt", "--quantize"]],
            true,
        ),
        recipe(
            "point-cloud",
            "point-cloud",
            "cloud.3tz",
            &[
                &cloud,
                &["--sourceCrs", "EPSG:32633", "--heightOffset", "0"],
            ],
            true,
        ),
        recipe(
            "point-cloud-local",
            "point-cloud",
            "cloud-local.3tz",
            &[&cloud, &["--sourceCrs", "local"]],
            true,
        ),
        recipe(
            "terrain",
            "terrain",
            "terrain",
            &[&[
                "-i",
                "{in}/dem.tif",
                "--cells-per-leaf",
                "16",
                "--height-offset",
                "10.25",
                "--fill-height",
                "0",
            ]],
            true,
        ),
        recipe(
            "raster-gray",
            "raster",
            "raster-gray",
            &[&[
                "-i",
                "{in}/dem.asc",
                "--minZoom",
                "9",
                "--maxZoom",
                "11",
                "--display",
                "gray",
                "--displayMin",
                "50",
                "--displayMax",
                "200",
            ]],
            true,
        ),
        recipe(
            "raster-rgb",
            "raster",
            "raster-rgb",
            &[&["-i", "{in}/image.tif", "--minZoom", "9", "--maxZoom", "11"]],
            true,
        ),
    ]
}

/// Recipes this build can run.
pub fn enabled_recipes() -> Vec<Recipe> {
    recipes()
        .into_iter()
        .filter(|r| cfg!(feature = "native-geospatial") || !r.native)
        .collect()
}

/// Recipe arguments with `{in}` resolved, without `-o`.
pub fn resolved_args(recipe: &Recipe, inputs: &Path) -> Vec<String> {
    let input = inputs.to_str().unwrap();
    recipe
        .args
        .iter()
        .map(|a| a.replace("{in}", input))
        .collect()
}

/// Run one recipe with an empty PATH (no external tools); `before` options
/// (e.g. `--json`) go ahead of the subcommand.
pub fn run(recipe: &Recipe, inputs: &Path, out_dir: &Path, before: &[&str]) -> Output {
    let mut command = Command::new(bin());
    command.args(before).arg(recipe.command);
    if recipe.command == "createTilesetJson" {
        // This reference artifact has a fixed sibling output. Give this run
        // its own admitted source directory rather than passing a removed -o.
        fs::create_dir_all(out_dir).unwrap();
        fs::copy(inputs.join("mesh.glb"), out_dir.join("mesh.glb")).unwrap();
        let args = resolved_args(recipe, inputs);
        command
            .arg("-i")
            .arg(out_dir.join("mesh.glb"))
            .args(&args[2..]);
    } else {
        command
            .args(resolved_args(recipe, inputs))
            .arg("-o")
            .arg(out_dir.join(recipe.output));
    }
    command.env("PATH", "").output().unwrap()
}

/// Write every input the recipes read into `dir`.
pub fn write_inputs(dir: &Path) {
    fs::create_dir_all(dir.join("tileset")).unwrap();
    let glb = textured_grid_glb(24);
    fs::write(dir.join("mesh.glb"), &glb).unwrap();
    fs::write(dir.join("tileset/mesh.glb"), &glb).unwrap();
    // A nonconstant low-contrast colour control exercises ETC1S itself. The
    // existing XOR-gradient input is retained literally for the older recipes;
    // its default lossy delivery legitimately escalates on an appearance gate.
    let balanced = image::RgbaImage::from_fn(64, 64, |x, y| {
        image::Rgba([
            96 + (x / 8) as u8,
            144 + (y / 8) as u8,
            192 + ((x + y) / 16) as u8,
            255,
        ])
    });
    fs::write(
        dir.join("mesh-balanced.glb"),
        grid_with_texture(8, balanced),
    )
    .unwrap();
    fs::write(
        dir.join("tileset/tileset.json"),
        r#"{"asset":{"version":"1.1"},"geometricError":10,"root":{"boundingVolume":{"box":[10,10,0,10,0,0,0,10,0,0,0,1]},"geometricError":0,"refine":"REPLACE","content":{"uri":"mesh.glb"}}}"#,
    )
    .unwrap();
    fs::copy(
        repo_root().join("tests/fixtures/vector.geojson"),
        dir.join("vector.geojson"),
    )
    .unwrap();
    write_las(&dir.join("cloud.las"), 3000);
    write_dem(&dir.join("dem.asc"), 48);
    fs::write(
        dir.join("dem.tif"),
        include_bytes!("../fixtures/t1-plane.tif"),
    )
    .unwrap();
    #[cfg(feature = "native-geospatial")]
    write_rgb_geotiff(&dir.join("image.tif"), 96);
}

/// A gently curved n×n quad grid (2n² triangles) with a 64×64 PNG texture.
pub fn textured_grid_glb(n: usize) -> Vec<u8> {
    let texture = image::RgbaImage::from_fn(64, 64, |x, y| {
        image::Rgba([(x * 4) as u8, (y * 4) as u8, ((x ^ y) * 4) as u8, 255])
    });
    grid_with_texture(n, texture)
}

fn grid_with_texture(n: usize, texture: image::RgbaImage) -> Vec<u8> {
    let mut png = Vec::new();
    image::DynamicImage::ImageRgba8(texture)
        .write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
        .unwrap();
    let mut positions = Vec::new();
    let mut uvs = Vec::new();
    for j in 0..=n {
        for i in 0..=n {
            let (u, v) = (i as f32 / n as f32, j as f32 / n as f32);
            let z = (u * 7.0).sin() * (v * 5.0).cos() * 0.5;
            positions.push([u * 20.0, v * 20.0, z]);
            uvs.push([u, v]);
        }
    }
    let mut indices = Vec::new();
    for j in 0..n {
        for i in 0..n {
            let a = (j * (n + 1) + i) as u32;
            let c = a + (n + 1) as u32;
            indices.extend([a, a + 1, c, a + 1, c + 1, c]);
        }
    }
    let normals = vec![[0.0, 0.0, 1.0]; positions.len()];
    write_glb(&Primitive {
        positions,
        normals,
        uvs,
        indices,
        png: Some(png),
    })
}

/// One indexed triangle primitive for [`write_glb`].
#[derive(Clone, Debug, Default)]
pub struct Primitive {
    pub positions: Vec<[f32; 3]>,
    /// Empty, or one per position.
    pub normals: Vec<[f32; 3]>,
    /// Written only with a texture, one per position.
    pub uvs: Vec<[f32; 2]>,
    pub indices: Vec<u32>,
    /// Base colour texture bytes (PNG).
    pub png: Option<Vec<u8>>,
}

/// A one-node, one-primitive GLB 2.0 with a double-sided opaque material.
/// Views, accessors and JSON member order reproduce the retired legacy mesh
/// writer byte for byte, so the committed digests of recipes that pack these
/// inputs (`glb-to-3tz`, `createTilesetJson`, `convert`) do not depend on it.
pub fn write_glb(primitive: &Primitive) -> Vec<u8> {
    fn align(bytes: &mut Vec<u8>) {
        bytes.resize(bytes.len().next_multiple_of(4), 0);
    }
    fn number(value: f32) -> String {
        serde_json::to_string(&f64::from(value)).unwrap()
    }
    let Primitive {
        positions,
        normals,
        uvs,
        indices,
        png,
    } = primitive;
    assert!(normals.is_empty() || normals.len() == positions.len());
    let textured = png.is_some() && uvs.len() == positions.len();
    let count = positions.len();
    let mut bin = Vec::new();
    // (byte offset, byte length, target) per view, in accessor order.
    let mut views: Vec<(usize, usize, Option<u32>)> = Vec::new();
    let mut push = |bin: &mut Vec<u8>, data: &[u8], target: Option<u32>| {
        align(bin);
        views.push((bin.len(), data.len(), target));
        bin.extend_from_slice(data);
    };
    let floats = |values: &mut dyn Iterator<Item = f32>| -> Vec<u8> {
        values.flat_map(f32::to_le_bytes).collect()
    };
    push(
        &mut bin,
        &floats(&mut positions.iter().flatten().copied()),
        Some(34962),
    );
    if !normals.is_empty() {
        push(
            &mut bin,
            &floats(&mut normals.iter().flatten().copied()),
            Some(34962),
        );
    }
    if textured {
        push(
            &mut bin,
            &floats(&mut uvs.iter().flatten().copied()),
            Some(34962),
        );
    }
    let short = count <= 65535 && indices.iter().all(|&i| i <= 65535);
    let index_bytes: Vec<u8> = if short {
        indices
            .iter()
            .flat_map(|&i| (i as u16).to_le_bytes())
            .collect()
    } else {
        indices.iter().flat_map(|&i| i.to_le_bytes()).collect()
    };
    push(&mut bin, &index_bytes, Some(34963));
    if let Some(png) = png {
        push(&mut bin, png, None);
    }
    align(&mut bin);

    let mut min = [f32::INFINITY; 3];
    let mut max = [f32::NEG_INFINITY; 3];
    for position in positions {
        for axis in 0..3 {
            min[axis] = min[axis].min(position[axis]);
            max[axis] = max[axis].max(position[axis]);
        }
    }
    let vector = |v: [f32; 3]| v.map(number).join(",");
    let mut accessors = vec![format!(
        r#"{{"bufferView":0,"count":{count},"componentType":5126,"type":"VEC3","min":[{}],"max":[{}]}}"#,
        vector(min),
        vector(max)
    )];
    let mut attributes = vec![r#""POSITION":0"#.to_string()];
    if !normals.is_empty() {
        attributes.push(format!(r#""NORMAL":{}"#, accessors.len()));
        accessors.push(format!(
            r#"{{"bufferView":{},"count":{count},"componentType":5126,"type":"VEC3"}}"#,
            accessors.len()
        ));
    }
    if textured {
        attributes.push(format!(r#""TEXCOORD_0":{}"#, accessors.len()));
        accessors.push(format!(
            r#"{{"bufferView":{},"count":{count},"componentType":5126,"type":"VEC2"}}"#,
            accessors.len()
        ));
    }
    let index_accessor = accessors.len();
    accessors.push(format!(
        r#"{{"bufferView":{index_accessor},"count":{},"componentType":{},"type":"SCALAR"}}"#,
        indices.len(),
        if short { 5123 } else { 5125 }
    ));
    let views = views
        .iter()
        .map(|(offset, length, target)| match target {
            Some(target) => format!(
                r#"{{"buffer":0,"byteLength":{length},"byteOffset":{offset},"target":{target}}}"#
            ),
            None => format!(r#"{{"buffer":0,"byteLength":{length},"byteOffset":{offset}}}"#),
        })
        .collect::<Vec<_>>();
    let (image, colour, sampler) = if png.is_some() {
        (
            format!(
                r#""images":[{{"bufferView":{},"mimeType":"image/png"}}],"#,
                views.len() - 1
            ),
            r#""baseColorFactor":[1.0,1.0,1.0,1.0],"baseColorTexture":{"index":0,"texCoord":0}"#,
            r#""samplers":[{"magFilter":9729,"minFilter":9987,"wrapS":33071,"wrapT":33071}],"#,
        )
    } else {
        (
            String::new(),
            r#""baseColorFactor":[0.22,0.2,0.18,1.0]"#,
            "",
        )
    };
    let textures = if png.is_some() {
        r#","textures":[{"sampler":0,"source":0}]"#
    } else {
        ""
    };
    let mut json = format!(
        concat!(
            r#"{{"accessors":[{}],"asset":{{"generator":"rusty-tiles","version":"2.0"}},"#,
            r#""buffers":[{{"byteLength":{}}}],"bufferViews":[{}],"scene":0,{}"#,
            r#""materials":[{{"alphaMode":"OPAQUE","doubleSided":true,"pbrMetallicRoughness":{{{},"#,
            r#""metallicFactor":0.0,"roughnessFactor":1.0}},"emissiveFactor":[0.0,0.0,0.0]}}],"#,
            r#""meshes":[{{"primitives":[{{"attributes":{{{}}},"indices":{},"material":0}}]}}],"#,
            r#""nodes":[{{"mesh":0}}],{}"scenes":[{{"nodes":[0]}}]{}}}"#
        ),
        accessors.join(","),
        bin.len(),
        views.join(","),
        image,
        colour,
        attributes.join(","),
        index_accessor,
        sampler,
        textures
    )
    .into_bytes();
    json.resize(json.len().next_multiple_of(4), b' ');
    let total = 12 + 8 + json.len() + 8 + bin.len();
    let mut glb = Vec::with_capacity(total);
    glb.extend_from_slice(b"glTF");
    glb.extend_from_slice(&2u32.to_le_bytes());
    glb.extend_from_slice(&(total as u32).to_le_bytes());
    glb.extend_from_slice(&(json.len() as u32).to_le_bytes());
    glb.extend_from_slice(b"JSON");
    glb.extend_from_slice(&json);
    glb.extend_from_slice(&(bin.len() as u32).to_le_bytes());
    glb.extend_from_slice(b"BIN\0");
    glb.extend_from_slice(&bin);
    glb
}

/// LAS 1.4 point format 8 in UTM 33N metres (no CRS record: recipes pass it).
pub fn write_las(path: &Path, count: usize) {
    let mut builder = las::Builder::from((1, 4));
    builder.point_format = las::point::Format::new(8).unwrap();
    builder.transforms = las::Vector {
        x: las::Transform {
            scale: 0.001,
            offset: 500_000.0,
        },
        y: las::Transform {
            scale: 0.001,
            offset: 5_000_000.0,
        },
        z: las::Transform {
            scale: 0.001,
            offset: 0.0,
        },
    };
    let mut writer = las::Writer::from_path(path, builder.into_header().unwrap()).unwrap();
    for i in 0..count {
        writer
            .write_point(las::Point {
                x: 500_000.0 + (i % 61) as f64 * 1.37,
                y: 5_000_000.0 + (i / 61) as f64 * 1.11,
                z: 80.0 + (i as f64 * 0.3).sin() * 5.0,
                intensity: (i * 13) as u16,
                return_number: 1,
                number_of_returns: 1,
                classification: las::point::Classification::new((i % 10) as u8).unwrap(),
                gps_time: Some(i as f64 / 7.),
                color: Some(las::Color {
                    red: (i * 233) as u16,
                    green: (i * 431) as u16,
                    blue: (i * 717) as u16,
                }),
                nir: Some((i * 103) as u16),
                ..Default::default()
            })
            .unwrap();
    }
    writer.close().unwrap();
}

/// An n×n WGS 84 ASCII grid DEM (with one NoData cell) and its `.prj`.
pub fn write_dem(path: &Path, n: usize) {
    let mut text = format!(
        "ncols {n}\nnrows {n}\nxllcorner 12\nyllcorner 41.9\ncellsize {}\nNODATA_value -9999\n",
        0.1 / n as f64
    );
    for y in 0..n {
        let row: Vec<String> = (0..n)
            .map(|x| {
                if x == 3 && y == 5 {
                    "-9999".into()
                } else {
                    let h = 100.0 + 60.0 * (x as f64 * 0.3).sin() * (y as f64 * 0.2).cos();
                    format!("{h:.2}")
                }
            })
            .collect();
        text.push_str(&row.join(" "));
        text.push('\n');
    }
    fs::write(path, text).unwrap();
    fs::write(
        path.with_extension("prj"),
        r#"GEOGCS["WGS 84",DATUM["WGS_1984",SPHEROID["WGS 84",6378137,298.257223563]],PRIMEM["Greenwich",0],UNIT["degree",0.0174532925199433],AUTHORITY["EPSG","4326"]]"#,
    )
    .unwrap();
}

/// An n×n three-band byte GeoTIFF in WGS 84.
#[cfg(feature = "native-geospatial")]
pub fn write_rgb_geotiff(path: &Path, n: usize) {
    use std::{
        ffi::CString,
        ptr::{null, null_mut},
    };
    let filename = CString::new(path.as_os_str().as_encoded_bytes()).unwrap();
    let size = n as i32;
    // SAFETY: the dataset and SRS handles are released by their matching
    // calls, buffers match the written window and all strings are terminated.
    // Registration is not thread-safe; tests in one binary run in parallel.
    static REGISTER: std::sync::Once = std::sync::Once::new();
    REGISTER.call_once(|| unsafe { gdal_sys::GDALAllRegister() });
    unsafe {
        let driver = gdal_sys::GDALGetDriverByName(c"GTiff".as_ptr());
        assert!(!driver.is_null());
        // GDT_Byte / GDT_UInt8 have the stable C enum value 1.
        let dataset = gdal_sys::GDALCreate(driver, filename.as_ptr(), size, size, 3, 1, null_mut());
        assert!(!dataset.is_null());
        let srs = gdal_sys::OSRNewSpatialReference(null());
        assert_eq!(gdal_sys::OSRSetFromUserInput(srs, c"EPSG:4326".as_ptr()), 0);
        let mut wkt = null_mut();
        assert_eq!(gdal_sys::OSRExportToWkt(srs, &mut wkt), 0);
        assert_eq!(gdal_sys::GDALSetProjection(dataset, wkt), 0);
        gdal_sys::VSIFree(wkt.cast());
        gdal_sys::OSRDestroySpatialReference(srs);
        let step = 0.1 / n as f64;
        let mut gt = [12.0, step, 0., 42.0, 0., -step];
        assert_eq!(gdal_sys::GDALSetGeoTransform(dataset, gt.as_mut_ptr()), 0);
        for b in 0..3usize {
            let mut pixels: Vec<u8> = (0..n * n)
                .map(|k| ((k % n) * (b + 1) * 3 + (k / n) * (3 - b) * 2) as u8)
                .collect();
            let band = gdal_sys::GDALGetRasterBand(dataset, b as i32 + 1);
            assert_eq!(
                gdal_sys::GDALRasterIO(
                    band,
                    gdal_sys::GDALRWFlag::GF_Write,
                    0,
                    0,
                    size,
                    size,
                    pixels.as_mut_ptr().cast(),
                    size,
                    size,
                    1,
                    0,
                    0
                ),
                0
            );
        }
        assert_eq!(gdal_sys::GDALClose(dataset), 0);
    }
}

fn sha256(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

/// Vector `tileset.json` / `vector-build.json` carry a hash of the vector
/// encoder source; blank those fields so digests track output, not code.
pub fn normalise_vector_json(bytes: &[u8]) -> Vec<u8> {
    fn blank(value: Option<&mut serde_json::Value>) {
        if let Some(value) = value {
            *value = serde_json::Value::String("<fingerprint>".into());
        }
    }
    let mut value: serde_json::Value = serde_json::from_slice(bytes).unwrap();
    blank(value.pointer_mut("/asset/extras/vectorBuildStateSha256"));
    blank(value.pointer_mut("/config/encoder"));
    if let Some(records) = value
        .get_mut("records")
        .and_then(serde_json::Value::as_object_mut)
    {
        for record in records.values_mut() {
            blank(record.get_mut("signature"));
            // A quantized build's pre-survey signature hashes the encoder too.
            blank(record.get_mut("survey"));
        }
    }
    serde_json::to_vec(&value).unwrap()
}

/// Per-entry sha256 of a `.3tz` (plus `#order`, the digest of the archive's
/// entry order), of every file below a directory, or of one file.
pub fn digests(path: &Path, vector: bool) -> BTreeMap<String, String> {
    let mut out = BTreeMap::new();
    let mut add = |name: String, bytes: Vec<u8>| {
        let leaf = name.rsplit('/').next().unwrap();
        let bytes = if vector && (leaf == "tileset.json" || leaf == "vector-build.json") {
            normalise_vector_json(&bytes)
        } else {
            bytes
        };
        out.insert(name, sha256(&bytes));
    };
    if path.is_dir() {
        for entry in walkdir::WalkDir::new(path).sort_by_file_name() {
            let entry = entry.unwrap();
            if entry.file_type().is_file() {
                let name = entry
                    .path()
                    .strip_prefix(path)
                    .unwrap()
                    .to_string_lossy()
                    .replace('\\', "/");
                add(name, fs::read(entry.path()).unwrap());
            }
        }
    } else if path.extension().is_some_and(|e| e == "3tz") {
        let mut zip = zip::ZipArchive::new(fs::File::open(path).unwrap()).unwrap();
        let mut order = Vec::new();
        for i in 0..zip.len() {
            let mut entry = zip.by_index(i).unwrap();
            let mut bytes = Vec::new();
            entry.read_to_end(&mut bytes).unwrap();
            order.push(entry.name().to_string());
            add(entry.name().to_string(), bytes);
        }
        out.insert("#order".into(), sha256(order.join("\n").as_bytes()));
    } else {
        add(
            path.file_name().unwrap().to_string_lossy().into_owned(),
            fs::read(path).unwrap(),
        );
    }
    out
}

/// The built binary with an empty executable `PATH`, so no helper can run.
pub fn rusty_tiles() -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_rusty-tiles"));
    command.env("PATH", "");
    command
}

/// Run the binary under `umask 022`. The shell is named by absolute path
/// and execs the binary, so `PATH` stays empty.
#[cfg(unix)]
pub fn with_umask_022<S: AsRef<std::ffi::OsStr>>(args: &[S]) -> Output {
    Command::new("/bin/sh")
        .arg("-c")
        .arg("umask 022 && exec \"$0\" \"$@\"")
        .arg(env!("CARGO_BIN_EXE_rusty-tiles"))
        .args(args)
        .env("PATH", "")
        .output()
        .unwrap()
}

/// Permission bits of a published file or directory.
#[cfg(unix)]
pub fn mode(path: &Path) -> u32 {
    use std::os::unix::fs::PermissionsExt;
    fs::metadata(path).unwrap().permissions().mode() & 0o777
}

/// An existing output survives a run without `--force` and a failed forced
/// run. Only a successful forced run replaces it.
pub fn force_replaces_only_successful_output(
    command: &str,
    source: &Path,
    options: &[&str],
    directory: bool,
) {
    let root = tempfile::tempdir().unwrap();
    let out = root.path().join(if directory {
        command.to_owned()
    } else {
        format!("{command}.3tz")
    });
    let previous = if directory {
        fs::create_dir(&out).unwrap();
        out.join("previous")
    } else {
        out.clone()
    };
    fs::write(&previous, b"original").unwrap();
    let run = |input: &Path, extra: &[&str]| {
        rusty_tiles()
            .args([command, "-i"])
            .arg(input)
            .arg("-o")
            .arg(&out)
            .args(options)
            .args(extra)
            .output()
            .unwrap()
    };
    let rejected = run(source, &[]);
    assert_eq!(rejected.status.code(), Some(5), "{command}");
    assert!(
        String::from_utf8_lossy(&rejected.stderr).contains("exists"),
        "{command}: {}",
        String::from_utf8_lossy(&rejected.stderr)
    );
    let bad = root.path().join("bad");
    fs::write(&bad, b"invalid data").unwrap();
    assert!(!run(&bad, &["--force"]).status.success(), "{command}");
    assert_eq!(fs::read(&previous).unwrap(), b"original", "{command}");
    let success = run(source, &["-f"]);
    assert!(
        success.status.success(),
        "{command}: {}",
        String::from_utf8_lossy(&success.stderr)
    );
    if directory {
        assert!(!previous.exists(), "{command}");
        assert!(fs::read_dir(&out).unwrap().next().is_some(), "{command}");
    } else {
        assert_ne!(fs::read(&out).unwrap(), b"original", "{command}");
    }
    // Only the output and the bad source remain: no work or backup directory.
    assert_eq!(fs::read_dir(root.path()).unwrap().count(), 2, "{command}");
}
