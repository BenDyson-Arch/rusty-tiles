//! Finite T1 raw-metre terrain meshes published as a 3D Tiles 1.1 directory.
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
    pub tiles: u64,
    pub vertices: u64,
    pub height_offset_metres: f64,
    pub fill_height_metres: f64,
    pub position_error_metres: f64,
    pub height_range: [f64; 2],
    pub generated_bytes: u64,
    pub surface_reference: &'static str,
    pub routing_error_metres: f64,
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
fn finish_member<W: Write>(
    w: &mut W,
    b: &[u8],
    sync: impl FnOnce(&mut W) -> std::io::Result<()>,
) -> std::io::Result<()> {
    w.write_all(b)?;
    w.flush()?;
    sync(w)
}
#[cfg(feature = "native-geospatial")]
fn write(path: &Path, b: &[u8], bytes: &mut u64) -> Result<(), JobError> {
    *bytes = bytes
        .checked_add(b.len() as u64)
        .ok_or_else(|| error(JobErrorKind::Unsupported, "terrain byte budget overflow"))?;
    if *bytes > 1 << 30 {
        return Err(error(
            JobErrorKind::Unsupported,
            "terrain output exceeds 1 GiB",
        ));
    }
    let mut f = fs::File::options()
        .write(true)
        .create_new(true)
        .open(path)
        .map_err(|e| JobError::io("create terrain member", path, e))?;
    finish_member(&mut f, b, |f| f.sync_all())
        .map_err(|e| JobError::io("finish terrain member", path, e))
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
#[derive(Clone)]
struct Leaf {
    x: usize,
    y: usize,
    x1: usize,
    y1: usize,
    name: String,
    bounds: [[f64; 3]; 2],
}
#[cfg(feature = "native-geospatial")]
fn grid(
    raster: &raster::PreparedRaster,
    leaf: &Leaf,
    frame: &Frame,
    offset: f64,
    fill: f64,
    attempt: &Attempt,
) -> Result<glb::Grid, JobError> {
    let columns = leaf.x1 - leaf.x + 1;
    let rows = leaf.y1 - leaf.y + 1;
    let mut positions = Vec::with_capacity(columns * rows);
    let mut bounds = [[f64::INFINITY; 3], [f64::NEG_INFINITY; 3]];
    let mut max_position_error: f64 = 0.;
    let mut height_range = [f64::INFINITY, f64::NEG_INFINITY];
    for y in leaf.y..=leaf.y1 {
        attempt.check()?;
        for x in leaf.x..=leaf.x1 {
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
            let height = raster.sample(lon, lat).map_or(fill, |h| h + offset);
            if !height.is_finite() {
                return Err(error(JobErrorKind::InvalidInput, "terrain offset overflow"));
            }
            if !(-10000.0..=8000.0).contains(&height) {
                return Err(error(JobErrorKind::Unsupported,"terrain sampled/filled height must be within -10000..8000 metres for supported scene queries"));
            }
            height_range[0] = height_range[0].min(height);
            height_range[1] = height_range[1].max(height);
            let p =
                crate::georef::geodetic_to_ecef(crate::georef::Cartographic::new(lon, lat, height));
            let local = frame.local(p);
            let encoded = [local[0] as f32, local[2] as f32, -local[1] as f32];
            if !encoded.iter().all(|v| v.is_finite()) {
                return Err(error(
                    JobErrorKind::Unsupported,
                    "terrain local positions exceed finite Float32",
                ));
            }
            let decoded = [
                f64::from(encoded[0]),
                -f64::from(encoded[2]),
                f64::from(encoded[1]),
            ];
            for axis in 0..3 {
                bounds[0][axis] = bounds[0][axis].min(decoded[axis]);
                bounds[1][axis] = bounds[1][axis].max(decoded[axis]);
            }
            let error = ((0..3)
                .map(|axis| (decoded[axis] - local[axis]).powi(2))
                .sum::<f64>())
            .sqrt();
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
            let longitude =
                raster.bounds[0] + (leaf.x + col) as f64 * raster.pixel[0] + raster.pixel[0] / 2.;
            let latitude =
                raster.bounds[1] + (leaf.y + row) as f64 * raster.pixel[1] + raster.pixel[1] / 2.;
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
#[cfg(feature = "native-geospatial")]
fn plan(
    raster: &raster::PreparedRaster,
    cells: u16,
    frame: &Frame,
    offset: f64,
    fill: f64,
    attempt: &Attempt,
) -> Result<(Vec<Leaf>, [[f64; 3]; 2], u64, f64, [f64; 2]), JobError> {
    let cells = usize::from(cells);
    let count = raster.width.div_ceil(cells) * raster.height.div_ceil(cells);
    // Uncompressed positions, indices, and conservative JSON overhead per leaf.
    let estimate = count as u64 * (32 * (cells as u64 + 1).pow(2) + 8192);
    if count > 10_000 || estimate > 1 << 30 {
        return Err(error(
            JobErrorKind::Unsupported,
            "terrain plan exceeds 10000 leaves or 1 GiB conservative output budget",
        ));
    }
    let mut leaves = Vec::with_capacity(count);
    let mut bounds = [[f64::INFINITY; 3], [f64::NEG_INFINITY; 3]];
    let mut vertices = 0;
    let mut maximum: f64 = 0.;
    let mut height_range = [f64::INFINITY, f64::NEG_INFINITY];
    for y in (0..raster.height).step_by(cells) {
        for x in (0..raster.width).step_by(cells) {
            let mut leaf = Leaf {
                x,
                y,
                x1: (x + cells).min(raster.width),
                y1: (y + cells).min(raster.height),
                name: format!("tiles/{}/{}.glb", y / cells, x / cells),
                bounds: [[0.; 3]; 2],
            };
            let g = grid(raster, &leaf, frame, offset, fill, attempt)?;
            leaf.bounds = g.bounds;
            let child_box = box_for(g.bounds);
            for axis in 0..3 {
                bounds[0][axis] =
                    bounds[0][axis].min(child_box[axis] - child_box[3 + axis * 3 + axis]);
                bounds[1][axis] =
                    bounds[1][axis].max(child_box[axis] + child_box[3 + axis * 3 + axis]);
            }
            vertices += g.positions.len() as u64;
            maximum = maximum.max(g.max_position_error);
            height_range[0] = height_range[0].min(g.height_range[0]);
            height_range[1] = height_range[1].max(g.height_range[1]);
            leaves.push(leaf)
        }
    }
    if maximum > 0.05 {
        return Err(error(
            JobErrorKind::Unsupported,
            "terrain Float32 position error exceeds 0.05 metres",
        ));
    }
    Ok((leaves, bounds, vertices, maximum, height_range))
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
        let (leaves, bounds, vertices, position_error, height_range) = plan(
            &raster,
            request.options.cells_per_leaf,
            &frame,
            offset,
            fill,
            &attempt,
        )
        .map_err(|e| attempt.fail(e))?;
        let diameter = ((0..3)
            .map(|i| (bounds[1][i] - bounds[0][i]).powi(2))
            .sum::<f64>())
        .sqrt()
        .next_up();
        if !diameter.is_finite() || diameter <= 0. {
            return Err(attempt.fail(error(
                JobErrorKind::Unsupported,
                "terrain routing extent must have a positive finite diameter",
            )));
        }
        let mut report = TerrainReport {
            schema_version: 1,
            profile: "t1-terrain-mesh-3dtiles-1.1",
            source_crs: raster.source_crs.clone(),
            source_bytes: raster.source_bytes,
            width: raster.width,
            height: raster.height,
            bounds: raster.bounds,
            source_pixel_degrees: raster.pixel,
            cells_per_leaf: request.options.cells_per_leaf,
            tiles: leaves.len() as u64,
            vertices,
            height_offset_metres: offset,
            fill_height_metres: fill,
            position_error_metres: position_error,
            height_range,
            generated_bytes: 0,
            surface_reference:
                "decoded Float32 triangle mesh; no certified bound on the continuous source surface",
            routing_error_metres: diameter,
        };
        let staging = target.stage(&attempt)?;
        if let Err(e) = produce(
            staging.path(),
            &raster,
            &leaves,
            &frame,
            bounds,
            offset,
            fill,
            &mut report,
            &attempt,
            write,
        ) {
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
struct Tile {
    bounding_volume: BoundingVolume,
    geometric_error: f64,
    #[serde(skip_serializing_if = "Option::is_none")]
    refine: Option<&'static str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    transform: Option<[f64; 16]>,
    #[serde(skip_serializing_if = "Option::is_none")]
    content: Option<Content>,
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
#[cfg(feature = "native-geospatial")]
#[allow(clippy::too_many_arguments)]
fn produce(
    path: &Path,
    raster: &raster::PreparedRaster,
    leaves: &[Leaf],
    frame: &Frame,
    bounds: [[f64; 3]; 2],
    offset: f64,
    fill: f64,
    report: &mut TerrainReport,
    attempt: &Attempt,
    mut write: impl FnMut(&Path, &[u8], &mut u64) -> Result<(), JobError>,
) -> Result<(), JobError> {
    let mut bytes = 0;
    let mut inventory = std::collections::BTreeSet::new();
    let mut children = Vec::with_capacity(leaves.len());
    for (i, leaf) in leaves.iter().enumerate() {
        attempt.check()?;
        let g = grid(raster, leaf, frame, offset, fill, attempt)?;
        if g.bounds != leaf.bounds {
            return Err(error(
                JobErrorKind::InvalidInput,
                "terrain plan changed during encoding",
            ));
        }
        let encoded = glb::encode(&g, || attempt.check())?;
        let destination = path.join(&leaf.name);
        fs::create_dir_all(destination.parent().unwrap())
            .map_err(|e| JobError::io("create terrain leaf parent", &destination, e))?;
        attempt.check()?;
        write(&destination, &encoded, &mut bytes)?;
        inventory.insert(leaf.name.clone());
        children.push(Tile {
            bounding_volume: BoundingVolume {
                r#box: box_for(leaf.bounds),
            },
            geometric_error: 0.,
            refine: None,
            transform: None,
            content: Some(Content {
                uri: leaf.name.clone(),
            }),
            children: vec![],
        });
        attempt.emit(&RunEvent::Progress {
            phase: "terrain",
            done: i as u64 + 1,
            total: Some(report.tiles),
        })?;
    }
    let manifest = Tileset {
        asset: Asset { version: "1.1" },
        geometric_error: report.routing_error_metres,
        root: Tile {
            bounding_volume: BoundingVolume {
                r#box: box_for(bounds),
            },
            geometric_error: report.routing_error_metres,
            refine: Some("REPLACE"),
            transform: Some(frame.transform()),
            content: None,
            children,
        },
    };
    attempt.check()?;
    write(
        &path.join("tileset.json"),
        &serde_json::to_vec_pretty(&manifest).unwrap(),
        &mut bytes,
    )?;
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
    fn report(
        r: &raster::PreparedRaster,
        cells: u16,
        leaves: &[Leaf],
        bounds: [[f64; 3]; 2],
        vertices: u64,
        error: f64,
        range: [f64; 2],
    ) -> TerrainReport {
        TerrainReport {
            schema_version: 1,
            profile: "t1-terrain-mesh-3dtiles-1.1",
            source_crs: r.source_crs.clone(),
            source_bytes: r.source_bytes,
            width: r.width,
            height: r.height,
            bounds: r.bounds,
            source_pixel_degrees: r.pixel,
            cells_per_leaf: cells,
            tiles: leaves.len() as u64,
            vertices,
            height_offset_metres: 0.,
            fill_height_metres: 0.,
            position_error_metres: error,
            height_range: range,
            generated_bytes: 0,
            surface_reference:
                "decoded Float32 triangle mesh; no certified bound on the continuous source surface",
            routing_error_metres: ((0..3)
                .map(|i| (bounds[1][i] - bounds[0][i]).powi(2))
                .sum::<f64>())
            .sqrt(),
        }
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
            let (leaves, bounds, vertices, error, range) =
                plan(&raster, 16, &frame, 0., 0., &attempt).unwrap();
            let mut report = report(&raster, 16, &leaves, bounds, vertices, error, range);
            let staging = DirectoryTarget::prepare(&output, OutputPolicy::Replace)
                .unwrap()
                .stage(&attempt)
                .unwrap();
            let work = staging.path().to_owned();
            let mut injected = false;
            let e = produce(
                staging.path(),
                &raster,
                &leaves,
                &frame,
                bounds,
                0.,
                0.,
                &mut report,
                &attempt,
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
            let (leaves, bounds, vertices, error, range) =
                plan(&raster, 16, &frame, 0., 0., &attempt).unwrap();
            let mut report = report(&raster, 16, &leaves, bounds, vertices, error, range);
            let staging = DirectoryTarget::prepare(&output, OutputPolicy::Replace)
                .unwrap()
                .stage(&attempt)
                .unwrap();
            let work = staging.path().to_owned();
            let e = produce(
                staging.path(),
                &raster,
                &leaves,
                &frame,
                bounds,
                0.,
                0.,
                &mut report,
                &attempt,
                write,
            )
            .unwrap_err();
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
        for fault in [1, 2, 3] {
            let mut writer = Writer {
                bytes: vec![],
                fault,
            };
            let mut synced = false;
            let e = finish_member(&mut writer, b"mesh", |_| {
                synced = true;
                if fault == 3 {
                    Err(std::io::Error::other("sync fault"))
                } else {
                    Ok(())
                }
            })
            .unwrap_err();
            assert!(e.to_string().contains("fault"));
            assert_eq!(synced, fault == 3);
        }
    }
    #[test]
    fn inadmissible_height_fails_in_preflight() {
        let control = RunControl::default();
        let attempt = control.begin().unwrap();
        let r = raster::test_raster();
        let frame = Frame::new(r.bounds);
        let e = plan(&r, 16, &frame, 9000., 0., &attempt).err().unwrap();
        assert_eq!(e.kind(), JobErrorKind::Unsupported);
        assert!(e.message().contains("-10000..8000"));
    }
}
