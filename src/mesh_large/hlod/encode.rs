//! Coarse tile GLB: one node and glTF primitive per material group, non-indexed triangles
//! (one `source_primitive` feature ID per triangle at all three corners, D4), smooth
//! normals when the source carries them, and the S3 appearance: fresh-chart TEXCOORD_0 and
//! embedded pages for textured classes (tangents with a baked normal map), per-vertex
//! COLOR_0 or flat class factors otherwise; meshopt compression. Positions keep every
//! outer-interface vertex at its leaf-canonical decode. The bytes are decoded again and
//! checked before they are published (D1).
use super::{
    appearance::Appearance,
    store::{SourcePrimitive, TileGeometry},
};
use crate::{
    glb::{self, MeshoptLayout, MeshoptStream},
    mesh_large::{
        batching::material_key,
        leaves::geometry::{CoarsePositions, PositionStorage},
        records::Bounds,
    },
    JobError, JobErrorKind,
};
use serde_json::{json, Map, Value};
use std::collections::HashSet;

fn invalid(message: impl Into<String>) -> JobError {
    JobError::new(JobErrorKind::InvalidState, message.into())
}

/// Test controls: defects the pre-publication check or the independent census must catch.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum Mutation {
    #[default]
    None,
    /// Drop one face holding a required segment, with the pre-publication check on.
    DropSegment,
    /// Corrupt the expected measurement position while leaving published bytes intact.
    WrongMeasuredPosition,
    /// Move one locked vertex by one code/ulp and skip the pre-publication check.
    MoveVertexUnchecked,
}

pub(crate) struct Encoded {
    pub glb: Vec<u8>,
    pub bounds: Bounds,
    pub displacement: f64,
    pub decoded: Vec<[f32; 3]>,
    pub face_order: Vec<usize>,
    pub gltf_primitives: usize,
    pub quantized: bool,
}

/// Per vertex: whether it lies on an open-border edge (held by other than two faces) of
/// the nondegenerate triangles.
pub(crate) fn open_border(triangles: &[[u32; 3]], vertices: usize) -> Vec<bool> {
    let mut edges: Vec<(u32, u32)> = triangles
        .iter()
        .filter(|t| t[0] != t[1] && t[1] != t[2] && t[0] != t[2])
        .flat_map(|t| (0..3).map(move |k| (t[k].min(t[(k + 1) % 3]), t[k].max(t[(k + 1) % 3]))))
        .collect();
    edges.sort_unstable();
    let mut border = vec![false; vertices];
    let mut i = 0;
    while i < edges.len() {
        let mut j = i + 1;
        while j < edges.len() && edges[j] == edges[i] {
            j += 1;
        }
        if j - i != 2 {
            border[edges[i].0 as usize] = true;
            border[edges[i].1 as usize] = true;
        }
        i = j;
    }
    border
}

struct Wire {
    binary: Vec<u8>,
    views: Vec<Value>,
    streams: Vec<Option<MeshoptStream>>,
    accessors: Vec<Value>,
}

impl Wire {
    fn view(&mut self, bytes: &[u8], stream: Option<MeshoptStream>) -> usize {
        let offset = glb::append_aligned(&mut self.binary, 8, bytes);
        let mut v = json!({"buffer":0,"byteOffset":offset,"byteLength":bytes.len()});
        if let Some(MeshoptStream::Attributes { stride, .. }) = stream {
            v["byteStride"] = json!(stride);
            v["target"] = json!(34962);
        }
        self.views.push(v);
        self.streams.push(stream);
        self.views.len() - 1
    }
    fn attribute(&mut self, bytes: &[u8], count: usize, stride: usize, accessor: Value) -> usize {
        let view = self.view(bytes, Some(MeshoptStream::Attributes { count, stride }));
        let mut accessor = accessor;
        accessor["bufferView"] = json!(view);
        accessor["count"] = json!(count);
        self.accessors.push(accessor);
        self.accessors.len() - 1
    }
    /// A tightly packed property column; 4/8-byte values are a lossless meshopt stream.
    fn column(&mut self, bytes: &[u8], rows: usize, width: usize) -> usize {
        let id = self.view(bytes, None);
        if width.is_multiple_of(4) && !bytes.is_empty() {
            self.streams[id] = Some(MeshoptStream::Attributes {
                count: rows,
                stride: width,
            });
        }
        id
    }
}

