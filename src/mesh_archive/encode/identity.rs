//! Fixed generated source-identity schema; no imported metadata or mutable public IR.
use super::{EncodeError, Geometry, Leaf};
use crate::mesh_archive::source::SourceIdentity;
use crate::{JobError, JobErrorKind};
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet};

type PrimitiveKey = (u32, u32, u32);

pub(super) struct Plan<'a> {
    geometry: &'a Geometry,
    primitives: BTreeMap<PrimitiveKey, u32>,
    triangles: BTreeMap<SourceIdentity, u32>,
}
impl<'a> Plan<'a> {
    pub fn new<E>(
        geometry: &'a Geometry,
        leaf: &Leaf,
        check: &mut impl FnMut() -> Result<(), E>,
    ) -> Result<Self, EncodeError<E>> {
        let mut primitives = BTreeSet::new();
        let mut triangles = BTreeSet::new();
        for &index in &leaf.triangles {
            check().map_err(EncodeError::Checkpoint)?;
            let source = geometry
                .triangles
                .get(index)
                .ok_or(EncodeError::Invalid("source identity triangle index"))?
                .source;
            if source.node_index as usize >= geometry.node_names.len() {
                return Err(EncodeError::Invalid("source identity node index"));
            }
            primitives.insert(source.primitive());
            triangles.insert(source);
        }
        if triangles.len() != leaf.triangles.len() || triangles.is_empty() {
            return Err(EncodeError::Invalid(
                "source triangle identity multiplicity",
            ));
        }
        Ok(Self {
            geometry,
            primitives: primitives
                .into_iter()
                .enumerate()
                .map(|(i, key)| (key, i as u32))
                .collect(),
            triangles: triangles
                .into_iter()
                .enumerate()
                .map(|(i, key)| (key, i as u32))
                .collect(),
        })
    }
    pub fn append_attributes<E>(
        &self,
        triangles: &[usize],
        buffer: &mut Vec<u8>,
        views: &mut Vec<Value>,
        accessors: &mut Vec<Value>,
        attributes: &mut Value,
        check: &mut impl FnMut() -> Result<(), E>,
    ) -> Result<Value, EncodeError<E>> {
        let mut features = Vec::new();
        for set in 0..2 {
            let mut bytes = Vec::with_capacity(triangles.len() * 12);
            let mut used = BTreeSet::new();
            for &index in triangles {
                check().map_err(EncodeError::Checkpoint)?;
                let source = self.geometry.triangles[index].source;
                let id = if set == 0 {
                    self.primitives[&source.primitive()]
                } else {
                    self.triangles[&source]
                };
                used.insert(id);
                for _ in 0..3 {
                    bytes.extend_from_slice(&(id as f32).to_le_bytes());
                }
            }
            let view = append_view(buffer, views, &bytes, true);
            let accessor = accessors.len();
            accessors.push(json!({"bufferView":view,"componentType":5126,"count":triangles.len()*3,"type":"SCALAR","normalized":false}));
            attributes[format!("_FEATURE_ID_{set}")] = json!(accessor);
            features.push(
                json!({"featureCount":used.len(),"attribute":set,"propertyTable":set,
                "label":if set==0 {"source_primitive"} else {"source_triangle"}}),
            );
        }
        Ok(json!({"featureIds":features}))
    }
    pub fn append_metadata<E>(
        &self,
        buffer: &mut Vec<u8>,
        views: &mut Vec<Value>,
        check: &mut impl FnMut() -> Result<(), E>,
    ) -> Result<Value, EncodeError<E>> {
        let mut primitive_properties = serde_json::Map::new();
        for column in 0..3 {
            let mut bytes = Vec::with_capacity(self.primitives.len() * 4);
            for &(node, mesh, primitive) in self.primitives.keys() {
                check().map_err(EncodeError::Checkpoint)?;
                bytes.extend_from_slice(&[node, mesh, primitive][column].to_le_bytes());
            }
            let view = append_view(buffer, views, &bytes, false);
            primitive_properties.insert(
                ["node_index", "mesh_index", "primitive_index"][column].into(),
                json!({"values":view}),
            );
        }
        let mut names = Vec::new();
        let mut presence = Vec::with_capacity(self.primitives.len());
        let mut offsets = Vec::with_capacity((self.primitives.len() + 1) * 4);
        offsets.extend_from_slice(&0u32.to_le_bytes());
        for &(node, _, _) in self.primitives.keys() {
            check().map_err(EncodeError::Checkpoint)?;
            let name = &self.geometry.node_names[node as usize];
            presence.push(u8::from(name.is_some()));
            if let Some(name) = name {
                names.extend_from_slice(name.as_bytes());
            }
            let offset = u32::try_from(names.len())
                .map_err(|_| EncodeError::Invalid("source name offsets"))?;
            offsets.extend_from_slice(&offset.to_le_bytes());
        }
        // glTF bufferView.byteLength must be positive even when every string is empty.
        // Offset endpoints stay zero, so this unused zero byte is not name text.
        if names.is_empty() {
            names.push(0);
        }
        let values = append_view(buffer, views, &names, false);
        let string_offsets = append_view(buffer, views, &offsets, false);
        let present = append_view(buffer, views, &presence, false);
        primitive_properties.insert(
            "node_name".into(),
            json!({"values":values,"stringOffsets":string_offsets,"stringOffsetType":"UINT32"}),
        );
        primitive_properties.insert("node_name_present".into(), json!({"values":present}));
        let mut triangle_properties = serde_json::Map::new();
        for column in 0..4 {
            let mut bytes = Vec::with_capacity(self.triangles.len() * 4);
            for source in self.triangles.keys() {
                check().map_err(EncodeError::Checkpoint)?;
                let value = [
                    source.node_index,
                    source.mesh_index,
                    source.primitive_index,
                    source.triangle_index,
                ][column];
                bytes.extend_from_slice(&value.to_le_bytes());
            }
            let view = append_view(buffer, views, &bytes, false);
            triangle_properties.insert(
                [
                    "node_index",
                    "mesh_index",
                    "primitive_index",
                    "triangle_index",
                ][column]
                    .into(),
                json!({"values":view}),
            );
        }
        let scalar = || json!({"type":"SCALAR","componentType":"UINT32","required":true});
        Ok(json!({"schema":{"id":"rusty_tiles_source_v1","classes":{
            "source_primitive":{"properties":{"node_index":scalar(),"mesh_index":scalar(),"primitive_index":scalar(),
                "node_name":{"type":"STRING","required":true},"node_name_present":{"type":"SCALAR","componentType":"UINT8","required":true}}},
            "source_triangle":{"properties":{"node_index":scalar(),"mesh_index":scalar(),"primitive_index":scalar(),"triangle_index":scalar()}}
        }},"propertyTables":[{"class":"source_primitive","count":self.primitives.len(),"properties":primitive_properties},
            {"class":"source_triangle","count":self.triangles.len(),"properties":triangle_properties}]}))
    }
}
fn append_view(
    buffer: &mut Vec<u8>,
    views: &mut Vec<Value>,
    bytes: &[u8],
    attribute: bool,
) -> usize {
    buffer.resize(buffer.len().next_multiple_of(8), 0);
    let offset = buffer.len();
    buffer.extend_from_slice(bytes);
    let id = views.len();
    let mut view = json!({"buffer":0,"byteOffset":offset,"byteLength":bytes.len()});
    if attribute {
        view["target"] = json!(34962);
    }
    views.push(view);
    id
}

