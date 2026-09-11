//! Indexed mesh IR for the spatial tiler. Positions stay glTF Y-up (after node
//! transforms). Bounding volumes convert to Z-up at tileset write time.
//!
//! Geometry is copied out of a mmap'd GLB; images stay encoded until crop.
//! Never call `gltf::import()` — that decodes every texture.

use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

use memmap2::Mmap;
use rayon::prelude::*;

use crate::bbox::{mul4, transform_point};
use crate::error::Error;
use crate::georef::{
    geographic_bbox_wgs84, geographic_origin_yup, looks_geographic_yup, looks_web_mercator_yup,
    mercator_bbox_wgs84, mercator_bbox_wgs84_offset, mercator_origin_yup, mercator_shift_origin,
    Cartographic, CrsKind, EnuFrame, SourceCrs, SourceOffset,
};

const IDENTITY: [[f32; 4]; 4] = [
    [1.0, 0.0, 0.0, 0.0],
    [0.0, 1.0, 0.0, 0.0],
    [0.0, 0.0, 1.0, 0.0],
    [0.0, 0.0, 0.0, 1.0],
];

#[derive(Clone, Copy, Debug)]
pub struct Vertex {
    pub pos: [f32; 3],
    pub nrm: [f32; 3],
    pub uv: [f32; 2],
}

#[derive(Clone, Copy, Debug)]
pub struct Triangle {
    pub verts: [u32; 3],
    pub image: Option<u32>,
    pub material: Option<u32>,
}

/// JPEG/PNG/WebP bytes live on disk; decode one image at a time (8K atlases
/// can expand to ~256 MiB RGBA each).
#[derive(Clone, Debug)]
pub struct EncodedImage {
    path: PathBuf,
    offset: u64,
    length: u64,
    owned: Option<Vec<u8>>,
}

impl EncodedImage {
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    pub fn len(&self) -> usize {
        self.owned
            .as_ref()
            .map(|b| b.len())
            .unwrap_or(self.length as usize)
    }

    pub fn load(&self) -> Result<Vec<u8>, Error> {
        self.load_prefix(usize::MAX)
    }

    /// First `max` bytes — enough for image headers without reading 15 MB.
    pub fn load_prefix(&self, max: usize) -> Result<Vec<u8>, Error> {
        let n = (self.length as usize).min(max);
        if let Some(b) = &self.owned {
            return Ok(b[..n.min(b.len())].to_vec());
        }
        let mut f = File::open(&self.path)?;
        f.seek(SeekFrom::Start(self.offset))?;
        let mut buf = vec![0u8; n];
        f.read_exact(&mut buf)?;
        Ok(buf)
    }
}

pub struct Scene {
    pub vertices: Vec<Vertex>,
    pub triangles: Vec<Triangle>,
    pub images: Vec<EncodedImage>,
    pub source_bytes: u64,
    pub materials: Vec<serde_json::Value>,
}

impl Scene {
    pub fn triangle_count(&self) -> usize {
        self.triangles.len()
    }

    pub fn under_budget(&self, max_triangles: usize, max_bytes: u64) -> bool {
        self.triangle_count() <= max_triangles && self.source_bytes <= max_bytes
    }
}

pub fn load(path: &Path) -> Result<Scene, Error> {
    if !path.is_file() {
        return Err(Error::InputNotFound(path.to_path_buf()));
    }
    let source_bytes = std::fs::metadata(path)?.len();
    let file = File::open(path)?;
    let mmap = unsafe { Mmap::map(&file)? };
    load_bytes(path, &mmap, source_bytes)
}

fn load_bytes(path: &Path, bytes: &[u8], source_bytes: u64) -> Result<Scene, Error> {
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();

    if ext == "glb" {
        let glb = gltf::Glb::from_slice(bytes)?;
        let json_len = u32::from_le_bytes(bytes[12..16].try_into().unwrap()) as u64;
        let bin_start = 20 + json_len + 8;
        let g = gltf::Gltf::from_slice(&glb.json)?;
        let bin = glb.bin.as_ref().map(|c| c.as_ref()).unwrap_or(&[]);
        extract(&g.document, path, &[bin], Some(bin_start), source_bytes)
    } else {
        let g = gltf::Gltf::from_slice(bytes)?;
        let buffers = gltf::import_buffers(&g.document, path.parent(), g.blob)?;
        let slices: Vec<&[u8]> = buffers.iter().map(|b| b.0.as_slice()).collect();
        extract(&g.document, path, &slices, None, source_bytes)
    }
}

