//! Portable, bounded-memory GeoJSON and immutable GeoPackage ingestion.
//!
//! GeoJSON collections are streamed into a temporary SQLite spool one feature
//! at a time; original JSON scalars and IDs never pass through float coercions.
//! GeoPackage geometry is standard GP binary with ISO WKB, decoded by geozero.
use super::super::source_fields;
use super::*;
use geozero::{CoordDimensions, GeomProcessor, GeozeroGeometry};
use rusqlite::{
    config::DbConfig,
    hooks::{AuthAction, AuthContext, Authorization},
    params, params_from_iter,
    types::ValueRef,
    Connection, OpenFlags,
};
use serde::de::{DeserializeSeed, IgnoredAny, MapAccess, SeqAccess, Visitor};
use std::{
    fmt,
    fs::File,
    io::{BufReader, Read},
};

fn input_sql(error: rusqlite::Error) -> Error {
    data(format!("vector input: {error}"))
}
fn quoted(name: &str) -> String {
    format!("\"{}\"", name.replace('"', "\"\""))
}

#[derive(Clone)]
struct Field {
    name: String,
    kind: String,
}
#[derive(Clone)]
struct Layer {
    name: String,
    geometry: String,
    fid: String,
    srs: i32,
    geometry_type: Option<String>,
    z: i32,
    m: i32,
    definition: Option<String>,
    fields: Vec<Field>,
}
struct GeoJsonSource {
    // Close SQLite before removing the spool, including on Windows.
    db: Connection,
    _file: tempfile::NamedTempFile,
    layer: Layer,
    row_column: String,
}
struct GpkgSource {
    db: Connection,
    layers: Vec<Layer>,
}
enum Source {
    GeoJson(GeoJsonSource),
    Gpkg(GpkgSource),
}
pub(super) struct Reader {
    source: Source,
    pub driver: String,
    pub schemas: BTreeMap<String, String>,
    pub layer_reports: Vec<Value>,
    pub frame: Option<Frame>,
    pub without_geometry: usize,
}

impl Reader {
    pub fn new(input: &Path, options: &VectorOptions, frame: Option<Frame>) -> Result<Self, Error> {
        validate_filter(options.where_clause.as_deref())?;
        let extension = input.extension().and_then(|s| s.to_str()).unwrap_or("");
        let source = if extension.eq_ignore_ascii_case("gpkg") {
            Source::Gpkg(open_gpkg(input, options)?)
        } else if extension.eq_ignore_ascii_case("geojson")
            || extension.eq_ignore_ascii_case("json")
        {
            Source::GeoJson(open_geojson(input, options)?)
        } else {
            return Err(Error::Environment("portable vector input supports GeoJSON and GeoPackage; use native-geospatial for other drivers".into()));
        };
        let (driver, layers) = match &source {
            Source::GeoJson(source) => ("GeoJSON", std::slice::from_ref(&source.layer)),
            Source::Gpkg(source) => ("GPKG", source.layers.as_slice()),
        };
        let known: BTreeSet<_> = layers
            .iter()
            .flat_map(|l| l.fields.iter().map(|f| &f.name))
            .collect();
        if options
            .fields
            .iter()
            .chain(&options.drop_fields)
            .any(|f| !known.contains(f))
        {
            return Err(data("unknown selected/excluded fields"));
        }
        Ok(Self {
            driver: driver.into(),
            source,
            schemas: BTreeMap::new(),
            layer_reports: Vec::new(),
            frame,
            without_geometry: 0,
        })
    }

