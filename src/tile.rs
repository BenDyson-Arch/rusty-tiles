//! Orchestrate wrap-or-split → leaf GLBs → bottom-up HLOD parents → REPLACE
//! tileset → `.3tz`.
//!
//! Pipeline (memory-bounded, every stage parallel):
//! 1. k-d split, grouped into an n-ary tree (`split::split_grouped`).
//! 2. Leaves: clip, plan one chart-packed atlas each (geometry only).
//! 3. Source images decoded a few at a time at the scale the charts need;
//!    charts blitted into leaf atlases; a leaf is encoded + written the
//!    moment its last image passes. Leaf proxies (with WebP) stay in RAM.
//! 4. Parents bottom-up by height: children proxies → weld/simplify → bake
//!    atlas from children atlases → write GLB. Children proxies are dropped
//!    once their grandparent exists.
//! 5. Tight content boxes, two-sided measured GE, JSON, pack.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::Instant;

use image::RgbImage;
use rayon::prelude::*;
use serde_json::{json, Value};

use crate::bbox::{y_up_to_z_up, Obb};
use crate::compress::write_glb_compressed;
use crate::error::Error;
use crate::georef::{root_transform, Cartographic, RotationDegrees, SourceCrs, SourceOffset};
use crate::glb_write::{write_glb, TilePrimitive};
use crate::hlod::{build_parent, MIN_PARENT_GE};
use crate::mesh::{self, Scene};
use crate::pack::{pack_named_files, PackOptions};
use crate::split::{self, ClippedTri, SplitNode, SplitOpts};
use crate::texture::{self, Blit, LeafGroup, LeafPlan};
use crate::tileset::{glb_to_3tz, CreateTilesetOptions};

pub const DEFAULT_MAX_TRIANGLES: usize = 20_000;
pub const DEFAULT_MAX_BYTES: u64 = 204_800;
/// Max atlas edge for leaves and parents.
pub const DEFAULT_TILE_SIZE: u32 = 1024;
/// GE = this × metres-per-texel. Cesium SSE is
/// `GE * height / (distance * 2 tan(fov/2))` and refines when SSE >
/// `maximumScreenSpaceError` (16 in stock Cesium, 4 in the hub). A factor of
/// 16 means one parent texel ≈ one pixel at Cesium's default — factor 2 was
/// tuned for the hub and left Cesium stuck on muddy parents until centimetres
/// from the wall.
const TEXEL_GE_FACTOR: f64 = 16.0;
/// Max source images decoded at once, and the RGB bytes they may occupy
/// together (an 8K JPEG is ~200 MB decoded).
const DECODE_CHUNK_CAP: usize = 8;
const DECODE_BYTES_CAP: u64 = 1 << 30;
/// Leaf texel budget as a fraction of the atlas: chart AABB slack + gutters
/// + packing waste leave roughly this much for real texels.
const LEAF_TEXEL_FILL: f64 = 0.6;
/// 0.5 mm texels — beyond what any screen resolves at arm's length.
pub const DEFAULT_MAX_TEXEL_DENSITY: f64 = 2000.0;

#[derive(Clone, Debug)]
pub struct MeshTo3tzOptions {
    pub cartographic: Option<Cartographic>,
    pub rotation: Option<RotationDegrees>,
    pub force: bool,
    pub max_triangles: usize,
    pub max_bytes: u64,
    pub tile_size: u32,
    /// Source texels per metre kept in leaves; 0 = unlimited.
    pub max_texel_density: f64,
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
            max_texel_density: DEFAULT_MAX_TEXEL_DENSITY,
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

/// Flattened tree node (preorder ids; leaves and parents numbered separately
/// so URIs stay `t/<depth>/<n>.glb` and `t/<depth>/p<n>.glb`).
struct Node {
    depth: u32,
    height: u32,
    children: Vec<usize>,
    cell_min: [f64; 3],
    cell_max: [f64; 3],
    /// Leaf: index into the leaf table.
    leaf: Option<usize>,
    uri: String,
}

struct LeafWork {
    node: usize,
    plan: LeafPlan,
    images_left: Vec<u32>,
    /// Finished proxies (WebP attached); GLB already written.
    out: Option<Vec<TilePrimitive>>,
}

/// Per-node results once its GLB exists.
#[derive(Clone)]
struct Baked {
    prims: Vec<TilePrimitive>,
    /// Tight content box (oriented when that helps), Z-up.
    obb: Obb,
    error_m: f64,
    texel_m: f64,
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

