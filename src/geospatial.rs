//! Native GDAL/PROJ operations for the converter migration.
//!
//! Handles belong to their creating worker and are neither Send nor Sync.
//! GDAL owns the PROJ contexts. Native operations disable GDAL's process-wide
//! PROJ networking policy; callers must not re-enable it while converting.
//! Local grid data and the PROJ database are still required.

pub(crate) mod native;

use crate::Error;
use std::ffi::{c_void, CStr, CString};
use std::ptr::{null, null_mut, NonNull};

const MIN_GDAL_VERSION: u32 = 3_120_000;

/// Actual linked library versions, rather than the build machine's versions.
#[derive(Debug, Clone, serde::Serialize)]
pub struct Versions {
    pub gdal: String,
    pub proj: [i32; 3],
    pub geos: Option<[i32; 3]>,
}

pub fn versions() -> Result<Versions, Error> {
    // SAFETY: GDAL returns static, null-terminated version strings. The version
    // queries write only into the three stack integers passed to each call.
    unsafe {
        let number = string(gdal_sys::GDALVersionInfo(c"VERSION_NUM".as_ptr()));
        if number.parse::<u32>().unwrap_or(0) < MIN_GDAL_VERSION {
            return Err(Error::Environment(format!(
                "native geospatial operations require GDAL >= 3.12 (found {number})"
            )));
        }
        let mut proj = [0; 3];
        gdal_sys::OSRGetPROJVersion(&mut proj[0], &mut proj[1], &mut proj[2]);
        if proj < [9, 2, 0] {
            return Err(Error::Environment(format!(
                "strict native CRS operations require PROJ >= 9.2 (found {}.{}.{})",
                proj[0], proj[1], proj[2]
            )));
        }
        let mut geos = [0; 3];
        let has_geos = gdal_sys::OGRGetGEOSVersion(&mut geos[0], &mut geos[1], &mut geos[2]);
        Ok(Versions {
            gdal: string(gdal_sys::GDALVersionInfo(c"RELEASE_NAME".as_ptr())),
            proj,
            geos: (has_geos as i32 != 0).then_some(geos),
        })
    }
}

/// Copy GDAL's effective PROJ search paths; the API returns an owned CSL list.
pub(crate) fn proj_search_paths() -> Vec<std::path::PathBuf> {
    let mut paths = Vec::new();
    // SAFETY: GDAL returns a null-terminated list owned by this call. Each
    // live string is copied before the matching CSLDestroy frees the list.
    unsafe {
        let list = gdal_sys::OSRGetPROJSearchPaths();
        if !list.is_null() {
            let mut cursor = list;
            while !(*cursor).is_null() {
                paths.push(string(*cursor).into());
                cursor = cursor.add(1);
            }
            gdal_sys::CSLDestroy(list);
        }
    }
    paths
}

pub(crate) fn offline() -> Result<(), Error> {
    versions()?;
    // SAFETY: This GDAL API updates its mutex-protected network policy. All
    // operations in this module require offline resource resolution.
    unsafe { gdal_sys::OSRSetPROJEnableNetwork(0) };
    Ok(())
}

// Suppress native stderr noise while retaining GDAL's thread-local error message.
// Each guard is private, stack-scoped and cannot outlive its native operation.
pub(crate) struct QuietErrors;

impl QuietErrors {
    pub(crate) fn new() -> Self {
        // SAFETY: GDAL's error-handler stack is thread-local; the callback has
        // the exact C ABI and is paired with a pop in Drop.
        unsafe {
            gdal_sys::CPLErrorReset();
            gdal_sys::CPLPushErrorHandler(Some(gdal_sys::CPLQuietErrorHandler));
        }
        Self
    }
}

impl Drop for QuietErrors {
    fn drop(&mut self) {
        // SAFETY: The guard has exactly one matching push on this thread.
        unsafe { gdal_sys::CPLPopErrorHandler() };
    }
}