    pub fn read(
        &mut self,
        options: &VectorOptions,
        mut accept: impl FnMut(Feature) -> Result<(), Error>,
        mut report: impl FnMut(Value, bool) -> Result<(), Error>,
    ) -> Result<(), Error> {
        let local = options.source_crs.as_deref() == Some("local");
        if local && options.height_offset.is_some() {
            return Err(data(
                "local XYZ is in metres; height-offset is for geospatial placement",
            ));
        }
        let (db, layers, geojson, row_column) = match &self.source {
            Source::GeoJson(source) => (
                &source.db,
                vec![source.layer.clone()],
                true,
                source.row_column.as_str(),
            ),
            Source::Gpkg(source) => (&source.db, source.layers.clone(), false, ""),
        };
        for layer in layers {
            let name = &layer.name;
            let definition = if local {
                None
            } else {
                Some(
                    options
                        .source_crs
                        .as_ref()
                        .or(layer.definition.as_ref())
                        .ok_or_else(|| {
                            data(format!(
                                "layer {name:?} needs a declared CRS or --source-crs override"
                            ))
                        })?,
                )
            };
            let mut transform = definition
                .map(|definition| {
                    crate::crs::Transform::new(definition, options.height_offset.unwrap_or(0.))
                })
                .transpose()?;
            let mut json_fields = BTreeSet::new();
            let fields: Vec<_> = layer
                .fields
                .iter()
                .filter(|field| source_fields::keep(options, &field.name))
                .collect();
            for field in &fields {
                if matches!(field.name.as_str(), "_source_id" | "_source_layer") {
                    return Err(data(format!("reserved source property: {}", field.name)));
                }
                let kind = if field.kind == "list" && options.list_fields == "json" {
                    json_fields.insert(field.name.clone());
                    "string"
                } else {
                    field.kind.as_str()
                };
                if !matches!(kind, "boolean" | "integer" | "real" | "string") {
                    return Err(data(format!(
                        "unsupported field type for {name}.{}",
                        field.name
                    )));
                }
                source_fields::register(&mut self.schemas, &field.name, kind)?;
            }
            let expression = options.where_clause.as_deref().unwrap_or("1");
            let query = if geojson && row_column.is_empty() {
                "SELECT seq,json FROM rusty_features ORDER BY seq".into()
            } else if geojson {
                format!("SELECT {0}.seq, {0}.json FROM rusty_features AS {0} JOIN (SELECT {1} FROM rusty_attributes WHERE ({expression})) AS attrs ON {0}.seq=attrs.{1} ORDER BY {0}.seq", "payload", quoted(row_column))
            } else {
                let columns = std::iter::once(&layer.fid)
                    .chain(std::iter::once(&layer.geometry))
                    .chain(fields.iter().map(|f| &f.name))
                    .map(|s| quoted(s))
                    .collect::<Vec<_>>()
                    .join(",");
                format!(
                    "SELECT {columns} FROM {} WHERE ({expression}) ORDER BY {}",
                    quoted(name),
                    quoted(&layer.fid)
                )
            };
            let mut statement = db
                .prepare(&query)
                .map_err(|error| data(format!("invalid attribute filter: {error}")))?;
            if !statement.readonly() || statement.parameter_count() != 0 {
                return Err(data(
                    "invalid attribute filter: expected a read-only expression without parameters",
                ));
            }
            let mut rows = statement.query([]).map_err(input_sql)?;
            let (mut accepted, mut rejected, mut without) = (0, 0, 0);
            while let Some(row) = rows.next().map_err(input_sql)? {
                let (id, raw_geometry, native) = if geojson {
                    let fid = row.get::<_, i64>(0).map_err(input_sql)?;
                    let native: Value =
                        serde_json::from_str(&row.get::<_, String>(1).map_err(input_sql)?)?;
                    let id = source_fields::source_id(Some(&native), fid)?;
                    (id, None, Some(native))
                } else {
                    (
                        source_fields::source_id(None, row.get(0).map_err(input_sql)?)?,
                        Some(row.get_ref(1).map_err(input_sql)?),
                        None,
                    )
                };
                let decoded = if let Some(native) = &native {
                    if !matches!(
                        native.get("properties"),
                        None | Some(Value::Null | Value::Object(_))
                    ) {
                        Err(data("GeoJSON feature properties must be an object or null"))
                    } else if let Some(geometry) = native.get("geometry") {
                        decode_geojson(geometry, options.max_source_vertices)
                    } else {
                        Err(data(
                            "GeoJSON feature needs a geometry member (null is allowed)",
                        ))
                    }
                } else {
                    match raw_geometry.unwrap() {
                        ValueRef::Null => Ok(None),
                        ValueRef::Blob(blob) => {
                            decode_gpkg(blob, layer.srs, options.max_source_vertices)
                        }
                        _ => Err(data("GeoPackage geometry must be a binary blob or null")),
                    }
                };
                if matches!(decoded, Ok(None)) {
                    without += 1;
                    self.without_geometry += 1;
                    report(
                        json!({"sourceLayer":name,"sourceId":id,"reason":"source geometry is null or empty","outcome":"no-geometry"}),
                        false,
                    )?;
                    continue;
                }
                let result = (|| {
                    let (mut geometry, has_z) = decoded?.unwrap();
                    if !geojson {
                        validate_declared_geometry(&layer, &geometry, has_z)?;
                    }
                    if has_z && !local && !geojson && options.height_offset.is_none() {
                        return Err(data("3D horizontal-CRS input requires explicit height-offset to ellipsoidal metres, or --sourceCrs with the correct compound CRS"));
                    }
                    let mut properties = if let Some(native) = &native {
                        source_fields::geojson_properties(
                            native,
                            options,
                            &mut self.schemas,
                            &mut json_fields,
                        )?
                    } else {
                        let mut properties = BTreeMap::new();
                        for (index, field) in fields.iter().enumerate() {
                            properties.insert(
                                field.name.clone(),
                                sql_property(
                                    row.get_ref(index + 2).map_err(input_sql)?,
                                    &field.kind,
                                    &field.name,
                                )?,
                            );
                        }
                        properties
                    };
                    let mut intrinsic = IntrinsicGeometry::capture(
                        &geometry,
                        transform
                            .as_ref()
                            .and_then(crate::crs::Transform::polygon_units),
                    )?;
                    let points: Vec<_> = geometry.points().copied().collect();
                    let points = if let Some(transform) = &mut transform {
                        transform.transform(&points)?
                    } else {
                        points
                    };
                    if self.frame.is_none() {
                        self.frame = Some(if local {
                            Frame::local(points[0])
                        } else {
                            Frame::georeferenced(points[0])?
                        });
                    }
                    let frame = self.frame.as_ref().unwrap();
                    if let Some(intrinsic) = &mut intrinsic {
                        intrinsic.earth_center = frame.project([0.; 3]);
                    }
                    let mut points = points.into_iter();
                    geometry.map(|_| frame.project(points.next().unwrap()));
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
            }
            self.layer_reports.push(json!({"name":name,"features":accepted,"attributeFilter":options.where_clause,
                "invalidFeatures":rejected,"featuresWithoutGeometry":without,"jsonFields":json_fields,"sourceCrs":definition,
                "heightMode":if local {"local metres"} else if options.height_offset.is_some() {"explicit offset"} else if geojson {"GeoJSON ellipsoidal metres"} else {"2D ellipsoid zero"},"heightOffset":options.height_offset}));
        }
        self.schemas.insert("_source_id".into(), "string".into());
        self.schemas.insert("_source_layer".into(), "string".into());
        Ok(())
    }
}

fn sql_property(value: ValueRef<'_>, kind: &str, name: &str) -> Result<Value, Error> {
    Ok(match (value, kind) {
        (ValueRef::Null, _) => Value::Null,
        (ValueRef::Integer(value), "boolean") if matches!(value, 0 | 1) => json!(value != 0),
        (ValueRef::Integer(value), "integer") => json!(value),
        (ValueRef::Integer(value), "real") => {
            // Conversion must not destroy a selected integer before the shared
            // encoder has the chance to enforce exact float64 representation.
            json!(value)
        }
        (ValueRef::Real(value), "real") if value.is_finite() => json!(value),
        (ValueRef::Text(value), "string") => json!(std::str::from_utf8(value)
            .map_err(|_| data(format!("invalid UTF-8 property: {name}")))?),
        _ => return Err(data(format!("invalid typed property: {name}"))),
    })
}

/// A filter is one expression, never SQL statements. Quote-aware validation
/// permits semicolons inside string/identifier literals but excludes comments,
/// parameters and unmatched parentheses that could escape the WHERE wrapper.
fn validate_filter(expression: Option<&str>) -> Result<(), Error> {
    let Some(expression) = expression else {
        return Ok(());
    };
    if expression.trim().is_empty() {
        return Err(data("invalid attribute filter: empty expression"));
    }
    let mut chars = expression.chars().peekable();
    let mut quote = None;
    let mut depth = 0usize;
    while let Some(ch) = chars.next() {
        if ch == '\0' {
            return Err(data("invalid attribute filter: NUL byte"));
        }
        if let Some(end) = quote {
            if ch == end {
                if chars.peek() == Some(&end) {
                    chars.next();
                } else {
                    quote = None;
                }
            }
            continue;
        }
        match ch {
            '\'' | '"' | '`' => quote = Some(ch),
            '[' => quote = Some(']'),
            '(' => depth += 1,
            ')' if depth > 0 => depth -= 1,
            ')' | ';' | '?' | ':' | '@' | '$' => {
                return Err(data("invalid attribute filter: expected one expression"))
            }
            '-' if chars.peek() == Some(&'-') => {
                return Err(data(
                    "invalid attribute filter: SQL comments are unsupported",
                ))
            }
            '/' if chars.peek() == Some(&'*') => {
                return Err(data(
                    "invalid attribute filter: SQL comments are unsupported",
                ))
            }
            _ => {}
        }
    }
    if quote.is_some() || depth != 0 {
        return Err(data(
            "invalid attribute filter: unterminated quote or parentheses",
        ));
    }
    Ok(())
}

fn selected_names(
    names: BTreeSet<String>,
    options: &VectorOptions,
) -> Result<BTreeSet<String>, Error> {
    let requested: BTreeSet<_> = options.layers.iter().cloned().collect();
    if requested.len() != options.layers.len() {
        return Err(data("duplicate layer selection"));
    }
    if !requested.is_subset(&names) {
        return Err(data(format!(
            "unknown spatial layer(s); available: {names:?}"
        )));
    }
    if requested.is_empty() && names.len() > 1 && !options.all_layers {
        return Err(data(format!(
            "select --layer NAME (repeatable) or --all-layers; available: {names:?}"
        )));
    }
    let selected = if requested.is_empty() {
        names
    } else {
        requested
    };
    if selected.is_empty() {
        return Err(data("input has no selected spatial layers"));
    }
    Ok(selected)
}
fn select_layers(mut layers: Vec<Layer>, options: &VectorOptions) -> Result<Vec<Layer>, Error> {
    let selected = selected_names(
        layers.iter().map(|layer| layer.name.clone()).collect(),
        options,
    )?;
    layers.retain(|layer| selected.contains(&layer.name));
    Ok(layers)
}

struct JsonSpool<'a> {
    db: &'a Connection,
    next: i64,
    kinds: BTreeMap<String, Option<String>>,
    large_unsigned: bool,
    options: &'a VectorOptions,
}
impl JsonSpool<'_> {
    fn feature(&mut self, feature: Value) -> Result<(), Error> {
        if feature["type"] != "Feature" {
            return Err(data("GeoJSON collection entries must be Features"));
        }
        if let Some(properties) = feature["properties"].as_object() {
            for (name, value) in properties {
                let current = self.kinds.entry(name.clone()).or_default();
                if value.is_null() {
                    continue;
                }
                self.large_unsigned |= value.as_u64().is_some_and(|v| v > i64::MAX as u64);
                let kind = if value.is_boolean() {
                    "boolean"
                } else if value.is_i64() || value.is_u64() {
                    "integer"
                } else if value.is_f64() {
                    "real"
                } else if value.is_array() {
                    if self.options.list_fields == "json" {
                        "string"
                    } else {
                        "list"
                    }
                } else {
                    "string"
                };
                match current.as_deref() {
                    None => *current = Some(kind.into()),
                    Some(previous) if previous == kind => {},
                    Some("integer" | "real") if matches!(kind, "integer" | "real") => *current = Some("real".into()),
                    Some(_) if source_fields::keep(self.options, name) => return Err(data(format!("incompatible scalar schemas for {name:?}; convert these layers separately"))),
                    Some(_) => {},
                }
            }
        }
        self.db
            .execute(
                "INSERT INTO rusty_features(seq,json) VALUES (?1,?2)",
                params![self.next, serde_json::to_string(&feature)?],
            )
            .map_err(input_sql)?;
        self.next = self
            .next
            .checked_add(1)
            .ok_or_else(|| data("source feature count overflow"))?;
        Ok(())
    }
}
// serde_json::Value normally keeps only the last duplicate member. Reject
// duplicates recursively before IDs, attributes or coordinate members can be lost.
struct UniqueValue(Value);
impl<'de> serde::Deserialize<'de> for UniqueValue {
    fn deserialize<D: serde::Deserializer<'de>>(de: D) -> Result<Self, D::Error> {
        struct UniqueVisitor;
        impl<'de> Visitor<'de> for UniqueVisitor {
            type Value = UniqueValue;
            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str("JSON with unique object members")
            }
            fn visit_bool<E: serde::de::Error>(self, value: bool) -> Result<Self::Value, E> {
                Ok(UniqueValue(Value::Bool(value)))
            }
            fn visit_i64<E: serde::de::Error>(self, value: i64) -> Result<Self::Value, E> {
                Ok(UniqueValue(json!(value)))
            }
            fn visit_u64<E: serde::de::Error>(self, value: u64) -> Result<Self::Value, E> {
                Ok(UniqueValue(json!(value)))
            }
            fn visit_f64<E: serde::de::Error>(self, value: f64) -> Result<Self::Value, E> {
                let number = serde_json::Number::from_f64(value)
                    .ok_or_else(|| E::custom("non-finite JSON number"))?;
                Ok(UniqueValue(Value::Number(number)))
            }
            fn visit_str<E: serde::de::Error>(self, value: &str) -> Result<Self::Value, E> {
                Ok(UniqueValue(Value::String(value.into())))
            }
            fn visit_string<E: serde::de::Error>(self, value: String) -> Result<Self::Value, E> {
                Ok(UniqueValue(Value::String(value)))
            }
            fn visit_unit<E: serde::de::Error>(self) -> Result<Self::Value, E> {
                Ok(UniqueValue(Value::Null))
            }
            fn visit_none<E: serde::de::Error>(self) -> Result<Self::Value, E> {
                self.visit_unit()
            }
            fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<Self::Value, A::Error> {
                let mut values = Vec::new();
                while let Some(value) = seq.next_element::<UniqueValue>()? {
                    values.push(value.0);
                }
                Ok(UniqueValue(Value::Array(values)))
            }
            fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Self::Value, A::Error> {
                let mut values = serde_json::Map::new();
                while let Some(key) = map.next_key::<String>()? {
                    if values.contains_key(&key) {
                        return Err(serde::de::Error::custom(format!(
                            "duplicate GeoJSON member: {key}"
                        )));
                    }
                    values.insert(key, map.next_value::<UniqueValue>()?.0);
                }
                Ok(UniqueValue(Value::Object(values)))
            }
        }
        de.deserialize_any(UniqueVisitor)
    }
}

struct FeaturesSeed<'a, 'b>(&'a mut JsonSpool<'b>);
impl<'de> DeserializeSeed<'de> for FeaturesSeed<'_, '_> {
    type Value = ();
    fn deserialize<D: serde::Deserializer<'de>>(self, de: D) -> Result<(), D::Error> {
        struct FeaturesVisitor<'a, 'b>(&'a mut JsonSpool<'b>);
        impl<'de> Visitor<'de> for FeaturesVisitor<'_, '_> {
            type Value = ();
            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str("a GeoJSON features array")
            }
            fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<(), A::Error> {
                while let Some(feature) = seq.next_element::<UniqueValue>()? {
                    self.0
                        .feature(feature.0)
                        .map_err(serde::de::Error::custom)?;
                }
                Ok(())
            }
        }
        de.deserialize_seq(FeaturesVisitor(self.0))
    }
}
struct RootSeed<'a, 'b>(&'a mut JsonSpool<'b>);
impl<'de> DeserializeSeed<'de> for RootSeed<'_, '_> {
    type Value = (serde_json::Map<String, Value>, bool);
    fn deserialize<D: serde::Deserializer<'de>>(self, de: D) -> Result<Self::Value, D::Error> {
        struct RootVisitor<'a, 'b>(&'a mut JsonSpool<'b>);
        impl<'de> Visitor<'de> for RootVisitor<'_, '_> {
            type Value = (serde_json::Map<String, Value>, bool);
            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str("a GeoJSON object")
            }
            fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Self::Value, A::Error> {
                let mut values = serde_json::Map::new();
                let mut features = false;
                let mut seen = BTreeSet::new();
                while let Some(key) = map.next_key::<String>()? {
                    if !seen.insert(key.clone()) {
                        return Err(serde::de::Error::custom(format!(
                            "duplicate GeoJSON root member: {key}"
                        )));
                    }
                    match key.as_str() {
                        "features" => {
                            features = true;
                            map.next_value_seed(FeaturesSeed(self.0))?;
                        }
                        "type" | "name" | "crs" | "id" | "properties" | "geometry"
                        | "coordinates" | "geometries" => {
                            values.insert(key, map.next_value::<UniqueValue>()?.0);
                        }
                        _ => {
                            map.next_value::<IgnoredAny>()?;
                        }
                    }
                }
                Ok((values, features))
            }
        }
        de.deserialize_map(RootVisitor(self.0))
    }
}

