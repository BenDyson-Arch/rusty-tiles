//! Streaming OGR features; explicit axes, heights, filters and exact typed fields.
use super::super::source_fields;
use super::*;
use crate::geospatial::{
    self,
    native::{c_str, Dataset},
    Crs, EcefTransform, QuietErrors, StrictTransform,
};
use std::{ffi::c_void, ptr::NonNull};

struct Row(NonNull<c_void>);
impl Drop for Row {
    fn drop(&mut self) {
        // SAFETY: The feature is uniquely owned and destroyed exactly once.
        unsafe {
            gdal_sys::OGR_F_Destroy(self.0.as_ptr());
        }
    }
}
pub(super) struct Reader {
    dataset: Option<Dataset<'static>>,
    layers: Vec<gdal_sys::OGRLayerH>,
    pub driver: String,
    pub schemas: BTreeMap<String, String>,
    pub layer_reports: Vec<Value>,
    pub frame: Option<Frame>,
    pub without_geometry: usize,
}
impl Reader {
    pub fn new(
        input: &Path,
        options: &VectorOptions,
        frame: Option<Frame>,
        _scratch: &Path,
    ) -> Result<Self, Error> {
        // Native GeoJSON members are retained only for GeoJSON ingestion.
        let geojson = input
            .extension()
            .is_some_and(|s| s.eq_ignore_ascii_case("geojson") || s.eq_ignore_ascii_case("json"));
        let dataset = Dataset::open_vector(
            input,
            if geojson { &[c"NATIVE_DATA=YES"] } else { &[] },
            "OGR cannot open vector input",
        )?;
        let raw = dataset.raw();
        let _errors = QuietErrors::new();
        // SAFETY: The dataset owns its handle on this thread. Layers borrowed
        // below remain live until the Reader's dataset is closed.
        unsafe {
            let driver = geospatial::string(gdal_sys::GDALGetDriverShortName(
                gdal_sys::GDALGetDatasetDriver(raw),
            ));
            let layers: Vec<_> = (0..gdal_sys::GDALDatasetGetLayerCount(raw))
                .map(|i| gdal_sys::GDALDatasetGetLayer(raw, i))
                .filter(|layer| {
                    !layer.is_null()
                        && gdal_sys::OGR_GT_Flatten(gdal_sys::OGR_L_GetGeomType(*layer))
                            != gdal_sys::OGRwkbGeometryType::wkbNone
                })
                .collect();
            let names: BTreeSet<_> = layers
                .iter()
                .map(|l| geospatial::string(gdal_sys::OGR_L_GetName(*l)))
                .collect();
            let requested: BTreeSet<_> = options.layers.iter().cloned().collect();
            if requested.len() != options.layers.len() {
                return Err(data("duplicate layer selection"));
            }
            if !requested.is_subset(&names) {
                return Err(data(format!(
                    "unknown spatial layer(s); available: {names:?}"
                )));
            }
            if requested.is_empty() && layers.len() > 1 && !options.all_layers {
                return Err(data(format!(
                    "select --layer NAME (repeatable) or --all-layers; available: {names:?}"
                )));
            }
            let layers: Vec<_> = layers
                .into_iter()
                .filter(|l| {
                    requested.is_empty()
                        || requested.contains(&geospatial::string(gdal_sys::OGR_L_GetName(*l)))
                })
                .collect();
            if layers.is_empty() {
                return Err(data("input has no selected spatial layers"));
            }
            let mut known = BTreeSet::new();
            for layer in &layers {
                if let Some(expression) = &options.where_clause {
                    let expr = c_str(expression)?;
                    let result = gdal_sys::OGR_L_SetAttributeFilter(*layer, expr.as_ptr());
                    gdal_sys::OGR_L_GetFeatureCount(*layer, 1);
                    if result != 0
                        || gdal_sys::CPLGetLastErrorType() >= gdal_sys::CPLErr::CE_Failure
                    {
                        return Err(data(geospatial::diagnostic("invalid attribute filter")));
                    }
                }
                let definition = gdal_sys::OGR_L_GetLayerDefn(*layer);
                for i in 0..gdal_sys::OGR_FD_GetFieldCount(definition) {
                    known.insert(geospatial::string(gdal_sys::OGR_Fld_GetNameRef(
                        gdal_sys::OGR_FD_GetFieldDefn(definition, i),
                    )));
                }
            }
            if options
                .fields
                .iter()
                .chain(&options.drop_fields)
                .any(|f| !known.contains(f))
            {
                return Err(data("unknown selected/excluded fields"));
            }
            Ok(Self {
                dataset: Some(dataset),
                layers,
                driver,
                schemas: BTreeMap::new(),
                layer_reports: Vec::new(),
                frame,
                without_geometry: 0,
            })
        }
    }
    pub fn read(
        &mut self,
        options: &VectorOptions,
        mut accept: impl FnMut(Feature) -> FeatureResult<()>,
        mut report: impl FnMut(Value, bool) -> Result<(), Error>,
    ) -> Result<(), Error> {
        if self.dataset.is_none() {
            return Err(data("vector reader has already been consumed"));
        }
        let result = self.read_features(options, &mut accept, &mut report);
        // Layer handles are borrowed from this dataset and never used after finish.
        self.layers.clear();
        let closed = self
            .dataset
            .take()
            .expect("native vector reader is single-use")
            .finish("close vector input")
            .map_err(|error| Error::Io(std::io::Error::other(error)));
        match (result, closed) {
            (Ok(()), Ok(())) => Ok(()),
            (Ok(()), Err(error)) | (Err(error), Ok(())) => Err(error),
            (Err(primary), Err(close)) => {
                let secondary = super::super::job_error(close);
                match primary {
                    Error::Job(mut failure) => {
                        failure.secondary.push(secondary);
                        Err(Error::Job(failure))
                    }
                    error => Err(Error::Job(crate::JobFailure {
                        error: super::super::job_error(error),
                        secondary: vec![secondary],
                        retained_paths: Vec::new(),
                        recovery: None,
                    })),
                }
            }
        }
    }
    fn read_features(
        &mut self,
        options: &VectorOptions,
        accept: &mut impl FnMut(Feature) -> FeatureResult<()>,
        report: &mut impl FnMut(Value, bool) -> Result<(), Error>,
    ) -> Result<(), Error> {
        let local = options.source_crs.as_deref() == Some("local");
        let mut from_ecef = if local {
            None
        } else {
            let target = Crs::from_definition("EPSG:4979")?;
            Some(StrictTransform::new(
                &Crs::from_definition("EPSG:4978")?,
                &target,
            )?)
        };
        for layer in self.layers.clone() {
            let _errors = super::geometry::quiet_unless_diagnostics();
            // SAFETY: All layer, field, row and geometry references belong to the
            // live dataset on this ingestion thread. Row RAII spans every query.
            unsafe {
                let name = geospatial::string(gdal_sys::OGR_L_GetName(layer));
                let source = if local {
                    None
                } else if let Some(definition) = &options.source_crs {
                    Some(Crs::from_definition(definition)?)
                } else {
                    let raw = gdal_sys::OGR_L_GetSpatialRef(layer);
                    if raw.is_null() {
                        return Err(data(format!(
                            "layer {name:?} needs a declared CRS or --source-crs override"
                        )));
                    }
                    Some(Crs::clone_native(raw)?)
                };
                let native_height = source.as_ref().is_some_and(Crs::has_native_height);
                if local && options.height_offset.is_some() {
                    return Err(data(
                        "local XYZ is in metres; height-offset is for geospatial placement",
                    ));
                }
                if native_height && options.height_offset.is_some() {
                    return Err(data("declared 3D/vertical CRS already defines heights; use a horizontal CRS override to apply height-offset"));
                }
                let source_wkt = source
                    .as_ref()
                    .map(Crs::wkt)
                    .transpose()?
                    .map_or(Value::Null, Value::String);
                let polygon_units = source.as_ref().and_then(Crs::polygon_units);
                let mut transform = source
                    .map(|source| {
                        EcefTransform::new(
                            source,
                            if native_height {
                                None
                            } else {
                                Some(options.height_offset.unwrap_or(0.))
                            },
                        )
                    })
                    .transpose()?;
                let definition = gdal_sys::OGR_L_GetLayerDefn(layer);
                let mut fields = Vec::new();
                let mut json_fields = BTreeSet::new();
                let mut declared_json_fields = BTreeSet::new();
                let mut declared_schemas = BTreeMap::new();
                let mut declared_failure = None;
                for i in 0..gdal_sys::OGR_FD_GetFieldCount(definition) {
                    if self.driver == "GeoJSON" {
                        continue;
                    }
                    let field = gdal_sys::OGR_FD_GetFieldDefn(definition, i);
                    let key = geospatial::string(gdal_sys::OGR_Fld_GetNameRef(field));
                    if !source_fields::keep(options, &key) {
                        continue;
                    }
                    if matches!(key.as_str(), "_source_id" | "_source_layer") {
                        declared_failure
                            .get_or_insert_with(|| format!("reserved source property: {key}"));
                        continue;
                    }
                    let kind = gdal_sys::OGR_Fld_GetType(field);
                    let scalar = if matches!(kind, 1 | 3 | 5 | 13) && options.list_fields == "json"
                    {
                        declared_json_fields.insert(key.clone());
                        "string"
                    } else if gdal_sys::OGR_Fld_GetSubType(field)
                        == gdal_sys::OGRFieldSubType::OFSTBoolean
                    {
                        "boolean"
                    } else {
                        match kind {
                            0 | 12 => "integer",
                            2 => "real",
                            4 | 9 | 10 | 11 => "string",
                            1 | 3 | 5 | 13 if options.list_fields == "json" => {
                                declared_json_fields.insert(key.clone());
                                "string"
                            }
                            _ => {
                                declared_failure.get_or_insert_with(|| {
                                    format!("unsupported field type for {name}.{key}")
                                });
                                continue;
                            }
                        }
                    };
                    if self.driver != "GeoJSON" {
                        match source_fields::register(&mut declared_schemas, &key, scalar) {
                            Ok(()) => {}
                            Err(FeatureFailure::Rejected(rejection)) => {
                                declared_failure.get_or_insert(rejection.message);
                            }
                            Err(FeatureFailure::Fatal(error)) => return Err(error),
                        }
                    }
                    fields.push((i, key, scalar, kind));
                }
                let mut accepted = 0;
                let mut rejected = 0;
                let mut without = 0;
                gdal_sys::OGR_L_ResetReading(layer);
                loop {
                    gdal_sys::CPLErrorReset();
                    let raw = gdal_sys::OGR_L_GetNextFeature(layer);
                    if raw.is_null() {
                        if gdal_sys::CPLGetLastErrorType() >= gdal_sys::CPLErr::CE_Failure {
                            return Err(Error::Io(std::io::Error::other(geospatial::diagnostic(
                                "cannot read vector layer",
                            ))));
                        }
                        break;
                    }
                    let row = Row(NonNull::new(raw).unwrap());
                    let fid = gdal_sys::OGR_F_GetFID(raw);
                    let native = if self.driver == "GeoJSON" {
                        let p = gdal_sys::OGR_F_GetNativeData(raw);
                        if p.is_null() {
                            None
                        } else {
                            Some(serde_json::from_str::<Value>(&geospatial::string(p))?)
                        }
                    } else {
                        None
                    };
                    check_native("read native feature metadata")?;
                    if self.driver == "GeoJSON" && native.is_none() {
                        return Err(Error::Io(std::io::Error::other(
                            "OGR GeoJSON feature is missing required native data",
                        )));
                    }
                    let id = source_fields::source_id(native.as_ref(), fid)?;
                    if let Some(native) = &native {
                        let reason = if !matches!(
                            native.get("properties"),
                            None | Some(Value::Null | Value::Object(_))
                        ) {
                            Some("GeoJSON feature properties must be an object or null")
                        } else if native.get("geometry").is_none() {
                            Some("GeoJSON feature needs a geometry member (null is allowed)")
                        } else {
                            None
                        };
                        if let Some(reason) = reason {
                            rejected += 1;
                            report(
                                json!({"sourceLayer":name,"sourceId":id,"reason":reason}),
                                true,
                            )?;
                            continue;
                        }
                    }
                    let g = gdal_sys::OGR_F_GetGeometryRef(raw);
                    check_native("read native feature geometry")?;
                    let empty = g.is_null() || gdal_sys::OGR_G_IsEmpty(g) != 0;
                    check_native("read native geometry emptiness")?;
                    if empty {
                        without += 1;
                        self.without_geometry += 1;
                        report(
                            json!({"sourceLayer":name,"sourceId":id,"reason":"source geometry is null or empty","outcome":"no-geometry"}),
                            false,
                        )?;
                        continue;
                    }
                    let mut candidate_schemas = self.schemas.clone();
                    let mut candidate_json_fields = json_fields.clone();
                    let mut candidate_frame = self.frame.clone();
                    let result = (|| -> FeatureResult<()> {
                        if let Some(message) = &declared_failure {
                            return Err(FeatureFailure::reject(message.clone()));
                        }
                        if self.driver != "GeoJSON" {
                            candidate_json_fields.extend(declared_json_fields.iter().cloned());
                            for (key, kind) in &declared_schemas {
                                source_fields::register(&mut candidate_schemas, key, kind)?;
                            }
                        }
                        if gdal_sys::OGR_G_IsMeasured(g) != 0 {
                            return Err(FeatureFailure::reject(
                                "measured geometry is unsupported; retain or explicitly remove M",
                            ));
                        }
                        let has_z = gdal_sys::OGR_GT_HasZ(gdal_sys::OGR_G_GetGeometryType(g)) != 0;
                        if has_z
                            && !local
                            && !native_height
                            && options.height_offset.is_none()
                            && self.driver != "GeoJSON"
                        {
                            return Err(FeatureFailure::reject("3D horizontal-CRS input requires explicit height-offset to ellipsoidal metres, or --sourceCrs with the correct compound CRS"));
                        }
                        let mut count = 0;
                        let mut geometry =
                            read_geometry(g, &mut count, options.max_source_vertices)?;
                        let mut properties = BTreeMap::new();
                        if let Some(native) = &native {
                            properties = source_fields::geojson_properties(
                                native,
                                options,
                                &mut candidate_schemas,
                                &mut candidate_json_fields,
                            )?;
                        } else {
                            for (index, key, scalar, kind) in &fields {
                                let value = if gdal_sys::OGR_F_IsFieldSetAndNotNull(raw, *index)
                                    == 0
                                {
                                    Value::Null
                                } else if candidate_json_fields.contains(key) {
                                    let values = read_list(raw, *index, *kind)?;
                                    json!(serde_json::to_string(&values)?)
                                } else {
                                    match *scalar {
                                        "integer" => {
                                            json!(gdal_sys::OGR_F_GetFieldAsInteger64(raw, *index))
                                        }
                                        "real" => {
                                            let v = gdal_sys::OGR_F_GetFieldAsDouble(raw, *index);
                                            if !v.is_finite() {
                                                return Err(FeatureFailure::reject(format!(
                                                    "nonfinite property: {key}"
                                                )));
                                            }
                                            json!(v)
                                        }
                                        "boolean" => json!(
                                            gdal_sys::OGR_F_GetFieldAsInteger(raw, *index) != 0
                                        ),
                                        _ => json!(geospatial::string(
                                            gdal_sys::OGR_F_GetFieldAsString(raw, *index)
                                        )),
                                    }
                                };
                                check_native("read native feature property")?;
                                properties.insert(key.clone(), value);
                            }
                        }
                        let points: Vec<_> = geometry.points().copied().collect();
                        if !points.iter().flatten().all(|p| p.is_finite()) {
                            return Err(FeatureFailure::reject("coordinates must be finite XYZ"));
                        }
                        let mut intrinsic = IntrinsicGeometry::capture(&geometry, polygon_units)?;
                        let points = if let Some(transform) = &mut transform {
                            transform.transform(&points)?
                        } else {
                            points
                        };
                        if candidate_frame.is_none() {
                            let anchor = points[0];
                            candidate_frame = Some(if local {
                                Frame::local(anchor)
                            } else {
                                let geographic =
                                    from_ecef.as_mut().unwrap().transform(&[anchor])?[0];
                                let matrix = crate::georef::root_transform(
                                    crate::georef::Cartographic::new(
                                        geographic[0],
                                        geographic[1],
                                        geographic[2],
                                    ),
                                    None,
                                );
                                Frame {
                                    anchor,
                                    axes: [
                                        [matrix[0], matrix[1], matrix[2]],
                                        [matrix[4], matrix[5], matrix[6]],
                                        [matrix[8], matrix[9], matrix[10]],
                                    ],
                                }
                            });
                        }
                        let frame = candidate_frame.as_ref().unwrap();
                        if let Some(intrinsic) = &mut intrinsic {
                            intrinsic.earth_center = frame.project([0.; 3]);
                        }
                        let mut iter = points.into_iter();
                        geometry.map(|_| frame.project(iter.next().unwrap()));
                        properties.insert("_source_id".into(), json!(id));
                        properties.insert("_source_layer".into(), json!(name));
                        accept(Feature {
                            properties,
                            geometry,
                            intrinsic,
                            surface_fragment: false,
                            triangle_boundaries: Vec::new(),
                            fragment_path: String::new(),
                        })
                    })();
                    let result = match (result, check_native("decode native vector feature")) {
                        (Err(FeatureFailure::Fatal(error)), _) => Err(FeatureFailure::Fatal(error)),
                        (_, Err(error)) => Err(FeatureFailure::Fatal(error)),
                        (result, Ok(())) => result,
                    };
                    drop(row);
                    match result {
                        Ok(()) => {
                            accepted += 1;
                            self.schemas = candidate_schemas;
                            self.frame = candidate_frame;
                            json_fields = candidate_json_fields;
                        }
                        Err(FeatureFailure::Fatal(error)) => return Err(error),
                        Err(FeatureFailure::Rejected(error)) => {
                            rejected += 1;
                            report(
                                json!({"sourceLayer":name,"sourceId":id,"reason":error.message}),
                                true,
                            )?;
                        }
                    }
                    // Reset recoverable native errors before reading the next row.
                    gdal_sys::CPLErrorReset();
                }
                self.layer_reports.push(json!({"name":name,"features":accepted,"attributeFilter":options.where_clause,
                    "invalidFeatures":rejected,"featuresWithoutGeometry":without,"jsonFields":json_fields,"sourceCrs":source_wkt,
                    "heightMode":if local {"local metres"}else if native_height {"declared CRS"}else if options.height_offset.is_some(){"explicit offset"}else if self.driver!="GeoJSON"{"2D ellipsoid zero"}else{"GeoJSON ellipsoidal metres"},"heightOffset":options.height_offset}));
            }
        }
        self.schemas.insert("_source_id".into(), "string".into());
        self.schemas.insert("_source_layer".into(), "string".into());
        Ok(())
    }
}
fn check_native(context: &str) -> Result<(), Error> {
    // SAFETY: Reads thread-local GDAL diagnostics on the ingestion thread.
    if unsafe { gdal_sys::CPLGetLastErrorType() } >= gdal_sys::CPLErr::CE_Failure {
        Err(Error::Io(std::io::Error::other(geospatial::diagnostic(
            context,
        ))))
    } else {
        Ok(())
    }
}

