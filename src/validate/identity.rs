//! Archive-only integrity of HLOD's embedded attribute identity. Source truth is
//! deliberately separate: an in-range ID may still name the wrong source face.
//! Called after C1 admission, so codec/range/JSON budgets already hold. Decode
//! one identity/index view at a time; no archive-wide identity inventory is kept.
use super::{invalid, Error};
use crate::content_integrity::meshopt::{self, Mode};
use serde_json::Value;

fn fail() -> Error {
    invalid("hlodFeatureIds: invalid source identity attribute or property table")
}
fn uint(v: &Value) -> Result<usize, Error> {
    v.as_u64()
        .and_then(|n| usize::try_from(n).ok())
        .ok_or_else(fail)
}
fn slice(bytes: &[u8], offset: usize, length: usize) -> Result<&[u8], Error> {
    bytes
        .get(offset..offset.checked_add(length).ok_or_else(fail)?)
        .ok_or_else(fail)
}
fn view(doc: &Value, bin: &[u8], id: usize) -> Result<Vec<u8>, Error> {
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
fn scalars(doc: &Value, bin: &[u8], id: usize) -> Result<Vec<u32>, Error> {
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
    let vi = uint(&a["bufferView"])?;
    let bytes = view(doc, bin, vi)?;
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

pub(super) fn inspect(bytes: &[u8]) -> Result<bool, Error> {
    if bytes.get(..4) != Some(b"glTF") {
        return Ok(false);
    }
    let length =
        u32::from_le_bytes(bytes.get(12..16).ok_or_else(fail)?.try_into().unwrap()) as usize;
    let doc: Value = serde_json::from_slice(slice(bytes, 20, length)?)?;
    let metadata = &doc["extensions"]["EXT_structural_metadata"];
    let declared = doc["meshes"].as_array().into_iter().flatten().any(|m| {
        m["primitives"].as_array().into_iter().flatten().any(|p| {
            p["extensions"]["EXT_mesh_features"]["featureIds"]
                .as_array()
                .into_iter()
                .flatten()
                .any(|f| f["label"] == "source_primitive")
        })
    });
    let tables = metadata["propertyTables"].as_array();
    let has_table = tables.is_some_and(|ts| ts.iter().any(|t| t["class"] == "source_primitive"));
    if !declared && !has_table {
        return Ok(false);
    }
    let tables = tables.ok_or_else(fail)?;
    if !has_table {
        return Err(fail());
    }
    let bin = bytes.get(28 + length..).ok_or_else(fail)?;
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
            if !props.contains_key(key) || table["properties"].get(key).is_none() {
                return Err(fail());
            }
        }
        if class == "source_triangle" {
            for key in ["triangle_index", "vertex_0", "vertex_1", "vertex_2"] {
                if !props.contains_key(key) || table["properties"].get(key).is_none() {
                    return Err(fail());
                }
            }
        }
        for (key, schema) in props {
            if schema["required"] != true {
                continue;
            }
            let width = match schema["componentType"].as_str() {
                Some("UINT8") => 1,
                Some("UINT32") => 4,
                Some("UINT64") => 8,
                _ => return Err(fail()),
            };
            let data = view(&doc, bin, uint(&table["properties"][key]["values"])?)?;
            if data.len() < count.checked_mul(width).ok_or_else(fail)? {
                return Err(fail());
            }
        }
    }
    for mesh in doc["meshes"].as_array().into_iter().flatten() {
        for p in mesh["primitives"].as_array().into_iter().flatten() {
            let sets = p["extensions"]["EXT_mesh_features"]["featureIds"]
                .as_array()
                .ok_or_else(fail)?;
            let mut source = false;
            for set in sets {
                if !matches!(
                    set["label"].as_str(),
                    Some("source_primitive" | "source_triangle")
                ) {
                    continue;
                }
                source |= set["label"] == "source_primitive";
                let table = tables.get(uint(&set["propertyTable"])?).ok_or_else(fail)?;
                let count = uint(&table["count"])?;
                if table["class"] != set["label"] || uint(&set["featureCount"])? != count {
                    return Err(fail());
                }
                let key = format!("_FEATURE_ID_{}", uint(&set["attribute"])?);
                let ids = scalars(&doc, bin, uint(&p["attributes"][&key])?)?;
                if ids.len()
                    != uint(&doc["accessors"][uint(&p["attributes"]["POSITION"])?]["count"])?
                    || ids.iter().any(|&i| i as usize >= count)
                {
                    return Err(fail());
                }
                let indices = p
                    .get("indices")
                    .map(|a| scalars(&doc, bin, uint(a)?))
                    .transpose()?;
                let at = |i: usize| -> Result<u32, Error> {
                    let index = match &indices {
                        Some(v) => *v.get(i).ok_or_else(fail)? as usize,
                        None => i,
                    };
                    ids.get(index).copied().ok_or_else(fail)
                };
                let corners = indices.as_ref().map_or(ids.len(), Vec::len);
                if p["mode"].as_u64().unwrap_or(4) != 4 || !corners.is_multiple_of(3) {
                    return Err(fail());
                }
                for i in (0..corners).step_by(3) {
                    if at(i)? != at(i + 1)? || at(i)? != at(i + 2)? {
                        return Err(fail());
                    }
                }
            }
            if !source {
                return Err(fail());
            }
        }
    }
    Ok(true)
}
