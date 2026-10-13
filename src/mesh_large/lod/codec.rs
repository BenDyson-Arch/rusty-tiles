//! Proxy GLB writing, and the GLB decoder H17 certifies from: published leaf and proxy
//! bytes are decoded exactly as a client does (meshopt, `u * scale + translation` in f32),
//! never re-derived from the producer's in-memory geometry.
use super::super::leaves::geometry::{LatticePlan, PositionStorage};
use crate::{
    content_integrity::meshopt::{self as codec, Mode},
    glb::{self, MeshoptLayout, MeshoptStream},
    JobError, JobErrorKind,
};
use serde_json::{json, Map, Value};
use std::collections::BTreeMap;

fn invalid(message: impl Into<String>) -> JobError {
    JobError::new(JobErrorKind::InvalidState, message.into())
}

/// One decoded triangle and its feature IDs (`u32::MAX` when absent).
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct DecodedFace {
    pub p: [[f32; 3]; 3],
    pub features: [u32; 2],
}

/// A property table's columns as unsigned integers (or f64 bit patterns for FLOAT64).
#[derive(Clone, Debug, Default)]
pub(crate) struct Table {
    pub class: String,
    pub count: usize,
    pub columns: BTreeMap<String, Vec<u64>>,
}

#[derive(Clone, Debug, Default)]
pub(crate) struct Decoded {
    pub faces: Vec<DecodedFace>,
    pub tables: Vec<Table>,
}

struct Reader<'a> {
    doc: Value,
    bin: &'a [u8],
}

impl Reader<'_> {
    fn view(&self, index: usize) -> Result<Vec<u8>, JobError> {
        let view = &self.doc["bufferViews"][index];
        if let Some(ext) = view
            .get("extensions")
            .and_then(|e| e.get("EXT_meshopt_compression"))
        {
            let start = ext["byteOffset"].as_u64().unwrap_or(0) as usize;
            let length = ext["byteLength"]
                .as_u64()
                .ok_or_else(|| invalid("meshopt view without length"))? as usize;
            let count = ext["count"]
                .as_u64()
                .ok_or_else(|| invalid("meshopt view without count"))? as usize;
            let stride = ext["byteStride"]
                .as_u64()
                .ok_or_else(|| invalid("meshopt view without stride"))? as usize;
            let mode = match ext["mode"].as_str() {
                Some("ATTRIBUTES") => Mode::Attributes,
                Some("TRIANGLES") => Mode::Triangles,
                Some("INDICES") => Mode::Indices,
                _ => return Err(invalid("meshopt view mode")),
            };
            let bytes = self
                .bin
                .get(start..start + length)
                .ok_or_else(|| invalid("meshopt view outside BIN"))?;
            codec::check_header(bytes, mode).map_err(|e| invalid(e.to_string()))?;
            return codec::decode(bytes, count, stride, mode).map_err(|e| invalid(e.to_string()));
        }
        let start = view["byteOffset"].as_u64().unwrap_or(0) as usize;
        let length = view["byteLength"]
            .as_u64()
            .ok_or_else(|| invalid("view without length"))? as usize;
        Ok(self
            .bin
            .get(start..start + length)
            .ok_or_else(|| invalid("view outside BIN"))?
            .to_vec())
    }

    /// Elements of an accessor as raw little-endian component bytes, `width` bytes each.
    fn accessor(&self, index: usize) -> Result<(Vec<Vec<u8>>, u64, usize), JobError> {
        let accessor = &self.doc["accessors"][index];
        let count = accessor["count"]
            .as_u64()
            .ok_or_else(|| invalid("accessor count"))? as usize;
        let component = accessor["componentType"]
            .as_u64()
            .ok_or_else(|| invalid("accessor component"))?;
        let size = match component {
            5121 => 1,
            5123 => 2,
            5125 | 5126 => 4,
            _ => return Err(invalid("accessor component type")),
        };
        let width = match accessor["type"].as_str() {
            Some("SCALAR") => 1,
            Some("VEC2") => 2,
            Some("VEC3") => 3,
            Some("VEC4") => 4,
            _ => return Err(invalid("accessor type")),
        };
        let view_index = accessor["bufferView"]
            .as_u64()
            .ok_or_else(|| invalid("accessor without view"))? as usize;
        let bytes = self.view(view_index)?;
        let stride = self.doc["bufferViews"][view_index]["byteStride"]
            .as_u64()
            .map_or(size * width, |s| s as usize);
        let offset = accessor["byteOffset"].as_u64().unwrap_or(0) as usize;
        let mut out = Vec::with_capacity(count);
        for i in 0..count {
            let start = offset + i * stride;
            out.push(
                bytes
                    .get(start..start + size * width)
                    .ok_or_else(|| invalid("accessor outside its view"))?
                    .to_vec(),
            );
        }
        Ok((out, component, width))
    }
}