// SAFETY: These helpers are called only with geometries/rows borrowed from the
// live ingestion dataset; all native point counts are bounded before allocation.
unsafe fn read_geometry(
    g: gdal_sys::OGRGeometryH,
    count: &mut usize,
    limit: usize,
) -> FeatureResult<Geometry> {
    if g.is_null() {
        return Err(Error::Io(std::io::Error::other(
            "native geometry child handle is null",
        ))
        .into());
    }
    let kind = gdal_sys::OGR_GT_Flatten(gdal_sys::OGR_G_GetGeometryType(g));
    let mut points = |raw: gdal_sys::OGRGeometryH| -> FeatureResult<Vec<Point>> {
        if raw.is_null() {
            return Err(
                Error::Io(std::io::Error::other("native geometry ring handle is null")).into(),
            );
        }
        let raw_count = gdal_sys::OGR_G_GetPointCount(raw);
        check_native("read native point count")?;
        if raw_count < 0 {
            return Err(Error::Io(std::io::Error::other("native point count is negative")).into());
        }
        let n = raw_count as usize;
        *count = count
            .checked_add(n)
            .ok_or_else(|| FeatureFailure::reject("source vertex count overflow"))?;
        if *count > limit {
            return Err(FeatureFailure::reject("source feature exceeds maxSourceVertices; explicitly raise the limit or subdivide the source"));
        }
        Ok((0..n)
            .map(|i| {
                [
                    gdal_sys::OGR_G_GetX(raw, i as i32),
                    gdal_sys::OGR_G_GetY(raw, i as i32),
                    gdal_sys::OGR_G_GetZ(raw, i as i32),
                ]
            })
            .collect())
    };
    Ok(match kind {
        1 => {
            let p = points(g)?;
            Geometry::Point(
                *p.first()
                    .ok_or_else(|| FeatureFailure::reject("empty point"))?,
            )
        }
        2 => Geometry::LineString(points(g)?),
        3 => Geometry::Polygon(
            (0..gdal_sys::OGR_G_GetGeometryCount(g))
                .map(|i| points(gdal_sys::OGR_G_GetGeometryRef(g, i)))
                .collect::<Result<_, _>>()?,
        ),
        4..=6 => {
            let parts = (0..gdal_sys::OGR_G_GetGeometryCount(g))
                .map(|i| read_geometry(gdal_sys::OGR_G_GetGeometryRef(g, i), count, limit))
                .collect::<Result<Vec<_>, _>>()?;
            match kind {
                4 => Geometry::MultiPoint(
                    parts
                        .into_iter()
                        .map(|g| {
                            if let Geometry::Point(p) = g {
                                Ok(p)
                            } else {
                                Err(FeatureFailure::reject("invalid multipoint"))
                            }
                        })
                        .collect::<Result<_, _>>()?,
                ),
                5 => Geometry::MultiLineString(
                    parts
                        .into_iter()
                        .map(|g| {
                            if let Geometry::LineString(p) = g {
                                Ok(p)
                            } else {
                                Err(FeatureFailure::reject("invalid multilinestring"))
                            }
                        })
                        .collect::<Result<_, _>>()?,
                ),
                _ => Geometry::MultiPolygon(
                    parts
                        .into_iter()
                        .map(|g| {
                            if let Geometry::Polygon(p) = g {
                                Ok(p)
                            } else {
                                Err(FeatureFailure::reject("invalid multipolygon"))
                            }
                        })
                        .collect::<Result<_, _>>()?,
                ),
            }
        }
        _ => {
            return Err(FeatureFailure::reject(format!(
                "unsupported geometry: {}",
                geospatial::string(gdal_sys::OGR_G_GetGeometryName(g))
            )))
        }
    })
}
unsafe fn read_list(row: gdal_sys::OGRFeatureH, index: i32, kind: u32) -> FeatureResult<Value> {
    let mut count = 0;
    Ok(match kind {
        1 => {
            let p = gdal_sys::OGR_F_GetFieldAsIntegerList(row, index, &mut count);
            check_native("read native list property")?;
            json!(if count == 0 {
                &[]
            } else if p.is_null() || count < 0 {
                return Err(FeatureFailure::reject("invalid list property"));
            } else {
                std::slice::from_raw_parts(p, count as usize)
            })
        }
        13 => {
            let p = gdal_sys::OGR_F_GetFieldAsInteger64List(row, index, &mut count);
            check_native("read native list property")?;
            json!(if count == 0 {
                &[]
            } else if p.is_null() || count < 0 {
                return Err(FeatureFailure::reject("invalid list property"));
            } else {
                std::slice::from_raw_parts(p, count as usize)
            })
        }
        3 => {
            let p = gdal_sys::OGR_F_GetFieldAsDoubleList(row, index, &mut count);
            check_native("read native list property")?;
            let values = if count == 0 {
                &[]
            } else if p.is_null() || count < 0 {
                return Err(FeatureFailure::reject("invalid list property"));
            } else {
                std::slice::from_raw_parts(p, count as usize)
            };
            if !values.iter().all(|v| v.is_finite()) {
                return Err(FeatureFailure::reject("nonfinite list property"));
            }
            json!(values)
        }
        5 => {
            let p = gdal_sys::OGR_F_GetFieldAsStringList(row, index);
            check_native("read native string list property")?;
            let mut values = Vec::new();
            if !p.is_null() {
                while !(*p.add(values.len())).is_null() {
                    values.push(geospatial::string(*p.add(values.len())));
                }
            }
            json!(values)
        }
        _ => return Err(FeatureFailure::reject("unsupported list property")),
    })
}

