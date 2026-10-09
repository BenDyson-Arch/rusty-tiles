//! Shared disk-backed vector hierarchy, LOD, encoding and reuse. Source readers
//! and polygon operations are selected by build; workers receive owned data.
pub(super) mod acceptance;
mod aggregation;
use acceptance::{FeatureFailure, FeatureResult};
mod encoding;
mod geometry;
mod reuse;
#[cfg(feature = "native-geospatial")]
#[path = "pipeline/source_native.rs"]
mod source;
#[cfg(not(feature = "native-geospatial"))]
#[path = "portable.rs"]
mod source;
mod store;

use super::{
    model::{Feature, Frame, Geometry, IntrinsicGeometry, Point},
    VectorOptions,
};
use crate::vec3::{add, dot, mul, norm, sub, y_up_to_z_up as zup};
use crate::Error;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::Path,
};

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
    Error::Io(std::io::Error::other(error))
}
fn data(message: impl Into<String>) -> Error {
    Error::Data(message.into())
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

pub(super) struct VectorMember {
    pub name: String,
    pub path: std::path::PathBuf,
}
pub(super) struct CompletedVector {
    pub report: Value,
    pub members: Vec<VectorMember>,
}

pub(super) fn convert(
    input: &Path,
    output: &Path,
    options: &VectorOptions,
    attempt: &crate::runtime::Attempt,
) -> Result<CompletedVector, Error> {
    store::convert(input, output, options, attempt)
}

pub(crate) fn available() -> Result<(), Error> {
    #[cfg(feature = "native-geospatial")]
    {
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
        geometry::GeometryHandle::polygon(&[vec![[0., 0., 0.], [1., 0., 0.], [0., 1., 0.]]])
            .map_err(FeatureFailure::into_error)?
            .triangulate()
            .map(drop)
            .map_err(FeatureFailure::into_error)
    }
    #[cfg(not(feature = "native-geospatial"))]
    {
        Ok(())
    }
}
