//! Finite T1 raw-metre terrain meshes published as a 3D Tiles 1.1 directory.
//!
//! Full-detail leaves are the T1 lattice patches. Coarser levels decimate the
//! same lattice (no new heights), each tile states a certified bound on its
//! distance to the leaf mesh, interior skirts cover cracks between levels, and
//! the hierarchy is published as the bounded implicit forest.
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
    RawMetres {
        height_offset_metres: f64,
        fill_height_metres: f64,
    },
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
    /// Surface vertices of every tile; skirt vertices are counted separately.
    pub vertices: u64,
    pub skirt_vertices: u64,
    pub height_offset_metres: f64,
    pub fill_height_metres: f64,
    pub position_error_metres: f64,
    pub height_range: [f64; 2],
    pub generated_bytes: u64,
    pub surface_reference: &'static str,
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
    /// West, east, south and north edges shared with a tile of the same level.
    interior: [bool; 4],
    children: Vec<usize>,
    /// Certified bound on the distance from this surface to the leaf surface.
    error: f64,
    skirt_depth: f64,
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
    fn skirted(&self) -> bool {
        self.interior.contains(&true)
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
    fill: f64,
) -> Result<f64, JobError> {
    let height = raster.sample(lon, lat).map_or(fill, |h| h + offset);
    if !height.is_finite() {
        return Err(error(JobErrorKind::InvalidInput, "terrain offset overflow"));
    }
    if !(-10000.0..=8000.0).contains(&height) {
        return Err(error(JobErrorKind::Unsupported,"terrain sampled/filled height must be within -10000..8000 metres for supported scene queries"));
    }
    Ok(height)
}
/// The decoded z-up frame position of a stored Y-up vertex.
#[cfg(feature = "native-geospatial")]
fn decoded(p: [f32; 3]) -> [f64; 3] {
    [f64::from(p[0]), -f64::from(p[2]), f64::from(p[1])]
}
/// Float32 storage of a geodetic point, its decoded position and storage error.
#[cfg(feature = "native-geospatial")]
fn place(
    frame: &Frame,
    lon: f64,
    lat: f64,
    height: f64,
) -> Result<([f32; 3], [f64; 3], f64), JobError> {
    let p = crate::georef::geodetic_to_ecef(crate::georef::Cartographic::new(lon, lat, height));
    let local = frame.local(p);
    let encoded = [local[0] as f32, local[2] as f32, -local[1] as f32];
    if !encoded.iter().all(|v| v.is_finite()) {
        return Err(error(
            JobErrorKind::Unsupported,
            "terrain local positions exceed finite Float32",
        ));
    }
    let position = decoded(encoded);
    let error = ((0..3)
        .map(|axis| (position[axis] - local[axis]).powi(2))
        .sum::<f64>())
    .sqrt();
    Ok((encoded, position, error))
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
    raster: &raster::PreparedRaster,
    xs: &[usize],
    ys: &[usize],
    frame: &Frame,
    offset: f64,
    fill: f64,
    attempt: &Attempt,
) -> Result<glb::Grid, JobError> {
    let columns = xs.len();
    let rows = ys.len();
    let mut positions = Vec::with_capacity(columns * rows);
    let mut bounds = [[f64::INFINITY; 3], [f64::NEG_INFINITY; 3]];
    let mut max_position_error: f64 = 0.;
    let mut height_range = [f64::INFINITY, f64::NEG_INFINITY];
    for &y in ys {
        attempt.check()?;
        for &x in xs {
            let (lon, lat) = coordinates(raster, x, y);
            let height = surface_height(raster, lon, lat, offset, fill)?;
            height_range[0] = height_range[0].min(height);
            height_range[1] = height_range[1].max(height);
            let (encoded, position, error) = place(frame, lon, lat, height)?;
            extend(&mut bounds, position);
            max_position_error = max_position_error.max(error);
            positions.push(encoded);
        }
    }
    // Reject topology that Float32 storage collapses or turns away from the
    // footprint's geodetic normal. Scene height queries require a surface.
    for row in 0..rows - 1 {
        attempt.check()?;
        for col in 0..columns - 1 {
            let i = row * columns + col;
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
            for [a, b, c] in [
                [i, i + 1, i + columns],
                [i + 1, i + columns + 1, i + columns],
            ] {
                if !triangle_is_surface(positions[a], positions[b], positions[c], up) {
                    return Err(error(
                        JobErrorKind::Unsupported,
                        "terrain Float32 topology collapses or reverses a footprint triangle",
                    ));
                }
            }
        }
    }
    Ok(glb::Grid {
        positions,
        columns,
        rows,
        bounds,
        max_position_error,
        height_range,
    })
}
#[cfg(feature = "native-geospatial")]
fn triangle_is_surface(a: [f32; 3], b: [f32; 3], c: [f32; 3], up: [f64; 3]) -> bool {
    let enu = |p: [f32; 3]| [f64::from(p[0]), -f64::from(p[2]), f64::from(p[1])];
    let (a, b, c) = (enu(a), enu(b), enu(c));
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
/// `(c+1,r),(c+1,r+1),(c,r+1)`.
#[cfg(feature = "native-geospatial")]
struct Sheet {
    xs: Vec<usize>,
    ys: Vec<usize>,
    positions: Vec<[f32; 3]>,
}
#[cfg(feature = "native-geospatial")]
impl Sheet {
    fn vertex(&self, c: usize, r: usize) -> [f64; 3] {
        decoded(self.positions[r * self.xs.len() + c])
    }
    fn cell(&self, c: usize, r: usize, x: f64, y: f64) -> [f64; 3] {
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
    fn at(&self, x: f64, y: f64) -> [f64; 3] {
        let index = |nodes: &[usize], v: f64| {
            nodes
                .partition_point(|&n| n as f64 <= v)
                .clamp(1, nodes.len() - 1)
                - 1
        };
        self.cell(index(&self.xs, x), index(&self.ys, y), x, y)
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
/// diagonals lie strictly on one side of it.
#[cfg(feature = "native-geospatial")]
fn deviation(patch: &Patch, sheet: &Sheet, children: &[(usize, Sheet)], cells: usize) -> f64 {
    let half = cells * patch.stride / 2;
    let mut worst: f64 = 0.;
    let mut points = Vec::with_capacity(16);
    for r in 0..sheet.ys.len() - 1 {
        for c in 0..sheet.xs.len() - 1 {
            let (xa, xb, ya, yb) = (sheet.xs[c], sheet.xs[c + 1], sheet.ys[r], sheet.ys[r + 1]);
            let slot = usize::from(xa >= patch.x + half) | usize::from(ya >= patch.y + half) << 1;
            let fine = &children
                .iter()
                .find(|(s, _)| *s == slot)
                .expect("a child covers every cell")
                .1;
            let within = |nodes: &[usize], a: usize, b: usize| {
                nodes.partition_point(|&n| n < a)..nodes.partition_point(|&n| n <= b)
            };
            let fx = &fine.xs[within(&fine.xs, xa, xb)];
            let fy = &fine.ys[within(&fine.ys, ya, yb)];
            points.clear();
            points.extend(
                fy.iter()
                    .flat_map(|&y| fx.iter().map(move |&x| (x as f64, y as f64))),
            );
            if xb - xa < patch.stride || yb - ya < patch.stride {
                let (w, h) = ((xb - xa) as f64, (yb - ya) as f64);
                // This tile's diagonal runs from (xb, ya) to (xa, yb).
                let (start, direction) = ((xb as f64, ya as f64), (-w, h));
                let on = |t: f64| (start.0 + t * direction.0, start.1 + t * direction.1);
                points.extend(fx[1..fx.len() - 1].iter().map(|&x| on((xb - x) as f64 / w)));
                points.extend(fy[1..fy.len() - 1].iter().map(|&y| on((y - ya) as f64 / h)));
            }
            for &(x, y) in &points {
                worst = worst.max(distance(fine.at(x, y), sheet.cell(c, r, x, y)));
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
    fill: f64,
    attempt: &'a Attempt,
}
#[cfg(feature = "native-geospatial")]
impl<'a> Pyramid<'a> {
    fn new(
        raster: &'a raster::PreparedRaster,
        cells: u16,
        frame: &'a Frame,
        offset: f64,
        fill: f64,
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
            fill,
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
            interior: [column > 0, x + span < width, row > 0, y + span < height],
            children: vec![],
            error: 0.,
            skirt_depth: 0.,
            surface: [[0.; 3]; 2],
            bounds: [[0.; 3]; 2],
            hierarchy: [[0.; 3]; 2],
        }
    }
    fn grid(&self, patch: &Patch) -> Result<glb::Grid, JobError> {
        grid(
            self.raster,
            &patch.columns(),
            &patch.rows(),
            self.frame,
            self.offset,
            self.fill,
            self.attempt,
        )
    }
    /// Plan one tile after its children (post-order), keeping only their
    /// surfaces while this tile's error is measured.
    fn visit(
        &self,
        level: u32,
        column: usize,
        row: usize,
        plan: &mut PreparedPlan,
    ) -> Result<(usize, Sheet), JobError> {
        let mut patch = self.patch(level, column, row);
        let mut children = Vec::with_capacity(4);
        if level < self.depth {
            let span = self.span(level + 1);
            for slot in 0..4 {
                let (c, r) = (2 * column + (slot & 1), 2 * row + (slot >> 1));
                if c * span < self.raster.width && r * span < self.raster.height {
                    let (id, sheet) = self.visit(level + 1, c, r, plan)?;
                    patch.children.push(id);
                    children.push((slot, sheet));
                }
            }
        }
        self.attempt.check()?;
        let g = self.grid(&patch)?;
        plan.vertices += g.positions.len() as u64;
        plan.position_error = plan.position_error.max(g.max_position_error);
        plan.height_range[0] = plan.height_range[0].min(g.height_range[0]);
        plan.height_range[1] = plan.height_range[1].max(g.height_range[1]);
        patch.surface = g.bounds;
        let sheet = Sheet {
            xs: patch.columns(),
            ys: patch.rows(),
            positions: g.positions,
        };
        if !children.is_empty() {
            // e(t) = d(t) + max e(child) bounds the distance to the leaf mesh.
            let below = patch
                .children
                .iter()
                .map(|&c| plan.patches[c].error)
                .fold(0., f64::max);
            let sum = deviation(&patch, &sheet, &children, self.cells) + below;
            patch.error = if sum > 0. { sum.next_up() } else { 0. };
        }
        plan.patches.push(patch);
        Ok((plan.patches.len() - 1, sheet))
    }
    /// Curtains from each interior edge node down to `height - depth`, wound
    /// to face away from the tile under its single-sided material.
    fn skirt(
        &self,
        patch: &Patch,
        depth: f64,
    ) -> Result<(glb::Skirt, [[f64; 3]; 2], f64), JobError> {
        let (xs, ys) = (patch.columns(), patch.rows());
        let (east, north) = (xs[xs.len() - 1], ys[ys.len() - 1]);
        let edges: [Vec<(usize, usize)>; 4] = [
            ys.iter().map(|&y| (xs[0], y)).collect(),
            ys.iter().rev().map(|&y| (east, y)).collect(),
            xs.iter().rev().map(|&x| (x, ys[0])).collect(),
            xs.iter().map(|&x| (x, north)).collect(),
        ];
        let mut skirt = glb::Skirt {
            positions: vec![],
            indices: vec![],
        };
        let mut bounds = patch.surface;
        let mut worst: f64 = 0.;
        for (nodes, _) in edges.iter().zip(patch.interior).filter(|(_, i)| *i) {
            self.attempt.check()?;
            for (k, &(x, y)) in nodes.iter().enumerate() {
                let (lon, lat) = coordinates(self.raster, x, y);
                let height = surface_height(self.raster, lon, lat, self.offset, self.fill)?;
                let (top, _, _) = place(self.frame, lon, lat, height)?;
                let (bottom, position, error) = place(self.frame, lon, lat, height - depth)?;
                extend(&mut bounds, position);
                worst = worst.max(error);
                let base = skirt.positions.len() as u16;
                skirt.positions.extend([top, bottom]);
                if k > 0 {
                    let b = base - 2;
                    skirt.indices.extend([b, b + 2, b + 1, b + 2, b + 3, b + 1]);
                }
            }
        }
        Ok((skirt, bounds, worst))
    }
}
#[cfg(feature = "native-geospatial")]
struct PreparedPlan {
    /// Post-order: children precede parents and the root is last.
    patches: Vec<Patch>,
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
    let mut plan = PreparedPlan {
        patches: Vec::with_capacity(tiles),
        vertices: 0,
        skirt_vertices: 0,
        position_error: 0.,
        height_range: [f64::INFINITY, f64::NEG_INFINITY],
    };
    pyramid.visit(0, 0, 0, &mut plan)?;
    // Neighbours at different levels each lie within their errors of the leaf
    // edge, so a T-junction gap is at most e(t) + e(root).
    let root_error = plan.root().error;
    for id in 0..plan.patches.len() {
        let depth = (plan.patches[id].error + root_error).max(1.);
        let (skirt, bounds, error) = pyramid.skirt(&plan.patches[id], depth)?;
        plan.skirt_vertices += skirt.positions.len() as u64;
        plan.position_error = plan.position_error.max(error);
        let mut hierarchy = bounds;
        for &child in &plan.patches[id].children {
            let b = box_for(plan.patches[child].hierarchy);
            extend(&mut hierarchy, std::array::from_fn(|i| b[i] - b[3 + i * 4]));
            extend(&mut hierarchy, std::array::from_fn(|i| b[i] + b[3 + i * 4]));
        }
        let patch = &mut plan.patches[id];
        patch.skirt_depth = depth;
        patch.bounds = bounds;
        patch.hierarchy = hierarchy;
    }
    if plan.position_error > 0.05 {
        return Err(error(
            JobErrorKind::Unsupported,
            "terrain Float32 position error exceeds 0.05 metres",
        ));
    }
    Ok(plan)
}
#[cfg(feature = "native-geospatial")]
fn plan_report(
    raster: &raster::PreparedRaster,
    cells: u16,
    plan: &PreparedPlan,
    offset: f64,
    fill: f64,
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
        if patch.skirted() {
            let [low, high] = skirts.unwrap_or([f64::INFINITY, f64::NEG_INFINITY]);
            skirts = Some([low.min(patch.skirt_depth), high.max(patch.skirt_depth)]);
        }
    }
    Ok(TerrainReport {
        schema_version: 2,
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
        fill_height_metres: fill,
        position_error_metres: plan.position_error,
        height_range: plan.height_range,
        generated_bytes: 0,
        surface_reference:
            "decoded Float32 triangle mesh; no certified bound on the continuous source surface",
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
        fill_height_metres: fill,
    } = request.heights;
    if request.input.as_os_str().is_empty()
        || request.output.as_os_str().is_empty()
        || ![16, 32, 64, 128].contains(&request.options.cells_per_leaf)
        || !offset.is_finite()
        || !fill.is_finite()
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
        let pyramid = Pyramid::new(&raster, cells, &frame, offset, fill, &attempt);
        let planned =
            plan(&pyramid).and_then(|p| Ok((plan_report(&raster, cells, &p, offset, fill)?, p)));
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
    content: Content,
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
/// The explicit hierarchy H: every tile has content, children state their
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
            content: Content { uri: patch.name() },
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
        let g = pyramid.grid(patch)?;
        let (skirt, bounds, _) = pyramid.skirt(patch, patch.skirt_depth)?;
        if g.bounds != patch.surface || bounds != patch.bounds {
            return Err(error(
                JobErrorKind::InvalidInput,
                "terrain plan changed during encoding",
            ));
        }
        let encoded = glb::encode(&g, &skirt, || attempt.check())?;
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
    /// The relief with a NoData block, filled 100 m below it: a fill cliff.
    fn holed(width: usize, height: usize) -> raster::PreparedRaster {
        raster::test_raster_with(width, height, 1e-3, |c, r| {
            if (20..30).contains(&c) && (10..18).contains(&r) {
                f64::NAN
            } else {
                300. + 40. * (c as f64 / 5.).sin() * (r as f64 / 7.).cos() + 0.5 * c as f64
            }
        })
    }
    fn sheet(pyramid: &Pyramid, patch: &Patch) -> Sheet {
        Sheet {
            xs: patch.columns(),
            ys: patch.rows(),
            positions: pyramid.grid(patch).unwrap().positions,
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
            let pyramid = Pyramid::new(&raster, 16, &frame, 0., 0., &attempt);
            let plan = plan(&pyramid).unwrap();
            let mut report = plan_report(&raster, 16, &plan, 0., 0.).unwrap();
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
        let pyramid = Pyramid::new(&raster, 16, &frame, 0., 0., &attempt);
        let plan = plan(&pyramid).unwrap();
        assert_eq!(pyramid.depth, 5);
        let mut report = plan_report(&raster, 16, &plan, 0., 0.).unwrap();
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
        let pyramid = Pyramid::new(&raster, 16, &frame, 0., 0., &attempt);
        let mut report = plan_report(&raster, 16, &plan, 0., 0.).unwrap();
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
            let pyramid = Pyramid::new(&raster, 16, &frame, 0., 0., &attempt);
            let plan = plan(&pyramid).unwrap();
            let mut report = plan_report(&raster, 16, &plan, 0., 0.).unwrap();
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
        let e = plan(&Pyramid::new(&r, 16, &frame, 9000., 0., &attempt))
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
            let pyramid = Pyramid::new(&r, 16, &frame, 0., 0., &attempt);
            let plan = plan(&pyramid).unwrap();
            assert_eq!(pyramid.depth, depth);
            let report = plan_report(&r, 16, &plan, 0., 0.).unwrap();
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
                    patch.interior,
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
            0.,
            &attempt,
        ))
        .unwrap();
        assert_eq!(single.patches.len(), 1);
        assert_eq!((single.root().error, single.skirt_vertices), (0., 0));
        let r = raster::test_raster();
        assert_eq!(
            plan_report(&r, 16, &single, 0., 0.)
                .unwrap()
                .skirt_depth_metres,
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
                .map(|(x, y)| [x as f32, h(x, y) as f32, -(y as f32)])
                .collect(),
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
            interior: [false; 4],
            children: vec![],
            error: 0.,
            skirt_depth: 0.,
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
                worst = worst.max(distance(fine.at(x, y), sheet.at(x, y)));
            }
        }
        worst
    }
    #[test]
    fn deviation_is_exact_on_planes_and_includes_clipped_diagonal_crossings() {
        let plane = |x: usize, y: usize| 2. * x as f64 + 3. * y as f64 + 5.;
        let (patch, sheet, children) = synthetic(8, 8, 4, 2, plane, |_, x, y| plane(x, y));
        assert_eq!(deviation(&patch, &sheet, &children, 4), 0.);
        // Full-stride cells: the bound is the vertex maximum and equals the dense maximum.
        let wavy = |x: usize, y: usize| ((x * 7 + y * 13) % 5) as f64;
        let (patch, sheet, children) = synthetic(8, 8, 4, 2, wavy, |_, x, y| wavy(x, y));
        let d = deviation(&patch, &sheet, &children, 4);
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
            coarse.at(x as f64, y as f64)[2]
        });
        let vertices = children
            .iter()
            .flat_map(|(_, fine)| {
                fine.ys
                    .iter()
                    .flat_map(|&y| fine.xs.iter().map(move |&x| (x as f64, y as f64)))
                    .map(|(x, y)| distance(fine.at(x, y), sheet.at(x, y)))
            })
            .fold(0., f64::max);
        assert_eq!(vertices, 0., "a vertex-only bound would certify zero");
        let d = deviation(&patch, &sheet, &children, 2);
        assert!((d - 5.).abs() <= 1e-9, "{d}");
        assert!((d - dense(&sheet, &children, 2)).abs() <= 1e-9);
        // The transposed saddle in [0, 2] x [2, 3]: only the crossing with the
        // child column x = 1, at y = 2.5, sees it.
        let saddle = |x: usize, y: usize| match (x, y) {
            (0, 2) | (2, 3) => 10.,
            _ => 0.,
        };
        let (patch, sheet, children) = synthetic(4, 3, 2, 2, saddle, |coarse, x, y| {
            coarse.at(x as f64, y as f64)[2]
        });
        let d = deviation(&patch, &sheet, &children, 2);
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
                        vertices =
                            vertices.max(distance(fine.at(p.0, p.1), sheet.cell(c, r, p.0, p.1)));
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
                    worst = worst.max(distance(fine.at(x, y), sheet.cell(c, r, x, y)));
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
                            let base = coarse.at(x as f64, y as f64)[2];
                            if node(x, width) && node(y, depth) {
                                base
                            } else {
                                base + 0.05 * height(y, x)
                            }
                        });
                    let d = deviation(&patch, &sheet, &children, 2);
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
    #[test]
    fn certified_errors_bound_dense_leaf_distance_and_are_monotone() {
        let control = RunControl::default();
        let attempt = control.begin().unwrap();
        // Even dimensions keep every cell full-stride; odd ones clip end strips;
        // NoData filled at 200 m adds a cliff.
        for (r, full_stride, fill) in [
            (bump(64, 48), true, 0.),
            (bump(67, 45), false, 0.),
            (holed(67, 45), false, 200.),
        ] {
            let frame = Frame::new(r.bounds);
            let pyramid = Pyramid::new(&r, 16, &frame, 0., fill, &attempt);
            let plan = plan(&pyramid).unwrap();
            let sheets: Vec<_> = plan.patches.iter().map(|p| sheet(&pyramid, p)).collect();
            let leaves: Vec<_> = (0..plan.patches.len())
                .filter(|&i| plan.patches[i].level == pyramid.depth)
                .collect();
            for (id, patch) in plan.patches.iter().enumerate() {
                if patch.level == pyramid.depth {
                    assert_eq!(patch.error, 0.);
                    continue;
                }
                for &child in &patch.children {
                    assert!(patch.error >= plan.patches[child].error);
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
                            measured =
                                measured.max(distance(sheets[id].at(x, y), sheets[leaf].at(x, y)));
                        }
                    }
                }
                assert!(
                    measured > 0.1,
                    "the fixture must exercise level {}",
                    patch.level
                );
                assert!(patch.error >= measured, "{} < {measured}", patch.error);
                if full_stride && patch.level + 1 == pyramid.depth {
                    assert!(
                        patch.error <= measured + 1e-9,
                        "{} > {measured}",
                        patch.error
                    );
                }
            }
            let report = plan_report(&r, 16, &plan, 0., 0.).unwrap();
            assert!(report
                .geometric_error_metres
                .windows(2)
                .all(|w| w[0] >= w[1]));
            assert_eq!(report.root_geometric_error_metres, plan.root().error);
        }
        // A constant height only leaves the Earth-curvature sag of decimated chords.
        let r = raster::test_raster_with(64, 64, 1e-4, |_, _| 120.);
        let frame = Frame::new(r.bounds);
        let plan = plan(&Pyramid::new(&r, 16, &frame, 0., 0., &attempt)).unwrap();
        assert!(plan.root().error < 1e-3, "{}", plan.root().error);
    }
    #[test]
    fn skirts_cover_every_mixed_level_seam_and_face_outward() {
        let control = RunControl::default();
        let attempt = control.begin().unwrap();
        for (r, fill) in [(bump(67, 45), 0.), (holed(67, 45), 200.)] {
            let frame = Frame::new(r.bounds);
            let pyramid = Pyramid::new(&r, 16, &frame, 0., fill, &attempt);
            let plan = plan(&pyramid).unwrap();
            let root = plan.root().error;
            let sheets: Vec<_> = plan.patches.iter().map(|p| sheet(&pyramid, p)).collect();
            for patch in &plan.patches {
                assert!(patch.skirt_depth >= patch.error + root && patch.skirt_depth >= 1.);
                let (skirt, _, _) = pyramid.skirt(patch, patch.skirt_depth).unwrap();
                let surface = pyramid.grid(patch).unwrap().positions;
                let edges = patch.interior.iter().filter(|i| **i).count();
                assert_eq!(skirt.indices.is_empty(), edges == 0);
                let centre: [f64; 3] =
                    std::array::from_fn(|i| (patch.surface[0][i] + patch.surface[1][i]) / 2.);
                for pair in skirt.positions.chunks(2) {
                    assert!(
                        surface.contains(&pair[0]),
                        "skirt tops are surface vertices"
                    );
                    let drop = distance(decoded(pair[0]), decoded(pair[1]));
                    assert!((drop - patch.skirt_depth).abs() <= 0.1, "{drop}");
                }
                for t in skirt.indices.chunks(3) {
                    let [a, b, c] = [0, 1, 2].map(|k| decoded(skirt.positions[t[k] as usize]));
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
            // the shared edge is covered by the skirt hanging from either side.
            let mut worst: f64 = 0.;
            let mut pairs = 0;
            for (i, a) in plan.patches.iter().enumerate() {
                for (j, b) in plan.patches.iter().enumerate() {
                    let overlap = |a0: usize, a1: usize, b0: usize, b1: usize| {
                        (a0.max(b0) < a1.min(b1)).then(|| (a0.max(b0), a1.min(b1)))
                    };
                    let segment = if a.x1 == b.x {
                        overlap(a.y, a.y1, b.y, b.y1).map(|(y0, y1)| {
                            assert!(a.interior[1] && b.interior[0]);
                            ((a.x1, y0), (a.x1, y1))
                        })
                    } else if a.y1 == b.y {
                        overlap(a.x, a.x1, b.x, b.x1).map(|(x0, x1)| {
                            assert!(a.interior[3] && b.interior[2]);
                            ((x0, a.y1), (x1, a.y1))
                        })
                    } else {
                        None
                    };
                    let Some(((x0, y0), (x1, y1))) = segment else {
                        continue;
                    };
                    pairs += 1;
                    let steps = 4 * (x1 - x0 + y1 - y0);
                    for k in 0..=steps {
                        let t = k as f64 / steps as f64;
                        let (x, y) = (
                            x0 as f64 + t * (x1 - x0) as f64,
                            y0 as f64 + t * (y1 - y0) as f64,
                        );
                        let gap = distance(sheets[i].at(x, y), sheets[j].at(x, y));
                        worst = worst.max(gap);
                        // Both edges lie within their errors of the leaf edge.
                        assert!(gap <= a.error + b.error + 1e-9, "{gap}");
                        assert!(gap <= a.skirt_depth.min(b.skirt_depth), "{gap}");
                    }
                }
            }
            assert!(pairs > 20 && worst > 0.1, "{pairs} {worst}");
        }
    }
    #[test]
    fn every_box_contains_its_own_and_all_descendant_surface_and_skirt_vertices() {
        let control = RunControl::default();
        let attempt = control.begin().unwrap();
        let r = bump(67, 45);
        let frame = Frame::new(r.bounds);
        let pyramid = Pyramid::new(&r, 16, &frame, 0., 0., &attempt);
        let plan = plan(&pyramid).unwrap();
        let parents = parents(&plan);
        for (id, patch) in plan.patches.iter().enumerate() {
            let (skirt, _, _) = pyramid.skirt(patch, patch.skirt_depth).unwrap();
            let grid = pyramid.grid(patch).unwrap();
            let mut ancestors = vec![id];
            while let Some(&p) = parents.get(ancestors.last().unwrap()) {
                ancestors.push(p);
            }
            assert_eq!(*ancestors.last().unwrap(), plan.patches.len() - 1);
            for p in grid.positions.iter().chain(&skirt.positions) {
                for &a in &ancestors {
                    assert!(inside(box_for(plan.patches[a].hierarchy), decoded(*p)));
                }
            }
        }
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
        let r = bump(260, 40);
        let frame = Frame::new(r.bounds);
        let control = RunControl::default();
        let attempt = control.begin().unwrap();
        let pyramid = Pyramid::new(&r, 16, &frame, 0., 0., &attempt);
        let plan = plan(&pyramid).unwrap();
        let mut report = plan_report(&r, 16, &plan, 0., 0.).unwrap();
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
        let pyramid = Pyramid::new(&r, 16, &frame, 0., 0., &attempt);
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