fn open_geojson(input: &Path, options: &VectorOptions) -> Result<GeoJsonSource, Error> {
    let file = tempfile::NamedTempFile::new()?;
    let db = Connection::open(file.path()).map_err(input_sql)?;
    db.set_db_config(DbConfig::SQLITE_DBCONFIG_DQS_DML, false)
        .map_err(input_sql)?;
    db.execute_batch("PRAGMA journal_mode=OFF; PRAGMA synchronous=OFF; PRAGMA temp_store=FILE; PRAGMA cache_size=-8192; CREATE TABLE rusty_features(seq INTEGER PRIMARY KEY,json TEXT NOT NULL); BEGIN").map_err(input_sql)?;
    let mut spool = JsonSpool {
        db: &db,
        next: 0,
        kinds: BTreeMap::new(),
        large_unsigned: false,
        options,
    };
    let mut de = serde_json::Deserializer::from_reader(BufReader::new(File::open(input)?));
    let (metadata, has_features) = RootSeed(&mut spool).deserialize(&mut de)?;
    de.end()?;
    let root = Value::Object(metadata);
    match root["type"].as_str() {
        Some("FeatureCollection") if has_features => {}
        Some("Feature") if !has_features => spool.feature(root.clone())?,
        Some(
            "Point" | "MultiPoint" | "LineString" | "MultiLineString" | "Polygon" | "MultiPolygon"
            | "GeometryCollection",
        ) if !has_features => {
            spool.feature(json!({"type":"Feature","properties":{},"geometry":root}))?;
        }
        _ => return Err(data("invalid GeoJSON root type or features array")),
    }
    if options.where_clause.is_some() && spool.large_unsigned {
        return Err(Error::Environment("portable GeoJSON attribute filters cannot represent unsigned integers above INT64 exactly; use native-geospatial for this filter".into()));
    }
    let fields: Vec<_> = spool
        .kinds
        .iter()
        .map(|(name, kind)| Field {
            name: name.clone(),
            kind: kind.clone().unwrap_or_else(|| "string".into()),
        })
        .collect();
    let name = root
        .get("name")
        .and_then(Value::as_str)
        .map(str::to_owned)
        .unwrap_or_else(|| {
            input
                .file_stem()
                .unwrap_or_default()
                .to_string_lossy()
                .into_owned()
        });
    let definition =
        if options.source_crs.is_some() {
            None
        } else {
            match root.get("crs") {
                None | Some(Value::Null) => Some("EPSG:4326".into()),
                Some(crs) if crs["type"] == "name" => {
                    let name = crs["properties"]["name"]
                        .as_str()
                        .ok_or_else(|| data("GeoJSON named CRS needs a name"))?;
                    Some(
                        if matches!(
                            name,
                            "urn:ogc:def:crs:OGC:1.3:CRS84" | "OGC:CRS84" | "CRS84"
                        ) {
                            "EPSG:4326".into()
                        } else if name.starts_with("urn:ogc:def:crs:EPSG:") {
                            format!("EPSG:{}", name.rsplit(':').next().unwrap())
                        } else {
                            name.into()
                        },
                    )
                }
                Some(_) => return Err(Error::Environment(
                    "portable GeoJSON requires a named CRS; linked CRS definitions are unsupported"
                        .into(),
                )),
            }
        };
    let layer = select_layers(
        vec![Layer {
            name,
            geometry: String::new(),
            fid: String::new(),
            srs: 4326,
            geometry_type: None,
            z: 2,
            m: 0,
            definition,
            fields,
        }],
        options,
    )?
    .remove(0);
    let mut row_column = String::new();
    let mut ambiguous_columns = BTreeSet::new();
    let mut ambiguous_names = Vec::new();
    if options.where_clause.is_some() {
        row_column = "_rusty_row".into();

        while spool
            .kinds
            .keys()
            .any(|name| name.eq_ignore_ascii_case(&row_column))
        {
            row_column.push('_');
        }
        // GeoJSON property keys are case-sensitive, but SQLite identifiers use
        // ASCII case folding. Retain a single inaccessible placeholder for
        // each ambiguous group so it cannot fall back to a rowid or keyword.
        let mut groups: BTreeMap<String, Vec<&Field>> = BTreeMap::new();
        for field in &layer.fields {
            groups
                .entry(field.name.to_ascii_lowercase())
                .or_default()
                .push(field);
        }
        for (column, group) in &groups {
            if group.len() > 1 {
                ambiguous_columns.insert(column.clone());
                ambiguous_names.push(group.iter().map(|f| f.name.clone()).collect::<Vec<_>>());
            }
        }
        let filter_fields: Vec<_> = groups
            .values()
            .map(|group| (group[0], group.len() > 1))
            .collect();
        let definitions = filter_fields
            .iter()
            .map(|(field, _)| quoted(&field.name))
            .collect::<Vec<_>>()
            .join(",");
        db.execute_batch(&format!(
            "CREATE TABLE rusty_attributes({} INTEGER PRIMARY KEY{}{})",
            quoted(&row_column),
            if definitions.is_empty() { "" } else { "," },
            definitions
        ))
        .map_err(input_sql)?;
        let placeholders = (1..=filter_fields.len() + 1)
            .map(|i| format!("?{i}"))
            .collect::<Vec<_>>()
            .join(",");
        let mut insert = db
            .prepare(&format!(
                "INSERT INTO rusty_attributes VALUES({placeholders})"
            ))
            .map_err(input_sql)?;
        let mut select = db
            .prepare("SELECT seq,json FROM rusty_features ORDER BY seq")
            .map_err(input_sql)?;
        let mut rows = select.query([]).map_err(input_sql)?;
        while let Some(row) = rows.next().map_err(input_sql)? {
            let feature: Value =
                serde_json::from_str(&row.get::<_, String>(1).map_err(input_sql)?)?;
            let mut values = vec![rusqlite::types::Value::Integer(
                row.get(0).map_err(input_sql)?,
            )];
            for (field, ambiguous) in &filter_fields {
                if *ambiguous {
                    values.push(rusqlite::types::Value::Null);
                    continue;
                }
                let value = &feature["properties"][&field.name];
                values.push(match value {
                    Value::Null => rusqlite::types::Value::Null,
                    Value::Bool(v) => rusqlite::types::Value::Integer(i64::from(*v)),
                    Value::Number(v) if v.is_i64() => {
                        rusqlite::types::Value::Integer(v.as_i64().unwrap())
                    }
                    Value::Number(v) if v.is_u64() => rusqlite::types::Value::Text(v.to_string()),
                    Value::Number(v) => rusqlite::types::Value::Real(v.as_f64().unwrap()),
                    Value::String(v) => rusqlite::types::Value::Text(v.clone()),
                    v => rusqlite::types::Value::Text(serde_json::to_string(v)?),
                });
            }
            insert
                .execute(params_from_iter(values))
                .map_err(input_sql)?;
        }
        drop(rows);
        drop(select);
        drop(insert);
    }
    drop(spool);
    db.execute_batch("COMMIT; PRAGMA query_only=ON; PRAGMA trusted_schema=OFF")
        .map_err(input_sql)?;
    // Resolve and validate the expression before any callbacks can receive data.
    if options.where_clause.is_some() {
        if !ambiguous_columns.is_empty() {
            db.authorizer(Some(move |context: AuthContext<'_>| {
                if let AuthAction::Read {
                    table_name: "rusty_attributes",
                    column_name,
                } = context.action
                {
                    if ambiguous_columns.contains(&column_name.to_ascii_lowercase()) {
                        return Authorization::Deny;
                    }
                }
                Authorization::Allow
            }))
            .map_err(input_sql)?;
        }
        let check = format!(
            "SELECT 1 FROM rusty_attributes WHERE ({})",
            options.where_clause.as_deref().unwrap_or("1")
        );
        let mut statement = db
            .prepare(&check)
            .map_err(|e| {
                if e.sqlite_error_code()
                    == Some(rusqlite::ErrorCode::AuthorizationForStatementDenied)
                {
                    data(format!("invalid attribute filter: ambiguous GeoJSON property names {} use case-insensitive SQLite identifiers; rename these properties before filtering them", json!(ambiguous_names)))
                } else {
                    data(format!("invalid attribute filter: {e}"))
                }
            })?;
        statement
            .query([])
            .map_err(input_sql)?
            .next()
            .map_err(input_sql)?;
        drop(statement);
    }
    Ok(GeoJsonSource {
        _file: file,
        db,
        layer,
        row_column,
    })
}

// SQLite file URIs use forward slashes even when canonical Windows paths use
// verbatim drive/UNC syntax. Keep the URI authority empty for UNC paths.
fn immutable_uri(filename: &str, windows: bool) -> String {
    let path = if windows {
        let normalized = filename.replace('\\', "/");
        if let Some(rest) = normalized.strip_prefix("//?/UNC/") {
            format!("//{rest}")
        } else {
            normalized
                .strip_prefix("//?/")
                .unwrap_or(&normalized)
                .to_owned()
        }
    } else {
        filename.to_owned()
    };
    let path = if windows && path.as_bytes().get(1) == Some(&b':') {
        format!("/{path}")
    } else {
        path
    };
    // Slash separators and drive colon remain literal; encode query, fragment,
    // percent and all non-ASCII bytes, including spaces in file names.
    let encoded = path
        .split('/')
        .map(|part| {
            if windows
                && part.len() == 2
                && part.as_bytes()[1] == b':'
                && part.as_bytes()[0].is_ascii_alphabetic()
            {
                part.to_owned()
            } else {
                percent_encoding::utf8_percent_encode(part, percent_encoding::NON_ALPHANUMERIC)
                    .to_string()
            }
        })
        .collect::<Vec<_>>()
        .join("/");
    format!("file://{encoded}?mode=ro&immutable=1")
}