fn extract(
    document: &gltf::Document,
    path: &Path,
    buffers: &[&[u8]],
    glb_bin_start: Option<u64>,
    source_bytes: u64,
) -> Result<Scene, Error> {
    let images = load_images(document, path, buffers, glb_bin_start)?;
    let mut vertices = Vec::new();
    let mut triangles = Vec::new();

    if document.scenes().len() == 0 {
        collect_nodes(
            document.nodes(),
            buffers,
            IDENTITY,
            &images,
            &mut vertices,
            &mut triangles,
        )?;
    } else {
        for scene in document.default_scene().into_iter().chain(
            document
                .scenes()
                .take(usize::from(document.default_scene().is_none())),
        ) {
            collect_nodes(
                scene.nodes(),
                buffers,
                IDENTITY,
                &images,
                &mut vertices,
                &mut triangles,
            )?;
        }
    }

    if triangles.is_empty() {
        return Err(Error::msg("glTF has no triangles"));
    }

    Ok(Scene {
        vertices,
        triangles,
        images,
        source_bytes,
        materials: document
            .materials()
            .map(|m| {
                serde_json::to_value(&document.as_json().materials[m.index().unwrap()]).unwrap()
            })
            .collect(),
    })
}

/// Result of rewriting projected/geographic positions into local ENU metres.
#[derive(Clone, Copy, Debug)]
pub struct GeographicBake {
    pub origin: Cartographic,
    pub bbox_wgs84: [f64; 4],
    pub kind: CrsKind,
}

pub fn position_aabb_yup(scene: &Scene) -> ([f64; 3], [f64; 3]) {
    let mut min = [f64::INFINITY; 3];
    let mut max = [f64::NEG_INFINITY; 3];
    for v in &scene.vertices {
        for i in 0..3 {
            min[i] = min[i].min(v.pos[i] as f64);
            max[i] = max[i].max(v.pos[i] as f64);
        }
    }
    (min, max)
}

/// How to bake POSITION into local ENU metres.
#[derive(Clone, Copy, Debug, Default)]
pub struct BakeToEnu {
    pub prefer: Option<Cartographic>,
    pub crs: SourceCrs,
    pub offset: Option<SourceOffset>,
}

/// Rewrite Metashape Y-up POSITION into local ENU metres (X east, Y up, Z −north).
/// Detects geographic degrees or EPSG:3857, or uses an explicit CRS + Metashape offset.
/// Source file is not modified.
pub fn bake_to_enu(scene: &mut Scene, opts: &BakeToEnu) -> Option<GeographicBake> {
    let (min, max) = position_aabb_yup(scene);
    let offset = opts.offset;
    let crs = match opts.crs {
        SourceCrs::Auto if offset.is_some() => SourceCrs::WebMercator,
        other => other,
    };
    let (mut origin, bbox_wgs84, kind) = match crs {
        SourceCrs::Geographic => {
            if opts.crs == SourceCrs::Auto && !looks_geographic_yup(min, max) {
                return None;
            }
            (
                geographic_origin_yup(min, max),
                geographic_bbox_wgs84(min, max),
                CrsKind::Geographic,
            )
        }
        SourceCrs::WebMercator => {
            if let Some(off) = offset {
                (
                    mercator_shift_origin(off),
                    mercator_bbox_wgs84_offset(min, max, off),
                    CrsKind::WebMercator,
                )
            } else {
                if opts.crs == SourceCrs::Auto && !looks_web_mercator_yup(min, max) {
                    return None;
                }
                (
                    mercator_origin_yup(min, max),
                    mercator_bbox_wgs84(min, max),
                    CrsKind::WebMercator,
                )
            }
        }
        SourceCrs::Auto => {
            if looks_geographic_yup(min, max) {
                (
                    geographic_origin_yup(min, max),
                    geographic_bbox_wgs84(min, max),
                    CrsKind::Geographic,
                )
            } else if looks_web_mercator_yup(min, max) {
                (
                    mercator_origin_yup(min, max),
                    mercator_bbox_wgs84(min, max),
                    CrsKind::WebMercator,
                )
            } else {
                return None;
            }
        }
    };
    if offset.is_none() {
        if let Some(p) = opts.prefer {
            origin.lon_deg = p.lon_deg;
            origin.lat_deg = p.lat_deg;
            if p.height_m != 0.0 {
                origin.height_m = p.height_m;
            }
        }
    }
    match (kind, offset) {
        (CrsKind::WebMercator, Some(off)) => {
            // The shift improves storage precision; it does not remove the
            // source projection's scale or turn grid axes into a tangent frame.
            let frame = EnuFrame::new(origin);
            scene.vertices.par_iter_mut().for_each(|v| {
                v.nrm = frame.normal_to_enu_yup_offset(v.pos, v.nrm, kind, off);
                v.pos = frame.mercator_yup_offset_to_enu_yup(v.pos, off);
            });
        }
        (CrsKind::Geographic, _) => {
            let frame = EnuFrame::new(origin);
            scene.vertices.par_iter_mut().for_each(|v| {
                v.nrm = frame.normal_to_enu_yup(v.pos, v.nrm, CrsKind::Geographic);
                v.pos = frame.geog_yup_to_enu_yup(v.pos);
            });
        }
        (CrsKind::WebMercator, None) => {
            let frame = EnuFrame::new(origin);
            scene.vertices.par_iter_mut().for_each(|v| {
                v.nrm = frame.normal_to_enu_yup(v.pos, v.nrm, CrsKind::WebMercator);
                v.pos = frame.mercator_yup_to_enu_yup(v.pos);
            });
        }
    }
    Some(GeographicBake {
        origin,
        bbox_wgs84,
        kind,
    })
}