pub(crate) unsafe fn string(value: *const std::ffi::c_char) -> String {
    if value.is_null() {
        String::new()
    } else {
        // SAFETY: The caller supplies a live GDAL null-terminated string.
        unsafe { CStr::from_ptr(value) }
            .to_string_lossy()
            .into_owned()
    }
}

pub(crate) fn diagnostic(context: &str) -> String {
    // SAFETY: GDAL owns this thread-local null-terminated error string.
    let detail = unsafe { string(gdal_sys::CPLGetLastErrorMsg()) };
    if detail.is_empty() {
        context.to_owned()
    } else {
        format!("{context}: {detail}")
    }
}

/// An owned spatial reference using traditional easting/northing or lon/lat axes.
#[derive(Debug)]
pub struct Crs(NonNull<c_void>);

impl Crs {
    /// Clone a dataset-owned SRS, retaining its coordinate epoch and datum.
    /// Caller must keep the borrowed native handle live for this call.
    pub(crate) unsafe fn clone_native(raw: gdal_sys::OGRSpatialReferenceH) -> Result<Self, Error> {
        // SAFETY: The caller keeps raw live; OSRClone returns an independent SRS.
        let owned = NonNull::new(unsafe { gdal_sys::OSRClone(raw) })
            .ok_or_else(|| Error::Data("cannot clone source CRS".into()))?;
        // SAFETY: Configures the uniquely owned clone.
        unsafe {
            gdal_sys::OSRSetAxisMappingStrategy(
                owned.as_ptr(),
                gdal_sys::OSRAxisMappingStrategy::OAMS_TRADITIONAL_GIS_ORDER,
            )
        };
        Ok(Self(owned.cast()))
    }

    pub(crate) fn wkt(&self) -> Result<String, Error> {
        let _errors = QuietErrors::new();
        let mut raw = null_mut();
        // SAFETY: The SRS is live; the exported string is freed exactly once.
        unsafe {
            if gdal_sys::OSRExportToWkt(self.0.as_ptr(), &mut raw) != 0 {
                return Err(Error::Data(diagnostic("cannot export source CRS")));
            }
            let value = string(raw);
            gdal_sys::VSIFree(raw.cast());
            Ok(value)
        }
    }

    fn empty() -> Result<Self, Error> {
        // SAFETY: A null definition requests an empty, independently owned SRS.
        NonNull::new(unsafe { gdal_sys::OSRNewSpatialReference(null()) })
            .map(Self)
            .ok_or_else(|| Error::Environment(diagnostic("cannot allocate spatial reference")))
    }

    pub fn from_definition(definition: &str) -> Result<Self, Error> {
        offline()?;
        let _errors = QuietErrors::new();
        // Check the database separately so unavailable EPSG resources are not
        // misreported as a malformed user-supplied CRS.
        let probe = Self::empty()?;
        // SAFETY: probe owns a valid SRS handle.
        if unsafe { gdal_sys::OSRImportFromEPSG(probe.0.as_ptr(), 4326) } != 0 {
            return Err(Error::Environment(diagnostic("PROJ database unavailable")));
        }
        let definition = CString::new(definition)
            .map_err(|_| Error::Data("CRS definition contains a NUL byte".into()))?;
        let crs = Self::empty()?;
        // A CRS argument/header contains a definition, not a URL or a filename.
        // PROJ's offline policy alone does not constrain GDAL's CRS parser.
        let options = [
            c"ALLOW_NETWORK_ACCESS=NO".as_ptr(),
            c"ALLOW_FILE_ACCESS=NO".as_ptr(),
            null(),
        ];
        // SAFETY: The SRS, input and null-terminated option list remain live.
        // GDAL reads CSLConstList without modifying its list/strings, although
        // the C bindings expose it as char** rather than const char* const*.
        if unsafe {
            gdal_sys::OSRSetFromUserInputEx(
                crs.0.as_ptr(),
                definition.as_ptr(),
                options.as_ptr().cast_mut().cast(),
            )
        } != 0
        {
            return Err(Error::Data(diagnostic("invalid source CRS")));
        }
        // SAFETY: A valid owned SRS is configured before being returned.
        unsafe {
            gdal_sys::OSRSetAxisMappingStrategy(
                crs.0.as_ptr(),
                gdal_sys::OSRAxisMappingStrategy::OAMS_TRADITIONAL_GIS_ORDER,
            );
        }
        Ok(crs)
    }

