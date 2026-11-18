//! `mesh-inspect` (#348): a pre-bake check of a `mesh` source.
//!
//! One operation captures the source exactly as `mesh` does (the same capture, cell tree,
//! lattice, incidence pass and shared-border contracts, from the same request options), so
//! its cells are the bake's cells. It then reports, without building any tile:
//!
//! - the source's exact-position topology ([`topology`]): open edges and loops by size class,
//!   primitive boundaries and how many coincide with another primitive's edge, repeated
//!   positions and the open edges welding them closes, connected components;
//! - a coarse-omission forecast per cell ([`super::hlod::forecast`]), labelled an estimate;
//! - optionally a GeoJSON of open loops and seams above a length threshold and of the cells
//!   at risk, in WGS84 when the source is placed (anchor or source CRS), else local metres.
//!
//! The source is read once (capture); later passes read the captured spills. Every sort and
//! sequence is admitted on the scratch, every resident array against the memory budget,
//! before it is written. The report does not depend on `--jobs`: only capture runs workers,
//! and its output is job-independent; every later pass is sequential.
#[cfg(test)]
mod tests;
mod topology;

use super::{
    config::{Jobs, TreeConfig},
    full,
    hlod::{
        self,
        forecast::{self, CellForecast, Outcome, Risk},
    },
    input::StreamSource,
    phases::{Step, StepProgress},
    resources::MemoryBudget,
    source_crs::MeshSourceCrs,
    TilerJobs, TilerProfile,
};
use crate::{
    mesh_archive::ResolvedPlacement, runtime::Staging, CleanupDiagnostic, JobError, JobErrorKind,
    JobFailure, MeshPlacement, OutputPolicy, RunControl, RunEvent,
};
use serde::Serialize;
use std::{
    cell::Cell,
    io::{BufWriter, Write},
    path::{Path, PathBuf},
};

/// Default length threshold of GeoJSON line features, in metres.
pub const DEFAULT_MIN_FEATURE_LENGTH: f64 = 5.0;
/// Most GeoJSON line segments written; longer features first.
const MAX_GEOJSON_SEGMENTS: u64 = 250_000;
/// Runtime and writer reserve of the inspection beside its admitted parts.
const RESERVE_BYTES: u64 = 64 << 20;
/// Cells listed in the report, highest load first.
const TOP_CELLS: usize = 20;

fn invalid(message: impl Into<String>) -> JobError {
    JobError::new(JobErrorKind::InvalidRequest, message.into())
}
fn resource(message: impl Into<String>) -> JobError {
    JobError::new(JobErrorKind::ResourceLimit, message.into())
}

/// What to inspect and how. Source options are `mesh`'s: the same profile, placement and
/// source CRS give the same captured cells.
#[derive(Clone, Debug)]
pub struct InspectRequest {
    input: PathBuf,
    geojson: Option<PathBuf>,
    geojson_local: bool,
    min_feature_length: f64,
    profile: TilerProfile,
    jobs: TilerJobs,
    scratch_dir: Option<PathBuf>,
    placement: MeshPlacement,
    source_crs: Option<MeshSourceCrs>,
    policy: OutputPolicy,
    memory_budget: Option<u64>,
    #[cfg(test)]
    hlod: hlod::Settings,
    #[cfg(test)]
    leaf_triangles: Option<u64>,
    #[cfg(test)]
    test_spill: Option<(crate::spill::SortConfig, usize)>,
}

impl InspectRequest {
    pub fn new(input: impl Into<PathBuf>) -> Self {
        Self {
            input: input.into(),
            geojson: None,
            geojson_local: false,
            min_feature_length: DEFAULT_MIN_FEATURE_LENGTH,
            profile: TilerProfile::Balanced,
            jobs: TilerJobs::Auto,
            scratch_dir: None,
            placement: MeshPlacement::Local,
            source_crs: None,
            policy: OutputPolicy::CreateNew,
            memory_budget: None,
            #[cfg(test)]
            hlod: hlod::Settings::default(),
            #[cfg(test)]
            leaf_triangles: None,
            #[cfg(test)]
            test_spill: None,
        }
    }
    /// Also write a GeoJSON (`.geojson`) of open loops, seams and cells at risk.
    pub fn with_geojson(mut self, path: impl Into<PathBuf>) -> Self {
        self.geojson = Some(path.into());
        self
    }
    /// Write the GeoJSON in local east/north metres instead of WGS84 (RFC 7946). Without
    /// it, a GeoJSON needs a placed source (an anchor or a source CRS).
    pub fn with_geojson_local(mut self, local: bool) -> Self {
        self.geojson_local = local;
        self
    }
    /// Shortest open loop or seam written to the GeoJSON, in metres (default 5).
    pub fn with_min_feature_length(mut self, metres: f64) -> Self {
        self.min_feature_length = metres;
        self
    }
    pub fn with_profile(mut self, profile: TilerProfile) -> Self {
        self.profile = profile;
        self
    }
    pub fn with_jobs(mut self, jobs: TilerJobs) -> Self {
        self.jobs = jobs;
        self
    }
    pub fn with_scratch_dir(mut self, path: impl Into<PathBuf>) -> Self {
        self.scratch_dir = Some(path.into());
        self
    }
    pub fn with_placement(mut self, placement: MeshPlacement) -> Self {
        self.placement = placement;
        self
    }
    pub fn with_source_crs(mut self, source_crs: MeshSourceCrs) -> Self {
        self.source_crs = Some(source_crs);
        self
    }
    /// Whether an existing GeoJSON may be replaced.
    pub fn with_output_policy(mut self, policy: OutputPolicy) -> Self {
        self.policy = policy;
        self
    }
    /// The declared memory budget, as `mesh`'s: a ceiling, never raised.
    pub fn with_memory_budget(mut self, bytes: u64) -> Self {
        self.memory_budget = Some(bytes);
        self
    }
    #[cfg(test)]
    pub(crate) fn with_hlod_settings(mut self, settings: hlod::Settings) -> Self {
        self.hlod = settings;
        self
    }
    #[cfg(test)]
    pub(crate) fn with_test_leaf_triangles(mut self, leaf_triangles: u64) -> Self {
        self.leaf_triangles = Some(leaf_triangles);
        self
    }
    #[cfg(test)]
    pub(crate) fn with_test_spill(mut self, sort: crate::spill::SortConfig, chunk: usize) -> Self {
        self.test_spill = Some((sort, chunk));
        self
    }
}

