//! Coarse tile GLB: one node and glTF primitive per material class, non-indexed triangles
//! (one `source_primitive` feature ID per triangle at all three corners, D4), smooth
//! normals, flat class materials (S1 appearance; S3 replaces it with fresh charts), meshopt
//! compression. Positions keep every outer-interface vertex at its leaf-canonical decode.
//! The bytes are decoded again and checked before they are published (D1).
use super::store::{SourcePrimitive, TileGeometry};
use crate::{
    glb::{self, MeshoptLayout, MeshoptStream},
    mesh_large::{
        batching::{material_key, MaterialClasses},
        leaves::geometry::{LatticePlan, PositionStorage},
        records::Bounds,
    },
    JobError, JobErrorKind,
};
use serde_json::{json, Map, Value};
use std::collections::{BTreeMap, HashSet};

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
    /// Move one locked vertex by one code/ulp and skip the pre-publication check.
    MoveVertexUnchecked,
}

pub(crate) struct Encoded {
    pub glb: Vec<u8>,
    pub bounds: Bounds,
    pub displacement: f64,
    pub decoded: Vec<[f32; 3]>,
    pub gltf_primitives: usize,
    pub quantized: bool,
}

/// The tile's flat material per class: the class representative's literal factors.
pub(crate) fn flat_materials(materials: &[crate::gltf_source::source::Material]) -> Vec<Value> {
    materials
        .iter()
        .map(|m| {
            let mut value = m.flat_proxy();
            if let Some(object) = value.as_object_mut() {
                object.remove("name");
            }
            value
        })
        .collect()
}

/// Area-weighted smooth normal per vertex over decoded positions.
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

/// Encode `tile` (its `locked` vertices on the outer interface) at its tile depth.
pub(crate) fn encode(
    tile: &TileGeometry,
    locked: &[bool],
    lattice: &LatticePlan,
    materials: &[Value],
    classes: &MaterialClasses,
    mutation: Mutation,
) -> Result<Encoded, JobError> {
    let rows = tile.primitives.len();
    let limit = crate::mesh_large::config::CONSUMER_TABLE_ROWS as usize;
    if rows == 0 || rows > limit || tile.triangles.is_empty() || tile.triangles.len() > limit {
        return Err(invalid("coarse tile rows or triangles outside 1..=8192"));
    }
    let mut positions =
        lattice.encode_coarse(&tile.positions, &tile.ells, locked, tile.key.depth)?;
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
    let smooth = normals(&positions.decoded, &tile.triangles);
    let mut wire = Wire {
        binary: Vec::new(),
        views: Vec::new(),
        streams: Vec::new(),
        accessors: Vec::new(),
    };
    let (classes_json, table_json) = table(&mut wire, &tile.primitives);
    // Group triangles by the material class of their representative; equal flat materials
    // (H28 material classes, numbers by value) share one glTF primitive.
    let default = json!({"pbrMetallicRoughness":{"metallicFactor":0.0,"roughnessFactor":1.0}});
    let mut groups: BTreeMap<u32, Vec<usize>> = BTreeMap::new();
    for (t, r) in tile.representative.iter().enumerate() {
        let material = tile.primitives[*r as usize].material;
        groups.entry(classes.of(material)).or_default().push(t);
    }
    let mut keyed: Vec<(Value, Value, Vec<usize>)> = Vec::new();
    for (class, faces) in groups {
        let material = materials
            .get(class as usize)
            .cloned()
            .unwrap_or_else(|| default.clone());
        let key = material_key(&material, false);
        match keyed.iter_mut().find(|(k, _, _)| *k == key) {
            Some((_, _, all)) => all.extend(faces),
            None => keyed.push((key, material, faces)),
        }
    }
    let mut nodes = Vec::new();
    let mut meshes = Vec::new();
    let mut materials_json = Vec::new();
    for (index, (_, material, mut faces)) in keyed.into_iter().enumerate() {
        faces.sort_unstable();
        materials_json.push(material);
        let count = faces.len() * 3;
        let corners = || faces.iter().flat_map(|&t| tile.triangles[t]);
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
        let mut bytes = Vec::with_capacity(count * 4);
        for v in corners() {
            for c in smooth[v as usize] {
                bytes.push(((c.clamp(-1., 1.) * 127.).round() as i8) as u8);
            }
            bytes.push(0);
        }
        let normal = wire.attribute(
            &bytes,
            count,
            4,
            json!({"componentType":5120,"type":"VEC3","normalized":true}),
        );
        let mut bytes = Vec::with_capacity(count * 4);
        for &t in &faces {
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
        meshes.push(json!({"primitives":[{
            "attributes":{"POSITION":position,"NORMAL":normal,"_FEATURE_ID_0":feature},
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
        "materials":materials_json,"buffers":[{"byteLength":binary.len()}],"bufferViews":views,
        "accessors":accessors,"extensionsUsed":["EXT_mesh_features","EXT_structural_metadata"],
        "extensions":{"EXT_structural_metadata":{"schema":{"id":"rusty_tiles_large_hlod_v1",
        "classes":classes_json},"propertyTables":[table_json]}}});
    // Byte normals need KHR_mesh_quantization even for float positions.
    glb::add_extension(&mut document, "KHR_mesh_quantization", true)
        .map_err(|e| invalid(e.to_string()))?;
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
    Ok(Encoded {
        glb: bytes,
        bounds: positions.bounds,
        displacement: positions.displacement,
        decoded: positions.decoded,
        gltf_primitives,
        quantized,
    })
}

/// D1's pre-publication check on the encoded bytes, decoded as a client does: the triangle
/// count, every required segment and interface vertex at its exact leaf-canonical decode,
/// and every triangle's representative row.
pub(crate) fn check(tile: &TileGeometry, locked: &[bool], bytes: &[u8]) -> Result<(), JobError> {
    let decoded = crate::mesh_large::lod::codec::decode(bytes)?;
    let what = || format!("coarse tile f{}-d{}", tile.key.first, tile.key.depth);
    if decoded.faces.len() != tile.triangles.len() {
        return Err(invalid(format!(
            "{}: decoded triangle count differs",
            what()
        )));
    }
    let bits = |p: [f32; 3]| p.map(f32::to_bits);
    let mut edges = HashSet::with_capacity(decoded.faces.len() * 3);
    let mut points = HashSet::with_capacity(decoded.faces.len() * 3);
    for face in &decoded.faces {
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
        for k in 0..3 {
            let (a, b) = (bits(face.p[k]), bits(face.p[(k + 1) % 3]));
            edges.insert(if a <= b { (a, b) } else { (b, a) });
            points.insert(a);
        }
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
    Ok(())
}
