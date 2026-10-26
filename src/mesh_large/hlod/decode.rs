//! Client-order GLB decoder: published leaf and coarse-tile bytes are decoded exactly as a
//! client does (meshopt, `u * scale + translation` in f32), never re-derived from the
//! producer's in-memory geometry. HLOD's encode check and its tests read it.
use crate::{
    content_integrity::meshopt::{self as codec, Mode},
    JobError, JobErrorKind,
};
use serde_json::Value;
use std::collections::BTreeMap;

fn invalid(message: impl Into<String>) -> JobError {
    JobError::new(JobErrorKind::InvalidState, message.into())
}

/// One decoded triangle and, per feature-ID set of its primitive (in `featureIds` order),
/// its feature ID and that set's property table.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct DecodedFace {
    pub p: [[f32; 3]; 3],
    pub features: Vec<(u32, Option<usize>)>,
    /// Exact POSITION storage words per corner: u16 codes widened, or f32 bit patterns.
    pub raw: [[u32; 3]; 3],
    /// The decoded glTF primitive this face belongs to (index into `Decoded::primitives`).
    pub primitive: u32,
    /// Every feature-ID set has the same value at the face's three corners (`features`
    /// holds corner 0's).
    pub uniform_features: bool,
}

/// One decoded glTF primitive: its scene node TRS bits and declared feature-ID sets.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct DecodedPrimitive {
    pub node: u32,
    pub mesh: u32,
    pub index: u32,
    /// Node translation and scale as f32 bit patterns (identity when absent).
    pub translation: [u32; 3],
    pub scale: [u32; 3],
    /// Quantized (u16) or float POSITION storage.
    pub quantized: bool,
    /// The `EXT_mesh_features.featureIds` array exactly as published.
    pub feature_ids: Value,
    pub material: Option<u32>,
}

impl DecodedFace {
    /// The feature ID of the set whose property table has class `class`.
    pub(crate) fn feature(&self, tables: &[Table], class: &str) -> Option<(u32, usize)> {
        self.features.iter().find_map(|&(id, table)| {
            let t = table?;
            (tables.get(t)?.class == class).then_some((id, t))
        })
    }
}

/// A property table's columns as unsigned integers (or f64 bit patterns for FLOAT64).
#[derive(Clone, Debug, Default)]
pub(crate) struct Table {
    pub class: String,
    pub count: usize,
    pub columns: BTreeMap<String, Vec<u64>>,
    /// Column component types (`UINT8`, `UINT32`, `UINT64`, `FLOAT64`, ...).
    pub components: BTreeMap<String, String>,
}

