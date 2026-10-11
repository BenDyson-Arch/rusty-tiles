//! Source-faithful COG and a bounded, directly sampled imagery pyramid.
use crate::{CleanupDiagnostic, JobError, JobErrorKind, JobFailure, OutputPolicy, RunControl};
use serde::Serialize;
use std::path::PathBuf;

#[derive(Clone, Copy, Debug, Serialize)]
#[serde(tag = "mode", rename_all = "snake_case")]
pub enum RasterDisplay {
    Image {
        alpha_band: Option<u16>,
    },
    Gray {
        band: u16,
        low: f64,
        high: f64,
        alpha_band: Option<u16>,
    },
}

/// Admission/work limits, not a physical memory or filesystem quota.
#[derive(Clone, Copy, Debug, Serialize)]
pub struct RasterLimits {
    pub max_source_bytes: u64,
    pub max_source_pixels: u64,
    pub max_decoded_bytes: u64,
    pub max_tiles: u64,
    pub max_output_bytes: u64,
    pub max_working_bytes: u64,
    pub workers: u8,
}
impl Default for RasterLimits {
    fn default() -> Self {
        Self {
            max_source_bytes: 512 * 1024 * 1024,
            max_source_pixels: 268_435_456,
            max_decoded_bytes: 2 * 1024 * 1024 * 1024,
            max_tiles: 100_000,
            max_output_bytes: 8 * 1024 * 1024 * 1024,
            max_working_bytes: 16 * 1024 * 1024 * 1024,
            workers: 2,
        }
    }
}

#[derive(Clone, Debug)]
pub struct RasterRequest {
    input: PathBuf,
    output: PathBuf,
    min_zoom: u8,
    max_zoom: u8,
    display: RasterDisplay,
    policy: OutputPolicy,
    limits: RasterLimits,
}
impl RasterRequest {
    pub fn image(
        input: impl Into<PathBuf>,
        output: impl Into<PathBuf>,
        min_zoom: u8,
        max_zoom: u8,
    ) -> Self {
        Self {
            input: input.into(),
            output: output.into(),
            min_zoom,
            max_zoom,
            display: RasterDisplay::Image { alpha_band: None },
            policy: OutputPolicy::CreateNew,
            limits: RasterLimits::default(),
        }
    }
    pub fn gray(
        input: impl Into<PathBuf>,
        output: impl Into<PathBuf>,
        min_zoom: u8,
        max_zoom: u8,
        band: u16,
        low: f64,
        high: f64,
    ) -> Self {
        let mut request = Self::image(input, output, min_zoom, max_zoom);
        request.display = RasterDisplay::Gray {
            band,
            low,
            high,
            alpha_band: None,
        };
        request
    }
    pub fn with_alpha_band(mut self, band: u16) -> Self {
        match &mut self.display {
            RasterDisplay::Image { alpha_band } | RasterDisplay::Gray { alpha_band, .. } => {
                *alpha_band = Some(band)
            }
        }
        self
    }
    pub fn with_policy(mut self, policy: OutputPolicy) -> Self {
        self.policy = policy;
        self
    }
    pub fn with_limits(mut self, limits: RasterLimits) -> Self {
        self.limits = limits;
        self
    }
}

#[cfg(feature = "native-geospatial")]
#[derive(Debug, Serialize)]
pub(crate) struct RasterCapabilities {
    pub(crate) native_version: String,
    pub(crate) gtiff: bool,
    pub(crate) png: bool,
    pub(crate) jpeg: bool,
    pub(crate) aaigrid: bool,
    pub(crate) cog: bool,
    pub(crate) tile: bool,
}

#[cfg(feature = "native-geospatial")]
mod display;
// Pure planning checks also run in portable builds; production planning
// requires the native capability, while zoom validation remains shared.
#[cfg_attr(not(feature = "native-geospatial"), allow(dead_code))]
mod grid;
#[cfg(feature = "native-geospatial")]
mod native;
#[cfg(feature = "native-geospatial")]
mod source;
#[cfg(feature = "native-geospatial")]
mod tile;

