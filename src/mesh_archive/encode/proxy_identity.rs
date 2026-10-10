//! Fixed proxy-region provenance. Synthetic faces never receive authored face IDs.
use super::{Geometry, ProxyGeometry};
use crate::{JobError, JobErrorKind};
use serde_json::{json, Value};
use std::collections::BTreeSet;

fn invalid(message: &'static str) -> JobError {
    JobError::new(JobErrorKind::InvalidState, message)
}

fn region_name<'a>(
    geometry: &'a Geometry,
    proxy: &ProxyGeometry,
    region: usize,
) -> Result<&'a Option<String>, JobError> {
    let first = proxy
        .regions
        .get(region)
        .and_then(|region| region.members.first())
        .ok_or_else(|| invalid("empty prepared proxy region"))?;
    geometry
        .node_names
        .get(first.node_index as usize)
        .ok_or_else(|| invalid("proxy region source node index"))
}

pub(super) fn name_bytes(
    geometry: &Geometry,
    proxy: &ProxyGeometry,
    check: &mut impl FnMut() -> Result<(), JobError>,
) -> Result<usize, JobError> {
    let mut total = 0usize;
    for region in 0..proxy.regions.len() {
        check()?;
        total = total
            .checked_add(
                region_name(geometry, proxy, region)?
                    .as_ref()
                    .map_or(0, String::len),
            )
            .ok_or_else(|| {
                JobError::new(JobErrorKind::Unsupported, "emitted source names overflow")
            })?;
    }
    Ok(total)
}

pub(super) struct Plan<'a> {
    geometry: &'a Geometry,
    proxy: &'a ProxyGeometry,
}

impl<'a> Plan<'a> {
    pub fn new(
        geometry: &'a Geometry,
        proxy: &'a ProxyGeometry,
        check: &mut impl FnMut() -> Result<(), JobError>,
    ) -> Result<Self, JobError> {
        if proxy.regions.is_empty()
            || proxy.triangles.is_empty()
            || proxy.regions.len() > super::super::source::MAX_TRIANGLES
            || proxy.triangles.len() > super::super::source::MAX_TRIANGLES
        {
            return Err(invalid("prepared proxy count"));
        }
        let mut member_count = 0usize;
        let mut previous_key = None;
        for (id, region) in proxy.regions.iter().enumerate() {
            check()?;
            region_name(geometry, proxy, id)?;
            let key = region.members[0].primitive();
            if previous_key.is_some_and(|previous| previous >= key) {
                return Err(invalid("prepared proxy region ordering"));
            }
            previous_key = Some(key);
            let mut previous_member = None;
            for member in &region.members {
                check()?;
                if member.primitive() != key
                    || previous_member.is_some_and(|previous| previous >= *member)
                {
                    return Err(invalid("prepared proxy membership ordering"));
                }
                previous_member = Some(*member);
            }
            member_count = member_count
                .checked_add(region.members.len())
                .ok_or_else(|| invalid("prepared proxy membership count"))?;
        }
        if member_count > super::super::source::MAX_TRIANGLES {
            return Err(invalid("prepared proxy membership ceiling"));
        }
        let mut used = BTreeSet::new();
        for triangle in &proxy.triangles {
            check()?;
            if triangle.region >= proxy.regions.len() {
                return Err(invalid("prepared proxy face region"));
            }
            used.insert(triangle.region);
        }
        if used.len() != proxy.regions.len() {
            return Err(invalid("prepared proxy region without geometry"));
        }
        Ok(Self { geometry, proxy })
    }