/// The captured cell tree the forecast ran over.
#[derive(Clone, Debug, Serialize)]
pub struct InspectTree {
    pub leaves: u64,
    pub coarse_cells: u64,
    pub height: u32,
    pub leaf_triangles: u64,
    pub triangle_cap: u64,
}

/// What the bake's own incidence pass sees: positions welded on the leaf-canonical lattice.
#[derive(Clone, Debug, Serialize)]
pub struct HlodView {
    /// Edges with exactly one face after the lattice weld (borders, T-junction sides,
    /// unwelded seams); the bake keeps every one exactly.
    pub open_edges: u64,
    /// Of them, edges whose two ends each meet other leaves (candidate T-junctions and
    /// unwelded seams).
    pub open_edges_with_shared_endpoints: u64,
    /// Source triangles whose corners decode onto fewer than three points.
    pub decoded_degenerate_triangles: u64,
    pub shared_points: u64,
    pub shared_segments: u64,
}

/// One tree depth of the forecast.
#[derive(Clone, Debug, Default, Serialize)]
pub struct DepthForecast {
    pub depth: u32,
    pub cells: u64,
    pub pass_through: u64,
    pub risk_low: u64,
    pub risk_elevated: u64,
    pub risk_high: u64,
    pub risk_certain: u64,
    /// Predicted omitted cells (sources and descendants), as `omitted_by_depth` counts them.
    pub predicted_omitted: u64,
    pub predicted_sources: u64,
    pub max_load: f64,
}

/// The coarse-omission forecast: an estimate, not a bake.
#[derive(Clone, Debug, Serialize)]
pub struct OmissionForecast {
    pub estimate: bool,
    pub note: &'static str,
    pub method: &'static str,
    pub high_load: f64,
    pub elevated_load: f64,
    /// `built`, `recovered` (from a bounded frontier) or `omitted`.
    pub root: &'static str,
    pub predicted_omitted: u64,
    pub predicted_sources: u64,
    /// Sources the bake omits whatever its build does (the contract alone exceeds the cap).
    pub certain_sources: u64,
    pub by_depth: Vec<DepthForecast>,
    /// The cells with the highest load, and every certain source.
    pub top_cells: Vec<CellForecastRow>,
}

/// A reported cell.
#[derive(Clone, Debug, Serialize)]
pub struct CellForecastRow(pub(crate) CellForecast);

/// What the GeoJSON holds.
#[derive(Clone, Debug, Serialize)]
pub struct GeoJsonSummary {
    /// `wgs84` (longitude, latitude in degrees) or `local_east_north_metres` (requested with
    /// `--geojson-local`; not RFC 7946).
    pub coordinates: &'static str,
    pub min_feature_length_m: f64,
    pub open_loops: u64,
    pub seams: u64,
    /// Line features at or above the threshold left out by the segment budget.
    pub omitted_line_features: u64,
    pub cells: u64,
}

/// The inspection report. Deterministic for a given source and request: independent of
/// `--jobs`, the memory budget, the scratch location and the GeoJSON path (which is not in
/// it; see [`InspectResult::geojson`]).
#[derive(Clone, Debug, Serialize)]
pub struct InspectReport {
    pub schema_version: u32,
    pub operation: &'static str,
    /// `local-gltf` or `source-crs`, as the tiler reports.
    pub source_coordinates: &'static str,
    pub source: topology::TopologyReport,
    pub hlod_view: HlodView,
    pub tree: InspectTree,
    pub forecast: OmissionForecast,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub geojson: Option<GeoJsonSummary>,
}

/// Run facts (not part of the deterministic report).
#[derive(Clone, Debug, Serialize)]
pub struct InspectDiagnostics {
    pub requested_jobs: String,
    pub capture_jobs: u64,
    pub capture_jobs_limited_by: &'static str,
    pub memory: InspectMemory,
    pub scratch_peak_bytes: u64,
    pub incidence_scratch_peak_bytes: u64,
    pub capture_seconds: f64,
    pub topology_seconds: f64,
    pub forecast_seconds: f64,
}

