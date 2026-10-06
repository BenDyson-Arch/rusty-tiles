//! Disk-backed spatial partitioning; only an input chunk or a tile's bounded
//! representatives/full-detail records are held in memory at a time.
use super::source::{position, Layout, RAW};
use super::{progress, PointCloudOptions};
use crate::{glb_write::MetadataGlb, Error};
use serde_json::{json, Value};
use std::{
    collections::BTreeMap,
    fs::File,
    io::{BufReader, BufWriter, Read, Write},
    path::Path,
};

fn norm(point: [f64; 3]) -> f64 {
    point[0].hypot(point[1]).hypot(point[2])
}

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
        let extent: [f64; 3] = std::array::from_fn(|i| hi[i] - lo[i]);
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
        let rounding = emit(&rows, center, self.layout, &self.output.join(&uri))?;
        self.max_rounding = self.max_rounding.max(rounding);
        drop(rows);
        let mut half = std::array::from_fn(|i| (extent[i] / 2. + rounding).max(1e-6));
        let delta: [f64; 3] = std::array::from_fn(|i| center[i] - parent_center[i]);
        let mut node = json!({"boundingVolume":{"box":box_values(half)},
            "transform":[1,0,0,0,0,1,0,0,0,0,1,0,delta[0],delta[1],delta[2],1],
            "geometricError":error + if leaf {0.} else {rounding},
            "refine":"REPLACE","content":{"uri":uri}});
        if leaf {
            std::fs::remove_file(path)?;
            self.leaf_points += count;
            progress("tiling", self.leaf_points, self.total_points);
            return Ok(node);
        }
        if depth >= 64 {
            return Err(Error::Data(
                "point partition exceeds 64 levels; inspect source extent or raise maxPoints"
                    .into(),
            ));
        }
        let axis = (0..3)
            .max_by(|a, b| extent[*a].total_cmp(&extent[*b]).then_with(|| b.cmp(a)))
            .unwrap();
        let paths = [
            self.output.join(format!("scratch/{index}-0.bin")),
            self.output.join(format!("scratch/{index}-1.bin")),
        ];
        {
            let mut files = [
                BufWriter::new(File::create(&paths[0])?),
                BufWriter::new(File::create(&paths[1])?),
            ];
            let mut counts = [0_u64; 2];
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
                    let branch = usize::from(!left);
                    files[branch].write_all(row)?;
                    counts[branch] += 1;
                    seen += 1;
                }
            }
            for file in &mut files {
                file.flush()?;
            }
            if counts.contains(&0) {
                return Err(Error::Data("spatial split made no progress".into()));
            }
        }
        std::fs::remove_file(path)?;
        let children = [
            self.build(&paths[0], center, depth + 1)?,
            self.build(&paths[1], center, depth + 1)?,
        ];
        for child in &children {
            for i in 0..3 {
                let offset = child["transform"][12 + i].as_f64().unwrap().abs();
                let child_half = child["boundingVolume"]["box"][3 + i * 4].as_f64().unwrap();
                half[i] = half[i].max(offset + child_half);
            }
            node["geometricError"] = node["geometricError"]
                .as_f64()
                .unwrap()
                .max(child["geometricError"].as_f64().unwrap())
                .into();
        }
        node["boundingVolume"]["box"] = json!(box_values(half));
        node["children"] = json!(children);
        Ok(node)
    }

    fn sample(&self, path: &Path, lo: [f64; 3], extent: [f64; 3]) -> Result<(Vec<u8>, f64), Error> {
        let budget = self.options.max_points;
        let mut side = ((budget as f64).cbrt().round() as usize).max(1);
        while side.checked_pow(3).is_none_or(|n| n > budget) {
            side -= 1;
        }
        let mut representatives = BTreeMap::new();
        let mut records = Records::new(path, self.layout.record_len, self.options.chunk_points)?;
        loop {
            let batch = records.next()?;
            if batch.is_empty() {
                break;
            }
            for row in batch.chunks_exact(self.layout.record_len) {
                let p = position(row);
                let cell: [usize; 3] = std::array::from_fn(|i| {
                    if extent[i] == 0. {
                        0
                    } else {
                        (((p[i] - lo[i]) / extent[i] * side as f64) as usize).min(side - 1)
                    }
                });
                let key = (cell[0] * side + cell[1]) * side + cell[2];
                representatives.entry(key).or_insert_with(|| row.to_vec());
            }
        }
        // First source record in each voxel, sorted by voxel key for stable output.
        Ok((
            representatives.into_values().flatten().collect(),
            norm(extent.map(|v| v / side as f64)),
        ))
    }
}

fn box_values(half: [f64; 3]) -> [f64; 12] {
    [
        0., 0., 0., half[0], 0., 0., 0., half[1], 0., 0., 0., half[2],
    ]
}

fn emit(rows: &[u8], center: [f64; 3], layout: &Layout, path: &Path) -> Result<f64, Error> {
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
        rounding = rounding.max(norm(std::array::from_fn(|i| local[i] - encoded[i] as f64)));
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
    }
    glb.document["extensionsUsed"] = json!([
        "EXT_mesh_features",
        "EXT_structural_metadata",
        "KHR_materials_unlit"
    ]);
    glb.document["extensions"] = json!({"EXT_structural_metadata":{
        "schema":{"id":"rusty_tiles_point_cloud","classes":{"point":{"properties":schema}}},
        "propertyTables":[{"name":"points","class":"point","count":count,"properties":columns}]}});
    glb.document["materials"] = json!([{"extensions":{"KHR_materials_unlit":{}},"pbrMetallicRoughness":{"metallicFactor":0,"roughnessFactor":1}}]);
    glb.document["meshes"] = json!([{"primitives":[{"mode":0,"attributes":attributes,"material":0,
        "extensions":{"EXT_mesh_features":{"featureIds":[{"featureCount":count,"attribute":0,"propertyTable":0}]}}}]}]);
    std::fs::write(path, glb.finish()?)?;
    Ok(rounding)
}

pub(super) fn tileset_error(root: &Value) -> f64 {
    let box_ = &root["boundingVolume"]["box"];
    (2. * norm([
        box_[3].as_f64().unwrap(),
        box_[7].as_f64().unwrap(),
        box_[11].as_f64().unwrap(),
    ]))
    .max(1.)
    .max(root["geometricError"].as_f64().unwrap())
}
