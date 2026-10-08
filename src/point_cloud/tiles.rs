//! Disk-backed spatial partitioning; only an input chunk or a tile's bounded
//! representatives/full-detail records are held in memory at a time.
use super::source::{position, Layout, RAW};
use super::PointCloudOptions;
use crate::report::Reporter;
use crate::{
    metadata::MetadataGlb,
    tileset_node::{
        box_half, box_json, enclose_children, top_level_error, translation, translation_offset,
    },
    vec3::{norm_hypot as norm, sub},
    Error,
};
use serde_json::{json, Value};
use std::{
    collections::BTreeMap,
    fs::File,
    io::{BufReader, BufWriter, Read, Write},
    path::Path,
};

struct Records {
    reader: BufReader<File>,
    record_len: usize,
    batch: Vec<u8>,
}

impl Records {
    fn new(path: &Path, record_len: usize, budget: usize) -> Result<Self, Error> {
        let file = File::open(path)?;
        let size = file.metadata()?.len();
        if !size.is_multiple_of(record_len as u64) {
            return Err(Error::Data("truncated point scratch records".into()));
        }
        let count = usize::try_from((size / record_len as u64).min(budget as u64))
            .map_err(|_| Error::Data("point chunk exceeds platform limits".into()))?;
        let bytes = record_len
            .checked_mul(count)
            .ok_or_else(|| Error::Data("point chunk exceeds platform limits".into()))?;
        Ok(Self {
            reader: BufReader::new(file),
            record_len,
            batch: vec![0; bytes],
        })
    }

    fn next(&mut self) -> Result<&[u8], Error> {
        let mut read = 0;
        while read < self.batch.len() {
            let n = self.reader.read(&mut self.batch[read..])?;
            if n == 0 {
                break;
            }
            read += n;
        }
        if !read.is_multiple_of(self.record_len) {
            return Err(Error::Data("truncated point scratch records".into()));
        }
        Ok(&self.batch[..read])
    }
}

pub(super) struct Tree<'a> {
    pub reporter: &'a Reporter,
    pub layout: &'a Layout,
    pub options: &'a PointCloudOptions,
    pub output: &'a Path,
    pub tiles: usize,
    pub max_rounding: f64,
    pub total_points: u64,
    pub leaf_points: u64,
}

