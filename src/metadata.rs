//! Shared 3D Tiles metadata and glTF extension payloads.
//!
//! In-tree serde models for [EXT_mesh_features](https://github.com/CesiumGS/glTF/tree/3d-tiles-next/extensions/2.0/Vendor/EXT_mesh_features)
//! and [EXT_structural_metadata](https://github.com/CesiumGS/glTF/tree/3d-tiles-next/extensions/2.0/Vendor/EXT_structural_metadata).
//!
//! Buffer-view indices refer to the containing document. Use [`MetadataGlb`]
//! to append aligned property values, then [`StructuralMetadata::attach`] to
//! attach tables/attributes. Unknown schema fields round-trip in `additional`.
use crate::{glb, Error};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet};

pub use crate::glb::MetadataGlb;
pub const MESH_FEATURES: &str = "EXT_mesh_features";
pub const STRUCTURAL_METADATA: &str = "EXT_structural_metadata";
pub const TILE_BOUNDING_BOX: &str = "TILE_BOUNDING_BOX";
pub const TILE_GEOMETRIC_ERROR: &str = "TILE_GEOMETRIC_ERROR";

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MeshFeatures {
    pub feature_ids: Vec<FeatureId>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FeatureId {
    pub feature_count: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub attribute: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub property_table: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub null_feature_id: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    #[serde(default, flatten)]
    pub additional: BTreeMap<String, Value>,
}

impl MeshFeatures {
    pub fn attribute(count: usize, attribute: usize, table: Option<usize>) -> Self {
        Self {
            feature_ids: vec![FeatureId {
                feature_count: count,
                attribute: Some(attribute),
                property_table: table,
                ..Default::default()
            }],
        }
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StructuralMetadata {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub schema: Option<Schema>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub schema_uri: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub property_tables: Vec<PropertyTable>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub property_attributes: Vec<PropertyAttribute>,
    #[serde(default, flatten)]
    pub additional: BTreeMap<String, Value>,
}

impl StructuralMetadata {
    /// Attach to a JSON glTF, preserving unrelated extensions.
    pub fn attach(&self, document: &mut Value) -> Result<(), Error> {
        if document["extensions"].is_null() {
            document["extensions"] = json!({});
        }
        document["extensions"][STRUCTURAL_METADATA] = serde_json::to_value(self)?;
        glb::add_extension(document, STRUCTURAL_METADATA, false)
    }

    /// Attach through gltf-json's generic extension map.
    pub fn attach_gltf(&self, root: &mut gltf_json::Root) -> Result<(), Error> {
        root.extensions
            .get_or_insert_with(Default::default)
            .others
            .insert(STRUCTURAL_METADATA.into(), serde_json::to_value(self)?);
        if !root
            .extensions_used
            .iter()
            .any(|s| s == STRUCTURAL_METADATA)
        {
            root.extensions_used.push(STRUCTURAL_METADATA.into());
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Schema {
    pub id: String,
    pub classes: BTreeMap<String, Class>,
    #[serde(default, flatten)]
    pub additional: BTreeMap<String, Value>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Class {
    pub properties: BTreeMap<String, ClassProperty>,
    #[serde(default, flatten)]
    pub additional: BTreeMap<String, Value>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ClassProperty {
    #[serde(rename = "type")]
    pub property_type: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub component_type: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub semantic: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub array: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub count: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub no_data: Option<Value>,
    #[serde(default, flatten)]
    pub additional: BTreeMap<String, Value>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct PropertyTable {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    pub class: String,
    pub count: usize,
    pub properties: BTreeMap<String, PropertyTableProperty>,
    #[serde(default, flatten)]
    pub additional: BTreeMap<String, Value>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PropertyTableProperty {
    pub values: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub string_offsets: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub string_offset_type: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub array_offsets: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub array_offset_type: Option<String>,
    #[serde(default, flatten)]
    pub additional: BTreeMap<String, Value>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct PropertyAttribute {
    pub class: String,
    pub properties: BTreeMap<String, PropertyAttributeProperty>,
    #[serde(default, flatten)]
    pub additional: BTreeMap<String, Value>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct PropertyAttributeProperty {
    pub attribute: String,
    #[serde(default, flatten)]
    pub additional: BTreeMap<String, Value>,
}

/// Schema shared by mesh/vector/point implicit subtrees.
pub fn tile_schema() -> Schema {
    serde_json::from_value(json!({"id":"rustyTilesImplicit","classes":{"rustyTile":{"properties":{
        "boundingBox":{"type":"SCALAR","componentType":"FLOAT64","array":true,"count":12,"semantic":TILE_BOUNDING_BOX},
        "geometricError":{"type":"SCALAR","componentType":"FLOAT64","semantic":TILE_GEOMETRIC_ERROR},
        "extras":{"type":"STRING"}
    }}}})).expect("static tile schema")
}

fn view(glb: &mut MetadataGlb, bytes: &[u8]) -> usize {
    glb.view(if bytes.is_empty() { &[0] } else { bytes })
}

/// Encode scalar/string/boolean rows without losing 64-bit integers. Schema
/// hints accept `integer`, `real`, `string`, or `boolean`. Nullable values use
/// collision-free noData sentinels; ambiguous mixed types are rejected.
pub fn encode_property_table(
    glb: &mut MetadataGlb,
    items: &[&BTreeMap<String, Value>],
    schemas: &BTreeMap<String, String>,
    schema_id: &str,
    class: &str,
    name: &str,
) -> Result<(), Error> {
    let keys: BTreeSet<_> = items.iter().flat_map(|f| f.keys()).collect();
    let mut schema = BTreeMap::new();
    let mut columns = BTreeMap::new();
    for key in keys {
        let mut values: Vec<_> = items
            .iter()
            .map(|f| f.get(key).cloned().unwrap_or(Value::Null))
            .collect();
        let present: Vec<_> = values.iter().filter(|v| !v.is_null()).collect();
        let missing = present.len() != values.len();
        let expected = schemas.get(key).map(String::as_str);
        let real = expected == Some("real")
            || (expected.is_none()
                && present.iter().all(|v| v.is_number())
                && present.iter().any(|v| v.is_f64()));
        if real {
            for v in &present {
                // Check the original integer: converting first rounds 2^53+1
                // down to the boundary and would silently accept a lossy value.
                if v.as_i64()
                    .is_some_and(|n| !(-(1i64 << 53)..=(1i64 << 53)).contains(&n))
                    || v.as_u64().is_some_and(|n| n > (1u64 << 53))
                {
                    return Err(Error::Data(format!(
                        "property {key:?} cannot represent large integers as float64 without loss"
                    )));
                }
            }
            for value in &mut values {
                if !value.is_null() {
                    *value = json!(value
                        .as_f64()
                        .ok_or_else(|| Error::Data("incompatible numeric property".into()))?);
                }
            }
        }
        let mut no_data = None;
        if missing {
            let present: Vec<_> = values.iter().filter(|v| !v.is_null()).collect();
            let sentinel = if expected == Some("integer")
                || (expected.is_none()
                    && !present.is_empty()
                    && present.iter().all(|v| v.is_i64() || v.is_u64()))
            {
                let mut sentinel = -(1i64 << 53);
                while present.iter().any(|v| v.as_i64() == Some(sentinel)) {
                    sentinel += 1;
                }
                json!(sentinel)
            } else if real || (!present.is_empty() && present.iter().all(|v| v.is_number())) {
                let sentinel = -f64::MAX;
                if present.iter().any(|v| v.as_f64() == Some(sentinel)) {
                    return Err(Error::Data(
                        "reserved missing-value sentinel occurs in source".into(),
                    ));
                }
                json!(sentinel)
            } else if expected == Some("boolean") {
                return Err(Error::Data(format!(
                    "nullable boolean property {key:?} requires an explicit schema"
                )));
            } else if expected == Some("string") || present.iter().all(|v| v.is_string()) {
                let mut sentinel = "__RUSTY_TILES_MISSING__".to_string();
                while present.iter().any(|v| v.as_str() == Some(&sentinel)) {
                    sentinel.push('_');
                }
                json!(sentinel)
            } else {
                return Err(Error::Data(format!(
                    "nullable boolean/complex property {key:?} requires an explicit schema"
                )));
            };
            for v in &mut values {
                if v.is_null() {
                    *v = sentinel.clone();
                }
            }
            no_data = Some(sentinel);
        }
        let (mut definition, column) = if values.iter().all(Value::is_boolean) {
            let mut bytes = vec![0; values.len().div_ceil(8)];
            for (i, v) in values.iter().enumerate() {
                if v.as_bool().unwrap() {
                    bytes[i / 8] |= 1 << (i % 8);
                }
            }
            (
                json!({"type":"BOOLEAN"}),
                json!({"values":view(glb,&bytes)}),
            )
        } else if values.iter().all(|v| v.is_i64() || v.is_u64()) {
            let mut bytes = Vec::new();
            for v in &values {
                bytes.extend(
                    v.as_i64()
                        .ok_or_else(|| {
                            Error::Data(format!(
                                "integer property {key:?} is outside signed INT64 range"
                            ))
                        })?
                        .to_le_bytes(),
                );
            }
            (
                json!({"type":"SCALAR","componentType":"INT64"}),
                json!({"values":view(glb,&bytes)}),
            )
        } else if values.iter().all(Value::is_number) {
            let mut bytes = Vec::new();
            for v in &values {
                let value = v
                    .as_f64()
                    .filter(|v| v.is_finite())
                    .ok_or_else(|| Error::Data("nonfinite scalar property".into()))?;
                bytes.extend(value.to_le_bytes());
            }
            (
                json!({"type":"SCALAR","componentType":"FLOAT64"}),
                json!({"values":view(glb,&bytes)}),
            )
        } else if values.iter().all(Value::is_string) {
            let mut bytes = Vec::new();
            let mut offsets = 0u32.to_le_bytes().to_vec();
            for v in &values {
                bytes.extend(v.as_str().unwrap().as_bytes());
                let offset = u32::try_from(bytes.len())
                    .map_err(|_| Error::Data("metadata strings exceed UINT32 range".into()))?;
                offsets.extend(offset.to_le_bytes());
            }
            (
                json!({"type":"STRING"}),
                json!({"values":view(glb,&bytes),"stringOffsets":view(glb,&offsets),"stringOffsetType":"UINT32"}),
            )
        } else {
            return Err(Error::Data(format!(
                "unsupported/null/mixed property {key:?}; retain source and normalize explicitly"
            )));
        };
        if let Some(no_data) = no_data {
            definition["noData"] = no_data;
        }
        schema.insert(key.clone(), definition);
        columns.insert(key.clone(), column);
    }
    let extension = StructuralMetadata {
        schema: Some(Schema {
            id: schema_id.into(),
            classes: BTreeMap::from([(
                class.into(),
                Class {
                    properties: schema
                        .into_iter()
                        .map(|(k, v)| Ok((k, serde_json::from_value(v)?)))
                        .collect::<Result<_, Error>>()?,
                    ..Default::default()
                },
            )]),
            ..Default::default()
        }),
        property_tables: vec![PropertyTable {
            name: Some(name.into()),
            class: class.into(),
            count: items.len(),
            properties: columns
                .into_iter()
                .map(|(k, v)| Ok((k, serde_json::from_value(v)?)))
                .collect::<Result<_, Error>>()?,
            ..Default::default()
        }],
        ..Default::default()
    };
    extension.attach(&mut glb.document)?;
    Ok(())
}

/// Attach per-primitive source-node IDs. Primitive groups must contain exactly
/// one source node; the mesh tiler keeps that boundary through every LOD.
pub(crate) fn attach_node_features(
    glb: &mut MetadataGlb,
    features: &[(u32, String)],
) -> Result<(), Error> {
    let unique: BTreeMap<_, _> = features.iter().cloned().collect();
    if unique.len() > 16_777_217 {
        return Err(Error::Data("too many exact node feature IDs".into()));
    }
    let rows: Vec<BTreeMap<String, Value>> = unique
        .values()
        .map(|name| BTreeMap::from([("name".into(), json!(name))]))
        .collect();
    let ids: BTreeMap<_, _> = unique.keys().enumerate().map(|(i, id)| (*id, i)).collect();
    encode_property_table(
        glb,
        &rows.iter().collect::<Vec<_>>(),
        &BTreeMap::new(),
        "rusty_tiles_mesh",
        "node",
        "nodes",
    )?;
    // A source node index is UINT32, so Cesium exposes ordinary numbers for
    // style expressions (INT64 values are BigInt in JavaScript).
    let source_ids: Vec<_> = unique.keys().flat_map(|id| id.to_le_bytes()).collect();
    let values = glb.view(&source_ids);
    let mut metadata: StructuralMetadata =
        serde_json::from_value(glb.document["extensions"][STRUCTURAL_METADATA].clone())?;
    metadata
        .schema
        .as_mut()
        .unwrap()
        .classes
        .get_mut("node")
        .unwrap()
        .properties
        .insert(
            "node_index".into(),
            ClassProperty {
                property_type: "SCALAR".into(),
                component_type: Some("UINT32".into()),
                ..Default::default()
            },
        );
    metadata.property_tables[0].properties.insert(
        "node_index".into(),
        PropertyTableProperty {
            values,
            ..Default::default()
        },
    );
    metadata.attach(&mut glb.document)?;
    glb::add_extension(&mut glb.document, MESH_FEATURES, false)?;
    for (i, (node, _)) in features.iter().enumerate() {
        let pos = glb.document["meshes"][0]["primitives"][i]["attributes"]["POSITION"]
            .as_u64()
            .ok_or_else(|| Error::Data("missing node feature positions".into()))?;
        let count = glb.document["accessors"][pos as usize]["count"]
            .as_u64()
            .unwrap() as usize;
        let id = ids[node];
        let small = unique.len() <= 65_536;
        let value = if small {
            (id as u32).to_le_bytes()
        } else {
            (id as f32).to_le_bytes()
        };
        let bytes: Vec<_> = std::iter::repeat_n(value, count).flatten().collect();
        let view = glb.view(&bytes);
        glb.document["bufferViews"][view]["byteStride"] = 4.into();
        glb.document["bufferViews"][view]["target"] = 34962.into();
        let accessor = glb.accessor(json!({"bufferView":view,"componentType":if small {5123} else {5126},"count":count,"type":"SCALAR"}));
        glb.document["meshes"][0]["primitives"][i]["attributes"]["_FEATURE_ID_0"] = accessor.into();
        glb.document["meshes"][0]["primitives"][i]["extensions"][MESH_FEATURES] =
            serde_json::to_value(MeshFeatures::attribute(unique.len(), 0, Some(0)))?;
    }
    Ok(())
}