    let image_dims: Vec<(u32, u32)> = referenced_image_dims(&scene)?;
    let tri_texels = triangle_texels(&scene, &image_dims, opts.max_texel_density);
    let max_texels = LEAF_TEXEL_FILL * (opts.tile_size as f64).powi(2);
    let tree = split::split_grouped(
        &scene,
        &SplitOpts {
            max_triangles: opts.max_triangles,
            tri_texels: &tri_texels,
            max_texels,
        },
    );
    drop(tri_texels);
    let (nodes, leaf_ids) = flatten(&tree);
    let max_depth = nodes.iter().map(|n| n.depth).max().unwrap_or(0);
    let max_fanout = nodes.iter().map(|n| n.children.len()).max().unwrap_or(0);
    let leaf_tri_ids: usize = leaf_ids
        .iter()
        .map(|(_, l)| match l {
            SplitNode::Leaf { triangle_ids, .. } => triangle_ids.len(),
            _ => 0,
        })
        .sum();
    log.tick(
        "split",
        format!(
            "leaves={} parents={} depth={max_depth} maxChildren={max_fanout} leafTris={leaf_tri_ids}",
            leaf_ids.len(),
            nodes.len() - leaf_ids.len()
        ),
    );

    let tmp = output.with_extension("mesh-work");
    if tmp.exists() {
        fs::remove_dir_all(&tmp)?;
    }
    fs::create_dir_all(&tmp)?;
    let result = (|| {
        // --- leaves: clip + plan (geometry only) --------------------------
        let mut works: Vec<LeafWork> = leaf_ids
            .par_iter()
            .map(|&(node_i, leaf)| -> Result<LeafWork, Error> {
                let SplitNode::Leaf {
                    triangle_ids,
                    min,
                    max,
                } = leaf
                else {
                    return Err(Error::msg("leaf table holds a branch"));
                };
                let clipped = split::clip_to_cell(&scene, triangle_ids, *min, *max);
                let groups = group_clipped(&clipped);
                let plan = texture::plan_leaf_atlas(
                    groups,
                    &image_dims,
                    opts.tile_size,
                    opts.max_texel_density,
                )?;
                let mut imgs: Vec<u32> = plan.blits.iter().map(|b| b.image).collect();
                imgs.sort_unstable();
                imgs.dedup();
                Ok(LeafWork {
                    node: node_i,
                    plan,
                    images_left: imgs,
                    out: None,
                })
            })
            .collect::<Result<Vec<_>, Error>>()?;
        let scales: Vec<f32> = works
            .iter()
            .filter(|w| w.plan.textured.is_some())
            .map(|w| w.plan.scale)
            .collect();
        let atlas_px: Vec<u32> = works.iter().map(|w| w.plan.atlas_wh.0).collect();
        let half_atlases = works
            .iter()
            .filter(|w| w.plan.atlas_wh.1 > 0 && w.plan.atlas_wh.1 < w.plan.atlas_wh.0)
            .count();
        let planned_tris: usize = works
            .iter()
            .map(|w| {
                w.plan.textured.as_ref().map_or(0, |p| p.indices.len() / 3)
                    + w.plan
                        .untextured
                        .as_ref()
                        .map_or(0, |p| p.indices.len() / 3)
            })
            .sum();
        let fills: Vec<f32> = works
            .iter()
            .filter(|w| w.plan.textured.is_some())
            .map(|w| w.plan.fill)
            .collect();
        let overflow_atlases = works
            .iter()
            .filter(|w| w.plan.atlas_wh.0 > opts.tile_size || w.plan.atlas_wh.1 > opts.tile_size)
            .count();
        log.tick(
            "plan-leaves",
            format!(
                "tris={planned_tris} charts={} tiny={} scale(min/med)={:.2}/{:.2} fill(med)={:.2} atlas(max)={} halfAtlases={half_atlases} overflowAtlases={overflow_atlases}",
                works.iter().map(|w| w.plan.blits.len()).sum::<usize>(),
                works.iter().map(|w| w.plan.tiny_charts).sum::<usize>(),
                scales.iter().cloned().fold(1.0f32, f32::min),
                median_f32(&scales),
                median_f32(&fills),
                atlas_px.iter().max().unwrap_or(&0)
            ),
        );

        // Untextured leaves finish now.
        works
            .par_iter_mut()
            .filter(|w| w.images_left.is_empty())
            .try_for_each(|w| finalize_leaf(w, &nodes, &tmp, opts.meshopt))?;

        // --- leaves: one chunked image pass, charts spilled to disk ------
        // A leaf's charts come from most of the source images, so an atlas
        // would live for the whole pass; with thousands of leaves that is
        // tens of GB. Instead each resampled chart is appended to the leaf's
        // scratch file and the atlas is composed once its last image is done.
        let spill_dir = tmp.join("spill");
        fs::create_dir_all(&spill_dir)?;
        let mut by_image: HashMap<u32, Vec<usize>> = HashMap::new();
        for (li, w) in works.iter().enumerate() {
            for &img in &w.images_left {
                by_image.entry(img).or_default().push(li);
            }
        }
        let mut image_order: Vec<u32> = by_image.keys().copied().collect();
        image_order.sort_unstable();
        // Decode size per image (IDCT-scaled to what its blits need), then
        // chunk by bytes so eight 8K images do not sit in RAM at once.
        let decode_wh: HashMap<u32, (u32, u32)> = image_order
            .iter()
            .map(|&img| {
                let blits: Vec<&Blit> = by_image[&img]
                    .iter()
                    .flat_map(|&li| works[li].plan.blits.iter().filter(move |b| b.image == img))
                    .collect();
                (
                    img,
                    texture::needed_decode_size(&blits, image_dims[img as usize]),
                )
            })
            .collect();
        let mut chunks: Vec<Vec<u32>> = Vec::new();
        let mut cur: Vec<u32> = Vec::new();
        let mut cur_bytes: u64 = 0;
        let max_n = rayon::current_num_threads().clamp(1, DECODE_CHUNK_CAP);
        for &img in &image_order {
            let (w, h) = decode_wh[&img];
            let bytes = 3 * w as u64 * h as u64;
            if !cur.is_empty() && (cur_bytes + bytes > DECODE_BYTES_CAP || cur.len() >= max_n) {
                chunks.push(std::mem::take(&mut cur));
                cur_bytes = 0;
            }
            cur.push(img);
            cur_bytes += bytes;
        }
        if !cur.is_empty() {
            chunks.push(cur);
        }
        let mut decoded_px: u64 = 0;
        for imgs in &chunks {
            let decoded: Vec<(u32, RgbImage)> = imgs
                .par_iter()
                .map(|&img| -> Result<(u32, RgbImage), Error> {
                    let (w, h) = decode_wh[&img];
                    let rgb = texture::decode_rgb_max(&scene.images[img as usize], w, h)?;
                    Ok((img, rgb))
                })
                .collect::<Result<Vec<_>, Error>>()?;
            decoded_px += decoded
                .iter()
                .map(|(_, im)| im.width() as u64 * im.height() as u64)
                .sum::<u64>();
            let lookup: HashMap<u32, &RgbImage> = decoded.iter().map(|(i, im)| (*i, im)).collect();
            let touched: HashSet<usize> = imgs
                .iter()
                .flat_map(|i| by_image[i].iter().copied())
                .collect();
            works
                .par_iter_mut()
                .enumerate()
                .filter(|(li, _)| touched.contains(li))
                .try_for_each(|(_, w)| -> Result<(), Error> {
                    let mut buf: Vec<u8> = Vec::new();
                    for (bi, b) in w.plan.blits.iter().enumerate() {
                        if let Some(src) = lookup.get(&b.image) {
                            let chart = texture::resample_chart(src, b);
                            buf.extend_from_slice(&(bi as u32).to_le_bytes());
                            buf.extend_from_slice(chart.as_raw());
                        }
                    }
                    if !buf.is_empty() {
                        let mut f = fs::OpenOptions::new()
                            .create(true)
                            .append(true)
                            .open(spill_path(&spill_dir, w.node))?;
                        f.write_all(&buf)?;
                    }
                    w.images_left.retain(|i| !lookup.contains_key(i));
                    if w.images_left.is_empty() && w.out.is_none() {
                        finalize_leaf(w, &nodes, &tmp, opts.meshopt)?;
                    }
                    Ok(())
                })?;
        }
        let _ = fs::remove_dir_all(&spill_dir);
        log.tick(
            "bake-leaves",
            format!(
                "images={} decodedMpx={:.0} leafGlbs={}",
                image_order.len(),
                decoded_px as f64 / 1e6,
                works.len()
            ),
        );

        // --- assemble per-node results ------------------------------------
        let mut baked: Vec<Option<Baked>> = (0..nodes.len()).map(|_| None).collect();
        for w in works.iter_mut() {
            let prims = w
                .out
                .take()
                .ok_or_else(|| Error::msg("leaf never finalized"))?;
            let obb = Obb::fit(&prims_points_zup(&prims))
                .unwrap_or_else(|| Obb::from_aabb(nodes[w.node].cell_min, nodes[w.node].cell_max));
            baked[w.node] = Some(Baked {
                prims,
                obb,
                error_m: 0.0,
                texel_m: 0.0,
            });
        }
        drop(works);

        // --- parents, bottom-up by height ---------------------------------
        let root_height = nodes[0].height;
        let mut parent_count = 0usize;
        let mut max_tris = 0usize;
        for h in 1..=root_height {
            let level: Vec<usize> = (0..nodes.len())
                .filter(|&i| nodes[i].leaf.is_none() && nodes[i].height == h)
                .collect();
            let results: Vec<(usize, Baked)> = level
                .par_iter()
                .map(|&ni| -> Result<(usize, Baked), Error> {
                    let node = &nodes[ni];
                    let mut children: Vec<TilePrimitive> = Vec::new();
                    // Own proxy vertices + child box corners: the fit then
                    // encloses every child box, as the spec requires.
                    let mut pts: Vec<[f64; 3]> = Vec::new();
                    for &c in &node.children {
                        let b = baked[c]
                            .as_ref()
                            .ok_or_else(|| Error::msg("child not baked before parent"))?;
                        children.extend(b.prims.iter().cloned());
                        pts.extend_from_slice(&b.obb.corners());
                    }
                    let r = build_parent(&children, opts.max_triangles, opts.tile_size)?;
                    write_tile_glb(&tmp, &node.uri, &r.prims, opts.meshopt)?;
                    pts.extend(prims_points_zup(&r.prims));
                    let obb = Obb::fit(&pts)
                        .unwrap_or_else(|| Obb::from_aabb(node.cell_min, node.cell_max));
                    Ok((
                        ni,
                        Baked {
                            prims: r.prims,
                            obb,
                            error_m: r.error_m,
                            texel_m: r.texel_m,
                        },
                    ))
                })
                .collect::<Result<Vec<_>, Error>>()?;
            for (ni, b) in results {
                max_tris = max_tris.max(prim_tris(&b.prims));
                parent_count += 1;
                baked[ni] = Some(b);
            }
            // Grandchildren proxies are no longer needed.
            if h >= 2 {
                for &ni in &level {
                    for &c in &nodes[ni].children {
                        for &gc in &nodes[c].children {
                            if let Some(b) = baked[gc].as_mut() {
                                b.prims = Vec::new();
                            }
                        }
                    }
                }
            }
        }
        log.tick(
            "hlod",
            format!(
                "parents={parent_count} maxTris={max_tris} levels={root_height} {}",
                crate::hlod::TIMING.summary()
            ),
        );

        // --- tileset.json ---------------------------------------------------
        let files: Vec<(String, PathBuf)> = nodes
            .iter()
            .map(|n| (n.uri.clone(), tmp.join(&n.uri)))
            .collect();
        let (root, root_ge) = emit(&nodes, &baked, 0, true, &opts);
        let tileset = json!({
            "asset": { "version": "1.1" },
            "geometricError": (root_ge * 2.0).max(MIN_PARENT_GE),
            "root": root,
        });
        let json_path = tmp.join("tileset.json");
        fs::write(&json_path, serde_json::to_vec_pretty(&tileset)?)?;
        let mut all = vec![("tileset.json".to_string(), json_path)];
        all.extend(files);
        pack_named_files(&all, output, &PackOptions { force: true })?;
        let out_len = fs::metadata(output).map(|m| m.len()).unwrap_or(0);
        log.tick(
            "write+pack",
            format!("glbs={} rootGE={root_ge:.3} bytes={out_len}", all.len() - 1),
        );
        Ok(())
    })();
    let _ = fs::remove_dir_all(&tmp);
    if result.is_ok() {
        let out_len = fs::metadata(output).map(|m| m.len()).unwrap_or(0);
        log.done(format!("{}  bytes={out_len}", output.display()));
    }
    result
}

/// Preorder flatten. Leaf table pairs node id with the leaf node.
fn flatten(tree: &SplitNode) -> (Vec<Node>, Vec<(usize, &SplitNode)>) {
    let mut nodes = Vec::new();
    let mut leaves = Vec::new();
    let mut leaf_n = 0usize;
    let mut parent_n = 0usize;
    fn walk<'a>(
        n: &'a SplitNode,
        depth: u32,
        nodes: &mut Vec<Node>,
        leaves: &mut Vec<(usize, &'a SplitNode)>,
        leaf_n: &mut usize,
        parent_n: &mut usize,
    ) -> usize {
        let (min, max) = n.aabb();
        let id = nodes.len();
        match n {
            SplitNode::Leaf { .. } => {
                let uri = format!("t/{depth}/{}.glb", *leaf_n);
                *leaf_n += 1;
                nodes.push(Node {
                    depth,
                    height: 0,
                    children: Vec::new(),
                    cell_min: min,
                    cell_max: max,
                    leaf: Some(leaves.len()),
                    uri,
                });
                leaves.push((id, n));
            }
            SplitNode::Branch { children, .. } => {
                let uri = format!("t/{depth}/p{}.glb", *parent_n);
                *parent_n += 1;
                nodes.push(Node {
                    depth,
                    height: 0,
                    children: Vec::new(),
                    cell_min: min,
                    cell_max: max,
                    leaf: None,
                    uri,
                });
                let mut kids = Vec::with_capacity(children.len());
                let mut height = 0;
                for c in children {
                    let cid = walk(c, depth + 1, nodes, leaves, leaf_n, parent_n);
                    height = height.max(nodes[cid].height + 1);
                    kids.push(cid);
                }
                nodes[id].children = kids;
                nodes[id].height = height;
            }
        }
        id
    }
    walk(tree, 0, &mut nodes, &mut leaves, &mut leaf_n, &mut parent_n);
    (nodes, leaves)
}

/// Source texels each triangle covers, capped at `max_density` px/m so an
/// over-photographed patch does not force a forest of tiny leaves.
fn triangle_texels(scene: &Scene, image_dims: &[(u32, u32)], max_density: f64) -> Vec<f32> {
    let cap2 = if max_density > 0.0 {
        max_density * max_density
    } else {
        f64::INFINITY
    };
    scene
        .triangles
        .par_iter()
        .map(|t| {
            let Some(img) = t.image else { return 0.0 };
            let (w, h) = image_dims[img as usize];
            if w == 0 || h == 0 {
                return 0.0;
            }
            let v = [
                &scene.vertices[t.verts[0] as usize],
                &scene.vertices[t.verts[1] as usize],
                &scene.vertices[t.verts[2] as usize],
            ];
            let e1 = sub3(v[1].pos, v[0].pos);
            let e2 = sub3(v[2].pos, v[0].pos);
            let c = cross3(e1, e2);
            let area = 0.5
                * (c[0] as f64 * c[0] as f64
                    + c[1] as f64 * c[1] as f64
                    + c[2] as f64 * c[2] as f64)
                    .sqrt();
            let (u0, v0) = (v[0].uv[0] as f64 * w as f64, v[0].uv[1] as f64 * h as f64);
            let (u1, v1) = (v[1].uv[0] as f64 * w as f64, v[1].uv[1] as f64 * h as f64);
            let (u2, v2) = (v[2].uv[0] as f64 * w as f64, v[2].uv[1] as f64 * h as f64);
            let texels = 0.5 * ((u1 - u0) * (v2 - v0) - (u2 - u0) * (v1 - v0)).abs();
            texels.min(area * cap2) as f32
        })
        .collect()
}

fn sub3(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

fn cross3(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}

fn referenced_image_dims(scene: &Scene) -> Result<Vec<(u32, u32)>, Error> {
    let mut used = vec![false; scene.images.len()];
    for t in &scene.triangles {
        if let Some(i) = t.image {
            if (i as usize) < used.len() {
                used[i as usize] = true;
            }
        }
    }
    (0..scene.images.len())
        .into_par_iter()
        .map(|i| {
            if used[i] {
                texture::image_dimensions(&scene.images[i])
            } else {
                Ok((0, 0))
            }
        })
        .collect()
}

fn group_clipped(tris: &[ClippedTri]) -> Vec<LeafGroup> {
    let mut groups: BTreeMap<Option<u32>, Vec<&ClippedTri>> = BTreeMap::new();
    for t in tris {
        groups.entry(t.image).or_default().push(t);
    }
    groups
        .into_iter()
        .map(|(image, group)| LeafGroup {
            image,
            prim: build_clipped(&group),
        })
        .collect()
}

/// Weld clipped triangles by (position, uv) into one primitive.
fn build_clipped(tris: &[&ClippedTri]) -> TilePrimitive {
    let mut remap: HashMap<([i64; 3], [i64; 2]), u32> = HashMap::new();
    let mut positions = Vec::new();
    let mut normals = Vec::new();
    let mut uvs = Vec::new();
    let mut indices = Vec::new();
    for t in tris {
        let a = t.verts[0].pos;
        let b = t.verts[1].pos;
        let c = t.verts[2].pos;
        if sliver_area2(a, b, c) {
            continue;
        }
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
    TilePrimitive {
        positions,
        normals,
        uvs,
        indices,
        jpeg: None,
    }
}

fn spill_path(spill_dir: &Path, node: usize) -> PathBuf {
    spill_dir.join(format!("{node}.rgb"))
}

/// Compose the atlas from the leaf's spilled charts, encode, write the GLB.
fn finalize_leaf(w: &mut LeafWork, nodes: &[Node], tmp: &Path, meshopt: bool) -> Result<(), Error> {
    let mut prims = Vec::new();
    if let Some(mut t) = w.plan.textured.take() {
        let (aw, ah) = w.plan.atlas_wh;
        let mut atlas = RgbImage::from_pixel(aw.max(1), ah.max(1), image::Rgb([128, 128, 128]));
        let path = spill_path(&tmp.join("spill"), w.node);
        if let Ok(bytes) = fs::read(&path) {
            let mut off = 0usize;
            while off + 4 <= bytes.len() {
                let bi = u32::from_le_bytes(bytes[off..off + 4].try_into().unwrap()) as usize;
                off += 4;
                let Some(b) = w.plan.blits.get(bi) else { break };
                let [_, _, bw, bh] = b.dst;
                let n = (bw.max(1) * bh.max(1) * 3) as usize;
                if off + n > bytes.len() {
                    break;
                }
                if let Some(chart) =
                    RgbImage::from_raw(bw.max(1), bh.max(1), bytes[off..off + n].to_vec())
                {
                    texture::paste_chart(&mut atlas, &chart, b);
                }
                off += n;
            }
            let _ = fs::remove_file(&path);
        }
        t.jpeg = Some(texture::finish_leaf_atlas(atlas, &w.plan.blits)?);
        if t.indices.len() >= 3 {
            prims.push(t);
        }
    }
    if let Some(u) = w.plan.untextured.take() {
        if u.indices.len() >= 3 {
            prims.push(u);
        }
    }
    write_tile_glb(tmp, &nodes[w.node].uri, &prims, meshopt)?;
    w.out = Some(prims);
    Ok(())
}

fn prim_tris(prims: &[TilePrimitive]) -> usize {
    prims.iter().map(|p| p.indices.len() / 3).sum()
}

fn prims_points_zup(prims: &[TilePrimitive]) -> Vec<[f64; 3]> {
    prims
        .iter()
        .flat_map(|p| p.positions.iter())
        .map(|&pos| {
            let z = y_up_to_z_up(pos);
            [z[0] as f64, z[1] as f64, z[2] as f64]
        })
        .collect()
}

fn median_f32(xs: &[f32]) -> f32 {
    if xs.is_empty() {
        return 1.0;
    }
    let mut v = xs.to_vec();
    v.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    v[v.len() / 2]
}

/// Tile JSON + this node's GE. Parent GE = max(measured two-sided error,
/// texel term, child GE + ε) so REPLACE always has a reason to refine and the
/// pyramid never inverts. Boxes are the k-d cells (expanded to cover content),
/// not skin-tight OBBs: Cesium's distance-to-volume is 0 inside the cell, so
/// a camera in a cave actually refines.
fn emit(
    nodes: &[Node],
    baked: &[Option<Baked>],
    ni: usize,
    is_root: bool,
    opts: &MeshTo3tzOptions,
) -> (Value, f64) {
    let node = &nodes[ni];
    let b = baked[ni].as_ref();
    let content = b
        .map(|b| b.obb)
        .filter(|o| o.center[0].is_finite())
        .unwrap_or_else(|| Obb::from_aabb(node.cell_min, node.cell_max));
    let obb = Obb::for_tile(node.cell_min, node.cell_max, content);
    let mut tile = json!({
        "boundingVolume": { "box": obb.to_box() },
    });
    let ge = if node.leaf.is_some() {
        tile["geometricError"] = json!(0.0);
        0.0
    } else {
        let mut kids = Vec::with_capacity(node.children.len());
        let mut child_ge = 0.0f64;
        for &c in &node.children {
            let (kid, g) = emit(nodes, baked, c, false, opts);
            child_ge = child_ge.max(g);
            kids.push(kid);
        }
        let (measured, texel) = b.map(|b| (b.error_m, b.texel_m)).unwrap_or((0.0, 0.0));
        // Average texel size can be optimistic (dense packed charts); also
        // take the atlas stretched across the content's longest edge.
        let span = if texel > 0.0 {
            let longest = 2.0 * content.half.iter().fold(0.0f64, |a, &h| a.max(h));
            let atlas = opts.tile_size.max(1) as f64;
            texel.max(longest / atlas)
        } else {
            texel
        };
        let local = measured.max(span * TEXEL_GE_FACTOR).max(MIN_PARENT_GE);
        let ge = local.max(child_ge * 1.05 + MIN_PARENT_GE);
        tile["geometricError"] = json!(ge);
        tile["children"] = json!(kids);
        ge
    };
    tile["content"] = json!({ "uri": node.uri });
    if is_root {
        if let Some(xf) = root_ecef(opts) {
            tile["transform"] = json!(xf.to_vec());
        }
        tile["refine"] = json!("REPLACE");
    }
    (tile, ge)
}

fn write_tile_glb(
    tmp: &Path,
    uri: &str,
    prims: &[TilePrimitive],
    meshopt: bool,
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
    Ok(())
}

fn root_ecef(opts: &MeshTo3tzOptions) -> Option<[f64; 16]> {
    opts.cartographic
        .map(|pos| root_transform(pos, opts.rotation))
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

/// Squared area of the cross product. Drop clip slivers that survive
/// AREA2_EPS then bloom into white shards after i16 quantization.
fn sliver_area2(a: [f32; 3], b: [f32; 3], c: [f32; 3]) -> bool {
    let u = [b[0] - a[0], b[1] - a[1], b[2] - a[2]];
    let v = [c[0] - a[0], c[1] - a[1], c[2] - a[2]];
    let cr = [
        u[1] * v[2] - u[2] * v[1],
        u[2] * v[0] - u[0] * v[2],
        u[0] * v[1] - u[1] * v[0],
    ];
    cr[0] * cr[0] + cr[1] * cr[1] + cr[2] * cr[2] < 1e-16
}