#[derive(Clone, Copy, Debug, Default, Serialize)]
pub struct RasterBandReport {
    pub datatype: &'static str,
    pub color_interp: u8,
    pub mask_class: u8,
}
/// At most 32 actually admitted bands; unused storage is never serialized.
#[derive(Clone, Debug)]
pub struct RasterBands {
    count: u8,
    records: [RasterBandReport; 32],
}
impl RasterBands {
    pub fn as_slice(&self) -> &[RasterBandReport] {
        &self.records[..usize::from(self.count)]
    }
}
impl Serialize for RasterBands {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        self.as_slice().serialize(serializer)
    }
}
#[derive(Debug, Serialize)]
pub struct RasterSourceReport {
    pub driver: &'static str,
    pub bytes: u64,
    pub width: u32,
    pub height: u32,
    pub epsg: u16,
    pub area_point: &'static str,
    pub bands: RasterBands,
}
#[derive(Debug, Serialize)]
pub struct RasterGridReport {
    pub source_affine: [f64; 6],
    pub target_geotransform: [f64; 6],
    pub target_width: u32,
    pub target_height: u32,
    /// Approximate descriptive geographic W,S,E,N; not certified coverage.
    pub bounds: [f64; 4],
    /// Inclusive XYZ x0,y0,x1,y1 at max_zoom; coarser rectangles are ancestors.
    pub finest_tiles: [u32; 4],
    pub min_zoom: u8,
    pub max_zoom: u8,
}
#[derive(Debug, Serialize)]
pub struct RasterReport {
    pub schema_version: u32,
    pub profile: &'static str,
    pub source: RasterSourceReport,
    pub grid: RasterGridReport,
    pub display: RasterDisplay,
    pub limits: RasterLimits,
    pub tiles: u64,
    /// Closed COG and PNG bytes, before the two bounded JSON members.
    pub artifact_bytes: u64,
    pub requested_workers: u8,
    pub resolved_workers: u8,
    pub worker_policy: &'static str,
    pub native_version: String,
    pub resampling: &'static str,
    pub color_role_migration: &'static str,
    pub source_color_interp: u8,
    pub target_color_interp: u8,
}
#[derive(Debug)]
pub struct RasterResult {
    pub output: PathBuf,
    pub report: RasterReport,
    pub cleanup_diagnostics: Vec<CleanupDiagnostic>,
}
fn error(kind: JobErrorKind, message: &'static str) -> JobError {
    JobError::new(kind, message)
}
fn validate(request: &RasterRequest) -> Result<grid::ZoomRange, JobError> {
    if request.input.as_os_str().is_empty() || request.output.as_os_str().is_empty() {
        return Err(error(
            JobErrorKind::InvalidRequest,
            "raster requires source and output paths",
        ));
    }
    let zooms = grid::ZoomRange::new(request.min_zoom, request.max_zoom).map_err(|_| {
        error(
            JobErrorKind::InvalidRequest,
            "raster requires minZoom <= maxZoom <= 24",
        )
    })?;
    let limits = request.limits;
    if [
        limits.max_source_bytes,
        limits.max_source_pixels,
        limits.max_decoded_bytes,
        limits.max_tiles,
        limits.max_output_bytes,
        limits.max_working_bytes,
    ]
    .contains(&0)
        || !(1..=4).contains(&limits.workers)
    {
        return Err(error(
            JobErrorKind::InvalidRequest,
            "raster needs six positive limits and one to four workers",
        ));
    }
    let alpha = match request.display {
        RasterDisplay::Image { alpha_band } => alpha_band,
        RasterDisplay::Gray {
            band,
            low,
            high,
            alpha_band,
        } => {
            if band == 0
                || !low.is_finite()
                || !high.is_finite()
                || low >= high
                || !(high - low).is_finite()
            {
                return Err(error(JobErrorKind::InvalidRequest, "gray display needs a positive band and finite increasing range with finite width"));
            }
            alpha_band
        }
    };
    if alpha == Some(0) {
        return Err(error(
            JobErrorKind::InvalidRequest,
            "an explicit alpha band must be positive",
        ));
    }
    Ok(zooms)
}
#[cfg(feature = "native-geospatial")]
pub(crate) fn capabilities() -> Result<RasterCapabilities, JobError> {
    native::capabilities()
}

pub fn raster_to_pyramid(
    request: RasterRequest,
    control: &RunControl,
) -> Result<RasterResult, JobFailure> {
    let attempt = control.begin()?;
    attempt.check().map_err(|e| attempt.fail(e))?;
    let zooms = validate(&request).map_err(|e| attempt.fail(e))?;
    #[cfg(not(feature = "native-geospatial"))]
    {
        let _ = zooms;
        Err(attempt.fail(error(
            JobErrorKind::Unsupported,
            "raster pyramid requires native-geospatial GDAL/PROJ capability",
        )))
    }
    #[cfg(feature = "native-geospatial")]
    {
        run_native(request, zooms, &attempt)
    }
}

#[cfg(feature = "native-geospatial")]
use crate::runtime::{
    directory::{DirectoryStaging, DirectoryTarget},
    Attempt,
};
#[cfg(feature = "native-geospatial")]
use std::{fs, io::Write, mem::size_of, path::Path};

