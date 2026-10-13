//! H8 numeric-provenance leaf GLB writer. Atlas policy and codecs are resolved
//! before this boundary; meshopt is lossless wire compression, not approximation.
use super::super::{
    attributes::{FullTriangle, COLOR, NORMAL, TANGENT},
    config::CONSUMER_TABLE_ROWS,
    records::Bounds,
};
use super::{
    geometry::{LatticePlan, PositionStorage},
    prepared::PreparedAtlas,
};
use crate::{
    glb::{self, MeshoptLayout, MeshoptStream},
    JobError, JobErrorKind,
};
use serde_json::{json, Map, Value};
use std::collections::{BTreeMap, BTreeSet};

const TILE_LIMIT: u64 = 96 << 20;
type PrimitiveKey = (u32, u32, u32);
#[derive(Debug)]
pub(super) struct EncodedLeaf {
    pub glb: Vec<u8>,
    pub bounds: Bounds,
    pub geometry_error: f64,
}
/// Only a measured/necessary member-size overrun may request density change.
#[derive(Debug)]
pub(super) enum WriteError {
    Budget(JobError),
    Failure(JobError),
}
impl WriteError {
    pub(super) fn kind(&self) -> JobErrorKind {
        match self {
            Self::Budget(error) | Self::Failure(error) => error.kind(),
        }
    }
}
impl From<JobError> for WriteError {
    fn from(error: JobError) -> Self {
        Self::Failure(error)
    }
}
impl From<WriteError> for JobError {
    fn from(error: WriteError) -> Self {
        match error {
            WriteError::Budget(error) | WriteError::Failure(error) => error,
        }
    }
}
fn invalid(message: &str) -> JobError {
    JobError::new(JobErrorKind::InvalidState, message)
}
fn overflow() -> JobError {
    JobError::new(
        JobErrorKind::ResourceLimit,
        "leaf wire size/allocation exhausted",
    )
}
fn budget() -> JobError {
    JobError::new(
        JobErrorKind::Unsupported,
        "complete leaf GLB exceeds tile byte budget",
    )
}
fn key(row: &FullTriangle) -> PrimitiveKey {
    (
        row.triangle.source.node,
        row.triangle.source.mesh,
        row.triangle.source.primitive,
    )
}
fn allocate(length: usize) -> Result<Vec<u8>, JobError> {
    let mut bytes = Vec::new();
    bytes.try_reserve_exact(length).map_err(|_| overflow())?;
    Ok(bytes)
}
struct Wire {
    binary: Vec<u8>,
    views: Vec<Value>,
    streams: Vec<Option<MeshoptStream>>,
    accessors: Vec<Value>,
}
impl Wire {
    fn new() -> Self {
        Self {
            binary: Vec::new(),
            views: Vec::new(),
            streams: Vec::new(),
            accessors: Vec::new(),
        }
    }
    fn view(&mut self, bytes: &[u8], stream: Option<MeshoptStream>) -> Result<usize, JobError> {
        if bytes.is_empty() {
            return Err(invalid("empty prepared leaf buffer view"));
        }
        let offset = self.binary.len().checked_add(7).ok_or_else(overflow)? & !7;
        let end = offset.checked_add(bytes.len()).ok_or_else(overflow)?;
        self.binary
            .try_reserve(end - self.binary.len())
            .map_err(|_| overflow())?;
        self.binary.resize(offset, 0);
        self.binary.extend_from_slice(bytes);
        let mut view = json!({"buffer":0,"byteOffset":offset,"byteLength":bytes.len()});
        if let Some(MeshoptStream::Attributes { stride, .. }) = stream {
            view["target"] = json!(34962);
            view["byteStride"] = json!(stride);
        }
        let index = self.views.len();
        self.views.push(view);
        self.streams.push(stream);
        Ok(index)
    }
    fn attribute(
        &mut self,
        bytes: &[u8],
        count: usize,
        stride: usize,
        component: u32,
        shape: &str,
    ) -> Result<usize, JobError> {
        if count.checked_mul(stride) != Some(bytes.len()) {
            return Err(invalid("prepared leaf attribute count/stride mismatch"));
        }
        let view = self.view(bytes, Some(MeshoptStream::Attributes { count, stride }))?;
        let index = self.accessors.len();
        self.accessors.push(json!({"bufferView":view,"componentType":component,"count":count,"type":shape,"normalized":false}));
        Ok(index)
    }
    fn floats(&mut self, rows: &[Vec<f32>], shape: &str) -> Result<usize, JobError> {
        let width = rows
            .first()
            .ok_or_else(|| invalid("empty float attribute"))?
            .len();
        let length = rows
            .len()
            .checked_mul(width)
            .and_then(|x| x.checked_mul(4))
            .ok_or_else(overflow)?;
        let mut bytes = allocate(length)?;
        for row in rows {
            if row.len() != width || row.iter().any(|v| !v.is_finite()) {
                return Err(invalid("nonfinite/inconsistent prepared float attribute"));
            }
            for value in row {
                bytes.extend_from_slice(&value.to_le_bytes());
            }
        }
        self.attribute(&bytes, rows.len(), width * 4, 5126, shape)
    }
}
fn index(value: &Value, count: usize, message: &str) -> Result<usize, JobError> {
    let n = value
        .as_u64()
        .and_then(|v| usize::try_from(v).ok())
        .ok_or_else(|| invalid(message))?;
    if n >= count {
        return Err(invalid(message));
    }
    Ok(n)
}
fn validate_closure(atlas: &PreparedAtlas) -> Result<bool, JobError> {
    let mut basis = false;
    for page in &atlas.pages {
        let magic: &[u8] = match page.mime {
            "image/png" => b"\x89PNG\r\n\x1a\n",
            "image/ktx2" => {
                basis = true;
                b"\xabKTX 20\xbb\r\n\x1a\n"
            }
            _ => return Err(invalid("prepared page MIME is not PNG or KTX2")),
        };
        if !page.bytes.starts_with(magic) {
            return Err(invalid("prepared page MIME/signature mismatch"));
        }
    }
    for texture in &atlas.textures {
        let source = if let Some(source) = texture
            .get("extensions")
            .and_then(|v| v.get("KHR_texture_basisu"))
            .and_then(|v| v.get("source"))
        {
            let n = index(
                source,
                atlas.pages.len(),
                "prepared basis texture image missing",
            )?;
            if atlas.pages[n].mime != "image/ktx2" {
                return Err(invalid("basis texture refers to non-KTX2 page"));
            }
            n
        } else {
            let n = index(
                &texture["source"],
                atlas.pages.len(),
                "prepared texture image missing",
            )?;
            if atlas.pages[n].mime != "image/png" {
                return Err(invalid("KTX2 texture lacks KHR_texture_basisu"));
            }
            n
        };
        let _ = source;
        if let Some(s) = texture.get("sampler") {
            index(s, atlas.samplers.len(), "prepared texture sampler missing")?;
        }
        if texture
            .get("extensions")
            .and_then(|v| v.as_object())
            .is_some_and(|o| o.keys().any(|k| k != "KHR_texture_basisu"))
        {
            return Err(invalid("unresolved prepared texture extension"));
        }
    }
    for primitive in &atlas.primitives {
        let count = primitive
            .triangles
            .len()
            .checked_mul(3)
            .ok_or_else(overflow)?;
        if count == 0
            || primitive.corner_depths.len() != primitive.triangles.len()
            || primitive.uvs.len() > 5
            || primitive
                .uvs
                .iter()
                .any(|uv| uv.len() != count || uv.iter().flatten().any(|v| !v.is_finite()))
        {
            return Err(invalid("prepared primitive corner/UV closure mismatch"));
        }
        if let Some(material) = &primitive.material {
            if !material.is_object() {
                return Err(invalid("prepared material is not object"));
            }
            for binding in [
                material.get("normalTexture"),
                material.get("occlusionTexture"),
                material.get("emissiveTexture"),
                material
                    .get("pbrMetallicRoughness")
                    .and_then(|p| p.get("baseColorTexture")),
                material
                    .get("pbrMetallicRoughness")
                    .and_then(|p| p.get("metallicRoughnessTexture")),
            ]
            .into_iter()
            .flatten()
            {
                index(
                    &binding["index"],
                    atlas.textures.len(),
                    "prepared material texture missing",
                )?;
                let uv = binding.get("texCoord").cloned().unwrap_or(json!(0));
                index(
                    &uv,
                    primitive.uvs.len(),
                    "prepared material TEXCOORD missing",
                )?;
            }
        }
    }
    Ok(basis)
}