/// Auto-detect geographic or world Web Mercator (no Metashape shift).
pub fn bake_geographic(scene: &mut Scene, prefer: Option<Cartographic>) -> Option<GeographicBake> {
    bake_to_enu(
        scene,
        &BakeToEnu {
            prefer,
            ..BakeToEnu::default()
        },
    )
}

fn load_images(
    document: &gltf::Document,
    path: &Path,
    buffers: &[&[u8]],
    glb_bin_start: Option<u64>,
) -> Result<Vec<EncodedImage>, Error> {
    let mut out = Vec::new();
    for image in document.images() {
        let img = match image.source() {
            gltf::image::Source::View { view, mime_type: _ } => {
                if let Some(bin_start) = glb_bin_start {
                    if view.buffer().index() != 0 {
                        return Err(Error::msg("GLB image is not in buffer 0"));
                    }
                    EncodedImage {
                        path: path.to_path_buf(),
                        offset: bin_start + view.offset() as u64,
                        length: view.length() as u64,
                        owned: None,
                    }
                } else {
                    let buf = buffers
                        .get(view.buffer().index())
                        .ok_or_else(|| Error::msg("image buffer view out of range"))?;
                    let start = view.offset();
                    let end = start
                        .checked_add(view.length())
                        .ok_or_else(|| Error::msg("image buffer view overflow"))?;
                    if end > buf.len() {
                        return Err(Error::msg("image buffer view exceeds buffer"));
                    }
                    EncodedImage {
                        path: path.to_path_buf(),
                        offset: 0,
                        length: (end - start) as u64,
                        owned: Some(buf[start..end].to_vec()),
                    }
                }
            }
            gltf::image::Source::Uri { uri, mime_type: _ } => {
                let p = path.parent().unwrap_or(Path::new(".")).join(uri);
                let length = std::fs::metadata(&p)?.len();
                EncodedImage {
                    path: p,
                    offset: 0,
                    length,
                    owned: None,
                }
            }
        };
        out.push(img);
    }
    Ok(out)
}

fn collect_nodes<'a>(
    nodes: impl Iterator<Item = gltf::Node<'a>>,
    buffers: &[&[u8]],
    parent: [[f32; 4]; 4],
    images: &[EncodedImage],
    vertices: &mut Vec<Vertex>,
    triangles: &mut Vec<Triangle>,
) -> Result<(), Error> {
    for node in nodes {
        let world = mul4(parent, node.transform().matrix());
        if let Some(mesh) = node.mesh() {
            for prim in mesh.primitives() {
                ingest_primitive(prim, buffers, world, images, vertices, triangles)?;
            }
        }
        collect_nodes(node.children(), buffers, world, images, vertices, triangles)?;
    }
    Ok(())
}

