//! Indexed mesh IR for the spatial tiler. Positions stay glTF Y-up (after node
//! transforms). Bounding volumes convert to Z-up at tileset write time.
//!
//! Geometry is copied out of a mmap'd GLB; images stay encoded until crop.
//! Never call `gltf::import()` — that decodes every texture.

use std::fs::File;
use std::path::Path;

use memmap2::Mmap;

use crate::bbox::{mul4, transform_point};
use crate::error::Error;

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
}

#[derive(Clone, Debug)]
pub struct EncodedImage {
    pub bytes: Vec<u8>,
}

pub struct Scene {
    pub vertices: Vec<Vertex>,
    pub triangles: Vec<Triangle>,
    pub images: Vec<EncodedImage>,
    pub source_bytes: u64,
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
        let g = gltf::Gltf::from_slice(&glb.json)?;
        let bin = glb.bin.as_ref().map(|c| c.as_ref()).unwrap_or(&[]);
        extract(&g.document, path, &[bin], source_bytes)
    } else {
        let g = gltf::Gltf::from_slice(bytes)?;
        let buffers = gltf::import_buffers(&g.document, path.parent(), g.blob)?;
        let slices: Vec<&[u8]> = buffers.iter().map(|b| b.0.as_slice()).collect();
        extract(&g.document, path, &slices, source_bytes)
    }
}

fn extract(
    document: &gltf::Document,
    path: &Path,
    buffers: &[&[u8]],
    source_bytes: u64,
) -> Result<Scene, Error> {
    let images = load_images(document, path, buffers)?;
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
        for scene in document.scenes() {
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
    })
}

fn load_images(
    document: &gltf::Document,
    path: &Path,
    buffers: &[&[u8]],
) -> Result<Vec<EncodedImage>, Error> {
    let mut out = Vec::new();
    for image in document.images() {
        let bytes = match image.source() {
            gltf::image::Source::View { view, mime_type: _ } => {
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
                buf[start..end].to_vec()
            }
            gltf::image::Source::Uri { uri, mime_type: _ } => {
                let parent = path.parent().unwrap_or(Path::new("."));
                std::fs::read(parent.join(uri))?
            }
        };
        out.push(EncodedImage { bytes });
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

    let index_list: Vec<u32> = if let Some(inds) = reader.read_indices() {
        inds.into_u32().collect()
    } else {
        (0..positions.len() as u32).collect()
    };
    if index_list.len() < 3 {
        return Ok(());
    }

    let mut normals: Vec<[f32; 3]> = if let Some(iter) = reader.read_normals() {
        iter.map(|n| normalize(transform_vector(world, n)))
            .collect()
    } else {
        Vec::new()
    };
    if normals.len() != positions.len() {
        normals = vertex_normals(&positions, &index_list);
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
        });
    }
    Ok(())
}

fn transform_vector(m: [[f32; 4]; 4], n: [f32; 3]) -> [f32; 3] {
    [
        m[0][0] * n[0] + m[1][0] * n[1] + m[2][0] * n[2],
        m[0][1] * n[0] + m[1][1] * n[1] + m[2][1] * n[2],
        m[0][2] * n[0] + m[1][2] * n[1] + m[2][2] * n[2],
    ]
}

fn vertex_normals(positions: &[[f32; 3]], indices: &[u32]) -> Vec<[f32; 3]> {
    let mut nrms = vec![[0.0f32; 3]; positions.len()];
    for tri in indices.chunks_exact(3) {
        let a = positions[tri[0] as usize];
        let b = positions[tri[1] as usize];
        let c = positions[tri[2] as usize];
        let f = face_normal(a, b, c);
        for &i in tri {
            let n = &mut nrms[i as usize];
            n[0] += f[0];
            n[1] += f[1];
            n[2] += f[2];
        }
    }
    for n in &mut nrms {
        *n = normalize(*n);
    }
    nrms
}

fn face_normal(a: [f32; 3], b: [f32; 3], c: [f32; 3]) -> [f32; 3] {
    let u = [b[0] - a[0], b[1] - a[1], b[2] - a[2]];
    let v = [c[0] - a[0], c[1] - a[1], c[2] - a[2]];
    [
        u[1] * v[2] - u[2] * v[1],
        u[2] * v[0] - u[0] * v[2],
        u[0] * v[1] - u[1] * v[0],
    ]
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
