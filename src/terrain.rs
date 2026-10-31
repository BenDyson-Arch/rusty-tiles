//! Finite T1 raw-metre terrain meshes published as a 3D Tiles 1.1 directory.
//!
//! Full-detail leaves are the T1 lattice patches. Coarser levels decimate the
//! same lattice (no new heights), each tile states a certified bound on its
//! distance to the leaf mesh, interior skirts cover cracks between levels, and
//! the hierarchy is published as the bounded implicit forest. NoData is a hole:
//! only triangles whose three lattice nodes are valid are emitted.
#[cfg(feature = "native-geospatial")]
use crate::{
    runtime::{directory::DirectoryTarget, Attempt},
    RunEvent,
};
use crate::{CleanupDiagnostic, JobError, JobErrorKind, JobFailure, OutputPolicy, RunControl};
use serde::Serialize;
use std::path::PathBuf;
#[cfg(feature = "native-geospatial")]
use std::{fs, io::Write, path::Path};
#[cfg(feature = "native-geospatial")]
mod glb;
#[cfg(feature = "native-geospatial")]
mod raster;
#[cfg(feature = "native-geospatial")]
use glb::Storage;
#[derive(Clone, Debug)]
pub struct TerrainOptions {
    pub cells_per_leaf: u16,
}
impl TerrainOptions {
    pub fn new(cells_per_leaf: u16) -> Self {
        Self { cells_per_leaf }
    }
}
#[derive(Clone, Debug)]
pub enum TerrainHeights {
    RawMetres { height_offset_metres: f64 },
}
#[derive(Clone, Debug)]
pub struct TerrainRequest {
    input: PathBuf,
    output: PathBuf,
    heights: TerrainHeights,
    options: TerrainOptions,
    policy: OutputPolicy,
}
impl TerrainRequest {
    pub fn new(
        input: impl Into<PathBuf>,
        output: impl Into<PathBuf>,
        heights: TerrainHeights,
        options: TerrainOptions,
    ) -> Self {
        Self {
            input: input.into(),
            output: output.into(),
            heights,
            options,
            policy: OutputPolicy::CreateNew,
        }
    }
    pub fn with_policy(mut self, policy: OutputPolicy) -> Self {
        self.policy = policy;
        self
    }
}
#[derive(Clone, Debug, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct TerrainReport {
    pub schema_version: u64,
    pub profile: &'static str,
    pub source_crs: String,
    pub source_bytes: u64,
    pub width: usize,
    pub height: usize,
    pub bounds: [f64; 4],
    pub source_pixel_degrees: [f64; 2],
    pub cells_per_leaf: u16,
    /// Pyramid levels; the last holds the full-detail leaves.
    pub levels: u32,
    pub tiles: u64,
    pub tiles_per_level: Vec<u64>,
    /// Surface vertices of every tile. Skirts reuse those as their tops and
    /// add `skirt_vertices` bottoms.
    pub vertices: u64,
    pub skirt_vertices: u64,
    pub height_offset_metres: f64,
    /// The largest distance from a decoded full-detail (leaf) vertex, surface
    /// or skirt, to the geodetic position it stores.
    pub position_error_metres: f64,
    pub height_range: [f64; 2],
    pub generated_bytes: u64,
    pub surface_reference: &'static str,
    /// Per level, coarsest first: the glTF (y-up) axis steps (node scale) of
    /// uint16 quantized positions, or `None` for Float32 positions.
    pub quantization_step_metres: Vec<Option<[f64; 3]>>,
    /// The largest certified geometric error per level, coarsest first.
    pub geometric_error_metres: Vec<f64>,
    pub root_geometric_error_metres: f64,
    pub tileset_geometric_error_metres: f64,
    /// The smallest and largest skirt depth, absent without interior edges.
    pub skirt_depth_metres: Option<[f64; 2]>,
}
#[derive(Debug)]
pub struct TerrainResult {
    pub output: PathBuf,
    pub report: TerrainReport,
    pub cleanup_diagnostics: Vec<CleanupDiagnostic>,
}
fn error(kind: JobErrorKind, message: impl Into<String>) -> JobError {
    JobError::new(kind, message)
}
#[cfg(feature = "native-geospatial")]
fn overlap(source: &Path, output: &Path) -> Result<(), JobError> {
    let m = match fs::symlink_metadata(output) {
        Ok(m) => m,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(e) => return Err(JobError::io("inspect terrain overlap", output, e)),
    };
    if m.file_type().is_symlink() {
        return Ok(());
    }
    let candidates: Vec<_> = if m.is_dir() {
        source.ancestors().skip(1).collect()
    } else {
        vec![source]
    };
    for candidate in candidates {
        if same_file::is_same_file(candidate, output)
            .map_err(|e| JobError::io("compare terrain source/output", output, e))?
        {
            return Err(error(
                JobErrorKind::InvalidRequest,
                "terrain output overlaps its source",
            ));
        }
    }
    Ok(())
}
#[cfg(feature = "native-geospatial")]
fn finish_member<W: Write>(w: &mut W, b: &[u8]) -> std::io::Result<()> {
    w.write_all(b)?;
    w.flush()
}
#[cfg(feature = "native-geospatial")]
fn count(bytes: &mut u64, length: u64) -> Result<(), JobError> {
    *bytes = bytes
        .checked_add(length)
        .ok_or_else(|| error(JobErrorKind::Unsupported, "terrain byte budget overflow"))?;
    if *bytes > 1 << 30 {
        return Err(error(
            JobErrorKind::Unsupported,
            "terrain output exceeds 1 GiB",
        ));
    }
    Ok(())
}
/// Create, count and flush a staged member. Members that are published are
/// synced once by [`seal`]; payloads the forest relocates are never synced.
#[cfg(feature = "native-geospatial")]
fn write(path: &Path, b: &[u8], bytes: &mut u64) -> Result<(), JobError> {
    count(bytes, b.len() as u64)?;
    let mut f = fs::File::options()
        .write(true)
        .create_new(true)
        .open(path)
        .map_err(|e| JobError::io("create terrain member", path, e))?;
    finish_member(&mut f, b).map_err(|e| JobError::io("finish terrain member", path, e))
}
/// Sync a finished member, including those the implicit forest writes without syncing.
#[cfg(feature = "native-geospatial")]
fn seal(path: &Path) -> Result<(), JobError> {
    fs::File::options()
        .write(true)
        .open(path)
        .and_then(|file| file.sync_all())
        .map_err(|e| JobError::io("sync terrain member", path, e))
}
#[cfg(feature = "native-geospatial")]
#[derive(Clone)]
struct Frame {
    origin: [f64; 3],
    axes: [[f64; 3]; 3],
}
#[cfg(feature = "native-geospatial")]
impl Frame {
    fn new(bounds: [f64; 4]) -> Self {
        let lon = (bounds[0] + bounds[2]) / 2.;
        let lat = (bounds[1] + bounds[3]) / 2.;
        let (l, p) = (lon.to_radians(), lat.to_radians());
        Self {
            origin: crate::georef::geodetic_to_ecef(crate::georef::Cartographic::new(lon, lat, 0.)),
            axes: [
                [-l.sin(), l.cos(), 0.],
                [-p.sin() * l.cos(), -p.sin() * l.sin(), p.cos()],
                [p.cos() * l.cos(), p.cos() * l.sin(), p.sin()],
            ],
        }
    }
    fn local(&self, p: [f64; 3]) -> [f64; 3] {
        std::array::from_fn(|axis| {
            (0..3)
                .map(|i| (p[i] - self.origin[i]) * self.axes[axis][i])
                .sum()
        })
    }
    fn transform(&self) -> [f64; 16] {
        [
            self.axes[0][0],
            self.axes[0][1],
            self.axes[0][2],
            0.,
            self.axes[1][0],
            self.axes[1][1],
            self.axes[1][2],
            0.,
            self.axes[2][0],
            self.axes[2][1],
            self.axes[2][2],
            0.,
            self.origin[0],
            self.origin[1],
            self.origin[2],
            1.,
        ]
    }
}
#[cfg(feature = "native-geospatial")]
fn lattice(start: usize, end: usize, stride: usize) -> Vec<usize> {
    let mut nodes: Vec<usize> = (start..end).step_by(stride).collect();
    nodes.push(end);
    nodes
}
/// One tile of the decimated lattice pyramid. Level `L` of `D` keeps every
/// `2^(D-L)`-th leaf lattice node plus the clipped far edges, so its vertices
/// are leaf vertices and leaves (level `D`) are the full-detail T1 patches.
#[cfg(feature = "native-geospatial")]
#[derive(Clone, Debug)]
struct Patch {
    level: u32,
    column: usize,
    row: usize,
    stride: usize,
    x: usize,
    y: usize,
    x1: usize,
    y1: usize,
    /// Whether it has surface triangles; one without has descendants that do.
    content: bool,
    children: Vec<usize>,
    /// Certified bound on the distance from this surface to the leaf surface;
    /// for a tile without triangles, the routing floor.
    error: f64,
    /// The largest error of this tile and its descendants that draw.
    drawn: f64,
    /// Below the west, east, south and north edges; zero where no segment is
    /// skirted.
    skirt_depth: [f64; 4],
    /// Decoded surface vertices, then surface and skirt vertices.
    surface: [[f64; 3]; 2],
    bounds: [[f64; 3]; 2],
    /// Own vertices and every descendant box.
    hierarchy: [[f64; 3]; 2],
}
#[cfg(feature = "native-geospatial")]
impl Patch {
    fn name(&self) -> String {
        format!("tiles/{}/{}/{}.glb", self.level, self.row, self.column)
    }
    fn columns(&self) -> Vec<usize> {
        lattice(self.x, self.x1, self.stride)
    }
    fn rows(&self) -> Vec<usize> {
        lattice(self.y, self.y1, self.stride)
    }
    fn slot(&self) -> usize {
        (self.column & 1) | (self.row & 1) << 1
    }
}
#[cfg(feature = "native-geospatial")]
fn coordinates(raster: &raster::PreparedRaster, x: usize, y: usize) -> (f64, f64) {
    let lon = if x == raster.width {
        raster.bounds[2]
    } else {
        raster.bounds[0] + x as f64 * raster.pixel[0]
    };
    let lat = if y == raster.height {
        raster.bounds[3]
    } else {
        raster.bounds[1] + y as f64 * raster.pixel[1]
    };
    (lon, lat)
}
#[cfg(feature = "native-geospatial")]
fn surface_height(
    raster: &raster::PreparedRaster,
    lon: f64,
    lat: f64,
    offset: f64,
) -> Result<Option<f64>, JobError> {
    let Some(height) = raster.sample(lon, lat).map(|h| h + offset) else {
        return Ok(None);
    };
    if !height.is_finite() {
        return Err(error(JobErrorKind::InvalidInput, "terrain offset overflow"));
    }
    if !(-10000.0..=8000.0).contains(&height) {
        return Err(error(
            JobErrorKind::Unsupported,
            "terrain sampled height must be within -10000..8000 metres for supported scene queries",
        ));
    }
    Ok(Some(height))
}
/// The z-up frame position of a decoded glTF (y-up) vertex.
#[cfg(feature = "native-geospatial")]
fn decoded(p: [f64; 3]) -> [f64; 3] {
    [p[0], -p[2], p[1]]
}
/// The stored (decoded glTF) position of a geodetic point, its decoded frame
/// position and storage error.
#[cfg(feature = "native-geospatial")]
fn place(
    frame: &Frame,
    storage: Storage,
    lon: f64,
    lat: f64,
    height: f64,
) -> Result<([f64; 3], [f64; 3], f64), JobError> {
    let p = crate::georef::geodetic_to_ecef(crate::georef::Cartographic::new(lon, lat, height));
    let local = frame.local(p);
    let stored = storage.store([local[0], local[2], -local[1]]);
    if !stored.iter().all(|v| v.is_finite()) {
        return Err(error(
            JobErrorKind::Unsupported,
            "terrain local positions exceed finite storage",
        ));
    }
    let position = decoded(stored);
    let error = ((0..3)
        .map(|axis| (position[axis] - local[axis]).powi(2))
        .sum::<f64>())
    .sqrt();
    Ok((stored, position, error))
}
#[cfg(feature = "native-geospatial")]
fn extend(bounds: &mut [[f64; 3]; 2], p: [f64; 3]) {
    for axis in 0..3 {
        bounds[0][axis] = bounds[0][axis].min(p[axis]);
        bounds[1][axis] = bounds[1][axis].max(p[axis]);
    }
}
#[cfg(feature = "native-geospatial")]
fn grid(
    pyramid: &Pyramid,
    xs: &[usize],
    ys: &[usize],
    storage: Storage,
) -> Result<glb::Grid, JobError> {
    let (raster, frame, attempt) = (pyramid.raster, pyramid.frame, pyramid.attempt);
    let columns = xs.len();
    let rows = ys.len();
    let mut positions = Vec::with_capacity(columns * rows);
    let mut heights = Vec::with_capacity(columns * rows);
    for &y in ys {
        attempt.check()?;
        for &x in xs {
            let (lon, lat) = coordinates(raster, x, y);
            let height = surface_height(raster, lon, lat, pyramid.offset)?;
            positions.push(place(frame, storage, lon, lat, height.unwrap_or(0.))?);
            heights.push(height);
        }
    }
    // Emit the triangles of three valid nodes. Reject topology that storage
    // collapses or turns away from the footprint's geodetic normal: scene
    // height queries require a surface.
    let mut used = vec![false; columns * rows];
    for row in 0..rows - 1 {
        attempt.check()?;
        for col in 0..columns - 1 {
            let i = row * columns + col;
            let cell = [
                [i, i + 1, i + columns],
                [i + 1, i + columns + 1, i + columns],
            ]
            .map(|t| (t, t.iter().all(|&k| heights[k].is_some())));
            if !cell[0].1 && !cell[1].1 {
                continue;
            }
            let longitude = raster.bounds[0]
                + xs[col] as f64 * raster.pixel[0]
                + (xs[col + 1] - xs[col]) as f64 * raster.pixel[0] / 2.;
            let latitude = raster.bounds[1]
                + ys[row] as f64 * raster.pixel[1]
                + (ys[row + 1] - ys[row]) as f64 * raster.pixel[1] / 2.;
            let (l, p) = (longitude.to_radians(), latitude.to_radians());
            let up = [p.cos() * l.cos(), p.cos() * l.sin(), p.sin()];
            let up: [f64; 3] =
                std::array::from_fn(|axis| (0..3).map(|j| up[j] * frame.axes[axis][j]).sum());
            for (t, _) in cell.into_iter().filter(|(_, valid)| *valid) {
                t.iter().for_each(|&k| used[k] = true);
                let [a, b, c] = t.map(|k| positions[k].0);
                if !triangle_is_surface(a, b, c, up) {
                    return Err(error(
                        JobErrorKind::Unsupported,
                        "terrain stored topology collapses or reverses a footprint triangle",
                    ));
                }
            }
        }
    }
    let mut bounds = [[f64::INFINITY; 3], [f64::NEG_INFINITY; 3]];
    let mut max_position_error: f64 = 0.;
    let mut height_range = [f64::INFINITY, f64::NEG_INFINITY];
    for (k, &(_, position, error)) in positions.iter().enumerate().filter(|(k, _)| used[*k]) {
        let height = heights[k].expect("used nodes are valid");
        height_range[0] = height_range[0].min(height);
        height_range[1] = height_range[1].max(height);
        extend(&mut bounds, position);
        max_position_error = max_position_error.max(error);
    }
    let positions = positions
        .into_iter()
        .map(|(encoded, _, _)| encoded)
        .collect();
    Ok(glb::Grid {
        positions,
        used,
        columns,
        rows,
        bounds,
        max_position_error,
        height_range,
    })
}
#[cfg(feature = "native-geospatial")]
fn triangle_is_surface(a: [f64; 3], b: [f64; 3], c: [f64; 3], up: [f64; 3]) -> bool {
    let (a, b, c) = (decoded(a), decoded(b), decoded(c));
    let u: [f64; 3] = std::array::from_fn(|i| b[i] - a[i]);
    let v: [f64; 3] = std::array::from_fn(|i| c[i] - a[i]);
    let cross = [
        u[1] * v[2] - u[2] * v[1],
        u[2] * v[0] - u[0] * v[2],
        u[0] * v[1] - u[1] * v[0],
    ];
    cross.iter().all(|v| v.is_finite())
        && cross.iter().map(|v| v * v).sum::<f64>() > 0.
        && (0..3).map(|i| cross[i] * up[i]).sum::<f64>() > 0.
}
#[cfg(feature = "native-geospatial")]
fn box_for(bounds: [[f64; 3]; 2]) -> [f64; 12] {
    let center: [f64; 3] = std::array::from_fn(|i| (bounds[0][i] + bounds[1][i]) / 2.);
    let half: [f64; 3] = std::array::from_fn(|i| {
        ((bounds[1][i] - bounds[0][i]) / 2. + 1e-8 + 64. * f64::EPSILON * center[i].abs().max(1.))
            .next_up()
    });
    [
        center[0], center[1], center[2], half[0], 0., 0., 0., half[1], 0., 0., 0., half[2],
    ]
}
/// A tile's decoded surface over its lattice nodes, evaluated at lattice
/// parameters on the encoder's triangles: `(c,r),(c+1,r),(c,r+1)` and
/// `(c+1,r),(c+1,r+1),(c,r+1)`, where all three nodes are `used`.
#[cfg(feature = "native-geospatial")]
struct Sheet {
    xs: Vec<usize>,
    ys: Vec<usize>,
    positions: Vec<[f64; 3]>,
    used: Vec<bool>,
}
#[cfg(feature = "native-geospatial")]
impl Sheet {
    fn vertex(&self, c: usize, r: usize) -> [f64; 3] {
        decoded(self.positions[r * self.xs.len() + c])
    }
    /// Whether cell `(c, r)`'s lower or upper triangle is emitted.
    fn emitted(&self, c: usize, r: usize, upper: bool) -> bool {
        let (i, n) = (r * self.xs.len() + c, self.xs.len());
        let t = if upper {
            [i + 1, i + n + 1, i + n]
        } else {
            [i, i + 1, i + n]
        };
        t.iter().all(|&k| self.used[k])
    }
    /// The point at `(x, y)` on cell `(c, r)`, if a triangle there covers it.
    fn cell(&self, c: usize, r: usize, x: f64, y: f64) -> Option<[f64; 3]> {
        let u = (x - self.xs[c] as f64) / (self.xs[c + 1] - self.xs[c]) as f64;
        let v = (y - self.ys[r] as f64) / (self.ys[r + 1] - self.ys[r]) as f64;
        let lower = u + v <= 1. && self.emitted(c, r, false);
        (lower || u + v >= 1. && self.emitted(c, r, true)).then(|| self.plane(c, r, x, y))
    }
    /// The point at `(x, y)` on cell `(c, r)`'s two triangles, emitted or not.
    fn plane(&self, c: usize, r: usize, x: f64, y: f64) -> [f64; 3] {
        let u = (x - self.xs[c] as f64) / (self.xs[c + 1] - self.xs[c]) as f64;
        let v = (y - self.ys[r] as f64) / (self.ys[r + 1] - self.ys[r]) as f64;
        // Lattice nodes are the stored vertices themselves.
        if (u == 0. || u == 1.) && (v == 0. || v == 1.) {
            return self.vertex(c + u as usize, r + v as usize);
        }
        let (b, c_) = (self.vertex(c + 1, r), self.vertex(c, r + 1));
        if u + v <= 1. {
            let a = self.vertex(c, r);
            std::array::from_fn(|i| a[i] + u * (b[i] - a[i]) + v * (c_[i] - a[i]))
        } else {
            let d = self.vertex(c + 1, r + 1);
            std::array::from_fn(|i| d[i] + (1. - u) * (c_[i] - d[i]) + (1. - v) * (b[i] - d[i]))
        }
    }
    /// The surface point at `(x, y)`, if a cell around it covers it.
    fn at(&self, x: f64, y: f64) -> Option<[f64; 3]> {
        let cells = |nodes: &[usize], v: f64| {
            let c = nodes
                .partition_point(|&n| n as f64 <= v)
                .clamp(1, nodes.len() - 1)
                - 1;
            c - usize::from(c > 0 && nodes[c] as f64 == v)..=c
        };
        cells(&self.ys, y)
            .flat_map(|r| cells(&self.xs, x).map(move |c| (c, r)))
            .find_map(|(c, r)| self.cell(c, r, x, y))
    }
    /// The value at `(x, y)` of each emitted triangle of cell `(c, r)` that
    /// holds it within rounding: a point computed on the diagonal belongs to
    /// both triangles, whichever side its arithmetic lands on.
    fn near_cell(&self, c: usize, r: usize, x: f64, y: f64, out: &mut Vec<[f64; 3]>) {
        const ROUNDING: f64 = 1e-9;
        let u = (x - self.xs[c] as f64) / (self.xs[c + 1] - self.xs[c]) as f64;
        let v = (y - self.ys[r] as f64) / (self.ys[r + 1] - self.ys[r]) as f64;
        if [u, v, 1. - u, 1. - v].iter().any(|t| *t < -ROUNDING) {
            return;
        }
        let (b, c_) = (self.vertex(c + 1, r), self.vertex(c, r + 1));
        if u + v <= 1. + ROUNDING && self.emitted(c, r, false) {
            let a = self.vertex(c, r);
            out.push(std::array::from_fn(|i| {
                a[i] + u * (b[i] - a[i]) + v * (c_[i] - a[i])
            }));
        }
        if u + v >= 1. - ROUNDING && self.emitted(c, r, true) {
            let d = self.vertex(c + 1, r + 1);
            out.push(std::array::from_fn(|i| {
                d[i] + (1. - u) * (c_[i] - d[i]) + (1. - v) * (b[i] - d[i])
            }));
        }
    }
    /// [`Self::near_cell`] over every cell whose closure holds `(x, y)` within rounding.
    fn near(&self, x: f64, y: f64, out: &mut Vec<[f64; 3]>) {
        let cells = |nodes: &[usize], v: f64| {
            let last = nodes.len() - 2;
            let lo = nodes
                .partition_point(|&n| (n as f64) < v - 1e-9)
                .saturating_sub(1);
            lo.min(last)
                ..=nodes
                    .partition_point(|&n| (n as f64) <= v + 1e-9)
                    .saturating_sub(1)
                    .min(last)
        };
        for r in cells(&self.ys, y) {
            for c in cells(&self.xs, x) {
                self.near_cell(c, r, x, y, out);
            }
        }
    }
    /// The longer horizontal diagonal of cell `(c, r)`.
    fn diagonal(&self, c: usize, r: usize) -> f64 {
        let flat = |a: [f64; 3], b: [f64; 3]| (a[0] - b[0]).hypot(a[1] - b[1]);
        flat(self.vertex(c, r), self.vertex(c + 1, r + 1))
            .max(flat(self.vertex(c + 1, r), self.vertex(c, r + 1)))
    }
}
#[cfg(feature = "native-geospatial")]
fn distance(a: [f64; 3], b: [f64; 3]) -> f64 {
    (0..3).map(|i| (a[i] - b[i]).powi(2)).sum::<f64>().sqrt()
}
/// The largest distance between a child-level surface point and the point with
/// the same lattice parameters on this tile's surface. Both surfaces are affine
/// on every piece of the overlay of their triangulations, so the maximum is at
/// overlay vertices. Every cell lies in one child because child boundaries are
/// multiples of this tile's stride, and holds at most 2x2 child cells. In a
/// full-stride cell this tile's diagonal is a union of child diagonals, so the
/// overlay vertices are the child vertices. In a clipped cell the diagonal also
/// crosses the interior child grid lines. It meets no child diagonal elsewhere:
/// the child cells at its ends share its endpoints, and the other two child
/// diagonals lie strictly on one side of it. Distances are taken where both
/// surfaces exist; a cell that is neither full nor empty on both levels may
/// cover differently and costs at least its horizontal diagonal.
#[cfg(feature = "native-geospatial")]
fn deviation(
    patch: &Patch,
    sheet: &Sheet,
    children: &[(usize, Sheet)],
    cells: usize,
    partial: &mut Vec<(usize, usize)>,
) -> f64 {
    let half = cells * patch.stride / 2;
    let mut worst: f64 = 0.;
    let mut points = Vec::with_capacity(16);
    for r in 0..sheet.ys.len() - 1 {
        for c in 0..sheet.xs.len() - 1 {
            let (xa, xb, ya, yb) = (sheet.xs[c], sheet.xs[c + 1], sheet.ys[r], sheet.ys[r + 1]);
            let slot = usize::from(xa >= patch.x + half) | usize::from(ya >= patch.y + half) << 1;
            let coarse = [false, true].map(|upper| sheet.emitted(c, r, upper));
            // A missing child has no surface.
            let Some((_, fine)) = children.iter().find(|(s, _)| *s == slot) else {
                if coarse.contains(&true) {
                    worst = worst.max(sheet.diagonal(c, r));
                    partial.push((c, r));
                }
                continue;
            };
            let within = |nodes: &[usize], a: usize, b: usize| {
                nodes.partition_point(|&n| n < a)..nodes.partition_point(|&n| n <= b)
            };
            let (rx, ry) = (within(&fine.xs, xa, xb), within(&fine.ys, ya, yb));
            let (fx, fy) = (&fine.xs[rx.clone()], &fine.ys[ry.clone()]);
            let (mut some, mut all) = (false, true);
            for j in ry.start..ry.end - 1 {
                for i in rx.start..rx.end - 1 {
                    for upper in [false, true] {
                        let e = fine.emitted(i, j, upper);
                        (some, all) = (some || e, all && e);
                    }
                }
            }
            let full = coarse == [true; 2] && all;
            if !full && (coarse.contains(&true) || some) {
                worst = worst.max(sheet.diagonal(c, r));
            }
            if !coarse.contains(&true) {
                continue;
            }
            // Only a child covering the whole cell carries its own bound to the
            // leaves here; elsewhere the caller measures the leaves directly.
            if !full {
                partial.push((c, r));
                continue;
            }
            // Child lattice nodes are its stored vertices: no search needed.
            for (j, &y) in fy.iter().enumerate() {
                for (i, &x) in fx.iter().enumerate() {
                    let here = sheet.plane(c, r, x as f64, y as f64);
                    worst = worst.max(distance(fine.vertex(rx.start + i, ry.start + j), here));
                }
            }
            points.clear();
            if xb - xa < patch.stride || yb - ya < patch.stride {
                let (w, h) = ((xb - xa) as f64, (yb - ya) as f64);
                // This tile's diagonal runs from (xb, ya) to (xa, yb).
                let (start, direction) = ((xb as f64, ya as f64), (-w, h));
                let on = |t: f64| (start.0 + t * direction.0, start.1 + t * direction.1);
                points.extend(fx[1..fx.len() - 1].iter().map(|&x| on((xb - x) as f64 / w)));
                points.extend(fy[1..fy.len() - 1].iter().map(|&y| on((y - ya) as f64 / h)));
            }
            for &(x, y) in &points {
                let fine = fine.at(x, y).expect("a full cell's child covers it");
                worst = worst.max(distance(fine, sheet.plane(c, r, x, y)));
            }
        }
    }
    worst
}
/// The fixed inputs shared by planning and production.
#[cfg(feature = "native-geospatial")]
struct Pyramid<'a> {
    raster: &'a raster::PreparedRaster,
    frame: &'a Frame,
    cells: usize,
    depth: u32,
    offset: f64,
    /// Largest quantized coordinate a tile may use ([`glb::QUANTIZED_STEPS`]
    /// outside tests, which coarsen the grid to expose quantization).
    quantization_steps: f64,
    attempt: &'a Attempt,
}
#[cfg(feature = "native-geospatial")]
impl<'a> Pyramid<'a> {
    fn new(
        raster: &'a raster::PreparedRaster,
        cells: u16,
        frame: &'a Frame,
        offset: f64,
        attempt: &'a Attempt,
    ) -> Self {
        let cells = usize::from(cells);
        let leaves = raster
            .width
            .div_ceil(cells)
            .max(raster.height.div_ceil(cells));
        Self {
            raster,
            frame,
            cells,
            depth: leaves.next_power_of_two().trailing_zeros(),
            offset,
            quantization_steps: glb::QUANTIZED_STEPS,
            attempt,
        }
    }
    /// Source cells per tile edge at `level`.
    fn span(&self, level: u32) -> usize {
        self.cells << (self.depth - level)
    }
    fn patch(&self, level: u32, column: usize, row: usize) -> Patch {
        let span = self.span(level);
        let (x, y) = (column * span, row * span);
        let (width, height) = (self.raster.width, self.raster.height);
        Patch {
            level,
            column,
            row,
            stride: 1 << (self.depth - level),
            x,
            y,
            x1: (x + span).min(width),
            y1: (y + span).min(height),
            content: false,
            children: vec![],
            error: 0.,
            drawn: 0.,
            skirt_depth: [0.; 4],
            surface: [[0.; 3]; 2],
            bounds: [[0.; 3]; 2],
            hierarchy: [[0.; 3]; 2],
        }
    }
    /// The neighbour-side child of the lowest common ancestor of a tile and its
    /// same-level neighbour across edge `k` (west, east, south, north). Every
    /// tile drawable beside it there lies in this one.
    fn across(
        &self,
        level: u32,
        column: usize,
        row: usize,
        k: usize,
    ) -> Option<(u32, usize, usize)> {
        let (dc, dr) = [(-1, 0), (1, 0), (0, -1), (0, 1)][k];
        let (c, r) = (column.checked_add_signed(dc)?, row.checked_add_signed(dr)?);
        let span = self.span(level);
        if c * span >= self.raster.width || r * span >= self.raster.height {
            return None;
        }
        let (mut a, mut b, mut level) = ((column, row), (c, r), level);
        while (a.0 / 2, a.1 / 2) != (b.0 / 2, b.1 / 2) {
            (a, b, level) = ((a.0 / 2, a.1 / 2), (b.0 / 2, b.1 / 2), level - 1);
        }
        Some((level, b.0, b.1))
    }
    /// Whether nodes `a` and `b` of the grid line `line` (x = line on sides 0
    /// and 1, y = line otherwise) bound a triangle west, east, south or north
    /// of it on the lattice of stride `s`.
    fn triangle(&self, side: usize, line: usize, (a, b): (usize, usize), s: usize) -> bool {
        let (w, h) = (self.raster.width, self.raster.height);
        let (third, ends) = match side {
            0 => ((line - s, b), [(line, a), (line, b)]),
            1 => (((line + s).min(w), a), [(line, a), (line, b)]),
            2 => ((b, line - s), [(a, line), (b, line)]),
            _ => ((a, (line + s).min(h)), [(a, line), (b, line)]),
        };
        (ends.into_iter().chain([third])).all(|(x, y)| self.raster.valid_node(x, y))
    }
    /// Leaf-decoded positions of the valid nodes of one grid line.
    fn line<'p>(
        &self,
        vertical: bool,
        fixed: usize,
        plan: &'p mut PreparedPlan,
    ) -> Result<&'p [Option<[f64; 3]>], JobError> {
        let storage = plan.storage[self.depth as usize];
        match plan.lines.entry((vertical, fixed)) {
            std::collections::hash_map::Entry::Occupied(line) => Ok(line.into_mut()),
            std::collections::hash_map::Entry::Vacant(line) => {
                self.attempt.check()?;
                let n = [self.raster.width, self.raster.height][usize::from(vertical)];
                let mut nodes = Vec::with_capacity(n + 1);
                for v in 0..=n {
                    let (x, y) = if vertical { (fixed, v) } else { (v, fixed) };
                    let (lon, lat) = coordinates(self.raster, x, y);
                    nodes.push(match surface_height(self.raster, lon, lat, self.offset)? {
                        Some(h) => Some(place(self.frame, storage, lon, lat, h)?.1),
                        None => None,
                    });
                }
                Ok(line.insert(nodes))
            }
        }
    }
    /// The largest distance from this tile's surface on its interior edges to
    /// the leaf surface at the valid leaf nodes there. Both sides of a seam are
    /// measured against the same nodes, so its gap is at most the sum of their
    /// errors, also where a coarse level bridges or loses a NoData hole.
    fn border(
        &self,
        patch: &Patch,
        sheet: &Sheet,
        plan: &mut PreparedPlan,
    ) -> Result<f64, JobError> {
        let (w, h) = (self.raster.width, self.raster.height);
        let (c1, r1) = (sheet.xs.len() - 1, sheet.ys.len() - 1);
        let mut worst: f64 = 0.;
        for (k, inner) in [patch.x > 0, patch.x1 < w, patch.y > 0, patch.y1 < h]
            .into_iter()
            .enumerate()
        {
            if !inner {
                continue;
            }
            let along = if k < 2 { &sheet.ys } else { &sheet.xs };
            let line = self.line(k < 2, [patch.x, patch.x1, patch.y, patch.y1][k], plan)?;
            for i in 0..along.len() - 1 {
                // The segment's own triangle: lower west and south, upper east and north.
                let (c, r) = [(0, i), (c1 - 1, i), (i, 0), (i, r1 - 1)][k];
                if !sheet.emitted(c, r, k % 2 == 1) {
                    continue;
                }
                let [a, b] = [i, i + 1].map(|j| {
                    let (c, r) = [(0, j), (c1, j), (j, 0), (j, r1)][k];
                    sheet.vertex(c, r)
                });
                let (ta, tb) = (along[i], along[i + 1]);
                for (v, leaf) in line[ta..=tb].iter().enumerate() {
                    let t = v as f64 / (tb - ta) as f64;
                    let here = std::array::from_fn(|q| a[q] + t * (b[q] - a[q]));
                    worst = worst.max(leaf.map_or(0., |leaf| distance(here, leaf)));
                }
            }
        }
        Ok(worst)
    }
    /// The leaf surface over `[x0, x1] x [y0, y1]` as the leaves store it.
    fn leaves(&self, x: [usize; 2], y: [usize; 2], storage: Storage) -> Result<Sheet, JobError> {
        let (xs, ys): (Vec<_>, Vec<_>) = ((x[0]..=x[1]).collect(), (y[0]..=y[1]).collect());
        let (mut positions, mut used) = (vec![], vec![]);
        for &y in &ys {
            for &x in &xs {
                let (lon, lat) = coordinates(self.raster, x, y);
                let height = surface_height(self.raster, lon, lat, self.offset)?;
                // A triangle of three valid nodes is a leaf triangle.
                used.push(height.is_some());
                positions.push(match height {
                    Some(h) => place(self.frame, storage, lon, lat, h)?.0,
                    None => [0.; 3],
                });
            }
        }
        Ok(Sheet {
            xs,
            ys,
            positions,
            used,
        })
    }
    /// The largest distance from this tile's surface to the leaf surface in
    /// `cells`, where a child does not cover the whole cell, so its error does
    /// not bound the leaves there. Both surfaces are affine on the overlay of
    /// this cell's triangles with the leaf cells, so the maximum is at leaf
    /// nodes and at the crossings of this cell's diagonal with leaf edges.
    fn direct(
        &self,
        sheet: &Sheet,
        cells: &[(usize, usize)],
        storage: Storage,
    ) -> Result<f64, JobError> {
        let mut worst: f64 = 0.;
        for &(c, r) in cells {
            self.attempt.check()?;
            let (xa, xb, ya, yb) = (sheet.xs[c], sheet.xs[c + 1], sheet.ys[r], sheet.ys[r + 1]);
            let leaf = self.leaves([xa, xb], [ya, yb], storage)?;
            // A valid leaf node is on the leaf surface whichever side covers it.
            // Points are evaluated on every emitted triangle within rounding.
            let mut here = vec![];
            for (j, y) in (ya..=yb).enumerate() {
                for (i, x) in (xa..=xb).enumerate() {
                    if leaf.used[j * leaf.xs.len() + i] {
                        here.clear();
                        match sheet.cell(c, r, x as f64, y as f64) {
                            Some(a) => here.push(a),
                            None => sheet.near_cell(c, r, x as f64, y as f64, &mut here),
                        }
                        for a in &here {
                            worst = worst.max(distance(*a, leaf.vertex(i, j)));
                        }
                    }
                }
            }
            // The diagonal runs from (xb, ya) to (xa, yb); in a square cell it
            // follows leaf diagonals through leaf nodes.
            let (w, h) = (xb - xa, yb - ya);
            let mut below = vec![];
            for n in [w, h, w.abs_diff(h)].into_iter().filter(|_| w != h) {
                for k in 0..=n {
                    let t = k as f64 / n as f64;
                    let (x, y) = (xb as f64 - t * w as f64, ya as f64 + t * h as f64);
                    here.clear();
                    below.clear();
                    sheet.near_cell(c, r, x, y, &mut here);
                    leaf.near(x, y, &mut below);
                    for (a, b) in here.iter().flat_map(|a| below.iter().map(move |b| (a, b))) {
                        worst = worst.max(distance(*a, *b));
                    }
                }
            }
        }
        Ok(worst)
    }
    fn grid(&self, patch: &Patch, storage: Storage) -> Result<glb::Grid, JobError> {
        grid(self, &patch.columns(), &patch.rows(), storage)
    }
    /// Plan one tile after its children (post-order), keeping only their
    /// surfaces while this tile's error is measured. Errors are measured on
    /// the decoded positions every level stores. A tile with no surface at or
    /// below it is not planned.
    fn visit(
        &self,
        level: u32,
        column: usize,
        row: usize,
        plan: &mut PreparedPlan,
    ) -> Result<Option<(usize, Sheet)>, JobError> {
        let mut patch = self.patch(level, column, row);
        let mut children = Vec::with_capacity(4);
        if level < self.depth {
            let span = self.span(level + 1);
            for slot in 0..4 {
                let (c, r) = (2 * column + (slot & 1), 2 * row + (slot >> 1));
                if c * span < self.raster.width && r * span < self.raster.height {
                    if let Some((id, sheet)) = self.visit(level + 1, c, r, plan)? {
                        patch.children.push(id);
                        children.push((slot, sheet));
                    }
                }
            }
        }
        self.attempt.check()?;
        let g = self.grid(&patch, plan.storage[level as usize])?;
        patch.content = g.used.contains(&true);
        if !patch.content && children.is_empty() {
            return Ok(None);
        }
        plan.vertices += g.used.iter().filter(|u| **u).count() as u64;
        if level == self.depth {
            // The full-detail leaves are the reference surface.
            plan.position_error = plan.position_error.max(g.max_position_error);
        }
        plan.height_range[0] = plan.height_range[0].min(g.height_range[0]);
        plan.height_range[1] = plan.height_range[1].max(g.height_range[1]);
        patch.surface = g.bounds;
        // The surfaces at and below this tile, until skirts are planned.
        patch.hierarchy = g.bounds;
        for &child in &patch.children {
            for corner in plan.patches[child].hierarchy {
                extend(&mut patch.hierarchy, corner);
            }
        }
        let sheet = Sheet {
            xs: patch.columns(),
            ys: patch.rows(),
            positions: g.positions,
            used: g.used,
        };
        if !children.is_empty() {
            // e(t) = d(t) + max e(child) bounds the distance to the leaf mesh;
            // tiles without triangles pass on their descendants' errors.
            let below = (patch.children.iter())
                .map(|&c| plan.patches[c].drawn)
                .fold(0., f64::max);
            let mut partial = vec![];
            let mut sum = deviation(&patch, &sheet, &children, self.cells, &mut partial) + below;
            if patch.content {
                sum = sum.max(self.border(&patch, &sheet, plan)?);
                sum = sum.max(self.direct(&sheet, &partial, plan.storage[self.depth as usize])?);
            }
            patch.error = if sum > 0. { outward(sum, &sheet) } else { 0. };
            patch.drawn = if patch.content { patch.error } else { below };
        }
        plan.patches.push(patch);
        Ok(Some((plan.patches.len() - 1, sheet)))
    }
    /// Curtains from each skirted edge down to `height - depth`, wound to face
    /// away from the tile under its single-sided material. The tops are the
    /// tile's own surface vertices. A segment is skirted where this tile and
    /// some level drawable across it have a triangle on it: NoData edges stay
    /// open.
    fn skirt(
        &self,
        patch: &Patch,
        storage: Storage,
        depths: [f64; 4],
    ) -> Result<(glb::Skirt, [[f64; 3]; 2], f64), JobError> {
        let (xs, ys) = (patch.columns(), patch.rows());
        let (columns, rows) = (xs.len(), ys.len());
        let edges: [Vec<(usize, usize)>; 4] = [
            (0..rows).map(|r| (0, r)).collect(),
            (0..rows).rev().map(|r| (columns - 1, r)).collect(),
            (0..columns).rev().map(|c| (c, 0)).collect(),
            (0..columns).map(|c| (c, rows - 1)).collect(),
        ];
        let lines = [patch.x, patch.x1, patch.y, patch.y1];
        let ends = [self.raster.height, self.raster.width];
        let open = |k: usize, top: u32, (lo, hi): (usize, usize)| {
            self.triangle(k ^ 1, lines[k], (lo, hi), patch.stride)
                && (top..=self.depth).any(|level| {
                    let s = 1 << (self.depth - level);
                    let mut a = lo / s * s;
                    while a < hi {
                        let b = (a + s).min(ends[k / 2]);
                        if self.triangle(k, lines[k], (a, b), s) {
                            return true;
                        }
                        a = b;
                    }
                    false
                })
        };
        let mut skirt = glb::Skirt {
            bottoms: vec![],
            indices: vec![],
            edges: [false; 4],
        };
        let surface = (columns * rows) as u32;
        let mut bounds = patch.surface;
        let mut worst: f64 = 0.;
        for (k, nodes) in edges.iter().enumerate().filter(|(k, _)| depths[*k] > 0.) {
            self.attempt.check()?;
            let Some((top, _, _)) = self.across(patch.level, patch.column, patch.row, k) else {
                continue;
            };
            let along: Vec<_> = (nodes.iter())
                .map(|&(c, r)| [ys[r], xs[c]][k / 2])
                .collect();
            let open: Vec<_> = (along.windows(2))
                .map(|p| open(k, top, (p[0].min(p[1]), p[0].max(p[1]))))
                .collect();
            let mut previous = None;
            for (j, &(c, r)) in nodes.iter().enumerate() {
                let after = j > 0 && open[j - 1];
                if !after && !open.get(j).copied().unwrap_or(false) {
                    continue;
                }
                skirt.edges[k] = true;
                let (lon, lat) = coordinates(self.raster, xs[c], ys[r]);
                let height = surface_height(self.raster, lon, lat, self.offset)?
                    .expect("skirted nodes are valid");
                let (bottom, position, error) =
                    place(self.frame, storage, lon, lat, height - depths[k])?;
                extend(&mut bounds, position);
                worst = worst.max(error);
                let (top, base) = (
                    (r * columns + c) as u32,
                    surface + skirt.bottoms.len() as u32,
                );
                skirt.bottoms.push(bottom);
                if let (true, Some((t, b))) = (after, previous) {
                    skirt.indices.extend([t, top, b, top, base, b]);
                }
                previous = Some((top, base));
            }
        }
        Ok((skirt, bounds, worst))
    }
}
/// `value` rounded up past the floating-point error of a deviation evaluated
/// on `sheet`'s coordinates: a few units in the last place of the largest one.
#[cfg(feature = "native-geospatial")]
fn outward(value: f64, sheet: &Sheet) -> f64 {
    let magnitude = sheet
        .positions
        .iter()
        .flatten()
        .fold(1f64, |m, v| m.max(v.abs()));
    (value + 64. * f64::EPSILON * (magnitude + value)).next_up()
}
#[cfg(feature = "native-geospatial")]
struct PreparedPlan {
    /// Post-order: children precede parents and the root is last.
    patches: Vec<Patch>,
    /// Leaf-decoded nodes of grid lines that coarse tile edges lie on.
    lines: std::collections::HashMap<(bool, usize), Vec<Option<[f64; 3]>>>,
    /// Position storage per level, coarsest first.
    storage: Vec<Storage>,
    vertices: u64,
    skirt_vertices: u64,
    position_error: f64,
    height_range: [f64; 2],
}
#[cfg(feature = "native-geospatial")]
impl PreparedPlan {
    fn root(&self) -> &Patch {
        self.patches.last().expect("a plan has a root")
    }
    fn storage(&self, patch: &Patch) -> Storage {
        self.storage[patch.level as usize]
    }
}
/// Largest admitted storage error of the full-detail reference surface.
#[cfg(feature = "native-geospatial")]
const POSITION_LIMIT: f64 = 0.05;
/// The glTF (y-up) range of decoded frame `bounds`.
#[cfg(feature = "native-geospatial")]
fn stored_range(bounds: [[f64; 3]; 2]) -> [[f64; 3]; 2] {
    [
        [bounds[0][0], bounds[0][2], -bounds[1][1]],
        [bounds[1][0], bounds[1][2], -bounds[0][1]],
    ]
}
/// The smallest power of two at least `value` (positive and finite).
#[cfg(feature = "native-geospatial")]
fn power_of_two(value: f64) -> f64 {
    let mut step = 2f64.powi(value.log2().ceil() as i32);
    while step < value {
        step *= 2.
    }
    while step / 2. >= value {
        step /= 2.
    }
    step
}
/// Per level and glTF axis: the largest span and magnitude of a tile.
#[cfg(feature = "native-geospatial")]
type Extents = (Vec<[f64; 3]>, Vec<[f64; 3]>);
/// The [`Extents`] of every level's unrounded surfaces. A coarse tile's nodes
/// are nodes of the leaves below it, so the union of their ranges bounds its
/// own: only leaves are sampled.
#[cfg(feature = "native-geospatial")]
fn surface_extents(pyramid: &Pyramid) -> Result<Extents, JobError> {
    let (depth, cells) = (pyramid.depth, pyramid.cells);
    let (width, height) = (pyramid.raster.width, pyramid.raster.height);
    let empty = [[f64::INFINITY; 3], [f64::NEG_INFINITY; 3]];
    let mut ranges: Vec<Vec<[[f64; 3]; 2]>> = (0..=depth)
        .map(|level| {
            let span = pyramid.span(level);
            vec![empty; width.div_ceil(span) * height.div_ceil(span)]
        })
        .collect();
    for row in 0..height.div_ceil(cells) {
        for column in 0..width.div_ceil(cells) {
            let patch = pyramid.patch(depth, column, row);
            let range = stored_range(pyramid.grid(&patch, Storage::Exact)?.bounds);
            for (level, tiles) in ranges.iter_mut().enumerate() {
                let shift = depth as usize - level;
                let columns = width.div_ceil(pyramid.span(level as u32));
                let tile = &mut tiles[(row >> shift) * columns + (column >> shift)];
                for axis in 0..3 {
                    tile[0][axis] = tile[0][axis].min(range[0][axis]);
                    tile[1][axis] = tile[1][axis].max(range[1][axis]);
                }
            }
        }
    }
    let mut extent = vec![[0f64; 3]; ranges.len()];
    let mut magnitude = vec![[0f64; 3]; ranges.len()];
    for (level, tiles) in ranges.iter().enumerate() {
        for [low, high] in tiles.iter().filter(|t| t[0][0] <= t[1][0]) {
            for axis in 0..3 {
                extent[level][axis] = extent[level][axis].max(high[axis] - low[axis]);
                magnitude[level][axis] = magnitude[level][axis]
                    .max(low[axis].abs())
                    .max(high[axis].abs());
            }
        }
    }
    Ok((extent, magnitude))
}
/// Leaves are the reference surface: they keep Float32 when a quantized grid
/// could move a vertex more than [`POSITION_LIMIT`].
#[cfg(feature = "native-geospatial")]
fn leaf_storage(storage: Storage) -> Storage {
    if storage.margin(0.) / 2. > POSITION_LIMIT {
        Storage::Float
    } else {
        storage
    }
}
/// Per level, the finest power-of-two grid, one step for every axis, on which
/// each tile's surface spans at most `steps` steps. One step for all three
/// axes keeps vertical precision equal to horizontal, and the height stream as
/// compressible as the others. Coarse levels always quantize: their grid
/// error is part of their measured error.
#[cfg(feature = "native-geospatial")]
fn storage_for((extent, magnitude): &Extents, steps: f64) -> Vec<Storage> {
    let depth = extent.len() - 1;
    (0..=depth)
        .map(|level| {
            // Rounding each end can add one step; leave headroom for skirts.
            let step = (0..3)
                .map(|axis| {
                    (extent[level][axis] * (1. + 1. / 1024.) / (steps - 1.))
                        .max(magnitude[level][axis] * 2f64.powi(-40))
                })
                .fold(2f64.powi(-40), f64::max);
            let storage = Storage::Quantized([power_of_two(step); 3]);
            if level == depth {
                leaf_storage(storage)
            } else {
                storage
            }
        })
        .collect()
}
/// A coarser grid for each level where some tile spans more than `steps`
/// steps, or `None` when every tile fits.
#[cfg(feature = "native-geospatial")]
fn refit(plan: &PreparedPlan, depth: u32, steps: f64) -> Option<Vec<Storage>> {
    let mut storage = plan.storage.clone();
    for patch in &plan.patches {
        let Storage::Quantized(step) = &mut storage[patch.level as usize] else {
            continue;
        };
        let [low, high] = stored_range(patch.bounds);
        for axis in 0..3 {
            let span = high[axis] - low[axis];
            if span / step[axis] > steps {
                let coarser = power_of_two(span / (steps - 1.)).max(2. * step[axis]);
                *step = [coarser; 3];
            }
        }
    }
    if let Some(leaves) = storage.get_mut(depth as usize) {
        *leaves = leaf_storage(*leaves);
    }
    (storage != plan.storage).then_some(storage)
}
#[cfg(feature = "native-geospatial")]
fn plan(pyramid: &Pyramid) -> Result<PreparedPlan, JobError> {
    let (width, height, cells) = (pyramid.raster.width, pyramid.raster.height, pyramid.cells);
    let leaves = width.div_ceil(cells) * height.div_ceil(cells);
    let tiles: usize = (0..=pyramid.depth)
        .map(|level| {
            let span = pyramid.span(level);
            width.div_ceil(span) * height.div_ceil(span)
        })
        .sum();
    // Uncompressed positions, indices, skirts and conservative JSON overhead per tile.
    let side = cells as u64 + 1;
    let estimate = tiles as u64 * (32 * side.pow(2) + 128 * side + 8192);
    if leaves > 10_000 || estimate > 1 << 30 {
        return Err(error(
            JobErrorKind::Unsupported,
            "terrain plan exceeds 10000 leaves or 1 GiB conservative output budget",
        ));
    }
    // Unrounded leaf surfaces size each level's grid; the stored plan is then
    // measured on decoded positions, skirts included, and coarsened until
    // every tile fits.
    let mut storage = storage_for(&surface_extents(pyramid)?, pyramid.quantization_steps);
    for _ in 0..64 {
        let plan = plan_with(pyramid, storage, tiles)?;
        match refit(&plan, pyramid.depth, pyramid.quantization_steps) {
            Some(coarser) => storage = coarser,
            None => {
                if plan.position_error > POSITION_LIMIT {
                    return Err(error(
                        JobErrorKind::Unsupported,
                        "terrain Float32 position error exceeds 0.05 metres",
                    ));
                }
                return Ok(plan);
            }
        }
    }
    Err(error(
        JobErrorKind::InvalidState,
        "terrain quantization grids did not converge",
    ))
}
/// One complete plan with fixed per-level storage.
#[cfg(feature = "native-geospatial")]
fn plan_with(
    pyramid: &Pyramid,
    storage: Vec<Storage>,
    tiles: usize,
) -> Result<PreparedPlan, JobError> {
    let mut plan = PreparedPlan {
        patches: Vec::with_capacity(tiles),
        lines: Default::default(),
        storage,
        vertices: 0,
        skirt_vertices: 0,
        position_error: 0.,
        height_range: [f64::INFINITY, f64::NEG_INFINITY],
    };
    if pyramid.visit(0, 0, 0, &mut plan)?.is_none() {
        return Err(error(
            JobErrorKind::InvalidInput,
            "terrain source has no triangle of four valid samples",
        ));
    }
    // Across an edge, a tile renders beside a drawing descendant of the
    // neighbour-side child of their lowest common ancestor. Both lie within
    // their errors of the leaf nodes on the edge, so the gap is at most e(t)
    // plus that child's drawn error. Skirts also drop by their storage margin,
    // so decoded bottoms are that far below the surface.
    let ids: std::collections::HashMap<_, _> = (plan.patches.iter().enumerate())
        .map(|(id, p)| ((p.level, p.column, p.row), id))
        .collect();
    for id in 0..plan.patches.len() {
        let patch = &plan.patches[id];
        let storage = plan.storage(patch);
        let magnitude = (patch.surface.iter().flatten()).fold(0f64, |m, v| m.max(v.abs()));
        let mut depths = [0.; 4];
        for k in (0..4).filter(|_| patch.content) {
            let across = pyramid.across(patch.level, patch.column, patch.row, k);
            let Some(&beside) = across.and_then(|n| ids.get(&n)) else {
                continue;
            };
            let mut depth = (patch.error + plan.patches[beside].drawn).max(1.);
            let margin = storage.margin(magnitude + depth + 1.);
            if margin > 0. {
                depth = (depth + margin).next_up();
            }
            depths[k] = depth;
        }
        let (skirt, bounds, error) = pyramid.skirt(patch, storage, depths)?;
        let depths: [f64; 4] =
            std::array::from_fn(|k| [0., depths[k]][usize::from(skirt.edges[k])]);
        plan.skirt_vertices += skirt.bottoms.len() as u64;
        if plan.patches[id].level == pyramid.depth {
            plan.position_error = plan.position_error.max(error);
        }
        let mut hierarchy = bounds;
        for &child in &plan.patches[id].children {
            let b = box_for(plan.patches[child].hierarchy);
            extend(&mut hierarchy, std::array::from_fn(|i| b[i] - b[3 + i * 4]));
            extend(&mut hierarchy, std::array::from_fn(|i| b[i] + b[3 + i * 4]));
        }
        let patch = &mut plan.patches[id];
        patch.skirt_depth = depths;
        patch.bounds = bounds;
        patch.hierarchy = hierarchy;
        if !patch.content {
            // As meshes' `EMPTY_PARENT_SCALE`: Cesium stops at an empty tile
            // whose error passes, so it refines until its published box is
            // about a pixel. Ancestors, skirts and boxes do not inherit this.
            let b = box_for(hierarchy);
            let diagonal = 2. * (b[3].powi(2) + b[7].powi(2) + b[11].powi(2)).sqrt();
            patch.error = patch.error.max(16. * diagonal);
        }
    }
    Ok(plan)
}
#[cfg(feature = "native-geospatial")]
fn plan_report(
    raster: &raster::PreparedRaster,
    cells: u16,
    plan: &PreparedPlan,
    offset: f64,
) -> Result<TerrainReport, JobError> {
    let root = plan.root();
    let bounds = root.hierarchy;
    let diameter = ((0..3)
        .map(|i| (bounds[1][i] - bounds[0][i]).powi(2))
        .sum::<f64>())
    .sqrt()
    .next_up();
    if !diameter.is_finite() || diameter <= 0. {
        return Err(error(
            JobErrorKind::Unsupported,
            "terrain extent must have a positive finite diameter",
        ));
    }
    let levels = plan.patches.iter().map(|p| p.level).max().unwrap_or(0) + 1;
    let mut tiles_per_level = vec![0; levels as usize];
    let mut errors = vec![0f64; levels as usize];
    let mut skirts: Option<[f64; 2]> = None;
    for patch in &plan.patches {
        tiles_per_level[patch.level as usize] += 1;
        errors[patch.level as usize] = errors[patch.level as usize].max(patch.error);
        for &depth in patch.skirt_depth.iter().filter(|d| **d > 0.) {
            let [low, high] = skirts.unwrap_or([f64::INFINITY, f64::NEG_INFINITY]);
            skirts = Some([low.min(depth), high.max(depth)]);
        }
    }
    Ok(TerrainReport {
        schema_version: 3,
        profile: "t1-terrain-lod-mesh-3dtiles-1.1",
        source_crs: raster.source_crs.clone(),
        source_bytes: raster.source_bytes,
        width: raster.width,
        height: raster.height,
        bounds: raster.bounds,
        source_pixel_degrees: raster.pixel,
        cells_per_leaf: cells,
        levels,
        tiles: plan.patches.len() as u64,
        tiles_per_level,
        vertices: plan.vertices,
        skirt_vertices: plan.skirt_vertices,
        height_offset_metres: offset,
        position_error_metres: plan.position_error,
        height_range: plan.height_range,
        generated_bytes: 0,
        surface_reference:
            "decoded full-detail triangle mesh; no certified bound on the continuous source surface",
        quantization_step_metres: plan
            .storage
            .iter()
            .map(|storage| match storage {
                Storage::Quantized(step) => Some(*step),
                Storage::Exact | Storage::Float => None,
            })
            .collect(),
        geometric_error_metres: errors,
        root_geometric_error_metres: root.error,
        tileset_geometric_error_metres: crate::tileset_node::top_level_error(
            diameter, root.error, 2.,
        ),
        skirt_depth_metres: skirts,
    })
}
pub fn terrain_to_directory(
    request: TerrainRequest,
    control: &RunControl,
) -> Result<TerrainResult, JobFailure> {
    let attempt = control.begin()?;
    attempt.check().map_err(|e| attempt.fail(e))?;
    let TerrainHeights::RawMetres {
        height_offset_metres: offset,
    } = request.heights;
    if request.input.as_os_str().is_empty()
        || request.output.as_os_str().is_empty()
        || ![16, 32, 64, 128].contains(&request.options.cells_per_leaf)
        || !offset.is_finite()
    {
        return Err(attempt.fail(error(
            JobErrorKind::InvalidRequest,
            "T1 requires paths, cells_per_leaf 16/32/64/128 and finite metre heights",
        )));
    }
    #[cfg(not(feature = "native-geospatial"))]
    {
        Err(attempt.fail(error(
            JobErrorKind::Unsupported,
            "T1 terrain requires native-geospatial GDAL capability",
        )))
    }
    #[cfg(feature = "native-geospatial")]
    {
        let prepare = || -> Result<_, JobError> {
            let target = DirectoryTarget::prepare(&request.output, request.policy)?;
            let m = fs::symlink_metadata(&request.input)
                .map_err(|e| JobError::io("inspect terrain source", &request.input, e))?;
            if !m.is_file() {
                return Err(error(
                    JobErrorKind::InvalidInput,
                    "terrain source must be regular file without symlink leaf",
                ));
            }
            let source = fs::canonicalize(&request.input)
                .map_err(|e| JobError::io("resolve terrain source", &request.input, e))?;
            overlap(&source, target.output())?;
            Ok((source, target))
        };
        let (source, target) = prepare().map_err(|e| attempt.fail(e))?;
        let raster = raster::prepare(&source, &attempt)?;
        let frame = Frame::new(raster.bounds);
        let cells = request.options.cells_per_leaf;
        let pyramid = Pyramid::new(&raster, cells, &frame, offset, &attempt);
        let planned =
            plan(&pyramid).and_then(|p| Ok((plan_report(&raster, cells, &p, offset)?, p)));
        let (mut report, plan) = planned.map_err(|e| attempt.fail(e))?;
        let staging = target.stage(&attempt)?;
        if let Err(e) = produce(staging.path(), &pyramid, &plan, &mut report, write, seal) {
            return Err(staging.fail(e));
        }
        let published = staging.seal()?.publish()?;
        Ok(TerrainResult {
            output: published.output,
            report,
            cleanup_diagnostics: published.cleanup_diagnostics,
        })
    }
}
#[cfg(feature = "native-geospatial")]
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct BoundingVolume {
    r#box: [f64; 12],
}
#[cfg(feature = "native-geospatial")]
#[derive(Serialize)]
struct Content {
    uri: String,
}
#[cfg(feature = "native-geospatial")]
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Extras {
    implicit_child_index: usize,
}
#[cfg(feature = "native-geospatial")]
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Tile {
    bounding_volume: BoundingVolume,
    geometric_error: f64,
    #[serde(skip_serializing_if = "Option::is_none")]
    refine: Option<&'static str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    transform: Option<[f64; 16]>,
    #[serde(skip_serializing_if = "Option::is_none")]
    content: Option<Content>,
    #[serde(skip_serializing_if = "Option::is_none")]
    extras: Option<Extras>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    children: Vec<Tile>,
}
#[cfg(feature = "native-geospatial")]
#[derive(Serialize)]
struct Asset {
    version: &'static str,
}
#[cfg(feature = "native-geospatial")]
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Tileset {
    asset: Asset,
    geometric_error: f64,
    root: Tile,
}
/// The explicit hierarchy H: tiles with content name it, children state their
/// quadrant slot, and only the root carries the frame and `REPLACE`.
#[cfg(feature = "native-geospatial")]
fn hierarchy(plan: &PreparedPlan, frame: &Frame, geometric_error: f64) -> serde_json::Value {
    fn tile(plan: &PreparedPlan, id: usize, frame: Option<&Frame>) -> Tile {
        let patch = &plan.patches[id];
        Tile {
            bounding_volume: BoundingVolume {
                r#box: box_for(patch.hierarchy),
            },
            geometric_error: patch.error,
            refine: frame.map(|_| "REPLACE"),
            transform: frame.map(Frame::transform),
            content: patch.content.then(|| Content { uri: patch.name() }),
            extras: frame.is_none().then(|| Extras {
                implicit_child_index: patch.slot(),
            }),
            children: patch
                .children
                .iter()
                .map(|&child| tile(plan, child, None))
                .collect(),
        }
    }
    serde_json::to_value(Tileset {
        asset: Asset { version: "1.1" },
        geometric_error,
        root: tile(plan, plan.patches.len() - 1, Some(frame)),
    })
    .expect("terrain hierarchy serializes")
}
/// Encode and write every tile payload under its explicit-hierarchy name.
#[cfg(feature = "native-geospatial")]
fn payloads(
    path: &Path,
    pyramid: &Pyramid,
    plan: &PreparedPlan,
    write: &mut impl FnMut(&Path, &[u8], &mut u64) -> Result<(), JobError>,
) -> Result<(), JobError> {
    let attempt = pyramid.attempt;
    let mut bytes = 0;
    for (i, patch) in plan.patches.iter().enumerate() {
        attempt.check()?;
        if !patch.content {
            continue;
        }
        let storage = plan.storage(patch);
        let g = pyramid.grid(patch, storage)?;
        let (skirt, bounds, _) = pyramid.skirt(patch, storage, patch.skirt_depth)?;
        if g.bounds != patch.surface || bounds != patch.bounds {
            return Err(error(
                JobErrorKind::InvalidInput,
                "terrain plan changed during encoding",
            ));
        }
        let encoded = glb::encode(&g, &skirt, storage, || attempt.check())?;
        let destination = path.join(patch.name());
        fs::create_dir_all(destination.parent().unwrap())
            .map_err(|e| JobError::io("create terrain tile parent", &destination, e))?;
        attempt.check()?;
        write(&destination, &encoded, &mut bytes)?;
        attempt.emit(&RunEvent::Progress {
            phase: "terrain",
            done: i as u64 + 1,
            total: Some(plan.patches.len() as u64),
        })?;
    }
    Ok(())
}
/// Publish H as the bounded implicit forest. A cancellation or observer stop
/// raised inside the writer keeps its own typed error.
#[cfg(feature = "native-geospatial")]
fn forest(
    manifest: &mut serde_json::Value,
    path: &Path,
    attempt: &Attempt,
) -> Result<crate::implicit::ForestOutput, JobError> {
    let mut stopped = None;
    let written = crate::implicit::write_forest(
        manifest,
        path,
        crate::implicit::SubdivisionScheme::Quadtree,
        &mut || {
            attempt.check().map_err(|e| {
                let message = e.message().to_owned();
                stopped = Some(e);
                crate::Error::Message(message)
            })
        },
    );
    written.map_err(|e| {
        stopped.take().unwrap_or_else(|| match e {
            crate::Error::Io(e) => JobError::io("write terrain implicit forest", path, e),
            e => error(
                JobErrorKind::InvalidState,
                format!("terrain implicit forest: {e}"),
            ),
        })
    })
}
/// Remove directories the forest emptied when it relocated implicit payloads.
#[cfg(feature = "native-geospatial")]
fn prune(directory: &Path) -> Result<(), JobError> {
    let entries =
        |d: &Path| fs::read_dir(d).map_err(|e| JobError::io("inspect terrain staging", d, e));
    for entry in entries(directory)? {
        let entry = entry.map_err(|e| JobError::io("read terrain staging", directory, e))?;
        let kind = entry
            .file_type()
            .map_err(|e| JobError::io("inspect terrain staging", &entry.path(), e))?;
        if kind.is_dir() {
            prune(&entry.path())?;
        }
    }
    if entries(directory)?.next().is_none() {
        fs::remove_dir(directory)
            .map_err(|e| JobError::io("remove empty terrain directory", directory, e))?;
    }
    Ok(())
}
#[cfg(feature = "native-geospatial")]
fn produce(
    path: &Path,
    pyramid: &Pyramid,
    plan: &PreparedPlan,
    report: &mut TerrainReport,
    mut write: impl FnMut(&Path, &[u8], &mut u64) -> Result<(), JobError>,
    mut seal: impl FnMut(&Path) -> Result<(), JobError>,
) -> Result<(), JobError> {
    let attempt = pyramid.attempt;
    payloads(path, pyramid, plan, &mut write)?;
    let mut manifest = hierarchy(plan, pyramid.frame, report.tileset_geometric_error_metres);
    let forest = forest(&mut manifest, path, attempt)?;
    prune(&path.join("tiles"))?;
    // Count and sync every member the forest kept or wrote, once.
    let mut bytes = 0;
    let mut inventory = std::collections::BTreeSet::new();
    for member in &forest.members {
        attempt.check()?;
        let member_path = path.join(member);
        let length = fs::metadata(&member_path)
            .map_err(|e| JobError::io("inspect terrain member", &member_path, e))?
            .len();
        count(&mut bytes, length)?;
        seal(&member_path)?;
        inventory.insert(member.clone());
    }
    attempt.check()?;
    write(
        &path.join("tileset.json"),
        &serde_json::to_vec(&manifest).unwrap(),
        &mut bytes,
    )?;
    seal(&path.join("tileset.json"))?;
    inventory.insert("tileset.json".into());
    report.generated_bytes = bytes;
    loop {
        let length = serde_json::to_vec_pretty(report).unwrap().len() as u64;
        let total = bytes + length;
        if total == report.generated_bytes {
            break;
        }
        report.generated_bytes = total;
    }
    attempt.check()?;
    write(
        &path.join("conversion.json"),
        &serde_json::to_vec_pretty(report).unwrap(),
        &mut bytes,
    )?;
    seal(&path.join("conversion.json"))?;
    inventory.insert("conversion.json".into());
    receipt(path, &inventory, bytes, report.generated_bytes, attempt)?;
    attempt.emit(&RunEvent::Note {
        message: "terrain mesh directory ready for publication",
    })?;
    attempt.close_events()
}
#[cfg(feature = "native-geospatial")]
fn receipt(
    path: &Path,
    inventory: &std::collections::BTreeSet<String>,
    bytes: u64,
    reported: u64,
    attempt: &Attempt,
) -> Result<(), JobError> {
    fn inspect(
        root: &Path,
        here: &Path,
        found: &mut std::collections::BTreeSet<String>,
        dirs: &mut std::collections::BTreeSet<String>,
        length: &mut u64,
        attempt: &Attempt,
    ) -> Result<(), JobError> {
        for entry in
            fs::read_dir(here).map_err(|e| JobError::io("inspect terrain receipt", here, e))?
        {
            attempt.check()?;
            let entry = entry.map_err(|e| JobError::io("read terrain receipt", here, e))?;
            let p = entry.path();
            let m = fs::symlink_metadata(&p)
                .map_err(|e| JobError::io("inspect terrain member", &p, e))?;
            let relative = p
                .strip_prefix(root)
                .unwrap()
                .components()
                .map(|component| component.as_os_str().to_str().unwrap())
                .collect::<Vec<_>>()
                .join("/");
            if m.is_dir() {
                dirs.insert(relative);
                inspect(root, &p, found, dirs, length, attempt)?
            } else if m.is_file() {
                found.insert(relative);
                *length += m.len();
            } else {
                return Err(error(
                    JobErrorKind::InvalidInput,
                    "unexpected terrain receipt entry",
                ));
            }
        }
        Ok(())
    }
    let mut expected_dirs = std::collections::BTreeSet::new();
    for file in inventory {
        for parent in Path::new(file)
            .ancestors()
            .skip(1)
            .filter(|p| !p.as_os_str().is_empty())
        {
            expected_dirs.insert(
                parent
                    .components()
                    .map(|component| component.as_os_str().to_str().unwrap())
                    .collect::<Vec<_>>()
                    .join("/"),
            );
        }
    }
    let mut found = std::collections::BTreeSet::new();
    let mut dirs = std::collections::BTreeSet::new();
    let mut length = 0;
    inspect(path, path, &mut found, &mut dirs, &mut length, attempt)?;
    if &found != inventory || dirs != expected_dirs || length != bytes || bytes != reported {
        return Err(error(
            JobErrorKind::InvalidInput,
            "terrain receipt mismatch",
        ));
    }
    Ok(())
}
#[cfg(all(test, feature = "native-geospatial"))]
mod tests {
    use super::*;
    use std::collections::{BTreeMap, BTreeSet};
    /// A smooth relief with a slope, so every coarse level has a nonzero error.
    fn bump(width: usize, height: usize) -> raster::PreparedRaster {
        raster::test_raster_with(width, height, 1e-3, |c, r| {
            300. + 40. * (c as f64 / 5.).sin() * (r as f64 / 7.).cos() + 0.5 * c as f64
        })
    }
    /// NoData pixels (column, row from the north-west) of [`holed`]: an inner
    /// block and a south-east corner.
    fn missing(c: usize, r: usize) -> bool {
        (20..30).contains(&c) && (10..18).contains(&r) || c >= 46 && r >= 30
    }
    /// The relief with NoData holes.
    fn holed(width: usize, height: usize) -> raster::PreparedRaster {
        raster::test_raster_with(width, height, 1e-3, |c, r| {
            let h = 300. + 40. * (c as f64 / 5.).sin() * (r as f64 / 7.).cos() + 0.5 * c as f64;
            if missing(c, r) {
                f64::NAN
            } else {
                h
            }
        })
    }
    /// The lattice triangles `(c,r),(c+1,r),(c,r+1)` and `(c+1,r),(c+1,r+1),(c,r+1)`
    /// of every cell.
    fn triangles(columns: usize, rows: usize) -> impl Iterator<Item = [usize; 3]> {
        (0..rows - 1)
            .flat_map(move |r| (0..columns - 1).map(move |c| r * columns + c))
            .flat_map(move |i| {
                [
                    [i, i + 1, i + columns],
                    [i + 1, i + columns + 1, i + columns],
                ]
            })
    }
    fn sheet(pyramid: &Pyramid, plan: &PreparedPlan, patch: &Patch) -> Sheet {
        let g = pyramid.grid(patch, plan.storage(patch)).unwrap();
        Sheet {
            xs: patch.columns(),
            ys: patch.rows(),
            positions: g.positions,
            used: g.used,
        }
    }
    fn parents(plan: &PreparedPlan) -> BTreeMap<usize, usize> {
        let mut parents = BTreeMap::new();
        for (id, patch) in plan.patches.iter().enumerate() {
            for &child in &patch.children {
                parents.insert(child, id);
            }
        }
        parents
    }
    fn inside(b: [f64; 12], p: [f64; 3]) -> bool {
        (0..3).all(|i| (p[i] - b[i]).abs() <= b[3 + i * 4])
    }
    fn files(root: &Path) -> BTreeSet<String> {
        walkdir::WalkDir::new(root)
            .into_iter()
            .map(Result::unwrap)
            .filter(|e| e.file_type().is_file())
            .map(|e| {
                e.path()
                    .strip_prefix(root)
                    .unwrap()
                    .to_str()
                    .unwrap()
                    .replace('\\', "/")
            })
            .collect()
    }
    #[test]
    fn physical_partial_leaf_and_report_failures_preserve_old_output() {
        for member in ["0.glb", "conversion.json"] {
            let parent = crate::runtime::directory::test_directory();
            let output = parent.path().join("published");
            fs::create_dir(&output).unwrap();
            fs::write(output.join("old"), b"old inventory").unwrap();
            let control = RunControl::default();
            let attempt = control.begin().unwrap();
            let raster = raster::test_raster();
            let frame = Frame::new(raster.bounds);
            let pyramid = Pyramid::new(&raster, 16, &frame, 0., &attempt);
            let plan = plan(&pyramid).unwrap();
            let mut report = plan_report(&raster, 16, &plan, 0.).unwrap();
            let staging = DirectoryTarget::prepare(&output, OutputPolicy::Replace)
                .unwrap()
                .stage(&attempt)
                .unwrap();
            let work = staging.path().to_owned();
            let mut injected = false;
            let e = produce(
                staging.path(),
                &pyramid,
                &plan,
                &mut report,
                |path, bytes, count| {
                    if path.file_name().unwrap() == member {
                        let mut file = fs::File::options()
                            .write(true)
                            .create_new(true)
                            .open(path)
                            .unwrap();
                        file.write_all(&bytes[..3]).unwrap();
                        file.flush().unwrap();
                        injected = true;
                        return Err(JobError::io(
                            "injected partial terrain member",
                            path,
                            std::io::Error::other("persistent member fault"),
                        ));
                    }
                    write(path, bytes, count)
                },
                seal,
            )
            .unwrap_err();
            assert!(injected);
            assert!(work.exists());
            let failure = staging.fail(e);
            assert_eq!(failure.error.kind(), JobErrorKind::Io);
            assert!(failure.error.message().contains("persistent member fault"));
            assert!(!work.exists());
            assert_eq!(fs::read(output.join("old")).unwrap(), b"old inventory");
            assert_eq!(fs::read_dir(&output).unwrap().count(), 1);
            assert_eq!(fs::read_dir(parent.path()).unwrap().count(), 1);
        }
    }
    #[test]
    fn forest_members_are_synced_counted_and_their_faults_preserve_old_output() {
        // 17x3 leaves: six levels, so the forest has skeleton tiles, explicit
        // payloads in place, implicit roots, subtrees and relocated payloads.
        let raster = bump(260, 40);
        let frame = Frame::new(raster.bounds);
        let parent = crate::runtime::directory::test_directory();
        let output = parent.path().join("published");
        fs::create_dir(&output).unwrap();
        fs::write(output.join("old"), b"old inventory").unwrap();
        let control = RunControl::default();
        let attempt = control.begin().unwrap();
        let pyramid = Pyramid::new(&raster, 16, &frame, 0., &attempt);
        let plan = plan(&pyramid).unwrap();
        assert_eq!(pyramid.depth, 5);
        let mut report = plan_report(&raster, 16, &plan, 0.).unwrap();
        let staging = DirectoryTarget::prepare(&output, OutputPolicy::Replace)
            .unwrap()
            .stage(&attempt)
            .unwrap();
        let mut synced = BTreeSet::new();
        produce(
            staging.path(),
            &pyramid,
            &plan,
            &mut report,
            write,
            |path| {
                let name = path.strip_prefix(staging.path()).unwrap();
                assert!(synced.insert(name.to_str().unwrap().replace('\\', "/")));
                seal(path)
            },
        )
        .unwrap();
        let members = files(staging.path());
        assert_eq!(synced, members, "every published member is synced once");
        assert!(members.contains("tiles/0/0/0.glb") && members.contains("tiles/1/0/1.glb"));
        assert!(members.iter().any(|m| m.ends_with(".subtree")));
        assert!(members.iter().any(|m| m.starts_with("implicit-content/")));
        assert!(
            !staging.path().join("tiles/2").exists(),
            "emptied levels are pruned"
        );
        assert_eq!(
            files(staging.path())
                .iter()
                .map(|m| fs::metadata(staging.path().join(m)).unwrap().len())
                .sum::<u64>(),
            report.generated_bytes
        );
        drop(staging);
        // A persistent sync fault on a forest member is a typed failure before publication.
        let control = RunControl::default();
        let attempt = control.begin().unwrap();
        let pyramid = Pyramid::new(&raster, 16, &frame, 0., &attempt);
        let mut report = plan_report(&raster, 16, &plan, 0.).unwrap();
        let staging = DirectoryTarget::prepare(&output, OutputPolicy::Replace)
            .unwrap()
            .stage(&attempt)
            .unwrap();
        let work = staging.path().to_owned();
        let e = produce(
            staging.path(),
            &pyramid,
            &plan,
            &mut report,
            write,
            |path| {
                if path.extension().unwrap() == "subtree" {
                    return Err(JobError::io(
                        "sync terrain member",
                        path,
                        std::io::Error::other("injected sync fault"),
                    ));
                }
                seal(path)
            },
        )
        .unwrap_err();
        let failure = staging.fail(e);
        assert_eq!(failure.error.kind(), JobErrorKind::Io);
        assert!(failure.error.message().contains("injected sync fault"));
        assert!(!work.exists());
        assert_eq!(fs::read(output.join("old")).unwrap(), b"old inventory");
        assert_eq!(fs::read_dir(&output).unwrap().count(), 1);
    }
    #[test]
    fn collapsed_and_vertical_decoded_triangles_are_rejected() {
        let a = [0., 0., 0.];
        assert!(!triangle_is_surface(a, a, [0., 0., -1.], [0., 0., 1.]));
        assert!(!triangle_is_surface(
            a,
            [0., 1., 0.],
            [0., 0., -1.],
            [0., 0., 1.]
        ));
        assert!(triangle_is_surface(
            a,
            [1., 0., 0.],
            [0., 0., -1.],
            [0., 0., 1.]
        ));
        assert!(!triangle_is_surface(
            a,
            [0., 0., -1.],
            [1., 0., 0.],
            [0., 0., 1.]
        ));
    }
    struct ReadyObserver {
        cancel: std::sync::Mutex<Option<crate::CancellationHandle>>,
        reject: bool,
    }
    impl crate::Observer for ReadyObserver {
        fn observe(&self, event: &RunEvent<'_>) -> Result<(), JobError> {
            if matches!(event,RunEvent::Note{message} if *message=="terrain mesh directory ready for publication")
            {
                if self.reject {
                    return Err(error(JobErrorKind::ObserverFailure, "ready observer fault"));
                }
                if let Some(handle) = self.cancel.lock().unwrap().take() {
                    handle.cancel();
                }
            }
            Ok(())
        }
    }
    #[test]
    fn ready_observer_failure_and_cancellation_clean_finished_tree() {
        for reject in [true, false] {
            let observer = std::sync::Arc::new(ReadyObserver {
                cancel: std::sync::Mutex::new(None),
                reject,
            });
            let control = RunControl::new(Some(observer.clone()));
            if !reject {
                *observer.cancel.lock().unwrap() = Some(control.cancellation_handle());
            }
            let attempt = control.begin().unwrap();
            let parent = crate::runtime::directory::test_directory();
            let output = parent.path().join("published");
            fs::create_dir(&output).unwrap();
            fs::write(output.join("old"), b"old inventory").unwrap();
            let raster = raster::test_raster();
            let frame = Frame::new(raster.bounds);
            let pyramid = Pyramid::new(&raster, 16, &frame, 0., &attempt);
            let plan = plan(&pyramid).unwrap();
            let mut report = plan_report(&raster, 16, &plan, 0.).unwrap();
            let staging = DirectoryTarget::prepare(&output, OutputPolicy::Replace)
                .unwrap()
                .stage(&attempt)
                .unwrap();
            let work = staging.path().to_owned();
            let e = produce(staging.path(), &pyramid, &plan, &mut report, write, seal).unwrap_err();
            assert!(work.join("conversion.json").exists());
            let failure = staging.fail(e);
            assert_eq!(
                failure.error.kind(),
                if reject {
                    JobErrorKind::ObserverFailure
                } else {
                    JobErrorKind::Cancelled
                }
            );
            assert!(!work.exists());
            assert_eq!(fs::read(output.join("old")).unwrap(), b"old inventory");
            assert_eq!(fs::read_dir(parent.path()).unwrap().count(), 1);
        }
    }
    struct Writer {
        bytes: Vec<u8>,
        fault: u8,
    }
    impl Write for Writer {
        fn write(&mut self, b: &[u8]) -> std::io::Result<usize> {
            if self.fault == 1 && !self.bytes.is_empty() {
                return Err(std::io::Error::other("write fault"));
            }
            let n = if self.fault == 1 { 1 } else { b.len() };
            self.bytes.extend_from_slice(&b[..n]);
            Ok(n)
        }
        fn flush(&mut self) -> std::io::Result<()> {
            if self.fault == 2 {
                Err(std::io::Error::other("flush fault"))
            } else {
                Ok(())
            }
        }
    }
    #[test]
    fn member_finish_faults_remain_errors() {
        for fault in [1, 2] {
            let mut writer = Writer {
                bytes: vec![],
                fault,
            };
            let e = finish_member(&mut writer, b"mesh").unwrap_err();
            assert!(e.to_string().contains("fault"));
        }
    }
    #[test]
    fn inadmissible_height_fails_in_preflight() {
        let control = RunControl::default();
        let attempt = control.begin().unwrap();
        let r = raster::test_raster();
        let frame = Frame::new(r.bounds);
        let e = plan(&Pyramid::new(&r, 16, &frame, 9000., &attempt))
            .err()
            .unwrap();
        assert_eq!(e.kind(), JobErrorKind::Unsupported);
        assert!(e.message().contains("-10000..8000"));
    }
    #[test]
    fn pyramid_shape_slots_clipped_end_nodes_and_flat_leaf_identity() {
        let control = RunControl::default();
        let attempt = control.begin().unwrap();
        for (width, height, depth, per_level) in [
            (37, 20, 2, vec![1, 2, 6]),
            (16, 70, 3, vec![1, 2, 3, 5]),
            (2, 2, 0, vec![1]),
        ] {
            let r = bump(width, height);
            let frame = Frame::new(r.bounds);
            let pyramid = Pyramid::new(&r, 16, &frame, 0., &attempt);
            let plan = plan(&pyramid).unwrap();
            assert_eq!(pyramid.depth, depth);
            let report = plan_report(&r, 16, &plan, 0.).unwrap();
            assert_eq!(report.tiles_per_level, per_level);
            assert_eq!(report.levels, depth + 1);
            let existing: BTreeSet<_> = plan
                .patches
                .iter()
                .map(|p| (p.level, p.column, p.row))
                .collect();
            for patch in &plan.patches {
                let (xs, ys) = (patch.columns(), patch.rows());
                assert!(xs.len() <= 17 && ys.len() <= 17);
                assert_eq!(xs.last(), Some(&patch.x1));
                assert_eq!(ys.last(), Some(&patch.y1));
                assert!(xs[..xs.len() - 1].iter().all(|x| x % patch.stride == 0));
                assert_eq!(xs.len() - 1, (patch.x1 - patch.x).div_ceil(patch.stride));
                let neighbour = |c: Option<usize>, r: Option<usize>| {
                    c.zip(r)
                        .is_some_and(|(c, r)| existing.contains(&(patch.level, c, r)))
                };
                let (c, r) = (patch.column, patch.row);
                assert_eq!(
                    patch.skirt_depth.map(|d| d > 0.),
                    [
                        neighbour(c.checked_sub(1), Some(r)),
                        neighbour(Some(c + 1), Some(r)),
                        neighbour(Some(c), r.checked_sub(1)),
                        neighbour(Some(c), Some(r + 1)),
                    ]
                );
                let slots: Vec<_> = patch
                    .children
                    .iter()
                    .map(|&c| plan.patches[c].slot())
                    .collect();
                assert!(slots.windows(2).all(|w| w[0] < w[1]));
                assert!(patch.children.is_empty() || slots[0] == 0);
                for &child in &patch.children {
                    let child = &plan.patches[child];
                    assert_eq!(
                        (child.column / 2, child.row / 2, child.level),
                        (patch.column, patch.row, patch.level + 1)
                    );
                }
            }
            // Leaves are exactly the flat T1 patches, names aside.
            let leaves: BTreeSet<_> = plan
                .patches
                .iter()
                .filter(|p| p.level == depth)
                .map(|p| (p.x, p.y, p.x1, p.y1, p.stride, p.name()))
                .collect();
            let flat: BTreeSet<_> = (0..height)
                .step_by(16)
                .flat_map(|y| (0..width).step_by(16).map(move |x| (x, y)))
                .map(|(x, y)| {
                    let name = format!("tiles/{depth}/{}/{}.glb", y / 16, x / 16);
                    (x, y, (x + 16).min(width), (y + 16).min(height), 1, name)
                })
                .collect();
            assert_eq!(leaves, flat);
        }
        let single = plan(&Pyramid::new(
            &raster::test_raster(),
            16,
            &Frame::new(raster::test_raster().bounds),
            0.,
            &attempt,
        ))
        .unwrap();
        assert_eq!(single.patches.len(), 1);
        assert_eq!((single.root().error, single.skirt_vertices), (0., 0));
        let r = raster::test_raster();
        assert_eq!(
            plan_report(&r, 16, &single, 0.).unwrap().skirt_depth_metres,
            None
        );
    }
    /// A coarse tile over `[0, width] x [0, height]` with `stride` and its four
    /// children at half the stride, all stored as `(x, height, -y)` so distances
    /// are vertical.
    fn synthetic(
        width: usize,
        height: usize,
        cells: usize,
        stride: usize,
        coarse: impl Fn(usize, usize) -> f64,
        fine: impl Fn(&Sheet, usize, usize) -> f64,
    ) -> (Patch, Sheet, Vec<(usize, Sheet)>) {
        let store = |xs: Vec<usize>, ys: Vec<usize>, h: &dyn Fn(usize, usize) -> f64| Sheet {
            positions: ys
                .iter()
                .flat_map(|&y| xs.iter().map(move |&x| (x, y)))
                .map(|(x, y)| [x as f64, h(x, y), -(y as f64)])
                .collect(),
            used: vec![true; xs.len() * ys.len()],
            xs,
            ys,
        };
        let patch = Patch {
            level: 0,
            column: 0,
            row: 0,
            stride,
            x: 0,
            y: 0,
            x1: width,
            y1: height,
            content: true,
            children: vec![],
            error: 0.,
            drawn: 0.,
            skirt_depth: [0.; 4],
            surface: [[0.; 3]; 2],
            bounds: [[0.; 3]; 2],
            hierarchy: [[0.; 3]; 2],
        };
        let sheet = store(patch.columns(), patch.rows(), &coarse);
        let span = cells * stride / 2;
        let children = (0..4)
            .map(|slot| (slot, (slot & 1) * span, (slot >> 1) * span))
            .filter(|&(_, x, y)| x < width && y < height)
            .map(|(slot, x, y)| {
                let xs = lattice(x, (x + span).min(width), stride / 2);
                let ys = lattice(y, (y + span).min(height), stride / 2);
                (slot, store(xs, ys, &|x, y| fine(&sheet, x, y)))
            })
            .collect();
        (patch, sheet, children)
    }
    /// Dense brute force of the coarse-to-child distance on a 1/32 parameter grid.
    fn dense(sheet: &Sheet, children: &[(usize, Sheet)], cells: usize) -> f64 {
        let (w, h) = (*sheet.xs.last().unwrap(), *sheet.ys.last().unwrap());
        let mut worst: f64 = 0.;
        for j in 0..=h * 32 {
            for i in 0..=w * 32 {
                let (x, y) = (i as f64 / 32., j as f64 / 32.);
                let slot = usize::from(x >= cells as f64) | usize::from(y >= cells as f64) << 1;
                let fine = &children.iter().find(|(s, _)| *s == slot).unwrap().1;
                worst = worst.max(distance(fine.at(x, y).unwrap(), sheet.at(x, y).unwrap()));
            }
        }
        worst
    }
    #[test]
    fn deviation_is_exact_on_planes_and_includes_clipped_diagonal_crossings() {
        let plane = |x: usize, y: usize| 2. * x as f64 + 3. * y as f64 + 5.;
        let (patch, sheet, children) = synthetic(8, 8, 4, 2, plane, |_, x, y| plane(x, y));
        assert_eq!(deviation(&patch, &sheet, &children, 4, &mut vec![]), 0.);
        // Full-stride cells: the bound is the vertex maximum and equals the dense maximum.
        let wavy = |x: usize, y: usize| ((x * 7 + y * 13) % 5) as f64;
        let (patch, sheet, children) = synthetic(8, 8, 4, 2, wavy, |_, x, y| wavy(x, y));
        let d = deviation(&patch, &sheet, &children, 4, &mut vec![]);
        assert!(d > 1.);
        assert!((d - dense(&sheet, &children, 4)).abs() <= 1e-9);
        // A clipped saddle cell [2, 3] x [0, 2] whose child vertices all lie on the
        // coarse surface: only the crossing of the coarse diagonal with the child
        // row y = 1, at x = 2.5, sees the 5 m difference.
        let saddle = |x: usize, y: usize| match (x, y) {
            (2, 0) | (3, 2) => 10.,
            _ => 0.,
        };
        let (patch, sheet, children) = synthetic(3, 4, 2, 2, saddle, |coarse, x, y| {
            coarse.at(x as f64, y as f64).unwrap()[2]
        });
        let vertices = children
            .iter()
            .flat_map(|(_, fine)| {
                fine.ys
                    .iter()
                    .flat_map(|&y| fine.xs.iter().map(move |&x| (x as f64, y as f64)))
                    .map(|(x, y)| distance(fine.at(x, y).unwrap(), sheet.at(x, y).unwrap()))
            })
            .fold(0., f64::max);
        assert_eq!(vertices, 0., "a vertex-only bound would certify zero");
        let d = deviation(&patch, &sheet, &children, 2, &mut vec![]);
        assert!((d - 5.).abs() <= 1e-9, "{d}");
        assert!((d - dense(&sheet, &children, 2)).abs() <= 1e-9);
        // The transposed saddle in [0, 2] x [2, 3]: only the crossing with the
        // child column x = 1, at y = 2.5, sees it.
        let saddle = |x: usize, y: usize| match (x, y) {
            (0, 2) | (2, 3) => 10.,
            _ => 0.,
        };
        let (patch, sheet, children) = synthetic(4, 3, 2, 2, saddle, |coarse, x, y| {
            coarse.at(x as f64, y as f64).unwrap()[2]
        });
        let d = deviation(&patch, &sheet, &children, 2, &mut vec![]);
        assert!((d - 5.).abs() <= 1e-9, "{d}");
        assert!((d - dense(&sheet, &children, 2)).abs() <= 1e-9);
    }
    /// An exact overlay reference, independent of `deviation`'s case analysis:
    /// child vertices plus every crossing of a coarse diagonal with any child
    /// edge, sides and diagonals alike. Returns the maximum distance, the vertex
    /// maximum and the number of crossings strictly inside a child diagonal.
    fn overlay(sheet: &Sheet, children: &[(usize, Sheet)], span: usize) -> (f64, f64, usize) {
        let cross = |a: (f64, f64), b: (f64, f64)| a.0 * b.1 - a.1 * b.0;
        let (mut worst, mut vertices, mut inside) = (0f64, 0f64, 0);
        for r in 0..sheet.ys.len() - 1 {
            for c in 0..sheet.xs.len() - 1 {
                let (xa, xb, ya, yb) = (sheet.xs[c], sheet.xs[c + 1], sheet.ys[r], sheet.ys[r + 1]);
                let slot = usize::from(xa >= span) | usize::from(ya >= span) << 1;
                let fine = &children.iter().find(|(s, _)| *s == slot).unwrap().1;
                let fx: Vec<_> = fine
                    .xs
                    .iter()
                    .copied()
                    .filter(|x| (xa..=xb).contains(x))
                    .collect();
                let fy: Vec<_> = fine
                    .ys
                    .iter()
                    .copied()
                    .filter(|y| (ya..=yb).contains(y))
                    .collect();
                let mut points = vec![];
                for &y in &fy {
                    for &x in &fx {
                        let p = (x as f64, y as f64);
                        vertices = vertices.max(distance(
                            fine.at(p.0, p.1).unwrap(),
                            sheet.cell(c, r, p.0, p.1).unwrap(),
                        ));
                        points.push(p);
                    }
                }
                let start = (xb as f64, ya as f64);
                let direction = (xa as f64 - start.0, yb as f64 - start.1);
                for j in 0..fy.len() - 1 {
                    for i in 0..fx.len() - 1 {
                        let (x0, x1, y0, y1) = (
                            fx[i] as f64,
                            fx[i + 1] as f64,
                            fy[j] as f64,
                            fy[j + 1] as f64,
                        );
                        let edges = [
                            ((x0, y0), (x1, y0), false),
                            ((x0, y1), (x1, y1), false),
                            ((x0, y0), (x0, y1), false),
                            ((x1, y0), (x1, y1), false),
                            ((x1, y0), (x0, y1), true),
                        ];
                        for (a, b, diagonal) in edges {
                            let other = (b.0 - a.0, b.1 - a.1);
                            let denominator = cross(direction, other);
                            if denominator == 0. {
                                continue;
                            }
                            let offset = (a.0 - start.0, a.1 - start.1);
                            let (t, u) = (
                                cross(offset, other) / denominator,
                                cross(offset, direction) / denominator,
                            );
                            if (0. ..=1.).contains(&t) && (0. ..=1.).contains(&u) {
                                inside += usize::from(diagonal && u > 1e-12 && u < 1. - 1e-12);
                                points.push((start.0 + t * direction.0, start.1 + t * direction.1));
                            }
                        }
                    }
                }
                for (x, y) in points {
                    worst = worst.max(distance(
                        fine.at(x, y).unwrap(),
                        sheet.cell(c, r, x, y).unwrap(),
                    ));
                }
            }
        }
        (worst, vertices, inside)
    }
    #[test]
    fn clipped_cells_match_an_exact_overlay_reference() {
        // Pseudo-random coarse heights, shared by both levels at common nodes, over every
        // clipped end width and height for child strides 1, 2 and 4.
        let height = |x: usize, y: usize| {
            let h = (x as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15)
                ^ (y as u64).wrapping_mul(0xC2B2_AE3D_27D4_EB4F);
            (h.wrapping_mul(0x1656_67B1_9E37_79F9) >> 40) as f64 / 65536.
        };
        let mut sensitive = 0;
        for stride in [2, 4, 8] {
            // Children split the coarse tile at `span`; widths clip its last cell by 1..=stride.
            let span = stride;
            for width in span + 1..=2 * span {
                for depth in span + 1..=2 * span {
                    // Child-only nodes sit near the coarse surface, so crossings in
                    // clipped saddle cells, not vertices, set most maxima.
                    let (patch, sheet, children) =
                        synthetic(width, depth, 2, stride, height, |coarse, x, y| {
                            let node = |n: usize, end: usize| n.is_multiple_of(stride) || n == end;
                            let base = coarse.at(x as f64, y as f64).unwrap()[2];
                            if node(x, width) && node(y, depth) {
                                base
                            } else {
                                base + 0.05 * height(y, x)
                            }
                        });
                    let d = deviation(&patch, &sheet, &children, 2, &mut vec![]);
                    let (reference, vertices, inside) = overlay(&sheet, &children, span);
                    // The clipping argument: no child diagonal is crossed in its interior.
                    assert_eq!(inside, 0, "{stride} {width} {depth}");
                    assert!(
                        (d - reference).abs() <= 1e-9,
                        "{stride} {width} {depth}: {d} {reference}"
                    );
                    sensitive += usize::from(reference > vertices + 1e-6);
                }
            }
        }
        // Crossings, not vertices, set the maximum in many of these cells.
        assert!(sensitive > 20, "{sensitive}");
    }
    /// The review's xorshift, to regenerate its fuzz rasters.
    struct Xorshift(u64);
    impl Xorshift {
        fn next(&mut self) -> u64 {
            self.0 ^= self.0 << 13;
            self.0 ^= self.0 >> 7;
            self.0 ^= self.0 << 17;
            self.0
        }
        fn unit(&mut self) -> f64 {
            (self.next() >> 11) as f64 / (1u64 << 53) as f64
        }
        fn below(&mut self, n: usize) -> usize {
            (self.next() % n as u64) as usize
        }
    }
    /// The exact largest distance from a tile to the leaf surface: in every
    /// cell, at each leaf node and each crossing of the cell diagonal with
    /// leaf edges, on any emitted triangle within rounding.
    fn exact(pyramid: &Pyramid, plan: &PreparedPlan, patch: &Patch) -> f64 {
        let s = sheet(pyramid, plan, patch);
        let mut worst: f64 = 0.;
        for r in 0..s.ys.len() - 1 {
            for c in 0..s.xs.len() - 1 {
                let (xa, xb, ya, yb) = (s.xs[c], s.xs[c + 1], s.ys[r], s.ys[r + 1]);
                let storage = plan.storage[pyramid.depth as usize];
                let leaf = pyramid.leaves([xa, xb], [ya, yb], storage).unwrap();
                let (w, h) = (xb - xa, yb - ya);
                let mut points: Vec<_> = (ya..=yb)
                    .flat_map(|y| (xa..=xb).map(move |x| (x as f64, y as f64)))
                    .collect();
                for n in [w, h, w.abs_diff(h)].into_iter().filter(|n| *n > 0) {
                    let t = |k: usize| k as f64 / n as f64;
                    points.extend(
                        (0..=n).map(|k| (xb as f64 - t(k) * w as f64, ya as f64 + t(k) * h as f64)),
                    );
                }
                for (x, y) in points {
                    let (mut a, mut b) = (vec![], vec![]);
                    s.near_cell(c, r, x, y, &mut a);
                    let node = (x.fract() == 0. && y.fract() == 0.)
                        .then(|| ((x as usize - xa), (y as usize - ya)))
                        .filter(|&(i, j)| leaf.used[j * leaf.xs.len() + i]);
                    match node {
                        Some((i, j)) => b.push(leaf.vertex(i, j)),
                        None => leaf.near(x, y, &mut b),
                    }
                    for (p, q) in a.iter().flat_map(|p| b.iter().map(move |q| (p, q))) {
                        worst = worst.max(distance(*p, *q));
                    }
                }
            }
        }
        worst
    }
    #[test]
    fn certified_errors_bound_dense_leaf_distance_and_are_monotone() {
        let control = RunControl::default();
        let attempt = control.begin().unwrap();
        // Even dimensions keep every cell full-stride; odd ones clip end strips;
        // a NoData hole is measured where both surfaces exist. In `bridged`,
        // NoData pixel column 9 removes level 1's cells over x 8..12, which
        // level 0 bridges, while leaves keep x 11..12 and its 30 m cliff (pixel
        // column 10). `spiky` repeats that with 150 m pixel spikes beside 1-pixel holes.
        let bridged = raster::test_raster_with(64, 32, 1e-5, |c, r| {
            [
                100. + 0.2 * c as f64 + 0.1 * r as f64 + [0., 60.][usize::from(c == 10)],
                f64::NAN,
            ][usize::from(c == 9)]
        });
        let spiky = raster::test_raster_with(67, 45, 1e-5, |c, r| {
            let row = r % 3 == 0;
            let spike = [0., 150.][usize::from(row && c % 4 == 2)];
            [100. + 0.3 * c as f64 + spike, f64::NAN][usize::from(row && c % 4 == 1)]
        });
        // The review's s85 raster: holes and spikes only in its clipped far
        // strips, where a clipped cell keeps one coarse triangle.
        let mut g = Xorshift(85 * 104_729 + 5);
        let cells85 = [4, 8][g.below(2)];
        let (w, h) = (
            16 * (2 + g.below(4)) + 1 + g.below(15),
            16 * (2 + g.below(4)) + 1 + g.below(15),
        );
        let (hole, spike, edge) = (0.1 + 0.3 * g.unit(), 0.1 + 0.4 * g.unit(), 1 + g.below(8));
        let v: Vec<f64> = (0..w * h)
            .map(|i| {
                let (c, r) = (i % w, i / w);
                let base = 100. + 0.2 * c as f64 + 0.1 * r as f64;
                let u = if c + edge < w && r + edge < h {
                    1.
                } else {
                    g.unit()
                };
                match (u < hole, u < hole + spike) {
                    (true, _) => f64::NAN,
                    (_, true) => base + 150. * g.unit(),
                    _ => base,
                }
            })
            .collect();
        assert_eq!((w, h, cells85), (35, 79, 4));
        let s85 = raster::test_raster_with(w, h, 1e-5, move |c, r| v[r * w + c]);
        for (n, (r, full_stride, cells)) in [
            (bump(64, 48), true, 16),
            (bump(67, 45), false, 16),
            (holed(67, 45), false, 16),
            (bridged, false, 16),
            (spiky, false, 16),
            (s85, false, cells85),
        ]
        .into_iter()
        .enumerate()
        {
            let frame = Frame::new(r.bounds);
            let pyramid = Pyramid::new(&r, cells, &frame, 0., &attempt);
            let plan = plan(&pyramid).unwrap();
            let sheets: Vec<_> = plan
                .patches
                .iter()
                .map(|p| sheet(&pyramid, &plan, p))
                .collect();
            let leaves: Vec<_> = (0..plan.patches.len())
                .filter(|&i| plan.patches[i].level == pyramid.depth)
                .collect();
            for (id, patch) in plan.patches.iter().enumerate() {
                if patch.level == pyramid.depth {
                    assert_eq!(patch.error, 0.);
                    continue;
                }
                for &child in patch.children.iter().filter(|_| patch.content) {
                    assert!(patch.error >= plan.patches[child].drawn);
                }
                let mut measured: f64 = 0.;
                for &leaf in &leaves {
                    let l = &plan.patches[leaf];
                    if l.x < patch.x || l.x1 > patch.x1 || l.y < patch.y || l.y1 > patch.y1 {
                        continue;
                    }
                    for j in 0..=(l.y1 - l.y) * 4 {
                        for i in 0..=(l.x1 - l.x) * 4 {
                            let (x, y) = (l.x as f64 + i as f64 / 4., l.y as f64 + j as f64 / 4.);
                            if let (Some(a), Some(b)) = (sheets[id].at(x, y), sheets[leaf].at(x, y))
                            {
                                measured = measured.max(distance(a, b));
                            }
                        }
                    }
                }
                assert!(
                    n > 2 || measured > 0.1,
                    "the fixture must exercise level {}",
                    patch.level
                );
                assert!(patch.error >= measured, "{n}: {} < {measured}", patch.error);
                let exact = exact(&pyramid, &plan, patch);
                assert!(patch.error >= exact, "{n}: {} < exact {exact}", patch.error);
                if full_stride && patch.level + 1 == pyramid.depth {
                    assert!(
                        patch.error <= measured + 1e-9,
                        "{} > {measured}",
                        patch.error
                    );
                }
            }
            let report = plan_report(&r, cells, &plan, 0.).unwrap();
            // Per-level maxima are monotone where no level holds a routing floor.
            let floors = plan.patches.iter().any(|p| !p.content);
            assert!(
                floors
                    || report
                        .geometric_error_metres
                        .windows(2)
                        .all(|w| w[0] >= w[1])
            );
            assert_eq!(report.root_geometric_error_metres, plan.root().error);
        }
        // A constant height only leaves the Earth-curvature sag of decimated
        // chords and the root and leaf grid errors.
        let r = raster::test_raster_with(64, 64, 1e-4, |_, _| 120.);
        let frame = Frame::new(r.bounds);
        let plan = plan(&Pyramid::new(&r, 16, &frame, 0., &attempt)).unwrap();
        // Each level's grid error enters the chain of level-to-level distances
        // at most twice.
        let grids: f64 = plan.storage.iter().map(|s| s.margin(0.)).sum();
        assert!(grids > 0. && grids < 0.06, "{grids}");
        assert!(plan.root().error < 1e-3 + grids, "{}", plan.root().error);
    }
    #[test]
    fn skirts_cover_every_mixed_level_seam_and_face_outward() {
        let control = RunControl::default();
        let attempt = control.begin().unwrap();
        // A 30 m ridge on pixel columns 63-64, beside the level-1 seam x = 64,
        // with a NoData column that level 2 bridges (65) or loses (60).
        let ridge = |hole: usize| {
            raster::test_raster_with(160, 48, 1e-5, move |c, r| {
                let top = if (63..65).contains(&c) && r / 2 % 2 == 0 {
                    30.
                } else {
                    0.
                };
                [100. + 0.2 * c as f64 + 0.1 * r as f64 + top, f64::NAN][usize::from(c == hole)]
            })
        };
        for (n, r) in [bump(67, 45), holed(67, 45), ridge(65), ridge(60)]
            .iter()
            .enumerate()
        {
            let frame = Frame::new(r.bounds);
            let pyramid = Pyramid::new(r, 16, &frame, 0., &attempt);
            let plan = plan(&pyramid).unwrap();
            let root = plan.root().error;
            let shallow = (plan.patches.iter())
                .filter(|p| p.level == pyramid.depth)
                .flat_map(|p| p.skirt_depth)
                .filter(|d| *d > 0. && *d < root / 4.)
                .count();
            assert!(n > 1 || root > 8. && shallow > 10, "{root} {shallow}");
            // Per tile: each skirted lattice segment and its decoded bottom heights.
            let mut curtains = vec![BTreeMap::new(); plan.patches.len()];
            let sheets: Vec<_> = plan
                .patches
                .iter()
                .map(|p| sheet(&pyramid, &plan, p))
                .collect();
            for (id, patch) in plan.patches.iter().enumerate() {
                let depths = patch.skirt_depth;
                assert!(depths.iter().all(|d| *d == 0. || *d >= patch.error.max(1.)));
                let storage = plan.storage(patch);
                let (skirt, _, _) = pyramid.skirt(patch, storage, patch.skirt_depth).unwrap();
                let surface = pyramid.grid(patch, storage).unwrap().positions;
                let vertex = |i: u32| {
                    let i = i as usize;
                    decoded(
                        *surface
                            .get(i)
                            .unwrap_or_else(|| &skirt.bottoms[i - surface.len()]),
                    )
                };
                assert_eq!(skirt.indices.is_empty(), depths == [0.; 4]);
                let centre: [f64; 3] =
                    std::array::from_fn(|i| (patch.surface[0][i] + patch.surface[1][i]) / 2.);
                // Each quad [t, top, b, top, base, b] hangs bottoms below surface tops.
                let (xs, ys) = (patch.columns(), patch.rows());
                let node = |i: u32| (xs[i as usize % xs.len()], ys[i as usize / xs.len()]);
                for quad in skirt.indices.chunks(6) {
                    let ends = [(quad[0], quad[2]), (quad[1], quad[4])];
                    let mut ends = ends.map(|(t, b)| (node(t), vertex(b)[2]));
                    ends.sort_by_key(|e| e.0);
                    curtains[id].insert((ends[0].0, ends[1].0), [ends[0].1, ends[1].1]);
                    for (top, bottom) in [(quad[0], quad[2]), (quad[1], quad[4])] {
                        assert!(
                            (top as usize) < surface.len(),
                            "skirt tops are surface vertices"
                        );
                        assert!(bottom as usize >= surface.len());
                        let drop = distance(vertex(top), vertex(bottom));
                        // Rounding moves each end by at most half the margin.
                        let margin = storage.margin(1e5);
                        assert!(
                            depths.iter().any(|d| (drop - d).abs() <= margin),
                            "{drop} {margin}"
                        );
                    }
                }
                for t in skirt.indices.chunks(3) {
                    let [a, b, c] = [0, 1, 2].map(|k| vertex(t[k]));
                    let (u, v) = (
                        [b[0] - a[0], b[1] - a[1], b[2] - a[2]],
                        [c[0] - a[0], c[1] - a[1], c[2] - a[2]],
                    );
                    let normal = [
                        u[1] * v[2] - u[2] * v[1],
                        u[2] * v[0] - u[0] * v[2],
                        u[0] * v[1] - u[1] * v[0],
                    ];
                    let outward: [f64; 2] =
                        std::array::from_fn(|i| (a[i] + b[i] + c[i]) / 3. - centre[i]);
                    assert!(normal[0] * outward[0] + normal[1] * outward[1] > 0.);
                }
            }
            // Every pair of tiles that a REPLACE cut can place side by side (any
            // levels, disjoint footprints sharing an edge segment): the gap along
            // the shared edge is covered by the skirt hanging from either side,
            // including a leaf beside a coarser tile.
            let (mut worst, mut pairs, mut mixed) = (0f64, 0, 0);
            for (i, a) in plan.patches.iter().enumerate() {
                for (j, b) in plan.patches.iter().enumerate() {
                    if !a.content || !b.content {
                        continue;
                    }
                    let overlap = |a0: usize, a1: usize, b0: usize, b1: usize| {
                        (a0.max(b0) < a1.min(b1)).then(|| (a0.max(b0), a1.min(b1)))
                    };
                    let segment = if a.x1 == b.x {
                        overlap(a.y, a.y1, b.y, b.y1).map(|(y0, y1)| ((a.x1, y0), (a.x1, y1), 1))
                    } else if a.y1 == b.y {
                        overlap(a.x, a.x1, b.x, b.x1).map(|(x0, x1)| ((x0, a.y1), (x1, a.y1), 3))
                    } else {
                        None
                    };
                    let Some(((x0, y0), (x1, y1), k)) = segment else {
                        continue;
                    };
                    pairs += 1;
                    let leaf = pyramid.depth;
                    let beside = (a.level == leaf) != (b.level == leaf);
                    let steps = 4 * (x1 - x0 + y1 - y0);
                    for step in 0..=steps {
                        let t = step as f64 / steps as f64;
                        let (x, y) = (
                            x0 as f64 + t * (x1 - x0) as f64,
                            y0 as f64 + t * (y1 - y0) as f64,
                        );
                        let (Some(p), Some(q)) = (sheets[i].at(x, y), sheets[j].at(x, y)) else {
                            continue;
                        };
                        let gap = distance(p, q);
                        worst = worst.max(gap);
                        mixed += usize::from(beside && gap > 0.1);
                        // Both edges lie within their errors of the leaf nodes.
                        assert!(gap <= a.error + b.error + 1e-9, "{gap}");
                        if gap < 1e-6 {
                            continue;
                        }
                        // The higher side's own curtain reaches the lower surface.
                        let (high, low) = if p[2] >= q[2] { (i, q) } else { (j, p) };
                        let h = &plan.patches[high];
                        let (along, s) = if k == 1 {
                            (h.rows(), y)
                        } else {
                            (h.columns(), x)
                        };
                        let covered = (along.windows(2))
                            .filter(|w| w[0] as f64 <= s && s <= w[1] as f64)
                            .filter_map(|w| {
                                let (lo, hi) = (w[0], w[1]);
                                let key = if k == 1 {
                                    ((x1, lo), (x1, hi))
                                } else {
                                    ((lo, y1), (hi, y1))
                                };
                                let t = (s - lo as f64) / (hi - lo) as f64;
                                (curtains[high].get(&key)).map(|b| b[0] + t * (b[1] - b[0]))
                            })
                            .any(|bottom| bottom <= low[2] + 1e-9);
                        assert!(covered, "uncovered seam at ({x}, {y}): gap {gap}");
                    }
                }
            }
            assert!(
                pairs > 20 && worst > 0.1 && mixed > 0,
                "{pairs} {worst} {mixed}"
            );
        }
    }
    #[test]
    fn nodata_is_a_hole_without_fill_or_skirts_and_empty_leaves_vanish() {
        let control = RunControl::default();
        let attempt = control.begin().unwrap();
        let (r, (w, h)) = (holed(67, 45), (67, 45));
        // A lattice node (from the south-west) is valid when every pixel with
        // a nonzero bilinear weight is.
        let near = |v: usize, n: usize| v.saturating_sub(1)..=v.min(n - 1);
        let valid = |(x, y)| near(x, w).all(|c| near(h - y, h).all(|r| !missing(c, r)));
        let frame = Frame::new(r.bounds);
        let pyramid = Pyramid::new(&r, 16, &frame, 0., &attempt);
        let plan = plan(&pyramid).unwrap();
        let leaf = |c| (plan.patches.iter()).any(|p| (p.level, p.column, p.row) == (3, c, 0));
        // Leaves (3, 0) and (4, 0) have no valid triangle; no height is filled.
        assert!(leaf(2) && !leaf(3) && !leaf(4) && plan.height_range[0] > 250.);
        let (mut open, mut hidden) = (0, 0);
        for patch in &plan.patches {
            let storage = plan.storage(patch);
            let grid = pyramid.grid(patch, storage).unwrap();
            let (skirt, _, _) = pyramid.skirt(patch, storage, patch.skirt_depth).unwrap();
            let (xs, ys) = (patch.columns(), patch.rows());
            let node = |i: u32| (xs[i as usize % xs.len()], ys[i as usize / xs.len()]);
            // Exactly the triangles of three valid nodes are encoded.
            let all = triangles(xs.len(), ys.len()).count();
            let wanted = triangles(xs.len(), ys.len())
                .filter(|t| t.iter().all(|&i| valid(node(i as u32))))
                .count();
            let bytes = glb::encode(&grid, &skirt, storage, || Ok(())).unwrap();
            assert_eq!(
                decode_tile(&bytes).2.len(),
                3 * wanted + skirt.indices.len()
            );
            hidden += all - wanted;
            // A skirt segment has a triangle of valid nodes on both sides.
            let s = patch.stride;
            for quad in skirt.indices.chunks(6) {
                let [p, q] = [quad[0], quad[1]].map(node);
                let ((xa, ya), (xb, yb)) = (p.min(q), p.max(q));
                let own = match (xa == xb, xa == patch.x, ya == patch.y) {
                    (true, true, _) => ((xa + s).min(w), ya),
                    (true, false, _) => (xa - s, yb),
                    (false, _, true) => (xa, (ya + s).min(h)),
                    _ => (xb, ya - s),
                };
                assert!([p, q, own].into_iter().all(valid));
            }
            let segments = |k: usize| [ys.len(), xs.len()][k / 2] - 1;
            let edges = (0..4).filter(|&k| patch.skirt_depth[k] > 0.);
            open += edges.map(segments).sum::<usize>() - skirt.indices.len() / 6;
        }
        assert!(hidden > 0 && open > 0, "{hidden} {open}");
    }
    /// Valid columns 0..100 and a three-pixel strip 150..153: above the leaves
    /// no level keeps two adjacent node columns of the strip.
    fn strip() -> raster::PreparedRaster {
        raster::test_raster_with(260, 40, 1e-3, |c, r| {
            if c < 100 || (150..153).contains(&c) {
                100. + c as f64 + (r * c % 7) as f64
            } else {
                f64::NAN
            }
        })
    }
    #[test]
    fn a_tile_without_triangles_above_content_routes_by_its_extent() {
        let control = RunControl::default();
        let attempt = control.begin().unwrap();
        let r = strip();
        let frame = Frame::new(r.bounds);
        let plan = plan(&Pyramid::new(&r, 16, &frame, 0., &attempt)).unwrap();
        let find = |l, c| (plan.patches.iter()).find(|p| (p.level, p.column, p.row) == (l, c, 0));
        let (empty, leaf) = (find(4, 4).unwrap(), find(5, 9).unwrap());
        assert!(find(5, 8).is_none() && !empty.content && leaf.content);
        assert_eq!(empty.children.len(), 2);
        let extent = distance(leaf.surface[0], leaf.surface[1]);
        // The floor stays on the empty tile: ancestors and skirts see only errors
        // of tiles that draw.
        assert!(empty.error >= 16. * extent && plan.root().error < empty.error);
        let deepest = (plan.patches.iter())
            .flat_map(|p| p.skirt_depth)
            .fold(0., f64::max);
        assert!(deepest < empty.error / 8., "{deepest}");
    }
    /// A tile GLB read back through its own JSON, the meshopt decoders and the
    /// node's translation and scale: the decoded glTF positions and indices.
    fn decode_tile(bytes: &[u8]) -> (serde_json::Value, Vec<[f64; 3]>, Vec<u32>) {
        let json = u32::from_le_bytes(bytes[12..16].try_into().unwrap()) as usize;
        let document: serde_json::Value = serde_json::from_slice(&bytes[20..20 + json]).unwrap();
        let binary = &bytes[28 + json..];
        let stream = |view: usize| {
            let e = &document["bufferViews"][view]["extensions"]["EXT_meshopt_compression"];
            assert_eq!(e["buffer"], 0);
            let start = e["byteOffset"].as_u64().unwrap() as usize;
            &binary[start..start + e["byteLength"].as_u64().unwrap() as usize]
        };
        let primitive = &document["meshes"][0]["primitives"][0];
        let (position, index) = (
            &document["accessors"][primitive["attributes"]["POSITION"].as_u64().unwrap() as usize],
            &document["accessors"][primitive["indices"].as_u64().unwrap() as usize],
        );
        let (count, indices) = (
            position["count"].as_u64().unwrap() as usize,
            index["count"].as_u64().unwrap() as usize,
        );
        let node = &document["nodes"][0];
        let vector = |key: &str, default: f64| -> [f64; 3] {
            node.get(key).map_or([default; 3], |v| {
                std::array::from_fn(|i| v[i].as_f64().unwrap())
            })
        };
        let (translation, scale) = (vector("translation", 0.), vector("scale", 1.));
        // The only header bytes EXT_meshopt_compression admits for these modes.
        let (vertices, triangles) = (stream(0), stream(1));
        assert_eq!((vertices[0], triangles[0]), (0xa0, 0xe1));
        let positions: Vec<[f64; 3]> = match position["componentType"].as_u64().unwrap() {
            5123 => meshopt::encoding::decode_vertex_buffer::<[u16; 4]>(vertices, count)
                .unwrap()
                .iter()
                .map(|q| std::array::from_fn(|i| translation[i] + scale[i] * f64::from(q[i])))
                .collect(),
            5126 => meshopt::encoding::decode_vertex_buffer::<[f32; 3]>(vertices, count)
                .unwrap()
                .iter()
                .map(|p| std::array::from_fn(|i| translation[i] + scale[i] * f64::from(p[i])))
                .collect(),
            other => panic!("position component type {other}"),
        };
        let indices = meshopt::encoding::decode_index_buffer::<u16>(triangles, indices)
            .unwrap()
            .into_iter()
            .map(u32::from)
            .collect();
        (document, positions, indices)
    }
    #[test]
    fn tiles_are_one_meshopt_quantized_primitive_that_decodes_to_the_plan() {
        let control = RunControl::default();
        let attempt = control.begin().unwrap();
        // ~110 m pixels quantize every level; ~1.1 km pixels give leaves a
        // grid coarser than the storage limit, so only they stay Float32.
        let plane =
            raster::test_raster_with(33, 17, 1e-2, |c, r| 100. + 2. * c as f64 + 3. * r as f64);
        for (r, leaves_quantized) in [(bump(67, 45), true), (plane, false)] {
            let frame = Frame::new(r.bounds);
            let pyramid = Pyramid::new(&r, 16, &frame, 0., &attempt);
            let plan = plan(&pyramid).unwrap();
            for (level, storage) in plan.storage.iter().enumerate() {
                let quantized = matches!(storage, Storage::Quantized(_));
                assert_eq!(
                    quantized,
                    level < pyramid.depth as usize || leaves_quantized,
                    "{level} {storage:?}"
                );
            }
            for patch in &plan.patches {
                let storage = plan.storage(patch);
                let grid = pyramid.grid(patch, storage).unwrap();
                let (skirt, _, _) = pyramid.skirt(patch, storage, patch.skirt_depth).unwrap();
                let bytes = glb::encode(&grid, &skirt, storage, || Ok(())).unwrap();
                let (document, positions, indices) = decode_tile(&bytes);
                // One draw call: the skirt shares the surface primitive and material.
                assert_eq!(
                    document["meshes"][0]["primitives"]
                        .as_array()
                        .unwrap()
                        .len(),
                    1
                );
                assert_eq!(document["materials"].as_array().unwrap().len(), 1);
                let required = document["extensionsRequired"].as_array().unwrap();
                match storage {
                    Storage::Quantized(step) => {
                        assert_eq!(
                            required,
                            &["EXT_meshopt_compression", "KHR_mesh_quantization"]
                        );
                        assert_eq!(document["nodes"][0]["scale"], serde_json::json!(step));
                    }
                    _ => {
                        assert_eq!(required, &["EXT_meshopt_compression"]);
                        assert!(document["nodes"][0].get("scale").is_none());
                    }
                }
                let expected: Vec<_> = grid
                    .positions
                    .iter()
                    .chain(&skirt.bottoms)
                    .copied()
                    .collect();
                assert_eq!(positions, expected, "decoding is exact");
                // The index codec may rotate a triangle's corners, never its
                // winding or the triangle order.
                let rotated = |t: &[u32]| {
                    let k = (0..3).min_by_key(|&k| t[k]).unwrap();
                    [t[k], t[(k + 1) % 3], t[(k + 2) % 3]]
                };
                let columns = grid.columns as u32;
                let mut wanted = vec![];
                for y in 0..grid.rows as u32 - 1 {
                    for x in 0..columns - 1 {
                        let a = y * columns + x;
                        wanted.extend([a, a + 1, a + columns, a + 1, a + columns + 1, a + columns]);
                    }
                }
                wanted.extend(&skirt.indices);
                assert_eq!(indices.len(), wanted.len());
                for (found, wanted) in indices.chunks(3).zip(wanted.chunks(3)) {
                    assert_eq!(rotated(found), rotated(wanted));
                }
            }
        }
    }
    /// The frame point's geodetic height.
    fn geodetic_height(frame: &Frame, p: [f64; 3]) -> f64 {
        let ecef = std::array::from_fn(|i| {
            frame.origin[i]
                + (0..3)
                    .map(|axis| p[axis] * frame.axes[axis][i])
                    .sum::<f64>()
        });
        crate::georef::ecef_to_cartographic(ecef).unwrap().height_m
    }
    #[test]
    fn certification_holds_on_decoded_quantized_positions() {
        let control = RunControl::default();
        let attempt = control.begin().unwrap();
        let r = bump(67, 45);
        let frame = Frame::new(r.bounds);
        let mut pyramid = Pyramid::new(&r, 16, &frame, 0., &attempt);
        // A 10-bit grid: metre steps on coarse tiles, so certifying unrounded
        // positions would visibly understate their distance to the leaves.
        pyramid.quantization_steps = 1023.;
        let plan = plan(&pyramid).unwrap();
        let depth = pyramid.depth as usize;
        for (level, storage) in plan.storage.iter().enumerate() {
            match storage {
                Storage::Quantized(step) => {
                    assert!(level < depth && step[0] >= 1., "{level} {storage:?}")
                }
                _ => assert_eq!((level, *storage), (depth, Storage::Float)),
            }
        }
        // Every tile as a consumer decodes it from its GLB.
        let decoded: Vec<(Sheet, Vec<[f64; 3]>)> = plan
            .patches
            .iter()
            .map(|patch| {
                let storage = plan.storage(patch);
                let grid = pyramid.grid(patch, storage).unwrap();
                let (skirt, _, _) = pyramid.skirt(patch, storage, patch.skirt_depth).unwrap();
                let bytes = glb::encode(&grid, &skirt, storage, || Ok(())).unwrap();
                let (_, positions, _) = decode_tile(&bytes);
                let positions: Vec<_> = positions.into_iter().map(decoded).collect();
                let (xs, ys) = (patch.columns(), patch.rows());
                let surface = positions[..xs.len() * ys.len()]
                    .iter()
                    .map(|p| [p[0], p[2], -p[1]])
                    .collect();
                (
                    Sheet {
                        xs,
                        ys,
                        positions: surface,
                        used: grid.used,
                    },
                    positions,
                )
            })
            .collect();
        let parents = parents(&plan);
        let mut measured = vec![0f64; plan.patches.len()];
        for (id, patch) in plan.patches.iter().enumerate() {
            // Boxes contain the decoded surface and skirt of the tile and its descendants.
            let mut ancestors = vec![id];
            while let Some(&p) = parents.get(ancestors.last().unwrap()) {
                ancestors.push(p);
            }
            for &p in &decoded[id].1 {
                for &a in &ancestors {
                    assert!(inside(box_for(plan.patches[a].hierarchy), p));
                }
            }
            // Decoded skirt bottoms drop at least the shallowest unrounded
            // depth, max(e + e_beside, 1), below their tops.
            let surface = decoded[id].0.positions.len();
            let vertex = |i: usize| decoded[id].1[i];
            let storage = plan.storage(patch);
            let grid = pyramid.grid(patch, storage).unwrap();
            let (skirt, _, _) = pyramid.skirt(patch, storage, patch.skirt_depth).unwrap();
            let magnitude = (patch.surface.iter().flatten()).fold(0f64, |m, v| m.max(v.abs()));
            let least =
                (patch.skirt_depth.iter().filter(|d| **d > 0.)).fold(f64::MAX, |a, &b| a.min(b));
            let required = least - storage.margin(magnitude + least + 1.);
            for quad in skirt.indices.chunks(6) {
                let (top, bottom) = (quad[1] as usize, quad[4] as usize);
                assert!(top < surface && bottom >= surface && grid.positions.len() == surface);
                let drop =
                    geodetic_height(&frame, vertex(top)) - geodetic_height(&frame, vertex(bottom));
                assert!(required >= patch.error.max(1.) - 1e-9);
                assert!(
                    drop >= required - 1e-6,
                    "{drop} < {required} at level {} {storage:?}",
                    patch.level
                );
            }
            if patch.level as usize == depth {
                continue;
            }
            // Brute-force distance to the decoded leaves at the same lattice parameters.
            for (leaf, l) in plan.patches.iter().enumerate() {
                if l.level as usize != depth
                    || l.x < patch.x
                    || l.x1 > patch.x1
                    || l.y < patch.y
                    || l.y1 > patch.y1
                {
                    continue;
                }
                for j in 0..=(l.y1 - l.y) * 4 {
                    for i in 0..=(l.x1 - l.x) * 4 {
                        let (x, y) = (l.x as f64 + i as f64 / 4., l.y as f64 + j as f64 / 4.);
                        let d = distance(
                            decoded[id].0.at(x, y).unwrap(),
                            decoded[leaf].0.at(x, y).unwrap(),
                        );
                        measured[id] = measured[id].max(d);
                    }
                }
            }
            assert!(
                patch.error >= measured[id],
                "{} < {}",
                patch.error,
                measured[id]
            );
        }
        // Control: the same pyramid certified on unrounded positions understates
        // the decoded distance, so the checks above fail for that certification.
        let unrounded = plan_with(&pyramid, vec![Storage::Exact; depth + 1], 0).unwrap();
        let understated = plan
            .patches
            .iter()
            .zip(&unrounded.patches)
            .enumerate()
            .filter(|(id, (p, u))| {
                assert_eq!((p.level, p.column, p.row), (u.level, u.column, u.row));
                u.error < measured[*id] - 0.1
            })
            .count();
        assert!(understated > 0, "{measured:?}");
    }
    fn validated(directory: &Path) -> serde_json::Value {
        let archive = directory.with_extension("3tz");
        crate::package::package(
            crate::package::PackageRequest::directory(directory, &archive),
            &RunControl::default(),
        )
        .unwrap();
        serde_json::to_value(
            crate::validate::inspect(crate::validate::ValidationRequest::new(&archive)).unwrap(),
        )
        .unwrap()
    }
    #[test]
    fn forest_output_and_explicit_hierarchy_pass_validation() {
        let root = crate::runtime::directory::test_directory();
        let r = strip();
        let frame = Frame::new(r.bounds);
        let control = RunControl::default();
        let attempt = control.begin().unwrap();
        let pyramid = Pyramid::new(&r, 16, &frame, 0., &attempt);
        let plan = plan(&pyramid).unwrap();
        let mut report = plan_report(&r, 16, &plan, 0.).unwrap();
        let forest = root.path().join("forest");
        fs::create_dir(&forest).unwrap();
        produce(&forest, &pyramid, &plan, &mut report, write, seal).unwrap();
        let checks = validated(&forest)["checks"].clone();
        for check in [
            "implicitAddressingAndAvailability",
            "hierarchyBounds",
            "geometricErrorOrder",
        ] {
            assert!(
                checks.as_array().unwrap().contains(&check.into()),
                "{checks}"
            );
        }
        // The same H, explicit, is inside C2's domain: decoded skirts and surfaces
        // are proved inside every tile and ancestor volume.
        let control = RunControl::default();
        let attempt = control.begin().unwrap();
        let pyramid = Pyramid::new(&r, 16, &frame, 0., &attempt);
        let explicit = root.path().join("explicit");
        payloads(&explicit, &pyramid, &plan, &mut write).unwrap();
        let manifest = hierarchy(&plan, &frame, report.tileset_geometric_error_metres);
        fs::write(
            explicit.join("tileset.json"),
            serde_json::to_vec(&manifest).unwrap(),
        )
        .unwrap();
        let checks = validated(&explicit)["checks"].clone();
        for check in ["decodedContentContainment", "geometricErrorOrder"] {
            assert!(
                checks.as_array().unwrap().contains(&check.into()),
                "{checks}"
            );
        }
    }
}