fn f32_exact(value: &Value) -> Result<f32, JobError> {
    let v = value.as_f64().ok_or_else(|| invalid("non-numeric TRS"))?;
    let f = v as f32;
    if f64::from(f) != v {
        return Err(invalid("TRS value is not an exact f32"));
    }
    Ok(f)
}

/// Decode an H8 leaf or H17 proxy GLB: every triangle's positions in client order and its
/// feature IDs, plus every property table.
pub(crate) fn decode(bytes: &[u8]) -> Result<Decoded, JobError> {
    let u32_at = |i: usize| -> Result<u32, JobError> {
        Ok(u32::from_le_bytes(
            bytes
                .get(i..i + 4)
                .ok_or_else(|| invalid("GLB truncated"))?
                .try_into()
                .unwrap(),
        ))
    };
    if bytes.get(..4) != Some(b"glTF") || u32_at(4)? != 2 || u32_at(8)? as usize != bytes.len() {
        return Err(invalid("not a GLB 2.0"));
    }
    let json_length = u32_at(12)? as usize;
    if bytes.get(16..20) != Some(b"JSON") {
        return Err(invalid("GLB JSON chunk"));
    }
    let doc: Value = serde_json::from_slice(&bytes[20..20 + json_length])
        .map_err(|e| invalid(format!("GLB JSON: {e}")))?;
    let bin_start = 20 + json_length;
    let bin = if bin_start + 8 <= bytes.len() && &bytes[bin_start + 4..bin_start + 8] == b"BIN\0"
    {
        let length = u32_at(bin_start)? as usize;
        &bytes[bin_start + 8..bin_start + 8 + length]
    } else {
        &[][..]
    };
    let reader = Reader { doc, bin };
    let doc = &reader.doc;
    let mut faces = Vec::new();
    let scene = doc["scene"].as_u64().unwrap_or(0) as usize;
    for root in doc["scenes"][scene]["nodes"]
        .as_array()
        .ok_or_else(|| invalid("GLB without scene nodes"))?
    {
        let node = &doc["nodes"][root.as_u64().ok_or_else(|| invalid("node index"))? as usize];
        let translation = match node.get("translation") {
            Some(t) => [f32_exact(&t[0])?, f32_exact(&t[1])?, f32_exact(&t[2])?],
            None => [0.; 3],
        };
        let scale = match node.get("scale") {
            Some(s) => [f32_exact(&s[0])?, f32_exact(&s[1])?, f32_exact(&s[2])?],
            None => [1.; 3],
        };
        let Some(mesh) = node.get("mesh").and_then(Value::as_u64) else {
            continue;
        };
        for primitive in doc["meshes"][mesh as usize]["primitives"]
            .as_array()
            .ok_or_else(|| invalid("mesh without primitives"))?
        {
            if primitive.get("mode").and_then(Value::as_u64).unwrap_or(4) != 4 {
                return Err(invalid("non-triangle primitive"));
            }
            let attributes = &primitive["attributes"];
            let (positions, component, _) = reader.accessor(
                attributes["POSITION"]
                    .as_u64()
                    .ok_or_else(|| invalid("no POSITION"))? as usize,
            )?;
            let decoded: Vec<[f32; 3]> = positions
                .iter()
                .map(|e| {
                    std::array::from_fn(|a| match component {
                        5123 => {
                            let u = u16::from_le_bytes([e[2 * a], e[2 * a + 1]]);
                            // Client order: exact u*s, then one rounded addition.
                            let p = f32::from(u) * scale[a] + translation[a];
                            if p == 0. {
                                0.
                            } else {
                                p
                            }
                        }
                        _ => {
                            let v = f32::from_le_bytes(e[4 * a..4 * a + 4].try_into().unwrap());
                            v * scale[a] + translation[a]
                        }
                    })
                })
                .collect();
            let feature = |name: &str| -> Result<Option<Vec<u32>>, JobError> {
                let Some(index) = attributes.get(name).and_then(Value::as_u64) else {
                    return Ok(None);
                };
                let (values, component, _) = reader.accessor(index as usize)?;
                values
                    .iter()
                    .map(|e| match component {
                        5126 => {
                            let f = f32::from_le_bytes(e[..4].try_into().unwrap());
                            if f < 0. || f.fract() != 0. {
                                return Err(invalid("feature ID is not a whole number"));
                            }
                            Ok(f as u32)
                        }
                        5125 => Ok(u32::from_le_bytes(e[..4].try_into().unwrap())),
                        5123 => Ok(u32::from(u16::from_le_bytes([e[0], e[1]]))),
                        _ => Ok(u32::from(e[0])),
                    })
                    .collect::<Result<Vec<_>, _>>()
                    .map(Some)
            };
            let f0 = feature("_FEATURE_ID_0")?;
            let f1 = feature("_FEATURE_ID_1")?;
            let indices: Vec<usize> = match primitive.get("indices").and_then(Value::as_u64) {
                Some(index) => {
                    let (values, component, _) = reader.accessor(index as usize)?;
                    values
                        .iter()
                        .map(|e| match component {
                            5125 => u32::from_le_bytes(e[..4].try_into().unwrap()) as usize,
                            5123 => usize::from(u16::from_le_bytes([e[0], e[1]])),
                            _ => usize::from(e[0]),
                        })
                        .collect()
                }
                None => (0..decoded.len()).collect(),
            };
            if !indices.len().is_multiple_of(3) || indices.iter().any(|&i| i >= decoded.len()) {
                return Err(invalid("triangle indices out of range"));
            }
            for t in indices.chunks_exact(3) {
                let id = |f: &Option<Vec<u32>>| f.as_ref().map_or(u32::MAX, |v| v[t[0]]);
                faces.push(DecodedFace {
                    p: [decoded[t[0]], decoded[t[1]], decoded[t[2]]],
                    features: [id(&f0), id(&f1)],
                });
            }
        }
    }
    let mut tables = Vec::new();
    if let Some(metadata) = doc
        .get("extensions")
        .and_then(|e| e.get("EXT_structural_metadata"))
    {
        let classes = &metadata["schema"]["classes"];
        for table in metadata["propertyTables"].as_array().into_iter().flatten() {
            let class = table["class"].as_str().unwrap_or_default().to_owned();
            let count = table["count"].as_u64().unwrap_or(0) as usize;
            let mut columns = BTreeMap::new();
            for (name, property) in table["properties"].as_object().into_iter().flatten() {
                let component = classes[&class]["properties"][name]["componentType"]
                    .as_str()
                    .unwrap_or_default();
                let width = match component {
                    "UINT8" => 1,
                    "UINT16" => 2,
                    "UINT32" | "FLOAT32" => 4,
                    "UINT64" | "FLOAT64" => 8,
                    _ => return Err(invalid("property component type")),
                };
                let bytes = reader.view(
                    property["values"]
                        .as_u64()
                        .ok_or_else(|| invalid("property values"))? as usize,
                )?;
                if bytes.len() < count * width {
                    return Err(invalid("property column shorter than its table"));
                }
                let values = bytes
                    .chunks_exact(width)
                    .take(count)
                    .map(|c| {
                        let mut b = [0u8; 8];
                        b[..width].copy_from_slice(c);
                        u64::from_le_bytes(b)
                    })
                    .collect();
                columns.insert(name.clone(), values);
            }
            tables.push(Table {
                class,
                count,
                columns,
            });
        }
    }
    Ok(Decoded { faces, tables })
}

