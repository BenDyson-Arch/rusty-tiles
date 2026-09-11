//! Author a single-mesh glTF 2.0 GLB (Y-up) for one tile.

use std::borrow::Cow;
use std::collections::BTreeMap;

use gltf_json::validation::{Checked::Valid, USize64};
use gltf_json::{accessor, buffer, image as gimage, material, mesh, scene, texture, Index, Root};

use crate::error::Error;

#[derive(Clone, Debug, Default)]
pub struct TilePrimitive {
    pub positions: Vec<[f32; 3]>,
    pub normals: Vec<[f32; 3]>,
    pub uvs: Vec<[f32; 2]>,
    pub indices: Vec<u32>,
    pub jpeg: Option<Vec<u8>>,
}

pub fn write_glb(prims: &[TilePrimitive]) -> Result<Vec<u8>, Error> {
    if prims.is_empty() {
        return Err(Error::msg("no primitives to write"));
    }

    let mut bin: Vec<u8> = Vec::new();
    let mut root = Root::default();
    root.asset.generator = Some("rusty-tiles".into());

    let mut primitives = Vec::new();
    for prim in prims {
        if prim.positions.is_empty() || prim.indices.len() < 3 {
            continue;
        }
        primitives.push(build_primitive(&mut root, &mut bin, prim)?);
    }
    if primitives.is_empty() {
        return Err(Error::msg("no primitives to write"));
    }

    pad4(&mut bin);
    let buffer = root.push(gltf_json::Buffer {
        byte_length: USize64::from(bin.len()),
        name: None,
        uri: None,
        extensions: Default::default(),
        extras: Default::default(),
    });
    for view in &mut root.buffer_views {
        view.buffer = buffer;
    }

    let mesh = root.push(mesh::Mesh {
        extensions: Default::default(),
        extras: Default::default(),
        name: None,
        primitives,
        weights: None,
    });
    let node = root.push(gltf_json::Node {
        camera: None,
        children: None,
        extensions: Default::default(),
        extras: Default::default(),
        matrix: None,
        mesh: Some(mesh),
        name: None,
        rotation: None,
        scale: None,
        translation: None,
        skin: None,
        weights: None,
    });
    let sc = root.push(scene::Scene {
        extensions: Default::default(),
        extras: Default::default(),
        name: None,
        nodes: vec![node],
    });
    root.scene = Some(sc);

    let mut json = root
        .to_vec()
        .map_err(|e| Error::msg(format!("gltf json: {e}")))?;
    while json.len() % 4 != 0 {
        json.push(b' ');
    }
    pad4(&mut bin);

    let json_len = json.len() as u32;
    let bin_len = bin.len() as u32;
    let length = 12 + 8 + json_len + 8 + bin_len;
    let glb = gltf::Glb {
        header: gltf::binary::Header {
            magic: *b"glTF",
            version: 2,
            length,
        },
        json: Cow::Owned(json),
        bin: Some(Cow::Owned(bin)),
    };
    Ok(glb.to_vec()?)
}