    fn promote_with_metre_height(&mut self) -> Result<(), Error> {
        let _errors = QuietErrors::new();
        let epoch = self.coordinate_epoch();
        let mut raw = null_mut();
        // SAFETY: Promotion mutates the uniquely owned SRS. The exported JSON
        // is a GDAL allocation copied into Rust before its matching free.
        unsafe {
            if gdal_sys::OSRPromoteTo3D(self.0.as_ptr(), null()) != 0
                || gdal_sys::OSRExportToPROJJSON(self.0.as_ptr(), &mut raw, null()) != 0
            {
                return Err(Error::Environment(diagnostic(
                    "cannot promote horizontal CRS to 3D",
                )));
            }
        }
        // SAFETY: Successful export returned a live null-terminated string.
        let definition = unsafe { string(raw) };
        // SAFETY: Successful export returned this allocation, freed once.
        unsafe { gdal_sys::VSIFree(raw.cast()) };
        let mut document: serde_json::Value = serde_json::from_str(&definition)?;
        let crs = if document["type"] == "BoundCRS" {
            &mut document["source_crs"]
        } else {
            &mut document
        };
        // PROJ 9.9 promotes projected Z using the horizontal linear units.
        // Our caller's new height axis is explicitly metres, independently of
        // source XY units. Retain every horizontal/datum/operation definition.
        let axis = crs
            .pointer_mut("/coordinate_system/axis/2")
            .ok_or_else(|| Error::Environment("promoted CRS has no third axis".into()))?;
        axis["unit"] = "metre".into();
        if let Some(axis) = crs.pointer_mut("/base_crs/coordinate_system/axis/2") {
            axis["unit"] = "metre".into();
        }
        let mut promoted = Self::from_definition(&serde_json::to_string(&document)?)?;
        if let Some(epoch) = epoch {
            promoted.set_coordinate_epoch(epoch)?;
        }
        *self = promoted;
        Ok(())
    }

    pub fn coordinate_epoch(&self) -> Option<f64> {
        // SAFETY: self retains its valid SRS handle for the call.
        let epoch = unsafe { gdal_sys::OSRGetCoordinateEpoch(self.0.as_ptr()) };
        (epoch != 0.).then_some(epoch)
    }

    pub fn set_coordinate_epoch(&mut self, epoch: f64) -> Result<(), Error> {
        if !epoch.is_finite() || epoch <= 0. {
            return Err(Error::Data(
                "coordinate epoch must be finite and positive".into(),
            ));
        }
        // SAFETY: The uniquely borrowed SRS is live and epoch was validated.
        unsafe { gdal_sys::OSRSetCoordinateEpoch(self.0.as_ptr(), epoch) };
        Ok(())
    }

    pub fn has_native_height(&self) -> bool {
        // SAFETY: All queries use self's valid SRS without modifying it.
        unsafe {
            gdal_sys::OSRIsCompound(self.0.as_ptr()) != 0
                || gdal_sys::OSRIsGeocentric(self.0.as_ptr()) != 0
                || gdal_sys::OSRGetAxesCount(self.0.as_ptr()) == 3
        }
    }

    pub fn is_horizontal(&self) -> bool {
        // SAFETY: Queries use this live SRS without modifying it.
        unsafe {
            !self.has_native_height()
                && gdal_sys::OSRGetAxesCount(self.0.as_ptr()) == 2
                && (gdal_sys::OSRIsProjected(self.0.as_ptr()) != 0
                    || gdal_sys::OSRIsGeographic(self.0.as_ptr()) != 0)
        }
    }
}

impl Drop for Crs {
    fn drop(&mut self) {
        // SAFETY: This handle is uniquely owned and has not been transferred.
        unsafe { gdal_sys::OSRDestroySpatialReference(self.0.as_ptr()) };
    }
}