fn validate_declared_geometry(
    layer: &Layer,
    geometry: &Geometry,
    has_z: bool,
) -> Result<(), Error> {
    let actual = match geometry {
        Geometry::Point(_) => "POINT",
        Geometry::MultiPoint(_) => "MULTIPOINT",
        Geometry::LineString(_) => "LINESTRING",
        Geometry::MultiLineString(_) => "MULTILINESTRING",
        Geometry::Polygon(_) => "POLYGON",
        Geometry::MultiPolygon(_) => "MULTIPOLYGON",
    };
    if layer
        .geometry_type
        .as_deref()
        .is_some_and(|declared| declared != "GEOMETRY" && declared != actual)
    {
        return Err(data(
            "GeoPackage geometry type differs from its layer declaration",
        ));
    }
    if (layer.z == 0 && has_z) || (layer.z == 1 && !has_z) || layer.m == 1 {
        return Err(data(
            "GeoPackage geometry dimensions differ from its layer declaration",
        ));
    }
    Ok(())
}

fn open_gpkg(input: &Path, options: &VectorOptions) -> Result<GpkgSource, Error> {
    // immutable avoids journals and source mutations, but ignores an active WAL.
    // Refuse that case so committed features are never silently omitted.
    let canonical = std::fs::canonicalize(input)?;
    let mut wal = canonical.as_os_str().to_os_string();
    wal.push("-wal");
    match std::fs::metadata(Path::new(&wal)) {
        Ok(metadata) if metadata.len() != 0 => return Err(data("GeoPackage has an active WAL; checkpoint and close its writer before immutable ingestion")),
        Ok(_) => {},
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {},
        Err(error) => return Err(error.into()),
    }
    let mut journal = canonical.as_os_str().to_os_string();
    journal.push("-journal");
    match File::open(Path::new(&journal)) {
        Ok(mut journal) => {
            let mut header = [0u8; 28];
            let size = journal.metadata()?.len().min(header.len() as u64) as usize;
            journal.read_exact(&mut header[..size])?;
            // PERSIST mode leaves an inert file with a zeroed header. A live
            // rollback journal requires recovery, which immutable mode skips.
            if header[..size].iter().any(|byte| *byte != 0) {
                return Err(data("GeoPackage has an active rollback journal; recover and close its writer before immutable ingestion"));
            }
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(error.into()),
    }
    let filename = canonical
        .to_str()
        .ok_or_else(|| data("GeoPackage path must be valid UTF-8"))?;
    let uri = immutable_uri(filename, cfg!(windows));
    let db = Connection::open_with_flags(
        uri,
        OpenFlags::SQLITE_OPEN_READ_ONLY
            | OpenFlags::SQLITE_OPEN_URI
            | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .map_err(input_sql)?;
    db.set_db_config(DbConfig::SQLITE_DBCONFIG_DQS_DML, false)
        .map_err(input_sql)?;
    db.set_db_config(DbConfig::SQLITE_DBCONFIG_DEFENSIVE, true)
        .map_err(input_sql)?;
    db.execute_batch("PRAGMA query_only=ON; PRAGMA trusted_schema=OFF; PRAGMA temp_store=FILE; PRAGMA cache_size=-8192").map_err(input_sql)?;
    let mut statement = db.prepare("SELECT g.table_name,g.column_name,g.srs_id,g.z,g.m,g.geometry_type_name FROM gpkg_geometry_columns g JOIN gpkg_contents c ON g.table_name=c.table_name WHERE c.data_type='features' ORDER BY g.table_name,g.column_name").map_err(input_sql)?;
    let metadata = statement
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, i32>(2)?,
                row.get::<_, i32>(3)?,
                row.get::<_, i32>(4)?,
                row.get::<_, String>(5)?,
            ))
        })
        .map_err(input_sql)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(input_sql)?;
    drop(statement);
    let selected = selected_names(
        metadata.iter().map(|entry| entry.0.clone()).collect(),
        options,
    )?;
    let mut layers = Vec::new();
    for (name, geometry, srs, z, m, geometry_type) in metadata {
        if !selected.contains(&name) {
            continue;
        }
        let geometry_type = geometry_type.to_ascii_uppercase();
        if !matches!(
            geometry_type.as_str(),
            "GEOMETRY"
                | "POINT"
                | "MULTIPOINT"
                | "LINESTRING"
                | "MULTILINESTRING"
                | "POLYGON"
                | "MULTIPOLYGON"
        ) {
            return Err(data(format!(
                "unsupported GeoPackage declared geometry type: {geometry_type}"
            )));
        }
        if m == 1 {
            return Err(data(
                "measured GeoPackage geometry is unsupported; retain or explicitly remove M",
            ));
        }
        if !matches!(z, 0..=2) || !matches!(m, 0..=2) {
            return Err(data("invalid GeoPackage Z/M declaration"));
        }
        let mut statement = db
            .prepare("SELECT name,type,pk FROM pragma_table_info(?1) ORDER BY cid")
            .map_err(input_sql)?;
        let columns = statement
            .query_map([&name], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, i32>(2)?,
                ))
            })
            .map_err(input_sql)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(input_sql)?;
        let primary: Vec<_> = columns.iter().filter(|(_, _, pk)| *pk > 0).collect();
        if primary.len() != 1 || !primary[0].1.eq_ignore_ascii_case("INTEGER") {
            return Err(data(format!(
                "GeoPackage layer {name:?} needs one INTEGER primary key"
            )));
        }
        let fid = primary[0].0.clone();
        if !columns.iter().any(|(name, _, _)| name == &geometry) {
            return Err(data(format!(
                "GeoPackage layer {name:?} has no declared geometry column"
            )));
        }
        let fields = columns
            .into_iter()
            .filter(|(name, _, _)| name != &fid && name != &geometry)
            .map(|(name, kind, _)| {
                let kind = kind
                    .split('(')
                    .next()
                    .unwrap_or("")
                    .trim()
                    .to_ascii_uppercase();
                Field {
                    name,
                    kind: match kind.as_str() {
                        "BOOLEAN" => "boolean",
                        "TINYINT" | "SMALLINT" | "MEDIUMINT" | "INT" | "INTEGER" | "BIGINT" => {
                            "integer"
                        }
                        "REAL" | "DOUBLE" | "FLOAT" => "real",
                        "TEXT" | "DATE" | "DATETIME" => "string",
                        _ => "unsupported",
                    }
                    .into(),
                }
            })
            .collect();
        let srs_columns = db
            .prepare("SELECT name FROM pragma_table_info('gpkg_spatial_ref_sys')")
            .map_err(input_sql)?
            .query_map([], |r| r.get::<_, String>(0))
            .map_err(input_sql)?
            .collect::<Result<BTreeSet<_>, _>>()
            .map_err(input_sql)?;
        if options.source_crs.is_none() && srs_columns.contains("epoch") {
            let has_epoch: bool = db
                .query_row(
                    "SELECT epoch IS NOT NULL FROM gpkg_spatial_ref_sys WHERE srs_id=?1",
                    [srs],
                    |row| row.get(0),
                )
                .map_err(input_sql)?;
            if has_epoch {
                return Err(Error::Environment("portable GeoPackage CRS coordinate epochs require native-geospatial; an explicit source-crs override replaces the declared CRS semantics".into()));
            }
        }
        let query = if srs_columns.contains("definition_12_063") {
            "SELECT CASE WHEN definition_12_063 IS NOT NULL AND definition_12_063 <> 'undefined' THEN definition_12_063 ELSE definition END FROM gpkg_spatial_ref_sys WHERE srs_id=?1"
        } else {
            "SELECT definition FROM gpkg_spatial_ref_sys WHERE srs_id=?1"
        };
        let definition: Option<String> = db
            .query_row(query, [srs], |r| r.get(0))
            .map_err(input_sql)?;
        let definition = definition.filter(|v| {
            !v.trim().is_empty() && !v.eq_ignore_ascii_case("undefined") && !matches!(srs, -1 | 0)
        });
        layers.push(Layer {
            name,
            geometry,
            fid,
            srs,
            geometry_type: Some(geometry_type),
            z,
            m,
            definition,
            fields,
        });
    }
    let layers = select_layers(layers, options)?;
    for layer in &layers {
        let mut statement = db
            .prepare(&format!(
                "SELECT 1 FROM {} WHERE ({})",
                quoted(&layer.name),
                options.where_clause.as_deref().unwrap_or("1")
            ))
            .map_err(|e| data(format!("invalid attribute filter: {e}")))?;
        if !statement.readonly() || statement.parameter_count() != 0 {
            return Err(data(
                "invalid attribute filter: expected a read-only expression without parameters",
            ));
        }
        statement
            .query([])
            .map_err(input_sql)?
            .next()
            .map_err(input_sql)?;
    }
    Ok(GpkgSource { db, layers })
}

fn vertex_limit() -> Error {
    data("source feature exceeds maxSourceVertices; explicitly raise the limit or subdivide the source")
}
fn inspect_coordinates(
    value: &Value,
    nesting: usize,
    count: &mut usize,
    limit: usize,
    has_z: &mut bool,
) -> Result<(), Error> {
    let array = value
        .as_array()
        .ok_or_else(|| data("geometry coordinates must be arrays"))?;
    if nesting > 0 {
        for value in array {
            inspect_coordinates(value, nesting - 1, count, limit, has_z)?;
        }
    } else {
        if !matches!(array.len(), 2 | 3) {
            return Err(data("coordinates must have exactly two or three ordinates; measured geometry is unsupported"));
        }
        *count = count.checked_add(1).ok_or_else(vertex_limit)?;
        if *count > limit {
            return Err(vertex_limit());
        }
        *has_z |= array.len() == 3;
        if !array.iter().all(|v| v.as_f64().is_some_and(f64::is_finite)) {
            return Err(data("coordinates must be finite XYZ"));
        }
    }
    Ok(())
}
fn decode_geojson(value: &Value, limit: usize) -> Result<Option<(Geometry, bool)>, Error> {
    if value.is_null() {
        return Ok(None);
    }
    let nesting = match value["type"].as_str() {
        Some("Point") => 0,
        Some("MultiPoint" | "LineString") => 1,
        Some("MultiLineString" | "Polygon") => 2,
        Some("MultiPolygon") => 3,
        Some(name) => return Err(data(format!("unsupported geometry: {name}"))),
        None => return Err(data("geometry needs a type")),
    };
    let mut count = 0;
    let mut has_z = false;
    if nesting == 0 && value["coordinates"].as_array().is_some_and(Vec::is_empty) {
        return Ok(None);
    }
    inspect_coordinates(
        &value["coordinates"],
        nesting,
        &mut count,
        limit,
        &mut has_z,
    )?;
    if count == 0 {
        return Ok(None);
    }
    let bytes = serde_json::to_vec(value)?;
    let mut builder = GeometryBuilder::new(limit);
    geozero::geojson::GeoJson(std::str::from_utf8(&bytes).unwrap())
        .process_geom(&mut builder)
        .map_err(|e| data(format!("invalid GeoJSON geometry: {e}")))?;
    Ok(Some((builder.finish()?, has_z)))
}

