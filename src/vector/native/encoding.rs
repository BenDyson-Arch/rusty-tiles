//! Draft vector GLBs, typed metadata, quantization and direct Rust meshopt.
use super::*;
use crate::glb::{self, MetadataGlb};
use std::io::Write;

#[derive(Default)]
struct Batch {
    points: Vec<Point>,
    ids: Vec<usize>,
    indices: Vec<u32>,
    loops: Vec<u32>,
    triangles: Vec<u32>,
    loop_offsets: Vec<u32>,
}
struct Encoded {
    bytes: Vec<u8>,
    vertices: usize,
    primitives: usize,
    rounding: f64,
    quantization: f64,
    before_bytes: usize,
}
pub(super) struct Candidate {
    pub node: Option<Value>,
    pub reports: Vec<Value>,
    pub reason: Option<&'static str>,
    pub worker: usize,
}
fn view(glb: &mut MetadataGlb, bytes: &[u8]) -> usize {
    glb.view(if bytes.is_empty() { &[0] } else { bytes })
}
fn u32_accessor(glb: &mut MetadataGlb, values: &[u32]) -> usize {
    let bytes: Vec<_> = values.iter().flat_map(|v| v.to_le_bytes()).collect();
    let v = view(glb, &bytes);
    glb.accessor(json!({"bufferView":v,"componentType":5125,"count":values.len(),"type":"SCALAR"}))
}
fn metadata(
    glb: &mut MetadataGlb,
    items: &[&Feature],
    schemas: &BTreeMap<String, String>,
) -> Result<(), Error> {
    let keys: BTreeSet<_> = items.iter().flat_map(|f| f.properties.keys()).collect();
    let mut schema = BTreeMap::new();
    let mut columns = BTreeMap::new();
    for key in keys {
        let mut values: Vec<_> = items
            .iter()
            .map(|f| f.properties.get(key).cloned().unwrap_or(Value::Null))
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
                    return Err(data(format!(
                        "property {key:?} cannot represent large integers as float64 without loss"
                    )));
                }
            }
            for value in &mut values {
                if !value.is_null() {
                    *value = json!(value
                        .as_f64()
                        .ok_or_else(|| data("incompatible numeric property"))?);
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
                    return Err(data("reserved missing-value sentinel occurs in source"));
                }
                json!(sentinel)
            } else if expected == Some("boolean") {
                return Err(data(format!(
                    "nullable boolean property {key:?} requires an explicit schema"
                )));
            } else if expected == Some("string") || present.iter().all(|v| v.is_string()) {
                let mut sentinel = "__RUSTY_TILES_MISSING__".to_string();
                while present.iter().any(|v| v.as_str() == Some(&sentinel)) {
                    sentinel.push('_');
                }
                json!(sentinel)
            } else {
                return Err(data(format!(
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
                            data(format!(
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
                    .ok_or_else(|| data("nonfinite scalar property"))?;
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
                    .map_err(|_| data("metadata strings exceed UINT32 range"))?;
                offsets.extend(offset.to_le_bytes());
            }
            (
                json!({"type":"STRING"}),
                json!({"values":view(glb,&bytes),"stringOffsets":view(glb,&offsets),"stringOffsetType":"UINT32"}),
            )
        } else {
            return Err(data(format!(
                "unsupported/null/mixed property {key:?}; retain source and normalize explicitly"
            )));
        };
        if let Some(no_data) = no_data {
            definition["noData"] = no_data;
        }
        schema.insert(key.clone(), definition);
        columns.insert(key.clone(), column);
    }
    let aggregate = items
        .iter()
        .all(|f| !f.properties.contains_key("_source_id"));
    let class = if aggregate {
        "pointAggregate"
    } else {
        "feature"
    };
    let name = if aggregate {
        "pointAggregates"
    } else {
        "features"
    };
    glb.document["extensions"] = json!({"EXT_structural_metadata":{"schema":{"id":"rusty_tiles_vector","classes":{class:{"properties":schema}}},
        "propertyTables":[{"name":name,"class":class,"count":items.len(),"properties":columns}]}});
    Ok(())
}

fn emit(
    items: &[&Feature],
    center: Point,
    repair: bool,
    schemas: &BTreeMap<String, String>,
    fill_only: bool,
    options: &VectorOptions,
    reports: &mut Vec<Value>,
) -> Result<Encoded, Error> {
    if items.len() > 16777217 {
        return Err(data("too many exact feature IDs in one tile"));
    }
    let mut glb = MetadataGlb::new("rusty-tiles native vector");
    glb.document["extensionsUsed"] = json!(["EXT_mesh_features", "EXT_structural_metadata"]);
    glb.document["meshes"] = json!([{"primitives":[]}]);
    metadata(&mut glb, items, schemas)?;
    let mut batches: BTreeMap<u32, Batch> = BTreeMap::new();
    let mut line_restart = false;
    let mut all_positions = Vec::new();
    for (fid, feature) in items.iter().enumerate() {
        let mut parts = Vec::new();
        match &feature.geometry {
            Geometry::Point(p) => parts.push((0, vec![*p], None)),
            Geometry::MultiPoint(p) => parts.push((0, p.clone(), None)),
            Geometry::LineString(p) => parts.push((3, p.clone(), None)),
            Geometry::MultiLineString(p) => parts.extend(p.iter().map(|p| (3, p.clone(), None))),
            geometry => {
                let polygons = match geometry {
                    Geometry::Polygon(r) => vec![r],
                    Geometry::MultiPolygon(p) => p.iter().collect(),
                    _ => unreachable!(),
                };
                for rings in polygons {
                    let local: Vec<Vec<_>> = rings
                        .iter()
                        .map(|r| r.iter().map(|p| sub(*p, center)).collect())
                        .collect();
                    let polygon = geometry::polygon(&local, repair)?;
                    let mut report = polygon.report.clone();
                    report["sourceId"] = feature.source_id().clone();
                    reports.push(report);
                    parts.push((
                        4,
                        polygon.positions.iter().map(|p| add(*p, center)).collect(),
                        Some(polygon),
                    ));
                }
            }
        }
        for (mode, positions, polygon) in parts {
            let points: Vec<_> = positions.iter().map(|p| sub(*p, center)).collect();
            let batch = batches.entry(mode).or_default();
            let base = u32::try_from(batch.points.len())
                .map_err(|_| data("tile vertex index overflow"))?;
            if mode == 3 && !batch.indices.is_empty() {
                batch.indices.push(u32::MAX);
                line_restart = true;
            }
            if let Some(polygon) = polygon {
                if !fill_only {
                    if !batch.loops.is_empty() {
                        batch.loops.push(u32::MAX);
                    }
                    batch.triangles.extend(
                        polygon
                            .triangle_offsets
                            .iter()
                            .map(|offset| batch.indices.len() as u32 + offset),
                    );
                    batch.loop_offsets.extend(
                        polygon
                            .loop_offsets
                            .iter()
                            .map(|offset| batch.loops.len() as u32 + offset),
                    );
                    batch.loops.extend(polygon.loops.iter().map(|i| {
                        if *i == u32::MAX {
                            *i
                        } else {
                            base + i
                        }
                    }));
                }
                batch
                    .indices
                    .extend(polygon.indices.iter().map(|i| base + i));
            } else {
                batch
                    .indices
                    .extend((0..points.len() as u32).map(|i| base + i));
            }
            batch.ids.extend(std::iter::repeat_n(fid, points.len()));
            batch.points.extend_from_slice(&points);
            all_positions.extend(points);
        }
    }
    if line_restart {
        glb::add_extension(&mut glb.document, "KHR_mesh_primitive_restart", true)?;
    }
    let mut position_accessors = Vec::new();
    for (mode, batch) in &batches {
        let mut ext = json!({"EXT_mesh_features":{"featureIds":[{"featureCount":items.len(),"attribute":0,"propertyTable":0}]}});
        if *mode == 4 && !fill_only {
            ext["EXT_mesh_polygon"] = json!({"count":batch.triangles.len(),"indicesOffsets":u32_accessor(&mut glb,&batch.triangles),
                "loopIndices":u32_accessor(&mut glb,&batch.loops),"loopIndicesOffsets":u32_accessor(&mut glb,&batch.loop_offsets)});
            glb::add_extension(&mut glb.document, "EXT_mesh_polygon", false)?;
        }
        let values: Vec<Point> = batch
            .points
            .iter()
            .map(|p| p.map(|v| v as f32 as f64))
            .collect();
        if !values.iter().flatten().all(|v| v.is_finite()) {
            return Err(data("positions exceed float32 range"));
        }
        let bytes: Vec<_> = values
            .iter()
            .flat_map(|p| p.iter().flat_map(|v| (*v as f32).to_le_bytes()))
            .collect();
        let position_view = view(&mut glb, &bytes);
        let (lo, hi) = bounds(values.iter())?;
        let position=glb.accessor(json!({"bufferView":position_view,"componentType":5126,"count":values.len(),"type":"VEC3","min":lo,"max":hi}));
        position_accessors.push((position, position_view, values));
        let mut ids = Vec::new();
        for id in &batch.ids {
            if items.len() <= 65536 {
                ids.extend((*id as u16).to_le_bytes());
                ids.extend(0u16.to_le_bytes());
            } else {
                ids.extend((*id as f32).to_le_bytes());
            }
        }
        let id_view = view(&mut glb, &ids);
        if items.len() <= 65536 {
            glb.document["bufferViews"][id_view]["byteStride"] = json!(4);
        }
        let id_accessor=glb.accessor(json!({"bufferView":id_view,"componentType":if items.len()<=65536 {5123}else{5126},"count":batch.ids.len(),"type":"SCALAR"}));
        let index = u32_accessor(&mut glb, &batch.indices);
        glb.document["meshes"][0]["primitives"].as_array_mut().unwrap().push(json!({"mode":mode,"attributes":{"POSITION":position,"_FEATURE_ID_0":id_accessor},"indices":index,"extensions":ext}));
    }
    if fill_only {
        glb::add_extension(&mut glb.document, "KHR_materials_unlit", false)?;
        glb.document["materials"] = json!([{"doubleSided":true,"extensions":{"KHR_materials_unlit":{}},"pbrMetallicRoughness":{"baseColorFactor":[1,1,1,1],"metallicFactor":0,"roughnessFactor":1}}]);
        for primitive in glb.document["meshes"][0]["primitives"]
            .as_array_mut()
            .unwrap()
        {
            primitive["material"] = json!(0);
        }
    }
    let rounding = all_positions
        .iter()
        .map(|p| norm(sub(*p, p.map(|v| v as f32 as f64))))
        .fold(0., f64::max);
    let mut quantization: f64 = 0.;
    // Uncompressed, unquantized GLB size: measured, not serialised.
    let before_bytes = glb.encoded_len()?;
    if options.quantize {
        let (lo, hi) = bounds(position_accessors.iter().flat_map(|(_, _, p)| p.iter()))?;
        let extent = sub(hi, lo);
        let scale = extent.map(|v| if v > 0. { v } else { 1. });
        for (accessor, view, points) in position_accessors {
            let mut bytes = Vec::new();
            let mut low = [u16::MAX; 3];
            let mut high = [0u16; 3];
            for p in points {
                let q: [u16; 3] = std::array::from_fn(|i| {
                    ((p[i] - lo[i]) / scale[i] * 65535.)
                        .round_ties_even()
                        .clamp(0., 65535.) as u16
                });
                let decoded: Point =
                    std::array::from_fn(|i| q[i] as f64 / 65535. * scale[i] + lo[i]);
                quantization = quantization.max(norm(sub(decoded, p)));
                for i in 0..3 {
                    low[i] = low[i].min(q[i]);
                    high[i] = high[i].max(q[i]);
                    bytes.extend(q[i].to_le_bytes());
                }
                bytes.extend(0u16.to_le_bytes());
            }
            glb.replace_view(view, &bytes);
            glb.document["bufferViews"][view]["byteStride"] = json!(8);
            let ac = &mut glb.document["accessors"][accessor];
            ac["componentType"] = json!(5123);
            ac["normalized"] = json!(true);
            ac["min"] = json!(low);
            ac["max"] = json!(high);
        }
        glb.compact_views();
        glb.document["nodes"][0]["translation"] = json!(lo);
        glb.document["nodes"][0]["scale"] = json!(scale);
        glb::add_extension(&mut glb.document, "KHR_mesh_quantization", true)?;
        quantization = quantization.max(norm(mul(extent, 1. / 131070.)));
        quantization += norm(sub(lo, lo.map(|v| v as f32 as f64)))
            + norm(sub(scale, scale.map(|v| v as f32 as f64)))
            + norm(scale) * 2f64.powi(-24);
    }
    // One serialisation: meshopt rewrites the in-memory document and binary.
    let bytes = if options.meshopt {
        let (mut document, binary) = glb.into_parts();
        let packed = crate::vector_encoding::compress_document(&mut document, &binary)?;
        glb::encode_glb(&document, &packed)?
    } else {
        glb.finish()?
    };
    Ok(Encoded {
        bytes,
        vertices: all_positions.len(),
        primitives: batches.len(),
        rounding,
        quantization,
        before_bytes,
    })
}

pub(super) fn encode(
    spool: &Path,
    prefix: &str,
    center: Point,
    level: u32,
    max_features: usize,
    repair: bool,
    options: &VectorOptions,
    schemas: &BTreeMap<String, String>,
    output: &Path,
) -> Result<Candidate, Error> {
    let db =
        rusqlite::Connection::open_with_flags(spool, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)
            .map_err(sql)?;
    db.execute_batch("PRAGMA cache_size=-32768; PRAGMA temp_store=FILE;")
        .map_err(sql)?;
    let end = format!("{prefix}~");
    let cap = if level > 0 {
        options.max_parent_features
    } else {
        max_features
    };
    let count: usize = db
        .query_row(
            "SELECT COUNT(*) FROM (SELECT 1 FROM features WHERE path>=?1 AND path<?2 LIMIT ?3)",
            rusqlite::params![
                prefix,
                end,
                i64::try_from(cap.saturating_add(1)).unwrap_or(i64::MAX)
            ],
            |r| Ok(r.get::<_, i64>(0)? as usize),
        )
        .map_err(sql)?;
    let mut result = Candidate {
        node: None,
        reports: Vec::new(),
        reason: None,
        worker: rayon::current_thread_index().unwrap_or(0),
    };
    let tolerance = if level > 0 {
        options.lod.tolerance_metres * 2f64.powi(level as i32 - 1)
    } else {
        0.
    };
    if !tolerance.is_finite() {
        return Err(data("vector LOD tolerance overflows; reduce lodTolerance"));
    }
    let mut aggregation = None;
    let mut items = Vec::new();
    let mut aggregation_reason = None;
    if level > 0 && options.aggregate_points {
        match aggregation::collect(&db, prefix, tolerance, options)? {
            aggregation::Outcome::Ready(value) => {
                aggregation = Some(value.summary);
                items = value.features;
            }
            aggregation::Outcome::Rejected(reason) => aggregation_reason = Some(reason),
            aggregation::Outcome::Unchanged => {}
        }
    }
    if aggregation.is_none() && count > cap {
        result.reason = Some(aggregation_reason.unwrap_or(if level > 0 {
            "parentFeatures"
        } else {
            "features"
        }));
        return Ok(result);
    }
    let mut vertices = 0;
    let mut estimate = 0;
    // The requested tolerance also promotes coincident aggregates to their
    // original identities on near refinement, even when spatial error is zero.
    let mut error: f64 = if aggregation.is_some() { tolerance } else { 0. };
    let mut stmt = db
        .prepare("SELECT data FROM features WHERE path>=?1 AND path<?2 ORDER BY id")
        .map_err(sql)?;
    let mut rows = stmt.query(rusqlite::params![prefix, end]).map_err(sql)?;
    let mut lock_query = db
        .prepare_cached("SELECT shared FROM vertices WHERE x=?1 AND y=?2 AND z=?3")
        .map_err(sql)?;
    while aggregation.is_none() {
        let Some(row) = rows.next().map_err(sql)? else {
            break;
        };
        // Load one original at a time; retain only budgeted simplified candidates.
        let feature: Feature = serde_json::from_str(&row.get::<_, String>(0).map_err(sql)?)?;
        let mut locked = BTreeSet::new();
        for p in feature.geometry.points() {
            if feature.surface_fragment
                || lock_query
                    .query_row(rusqlite::params![p[0], p[1], p[2]], |r| r.get::<_, i32>(0))
                    .map_err(sql)?
                    != 0
            {
                locked.insert(key(*p));
            }
        }
        let feature = if level > 0 {
            let (feature, e) = geometry::simplify(
                &feature,
                tolerance,
                &locked,
                &mut result.reports,
                options.parent_repair,
            )?;
            error = error.max(e);
            feature
        } else {
            feature
        };
        vertices += feature.geometry.size();
        estimate += feature.estimate() - if level > 0 { 2048 } else { 0 };
        if vertices > options.max_vertices {
            result.reason = Some("vertices");
            return Ok(result);
        }
        if estimate > options.max_bytes.saturating_mul(2) {
            result.reason = Some("estimatedBytes");
            return Ok(result);
        }
        items.push(feature);
    }
    let mut fills = Vec::new();
    let mut vectors = Vec::new();
    for feature in items {
        if feature.surface_fragment {
            let boundaries: Vec<_> = feature
                .triangle_boundaries
                .iter()
                .flatten()
                .cloned()
                .collect();
            if !boundaries.is_empty() {
                let mut boundary = feature.clone();
                boundary.geometry = Geometry::MultiLineString(boundaries);
                vectors.push(boundary);
            }
            fills.push(feature);
        } else {
            vectors.push(feature);
        }
    }
    let mut contents = Vec::new();
    let mut vertex_count = 0;
    let mut primitives = 0;
    let mut byte_count = 0;
    let mut before_bytes = 0;
    let mut rounding: f64 = 0.;
    let mut quantization: f64 = 0.;
    for (features, fill) in [(&fills, true), (&vectors, false)] {
        if features.is_empty() {
            continue;
        }
        let mut reports = Vec::new();
        let aggregate_schemas = BTreeMap::new();
        let encoded = emit(
            &features.iter().collect::<Vec<_>>(),
            center,
            repair,
            if aggregation.is_some() {
                &aggregate_schemas
            } else {
                schemas
            },
            fill,
            options,
            &mut reports,
        )?;
        if level == 0 {
            result.reports.extend(reports);
        }
        primitives += encoded.primitives;
        vertex_count += encoded.vertices;
        before_bytes += encoded.before_bytes;
        rounding = rounding.max(encoded.rounding);
        quantization = quantization.max(encoded.quantization);
        let mut bytes = encoded.bytes;
        let suffix = if fill { "b3dm" } else { "glb" };
        if fill {
            let mut table = b"{\"BATCH_LENGTH\":0}".to_vec();
            table.resize((28 + table.len()).next_multiple_of(8) - 28, b' ');
            before_bytes += 28 + table.len();
            let mut wrapped = b"b3dm".to_vec();
            for n in [
                1,
                u32::try_from(28 + table.len() + bytes.len())
                    .map_err(|_| data("b3dm exceeds format size"))?,
                table.len() as u32,
                0,
                0,
                0,
            ] {
                wrapped.extend(n.to_le_bytes());
            }
            wrapped.extend(table);
            wrapped.extend(bytes);
            bytes = wrapped;
        }
        let uri = format!("t/{}.{}", hash(&bytes), suffix);
        byte_count += bytes.len();
        let mut publication = tempfile::NamedTempFile::new_in(output.join("t"))?;
        publication.write_all(&bytes)?;
        match publication.persist_noclobber(output.join(&uri)) {
            Ok(_) => {}
            Err(error) if error.error.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(error) => return Err(error.error.into()),
        }
        let mut content = json!({"uri":uri});
        if !fill {
            content["extensions"] = json!({"3DTILES_content_gltf_vector":{"vector":true}});
        }
        contents.push(content);
    }
    if vertex_count > options.max_vertices || byte_count > options.max_bytes {
        result.reason = Some(if vertex_count > options.max_vertices {
            "vertices"
        } else {
            "bytes"
        });
        return Ok(result);
    }
    let mut node = json!({"extras":{"featureFragments":fills.len()+vectors.iter().filter(|f|!f.surface_fragment).count(),"vertices":vertex_count,"primitives":primitives,"encodedBytes":byte_count,
        "geometryErrorMetres":error,"positionRoundingMetres":rounding,"quantizationErrorMetres":quantization,"uncompressedBytes":before_bytes,"toleranceMetres":tolerance},
        "geometricError":if level>0 || options.quantize {error+rounding+quantization}else{0.}});
    if let Some(summary) = aggregation {
        node["extras"]["pointAggregation"] = summary.clone();
        let mut report = summary;
        report["substitution"] = json!("pointAggregates");
        report["buildPrefix"] = json!(prefix);
        report["toleranceMetres"] = json!(tolerance);
        result.reports.push(report);
    }
    if contents.len() == 1 {
        node["content"] = contents.remove(0);
    } else {
        node["contents"] = json!(contents);
    }
    result.node = Some(node);
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn float64_metadata_checks_original_signed_and_unsigned_integer_bounds() {
        let feature = |value| Feature {
            properties: BTreeMap::from([
                ("_source_id".into(), json!("0")),
                ("value".into(), value),
            ]),
            geometry: Geometry::Point([0.; 3]),
            surface_fragment: false,
            triangle_boundaries: Vec::new(),
            fragment_path: String::new(),
        };
        for schemas in [
            BTreeMap::new(),
            BTreeMap::from([("value".into(), "real".into())]),
        ] {
            let real = feature(json!(1.5));
            for integer in [
                json!((1i64 << 53) + 1),
                json!(-((1i64 << 53) + 1)),
                json!(i64::MIN),
                json!(i64::MAX),
                json!(u64::MAX),
            ] {
                let source = feature(integer);
                let mut glb = MetadataGlb::new("test");
                let error = metadata(&mut glb, &[&source, &real], &schemas).unwrap_err();
                assert!(error
                    .to_string()
                    .contains("large integers as float64 without loss"));
            }
            for integer in [
                -(1i64 << 53),
                -(1i64 << 53) + 1,
                (1i64 << 53) - 1,
                1i64 << 53,
            ] {
                let source = feature(json!(integer));
                let mut glb = MetadataGlb::new("test");
                metadata(&mut glb, &[&source, &real], &schemas).unwrap();
                let column = &glb.document["extensions"]["EXT_structural_metadata"]
                    ["propertyTables"][0]["properties"]["value"];
                let view =
                    &glb.document["bufferViews"][column["values"].as_u64().unwrap() as usize];
                let offset = view["byteOffset"].as_u64().unwrap() as usize;
                let bytes = glb.finish().unwrap();
                let json_length = u32::from_le_bytes(bytes[12..16].try_into().unwrap()) as usize;
                let start = 28 + json_length + offset;
                let decoded = f64::from_le_bytes(bytes[start..start + 8].try_into().unwrap());
                assert_eq!(decoded, integer as f64);
                assert_eq!(decoded as i64, integer);
            }
        }
    }
    #[test]
    fn parent_budget_stops_before_decoding_the_next_original() {
        for (name, label, max_vertices, expected) in [
            ("vertices", "", 4, "vertices"),
            ("bytes", "x", 64, "estimatedBytes"),
        ] {
            let root = tempfile::tempdir().unwrap();
            let spool = root.path().join(format!("{name}.sqlite"));
            let db = rusqlite::Connection::open(&spool).unwrap();
            db.execute_batch("CREATE TABLE features(id INTEGER PRIMARY KEY,path TEXT,data TEXT); CREATE TABLE vertices(x REAL,y REAL,z REAL,shared INTEGER);").unwrap();
            let points: Vec<_> = (0..5).map(|i| [i as f64, 0., 0.]).collect();
            let feature = json!({"properties":{"_source_id":"1","name":label.repeat(10000)},"geometry":{"type":"LineString","coordinates":points}});
            db.execute(
                "INSERT INTO features VALUES(1,'',?1)",
                [feature.to_string()],
            )
            .unwrap();
            // This row would fail JSON decoding if the guard retained/loading
            // loop continued past the first candidate's budget failure.
            db.execute(
                "INSERT INTO features VALUES(2,'','deliberately invalid JSON')",
                [],
            )
            .unwrap();
            for p in points {
                db.execute(
                    "INSERT INTO vertices VALUES(?1,?2,?3,1)",
                    rusqlite::params![p[0], p[1], p[2]],
                )
                .unwrap();
            }
            drop(db);
            let options = VectorOptions {
                max_vertices,
                max_bytes: 4096,
                ..Default::default()
            };
            let candidate = encode(
                &spool,
                "",
                [0.; 3],
                1,
                1,
                false,
                &options,
                &BTreeMap::new(),
                root.path(),
            )
            .unwrap();
            assert!(candidate.node.is_none());
            assert_eq!(candidate.reason, Some(expected));
        }
    }
}
