//! Declared source CRS of a mesh, applied at capture (H1) in f64. Each post-node source
//! position plus the source offset maps exactly, per vertex, through the shared conservative
//! resolver to WGS84 ECEF and then into one local ENU frame (glTF Y-up) at the source-bounds
//! centre. Normals use the Jacobian inverse transpose and tangents the Jacobian, then are
//! re-orthonormalized. Jacobians come from the legacy bake's central differences, evaluated
//! on an adaptive lattice over the source bounds and interpolated; a cell whose centre check
//! exceeds the tolerance uses per-vertex Jacobians. Every later stage sees local metres
//! only; the root transform carries ENU to ECEF.
use crate::{
    crs::Transform,
    georef::{ecef_to_cartographic, EnuFrame, SourceAxes, SourceOffset},
    gltf_source::source::within_local_domain,
    mesh_archive::ResolvedPlacement,
    mesh_crs, JobError, JobErrorKind, MeshPlacement,
};
use serde::Serialize;
use std::{io::Read, mem::Discriminant, path::Path};

type Result<T> = std::result::Result<T, JobError>;
type Columns = [[f64; 3]; 3];

/// Legacy adapter keywords and bare codes are refused: name the CRS explicitly.
const LEGACY_KEYWORDS: [&str; 9] = [
    "auto",
    "geographic",
    "wgs84",
    "webmercator",
    "mercator",
    "pseudo-mercator",
    "pseudomercator",
    "4326",
    "3857",
];
/// Offset files larger than this are refused before parsing.
const OFFSET_FILE_BYTES: u64 = 4096;

fn invalid(message: impl Into<String>) -> JobError {
    JobError::new(JobErrorKind::InvalidRequest, message)
}

/// A declared horizontal source CRS with axes, source offset and height decision.
/// Without it (`local`) the mesh is local metres with glTF Y up.
#[derive(Clone, Debug, PartialEq)]
pub struct MeshSourceCrs {
    definition: String,
    axes: SourceAxes,
    offset: SourceOffset,
    height_offset_metres: f64,
}

/// Parse a Metashape-style offset file: exactly the three lines `E: <number>`,
/// `N: <number>` and `A: <number>` in that order, each value finite, with an optional final
/// line break (LF or CRLF). Anything else is an `invalid_request`.
pub fn parse_offset_file(text: &str) -> Result<[f64; 3]> {
    let bad = || {
        invalid("source offset file must be exactly the lines \"E: <number>\", \"N: <number>\", \"A: <number>\"")
    };
    let body = text
        .strip_suffix("\r\n")
        .or_else(|| text.strip_suffix('\n'))
        .unwrap_or(text);
    let lines: Vec<&str> = body.split('\n').collect();
    if lines.len() != 3 {
        return Err(bad());
    }
    let mut values = [0.; 3];
    for ((line, key), value) in lines.iter().zip(["E: ", "N: ", "A: "]).zip(&mut values) {
        let line = line.strip_suffix('\r').unwrap_or(line);
        let number = line.strip_prefix(key).ok_or_else(bad)?;
        *value = number
            .parse::<f64>()
            .ok()
            .filter(|v| v.is_finite())
            .ok_or_else(bad)?;
    }
    Ok(values)
}

fn read_offset_file(path: &Path) -> Result<[f64; 3]> {
    let mut bytes = Vec::new();
    std::fs::File::open(path)
        .and_then(|file| file.take(OFFSET_FILE_BYTES + 1).read_to_end(&mut bytes))
        .map_err(|error| JobError::io("read source offset file", path, error))?;
    if bytes.len() as u64 > OFFSET_FILE_BYTES {
        return Err(invalid("source offset file exceeds 4 KiB"));
    }
    // Content, not I/O: a non-UTF-8 file is a malformed request like any other bad line.
    let text =
        std::str::from_utf8(&bytes).map_err(|_| invalid("source offset file is not UTF-8 text"))?;
    parse_offset_file(text)
}

