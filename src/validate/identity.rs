//! Archive-only integrity of HLOD's embedded attribute identity. Source truth is
//! deliberately separate: an in-range ID may still name the wrong source face.
//! Called after C1 admission, so codec/range/JSON budgets already hold. Decode
//! bounded identity views per payload; repeated scalar/corner work is charged
//! to the remaining payload-element budget before scanning. No archive inventory.
use super::{invalid, Budget, Error};
use crate::content_integrity::meshopt::{self, Mode};
use crate::content_integrity::numbers::{exact, Unsigned};
use serde_json::{value::RawValue, Value};
use std::{collections::BTreeMap, rc::Rc};

fn fail() -> Error {
    invalid("hlodFeatureIds: invalid source identity attribute or property table")
}
pub(super) fn uint(v: &Value) -> Result<usize, Error> {
    v.as_u64()
        .and_then(|n| usize::try_from(n).ok())
        .ok_or_else(fail)
}
pub(super) fn field<'a>(raw: &'a RawValue, key: &str) -> Option<&'a RawValue> {
    serde_json::from_str::<BTreeMap<String, &'a RawValue>>(raw.get())
        .ok()?
        .remove(key)
}
fn items(raw: Option<&RawValue>) -> Vec<&RawValue> {
    raw.and_then(|v| serde_json::from_str(v.get()).ok())
        .unwrap_or_default()
}
pub(super) fn declares(raw: &RawValue) -> bool {
    let label = |r: &RawValue, key| {
        field(r, key)
            .and_then(|v| serde_json::from_str::<String>(v.get()).ok())
            .is_some_and(|s| matches!(s.as_str(), "source_primitive" | "source_triangle"))
    };
    let tables = field(raw, "extensions")
        .and_then(|e| field(e, "EXT_structural_metadata"))
        .and_then(|m| field(m, "propertyTables"));
    if items(tables).iter().any(|t| label(t, "class")) {
        return true;
    }
    for mesh in items(field(raw, "meshes")) {
        for p in items(field(mesh, "primitives")) {
            let sets = field(p, "extensions")
                .and_then(|e| field(e, "EXT_mesh_features"))
                .and_then(|f| field(f, "featureIds"));
            if items(sets).iter().any(|f| label(f, "label")) {
                return true;
            }
        }
    }
    false
}