pub(super) fn name_bytes(
    geometry: &Geometry,
    leaves: &[Leaf],
    mut check: impl FnMut() -> Result<(), JobError>,
) -> Result<usize, JobError> {
    let mut total = 0usize;
    for leaf in leaves {
        let mut keys = BTreeSet::new();
        for &index in &leaf.triangles {
            check()?;
            let source = geometry
                .triangles
                .get(index)
                .ok_or_else(|| {
                    JobError::new(JobErrorKind::InvalidState, "metadata budget triangle index")
                })?
                .source;
            keys.insert(source.primitive());
        }
        for (node, _, _) in keys {
            check()?;
            let name = geometry.node_names.get(node as usize).ok_or_else(|| {
                JobError::new(JobErrorKind::InvalidState, "metadata budget node index")
            })?;
            total = total
                .checked_add(name.as_ref().map_or(0, String::len))
                .ok_or_else(|| {
                    JobError::new(JobErrorKind::Unsupported, "emitted source names overflow")
                })?;
        }
    }
    Ok(total)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mesh_archive::{partition::Bounds, source::Triangle};
    fn geometry(count: usize, name: Option<String>) -> Geometry {
        Geometry {
            node_names: vec![name],
            triangles: (0..count)
                .map(|i| Triangle {
                    source: SourceIdentity {
                        triangle_index: i as u32,
                        ..Default::default()
                    },
                    positions: [[0.; 3]; 3],
                    vertex_indices: [0, 1, 2],
                    normals: None,
                    tangents: None,
                    texcoords: [None; 2],
                    colors: None,
                    material: None,
                })
                .collect(),
            materials: vec![],
            images: vec![],
            textures: vec![],
            samplers: vec![],
        }
    }
    fn leaf(index: usize) -> Leaf {
        Leaf {
            triangles: vec![index],
            bounds: Bounds {
                min: [0.; 3],
                max: [0.; 3],
            },
        }
    }
    #[test]
    fn repeated_names_have_a_strict_archive_wide_byte_cap() {
        let geometry = geometry(2049, Some("🦉".repeat(1024)));
        let leaves: Vec<_> = (0..2049).map(leaf).collect();
        super::super::validate_identity_budget(&geometry, &leaves[..2048], None, || Ok(()))
            .unwrap();
        let error = super::super::validate_identity_budget(&geometry, &leaves, None, || Ok(()))
            .unwrap_err();
        assert_eq!(error.kind(), JobErrorKind::Unsupported);
    }
    #[test]
    fn root_region_name_counts_once_in_the_shared_archive_budget() {
        use crate::mesh_archive::approximation::{
            Certificate, ProxyGeometry, ProxyRegion, ProxyTriangle,
        };
        let geometry = geometry(2048, Some("🦉".repeat(1024)));
        let leaves: Vec<_> = (0..2048).map(leaf).collect();
        let proxy = ProxyGeometry {
            regions: vec![ProxyRegion {
                members: geometry
                    .triangles
                    .iter()
                    .map(|triangle| triangle.source)
                    .collect(),
                material: None,
            }],
            triangles: vec![ProxyTriangle {
                positions: [[0.; 3]; 3],
                region: 0,
            }],
            certificate: Certificate {
                error_metres: 0.,
                patch_face_tests: 0,
                accepted_patches: 0,
                max_depth: 0,
            },
        };
        super::super::validate_identity_budget(&geometry, &leaves[..2047], Some(&proxy), || Ok(()))
            .unwrap();
        let error =
            super::super::validate_identity_budget(&geometry, &leaves, Some(&proxy), || Ok(()))
                .unwrap_err();
        assert_eq!(error.kind(), JobErrorKind::Unsupported);
    }
    #[test]
    fn empty_names_emit_positive_length_views_without_invented_text() {
        let geometry = geometry(1, None);
        let leaf = leaf(0);
        let plan = Plan::new(&geometry, &leaf, &mut || {
            Ok::<_, std::convert::Infallible>(())
        })
        .unwrap();
        let mut buffer = Vec::new();
        let mut views = Vec::new();
        let metadata = plan
            .append_metadata(&mut buffer, &mut views, &mut || {
                Ok::<_, std::convert::Infallible>(())
            })
            .unwrap();
        assert!(views
            .iter()
            .all(|v| v["byteLength"].as_u64().unwrap() > 0
                && v["byteOffset"].as_u64().unwrap() % 8 == 0));
        let names = &metadata["propertyTables"][0]["properties"]["node_name"];
        let offsets = &views[names["stringOffsets"].as_u64().unwrap() as usize];
        let start = offsets["byteOffset"].as_u64().unwrap() as usize;
        assert_eq!(&buffer[start..start + 8], &[0u8; 8]);
    }
}