/// The budget and what each stage admitted under it.
#[derive(Clone, Debug, Serialize)]
pub struct InspectMemory {
    pub budget: MemoryBudget,
    /// The budget (capped by detected headroom) less memory-backed scratch.
    pub ceiling_bytes: u64,
    /// Captured metadata, the run reserve and the cell intervals, held throughout.
    pub fixed_bytes: u64,
    /// Memory-backed scratch charged to the budget (0 on disk).
    pub memory_backed_scratch_bytes: u64,
    pub capture_bound_bytes: u64,
    /// Largest admitted resident total of the topology pass and of the forecast.
    pub topology_bytes: u64,
    pub forecast_bytes: u64,
}

#[derive(Debug)]
pub struct InspectResult {
    pub report: InspectReport,
    pub diagnostics: InspectDiagnostics,
    /// The installed GeoJSON, when one was requested.
    pub geojson: Option<PathBuf>,
    pub cleanup_diagnostics: Vec<CleanupDiagnostic>,
}

impl InspectResult {
    /// A short human summary of the report.
    pub fn summary_lines(&self) -> Vec<String> {
        summary_lines(&self.report, self.geojson.as_deref())
    }
}

fn thousands(n: u64) -> String {
    let s = n.to_string();
    let mut out = String::new();
    for (i, c) in s.chars().enumerate() {
        if i > 0 && (s.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    out
}

fn summary_lines(r: &InspectReport, geojson: Option<&Path>) -> Vec<String> {
    let s = &r.source;
    let classes = |loops: &topology::Loops| {
        loops
            .by_size_class
            .iter()
            .map(|c| thousands(c.loops))
            .collect::<Vec<_>>()
            .join(" / ")
    };
    let b = &s.primitive_boundaries;
    let f = &r.forecast;
    let high: u64 = f.by_depth.iter().map(|d| d.risk_high).sum();
    let elevated: u64 = f.by_depth.iter().map(|d| d.risk_elevated).sum();
    let root = match f.root {
        "built" => "low risk",
        "recovered" => "rebuilt from its frontier",
        _ => "at risk",
    };
    let mut lines = vec![
        format!(
            "source: {} triangles, {} positions, {} components, {} primitive instances",
            thousands(s.triangles),
            thousands(s.positions),
            thousands(s.components),
            thousands(s.primitive_instances)
        ),
        format!(
            "open edges: {} ({:.1} m) in {} loops; loops <1 m / 1-5 m / 5-20 m / >20 m: {}",
            thousands(s.open.edges),
            s.open.length_m,
            thousands(s.open.loops),
            classes(&s.open)
        ),
        format!(
            "primitive boundaries: {} edges; {} coincide with another primitive's edge, {} \
             unwelded within a primitive, {} open",
            thousands(b.edges.edges),
            thousands(b.coincide_with_other_primitive.edges),
            thousands(b.unwelded_within_primitive.edges),
            thousands(b.open.edges)
        ),
        format!(
            "repeated positions: {} ({} extra vertices); welding them closes {} edges ({:.1} m)",
            thousands(s.repeated_positions.positions),
            thousands(s.repeated_positions.extra_vertices),
            thousands(s.repeated_positions.closes_open_edges.edges),
            s.repeated_positions.closes_open_edges.length_m
        ),
        format!(
            "bake view: {} open edges after the lattice weld; {} coarse cells over {} leaves, \
             height {}",
            thousands(r.hlod_view.open_edges),
            thousands(r.tree.coarse_cells),
            thousands(r.tree.leaves),
            r.tree.height
        ),
        format!(
            "omission risk (an estimate, not a bake): {} high and {} elevated of {} coarse \
             cells, root {root}; certain omissions (contract past the cap): {}",
            thousands(high),
            thousands(elevated),
            thousands(r.tree.coarse_cells),
            thousands(f.certain_sources)
        ),
    ];
    if let (Some(g), Some(path)) = (&r.geojson, geojson) {
        lines.push(format!(
            "geojson: {} ({} open loops, {} seams, {} cells; {})",
            path.display(),
            g.open_loops,
            g.seams,
            g.cells,
            g.coordinates
        ));
    }
    lines
}

/// glTF local metres to GeoJSON coordinates: WGS84 degrees through the root transform, or,
/// when requested, local east/north metres. Captured positions are the tileset's local
/// glTF frame, Y up; the tileset's own Y-up to Z-up convention (`mesh_archive::placement`)
/// makes +X east, +Y up and -Z north, so east is `x` and north is `-z`.
enum Frame {
    Wgs84([f64; 16]),
    Local,
}
impl Frame {
    fn coordinates(&self) -> &'static str {
        match self {
            Self::Wgs84(_) => "wgs84",
            Self::Local => "local_east_north_metres",
        }
    }
    fn point(&self, [x, y, z]: [f64; 3]) -> [f64; 2] {
        let tile = [x, -z, y];
        let Self::Wgs84(m) = self else {
            return [tile[0], tile[1]];
        };
        let ecef: [f64; 3] = std::array::from_fn(|r| {
            m[r] * tile[0] + m[4 + r] * tile[1] + m[8 + r] * tile[2] + m[12 + r]
        });
        geodetic(ecef)
    }
    fn write_point(&self, out: &mut String, p: [f64; 3]) {
        use std::fmt::Write as _;
        let [a, b] = self.point(p);
        if matches!(self, Self::Wgs84(_)) {
            let _ = write!(out, "[{a:.9},{b:.9}]");
        } else {
            let _ = write!(out, "[{a:.4},{b:.4}]");
        }
    }
}

/// The GeoJSON FeatureCollection, written as features stream in.
struct GeoWriter<'f> {
    out: BufWriter<&'f mut std::fs::File>,
    path: &'f Path,
    features: u64,
    buf: String,
}
impl<'f> GeoWriter<'f> {
    fn new(file: &'f mut std::fs::File, path: &'f Path, frame: &Frame) -> Self {
        let mut w = Self {
            out: BufWriter::new(file),
            path,
            features: 0,
            buf: String::new(),
        };
        w.buf.push_str("{\"type\":\"FeatureCollection\",");
        if matches!(frame, Frame::Local) {
            // RFC 7946 coordinates are WGS84; this file says plainly that its are not.
            w.buf.push_str(
                "\"rustyTilesFrame\":{\"wgs84\":false,\"coordinates\":\"local\",\
                 \"units\":\"metre\",\"x\":\"east (glTF +X)\",\"y\":\"north (glTF -Z)\"},",
            );
        }
        w.buf.push_str("\"features\":[");
        w
    }
    fn error(&self, e: std::io::Error) -> JobError {
        JobError::io("write GeoJSON", self.path, e)
    }
    /// Starts a feature: its separator, type and properties, up to its geometry's opening.
    fn begin(&mut self, properties: &serde_json::Value, geometry: &str) {
        if self.features > 0 {
            self.buf.push(',');
        }
        self.features += 1;
        self.buf.push_str("{\"type\":\"Feature\",\"properties\":");
        self.buf.push_str(&properties.to_string());
        self.buf.push_str(",\"geometry\":{\"type\":\"");
        self.buf.push_str(geometry);
        self.buf.push_str("\",\"coordinates\":");
    }
    fn spill(&mut self) -> Result<(), JobError> {
        if self.buf.len() > 1 << 16 {
            let r = self.out.write_all(self.buf.as_bytes());
            r.map_err(|e| self.error(e))?;
            self.buf.clear();
        }
        Ok(())
    }
    fn finish(mut self) -> Result<(), JobError> {
        self.buf.push_str("]}\n");
        let r = self
            .out
            .write_all(self.buf.as_bytes())
            .and_then(|()| self.out.flush());
        r.map_err(|e| self.error(e))
    }
}