fn table(wire: &mut Wire, rows: &[SourcePrimitive]) -> (Value, Value) {
    let columns: [(&str, &str, Vec<u64>); 5] = [
        (
            "node_index",
            "UINT32",
            rows.iter().map(|p| u64::from(p.node)).collect(),
        ),
        (
            "mesh_index",
            "UINT32",
            rows.iter().map(|p| u64::from(p.mesh)).collect(),
        ),
        (
            "primitive_index",
            "UINT32",
            rows.iter().map(|p| u64::from(p.primitive)).collect(),
        ),
        (
            "expanded_ordinal",
            "UINT64",
            rows.iter().map(|p| p.expanded_ordinal).collect(),
        ),
        (
            "material_index",
            "UINT32",
            rows.iter().map(|p| u64::from(p.material)).collect(),
        ),
    ];
    let mut schema = Map::new();
    let mut properties = Map::new();
    for (name, component, values) in columns {
        let width = if component == "UINT64" { 8 } else { 4 };
        let mut bytes = Vec::with_capacity(values.len() * width);
        for v in values {
            bytes.extend_from_slice(&v.to_le_bytes()[..width]);
        }
        let id = wire.column(&bytes, rows.len(), width);
        properties.insert(name.into(), json!({"values":id}));
        schema.insert(
            name.into(),
            json!({"type":"SCALAR","componentType":component,"required":true}),
        );
    }
    (
        json!({"source_primitive":{"properties":schema}}),
        json!({"class":"source_primitive","count":rows.len(),"properties":properties}),
    )
}

fn unorm16(v: f32) -> [u8; 2] {
    ((v.clamp(0., 1.) * 65535.).round() as u16).to_le_bytes()
}
fn snorm8(v: f32) -> u8 {
    ((v.clamp(-1., 1.) * 127.).round() as i8) as u8
}