#[cfg(feature = "native-geospatial")]
#[repr(C)]
#[derive(Clone, Copy, Debug)]
struct Member {
    bytes: u64,
    x: u32,
    y: u32,
    z: u8,
    kind: u8,
    reserved: [u8; 6],
}
#[cfg(feature = "native-geospatial")]
impl Member {
    fn fixed(bytes: u64, kind: u8) -> Self {
        Self {
            bytes,
            x: 0,
            y: 0,
            z: 0,
            kind,
            reserved: [0; 6],
        }
    }
}
#[cfg(feature = "native-geospatial")]
fn fail_child(attempt: &Attempt, child: JobFailure) -> JobFailure {
    let mut failure = attempt.fail(child.error);
    failure.secondary.extend(child.secondary);
    failure.retained_paths.extend(child.retained_paths);
    failure
}
#[cfg(feature = "native-geospatial")]
fn grid_error(cause: grid::GridError) -> JobError {
    let kind = match cause {
        grid::GridError::InvalidInput(_) => JobErrorKind::InvalidInput,
        grid::GridError::Unsupported(_) => JobErrorKind::Unsupported,
        grid::GridError::ResourceLimit(_) => JobErrorKind::ResourceLimit,
    };
    JobError::with_cause(kind, "raster grid admission failed", cause)
}
#[cfg(feature = "native-geospatial")]
fn checked_sum(terms: impl IntoIterator<Item = u64>) -> Result<u64, JobError> {
    terms.into_iter().try_fold(0u64, |sum, term| {
        sum.checked_add(term)
            .ok_or_else(|| error(JobErrorKind::ResourceLimit, "raster owner ledger overflow"))
    })
}
#[cfg(feature = "native-geospatial")]
fn root_work(
    request: &RasterRequest,
    source: &source::SourcePlan,
    staging: &DirectoryStaging<'_>,
    attempt: &Attempt,
    tiles: u64,
) -> Result<u64, JobError> {
    let inventory = tiles
        .checked_add(3)
        .and_then(|n| n.checked_mul(size_of::<Member>() as u64))
        .ok_or_else(|| error(JobErrorKind::ResourceLimit, "raster member ledger overflow"))?;
    // Native/source/grid owners and moved native-version backing are charged
    // by native::planned_work. Root inventory retains three nested traversal
    // shells and fixed names, plus exact generated paths. Opaque std/platform
    // traversal buffers and caller-owned observer allocations are outside this
    // logical owner model, as are opaque native allocations.
    let path = staging
        .path()
        .as_os_str()
        .len()
        .checked_add(33)
        .ok_or_else(|| error(JobErrorKind::ResourceLimit, "raster path ledger overflow"))?;
    checked_sum([
        inventory,
        size_of::<Vec<Member>>() as u64,
        size_of::<RasterRequest>() as u64,
        size_of::<RasterReport>() as u64,
        size_of::<RasterResult>() as u64,
        size_of::<RasterSourceReport>() as u64,
        size_of::<RasterGridReport>() as u64,
        size_of::<native::NativeProduced>() as u64,
        attempt.fixed_owned_bytes() as u64,
        staging.owned_bytes() as u64,
        request.input.capacity() as u64,
        request.output.capacity() as u64,
        32_768, // Two individually bounded JSON members.
        (3 * (size_of::<fs::ReadDir>()
            + size_of::<fs::DirEntry>()
            + size_of::<MemberName>()
            + size_of::<std::ffi::OsString>()
            + 32)
            + 6 * size_of::<PathBuf>()
            + size_of::<fs::Metadata>()
            + size_of::<fs::File>()
            + size_of::<BoundedWriter<'_>>()
            + size_of::<TileJson>()) as u64,
        (path as u64)
            .checked_mul(6)
            .ok_or_else(|| error(JobErrorKind::ResourceLimit, "raster path ledger overflow"))?,
        // At most five R2-generated secondaries: two mutually exclusive
        // native closes, scoped restore, scratch cleanup and a distinct cause
        // beside the existing first abort. Vec push may round five to eight.
        // D2 publication's restore/cleanup branch has fewer records.
        (16 * size_of::<JobError>()
            + 6 * Attempt::failure_record_bytes()
            + size_of::<JobFailure>()
            + size_of::<CleanupDiagnostic>()
            + size_of::<crate::DirectoryRecovery>()
            + 4 * size_of::<PathBuf>()) as u64,
        // Root-owned messages and retained/recovery path backing. Child native
        // diagnostic payload storage is charged separately in planned_work.
        6 * 256,
        6 * native::diagnostic_owner_bytes() as u64,
        (staging.owned_bytes() as u64)
            .max(request.input.capacity() as u64)
            .max(
                source
                    .leaves
                    .iter()
                    .map(|leaf| leaf.path.capacity() as u64)
                    .max()
                    .unwrap_or(0),
            )
            .max(path as u64)
            .checked_add(33)
            .and_then(|n| n.checked_mul(4))
            .ok_or_else(|| {
                error(
                    JobErrorKind::ResourceLimit,
                    "raster recovery path ledger overflow",
                )
            })?,
    ])
}
#[cfg(feature = "native-geospatial")]
fn source_report(facts: &source::SourceFacts) -> RasterSourceReport {
    let mut bands = RasterBands {
        count: facts.bands().len() as u8,
        records: [RasterBandReport::default(); 32],
    };
    for (i, band) in facts.bands().iter().enumerate() {
        bands.records[i] = RasterBandReport {
            datatype: band.datatype.name(),
            color_interp: band.color_interp,
            mask_class: band.mask_class,
        };
    }
    RasterSourceReport {
        driver: facts.driver.name(),
        bytes: facts.source_bytes,
        width: facts.width,
        height: facts.height,
        epsg: match facts.crs {
            grid::StaticCrs::Geographic4326 => 4326,
            grid::StaticCrs::WebMercator3857 => 3857,
        },
        area_point: facts.area_point.as_str(),
        bands,
    }
}
#[cfg(feature = "native-geospatial")]
fn run_native(
    request: RasterRequest,
    zooms: grid::ZoomRange,
    attempt: &Attempt,
) -> Result<RasterResult, JobFailure> {
    let workers = request
        .limits
        .workers
        .min(std::thread::available_parallelism().map_or(1, |n| n.get().min(4) as u8));
    let target =
        DirectoryTarget::prepare(&request.output, request.policy).map_err(|e| attempt.fail(e))?;
    let source = source::admit(
        &request.input,
        target.output(),
        &request.limits,
        &request.display,
        workers,
        attempt,
    )
    .map_err(|e| fail_child(attempt, e))?;
    let facts = source.facts();
    let grid = match grid::plan(
        facts.crs,
        facts.affine,
        [facts.width, facts.height],
        zooms,
        grid::GridCaps {
            max_total_tiles: request.limits.max_tiles,
            max_finest_rgba_bytes: 2 * 1024 * 1024 * 1024,
        },
    ) {
        Ok(grid) => grid,
        Err(e) => return Err(fail_child(attempt, source.finish_failure(grid_error(e)))),
    };
    let source_summary = source_report(facts);
    let rect = grid.finest_rect();
    let grid_summary = RasterGridReport {
        source_affine: facts.affine,
        target_geotransform: grid.target().geotransform,
        target_width: grid.target().width,
        target_height: grid.target().height,
        bounds: grid.geographic_bounds(),
        finest_tiles: [rect.x0, rect.y0, rect.x1, rect.y1],
        min_zoom: zooms.min(),
        max_zoom: zooms.max(),
    };
    let staging = match target.stage_pending(attempt) {
        Ok(stage) => stage,
        Err(error) => return Err(fail_child(attempt, source.finish_failure(error))),
    };
    let work = native::planned_work(&source, &grid, staging.path()).and_then(|own| {
        checked_sum([
            own,
            root_work(&request, &source, &staging, attempt, grid.total_tiles())?,
        ])
    });
    // The ledger charges output-location path lengths, so it gates admission
    // but is not published: report bytes must not depend on where they land.
    match work {
        Ok(work) if work <= request.limits.max_working_bytes => {}
        Ok(_) => {
            let closed = source.finish_failure(error(
                JobErrorKind::ResourceLimit,
                "raster logical working allowance exceeded",
            ));
            let mut failure = staging.fail(closed.error);
            failure.secondary.extend(closed.secondary);
            return Err(failure);
        }
        Err(e) => {
            let closed = source.finish_failure(e);
            let mut failure = staging.fail(closed.error);
            failure.secondary.extend(closed.secondary);
            return Err(failure);
        }
    }
    let native = match native::produce(source, &grid, staging.path(), attempt) {
        Ok(native) => native,
        Err(child) => {
            let mut failure = staging.fail(child.error);
            failure.secondary.extend(child.secondary);
            failure.retained_paths.extend(child.retained_paths);
            return Err(failure);
        }
    };
    let finish = || -> Result<RasterReport, JobError> {
        attempt.check()?;
        if native.tiles != grid.total_tiles()
            || native.finest_rgba_bytes != grid.finest_rgba_bytes()
        {
            return Err(error(
                JobErrorKind::InvalidState,
                "native raster facts disagree with the admitted grid",
            ));
        }
        let mut members = inventory(staging.path(), &grid, request.limits.max_output_bytes)?;
        if members[0].bytes != native.source_cog_bytes {
            return Err(error(
                JobErrorKind::InvalidState,
                "closed raster source COG size changed",
            ));
        }
        let artifact_bytes = checked_sum(members.iter().map(|m| m.bytes))?;
        let report = RasterReport {
            schema_version: 1,
            profile: "r2-source-cog-direct-nearest-pyramid",
            source: source_summary,
            grid: grid_summary,
            display: request.display,
            limits: request.limits,
            tiles: grid.total_tiles(),
            artifact_bytes,
            requested_workers: request.limits.workers,
            resolved_workers: workers,
            worker_policy: "configured cap; native default threshold may select fewer workers",
            native_version: native.native_version,
            resampling: "nearest-direct-finest-east-south-ties",
            color_role_migration: native.role_delta.as_str(),
            source_color_interp: native.source_color_interp,
            target_color_interp: native.target_color_interp,
        };
        let tilejson = TileJson {
            tilejson: "3.0.0",
            scheme: "xyz",
            tiles: ["tiles/{z}/{x}/{y}.png"],
            minzoom: report.grid.min_zoom,
            maxzoom: report.grid.max_zoom,
            bounds: report.grid.bounds,
        };
        let bytes = write_json(
            &native::output_path(staging.path(), "tilejson.json")?,
            &tilejson,
        )?;
        members.push(Member::fixed(bytes, 2));
        check_output_bytes(&members, request.limits.max_output_bytes)?;
        check_output_bytes(&members, request.limits.max_working_bytes).map_err(|_| {
            error(
                JobErrorKind::ResourceLimit,
                "closed raster workspace byte limit exceeded",
            )
        })?;
        attempt.check()?;
        let bytes = write_json(
            &native::output_path(staging.path(), "report.json")?,
            &report,
        )?;
        members.push(Member::fixed(bytes, 3));
        check_output_bytes(&members, request.limits.max_output_bytes)?;
        check_output_bytes(&members, request.limits.max_working_bytes).map_err(|_| {
            error(
                JobErrorKind::ResourceLimit,
                "closed raster workspace byte limit exceeded",
            )
        })?;
        if members.len() as u64 != grid.total_tiles() + 3 {
            return Err(error(
                JobErrorKind::InvalidState,
                "raster required member count disagrees",
            ));
        }
        match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            attempt.emit(&crate::RunEvent::Progress {
                phase: "raster_complete",
                done: report.tiles,
                total: Some(report.tiles),
            })
        })) {
            Ok(observed) => observed?,
            Err(_) => {
                return Err(attempt.check().err().unwrap_or_else(|| {
                    error(
                        JobErrorKind::ObserverFailure,
                        "raster completion observer panicked",
                    )
                }))
            }
        }
        attempt.close_events()?;
        Ok(report)
    };
    let report = match finish() {
        Ok(report) => report,
        Err(e) => return Err(staging.fail(e)),
    };
    let published = staging.seal()?.publish()?;
    Ok(RasterResult {
        output: published.output,
        report,
        cleanup_diagnostics: published.cleanup_diagnostics,
    })
}