#[cfg(test)]
mod foundation_tests {
    use super::*;
    fn fixture(path: &Path, features: Value) -> Vec<u8> {
        let bytes =
            serde_json::to_vec(&json!({"type":"FeatureCollection","features":features})).unwrap();
        std::fs::write(path, &bytes).unwrap();
        bytes
    }
    #[test]
    fn callback_io_failure_is_fatal_and_closes_source_without_candidate_effects() {
        let work = tempfile::tempdir().unwrap();
        let input = work.path().join("source.geojson");
        let before = fixture(
            &input,
            json!([
                {"type":"Feature","id":"first","properties":{"candidate":"value"},"geometry":{"type":"Point","coordinates":[100,200,300]}},
                {"type":"Feature","id":"second","properties":{},"geometry":{"type":"Point","coordinates":[1,2,3]}}
            ]),
        );
        let options = VectorOptions {
            source_crs: Some("local".into()),
            skip_invalid: true,
            ..VectorOptions::default()
        };
        let mut reader = Reader::new(&input, &options, None, work.path()).unwrap();
        let mut calls = 0;
        let mut reports = Vec::new();
        let failure = reader
            .read(
                &options,
                |_| {
                    calls += 1;
                    Err(FeatureFailure::Fatal(Error::Io(std::io::Error::new(
                        std::io::ErrorKind::StorageFull,
                        "native accept storage fault",
                    ))))
                },
                |value, invalid| {
                    reports.push((value, invalid));
                    Ok(())
                },
            )
            .unwrap_err();
        match failure {
            Error::Io(error) => {
                assert_eq!(error.kind(), std::io::ErrorKind::StorageFull);
                assert_eq!(error.to_string(), "native accept storage fault");
            }
            error => panic!("unexpected primary error: {error}"),
        }
        assert_eq!(calls, 1);
        assert!(reports.is_empty());
        assert!(reader.frame.is_none());
        assert!(reader.schemas.is_empty());
        assert!(reader.dataset.is_none());
        assert_eq!(std::fs::read(input).unwrap(), before);
    }
    #[test]
    fn malformed_geojson_properties_reject_before_null_geometry_disposition() {
        let work = tempfile::tempdir().unwrap();
        let input = work.path().join("source.geojson");
        fixture(
            &input,
            json!([
                {"type":"Feature","id":"bad-point","properties":7,"geometry":{"type":"Point","coordinates":[100,200,300]}},
                {"type":"Feature","id":"bad-null","properties":"wrong","geometry":null},
                {"type":"Feature","id":"accepted","properties":{"name":"kept"},"geometry":{"type":"Point","coordinates":[1,2,3]}}
            ]),
        );
        let options = VectorOptions {
            source_crs: Some("local".into()),
            skip_invalid: true,
            ..VectorOptions::default()
        };
        let mut reader = Reader::new(&input, &options, None, work.path()).unwrap();
        let mut accepted = Vec::new();
        let mut rejected = Vec::new();
        reader
            .read(
                &options,
                |feature| {
                    accepted.push(feature.source_id().clone());
                    Ok(())
                },
                |value, invalid| {
                    rejected.push((value, invalid));
                    Ok(())
                },
            )
            .unwrap();
        assert_eq!(accepted, vec![json!("\"accepted\"")]);
        assert_eq!(rejected.len(), 2);
        assert!(rejected.iter().all(|(value, invalid)| *invalid
            && value["reason"] == "GeoJSON feature properties must be an object or null"));
        assert_eq!(reader.without_geometry, 0);
        assert_eq!(reader.frame.as_ref().unwrap().anchor, [1., 2., 3.]);
        assert_eq!(
            reader.schemas.get("name").map(String::as_str),
            Some("string")
        );
        assert!(reader.dataset.is_none());
    }
    #[test]
    fn declared_list_fields_from_rejected_or_null_rows_do_not_enter_report() {
        let work = tempfile::tempdir().unwrap();
        let input = work.path().join("source.geojsonl");
        let rows = [
            json!({"type":"Feature","id":"rejected","properties":{"values":[1,2]},"geometry":{"type":"LineString","coordinates":[[0,0,0],[1,1,1]]}}),
            json!({"type":"Feature","id":"null","properties":{"values":[3,4]},"geometry":null}),
        ];
        // RFC 8142 record separators force the sequence driver rather than
        // GDAL's permissive GeoJSON reader accepting multiple JSON objects.
        let bytes = rows
            .iter()
            .map(|row| format!("\x1e{}\n", serde_json::to_string(row).unwrap()))
            .collect::<String>();
        std::fs::write(&input, &bytes).unwrap();
        let options = VectorOptions {
            source_crs: Some("local".into()),
            list_fields: "json".into(),
            skip_invalid: true,
            max_source_vertices: 1,
            ..VectorOptions::default()
        };
        let mut reader = Reader::new(&input, &options, None, work.path()).unwrap();
        assert_eq!(reader.driver, "GeoJSONSeq");
        let mut reports = Vec::new();
        reader
            .read(
                &options,
                |_| panic!("no source geometry should be accepted"),
                |value, invalid| {
                    reports.push((value, invalid));
                    Ok(())
                },
            )
            .unwrap();
        assert_eq!(reports.iter().filter(|(_, invalid)| *invalid).count(), 1);
        assert_eq!(reader.without_geometry, 1);
        assert!(reader.frame.is_none());
        assert!(!reader.schemas.contains_key("values"));
        assert_eq!(reader.layer_reports.len(), 1);
        assert_eq!(reader.layer_reports[0]["jsonFields"], json!([]));
        assert_eq!(std::fs::read_to_string(&input).unwrap(), bytes);
    }
}