struct Options(NonNull<c_void>);

impl Drop for Options {
    fn drop(&mut self) {
        // SAFETY: Options owns the corresponding non-null native allocation.
        unsafe { gdal_sys::OCTDestroyCoordinateTransformationOptions(self.0.as_ptr().cast()) };
    }
}

/// Offline, only-best, non-ballpark XYZ transformation. Source epoch is passed
/// as the fourth axis; an unspecified epoch stays unspecified, never year zero.
#[derive(Debug)]
pub struct StrictTransform {
    handle: NonNull<c_void>,
    epoch: Option<f64>,
    angular_units: Option<f64>,
}

impl StrictTransform {
    pub fn new(source: &Crs, target: &Crs) -> Result<Self, Error> {
        Self::create(source, target, None)
    }

    fn create(source: &Crs, target: &Crs, operation: Option<&str>) -> Result<Self, Error> {
        offline()?;
        let _errors = QuietErrors::new();
        // SAFETY: GDAL allocates independent options. The RAII owner cleans
        // them up on every return path, including configuration failures.
        let options = Options(
            NonNull::new(unsafe {
                gdal_sys::OCTNewCoordinateTransformationOptions().cast::<c_void>()
            })
            .ok_or_else(|| Error::Environment(diagnostic("cannot allocate CRS options")))?,
        );
        // SAFETY: The options handle is owned and live for both setters.
        let configured = unsafe {
            gdal_sys::OCTCoordinateTransformationOptionsSetBallparkAllowed(
                options.0.as_ptr().cast(),
                0,
            ) != 0
                && gdal_sys::OCTCoordinateTransformationOptionsSetOnlyBest(
                    options.0.as_ptr().cast(),
                    true,
                ) != 0
        };
        if !configured {
            return Err(Error::Environment(diagnostic(
                "cannot configure strict CRS operation",
            )));
        }
        if let Some(operation) = operation {
            let operation = CString::new(operation)
                .map_err(|_| Error::Data("CRS operation contains a NUL byte".into()))?;
            // SAFETY: GDAL copies the string into the owned options.
            if unsafe {
                gdal_sys::OCTCoordinateTransformationOptionsSetOperation(
                    options.0.as_ptr().cast(),
                    operation.as_ptr(),
                    0,
                )
            } == 0
            {
                return Err(Error::Environment(diagnostic(
                    "cannot configure CRS pipeline",
                )));
            }
        }
        // SAFETY: GDAL copies both SRS objects; the returned transform has its
        // own lifetime. The source, target and options remain live for this call.
        let handle = NonNull::new(unsafe {
            gdal_sys::OCTNewCoordinateTransformationEx(
                source.0.as_ptr(),
                target.0.as_ptr(),
                options.0.as_ptr().cast(),
            )
        })
        .ok_or_else(|| {
            Error::Environment(diagnostic(
                "no strict CRS operation available; check local PROJ database/grids",
            ))
        })?;
        // SAFETY: These queries use the borrowed source SRS before return.
        let angular_units = unsafe {
            (gdal_sys::OSRIsGeographic(source.0.as_ptr()) != 0)
                .then(|| gdal_sys::OSRGetAngularUnits(source.0.as_ptr(), null_mut()))
        };
        Ok(Self {
            handle,
            epoch: source.coordinate_epoch(),
            angular_units,
        })
    }