struct WkbScan<'a> {
    bytes: &'a [u8],
    position: usize,
    vertices: usize,
    limit: usize,
    has_z: bool,
}
impl WkbScan<'_> {
    fn take<const N: usize>(&mut self) -> Result<[u8; N], Error> {
        let end = self
            .position
            .checked_add(N)
            .ok_or_else(|| data("invalid WKB length"))?;
        let value = self
            .bytes
            .get(self.position..end)
            .ok_or_else(|| data("truncated WKB geometry"))?
            .try_into()
            .unwrap();
        self.position = end;
        Ok(value)
    }
    fn int(&mut self, little: bool) -> Result<u32, Error> {
        let bytes = self.take()?;
        Ok(if little {
            u32::from_le_bytes(bytes)
        } else {
            u32::from_be_bytes(bytes)
        })
    }
    fn float(&mut self, little: bool) -> Result<f64, Error> {
        let bytes = self.take()?;
        Ok(if little {
            f64::from_le_bytes(bytes)
        } else {
            f64::from_be_bytes(bytes)
        })
    }
    fn point(&mut self, little: bool, z: bool, allow_empty: bool) -> Result<(), Error> {
        let x = self.float(little)?;
        let y = self.float(little)?;
        let height = if z { self.float(little)? } else { 0. };
        if allow_empty && x.is_nan() && y.is_nan() && (!z || height.is_nan()) {
            return Ok(());
        }
        if ![x, y, height].into_iter().all(f64::is_finite) {
            return Err(data("coordinates must be finite XYZ"));
        }
        self.vertices = self.vertices.checked_add(1).ok_or_else(vertex_limit)?;
        if self.vertices > self.limit {
            return Err(vertex_limit());
        }
        Ok(())
    }
    fn count(&mut self, little: bool) -> Result<usize, Error> {
        let size = self.int(little)? as usize;
        if size > self.limit {
            return Err(vertex_limit());
        }
        Ok(size)
    }
    fn geometry(&mut self, expected: Option<(u32, bool)>, depth: usize) -> Result<(), Error> {
        if depth > 64 {
            return Err(data("WKB geometry nesting limit exceeded"));
        }
        let endian = self.take::<1>()?[0];
        if endian > 1 {
            return Err(data("invalid WKB byte order"));
        }
        let little = endian == 1;
        let kind = self.int(little)?;
        let dimensions = kind / 1000;
        if matches!(dimensions, 2 | 3) {
            return Err(data(
                "measured geometry is unsupported; retain or explicitly remove M",
            ));
        }
        if dimensions > 1 {
            return Err(data("unsupported WKB type or extended encoding"));
        }
        let kind = kind % 1000;
        let z = dimensions == 1;
        if expected.is_some_and(|expected| expected != (kind, z)) {
            return Err(data("invalid WKB multi-geometry member type or dimensions"));
        }
        self.has_z |= z;
        match kind {
            1 => self.point(little, z, true)?,
            2 => {
                let size = self.count(little)?;
                for _ in 0..size {
                    self.point(little, z, false)?;
                }
            }
            3 => {
                let rings = self.count(little)?;
                for _ in 0..rings {
                    let size = self.count(little)?;
                    for _ in 0..size {
                        self.point(little, z, false)?;
                    }
                }
            }
            4..=6 => {
                let size = self.count(little)?;
                for _ in 0..size {
                    self.geometry(Some((kind - 3, z)), depth + 1)?;
                }
            }
            _ => return Err(data(format!("unsupported WKB geometry type: {kind}"))),
        }
        Ok(())
    }
}

fn decode_gpkg(blob: &[u8], srs: i32, limit: usize) -> Result<Option<(Geometry, bool)>, Error> {
    if blob.len() < 8 || &blob[..2] != b"GP" {
        return Err(data("invalid GeoPackage geometry header"));
    }
    if blob[2] != 0 || blob[3] & 0xe0 != 0 {
        return Err(data("unsupported extended GeoPackage geometry header"));
    }
    let flags = blob[3];
    let little = flags & 1 != 0;
    let raw_srs: [u8; 4] = blob[4..8].try_into().unwrap();
    let raw_srs = if little {
        i32::from_le_bytes(raw_srs)
    } else {
        i32::from_be_bytes(raw_srs)
    };
    if raw_srs != srs {
        return Err(data(
            "GeoPackage geometry SRS differs from its layer declaration",
        ));
    }
    let envelope = (flags >> 1) & 7;
    let envelope_bytes = match envelope {
        0 => 0,
        1 => 32,
        2 => 48,
        3 | 4 => {
            return Err(data(
                "measured GeoPackage geometry envelopes are unsupported",
            ))
        }
        _ => return Err(data("invalid GeoPackage envelope indicator")),
    };
    let offset = 8 + envelope_bytes;
    let wkb = blob
        .get(offset..)
        .ok_or_else(|| data("truncated GeoPackage envelope"))?;
    for bounds in blob[8..offset].as_chunks::<16>().0 {
        let minimum: [u8; 8] = bounds[..8].try_into().unwrap();
        let maximum: [u8; 8] = bounds[8..].try_into().unwrap();
        let (minimum, maximum) = if little {
            (f64::from_le_bytes(minimum), f64::from_le_bytes(maximum))
        } else {
            (f64::from_be_bytes(minimum), f64::from_be_bytes(maximum))
        };
        if !minimum.is_finite() || !maximum.is_finite() || minimum > maximum {
            return Err(data("invalid GeoPackage envelope bounds"));
        }
    }
    let mut scan = WkbScan {
        bytes: wkb,
        position: 0,
        vertices: 0,
        limit,
        has_z: false,
    };
    scan.geometry(None, 0)?;
    if scan.position != wkb.len() {
        return Err(data("trailing bytes after GeoPackage WKB geometry"));
    }
    let empty = flags & 0x10 != 0;
    if empty != (scan.vertices == 0) || (empty && envelope != 0) {
        return Err(data("invalid GeoPackage empty geometry flag or envelope"));
    }
    if envelope == 2 && !scan.has_z {
        return Err(data("GeoPackage XYZ envelope requires Z geometry"));
    }
    if scan.vertices == 0 {
        return Ok(None);
    }
    let mut builder = GeometryBuilder::new(limit);
    geozero::wkb::process_wkb_geom(&mut &wkb[..], &mut builder)
        .map_err(|e| data(format!("invalid GeoPackage WKB geometry: {e}")))?;
    Ok(Some((builder.finish()?, scan.has_z)))
}

