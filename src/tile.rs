//! Spatial leaves and remeshed, retextured replacement LODs.
//! Each original triangle belongs to exactly one leaf. Leaf charts copy source
//! texels on the integer pixel grid; only coarse LODs reduce resolution.
use crate::{
    bbox::aabb_to_box,
    error::Error,
    georef::{root_transform, Cartographic, RotationDegrees, SourceCrs, SourceOffset},
    glb_write::TilePrimitive,
    mesh::{self, Scene},
    output::Job,
    report::{ConversionResult, Reporter},
    texture::{self, LeafGroup, LeafPlan},
    tileset::CreateTilesetOptions,
};
use image::{ImageEncoder, RgbaImage};
use rayon::prelude::*;
use serde_json::{json, Value};
use std::{
    collections::{BTreeMap, HashMap},
    fs,
    io::{Read, Write},
    path::Path,
    time::Instant,
};
pub const DEFAULT_MAX_TRIANGLES: usize = 20_000;
pub const DEFAULT_MAX_BYTES: u64 = 204_800;
/// Max atlas edge for leaves and parents.
pub const DEFAULT_TILE_SIZE: u32 = 2048;
/// Preserve every source texel by default.
pub const DEFAULT_MAX_TEXEL_DENSITY: f64 = 0.0;

#[derive(Clone, Debug)]
pub struct MeshTo3tzOptions {
    /// Keep the legacy median-split explicit hierarchy and output bytes.
    pub explicit: bool,
    pub texture_format: TextureFormat,
    pub basisu: std::path::PathBuf,
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
    /// Lossless EXT_meshopt_compression; float32 attributes in either mode.
    pub meshopt: bool,
}