impl MeshSourceCrs {
    /// Shared adapter grammar. `None` or `local` keeps local coordinates and admits no
    /// other source-CRS parameter. Any other definition takes axes (default `y-up`, as glTF
    /// is Y up), a height offset in metres (default 0) and at most one of an `E N [A]`
    /// offset (A defaults to zero) and a Metashape-style offset file.
    pub fn from_parameters(
        source_crs: Option<&str>,
        axes: Option<SourceAxes>,
        offset: Option<&[f64]>,
        offset_file: Option<&Path>,
        height_offset_metres: Option<f64>,
    ) -> Result<Option<Self>> {
        let definition = source_crs
            .map(str::trim)
            .filter(|d| !d.eq_ignore_ascii_case("local"));
        let Some(definition) = definition else {
            if axes.is_some()
                || offset.is_some()
                || offset_file.is_some()
                || height_offset_metres.is_some()
            {
                return Err(invalid(
                    "source axes, source offset, offset file and height offset require a source CRS",
                ));
            }
            return Ok(None);
        };
        if LEGACY_KEYWORDS.contains(&definition.to_ascii_lowercase().as_str()) {
            return Err(invalid(
                "source CRS must be an EPSG:<code>, WKT or PROJ definition or local; legacy auto/geographic/Web Mercator keywords are not inferred",
            ));
        }
        let height_offset_metres = height_offset_metres.unwrap_or(0.);
        let offset = match (offset, offset_file) {
            (Some(_), Some(_)) => {
                return Err(invalid(
                    "pass only one of a source offset and a source offset file",
                ))
            }
            (None, Some(path)) => read_offset_file(path)?,
            (offset, None) => match offset.unwrap_or(&[0., 0.]) {
                [easting, northing] => [*easting, *northing, 0.],
                [easting, northing, height] => [*easting, *northing, *height],
                _ => return Err(invalid("source offset takes E N or E N A")),
            },
        };
        if !offset
            .iter()
            .chain([&height_offset_metres])
            .all(|v| v.is_finite())
        {
            return Err(invalid("source offset and height offset must be finite"));
        }
        Ok(Some(Self {
            definition: definition.to_owned(),
            axes: axes.unwrap_or(SourceAxes::YUp),
            offset: SourceOffset {
                easting: offset[0],
                northing: offset[1],
                height: offset[2],
            },
            height_offset_metres,
        }))
    }

    /// Resolve the definition without source I/O; a rejected or native-only CRS fails here.
    pub(crate) fn check(&self) -> Result<()> {
        self.operation().map(drop)
    }

    fn operation(&self) -> Result<Transform> {
        Transform::new(&self.definition, self.height_offset_metres).map_err(|error| match error {
            crate::Error::Environment(message) => JobError::new(JobErrorKind::Unsupported, message),
            error => invalid(format!("source CRS: {error}")),
        })
    }
}

/// The resolved source CRS, stated in the report and conversion.json.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct MeshSourceCrsReport {
    pub definition: String,
    /// `xyz` (E, N, height) or `y-up` (E, height, -N), after node transforms.
    pub axes: &'static str,
    /// E/N in horizontal CRS units and A in metres, added in f64.
    pub source_offset: [f64; 3],
    pub height_offset_metres: f64,
    /// `portable` (pure Rust) or `native` (strict PROJ), fixed for the whole capture.
    pub operation: &'static str,
    /// The local ENU origin: the mapped centre of the used post-node source bounds.
    pub origin_degrees_metres: [f64; 3],
    /// Jacobian lattice cells per post-node source axis.
    pub jacobian_lattice_cells: [u64; 3],
    /// Lattice cells that use per-vertex Jacobians (failed centre check or domain edge).
    pub jacobian_exact_cells: u64,
}

/// Per-vertex Jacobian columns d(local Y-up)/d(post-node source axis).
#[derive(Clone, Copy, Debug)]
pub(crate) struct Jacobian(pub(crate) Columns);

impl Jacobian {
    /// Unnormalized inverse-transpose image of a normal.
    pub(crate) fn normal(&self, normal: [f64; 3]) -> Result<[f64; 3]> {
        mesh_crs::inverse_transpose(self.0, normal).map_err(|_| {
            JobError::new(
                JobErrorKind::Unsupported,
                "source CRS normal Jacobian is singular or nonfinite",
            )
        })
    }
    /// Unnormalized image of a tangent direction.
    pub(crate) fn tangent(&self, tangent: [f64; 3]) -> [f64; 3] {
        std::array::from_fn(|i| (0..3).map(|j| self.0[j][i] * tangent[j]).sum())
    }
    /// The sign of the determinant: a reflection flips tangent handedness.
    pub(crate) fn handedness(&self) -> f64 {
        let [a, b, c] = self.0;
        let cross = [
            b[1] * c[2] - b[2] * c[1],
            b[2] * c[0] - b[0] * c[2],
            b[0] * c[1] - b[1] * c[0],
        ];
        (0..3).map(|i| a[i] * cross[i]).sum::<f64>().signum()
    }
}

