//! Orchestrate wrap-or-split → leaf + HLOD parent GLBs → REPLACE tileset → `.3tz`.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::Instant;

use image::RgbaImage;
use rayon::prelude::*;
use serde_json::{json, Value};

use crate::bbox::aabb_to_box;
use crate::compress::write_glb_compressed;
use crate::error::Error;
use crate::georef::{root_transform, Cartographic, RotationDegrees, SourceCrs, SourceOffset};
use crate::glb_write::{write_glb, TilePrimitive};
use crate::hlod::{
    parent_atlas_size, parent_triangle_budget, sampled_hausdorff, simplify_tile,
    spatial_geometric_error, texel_geometric_error, MIN_PARENT_GE,
};
use crate::mesh::{self, Scene};
use crate::pack::{pack_named_files, PackOptions};
use crate::split::{self, ClippedTri, SplitNode, SplitOpts};
use crate::texture;
use crate::tileset::{glb_to_3tz, CreateTilesetOptions};

pub const DEFAULT_MAX_TRIANGLES: usize = 20_000;
pub const DEFAULT_MAX_BYTES: u64 = 204_800;
pub const DEFAULT_TILE_SIZE: u32 = 1024;

#[derive(Clone, Debug)]
pub struct MeshTo3tzOptions {
    pub cartographic: Option<Cartographic>,
    pub rotation: Option<RotationDegrees>,
    pub force: bool,
    pub max_triangles: usize,
    pub max_bytes: u64,
    pub tile_size: u32,
    pub source_crs: SourceCrs,
    pub source_offset: Option<SourceOffset>,
    /// Quantized EXT_meshopt_compression GLBs (default). `--noMeshopt` writes float32.
    pub meshopt: bool,
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
            source_crs: SourceCrs::Auto,
            source_offset: None,
            meshopt: true,
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

struct StageLog {
    t0: Instant,
    last: Instant,
}

impl StageLog {
    fn new() -> Self {
        let t = Instant::now();
        Self { t0: t, last: t }
    }

    fn tick(&mut self, stage: &str, extra: impl std::fmt::Display) {
        let ms = self.last.elapsed().as_secs_f64() * 1000.0;
        eprintln!("mesh-to-3tz: {stage}  {ms:.0} ms  {extra}");
        self.last = Instant::now();
    }

