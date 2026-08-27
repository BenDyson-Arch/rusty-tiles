//! 3D Tiles 1.1 tileset.json creation (createTilesetJson subset).

use std::fs;
use std::path::{Path, PathBuf};

use serde_json::{json, Value};
use walkdir::WalkDir;

use crate::bbox::{aabb_to_box, bounding_box_from_gltf_path, box_to_aabb, union_aabb, BoundingBox};
use crate::error::Error;
use crate::georef::{root_transform, Cartographic, RotationDegrees};

/// Leaf geometric error from 3d-tiles-tools TilesetJsonCreator.
pub const LEAF_GEOMETRIC_ERROR: f64 = 512.0;
/// Tileset geometric error from 3d-tiles-tools TilesetJsonCreator.
pub const TILESET_GEOMETRIC_ERROR: f64 = 4096.0;

#[derive(Clone, Debug, Default)]
pub struct CreateTilesetOptions {
    pub cartographic: Option<Cartographic>,
    pub rotation: Option<RotationDegrees>,
    pub force: bool,
}

pub fn create_tileset_json(
    input: &Path,
    output: &Path,
    opts: &CreateTilesetOptions,
) -> Result<Value, Error> {
    if output.exists() && !opts.force {
        return Err(Error::OutputExists(output.to_path_buf()));
    }
    let contents = collect_contents(input)?;
    if contents.is_empty() {
        return Err(Error::NoContent(input.to_path_buf()));
    }

    let mut leaves: Vec<Value> = Vec::new();
    let mut union_min = [f64::INFINITY; 3];
    let mut union_max = [f64::NEG_INFINITY; 3];

    for (abs, uri) in &contents {
        let boxv = bounding_box_from_gltf_path(abs)?;
        let (mn, mx) = box_to_aabb(boxv);
        let (u0, u1) = union_aabb(union_min, union_max, mn, mx);
        union_min = u0;
        union_max = u1;
        leaves.push(leaf_tile(boxv, uri));
    }

    let mut root = if leaves.len() == 1 {
        leaves.pop().unwrap()
    } else {
        json!({
            "boundingVolume": { "box": aabb_to_box(union_min, union_max) },
            "geometricError": LEAF_GEOMETRIC_ERROR * 2.0,
            "children": leaves,
        })
    };
    root["refine"] = json!("ADD");

    if let Some(pos) = opts.cartographic {
        let xf = root_transform(pos, opts.rotation);
        root["transform"] = json!(xf.to_vec());
    }

    let tileset = json!({
        "asset": { "version": "1.1" },
        "geometricError": TILESET_GEOMETRIC_ERROR,
        "root": root,
    });

    if let Some(parent) = output.parent() {
        if !parent.as_os_str().is_empty() {
            fs::create_dir_all(parent)?;
        }
    }
    let pretty = serde_json::to_vec_pretty(&tileset)?;
    fs::write(output, pretty)?;
    Ok(tileset)
}

fn leaf_tile(boxv: BoundingBox, uri: &str) -> Value {
    json!({
        "boundingVolume": { "box": boxv },
        "geometricError": LEAF_GEOMETRIC_ERROR,
        "content": { "uri": uri },
    })
}

fn collect_contents(input: &Path) -> Result<Vec<(PathBuf, String)>, Error> {
    if !input.exists() {
        return Err(Error::InputNotFound(input.to_path_buf()));
    }
    if input.is_file() {
        if is_gltf(input) {
            let uri = file_name(input)?;
            return Ok(vec![(input.to_path_buf(), uri)]);
        }
        return Err(Error::NoContent(input.to_path_buf()));
    }

    let mut out = Vec::new();
    for entry in WalkDir::new(input).into_iter().filter_map(|e| e.ok()) {
        if !entry.file_type().is_file() {
            continue;
        }
        let p = entry.path();
        if is_gltf(p) {
            let rel = p
                .strip_prefix(input)
                .unwrap_or(p)
                .to_string_lossy()
                .replace('\\', "/");
            out.push((p.to_path_buf(), rel));
        }
    }
    out.sort_by(|a, b| a.1.cmp(&b.1));
    Ok(out)
}

pub(crate) fn is_gltf(p: &Path) -> bool {
    matches!(
        p.extension().and_then(|e| e.to_str()).map(|s| s.to_ascii_lowercase()),
        Some(ref e) if e == "glb" || e == "gltf"
    )
}

fn file_name(p: &Path) -> Result<String, Error> {
    p.file_name()
        .map(|s| s.to_string_lossy().into_owned())
        .ok_or_else(|| Error::msg("path has no file name"))
}

/// Write tileset.json next to the source GLB URI, pack a 3TZ without copying the GLB.
pub fn glb_to_3tz(input: &Path, output: &Path, opts: &CreateTilesetOptions) -> Result<(), Error> {
    if !input.is_file() || !is_gltf(input) {
        return Err(Error::NoContent(input.to_path_buf()));
    }
    if output.exists() && !opts.force {
        return Err(Error::OutputExists(output.to_path_buf()));
    }
    let tmp = output.with_extension("tileset-work");
    if tmp.exists() {
        fs::remove_dir_all(&tmp)?;
    }
    fs::create_dir_all(&tmp)?;
    let name = file_name(input)?;
    let json_path = tmp.join("tileset.json");
    let result = (|| {
        create_tileset_json(input, &json_path, opts)?;
        crate::pack::pack_named_files(
            &[
                ("tileset.json".to_string(), json_path.clone()),
                (name, input.to_path_buf()),
            ],
            output,
            &crate::pack::PackOptions { force: true },
        )
    })();
    let _ = fs::remove_dir_all(&tmp);
    result
}