/// At most this many lattice cells per axis; each nominally spans 1000 probe steps
/// (1 km horizontally in metre units, 0.01 degree geographic, 100 m vertically).
const LATTICE_CELLS: usize = 32;
const LATTICE_PROBE_STEPS: f64 = 1000.;
/// Largest relative Frobenius error of the interpolated Jacobian at a cell centre. The
/// normal error that follows is about this times the Jacobian's condition number `cond(J)`
/// (for EPSG:3857 about `1/cos(latitude)`, the horizontal-to-vertical scale ratio), so roughly
/// 2e-7 rad at 60 degrees. The centre check estimates the quadratic (curvature) term of the
/// interpolation error, which dominates for smooth projections; it is not a proof for higher
/// order terms. Measured normal errors are given in the tests.
const LATTICE_TOLERANCE: f64 = 1e-7;

/// Exact Jacobians at the nodes of a regular lattice over the used post-node source bounds,
/// trilinearly interpolated inside cells that pass their centre check.
#[derive(Default)]
struct Lattice {
    min: [f64; 3],
    max: [f64; 3],
    cell: [f64; 3],
    cells: [usize; 3],
    nodes: Vec<Option<Columns>>,
    exact: Vec<bool>,
}

impl Lattice {
    fn coordinate(&self, axis: usize, k: usize) -> f64 {
        if k == self.cells[axis] {
            self.max[axis]
        } else {
            self.min[axis] + k as f64 * self.cell[axis]
        }
    }
    fn node(&self, k: [usize; 3]) -> usize {
        (k[2] * (self.cells[1] + 1) + k[1]) * (self.cells[0] + 1) + k[0]
    }
    fn cell_index(&self, k: [usize; 3]) -> usize {
        (k[2] * self.cells[1] + k[1]) * self.cells[0] + k[0]
    }
    /// The cell holding `world` and its fractional position, both clamped to the lattice.
    fn locate(&self, world: [f64; 3]) -> ([usize; 3], [f64; 3]) {
        let mut cell = [0; 3];
        let mut t = [0.; 3];
        for axis in 0..3 {
            if self.cell[axis] > 0. {
                let u = (world[axis] - self.min[axis]) / self.cell[axis];
                let k = u.floor().clamp(0., (self.cells[axis] - 1) as f64);
                cell[axis] = k as usize;
                t[axis] = (u - k).clamp(0., 1.);
            }
        }
        (cell, t)
    }
    /// Trilinear interpolation in a cell, or `None` when a corner node is unavailable.
    fn interpolate(&self, cell: [usize; 3], t: [f64; 3]) -> Option<Columns> {
        let mut out = [[0.; 3]; 3];
        for corner in 0..8 {
            let bit = |axis: usize| (corner >> axis) & 1;
            let weight = (0..3)
                .map(|axis| {
                    if bit(axis) == 1 {
                        t[axis]
                    } else {
                        1. - t[axis]
                    }
                })
                .product::<f64>();
            let node = self.nodes[self.node(std::array::from_fn(|axis| cell[axis] + bit(axis)))]?;
            for (column, value) in out.iter_mut().zip(node) {
                for (o, v) in column.iter_mut().zip(value) {
                    *o += weight * v;
                }
            }
        }
        Some(out)
    }
}

pub(crate) struct CrsFrame {
    operation: Transform,
    kind: Discriminant<Transform>,
    axes: SourceAxes,
    offset: SourceOffset,
    horizontal_step: f64,
    degrees_per_unit: Option<f64>,
    frame: EnuFrame,
    placement: ResolvedPlacement,
    report: MeshSourceCrsReport,
    lattice: Lattice,
}

/// Capture's coordinate frame: identity rounding, or a resolved source CRS.
pub(crate) enum SourceFrame {
    Local,
    Crs(Box<CrsFrame>),
}

fn source_error(error: crate::Error) -> JobError {
    match error {
        crate::Error::Environment(message) => JobError::new(JobErrorKind::Unsupported, message),
        error => JobError::new(
            JobErrorKind::InvalidInput,
            format!("source position outside the source CRS domain: {error}"),
        ),
    }
}