    fn done(&self, extra: impl std::fmt::Display) {
        let ms = self.t0.elapsed().as_secs_f64() * 1000.0;
        eprintln!("mesh-to-3tz: done  {ms:.0} ms  {extra}");
    }
}

pub fn mesh_to_3tz(input: &Path, output: &Path, opts: &MeshTo3tzOptions) -> Result<(), Error> {
    if !input.is_file() || !crate::tileset::is_gltf(input) {
        return Err(Error::NoContent(input.to_path_buf()));
    }
    if output.exists() && !opts.force {
        return Err(Error::OutputExists(output.to_path_buf()));
    }

    let mut log = StageLog::new();
    let mut scene = mesh::load(input)?;
    log.tick(
        "load",
        format!(
            "tris={} images={} bytes={}",
            scene.triangle_count(),
            scene.images.len(),
            scene.source_bytes
        ),
    );

    let mut opts = opts.clone();
    let auto_crs = opts.source_crs == SourceCrs::Auto;
    let baked_geog = mesh::bake_to_enu(
        &mut scene,
        &mesh::BakeToEnu {
            prefer: opts.cartographic,
            crs: opts.source_crs,
            offset: opts.source_offset,
        },
    );
    if let Some(baked) = baked_geog {
        opts.cartographic = Some(baked.origin);
        let stage = match baked.kind {
            crate::georef::CrsKind::Geographic => "geog",
            crate::georef::CrsKind::WebMercator => "mercator",
        };
        if auto_crs && opts.source_offset.is_none() {
            eprintln!(
                "mesh-to-3tz: warning  auto-detected {stage} CRS from AABB; \
                 pass --sourceCrs geographic|epsg:3857 to pin it"
            );
        }
        let extra = if let Some(off) = opts.source_offset {
            format!(
                "lon={:.6} lat={:.6} h={:.3} wgs84={:.6},{:.6},{:.6},{:.6} offset E={} N={} A={} local-frame",
                baked.origin.lon_deg,
                baked.origin.lat_deg,
                baked.origin.height_m,
                baked.bbox_wgs84[0],
                baked.bbox_wgs84[1],
                baked.bbox_wgs84[2],
                baked.bbox_wgs84[3],
                off.easting,
                off.northing,
                off.height
            )
        } else {
            format!(
                "lon={:.6} lat={:.6} h={:.3} wgs84={:.6},{:.6},{:.6},{:.6}",
                baked.origin.lon_deg,
                baked.origin.lat_deg,
                baked.origin.height_m,
                baked.bbox_wgs84[0],
                baked.bbox_wgs84[1],
                baked.bbox_wgs84[2],
                baked.bbox_wgs84[3]
            )
        };
        log.tick(stage, extra);
    }

    // Geographic sources must not wrap the degree-space GLB. Local metre
    // meshes still wrap when under the leaf budget.
    if baked_geog.is_none() && scene.under_budget(opts.max_triangles, opts.max_bytes) {
        glb_to_3tz(input, output, &CreateTilesetOptions::from(&opts))?;
        log.tick("wrap", "under budget");
        let out_len = fs::metadata(output).map(|m| m.len()).unwrap_or(0);
        log.done(format!("{}  bytes={out_len}", output.display()));
        return Ok(());
    }

    let tree = split::split(
        &scene,
        &SplitOpts {
            max_triangles: opts.max_triangles,
        },
    );
    let mut leaves = Vec::new();
    collect_leaves(&tree, 0, &mut leaves);
    let max_depth = leaves.iter().map(|(d, _)| *d).max().unwrap_or(0);
    log.tick(
        "split",
        format!("leaves={} depth={max_depth}", leaves.len()),
    );

    let tmp = output.with_extension("mesh-work");
    if tmp.exists() {
        fs::remove_dir_all(&tmp)?;
    }
    fs::create_dir_all(&tmp)?;
    let result = (|| {
        let decoded = decode_referenced(&scene, opts.tile_size)?;
        let baked = bake_leaves(&scene, &leaves, &decoded, opts.tile_size)?;
        log.tick(
            "bake-leaves",
            format!("textures={} leafGlbs={}", decoded.len(), baked.len()),
        );

        // Full REPLACE pyramid: every branch has a simplified proxy. Parent
        // triangle budgets scale with descendant count so far views stay on
        // a few coarse GLBs; spatial GE floors force refinement when close.
        let mut branches = Vec::new();
        collect_branches(&tree, 0, &mut branches);
        let sampler = texture::SceneSampler::from_scene(&scene, &decoded).ok();
        log.tick(
            "sampler",
            if sampler.is_some() {
                "scene-wide source grid"
            } else {
                "untextured"
            },
        );
        // Near: 1024 baked atlas. Far: 512. Sample the decoded source images.
        let parent_prims: Vec<Vec<TilePrimitive>> = branches
            .par_iter()
            .map(|(_depth, node)| -> Result<Vec<TilePrimitive>, Error> {
                let ids = descendant_ids(node);
                let from_leaf = node.subtree_height();
                let budget = parent_triangle_budget(ids.len(), opts.max_triangles, from_leaf);
                let atlas = parent_atlas_size(from_leaf);
                let (min, max) = node.aabb();
                let clipped = split::clip_to_cell(&scene, &ids, min, max);
                let geom = geom_from_clipped(&clipped);
                simplify_tile(&[geom], sampler.as_ref(), budget, atlas)
            })
            .collect::<Result<Vec<_>, Error>>()?;
        let parent_tris: Vec<usize> = parent_prims.iter().map(|p| prim_tris(p)).collect();
        let max_tris = parent_tris.iter().copied().max().unwrap_or(0);
        let sum_tris: usize = parent_tris.iter().sum();
        log.tick(
            "hlod",
            format!(
                "parents={} maxTris={max_tris} sumTris={sum_tris}",
                parent_prims.len()
            ),
        );

        let ge_cap = opts.max_triangles.saturating_mul(2);
        let measured_ge: Vec<f64> = parent_prims
            .par_iter()
            .zip(branches.par_iter())
            .zip(parent_tris.par_iter())
            .map(|((prims, (_, node)), &ntris)| {
                if ntris > ge_cap {
                    0.0
                } else {
                    let ids = descendant_ids(node);
                    sampled_hausdorff(prims, &scene, &ids)
                }
            })
            .collect();
        log.tick("ge", format!("parents={} cap={ge_cap}", measured_ge.len()));

        let mut files: Vec<(String, PathBuf)> = Vec::new();
        let mut leaf_i = 0usize;
        let mut branch_i = 0usize;
        let (root, root_ge) = emit_and_write(
            &tree,
            0,
            true,
            max_depth,
            &opts,
            &tmp,
            &baked,
            &parent_prims,
            &measured_ge,
            &mut leaf_i,
            &mut branch_i,
            &mut files,
        )?;
        log.tick(
            if opts.meshopt {
                "compress"
            } else {
                "write-glb"
            },
            format!("glbs={} meshopt={}", files.len(), opts.meshopt),
        );

        let tileset = json!({
            "asset": { "version": "1.1" },
            "geometricError": root_ge.max(MIN_PARENT_GE),
            "root": root,
        });
        let json_path = tmp.join("tileset.json");
        fs::write(&json_path, serde_json::to_vec_pretty(&tileset)?)?;
        files.insert(0, ("tileset.json".into(), json_path));
        pack_named_files(&files, output, &PackOptions { force: true })?;
        let out_len = fs::metadata(output).map(|m| m.len()).unwrap_or(0);
        log.tick("write+pack", format!("bytes={out_len}"));
        Ok(())
    })();
    let _ = fs::remove_dir_all(&tmp);
    if result.is_ok() {
        let out_len = fs::metadata(output).map(|m| m.len()).unwrap_or(0);
        log.done(format!("{}  bytes={out_len}", output.display()));
    }
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

fn collect_branches<'a>(node: &'a SplitNode, depth: u32, out: &mut Vec<(u32, &'a SplitNode)>) {
    if let SplitNode::Branch { children, .. } = node {
        out.push((depth, node));
        for c in children {
            collect_branches(c, depth + 1, out);
        }
    }
}

fn prim_tris(prims: &[TilePrimitive]) -> usize {
    prims.iter().map(|p| p.indices.len() / 3).sum()
}

fn descendant_ids(node: &SplitNode) -> Vec<usize> {
    let mut ids = Vec::new();
    collect_descendant_ids(node, &mut ids);
    ids.sort_unstable();
    ids.dedup();
    ids
}

fn collect_descendant_ids(node: &SplitNode, out: &mut Vec<usize>) {
    match node {
        SplitNode::Leaf { triangle_ids, .. } => out.extend_from_slice(triangle_ids),
        SplitNode::Branch { children, .. } => {
            for c in children {
                collect_descendant_ids(c, out);
            }
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn emit_and_write(
    node: &SplitNode,
    depth: u32,
    is_root: bool,
    max_depth: u32,
    opts: &MeshTo3tzOptions,
    tmp: &Path,
    leaves: &[Vec<TilePrimitive>],
    parents: &[Vec<TilePrimitive>],
    measured_ge: &[f64],
    leaf_i: &mut usize,
    branch_i: &mut usize,
    files: &mut Vec<(String, PathBuf)>,
) -> Result<(Value, f64), Error> {
    let (min, max) = node.aabb();
    match node {
        SplitNode::Leaf { .. } => {
            let prims = leaves
                .get(*leaf_i)
                .ok_or_else(|| Error::msg("leaf/prim mismatch"))?;
            *leaf_i += 1;
            let uri = format!("t/{depth}/{}.glb", *leaf_i - 1);
            write_tile_glb(tmp, &uri, prims, opts.meshopt, files)?;
            let mut tile = json!({
                "boundingVolume": { "box": aabb_to_box(min, max) },
                "geometricError": 0.0,
                "content": { "uri": uri },
            });
            if is_root {
                if let Some(xf) = root_ecef(opts) {
                    tile["transform"] = json!(xf.to_vec());
                }
                tile["refine"] = json!("REPLACE");
            }
            Ok((tile, 0.0))
        }
        SplitNode::Branch { children, .. } => {
            let prims = parents
                .get(*branch_i)
                .ok_or_else(|| Error::msg("branch/prim mismatch"))?;
            let measured = measured_ge.get(*branch_i).copied().unwrap_or(MIN_PARENT_GE);
            *branch_i += 1;
            let uri = format!("t/{depth}/p{}.glb", *branch_i - 1);
            write_tile_glb(tmp, &uri, prims, opts.meshopt, files)?;

            let mut kids = Vec::new();
            let mut child_ge = 0.0f64;
            for c in children {
                let (kid, ge) = emit_and_write(
                    c,
                    depth + 1,
                    false,
                    max_depth,
                    opts,
                    tmp,
                    leaves,
                    parents,
                    measured_ge,
                    leaf_i,
                    branch_i,
                    files,
                )?;
                child_ge = child_ge.max(ge);
                kids.push(kid);
            }
            let spatial = spatial_geometric_error(min, max);
            let from_leaf = node.subtree_height();
            let textured = prims.iter().any(|p| p.jpeg.is_some());
            let texel = if textured {
                texel_geometric_error(min, max, parent_atlas_size(from_leaf))
            } else {
                0.0
            };
            // Parent GE must exceed children so Cesium has a reason to refine.
            // `.max(child)` alone flattens the pyramid (137/945 tiles had equal GE).
            let local = measured.max(spatial).max(texel).max(MIN_PARENT_GE);
            let ge = local.max(child_ge * 1.15);
            let mut tile = json!({
                "boundingVolume": { "box": aabb_to_box(min, max) },
                "geometricError": ge,
                "content": { "uri": uri },
                "children": kids,
            });
            if is_root {
                if let Some(xf) = root_ecef(opts) {
                    tile["transform"] = json!(xf.to_vec());
                }
                tile["refine"] = json!("REPLACE");
            }
            Ok((tile, ge))
        }
    }
}

fn write_tile_glb(
    tmp: &Path,
    uri: &str,
    prims: &[TilePrimitive],
    meshopt: bool,
    files: &mut Vec<(String, PathBuf)>,
) -> Result<(), Error> {
    let path = tmp.join(uri);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let bytes = if meshopt {
        write_glb_compressed(prims)?
    } else {
        write_glb(prims)?
    };
    fs::write(&path, bytes)?;
    files.push((uri.to_string(), path));
    Ok(())
}

fn root_ecef(opts: &MeshTo3tzOptions) -> Option<[f64; 16]> {
    opts.cartographic
        .map(|pos| root_transform(pos, opts.rotation))
}

fn decode_referenced(scene: &Scene, tile_size: u32) -> Result<HashMap<u32, RgbaImage>, Error> {
    let needed: Vec<u32> = {
        let mut s = HashSet::new();
        for t in &scene.triangles {
            if let Some(i) = t.image {
                s.insert(i);
            }
        }
        s.into_iter().collect()
    };
    needed
        .into_par_iter()
        .map(|id| {
            let rgba = texture::decode_rgba(&scene.images[id as usize], tile_size)?;
            Ok((id, rgba))
        })
        .collect()
}

fn bake_leaves(
    scene: &Scene,
    leaves: &[(u32, &SplitNode)],
    decoded: &HashMap<u32, RgbaImage>,
    tile_size: u32,
) -> Result<Vec<Vec<TilePrimitive>>, Error> {
    leaves
        .par_iter()
        .map(|(_, node)| {
            let SplitNode::Leaf {
                triangle_ids,
                min,
                max,
            } = node
            else {
                return Err(Error::msg("collect_leaves returned a branch"));
            };
            let clipped = split::clip_to_cell(scene, triangle_ids, *min, *max);
            bake_clipped(&clipped, decoded, tile_size)
        })
        .collect()
}

fn geom_from_clipped(tris: &[ClippedTri]) -> TilePrimitive {
    let mut remap: HashMap<[i64; 3], u32> = HashMap::new();
    let mut positions = Vec::new();
    let mut normals = Vec::new();
    let mut indices = Vec::new();
    for t in tris {
        for v in &t.verts {
            let k = pos_key(v.pos);
            let n = *remap.entry(k).or_insert_with(|| {
                let i = positions.len() as u32;
                positions.push(v.pos);
                normals.push(v.nrm);
                i
            });
            indices.push(n);
        }
    }
    TilePrimitive {
        positions,
        normals,
        uvs: Vec::new(),
        indices,
        jpeg: None,
    }
}

fn bake_clipped(
    tris: &[ClippedTri],
    decoded: &HashMap<u32, RgbaImage>,
    tile_size: u32,
) -> Result<Vec<TilePrimitive>, Error> {
    let mut groups: BTreeMap<Option<u32>, Vec<&ClippedTri>> = BTreeMap::new();
    for t in tris {
        groups.entry(t.image).or_default().push(t);
    }
    let mut out = Vec::new();
    for (img, group) in groups {
        let rgba = img.and_then(|i| decoded.get(&i));
        out.push(build_clipped(&group, rgba, tile_size)?);
    }
    Ok(out)
}

fn build_clipped(
    tris: &[&ClippedTri],
    rgba: Option<&RgbaImage>,
    tile_size: u32,
) -> Result<TilePrimitive, Error> {
    let mut remap: HashMap<([i64; 3], [i64; 2]), u32> = HashMap::new();
    let mut positions = Vec::new();
    let mut normals = Vec::new();
    let mut uvs = Vec::new();
    let mut indices = Vec::new();
    for t in tris {
        for v in &t.verts {
            let k = (pos_key(v.pos), uv_key(v.uv));
            let n = *remap.entry(k).or_insert_with(|| {
                let i = positions.len() as u32;
                positions.push(v.pos);
                normals.push(v.nrm);
                uvs.push(v.uv);
                i
            });
            indices.push(n);
        }
    }

    let jpeg = if let Some(img) = rgba {
        let (jpeg, remapped) = texture::crop_leaf(img, &uvs, &indices, tile_size)?;
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

fn pos_key(p: [f32; 3]) -> [i64; 3] {
    [
        (p[0] as f64 * 1e7).round() as i64,
        (p[1] as f64 * 1e7).round() as i64,
        (p[2] as f64 * 1e7).round() as i64,
    ]
}

fn uv_key(uv: [f32; 2]) -> [i64; 2] {
    [
        (uv[0] as f64 * 1e7).round() as i64,
        (uv[1] as f64 * 1e7).round() as i64,
    ]
}