/// WGS84 ECEF metres to longitude and latitude in degrees (Bowring iteration; sub-millimetre
/// for terrestrial points).
fn geodetic([x, y, z]: [f64; 3]) -> [f64; 2] {
    const A: f64 = 6_378_137.0;
    const F: f64 = 1.0 / 298.257_223_563;
    let e2 = F * (2.0 - F);
    let lon = y.atan2(x);
    let p = x.hypot(y);
    let mut lat = z.atan2(p * (1.0 - e2));
    for _ in 0..6 {
        let s = lat.sin();
        let n = A / (1.0 - e2 * s * s).sqrt();
        let h = if lat.cos().abs() > 1e-12 {
            p / lat.cos() - n
        } else {
            z.abs() - n * (1.0 - e2)
        };
        lat = z.atan2(p * (1.0 - e2 * n / (n + h)));
    }
    [lon.to_degrees(), lat.to_degrees()]
}

/// Resident admission of one stage: a base plus named allocations, against the ceiling
/// less what is held throughout.
struct Stage<'a> {
    name: &'static str,
    available: u64,
    used: Cell<u64>,
    peak: &'a Cell<u64>,
}
impl Stage<'_> {
    fn admit(&self, what: &'static str, bytes: u64) -> Result<(), JobError> {
        let used = self.used.get().saturating_add(bytes);
        if used > self.available {
            return Err(resource(format!(
                "mesh-inspect {} needs {used} bytes for {what} but the memory budget leaves \
                 {} for it",
                self.name, self.available
            )));
        }
        self.used.set(used);
        self.peak.set(self.peak.get().max(used));
        Ok(())
    }
}

fn check_request(request: &InspectRequest) -> Result<Option<PathBuf>, JobError> {
    if let Some(bytes) = request.memory_budget {
        super::resources::validate_memory_budget(bytes)?;
    }
    if !(request.min_feature_length.is_finite() && request.min_feature_length >= 0.) {
        return Err(invalid(
            "the minimum feature length must be a finite, non-negative number of metres",
        ));
    }
    ResolvedPlacement::resolve(&request.placement)?;
    if let Some(source_crs) = &request.source_crs {
        if request.placement != MeshPlacement::Local {
            return Err(invalid(
                "a source CRS cannot be combined with --anchor, --orientation-xyzw or --scene-offset",
            ));
        }
        source_crs.check()?;
    }
    let Some(path) = &request.geojson else {
        return Ok(None);
    };
    if !request.geojson_local
        && request.placement == MeshPlacement::Local
        && request.source_crs.is_none()
    {
        return Err(invalid(
            "an unplaced source has no WGS84 coordinates for --geojson: give --anchor or \
             --source-crs, or --geojson-local for local east/north metres (not RFC 7946)",
        ));
    }
    if !path
        .extension()
        .is_some_and(|e| e.eq_ignore_ascii_case("geojson"))
    {
        return Err(invalid(
            "the GeoJSON output must have the .geojson extension",
        ));
    }
    match std::fs::symlink_metadata(path) {
        Ok(m) if !m.is_file() => {
            return Err(invalid("the GeoJSON output must be a regular file path"))
        }
        Ok(_) if request.policy == OutputPolicy::CreateNew => {
            return Err(JobError::conflict(
                path,
                std::io::ErrorKind::AlreadyExists.into(),
            ))
        }
        Ok(_) => {}
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => return Err(JobError::io("inspect GeoJSON output", path, e)),
    }
    let output = crate::output_path::resolve(path)?;
    let input = crate::output_path::resolve(&request.input)?;
    if output == input {
        return Err(invalid("the GeoJSON output aliases the source"));
    }
    Ok(Some(output))
}