// geozero drives this owned-value builder for both formats. No native handles,
// 2D geo-types conversion, guessed dimension, or unbounded advertised allocation.
enum Part {
    Point(Vec<Point>),
    Points(Vec<Point>),
    Line(Vec<Point>),
    Lines(Vec<Vec<Point>>),
    Polygon(Vec<Vec<Point>>),
    Polygons(Vec<Vec<Vec<Point>>>),
}
struct GeometryBuilder {
    stack: Vec<Part>,
    geometry: Option<Geometry>,
    limit: usize,
    count: usize,
}
impl GeometryBuilder {
    fn new(limit: usize) -> Self {
        Self {
            stack: Vec::new(),
            geometry: None,
            limit,
            count: 0,
        }
    }
    fn finish(self) -> Result<Geometry, Error> {
        if !self.stack.is_empty() {
            return Err(data("unclosed geometry"));
        }
        self.geometry.ok_or_else(|| data("empty geometry"))
    }
    fn error(message: &str) -> geozero::error::GeozeroError {
        geozero::error::GeozeroError::Geometry(message.into())
    }
    fn begin(&mut self, part: Part, size: usize) -> geozero::error::Result<()> {
        if size > self.limit || self.stack.len() > 8 {
            return Err(Self::error("geometry exceeds source limits"));
        }
        self.stack.push(part);
        Ok(())
    }
    fn end(&mut self) -> geozero::error::Result<()> {
        let geometry = match self
            .stack
            .pop()
            .ok_or_else(|| Self::error("unexpected geometry end"))?
        {
            Part::Point(p) if p.len() == 1 => Geometry::Point(p[0]),
            Part::Point(_) => return Err(Self::error("empty or invalid point")),
            Part::Points(p) => Geometry::MultiPoint(p),
            Part::Line(p) => Geometry::LineString(p),
            Part::Lines(p) => Geometry::MultiLineString(p),
            Part::Polygon(p) => Geometry::Polygon(p),
            Part::Polygons(p) => Geometry::MultiPolygon(p),
        };
        match (self.stack.last_mut(), geometry) {
            (None, geometry) => self.geometry = Some(geometry),
            (Some(Part::Lines(parts) | Part::Polygon(parts)), Geometry::LineString(points)) => {
                parts.push(points)
            }
            (Some(Part::Polygons(parts)), Geometry::Polygon(rings)) => parts.push(rings),
            _ => return Err(Self::error("invalid nested geometry")),
        }
        Ok(())
    }
}
impl GeomProcessor for GeometryBuilder {
    fn dimensions(&self) -> CoordDimensions {
        CoordDimensions::xyzm()
    }
    fn xy(&mut self, x: f64, y: f64, idx: usize) -> geozero::error::Result<()> {
        self.coordinate(x, y, None, None, None, None, idx)
    }
    fn coordinate(
        &mut self,
        x: f64,
        y: f64,
        z: Option<f64>,
        m: Option<f64>,
        _: Option<f64>,
        _: Option<u64>,
        _: usize,
    ) -> geozero::error::Result<()> {
        if m.is_some() {
            return Err(Self::error("measured geometry is unsupported"));
        }
        if matches!(self.stack.last(), Some(Part::Points(_)))
            && x.is_nan()
            && y.is_nan()
            && z.is_none_or(f64::is_nan)
        {
            return Ok(());
        }
        self.count += 1;
        if self.count > self.limit {
            return Err(Self::error("source feature exceeds maxSourceVertices"));
        }
        let point = [x, y, z.unwrap_or(0.)];
        if !point.into_iter().all(f64::is_finite) {
            return Err(Self::error("coordinates must be finite XYZ"));
        }
        match self.stack.last_mut() {
            Some(Part::Point(points) | Part::Points(points) | Part::Line(points)) => {
                points.push(point)
            }
            _ => return Err(Self::error("coordinate outside a path")),
        }
        Ok(())
    }
    fn point_begin(&mut self, _: usize) -> geozero::error::Result<()> {
        self.begin(Part::Point(Vec::new()), 1)
    }
    fn point_end(&mut self, _: usize) -> geozero::error::Result<()> {
        self.end()
    }
    fn multipoint_begin(&mut self, size: usize, _: usize) -> geozero::error::Result<()> {
        self.begin(Part::Points(Vec::new()), size)
    }
    fn multipoint_end(&mut self, _: usize) -> geozero::error::Result<()> {
        self.end()
    }
    fn linestring_begin(&mut self, _: bool, size: usize, _: usize) -> geozero::error::Result<()> {
        self.begin(Part::Line(Vec::new()), size)
    }
    fn linestring_end(&mut self, _: bool, _: usize) -> geozero::error::Result<()> {
        self.end()
    }
    fn multilinestring_begin(&mut self, size: usize, _: usize) -> geozero::error::Result<()> {
        self.begin(Part::Lines(Vec::new()), size)
    }
    fn multilinestring_end(&mut self, _: usize) -> geozero::error::Result<()> {
        self.end()
    }
    fn polygon_begin(&mut self, _: bool, size: usize, _: usize) -> geozero::error::Result<()> {
        self.begin(Part::Polygon(Vec::new()), size)
    }
    fn polygon_end(&mut self, _: bool, _: usize) -> geozero::error::Result<()> {
        self.end()
    }
    fn multipolygon_begin(&mut self, size: usize, _: usize) -> geozero::error::Result<()> {
        self.begin(Part::Polygons(Vec::new()), size)
    }
    fn multipolygon_end(&mut self, _: usize) -> geozero::error::Result<()> {
        self.end()
    }
    fn geometrycollection_begin(&mut self, _: usize, _: usize) -> geozero::error::Result<()> {
        Err(Self::error("unsupported geometry: GeometryCollection"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use geozero::ToWkb;
    use std::fs;

    fn local_options() -> VectorOptions {
        VectorOptions {
            source_crs: Some("local".into()),
            ..VectorOptions::default()
        }
    }
    fn distance(a: Point, b: Point) -> f64 {
        a.into_iter()
            .zip(b)
            .map(|(x, y)| (x - y).powi(2))
            .sum::<f64>()
            .sqrt()
    }
    fn source(path: &Path, features: Value) {
        fs::write(
            path,
            serde_json::to_vec(
                &json!({"type":"FeatureCollection","features":features,"name":"source layer"}),
            )
            .unwrap(),
        )
        .unwrap();
    }
    fn read(reader: &mut Reader, options: &VectorOptions) -> (Vec<Feature>, Vec<(Value, bool)>) {
        let mut features = Vec::new();
        let mut reports = Vec::new();
        reader
            .read(
                options,
                |feature| {
                    features.push(feature);
                    Ok(())
                },
                |value, invalid| {
                    reports.push((value, invalid));
                    Ok(())
                },
            )
            .unwrap();
        (features, reports)
    }
    fn feature(id: Value, properties: Value, geometry: Value) -> Value {
        json!({"type":"Feature","id":id,"properties":properties,"geometry":geometry})
    }
    fn point(x: f64) -> Value {
        json!({"type":"Point","coordinates":[x,2.,3.]})
    }
    fn gpkg_blob(geometry: &Value, srs: i32) -> Vec<u8> {
        let wkb = geozero::geojson::GeoJson(&geometry.to_string())
            .to_wkb(CoordDimensions::xyz())
            .unwrap();
        let mut blob = vec![b'G', b'P', 0, 1];
        blob.extend(srs.to_le_bytes());
        blob.extend(wkb);
        blob
    }
    fn gpkg(path: &Path) {
        let db = Connection::open(path).unwrap();
        db.execute_batch("CREATE TABLE gpkg_spatial_ref_sys(srs_id INTEGER PRIMARY KEY,definition TEXT); INSERT INTO gpkg_spatial_ref_sys VALUES(4326,'EPSG:4326');
            CREATE TABLE gpkg_contents(table_name TEXT PRIMARY KEY,data_type TEXT); CREATE TABLE gpkg_geometry_columns(table_name TEXT PRIMARY KEY,column_name TEXT,srs_id INTEGER,z INTEGER,m INTEGER,geometry_type_name TEXT);
            CREATE TABLE roads(fid INTEGER PRIMARY KEY,geom BLOB,large INTEGER,label TEXT,flag BOOLEAN,payload BLOB,created DATE);
            CREATE TABLE sites(fid INTEGER PRIMARY KEY,geom BLOB,large INTEGER,label TEXT,flag BOOLEAN,payload BLOB,created DATE);
            INSERT INTO gpkg_contents VALUES('roads','features'),('sites','features'); INSERT INTO gpkg_geometry_columns VALUES('roads','geom',4326,1,0,'POINT'),('sites','geom',4326,1,0,'POINT');").unwrap();
        for (table, id, geometry, large, label, flag) in [
            (
                "roads",
                7,
                Some(gpkg_blob(&point(1.), 4326)),
                Some(1152921504606846979i64),
                "x; DROP TABLE roads",
                1,
            ),
            ("roads", 8, None, None, "no geometry", 0),
            (
                "sites",
                9,
                Some(gpkg_blob(&point(5.), 4326)),
                Some(1),
                "site",
                0,
            ),
        ] {
            db.execute(
                &format!("INSERT INTO {table} VALUES(?1,?2,?3,?4,?5,NULL,'2026-10-09')"),
                params![id, geometry, large, label, flag],
            )
            .unwrap();
        }
    }
    fn gpkg_options() -> VectorOptions {
        VectorOptions {
            fields: vec![
                "large".into(),
                "label".into(),
                "flag".into(),
                "created".into(),
            ],
            layers: vec!["roads".into()],
            ..local_options()
        }
    }

    #[test]
    fn geojson_streams_original_ids_scalars_lists_and_late_metadata() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("input with spaces.geojson");
        source(
            &path,
            json!([
                feature(
                    json!("a\"b"),
                    json!({"large":1152921504606846979i64,"unsigned":u64::MAX,"flag":true,"list":[1,null,"a",{"x":2}],"null":null}),
                    point(1.)
                ),
                json!({"type":"Feature","properties":{"flag":false},"geometry":point(4.)}),
                feature(Value::Null, json!({"flag":null}), Value::Null),
            ]),
        );
        let options = VectorOptions {
            list_fields: "json".into(),
            ..local_options()
        };
        let before = fs::read(&path).unwrap();
        let mut reader = Reader::new(&path, &options, None).unwrap();
        let (features, reports) = read(&mut reader, &options);
        assert_eq!(reader.driver, "GeoJSON");
        assert_eq!(reader.without_geometry, 1);
        assert_eq!(features.len(), 2);
        assert_eq!(reports.len(), 1);
        assert!(!reports[0].1);
        assert_eq!(features[0].source_id(), &json!("\"a\\\"b\""));
        assert_eq!(features[1].source_id(), &json!("1"));
        assert_eq!(features[0].layer(), "source layer");
        assert_eq!(
            features[0].properties["large"].as_i64(),
            Some(1152921504606846979)
        );
        assert_eq!(features[0].properties["unsigned"].as_u64(), Some(u64::MAX));
        assert_eq!(features[0].properties["flag"], true);
        assert!(features[0].properties["null"].is_null());
        assert!(!features[1].properties.contains_key("large"));
        assert_eq!(
            serde_json::from_str::<Value>(features[0].properties["list"].as_str().unwrap())
                .unwrap(),
            json!([1,null,"a",{"x":2}])
        );
        assert_eq!(reader.schemas["large"], "integer");
        assert_eq!(reader.schemas["flag"], "boolean");
        assert_eq!(reader.layer_reports[0]["jsonFields"], json!(["list"]));
        assert_eq!(fs::read(path).unwrap(), before);
    }

    #[test]
    fn geojson_filters_exact_signed_int64_on_excluded_fields_before_geometry_validation() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("input.geojson");
        source(
            &path,
            json!([
                feature(
                    json!(1),
                    json!({"large":1152921504606846979i64,"seq":7,"label":"x; -- /* '"}),
                    point(1.)
                ),
                feature(
                    json!(2),
                    json!({"large":1152921504606846980i64,"seq":8,"label":"other"}),
                    json!({"type":"GeometryCollection","geometries":[]})
                ),
            ]),
        );
        let options = VectorOptions {
            where_clause: Some(
                "large = 1152921504606846979 AND seq IN (7,9) AND label = 'x; -- /* '''".into(),
            ),
            fields: vec!["label".into()],
            ..local_options()
        };
        let mut reader = Reader::new(&path, &options, None).unwrap();
        let (features, reports) = read(&mut reader, &options);
        assert_eq!(features.len(), 1);
        assert!(reports.is_empty());
        assert_eq!(features[0].source_id(), &json!("1"));
        assert!(!features[0].properties.contains_key("large"));
        assert!(!features[0].properties.contains_key("seq"));
    }

    #[test]
    fn geojson_case_distinct_properties_allow_unrelated_and_constant_filters() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("input.geojson");
        source(
            &path,
            json!([
                feature(
                    json!(1),
                    json!({"keep":1,"A":1,"a":2,"ROWID":7,"rowid":8,
                    "TRUE":5,"true":6,"with space":3,"quo\"te":4,"select":9,"Ä":10,"ä":11}),
                    point(1.)
                ),
                feature(json!(2), json!({"keep":0,"A":3,"a":4}), point(2.))
            ]),
        );
        let before = fs::read(&path).unwrap();
        for (expression, count) in [
            ("KEEP = 1", 1),
            ("1", 2),
            ("0", 0),
            (
                "'A' = 'A' AND 'rowid' = 'rowid' AND 'true' = 'true' AND keep = 1",
                1,
            ),
            (r#""with space" = 3 AND "quo""te" = 4 AND "select" = 9"#, 1),
            (r#""Ä" = 10 AND "ä" = 11"#, 1),
        ] {
            let options = VectorOptions {
                where_clause: Some(expression.into()),
                fields: vec!["keep".into()],
                ..local_options()
            };
            let mut reader = Reader::new(&path, &options, None).unwrap();
            let (features, reports) = read(&mut reader, &options);
            assert_eq!(features.len(), count, "{expression}");
            assert!(reports.is_empty());
            for feature in features {
                assert!(!feature.properties.contains_key("A"));
                assert!(!feature.properties.contains_key("a"));
            }
        }
        let options = VectorOptions {
            where_clause: Some("keep = 1".into()),
            ..local_options()
        };
        let mut reader = Reader::new(&path, &options, None).unwrap();
        let features = read(&mut reader, &options).0;
        assert_eq!(features[0].properties["A"], 1);
        assert_eq!(features[0].properties["a"], 2);
        assert_eq!(features[0].properties["ROWID"], 7);
        assert_eq!(features[0].properties["rowid"], 8);
        assert_eq!(features[0].properties["TRUE"], 5);
        assert_eq!(features[0].properties["true"], 6);
        assert_eq!(fs::read(&path).unwrap(), before);
    }

    #[test]
    fn geojson_filters_refuse_ambiguous_identifiers_without_rowid_or_literal_fallback() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("input.geojson");
        source(
            &path,
            json!([feature(
                json!(1),
                json!({"keep":1,"A":1,"a":2,
                "ROWID":7,"rowid":8,"TRUE":5,"true":6}),
                point(1.)
            )]),
        );
        for expression in [
            "A = 1",
            "a = 2",
            r#""A" = 1"#,
            r#""a" = 2"#,
            "[a] = 2",
            "`A` = 1",
            "rusty_attributes.a = 2",
            "ROWID = 7",
            r#""rowid" = 8"#,
            "TRUE = 5",
            r#""true" = 6"#,
        ] {
            let options = VectorOptions {
                where_clause: Some(expression.into()),
                fields: vec!["keep".into()],
                ..local_options()
            };
            let error = Reader::new(&path, &options, None).err().unwrap();
            assert!(
                error
                    .to_string()
                    .contains("ambiguous GeoJSON property names"),
                "{expression}: {error}"
            );
            assert!(error.to_string().contains("rename these properties"));
        }
    }

    #[test]
    fn filters_refuse_multiple_statements_escapes_parameters_and_unknown_quoted_fields() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("input.geojson");
        source(
            &path,
            json!([feature(json!(1), json!({"name":"x"}), point(1.))]),
        );
        for expression in [
            "1); DROP TABLE rusty_features; --",
            "1; SELECT 1",
            "1 -- comment",
            "1/*comment*/",
            "name=?",
            "name=:name",
            "name='unclosed",
            "(name='x'",
            "\"missing\" = 'x'",
            "load_extension('malicious')",
            "name='x' UNION SELECT 1",
        ] {
            let options = VectorOptions {
                where_clause: Some(expression.into()),
                ..local_options()
            };
            assert!(Reader::new(&path, &options, None).is_err(), "{expression}");
        }
        let options = VectorOptions {
            where_clause: Some("name = 'x; DROP TABLE roads'".into()),
            ..local_options()
        };
        let mut reader = Reader::new(&path, &options, None).unwrap();
        assert!(read(&mut reader, &options).0.is_empty());
    }

    #[test]
    fn selected_geojson_fields_and_layers_are_checked_without_coercing_unsupported_fields() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("input.geojson");
        source(
            &path,
            json!([feature(
                json!(1),
                json!({"name":"a","list":[1,2],"_source_id":"reserved"}),
                point(1.)
            )]),
        );
        let options = VectorOptions {
            fields: vec!["name".into()],
            ..local_options()
        };
        let mut reader = Reader::new(&path, &options, None).unwrap();
        assert_eq!(read(&mut reader, &options).0.len(), 1);
        for options in [
            VectorOptions {
                fields: vec!["missing".into()],
                ..local_options()
            },
            VectorOptions {
                layers: vec!["missing".into()],
                ..local_options()
            },
            VectorOptions {
                layers: vec!["source layer".into(), "source layer".into()],
                ..local_options()
            },
        ] {
            assert!(Reader::new(&path, &options, None).is_err());
        }
        let mut reader = Reader::new(&path, &local_options(), None).unwrap();
        assert!(reader
            .read(&local_options(), |_| Ok(()), |_, _| Ok(()))
            .is_err());
    }

