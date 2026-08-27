//! KHR_mesh_quantization + EXT_meshopt_compression GLB writer.

use std::borrow::Cow;
use std::collections::BTreeMap;

use gltf_json::validation::{Checked::Valid, USize64};
use gltf_json::{accessor, buffer, image as gimage, material, mesh, scene, texture, Index, Root};
use meshopt::encoding::{encode_index_buffer, encode_vertex_buffer};
use meshopt::optimize::{optimize_vertex_cache, optimize_vertex_fetch};
use serde_json::{json, Map, Value};

use crate::error::Error;
use crate::glb_write::TilePrimitive;

#[derive(Clone, Copy, Default)]
#[repr(C)]
struct Vtx {
    p: [f32; 3],
    n: [f32; 3],
    t: [f32; 2],
}

#[derive(Clone, Copy)]
#[repr(C)]
struct QuantPos {
    v: [i16; 4],
}

#[derive(Clone, Copy)]
#[repr(C)]
struct QuantNrm {
    v: [i8; 4],
}

#[derive(Clone, Copy)]
#[repr(C)]
struct QuantUv {
    v: [u16; 2],
}

/// Author a quantized, meshopt-compressed GLB. JPEG payloads stay raw.
pub fn write_glb_compressed(prims: &[TilePrimitive]) -> Result<Vec<u8>, Error> {
    if prims.is_empty() {
        return Err(Error::msg("no primitives to write"));
    }

    let mut packed_prims = Vec::new();
    for prim in prims {
        if prim.positions.is_empty() || prim.indices.len() < 3 {
            continue;
        }
        packed_prims.push(pack_prim(prim));
    }
    if packed_prims.is_empty() {
        return Err(Error::msg("no primitives to write"));
    }

    let mut max_abs = 0.0f32;
    for p in &packed_prims {
        for v in &p.verts {
            max_abs = max_abs
                .max(v.p[0].abs())
                .max(v.p[1].abs())
                .max(v.p[2].abs());
        }
    }
    let scale = (max_abs / 32767.0).max(1e-20);
    let prepared: Vec<Prepared> = packed_prims
        .into_iter()
        .map(|p| quantize_prim(p, scale))
        .collect();

    let mut bin: Vec<u8> = Vec::new();
    let mut fallback_len = 0usize;
    let mut root = Root::default();
    root.asset.generator = Some("tinyowl-tiles".into());
    root.extensions_used = vec![
        "KHR_mesh_quantization".into(),
        "EXT_meshopt_compression".into(),
    ];
    root.extensions_required = root.extensions_used.clone();

    let dummy0 = Index::new(0);
    let dummy1 = Index::new(1);

    let mut primitives = Vec::new();
    for p in &prepared {
        primitives.push(build_compressed_primitive(
            &mut root,
            &mut bin,
            &mut fallback_len,
            dummy0,
            dummy1,
            p,
        )?);
    }

    pad4(&mut bin);
    let data_buffer = root.push(gltf_json::Buffer {
        byte_length: USize64::from(bin.len()),
        name: None,
        uri: None,
        extensions: Default::default(),
        extras: Default::default(),
    });
    let fallback_buffer = root.push(gltf_json::Buffer {
        byte_length: USize64::from(fallback_len.max(1)),
        name: None,
        uri: None,
        extensions: Some(gltf_json::extensions::buffer::Buffer {
            others: ext_map(json!({ "EXT_meshopt_compression": { "fallback": true } })),
        }),
        extras: Default::default(),
    });
    for view in &mut root.buffer_views {
        if view.buffer == dummy1 {
            view.buffer = fallback_buffer;
        } else {
            view.buffer = data_buffer;
        }
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
        scale: Some([scale, scale, scale]),
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

    let mut json_bytes = root
        .to_vec()
        .map_err(|e| Error::msg(format!("gltf json: {e}")))?;
    while !json_bytes.len().is_multiple_of(4) {
        json_bytes.push(b' ');
    }
    pad4(&mut bin);

    let json_len = json_bytes.len() as u32;
    let bin_len = bin.len() as u32;
    let length = 12 + 8 + json_len + 8 + bin_len;
    let glb = gltf::Glb {
        header: gltf::binary::Header {
            magic: *b"glTF",
            version: 2,
            length,
        },
        json: Cow::Owned(json_bytes),
        bin: Some(Cow::Owned(bin)),
    };
    Ok(glb.to_vec()?)
}

struct PackedPrim {
    verts: Vec<Vtx>,
    indices: Vec<u32>,
    jpeg: Option<Vec<u8>>,
    has_uv: bool,
}

struct Prepared {
    pos: Vec<QuantPos>,
    nrm: Vec<QuantNrm>,
    uv: Option<Vec<QuantUv>>,
    indices: Vec<u32>,
    jpeg: Option<Vec<u8>>,
    pos_min: [i16; 3],
    pos_max: [i16; 3],
}

fn pack_prim(prim: &TilePrimitive) -> PackedPrim {
    let mut packed = Vec::with_capacity(prim.positions.len());
    let has_n = prim.normals.len() == prim.positions.len();
    let has_uv = prim.jpeg.is_some() && prim.uvs.len() == prim.positions.len();
    for i in 0..prim.positions.len() {
        packed.push(Vtx {
            p: prim.positions[i],
            n: if has_n {
                prim.normals[i]
            } else {
                [0.0, 1.0, 0.0]
            },
            t: if has_uv { prim.uvs[i] } else { [0.0, 0.0] },
        });
    }
    let mut indices = optimize_vertex_cache(&prim.indices, packed.len());
    let verts = optimize_vertex_fetch(&mut indices, &packed);
    PackedPrim {
        verts,
        indices,
        jpeg: prim.jpeg.clone(),
        has_uv,
    }
}

fn quantize_prim(p: PackedPrim, scale: f32) -> Prepared {
    let mut pos = Vec::with_capacity(p.verts.len());
    let mut nrm = Vec::with_capacity(p.verts.len());
    let mut uv = if p.has_uv {
        Some(Vec::with_capacity(p.verts.len()))
    } else {
        None
    };
    let mut pos_min = [i16::MAX; 3];
    let mut pos_max = [i16::MIN; 3];
    for v in &p.verts {
        let q = [
            quant_i16(v.p[0], scale),
            quant_i16(v.p[1], scale),
            quant_i16(v.p[2], scale),
        ];
        for i in 0..3 {
            pos_min[i] = pos_min[i].min(q[i]);
            pos_max[i] = pos_max[i].max(q[i]);
        }
        pos.push(QuantPos {
            v: [q[0], q[1], q[2], 0],
        });
        nrm.push(QuantNrm {
            v: [
                quant_snorm8(v.n[0]),
                quant_snorm8(v.n[1]),
                quant_snorm8(v.n[2]),
                0,
            ],
        });
        if let Some(uvs) = uv.as_mut() {
            uvs.push(QuantUv {
                v: [quant_unorm16(v.t[0]), quant_unorm16(v.t[1])],
            });
        }
    }
    Prepared {
        pos,
        nrm,
        uv,
        indices: p.indices,
        jpeg: p.jpeg,
        pos_min,
        pos_max,
    }
}

fn build_compressed_primitive(
    root: &mut Root,
    bin: &mut Vec<u8>,
    fallback_len: &mut usize,
    data_buf: Index<gltf_json::Buffer>,
    fallback_buf: Index<gltf_json::Buffer>,
    p: &Prepared,
) -> Result<mesh::Primitive, Error> {
    let nvert = p.pos.len();
    let pos_enc = encode_vertex_buffer(&p.pos).map_err(|e| Error::msg(format!("meshopt: {e}")))?;
    let nrm_enc = encode_vertex_buffer(&p.nrm).map_err(|e| Error::msg(format!("meshopt: {e}")))?;
    let uv_enc = if let Some(uv) = &p.uv {
        Some(encode_vertex_buffer(uv).map_err(|e| Error::msg(format!("meshopt: {e}")))?)
    } else {
        None
    };
    let idx_enc =
        encode_index_buffer(&p.indices, nvert).map_err(|e| Error::msg(format!("meshopt: {e}")))?;
    let use_u16 = nvert <= 65535 && p.indices.iter().all(|&i| i <= 65535);
    let idx_stride = if use_u16 { 2 } else { 4 };

    let acc_pos = push_attr_view(
        root,
        bin,
        fallback_len,
        data_buf,
        fallback_buf,
        &pos_enc,
        nvert,
        8,
        "ATTRIBUTES",
        accessor::ComponentType::I16,
        accessor::Type::Vec3,
        false,
    )?;
    if let Some(acc) = root.accessors.get_mut(acc_pos.value() as usize) {
        acc.min = Some(gltf_json::Value::from(vec![
            p.pos_min[0] as f64,
            p.pos_min[1] as f64,
            p.pos_min[2] as f64,
        ]));
        acc.max = Some(gltf_json::Value::from(vec![
            p.pos_max[0] as f64,
            p.pos_max[1] as f64,
            p.pos_max[2] as f64,
        ]));
    }

    let acc_nrm = push_attr_view(
        root,
        bin,
        fallback_len,
        data_buf,
        fallback_buf,
        &nrm_enc,
        nvert,
        4,
        "ATTRIBUTES",
        accessor::ComponentType::I8,
        accessor::Type::Vec3,
        true,
    )?;

    let mut attributes: BTreeMap<
        gltf_json::validation::Checked<mesh::Semantic>,
        Index<accessor::Accessor>,
    > = BTreeMap::new();
    attributes.insert(Valid(mesh::Semantic::Positions), acc_pos);
    attributes.insert(Valid(mesh::Semantic::Normals), acc_nrm);

    if let Some(enc) = uv_enc {
        let acc_uv = push_attr_view(
            root,
            bin,
            fallback_len,
            data_buf,
            fallback_buf,
            &enc,
            nvert,
            4,
            "ATTRIBUTES",
            accessor::ComponentType::U16,
            accessor::Type::Vec2,
            true,
        )?;
        attributes.insert(Valid(mesh::Semantic::TexCoords(0)), acc_uv);
    }

    let acc_idx = push_index_view(
        root,
        bin,
        fallback_len,
        data_buf,
        fallback_buf,
        &idx_enc,
        p.indices.len(),
        idx_stride,
        use_u16,
    )?;

    let material = if let Some(jpeg) = &p.jpeg {
        Some(push_jpeg(root, bin, data_buf, jpeg)?)
    } else {
        None
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

#[allow(clippy::too_many_arguments)]
fn push_attr_view(
    root: &mut Root,
    bin: &mut Vec<u8>,
    fallback_len: &mut usize,
    data_buf: Index<gltf_json::Buffer>,
    fallback_buf: Index<gltf_json::Buffer>,
    encoded: &[u8],
    count: usize,
    stride: usize,
    mode: &str,
    comp: accessor::ComponentType,
    ty: accessor::Type,
    normalized: bool,
) -> Result<Index<accessor::Accessor>, Error> {
    pad4(bin);
    let off = bin.len();
    bin.extend_from_slice(encoded);
    let enc_len = encoded.len();

    let uncomp = stride * count;
    let fb_off = *fallback_len;
    *fallback_len += uncomp;

    let view = root.push(buffer::View {
        buffer: fallback_buf,
        byte_length: USize64::from(uncomp),
        byte_offset: Some(USize64::from(fb_off)),
        byte_stride: Some(buffer::Stride(stride)),
        extensions: Some(gltf_json::extensions::buffer::View {
            others: ext_map(json!({
                "EXT_meshopt_compression": {
                    "buffer": data_buf.value(),
                    "byteOffset": off,
                    "byteLength": enc_len,
                    "byteStride": stride,
                    "count": count,
                    "mode": mode
                }
            })),
        }),
        extras: Default::default(),
        name: None,
        target: Some(Valid(buffer::Target::ArrayBuffer)),
    });
    Ok(root.push(accessor::Accessor {
        buffer_view: Some(view),
        byte_offset: None,
        count: USize64::from(count),
        component_type: Valid(accessor::GenericComponentType(comp)),
        extensions: Default::default(),
        extras: Default::default(),
        max: None,
        min: None,
        name: None,
        normalized,
        sparse: None,
        type_: Valid(ty),
    }))
}

#[allow(clippy::too_many_arguments)]
fn push_index_view(
    root: &mut Root,
    bin: &mut Vec<u8>,
    fallback_len: &mut usize,
    data_buf: Index<gltf_json::Buffer>,
    fallback_buf: Index<gltf_json::Buffer>,
    encoded: &[u8],
    count: usize,
    stride: usize,
    use_u16: bool,
) -> Result<Index<accessor::Accessor>, Error> {
    pad4(bin);
    let off = bin.len();
    bin.extend_from_slice(encoded);
    let enc_len = encoded.len();
    let uncomp = stride * count;
    let fb_off = *fallback_len;
    *fallback_len += uncomp;

    let view = root.push(buffer::View {
        buffer: fallback_buf,
        byte_length: USize64::from(uncomp),
        byte_offset: Some(USize64::from(fb_off)),
        byte_stride: None,
        extensions: Some(gltf_json::extensions::buffer::View {
            others: ext_map(json!({
                "EXT_meshopt_compression": {
                    "buffer": data_buf.value(),
                    "byteOffset": off,
                    "byteLength": enc_len,
                    "byteStride": stride,
                    "count": count,
                    "mode": "TRIANGLES"
                }
            })),
        }),
        extras: Default::default(),
        name: None,
        target: Some(Valid(buffer::Target::ElementArrayBuffer)),
    });
    Ok(root.push(accessor::Accessor {
        buffer_view: Some(view),
        byte_offset: None,
        count: USize64::from(count),
        component_type: Valid(accessor::GenericComponentType(if use_u16 {
            accessor::ComponentType::U16
        } else {
            accessor::ComponentType::U32
        })),
        extensions: Default::default(),
        extras: Default::default(),
        max: None,
        min: None,
        name: None,
        normalized: false,
        sparse: None,
        type_: Valid(accessor::Type::Scalar),
    }))
}

fn push_jpeg(
    root: &mut Root,
    bin: &mut Vec<u8>,
    data_buf: Index<gltf_json::Buffer>,
    jpeg: &[u8],
) -> Result<Index<material::Material>, Error> {
    pad4(bin);
    let off = bin.len();
    bin.extend_from_slice(jpeg);
    let view = root.push(buffer::View {
        buffer: data_buf,
        byte_length: USize64::from(jpeg.len()),
        byte_offset: Some(USize64::from(off)),
        byte_stride: None,
        extensions: Default::default(),
        extras: Default::default(),
        name: None,
        target: None,
    });
    let image = root.push(gimage::Image {
        buffer_view: Some(view),
        mime_type: Some(gimage::MimeType("image/jpeg".into())),
        name: None,
        uri: None,
        extensions: Default::default(),
        extras: Default::default(),
    });
    let tex = root.push(texture::Texture {
        extensions: Default::default(),
        extras: Default::default(),
        name: None,
        sampler: None,
        source: image,
    });
    let pbr = material::PbrMetallicRoughness {
        base_color_texture: Some(texture::Info {
            extensions: Default::default(),
            extras: Default::default(),
            index: tex,
            tex_coord: 0,
        }),
        ..Default::default()
    };
    Ok(root.push(material::Material {
        alpha_cutoff: None,
        alpha_mode: Valid(material::AlphaMode::Opaque),
        double_sided: false,
        extensions: Default::default(),
        extras: Default::default(),
        name: None,
        pbr_metallic_roughness: pbr,
        normal_texture: None,
        occlusion_texture: None,
        emissive_texture: None,
        emissive_factor: material::EmissiveFactor([0.0, 0.0, 0.0]),
    }))
}

fn ext_map(v: Value) -> Map<String, Value> {
    match v {
        Value::Object(m) => m,
        _ => Map::new(),
    }
}

fn quant_i16(x: f32, scale: f32) -> i16 {
    (x / scale).round().clamp(-32767.0, 32767.0) as i16
}

fn quant_snorm8(x: f32) -> i8 {
    (x.clamp(-1.0, 1.0) * 127.0).round() as i8
}

fn quant_unorm16(x: f32) -> u16 {
    (x.clamp(0.0, 1.0) * 65535.0).round() as u16
}

fn pad4(buf: &mut Vec<u8>) {
    while !buf.len().is_multiple_of(4) {
        buf.push(0);
    }
}