/// One primitive of a proxy: its identity and indexed geometry over source positions.
#[derive(Clone, Debug)]
pub(crate) struct ProxyPrimitive {
    pub key: (u32, u32, u32),
    pub expanded_ordinal: u64,
    pub material: u32,
    /// Baked source positions (existing source vertices only).
    pub positions: Vec<[f32; 3]>,
    pub ell: Vec<u8>,
    pub faces: Vec<[u32; 3]>,
}

/// A published removal/fill record column set.
#[derive(Clone, Debug)]
pub(crate) struct RecordTable {
    pub class: &'static str,
    /// Column name, component type and values per row.
    pub columns: Vec<(&'static str, &'static str, Vec<u64>)>,
}

impl RecordTable {
    fn rows(&self) -> usize {
        self.columns.first().map_or(0, |c| c.2.len())
    }
}

pub(crate) struct EncodedProxy {
    pub glb: Vec<u8>,
    /// Decoded corners per face in output order (primitives in order, faces in order).
    pub faces: Vec<[[f32; 3]; 3]>,
}

const TABLE_ROWS: usize = crate::mesh_large::config::CONSUMER_TABLE_ROWS as usize;

fn component_width(component: &str) -> usize {
    match component {
        "UINT8" => 1,
        "UINT16" => 2,
        "UINT32" | "FLOAT32" => 4,
        _ => 8,
    }
}

