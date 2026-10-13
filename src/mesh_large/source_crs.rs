//! Declared source CRS of a mesh, applied at capture (H1) in f64. Each post-node source
//! position plus the source offset maps through the shared conservative resolver to WGS84
//! ECEF and then into one local ENU frame (glTF Y-up) at the source-bounds centre. Normals
//! use the per-vertex Jacobian inverse transpose and tangents the Jacobian, both from the
//! legacy bake's central differences. Every later stage sees local metres only; the root
//! transform carries ENU to ECEF.
use crate::{
    crs::Transform,
    georef::{ecef_to_cartographic, EnuFrame, SourceAxes, SourceOffset},
    gltf_source::source::within_local_domain,
    mesh_archive::ResolvedPlacement,
    mesh_crs, JobError, JobErrorKind, MeshPlacement,
};
use serde::Serialize;
use std::mem::Discriminant;

type Result<T> = std::result::Result<T, JobError>;

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

fn invalid(message: impl Into<String>) -> JobError {
    JobError::new(JobErrorKind::InvalidRequest, message)
}

/// A declared horizontal source CRS with explicit axes, source offset and height decision.
/// Without it (`local`) the mesh is local metres with glTF Y up.
#[derive(Clone, Debug, PartialEq)]
pub struct MeshSourceCrs {
    definition: String,
    axes: SourceAxes,
    offset: SourceOffset,
    height_offset_metres: f64,
}

impl MeshSourceCrs {
    /// Shared adapter grammar. `None` or `local` keeps local coordinates and admits no
    /// other source-CRS parameter. Any other definition requires explicit axes and a height
    /// offset (including zero); the offset is `E N [A]`, with A defaulting to zero.
    pub fn from_parameters(
        source_crs: Option<&str>,
        axes: Option<SourceAxes>,
        offset: Option<&[f64]>,
        height_offset_metres: Option<f64>,
    ) -> Result<Option<Self>> {
        let definition = source_crs
            .map(str::trim)
            .filter(|d| !d.eq_ignore_ascii_case("local"));
        let Some(definition) = definition else {
            if axes.is_some() || offset.is_some() || height_offset_metres.is_some() {
                return Err(invalid(
                    "source axes, source offset and height offset require a source CRS",
                ));
            }
            return Ok(None);
        };
        if LEGACY_KEYWORDS.contains(&definition.to_ascii_lowercase().as_str()) {
            return Err(invalid(
                "source CRS must be an EPSG:<code>, WKT or PROJ definition or local; legacy auto/geographic/Web Mercator keywords are not inferred",
            ));
        }
        let axes = axes.ok_or_else(|| {
            invalid("a source CRS requires source axes xyz or y-up; axes are not inferred")
        })?;
        let height_offset_metres = height_offset_metres.ok_or_else(|| {
            invalid("a source CRS requires a height offset in metres to ellipsoidal height, including an explicit 0")
        })?;
        let offset = match offset.unwrap_or(&[0., 0.]) {
            [easting, northing] => [*easting, *northing, 0.],
            [easting, northing, height] => [*easting, *northing, *height],
            _ => return Err(invalid("source offset takes E N or E N A")),
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
            axes,
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
}

/// Per-vertex Jacobian columns d(local Y-up)/d(post-node source axis).
#[derive(Clone, Copy, Debug)]
pub(crate) struct Jacobian([[f64; 3]; 3]);

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

const CACHE_SLOTS: usize = 4096;
const EMPTY: [u64; 3] = [u64::MAX; 3];

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
    /// Direct-mapped memo of the pure position map for repeated scan corners.
    cache: Vec<([u64; 3], [f32; 3])>,
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
        };
        Ok(Self::Crs(Box::new(CrsFrame {
            operation,
            kind,
            axes: spec.axes,
            offset: spec.offset,
            horizontal_step,
            degrees_per_unit,
            frame: EnuFrame::new(origin),
            placement,
            report,
            cache: vec![(EMPTY, [0.; 3]); CACHE_SLOTS],
        })))
    }

    /// Root placement and report of a resolved source CRS.
    pub(crate) fn placement(&self) -> Option<(ResolvedPlacement, MeshSourceCrsReport)> {
        match self {
            Self::Local => None,
            Self::Crs(frame) => Some((frame.placement.clone(), frame.report.clone())),
        }
    }

    /// The single f32 rounding of a post-node f64 position (memoized for CRS).
    pub(crate) fn position(&mut self, world: [f64; 3]) -> Result<[f32; 3]> {
        match self {
            Self::Local => Ok(world.map(|x| x as f32)),
            Self::Crs(frame) => {
                let key = world.map(f64::to_bits);
                let slot = (key[0] ^ key[1].rotate_left(21) ^ key[2].rotate_left(42))
                    .wrapping_mul(0x9e37_79b9_7f4a_7c15)
                    >> (64 - CACHE_SLOTS.trailing_zeros());
                let slot = slot as usize;
                if frame.cache[slot].0 == key {
                    return Ok(frame.cache[slot].1);
                }
                let (position, _) = frame.vertex(world, false)?;
                frame.cache[slot] = (key, position);
                Ok(position)
            }
        }
    }

    /// A position and, when requested in CRS mode, its Jacobian.
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
    fn vertex(&mut self, world: [f64; 3], jacobian: bool) -> Result<([f32; 3], Option<Jacobian>)> {
        let ecef = self
            .operation
            .transform(&[self.axes.coordinates(world, self.offset)])
            .map_err(source_error)?[0];
        if std::mem::discriminant(&self.operation) != self.kind {
            return Err(promoted());
        }
        let local = mesh_crs::to_y_up(self.frame.to_enu(ecef));
        if !local.iter().all(|v| within_local_domain(*v)) {
            return Err(JobError::new(
                JobErrorKind::Unsupported,
                "source CRS position maps outside the local mesh domain",
            ));
        }
        let jacobian = if jacobian {
            let columns = mesh_crs::vertex_jacobian(
                &mut self.operation,
                world,
                ecef,
                self.axes,
                self.offset,
                self.horizontal_step,
                self.degrees_per_unit,
                &self.frame,
            )
            .map_err(source_error)?;
            if std::mem::discriminant(&self.operation) != self.kind {
                return Err(promoted());
            }
            Some(Jacobian(columns))
        } else {
            None
        };
        Ok((local.map(|v| v as f32), jacobian))
    }
}

#[cfg(test)]
#[path = "source_crs_tests.rs"]
mod tests;
