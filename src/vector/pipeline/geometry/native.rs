//! Unchanged OGR/GEOS ownership and geometry operations.
use super::{data, outline, Error, Point};
use crate::geospatial::{self, QuietErrors};
use std::{ffi::c_void, ptr::NonNull};

/// Silence GDAL/GEOS geometry warnings unless native diagnostics are enabled
/// with `RUSTY_TILES_NATIVE_DIAGNOSTICS=1`. The deprecated
/// `RUSTY_TILES_PYTHON_TRACEBACK=1` is honoured when the new name is unset.
pub(in super::super) fn quiet_unless_diagnostics() -> Option<QuietErrors> {
    let enabled = match std::env::var("RUSTY_TILES_NATIVE_DIAGNOSTICS") {
        Ok(value) => value == "1",
        Err(_) => std::env::var("RUSTY_TILES_PYTHON_TRACEBACK").as_deref() == Ok("1"),
    };
    (!enabled).then(QuietErrors::new)
}

pub(in super::super) struct GeometryHandle(NonNull<c_void>);
impl Drop for GeometryHandle {
    fn drop(&mut self) {
        // SAFETY: Exactly one destroy of the uniquely owned OGR geometry.
        unsafe { gdal_sys::OGR_G_DestroyGeometry(self.0.as_ptr()) };
    }
}
impl GeometryHandle {
    fn owned(raw: gdal_sys::OGRGeometryH) -> Result<Self, Error> {
        NonNull::new(raw)
            .map(Self)
            .ok_or_else(|| data(geospatial::diagnostic("native geometry operation failed")))
    }
    pub(in super::super) fn polygon(rings: &[Vec<Point>]) -> Result<Self, Error> {
        let _errors = quiet_unless_diagnostics();
        // SAFETY: Every allocated geometry is owned by its RAII wrapper; adding a
        // ring copies it. Coordinate buffers contain finite, validated XY values.
        unsafe {
            let polygon = Self::owned(gdal_sys::OGR_G_CreateGeometry(
                gdal_sys::OGRwkbGeometryType::wkbPolygon,
            ))?;
            for points in rings {
                let ring = Self::owned(gdal_sys::OGR_G_CreateGeometry(
                    gdal_sys::OGRwkbGeometryType::wkbLinearRing,
                ))?;
                for p in points {
                    gdal_sys::OGR_G_AddPoint_2D(ring.0.as_ptr(), p[0], p[1]);
                }
                gdal_sys::OGR_G_CloseRings(ring.0.as_ptr());
                if gdal_sys::OGR_G_AddGeometry(polygon.0.as_ptr(), ring.0.as_ptr()) != 0 {
                    return Err(data("cannot create polygon rings"));
                }
            }
            Ok(polygon)
        }
    }
    pub(super) fn valid(&self) -> bool {
        let _errors = quiet_unless_diagnostics();
        // SAFETY: A live geometry; GDAL/GEOS validity queries do not mutate it.
        unsafe { gdal_sys::OGR_G_IsValid(self.0.as_ptr()) != 0 }
    }
    pub(super) fn repair(&self) -> Result<Self, Error> {
        let _errors = quiet_unless_diagnostics();
        // SAFETY: MakeValid returns an independently owned geometry.
        Self::owned(unsafe { gdal_sys::OGR_G_MakeValid(self.0.as_ptr()) })
    }
    pub(super) fn area(&self) -> f64 {
        // SAFETY: A live polygon geometry.
        unsafe { gdal_sys::OGR_G_Area(self.0.as_ptr()) }
    }
    fn clone_raw(raw: gdal_sys::OGRGeometryH) -> Result<Self, Error> {
        // SAFETY: Caller retains the parent of this borrowed non-null geometry.
        Self::owned(unsafe { gdal_sys::OGR_G_Clone(raw) })
    }
    pub(super) fn parts(&self) -> Result<Vec<Self>, Error> {
        fn walk(
            raw: gdal_sys::OGRGeometryH,
            values: &mut Vec<GeometryHandle>,
        ) -> Result<(), Error> {
            // SAFETY: The owning parent stays live for this recursive traversal.
            unsafe {
                let kind = gdal_sys::OGR_GT_Flatten(gdal_sys::OGR_G_GetGeometryType(raw));
                if kind == gdal_sys::OGRwkbGeometryType::wkbPolygon {
                    values.push(GeometryHandle::clone_raw(raw)?);
                } else if kind == gdal_sys::OGRwkbGeometryType::wkbMultiPolygon
                    || kind == gdal_sys::OGRwkbGeometryType::wkbGeometryCollection
                {
                    for i in 0..gdal_sys::OGR_G_GetGeometryCount(raw) {
                        walk(gdal_sys::OGR_G_GetGeometryRef(raw, i), values)?;
                    }
                } else if gdal_sys::OGR_G_IsEmpty(raw) == 0 {
                    return Err(outline(
                        "repair produced collapsed non-polygon geometry; source needs review",
                    ));
                }
            }
            Ok(())
        }
        let mut values = Vec::new();
        walk(self.0.as_ptr(), &mut values)?;
        Ok(values)
    }
    pub(super) fn rings(&self) -> Vec<Vec<[f64; 2]>> {
        // SAFETY: Owned polygon and its ring references remain live throughout.
        unsafe {
            (0..gdal_sys::OGR_G_GetGeometryCount(self.0.as_ptr()))
                .map(|i| {
                    let ring = gdal_sys::OGR_G_GetGeometryRef(self.0.as_ptr(), i);
                    (0..gdal_sys::OGR_G_GetPointCount(ring) - 1)
                        .map(|j| [gdal_sys::OGR_G_GetX(ring, j), gdal_sys::OGR_G_GetY(ring, j)])
                        .collect()
                })
                .collect()
        }
    }
    pub(in super::super) fn triangulate(&self) -> Result<Self, Error> {
        let _errors = quiet_unless_diagnostics();
        // SAFETY: GDAL 3.12+ returns an independent owned CDT geometry; its
        // GEOS context and this polygon remain on the calling worker.
        Self::owned(unsafe { gdal_sys::OGR_G_ConstrainedDelaunayTriangulation(self.0.as_ptr()) })
    }
}