#[cfg(feature = "native-geospatial")]
#[derive(Serialize)]
struct TileJson {
    tilejson: &'static str,
    scheme: &'static str,
    tiles: [&'static str; 1],
    minzoom: u8,
    maxzoom: u8,
    bounds: [f64; 4],
}

#[cfg(feature = "native-geospatial")]
fn check_output_bytes(members: &[Member], cap: u64) -> Result<(), JobError> {
    if checked_sum(members.iter().map(|m| m.bytes))? > cap {
        return Err(error(
            JobErrorKind::ResourceLimit,
            "raster completed output byte limit exceeded",
        ));
    }
    Ok(())
}
/// Closed private files only: this is a phase-boundary check, not an in-call
/// filesystem quota. No writer/utility is live when the producer calls it.
#[cfg(feature = "native-geospatial")]
fn check_closed_workspace(stage: &Path, tiles: u64, cap: u64) -> Result<(), JobError> {
    fn add(
        total: &mut u64,
        files: &mut u64,
        entry: &fs::DirEntry,
        cap: u64,
        count: u64,
    ) -> Result<(), JobError> {
        regular(entry, false)?;
        let bytes = entry
            .metadata()
            .map_err(|e| JobError::io("inspect closed raster workspace file", &entry.path(), e))?
            .len();
        *total = checked_sum([*total, bytes])?;
        *files = checked_sum([*files, 1])?;
        if *total > cap {
            return Err(error(
                JobErrorKind::ResourceLimit,
                "closed raster workspace byte limit exceeded",
            ));
        }
        if *files > count {
            return Err(error(
                JobErrorKind::InvalidState,
                "unexpected raster workspace file count",
            ));
        }
        Ok(())
    }
    let count = tiles.checked_add(3).ok_or_else(|| {
        error(
            JobErrorKind::ResourceLimit,
            "raster workspace count overflow",
        )
    })?;
    let (mut total, mut files, mut has_tiles) = (0, 0, false);
    for entry in entries(stage)? {
        let entry = entry.map_err(|e| JobError::io("read closed raster workspace", stage, e))?;
        match entry_name(&entry)?.as_str() {
            "source.cog.tif" | "display-source.tif" | "display-finest.tif" => {
                add(&mut total, &mut files, &entry, cap, count)?
            }
            "tiles" => {
                regular(&entry, true)?;
                has_tiles = true;
            }
            _ => {
                return Err(error(
                    JobErrorKind::InvalidState,
                    "unexpected raster workspace member",
                ))
            }
        }
    }
    if has_tiles {
        let tile_path = native::output_path(stage, "tiles")?;
        for zoom in entries(&tile_path)? {
            let zoom =
                zoom.map_err(|e| JobError::io("read closed workspace zoom", &tile_path, e))?;
            regular(&zoom, true)?;
            let zoom_path = native::output_path(&tile_path, entry_name(&zoom)?.as_str())?;
            for column in entries(&zoom_path)? {
                let column = column
                    .map_err(|e| JobError::io("read closed workspace column", &zoom_path, e))?;
                regular(&column, true)?;
                let column_path = native::output_path(&zoom_path, entry_name(&column)?.as_str())?;
                for tile in entries(&column_path)? {
                    let tile = tile
                        .map_err(|e| JobError::io("read closed workspace tile", &column_path, e))?;
                    let _ = entry_name(&tile)?;
                    add(&mut total, &mut files, &tile, cap, count)?;
                }
            }
        }
    }
    Ok(())
}
#[cfg(feature = "native-geospatial")]
fn number(name: &str) -> Result<u32, JobError> {
    if name.is_empty()
        || (name.len() > 1 && name.starts_with('0'))
        || !name.bytes().all(|b| b.is_ascii_digit())
    {
        return Err(error(
            JobErrorKind::InvalidState,
            "noncanonical raster output address",
        ));
    }
    name.parse()
        .map_err(|_| error(JobErrorKind::InvalidState, "raster output address overflow"))
}
#[cfg(feature = "native-geospatial")]
struct MemberName {
    bytes: [u8; 32],
    len: u8,
}
#[cfg(feature = "native-geospatial")]
impl MemberName {
    fn as_str(&self) -> &str {
        // Construction accepts ASCII bytes only.
        std::str::from_utf8(&self.bytes[..self.len as usize]).expect("ASCII member")
    }
}
#[cfg(feature = "native-geospatial")]
fn entry_name(entry: &fs::DirEntry) -> Result<MemberName, JobError> {
    let name = entry.file_name();
    let text = name
        .to_str()
        .ok_or_else(|| error(JobErrorKind::InvalidState, "non-ASCII raster member"))?;
    if text.len() > 32 || !text.is_ascii() {
        return Err(error(
            JobErrorKind::InvalidState,
            "raster member name exceeds profile",
        ));
    }
    let mut result = MemberName {
        bytes: [0; 32],
        len: text.len() as u8,
    };
    result.bytes[..text.len()].copy_from_slice(text.as_bytes());
    Ok(result)
}
#[cfg(feature = "native-geospatial")]
fn entries(path: &Path) -> Result<fs::ReadDir, JobError> {
    fs::read_dir(path).map_err(|e| JobError::io("enumerate closed raster output", path, e))
}
#[cfg(feature = "native-geospatial")]
fn regular(entry: &fs::DirEntry, directory: bool) -> Result<(), JobError> {
    let ty = entry
        .file_type()
        .map_err(|e| JobError::io("inspect raster member", &entry.path(), e))?;
    if (directory && !ty.is_dir()) || (!directory && !ty.is_file()) {
        return Err(error(
            JobErrorKind::InvalidState,
            "raster output contains a symlink or wrong member type",
        ));
    }
    Ok(())
}
#[cfg(feature = "native-geospatial")]
fn inventory(stage: &Path, grid: &grid::GridPlan, cap: u64) -> Result<Vec<Member>, JobError> {
    #[cfg(test)]
    enter_phase(3);
    let count = usize::try_from(grid.total_tiles().checked_add(3).ok_or_else(|| {
        error(
            JobErrorKind::ResourceLimit,
            "raster inventory count overflow",
        )
    })?)
    .map_err(|_| {
        error(
            JobErrorKind::ResourceLimit,
            "raster inventory cannot fit platform",
        )
    })?;
    let mut members = Vec::new();
    members.try_reserve_exact(count).map_err(|_| {
        error(
            JobErrorKind::ResourceLimit,
            "cannot reserve actual raster inventory",
        )
    })?;
    if members.capacity() != count {
        return Err(error(
            JobErrorKind::ResourceLimit,
            "raster inventory capacity exceeds admitted backing",
        ));
    }
    let cog = native::output_path(stage, "source.cog.tif")?;
    let metadata = fs::symlink_metadata(&cog)
        .map_err(|e| JobError::io("inspect closed source COG", &cog, e))?;
    if !metadata.is_file() {
        return Err(error(
            JobErrorKind::InvalidState,
            "source COG must be a regular file",
        ));
    }
    members.push(Member::fixed(metadata.len(), 0));
    let mut total = metadata.len();
    let mut saw_tiles = false;
    for entry in entries(stage)? {
        let entry = entry.map_err(|e| JobError::io("read raster member", stage, e))?;
        match entry_name(&entry)?.as_str() {
            "source.cog.tif" => regular(&entry, false)?,
            "tiles" => {
                regular(&entry, true)?;
                saw_tiles = true;
            }
            _ => {
                return Err(error(
                    JobErrorKind::InvalidState,
                    "unexpected raster staging member",
                ))
            }
        }
    }
    if !saw_tiles {
        return Err(error(
            JobErrorKind::InvalidState,
            "raster tile directory is missing",
        ));
    }
    drop(cog);
    let tiles = native::output_path(stage, "tiles")?;
    for z in entries(&tiles)? {
        let z = z.map_err(|e| JobError::io("read raster zoom", &tiles, e))?;
        regular(&z, true)?;
        let zoom = number(entry_name(&z)?.as_str())?;
        let rect = grid
            .rectangles()
            .iter()
            .find(|r| u32::from(r.z) == zoom)
            .ok_or_else(|| error(JobErrorKind::InvalidState, "unexpected raster zoom"))?;
        let mut xs = 0u64;
        let zoom_path = native::output_path(&tiles, entry_name(&z)?.as_str())?;
        for x in entries(&zoom_path)? {
            let x = x.map_err(|e| JobError::io("read raster column", &zoom_path, e))?;
            regular(&x, true)?;
            let column = number(entry_name(&x)?.as_str())?;
            if !(rect.x0..=rect.x1).contains(&column) {
                return Err(error(
                    JobErrorKind::InvalidState,
                    "unexpected raster column",
                ));
            }
            xs += 1;
            let mut ys = 0u64;
            let column_path = native::output_path(&zoom_path, entry_name(&x)?.as_str())?;
            for y in entries(&column_path)? {
                let y = y.map_err(|e| JobError::io("read raster tile", &column_path, e))?;
                regular(&y, false)?;
                let filename = entry_name(&y)?;
                let row = number(filename.as_str().strip_suffix(".png").ok_or_else(|| {
                    error(JobErrorKind::InvalidState, "unexpected raster tile suffix")
                })?)?;
                if !(rect.y0..=rect.y1).contains(&row) || members.len() >= count - 2 {
                    return Err(error(
                        JobErrorKind::InvalidState,
                        "unexpected raster row or member count",
                    ));
                }
                let bytes = y
                    .metadata()
                    .map_err(|e| JobError::io("inspect closed PNG", &y.path(), e))?
                    .len();
                total = checked_sum([total, bytes])?;
                if total > cap {
                    return Err(error(
                        JobErrorKind::ResourceLimit,
                        "raster completed output byte limit exceeded",
                    ));
                }
                members.push(Member {
                    bytes,
                    x: column,
                    y: row,
                    z: rect.z,
                    kind: 1,
                    reserved: [0; 6],
                });
                ys += 1;
            }
            if ys != u64::from(rect.y1) - u64::from(rect.y0) + 1 {
                return Err(error(
                    JobErrorKind::InvalidState,
                    "raster column omits required tiles",
                ));
            }
        }
        if xs != u64::from(rect.x1) - u64::from(rect.x0) + 1 {
            return Err(error(
                JobErrorKind::InvalidState,
                "raster zoom omits required columns",
            ));
        }
    }
    if members.len() as u64 != grid.total_tiles() + 1 {
        return Err(error(
            JobErrorKind::InvalidState,
            "raster inventory omits required members",
        ));
    }
    check_output_bytes(&members, cap)?;
    Ok(members)
}
#[cfg(feature = "native-geospatial")]
struct BoundedWriter<'a> {
    file: &'a mut fs::File,
    bytes: u64,
    exceeded: bool,
}
#[cfg(feature = "native-geospatial")]
impl Write for BoundedWriter<'_> {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        if self.bytes + bytes.len() as u64 > 16_384 {
            self.exceeded = true;
            return Err(std::io::Error::other("bounded raster JSON member exceeded"));
        }
        let written = self.file.write(bytes)?;
        self.bytes += written as u64;
        Ok(written)
    }
    fn flush(&mut self) -> std::io::Result<()> {
        self.file.flush()
    }
}