/// Inspect an admitted `mesh` source: report its topology and forecast its coarse
/// omissions, without baking. See the module notes.
pub fn inspect_mesh(
    request: InspectRequest,
    control: &RunControl,
) -> Result<InspectResult, JobFailure> {
    let attempt = control.begin()?;
    attempt.check().map_err(|e| attempt.fail(e))?;
    let geojson = check_request(&request).map_err(|e| attempt.fail(e))?;
    #[cfg(test)]
    let settings = request.hlod.clone();
    #[cfg(not(test))]
    let settings = hlod::Settings::production();
    let mut config = TreeConfig::new(request.profile.into());
    config.jobs = Jobs::from(request.jobs);
    config.source_crs = request.source_crs.clone();
    config.stop = Some(attempt.probe());
    let budget = MemoryBudget::resolve(request.memory_budget).map_err(|e| attempt.fail(e))?;
    config.memory_budget = Some(budget);
    #[cfg(test)]
    {
        if let Some(leaf_triangles) = request.leaf_triangles {
            config = config.with_leaf_triangles(leaf_triangles);
        }
        if let Some((sort, chunk)) = request.test_spill {
            config.sort = sort;
            config.test_chunk_bytes = Some(chunk);
        }
    }
    config.scratch_dir = request
        .scratch_dir
        .as_deref()
        .map(crate::output_path::resolve)
        .transpose()
        .map_err(|e| attempt.fail(e))?;
    config.validate().map_err(|e| attempt.fail(e))?;
    let rigid = match &request.placement {
        MeshPlacement::Local => None,
        placement => Some(
            ResolvedPlacement::resolve(placement)
                .map_err(|e| attempt.fail(e))?
                .transform(),
        ),
    };
    let source =
        StreamSource::open(&request.input, &mut || attempt.check()).map_err(|e| attempt.fail(e))?;
    if let Some(path) = &geojson {
        source.reject_output(path).map_err(|e| attempt.fail(e))?;
    }
    let mut staging = match &geojson {
        Some(path) => Some(Staging::create(path, &attempt)?),
        None => None,
    };
    let fail = |staging: Option<Staging<'_>>, error: JobError| match staging {
        Some(staging) => staging.fail(error),
        None => attempt.fail(error),
    };
    let emit = |event: &RunEvent<'_>| attempt.emit(event);
    let check = || attempt.check();
    let started = std::time::Instant::now();
    let capture_progress = StepProgress::new(&check, Some(&emit));
    let captured = match attempt
        .emit(&RunEvent::Progress {
            phase: "capture",
            done: 0,
            total: None,
        })
        .map_err(super::failure)
        .and_then(|_| full::capture_admitted(source, &config, &capture_progress))
    {
        Ok(captured) => captured,
        Err(inner) => {
            let mut failure = fail(staging, inner.error);
            failure.secondary.extend(inner.secondary);
            failure.retained_paths.extend(inner.retained_paths);
            return Err(failure);
        }
    };
    let capture_seconds = started.elapsed().as_secs_f64();
    let capture_path = captured.scratch.path().to_owned();
    // A source CRS derives the root placement from the captured origin.
    let placed = captured.placement.as_ref().map(|p| p.transform()).or(rigid);
    let frame = match placed {
        Some(m) if !request.geojson_local => Frame::Wgs84(m),
        _ => Frame::Local,
    };
    let result = (|| {
        attempt.emit(&RunEvent::Progress {
            phase: "capture",
            done: 1,
            total: Some(1),
        })?;
        run(Run {
            request: &request,
            config: &config,
            settings: &settings,
            captured: &captured,
            budget,
            frame: &frame,
            geojson: staging.as_mut().map(|s| s.writer()).zip(geojson.as_deref()),
            check: &check,
            emit: &emit,
            capture_seconds,
        })
    })();
    let cleanup = captured.close();
    let (report, diagnostics) = match (result, cleanup) {
        (Ok(done), Ok(())) => done,
        (Err(error), cleanup) => {
            let mut failure = fail(staging, error);
            if let Err(cleanup) = cleanup {
                failure.secondary.push(cleanup);
                if capture_path.exists() {
                    failure.retained_paths.push(capture_path);
                }
            }
            return Err(failure);
        }
        (Ok(_), Err(error)) => {
            let mut failure = fail(staging, error);
            if capture_path.exists() {
                failure.retained_paths.push(capture_path);
            }
            return Err(failure);
        }
    };
    if let Err(error) = attempt
        .emit(&RunEvent::Progress {
            phase: "ready_to_publish",
            done: 1,
            total: Some(1),
        })
        .and_then(|_| attempt.close_events())
    {
        return Err(fail(staging, error));
    }
    let (geojson, cleanup_diagnostics) = match (staging, geojson) {
        (Some(staging), Some(path)) => {
            let published = staging.seal()?.publish(&path, request.policy)?;
            (Some(published.output), published.cleanup_diagnostics)
        }
        // Dropping the attempt finishes it: nothing to publish.
        _ => (None, Vec::new()),
    };
    Ok(InspectResult {
        report,
        diagnostics,
        geojson,
        cleanup_diagnostics,
    })
}