/// Encode `tile` (its `locked` vertices on the outer interface) at its tile depth, at
/// `positions` with `appearance`.
pub(crate) fn encode(
    tile: &TileGeometry,
    locked: &[bool],
    mut positions: CoarsePositions,
    appearance: &Appearance,
    mutation: Mutation,
) -> Result<Encoded, JobError> {
    let rows = tile.primitives.len();
    let limit = crate::mesh_large::config::CONSUMER_TABLE_ROWS as usize;
    if rows == 0 || rows > limit || tile.triangles.is_empty() || tile.triangles.len() > limit {
        return Err(invalid("coarse tile rows or triangles outside 1..=8192"));
    }
    if mutation == Mutation::MoveVertexUnchecked {
        if let Some(v) = locked.iter().position(|l| *l) {
            match &mut positions.storage {
                PositionStorage::Quantized { codes, .. } => {
                    codes[v][0] = codes[v][0].wrapping_add(1);
                }
                PositionStorage::Float { positions } => {
                    positions[v][0] = f32::from_bits(positions[v][0].to_bits() + 1);
                }
            }
        }
    }
    let mut wire = Wire {
        binary: Vec::new(),
        views: Vec::new(),
        streams: Vec::new(),
        accessors: Vec::new(),
    };
    let (classes_json, table_json) = table(&mut wire, &tile.primitives);
    // Untextured, uncoloured groups with equal flat materials (H28 classes, numbers by
    // value) share one glTF primitive, as in S1; textured or coloured groups stand alone.
    struct Out<'g> {
        key: Option<Value>,
        material: Value,
        groups: Vec<&'g super::appearance::Group>,
    }
    let mut outs: Vec<Out<'_>> = Vec::new();
    for g in &appearance.groups {
        let plain = g.uvs.is_none() && g.colors.is_none();
        let key = plain.then(|| material_key(&g.material, false));
        match outs.iter_mut().find(|o| key.is_some() && o.key == key) {
            Some(o) => o.groups.push(g),
            None => outs.push(Out {
                key,
                material: g.material.clone(),
                groups: vec![g],
            }),
        }
    }
    let mut images = Vec::new();
    let mut textures = Vec::new();
    let mut basis = false;
    let mut face_order = Vec::with_capacity(tile.triangles.len());
    let mut nodes = Vec::new();
    let mut meshes = Vec::new();
    let mut materials_json = Vec::new();
    for (index, out) in outs.into_iter().enumerate() {
        // (tile face, group, group face) in tile face order.
        let mut faces: Vec<(usize, usize, usize)> = out
            .groups
            .iter()
            .enumerate()
            .flat_map(|(g, group)| {
                group
                    .faces
                    .iter()
                    .enumerate()
                    .map(move |(gf, &f)| (f, g, gf))
            })
            .collect();
        faces.sort_unstable();
        face_order.extend(faces.iter().map(|f| f.0));
        let mut material = out.material;
        let lead = out.groups[0];
        if !lead.pages.is_empty() {
            let base = textures.len();
            for page in &lead.pages {
                let image = images.len();
                images
                    .push(json!({"bufferView":wire.view(&page.bytes, None),"mimeType":page.mime}));
                if page.mime == "image/ktx2" {
                    basis = true;
                    textures.push(json!({"sampler":u32::from(!page.mipmapped),"extensions":{"KHR_texture_basisu":{"source":image}}}));
                } else {
                    textures.push(json!({"sampler":u32::from(!page.mipmapped),"source":image}));
                }
            }
            offset_textures(&mut material, base);
        }
        materials_json.push(material);
        let count = faces.len() * 3;
        let corners = || faces.iter().flat_map(|&(t, _, _)| tile.triangles[t]);
        let position = match &positions.storage {
            PositionStorage::Quantized { codes, .. } => {
                let mut bytes = Vec::with_capacity(count * 8);
                let mut min = [u16::MAX; 3];
                let mut max = [0u16; 3];
                for v in corners() {
                    for (a, c) in codes[v as usize].iter().enumerate() {
                        bytes.extend_from_slice(&c.to_le_bytes());
                        min[a] = min[a].min(*c);
                        max[a] = max[a].max(*c);
                    }
                    bytes.extend_from_slice(&[0, 0]);
                }
                wire.attribute(&bytes, count, 8, json!({"componentType":5123,"type":"VEC3","normalized":false,"min":min,"max":max}))
            }
            PositionStorage::Float { positions } => {
                let mut bytes = Vec::with_capacity(count * 12);
                let mut min = [f32::INFINITY; 3];
                let mut max = [f32::NEG_INFINITY; 3];
                for v in corners() {
                    for (a, c) in positions[v as usize].iter().enumerate() {
                        bytes.extend_from_slice(&c.to_le_bytes());
                        min[a] = min[a].min(*c);
                        max[a] = max[a].max(*c);
                    }
                }
                wire.attribute(&bytes, count, 12, json!({"componentType":5126,"type":"VEC3","min":min.map(f64::from),"max":max.map(f64::from)}))
            }
        };
        let mut attributes = serde_json::Map::new();
        attributes.insert("POSITION".into(), json!(position));
        if let Some(smooth) = &appearance.normals {
            let mut bytes = Vec::with_capacity(count * 4);
            for v in corners() {
                for c in smooth[v as usize] {
                    bytes.push(snorm8(c));
                }
                bytes.push(0);
            }
            let normal = wire.attribute(
                &bytes,
                count,
                4,
                json!({"componentType":5120,"type":"VEC3","normalized":true}),
            );
            attributes.insert("NORMAL".into(), json!(normal));
        }
        let groups = &out.groups;
        if let Some(uvs) = &lead.uvs {
            let mut bytes = Vec::with_capacity(count * 4);
            for &(_, _, gf) in &faces {
                for uv in uvs[gf] {
                    bytes.extend(unorm16(uv[0]));
                    bytes.extend(unorm16(uv[1]));
                }
            }
            let id = wire.attribute(
                &bytes,
                count,
                4,
                json!({"componentType":5123,"type":"VEC2","normalized":true}),
            );
            attributes.insert("TEXCOORD_0".into(), json!(id));
        }
        if let Some(tangents) = &lead.tangents {
            let mut bytes = Vec::with_capacity(count * 4);
            for &(_, _, gf) in &faces {
                for t in tangents[gf] {
                    bytes.extend(t.map(snorm8));
                }
            }
            let id = wire.attribute(
                &bytes,
                count,
                4,
                json!({"componentType":5120,"type":"VEC4","normalized":true}),
            );
            attributes.insert("TANGENT".into(), json!(id));
        }
        if lead.colors.is_some() {
            let mut bytes = Vec::with_capacity(count * 8);
            for &(_, g, gf) in &faces {
                let colors = groups[g]
                    .colors
                    .as_ref()
                    .ok_or_else(|| invalid("mixed colour group"))?;
                for c in colors[gf] {
                    for v in c {
                        bytes.extend(unorm16(v));
                    }
                }
            }
            let id = wire.attribute(
                &bytes,
                count,
                8,
                json!({"componentType":5123,"type":"VEC4","normalized":true}),
            );
            attributes.insert("COLOR_0".into(), json!(id));
        }
        let mut bytes = Vec::with_capacity(count * 4);
        for &(t, _, _) in &faces {
            let id = tile.representative[t] as f32;
            for _ in 0..3 {
                bytes.extend_from_slice(&id.to_le_bytes());
            }
        }
        let feature = wire.attribute(
            &bytes,
            count,
            4,
            json!({"componentType":5126,"type":"SCALAR"}),
        );
        let mut node = json!({"mesh":meshes.len()});
        if let PositionStorage::Quantized {
            translation, scale, ..
        } = &positions.storage
        {
            node["translation"] = json!(translation.map(f64::from));
            node["scale"] = json!([f64::from(*scale), f64::from(*scale), f64::from(*scale)]);
        }
        nodes.push(node);
        attributes.insert("_FEATURE_ID_0".into(), json!(feature));
        meshes.push(json!({"primitives":[{
            "attributes":attributes,
            "mode":4,"material":index,
            "extensions":{"EXT_mesh_features":{"featureIds":[
                {"featureCount":rows,"attribute":0,"propertyTable":0,"label":"source_primitive"}]}}}]}));
    }
    let gltf_primitives = meshes.len();
    let Wire {
        binary,
        views,
        streams,
        accessors,
    } = wire;
    let mut document = json!({"asset":{"version":"2.0","generator":"rusty-tiles HLOD"},"scene":0,
        "scenes":[{"nodes":(0..nodes.len()).collect::<Vec<_>>()}],"nodes":nodes,"meshes":meshes,
        "materials":materials_json,"images":images,"textures":textures,
        // Complete KTX2 chains are mip-sampled; a PNG fallback page is level 0 only.
        "samplers":[{"magFilter":9729,"minFilter":9987,"wrapS":33071,"wrapT":33071},
            {"magFilter":9729,"minFilter":9729,"wrapS":33071,"wrapT":33071}],
        "buffers":[{"byteLength":binary.len()}],"bufferViews":views,
        "accessors":accessors,"extensionsUsed":["EXT_mesh_features","EXT_structural_metadata"],
        "extensions":{"EXT_structural_metadata":{"schema":{"id":"rusty_tiles_large_hlod_v1",
        "classes":classes_json},"propertyTables":[table_json]}}});
    if images.is_empty() {
        for field in ["images", "textures", "samplers"] {
            if let Some(o) = document.as_object_mut() {
                o.remove(field);
            }
        }
    }
    // Byte normals and normalized UVs need KHR_mesh_quantization even for float positions.
    glb::add_extension(&mut document, "KHR_mesh_quantization", true)
        .map_err(|e| invalid(e.to_string()))?;
    if basis {
        glb::add_extension(&mut document, "KHR_texture_basisu", true)
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
    let bytes = glb::encode_glb(&document, &compressed).map_err(|e| invalid(e.to_string()))?;
    let quantized = matches!(positions.storage, PositionStorage::Quantized { .. });
    let mut result = Encoded {
        glb: bytes,
        face_order,
        bounds: positions.bounds,
        displacement: positions.displacement,
        decoded: positions.decoded,
        gltf_primitives,
        quantized,
    };
    if mutation == Mutation::WrongMeasuredPosition {
        let v = tile.triangles[0][0] as usize;
        result.decoded[v][0] = result.decoded[v][0].next_up();
    }
    Ok(result)
}