/// Project only identity-owned fields from already admitted raw JSON. Opaque
/// extras/extensions never become f64, and integer lexemes use C1's exact owner.
pub(super) fn project(raw: &RawValue, kind: &str) -> Result<Value, Error> {
    let element = match kind {
        "meshes" => Some("mesh"),
        "buffers" => Some("buffer"),
        "primitives" => Some("primitive"),
        "accessors" => Some("accessor"),
        "views" => Some("view"),
        "tables" => Some("table"),
        "sets" => Some("set"),
        "nodes" => Some("node"),
        "scenes" => Some("scene"),
        "numbers" => Some("number"),
        _ => None,
    };
    if let Some(element) = element {
        let values: Vec<&RawValue> = serde_json::from_str(raw.get())?;
        return values
            .into_iter()
            .map(|v| project(v, element))
            .collect::<Result<Vec<_>, _>>()
            .map(Value::Array);
    }
    if kind == "number" {
        return match exact(Some(raw)).map_err(super::format_error)? {
            Unsigned::Native(n) => Ok(Value::from(n)),
            _ => Err(fail()),
        };
    }
    if kind == "leaf" {
        return serde_json::from_str(raw.get()).map_err(Into::into);
    }
    if kind == "present" {
        return Ok(Value::Bool(true));
    }
    let fields: BTreeMap<String, &RawValue> = serde_json::from_str(raw.get())?;
    let mut out = serde_json::Map::new();
    for (key, raw) in fields {
        let next = match (kind, key.as_str()) {
            ("doc", "meshes") => Some("meshes"),
            ("doc", "nodes") => Some("nodes"),
            ("doc", "scenes") => Some("scenes"),
            ("doc", "scene") | ("node", "mesh") => Some("number"),
            ("scene", "nodes") | ("node", "children") => Some("numbers"),
            ("node", "matrix" | "translation" | "rotation" | "scale") => Some("leaf"),
            ("node", "skin" | "weights") | ("mesh", "weights") | ("primitive", "targets") => {
                Some("present")
            }
            ("doc", "accessors") => Some("accessors"),
            ("doc", "bufferViews") => Some("views"),
            ("doc", "buffers") => Some("buffers"),
            ("buffer", "byteLength") => Some("number"),
            ("buffer", "uri") => Some("present"),
            ("doc", "extensionsRequired") => Some("leaf"),
            (_, "extensions")
                if matches!(kind, "doc" | "primitive" | "view" | "buffer" | "node") =>
            {
                Some("extensions")
            }
            ("mesh", "primitives") => Some("primitives"),
            ("primitive", "attributes") => Some("attributes"),
            ("primitive", "indices" | "mode") => Some("number"),
            ("attributes", "POSITION" | "NORMAL") => Some("number"),
            ("attributes", k) if k.starts_with("TEXCOORD_") => Some("number"),
            ("extensions", "EXT_mesh_gpu_instancing") => Some("present"),
            ("attributes", k) if k.starts_with("_FEATURE_ID_") => Some("number"),
            ("accessor", "type" | "normalized") => Some("leaf"),
            ("accessor", "sparse") => Some("present"),
            ("accessor", "bufferView" | "byteOffset" | "count" | "componentType") => Some("number"),
            ("view", "buffer" | "byteOffset" | "byteLength" | "byteStride") => Some("number"),
            ("extensions", "EXT_meshopt_compression") => Some("codec"),
            ("extensions", "EXT_structural_metadata") => Some("metadata"),
            ("extensions", "EXT_mesh_features") => Some("features"),
            ("codec", "buffer" | "byteOffset" | "byteLength" | "byteStride" | "count") => {
                Some("number")
            }
            ("codec", "mode" | "filter" | "fallback") => Some("leaf"),
            ("metadata", "schema") => Some("schema"),
            ("metadata", "propertyTables") => Some("tables"),
            ("schema", "classes") => Some("classes"),
            ("classes", "source_primitive" | "source_triangle") => Some("class"),
            ("class", "properties") => Some("properties"),
            ("properties", _) => Some("property"),
            ("property", "type" | "componentType" | "required" | "normalized" | "array") => {
                Some("leaf")
            }
            ("property" | "column", "count" | "offset" | "scale" | "noData" | "default") => {
                Some("present")
            }
            ("table", "class") => Some("leaf"),
            ("table", "count") => Some("number"),
            ("table", "properties") => Some("columns"),
            ("columns", _) => Some("column"),
            ("column", "values") => Some("number"),
            ("features", "featureIds") => Some("sets"),
            ("set", "label") => Some("leaf"),
            ("set", "attribute" | "featureCount" | "propertyTable" | "nullFeatureId") => {
                Some("number")
            }
            _ => None,
        };
        if let Some(next) = next {
            out.insert(key, project(raw, next)?);
        }
    }
    Ok(Value::Object(out))
}

pub(super) struct Work {
    pub(super) used: u64,
    base: u64,
    budget: Budget,
    bytes: usize,
    cap: usize,
    views: BTreeMap<usize, Rc<Vec<u8>>>,
}
impl Work {
    pub(super) fn new(budget: Budget, base: u64, cap: usize) -> Self {
        Self {
            used: 0,
            base,
            budget,
            bytes: 0,
            cap,
            views: BTreeMap::new(),
        }
    }
    pub(super) fn charge(&mut self, count: usize) -> Result<(), Error> {
        let total =
            self.budget
                .charge(self.base.saturating_add(self.used), count as u64, || {
                    "hlodFeatureIds: identity work exceeds payload element budget".into()
                })?;
        self.used = total - self.base;
        Ok(())
    }
}