impl SourceFrame {
    /// Resolve the frame once from the used post-node source bounds `[min, max]`.
    pub(crate) fn resolve(spec: &MeshSourceCrs, bounds: [[f64; 3]; 2]) -> Result<Self> {
        let [min, max] = bounds;
        if !(0..3).all(|i| min[i] <= max[i]) {
            return Err(JobError::new(
                JobErrorKind::InvalidInput,
                "a source CRS needs at least one source triangle to choose its origin",
            ));
        }
        let mut operation = spec.operation()?;
        let kind = std::mem::discriminant(&operation);
        let (horizontal_step, degrees_per_unit) = mesh_crs::horizontal_step(&operation)
            .ok_or_else(|| {
                JobError::new(
                    JobErrorKind::Unsupported,
                    "cannot determine horizontal source CRS units",
                )
            })?;
        let centre: [f64; 3] = std::array::from_fn(|i| (min[i] + max[i]) * 0.5);
        let ecef = operation
            .transform(&[spec.axes.coordinates(centre, spec.offset)])
            .map_err(source_error)?[0];
        let origin = ecef_to_cartographic(ecef).map_err(source_error)?;
        if std::mem::discriminant(&operation) != kind {
            return Err(promoted());
        }
        let anchor = [origin.lon_deg, origin.lat_deg, origin.height_m];
        let placement = ResolvedPlacement::resolve(&MeshPlacement::Wgs84 {
            anchor_degrees_metres: anchor,
            orientation_xyzw: [0., 0., 0., 1.],
            scene_offset_metres: [0.; 3],
        })?;
        let report = MeshSourceCrsReport {
            definition: spec.definition.clone(),
            axes: match spec.axes {
                SourceAxes::Xyz => "xyz",
                SourceAxes::YUp => "y-up",
            },
            source_offset: [
                spec.offset.easting,
                spec.offset.northing,
                spec.offset.height,
            ],
            height_offset_metres: spec.height_offset_metres,
            operation: match operation {
                Transform::Pure(_) => "portable",
                #[cfg(feature = "native-geospatial")]
                Transform::Native(_) => "native",
            },
            origin_degrees_metres: anchor,
            jacobian_lattice_cells: [0; 3],
            jacobian_exact_cells: 0,
        };
        let mut frame = Box::new(CrsFrame {
            operation,
            kind,
            axes: spec.axes,
            offset: spec.offset,
            horizontal_step,
            degrees_per_unit,
            frame: EnuFrame::new(origin),
            placement,
            report,
            lattice: Lattice::default(),
        });
        frame.lattice = frame.build_lattice(min, max)?;
        frame.report.jacobian_lattice_cells = frame.lattice.cells.map(|n| n as u64);
        frame.report.jacobian_exact_cells =
            frame.lattice.exact.iter().filter(|e| **e).count() as u64;
        Ok(Self::Crs(frame))
    }

    /// Root placement and report of a resolved source CRS.
    pub(crate) fn placement(&self) -> Option<(ResolvedPlacement, MeshSourceCrsReport)> {
        match self {
            Self::Local => None,
            Self::Crs(frame) => Some((frame.placement.clone(), frame.report.clone())),
        }
    }

    /// The used post-node source bounds a CRS frame was resolved from.
    pub(crate) fn source_bounds(&self) -> Option<[[f64; 3]; 2]> {
        match self {
            Self::Local => None,
            Self::Crs(frame) => Some([frame.lattice.min, frame.lattice.max]),
        }
    }

    /// A position, rounded once to f32, and, when requested in CRS mode, its Jacobian.
    pub(crate) fn vertex(
        &mut self,
        world: [f64; 3],
        jacobian: bool,
    ) -> Result<([f32; 3], Option<Jacobian>)> {
        match self {
            Self::Local => Ok((world.map(|x| x as f32), None)),
            Self::Crs(frame) => frame.vertex(world, jacobian),
        }
    }
}

fn promoted() -> JobError {
    JobError::new(
        JobErrorKind::Unsupported,
        "part of the source needs a different (native) CRS operation than its origin; one operation must serve the whole capture",
    )
}

impl CrsFrame {
    fn ecef(&mut self, world: [f64; 3]) -> std::result::Result<[f64; 3], crate::Error> {
        Ok(self
            .operation
            .transform(&[self.axes.coordinates(world, self.offset)])?[0])
    }

    fn check_kind(&self) -> Result<()> {
        if std::mem::discriminant(&self.operation) != self.kind {
            return Err(promoted());
        }
        Ok(())
    }

    /// The per-vertex Jacobian from central differences at a validated ECEF position.
    fn exact(
        &mut self,
        world: [f64; 3],
        ecef: [f64; 3],
    ) -> std::result::Result<Columns, crate::Error> {
        mesh_crs::vertex_jacobian(
            &mut self.operation,
            world,
            ecef,
            self.axes,
            self.offset,
            self.horizontal_step,
            self.degrees_per_unit,
            &self.frame,
        )
    }

