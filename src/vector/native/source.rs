//! Streaming OGR features; explicit axes, heights, filters and exact typed fields.
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
#[derive(Clone, Serialize, Deserialize)]
pub(super) struct Frame {
    pub anchor: Point,
    pub axes: [Point; 3],
}
impl Frame {
    pub fn local(anchor: Point) -> Self {
        Self {
            anchor,
            axes: [[1., 0., 0.], [0., 1., 0.], [0., 0., 1.]],
        }
    }
    pub fn project(&self, p: Point) -> Point {
        let d = sub(p, self.anchor);
        [
            dot(d, self.axes[0]),
            dot(d, self.axes[2]),
            -dot(d, self.axes[1]),
        ]
    }
}
pub(super) struct Reader {
    _dataset: Dataset<'static>,
    layers: Vec<gdal_sys::OGRLayerH>,
    pub driver: String,
    pub schemas: BTreeMap<String, String>,
    pub layer_reports: Vec<Value>,
    pub frame: Option<Frame>,
    pub without_geometry: usize,
}
impl Reader {
    pub fn new(input: &Path, options: &VectorOptions, frame: Option<Frame>) -> Result<Self, Error> {
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
                _dataset: dataset,
                layers,
                driver,
                schemas: BTreeMap::new(),
                layer_reports: Vec::new(),
                frame,
                without_geometry: 0,
            })
        }
    }
    fn register(&mut self, name: &str, kind: &str) -> Result<(), Error> {
        let kind = match self.schemas.get(name).map(String::as_str) {
            Some(previous) if previous != kind => {
                if matches!(previous, "integer" | "real") && matches!(kind, "integer" | "real") {
                    "real"
                } else {
                    return Err(data(format!(
                        "incompatible scalar schemas for {name:?}; convert these layers separately"
                    )));
                }
            }
            _ => kind,
        };
        self.schemas.insert(name.into(), kind.into());
        Ok(())
    }
    pub fn read(
        &mut self,
        options: &VectorOptions,
        mut accept: impl FnMut(Feature) -> Result<(), Error>,
        mut report: impl FnMut(Value, bool) -> Result<(), Error>,
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
                let keep = |name: &str| {
                    (options.fields.is_empty() || options.fields.iter().any(|f| f == name))
                        && !options.drop_fields.iter().any(|f| f == name)
                };
                for i in 0..gdal_sys::OGR_FD_GetFieldCount(definition) {
                    let field = gdal_sys::OGR_FD_GetFieldDefn(definition, i);
                    let key = geospatial::string(gdal_sys::OGR_Fld_GetNameRef(field));
                    if !keep(&key) {
                        continue;
                    }
                    if matches!(key.as_str(), "_source_id" | "_source_layer") {
                        return Err(data(format!("reserved source property: {key}")));
                    }
                    let kind = gdal_sys::OGR_Fld_GetType(field);
                    let scalar = if matches!(kind, 1 | 3 | 5 | 13) && options.list_fields == "json"
                    {
                        json_fields.insert(key.clone());
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
                                json_fields.insert(key.clone());
                                "string"
                            }
                            _ => {
                                return Err(data(format!(
                                    "unsupported field type for {name}.{key}"
                                )))
                            }
                        }
                    };
                    self.register(&key, scalar)?;
                    fields.push((i, key, scalar, kind));
                }
                let mut accepted = 0;
                let mut rejected = 0;
                let mut without = 0;
                gdal_sys::OGR_L_ResetReading(layer);
                loop {
                    let raw = gdal_sys::OGR_L_GetNextFeature(layer);
                    if raw.is_null() {
                        if gdal_sys::CPLGetLastErrorType() >= gdal_sys::CPLErr::CE_Failure {
                            return Err(data(geospatial::diagnostic("cannot read vector layer")));
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
                    let id = native
                        .as_ref()
                        .and_then(|v| v.get("id"))
                        .cloned()
                        .unwrap_or(json!(fid));
                    let id = serde_json::to_string(&id)?;
                    let g = gdal_sys::OGR_F_GetGeometryRef(raw);
                    if g.is_null() || gdal_sys::OGR_G_IsEmpty(g) != 0 {
                        without += 1;
                        self.without_geometry += 1;
                        report(
                            json!({"sourceLayer":name,"sourceId":id,"reason":"source geometry is null or empty","outcome":"no-geometry"}),
                            false,
                        )?;
                        continue;
                    }
                    let result = (|| -> Result<(), Error> {
                        if gdal_sys::OGR_G_IsMeasured(g) != 0 {
                            return Err(data(
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
                            return Err(data("3D horizontal-CRS input requires explicit height-offset to ellipsoidal metres, or --sourceCrs with the correct compound CRS"));
                        }
                        let mut count = 0;
                        let mut geometry =
                            read_geometry(g, &mut count, options.max_source_vertices)?;
                        let mut properties = BTreeMap::new();
                        if let Some(native) = &native {
                            if let Some(props) = native["properties"].as_object() {
                                for (key, value) in props {
                                    if !keep(key) {
                                        continue;
                                    }
                                    if matches!(key.as_str(), "_source_id" | "_source_layer") {
                                        return Err(data(format!(
                                            "reserved source property: {key}"
                                        )));
                                    }
                                    let value = if value.is_array() && options.list_fields == "json"
                                    {
                                        json_fields.insert(key.clone());
                                        json!(serde_json::to_string(value)?)
                                    } else {
                                        value.clone()
                                    };
                                    if !value.is_null() {
                                        let kind = if value.is_boolean() {
                                            "boolean"
                                        } else if value.is_i64() || value.is_u64() {
                                            "integer"
                                        } else if value.is_f64() {
                                            "real"
                                        } else if value.is_string() {
                                            "string"
                                        } else {
                                            return Err(data(format!(
                                                "unsupported complex property: {key}"
                                            )));
                                        };
                                        self.register(key, kind)?;
                                    }
                                    properties.insert(key.clone(), value);
                                }
                            }
                        } else {
                            for (index, key, scalar, kind) in &fields {
                                let value = if gdal_sys::OGR_F_IsFieldSetAndNotNull(raw, *index)
                                    == 0
                                {
                                    Value::Null
                                } else if json_fields.contains(key) {
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
                                                return Err(data(format!(
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
                                properties.insert(key.clone(), value);
                            }
                        }
                        let points: Vec<_> = geometry.points().copied().collect();
                        if !points.iter().flatten().all(|p| p.is_finite()) {
                            return Err(data("coordinates must be finite XYZ"));
                        }
                        let points = if let Some(transform) = &mut transform {
                            transform.transform(&points)?
                        } else {
                            points
                        };
                        if self.frame.is_none() {
                            let anchor = points[0];
                            self.frame = Some(if local {
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
                        let frame = self.frame.as_ref().unwrap();
                        let mut iter = points.into_iter();
                        geometry.map(|_| frame.project(iter.next().unwrap()));
                        properties.insert("_source_id".into(), json!(id));
                        properties.insert("_source_layer".into(), json!(name));
                        accept(Feature {
                            properties,
                            geometry,
                            surface_fragment: false,
                            triangle_boundaries: Vec::new(),
                            fragment_path: String::new(),
                        })
                    })();
                    drop(row);
                    match result {
                        Ok(()) => accepted += 1,
                        Err(error @ Error::Environment(_)) => return Err(error),
                        Err(error) => {
                            rejected += 1;
                            report(
                                json!({"sourceLayer":name,"sourceId":id,"reason":error.to_string()}),
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
// SAFETY: These helpers are called only with geometries/rows borrowed from the
// live ingestion dataset; all native point counts are bounded before allocation.
unsafe fn read_geometry(
    g: gdal_sys::OGRGeometryH,
    count: &mut usize,
    limit: usize,
) -> Result<Geometry, Error> {
    let kind = gdal_sys::OGR_GT_Flatten(gdal_sys::OGR_G_GetGeometryType(g));
    let mut points = |raw: gdal_sys::OGRGeometryH| -> Result<Vec<Point>, Error> {
        let n = gdal_sys::OGR_G_GetPointCount(raw) as usize;
        *count = count
            .checked_add(n)
            .ok_or_else(|| data("source vertex count overflow"))?;
        if *count > limit {
            return Err(data("source feature exceeds maxSourceVertices; explicitly raise the limit or subdivide the source"));
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
            Geometry::Point(*p.first().ok_or_else(|| data("empty point"))?)
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
                                Err(data("invalid multipoint"))
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
                                Err(data("invalid multilinestring"))
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
                                Err(data("invalid multipolygon"))
                            }
                        })
                        .collect::<Result<_, _>>()?,
                ),
            }
        }
        _ => {
            return Err(data(format!(
                "unsupported geometry: {}",
                geospatial::string(gdal_sys::OGR_G_GetGeometryName(g))
            )))
        }
    })
}
unsafe fn read_list(row: gdal_sys::OGRFeatureH, index: i32, kind: u32) -> Result<Value, Error> {
    let mut count = 0;
    Ok(match kind {
        1 => {
            let p = gdal_sys::OGR_F_GetFieldAsIntegerList(row, index, &mut count);
            json!(if count == 0 {
                &[]
            } else if p.is_null() || count < 0 {
                return Err(data("invalid list property"));
            } else {
                std::slice::from_raw_parts(p, count as usize)
            })
        }
        13 => {
            let p = gdal_sys::OGR_F_GetFieldAsInteger64List(row, index, &mut count);
            json!(if count == 0 {
                &[]
            } else if p.is_null() || count < 0 {
                return Err(data("invalid list property"));
            } else {
                std::slice::from_raw_parts(p, count as usize)
            })
        }
        3 => {
            let p = gdal_sys::OGR_F_GetFieldAsDoubleList(row, index, &mut count);
            let values = if count == 0 {
                &[]
            } else if p.is_null() || count < 0 {
                return Err(data("invalid list property"));
            } else {
                std::slice::from_raw_parts(p, count as usize)
            };
            if !values.iter().all(|v| v.is_finite()) {
                return Err(data("nonfinite list property"));
            }
            json!(values)
        }
        5 => {
            let p = gdal_sys::OGR_F_GetFieldAsStringList(row, index);
            let mut values = Vec::new();
            if !p.is_null() {
                while !(*p.add(values.len())).is_null() {
                    values.push(geospatial::string(*p.add(values.len())));
                }
            }
            json!(values)
        }
        _ => return Err(data("unsupported list property")),
    })
}