/// What the passes after capture read: the request, the captured stores, the GeoJSON
/// candidate and the run's hooks.
struct Run<'a> {
    request: &'a InspectRequest,
    config: &'a TreeConfig,
    settings: &'a hlod::Settings,
    captured: &'a full::FullCaptured,
    budget: MemoryBudget,
    frame: &'a Frame,
    /// The staged GeoJSON and its final path.
    geojson: Option<(&'a mut std::fs::File, &'a Path)>,
    check: &'a (dyn Fn() -> Result<(), JobError> + Sync),
    emit: &'a (dyn Fn(&RunEvent<'_>) -> Result<(), JobError> + Sync),
    capture_seconds: f64,
}

/// A line feature's segments joined into ordered polylines: each starts at a point that is
/// not on exactly two of its segments (an end or a branch), in point order, and follows one
/// unused segment at a time until it reaches another such point; what is left is closed
/// rings, each starting and ending at its smallest point. Deterministic.
fn polylines(segments: &[([u32; 3], [u32; 3])]) -> Vec<Vec<[u32; 3]>> {
    use std::collections::BTreeMap;
    let mut at: BTreeMap<[u32; 3], Vec<usize>> = BTreeMap::new();
    for (i, (a, b)) in segments.iter().enumerate() {
        at.entry(*a).or_default().push(i);
        at.entry(*b).or_default().push(i);
    }
    let mut used = vec![false; segments.len()];
    let mut lines = Vec::new();
    let walk = |start: [u32; 3], used: &mut Vec<bool>, lines: &mut Vec<Vec<[u32; 3]>>| {
        while let Some(&first) = at[&start].iter().find(|&&i| !used[i]) {
            let mut line = vec![start];
            let (mut point, mut segment) = (start, first);
            loop {
                used[segment] = true;
                let (a, b) = segments[segment];
                point = if a == point { b } else { a };
                line.push(point);
                let here = &at[&point];
                if here.len() != 2 {
                    break;
                }
                match here.iter().find(|&&i| !used[i]) {
                    Some(&next) => segment = next,
                    None => break,
                }
            }
            lines.push(line);
        }
    };
    let ends: Vec<[u32; 3]> = at
        .iter()
        .filter(|(_, s)| s.len() != 2)
        .map(|(p, _)| *p)
        .collect();
    for p in ends {
        walk(p, &mut used, &mut lines);
    }
    let points: Vec<[u32; 3]> = at.keys().copied().collect();
    for p in points {
        walk(p, &mut used, &mut lines);
    }
    lines
}

/// Resident bytes of [`polylines`] for `segments` segments: the segments, the point index
/// (an ordered map's nodes at about twice their entries) and the output.
fn polyline_bytes(segments: u64) -> u64 {
    segments.saturating_mul(24 + 2 * (12 + 24 + 16) * 2 + 1 + 2 * 12)
}