pub(super) fn slice(bytes: &[u8], offset: usize, length: usize) -> Result<&[u8], Error> {
    bytes
        .get(offset..offset.checked_add(length).ok_or_else(fail)?)
        .ok_or_else(fail)
}
fn decode_view(doc: &Value, bin: &[u8], id: usize) -> Result<Vec<u8>, Error> {
    let v = &doc["bufferViews"][id];
    if let Some(e) = v["extensions"].get("EXT_meshopt_compression") {
        if e["buffer"] != 0 || e["filter"].as_str().is_some_and(|f| f != "NONE") {
            return Err(fail());
        }
        let mode = match e["mode"].as_str() {
            Some("ATTRIBUTES") => Mode::Attributes,
            Some("TRIANGLES") => Mode::Triangles,
            Some("INDICES") => Mode::Indices,
            _ => return Err(fail()),
        };
        meshopt::decode(
            slice(
                bin,
                e["byteOffset"].as_u64().unwrap_or(0) as usize,
                uint(&e["byteLength"])?,
            )?,
            uint(&e["count"])?,
            uint(&e["byteStride"])?,
            mode,
        )
        .map_err(super::format_error)
    } else {
        if v["buffer"] != 0 {
            return Err(fail());
        }
        Ok(slice(
            bin,
            v["byteOffset"].as_u64().unwrap_or(0) as usize,
            uint(&v["byteLength"])?,
        )?
        .to_vec())
    }
}
pub(super) fn view(
    doc: &Value,
    bin: &[u8],
    id: usize,
    work: &mut Work,
) -> Result<Rc<Vec<u8>>, Error> {
    if let Some(data) = work.views.get(&id) {
        return Ok(data.clone());
    }
    let v = &doc["bufferViews"][id];
    let length = if let Some(e) = v["extensions"].get("EXT_meshopt_compression") {
        uint(&e["count"])?
            .checked_mul(uint(&e["byteStride"])?)
            .ok_or_else(fail)?
    } else {
        uint(&v["byteLength"])?
    };
    work.charge(length.div_ceil(4))?;
    work.bytes = work
        .bytes
        .checked_add(length)
        .filter(|n| *n <= work.cap)
        .ok_or_else(|| {
            Error::limit("hlodFeatureIds: decoded identity views exceed document budget")
        })?;
    let data = Rc::new(decode_view(doc, bin, id)?);
    work.views.insert(id, data.clone());
    Ok(data)
}

fn scalars(doc: &Value, bin: &[u8], id: usize, work: &mut Work) -> Result<Vec<u32>, Error> {
    let a = &doc["accessors"][id];
    if a["type"] != "SCALAR" || a["normalized"] == true || a.get("sparse").is_some() {
        return Err(fail());
    }
    let width = match a["componentType"].as_u64() {
        Some(5121) => 1,
        Some(5123) => 2,
        Some(5125 | 5126) => 4,
        _ => return Err(fail()),
    };
    work.charge(uint(&a["count"])?)?;
    let vi = uint(&a["bufferView"])?;
    let bytes = view(doc, bin, vi, work)?;
    let stride = doc["bufferViews"][vi]["byteStride"]
        .as_u64()
        .unwrap_or(width as u64) as usize;
    let offset = a["byteOffset"].as_u64().unwrap_or(0) as usize;
    let mut out = Vec::new();
    out.try_reserve_exact(uint(&a["count"])?)
        .map_err(|_| super::refused())?;
    for i in 0..uint(&a["count"])? {
        let start = i
            .checked_mul(stride)
            .and_then(|n| n.checked_add(offset))
            .ok_or_else(fail)?;
        let b = slice(&bytes, start, width)?;
        out.push(match width {
            1 => b[0] as u32,
            2 => u16::from_le_bytes(b.try_into().unwrap()) as u32,
            _ if a["componentType"] == 5126 => {
                let f = f32::from_le_bytes(b.try_into().unwrap());
                if !f.is_finite() || f < 0. || f.fract() != 0. || f as f64 > u32::MAX as f64 {
                    return Err(fail());
                }
                f as u32
            }
            _ => u32::from_le_bytes(b.try_into().unwrap()),
        });
    }
    Ok(out)
}

