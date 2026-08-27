//! Orchestrate wrap-or-split → per-leaf GLB → REPLACE tileset → `.3tz`.

use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};

use image::RgbaImage;
use rayon::prelude::*;
use serde_json::{json, Value};

use crate::bbox::{aabb_diagonal, aabb_to_box};
use crate::error::Error;
use crate::georef::{root_transform, Cartographic, RotationDegrees};
use crate::glb_write::{write_glb, TilePrimitive};
use crate::mesh::{self, Scene};
use crate::pack::{pack_named_files, PackOptions};
use crate::split::{self, SplitNode, SplitOpts};
use crate::texture;
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
    eprintln!(
        "mesh-to-3tz: {} triangles, {} images — splitting",
        scene.triangle_count(),
        scene.images.len()
    );

    let tree = split::split(
        &scene,
        &SplitOpts {
            max_triangles: opts.max_triangles,
        },
    );

    let tmp = output.with_extension("mesh-work");
    if tmp.exists() {
        fs::remove_dir_all(&tmp)?;
    }
    fs::create_dir_all(&tmp)?;
    let result = (|| {
        let mut leaves = Vec::new();
        collect_leaves(&tree, 0, &mut leaves);
        eprintln!(
            "mesh-to-3tz: {} leaves — decoding {} textures once each",
            leaves.len(),
            scene.images.len()
        );
        let baked = bake_leaves(&scene, &leaves, opts.tile_size)?;
        eprintln!("mesh-to-3tz: writing leaf GLBs and packing .3tz");

        let glb_files: Vec<(String, PathBuf)> = leaves
            .par_iter()
            .enumerate()
            .map(|(i, (depth, _))| -> Result<(String, PathBuf), Error> {
                let uri = format!("t/{depth}/{i}.glb");
                let path = tmp.join(&uri);
                if let Some(parent) = path.parent() {
                    fs::create_dir_all(parent)?;
                }
                fs::write(&path, write_glb(&baked[i])?)?;
                Ok((uri, path))
            })
            .collect::<Result<Vec<_>, Error>>()?;
        let mut files = glb_files;

        let uris: Vec<String> = files.iter().map(|(u, _)| u.clone()).collect();
        let mut uri_i = 0usize;
        let mut root = emit_tree(&tree, &uris, &mut uri_i)?;
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

fn collect_leaves<'a>(node: &'a SplitNode, depth: u32, out: &mut Vec<(u32, &'a SplitNode)>) {
    match node {
        SplitNode::Leaf { .. } => out.push((depth, node)),
        SplitNode::Branch { children, .. } => {
            for c in children {
                collect_leaves(c, depth + 1, out);
            }
        }
    }
}

fn emit_tree(node: &SplitNode, uris: &[String], uri_i: &mut usize) -> Result<Value, Error> {
    match node {
        SplitNode::Leaf { min, max, .. } => {
            let uri = uris
                .get(*uri_i)
                .ok_or_else(|| Error::msg("leaf/uri mismatch"))?
                .clone();
            *uri_i += 1;
            Ok(json!({
                "boundingVolume": { "box": aabb_to_box(*min, *max) },
                "geometricError": 0.0,
                "content": { "uri": uri },
            }))
        }
        SplitNode::Branch { children, min, max } => {
            let mut kids = Vec::new();
            for c in children {
                kids.push(emit_tree(c, uris, uri_i)?);
            }
            Ok(json!({
                "boundingVolume": { "box": aabb_to_box(*min, *max) },
                "geometricError": aabb_diagonal(*min, *max) / 2.0,
                "children": kids,
            }))
        }
    }
}

/// Decode each unique source image once, crop every leaf that uses it, then drop RGBA.
fn bake_leaves(
    scene: &Scene,
    leaves: &[(u32, &SplitNode)],
    tile_size: u32,
) -> Result<Vec<Vec<TilePrimitive>>, Error> {
    let mut groups: Vec<HashMap<Option<u32>, Vec<usize>>> = Vec::with_capacity(leaves.len());
    let mut needed: HashSet<u32> = HashSet::new();
    for (_, node) in leaves {
        let SplitNode::Leaf { triangle_ids, .. } = node else {
            return Err(Error::msg("collect_leaves returned a branch"));
        };
        let mut g: HashMap<Option<u32>, Vec<usize>> = HashMap::new();
        for &id in triangle_ids {
            let img = scene.triangles[id].image;
            if let Some(i) = img {
                needed.insert(i);
            }
            g.entry(img).or_default().push(id);
        }
        groups.push(g);
    }

    let mut out: Vec<Vec<TilePrimitive>> = vec![Vec::new(); leaves.len()];
    for (i, g) in groups.iter().enumerate() {
        if let Some(ids) = g.get(&None) {
            out[i].push(build_prim(scene, ids, None, tile_size)?);
        }
    }
    let needed: Vec<u32> = needed.into_iter().collect();
    let extras = needed
        .into_par_iter()
        .map(|img_id| -> Result<Vec<(usize, TilePrimitive)>, Error> {
            let rgba = texture::decode_rgba(&scene.images[img_id as usize], tile_size)?;
            let mut local = Vec::new();
            for (i, g) in groups.iter().enumerate() {
                if let Some(ids) = g.get(&Some(img_id)) {
                    local.push((i, build_prim(scene, ids, Some(&rgba), tile_size)?));
                }
            }
            Ok(local)
        })
        .collect::<Result<Vec<_>, Error>>()?;
    for batch in extras {
        for (i, prim) in batch {
            out[i].push(prim);
        }
    }
    Ok(out)
}

fn build_prim(
    scene: &Scene,
    ids: &[usize],
    rgba: Option<&RgbaImage>,
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

    let jpeg = if let Some(img) = rgba {
        let (jpeg, remapped) = texture::crop_leaf(img, &uvs, tile_size)?;
        uvs = remapped;
        Some(jpeg)
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