#[cfg(feature = "native-geospatial")]
fn write_json(path: &Path, value: &impl Serialize) -> Result<u64, JobError> {
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .map_err(|e| JobError::io("create raster JSON member", path, e))?;
    let bytes;
    {
        let mut writer = BoundedWriter {
            file: &mut file,
            bytes: 0,
            exceeded: false,
        };
        if let Err(e) = serde_json::to_writer_pretty(&mut writer, value) {
            return Err(if writer.exceeded {
                error(
                    JobErrorKind::ResourceLimit,
                    "raster JSON member exceeds 16 KiB",
                )
            } else {
                JobError::with_cause(JobErrorKind::Io, "write required raster JSON member", e)
            });
        }
        writer
            .flush()
            .map_err(|e| JobError::io("flush raster JSON member", path, e))?;
        bytes = writer.bytes;
    }
    file.sync_all()
        .map_err(|e| JobError::io("finish raster JSON member", path, e))?;
    drop(file);
    Ok(bytes)
}

#[cfg(all(test, feature = "native-geospatial"))]
static PHASE_ENTRIES: [std::sync::atomic::AtomicUsize; 4] =
    [const { std::sync::atomic::AtomicUsize::new(0) }; 4];
#[cfg(all(test, feature = "native-geospatial"))]
fn enter_phase(index: usize) {
    PHASE_ENTRIES[index].fetch_add(1, std::sync::atomic::Ordering::SeqCst);
}
#[cfg(all(test, feature = "native-geospatial"))]
mod owner_controls {
    use super::*;
    fn request(root: &Path, limits: RasterLimits) -> RasterRequest {
        RasterRequest::image(root.join("source.tif"), root.join("output"), 2, 4)
            .with_policy(OutputPolicy::Replace)
            .with_limits(limits)
    }
    fn admitted_work(root: &Path, limits: RasterLimits) -> u64 {
        let request = request(root, limits);
        let control = RunControl::default();
        let attempt = control.begin().unwrap();
        let target = DirectoryTarget::prepare(&request.output, request.policy).unwrap();
        let source = source::admit(
            &request.input,
            target.output(),
            &request.limits,
            &request.display,
            2,
            &attempt,
        )
        .unwrap();
        assert_eq!(source.facts().bands().len(), 3);
        let facts = source.facts();
        let grid = grid::plan(
            facts.crs,
            facts.affine,
            [facts.width, facts.height],
            validate(&request).unwrap(),
            grid::GridCaps {
                max_total_tiles: limits.max_tiles,
                max_finest_rgba_bytes: 2 * 1024 * 1024 * 1024,
            },
        )
        .unwrap();
        let staging = target.stage_pending(&attempt).unwrap();
        let own = native::planned_work(&source, &grid, staging.path()).unwrap();
        let root_own =
            root_work(&request, &source, &staging, &attempt, grid.total_tiles()).unwrap();
        let work = checked_sum([own, root_own]).unwrap();
        println!(
            "R2 actual root Member={} report={} GridPlan={} rootH={} nativeOwn={} L={} N={}",
            size_of::<Member>(),
            size_of::<RasterReport>(),
            size_of::<grid::GridPlan>(),
            root_own,
            own,
            work,
            grid.total_tiles()
        );
        assert_eq!(fs::read_dir(staging.path()).unwrap().count(), 0);
        let closed = source.finish_failure(error(
            JobErrorKind::ResourceLimit,
            "owner control completed",
        ));
        assert!(
            closed.secondary.is_empty(),
            "explicit source/config finish must succeed"
        );
        let failure = staging.fail(closed.error);
        assert!(failure.secondary.is_empty());
        work
    }
    #[test]
    fn tiny_actual_owners_ignore_huge_caps_and_l_minus_one_refuses_before_backing() {
        #[cfg(target_os = "linux")]
        let root = tempfile::tempdir_in("/dev/shm").unwrap();
        #[cfg(not(target_os = "linux"))]
        let root = tempfile::tempdir().unwrap();
        fs::write(
            root.path().join("source.tif"),
            include_bytes!("../tests/fixtures/d1-rgb.tif"),
        )
        .unwrap();
        fs::create_dir(root.path().join("output")).unwrap();
        fs::write(root.path().join("output/previous"), b"original").unwrap();
        let normal = RasterLimits::default();
        let work = admitted_work(root.path(), normal);
        let huge = RasterLimits {
            max_source_bytes: u64::MAX,
            max_source_pixels: u64::MAX,
            max_decoded_bytes: u64::MAX,
            max_tiles: u64::MAX,
            max_output_bytes: u64::MAX,
            max_working_bytes: u64::MAX,
            workers: 2,
        };
        assert_eq!(
            admitted_work(root.path(), huge),
            work,
            "configured allowances cannot size retained owner backing"
        );
        for count in &PHASE_ENTRIES {
            count.store(0, std::sync::atomic::Ordering::SeqCst);
        }
        let failure = raster_to_pyramid(
            request(
                root.path(),
                RasterLimits {
                    max_working_bytes: work - 1,
                    ..normal
                },
            ),
            &RunControl::default(),
        )
        .unwrap_err();
        assert_eq!(failure.error.kind(), JobErrorKind::ResourceLimit);
        assert!(failure.error.message().contains("logical working"));
        assert!(failure.secondary.is_empty());
        for count in &PHASE_ENTRIES {
            assert_eq!(
                count.load(std::sync::atomic::Ordering::SeqCst),
                0,
                "L-1 must precede COG, coherence, display and member backing"
            );
        }
        assert_eq!(
            fs::read(root.path().join("output/previous")).unwrap(),
            b"original"
        );
        assert_eq!(fs::read_dir(root.path()).unwrap().count(), 2);
    }
}