    pub fn append_attributes(
        &self,
        triangles: &[usize],
        buffer: &mut Vec<u8>,
        views: &mut Vec<Value>,
        accessors: &mut Vec<Value>,
        attributes: &mut Value,
        check: &mut impl FnMut() -> Result<(), JobError>,
    ) -> Result<Value, JobError> {
        let mut bytes = Vec::with_capacity(triangles.len() * 12);
        let mut used = BTreeSet::new();
        for &index in triangles {
            check()?;
            let region = self.proxy.triangles[index].region;
            used.insert(region);
            for _ in 0..3 {
                bytes.extend_from_slice(&(region as f32).to_le_bytes());
            }
        }
        let view = append_view(buffer, views, &bytes, true);
        let accessor = accessors.len();
        accessors.push(json!({"bufferView":view,"componentType":5126,"count":triangles.len()*3,"type":"SCALAR","normalized":false}));
        attributes["_FEATURE_ID_0"] = json!(accessor);
        Ok(
            json!({"featureIds":[{"featureCount":used.len(),"attribute":0,"propertyTable":0,"label":"proxy_region"}]}),
        )
    }

    pub fn append_metadata(
        &self,
        buffer: &mut Vec<u8>,
        views: &mut Vec<Value>,
        check: &mut impl FnMut() -> Result<(), JobError>,
    ) -> Result<Value, JobError> {
        let mut offsets = Vec::with_capacity((self.proxy.regions.len() + 1) * 4);
        let mut member_count = 0u32;
        offsets.extend_from_slice(&member_count.to_le_bytes());
        for region in &self.proxy.regions {
            check()?;
            member_count += region.members.len() as u32;
            offsets.extend_from_slice(&member_count.to_le_bytes());
        }
        let array_offsets = append_view(buffer, views, &offsets, false);
        let columns = [
            "source_node_indices",
            "source_mesh_indices",
            "source_primitive_indices",
            "source_triangle_indices",
        ];
        let mut properties = serde_json::Map::new();
        let mut class_properties = serde_json::Map::new();
        for (column, name) in columns.into_iter().enumerate() {
            let mut bytes = Vec::with_capacity(member_count as usize * 4);
            for region in &self.proxy.regions {
                for member in &region.members {
                    check()?;
                    let value = [
                        member.node_index,
                        member.mesh_index,
                        member.primitive_index,
                        member.triangle_index,
                    ][column];
                    bytes.extend_from_slice(&value.to_le_bytes());
                }
            }
            let values = append_view(buffer, views, &bytes, false);
            properties.insert(
                name.into(),
                json!({"values":values,"arrayOffsets":array_offsets,"arrayOffsetType":"UINT32"}),
            );
            class_properties.insert(
                name.into(),
                json!({"type":"SCALAR","componentType":"UINT32","array":true,"required":true}),
            );
        }
        let mut names = Vec::new();
        let mut presence = Vec::with_capacity(self.proxy.regions.len());
        let mut string_offsets = Vec::with_capacity((self.proxy.regions.len() + 1) * 4);
        string_offsets.extend_from_slice(&0u32.to_le_bytes());
        for id in 0..self.proxy.regions.len() {
            check()?;
            let name = region_name(self.geometry, self.proxy, id)?;
            presence.push(u8::from(name.is_some()));
            if let Some(name) = name {
                names.extend_from_slice(name.as_bytes());
            }
            let offset = u32::try_from(names.len()).map_err(|_| invalid("proxy name offsets"))?;
            string_offsets.extend_from_slice(&offset.to_le_bytes());
        }
        // Positive buffer-view size without inventing text for absent/empty labels.
        if names.is_empty() {
            names.push(0);
        }
        let values = append_view(buffer, views, &names, false);
        let string_offsets = append_view(buffer, views, &string_offsets, false);
        let presence = append_view(buffer, views, &presence, false);
        properties.insert(
            "source_node_name".into(),
            json!({"values":values,"stringOffsets":string_offsets,"stringOffsetType":"UINT32"}),
        );
        properties.insert(
            "source_node_name_present".into(),
            json!({"values":presence}),
        );
        class_properties.insert(
            "source_node_name".into(),
            json!({"type":"STRING","required":true}),
        );
        class_properties.insert(
            "source_node_name_present".into(),
            json!({"type":"SCALAR","componentType":"UINT8","required":true}),
        );
        Ok(
            json!({"schema":{"id":"rusty_tiles_proxy_v1","classes":{"proxy_region":{"properties":class_properties}}},
            "propertyTables":[{"class":"proxy_region","count":self.proxy.regions.len(),"properties":properties}]}),
        )
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