pub(super) fn encode(
    atlas: &PreparedAtlas,
    lattice: &LatticePlan,
    depth: u8,
    max_bytes: u64,
) -> Result<EncodedLeaf, WriteError> {
    let limit = max_bytes.min(TILE_LIMIT);
    if limit == 0 {
        return Err(JobError::new(
            JobErrorKind::InvalidRequest,
            "leaf byte budget must be positive",
        )
        .into());
    }
    if atlas.primitives.is_empty() {
        return Err(invalid("prepared leaf has no primitives").into());
    }
    let basis = validate_closure(atlas)?;
    let total = atlas.primitives.iter().try_fold(0usize, |sum, p| {
        sum.checked_add(p.triangles.len()).ok_or_else(overflow)
    })?;
    if total > 1 << 24 {
        return Err(invalid("leaf feature ID exceeds exact float integer domain").into());
    }
    // Both property tables have at most `total` rows (one per triangle, one per present
    // primitive). The tree's leaf budget already bounds `total`; a wider table is a broken
    // plan, never a silent consumer-incompatible leaf.
    if total as u64 > CONSUMER_TABLE_ROWS {
        return Err(invalid("leaf property table exceeds the consumer row limit").into());
    }
    // Every triangle's mandatory numeric columns are uncompressed: this lower
    // bound rejects impossible tiles BEFORE allocating geometry or compression.
    let page_bytes = atlas.pages.iter().try_fold(0u64, |sum, p| {
        sum.checked_add(p.bytes.len() as u64).ok_or_else(overflow)
    })?;
    if (total as u64)
        .checked_mul(58)
        .and_then(|n| n.checked_add(page_bytes))
        .ok_or_else(overflow)?
        > limit
    {
        return Err(WriteError::Budget(budget()));
    }
    let mut primitive_rows: BTreeMap<PrimitiveKey, &FullTriangle> = BTreeMap::new();
    let mut identities = BTreeSet::new();
    for p in &atlas.primitives {
        for row in &p.triangles {
            let source = row.triangle.source;
            if !identities.insert((source.node, source.mesh, source.primitive, source.triangle)) {
                return Err(invalid("duplicate source triangle in prepared leaf").into());
            }
            if let Some(previous) = primitive_rows.insert(key(row), row) {
                if previous.expanded_ordinal != row.expanded_ordinal
                    || previous.material != row.material
                {
                    return Err(
                        invalid("source primitive expanded identity/material mismatch").into(),
                    );
                }
            }
        }
    }
    let primitive_ids: BTreeMap<_, _> = primitive_rows
        .keys()
        .enumerate()
        .map(|(i, k)| (*k, i))
        .collect();
    let mut wire = Wire::new();
    let mut nodes = Vec::new();
    let mut meshes = Vec::new();
    let mut materials = Vec::new();
    let mut bounds = Bounds::EMPTY;
    let mut geometry_error = 0.0_f64;
    let mut rank = 0usize;
    let mut quantized = false;
    for primitive in &atlas.primitives {
        let flags = primitive.triangles[0].flags;
        if flags & !31 != 0 || primitive.triangles.iter().any(|p| p.flags != flags) {
            return Err(invalid("prepared primitive mixes attribute presence").into());
        }
        let points: Vec<_> = primitive
            .triangles
            .iter()
            .flat_map(|t| t.triangle.positions)
            .collect();
        let depths: Vec<_> = primitive.corner_depths.iter().flatten().copied().collect();
        let position = lattice.encode(&points, &depths, u32::from(depth))?;
        bounds.union(&position.bounds);
        geometry_error = geometry_error.max(position.correspondence_error);
        let count = points.len();
        let mut node = json!({"mesh":meshes.len()});
        let pos = match &position.storage {
            PositionStorage::Quantized {
                codes,
                translation,
                scale,
            } => {
                quantized = true;
                let mut bytes = allocate(count.checked_mul(8).ok_or_else(overflow)?)?;
                let mut min = [u16::MAX; 3];
                let mut max = [0u16; 3];
                for code in codes {
                    for (a, value) in code.iter().enumerate() {
                        bytes.extend_from_slice(&value.to_le_bytes());
                        min[a] = min[a].min(*value);
                        max[a] = max[a].max(*value);
                    }
                    bytes.extend_from_slice(&[0, 0]);
                }
                let id = wire.attribute(&bytes, count, 8, 5123, "VEC3")?;
                wire.accessors[id]["min"] = json!(min);
                wire.accessors[id]["max"] = json!(max);
                // Promote BEFORE Value/JSON construction; f32 shortest decimals
                // parsed as f64 can otherwise define a different real transform.
                node["translation"] = json!(translation.map(f64::from));
                let scale_values = [f64::from(*scale); 3];
                node["scale"] = json!(scale_values);
                id
            }
            PositionStorage::Float { positions } => {
                let rows: Vec<_> = positions.iter().map(|p| p.to_vec()).collect();
                let id = wire.floats(&rows, "VEC3")?;
                wire.accessors[id]["min"] = json!(position.bounds.min.map(f64::from));
                wire.accessors[id]["max"] = json!(position.bounds.max.map(f64::from));
                id
            }
        };
        let mut attributes = json!({"POSITION":pos});
        for (flag, name, shape) in [
            (NORMAL, "NORMAL", "VEC3"),
            (TANGENT, "TANGENT", "VEC4"),
            (COLOR, "COLOR_0", "VEC4"),
        ] {
            if flags & flag != 0 {
                let rows: Vec<Vec<f32>> = primitive
                    .triangles
                    .iter()
                    .flat_map(|t| t.corners.iter())
                    .map(|c| match flag {
                        NORMAL => c.normal.to_vec(),
                        TANGENT => c.tangent.to_vec(),
                        _ => c.color.to_vec(),
                    })
                    .collect();
                attributes[name] = json!(wire.floats(&rows, shape)?);
            }
        }
        for (set, uv) in primitive.uvs.iter().enumerate() {
            let rows: Vec<_> = uv.iter().map(|p| p.to_vec()).collect();
            attributes[format!("TEXCOORD_{set}")] = json!(wire.floats(&rows, "VEC2")?);
        }
        for set in 0..2 {
            let mut bytes = allocate(count.checked_mul(4).ok_or_else(overflow)?)?;
            for (i, row) in primitive.triangles.iter().enumerate() {
                let id = if set == 0 {
                    primitive_ids[&key(row)]
                } else {
                    rank + i
                };
                for _ in 0..3 {
                    bytes.extend_from_slice(&(id as f32).to_le_bytes());
                }
            }
            attributes[format!("_FEATURE_ID_{set}")] =
                json!(wire.attribute(&bytes, count, 4, 5126, "SCALAR")?);
        }
        let mut output = json!({"attributes":attributes,"mode":4,"extensions":{"EXT_mesh_features":{"featureIds":[
            {"featureCount":primitive_ids.len(),"attribute":0,"propertyTable":0,"label":"source_primitive"},
            {"featureCount":total,"attribute":1,"propertyTable":1,"label":"source_triangle"}]}}});
        if let Some(material) = &primitive.material {
            output["material"] = json!(materials.len());
            materials.push(material.clone());
        }
        meshes.push(json!({"primitives":[output]}));
        nodes.push(node);
        rank += primitive.triangles.len();
    }
    let rows: Vec<&FullTriangle> = atlas
        .primitives
        .iter()
        .flat_map(|p| p.triangles.iter())
        .collect();
    let mut classes = Map::new();
    let mut tables = Vec::new();
    for (class, input) in [
        (
            "source_primitive",
            primitive_rows.values().copied().collect::<Vec<_>>(),
        ),
        ("source_triangle", rows),
    ] {
        let mut schema = Map::new();
        let mut properties = Map::new();
        let columns = if class == "source_primitive" { 5 } else { 11 };
        for column in 0..columns {
            let (name, component, width) = match column {
                0 => ("node_index", "UINT32", 4),
                1 => ("mesh_index", "UINT32", 4),
                2 => ("primitive_index", "UINT32", 4),
                3 => ("expanded_ordinal", "UINT64", 8),
                4 => ("material_index", "UINT32", 4),
                5 => ("triangle_index", "UINT64", 8),
                6 => ("vertex_0", "UINT64", 8),
                7 => ("vertex_1", "UINT64", 8),
                8 => ("vertex_2", "UINT64", 8),
                9 => ("attribute_flags", "UINT8", 1),
                _ => ("degenerate", "UINT8", 1),
            };
            let mut bytes = allocate(input.len().checked_mul(width).ok_or_else(overflow)?)?;
            for row in &input {
                let source = row.triangle.source;
                let value = match column {
                    0 => u64::from(source.node),
                    1 => u64::from(source.mesh),
                    2 => u64::from(source.primitive),
                    3 => row.expanded_ordinal,
                    4 => u64::from(row.material),
                    5 => source.triangle,
                    6..=8 => row.triangle.vertices[column - 6],
                    9 => u64::from(row.flags),
                    _ => u64::from(row.triangle.degenerate),
                };
                bytes.extend_from_slice(&value.to_le_bytes()[..width]);
            }
            properties.insert(name.into(), json!({"values":wire.view(&bytes,None)?}));
            schema.insert(
                name.into(),
                json!({"type":"SCALAR","componentType":component,"required":true}),
            );
        }
        classes.insert(class.into(), json!({"properties":schema}));
        tables.push(json!({"class":class,"count":input.len(),"properties":properties}));
    }
    let mut images = Vec::new();
    for page in &atlas.pages {
        images.push(json!({"bufferView":wire.view(&page.bytes,None)?,"mimeType":page.mime}));
    }
    let scene_nodes: Vec<_> = (0..nodes.len()).collect();
    let mut document = json!({"asset":{"version":"2.0"},"scene":0,"scenes":[{"nodes":scene_nodes}],"nodes":nodes,"meshes":meshes,
        "buffers":[{"byteLength":wire.binary.len()}],"bufferViews":wire.views,"accessors":wire.accessors,"materials":materials,
        "images":images,"textures":atlas.textures,"samplers":atlas.samplers,
        "extensionsUsed":["EXT_mesh_features","EXT_structural_metadata"],
        "extensions":{"EXT_structural_metadata":{"schema":{"id":"rusty_tiles_large_source_v1","classes":classes},"propertyTables":tables}}});
    for field in ["materials", "images", "textures", "samplers"] {
        if document[field].as_array().is_some_and(|a| a.is_empty()) {
            document
                .as_object_mut()
                .ok_or_else(|| invalid("leaf document object missing"))?
                .remove(field);
        }
    }
    if quantized {
        glb::add_extension(&mut document, "KHR_mesh_quantization", true)
            .map_err(|e| invalid(&e.to_string()))?;
    }
    if basis {
        glb::add_extension(&mut document, "KHR_texture_basisu", true)
            .map_err(|e| invalid(&e.to_string()))?;
    }
    let compressed = glb::meshopt_compress(
        &mut document,
        &wire.binary,
        MeshoptLayout {
            align: 8,
            explicit_filter: true,
        },
        |i, _| Ok(wire.streams[i]),
    )
    .map_err(|e| invalid(&e.to_string()))?;
    drop(wire.binary);
    let json_bytes = serde_json::to_vec(&document).map_err(|e| invalid(&e.to_string()))?;
    let decoded: Value =
        serde_json::from_slice(&json_bytes).map_err(|e| invalid(&e.to_string()))?;
    if decoded["nodes"] != document["nodes"] {
        return Err(invalid("leaf JSON real TRS roundtrip mismatch").into());
    }
    let final_len = glb::glb_len(json_bytes.len(), compressed.len()).map_err(|_| overflow())?;
    if final_len as u64 > limit {
        return Err(WriteError::Budget(budget()));
    }
    let output = glb::encode_glb(&document, &compressed).map_err(|e| invalid(&e.to_string()))?;
    if output.len() != final_len {
        return Err(invalid("leaf GLB framing length mismatch").into());
    }
    Ok(EncodedLeaf {
        glb: output,
        bounds,
        geometry_error,
    })
}