pub(super) fn inspect(
    bytes: &[u8],
    budget: Budget,
    already: u64,
    decoded_limit: usize,
    expected: bool,
) -> Result<(bool, u64), Error> {
    let embedded = bytes.get(..4) == Some(b"glTF");
    let length = if embedded {
        u32::from_le_bytes(bytes.get(12..16).ok_or_else(fail)?.try_into().unwrap()) as usize
    } else {
        0
    };
    let raw: &RawValue = serde_json::from_slice(if embedded {
        slice(bytes, 20, length)?
    } else {
        bytes
    })?;
    if !declares(raw) {
        return if expected {
            Err(fail())
        } else {
            Ok((false, 0))
        };
    }
    if !embedded {
        return Err(Error::unsupported("hlodFeatureIds: declared source identity requires embedded GLB; local glTF identity is outside this profile"));
    }
    let doc = project(raw, "doc")?;
    let mut work = Work::new(budget, already, decoded_limit);
    let metadata = &doc["extensions"]["EXT_structural_metadata"];
    let tables = metadata["propertyTables"].as_array();
    let has_table = tables.is_some_and(|ts| ts.iter().any(|t| t["class"] == "source_primitive"));
    let tables = tables.ok_or_else(fail)?;
    if !has_table {
        return Err(fail());
    }
    let buffers = doc["buffers"].as_array().ok_or_else(fail)?;
    if buffers.is_empty()
        || buffers.iter().any(|b| b.get("uri").is_some())
        || buffers.iter().skip(1).any(|b| {
            b["extensions"]["EXT_meshopt_compression"]["fallback"] != true
                || !doc["extensionsRequired"]
                    .as_array()
                    .is_some_and(|xs| xs.iter().any(|x| x == "EXT_meshopt_compression"))
        })
    {
        return Err(Error::unsupported(
            "hlodFeatureIds: source identity requires a URI-free embedded BIN buffer and only virtual meshopt fallback buffers",
        ));
    }
    let header = slice(bytes, 20 + length, 8)?;
    if &header[4..8] != b"BIN\0" {
        return Err(invalid(
            "hlodFeatureIds: source identity requires an actual BIN chunk",
        ));
    }
    let bin_length = u32::from_le_bytes(header[..4].try_into().unwrap()) as usize;
    let bin = slice(bytes, 28 + length, bin_length)?;
    let bin = slice(bin, 0, uint(&buffers[0]["byteLength"])?)?;
    for table in tables.iter().filter(|t| {
        matches!(
            t["class"].as_str(),
            Some("source_primitive" | "source_triangle")
        )
    }) {
        let class = table["class"].as_str().ok_or_else(fail)?;
        let props = metadata["schema"]["classes"][class]["properties"]
            .as_object()
            .ok_or_else(fail)?;
        let count = uint(&table["count"])?;
        if count == 0 {
            return Err(fail());
        }
        for key in [
            "node_index",
            "mesh_index",
            "primitive_index",
            "expanded_ordinal",
            "material_index",
        ] {
            if !props.contains_key(key)
                || props[key]["required"] != true
                || table["properties"].get(key).is_none()
            {
                return Err(fail());
            }
        }
        if class == "source_triangle" {
            for key in ["triangle_index", "vertex_0", "vertex_1", "vertex_2"] {
                if !props.contains_key(key)
                    || props[key]["required"] != true
                    || table["properties"].get(key).is_none()
                {
                    return Err(fail());
                }
            }
        }
        for key in table["properties"].as_object().ok_or_else(fail)?.keys() {
            if !props.contains_key(key) {
                return Err(fail());
            }
        }
        work.charge(props.len())?;
        for (key, schema) in props {
            if schema["required"] != true && table["properties"].get(key).is_none() {
                continue;
            }
            let column = &table["properties"][key];
            if schema.get("normalized").is_some_and(|v| v != false)
                || schema.get("array").is_some_and(|v| v != false)
                || ["count", "offset", "scale", "noData", "default"]
                    .iter()
                    .any(|key| schema.get(key).is_some() || column.get(key).is_some())
            {
                return Err(fail());
            }
            if schema["type"] != "SCALAR" {
                return Err(fail());
            }
            let width = match schema["componentType"].as_str() {
                Some("UINT8") => 1,
                Some("UINT32") => 4,
                Some("UINT64") => 8,
                _ => return Err(fail()),
            };
            work.charge(count)?;
            let data = view(
                &doc,
                bin,
                uint(&table["properties"][key]["values"])?,
                &mut work,
            )?;
            if data.len() < count.checked_mul(width).ok_or_else(fail)? {
                return Err(fail());
            }
        }
    }
    let has_triangles = tables.iter().any(|t| t["class"] == "source_triangle");
    for mesh in doc["meshes"].as_array().into_iter().flatten() {
        for p in mesh["primitives"].as_array().into_iter().flatten() {
            if let Some(attributes) = p["attributes"].as_object() {
                for (semantic, accessor) in attributes {
                    let shape = if semantic == "NORMAL" {
                        Some(("VEC3", "sourceNormals"))
                    } else if semantic.starts_with("TEXCOORD_") {
                        Some(("VEC2", "sourceTexcoords"))
                    } else {
                        None
                    };
                    if let Some((shape, label)) = shape {
                        if doc["accessors"][uint(accessor)?]["type"] != shape {
                            return Err(invalid(format!("{label}: decoded content accessor has wrong rendered semantic shape")));
                        }
                    }
                }
            }
            let sets = p["extensions"]["EXT_mesh_features"]["featureIds"]
                .as_array()
                .ok_or_else(fail)?;
            let mut source = false;
            let mut triangle = false;
            for set in sets {
                if !matches!(
                    set["label"].as_str(),
                    Some("source_primitive" | "source_triangle")
                ) {
                    continue;
                }
                let seen = if set["label"] == "source_primitive" {
                    &mut source
                } else {
                    &mut triangle
                };
                if *seen {
                    return Err(fail());
                }
                *seen = true;
                let table = tables.get(uint(&set["propertyTable"])?).ok_or_else(fail)?;
                let count = uint(&table["count"])?;
                if table["class"] != set["label"] || uint(&set["featureCount"])? != count {
                    return Err(fail());
                }
                let key = format!("_FEATURE_ID_{}", uint(&set["attribute"])?);
                let id = uint(&p["attributes"][&key])?;
                work.charge(uint(&doc["accessors"][id]["count"])?)?;
                let ids = scalars(&doc, bin, id, &mut work)?;
                if ids.len()
                    != uint(&doc["accessors"][uint(&p["attributes"]["POSITION"])?]["count"])?
                    || ids.iter().any(|&i| {
                        i as usize >= count
                            || set
                                .get("nullFeatureId")
                                .is_some_and(|n| n.as_u64() == Some(i as u64))
                    })
                {
                    return Err(fail());
                }
                let indices = p
                    .get("indices")
                    .map(|a| scalars(&doc, bin, uint(a)?, &mut work))
                    .transpose()?;
                let at = |i: usize| -> Result<u32, Error> {
                    let index = match &indices {
                        Some(v) => *v.get(i).ok_or_else(fail)? as usize,
                        None => i,
                    };
                    ids.get(index).copied().ok_or_else(fail)
                };
                let corners = indices.as_ref().map_or(ids.len(), Vec::len);
                work.charge(corners)?;
                if p["mode"].as_u64().unwrap_or(4) != 4 || !corners.is_multiple_of(3) {
                    return Err(fail());
                }
                for i in (0..corners).step_by(3) {
                    if at(i)? != at(i + 1)? || at(i)? != at(i + 2)? {
                        return Err(fail());
                    }
                }
            }
            if !source || (has_triangles && !triangle) {
                return Err(fail());
            }
        }
    }
    Ok((true, work.used))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn raw_projection_keeps_opaque_numbers_and_exact_integer_meaning() {
        let raw: &RawValue = serde_json::from_str(r#"{"extras":{"opaque":1e400},"accessors":[{"bufferView":0.0,"count":3e0,"componentType":5126,"type":"SCALAR"}]}"#).unwrap();
        assert!(!declares(raw));
        let doc = project(raw, "doc").unwrap();
        assert_eq!(uint(&doc["accessors"][0]["bufferView"]).unwrap(), 0);
        assert_eq!(uint(&doc["accessors"][0]["count"]).unwrap(), 3);
    }

    #[test]
    fn archive_expectation_refuses_fully_stripped_payload() {
        let bytes = br#"{"asset":{"version":"2.0"},"extras":{"opaque":1e400}}"#;
        let budget = Budget {
            key: super::super::LimitKey::TotalPayloadElements,
            value: 100,
            source: super::super::limits::Derivation::Explicit,
        };
        assert_eq!(inspect(bytes, budget, 0, 1024, false).unwrap(), (false, 0));
        assert!(inspect(bytes, budget, 0, 1024, true)
            .unwrap_err()
            .to_string()
            .contains("hlodFeatureIds"));
    }

    #[test]
    fn shared_views_decode_once_and_repeated_scalar_work_is_preflighted() {
        let doc = json!({"bufferViews":[{"buffer":0,"byteLength":12}],
            "accessors":[{"bufferView":0,"componentType":5126,"type":"SCALAR","count":3}]});
        let bin = [0; 12];
        let budget = Budget {
            key: super::super::LimitKey::TotalPayloadElements,
            value: 9,
            source: super::super::limits::Derivation::Explicit,
        };
        let mut work = Work::new(budget, 0, 12);
        assert_eq!(scalars(&doc, &bin, 0, &mut work).unwrap(), [0, 0, 0]);
        assert_eq!(work.used, 6); // 3 scalars + one 3-word view decode.
        assert_eq!(scalars(&doc, &bin, 0, &mut work).unwrap(), [0, 0, 0]);
        assert_eq!(work.used, 9);
        let error = scalars(&doc, &bin, 0, &mut work).unwrap_err();
        assert!(error.to_string().contains("identity work exceeds"));
        let limit = error.exceeded().unwrap();
        assert_eq!(limit.value, 9);
        assert_eq!(limit.observed, 12);
        assert_eq!(work.views.len(), 1);
        let mut small = Work::new(
            Budget {
                value: 100,
                ..budget
            },
            0,
            11,
        );
        assert!(view(&doc, &bin, 0, &mut small)
            .unwrap_err()
            .to_string()
            .contains("document budget"));
    }
}