fn ingest_primitive(
    prim: gltf::Primitive<'_>,
    buffers: &[&[u8]],
    world: [[f32; 4]; 4],
    images: &[EncodedImage],
    vertices: &mut Vec<Vertex>,
    triangles: &mut Vec<Triangle>,
) -> Result<(), Error> {
    let reader = prim.reader(|b| buffers.get(b.index()).copied());
    let positions: Vec<[f32; 3]> = match reader.read_positions() {
        Some(iter) => iter.map(|p| transform_point(world, p)).collect(),
        None => return Ok(()),
    };
    if positions.is_empty() {
        return Ok(());
    }

    let mut index_list: Vec<u32> = if let Some(inds) = reader.read_indices() {
        inds.into_u32().collect()
    } else {
        (0..positions.len() as u32).collect()
    };
    if prim.mode() != gltf::mesh::Mode::Triangles {
        return Err(Error::msg("mesh-to-3tz requires triangle primitives"));
    }
    if index_list.len() % 3 != 0 || index_list.iter().any(|&i| i as usize >= positions.len()) {
        return Err(Error::msg("invalid triangle indices"));
    }
    if positions.iter().flatten().any(|v| !v.is_finite()) {
        return Err(Error::msg("non-finite POSITION"));
    }
    if index_list.len() < 3 {
        return Ok(());
    }

    let a = [world[0][0], world[0][1], world[0][2]];
    let b = [world[1][0], world[1][1], world[1][2]];
    let c = [world[2][0], world[2][1], world[2][2]];
    let cross = |a: [f32; 3], b: [f32; 3]| {
        [
            a[1] * b[2] - a[2] * b[1],
            a[2] * b[0] - a[0] * b[2],
            a[0] * b[1] - a[1] * b[0],
        ]
    };
    let co = [cross(b, c), cross(c, a), cross(a, b)];
    let det = (0..3).map(|i| a[i] * co[0][i]).sum::<f32>();
    if det < 0.0 {
        for tri in index_list.chunks_exact_mut(3) {
            tri.swap(1, 2);
        }
    }
    let mut normals: Vec<[f32; 3]> = if let Some(iter) = reader.read_normals() {
        if world == IDENTITY {
            iter.collect()
        } else {
            iter.map(|n| {
                normalize(std::array::from_fn(|i| {
                    (co[0][i] * n[0] + co[1][i] * n[1] + co[2][i] * n[2]) * det.signum()
                }))
            })
            .collect()
        }
    } else {
        Vec::new()
    };
    if normals.len() != positions.len() {
        normals = vec![[0.0; 3]; positions.len()];
    }

    let mut uvs: Vec<[f32; 2]> = if let Some(iter) = reader.read_tex_coords(0) {
        iter.into_f32().collect()
    } else {
        Vec::new()
    };
    if uvs.len() != positions.len() {
        uvs = vec![[0.0, 0.0]; positions.len()];
    }

    let image = prim
        .material()
        .pbr_metallic_roughness()
        .base_color_texture()
        .map(|info| info.texture().source().index() as u32)
        .filter(|&i| (i as usize) < images.len());

    let base = vertices.len() as u32;
    for i in 0..positions.len() {
        vertices.push(Vertex {
            pos: positions[i],
            nrm: normals[i],
            uv: uvs[i],
        });
    }
    for tri in index_list.chunks_exact(3) {
        triangles.push(Triangle {
            verts: [base + tri[0], base + tri[1], base + tri[2]],
            image,
            material: prim.material().index().map(|i| i as u32),
        });
    }
    Ok(())
}

fn normalize(n: [f32; 3]) -> [f32; 3] {
    let len = (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt();
    if len < 1e-20 {
        [0.0, 1.0, 0.0]
    } else {
        [n[0] / len, n[1] / len, n[2] / len]
    }
}

pub fn centroid(scene: &Scene, tri: &Triangle) -> [f32; 3] {
    let a = scene.vertices[tri.verts[0] as usize].pos;
    let b = scene.vertices[tri.verts[1] as usize].pos;
    let c = scene.vertices[tri.verts[2] as usize].pos;
    [
        (a[0] + b[0] + c[0]) / 3.0,
        (a[1] + b[1] + c[1]) / 3.0,
        (a[2] + b[2] + c[2]) / 3.0,
    ]
}

pub fn triangle_aabb_yup(scene: &Scene, ids: &[usize]) -> ([f32; 3], [f32; 3]) {
    let mut min = [f32::INFINITY; 3];
    let mut max = [f32::NEG_INFINITY; 3];
    for &id in ids {
        for v in scene.triangles[id].verts {
            let p = scene.vertices[v as usize].pos;
            for i in 0..3 {
                min[i] = min[i].min(p[i]);
                max[i] = max[i].max(p[i]);
            }
        }
    }
    (min, max)
}