fn run(run: Run<'_>) -> Result<(InspectReport, InspectDiagnostics), JobError> {
    let Run {
        request,
        config,
        settings,
        captured,
        budget,
        frame,
        geojson,
        check,
        emit,
        capture_seconds,
    } = run;
    let geojson_path = geojson.as_ref().map(|(_, p)| *p);
    let sorter = config.sorter(1);
    let cap = settings.cap;
    let stats = &captured.tree.stats;
    let c = &captured.report;
    // Memory-backed scratch: everything this inspection may write beside capture's files is
    // charged to memory before any of it is written.
    let charged = if captured.scratch.memory_backed() {
        let worst = topology::scratch_need(captured.triangles.len(), sorter)
            .zip(forecast::scratch_need(captured, config))
            .map(|(a, b)| a.max(b))
            .ok_or_else(|| resource("inspection scratch bound overflows"))?;
        captured.scratch.live_bytes().saturating_add(worst)
    } else {
        0
    };
    let ceiling = budget
        .ceiling(super::config::available_memory())
        .saturating_sub(charged);
    let fixed = c
        .memory
        .owned_metadata_bytes
        .saturating_add(c.memory.geometry.document_bytes)
        .saturating_add(RESERVE_BYTES)
        .saturating_add(forecast::interval_bytes(stats.internal));
    let topology_base = topology::memory_need(sorter);
    let forecast_need = forecast::memory_need(captured, config, cap);
    let minimum = fixed.saturating_add(topology_base.max(forecast_need));
    if ceiling < minimum {
        return Err(resource(format!(
            "mesh-inspect needs at least {minimum} bytes after capture (fixed {fixed}, topology \
             {topology_base}, forecast {forecast_need}) but the memory budget allows {ceiling} \
             (budget {}, memory-backed scratch {charged})",
            budget.resolved_bytes
        )));
    }
    let available = ceiling - fixed;
    let cells = forecast::cell_intervals(&captured.tree.nodes, stats.internal)?;

    // Topology.
    let topology_started = std::time::Instant::now();
    let topology_peak = Cell::new(0u64);
    let stage = Stage {
        name: "topology",
        available,
        used: Cell::new(0),
        peak: &topology_peak,
    };
    stage.admit("sorters", topology_base)?;
    let memory = |what, bytes| stage.admit(what, bytes);
    let admit_scratch = |bound: u64| {
        if captured.scratch.memory_backed() && bound > charged {
            return Err(resource(format!(
                "mesh-inspect needs {bound} bytes of memory-backed scratch, past the {charged} \
                 charged to memory"
            )));
        }
        crate::spill::preflight(
            captured.scratch.path(),
            bound.saturating_sub(captured.scratch.live_bytes()),
            crate::spill::SpaceRole::Scratch,
            crate::spill::PreflightStage::Inspection,
        )
        .map(|_| ())
        .map_err(JobError::from)
    };
    let progress = StepProgress::new(check, Some(emit)).merging_as(Step::InspectMerge);
    let geo_request = geojson.is_some().then_some(topology::GeoRequest {
        min_length: request.min_feature_length,
        max_segments: MAX_GEOJSON_SEGMENTS,
    });
    let admit = topology::Admit {
        memory: &memory,
        scratch: &admit_scratch,
    };
    let topology = topology::run(topology::Input {
        scratch: &captured.scratch,
        sorter,
        triangles: &captured.triangles,
        nodes: &captured.tree.nodes,
        leaves: stats.leaves,
        cells: &cells,
        geo: geo_request,
        admit: &admit,
        progress: &progress,
    })?;
    let topology::Topology {
        report: source,
        leaves,
        geo,
    } = topology;
    drop(cells);
    let mut writer = geojson.map(|(file, path)| GeoWriter::new(file, path, frame));
    let state = |e: serde_json::Error| JobError::new(JobErrorKind::InvalidState, e.to_string());
    let mut line_counts = (0u64, 0u64, 0u64);
    if let (Some(w), Some(geo)) = (writer.as_mut(), geo) {
        let topology::GeoLines {
            features,
            segments,
            omitted,
        } = geo;
        line_counts.2 = omitted;
        // One feature's segments at a time (at most the segment budget), joined into
        // ordered polylines.
        stage.admit("geojson_polylines", polyline_bytes(MAX_GEOJSON_SEGMENTS))?;
        let mut write_feature = |feature: u32,
                                 segments: &[([u32; 3], [u32; 3])],
                                 w: &mut GeoWriter<'_>|
         -> Result<(), JobError> {
            let index = usize::try_from(feature).map_err(|_| resource("feature index"))?;
            let f = features.get(index).ok_or_else(|| {
                JobError::new(JobErrorKind::InvalidState, "a segment without its feature")
            })?;
            if f.kind == "seam" {
                line_counts.1 += 1;
            } else {
                line_counts.0 += 1;
            }
            w.begin(&serde_json::to_value(f).map_err(state)?, "MultiLineString");
            w.buf.push('[');
            let p = |k: [u32; 3]| k.map(|v| f64::from(f32::from_bits(v)));
            for (i, line) in polylines(segments).iter().enumerate() {
                if i > 0 {
                    w.buf.push(',');
                }
                w.buf.push('[');
                for (j, point) in line.iter().enumerate() {
                    if j > 0 {
                        w.buf.push(',');
                    }
                    frame.write_point(&mut w.buf, p(*point));
                }
                w.buf.push(']');
                w.spill()?;
            }
            w.buf.push_str("]}}");
            w.spill()
        };
        let mut current: Option<u32> = None;
        let mut held: Vec<([u32; 3], [u32; 3])> = Vec::new();
        for (i, s) in segments.enumerate() {
            if i % 65_536 == 0 {
                check()?;
            }
            let s = s?;
            if let Some(feature) = current.filter(|&f| f != s.feature) {
                write_feature(feature, &held, w)?;
                held.clear();
            }
            current = Some(s.feature);
            held.push((s.pa, s.pb));
        }
        if let Some(feature) = current {
            write_feature(feature, &held, w)?;
        }
    }
    let topology_seconds = topology_started.elapsed().as_secs_f64();

    // Forecast.
    let forecast_started = std::time::Instant::now();
    let forecast_peak = Cell::new(0u64);
    let stage = Stage {
        name: "forecast",
        available,
        used: Cell::new(0),
        peak: &forecast_peak,
    };
    stage.admit(
        "leaf_facts",
        stats.leaves.saturating_mul(forecast::LEAF_FACT_BYTES),
    )?;
    stage.admit("incidence_and_borders", forecast_need)?;
    let incidence_progress = StepProgress::new(check, Some(emit)).merging_as(Step::IncidenceMerge);
    let forecast = forecast::forecast(
        captured,
        config,
        settings,
        &leaves,
        charged,
        &incidence_progress,
        check,
    )?;
    drop(leaves);
    let mut cell_features = 0u64;
    if let Some(w) = writer.as_mut() {
        for cell in forecast
            .cells
            .iter()
            .filter(|c| c.risk >= Risk::Elevated || c.outcome.omitted())
        {
            check()?;
            cell_features += 1;
            let mut props = serde_json::to_value(cell).map_err(state)?;
            props["kind"] = serde_json::json!("cell");
            w.begin(&props, "Polygon");
            let (lo, hi) = (
                cell.bounds.min.map(f64::from),
                cell.bounds.max.map(f64::from),
            );
            let y = 0.5 * (lo[1] + hi[1]);
            // Counter-clockwise in east/north (RFC 7946); -Z is north.
            let ring = [
                [lo[0], y, hi[2]],
                [hi[0], y, hi[2]],
                [hi[0], y, lo[2]],
                [lo[0], y, lo[2]],
                [lo[0], y, hi[2]],
            ];
            w.buf.push_str("[[");
            for (i, p) in ring.iter().enumerate() {
                if i > 0 {
                    w.buf.push(',');
                }
                frame.write_point(&mut w.buf, *p);
            }
            w.buf.push_str("]]}}");
            w.spill()?;
        }
    }
    if let Some(w) = writer {
        w.finish()?;
    }

    let forecast_seconds = forecast_started.elapsed().as_secs_f64();
    let report = InspectReport {
        schema_version: 1,
        operation: "mesh_inspect",
        source_coordinates: if c.source_crs.is_some() {
            "source-crs"
        } else {
            "local-gltf"
        },
        source,
        hlod_view: HlodView {
            open_edges: forecast.incidence.source_open_edges,
            open_edges_with_shared_endpoints: forecast
                .incidence
                .source_open_edges_with_shared_endpoints,
            decoded_degenerate_triangles: forecast.incidence.decoded_degenerate,
            shared_points: forecast.incidence.shared_points,
            shared_segments: forecast.incidence.shared_segments,
        },
        tree: InspectTree {
            leaves: stats.leaves,
            coarse_cells: stats.internal,
            height: stats.height,
            leaf_triangles: config.leaf_triangles(),
            triangle_cap: cap as u64,
        },
        forecast: summarize(&forecast.cells, stats.height),
        geojson: geojson_path.map(|_| GeoJsonSummary {
            coordinates: frame.coordinates(),
            min_feature_length_m: request.min_feature_length,
            open_loops: line_counts.0,
            seams: line_counts.1,
            omitted_line_features: line_counts.2,
            cells: cell_features,
        }),
    };
    let diagnostics = InspectDiagnostics {
        requested_jobs: c.jobs.requested.clone(),
        capture_jobs: c.jobs.jobs,
        capture_jobs_limited_by: c.jobs.limited_by,
        memory: InspectMemory {
            budget,
            ceiling_bytes: ceiling,
            fixed_bytes: fixed,
            memory_backed_scratch_bytes: charged,
            capture_bound_bytes: c.memory.bound_bytes,
            topology_bytes: topology_peak.get(),
            forecast_bytes: forecast_peak.get(),
        },
        scratch_peak_bytes: captured.scratch.peak_bytes(),
        incidence_scratch_peak_bytes: forecast.incidence_scratch_peak_bytes,
        capture_seconds,
        topology_seconds,
        forecast_seconds,
    };
    Ok((report, diagnostics))
}