    /// Returns a complete transformed batch or an error, leaving input untouched.
    pub fn transform(&mut self, points: &[[f64; 3]]) -> Result<Vec<[f64; 3]>, Error> {
        if points.is_empty() {
            return Ok(Vec::new());
        }
        let count = i32::try_from(points.len())
            .map_err(|_| Error::Data("CRS batch exceeds native point limit".into()))?;
        for (index, point) in points.iter().enumerate() {
            if !point.iter().all(|v| v.is_finite()) {
                return Err(Error::Data(format!(
                    "point {index}: coordinates must be finite XYZ"
                )));
            }
            if let Some(units) = self.angular_units {
                if (point[0] * units).abs() > std::f64::consts::PI + 1e-12
                    || (point[1] * units).abs() > std::f64::consts::FRAC_PI_2 + 1e-12
                {
                    return Err(Error::Data(format!(
                        "point {index}: geographic coordinates outside longitude/latitude range"
                    )));
                }
            }
        }
        offline()?;
        let _errors = QuietErrors::new();
        let mut x: Vec<_> = points.iter().map(|p| p[0]).collect();
        let mut y: Vec<_> = points.iter().map(|p| p[1]).collect();
        let mut z: Vec<_> = points.iter().map(|p| p[2]).collect();
        let mut time = vec![self.epoch.unwrap_or(f64::INFINITY); points.len()];
        let mut codes = vec![0; points.len()];
        // SAFETY: All five arrays have count entries and remain live throughout
        // the call. Exclusive &mut self prevents concurrent use of the handle.
        let success = unsafe {
            gdal_sys::OCTTransform4DWithErrorCodes(
                self.handle.as_ptr(),
                count,
                x.as_mut_ptr(),
                y.as_mut_ptr(),
                z.as_mut_ptr(),
                time.as_mut_ptr(),
                codes.as_mut_ptr(),
            )
        };
        if let Some((index, code)) = codes.iter().enumerate().find(|(_, code)| **code != 0) {
            let message = diagnostic(&format!(
                "point {index}: CRS operation failed (PROJ error {code})"
            ));
            // PROJ codes 2048..4095 describe coordinate/domain failures, except
            // 2051 (no eligible operation). Initialization/resource failures can
            // also occur lazily during transformation and are environment errors.
            return Err(if (2048..4096).contains(code) && *code != 2051 {
                Error::Data(message)
            } else {
                Error::Environment(message)
            });
        }
        if success == 0 {
            return Err(Error::Environment(diagnostic(
                "CRS operation unavailable; check local PROJ database/grids",
            )));
        }
        x.into_iter()
            .zip(y)
            .zip(z)
            .enumerate()
            .map(|(index, ((x, y), z))| {
                let point = [x, y, z];
                if point.iter().all(|v| v.is_finite()) {
                    Ok(point)
                } else {
                    Err(Error::Data(format!(
                        "point {index}: CRS operation produced nonfinite XYZ"
                    )))
                }
            })
            .collect()
    }
}

impl Drop for StrictTransform {
    fn drop(&mut self) {
        // SAFETY: This transform owns its non-null allocation independently of
        // the source/target SRS objects and is destroyed exactly once.
        unsafe { gdal_sys::OCTDestroyCoordinateTransformation(self.handle.as_ptr()) };
    }
}

/// Place source coordinates in ECEF without inferring their height reference.
/// Horizontal CRSs require an explicit metre offset (zero when established).
/// Declared 3D/compound/geocentric CRSs use their native heights and reject offsets.
#[derive(Debug)]
pub struct EcefTransform {
    operation: StrictTransform,
    height_offset: f64,
}

impl EcefTransform {
    pub fn new(mut source: Crs, height_offset: Option<f64>) -> Result<Self, Error> {
        if height_offset.is_some_and(|v| !v.is_finite()) {
            return Err(Error::Data("height offset must be finite".into()));
        }
        let native_height = source.has_native_height();
        if native_height && height_offset.is_some() {
            return Err(Error::Data("declared 3D/vertical CRS already defines heights; use a horizontal CRS override to apply height offset".into()));
        }
        if !native_height {
            // SAFETY: Source is uniquely owned and both queries use its handle.
            let horizontal = unsafe {
                gdal_sys::OSRIsProjected(source.0.as_ptr()) != 0
                    || gdal_sys::OSRIsGeographic(source.0.as_ptr()) != 0
            };
            if !horizontal || height_offset.is_none() {
                return Err(Error::Data("horizontal CRS input requires an explicit height offset to ellipsoidal metres (0 when established)".into()));
            }
            source.promote_with_metre_height()?;
        }
        let target = Crs::from_definition("EPSG:4978")?;
        Ok(Self {
            operation: StrictTransform::new(&source, &target)?,
            height_offset: height_offset.unwrap_or(0.),
        })
    }