fn build_primitive(
    root: &mut Root,
    bin: &mut Vec<u8>,
    prim: &TilePrimitive,
) -> Result<mesh::Primitive, Error> {
    pad4(bin);
    let pos_off = bin.len();
    for p in &prim.positions {
        bin.extend_from_slice(bytemuck::bytes_of(p));
    }
    let pos_len = prim.positions.len() * 12;
    let (pos_min, pos_max) = min_max_vec3(&prim.positions);

    pad4(bin);
    let nrm_off = bin.len();
    if !prim.normals.is_empty() && prim.normals.len() != prim.positions.len() {
        return Err(Error::msg("normal count must match position count"));
    }
    let normals = &prim.normals;
    for n in normals {
        bin.extend_from_slice(bytemuck::bytes_of(n));
    }

    let has_uv = prim.jpeg.is_some() && prim.uvs.len() == prim.positions.len();
    let uv_off = if has_uv {
        pad4(bin);
        let off = bin.len();
        for uv in &prim.uvs {
            bin.extend_from_slice(bytemuck::bytes_of(uv));
        }
        Some(off)
    } else {
        None
    };

    pad4(bin);
    let idx_off = bin.len();
    let use_u16 = prim.positions.len() <= 65535 && prim.indices.iter().all(|&i| i <= 65535);
    if use_u16 {
        for &i in &prim.indices {
            bin.extend_from_slice(&(i as u16).to_le_bytes());
        }
    } else {
        for &i in &prim.indices {
            bin.extend_from_slice(&i.to_le_bytes());
        }
    }
    let idx_len = bin.len() - idx_off;

    let jpeg_view = if let Some(jpeg) = &prim.jpeg {
        pad4(bin);
        let off = bin.len();
        bin.extend_from_slice(jpeg);
        Some((off, jpeg.len()))
    } else {
        None
    };

    // Buffer index is filled in after the single buffer is pushed.
    let dummy_buffer = Index::new(0);

    let pos_view = root.push(buffer::View {
        buffer: dummy_buffer,
        byte_length: USize64::from(pos_len),
        byte_offset: Some(USize64::from(pos_off)),
        byte_stride: None,
        extensions: Default::default(),
        extras: Default::default(),
        name: None,
        target: Some(Valid(buffer::Target::ArrayBuffer)),
    });
    let nrm_view = (!normals.is_empty()).then(|| {
        root.push(buffer::View {
            buffer: dummy_buffer,
            byte_length: USize64::from(normals.len() * 12),
            byte_offset: Some(USize64::from(nrm_off)),
            byte_stride: None,
            extensions: Default::default(),
            extras: Default::default(),
            name: None,
            target: Some(Valid(buffer::Target::ArrayBuffer)),
        })
    });
    let acc_pos = root.push(accessor::Accessor {
        buffer_view: Some(pos_view),
        byte_offset: None,
        count: USize64::from(prim.positions.len()),
        component_type: Valid(accessor::GenericComponentType(accessor::ComponentType::F32)),
        extensions: Default::default(),
        extras: Default::default(),
        max: Some(gltf_json::Value::from(vec![
            pos_max[0] as f64,
            pos_max[1] as f64,
            pos_max[2] as f64,
        ])),
        min: Some(gltf_json::Value::from(vec![
            pos_min[0] as f64,
            pos_min[1] as f64,
            pos_min[2] as f64,
        ])),
        name: None,
        normalized: false,
        sparse: None,
        type_: Valid(accessor::Type::Vec3),
    });
    let acc_nrm = nrm_view.map(|nrm_view| {
        root.push(accessor::Accessor {
            buffer_view: Some(nrm_view),
            byte_offset: None,
            count: USize64::from(normals.len()),
            component_type: Valid(accessor::GenericComponentType(accessor::ComponentType::F32)),
            extensions: Default::default(),
            extras: Default::default(),
            max: None,
            min: None,
            name: None,
            normalized: false,
            sparse: None,
            type_: Valid(accessor::Type::Vec3),
        })
    });

    let mut attributes: BTreeMap<
        gltf_json::validation::Checked<mesh::Semantic>,
        Index<accessor::Accessor>,
    > = BTreeMap::new();
    attributes.insert(Valid(mesh::Semantic::Positions), acc_pos);
    if let Some(acc_nrm) = acc_nrm {
        attributes.insert(Valid(mesh::Semantic::Normals), acc_nrm);
    }

    if let Some(off) = uv_off {
        let uv_view = root.push(buffer::View {
            buffer: dummy_buffer,
            byte_length: USize64::from(prim.uvs.len() * 8),
            byte_offset: Some(USize64::from(off)),
            byte_stride: None,
            extensions: Default::default(),
            extras: Default::default(),
            name: None,
            target: Some(Valid(buffer::Target::ArrayBuffer)),
        });
        let acc_uv = root.push(accessor::Accessor {
            buffer_view: Some(uv_view),
            byte_offset: None,
            count: USize64::from(prim.uvs.len()),
            component_type: Valid(accessor::GenericComponentType(accessor::ComponentType::F32)),
            extensions: Default::default(),
            extras: Default::default(),
            max: None,
            min: None,
            name: None,
            normalized: false,
            sparse: None,
            type_: Valid(accessor::Type::Vec2),
        });
        attributes.insert(Valid(mesh::Semantic::TexCoords(0)), acc_uv);
    }

    let idx_view = root.push(buffer::View {
        buffer: dummy_buffer,
        byte_length: USize64::from(idx_len),
        byte_offset: Some(USize64::from(idx_off)),
        byte_stride: None,
        extensions: Default::default(),
        extras: Default::default(),
        name: None,
        target: Some(Valid(buffer::Target::ElementArrayBuffer)),
    });
    let idx_ty = if use_u16 {
        accessor::ComponentType::U16
    } else {
        accessor::ComponentType::U32
    };
    let acc_idx = root.push(accessor::Accessor {
        buffer_view: Some(idx_view),
        byte_offset: None,
        count: USize64::from(prim.indices.len()),
        component_type: Valid(accessor::GenericComponentType(idx_ty)),
        extensions: Default::default(),
        extras: Default::default(),
        max: None,
        min: None,
        name: None,
        normalized: false,
        sparse: None,
        type_: Valid(accessor::Type::Scalar),
    });

    let material = if let Some(bytes) = prim.jpeg.as_ref() {
        let (off, len) = jpeg_view.expect("image bytes were written");
        let view = root.push(buffer::View {
            buffer: dummy_buffer,
            byte_length: USize64::from(len),
            byte_offset: Some(USize64::from(off)),
            byte_stride: None,
            extensions: Default::default(),
            extras: Default::default(),
            name: None,
            target: None,
        });
        let image = root.push(gimage::Image {
            buffer_view: Some(view),
            mime_type: Some(gimage::MimeType(image_mime(bytes).into())),
            name: None,
            uri: None,
            extensions: Default::default(),
            extras: Default::default(),
        });
        let sampler = push_clamp_linear_sampler(root);
        let mut tex = texture::Texture {
            extensions: Default::default(),
            extras: Default::default(),
            name: None,
            sampler: Some(sampler),
            source: image,
        };
        attach_webp_extension(root, &mut tex, image, bytes);
        let tex = root.push(tex);
        Some(root.push(photo_material(photo_pbr(Some(texture::Info {
            extensions: Default::default(),
            extras: Default::default(),
            index: tex,
            tex_coord: 0,
        })))))
    } else {
        Some(root.push(photo_material(untextured_pbr())))
    };

    Ok(mesh::Primitive {
        attributes,
        extensions: Default::default(),
        extras: Default::default(),
        indices: Some(acc_idx),
        material,
        mode: Valid(mesh::Mode::Triangles),
        targets: None,
    })
}