/// Shift every texture index of a coarse material by `base` (group-local → document).
fn offset_textures(material: &mut Value, base: usize) {
    for pointer in [
        "/pbrMetallicRoughness/baseColorTexture/index",
        "/pbrMetallicRoughness/metallicRoughnessTexture/index",
        "/normalTexture/index",
        "/occlusionTexture/index",
        "/emissiveTexture/index",
    ] {
        if let Some(v) = material.pointer_mut(pointer) {
            *v = json!(v.as_u64().unwrap_or(0) as usize + base);
        }
    }
}

/// D1's pre-publication check on the encoded bytes, decoded as a client does: the triangle
/// count, every required segment and interface vertex at its exact leaf-canonical decode,
/// every decoded open-border vertex at a leaf-canonical position of the tile (no border
/// vertex is displaced, perpendicular or otherwise), and every triangle's representative
/// row. Every decoded corner must carry the measured position bits; returns the
/// published positions.
pub(crate) fn check(
    tile: &TileGeometry,
    locked: &[bool],
    encoded: &Encoded,
) -> Result<Vec<[f32; 3]>, JobError> {
    let decoded = super::decode::decode(&encoded.glb)?;
    let mut published = vec![[0.; 3]; tile.positions.len()];
    let what = || format!("coarse tile f{}-d{}", tile.key.first, tile.key.depth);
    if decoded.faces.len() != tile.triangles.len()
        || encoded.face_order.len() != tile.triangles.len()
    {
        return Err(invalid(format!(
            "{}: decoded triangle count differs",
            what()
        )));
    }
    let bits = |p: [f32; 3]| p.map(f32::to_bits);
    let mut edges = HashSet::with_capacity(decoded.faces.len() * 3);
    let mut points = HashSet::with_capacity(decoded.faces.len() * 3);
    let mut counted: std::collections::HashMap<([u32; 3], [u32; 3]), u32> =
        std::collections::HashMap::with_capacity(decoded.faces.len() * 3);
    let mut seen = vec![false; tile.triangles.len()];
    for (face, &triangle) in decoded.faces.iter().zip(&encoded.face_order) {
        let mapped = seen
            .get_mut(triangle)
            .ok_or_else(|| invalid("decoded face mapping outside tile"))?;
        if std::mem::replace(mapped, true) {
            return Err(invalid("duplicate decoded face mapping"));
        }
        let (id, _) = face
            .features
            .first()
            .copied()
            .ok_or_else(|| invalid(format!("{}: a face without a feature ID", what())))?;
        if !face.uniform_features || id as usize >= tile.primitives.len() {
            return Err(invalid(format!(
                "{}: invalid representative feature ID",
                what()
            )));
        }
        if id != tile.representative[triangle] {
            return Err(invalid("decoded representative differs"));
        }
        for (k, &vertex) in tile.triangles[triangle].iter().enumerate() {
            if bits(face.p[k]) != bits(encoded.decoded[vertex as usize]) {
                return Err(invalid("measured positions differ from decoded GLB bits"));
            }
            published[vertex as usize] = face.p[k];
        }
        let corners = face.p.map(bits);
        let degenerate =
            corners[0] == corners[1] || corners[1] == corners[2] || corners[0] == corners[2];
        for k in 0..3 {
            let (a, b) = (corners[k], corners[(k + 1) % 3]);
            let e = if a <= b { (a, b) } else { (b, a) };
            edges.insert(e);
            points.insert(a);
            if !degenerate {
                *counted.entry(e).or_default() += 1;
            }
        }
    }
    let canonical: HashSet<[u32; 3]> = tile.positions.iter().map(|p| bits(*p)).collect();
    if counted
        .iter()
        .any(|((a, b), n)| *n != 2 && !(canonical.contains(a) && canonical.contains(b)))
    {
        return Err(invalid(format!(
            "{}: a decoded open-border vertex left its leaf-canonical position",
            what()
        )));
    }
    let locked_points: HashSet<[u32; 3]> = tile
        .positions
        .iter()
        .zip(locked)
        .filter(|(_, l)| **l)
        .map(|(p, _)| bits(*p))
        .collect();
    for row in &tile.interface {
        if !locked_points.contains(&row.a) || !locked_points.contains(&row.b) {
            return Err(invalid(format!("{}: interface point not locked", what())));
        }
        if row.is_vertex() {
            if !points.contains(&row.a) {
                return Err(invalid(format!(
                    "{}: decoded bytes lost interface point {:?}",
                    what(),
                    row.a.map(f32::from_bits)
                )));
            }
        } else if !edges.contains(&(row.a, row.b)) {
            return Err(invalid(format!(
                "{}: decoded bytes lost interface segment {:?}-{:?}",
                what(),
                row.a.map(f32::from_bits),
                row.b.map(f32::from_bits)
            )));
        }
    }
    Ok(published)
}