    pub fn transform(&mut self, points: &[[f64; 3]]) -> Result<Vec<[f64; 3]>, Error> {
        let points: Vec<_> = points
            .iter()
            .map(|p| [p[0], p[1], p[2] + self.height_offset])
            .collect();
        self.operation.transform(&points)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn epoch_is_unspecified_or_passed_to_time_dependent_pipeline() {
        let target = Crs::from_definition("EPSG:4978").unwrap();
        let mut source = Crs::from_definition("EPSG:4979").unwrap();
        let pipeline = "+proj=pipeline +step +proj=unitconvert +xy_in=deg +xy_out=rad \
            +step +proj=cart +ellps=WGS84 +step +proj=helmert +x=0 +y=0 +z=0 \
            +dx=1 +dy=0 +dz=0 +t_epoch=2020 +convention=position_vector";
        for (epoch, shift) in [(None, 0.), (Some(2021.), 1.)] {
            if let Some(epoch) = epoch {
                source.set_coordinate_epoch(epoch).unwrap();
            }
            let point = StrictTransform::create(&source, &target, Some(pipeline))
                .unwrap()
                .transform(&[[0., 0., 120.]])
                .unwrap()[0];
            assert!(
                (point[0] - (6378137. + 120. + shift)).abs() < 1e-7,
                "{point:?}"
            );
            assert!(point[1].abs() < 1e-7 && point[2].abs() < 1e-7);
        }
    }

    #[test]
    fn a_required_missing_grid_is_an_environment_error_with_networking_off() {
        let source = Crs::from_definition("EPSG:4979").unwrap();
        let target = Crs::from_definition("EPSG:4979").unwrap();
        let error = StrictTransform::create(
            &source,
            &target,
            Some(
                "+proj=pipeline +step +proj=unitconvert +xy_in=deg +xy_out=rad \
                +step +proj=vgridshift +grids=rusty-tiles-deliberately-missing-grid.gtx \
                +step +proj=unitconvert +xy_in=rad +xy_out=deg",
            ),
        )
        .unwrap_err();
        assert_eq!(error.category(), ("environment", 4));
        assert!(error.to_string().contains("grid"));
        // SAFETY: This query only reads GDAL's network setting.
        assert_eq!(unsafe { gdal_sys::OSRGetPROJEnableNetwork() }, 0);
    }
    #[test]
    fn native_quiet_errors_restore_the_callers_thread_local_handler() {
        use std::cell::Cell;
        thread_local! { static WARNINGS: Cell<usize> = const { Cell::new(0) }; }
        unsafe extern "C" fn count_warning(
            _: gdal_sys::CPLErr::Type,
            _: i32,
            _: *const std::ffi::c_char,
        ) {
            WARNINGS.with(|count| count.set(count.get() + 1));
        }
        struct Pop;
        impl Drop for Pop {
            fn drop(&mut self) {
                // SAFETY: Pops the single handler this test pushed.
                unsafe { gdal_sys::CPLPopErrorHandler() };
            }
        }
        // SAFETY: ABI matches GDAL's thread-local handler, popped by the guard.
        unsafe {
            gdal_sys::CPLPushErrorHandler(Some(count_warning));
        }
        let _pop = Pop;
        WARNINGS.with(|count| count.set(0));
        {
            let _quiet = QuietErrors::new();
            // SAFETY: No format placeholders; static null-terminated string.
            unsafe {
                gdal_sys::CPLError(gdal_sys::CPLErr::CE_Warning, 1, c"scoped warning".as_ptr());
            }
        }
        // SAFETY: No format placeholders; static null-terminated string.
        unsafe {
            gdal_sys::CPLError(
                gdal_sys::CPLErr::CE_Warning,
                1,
                c"restored handler".as_ptr(),
            );
        }
        assert_eq!(WARNINGS.with(Cell::get), 1);
    }
}