impl Default for MeshTo3tzOptions {
    fn default() -> Self {
        Self {
            explicit: false,
            texture_format: TextureFormat::Lossless,
            basisu: "basisu".into(),
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

#[derive(Clone, Copy, Debug, clap::ValueEnum, PartialEq, Eq)]
pub enum TextureFormat {
    Lossless,
    Jpeg,
    Webp,
    Uastc,
}

/// Material, textured, has-normals.
type GroupKey = (usize, bool, bool);
struct Planned {
    material: usize,
    plan: LeafPlan,
}
struct Node {
    slot: usize,
    children: Vec<Node>,
    plans: Vec<Planned>,
    min: [f64; 3],
    max: [f64; 3],
    id: usize,
    error: f64,
}
#[derive(Clone)]
struct Piece {
    material: usize,
    prim: TilePrimitive,
    delivery_image: Option<Vec<u8>>,
}
struct Proxy {
    pieces: Vec<Piece>,
    error: f64,
    geometry_error: f64,
}

pub fn mesh_to_3tz(input: &Path, output: &Path, opts: &MeshTo3tzOptions) -> Result<(), Error> {
    mesh_to_3tz_reported(input, output, opts, &Reporter::default()).map(drop)
}

/// [`mesh_to_3tz`] with stage timings sent to `reporter` as notes, returning
/// the published result.
pub fn mesh_to_3tz_reported(
    input: &Path,
    output: &Path,
    opts: &MeshTo3tzOptions,
    reporter: &Reporter,
) -> Result<ConversionResult, Error> {
    if opts.max_triangles == 0
        || !opts.tile_size.is_power_of_two()
        || !(64..=4096).contains(&opts.tile_size)
    {
        return Err(Error::msg(
            "maxTriangles must be positive; tileSize must be a power of two from 64 to 4096",
        ));
    }
    if !opts.max_texel_density.is_finite() || opts.max_texel_density != 0.0 {
        return Err(Error::msg("the fidelity-preserving pipeline requires maxTexelDensity=0; coarse LODs reduce resolution automatically"));
    }
    let job = Job::begin(output, opts.force)?;
    if opts.texture_format == TextureFormat::Uastc {
        crate::gpu_texture::check(&opts.basisu)?;
    }
    let start = Instant::now();
    if opts.texture_format == TextureFormat::Jpeg {
        reporter.note(&format!(
            "mesh-to-3tz: {} (quality 95, full chroma)",
            crate::jpeg::backend()
        ));
    }
    let mut scene = mesh::load(input)?;
    let baked = mesh::bake_to_enu(
        &mut scene,
        &mesh::BakeToEnu {
            prefer: opts.cartographic,
            crs: opts.source_crs,
            offset: opts.source_offset,
        },
    );
    if baked.is_none() && scene.under_budget(opts.max_triangles, opts.max_bytes) {
        if opts.explicit {
            return crate::tileset::glb_job(input, job, &CreateTilesetOptions::from(opts));
        }
        let mut manifest = crate::tileset::create_tileset_json(
            input,
            &job.path().join("tileset.json"),
            &CreateTilesetOptions::from(opts),
        )?;
        fs::copy(
            input,
            job.path().join(
                input
                    .file_name()
                    .ok_or_else(|| Error::msg("input has no filename"))?,
            ),
        )?;
        crate::implicit::write_tileset(
            &mut manifest,
            job.path(),
            crate::implicit::SubdivisionScheme::Octree,
            false,
        )?;
        fs::write(
            job.path().join("tileset.json"),
            serde_json::to_vec(&manifest)?,
        )?;
        let work = job.path().to_owned();
        let report = json!({"encoder":"rusty-tiles-native-mesh-implicit-v2","tiling":"implicit","tiles":1,"leafTiles":1});
        crate::output::write_report(&work, report.clone(), true)?;
        return job.publish_tree_3tz(&work, Some(report));
    }
    validate_source(input)?;
    if scene.vertices.iter().any(|v| {
        v.uv.iter()
            .any(|u| !u.is_finite() || !(0.0..=1.0).contains(u))
    }) {
        return Err(Error::msg(
            "UV coordinates outside [0,1] require an unchanged glb-to-3tz wrap",
        ));
    }
    let (materials, material_ids) = materials(&scene)?;
    let wraps = image_wraps(input, scene.images.len())?;
    let dims = scene
        .images
        .iter()
        .map(texture::image_dimensions)
        .collect::<Result<Vec<_>, _>>()?;
    reporter.note(&format!(
        "mesh-to-3tz: load {:.2}s triangles={} images={}",
        start.elapsed().as_secs_f64(),
        scene.triangles.len(),
        dims.len()
    ));
    let mut root = partition(
        &scene,
        &dims,
        &material_ids,
        (0..scene.triangles.len()).collect(),
        opts,
        None,
        0,
    )?;
    if opts.explicit {
        fold(&mut root);
    }
    let mut nodes = Vec::new();
    flatten(&mut root, &mut nodes);
    let leaves = nodes.iter().filter(|n| n.children.is_empty()).count();
    reporter.note(&format!(
        "mesh-to-3tz: plan {:.2}s leaves={} nodes={}",
        start.elapsed().as_secs_f64(),
        leaves,
        nodes.len()
    ));
    let work = job.path();
    fs::create_dir(work.join("t"))?;
    let mut pieces: Vec<Vec<Piece>> = (0..nodes.len()).map(|_| Vec::new()).collect();
    // Decode a bounded batch in parallel, then spill each image's charts.
    // Writes to a leaf's spill file stay ordered and cannot interleave.
    let decode_batch = rayon::current_num_threads().clamp(1, 4);
    for (batch, encoded) in scene.images.chunks(decode_batch).enumerate() {
        let decoded = encoded
            .par_iter()
            .map(|image| -> Result<_, Error> { crate::jpeg::decode(&image.load()?) })
            .collect::<Result<Vec<_>, _>>()?;
        for (within, source) in decoded.into_iter().enumerate() {
            let image_id = batch * decode_batch + within;
            let touched: Vec<_> = nodes
                .iter()
                .enumerate()
                .flat_map(|(ni, n)| n.plans.iter().enumerate().map(move |(pi, p)| (ni, pi, p)))
                .filter(|(_, _, p)| p.plan.blits.iter().any(|b| b.image as usize == image_id))
                .collect();
            if touched.is_empty() {
                continue;
            }
            touched
                .par_iter()
                .try_for_each(|&(ni, pi, p)| -> Result<(), Error> {
                    let mut file = fs::OpenOptions::new()
                        .create(true)
                        .append(true)
                        .open(work.join(format!("{ni}-{pi}.pixels")))?;
                    for (bi, b) in p
                        .plan
                        .blits
                        .iter()
                        .enumerate()
                        .filter(|(_, b)| b.image as usize == image_id)
                    {
                        let [_, _, w, h] = b.dst;
                        let x0 = (b.src[0] as f64 * source.width() as f64).round() as i64;
                        let y0 = (b.src[1] as f64 * source.height() as f64).round() as i64;
                        let mut pixels = Vec::with_capacity(w as usize * h as usize * 4);
                        for y in 0..h {
                            let sy = wrap(y0 + y as i64, source.height(), wraps[image_id].1);
                            if x0 >= 0 && x0 + w as i64 <= source.width() as i64 {
                                let offset =
                                    (sy as usize * source.width() as usize + x0 as usize) * 4;
                                pixels.extend_from_slice(
                                    &source.as_raw()[offset..offset + w as usize * 4],
                                );
                                continue;
                            }
                            for x in 0..w {
                                // Source sampler wrap is applied to chart gutters too.
                                let sx = wrap(x0 + x as i64, source.width(), wraps[image_id].0);
                                let sy = wrap(y0 + y as i64, source.height(), wraps[image_id].1);
                                pixels.extend_from_slice(&source.get_pixel(sx, sy).0);
                            }
                        }
                        file.write_all(&(bi as u32).to_le_bytes())?;
                        file.write_all(&pixels)?;
                    }
                    Ok(())
                })?;
        }
    }
    reporter.note(&format!(
        "mesh-to-3tz: chart extraction {:.2}s",
        start.elapsed().as_secs_f64()
    ));
    drop(scene);
    nodes
        .par_iter_mut()
        .zip(pieces.par_iter_mut())
        .try_for_each(|(node, out)| -> Result<(), Error> {
            for (pi, p) in node.plans.drain(..).enumerate() {
                let mut prim = p
                    .plan
                    .textured
                    .or(p.plan.untextured)
                    .ok_or_else(|| Error::msg("empty leaf plan"))?;
                let mut delivery_image = None;
                if !p.plan.blits.is_empty() {
                    let mut atlas = RgbaImage::new(p.plan.atlas_wh.0, p.plan.atlas_wh.1);
                    let path = work.join(format!("{}-{pi}.pixels", node.id));
                    let mut file = fs::File::open(&path)?;
                    let mut seen = vec![false; p.plan.blits.len()];
                    for _ in 0..p.plan.blits.len() {
                        let mut id = [0; 4];
                        file.read_exact(&mut id)?;
                        let bi = u32::from_le_bytes(id) as usize;
                        if bi >= seen.len() || seen[bi] {
                            return Err(Error::msg("invalid chart spill"));
                        }
                        seen[bi] = true;
                        let [x, y, w, h] = p.plan.blits[bi].dst;
                        let mut data = vec![0; w as usize * h as usize * 4];
                        file.read_exact(&mut data)?;
                        let chart = RgbaImage::from_raw(w, h, data).unwrap();
                        image::imageops::replace(&mut atlas, &chart, x as i64, y as i64);
                    }
                    fill_background(
                        &mut atlas,
                        &p.plan.blits.iter().map(|b| b.dst).collect::<Vec<_>>(),
                    );
                    delivery_image = Some(encode_delivery(
                        &atlas,
                        work,
                        &format!("gpu-{}-{pi}", node.id),
                        &materials[p.material],
                        opts,
                    )?);
                    if materials[p.material]["alphaMode"]
                        .as_str()
                        .unwrap_or("OPAQUE")
                        == "OPAQUE"
                    {
                        // Alpha is ignored by the material. Premultiplied
                        // filtering must not erase otherwise-visible RGB.
                        for pixel in atlas.pixels_mut() {
                            pixel[3] = 255;
                        }
                    }
                    // Parents need a filtered working image, not the entire
                    // full-resolution leaf. Keep final delivery independent.
                    let divisor = (atlas.width().max(atlas.height()) / 1024).max(1);
                    let proxy_image = if divisor > 1 {
                        resize_colour(&atlas, atlas.width() / divisor, atlas.height() / divisor)
                    } else {
                        atlas
                    };
                    prim.jpeg = Some(encode_png(&proxy_image)?);
                    fs::remove_file(path)?;
                }
                out.push(Piece {
                    material: p.material,
                    prim,
                    delivery_image,
                });
            }
            if node.children.is_empty() {
                write_node(work, node.id, out, &materials, opts)?;
                spill_images(work, node.id, out)?;
            }
            Ok(())
        })?;
    reporter.note(&format!(
        "mesh-to-3tz: leaves {:.2}s",
        start.elapsed().as_secs_f64()
    ));
    let mut proxies: Vec<Option<Proxy>> = pieces
        .into_iter()
        .enumerate()
        .map(|(i, pieces)| {
            nodes[i].children.is_empty().then_some(Proxy {
                pieces,
                error: 0.0,
                geometry_error: 0.0,
            })
        })
        .collect();
    let mut completed: Vec<bool> = nodes.iter().map(|n| n.children.is_empty()).collect();
    let parent_pool = rayon::ThreadPoolBuilder::new()
        .num_threads(rayon::current_num_threads().clamp(1, 12))
        .build()
        .map_err(|e| Error::msg(format!("parent worker pool: {e}")))?;
    while proxies[0].is_none() {
        let ready: Vec<usize> = (0..nodes.len())
            .filter(|&i| {
                !completed[i]
                    && !nodes[i].children.is_empty()
                    && nodes[i].children.iter().all(|c| proxies[c.id].is_some())
            })
            .collect();
        if ready.is_empty() {
            return Err(Error::msg("LOD dependency cycle"));
        }
        {
            let jobs: Vec<_> = ready
                .iter()
                .map(|&id| {
                    let children = nodes[id]
                        .children
                        .iter()
                        .map(|c| (c.id, proxies[c.id].take().unwrap()))
                        .collect::<Vec<_>>();
                    (id, children)
                })
                .collect();
            let results = parent_pool.install(|| {
                jobs.into_par_iter()
                    .map(|(id, mut children)| -> Result<_, Error> {
                        for (child_id, proxy) in &mut children {
                            restore_images(work, *child_id, &mut proxy.pieces)?;
                        }
                        let mut proxy =
                            parent_proxy(children.into_iter().map(|(_, p)| p).collect(), opts)?;
                        write_node(work, id, &proxy.pieces, &materials, opts)?;
                        spill_images(work, id, &mut proxy.pieces)?;
                        Ok((id, proxy))
                    })
                    .collect::<Result<Vec<_>, _>>()
            })?;
            for (id, proxy) in results {
                nodes[id].error = proxy.error;
                completed[id] = true;
                proxies[id] = Some(proxy);
            }
        }
    }
    reporter.note(&format!("mesh-to-3tz: {}", crate::hlod::TIMING.summary()));
    let root_proxy = proxies[0].as_ref().unwrap();
    reporter.note(&format!(
        "mesh-to-3tz: root triangles={} geometry_error={:.4} refinement_error={:.4}",
        root_proxy
            .pieces
            .iter()
            .map(|p| p.prim.indices.len() / 3)
            .sum::<usize>(),
        root_proxy.geometry_error,
        root_proxy.error
    ));
    let mut ts = json!({"asset":{"version":"1.1","generator":"rusty-tiles"},"geometricError":nodes[0].error*2.0,"root":tile_json(0,&nodes,!opts.explicit)});
    if !opts.explicit {
        let diagonal = crate::vec3::norm(crate::vec3::sub(nodes[0].max, nodes[0].min));
        ts["geometricError"] = json!(crate::tileset_node::top_level_error(
            diagonal,
            nodes[0].error,
            2.
        ));
    }
    if let Some(origin) = baked.map(|b| b.origin).or(opts.cartographic) {
        ts["root"]["transform"] = json!(root_transform(origin, opts.rotation));
    }
    if !opts.explicit {
        crate::implicit::write_tileset(
            &mut ts,
            work,
            crate::implicit::SubdivisionScheme::Octree,
            false,
        )?;
        fs::write(work.join("tileset.json"), serde_json::to_vec(&ts)?)?;
        let stage = job.staging("published")?;
        fs::copy(work.join("tileset.json"), stage.join("tileset.json"))?;
        for name in ["implicit-content", "subtrees"] {
            fs::rename(work.join(name), stage.join(name))?;
        }
        for entry in fs::read_dir(work)? {
            let entry = entry?;
            if entry
                .file_name()
                .to_string_lossy()
                .starts_with("implicit-tileset-")
            {
                fs::rename(entry.path(), stage.join(entry.file_name()))?;
            }
        }
        let report = json!({"encoder":"rusty-tiles-native-mesh-implicit-v2","tiling":"implicit","tiles":nodes.len(),"leafTiles":leaves});
        crate::output::write_report(&stage, report.clone(), true)?;
        return job.publish_tree_3tz(&stage, Some(report));
    }
    fs::write(work.join("tileset.json"), serde_json::to_vec(&ts)?)?;
    let mut files = vec![("tileset.json".into(), work.join("tileset.json"))];
    for node in &nodes {
        let name = format!("t/{}.glb", node.id);
        files.push((name.clone(), work.join(name)));
    }
    let result = job.publish_3tz(&files, None)?;
    reporter.note(&format!(
        "mesh-to-3tz: done {:.2}s bytes={}",
        start.elapsed().as_secs_f64(),
        fs::metadata(output)?.len()
    ));
    Ok(result)
}

fn wrap(v: i64, n: u32, mode: u32) -> u32 {
    let n = n as i64;
    match mode {
        33071 => v.clamp(0, n - 1) as u32,
        33648 => {
            let p = v.rem_euclid(n * 2);
            if p < n {
                p as u32
            } else {
                (n * 2 - 1 - p) as u32
            }
        }
        _ => v.rem_euclid(n) as u32,
    }
}
fn image_wraps(input: &Path, n: usize) -> Result<Vec<(u32, u32)>, Error> {
    let doc = source_document(input)?;
    let mut wraps = vec![None; n];
    for t in doc["textures"].as_array().into_iter().flatten() {
        let Some(i) = t["source"].as_u64() else {
            continue;
        };
        let sampler = t["sampler"].as_u64().map(|i| &doc["samplers"][i as usize]);
        let pair = sampler
            .map(|s| {
                (
                    s["wrapS"].as_u64().unwrap_or(10497) as u32,
                    s["wrapT"].as_u64().unwrap_or(10497) as u32,
                )
            })
            .unwrap_or((10497, 10497));
        if let Some(slot) = wraps.get_mut(i as usize) {
            if slot.is_some_and(|v| v != pair) {
                return Err(Error::msg(
                    "an image used with multiple wrap modes needs an unchanged glb-to-3tz wrap",
                ));
            }
            *slot = Some(pair);
        }
    }
    Ok(wraps
        .into_iter()
        .map(|v| v.unwrap_or((10497, 10497)))
        .collect())
}

fn partition(
    scene: &Scene,
    dims: &[(u32, u32)],
    materials: &[usize],
    mut ids: Vec<usize>,
    opts: &MeshTo3tzOptions,
    cell: Option<([f64; 3], [f64; 3])>,
    depth: u32,
) -> Result<Node, Error> {
    let (lo, hi) = mesh::triangle_aabb_yup(scene, &ids);
    let mut node = Node {
        slot: 0,
        children: Vec::new(),
        plans: Vec::new(),
        min: lo.map(f64::from),
        max: hi.map(f64::from),
        id: 0,
        error: 0.0,
    };
    if ids.len() <= opts.max_triangles {
        node.plans = plan(scene, dims, materials, &ids, opts.tile_size)?;
        if node.plans.iter().all(|p| p.plan.scale == 1.0) {
            return Ok(node);
        }
        node.plans.clear();
        if ids.len() == 1 {
            // One triangle may span a large source chart. Keep it whole and
            // enlarge this exceptional leaf rather than alter its surface.
            let edge = dims
                .iter()
                .map(|&(w, h)| w.max(h).saturating_add(16).next_power_of_two())
                .max()
                .unwrap_or(opts.tile_size)
                .max(opts.tile_size);
            if edge > 32768 {
                return Err(Error::msg(
                    "a source triangle requires an atlas larger than 32768 pixels",
                ));
            }
            node.plans = plan(scene, dims, materials, &ids, edge)?;
            if node.plans.iter().any(|p| p.plan.scale != 1.0) {
                return Err(Error::msg(
                    "cannot retain source texels for an individual triangle",
                ));
            }
            return Ok(node);
        }
    }
    if !opts.explicit {
        if depth >= 31 {
            return Err(Error::Data(
                "mesh octree exceeds 31 levels; raise maxTriangles or inspect coincident geometry"
                    .into(),
            ));
        }
        let (cell_lo, cell_hi) = cell.unwrap_or((lo.map(f64::from), hi.map(f64::from)));
        let mid: [f64; 3] = std::array::from_fn(|i| cell_lo[i] + (cell_hi[i] - cell_lo[i]) / 2.);
        let mut groups: [Vec<usize>; 8] = std::array::from_fn(|_| Vec::new());
        for &id in &ids {
            let p = mesh::centroid(scene, &scene.triangles[id]);
            // The source is Y-up. Address octants in the tileset's Z-up frame.
            let slot = usize::from(p[0] as f64 >= mid[0])
                | (usize::from(-(p[2] as f64) >= -mid[2]) << 1)
                | (usize::from(p[1] as f64 >= mid[1]) << 2);
            groups[slot].push(id);
        }
        if groups.iter().filter(|g| !g.is_empty()).count() == 1
            && ids.iter().all(|&id| {
                mesh::centroid(scene, &scene.triangles[id])
                    == mesh::centroid(scene, &scene.triangles[ids[0]])
            })
        {
            // Coincident centroids cannot be separated spatially. Preserve every
            // triangle in stable buckets; semantic boxes describe the overlap.
            groups = std::array::from_fn(|_| Vec::new());
            for (i, id) in ids.into_iter().enumerate() {
                groups[i % 8].push(id);
            }
        }
        for (slot, ids) in groups
            .into_iter()
            .enumerate()
            .filter(|(_, ids)| !ids.is_empty())
        {
            let upper = [slot & 1 != 0, slot & 4 != 0, slot & 2 == 0];
            let child_lo = std::array::from_fn(|i| if upper[i] { mid[i] } else { cell_lo[i] });
            let child_hi = std::array::from_fn(|i| if upper[i] { cell_hi[i] } else { mid[i] });
            let mut child = partition(
                scene,
                dims,
                materials,
                ids,
                opts,
                Some((child_lo, child_hi)),
                depth + 1,
            )?;
            child.slot = slot;
            node.children.push(child);
        }
        return Ok(node);
    }
    let axis = (0..3)
        .max_by(|&a, &b| (hi[a] - lo[a]).total_cmp(&(hi[b] - lo[b])))
        .unwrap();
    let mid = ids.len() / 2;
    ids.select_nth_unstable_by(mid, |&a, &b| {
        mesh::centroid(scene, &scene.triangles[a])[axis]
            .total_cmp(&mesh::centroid(scene, &scene.triangles[b])[axis])
            .then(a.cmp(&b))
    });
    let right = ids.split_off(mid);
    let (a, b) = if ids.len() > 20000 {
        rayon::join(
            || partition(scene, dims, materials, ids, opts, None, depth + 1),
            || partition(scene, dims, materials, right, opts, None, depth + 1),
        )
    } else {
        (
            partition(scene, dims, materials, ids, opts, None, depth + 1),
            partition(scene, dims, materials, right, opts, None, depth + 1),
        )
    };
    node.children = vec![a?, b?];
    Ok(node)
}
fn plan(
    scene: &Scene,
    dims: &[(u32, u32)],
    materials: &[usize],
    ids: &[usize],
    edge: u32,
) -> Result<Vec<Planned>, Error> {
    let mut groups: BTreeMap<GroupKey, BTreeMap<Option<u32>, Vec<usize>>> = BTreeMap::new();
    for &id in ids {
        let t = &scene.triangles[id];
        groups
            .entry((
                materials[t.material.map_or(0, |m| m as usize + 1)],
                t.image.is_some(),
                scene.vertices[t.verts[0] as usize].nrm != [0.0; 3],
            ))
            .or_default()
            .entry(t.image)
            .or_default()
            .push(id);
    }
    groups
        .into_iter()
        .map(|((material, _, _), imgs)| {
            let mut groups = Vec::new();
            for (image, ids) in imgs {
                let mut prim = TilePrimitive::default();
                let mut remap = HashMap::new();
                for id in ids {
                    for vi in scene.triangles[id].verts {
                        let dst = *remap.entry(vi).or_insert_with(|| {
                            let v = scene.vertices[vi as usize];
                            let i = prim.positions.len() as u32;
                            prim.positions.push(v.pos);
                            prim.normals.push(v.nrm);
                            prim.uvs.push(v.uv);
                            i
                        });
                        prim.indices.push(dst);
                    }
                }
                if prim.normals.iter().all(|n| *n == [0.0; 3]) {
                    prim.normals.clear();
                }
                if image.is_none() {
                    prim.uvs.clear();
                }
                groups.push(LeafGroup { image, prim });
            }
            Ok(Planned {
                material,
                plan: texture::plan_leaf_atlas_limit(groups, dims, edge, 0.0)?,
            })
        })
        .collect()
}
fn fold(node: &mut Node) {
    fn weight(node: &Node) -> usize {
        if node.children.is_empty() {
            1
        } else {
            node.children.iter().map(weight).sum()
        }
    }
    // Expand the largest remaining subtree first, before folding descendants.
    // This balances leaf counts even where source texture density varies.
    let mut frontier = std::mem::take(&mut node.children);
    loop {
        let next = frontier
            .iter()
            .enumerate()
            .filter(|(_, n)| !n.children.is_empty() && frontier.len() - 1 + n.children.len() <= 8)
            .max_by_key(|(i, n)| (weight(n), std::cmp::Reverse(*i)))
            .map(|(i, _)| i);
        let Some(i) = next else { break };
        let child = frontier.remove(i);
        frontier.splice(i..i, child.children);
    }
    node.children = frontier;
    for child in &mut node.children {
        fold(child);
    }
}
fn flatten(node: &mut Node, out: &mut Vec<Node>) {
    node.id = out.len();
    out.push(Node {
        slot: node.slot,
        children: Vec::new(),
        plans: std::mem::take(&mut node.plans),
        min: node.min,
        max: node.max,
        id: node.id,
        error: 0.0,
    });
    for c in &mut node.children {
        flatten(c, out);
    }
    out[node.id].children = node
        .children
        .iter()
        .map(|c| Node {
            slot: c.slot,
            children: Vec::new(),
            plans: Vec::new(),
            min: c.min,
            max: c.max,
            id: c.id,
            error: 0.0,
        })
        .collect();
}
fn tile_json(id: usize, nodes: &[Node], implicit: bool) -> Value {
    let n = &nodes[id];
    let lo = [n.min[0], -n.max[2], n.min[1]];
    let hi = [n.max[0], -n.min[2], n.max[1]];
    let mut v = json!({"boundingVolume":{"box":aabb_to_box(lo,hi)},"geometricError":n.error,"refine":"REPLACE","content":{"uri":format!("t/{id}.glb")}});
    if implicit && id != 0 {
        v["extras"] = json!({"implicitChildIndex":n.slot});
    }
    if !n.children.is_empty() {
        v["children"] = json!(n
            .children
            .iter()
            .map(|c| tile_json(c.id, nodes, implicit))
            .collect::<Vec<_>>());
    }
    v
}
fn write_node(
    dir: &Path,
    id: usize,
    pieces: &[Piece],
    materials: &[Value],
    opts: &MeshTo3tzOptions,
) -> Result<(), Error> {
    let mut prims: Vec<_> = pieces.iter().map(|p| p.prim.clone()).collect();
    for (i, prim) in prims.iter_mut().enumerate() {
        if let Some(bytes) = &pieces[i].delivery_image {
            prim.jpeg = Some(bytes.clone());
        } else if let Some(bytes) = prim.jpeg.take() {
            let image = image::load_from_memory(&bytes)
                .map_err(|e| {
                    if let Some(path) = std::env::var_os("RUSTY_TILES_DEBUG_IMAGES") {
                        let path = std::path::PathBuf::from(path);
                        let _ = fs::create_dir_all(&path);
                        let _ = fs::write(path.join(format!("failed-{id}-{i}.png")), &bytes);
                    }
                    Error::msg(format!("parent {id} image {i}: {e}"))
                })?
                .to_rgba8();
            prim.jpeg = Some(encode_delivery(
                &image,
                dir,
                &format!("gpu-{id}-{i}"),
                &materials[pieces[i].material],
                opts,
            )?);
        }
    }
    let mats: Vec<_> = pieces
        .iter()
        .map(|p| materials[p.material].clone())
        .collect();
    fs::write(
        dir.join(format!("t/{id}.glb")),
        crate::lossless::write(&prims, &mats, opts.meshopt)?,
    )?;
    Ok(())
}
fn encode_delivery(
    image: &RgbaImage,
    dir: &Path,
    name: &str,
    material: &Value,
    opts: &MeshTo3tzOptions,
) -> Result<Vec<u8>, Error> {
    match opts.texture_format {
        TextureFormat::Lossless => encode_png(image),
        TextureFormat::Jpeg => {
            if material["alphaMode"].as_str().unwrap_or("OPAQUE") == "OPAQUE" {
                crate::jpeg::encode(image)
            } else {
                encode_png(image)
            }
        }
        TextureFormat::Webp => texture::encode_perceptual(
            image,
            material["alphaMode"].as_str().unwrap_or("OPAQUE") == "OPAQUE",
        ),
        TextureFormat::Uastc => crate::gpu_texture::encode(
            &opts.basisu,
            dir,
            name,
            &encode_png(image)?,
            material["alphaMode"].as_str().unwrap_or("OPAQUE") == "OPAQUE",
        ),
    }
}

pub(crate) fn encode_png(image: &RgbaImage) -> Result<Vec<u8>, Error> {
    let mut bytes = Vec::new();
    image::codecs::png::PngEncoder::new_with_quality(
        &mut bytes,
        image::codecs::png::CompressionType::Fast,
        image::codecs::png::FilterType::Adaptive,
    )
    .write_image(
        image.as_raw(),
        image.width(),
        image.height(),
        image::ExtendedColorType::Rgba8,
    )?;
    Ok(bytes)
}

fn parent_proxy(children: Vec<Proxy>, opts: &MeshTo3tzOptions) -> Result<Proxy, Error> {
    let inherited = children
        .iter()
        .map(|c| c.geometry_error)
        .fold(0.0f64, f64::max);
    let previous_error = children.iter().map(|c| c.error).fold(0.0f64, f64::max);
    // Upper levels cover many leaves: a larger atlas keeps them useful in
    // the viewer instead of forcing an early jump to many full-detail tiles.
    let parent_atlas = if children.iter().any(|c| c.error > 0.0) {
        opts.tile_size
    } else {
        opts.tile_size.min(1024)
    };
    let mut groups: BTreeMap<(usize, bool, bool), Vec<TilePrimitive>> = BTreeMap::new();
    for child in children {
        for p in child.pieces {
            groups
                .entry((
                    p.material,
                    p.prim.jpeg.is_some(),
                    !p.prim.normals.is_empty(),
                ))
                .or_default()
                .push(p.prim);
        }
    }
    let mut pieces = Vec::new();
    let mut geometry_error = 0.0f64;
    let mut texture_error = 0.0f64;
    let total_triangles: usize = groups.values().flatten().map(|p| p.indices.len() / 3).sum();
    for ((material, _, _), prims) in groups {
        let triangles: usize = prims.iter().map(|p| p.indices.len() / 3).sum();
        let budget = (opts.max_triangles * triangles / total_triangles.max(1)).max(1);
        let parent = crate::hlod::build_parent(&prims, budget, parent_atlas)?;
        geometry_error = geometry_error.max(parent.error_m);
        texture_error = texture_error.max(parent.texel_m * 16.0);
        pieces.extend(parent.prims.into_iter().map(|prim| Piece {
            material,
            prim,
            delivery_image: None,
        }));
    }
    Ok(Proxy {
        pieces,
        geometry_error: inherited + geometry_error,
        error: (inherited + geometry_error)
            .max(texture_error)
            .max(previous_error + 0.000001),
    })
}
fn materials(scene: &Scene) -> Result<(Vec<Value>, Vec<usize>), Error> {
    let mut templates = Vec::new();
    let mut ids = Vec::new();
    for mut m in std::iter::once(json!({})).chain(scene.materials.iter().cloned()) {
        if let Some(obj) = m.as_object_mut() {
            obj.remove("name");
        }
        if let Some(pbr) = m["pbrMetallicRoughness"].as_object_mut() {
            pbr.remove("baseColorTexture");
        }
        let id = templates.iter().position(|v| v == &m).unwrap_or_else(|| {
            let id = templates.len();
            templates.push(m);
            id
        });
        ids.push(id);
    }
    Ok((templates, ids))
}
// Reject features the photogrammetry IR cannot retain, instead of silently
// turning a rich glTF into a different-looking model. The wrapping command
// remains available for general glTF assets.
fn validate_source(path: &Path) -> Result<(), Error> {
    let doc = source_document(path)?;
    if doc
        .get("animations")
        .and_then(Value::as_array)
        .is_some_and(|a| !a.is_empty())
        || doc
            .get("skins")
            .and_then(Value::as_array)
            .is_some_and(|a| !a.is_empty())
    {
        return Err(Error::msg(
            "mesh-to-3tz supports static meshes; use glb-to-3tz to retain animation or skins",
        ));
    }
    if doc
        .get("extensionsUsed")
        .and_then(Value::as_array)
        .is_some_and(|a| !a.is_empty())
    {
        return Err(Error::msg("mesh-to-3tz cannot yet preserve source glTF extensions; use glb-to-3tz for an unchanged wrap"));
    }
    for mesh in doc["meshes"].as_array().into_iter().flatten() {
        for p in mesh["primitives"].as_array().into_iter().flatten() {
            if p.get("targets").is_some() {
                return Err(Error::msg(
                    "morph targets cannot be spatially tiled without losing fidelity",
                ));
            }
            if p["attributes"].as_object().is_some_and(|a| {
                a.keys()
                    .any(|k| !matches!(k.as_str(), "POSITION" | "NORMAL" | "TEXCOORD_0"))
            }) {
                return Err(Error::msg("mesh-to-3tz currently supports POSITION, NORMAL and TEXCOORD_0; additional attributes require glb-to-3tz to retain fidelity"));
            }
        }
    }
    for sampler in doc["samplers"].as_array().into_iter().flatten() {
        if sampler["magFilter"].as_u64().is_some_and(|f| f != 9729)
            || sampler["minFilter"]
                .as_u64()
                .is_some_and(|f| f != 9987 && f != 9729)
        {
            return Err(Error::msg("mesh-to-3tz currently requires linear magnification and trilinear minification; glb-to-3tz preserves other samplers"));
        }
    }
    for m in doc["materials"].as_array().into_iter().flatten() {
        if ["normalTexture", "occlusionTexture", "emissiveTexture"]
            .iter()
            .any(|k| m.get(k).is_some())
            || m["pbrMetallicRoughness"]
                .get("metallicRoughnessTexture")
                .is_some()
        {
            return Err(Error::msg("spatial tiling of additional PBR texture channels is not yet supported; glb-to-3tz preserves the source"));
        }
        if m["pbrMetallicRoughness"]["baseColorTexture"]["texCoord"]
            .as_u64()
            .unwrap_or(0)
            != 0
        {
            return Err(Error::msg("base colour must use TEXCOORD_0"));
        }
    }
    Ok(())
}
fn source_document(path: &Path) -> Result<Value, Error> {
    let mut file = fs::File::open(path)?;
    if path
        .extension()
        .is_some_and(|e| e.eq_ignore_ascii_case("glb"))
    {
        let mut hdr = [0u8; 20];
        file.read_exact(&mut hdr)?;
        let len = u32::from_le_bytes(hdr[12..16].try_into().unwrap()) as usize;
        let mut bytes = vec![0; len];
        file.read_exact(&mut bytes)?;
        Ok(serde_json::from_slice(&bytes)?)
    } else {
        Ok(serde_json::from_reader(file)?)
    }
}

fn spill_images(dir: &Path, id: usize, pieces: &mut [Piece]) -> Result<(), Error> {
    for (i, p) in pieces.iter_mut().enumerate() {
        p.delivery_image = None;
        if let Some(bytes) = p.prim.jpeg.take() {
            fs::write(dir.join(format!("{id}-{i}.png")), bytes)?;
        }
    }
    Ok(())
}
fn restore_images(dir: &Path, id: usize, pieces: &mut [Piece]) -> Result<(), Error> {
    for (i, p) in pieces.iter_mut().enumerate() {
        let path = dir.join(format!("{id}-{i}.png"));
        if path.is_file() {
            p.prim.jpeg = Some(fs::read(&path)?);
            fs::remove_file(path)?;
        }
    }
    Ok(())
}
// Unused atlas pixels must not become black in coarse mip levels. Source
// chart rectangles, including their original gutters, are never overwritten.
fn fill_background(atlas: &mut RgbaImage, rects: &[[u32; 4]]) {
    let mut cover = vec![false; atlas.width() as usize * atlas.height() as usize];
    let mut sum = [0u64; 4];
    let mut count = 0;
    for &[x, y, w, h] in rects {
        for row in y..y + h {
            let start = (row * atlas.width() + x) as usize;
            cover[start..start + w as usize].fill(true);
            for col in (x..x + w).step_by(16) {
                let px = atlas.get_pixel(col, row);
                for i in 0..4 {
                    sum[i] += px[i] as u64;
                }
                count += 1;
            }
        }
    }
    if count == 0 {
        return;
    }
    let mean = image::Rgba(sum.map(|v| (v / count) as u8));
    for (i, p) in atlas.pixels_mut().enumerate() {
        if !cover[i] {
            *p = mean;
        }
    }
}
// Filter colour in linear light with premultiplied alpha. Repeated sRGB-byte
// averaging darkens painted details and produces fringes on transparent edges.
pub(crate) fn colour_tables() -> (&'static [f32; 256], &'static [u8]) {
    use std::sync::OnceLock;
    static TO_LINEAR: OnceLock<[f32; 256]> = OnceLock::new();
    static TO_SRGB: OnceLock<Vec<u8>> = OnceLock::new();
    let linear = TO_LINEAR.get_or_init(|| {
        std::array::from_fn(|i| {
            let v = i as f32 / 255.0;
            if v <= 0.04045 {
                v / 12.92
            } else {
                ((v + 0.055) / 1.055).powf(2.4)
            }
        })
    });
    let srgb = TO_SRGB.get_or_init(|| {
        (0..65536)
            .map(|i| {
                let v = i as f32 / 65535.0;
                let c = if v <= 0.0031308 {
                    12.92 * v
                } else {
                    1.055 * v.powf(1.0 / 2.4) - 0.055
                };
                (c * 255.0).round() as u8
            })
            .collect()
    });
    (linear, srgb)
}
pub(crate) fn resize_colour(source: &RgbaImage, w: u32, h: u32) -> RgbaImage {
    let (linear, srgb) = colour_tables();
    if source.width().is_multiple_of(w) && source.height().is_multiple_of(h) {
        let (sx, sy) = (source.width() / w, source.height() / h);
        return RgbaImage::from_fn(w, h, |x, y| {
            let mut sum = [0.0f32; 4];
            for yy in y * sy..(y + 1) * sy {
                for xx in x * sx..(x + 1) * sx {
                    let p = source.get_pixel(xx, yy);
                    let a = p[3] as f32 / 255.;
                    for c in 0..3 {
                        sum[c] += linear[p[c] as usize] * a;
                    }
                    sum[3] += a;
                }
            }
            let mut out = [0u8; 4];
            if sum[3] > 0. {
                for c in 0..3 {
                    out[c] = srgb[((sum[c] / sum[3]).clamp(0., 1.) * 65535.).round() as usize];
                }
            }
            out[3] = (sum[3] * 255. / (sx * sy) as f32).round() as u8;
            image::Rgba(out)
        });
    }
    let float = image::Rgba32FImage::from_fn(source.width(), source.height(), |x, y| {
        let p = source.get_pixel(x, y);
        let a = p[3] as f32 / 255.0;
        image::Rgba([
            linear[p[0] as usize] * a,
            linear[p[1] as usize] * a,
            linear[p[2] as usize] * a,
            a,
        ])
    });
    let resized = image::imageops::resize(&float, w, h, image::imageops::FilterType::Triangle);
    RgbaImage::from_fn(w, h, |x, y| {
        let p = resized.get_pixel(x, y);
        let a = p[3].clamp(0.0, 1.0);
        let mut out = [0u8; 4];
        if a > 0.0 {
            for i in 0..3 {
                out[i] = srgb[((p[i] / a).clamp(0.0, 1.0) * 65535.0).round() as usize];
            }
        }
        out[3] = (a * 255.0).round() as u8;
        image::Rgba(out)
    })
}
pub(crate) fn max_texel_size(p: &TilePrimitive, w: u32, h: u32) -> f64 {
    let mut worst = 0.0f64;
    for tri in p.indices.as_chunks::<3>().0 {
        let [a, b, c] = [tri[0] as usize, tri[1] as usize, tri[2] as usize];
        let du1 = (p.uvs[b][0] - p.uvs[a][0]) as f64 * w as f64;
        let dv1 = (p.uvs[b][1] - p.uvs[a][1]) as f64 * h as f64;
        let du2 = (p.uvs[c][0] - p.uvs[a][0]) as f64 * w as f64;
        let dv2 = (p.uvs[c][1] - p.uvs[a][1]) as f64 * h as f64;
        let det = du1 * dv2 - du2 * dv1;
        if det.abs() < 1e-12 {
            continue;
        }
        let mut norm = 0.0;
        for k in 0..3 {
            let e1 = (p.positions[b][k] - p.positions[a][k]) as f64;
            let e2 = (p.positions[c][k] - p.positions[a][k]) as f64;
            norm += ((e1 * dv2 - e2 * dv1) / det).powi(2) + ((e2 * du1 - e1 * du2) / det).powi(2);
        }
        // The differential footprint assumes an infinite plane. On tiny
        // charts it can extend far beyond the entire face; texture error on
        // that finite face is bounded by its diameter.
        let diameter = [(a, b), (b, c), (c, a)]
            .into_iter()
            .map(|(i, j)| {
                (0..3)
                    .map(|k| (p.positions[i][k] as f64 - p.positions[j][k] as f64).powi(2))
                    .sum::<f64>()
                    .sqrt()
            })
            .fold(0.0f64, f64::max);
        worst = worst.max(norm.sqrt().min(diameter));
    }
    worst
}

#[cfg(test)]
mod hierarchy_tests {
    use super::*;
    #[test]
    fn balanced_binary_input_becomes_shallow_without_losing_leaves() {
        fn make(depth: usize, id: &mut usize) -> Node {
            let current = *id;
            *id += 1;
            Node {
                slot: 0,
                id: current,
                min: [0.; 3],
                max: [1.; 3],
                error: 0.,
                plans: vec![],
                children: if depth == 0 {
                    vec![]
                } else {
                    vec![make(depth - 1, id), make(depth - 1, id)]
                },
            }
        }
        fn inspect(
            n: &Node,
            depth: usize,
            leaves: &mut std::collections::BTreeSet<usize>,
        ) -> usize {
            assert!(n.children.len() <= 8);
            if n.children.is_empty() {
                assert!(leaves.insert(n.id));
                return depth;
            }
            n.children
                .iter()
                .map(|c| inspect(c, depth + 1, leaves))
                .max()
                .unwrap()
        }
        let mut tree = make(12, &mut 0);
        let mut expected = std::collections::BTreeSet::new();
        inspect(&tree, 0, &mut expected);
        fold(&mut tree);
        let mut actual = std::collections::BTreeSet::new();
        assert_eq!(inspect(&tree, 0, &mut actual), 4);
        assert_eq!(actual, expected);
    }
}
