//! Unchanged OGR/GEOS ownership and geometry operations.
use super::{outline, FeatureFailure, Point};
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

fn native_failure(operation: &str) -> FeatureFailure {
    FeatureFailure::Fatal(crate::Error::Io(std::io::Error::other(
        geospatial::diagnostic(operation),
    )))
}

/// A false validity result is semantic; CPL failure is infrastructure. Reset
/// stale thread-local diagnostics before each owned native operation.
fn checked<T>(operation: impl FnOnce() -> T) -> Result<T, FeatureFailure> {
    let _errors = quiet_unless_diagnostics();
    unsafe {
        gdal_sys::CPLErrorReset();
    }
    let value = operation();
    if unsafe { gdal_sys::CPLGetLastErrorType() } >= gdal_sys::CPLErr::CE_Failure {
        Err(native_failure("native geometry operation failed"))
    } else {
        Ok(value)
    }
}

pub(in super::super) struct GeometryHandle(NonNull<c_void>);
impl Drop for GeometryHandle {
    fn drop(&mut self) {
        // SAFETY: Exactly one destroy of the uniquely owned OGR geometry.
        unsafe { gdal_sys::OGR_G_DestroyGeometry(self.0.as_ptr()) };
    }
}
impl GeometryHandle {
    fn owned(operation: impl FnOnce() -> gdal_sys::OGRGeometryH) -> Result<Self, FeatureFailure> {
        let mut allocated = std::ptr::null_mut();
        let outcome = checked(|| {
            allocated = operation();
            allocated
        });
        match outcome {
            Ok(raw) => NonNull::new(raw).map(Self).ok_or_else(|| {
                native_failure("native geometry allocation or operation returned null")
            }),
            Err(error) => {
                if !allocated.is_null() {
                    unsafe {
                        gdal_sys::OGR_G_DestroyGeometry(allocated);
                    }
                }
                Err(error)
            }
        }
    }
    pub(in super::super) fn polygon(rings: &[Vec<Point>]) -> Result<Self, FeatureFailure> {
        let _errors = quiet_unless_diagnostics();
        // SAFETY: Every allocated geometry is owned by its RAII wrapper; adding a
        // ring copies it. Coordinate buffers contain finite, validated XY values.
        unsafe {
            let polygon = Self::owned(|| {
                gdal_sys::OGR_G_CreateGeometry(gdal_sys::OGRwkbGeometryType::wkbPolygon)
            })?;
            for points in rings {
                let ring = Self::owned(|| {
                    gdal_sys::OGR_G_CreateGeometry(gdal_sys::OGRwkbGeometryType::wkbLinearRing)
                })?;
                for p in points {
                    checked(|| gdal_sys::OGR_G_AddPoint_2D(ring.0.as_ptr(), p[0], p[1]))?;
                }
                checked(|| gdal_sys::OGR_G_CloseRings(ring.0.as_ptr()))?;
                if checked(|| gdal_sys::OGR_G_AddGeometry(polygon.0.as_ptr(), ring.0.as_ptr()))?
                    != 0
                {
                    return Err(native_failure("cannot create polygon rings"));
                }
            }
            Ok(polygon)
        }
    }
    pub(super) fn valid(&self) -> Result<bool, FeatureFailure> {
        let _errors = quiet_unless_diagnostics();
        // SAFETY: A live geometry; GDAL/GEOS validity queries do not mutate it.
        checked(|| unsafe { gdal_sys::OGR_G_IsValid(self.0.as_ptr()) != 0 })
    }
    pub(super) fn repair(&self) -> Result<Self, FeatureFailure> {
        let _errors = quiet_unless_diagnostics();
        // SAFETY: MakeValid returns an independently owned geometry.
        Self::owned(|| unsafe { gdal_sys::OGR_G_MakeValid(self.0.as_ptr()) })
    }
    pub(super) fn area(&self) -> Result<f64, FeatureFailure> {
        // SAFETY: A live polygon geometry.
        checked(|| unsafe { gdal_sys::OGR_G_Area(self.0.as_ptr()) })
    }
    fn clone_raw(raw: gdal_sys::OGRGeometryH) -> Result<Self, FeatureFailure> {
        // SAFETY: Caller retains the parent of this borrowed non-null geometry.
        Self::owned(|| unsafe { gdal_sys::OGR_G_Clone(raw) })
    }
    pub(super) fn parts(&self) -> Result<Vec<Self>, FeatureFailure> {
        fn walk(
            raw: gdal_sys::OGRGeometryH,
            values: &mut Vec<GeometryHandle>,
        ) -> Result<(), FeatureFailure> {
            // SAFETY: The owning parent stays live for this recursive traversal.
            unsafe {
                let kind =
                    checked(|| gdal_sys::OGR_GT_Flatten(gdal_sys::OGR_G_GetGeometryType(raw)))?;
                if kind == gdal_sys::OGRwkbGeometryType::wkbPolygon {
                    values.push(GeometryHandle::clone_raw(raw)?);
                } else if kind == gdal_sys::OGRwkbGeometryType::wkbMultiPolygon
                    || kind == gdal_sys::OGRwkbGeometryType::wkbGeometryCollection
                {
                    for i in 0..checked(|| gdal_sys::OGR_G_GetGeometryCount(raw))? {
                        let child = checked(|| gdal_sys::OGR_G_GetGeometryRef(raw, i))?;
                        if child.is_null() {
                            return Err(native_failure("native geometry child reference is null"));
                        }
                        walk(child, values)?;
                    }
                } else if checked(|| gdal_sys::OGR_G_IsEmpty(raw))? == 0 {
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
    pub(super) fn rings(&self) -> Result<Vec<Vec<[f64; 2]>>, FeatureFailure> {
        // SAFETY: Owned polygon and its ring references remain live throughout.
        checked(|| unsafe {
            (0..gdal_sys::OGR_G_GetGeometryCount(self.0.as_ptr()))
                .map(|i| {
                    let ring = gdal_sys::OGR_G_GetGeometryRef(self.0.as_ptr(), i);
                    (0..gdal_sys::OGR_G_GetPointCount(ring) - 1)
                        .map(|j| [gdal_sys::OGR_G_GetX(ring, j), gdal_sys::OGR_G_GetY(ring, j)])
                        .collect()
                })
                .collect()
        })
    }
    pub(in super::super) fn triangulate(&self) -> Result<Self, FeatureFailure> {
        let _errors = quiet_unless_diagnostics();
        // SAFETY: GDAL 3.12+ returns an independent owned CDT geometry; its
        // GEOS context and this polygon remain on the calling worker.
        Self::owned(|| unsafe { gdal_sys::OGR_G_ConstrainedDelaunayTriangulation(self.0.as_ptr()) })
    }
}

#[cfg(test)]
mod outcome_tests {
    use super::*;

    #[test]
    fn native_failure_during_false_query_is_fatal_not_topology_rejection() {
        let error = checked(|| unsafe {
            gdal_sys::CPLError(
                gdal_sys::CPLErr::CE_Failure,
                1,
                c"fixture native query failed".as_ptr(),
            );
            false
        })
        .unwrap_err();
        assert!(matches!(error, FeatureFailure::Fatal(crate::Error::Io(_))));
        assert!(error.to_string().contains("fixture native query failed"));
    }

    #[test]
    fn native_topology_warning_false_and_stale_failure_are_not_fatal() {
        let valid = checked(|| unsafe {
            gdal_sys::CPLError(
                gdal_sys::CPLErr::CE_Warning,
                1,
                c"fixture invalid topology".as_ptr(),
            );
            false
        })
        .unwrap();
        assert!(!valid);
        unsafe {
            gdal_sys::CPLError(
                gdal_sys::CPLErr::CE_Failure,
                1,
                c"stale unrelated failure".as_ptr(),
            );
        }
        assert!(!checked(|| false).unwrap());
    }
}
