//! Disk-backed spatial partitioning; only an input chunk or a tile's bounded
//! representatives/full-detail records are held in memory at a time.
use super::source::{position, Layout, RAW};
use super::PointCloudOptions;
use super::{checkpoint, progress, ProducerIo};
use crate::runtime::Attempt;
use crate::{
    glb,
    metadata::MetadataGlb,
    tileset_node::{
        box_half, box_json, enclose_children, top_level_error, translation, translation_offset,
    },
    vec3::{norm_hypot as norm, sub},
    Error,
};
use rustc_hash::FxHashMap;
use serde_json::{json, Value};
use std::{
    collections::hash_map::Entry,
    fs::File,
    io::{BufReader, BufWriter, Read, Seek, SeekFrom, Write},
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

pub(super) struct Tree<'a, O: ProducerIo> {
    pub operations: &'a mut O,
    pub attempt: &'a Attempt,
    pub members: Vec<String>,
    pub layout: &'a Layout,
    pub options: &'a PointCloudOptions,
    pub output: &'a Path,
    pub tiles: usize,
    pub max_rounding: f64,
    pub total_points: u64,
    pub leaf_points: u64,
    /// The root's minimum, the origin of every sampling grid.
    pub anchor: [f64; 3],
    /// Output metres of one source coordinate step; coarser leaves keep float32.
    pub precision: f64,
    pub float32_tiles: usize,
}