impl Tree<'_> {
    pub fn build(
        &mut self,
        path: &Path,
        parent_center: [f64; 3],
        depth: usize,
        cell: Option<([f64; 3], [f64; 3])>,
    ) -> Result<Value, Error> {
        let mut records = Records::new(path, self.layout.record_len, self.options.chunk_points)?;
        let mut lo = [f64::INFINITY; 3];
        let mut hi = [f64::NEG_INFINITY; 3];
        let mut count = 0_u64;
        loop {
            let batch = records.next()?;
            if batch.is_empty() {
                break;
            }
            for row in batch.chunks_exact(self.layout.record_len) {
                let p = position(row);
                for i in 0..3 {
                    lo[i] = lo[i].min(p[i]);
                    hi[i] = hi[i].max(p[i]);
                }
                count += 1;
            }
        }
        if count == 0 {
            return Err(Error::Data("empty point partition".into()));
        }
        drop(records);
        let extent = sub(hi, lo);
        if !norm(extent).is_finite() {
            return Err(Error::Data(
                "point extent exceeds finite coordinate range".into(),
            ));
        }
        let center: [f64; 3] = std::array::from_fn(|i| lo[i] + extent[i] / 2.);
        let leaf = count <= self.options.max_points as u64;
        if leaf && count > 16_777_217 {
            return Err(Error::Data("too many exact feature IDs in one tile".into()));
        }
        let (rows, error) = if leaf {
            (std::fs::read(path)?, 0.)
        } else {
            self.sample(path, lo, extent)?
        };
        let index = self.tiles;
        self.tiles += 1;
        let uri = format!("t/{index}.glb");
        let rounding = emit(
            &rows,
            center,
            self.layout,
            self.options.metadata_attributes,
            &self.output.join(&uri),
        )?;
        self.max_rounding = self.max_rounding.max(rounding);
        drop(rows);
        let mut half = std::array::from_fn(|i| (extent[i] / 2. + rounding).max(1e-6));
        let mut node = json!({"boundingVolume":{"box":box_json(0., half)},
            "transform":translation(sub(center, parent_center)),
            "geometricError":error + if leaf {0.} else {rounding},
            "refine":"REPLACE","content":{"uri":uri}});
        if leaf {
            std::fs::remove_file(path)?;
            self.leaf_points += count;
            self.reporter
                .progress("tiling", self.leaf_points, self.total_points);
            return Ok(node);
        }
        let depth_limit = if self.options.explicit { 64 } else { 31 };
        if depth >= depth_limit {
            return Err(Error::Data(format!(
                "point partition exceeds {depth_limit} levels; inspect source extent or raise maxPoints"
            )));
        }
        let axis = (0..3)
            .max_by(|a, b| extent[*a].total_cmp(&extent[*b]).then_with(|| b.cmp(a)))
            .unwrap();
        let branches = if self.options.explicit { 2 } else { 8 };
        let (cell_lo, cell_hi) = cell.unwrap_or((lo, hi));
        let midpoint: [f64; 3] =
            std::array::from_fn(|i| cell_lo[i] + (cell_hi[i] - cell_lo[i]) / 2.);
        let paths: Vec<_> = (0..branches)
            .map(|branch| self.output.join(format!("scratch/{index}-{branch}.bin")))
            .collect();
        let mut counts = vec![0_u64; branches];
        {
            let mut files: Vec<_> = paths
                .iter()
                .map(|p| File::create(p).map(BufWriter::new))
                .collect::<Result<_, _>>()?;
            let mut seen = 0_u64;
            let mut records =
                Records::new(path, self.layout.record_len, self.options.chunk_points)?;
            loop {
                let batch = records.next()?;
                if batch.is_empty() {
                    break;
                }
                for row in batch.chunks_exact(self.layout.record_len) {
                    let left = if extent[axis] == 0. {
                        seen < count / 2
                    } else {
                        position(row)[axis] < center[axis]
                    };
                    let branch = if self.options.explicit {
                        usize::from(!left)
                    } else if extent == [0.; 3] {
                        ((seen as u128 * 8 / count as u128) as usize).min(7)
                    } else {
                        let p = position(row);
                        usize::from(p[0] >= midpoint[0])
                            | (usize::from(p[1] >= midpoint[1]) << 1)
                            | (usize::from(p[2] >= midpoint[2]) << 2)
                    };
                    files[branch].write_all(row)?;
                    counts[branch] += 1;
                    seen += 1;
                }
            }
            for file in &mut files {
                file.flush()?;
            }
            if self.options.explicit && counts.contains(&0) {
                return Err(Error::Data("spatial split made no progress".into()));
            }
        }
        std::fs::remove_file(path)?;
        let mut children = Vec::new();
        for (branch, path) in paths.iter().enumerate() {
            if counts[branch] == 0 {
                std::fs::remove_file(path)?;
                continue;
            }
            let child_cell = if self.options.explicit {
                None
            } else {
                Some((
                    std::array::from_fn(|i| {
                        if branch & (1 << i) != 0 {
                            midpoint[i]
                        } else {
                            cell_lo[i]
                        }
                    }),
                    std::array::from_fn(|i| {
                        if branch & (1 << i) != 0 {
                            cell_hi[i]
                        } else {
                            midpoint[i]
                        }
                    }),
                ))
            };
            let mut child = self.build(path, center, depth + 1, child_cell)?;
            if !self.options.explicit {
                child["extras"] = json!({"implicitChildIndex":branch});
            }
            children.push(child);
        }
        let offsets = children
            .iter()
            .map(translation_offset)
            .collect::<Result<Vec<_>, _>>()?;
        let own_error = error + rounding;
        let error = enclose_children(&mut half, own_error, offsets.into_iter().zip(&children))?;
        node["geometricError"] = error.into();
        node["boundingVolume"]["box"] = box_json(0., half);
        // Move, rather than re-serialise, each finished subtree into its parent.
        node["children"] = Value::Array(children);
        Ok(node)
    }

    fn sample(&self, path: &Path, lo: [f64; 3], extent: [f64; 3]) -> Result<(Vec<u8>, f64), Error> {
        let grid = crate::point_sampling::VoxelGrid::new(lo, extent, self.options.max_points);
        let mut representatives = BTreeMap::new();
        let mut records = Records::new(path, self.layout.record_len, self.options.chunk_points)?;
        loop {
            let batch = records.next()?;
            if batch.is_empty() {
                break;
            }
            for row in batch.chunks_exact(self.layout.record_len) {
                let key = grid.key(position(row));
                representatives.entry(key).or_insert_with(|| row.to_vec());
            }
        }
        // First source record in each voxel, sorted by voxel key for stable output.
        Ok((
            representatives.into_values().flatten().collect(),
            grid.error_bound(),
        ))
    }
}