    #[test]
    fn unsigned_filter_refusal_is_explicit_and_default_geojson_crs_places_z_in_metres() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("input.geojson");
        source(
            &path,
            json!([feature(json!(1), json!({"large":u64::MAX}), point(1.))]),
        );
        let options = VectorOptions {
            where_clause: Some("large > 0".into()),
            ..local_options()
        };
        assert!(matches!(
            Reader::new(&path, &options, None),
            Err(Error::Environment(_))
        ));
        source(
            &path,
            json!([feature(
                json!(1),
                json!({}),
                json!({"type":"Point","coordinates":[153.,-27.,123.]})
            )]),
        );
        let options = VectorOptions::default();
        let mut reader = Reader::new(&path, &options, None).unwrap();
        let (features, _) = read(&mut reader, &options);
        let expected =
            crate::georef::geodetic_to_ecef(crate::georef::Cartographic::new(153., -27., 123.));
        assert!(distance(reader.frame.as_ref().unwrap().anchor, expected) < 1e-6);
        assert_eq!(features[0].geometry, Geometry::Point([0., 0., -0.]));
        assert_eq!(
            reader.layer_reports[0]["heightMode"],
            "GeoJSON ellipsoidal metres"
        );
    }

    #[test]
    fn geometry_decoders_preserve_every_supported_xyz_variant_and_hole() {
        let ring = json!([[0., 0., 10.], [2., 0., 20.], [2., 2., 30.], [0., 0., 10.]]);
        let hole = json!([
            [0.2, 0.2, 11.],
            [0.5, 0.2, 12.],
            [0.5, 0.5, 13.],
            [0.2, 0.2, 11.]
        ]);
        for value in [
            point(1.),
            json!({"type":"MultiPoint","coordinates":[[1.,2.,3.],[4.,5.,6.]]}),
            json!({"type":"LineString","coordinates":[[1.,2.,3.],[4.,5.,6.]]}),
            json!({"type":"MultiLineString","coordinates":[[[1.,2.,3.],[4.,5.,6.]],[[7.,8.,9.],[10.,11.,12.]]]}),
            json!({"type":"Polygon","coordinates":[ring,hole]}),
            json!({"type":"MultiPolygon","coordinates":[[ring,hole],[ring]]}),
        ] {
            let expected: Geometry = serde_json::from_value(value.clone()).unwrap();
            let json_geometry = decode_geojson(&value, 100).unwrap().unwrap();
            let wkb_geometry = decode_gpkg(&gpkg_blob(&value, 4326), 4326, 100)
                .unwrap()
                .unwrap();
            assert_eq!(json_geometry, (expected.clone(), true));
            assert_eq!(wkb_geometry, (expected, true));
        }
        assert!(decode_geojson(
            &json!({"type":"MultiPoint","coordinates":[[1,2,3],[4,5,6]]}),
            1
        )
        .is_err());
        assert!(decode_gpkg(&gpkg_blob(&point(1.), 4326), 4326, 0).is_err());
        for geometry in [
            json!({"type":"Point","coordinates":[1]}),
            json!({"type":"Point","coordinates":[1,2,3,4]}),
            json!({"type":"GeometryCollection","geometries":[]}),
        ] {
            assert!(decode_geojson(&geometry, 100).is_err());
        }
    }

    #[test]
    fn gpkg_headers_check_endianness_envelopes_m_extended_lengths_srs_and_empty_points() {
        let mut blob = vec![b'G', b'P', 0, 0];
        blob.extend(4326_i32.to_be_bytes());
        blob.push(0);
        blob.extend(1001_u32.to_be_bytes());
        for ordinate in [1_f64, 2., 3.] {
            blob.extend(ordinate.to_be_bytes());
        }
        assert_eq!(
            decode_gpkg(&blob, 4326, 10).unwrap().unwrap(),
            (Geometry::Point([1., 2., 3.]), true)
        );
        assert!(decode_gpkg(&blob, 3857, 10).is_err());
        for flags in [0x20, 0x40, 0x80, 0x0a, 0x06, 0x08] {
            let mut invalid = blob.clone();
            invalid[3] = flags;
            assert!(decode_gpkg(&invalid, 4326, 10).is_err());
        }
        let mut with_envelope = blob[..8].to_vec();
        with_envelope[3] = 2;
        for ordinate in [1_f64, 1., 2., 2.] {
            with_envelope.extend(ordinate.to_be_bytes());
        }
        with_envelope.extend(&blob[8..]);
        assert!(decode_gpkg(&with_envelope, 4326, 10).is_ok());
        for length in 0..blob.len() {
            assert!(
                decode_gpkg(&blob[..length], 4326, 10).is_err(),
                "truncated at {length}"
            );
        }
        let mut trailing = blob.clone();
        trailing.push(0);
        assert!(decode_gpkg(&trailing, 4326, 10).is_err());
        for kind in [2001_u32, 3001, 4001, 7, 0x80000001] {
            let mut invalid = blob.clone();
            invalid[9..13].copy_from_slice(&kind.to_be_bytes());
            assert!(decode_gpkg(&invalid, 4326, 10).is_err());
        }
        let mut empty = blob[..13].to_vec();
        empty[3] = 0x10;
        for _ in 0..3 {
            empty.extend(f64::NAN.to_be_bytes());
        }
        assert!(decode_gpkg(&empty, 4326, 10).unwrap().is_none());
        let mut false_empty = blob.clone();
        false_empty[3] = 0x10;
        assert!(decode_gpkg(&false_empty, 4326, 10).is_err());
    }

    #[test]
    fn gpkg_readonly_filters_typing_selection_and_nullable_fields_preserve_source() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("source with spaces.gpkg");
        gpkg(&path);
        let before = fs::read(&path).unwrap();
        let mut options = gpkg_options();
        options.where_clause =
            Some("large = 1152921504606846979 AND label = 'x; DROP TABLE roads'".into());
        options.fields.retain(|v| v != "large");
        let mut reader = Reader::new(&path, &options, None).unwrap();
        assert!(
            matches!(&reader.source,Source::Gpkg(source) if source.db.is_readonly(rusqlite::MAIN_DB).unwrap())
        );
        let (features, reports) = read(&mut reader, &options);
        assert_eq!(features.len(), 1);
        assert!(reports.is_empty());
        assert_eq!(features[0].source_id(), &json!("7"));
        assert_eq!(features[0].layer(), "roads");
        assert!(!features[0].properties.contains_key("large"));
        assert_eq!(features[0].properties["flag"], true);
        assert_eq!(features[0].properties["created"], "2026-10-09");
        assert_eq!(fs::read(&path).unwrap(), before);
        let mut reader = Reader::new(&path, &gpkg_options(), None).unwrap();
        let (features, reports) = read(&mut reader, &gpkg_options());
        assert_eq!(
            features[0].properties["large"].as_i64(),
            Some(1152921504606846979)
        );
        assert_eq!(reader.without_geometry, 1);
        assert_eq!(reports[0].0["sourceId"], "8");
        let options = VectorOptions {
            layers: Vec::new(),
            ..gpkg_options()
        };
        assert!(Reader::new(&path, &options, None).is_err());
        let options = VectorOptions {
            all_layers: true,
            ..options
        };
        let mut reader = Reader::new(&path, &options, None).unwrap();
        assert_eq!(read(&mut reader, &options).0.len(), 2);
        assert_eq!(reader.layer_reports.len(), 2);
        assert!(!dir.path().join("source with spaces.gpkg-journal").exists());
    }

    #[test]
    fn gpkg_z_and_crs_rules_refuse_inference_and_overrides_are_explicit() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("source.gpkg");
        gpkg(&path);
        let mut options = gpkg_options();
        options.source_crs = None;
        let mut reader = Reader::new(&path, &options, None).unwrap();
        let (features, reports) = read(&mut reader, &options);
        assert!(features.is_empty());
        assert!(reports.iter().any(
            |(v, invalid)| *invalid && v["reason"].as_str().unwrap().contains("height-offset")
        ));
        options.height_offset = Some(5.);
        let mut reader = Reader::new(&path, &options, None).unwrap();
        let (features, _) = read(&mut reader, &options);
        assert_eq!(features.len(), 1);
        let expected =
            crate::georef::geodetic_to_ecef(crate::georef::Cartographic::new(1., 2., 8.));
        assert!(distance(reader.frame.as_ref().unwrap().anchor, expected) < 1e-6);
        let db = Connection::open(&path).unwrap();
        db.execute("UPDATE gpkg_spatial_ref_sys SET definition='undefined'", [])
            .unwrap();
        drop(db);
        let mut reader = Reader::new(&path, &options, None).unwrap();
        assert!(reader.read(&options, |_| Ok(()), |_, _| Ok(())).is_err());
        options.source_crs = Some("EPSG:4326".into());
        let mut reader = Reader::new(&path, &options, None).unwrap();
        assert_eq!(read(&mut reader, &options).0.len(), 1);
        options.source_crs = Some("local".into());
        let mut reader = Reader::new(&path, &options, None).unwrap();
        assert!(reader.read(&options, |_| Ok(()), |_, _| Ok(())).is_err());
    }

    #[test]
    fn per_feature_invalid_reporting_does_not_drop_valid_rows_or_reused_frame() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("input.geojson");
        source(
            &path,
            json!([
                feature(
                    json!(1),
                    json!({}),
                    json!({"type":"Point","coordinates":[1]})
                ),
                feature(json!(2), json!({}), point(1.)),
                feature(
                    json!(3),
                    json!({}),
                    json!({"type":"LineString","coordinates":[]})
                ),
            ]),
        );
        let options = local_options();
        let frame = Frame::local([100., 200., 300.]);
        let mut reader = Reader::new(&path, &options, Some(frame.clone())).unwrap();
        let (features, reports) = read(&mut reader, &options);
        assert_eq!(features.len(), 1);
        assert_eq!(reports.len(), 2);
        assert!(reports[0].1);
        assert!(!reports[1].1);
        assert_eq!(
            features[0].geometry,
            Geometry::Point(frame.project([1., 2., 3.]))
        );
        assert_eq!(reader.frame.unwrap().anchor, frame.anchor);
    }
    #[test]
    fn malformed_feature_members_and_trailing_json_are_not_silently_discarded() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("input.geojson");
        source(
            &path,
            json!([
                feature(json!(1), json!(["lost property"]), point(1.)),
                json!({"type":"Feature","properties":{}}),
                feature(json!(3), json!({"_RUSTY_ROW":1}), point(3.))
            ]),
        );
        let options = local_options();
        let mut reader = Reader::new(&path, &options, None).unwrap();
        let (features, reports) = read(&mut reader, &options);
        assert_eq!(features.len(), 1);
        assert_eq!(reports.iter().filter(|(_, invalid)| *invalid).count(), 2);
        let filtered = VectorOptions {
            where_clause: Some("_RUSTY_ROW = 1".into()),
            ..options
        };
        let mut reader = Reader::new(&path, &filtered, None).unwrap();
        assert_eq!(read(&mut reader, &filtered).0.len(), 1);
        for contents in [
            r#"{"type":"FeatureCollection","features":[]} {}"#,
            r#"{"type":"FeatureCollection","features":[],"features":[]}"#,
            r#"{"type":"Feature","id":1,"id":2,"properties":{},"geometry":null}"#,
            r#"{"type":"FeatureCollection","features":[{"type":"Feature","properties":{"x":1,"x":2},"geometry":null}]}"#,
            r#"{"type":"Point","coordinates":[1,2],"coordinates":[3,4]}"#,
        ] {
            fs::write(&path, contents).unwrap();
            assert!(Reader::new(&path, &local_options(), None).is_err());
        }
    }

    #[test]
    fn immutable_file_uris_handle_windows_drives_unc_and_reserved_characters() {
        assert_eq!(
            immutable_uri("/tmp/a b?#%.gpkg", false),
            "file:///tmp/a%20b%3F%23%25%2Egpkg?mode=ro&immutable=1"
        );
        assert_eq!(
            immutable_uri(r"\\?\C:\data\a b.gpkg", true),
            "file:///C:/data/a%20b%2Egpkg?mode=ro&immutable=1"
        );
        assert_eq!(
            immutable_uri(r"\\?\UNC\server\share\a.gpkg", true),
            "file:////server/share/a%2Egpkg?mode=ro&immutable=1"
        );
    }

    #[test]
    fn gpkg_selected_layer_metadata_and_wkb_dimensions_must_agree() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("input.gpkg");
        gpkg(&path);
        let db = Connection::open(&path).unwrap();
        // An unrelated broken layer must not block an explicitly selected layer.
        db.execute(
            "UPDATE gpkg_geometry_columns SET z=5 WHERE table_name='sites'",
            [],
        )
        .unwrap();
        let options = gpkg_options();
        let mut reader = Reader::new(&path, &options, None).unwrap();
        assert_eq!(read(&mut reader, &options).0.len(), 1);
        for (declared, z) in [("LINESTRING", 1), ("POINT", 0)] {
            db.execute("UPDATE gpkg_geometry_columns SET geometry_type_name=?1,z=?2 WHERE table_name='roads'", params![declared, z]).unwrap();
            let mut reader = Reader::new(&path, &options, None).unwrap();
            let (features, reports) = read(&mut reader, &options);
            assert!(features.is_empty());
            assert!(reports.iter().any(|(value, invalid)| *invalid
                && value["reason"].as_str().unwrap().contains("declaration")));
        }
        let multi = json!({"type":"MultiPoint","coordinates":[[1.,2.,3.],[4.,5.,6.]]});
        let mut blob = gpkg_blob(&multi, 4326);
        // The first member begins after GP header + WKB header + point count.
        blob[18..22].copy_from_slice(&1u32.to_le_bytes());
        assert!(decode_gpkg(&blob, 4326, 20).is_err());
        let mut empty = gpkg_blob(&point(1.), 4326);
        for chunk in empty[13..].as_chunks_mut::<8>().0 {
            chunk.copy_from_slice(&f64::NAN.to_le_bytes());
        }
        assert!(decode_gpkg(&empty, 4326, 20).is_err());
        empty[3] |= 0x10;
        assert!(decode_gpkg(&empty, 4326, 20).unwrap().is_none());
    }
    #[test]
    fn gpkg_rowidless_metadata_and_declared_coordinate_epochs() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("input.gpkg");
        gpkg(&path);
        let db = Connection::open(&path).unwrap();
        db.execute_batch("ALTER TABLE gpkg_geometry_columns RENAME TO old_columns;
            CREATE TABLE gpkg_geometry_columns(table_name TEXT,column_name TEXT,srs_id INTEGER,z INTEGER,m INTEGER,geometry_type_name TEXT,PRIMARY KEY(table_name,column_name)) WITHOUT ROWID;
            INSERT INTO gpkg_geometry_columns SELECT * FROM old_columns;
            DROP TABLE old_columns;
            ALTER TABLE gpkg_spatial_ref_sys ADD COLUMN epoch DOUBLE;
            UPDATE gpkg_spatial_ref_sys SET epoch=2020.0;").unwrap();
        let mut options = gpkg_options();
        let mut reader = Reader::new(&path, &options, None).unwrap();
        assert_eq!(read(&mut reader, &options).0.len(), 1);
        options.source_crs = None;
        assert!(matches!(
            Reader::new(&path, &options, None),
            Err(Error::Environment(_))
        ));
        options.source_crs = Some("EPSG:4326".into());
        options.height_offset = Some(0.);
        let mut reader = Reader::new(&path, &options, None).unwrap();
        assert_eq!(read(&mut reader, &options).0.len(), 1);
    }

    #[test]
    fn gpkg_hot_rollback_journal_is_not_ignored_by_immutable_reads() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("input.gpkg");
        gpkg(&path);
        let journal = dir.path().join("input.gpkg-journal");
        fs::write(&journal, [0u8; 1024]).unwrap();
        assert!(Reader::new(&path, &gpkg_options(), None).is_ok());
        let mut header = [0u8; 1024];
        header[..8].copy_from_slice(&[0xd9, 0xd5, 0x05, 0xf9, 0x20, 0xa1, 0x63, 0xd7]);
        fs::write(&journal, header).unwrap();
        assert!(
            matches!(Reader::new(&path, &gpkg_options(), None), Err(Error::Data(message)) if message.contains("rollback journal"))
        );
    }
}