#[derive(Clone, Debug, Default)]
pub(crate) struct Decoded {
    pub faces: Vec<DecodedFace>,
    pub tables: Vec<Table>,
    pub primitives: Vec<DecodedPrimitive>,
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
                .ok_or_else(|| invalid("meshopt view without length"))?
                as usize;
            let count = ext["count"]
                .as_u64()
                .ok_or_else(|| invalid("meshopt view without count"))?
                as usize;
            let stride = ext["byteStride"]
                .as_u64()
                .ok_or_else(|| invalid("meshopt view without stride"))?
                as usize;
            let mode = match ext["mode"].as_str() {
                Some("ATTRIBUTES") => Mode::Attributes,
                Some("TRIANGLES") => Mode::Triangles,
                Some("INDICES") => Mode::Indices,
                _ => return Err(invalid("meshopt view mode")),
            };
            let filter = codec::Filter::parse(ext["filter"].as_str().unwrap_or("NONE"))
                .ok_or_else(|| invalid("meshopt view filter"))?;
            let bytes = start
                .checked_add(length)
                .and_then(|end| self.bin.get(start..end))
                .ok_or_else(|| invalid("meshopt view outside BIN"))?;
            codec::check_header(bytes, mode).map_err(|e| invalid(e.to_string()))?;
            return codec::decode_filtered(bytes, count, stride, mode, filter)
                .map_err(|e| invalid(e.to_string()));
        }
        let start = view["byteOffset"].as_u64().unwrap_or(0) as usize;
        let length = view["byteLength"]
            .as_u64()
            .ok_or_else(|| invalid("view without length"))? as usize;
        Ok(start
            .checked_add(length)
            .and_then(|end| self.bin.get(start..end))
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

/// Decode a leaf or coarse-tile GLB: every triangle's positions in client order and its
/// feature IDs, plus every property table.
pub(crate) fn decode(bytes: &[u8]) -> Result<Decoded, JobError> {
    decode_checked(bytes, &|| Ok(()))
}

/// [`decode`], polling `check` before every scene node, glTF primitive and property table.
pub(crate) fn decode_checked(
    bytes: &[u8],
    check: &dyn Fn() -> Result<(), JobError>,
) -> Result<Decoded, JobError> {
    check()?;
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
    let doc: Value = serde_json::from_slice(
        bytes
            .get(20..20 + json_length)
            .ok_or_else(|| invalid("GLB JSON chunk length"))?,
    )
    .map_err(|e| invalid(format!("GLB JSON: {e}")))?;
    let bin_start = 20 + json_length;
    let bin = if bin_start + 8 <= bytes.len() && &bytes[bin_start + 4..bin_start + 8] == b"BIN\0" {
        let length = u32_at(bin_start)? as usize;
        bytes
            .get(bin_start + 8..(bin_start + 8).saturating_add(length))
            .ok_or_else(|| invalid("GLB BIN chunk length"))?
    } else {
        &[][..]
    };
    let reader = Reader { doc, bin };
    let doc = &reader.doc;
    let mut faces = Vec::new();
    let mut decoded_primitives = Vec::new();
    let scene = doc["scene"].as_u64().unwrap_or(0) as usize;
    for root in doc["scenes"][scene]["nodes"]
        .as_array()
        .ok_or_else(|| invalid("GLB without scene nodes"))?
    {
        check()?;
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
        let node_index = root.as_u64().unwrap_or(0) as u32;
        for (primitive_index, primitive) in doc["meshes"][mesh as usize]["primitives"]
            .as_array()
            .ok_or_else(|| invalid("mesh without primitives"))?
            .iter()
            .enumerate()
        {
            check()?;
            if primitive.get("mode").and_then(Value::as_u64).unwrap_or(4) != 4 {
                return Err(invalid("non-triangle primitive"));
            }
            let attributes = &primitive["attributes"];
            let (positions, component, _) = reader.accessor(
                attributes["POSITION"]
                    .as_u64()
                    .ok_or_else(|| invalid("no POSITION"))? as usize,
            )?;
            let raw: Vec<[u32; 3]> = positions
                .iter()
                .map(|e| {
                    std::array::from_fn(|a| match component {
                        5123 => u32::from(u16::from_le_bytes([e[2 * a], e[2 * a + 1]])),
                        _ => u32::from_le_bytes(e[4 * a..4 * a + 4].try_into().unwrap()),
                    })
                })
                .collect();
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
            // Every feature-ID set the primitive declares, whatever their number.
            let mut sets: Vec<(Vec<u32>, Option<usize>)> = Vec::new();
            if let Some(ids) = primitive
                .get("extensions")
                .and_then(|e| e.get("EXT_mesh_features"))
                .and_then(|e| e.get("featureIds"))
                .and_then(Value::as_array)
            {
                for set in ids {
                    // Texture-based sets (H8 ID textures) carry no per-vertex IDs.
                    let Some(attribute) = set.get("attribute").and_then(Value::as_u64) else {
                        continue;
                    };
                    let values = feature(&format!("_FEATURE_ID_{attribute}"))?
                        .ok_or_else(|| invalid("feature-ID attribute missing"))?;
                    let table = set
                        .get("propertyTable")
                        .and_then(Value::as_u64)
                        .map(|t| t as usize);
                    sets.push((values, table));
                }
            }
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
            let primitive_slot = decoded_primitives.len() as u32;
            decoded_primitives.push(DecodedPrimitive {
                node: node_index,
                mesh: mesh as u32,
                index: primitive_index as u32,
                translation: translation.map(f32::to_bits),
                scale: scale.map(f32::to_bits),
                quantized: component == 5123,
                feature_ids: primitive
                    .get("extensions")
                    .and_then(|e| e.get("EXT_mesh_features"))
                    .and_then(|e| e.get("featureIds"))
                    .cloned()
                    .unwrap_or(Value::Array(Vec::new())),
                material: primitive
                    .get("material")
                    .and_then(Value::as_u64)
                    .map(|m| m as u32),
            });
            for t in indices.as_chunks::<3>().0 {
                faces.push(DecodedFace {
                    p: [decoded[t[0]], decoded[t[1]], decoded[t[2]]],
                    features: sets.iter().map(|(v, table)| (v[t[0]], *table)).collect(),
                    raw: [raw[t[0]], raw[t[1]], raw[t[2]]],
                    primitive: primitive_slot,
                    uniform_features: sets
                        .iter()
                        .all(|(v, _)| v[t[0]] == v[t[1]] && v[t[1]] == v[t[2]]),
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
            check()?;
            let class = table["class"].as_str().unwrap_or_default().to_owned();
            let count = table["count"].as_u64().unwrap_or(0) as usize;
            let mut columns = BTreeMap::new();
            let mut components = BTreeMap::new();
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
                components.insert(name.clone(), component.to_owned());
            }
            tables.push(Table {
                class,
                count,
                columns,
                components,
            });
        }
    }
    Ok(Decoded {
        faces,
        tables,
        primitives: decoded_primitives,
    })
}
