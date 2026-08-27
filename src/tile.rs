//! Orchestrate wrap-or-split → per-leaf GLB → REPLACE tileset → `.3tz`.

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

use serde_json::{json, Value};

use crate::bbox::{aabb_diagonal, aabb_to_box};
use crate::error::Error;
use crate::georef::{root_transform, Cartographic, RotationDegrees};
use crate::glb_write::{write_glb, TilePrimitive};
use crate::mesh::{self, Scene};
use crate::pack::{pack_named_files, PackOptions};
use crate::split::{self, SplitNode, SplitOpts};
use crate::texture::{self, DecodedImages};
use crate::tileset::{glb_to_3tz, CreateTilesetOptions};

pub const DEFAULT_MAX_TRIANGLES: usize = 20_000;
pub const DEFAULT_MAX_BYTES: u64 = 204_800;
pub const DEFAULT_TILE_SIZE: u32 = 256;

#[derive(Clone, Debug)]
pub struct MeshTo3tzOptions {
    pub cartographic: Option<Cartographic>,
    pub rotation: Option<RotationDegrees>,
    pub force: bool,
    pub max_triangles: usize,
    pub max_bytes: u64,
    pub tile_size: u32,
}

impl Default for MeshTo3tzOptions {
    fn default() -> Self {
        Self {
            cartographic: None,
            rotation: None,
            force: false,
            max_triangles: DEFAULT_MAX_TRIANGLES,
            max_bytes: DEFAULT_MAX_BYTES,
            tile_size: DEFAULT_TILE_SIZE,
        }
    }
}

impl From<&MeshTo3tzOptions> for CreateTilesetOptions {
    fn from(o: &MeshTo3tzOptions) -> Self {
        Self {
            cartographic: o.cartographic,
            rotation: o.rotation,
            force: o.force,
        }
    }
}

pub fn mesh_to_3tz(input: &Path, output: &Path, opts: &MeshTo3tzOptions) -> Result<(), Error> {
    if !input.is_file() || !crate::tileset::is_gltf(input) {
        return Err(Error::NoContent(input.to_path_buf()));
    }
    if output.exists() && !opts.force {
        return Err(Error::OutputExists(output.to_path_buf()));
    }

    let scene = mesh::load(input)?;
    if scene.under_budget(opts.max_triangles, opts.max_bytes) {
        return glb_to_3tz(input, output, &opts.into());
    }

    let decoded = if scene.images.is_empty() {
        None
    } else {
        Some(DecodedImages::decode(&scene.images)?)
    };
    let tree = split::split(
        &scene,
        &SplitOpts {
            max_triangles: opts.max_triangles,
            max_bytes: opts.max_bytes,
            tile_size: opts.tile_size,
        },
    );

    let tmp = output.with_extension("mesh-work");
    if tmp.exists() {
        fs::remove_dir_all(&tmp)?;
    }
    fs::create_dir_all(&tmp)?;
    let result = (|| {
        let mut files: Vec<(String, PathBuf)> = Vec::new();
        let mut leaf_i = 0u32;
        let root = write_tree(
            &tree,
            &scene,
            decoded.as_ref(),
            opts,
            &tmp,
            &mut files,
            &mut leaf_i,
            0,
        )?;
        let mut root = root;
        root["refine"] = json!("REPLACE");
        if let Some(pos) = opts.cartographic {
            let xf = root_transform(pos, opts.rotation);
            root["transform"] = json!(xf.to_vec());
        }
        let (rmin, rmax) = tree.aabb();
        let tileset = json!({
            "asset": { "version": "1.1" },
            "geometricError": aabb_diagonal(rmin, rmax).max(1e-6),
            "root": root,
        });
        let json_path = tmp.join("tileset.json");
        fs::write(&json_path, serde_json::to_vec_pretty(&tileset)?)?;
        files.insert(0, ("tileset.json".into(), json_path));
        pack_named_files(&files, output, &PackOptions { force: true })
    })();
    let _ = fs::remove_dir_all(&tmp);
    result
}

fn write_tree(
    node: &SplitNode,
    scene: &Scene,
    decoded: Option<&DecodedImages>,
    opts: &MeshTo3tzOptions,
    tmp: &Path,
    files: &mut Vec<(String, PathBuf)>,
    leaf_i: &mut u32,
    depth: u32,
) -> Result<Value, Error> {
    match node {
        SplitNode::Leaf {
            triangle_ids,
            min,
            max,
        } => {
            let prims = leaf_primitives(scene, triangle_ids, decoded, opts.tile_size)?;
            let glb = write_glb(&prims)?;
            let uri = format!("t/{depth}/{leaf_i}.glb");
            *leaf_i += 1;
            let path = tmp.join(&uri);
            if let Some(parent) = path.parent() {
                fs::create_dir_all(parent)?;
            }
            fs::write(&path, glb)?;
            files.push((uri.clone(), path));
            Ok(json!({
                "boundingVolume": { "box": aabb_to_box(*min, *max) },
                "geometricError": 0.0,
                "content": { "uri": uri },
            }))
        }
        SplitNode::Branch { children, min, max } => {
            let mut kids = Vec::new();
            for c in children {
                kids.push(write_tree(
                    c,
                    scene,
                    decoded,
                    opts,
                    tmp,
                    files,
                    leaf_i,
                    depth + 1,
                )?);
            }
            Ok(json!({
                "boundingVolume": { "box": aabb_to_box(*min, *max) },
                "geometricError": aabb_diagonal(*min, *max) / 2.0,
                "children": kids,
            }))
        }
    }
}

fn leaf_primitives(
    scene: &Scene,
    triangle_ids: &[usize],
    decoded: Option<&DecodedImages>,
    tile_size: u32,
) -> Result<Vec<TilePrimitive>, Error> {
    let mut groups: HashMap<Option<u32>, Vec<usize>> = HashMap::new();
    for &id in triangle_ids {
        groups
            .entry(scene.triangles[id].image)
            .or_default()
            .push(id);
    }
    let mut prims = Vec::new();
    for (image, ids) in groups {
        prims.push(build_prim(scene, &ids, image, decoded, tile_size)?);
    }
    Ok(prims)
}

fn build_prim(
    scene: &Scene,
    ids: &[usize],
    image: Option<u32>,
    decoded: Option<&DecodedImages>,
    tile_size: u32,
) -> Result<TilePrimitive, Error> {
    let mut remap: HashMap<u32, u32> = HashMap::new();
    let mut positions = Vec::new();
    let mut normals = Vec::new();
    let mut uvs = Vec::new();
    let mut indices = Vec::new();
    for &id in ids {
        let t = &scene.triangles[id];
        for v in t.verts {
            let n = *remap.entry(v).or_insert_with(|| {
                let vert = &scene.vertices[v as usize];
                let i = positions.len() as u32;
                positions.push(vert.pos);
                normals.push(vert.nrm);
                uvs.push(vert.uv);
                i
            });
            indices.push(n);
        }
    }

    let jpeg = if let (Some(img_id), Some(cache)) = (image, decoded) {
        if let Some(rgba) = cache.get(img_id) {
            let (jpeg, remapped) = texture::crop_leaf(rgba, &uvs, tile_size)?;
            uvs = remapped;
            Some(jpeg)
        } else {
            None
        }
    } else {
        None
    };

    Ok(TilePrimitive {
        positions,
        normals,
        uvs,
        indices,
        jpeg,
    })
}