impl<O: ProducerIo> Tree<'_, O> {
    pub fn build(
        &mut self,
        path: &Path,
        parent_center: [f64; 3],
        depth: usize,
        cell: Option<([f64; 3], [f64; 3])>,
        edge: Option<f64>,
    ) -> Result<Value, Error> {
        checkpoint(self.attempt)?;
        let mut records = Records::new(path, self.layout.record_len, self.options.chunk_points)?;
        let mut lo = [f64::INFINITY; 3];
        let mut hi = [f64::NEG_INFINITY; 3];
        let mut count = 0_u64;
        loop {
            checkpoint(self.attempt)?;
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
        if cell.is_none() {
            self.anchor = lo;
        }
        let index = self.tiles;
        self.tiles += 1;
        let uri = format!("t/{index}.glb");
        if leaf {
            let rows = std::fs::read(path)?;
            let (node, ..) = self.tile(&rows, &uri, center, parent_center, extent, None)?;
            std::fs::remove_file(path)?;
            self.leaf_points += count;
            progress(self.attempt, "tiling", self.leaf_points, self.total_points)?;
            return Ok(node);
        }
        // Implicit subtree addressing caps the octree at 31 levels.
        const DEPTH_LIMIT: usize = 31;
        if depth >= DEPTH_LIMIT {
            return Err(Error::Data(format!(
                "point partition exceeds {DEPTH_LIMIT} levels; inspect source extent or raise maxPoints (at most {})",
                super::MAX_POINTS_PER_TILE
            )));
        }
        let branches = 8;
        let (mut cell_lo, mut cell_hi) = cell.unwrap_or((lo, hi));
        let half_way =
            |lo: [f64; 3], hi: [f64; 3]| std::array::from_fn(|i| lo[i] + (hi[i] - lo[i]) / 2.);
        let mut midpoint: [f64; 3] = half_way(cell_lo, cell_hi);
        // While every point lies in one octant, that child would repeat this tile: halve
        // the cell toward it. Tiles record actual boxes, so addressing is unaffected.
        while extent != [0.; 3] && (0..3).all(|i| hi[i] < midpoint[i] || lo[i] >= midpoint[i]) {
            let before = (cell_lo, cell_hi);
            for i in 0..3 {
                *if lo[i] >= midpoint[i] {
                    &mut cell_lo[i]
                } else {
                    &mut cell_hi[i]
                } = midpoint[i];
            }
            if (cell_lo, cell_hi) == before {
                break;
            }
            midpoint = half_way(cell_lo, cell_hi);
        }
        let paths: Vec<_> = (0..branches)
            .map(|branch| self.output.join(format!("scratch/{index}-{branch}.bin")))
            .collect();
        let mut counts = vec![0_u64; branches];
        // One pass partitions the children and samples this tile.
        let mut sampler = Sampler::new(self.anchor, extent, edge, self.options.max_points);
        {
            let mut files: Vec<_> = paths
                .iter()
                .map(|p| File::create(p).map(BufWriter::new))
                .collect::<Result<_, _>>()?;
            let mut seen = 0_u64;
            let mut records =
                Records::new(path, self.layout.record_len, self.options.chunk_points)?;
            loop {
                checkpoint(self.attempt)?;
                let batch = records.next()?;
                if batch.is_empty() {
                    break;
                }
                for row in batch.chunks_exact(self.layout.record_len) {
                    sampler.insert(position(row), seen);
                    let branch = if extent == [0.; 3] {
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
        }
        let (rows, error, edge) = sampler.finish(path, self.layout.record_len, extent)?;
        std::fs::remove_file(path)?;
        let (mut node, mut half, rounding) =
            self.tile(&rows, &uri, center, parent_center, extent, Some(error))?;
        drop(rows);
        let mut children = Vec::new();
        for (branch, path) in paths.iter().enumerate() {
            if counts[branch] == 0 {
                std::fs::remove_file(path)?;
                continue;
            }
            let child_cell = (
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
            );
            let mut child = self.build(path, center, depth + 1, Some(child_cell), Some(edge))?;
            child["extras"] = json!({"implicitChildIndex":branch});
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

    /// Write one tile's content; a leaf has no sampling `error`. Returns its node, box
    /// half extents and position rounding.
    fn tile(
        &mut self,
        rows: &[u8],
        uri: &str,
        center: [f64; 3],
        parent_center: [f64; 3],
        extent: [f64; 3],
        error: Option<f64>,
    ) -> Result<(Value, [f64; 3], f64), Error> {
        let leaf = error.is_none();
        let (rounding, reach, float32) = emit(
            rows,
            center,
            self.layout,
            self.options,
            leaf.then_some(self.precision),
            &self.output.join(uri),
            self.attempt,
            self.operations,
        )?;
        self.max_rounding = self.max_rounding.max(rounding);
        self.float32_tiles += usize::from(float32);
        // `reach` bounds the decoded positions about the tile origin, so the own box
        // contains every decoded vertex (C2 decodedContentContainment, exact arithmetic).
        let half = std::array::from_fn(|i| (extent[i] / 2. + rounding).max(1e-6).max(reach[i]));
        let node = json!({"boundingVolume":{"box":box_json(0., half)},
            "transform":translation(sub(center, parent_center)),
            "geometricError":error.map_or(0., |e| e + rounding),
            "refine":"REPLACE","content":{"uri":uri}});
        Ok((node, half, rounding))
    }
}

/// Parent representatives: the first source record per occupied cubic voxel, in key
/// order, on the finest grid that fits the budget. Grids are anchored at the root's
/// minimum and a child's ladder starts at its parent's edge, which the child's subset
/// always fits: a child's voxels are never coarser than its parent's. Two chains of
/// edges `top / 2^j` and `top / (2^j √2)` run down to `budget` voxels along the longest
/// axis; halving is exact, so each grid nests in the coarser one. Each grid maps a
/// voxel to its first record ordinal and holds at most `budget + 1` voxels.
/// A grid's edge and each occupied voxel's first record ordinal.
type Grid = (f64, FxHashMap<[u64; 3], u64>);

struct Sampler {
    anchor: [f64; 3],
    budget: usize,
    chains: [Vec<Grid>; 2],
}

impl Sampler {
    fn new(anchor: [f64; 3], extent: [f64; 3], parent_edge: Option<f64>, budget: usize) -> Self {
        let longest = extent.into_iter().fold(0., f64::max);
        let side = (1..=budget).take_while(|s| s * s * s <= budget).count();
        // At most `side` root voxels per axis, including points on the upper bound.
        let top = parent_edge.unwrap_or(longest.max(f64::MIN_POSITIVE) / (side as f64 - 0.5));
        let chains = [top, top / std::f64::consts::SQRT_2].map(|mut edge| {
            let mut grids = vec![(edge, FxHashMap::default())];
            while longest > 0. && longest / edge < budget as f64 && edge / 2. > 0. {
                edge /= 2.;
                grids.push((edge, FxHashMap::default()));
            }
            grids
        });
        Self {
            anchor,
            budget,
            chains,
        }
    }

    fn insert(&mut self, p: [f64; 3], ordinal: u64) {
        for chain in &mut self.chains {
            // Finest first: once a voxel is occupied, so are its coarser ones.
            for (edge, voxels) in chain.iter_mut().rev() {
                let key = std::array::from_fn(|i| ((p[i] - self.anchor[i]) / *edge) as u64);
                match voxels.entry(key) {
                    Entry::Vacant(slot) => slot.insert(ordinal),
                    Entry::Occupied(_) => break,
                };
            }
            // Overflow trims the finer tail; the even chain's coarsest grid (the parent's,
            // or at most side³ root voxels) always fits.
            while chain.last().is_some_and(|(_, v)| v.len() > self.budget) {
                chain.pop();
            }
        }
    }

    /// The finest fitting grid's records, read by ordinal in key order, its cell diagonal
    /// (clamped to the extent on single-voxel axes) and its edge.
    fn finish(
        self,
        path: &Path,
        len: usize,
        extent: [f64; 3],
    ) -> Result<(Vec<u8>, f64, f64), Error> {
        let (edge, voxels) = (self.chains.into_iter().filter_map(|mut chain| chain.pop()))
            .min_by(|a, b| a.0.total_cmp(&b.0))
            .ok_or_else(|| Error::Data("point sample exceeds maxPoints".into()))?;
        let mut picks: Vec<_> = voxels.into_iter().collect();
        picks.sort_unstable();
        let mut file = BufReader::new(File::open(path)?);
        let mut rows = vec![0; picks.len() * len];
        for ((_, ordinal), row) in picks.iter().zip(rows.chunks_exact_mut(len)) {
            file.seek(SeekFrom::Start(ordinal * len as u64))?;
            file.read_exact(row)?;
        }
        let error = norm(std::array::from_fn(|i| edge.min(extent[i])));
        Ok((rows, error, edge))
    }
}

#[allow(clippy::too_many_arguments)]
fn emit(
    rows: &[u8],
    center: [f64; 3],
    layout: &Layout,
    options: &PointCloudOptions,
    leaf_precision: Option<f64>,
    path: &Path,
    attempt: &Attempt,
    operations: &mut impl ProducerIo,
) -> Result<(f64, [f64; 3], bool), Error> {
    let count = rows.len() / layout.record_len;
    // Admission caps tiles at MAX_POINTS_PER_TILE, so 16-bit feature IDs are exact.
    if count > super::MAX_POINTS_PER_TILE {
        return Err(Error::Data(
            "point tile exceeds its property-table budget".into(),
        ));
    }
    let mut glb = MetadataGlb::new("rusty-tiles native point cloud");
    // Tile-local glTF (Y-up) positions as KHR_mesh_quantization 16-bit codes, dequantized
    // by the content node's translation (the minimum) and scale (the extent).
    let local = |row| {
        let p = position(row);
        [p[0] - center[0], p[2] - center[2], -(p[1] - center[1])]
    };
    let (mut lo, mut hi) = ([f64::INFINITY; 3], [f64::NEG_INFINITY; 3]);
    for p in rows.chunks_exact(layout.record_len).map(local) {
        lo = std::array::from_fn(|i| lo[i].min(p[i]));
        hi = std::array::from_fn(|i| hi[i].max(p[i]));
    }
    let scale: [f64; 3] = std::array::from_fn(|i| if hi[i] > lo[i] { hi[i] - lo[i] } else { 1. });
    if !scale.iter().chain(&lo).all(|v| v.is_finite()) {
        return Err(Error::Data("tile positions exceed finite range".into()));
    }
    // A leaf keeps float32 when a 16-bit step would be coarser than the source precision.
    let float32 =
        leaf_precision.is_some_and(|p| (0..3).any(|i| hi[i] > lo[i] && scale[i] / 65535. > p));
    let mut positions = Vec::with_capacity(count * 12);
    let (mut low, mut high) = ([f32::INFINITY; 3], [f32::NEG_INFINITY; 3]);
    let mut rounding: f64 = 0.;
    for (index, p) in rows.chunks_exact(layout.record_len).map(local).enumerate() {
        if index % 4096 == 0 {
            checkpoint(attempt)?;
        }
        let decoded: [f64; 3] = if float32 {
            let value = p.map(|v| v as f32);
            low = std::array::from_fn(|i| low[i].min(value[i]));
            high = std::array::from_fn(|i| high[i].max(value[i]));
            positions.extend(value.into_iter().flat_map(f32::to_le_bytes));
            value.map(f64::from)
        } else {
            let code: [u16; 3] = std::array::from_fn(|i| {
                ((p[i] - lo[i]) / scale[i] * 65535.).round().min(65535.) as u16
            });
            positions.extend(code.into_iter().chain([0]).flat_map(u16::to_le_bytes));
            std::array::from_fn(|i| lo[i] + scale[i] * (f64::from(code[i]) / 65535.))
        };
        rounding = rounding.max(norm(sub(p, decoded)));
    }
    let view = glb.view(&positions);
    let (mut position, reach): (Value, [f64; 3]) = if float32 {
        if !low.iter().chain(&high).all(|v| v.is_finite()) {
            return Err(Error::Data(
                "tile positions exceed finite float32 range".into(),
            ));
        }
        let reach = std::array::from_fn(|i| f64::from(low[i].abs().max(high[i].abs())));
        (json!({"componentType":5126,"min":low,"max":high}), reach)
    } else {
        glb.document["bufferViews"][view]["byteStride"] = 8.into();
        glb.document["nodes"][0]["translation"] = json!(lo);
        glb.document["nodes"][0]["scale"] = json!(scale);
        glb::add_extension(&mut glb.document, "KHR_mesh_quantization", true)?;
        // Decoded positions lie in [lo, lo + scale] exactly; widen the rounded sum.
        let reach = std::array::from_fn(|i| {
            lo[i].abs().max((lo[i] + scale[i]).abs()) + 4. * f64::EPSILON * (lo[i].abs() + scale[i])
        });
        let high: [u16; 3] = std::array::from_fn(|i| if hi[i] > lo[i] { 65535 } else { 0 });
        (
            json!({"componentType":5123,"normalized":true,"min":[0,0,0],"max":high}),
            reach,
        )
    };
    position["bufferView"] = view.into();
    position["count"] = count.into();
    position["type"] = "VEC3".into();
    let mut attributes = json!({ "POSITION": glb.accessor(position) });
    if let Some(at) = layout.rgb {
        // One display colour: 8-bit, round(v / 257); exact 16-bit values are metadata.
        let mut colors = Vec::with_capacity(count * 4);
        for row in rows.chunks_exact(layout.record_len) {
            for c in row[RAW + at..RAW + at + 6].as_chunks::<2>().0 {
                colors.push(((u32::from(u16::from_le_bytes(*c)) + 128) / 257) as u8);
            }
            colors.push(0);
        }
        let view = glb.view(&colors);
        glb.document["bufferViews"][view]["byteStride"] = 4.into();
        attributes["COLOR_0"] = glb.accessor(json!({"bufferView":view,"componentType":5121,"count":count,"type":"VEC3","normalized":true})).into();
    }
    if options.metadata {
        let ids: Vec<u8> = (0..count as u32).flat_map(u32::to_le_bytes).collect();
        let view = glb.view(&ids);
        glb.document["bufferViews"][view]["byteStride"] = 4.into();
        attributes["_FEATURE_ID_0"] = glb
            .accessor(json!({"bufferView":view,"componentType":5123,"count":count,"type":"SCALAR"}))
            .into();
    }
    let mut schema = serde_json::Map::new();
    let mut columns = serde_json::Map::new();
    let mut values = Vec::new();
    let mut property_attributes = serde_json::Map::new();
    let mut attribute_schema = serde_json::Map::new();
    for dim in &layout.dimensions {
        let attribute = options.metadata_attributes
            && matches!(
                dim.name.as_str(),
                "classification" | "intensity" | "return_number"
            );
        if !options.metadata && !attribute {
            continue;
        }
        checkpoint(attempt)?;
        values.clear();
        for (index, row) in rows.chunks_exact(layout.record_len).enumerate() {
            if index % 4096 == 0 {
                checkpoint(attempt)?;
            }
            dim.append(row, &mut values);
        }
        if options.metadata {
            schema.insert(dim.name.clone(), dim.schema());
            columns.insert(dim.name.clone(), json!({"values":glb.view(&values)}));
        }
        if attribute {
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
            // Cesium emits shader fields for both tables and attributes.
            // Give attribute properties distinct IDs to avoid duplicate fields
            // when styling uses the table's original LAS property names.
            let property = format!("vertex_{}", dim.name);
            if layout.dimensions.iter().any(|d| d.name == property) {
                return Err(Error::Data(format!(
                    "LAS dimension {property:?} conflicts with metadata attribute property"
                )));
            }
            attribute_schema.insert(
                property.clone(),
                json!({"type":"SCALAR","componentType":dim.kind.component()}),
            );
            property_attributes.insert(property, json!({ "attribute": semantic }));
        }
    }
    glb::add_extension(&mut glb.document, "KHR_materials_unlit", false)?;
    let mut extensions = json!({});
    let mut metadata = json!({"schema":{"id":"rusty_tiles_point_cloud","classes":{}}});
    if options.metadata {
        metadata["schema"]["classes"]["point"] = json!({ "properties": schema });
        metadata["propertyTables"] =
            json!([{"name":"points","class":"point","count":count,"properties":columns}]);
        extensions[crate::metadata::MESH_FEATURES] =
            serde_json::to_value(crate::metadata::MeshFeatures::attribute(count, 0, Some(0)))?;
        glb::add_extension(&mut glb.document, crate::metadata::MESH_FEATURES, false)?;
    }
    if options.metadata_attributes {
        metadata["schema"]["classes"]["pointAttribute"] = json!({ "properties": attribute_schema });
        metadata["propertyAttributes"] =
            json!([{"class":"pointAttribute","properties":property_attributes}]);
        extensions[crate::metadata::STRUCTURAL_METADATA] = json!({"propertyAttributes":[0]});
    }
    if options.metadata || options.metadata_attributes {
        serde_json::from_value::<crate::metadata::StructuralMetadata>(metadata)?
            .attach(&mut glb.document)?;
    }
    glb.document["materials"] = json!([{"extensions":{"KHR_materials_unlit":{}},"pbrMetallicRoughness":{"metallicFactor":0,"roughnessFactor":1}}]);
    glb.document["meshes"] =
        json!([{"primitives":[{"mode":0,"attributes":attributes,"material":0}]}]);
    if extensions != json!({}) {
        glb.document["meshes"][0]["primitives"][0]["extensions"] = extensions;
    }
    checkpoint(attempt)?;
    operations.member(path, &glb.finish()?)?;
    // glTF stores (x, z, -y): box axis 1 reads glTF axis 2 and box axis 2 reads axis 1.
    Ok((rounding, [reach[0], reach[2], reach[1]], float32))
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
