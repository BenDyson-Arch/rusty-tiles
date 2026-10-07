//! Native OGR ingestion and disk-backed vector hierarchy. Native handles never
//! cross worker boundaries; only owned features, reports and bytes do.
mod aggregation;
mod encoding;
mod geometry;
mod reuse;
mod source;
mod store;

use super::VectorOptions;
use crate::vec3::{add, dot, mul, norm, sub, y_up_to_z_up as zup};
use crate::Error;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::Path,
};

type Point = [f64; 3];
type PointKey = [u64; 3];
fn key(point: Point) -> PointKey {
    point.map(|v| if v == 0. { 0 } else { v.to_bits() })
}
fn hash(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
fn canonical<T: Serialize>(value: &T) -> Result<Vec<u8>, Error> {
    Ok(serde_json::to_vec(&serde_json::to_value(value)?)?)
}
fn digest<T: Serialize>(value: &T) -> Result<String, Error> {
    Ok(hash(&canonical(value)?))
}
fn sql(error: rusqlite::Error) -> Error {
    Error::Data(format!("vector spool: {error}"))
}
fn data(message: impl Into<String>) -> Error {
    Error::Data(message.into())
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", content = "coordinates")]
enum Geometry {
    Point(Point),
    MultiPoint(Vec<Point>),
    LineString(Vec<Point>),
    MultiLineString(Vec<Vec<Point>>),
    Polygon(Vec<Vec<Point>>),
    MultiPolygon(Vec<Vec<Vec<Point>>>),
}
impl Geometry {
    fn paths(&self) -> Vec<&[Point]> {
        match self {
            Self::Point(p) => vec![std::slice::from_ref(p)],
            Self::MultiPoint(p) | Self::LineString(p) => vec![p],
            Self::MultiLineString(p) | Self::Polygon(p) => p.iter().map(Vec::as_slice).collect(),
            Self::MultiPolygon(p) => p.iter().flatten().map(Vec::as_slice).collect(),
        }
    }
    fn points(&self) -> impl Iterator<Item = &Point> {
        self.paths().into_iter().flatten()
    }
    fn size(&self) -> usize {
        self.points().count()
    }
    fn map(&mut self, mut f: impl FnMut(Point) -> Point) {
        match self {
            Self::Point(p) => *p = f(*p),
            Self::MultiPoint(p) | Self::LineString(p) => p.iter_mut().for_each(|p| *p = f(*p)),
            Self::MultiLineString(p) | Self::Polygon(p) => {
                p.iter_mut().flatten().for_each(|p| *p = f(*p))
            }
            Self::MultiPolygon(p) => p.iter_mut().flatten().flatten().for_each(|p| *p = f(*p)),
        }
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
struct Feature {
    properties: BTreeMap<String, Value>,
    geometry: Geometry,
    #[serde(
        default,
        rename = "_surface_fragment",
        skip_serializing_if = "std::ops::Not::not"
    )]
    surface_fragment: bool,
    #[serde(
        default,
        rename = "_triangle_boundaries",
        skip_serializing_if = "Vec::is_empty"
    )]
    triangle_boundaries: Vec<Vec<Vec<Point>>>,
    #[serde(
        default,
        rename = "_fragment_path",
        skip_serializing_if = "String::is_empty"
    )]
    fragment_path: String,
}
impl Feature {
    fn estimate(&self) -> usize {
        self.geometry.size() * 32
            + serde_json::to_vec(&self.properties).map_or(0, |p| p.len())
            + 2048
    }
    fn source_id(&self) -> &Value {
        &self.properties["_source_id"]
    }
    fn layer(&self) -> &str {
        self.properties["_source_layer"].as_str().unwrap()
    }
}
fn bounds<'a>(points: impl Iterator<Item = &'a Point>) -> Result<(Point, Point), Error> {
    let mut lo = [f64::INFINITY; 3];
    let mut hi = [f64::NEG_INFINITY; 3];
    for p in points {
        for i in 0..3 {
            lo[i] = lo[i].min(p[i]);
            hi[i] = hi[i].max(p[i]);
        }
    }
    if !lo.iter().chain(&hi).all(|v| v.is_finite()) {
        return Err(data("empty or nonfinite geometry bounds"));
    }
    Ok((lo, hi))
}

pub(super) fn convert(
    input: &Path,
    output: &Path,
    max_features: usize,
    repair: bool,
    ambiguous_outlines: bool,
    options: &VectorOptions,
) -> Result<(), Error> {
    store::convert(
        input,
        output,
        max_features,
        repair,
        ambiguous_outlines,
        options,
    )
}

pub(crate) fn available() -> Result<(), Error> {
    let versions = crate::geospatial::versions()?;
    if versions.geos.is_none_or(|v| v < [3, 10, 0]) {
        return Err(Error::Environment(
            "native vector requires GDAL with GEOS >= 3.10".into(),
        ));
    }
    crate::geospatial::native::init()?;
    // SAFETY: Drivers are registered; these lookups return borrowed
    // process-lifetime drivers and do not open or alter any datasets.
    unsafe {
        for driver in [c"GeoJSON", c"GPKG", c"ESRI Shapefile"] {
            if gdal_sys::GDALGetDriverByName(driver.as_ptr()).is_null() {
                return Err(Error::Environment(format!(
                    "native vector requires the GDAL {} driver",
                    driver.to_string_lossy()
                )));
            }
        }
    }
    geometry::GeometryHandle::polygon(&[vec![[0., 0., 0.], [1., 0., 0.], [0., 1., 0.]]])?
        .triangulate()
        .map(drop)
}