fn pad4(buf: &mut Vec<u8>) {
    while buf.len() % 4 != 0 {
        buf.push(0);
    }
}

/// Photogrammetry albedo: dielectric, fully rough, two-sided (cave interiors).
pub(crate) fn photo_pbr(
    base_color_texture: Option<texture::Info>,
) -> material::PbrMetallicRoughness {
    material::PbrMetallicRoughness {
        base_color_factor: material::PbrBaseColorFactor([1.0, 1.0, 1.0, 1.0]),
        base_color_texture,
        metallic_factor: material::StrengthFactor(0.0),
        roughness_factor: material::StrengthFactor(1.0),
        ..Default::default()
    }
}

/// Untextured clip leftover: dark rock, never the glTF default white metal.
pub(crate) fn untextured_pbr() -> material::PbrMetallicRoughness {
    material::PbrMetallicRoughness {
        base_color_factor: material::PbrBaseColorFactor([0.22, 0.20, 0.18, 1.0]),
        metallic_factor: material::StrengthFactor(0.0),
        roughness_factor: material::StrengthFactor(1.0),
        ..Default::default()
    }
}

pub(crate) fn photo_material(pbr: material::PbrMetallicRoughness) -> material::Material {
    material::Material {
        alpha_cutoff: None,
        alpha_mode: Valid(material::AlphaMode::Opaque),
        double_sided: true,
        extensions: Default::default(),
        extras: Default::default(),
        name: None,
        pbr_metallic_roughness: pbr,
        normal_texture: None,
        occlusion_texture: None,
        emissive_texture: None,
        emissive_factor: material::EmissiveFactor([0.0, 0.0, 0.0]),
    }
}

/// MIME for an encoded albedo payload (WebP from the tiler, JPEG from tests).
pub fn image_mime(bytes: &[u8]) -> &'static str {
    if bytes.len() >= 12 && bytes.starts_with(b"RIFF") && &bytes[8..12] == b"WEBP" {
        "image/webp"
    } else if bytes.starts_with(b"\xabKTX 20\xbb\r\n\x1a\n") {
        "image/ktx2"
    } else if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        "image/png"
    } else {
        "image/jpeg"
    }
}

pub(crate) fn attach_webp_extension(
    root: &mut Root,
    tex: &mut texture::Texture,
    image: Index<gimage::Image>,
    bytes: &[u8],
) {
    if image_mime(bytes) != "image/webp" {
        return;
    }
    if !root.extensions_used.iter().any(|e| e == "EXT_texture_webp") {
        root.extensions_used.push("EXT_texture_webp".into());
    }
    let mut others = serde_json::Map::new();
    others.insert(
        "EXT_texture_webp".into(),
        serde_json::json!({ "source": image.value() }),
    );
    tex.extensions = Some(gltf_json::extensions::texture::Texture { others });
}

/// Trilinear + clamp. Atlases are gutter-padded with dead space filled by the
/// mean chart colour, so mips no longer smear grey into the mesh.
fn push_clamp_linear_sampler(root: &mut Root) -> Index<texture::Sampler> {
    root.push(texture::Sampler {
        mag_filter: Some(Valid(texture::MagFilter::Linear)),
        min_filter: Some(Valid(texture::MinFilter::LinearMipmapLinear)),
        name: None,
        wrap_s: Valid(texture::WrappingMode::ClampToEdge),
        wrap_t: Valid(texture::WrappingMode::ClampToEdge),
        extensions: Default::default(),
        extras: Default::default(),
    })
}

fn min_max_vec3(pts: &[[f32; 3]]) -> ([f32; 3], [f32; 3]) {
    let mut min = [f32::INFINITY; 3];
    let mut max = [f32::NEG_INFINITY; 3];
    for p in pts {
        for i in 0..3 {
            min[i] = min[i].min(p[i]);
            max[i] = max[i].max(p[i]);
        }
    }
    (min, max)
}