    /// A lattice Jacobian; `None` outside the CRS domain. Other failures are errors.
    fn lattice_sample(&mut self, world: [f64; 3]) -> Result<Option<Columns>> {
        let result = self.ecef(world).and_then(|ecef| self.exact(world, ecef));
        self.check_kind()?;
        match result {
            Ok(columns) => Ok(Some(columns)),
            Err(crate::Error::Data(_)) => Ok(None),
            Err(error) => Err(source_error(error)),
        }
    }

    fn build_lattice(&mut self, min: [f64; 3], max: [f64; 3]) -> Result<Lattice> {
        let mut lattice = Lattice {
            min,
            max,
            ..Lattice::default()
        };
        for axis in 0..3 {
            let extent = max[axis] - min[axis];
            let span =
                mesh_crs::probe_step(self.axes, axis, self.horizontal_step) * LATTICE_PROBE_STEPS;
            lattice.cells[axis] = if extent > 0. {
                ((extent / span).ceil() as usize).clamp(1, LATTICE_CELLS)
            } else {
                1
            };
            lattice.cell[axis] = extent / lattice.cells[axis] as f64;
        }
        let [nx, ny, nz] = lattice.cells;
        lattice.nodes = Vec::with_capacity((nx + 1) * (ny + 1) * (nz + 1));
        for k in 0..=nz {
            for j in 0..=ny {
                for i in 0..=nx {
                    let world = [
                        lattice.coordinate(0, i),
                        lattice.coordinate(1, j),
                        lattice.coordinate(2, k),
                    ];
                    lattice.nodes.push(self.lattice_sample(world)?);
                }
            }
        }
        lattice.exact = Vec::with_capacity(nx * ny * nz);
        for k in 0..nz {
            for j in 0..ny {
                for i in 0..nx {
                    let cell = [i, j, k];
                    let centre: [f64; 3] = std::array::from_fn(|axis| {
                        (lattice.coordinate(axis, cell[axis])
                            + lattice.coordinate(axis, cell[axis] + 1))
                            * 0.5
                    });
                    let exact = match (
                        self.lattice_sample(centre)?,
                        lattice.interpolate(cell, [0.5; 3]),
                    ) {
                        (Some(want), Some(got)) => {
                            let error = relative_error(&want, &got);
                            !error.is_finite() || error > LATTICE_TOLERANCE
                        }
                        _ => true,
                    };
                    lattice.exact.push(exact);
                }
            }
        }
        Ok(lattice)
    }

    fn vertex(&mut self, world: [f64; 3], jacobian: bool) -> Result<([f32; 3], Option<Jacobian>)> {
        let ecef = self.ecef(world).map_err(source_error)?;
        self.check_kind()?;
        let local = mesh_crs::to_y_up(self.frame.to_enu(ecef));
        if !local.iter().all(|v| within_local_domain(*v)) {
            return Err(JobError::new(
                JobErrorKind::Unsupported,
                "source CRS position maps outside the local mesh domain",
            ));
        }
        let jacobian = if jacobian {
            let (cell, t) = self.lattice.locate(world);
            let interpolated = (!self.lattice.exact[self.lattice.cell_index(cell)])
                .then(|| self.lattice.interpolate(cell, t))
                .flatten();
            Some(Jacobian(match interpolated {
                Some(columns) => columns,
                None => {
                    let columns = self.exact(world, ecef).map_err(source_error)?;
                    self.check_kind()?;
                    columns
                }
            }))
        } else {
            None
        };
        Ok((local.map(|v| v as f32), jacobian))
    }

    /// The per-vertex Jacobian, bypassing the lattice (accuracy audits).
    #[cfg(test)]
    fn exact_jacobian(&mut self, world: [f64; 3]) -> Jacobian {
        let ecef = self.ecef(world).unwrap();
        Jacobian(self.exact(world, ecef).unwrap())
    }
}

fn relative_error(want: &Columns, got: &Columns) -> f64 {
    let (mut error, mut norm) = (0., 0.);
    for (w, g) in want.iter().flatten().zip(got.iter().flatten()) {
        error += (w - g) * (w - g);
        norm += w * w;
    }
    (error / norm).sqrt()
}

#[cfg(test)]
#[path = "source_crs_tests.rs"]
mod tests;