#[cfg(test)]
mod tests {
    use super::super::super::{
        attributes::{CornerAttributes, UV0, UV1},
        config::Profile,
        records::{Provenance, TriangleRecord},
    };
    use super::super::prepared::{PreparedPage, PreparedPrimitive};
    use super::*;
    fn row(ordinal: u64, x: f32, degenerate: bool) -> FullTriangle {
        let first = [x, -999_999.5, -999_999.0];
        FullTriangle {
            triangle: TriangleRecord {
                key: 0,
                source: Provenance {
                    node: 3,
                    mesh: 2,
                    primitive: 1,
                    triangle: ordinal,
                },
                vertices: [(1 << 60) + 3, (1 << 60) + 5, (1 << 60) + 7],
                positions: [
                    first,
                    if degenerate {
                        first
                    } else {
                        [x + 0.25, -999_999.5, -999_999.0]
                    },
                    [x, -999_999.0, -999_999.0],
                ],
                bytes: 272,
                degenerate,
            },
            expanded_ordinal: (1 << 54) + 9,
            material: 7,
            flags: NORMAL | TANGENT | UV0 | UV1 | COLOR,
            corners: [CornerAttributes {
                normal: [0., 0., 1.],
                tangent: [1., 0., 0., -1.],
                texcoords: [[0., 0.], [1., 1.]],
                color: [0.3, 0.5, 0.7, 0.9],
            }; 3],
        }
    }
    fn atlas() -> PreparedAtlas {
        let mut png = std::io::Cursor::new(Vec::new());
        image::DynamicImage::ImageRgba8(image::RgbaImage::from_pixel(
            1,
            1,
            image::Rgba([128, 128, 255, 255]),
        ))
        .write_to(&mut png, image::ImageFormat::Png)
        .unwrap();
        let material = json!({"normalTexture":{"index":0,"texCoord":2,"scale":-2.0},"occlusionTexture":{"index":0,"texCoord":3,"strength":0.5},
            "emissiveTexture":{"index":0,"texCoord":4},"emissiveFactor":[0.1,0.2,0.3],
            "pbrMetallicRoughness":{"baseColorTexture":{"index":0,"texCoord":0},"metallicRoughnessTexture":{"index":0,"texCoord":1},"roughnessFactor":0.75}});
        PreparedAtlas {
            primitives: vec![PreparedPrimitive {
                triangles: vec![
                    row((1 << 53) + 7, -999_999.75, false),
                    row((1 << 53) + 9, -999_990.25, true),
                ],
                corner_depths: vec![[1; 3]; 2],
                uvs: (0..5)
                    .map(|set| vec![[set as f32 / 8.0, 0.25]; 6])
                    .collect(),
                material: Some(material),
            }],
            pages: vec![PreparedPage {
                bytes: png.into_inner(),
                mime: "image/png",
            }],
            textures: vec![json!({"source":0,"sampler":0})],
            samplers: vec![json!({"magFilter":9729,"minFilter":9729,"wrapS":33071,"wrapT":33071})],
        }
    }
    fn plan(profile: Profile) -> LatticePlan {
        LatticePlan::new(
            profile,
            Bounds {
                min: [-1_000_000.; 3],
                max: [1_000_000.; 3],
            },
            &[2_000_000., 20.],
        )
        .unwrap()
    }
    fn parse(bytes: &[u8]) -> (Value, &[u8]) {
        assert_eq!(&bytes[..4], b"glTF");
        assert_eq!(u32::from_le_bytes(bytes[4..8].try_into().unwrap()), 2);
        assert_eq!(
            u32::from_le_bytes(bytes[8..12].try_into().unwrap()) as usize,
            bytes.len()
        );
        assert_eq!(&bytes[16..20], b"JSON");
        let length = u32::from_le_bytes(bytes[12..16].try_into().unwrap()) as usize;
        let doc = serde_json::from_slice(&bytes[20..20 + length]).unwrap();
        let start = 20 + length;
        assert_eq!(&bytes[start + 4..start + 8], b"BIN\0");
        let bin_length = u32::from_le_bytes(bytes[start..start + 4].try_into().unwrap()) as usize;
        assert_eq!(start + 8 + bin_length, bytes.len());
        (doc, &bytes[start + 8..])
    }
    fn view(doc: &Value, bin: &[u8], id: usize) -> Vec<u8> {
        let v = &doc["bufferViews"][id];
        if let Some(e) = v
            .get("extensions")
            .and_then(|x| x.get("EXT_meshopt_compression"))
        {
            assert_eq!(e["mode"], "ATTRIBUTES");
            assert_eq!(e["filter"], "NONE");
            let start = e["byteOffset"].as_u64().unwrap() as usize;
            let length = e["byteLength"].as_u64().unwrap() as usize;
            crate::content_integrity::meshopt::check_header(
                &bin[start..start + length],
                crate::content_integrity::meshopt::Mode::Attributes,
            )
            .unwrap();
            crate::content_integrity::meshopt::decode(
                &bin[start..start + length],
                e["count"].as_u64().unwrap() as usize,
                e["byteStride"].as_u64().unwrap() as usize,
                crate::content_integrity::meshopt::Mode::Attributes,
            )
            .unwrap()
        } else {
            assert_eq!(v["buffer"], 0);
            let start = v["byteOffset"].as_u64().unwrap() as usize;
            let length = v["byteLength"].as_u64().unwrap() as usize;
            bin[start..start + length].to_vec()
        }
    }
    fn column(doc: &Value, bin: &[u8], table: usize, name: &str) -> Vec<u8> {
        view(
            doc,
            bin,
            doc["extensions"]["EXT_structural_metadata"]["propertyTables"][table]["properties"]
                [name]["values"]
                .as_u64()
                .unwrap() as usize,
        )
    }
    #[test]
    fn quantized_meshopt_real_trs_bounds_full_attributes_slots_and_wide_identity() {
        let input = atlas();
        let lattice = plan(Profile::Balanced);
        let leaf = encode(&input, &lattice, 1, TILE_LIMIT).unwrap();
        let (doc, bin) = parse(&leaf.glb);
        assert!(doc["extensionsRequired"]
            .as_array()
            .unwrap()
            .contains(&json!("EXT_meshopt_compression")));
        assert!(doc["extensionsRequired"]
            .as_array()
            .unwrap()
            .contains(&json!("KHR_mesh_quantization")));
        let primitive = &doc["meshes"][0]["primitives"][0];
        assert!(primitive.get("indices").is_none());
        let attributes = &primitive["attributes"];
        assert_eq!(attributes.as_object().unwrap().len(), 11);
        let id = attributes["POSITION"].as_u64().unwrap() as usize;
        let accessor = &doc["accessors"][id];
        assert_eq!(accessor["componentType"], 5123);
        assert_eq!(accessor["normalized"], false);
        assert_eq!(accessor["count"], 6);
        let decoded = view(&doc, bin, accessor["bufferView"].as_u64().unwrap() as usize);
        let source: Vec<_> = input.primitives[0]
            .triangles
            .iter()
            .flat_map(|r| r.triangle.positions)
            .collect();
        let expected = lattice.encode(&source, &[1; 6], 1).unwrap();
        for (i, bytes) in decoded.as_chunks::<8>().0.iter().enumerate() {
            for a in 0..3 {
                let code = u16::from_le_bytes(bytes[a * 2..a * 2 + 2].try_into().unwrap());
                let scale = doc["nodes"][0]["scale"][a].as_f64().unwrap();
                let translation = doc["nodes"][0]["translation"][a].as_f64().unwrap();
                let real = f64::from(code) * scale + translation;
                assert_eq!(real, f64::from(expected.decoded[i][a]));
                assert!(
                    real >= f64::from(leaf.bounds.min[a]) && real <= f64::from(leaf.bounds.max[a])
                );
            }
        }
        assert_eq!(leaf.geometry_error, expected.correspondence_error);
        for (name, truth) in [
            ("NORMAL", vec![0., 0., 1.]),
            ("TANGENT", vec![1., 0., 0., -1.]),
            ("COLOR_0", vec![0.3, 0.5, 0.7, 0.9]),
        ] {
            let accessor = &doc["accessors"][attributes[name].as_u64().unwrap() as usize];
            let bytes = view(&doc, bin, accessor["bufferView"].as_u64().unwrap() as usize);
            let floats: Vec<_> = bytes
                .as_chunks::<4>()
                .0
                .iter()
                .map(|b| f32::from_le_bytes(*b))
                .collect();
            assert_eq!(floats, truth.repeat(6));
        }
        for set in 0..5 {
            let accessor =
                &doc["accessors"][attributes[format!("TEXCOORD_{set}")].as_u64().unwrap() as usize];
            assert_eq!(accessor["count"], 6);
        }
        assert_eq!(
            doc["materials"][0],
            input.primitives[0].material.clone().unwrap()
        );
        assert_eq!(doc["materials"][0]["normalTexture"]["scale"], -2.0);
        let triangle_bytes = column(&doc, bin, 1, "triangle_index");
        assert_eq!(
            u64::from_le_bytes(triangle_bytes[..8].try_into().unwrap()),
            (1 << 53) + 7
        );
        let expanded = column(&doc, bin, 1, "expanded_ordinal");
        assert_eq!(
            u64::from_le_bytes(expanded[..8].try_into().unwrap()),
            (1 << 54) + 9
        );
        let vertices = column(&doc, bin, 1, "vertex_2");
        assert_eq!(
            u64::from_le_bytes(vertices[..8].try_into().unwrap()),
            (1 << 60) + 7
        );
        assert_eq!(column(&doc, bin, 1, "degenerate"), [0, 1]);
        let fid = &doc["accessors"][attributes["_FEATURE_ID_1"].as_u64().unwrap() as usize];
        let bytes = view(&doc, bin, fid["bufferView"].as_u64().unwrap() as usize);
        let values: Vec<_> = bytes
            .as_chunks::<4>()
            .0
            .iter()
            .map(|b| f32::from_le_bytes(*b))
            .collect();
        assert_eq!(values, [0., 0., 0., 1., 1., 1.]);
        let page = view(
            &doc,
            bin,
            doc["images"][0]["bufferView"].as_u64().unwrap() as usize,
        );
        assert_eq!(page, input.pages[0].bytes);
    }
    #[test]
    fn float_profiles_keep_position_bits_and_each_node_owns_its_trs() {
        for profile in [Profile::Quality, Profile::Lossless] {
            let mut input = atlas();
            input.primitives.push(PreparedPrimitive {
                triangles: vec![row((1 << 53) + 11, -999_980.0, false)],
                corner_depths: vec![[1; 3]],
                uvs: vec![],
                material: None,
            });
            let leaf = encode(&input, &plan(profile), 1, TILE_LIMIT).unwrap();
            let (doc, bin) = parse(&leaf.glb);
            assert_eq!(doc["nodes"].as_array().unwrap().len(), 2);
            for (i, p) in input.primitives.iter().enumerate() {
                assert!(doc["nodes"][i].get("translation").is_none());
                assert!(doc["nodes"][i].get("scale").is_none());
                let attr = &doc["meshes"][i]["primitives"][0]["attributes"];
                let accessor = &doc["accessors"][attr["POSITION"].as_u64().unwrap() as usize];
                assert_eq!(accessor["componentType"], 5126);
                let bytes = view(&doc, bin, accessor["bufferView"].as_u64().unwrap() as usize);
                let positions: Vec<_> = bytes
                    .as_chunks::<4>()
                    .0
                    .iter()
                    .map(|b| u32::from_le_bytes(*b))
                    .collect();
                for set in 0..2 {
                    let features = &doc["meshes"][i]["primitives"][0]["extensions"]
                        ["EXT_mesh_features"]["featureIds"][set];
                    let feature_count = features["featureCount"].as_u64().unwrap() as usize;
                    let table_count = doc["extensions"]["EXT_structural_metadata"]["propertyTables"]
                        [set]["count"]
                        .as_u64()
                        .unwrap() as usize;
                    assert_eq!(feature_count, table_count);
                    let id_attr = &doc["accessors"]
                        [attr[format!("_FEATURE_ID_{set}")].as_u64().unwrap() as usize];
                    let id_bytes =
                        view(&doc, bin, id_attr["bufferView"].as_u64().unwrap() as usize);
                    for bytes in id_bytes.as_chunks::<4>().0 {
                        let id = f32::from_le_bytes(*bytes);
                        assert_eq!(id.fract(), 0.0);
                        assert!(id >= 0.0 && (id as usize) < feature_count);
                        if i == 1 && set == 1 {
                            assert_eq!(id, 2.0);
                        }
                    }
                }
                let expected: Vec<_> = p
                    .triangles
                    .iter()
                    .flat_map(|r| r.triangle.positions)
                    .flat_map(|v| v.map(f32::to_bits))
                    .collect();
                assert_eq!(positions, expected);
            }
            assert_eq!(leaf.geometry_error, 0.0);
        }
    }
    #[test]
    fn complete_budget_and_missing_slot_uv_are_typed_failures() {
        let input = atlas();
        assert_eq!(
            encode(&input, &plan(Profile::Balanced), 1, 100)
                .unwrap_err()
                .kind(),
            JobErrorKind::Unsupported
        );
        let mut bad = atlas();
        bad.primitives[0].uvs.pop();
        assert_eq!(
            encode(&bad, &plan(Profile::Balanced), 1, TILE_LIMIT)
                .unwrap_err()
                .kind(),
            JobErrorKind::InvalidState
        );
        let mut duplicate = atlas();
        duplicate.primitives[0].triangles[1] = duplicate.primitives[0].triangles[0];
        assert_eq!(
            encode(&duplicate, &plan(Profile::Balanced), 1, TILE_LIMIT)
                .unwrap_err()
                .kind(),
            JobErrorKind::InvalidState
        );
    }
    /// H16 consumer cap: every profile's leaf budget is at most 8192 triangles, an 8192-row
    /// leaf encodes with both tables at the cap, and an 8193-triangle plan is refused as a
    /// broken plan (not a retryable budget) before any table is built.
    #[test]
    fn property_tables_stay_within_the_consumer_row_limit() {
        for profile in [
            Profile::Fast,
            Profile::Balanced,
            Profile::Quality,
            Profile::Lossless,
        ] {
            assert!(
                profile.leaf_triangles() <= CONSUMER_TABLE_ROWS,
                "{profile:?}"
            );
        }
        let sized = |count: usize| {
            let mut input = atlas();
            let primitive = &mut input.primitives[0];
            primitive.triangles = (0..count)
                .map(|i| row(i as u64, -999_999.75 + (i % 64) as f32 * 0.25, false))
                .collect();
            primitive.corner_depths = vec![[1; 3]; count];
            for uv in &mut primitive.uvs {
                *uv = vec![[0.25, 0.25]; 3 * count];
            }
            input
        };
        let at_cap = encode(&sized(8192), &plan(Profile::Balanced), 1, TILE_LIMIT).unwrap();
        let (doc, _) = parse(&at_cap.glb);
        let counts: Vec<u64> = doc["extensions"]["EXT_structural_metadata"]["propertyTables"]
            .as_array()
            .unwrap()
            .iter()
            .map(|table| table["count"].as_u64().unwrap())
            .collect();
        assert_eq!(counts, [1, 8192]);
        assert!(matches!(
            encode(&sized(8193), &plan(Profile::Balanced), 1, TILE_LIMIT),
            Err(WriteError::Failure(error)) if error.kind() == JobErrorKind::InvalidState
                && error.message().contains("consumer row limit")
        ));
    }
    #[test]
    fn only_actual_member_size_errors_are_retryable_budget() {
        let input = atlas();
        assert!(matches!(
            encode(&input, &plan(Profile::Balanced), 1, 100),
            Err(WriteError::Budget(_))
        ));
        let complete = encode(&input, &plan(Profile::Balanced), 1, TILE_LIMIT).unwrap();
        assert!(matches!(
            encode(
                &input,
                &plan(Profile::Balanced),
                1,
                complete.glb.len() as u64 - 1
            ),
            Err(WriteError::Budget(_))
        ));
        // A valid prepared closure with geometry outside the frozen source frame
        // fails directly, rather than coarsening textures to conceal that defect.
        let mut geometry = atlas();
        geometry.primitives[0].triangles[0].triangle.positions[0][0] = 1_000_001.0;
        assert!(matches!(
            encode(&geometry, &plan(Profile::Balanced), 1, TILE_LIMIT),
            Err(WriteError::Failure(_))
        ));
        let mut closure = atlas();
        closure.primitives[0].uvs.pop();
        assert!(matches!(
            encode(&closure, &plan(Profile::Balanced), 1, TILE_LIMIT),
            Err(WriteError::Failure(_))
        ));
        let unsupported: WriteError =
            JobError::new(JobErrorKind::Unsupported, "fidelity gate control").into();
        assert_eq!(unsupported.kind(), JobErrorKind::Unsupported);
        assert!(matches!(unsupported, WriteError::Failure(_)));
    }
}