/// The forecast's per-depth summary and top cells.
fn summarize(cells: &[CellForecast], height: u32) -> OmissionForecast {
    let mut by_depth: Vec<DepthForecast> = (0..height)
        .map(|depth| DepthForecast {
            depth,
            ..Default::default()
        })
        .collect();
    for c in cells {
        let Some(d) = by_depth.get_mut(c.depth as usize) else {
            continue;
        };
        d.cells += 1;
        match c.risk {
            Risk::PassThrough => d.pass_through += 1,
            Risk::Low => d.risk_low += 1,
            Risk::Elevated => d.risk_elevated += 1,
            Risk::High => d.risk_high += 1,
            Risk::Certain => d.risk_certain += 1,
        }
        d.predicted_omitted += u64::from(c.outcome.omitted());
        d.predicted_sources += u64::from(c.outcome == Outcome::Source);
        d.max_load = d.max_load.max(c.load);
    }
    let root = cells
        .iter()
        .find(|c| c.depth == 0)
        .map_or("built", |c| match c.outcome {
            Outcome::Built => "built",
            Outcome::Recovered => "recovered",
            Outcome::Source | Outcome::Descendant => "omitted",
        });
    let mut top: Vec<&CellForecast> = cells
        .iter()
        .filter(|c| c.risk != Risk::PassThrough)
        .collect();
    let certain_source = |c: &CellForecast| c.risk == Risk::Certain && c.outcome == Outcome::Source;
    top.sort_by(|a, b| {
        certain_source(b)
            .cmp(&certain_source(a))
            .then_with(|| b.load.total_cmp(&a.load))
            .then_with(|| (a.depth, a.first).cmp(&(b.depth, b.first)))
    });
    let certain = cells.iter().filter(|c| certain_source(c)).count();
    top.truncate(TOP_CELLS.max(certain));
    OmissionForecast {
        estimate: true,
        note: "An estimate from border and open-boundary density against the triangle cap, \
               not a bake; only `certain` sources are certain. Omission propagates to every \
               ancestor unless the root recovers from a bounded frontier.",
        method: "load = (children's contract points + open edges on loops crossing the cell \
                 border, resampled at the children's capped resolution) / cap",
        high_load: forecast::HIGH_LOAD,
        elevated_load: forecast::ELEVATED_LOAD,
        root,
        predicted_omitted: by_depth.iter().map(|d| d.predicted_omitted).sum(),
        predicted_sources: by_depth.iter().map(|d| d.predicted_sources).sum(),
        certain_sources: certain as u64,
        by_depth,
        top_cells: top
            .into_iter()
            .map(|c| CellForecastRow(c.clone()))
            .collect(),
    }
}