fn normals(decoded: &[[f32; 3]], faces: &[[u32; 3]]) -> Vec<[f32; 3]> {
    let mut sum = vec![[0f64; 3]; decoded.len()];
    for f in faces {
        let p = f.map(|i| decoded[i as usize].map(f64::from));
        let e1 = [p[1][0] - p[0][0], p[1][1] - p[0][1], p[1][2] - p[0][2]];
        let e2 = [p[2][0] - p[0][0], p[2][1] - p[0][1], p[2][2] - p[0][2]];
        let n = [
            e1[1] * e2[2] - e1[2] * e2[1],
            e1[2] * e2[0] - e1[0] * e2[2],
            e1[0] * e2[1] - e1[1] * e2[0],
        ];
        for &i in f {
            for a in 0..3 {
                sum[i as usize][a] += n[a];
            }
        }
    }
    sum.iter()
        .map(|n| {
            let length = (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt();
            if length > 0. && length.is_finite() {
                n.map(|v| (v / length) as f32)
            } else {
                [0., 1., 0.]
            }
        })
        .collect()
}

/// Encode a proxy: one scene node and mesh per primitive (as H8 leaves), positions on the
/// frozen lattice at `depth`, smooth normals, a flat material, `_FEATURE_ID_0` into the
/// `source_primitive` table, and the removal/fill tables in <= 8192-row tables.
pub(crate) fn encode(
    primitives: &[ProxyPrimitive],
    materials: &[Value],
    lattice: &LatticePlan,
    depth: u32,
    records: &[RecordTable],
) -> Result<EncodedProxy, JobError> {
    if primitives.is_empty() || primitives.len() > TABLE_ROWS {
        return Err(invalid("proxy primitive count outside 1..=8192"));
    }
    let mut binary: Vec<u8> = Vec::new();
    let mut views: Vec<Value> = Vec::new();
    let mut streams: Vec<Option<MeshoptStream>> = Vec::new();
    let mut accessors: Vec<Value> = Vec::new();
    let mut view = |bytes: &[u8], stream: Option<MeshoptStream>, binary: &mut Vec<u8>| {
        let offset = glb::append_aligned(binary, 8, bytes);
        let mut v = json!({"buffer":0,"byteOffset":offset,"byteLength":bytes.len()});
        if let Some(MeshoptStream::Attributes { stride, .. }) = stream {
            v["byteStride"] = json!(stride);
            v["target"] = json!(34962);
        }
        if let Some(MeshoptStream::TriangleIndices { .. }) = stream {
            v["target"] = json!(34963);
        }
        views.push(v);
        streams.push(stream);
        views.len() - 1
    };
    let mut nodes = Vec::new();
    let mut meshes = Vec::new();
    let mut out_faces = Vec::new();
    let mut quantized = false;
    for (row, primitive) in primitives.iter().enumerate() {
        if primitive.faces.is_empty() || primitive.positions.is_empty() {
            return Err(invalid("empty proxy primitive"));
        }
        let count = primitive.positions.len();
        // `encode` takes whole triangles: pad by repeating the last position.
        let mut points = primitive.positions.clone();
        let mut ell = primitive.ell.clone();
        while !points.len().is_multiple_of(3) {
            points.push(*points.last().unwrap());
            ell.push(*ell.last().unwrap());
        }
        let encoded = lattice.encode(&points, &ell, depth)?;
        let decoded = &encoded.decoded[..count];
        let mut node = json!({"mesh":meshes.len()});
        let position = match &encoded.storage {
            PositionStorage::Quantized {
                codes,
                translation,
                scale,
            } => {
                quantized = true;
                let mut bytes = Vec::with_capacity(count * 8);
                let mut min = [u16::MAX; 3];
                let mut max = [0u16; 3];
                for code in &codes[..count] {
                    for (a, v) in code.iter().enumerate() {
                        bytes.extend_from_slice(&v.to_le_bytes());
                        min[a] = min[a].min(*v);
                        max[a] = max[a].max(*v);
                    }
                    bytes.extend_from_slice(&[0, 0]);
                }
                let v = view(&bytes, Some(MeshoptStream::Attributes { count, stride: 8 }), &mut binary);
                accessors.push(json!({"bufferView":v,"componentType":5123,"count":count,"type":"VEC3","normalized":false,"min":min,"max":max}));
                node["translation"] = json!(translation.map(f64::from));
                node["scale"] = json!([f64::from(*scale), f64::from(*scale), f64::from(*scale)]);
                accessors.len() - 1
            }
            PositionStorage::Float { positions } => {
                let mut bytes = Vec::with_capacity(count * 12);
                let mut min = [f32::INFINITY; 3];
                let mut max = [f32::NEG_INFINITY; 3];
                for p in &positions[..count] {
                    for (a, v) in p.iter().enumerate() {
                        bytes.extend_from_slice(&v.to_le_bytes());
                        min[a] = min[a].min(*v);
                        max[a] = max[a].max(*v);
                    }
                }
                let v = view(&bytes, Some(MeshoptStream::Attributes { count, stride: 12 }), &mut binary);
                accessors.push(json!({"bufferView":v,"componentType":5126,"count":count,"type":"VEC3","min":min.map(f64::from),"max":max.map(f64::from)}));
                accessors.len() - 1
            }
        };
        let normal = normals(decoded, &primitive.faces);
        let mut bytes = Vec::with_capacity(count * 12);
        for n in &normal {
            for v in n {
                bytes.extend_from_slice(&v.to_le_bytes());
            }
        }
        let v = view(&bytes, Some(MeshoptStream::Attributes { count, stride: 12 }), &mut binary);
        accessors.push(json!({"bufferView":v,"componentType":5126,"count":count,"type":"VEC3"}));
        let normal_id = accessors.len() - 1;
        let mut bytes = Vec::with_capacity(count * 4);
        for _ in 0..count {
            bytes.extend_from_slice(&(row as f32).to_le_bytes());
        }
        let v = view(&bytes, Some(MeshoptStream::Attributes { count, stride: 4 }), &mut binary);
        accessors.push(json!({"bufferView":v,"componentType":5126,"count":count,"type":"SCALAR"}));
        let feature_id = accessors.len() - 1;
        let wide = count > usize::from(u16::MAX);
        let width = if wide { 4 } else { 2 };
        let mut bytes = Vec::with_capacity(primitive.faces.len() * 3 * width);
        for f in &primitive.faces {
            for &i in f {
                if i as usize >= count {
                    return Err(invalid("proxy face index out of range"));
                }
                if wide {
                    bytes.extend_from_slice(&i.to_le_bytes());
                } else {
                    bytes.extend_from_slice(&(i as u16).to_le_bytes());
                }
            }
            out_faces.push(f.map(|i| decoded[i as usize]));
        }
        let indices = primitive.faces.len() * 3;
        let v = view(
            &bytes,
            Some(MeshoptStream::TriangleIndices {
                count: indices,
                width,
                vertex_count: count,
            }),
            &mut binary,
        );
        accessors.push(json!({"bufferView":v,"componentType":if wide {5125} else {5123},"count":indices,"type":"SCALAR"}));
        let index_id = accessors.len() - 1;
        let material = materials
            .get(primitive.material as usize)
            .cloned()
            .unwrap_or_else(|| json!({"pbrMetallicRoughness":{"metallicFactor":0.0,"roughnessFactor":1.0}}));
        let material_id = row;
        let _ = material;
        meshes.push(json!({"primitives":[{"attributes":{"POSITION":position,"NORMAL":normal_id,"_FEATURE_ID_0":feature_id},
            "indices":index_id,"mode":4,"material":material_id,
            "extensions":{"EXT_mesh_features":{"featureIds":[{"featureCount":primitives.len(),"attribute":0,"propertyTable":0,"label":"source_primitive"}]}}}]}));
        nodes.push(node);
    }
    let materials_json: Vec<Value> = primitives
        .iter()
        .map(|p| {
            materials.get(p.material as usize).cloned().unwrap_or_else(
                || json!({"pbrMetallicRoughness":{"metallicFactor":0.0,"roughnessFactor":1.0}}),
            )
        })
        .collect();
    // Tables: the present primitives, then removal/fill records split at 8192 rows.
    let mut classes = Map::new();
    let mut tables = Vec::new();
    let mut add_table = |class: &str,
                         columns: &[(&str, &str, &[u64])],
                         rows: std::ops::Range<usize>,
                         binary: &mut Vec<u8>,
                         classes: &mut Map<String, Value>| {
        let mut schema = Map::new();
        let mut properties = Map::new();
        for (name, component, values) in columns {
            let width = component_width(component);
            let mut bytes = Vec::with_capacity(rows.len() * width);
            for v in &values[rows.clone()] {
                bytes.extend_from_slice(&v.to_le_bytes()[..width]);
            }
            let id = view(&bytes, None, binary);
            properties.insert((*name).into(), json!({"values":id}));
            schema.insert(
                (*name).into(),
                json!({"type":"SCALAR","componentType":component,"required":true}),
            );
        }
        classes.insert(class.into(), json!({"properties":schema}));
        tables.push(json!({"class":class,"count":rows.len(),"properties":properties}));
    };
    let identity: [(&str, &str, Vec<u64>); 5] = [
        ("node_index", "UINT32", primitives.iter().map(|p| u64::from(p.key.0)).collect()),
        ("mesh_index", "UINT32", primitives.iter().map(|p| u64::from(p.key.1)).collect()),
        ("primitive_index", "UINT32", primitives.iter().map(|p| u64::from(p.key.2)).collect()),
        ("expanded_ordinal", "UINT64", primitives.iter().map(|p| p.expanded_ordinal).collect()),
        ("material_index", "UINT32", primitives.iter().map(|p| u64::from(p.material)).collect()),
    ];
    let columns: Vec<(&str, &str, &[u64])> =
        identity.iter().map(|(n, c, v)| (*n, *c, v.as_slice())).collect();
    add_table("source_primitive", &columns, 0..primitives.len(), &mut binary, &mut classes);
    for record in records {
        let rows = record.rows();
        let columns: Vec<(&str, &str, &[u64])> = record
            .columns
            .iter()
            .map(|(n, c, v)| (*n, *c, v.as_slice()))
            .collect();
        let mut start = 0;
        while start < rows {
            let end = (start + TABLE_ROWS).min(rows);
            add_table(record.class, &columns, start..end, &mut binary, &mut classes);
            start = end;
        }
    }
    let scene_nodes: Vec<usize> = (0..nodes.len()).collect();
    let mut document = json!({"asset":{"version":"2.0","generator":"rusty-tiles H17 proxy"},"scene":0,
        "scenes":[{"nodes":scene_nodes}],"nodes":nodes,"meshes":meshes,"materials":materials_json,
        "buffers":[{"byteLength":binary.len()}],"bufferViews":views,"accessors":accessors,
        "extensionsUsed":["EXT_mesh_features","EXT_structural_metadata"],
        "extensions":{"EXT_structural_metadata":{"schema":{"id":"rusty_tiles_large_proxy_v1","classes":classes},"propertyTables":tables}}});
    if quantized {
        glb::add_extension(&mut document, "KHR_mesh_quantization", true)
            .map_err(|e| invalid(e.to_string()))?;
    }
    let compressed = glb::meshopt_compress(
        &mut document,
        &binary,
        MeshoptLayout {
            align: 8,
            explicit_filter: true,
        },
        |i, _| Ok(streams[i]),
    )
    .map_err(|e| invalid(e.to_string()))?;
    let json_bytes = serde_json::to_vec(&document).map_err(|e| invalid(e.to_string()))?;
    let reparsed: Value = serde_json::from_slice(&json_bytes).map_err(|e| invalid(e.to_string()))?;
    if reparsed["nodes"] != document["nodes"] {
        return Err(invalid("proxy JSON real TRS round trip mismatch"));
    }
    let glb = glb::encode_glb(&document, &compressed).map_err(|e| invalid(e.to_string()))?;
    Ok(EncodedProxy {
        glb,
        faces: out_faces,
    })
}

/// Faces equal up to a rotation of their corners (the meshopt index codec may rotate a
/// triangle, preserving its winding).
pub(crate) fn same_face(a: &[[f32; 3]; 3], b: &[[f32; 3]; 3]) -> bool {
    let bits = |f: &[[f32; 3]; 3]| f.map(|p| p.map(f32::to_bits));
    let (a, b) = (bits(a), bits(b));
    (0..3).any(|r| (0..3).all(|i| a[i] == b[(i + r) % 3]))
}