fn emit(
    rows: &[u8],
    center: [f64; 3],
    layout: &Layout,
    metadata_attributes: bool,
    path: &Path,
) -> Result<f64, Error> {
    let count = rows.len() / layout.record_len;
    if count > 16_777_217 {
        return Err(Error::Data("too many exact feature IDs in one tile".into()));
    }
    let mut glb = MetadataGlb::new("rusty-tiles native point cloud");
    let mut positions = Vec::with_capacity(count * 12);
    let mut ids = Vec::with_capacity(count * 4);
    let mut colors = Vec::new();
    let mut lo = [f32::INFINITY; 3];
    let mut hi = [f32::NEG_INFINITY; 3];
    let mut rounding: f64 = 0.;
    for (index, row) in rows.chunks_exact(layout.record_len).enumerate() {
        let p = position(row);
        let local = [p[0] - center[0], p[2] - center[2], -(p[1] - center[1])];
        let encoded = local.map(|v| v as f32);
        if !encoded.iter().all(|v| v.is_finite()) {
            return Err(Error::Data(
                "tile positions exceed finite float32 range".into(),
            ));
        }
        rounding = rounding.max(norm(sub(local, encoded.map(f64::from))));
        for i in 0..3 {
            lo[i] = lo[i].min(encoded[i]);
            hi[i] = hi[i].max(encoded[i]);
            positions.extend_from_slice(&encoded[i].to_le_bytes());
        }
        if count <= 65_536 {
            ids.extend_from_slice(&(index as u16).to_le_bytes());
            ids.extend_from_slice(&[0, 0]);
        } else {
            ids.extend_from_slice(&(index as f32).to_le_bytes());
        }
        if let Some(at) = layout.rgb {
            colors.extend_from_slice(&row[RAW + at..RAW + at + 6]);
            colors.extend_from_slice(&65535_u16.to_le_bytes());
        }
    }
    let view = glb.view(&positions);
    let position_accessor = glb.accessor(json!({"bufferView":view,"componentType":5126,"count":count,"type":"VEC3","min":lo,"max":hi}));
    drop(positions);
    let view = glb.view(&ids);
    if count <= 65_536 {
        glb.document["bufferViews"][view]["byteStride"] = 4.into();
    }
    let feature_accessor = glb.accessor(json!({"bufferView":view,"componentType":if count <= 65_536 {5123} else {5126},"count":count,"type":"SCALAR"}));
    drop(ids);
    let mut attributes = json!({"POSITION":position_accessor,"_FEATURE_ID_0":feature_accessor});
    if layout.rgb.is_some() {
        let view = glb.view(&colors);
        attributes["COLOR_0"] = glb.accessor(json!({"bufferView":view,"componentType":5123,"count":count,"type":"VEC4","normalized":true})).into();
    }
    let mut schema = serde_json::Map::new();
    let mut columns = serde_json::Map::new();
    let mut values = Vec::new();
    let mut property_attributes = BTreeMap::new();
    for dim in &layout.dimensions {
        values.clear();
        for row in rows.chunks_exact(layout.record_len) {
            dim.append(row, &mut values);
        }
        schema.insert(
            dim.name.clone(),
            json!({"type":"SCALAR","componentType":dim.kind.component()}),
        );
        columns.insert(dim.name.clone(), json!({"values":glb.view(&values)}));
        if metadata_attributes
            && matches!(
                dim.name.as_str(),
                "classification" | "intensity" | "return_number"
            )
        {
            let semantic = format!("_{}", dim.name.to_ascii_uppercase());
            let width = dim.kind.width();
            let padded: Vec<u8> = values
                .chunks_exact(width)
                .flat_map(|v| {
                    let mut bytes = [0; 4];
                    bytes[..width].copy_from_slice(v);
                    bytes
                })
                .collect();
            let view = glb.view(&padded);
            glb.document["bufferViews"][view]["byteStride"] = 4.into();
            let accessor = glb.accessor(json!({"bufferView":view,"componentType":if width == 1 {5121} else {5123},"count":count,"type":"SCALAR"}));
            attributes[&semantic] = accessor.into();
            property_attributes.insert(
                dim.name.clone(),
                crate::metadata::PropertyAttributeProperty {
                    attribute: semantic,
                    ..Default::default()
                },
            );
        }
    }
    glb.document["extensionsUsed"] = json!([
        "EXT_mesh_features",
        "EXT_structural_metadata",
        "KHR_materials_unlit"
    ]);
    let mut metadata: crate::metadata::StructuralMetadata = serde_json::from_value(json!({
        "schema":{"id":"rusty_tiles_point_cloud","classes":{"point":{"properties":schema}}},
        "propertyTables":[{"name":"points","class":"point","count":count,"properties":columns}]}))?;
    if metadata_attributes {
        metadata
            .property_attributes
            .push(crate::metadata::PropertyAttribute {
                class: "point".into(),
                properties: property_attributes,
                ..Default::default()
            });
    }
    metadata.attach(&mut glb.document)?;
    glb.document["materials"] = json!([{"extensions":{"KHR_materials_unlit":{}},"pbrMetallicRoughness":{"metallicFactor":0,"roughnessFactor":1}}]);
    glb.document["meshes"] = json!([{"primitives":[{"mode":0,"attributes":attributes,"material":0,
        "extensions":{"EXT_mesh_features":crate::metadata::MeshFeatures::attribute(count, 0, Some(0))}}]}]);
    if metadata_attributes {
        glb.document["meshes"][0]["primitives"][0]["extensions"]
            [crate::metadata::STRUCTURAL_METADATA] = json!({"propertyAttributes":[0]});
    }
    std::fs::write(path, glb.finish()?)?;
    Ok(rounding)
}

/// Tileset-level error; NaN (reported by the caller as a nonfinite extent)
/// when the root is malformed.
pub(super) fn tileset_error(root: &Value) -> f64 {
    match (
        box_half(&root["boundingVolume"]["box"]),
        root["geometricError"].as_f64(),
    ) {
        (Ok(half), Some(error)) => top_level_error(2. * norm(half), error, 1.),
        _ => f64::NAN,
    }
}
