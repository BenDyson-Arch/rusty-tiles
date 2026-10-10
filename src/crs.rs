//! Shared horizontal-CRS to WGS84 ECEF operations, initially used by point clouds.
//!
//! Input coordinates use traditional GIS axis order (easting/northing or
//! longitude/latitude). Source Z and the explicit height offset are metres;
//! source horizontal units never rescale Z. Callers own source-specific CRS and
//! height selection, including any decision to leave coordinates local.
//!
//! Conservative, grid-free operations with strict native fallback when enabled.
//! EPSG definitions are an
//! explicit allowlist, not an old PROJ-string database with implicit datum shifts.
//! The WKT parser is used only for syntax: proj4wkt's formatter assumes a zero
//! datum shift and drops vertical/unknown metadata, so it is deliberately unused.
use crate::Error;
use proj4rs::Proj;
use proj4wkt::parser::{Attribute, Processor};
use std::collections::BTreeMap;

pub(crate) enum Transform {
    Pure(Box<PureTransform>),
    #[cfg(feature = "native-geospatial")]
    Native(Box<NativeTransform>),
}

#[cfg(feature = "native-geospatial")]
pub(crate) struct NativeTransform {
    ecef: crate::geospatial::EcefTransform,
    albers: Option<(crate::geospatial::StrictTransform, f64)>,
    units: Option<(bool, f64)>,
}

#[cfg(feature = "native-geospatial")]
impl NativeTransform {
    fn transform(&mut self, points: &[[f64; 3]]) -> Result<Vec<[f64; 3]>, Error> {
        if let Some((geographic, units)) = &mut self.albers {
            for (index, point) in geographic.transform(points)?.iter().enumerate() {
                if (point[1] * *units).abs() >= 80_f64.to_radians() {
                    return Err(albers_outside_domain(index));
                }
            }
        }
        self.ecef.transform(points)
    }
}

fn albers_outside_domain(index: usize) -> Error {
    Error::Data(format!("point {index}: Albers source latitude at or beyond 80 degrees is outside verified point-cloud accuracy"))
}

impl Transform {
    /// Resolve a 2D horizontal source CRS and an ellipsoidal height offset.
    /// Unsupported portable operations use strict native PROJ when available;
    /// invalid data never falls back to a more permissive operation.
    pub fn new(definition: &str, height_offset: f64) -> Result<Self, Error> {
        match PureTransform::new(definition, height_offset) {
            Ok(operation) => Ok(Self::Pure(Box::new(operation))),
            #[cfg(feature = "native-geospatial")]
            Err(Error::Environment(_)) => Self::native(definition, height_offset),
            Err(error) => Err(error),
        }
    }

    #[cfg(feature = "native-geospatial")]
    fn native(definition: &str, height_offset: f64) -> Result<Self, Error> {
        let source = crate::geospatial::Crs::from_definition(definition)?;
        if !source.is_horizontal() {
            return Err(horizontal_required());
        }
        if let Some((first, second)) = source.conic_parallels()? {
            validate_conic_conditioning(first, second)?;
        }
        let albers = source.albers_domain()?;
        let units = source.polygon_units();
        let ecef = crate::geospatial::EcefTransform::new(source, Some(height_offset))?;
        Ok(Self::Native(Box::new(NativeTransform {
            ecef,
            albers,
            units,
        })))
    }

    /// Source-coordinate normalization for intrinsic polygon charts.
    /// Geographic coordinates become degrees; projected coordinates metres.
    pub(crate) fn polygon_units(&self) -> Option<(bool, f64)> {
        match self {
            Self::Pure(operation) if operation.source.is_latlong() => {
                Some((true, operation.angular_units.to_degrees()))
            }
            Self::Pure(operation) => Some((false, operation.source.to_meter())),
            #[cfg(feature = "native-geospatial")]
            Self::Native(operation) => operation.units,
        }
    }

    /// Convert a complete batch to ECEF metres, retaining its input on failure.
    /// A point-dependent native fallback retries every point and is retained
    /// for subsequent batches. No partial result is returned to the caller.
    pub fn transform(&mut self, points: &[[f64; 3]]) -> Result<Vec<[f64; 3]>, Error> {
        let result = match self {
            Self::Pure(operation) => operation.transform(points),
            #[cfg(feature = "native-geospatial")]
            Self::Native(operation) => operation.transform(points),
        };
        #[cfg(feature = "native-geospatial")]
        if matches!(result, Err(Error::Environment(_))) {
            if let Self::Pure(operation) = self {
                let mut native = Self::native(&operation.definition, operation.height_offset)?;
                // Retry the entire batch, so no partial portable result escapes.
                let output = native.transform(points)?;
                *self = native;
                return Ok(output);
            }
        }
        result
    }
}

fn unsupported(reason: &str) -> Error {
    Error::Environment(format!("pure-Rust point-cloud CRS transform unavailable: {reason}; use a build with --features native-geospatial (GDAL >= 3.12, PROJ >= 9.2) and the required local PROJ database/grids"))
}

fn horizontal_required() -> Error {
    Error::Data("use a 2D horizontal CRS and explicit ellipsoidal height offset; compound/geocentric CRS is unsupported".into())
}

pub(crate) struct PureTransform {
    source: Proj,
    target: Proj,
    angular_units: f64,
    height_offset: f64,
    source_geographic: Option<Proj>,
    #[cfg(feature = "native-geospatial")]
    definition: String,
}

impl PureTransform {
    fn new(definition: &str, height_offset: f64) -> Result<Self, Error> {
        if !height_offset.is_finite() {
            return Err(Error::Data("height offset must be finite".into()));
        }
        let definition = definition.trim();
        #[cfg(feature = "native-geospatial")]
        let original_definition = definition.to_owned();
        if definition.contains('\0') {
            return Err(Error::Data("CRS definition contains a NUL byte".into()));
        }
        let (definition, angular_units) = if definition.starts_with('+') {
            (
                validate_proj(definition, false)?,
                std::f64::consts::PI / 180.,
            )
        } else if definition.to_ascii_uppercase().starts_with("EPSG:") {
            (epsg(definition)?, std::f64::consts::PI / 180.)
        } else if definition.contains('[') {
            wkt(definition)?
        } else {
            return Err(unsupported(
                "expected an EPSG code, WKT or PROJ CRS definition",
            ));
        };
        let source = Proj::from_proj_string(&definition).map_err(|error| match error {
            // Some proj4rs projections need an ellipsoid, including extended
            // transverse Mercator. Native PROJ handles their spherical forms;
            // keep the original definition available to that fallback.
            proj4rs::errors::Error::EllipsoidRequired => {
                unsupported("spherical form of this projection requires native PROJ")
            }
            error => Error::Data(format!("invalid source CRS: {error}")),
        })?;
        if source.is_geocent() {
            return Err(horizontal_required());
        }
        // proj4rs 0.2 does not apply prime meridians consistently (geographic
        // input skips them). Route those definitions through strict native PROJ.
        if source.from_greenwich() != 0. {
            return Err(unsupported(
                "non-Greenwich prime meridians require native PROJ",
            ));
        }
        if source.to_meter() <= 0. || !source.to_meter().is_finite() {
            return Err(Error::Data(
                "CRS unit factor must be finite and positive".into(),
            ));
        }
        if !definition.contains("+towgs84=") {
            let (a, b) = source.ellipse_parameters();
            let wgs_b = 6378137. * (1. - 1. / 298.257223563);
            if (a - 6378137.).abs() > 1e-8 || (b - wgs_b).abs() > 1e-8 {
                return Err(unsupported(
                    "WGS84 datum has a conflicting ellipsoid override",
                ));
            }
        }
        let target = Proj::from_proj_string("+proj=geocent +datum=WGS84 +units=m")
            .map_err(|error| unsupported(&error.to_string()))?;
        let polar_stereographic = source.projname() == "stere"
            && definition.split_whitespace().any(|parameter| {
                parameter
                    .strip_prefix("+lat_0=")
                    .is_some_and(|value| finite(value).is_ok_and(|value| value.abs() == 90.))
            });
        let (a, b) = source.ellipse_parameters();
        let source_geographic = if source.projname() == "sterea"
            || (source.projname() == "stere" && !polar_stereographic)
            || source.projname() == "aea"
        {
            // An unknown target datum deliberately skips datum conversion: this
            // probe checks latitude in the source projection, before any shift.
            Some(
                Proj::from_proj_string(&format!("+proj=longlat +a={a} +b={b}"))
                    .map_err(|error| unsupported(&error.to_string()))?,
            )
        } else {
            None
        };
        Ok(Self {
            source,
            target,
            angular_units,
            height_offset,
            source_geographic,
            #[cfg(feature = "native-geospatial")]
            definition: original_definition,
        })
    }

    fn transform(&self, points: &[[f64; 3]]) -> Result<Vec<[f64; 3]>, Error> {
        let mut output = Vec::with_capacity(points.len());
        for (index, point) in points.iter().enumerate() {
            let mut p = (point[0], point[1], point[2] + self.height_offset);
            if ![p.0, p.1, p.2].iter().all(|value| value.is_finite()) {
                return Err(Error::Data(format!(
                    "point {index}: coordinates must be finite XYZ"
                )));
            }
            if let Some(geographic) = &self.source_geographic {
                let mut probe = p;
                proj4rs::transform::transform(&self.source, geographic, &mut probe).map_err(
                    |error| unsupported(&format!("point {index}: cannot establish portable {} coordinate domain: {error}", self.source.projname())),
                )?;
                if ![probe.0, probe.1, probe.2]
                    .iter()
                    .all(|value| value.is_finite())
                    || probe.1.abs() >= 80_f64.to_radians()
                {
                    if self.source.projname() == "aea" {
                        return Err(albers_outside_domain(index));
                    }
                    return Err(unsupported(&format!(
                        "point {index}: {} coordinates at or beyond 80 degrees source latitude require native PROJ", self.source.projname()
                    )));
                }
            }
            if self.source.is_latlong() {
                p.0 *= self.angular_units;
                p.1 *= self.angular_units;
                if p.0.abs() > std::f64::consts::PI + 1e-12
                    || p.1.abs() > std::f64::consts::FRAC_PI_2 + 1e-12
                {
                    return Err(Error::Data(format!(
                        "point {index}: geographic coordinates outside longitude/latitude range"
                    )));
                }
            }
            proj4rs::transform::transform(&self.source, &self.target, &mut p).map_err(|error| {
                Error::Data(format!("point {index}: CRS transform failed: {error}"))
            })?;
            if ![p.0, p.1, p.2].iter().all(|value| value.is_finite()) {
                return Err(Error::Data(format!(
                    "point {index}: CRS transform produced nonfinite XYZ"
                )));
            }
            output.push([p.0, p.1, p.2]);
        }
        Ok(output)
    }
}

fn epsg(definition: &str) -> Result<String, Error> {
    if definition[5..].contains('@') {
        return Err(unsupported("EPSG coordinate epochs require native PROJ"));
    }
    if definition[5..].contains('+') {
        return Err(unsupported(
            "compound/vertical EPSG CRS may require a geoid grid",
        ));
    }
    let code = definition[5..]
        .trim()
        .parse::<u32>()
        .map_err(|_| Error::Data("invalid source EPSG code".into()))?;
    let projection = match code {
        4326 => "+proj=longlat".to_owned(),
        3857 => "+proj=webmerc".to_owned(),
        3395 => "+proj=merc".to_owned(),
        32601..=32660 => format!("+proj=utm +zone={}", code - 32600),
        32701..=32760 => format!("+proj=utm +zone={} +south", code - 32700),
        4978 | 4979 => return Err(horizontal_required()),
        _ => return Err(unsupported("EPSG CRS is outside the verified WGS84 geographic, UTM and Mercator allowlist; its best datum operation may require grids")),
    };
    Ok(format!("{projection} +datum=WGS84 +units=m"))
}

// Only parameters whose semantics we implement may reach proj4rs: it otherwise
// accepts unknown keys. Projection-specific keys must not silently be ignored.
fn projection_keys(projection: &str) -> Result<&'static [&'static str], Error> {
    match projection {
        "longlat" | "latlong" => Ok(&[]),
        "utm" => Ok(&["zone", "south"]),
        "tmerc" | "etmerc" => Ok(&[]),
        "merc" => Ok(&["lat_ts"]),
        "webmerc" => Ok(&[]),
        "lcc" | "aea" => Ok(&["lat_1", "lat_2"]),
        "stere" => Ok(&["lat_ts"]),
        "sterea" => Ok(&[]),
        "laea" => Err(unsupported(
            "Lambert azimuthal equal area requires native PROJ for millimetre accuracy",
        )),
        "geocent" => Err(horizontal_required()),
        _ => Err(unsupported(
            "projection method is outside the verified grid-free tier",
        )),
    }
}

fn validate_proj(definition: &str, from_wkt: bool) -> Result<String, Error> {
    // This tokenizer only handles unquoted, whitespace-separated parameters.
    // Defer quoted values (including embedded spaces) before splitting them.
    if definition.split_whitespace().any(|token| {
        token
            .split_once('=')
            .is_some_and(|(_, value)| value.starts_with(['\'', '"']))
    }) {
        return Err(unsupported("quoted PROJ values require native PROJ"));
    }
    let mut params = BTreeMap::new();
    for token in definition.split_whitespace() {
        let token = token
            .strip_prefix('+')
            .ok_or_else(|| unsupported("PROJ parameter syntax requires native PROJ"))?;
        let (key, value) = token.split_once('=').unwrap_or((token, ""));
        if params.insert(key, value).is_some() {
            return Err(Error::Data(format!("duplicate PROJ CRS parameter: +{key}")));
        }
    }
    if params.contains_key("init") {
        return Err(unsupported("PROJ +init references require native PROJ"));
    }
    let projection = params
        .get("proj")
        .copied()
        .ok_or_else(|| Error::Data("source CRS has no +proj parameter".into()))?;
    let keys = projection_keys(projection)?;
    for (&key, &value) in &params {
        if !matches!(
            key,
            "proj"
                | "datum"
                | "ellps"
                | "a"
                | "b"
                | "rf"
                | "f"
                | "towgs84"
                | "units"
                | "to_meter"
                | "pm"
                | "lon_0"
                | "lat_0"
                | "x_0"
                | "y_0"
                | "k"
                | "k_0"
                | "no_defs"
                | "type"
        ) && !keys.contains(&key)
        {
            return Err(unsupported(&format!(
                "parameter +{key} may require grids or unsupported CRS semantics"
            )));
        }
        if matches!(key, "no_defs" | "south") {
            if !value.is_empty() {
                return Err(Error::Data(format!("+{key} is a flag")));
            }
        } else if value.is_empty() {
            return Err(Error::Data(format!("missing value for +{key}")));
        }
        if key == "type" && value != "crs" {
            return Err(unsupported("only CRS definitions are supported"));
        }
        if key == "units"
            && !matches!(projection, "longlat" | "latlong")
            && proj4rs::units::find_units(value).is_none()
        {
            return Err(Error::Data(format!(
                "projected CRS requires a supported linear +units value, got {value}"
            )));
        }
        let named_greenwich = key == "pm" && value.eq_ignore_ascii_case("greenwich");
        if matches!(key, "lon_0" | "lat_0" | "lat_ts" | "lat_1" | "lat_2" | "pm")
            && !named_greenwich
            && value.parse::<f64>().is_err()
        {
            // PROJ supports DMS and directional/radian suffixes. They are
            // outside this decimal-degree tier, not necessarily malformed
            // definitions: preserve automatic strict native parsing/fallback.
            return Err(unsupported(&format!(
                "angular parameter +{key} uses syntax outside the verified decimal-degree tier"
            )));
        }
        if matches!(
            key,
            "a" | "b"
                | "rf"
                | "f"
                | "to_meter"
                | "lon_0"
                | "lat_0"
                | "x_0"
                | "y_0"
                | "k"
                | "k_0"
                | "lat_ts"
                | "lat_1"
                | "lat_2"
                | "pm"
                | "zone"
        ) && !named_greenwich
        {
            let number = finite(value)?;
            validate_projection_number(projection, key, number)?;
        }
    }
    if projection == "utm" && !params.contains_key("zone") {
        return Err(Error::Data(
            "UTM CRS requires an explicit +zone from 1 to 60".into(),
        ));
    }
    if projection == "stere" {
        let origin = params
            .get("lat_0")
            .map(|value| finite(value))
            .transpose()?
            .unwrap_or(0.);
        let parallel = params
            .get("lat_ts")
            .map(|value| finite(value))
            .transpose()?
            .unwrap_or(90.);
        let scale = params
            .get("k")
            .or_else(|| params.get("k_0"))
            .map(|value| finite(value))
            .transpose()?
            .unwrap_or(1.);
        if origin.abs() == 90. && params.contains_key("lat_ts") && parallel != origin && scale != 1.
        {
            return Err(Error::Data(
                "polar stereographic standard parallel conflicts with non-unit projection scale"
                    .into(),
            ));
        }
        if origin.abs() == 90.
            && params.contains_key("lat_ts")
            && (parallel == 0. || origin * parallel < 0.)
        {
            return Err(unsupported(
                "conflicting polar stereographic hemispheres or zero standard parallels require native CRS interpretation",
            ));
        }
    }
    if projection == "sterea"
        && params
            .get("lat_0")
            .is_some_and(|value| finite(value).is_ok_and(|value| value.abs() >= 80.))
    {
        // gauss_ini loses precision as sin(phi0) approaches 1, well before the
        // exact pole. Restrict portable origins to the tested range below 80
        // degrees in either hemisphere, with a generous stability margin.
        return Err(unsupported(
            "oblique stereographic origins at or beyond 80 degrees require native PROJ",
        ));
    }
    if projection == "stere"
        && params.get("lat_0").is_some_and(|value| {
            finite(value).is_ok_and(|value| (80. ..90.).contains(&value.abs()))
        })
    {
        // The oblique formula becomes unstable near the poles. Exact polar
        // origins use a separate, verified formula and remain portable.
        return Err(unsupported(
            "stereographic origins from 80 degrees to below 90 degrees require native PROJ",
        ));
    }
    if matches!(projection, "lcc" | "aea") {
        let first = params
            .get("lat_1")
            .map(|value| finite(value))
            .transpose()?
            .unwrap_or(0.);
        let second = params
            .get("lat_2")
            .map(|value| finite(value))
            .transpose()?
            .unwrap_or(first);
        validate_conic_conditioning(first, second)?;
        if first.abs().max(second.abs()) > 80. + 1e-10 {
            return Err(unsupported(
                "conic standard parallels beyond 80 degrees latitude require native PROJ",
            ));
        }
        if first != second && (first - second).abs() < 1. - 1e-10 {
            // Secant conic constants divide differences of nearly equal
            // quantities. Keep a generous margin around cancellation; the
            // exact tangent formula (equal parallels) has no such division.
            // Allow WKT unit roundoff at the one-degree boundary.
            return Err(unsupported(
                "distinct conic parallels less than one degree apart require native PROJ",
            ));
        }
        if projection == "lcc"
            && !from_wkt
            && (!params.contains_key("lat_0") || !params.contains_key("lat_2"))
        {
            return Err(unsupported("omitted LCC origins or second standard parallels require native CRS interpretation"));
        }
    }
    if matches!(projection, "longlat" | "latlong")
        && params
            .get("lon_0")
            .is_some_and(|value| finite(value).is_ok_and(|value| value != 0.))
    {
        // proj4rs ignores lam0 for geographic input, while native PROJ applies
        // it. Never select an operation that silently loses this offset.
        return Err(unsupported("geographic +lon_0 offsets require native PROJ"));
    }
    if params.contains_key("k") && params.contains_key("k_0") {
        return Err(Error::Data("conflicting +k and +k_0 parameters".into()));
    }
    if let Some(shift) = params.get("towgs84") {
        let values: Vec<_> = shift.split(',').collect();
        if !matches!(values.len(), 3 | 7) {
            return Err(Error::Data(
                "+towgs84 requires 3 or 7 finite parameters".into(),
            ));
        }
        for value in values {
            finite(value)?;
        }
        if !params.contains_key("ellps")
            && !params.contains_key("a")
            && !params.contains_key("datum")
        {
            return Err(unsupported(
                "Helmert parameters require an explicit source ellipsoid",
            ));
        }
    } else if params.get("datum") != Some(&"WGS84") {
        return Err(unsupported("datum is not explicitly WGS84 and no Helmert +towgs84 parameters are supplied; an ellipsoid alone does not establish a datum"));
    }
    // PROJ uses extended transverse Mercator by default. The older tmerc
    // approximation in proj4rs is not millimetre accurate away from lon_0.
    let mut result = String::new();
    for (key, value) in params {
        let value = if key == "proj" && value == "tmerc" {
            "etmerc"
        } else {
            value
        };
        // proj4rs 0.2 spells PROJ's k_0 alias as k0 internally; normalize to
        // the shared +k spelling so WKT conversion scale factors are retained.
        let key = if key == "k_0" { "k" } else { key };
        result.push_str(&format!(" +{key}"));
        if !value.is_empty() {
            result.push_str(&format!("={value}"));
        }
    }
    Ok(result)
}

fn finite(value: &str) -> Result<f64, Error> {
    value
        .parse::<f64>()
        .ok()
        .filter(|value| value.is_finite())
        .ok_or_else(|| Error::Data(format!("invalid finite CRS number: {value}")))
}

fn validate_projection_number(projection: &str, key: &str, number: f64) -> Result<(), Error> {
    if matches!(key, "k" | "k_0") && number <= 0. {
        return Err(Error::Data(format!(
            "projection scale +{key} must be positive"
        )));
    }
    if matches!(key, "lat_0" | "lat_1" | "lat_2" | "lat_ts") {
        let excludes_poles = (projection == "lcc" && matches!(key, "lat_1" | "lat_2"))
            || (projection == "merc" && key == "lat_ts");
        let lcc_near_pole = projection == "lcc"
            && matches!(key, "lat_1" | "lat_2")
            && number.to_radians().cos().abs() < 1e-10;
        if lcc_near_pole {
            return Err(Error::Data(format!(
                "projection latitude +{key} is too close to a pole for Lambert conformal conic"
            )));
        }
        if number.abs() > 90. || (excludes_poles && number.abs() == 90.) {
            let range = if excludes_poles {
                "(-90, 90)"
            } else {
                "[-90, 90]"
            };
            return Err(Error::Data(format!(
                "projection latitude +{key} must be in {range} degrees"
            )));
        }
    }
    Ok(())
}

fn validate_conic_conditioning(first: f64, second: f64) -> Result<(), Error> {
    if !first.is_finite() || !second.is_finite() || (first + second).abs() < 1. - 1e-10 {
        return Err(Error::Data("conic standard parallels too close to opposite latitudes or the equator for verified point-cloud accuracy".into()));
    }
    Ok(())
}

#[derive(Debug)]
struct Node<'a> {
    key: String,
    attrs: Vec<Attribute<'a, Node<'a>>>,
}

struct Tree;
impl<'a> Processor<'a> for Tree {
    type Err = ();
    type Output = Node<'a>;
    fn process<I>(&self, key: &'a str, _: usize, attrs: I) -> Result<Node<'a>, ()>
    where
        I: Iterator<Item = Attribute<'a, Node<'a>>>,
    {
        Ok(Node {
            key: key.to_ascii_uppercase(),
            attrs: attrs.collect(),
        })
    }
}

impl<'a> Node<'a> {
    fn children(&self) -> impl Iterator<Item = &Node<'a>> {
        self.attrs.iter().filter_map(|attr| match attr {
            Attribute::Keyword(_, node) => Some(node),
            _ => None,
        })
    }
    fn child(&self, names: &[&str]) -> Result<Option<&Node<'a>>, Error> {
        let mut children = self
            .children()
            .filter(|node| names.contains(&node.key.as_str()));
        let result = children.next();
        if children.next().is_some() {
            return Err(Error::Data("duplicate WKT CRS component".into()));
        }
        Ok(result)
    }
    fn required(&self, names: &[&str]) -> Result<&Node<'a>, Error> {
        self.child(names)?
            .ok_or_else(|| Error::Data(format!("WKT CRS is missing {}", names[0])))
    }
    fn text(&self, index: usize) -> Result<&str, Error> {
        match self.attrs.get(index) {
            Some(Attribute::Quoted(value) | Attribute::Label(value)) => Ok(value),
            _ => Err(Error::Data("invalid WKT CRS text".into())),
        }
    }
    fn number(&self, index: usize) -> Result<f64, Error> {
        match self.attrs.get(index) {
            Some(Attribute::Number(value)) => finite(value),
            _ => Err(Error::Data("invalid WKT CRS number".into())),
        }
    }
    fn validate(&self) -> Result<(), Error> {
        if !matches!(
            self.key.as_str(),
            "GEOGCS"
                | "PROJCS"
                | "GEOGCRS"
                | "GEODCRS"
                | "PROJCRS"
                | "BASEGEOGCRS"
                | "BASEGEODCRS"
                | "DATUM"
                | "GEODETICDATUM"
                | "ENSEMBLE"
                | "MEMBER"
                | "ENSEMBLEACCURACY"
                | "SPHEROID"
                | "ELLIPSOID"
                | "PRIMEM"
                | "UNIT"
                | "LENGTHUNIT"
                | "ANGLEUNIT"
                | "SCALEUNIT"
                | "AXIS"
                | "ORDER"
                | "CS"
                | "AUTHORITY"
                | "ID"
                | "PROJECTION"
                | "CONVERSION"
                | "METHOD"
                | "PARAMETER"
                | "TOWGS84"
                | "SCOPE"
                | "AREA"
                | "BBOX"
                | "USAGE"
                | "REMARK"
                | "CITATION"
                | "URI"
                | "EXTENSION"
        ) {
            return Err(unsupported(&format!(
                "WKT component {} may require grids, epochs or unsupported CRS semantics",
                self.key
            )));
        }
        // Reject misplaced operational metadata as well as unknown keywords.
        // A TOWGS84 or parameter nested in the wrong component must never be
        // accepted by the syntax parser and then silently discarded.
        let allowed: &[&str] = match self.key.as_str() {
            "GEOGCS" | "GEOGCRS" | "GEODCRS" | "BASEGEOGCRS" | "BASEGEODCRS" => &[
                "DATUM",
                "GEODETICDATUM",
                "ENSEMBLE",
                "PRIMEM",
                "UNIT",
                "ANGLEUNIT",
                "CS",
                "AXIS",
            ],
            "PROJCS" => &[
                "GEOGCS",
                "PROJECTION",
                "PARAMETER",
                "UNIT",
                "AXIS",
                "EXTENSION",
            ],
            "PROJCRS" => &[
                "BASEGEOGCRS",
                "BASEGEODCRS",
                "CONVERSION",
                "CS",
                "AXIS",
                "LENGTHUNIT",
            ],
            "DATUM" | "GEODETICDATUM" => &["SPHEROID", "ELLIPSOID", "TOWGS84"],
            "ENSEMBLE" => &["MEMBER", "ELLIPSOID", "ENSEMBLEACCURACY"],
            "CONVERSION" => &["METHOD", "PARAMETER"],
            "SPHEROID" | "ELLIPSOID" => &["LENGTHUNIT"],
            "PRIMEM" => &["ANGLEUNIT"],
            "PARAMETER" => &["ANGLEUNIT", "LENGTHUNIT", "SCALEUNIT"],
            "AXIS" => &["ORDER", "ANGLEUNIT", "LENGTHUNIT"],
            "USAGE" => &["SCOPE", "AREA", "BBOX"],
            _ => &[],
        };
        for child in self.children() {
            if !allowed.contains(&child.key.as_str())
                && !matches!(
                    child.key.as_str(),
                    "AUTHORITY"
                        | "ID"
                        | "SCOPE"
                        | "AREA"
                        | "BBOX"
                        | "USAGE"
                        | "REMARK"
                        | "CITATION"
                        | "URI"
                )
            {
                return Err(unsupported(&format!(
                    "WKT component {} is misplaced or unsupported inside {}",
                    child.key, self.key
                )));
            }
            child.validate()?;
        }
        Ok(())
    }
}

fn unit(node: &Node<'_>, names: &[&str], default: f64) -> Result<f64, Error> {
    let value = node
        .child(names)?
        .map(|unit| unit.number(1))
        .transpose()?
        .unwrap_or(default);
    if value <= 0. {
        return Err(Error::Data("CRS unit factor must be positive".into()));
    }
    Ok(value)
}

fn coordinate_units(node: &Node<'_>, angular: bool) -> Result<f64, Error> {
    let names: &[&str] = if angular {
        &["UNIT", "ANGLEUNIT"]
    } else {
        &["UNIT", "LENGTHUNIT"]
    };
    let default = if angular {
        std::f64::consts::PI / 180.
    } else {
        1.
    };
    let factor = unit(node, names, default)?;
    let mut axis_factor = None;
    let mut directions = std::collections::BTreeSet::new();
    let axes: Vec<_> = node
        .children()
        .filter(|child| child.key == "AXIS")
        .collect();
    if !axes.is_empty() && axes.len() != 2 {
        return Err(horizontal_required());
    }
    for axis in axes {
        let direction = axis.text(1)?.to_ascii_lowercase();
        if !matches!(direction.as_str(), "east" | "north") || !directions.insert(direction) {
            return Err(unsupported(
                "WKT axis direction is outside east/north GIS axes",
            ));
        }
        let value = unit(axis, names, factor)?;
        if axis_factor.is_some_and(|previous| previous != value) {
            return Err(unsupported("WKT axes use different units"));
        }
        axis_factor = Some(value);
    }
    if let Some(cs) = node.child(&["CS"])? {
        if cs.number(1)? != 2. {
            return Err(horizontal_required());
        }
        if !cs
            .text(0)?
            .eq_ignore_ascii_case(if angular { "ellipsoidal" } else { "Cartesian" })
        {
            return Err(unsupported("WKT coordinate system type is unsupported"));
        }
    }
    Ok(axis_factor.unwrap_or(factor))
}

// Preserve the declared WKT method rather than allowing generic PROJ defaults
// or parameters from another variant to change the operation. Origin/offset
// and scale defaults are shared only where both parsers implement them alike.
fn validate_wkt_parameter_identity(
    method: &str,
    wkt1: bool,
    parameter: &Node<'_>,
    name: &str,
    key: &str,
) -> Result<(), Error> {
    let conic = matches!(
        method,
        "lambertconformalconic2sp"
            | "lambertconicconformal2sp"
            | "albersconicequalarea"
            | "albersequalarea"
    );
    let centered = matches!(
        method,
        "albersconicequalarea" | "albersequalarea" | "lambertazimuthalequalarea"
    );
    let polar_parallel = matches!(method, "polarstereographic" | "polarstereographicvariantb");
    // WKT1 and WKT2 have distinct method-specific vocabularies. In particular,
    // Mercator's first standard parallel (8823) is not polar stereographic's
    // latitude of standard parallel (8832), despite sharing PROJ's lat_ts.
    let (legacy, canonical, code) = match key {
        "lat_0" if conic => (
            if centered {
                "latitudeofcenter"
            } else {
                "latitudeoforigin"
            },
            "latitudeoffalseorigin",
            8821,
        ),
        "lon_0" if conic => (
            if centered {
                "longitudeofcenter"
            } else {
                "centralmeridian"
            },
            "longitudeoffalseorigin",
            8822,
        ),
        "lat_1" => ("standardparallel1", "latitudeof1ststandardparallel", 8823),
        "lat_2" => ("standardparallel2", "latitudeof2ndstandardparallel", 8824),
        "x_0" if conic => ("falseeasting", "eastingatfalseorigin", 8826),
        "y_0" if conic => ("falsenorthing", "northingatfalseorigin", 8827),
        "lat_ts" if polar_parallel => ("latitudeoforigin", "latitudeofstandardparallel", 8832),
        "lat_ts" => ("standardparallel1", "latitudeof1ststandardparallel", 8823),
        "lon_0" if polar_parallel => ("centralmeridian", "longitudeoforigin", 8833),
        "lat_0" => (
            if centered {
                "latitudeofcenter"
            } else {
                "latitudeoforigin"
            },
            "latitudeofnaturalorigin",
            8801,
        ),
        "lon_0" => (
            if centered {
                "longitudeofcenter"
            } else {
                "centralmeridian"
            },
            "longitudeofnaturalorigin",
            8802,
        ),
        "k_0" => ("scalefactor", "scalefactoratnaturalorigin", 8805),
        "x_0" => ("falseeasting", "falseeasting", 8806),
        "y_0" => ("falsenorthing", "falsenorthing", 8807),
        _ => {
            return Err(unsupported(
                "WKT projection parameter identity is unsupported",
            ))
        }
    };
    if name != if wkt1 { legacy } else { canonical } {
        return Err(unsupported(
            "WKT parameter name does not match the declared projection method and WKT version",
        ));
    }
    validate_wkt_identity(parameter, code)
}

fn validate_wkt_identity(node: &Node<'_>, code: u32) -> Result<(), Error> {
    if let Some(id) = node.child(&["ID", "AUTHORITY"])? {
        let supplied = match id.attrs.get(1) {
            Some(Attribute::Number(value) | Attribute::Quoted(value) | Attribute::Label(value)) => {
                value.parse::<u32>().ok()
            }
            _ => None,
        };
        if id.text(0)? != "EPSG" || supplied != Some(code) {
            return Err(unsupported(
                "WKT identity conflicts with the declared projection method",
            ));
        }
    }
    Ok(())
}

fn validate_wkt_parameters(
    method: &str,
    seen: &std::collections::BTreeSet<&str>,
) -> Result<(), Error> {
    let origin_scale = &["lat_0", "lon_0", "k_0", "x_0", "y_0"][..];
    let origin = &["lat_0", "lon_0", "x_0", "y_0"][..];
    let conic = &["lat_0", "lon_0", "lat_1", "lat_2", "x_0", "y_0"][..];
    let parallel = &["lat_ts", "lon_0", "x_0", "y_0"][..];
    let (allowed, required): (&[&str], &[&str]) = match method {
        "lambertconformalconic2sp"
        | "lambertconicconformal2sp"
        | "albersconicequalarea"
        | "albersequalarea" => (conic, &["lat_1", "lat_2"]),
        "lambertconformalconic1sp" | "lambertconicconformal1sp" => (origin_scale, &["lat_0"]),
        "mercator2sp"
        | "mercatorvariantb"
        | "polarstereographic"
        | "polarstereographicvariantb" => (parallel, &["lat_ts"]),
        "polarstereographicvarianta" => (origin_scale, &["lat_0"]),
        "transversemercator"
        | "mercator1sp"
        | "mercatorvarianta"
        | "stereographic"
        | "obliquestereographic"
        | "doublestereographic" => (origin_scale, &[]),
        "popularvisualisationpseudomercator" | "lambertazimuthalequalarea" => (origin, &[]),
        _ => {
            return Err(unsupported(
                "WKT projection method is outside the verified tier",
            ))
        }
    };
    if seen.iter().any(|key| !allowed.contains(key))
        || required.iter().any(|key| !seen.contains(key))
    {
        return Err(unsupported(
            "WKT parameters do not match the declared projection method",
        ));
    }
    Ok(())
}

fn wkt(definition: &str) -> Result<(String, f64), Error> {
    // Bound recursion before using the generic recursive parser, including LAS
    // EVLRs which can be much larger than a normal WKT declaration.
    let mut quoted = false;
    let mut depth = 0;
    for c in definition.chars() {
        match c {
            '"' => quoted = !quoted,
            '[' if !quoted => {
                depth += 1;
                if depth > 32 {
                    return Err(Error::Data("WKT CRS nesting exceeds 32 levels".into()));
                }
            }
            ']' if !quoted => {
                depth -= 1;
                if depth < 0 {
                    return Err(Error::Data("malformed WKT CRS".into()));
                }
            }
            _ => (),
        }
    }
    if quoted || depth != 0 {
        return Err(Error::Data("malformed WKT CRS".into()));
    }
    let root = proj4wkt::parser::parse(definition, &Tree).map_err(|error| {
        unsupported(&format!(
            "WKT syntax is outside the portable parser: {error}"
        ))
    })?;
    root.validate()?;
    // GDAL's canonical WKT1 Web Mercator uses a PROJ4 extension to distinguish
    // spherical projection maths from its WGS84 datum. Accept exactly that
    // definition; arbitrary extensions or null-grid datum suppression are not
    // safe substitutes for validating a CRS.
    let web_mercator = if let Some(extension) = root.child(&["EXTENSION"])? {
        const WEB_MERCATOR: &str = "+proj=merc +a=6378137 +b=6378137 +lat_ts=0 +lon_0=0 +x_0=0 +y_0=0 +k=1 +units=m +nadgrids=@null +wktext +no_defs";
        let tokens = |value: &str| {
            value
                .split_whitespace()
                .map(str::to_owned)
                .collect::<std::collections::BTreeSet<_>>()
        };
        if extension.attrs.len() != 2
            || extension.text(0)? != "PROJ4"
            || tokens(extension.text(1)?) != tokens(WEB_MERCATOR)
        {
            return Err(unsupported(
                "WKT extension is outside the verified Web Mercator definition",
            ));
        }
        true
    } else {
        false
    };
    let projected = matches!(root.key.as_str(), "PROJCS" | "PROJCRS");
    let geographic = if projected {
        root.required(&["GEOGCS", "BASEGEOGCRS", "BASEGEODCRS"])?
    } else if matches!(root.key.as_str(), "GEOGCS" | "GEOGCRS" | "GEODCRS") {
        &root
    } else {
        return Err(unsupported(
            "WKT is not a horizontal geographic or projected CRS",
        ));
    };
    let angular_units = coordinate_units(geographic, true)?;
    let datum = geographic.required(&["DATUM", "GEODETICDATUM", "ENSEMBLE"])?;
    let ellipsoid = datum.required(&["SPHEROID", "ELLIPSOID"])?;
    let a = ellipsoid.number(1)? * unit(ellipsoid, &["LENGTHUNIT"], 1.)?;
    let rf = ellipsoid.number(2)?;
    if a <= 0. || rf <= 0. {
        return Err(unsupported(
            "WKT ellipsoid must have positive radius and inverse flattening",
        ));
    }
    let mut result = format!("+a={a} +rf={rf}");
    if let Some(shift) = datum.child(&["TOWGS84"])? {
        if web_mercator {
            return Err(unsupported(
                "Web Mercator extension conflicts with a datum shift",
            ));
        }
        if !matches!(shift.attrs.len(), 3 | 7) {
            return Err(Error::Data("WKT TOWGS84 requires 3 or 7 parameters".into()));
        }
        let values = (0..shift.attrs.len())
            .map(|index| shift.number(index).map(|value| value.to_string()))
            .collect::<Result<Vec<_>, _>>()?;
        result.push_str(&format!(" +towgs84={}", values.join(",")));
    } else {
        let name = datum.text(0)?.to_ascii_lowercase().replace([' ', '_'], "");
        if !matches!(
            name.as_str(),
            "wgs84" | "wgs1984" | "worldgeodeticsystem1984" | "worldgeodeticsystem1984ensemble"
        ) || (a - 6378137.).abs() > 1e-8
            || (rf - 298.257223563).abs() > 1e-9
        {
            return Err(unsupported(
                "WKT datum is not verified WGS84 and has no explicit TOWGS84 Helmert shift",
            ));
        }
        result.push_str(" +datum=WGS84");
    }
    if let Some(pm) = geographic.child(&["PRIMEM"])? {
        let longitude = pm.number(1)? * unit(pm, &["ANGLEUNIT"], angular_units)?;
        result.push_str(&format!(" +pm={}", longitude.to_degrees()));
    }
    if projected {
        let linear_units = coordinate_units(&root, false)?;
        if web_mercator && linear_units != 1. {
            return Err(unsupported(
                "Web Mercator extension conflicts with WKT units",
            ));
        }
        let conversion = root.child(&["CONVERSION"])?.unwrap_or(&root);
        let method = conversion.required(&["PROJECTION", "METHOD"])?;
        let method_name = method
            .text(0)?
            .to_ascii_lowercase()
            .replace([' ', '_', '-', '(', ')'], "");
        let method_code = match method_name.as_str() {
            "transversemercator" => 9807,
            "mercator1sp" | "mercatorvarianta" => 9804,
            "mercator2sp" | "mercatorvariantb" => 9805,
            "popularvisualisationpseudomercator" => 1024,
            "lambertconformalconic1sp" | "lambertconicconformal1sp" => 9801,
            "lambertconformalconic2sp" | "lambertconicconformal2sp" => 9802,
            "albersconicequalarea" | "albersequalarea" => 9822,
            "polarstereographicvarianta" => 9810,
            "polarstereographicvariantb" | "polarstereographic" => 9829,
            "obliquestereographic" | "doublestereographic" => 9809,
            "lambertazimuthalequalarea" => 9820,
            // Ordinary stereographic has no verified EPSG method identity.
            _ => 0,
        };
        validate_wkt_identity(method, method_code)?;
        let mut projection = match method_name.as_str() {
            "transversemercator" => "etmerc",
            "mercator1sp" | "mercatorvarianta" | "mercator2sp" | "mercatorvariantb" => "merc",
            "popularvisualisationpseudomercator" => "webmerc",
            "lambertconformalconic2sp"
            | "lambertconicconformal2sp"
            | "lambertconformalconic1sp"
            | "lambertconicconformal1sp" => "lcc",
            "albersconicequalarea" | "albersequalarea" => "aea",
            "polarstereographicvarianta"
            | "polarstereographicvariantb"
            | "polarstereographic"
            | "stereographic" => "stere",
            "obliquestereographic" | "doublestereographic" => "sterea",
            "lambertazimuthalequalarea" => "laea",
            _ => {
                return Err(unsupported(
                    "WKT projection method is outside the verified tier",
                ))
            }
        };
        if web_mercator {
            if method.text(0)? != "Mercator_1SP" {
                return Err(unsupported(
                    "Web Mercator extension conflicts with the WKT projection method",
                ));
            }
            projection = "webmerc";
        }
        result.push_str(&format!(" +proj={projection} +to_meter={linear_units}"));
        let mut seen = std::collections::BTreeSet::new();
        let mut latitude_origin = 0.;
        let mut latitude_parallel = None;
        let polar_parallel = matches!(
            method_name.as_str(),
            "polarstereographic" | "polarstereographicvariantb"
        );
        for parameter in conversion.children().filter(|node| node.key == "PARAMETER") {
            let name = parameter
                .text(0)?
                .to_ascii_lowercase()
                .replace([' ', '_'], "");
            let (key, value) = match name.as_str() {
                "latitudeoforigin"
                | "latitudeofnaturalorigin"
                | "latitudeoffalseorigin"
                | "latitudeofcenter" => (
                    "lat_0",
                    (parameter.number(1)? * unit(parameter, &["ANGLEUNIT"], angular_units)?)
                        .to_degrees(),
                ),
                "centralmeridian"
                | "longitudeofnaturalorigin"
                | "longitudeoffalseorigin"
                | "longitudeoforigin"
                | "longitudeofcenter" => (
                    "lon_0",
                    (parameter.number(1)? * unit(parameter, &["ANGLEUNIT"], angular_units)?)
                        .to_degrees(),
                ),
                "standardparallel1" | "latitudeof1ststandardparallel" => (
                    "lat_1",
                    (parameter.number(1)? * unit(parameter, &["ANGLEUNIT"], angular_units)?)
                        .to_degrees(),
                ),
                "standardparallel2" | "latitudeof2ndstandardparallel" => (
                    "lat_2",
                    (parameter.number(1)? * unit(parameter, &["ANGLEUNIT"], angular_units)?)
                        .to_degrees(),
                ),
                "scalefactor" | "scalefactoratnaturalorigin" => (
                    "k_0",
                    parameter.number(1)? * unit(parameter, &["SCALEUNIT"], 1.)?,
                ),
                "falseeasting" | "eastingatfalseorigin" => (
                    "x_0",
                    parameter.number(1)? * unit(parameter, &["LENGTHUNIT"], linear_units)?,
                ),
                "falsenorthing" | "northingatfalseorigin" => (
                    "y_0",
                    parameter.number(1)? * unit(parameter, &["LENGTHUNIT"], linear_units)?,
                ),
                "latitudeofstandardparallel" => (
                    "lat_ts",
                    (parameter.number(1)? * unit(parameter, &["ANGLEUNIT"], angular_units)?)
                        .to_degrees(),
                ),
                _ => return Err(unsupported("WKT projection parameter would be discarded")),
            };
            let key =
                if (projection == "merc" && key == "lat_1") || (polar_parallel && key == "lat_0") {
                    "lat_ts"
                } else {
                    key
                };
            // WKT unit conversion can put an exact pole a few ulps outside
            // its domain. Normalize only that roundoff before validation.
            let value = if matches!(key, "lat_0" | "lat_1" | "lat_2" | "lat_ts")
                && value.abs() > 90.
                && value.abs() <= 90. + 1e-10
            {
                value.signum() * 90.
            } else {
                value
            };
            validate_projection_number(projection, key, value)?;
            validate_wkt_parameter_identity(
                &method_name,
                root.key == "PROJCS",
                parameter,
                &name,
                key,
            )?;
            if key == "lat_0" {
                latitude_origin = value;
            }
            if key == "lat_ts" {
                latitude_parallel = Some(value);
            }
            if !seen.insert(key) {
                return Err(unsupported(
                    "colliding WKT projection parameter aliases require native interpretation",
                ));
            }
            if web_mercator
                && !matches!(
                    (key, value),
                    ("lon_0" | "lat_0" | "x_0" | "y_0", 0.) | ("k_0", 1.)
                )
            {
                return Err(unsupported(
                    "Web Mercator extension conflicts with WKT parameters",
                ));
            }
            result.push_str(&format!(" +{key}={value}"));
        }
        // Check the declared method before injecting generic PROJ defaults.
        validate_wkt_parameters(&method_name, &seen)?;
        if matches!(method_name.as_str(), "mercator1sp" | "mercatorvarianta")
            && latitude_origin != 0.
        {
            return Err(unsupported(
                "Mercator variant A requires a zero latitude origin",
            ));
        }
        if method_name == "polarstereographicvarianta" && latitude_origin.abs() != 90. {
            return Err(unsupported(
                "polar stereographic variant A requires a polar latitude origin",
            ));
        }
        if matches!(
            method_name.as_str(),
            "lambertconformalconic1sp" | "lambertconicconformal1sp"
        ) {
            if seen.contains("lat_1") || seen.contains("lat_2") {
                return Err(unsupported(
                    "Lambert 1SP method conflicts with standard-parallel parameters",
                ));
            }
            result.push_str(&format!(" +lat_1={latitude_origin}"));
        }
        if polar_parallel {
            let parallel = latitude_parallel.ok_or_else(|| {
                Error::Data("polar stereographic CRS is missing its standard parallel".into())
            })?;
            if parallel == 0. {
                return Err(Error::Data(
                    "polar stereographic standard parallel must be nonzero".into(),
                ));
            }
            result.push_str(&format!(
                " +lat_0={}",
                if parallel > 0. { 90. } else { -90. }
            ));
        }
        // Validate method-specific parameters too, without using the lossy WKT
        // formatter or trusting a root AUTHORITY to override the definition.
        result = validate_proj(&result, true)?;
    } else {
        result.push_str(" +proj=longlat");
    }
    Ok((result, angular_units))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::georef::{geodetic_to_ecef, Cartographic};

    const WGS_WKT: &str = r#"GEOGCS["WGS 84",DATUM["WGS_1984",SPHEROID["WGS 84",6378137,298.257223563]],PRIMEM["Greenwich",0],UNIT["degree",0.0174532925199433]]"#;
    const UTM_WKT2: &str = r#"PROJCRS["WGS 84 / UTM zone 56S",BASEGEOGCRS["WGS 84",ENSEMBLE["World Geodetic System 1984 ensemble",MEMBER["World Geodetic System 1984 (Transit)"],MEMBER["World Geodetic System 1984 (G730)"],ELLIPSOID["WGS 84",6378137,298.257223563,LENGTHUNIT["metre",1]],ENSEMBLEACCURACY[2]],PRIMEM["Greenwich",0,ANGLEUNIT["degree",0.0174532925199433]]],CONVERSION["UTM zone 56S",METHOD["Transverse Mercator",ID["EPSG",9807]],PARAMETER["Latitude of natural origin",0,ANGLEUNIT["degree",0.0174532925199433]],PARAMETER["Longitude of natural origin",153,ANGLEUNIT["degree",0.0174532925199433]],PARAMETER["Scale factor at natural origin",0.9996,SCALEUNIT["unity",1]],PARAMETER["False easting",500000,LENGTHUNIT["metre",1]],PARAMETER["False northing",10000000,LENGTHUNIT["metre",1]]],CS[Cartesian,2],AXIS["easting (E)",east,ORDER[1],LENGTHUNIT["metre",1]],AXIS["northing (N)",north,ORDER[2],LENGTHUNIT["metre",1]],ID["EPSG",32756]]"#;
    const INCOMPLETE_LCC: &str = include_str!("../tests/fixtures/crs/lcc-2sp-missing-parallel.wkt");
    const MERCATOR_ALIASES: &str =
        include_str!("../tests/fixtures/crs/mercator-redundant-origin.wkt");
    const MICHIGAN_LCC: &str = include_str!("../tests/fixtures/crs/lcc-michigan-near-equator.wkt");
    const DMS_TMERC: &str = r#"+proj=tmerc +lon_0=2d20'14.025"E +datum=WGS84 +type=crs"#;

    fn projection_wkts(
        method1: &str,
        method2: &str,
        lat: f64,
        lon: f64,
        scale: Option<f64>,
    ) -> [String; 2] {
        let scale1 = scale
            .map(|scale| format!(r#",PARAMETER["scale_factor",{scale}]"#))
            .unwrap_or_default();
        let scale2 = scale
            .map(|scale| {
                format!(
                    r#",PARAMETER["Scale factor at natural origin",{scale},SCALEUNIT["unity",1]]"#
                )
            })
            .unwrap_or_default();
        [
            format!(
                r#"PROJCS["Invented projection",{WGS_WKT},PROJECTION["{method1}"],PARAMETER["latitude_of_origin",{lat}],PARAMETER["central_meridian",{lon}]{scale1},UNIT["metre",1],AXIS["Easting",EAST],AXIS["Northing",NORTH]]"#
            ),
            format!(
                r#"PROJCRS["Invented projection",BASEGEOGCRS["WGS 84",DATUM["World Geodetic System 1984",ELLIPSOID["WGS 84",6378137,298.257223563,LENGTHUNIT["metre",1]]],PRIMEM["Greenwich",0,ANGLEUNIT["degree",0.0174532925199433]]],CONVERSION["Invented conversion",METHOD["{method2}"],PARAMETER["Latitude of natural origin",{lat},ANGLEUNIT["degree",0.0174532925199433]],PARAMETER["Longitude of natural origin",{lon},ANGLEUNIT["degree",0.0174532925199433]]{scale2}],CS[Cartesian,2],AXIS["Easting",east,ORDER[1],LENGTHUNIT["metre",1]],AXIS["Northing",north,ORDER[2],LENGTHUNIT["metre",1]]]"#
            ),
        ]
    }

    fn additional_native_definitions() -> Vec<String> {
        let mut definitions = vec![
            "+proj=lcc +lat_1=33 +datum=WGS84".into(),
            "+proj=lcc +lat_1=33 +lat_2=45 +datum=WGS84".into(),
            "+proj=lcc +lat_1=49 +lat_0=49 +lon_0=10 +k=0.99 +datum=WGS84".into(),
            "+init=epsg:32632".into(),
            "+proj=utm +zone = 32 +datum=WGS84".into(),
            "+proj=utm +zone= 32 +datum=WGS84".into(),
            "+proj=utm +zone =32 +datum=WGS84".into(),
            format!(
                "GEOGCS({})",
                WGS_WKT
                    .strip_prefix("GEOGCS[")
                    .unwrap()
                    .strip_suffix(']')
                    .unwrap()
            ),
            "EPSG:32632@2020".into(),
            "EPSG:4326@2020".into(),
            r#"+proj=tmerc +k="0.9996" +datum=WGS84"#.into(),
            r#"+proj=tmerc +ellps=WGS84 +towgs84="1,2,3""#.into(),
            r#"+proj="tmerc" +datum=WGS84"#.into(),
            r#"+proj=tmerc +k_0="0.9996" +datum=WGS84"#.into(),
            r#"+proj=tmerc +datum=WGS84 +title="Invented local grid""#.into(),
            "+proj=tmerc +ellps=WGS84 +towgs84=0,0,0 +pm=0dE".into(),
            "+proj=tmerc +a=6371000 +b=6371000 +towgs84=0,0,0".into(),
            "+proj=tmerc +a=6371000 +f=0 +towgs84=0,0,0".into(),
            "+proj=sterea +lat_0=-90 +datum=WGS84".into(),
            "+proj=sterea +lat_0=90 +datum=WGS84".into(),
            "+proj=laea +lat_0=-15 +lon_0=135 +datum=WGS84".into(),
        ];
        for latitude in [-90., -89.99999999, -85., -80., 80., 85., 89.99999999, 90.] {
            definitions.push(format!("+proj=sterea +lat_0={latitude} +datum=WGS84"));
            definitions.extend(projection_wkts(
                "Oblique_Stereographic",
                "Oblique Stereographic",
                latitude,
                0.,
                Some(1.),
            ));
        }
        for latitude in [-89.99999999, -85., -80., 80., 85., 89.99999999] {
            definitions.push(format!("+proj=stere +lat_0={latitude} +datum=WGS84"));
            definitions.extend(projection_wkts(
                "Stereographic",
                "Stereographic",
                latitude,
                0.,
                Some(1.),
            ));
        }
        for projection in ["lcc", "aea"] {
            for first in [-80., -45., 45., 80.] {
                for separation in [1e-8, 1e-6, 0.01, 0.999999] {
                    let second = first + separation;
                    definitions.push(format!("+proj={projection} +lat_1={first} +lat_2={second} +lat_0={first} +datum=WGS84"));
                    if separation == 1e-8 {
                        definitions.extend(conic_wkts(projection, first, second));
                    }
                }
            }
        }
        for latitude in [-89.999999, 89.999999] {
            definitions.push(format!(
                "+proj=lcc +lat_1={latitude} +lat_2={latitude} +lat_0={latitude} +datum=WGS84"
            ));
            definitions.extend(conic_wkts("lcc", latitude, latitude));
        }
        // Same projection parameters as EPSG:10601 (GLANCE Oceania), whose
        // inverse LAEA in proj4rs exceeds the 1 mm ECEF accuracy threshold.
        definitions.extend(projection_wkts(
            "Lambert_Azimuthal_Equal_Area",
            "Lambert Azimuthal Equal Area",
            -15.,
            135.,
            None,
        ));
        definitions
    }

    fn conic_wkts(projection: &str, first: f64, second: f64) -> [String; 2] {
        let (method1, method2) = if projection == "lcc" {
            (
                "Lambert_Conformal_Conic_2SP",
                "Lambert Conic Conformal (2SP)",
            )
        } else {
            ("Albers_Conic_Equal_Area", "Albers Equal Area")
        };
        let origin = if projection == "aea" {
            first.clamp(-75., 75.)
        } else {
            first
        };
        let [mut wkt1, wkt2] = projection_wkts(method1, method2, origin, 0., None);
        if projection == "aea" {
            wkt1 = wkt1
                .replace("latitude_of_origin", "latitude_of_center")
                .replace("central_meridian", "longitude_of_center");
        }
        let wkt2 = wkt2
            .replace("Latitude of natural origin", "Latitude of false origin")
            .replace("Longitude of natural origin", "Longitude of false origin");
        [
            wkt1.replace(
                r#",UNIT["metre",1],AXIS"#,
                &format!(r#",PARAMETER["standard_parallel_1",{first}],PARAMETER["standard_parallel_2",{second}],UNIT["metre",1],AXIS"#),
            ),
            wkt2.replace(
                "],CS[Cartesian,2]",
                &format!(r#",PARAMETER["Latitude of 1st standard parallel",{first},ANGLEUNIT["degree",0.0174532925199433]],PARAMETER["Latitude of 2nd standard parallel",{second},ANGLEUNIT["degree",0.0174532925199433]]],CS[Cartesian,2]"#),
            ),
        ]
    }

    fn assert_mm(actual: [f64; 3], expected: [f64; 3], label: &str) {
        let distance = actual
            .iter()
            .zip(expected)
            .map(|(a, b)| (a - b).powi(2))
            .sum::<f64>()
            .sqrt();
        assert!(
            distance < 0.001,
            "{label}: ECEF difference {distance} m: {actual:?} != {expected:?}"
        );
    }

    fn incomplete_or_mismatched_wkts() -> Vec<String> {
        let mut definitions = vec![
            INCOMPLETE_LCC.to_owned(),
            MERCATOR_ALIASES.to_owned(),
            include_str!("../tests/fixtures/crs/mercator-wrong-parallel.wkt").to_owned(),
        ];
        for definition in conic_wkts("lcc", 33., 45.) {
            definitions.push(definition.replace(r#",PARAMETER["standard_parallel_2",45]"#, "")
                .replace(r#",PARAMETER["Latitude of 2nd standard parallel",45,ANGLEUNIT["degree",0.0174532925199433]]"#, ""));
            definitions.push(definition.replace(r#",UNIT["metre",1],AXIS"#, r#",PARAMETER["scale_factor",2],UNIT["metre",1],AXIS"#)
                .replace("],CS[Cartesian,2]", r#",PARAMETER["Scale factor at natural origin",2,SCALEUNIT["unity",1]]],CS[Cartesian,2]"#));
        }
        definitions
    }

    #[test]
    fn wkt_original_parameter_names_and_identities_keep_method_roles() {
        let [mercator1, mercator2] =
            projection_wkts("Mercator_2SP", "Mercator (variant B)", 30., 20., None).map(|wkt| {
                wkt.replace("latitude_of_origin", "standard_parallel_1")
                    .replace(
                        "Latitude of natural origin",
                        "Latitude of 1st standard parallel",
                    )
            });
        let mercator2 = mercator2.replace(
            r#"PARAMETER["Latitude of 1st standard parallel",30,ANGLEUNIT["degree",0.0174532925199433]]"#,
            r#"PARAMETER["Latitude of 1st standard parallel",30,ANGLEUNIT["degree",0.0174532925199433],ID["EPSG",8823]]"#,
        ).replace(r#"METHOD["Mercator (variant B)"]"#, r#"METHOD["Mercator (variant B)",ID["EPSG",9805]]"#);
        for definition in [&mercator1, &mercator2] {
            let mut operation = Transform::new(definition, 7.).unwrap();
            assert!(matches!(operation, Transform::Pure(_)));
            assert_mm(
                operation.transform(&[[0., 0., 123.]]).unwrap()[0],
                geodetic_to_ecef(Cartographic::new(20., 0., 130.)),
                "Mercator 2SP analytic origin",
            );
            #[cfg(feature = "native-geospatial")]
            {
                use crate::geospatial::{Crs, EcefTransform};
                let points = [[200000., 567890., 123.], [0., 0., 123.]];
                let expected =
                    EcefTransform::new(Crs::from_definition(definition).unwrap(), Some(7.))
                        .unwrap()
                        .transform(&points)
                        .unwrap();
                for (actual, expected) in operation
                    .transform(&points)
                    .unwrap()
                    .into_iter()
                    .zip(expected)
                {
                    assert_mm(actual, expected, definition);
                }
            }
        }
        let mut unverified = vec![
            mercator1.replace("standard_parallel_1", "Latitude of standard parallel"),
            mercator1.replace("standard_parallel_1", "Latitude of 1st standard parallel"),
            mercator2.replace(
                "Latitude of 1st standard parallel",
                "Latitude of standard parallel",
            ),
            mercator2.replace("Latitude of 1st standard parallel", "standard_parallel_1"),
            mercator2.replace("8823", "8832"),
            mercator2.replace("9805", "9829"),
            mercator2.replace("Longitude of natural origin", "Longitude of false origin"),
        ];
        for projection in ["lcc", "aea"] {
            let [wkt1, wkt2] = conic_wkts(projection, 33., 45.);
            unverified
                .push(wkt1.replace("standard_parallel_1", "Latitude of 1st standard parallel"));
            unverified.push(wkt2.replace("Latitude of false origin", "Latitude of natural origin"));
            unverified
                .push(wkt2.replace("Longitude of false origin", "Longitude of natural origin"));
            unverified.push(wkt2.replace(
                "Latitude of 1st standard parallel",
                "Latitude of standard parallel",
            ));
        }
        for definition in unverified {
            assert!(
                matches!(
                    PureTransform::new(&definition, 7.),
                    Err(Error::Environment(_))
                ),
                "{definition}"
            );
            #[cfg(not(feature = "native-geospatial"))]
            assert!(matches!(
                Transform::new(&definition, 7.),
                Err(Error::Environment(_))
            ));
        }
    }

    #[test]
    fn wkt_parameter_sets_keep_declared_projection_method_and_native_aliases() {
        for definition in incomplete_or_mismatched_wkts() {
            assert!(
                matches!(
                    PureTransform::new(&definition, 7.),
                    Err(Error::Environment(_))
                ),
                "{definition}"
            );
            #[cfg(not(feature = "native-geospatial"))]
            assert!(matches!(
                Transform::new(&definition, 7.),
                Err(Error::Environment(_))
            ));
            #[cfg(feature = "native-geospatial")]
            {
                use crate::geospatial::{Crs, EcefTransform};
                let points = [[200000., 567890., 123.], [5000., 6000., 123.]];
                let mut operation = Transform::new(&definition, 7.).unwrap();
                assert!(matches!(operation, Transform::Native(_)));
                let expected =
                    EcefTransform::new(Crs::from_definition(&definition).unwrap(), Some(7.))
                        .unwrap()
                        .transform(&points)
                        .unwrap();
                for (actual, expected) in operation
                    .transform(&points)
                    .unwrap()
                    .into_iter()
                    .zip(expected)
                {
                    assert_mm(actual, expected, &definition);
                }
            }
        }
        // A complete method remains portable, with its false origin checked
        // independently of either projection library.
        let complete = INCOMPLETE_LCC.replace(
            r#",PARAMETER["false_easting""#,
            r#",PARAMETER["standard_parallel_2",45],PARAMETER["false_easting""#,
        );
        let mut operation = Transform::new(&complete, 7.).unwrap();
        assert!(matches!(operation, Transform::Pure(_)));
        assert_mm(
            operation.transform(&[[5000., 6000., 123.]]).unwrap()[0],
            geodetic_to_ecef(Cartographic::new(20., 10., 130.)),
            "complete LCC analytic false origin",
        );
        // Variant-specific parameters must not be repurposed: these methods
        // cannot acquire conic parallels or a scale belonging to another variant.
        for (method1, method2, extra1, extra2) in [
            (
                "Lambert_Conformal_Conic_1SP",
                "Lambert Conic Conformal (1SP)",
                "standard_parallel_1",
                "Latitude of 1st standard parallel",
            ),
            (
                "Mercator_1SP",
                "Mercator (variant A)",
                "standard_parallel_1",
                "Latitude of 1st standard parallel",
            ),
            (
                "Albers_Conic_Equal_Area",
                "Albers Equal Area",
                "scale_factor",
                "Scale factor at natural origin",
            ),
            (
                "Stereographic",
                "Stereographic",
                "standard_parallel_1",
                "Latitude of 1st standard parallel",
            ),
        ] {
            for definition in projection_wkts(method1, method2, 0., 0., None) {
                let definition = definition
                    .replace(
                        r#",UNIT["metre",1],AXIS"#,
                        &format!(r#",PARAMETER["{extra1}",2],UNIT["metre",1],AXIS"#),
                    )
                    .replace(
                        "],CS[Cartesian,2]",
                        &format!(
                            r#",PARAMETER["{extra2}",2,SCALEUNIT["unity",1]]],CS[Cartesian,2]"#
                        ),
                    );
                assert!(
                    matches!(
                        PureTransform::new(&definition, 7.),
                        Err(Error::Environment(_))
                    ),
                    "{definition}"
                );
            }
        }
    }

    #[test]
    fn native_conic_variants_cannot_bypass_conditioning() {
        #[cfg(not(feature = "native-geospatial"))]
        assert!(matches!(
            Transform::new(MICHIGAN_LCC, 7.),
            Err(Error::Environment(_))
        ));
        #[cfg(feature = "native-geospatial")]
        {
            use crate::geospatial::Crs;
            // Include every native LCC family, not just Michigan's display name.
            for (method, code) in [
                ("Lambert Conic Conformal (2SP Michigan)", 1051),
                ("Lambert Conic Conformal (2SP)", 9802),
                ("Lambert Conic Conformal (2SP Belgium)", 9803),
            ] {
                let definition = MICHIGAN_LCC
                    .replace("Lambert Conic Conformal (2SP Michigan)", method)
                    .replace("1051", &code.to_string());
                let source = Crs::from_definition(&definition).unwrap();
                assert_eq!(source.conic_parallels().unwrap(), Some((1e-8, 1e-8)));
                assert!(
                    matches!(Transform::new(&definition, 7.), Err(Error::Data(_))),
                    "{definition}"
                );
            }
            let valid = MICHIGAN_LCC.replace("0.00000001", "33");
            let mut operation = Transform::new(&valid, 7.).unwrap();
            assert!(matches!(operation, Transform::Native(_)));
            assert_mm(
                operation.transform(&[[5000., 6000., 123.]]).unwrap()[0],
                geodetic_to_ecef(Cartographic::new(20., 10., 130.)),
                "Michigan analytic false origin",
            );
            let grads = MICHIGAN_LCC.replace("0.00000001", "0.00000002").replace(
                r#"ANGLEUNIT["degree",0.0174532925199433]"#,
                r#"ANGLEUNIT["grad",0.015707963267949]"#,
            );
            let (first, second) = Crs::from_definition(&grads)
                .unwrap()
                .conic_parallels()
                .unwrap()
                .unwrap();
            assert!((first - 1.8e-8).abs() < 1e-20 && (second - 1.8e-8).abs() < 1e-20);
            assert!(matches!(Transform::new(&grads, 7.), Err(Error::Data(_))));
        }
    }

    #[test]
    fn spherical_albers_polar_point_is_refused_in_both_builds() {
        let definition = "+proj=aea +lat_1=80 +lat_2=80 +lat_0=80 +ellps=sphere +towgs84=0,0,0";
        let point = [612606.1867269421, 13949033.598816177, 123.];
        assert!(matches!(
            PureTransform::new(definition, 7.)
                .unwrap()
                .transform(&[point]),
            Err(Error::Data(_))
        ));
        let mut operation = Transform::new(definition, 7.).unwrap();
        assert!(matches!(operation.transform(&[point]), Err(Error::Data(_))));
        #[cfg(feature = "native-geospatial")]
        for definition in [
            definition.replace("+lat_1=80", "+lat_1=\"80\""),
            crate::geospatial::Crs::from_definition(definition)
                .unwrap()
                .wkt()
                .unwrap(),
        ] {
            let mut operation = Transform::new(&definition, 7.).unwrap();
            assert!(matches!(operation, Transform::Native(_)));
            assert!(
                matches!(operation.transform(&[point]), Err(Error::Data(_))),
                "{definition}"
            );
        }
    }

    #[test]
    fn ellipsoidal_albers_uses_independent_forward_and_ecef_references() {
        // Direct Albers forward equations, independent of both inverse
        // libraries: matching their shared polar clamp would hide the error.
        fn forward(a: f64, rf: f64, sign: f64, lon: f64, lat: f64) -> [f64; 3] {
            let flattening = 1. / rf;
            let e2 = flattening * (2. - flattening);
            let e = e2.sqrt();
            let q = |phi: f64| {
                let sin = phi.to_radians().sin();
                (1. - e2)
                    * (sin / (1. - e2 * sin * sin)
                        - ((1. - e * sin) / (1. + e * sin)).ln() / (2. * e))
            };
            let m2 = |phi: f64| {
                let (sin, cos) = phi.to_radians().sin_cos();
                cos * cos / (1. - e2 * sin * sin)
            };
            let n = (m2(sign * 33.) - m2(sign * 45.)) / (q(sign * 45.) - q(sign * 33.));
            let c = m2(sign * 33.) + n * q(sign * 33.);
            let rho = a * (c - n * q(lat)).sqrt() / n;
            let rho0 = a * (c - n * q(sign * 10.)).sqrt() / n;
            let (sin, cos) = (n * lon.to_radians()).sin_cos();
            [rho * sin, rho0 - rho * cos, 123.]
        }
        let reported = forward(6378137., 298.257223563, 1., 0., 89.99);
        assert!((reported[1] - 7279556.2302497085).abs() < 1e-8);
        let analytic = geodetic_to_ecef(Cartographic::new(0., 89.99, 130.));
        assert_mm(
            analytic,
            [1116.9624790864507, 0., 6356882.216771815],
            "reported analytic reference",
        );
        for (datum, a, rf, shift) in [
            ("+datum=WGS84", 6378137., 298.257223563, [0.; 3]),
            (
                "+ellps=airy +towgs84=12,-34,56",
                6377563.396,
                299.3249646,
                [12., -34., 56.],
            ),
        ] {
            for sign in [-1., 1.] {
                for (units, factor) in [
                    ("m", 1.),
                    ("ft", 0.3048),
                    ("us-ft", 1200. / 3937.),
                    ("km", 1000.),
                ] {
                    let definition = format!(
                        "+proj=aea +lat_1={} +lat_2={} +lat_0={} {datum} +units={units}",
                        sign * 33.,
                        sign * 45.,
                        sign * 10.
                    );
                    let definitions = vec![definition.clone()];
                    #[cfg(feature = "native-geospatial")]
                    let definitions = {
                        let mut definitions = definitions;
                        definitions.extend([
                            definition.replace("+proj=aea", "+proj=\"aea\""),
                            crate::geospatial::Crs::from_definition(&definition)
                                .unwrap()
                                .wkt()
                                .unwrap(),
                        ]);
                        if datum == "+datum=WGS84" && units == "m" {
                            let wkt2 = conic_wkts("aea", sign * 33., sign * 45.)[1].replace(
                                &format!(r#"PARAMETER["Latitude of false origin",{}"#, sign * 33.),
                                &format!(r#"PARAMETER["Latitude of false origin",{}"#, sign * 10.),
                            );
                            definitions.push(wkt2);
                        }
                        definitions
                    };
                    for lat in [
                        -90_f64, -89.99, -85., -80.000001, -79.999999, -45., 0., 45., 79.999999,
                        80.000001, 85., 89.99, 90.,
                    ] {
                        for lon in [-45., 0., 45.] {
                            let mut point = forward(a, rf, sign, lon, lat);
                            point[0] /= factor;
                            point[1] /= factor;
                            for definition in &definitions {
                                let mut operation = Transform::new(definition, 7.).unwrap();
                                #[cfg(feature = "native-geospatial")]
                                if definition != &definitions[0] {
                                    // Exercise the guard after native WKT parsing too,
                                    // even when the same WKT is also portable.
                                    operation = Transform::native(definition, 7.).unwrap();
                                    assert!(matches!(operation, Transform::Native(_)));
                                }
                                if lat.abs() > 80. {
                                    assert!(
                                        matches!(
                                            operation.transform(&[point]),
                                            Err(Error::Data(_))
                                        ),
                                        "{definition}: {lon}, {lat}"
                                    );
                                } else {
                                    let flattening = 1. / rf;
                                    let e2 = flattening * (2. - flattening);
                                    let (sin_lat, cos_lat) = lat.to_radians().sin_cos();
                                    let (sin_lon, cos_lon) = lon.to_radians().sin_cos();
                                    let n = a / (1. - e2 * sin_lat * sin_lat).sqrt();
                                    assert_mm(
                                        operation.transform(&[point]).unwrap()[0],
                                        [
                                            (n + 130.) * cos_lat * cos_lon + shift[0],
                                            (n + 130.) * cos_lat * sin_lon + shift[1],
                                            (n * (1. - e2) + 130.) * sin_lat + shift[2],
                                        ],
                                        &format!("{definition}: {lon}, {lat}"),
                                    );
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    #[cfg(feature = "native-geospatial")]
    #[test]
    fn spherical_albers_coordinate_matrix_uses_fresh_transforms() {
        use crate::geospatial::{Crs, EcefTransform, StrictTransform};
        for origin in [-80., 30., 80.] {
            let definition = format!("+proj=aea +lat_1={origin} +lat_2={origin} +lat_0={origin} +ellps=sphere +towgs84=0,0,0");
            let source = Crs::from_definition(&definition).unwrap();
            let geographic =
                Crs::from_definition("+proj=longlat +ellps=sphere +towgs84=0,0,0").unwrap();
            let mut forward = StrictTransform::new(&geographic, &source).unwrap();
            let mut expected = EcefTransform::new(source, Some(7.)).unwrap();
            for latitude in [
                -90_f64, -89.99999, -85., -79.999999, -45., 0., 45., 79.999999, 85., 89.99999, 90.,
            ] {
                for longitude in [-45_f64, 0., 45.] {
                    let point = forward.transform(&[[longitude, latitude, 123.]]).unwrap()[0];
                    let quoted = definition
                        .replace(&format!("+lat_1={origin}"), &format!("+lat_1=\"{origin}\""));
                    let mut native = Transform::new(&quoted, 7.).unwrap();
                    assert!(matches!(native, Transform::Native(_)));
                    if latitude.abs() > 80. {
                        assert!(matches!(native.transform(&[point]), Err(Error::Data(_))));
                    } else {
                        assert_mm(
                            native.transform(&[point]).unwrap()[0],
                            expected.transform(&[point]).unwrap()[0],
                            "native spherical Albers interior after domain check",
                        );
                    }
                    let mut operation = Transform::new(&definition, 7.).unwrap();
                    if latitude.abs() > 80. {
                        assert!(matches!(operation.transform(&[point]), Err(Error::Data(_))));
                    } else {
                        let actual = operation.transform(&[point]).unwrap()[0];
                        assert_mm(
                            actual,
                            expected.transform(&[point]).unwrap()[0],
                            &format!("{definition}: {longitude}, {latitude}"),
                        );
                        assert!(matches!(operation, Transform::Pure(_)));
                        let radius = 6370997. + 130.;
                        let (sin_lat, cos_lat) = latitude.to_radians().sin_cos();
                        let (sin_lon, cos_lon) = longitude.to_radians().sin_cos();
                        assert_mm(
                            actual,
                            [
                                radius * cos_lat * cos_lon,
                                radius * cos_lat * sin_lon,
                                radius * sin_lat,
                            ],
                            "analytic spherical Albers interior",
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn geographic_wkt_degrees_and_grads_preserve_metre_height() {
        for (definition, point) in [
            ("EPSG:4326".to_owned(), [153., -27., 123.]),
            (WGS_WKT.to_owned(), [153., -27., 123.]),
            (
                WGS_WKT.replace("degree\",0.0174532925199433", "grad\",0.015707963267948967"),
                [170., -30., 123.],
            ),
        ] {
            let transform = PureTransform::new(&definition, 7.).unwrap();
            assert_mm(
                transform.transform(&[point]).unwrap()[0],
                geodetic_to_ecef(Cartographic::new(153., -27., 130.)),
                &definition,
            );
            assert!(transform.transform(&[]).unwrap().is_empty());
        }
    }

    #[test]
    fn wkt2_utm_and_horizontal_feet_keep_explicit_height_in_metres() {
        for (definition, point) in [
            (UTM_WKT2, [500000., 10000000., 123.]),
            (
                "+proj=utm +zone=56 +south +datum=WGS84 +units=ft",
                [500000. / 0.3048, 10000000. / 0.3048, 123.],
            ),
            (
                "+proj=utm +zone=56 +south +datum=WGS84 +units=us-ft",
                [500000. * 3937. / 1200., 10000000. * 3937. / 1200., 123.],
            ),
        ] {
            assert_mm(
                PureTransform::new(definition, 7.)
                    .unwrap()
                    .transform(&[point])
                    .unwrap()[0],
                geodetic_to_ecef(Cartographic::new(153., 0., 130.)),
                definition,
            );
        }
    }

    #[test]
    fn explicit_helmert_translation_is_applied_in_ecef() {
        let definition = "+proj=longlat +ellps=WGS84 +towgs84=12,-34,56";
        let actual = PureTransform::new(definition, 7.)
            .unwrap()
            .transform(&[[153., -27., 123.]])
            .unwrap()[0];
        let mut expected = geodetic_to_ecef(Cartographic::new(153., -27., 130.));
        for (value, shift) in expected.iter_mut().zip([12., -34., 56.]) {
            *value += shift;
        }
        assert_mm(actual, expected, definition);
        let wkt = WGS_WKT.replace(
            "DATUM[\"WGS_1984\",",
            "DATUM[\"Invented datum\",TOWGS84[12,-34,56],",
        );
        assert_mm(
            PureTransform::new(&wkt, 7.)
                .unwrap()
                .transform(&[[153., -27., 123.]])
                .unwrap()[0],
            expected,
            &wkt,
        );
    }

    #[test]
    fn grids_unknown_datums_and_lossy_definitions_are_refused() {
        for definition in [
            "+proj=longlat +datum=WGS84 +geoidgrids=missing.gtx".to_owned(),
            "+proj=longlat +datum=WGS84 +nadgrids=@missing.gsb,@null".to_owned(),
            "+proj=longlat +datum=NAD27".to_owned(),
            "+proj=longlat +datum=NAD83".to_owned(),
            "+proj=longlat +ellps=WGS84".to_owned(),
            "+proj=longlat +datum=WGS84 +ellps=clrk66".to_owned(),
            "+proj=longlat +datum=WGS84 +vunits=ft".to_owned(),
            "+proj=longlat +datum=WGS84 +t_epoch=2020".to_owned(),
            "+proj=longlat +datum=WGS84 +pm=paris".to_owned(),
            "EPSG:27700".to_owned(),
            "EPSG:4326+5773".to_owned(),
            WGS_WKT.replace("WGS_1984", "NAD83"),
            WGS_WKT.replace("PRIMEM[", "EXTENSION[\"PROJ4_GRIDS\",\"missing.gsb\"],PRIMEM["),
            WGS_WKT.replace("PRIMEM[", "TOWGS84[1,2,3],PRIMEM["),
            UTM_WKT2.replace("AXIS[\"easting (E)\",east", "AXIS[\"easting (E)\",north"),
            UTM_WKT2.replace("CS[Cartesian,2]", "CS[Cartesian,2],LENGTHUNIT[\"metre\",1]")
                .replace("AXIS[\"easting (E)\",east,ORDER[1],LENGTHUNIT[\"metre\",1]]", "AXIS[\"easting (E)\",east,ORDER[1],LENGTHUNIT[\"foot\",0.3048]]")
                .replace("AXIS[\"northing (N)\",north,ORDER[2],LENGTHUNIT[\"metre\",1]]", "AXIS[\"northing (N)\",north,ORDER[2]]"),
            format!("COMPD_CS[\"WGS84 + geoid\",{WGS_WKT},VERT_CS[\"Geoid\",VERT_DATUM[\"Geoid\",2005],UNIT[\"metre\",1]]]"),
        ] {
            let error = PureTransform::new(&definition, 0.).err().unwrap();
            assert!(error.to_string().contains("--features native-geospatial"), "{definition}: {error}");
        }
    }

    #[test]
    fn malformed_parameters_and_invalid_coordinates_fail_without_partial_results() {
        for definition in [
            "+proj=longlat +datum=WGS84 +towgs84=1,2,NaN",
            "+proj=longlat +datum=WGS84 +towgs84=1,2",
            "+proj=utm +zone=32 +zone=33 +datum=WGS84",
            "+proj=utm +zone=61 +datum=WGS84",
            "GEOGCS[\"broken\"",
        ] {
            assert!(PureTransform::new(definition, 0.).is_err(), "{definition}");
        }
        let transform = PureTransform::new("EPSG:4326", 0.).unwrap();
        for invalid in [[181., 0., 0.], [0., 91., 0.], [0., 0., f64::NAN]] {
            let input = [[0., 0., 0.], invalid];
            assert!(transform.transform(&input).is_err());
            assert_eq!(input[0], [0., 0., 0.]);
        }
        assert!(PureTransform::new("EPSG:4326", f64::INFINITY).is_err());
    }

    #[test]
    fn invalid_scales_and_missing_utm_zone_are_data_errors() {
        let mut definitions = Vec::new();
        for key in ["k", "k_0"] {
            for value in ["0", "-1", "NaN", "inf"] {
                definitions.push(format!("+proj=stere +lat_0=45 +{key}={value} +datum=WGS84"));
            }
        }
        definitions.push("+proj=utm +datum=WGS84".into());
        definitions.push("+proj=utm +lon_0=10 +datum=WGS84".into());
        for scale in [0., -1.] {
            definitions.extend(projection_wkts(
                "Transverse_Mercator",
                "Transverse Mercator",
                0.,
                3.,
                Some(scale),
            ));
        }
        for definition in definitions {
            assert!(
                matches!(Transform::new(&definition, 7.), Err(Error::Data(_))),
                "{definition}"
            );
        }
        for definition in [
            "+proj=stere +lat_0=45 +k=1 +datum=WGS84",
            "EPSG: 32632",
            "+proj=utm +zone=32 +datum=WGS84",
            "+proj=tmerc +pm=greenwich +datum=WGS84",
        ] {
            assert!(
                matches!(Transform::new(definition, 7.), Ok(Transform::Pure(_))),
                "{definition}"
            );
        }
    }

    #[test]
    fn projection_latitude_domains_and_units_are_validated() {
        let mut definitions = Vec::new();
        for (projection, key, other) in [
            ("tmerc", "lat_0", ""),
            ("utm", "lat_0", "+zone=32"),
            ("stere", "lat_0", ""),
            ("sterea", "lat_0", ""),
            ("merc", "lat_ts", ""),
            ("stere", "lat_ts", "+lat_0=90"),
            ("aea", "lat_1", "+lat_2=45"),
            ("aea", "lat_2", "+lat_1=45"),
            ("lcc", "lat_1", "+lat_2=45"),
            ("lcc", "lat_2", "+lat_1=45"),
        ] {
            for latitude in [-100., 100.] {
                definitions.push(format!(
                    "+proj={projection} +{key}={latitude} {other} +datum=WGS84"
                ));
            }
        }
        for latitude in [-90., -89.999999999, 89.999999999, 90.] {
            for projection in [
                format!("+proj=lcc +lat_1={latitude} +lat_2=45"),
                format!("+proj=lcc +lat_1=45 +lat_2={latitude}"),
            ] {
                definitions.push(format!("{projection} +datum=WGS84"));
            }
            definitions.extend(projection_wkts(
                "Lambert_Conformal_Conic_1SP",
                "Lambert Conic Conformal (1SP)",
                latitude,
                0.,
                Some(1.),
            ));
        }
        for latitude in [-90., 90.] {
            definitions.push(format!("+proj=merc +lat_ts={latitude} +datum=WGS84"));
        }
        for latitude in [-100., 100.] {
            definitions.extend(projection_wkts(
                "Transverse_Mercator",
                "Transverse Mercator",
                latitude,
                0.,
                Some(1.),
            ));
            definitions.extend(
                projection_wkts(
                    "Albers_Conic_Equal_Area",
                    "Albers Equal Area",
                    latitude,
                    0.,
                    None,
                )
                .map(|wkt| {
                    wkt.replace("latitude_of_origin", "standard_parallel_1")
                        .replace(
                            "Latitude of natural origin",
                            "Latitude of 1st standard parallel",
                        )
                }),
            );
        }
        for latitude in [-100., -90., 90., 100.] {
            definitions.extend(
                projection_wkts("Mercator_2SP", "Mercator (variant B)", latitude, 0., None).map(
                    |wkt| {
                        wkt.replace("latitude_of_origin", "standard_parallel_1")
                            .replace(
                                "Latitude of natural origin",
                                "Latitude of 1st standard parallel",
                            )
                    },
                ),
            );
        }
        for projection in ["utm +zone=32", "tmerc", "merc", "lcc +lat_1=33 +lat_2=45"] {
            for units in ["degrees", "rad", "grad"] {
                definitions.push(format!("+proj={projection} +datum=WGS84 +units={units}"));
            }
        }
        for definition in definitions {
            let error = Transform::new(&definition, 7.).err().expect(&definition);
            assert!(matches!(error, Error::Data(_)), "{definition}: {error}");
            #[cfg(feature = "native-geospatial")]
            // PROJ ignores out-of-domain lat_ts for stere; the portable tier
            // still refuses those physically invalid standard parallels.
            if !definition.starts_with("+proj=stere +lat_ts=") {
                assert!(
                    crate::geospatial::Crs::from_definition(&definition)
                        .and_then(|crs| crate::geospatial::EcefTransform::new(crs, Some(7.)))
                        .and_then(|mut operation| operation.transform(&[[1000., 2000., 123.]]))
                        .is_err(),
                    "native PROJ should reject {definition}"
                );
            }
        }
    }

    #[test]
    fn conflicting_polar_scales_and_ill_conditioned_conics_are_refused() {
        let mut definitions = Vec::new();
        for origin in [-90_f64, 90.] {
            for parallel in [-90., -70., 70., 90.] {
                if origin == parallel {
                    continue;
                }
                for key in ["k", "k_0"] {
                    for scale in [0.99, 2.] {
                        definitions.push(format!("+proj=stere +lat_0={origin} +lat_ts={parallel} +{key}={scale} +datum=WGS84"));
                    }
                }
            }
        }
        for projection in ["lcc", "aea"] {
            for (first, second) in [
                (30., -29.99999999),
                (-30., 29.99999999),
                (0.1, 0.1),
                (-0.1, -0.1),
            ] {
                definitions.push(format!(
                    "+proj={projection} +lat_1={first} +lat_2={second} +lat_0=0 +datum=WGS84"
                ));
                definitions.extend(conic_wkts(projection, first, second));
            }
        }
        for definition in definitions {
            let error = Transform::new(&definition, 7.).err().expect(&definition);
            assert!(matches!(error, Error::Data(_)), "{definition}: {error}");
        }
        // Native parsing must not let alternate syntax bypass the conic limit.
        for definition in [
            r#"+proj=aea +lat_1="30" +lat_2="-29.99999999" +datum=WGS84"#,
            "+proj=aea +lat_1 = 30 +lat_2 = -29.99999999 +datum=WGS84",
        ] {
            let error = Transform::new(definition, 7.).err().expect(definition);
            #[cfg(feature = "native-geospatial")]
            assert!(matches!(error, Error::Data(_)), "{definition}: {error}");
            #[cfg(not(feature = "native-geospatial"))]
            assert!(
                matches!(error, Error::Environment(_)),
                "{definition}: {error}"
            );
        }
        for definition in [
            "+proj=stere +lat_0=90 +lat_ts=70 +k=1 +datum=WGS84",
            "+proj=stere +lat_0=-90 +lat_ts=-90 +k_0=0.99 +datum=WGS84",
            "+proj=stere +lat_0=-90 +k=0.99 +datum=WGS84",
        ] {
            assert!(matches!(
                Transform::new(definition, 7.),
                Ok(Transform::Pure(_))
            ));
        }
    }

    #[test]
    fn opposite_sign_polar_stereographic_parameters_keep_native_hemisphere() {
        for origin in [-90_f64, 90.] {
            for parallel in [-origin, -origin.signum() * 70., 0.] {
                let definition =
                    format!("+proj=stere +lat_0={origin} +lat_ts={parallel} +datum=WGS84");
                assert!(matches!(
                    PureTransform::new(&definition, 7.),
                    Err(Error::Environment(_))
                ));
                #[cfg(not(feature = "native-geospatial"))]
                assert!(matches!(
                    Transform::new(&definition, 7.),
                    Err(Error::Environment(_))
                ));
                #[cfg(feature = "native-geospatial")]
                {
                    let mut operation = Transform::new(&definition, 7.).unwrap();
                    assert!(matches!(operation, Transform::Native(_)));
                    assert_mm(
                        operation.transform(&[[0., 0., 123.]]).unwrap()[0],
                        geodetic_to_ecef(Cartographic::new(
                            0.,
                            if parallel < 0. { -90. } else { 90. },
                            130.,
                        )),
                        "native polar hemisphere at analytic origin",
                    );
                }
            }
        }
    }

    #[cfg(feature = "native-geospatial")]
    #[test]
    fn native_lcc_variant_b_checks_natural_origin_instead_of_false_origin() {
        use crate::geospatial::Crs;
        for latitude in [-33., -0.1, -1e-8, 1e-8, 0.1, 33.] {
            let definition = format!("+proj=lcc +lat_1=\"{latitude}\" +lat_0=5 +datum=WGS84");
            let source = Crs::from_definition(&definition).unwrap();
            assert_eq!(
                source.conic_parallels().unwrap(),
                Some((latitude, latitude))
            );
            if latitude.abs() < 0.5 {
                let extra_parallel = source.wkt().unwrap().replace("],CS[Cartesian,2]",
                    r#",PARAMETER["Latitude of 1st standard parallel",33,ANGLEUNIT["degree",0.0174532925199433]]],CS[Cartesian,2]"#);
                let source = Crs::from_definition(&extra_parallel).unwrap();
                assert_eq!(
                    source.conic_parallels().unwrap(),
                    Some((latitude, latitude))
                );
                assert!(matches!(
                    Transform::new(&extra_parallel, 7.),
                    Err(Error::Data(_))
                ));
            }
            for definition in [definition, source.wkt().unwrap()] {
                if latitude.abs() < 0.5 {
                    assert!(
                        matches!(Transform::new(&definition, 7.), Err(Error::Data(_))),
                        "{definition}"
                    );
                } else {
                    let mut operation = Transform::new(&definition, 7.).unwrap();
                    assert!(matches!(operation, Transform::Native(_)));
                    assert_mm(
                        operation.transform(&[[0., 0., 123.]]).unwrap()[0],
                        geodetic_to_ecef(Cartographic::new(0., 5., 130.)),
                        "well-conditioned variant B at analytic false origin",
                    );
                }
            }
        }
    }

    #[test]
    fn stereographic_polar_points_and_failed_probes_retry_whole_batch() {
        let mut cases = vec![
            (
                "+proj=sterea +lat_0=45 +datum=WGS84".to_owned(),
                [0.9141389405141953, 5290076.530007198, 123.],
            ),
            (
                "+proj=sterea +lat_0=79.999999 +ellps=airy +towgs84=12,-34,56".to_owned(),
                [0.00112530269295305, 1119554.6907156024, 123.],
            ),
            (
                "+proj=sterea +lat_0=-79.999999 +ellps=airy +towgs84=12,-34,56".to_owned(),
                [0.00112530269295305, -1119554.6907156024, 123.],
            ),
        ];
        for origin in [-75_f64, 75.] {
            let point = [0., origin.signum() * 1685039.1152903102, 123.];
            cases.push((format!("+proj=stere +lat_0={origin} +datum=WGS84"), point));
            cases.extend(
                projection_wkts("Stereographic", "Stereographic", origin, 0., None)
                    .map(|definition| (definition, point)),
            );
        }
        for (definition, polar_point) in cases {
            let points = [[0., 0., 123.], polar_point];
            // Fresh transforms per case and order: an earlier fallback must not
            // mask a failed probe or a portable point-domain violation.
            for points in [points, [polar_point, points[0]]] {
                let mut operation = Transform::new(&definition, 7.).unwrap();
                assert!(matches!(operation, Transform::Pure(_)), "{definition}");
                assert!(
                    matches!(
                        PureTransform::new(&definition, 7.)
                            .unwrap()
                            .transform(&points),
                        Err(Error::Environment(_))
                    ),
                    "{definition}"
                );
                #[cfg(not(feature = "native-geospatial"))]
                {
                    assert!(matches!(
                        operation.transform(&points),
                        Err(Error::Environment(_))
                    ));
                    assert!(matches!(operation, Transform::Pure(_)));
                }
                #[cfg(feature = "native-geospatial")]
                {
                    use crate::geospatial::{Crs, EcefTransform};
                    let mut expected =
                        EcefTransform::new(Crs::from_definition(&definition).unwrap(), Some(7.))
                            .unwrap();
                    // Exact pole inverses may be refused by native PROJ on
                    // another platform. Verify its outcome rather than forcing
                    // success or publishing a partial portable batch.
                    match (operation.transform(&points), expected.transform(&points)) {
                        (Ok(actual), Ok(expected)) => {
                            for (actual, expected) in actual.into_iter().zip(expected) {
                                assert_mm(actual, expected, &definition);
                            }
                            assert!(matches!(operation, Transform::Native(_)));
                        }
                        (Err(actual), Err(expected)) => {
                            assert_eq!(actual.category(), expected.category(), "{definition}");
                            assert!(matches!(expected, Error::Data(_)), "{expected}");
                            assert!(matches!(operation, Transform::Pure(_)));
                        }
                        _ => panic!("native retry outcome differs for {definition}"),
                    }
                    let subsequent = [[0., 0., 123.]];
                    assert_mm(
                        operation.transform(&subsequent).unwrap()[0],
                        expected.transform(&subsequent).unwrap()[0],
                        "subsequent native batch",
                    );
                }
            }
            // Also exercise the point alone, before any successful fallback.
            let mut invalid = Transform::new(&definition, 7.).unwrap();
            assert!(matches!(
                invalid.transform(&[[f64::NAN, 0., 123.]]),
                Err(Error::Data(_))
            ));
            assert!(matches!(invalid, Transform::Pure(_)));
            let mut operation = Transform::new(&definition, 7.).unwrap();
            let result = operation.transform(&[polar_point]);
            #[cfg(not(feature = "native-geospatial"))]
            assert!(matches!(result, Err(Error::Environment(_))));
            #[cfg(feature = "native-geospatial")]
            {
                use crate::geospatial::{Crs, EcefTransform};
                let expected =
                    EcefTransform::new(Crs::from_definition(&definition).unwrap(), Some(7.))
                        .unwrap()
                        .transform(&[polar_point]);
                match (result, expected) {
                    (Ok(actual), Ok(expected)) => {
                        assert_mm(actual[0], expected[0], "fresh transform at polar point");
                        assert!(matches!(operation, Transform::Native(_)));
                    }
                    (Err(actual), Err(expected)) => {
                        assert_eq!(actual.category(), expected.category(), "{definition}");
                        assert!(matches!(expected, Error::Data(_)), "{expected}");
                        assert!(matches!(operation, Transform::Pure(_)));
                    }
                    _ => panic!("native retry outcome differs for {definition}"),
                }
            }
        }
    }

    #[cfg(feature = "native-geospatial")]
    #[test]
    fn stereographic_coordinate_matrix_uses_fresh_transforms_per_point() {
        use crate::geospatial::{Crs, EcefTransform, StrictTransform};
        for projection in ["stere", "sterea"] {
            for origin in [-79.999999, -75., -45., 0., 45., 75., 79.999999] {
                for datum in ["+datum=WGS84", "+ellps=airy +towgs84=12,-34,56"] {
                    let definition =
                        format!("+proj={projection} +lat_0={origin} +lon_0=10 {datum}");
                    let source = Crs::from_definition(&definition).unwrap();
                    let geographic =
                        Crs::from_definition(&format!("+proj=longlat {datum}")).unwrap();
                    let mut forward = StrictTransform::new(&geographic, &source).unwrap();
                    let mut expected = EcefTransform::new(source, Some(7.)).unwrap();
                    for latitude in [
                        -89.99999_f64,
                        -85.,
                        -80.,
                        -79.999999,
                        -75.,
                        -45.,
                        0.,
                        45.,
                        75.,
                        79.999999,
                        80.,
                        85.,
                        89.99999,
                    ] {
                        for longitude in [-45., 10., 45.] {
                            let point =
                                forward.transform(&[[longitude, latitude, 123.]]).unwrap()[0];
                            let mut operation = Transform::new(&definition, 7.).unwrap();
                            let result = operation.transform(&[point]).unwrap();
                            assert_mm(
                                result[0],
                                expected.transform(&[point]).unwrap()[0],
                                &format!("{definition}: {longitude}, {latitude}"),
                            );
                            if latitude.abs() > 80. {
                                assert!(
                                    matches!(operation, Transform::Native(_)),
                                    "{definition}: {latitude}"
                                );
                            } else if latitude.abs() < 80. - 1e-6 {
                                assert!(
                                    matches!(operation, Transform::Pure(_)),
                                    "{definition}: {latitude}"
                                );
                            }
                        }
                    }
                }
            }
        }
    }

    #[cfg(feature = "native-geospatial")]
    #[test]
    fn portable_oblique_stereographic_origin_range_matches_strict_native() {
        use crate::geospatial::{Crs, EcefTransform};
        for latitude in [
            -79.999999, -75., -60., -52., -30., 0., 30., 52., 60., 75., 79.999999,
        ] {
            let direction = if latitude < 0. { 1. } else { -1. };
            let points = [
                [0., 0., 123.],
                [1000., direction * 2000., 123.],
                [-300000., direction * 700000., -30.],
                [3000000., direction * 3000000., 500.],
            ];
            for longitude in [0., 123.] {
                for datum in ["+datum=WGS84", "+ellps=airy +towgs84=12,-34,56"] {
                    let definition = format!(
                        "+proj=sterea +lat_0={latitude} +lon_0={longitude} +k=0.9996 {datum}"
                    );
                    let mut operation = Transform::new(&definition, 7.).unwrap();
                    assert!(matches!(operation, Transform::Pure(_)), "{definition}");
                    let expected =
                        EcefTransform::new(Crs::from_definition(&definition).unwrap(), Some(7.))
                            .unwrap()
                            .transform(&points)
                            .unwrap();
                    for (actual, expected) in operation
                        .transform(&points)
                        .unwrap()
                        .into_iter()
                        .zip(expected)
                    {
                        assert_mm(actual, expected, &definition);
                    }
                    assert!(matches!(operation, Transform::Pure(_)), "{definition}");
                }
            }
        }
    }

    #[cfg(feature = "native-geospatial")]
    #[test]
    fn portable_conic_and_stereographic_boundaries_match_strict_native() {
        use crate::geospatial::{Crs, EcefTransform};
        let mut definitions = Vec::new();
        for projection in ["lcc", "aea"] {
            for (first, second) in [
                (-80_f64, -80.),
                (80., 80.),
                (-80., -79.),
                (79., 80.),
                (-40., 41.),
                (40., -39.),
                (-0.5, -0.5),
                (0.5, 0.5),
                (1., 2.),
                (-2., -1.),
                (33., 45.),
            ] {
                let origin = if projection == "aea" {
                    first.clamp(-75., 75.)
                } else {
                    first
                };
                definitions.push((
                    format!(
                    "+proj={projection} +lat_1={first} +lat_2={second} +lat_0={origin} +datum=WGS84"
                ),
                    origin,
                ));
                definitions.extend(
                    conic_wkts(projection, first, second).map(|definition| (definition, origin)),
                );
            }
        }
        for latitude in [-90., -79.999999, -52., 0., 52., 79.999999, 90.] {
            definitions.push((
                format!("+proj=stere +lat_0={latitude} +datum=WGS84"),
                latitude,
            ));
        }
        for (definition, origin) in definitions {
            // Keep the far point inside Albers' finite inverse domain by
            // moving toward the equator from the projection origin.
            let points = [
                [0., 0., 123.],
                [200000., if origin < 0. { 567890. } else { -567890. }, 123.],
                [
                    3000000.,
                    if origin < 0. { 3000000. } else { -3000000. },
                    500.,
                ],
            ];
            let mut operation = Transform::new(&definition, 7.).unwrap();
            assert!(matches!(operation, Transform::Pure(_)), "{definition}");
            assert_mm(
                operation.transform(&points[..1]).unwrap()[0],
                geodetic_to_ecef(Cartographic::new(0., origin, 130.)),
                "analytic projection origin",
            );
            let expected = EcefTransform::new(Crs::from_definition(&definition).unwrap(), Some(7.))
                .unwrap()
                .transform(&points)
                .unwrap_or_else(|error| panic!("{definition}: {error}"));
            for (actual, expected) in operation
                .transform(&points)
                .unwrap()
                .into_iter()
                .zip(expected)
            {
                assert_mm(actual, expected, &definition);
            }
            assert!(matches!(operation, Transform::Pure(_)), "{definition}");
        }
    }

    #[test]
    fn additional_valid_native_crs_are_not_misclassified_as_bad_data() {
        for definition in additional_native_definitions() {
            assert!(
                matches!(
                    PureTransform::new(&definition, 7.),
                    Err(Error::Environment(_))
                ),
                "{definition}"
            );
            #[cfg(not(feature = "native-geospatial"))]
            assert!(
                matches!(Transform::new(&definition, 7.), Err(Error::Environment(_))),
                "{definition}"
            );
        }
    }

    #[cfg(feature = "native-geospatial")]
    #[test]
    fn additional_native_operations_preserve_positions_and_laea_accuracy() {
        use crate::geospatial::{Crs, EcefTransform};
        let points = [
            [0., 0., 123.],
            [1000., 2000., 123.],
            [-300000., 700000., -30.],
            [200000., 567890., 123.],
            [3000000., -3000000., 500.],
        ];
        for definition in additional_native_definitions() {
            let native_source = Crs::from_definition(&definition).unwrap();
            // A conic origin may be outside the point-latitude accuracy domain;
            // compare its native interior instead of assuming XY=0 is usable.
            let albers_points = if native_source.albers_domain().unwrap().is_some() {
                crate::geospatial::StrictTransform::new(
                    &Crs::from_definition("EPSG:4326").unwrap(),
                    &native_source,
                )
                .unwrap()
                .transform(&[[-2., -45., 123.], [0., 0., -30.], [20., 45., 500.]])
                .unwrap()
            } else {
                Vec::new()
            };
            let geographic_points = [[0., 0., 123.], [153., -27., 123.], [-2., 52., -30.]];
            let points = if !albers_points.is_empty() {
                &albers_points[..]
            } else if definition.starts_with("EPSG:4326@") || definition.starts_with("GEOGCS") {
                &geographic_points[..]
            } else {
                &points[..]
            };
            let mut operation = Transform::new(&definition, 7.)
                .unwrap_or_else(|error| panic!("{definition}: {error}"));
            assert!(matches!(operation, Transform::Native(_)), "{definition}");
            if definition.starts_with("EPSG:") && definition.contains('@') {
                assert_eq!(native_source.coordinate_epoch(), Some(2020.));
            }
            let expected = EcefTransform::new(native_source, Some(7.))
                .unwrap()
                .transform(points)
                .unwrap_or_else(|error| panic!("{definition}: {error}"));
            if definition == "+proj=lcc +lat_1=33 +datum=WGS84" {
                assert_mm(
                    operation.transform(&points[..1]).unwrap()[0],
                    geodetic_to_ecef(Cartographic::new(0., 0., 130.)),
                    "native LCC defaults at analytic origin",
                );
            }
            for (actual, expected) in operation
                .transform(points)
                .unwrap()
                .into_iter()
                .zip(expected)
            {
                assert_mm(actual, expected, &definition);
            }
        }
        for definition in projection_wkts(
            "Lambert_Azimuthal_Equal_Area",
            "Lambert Azimuthal Equal Area",
            -15.,
            135.,
            None,
        ) {
            let mut operation = Transform::new(&definition, 7.).unwrap();
            assert_mm(
                operation.transform(&points[..1]).unwrap()[0],
                geodetic_to_ecef(Cartographic::new(135., -15., 130.)),
                "GLANCE Oceania centre",
            );
        }
    }

    #[test]
    fn geographic_longitude_offsets_and_nondecimal_angles_require_native_tier() {
        for definition in [
            "+proj=longlat +lon_0=10 +datum=WGS84",
            "+proj=latlong +lon_0=-10 +datum=WGS84",
            DMS_TMERC,
            r#"+proj=tmerc +lat_0=49d30'0"N +datum=WGS84"#,
            r#"+proj=merc +lat_ts=30d0'0"N +datum=WGS84"#,
            r#"+proj=lcc +lat_1=33d0'0"N +lat_2=45 +datum=WGS84"#,
            r#"+proj=lcc +lat_1=33 +lat_2=45d0'0"N +datum=WGS84"#,
        ] {
            let error = PureTransform::new(definition, 7.).err().expect(definition);
            assert!(
                matches!(error, Error::Environment(_)),
                "{definition}: {error}"
            );
            assert!(error.to_string().contains("--features native-geospatial"));
            #[cfg(not(feature = "native-geospatial"))]
            assert!(matches!(
                Transform::new(definition, 7.),
                Err(Error::Environment(_))
            ));
        }
        for projection in ["longlat", "latlong"] {
            let definition = format!("+proj={projection} +lon_0=0 +datum=WGS84");
            let operation = PureTransform::new(&definition, 7.).unwrap();
            assert_mm(
                operation.transform(&[[0., 0., 123.]]).unwrap()[0],
                [6378137. + 130., 0., 0.],
                &definition,
            );
        }
        // A nonfinite numeric angle remains a data error, rather than being
        // mistaken for a supported native syntax such as DMS.
        assert!(matches!(
            PureTransform::new("+proj=tmerc +lon_0=NaN +datum=WGS84", 7.),
            Err(Error::Data(_))
        ));
    }

    #[cfg(feature = "native-geospatial")]
    #[test]
    fn geographic_offsets_and_dms_fallback_match_strict_native_positions() {
        use crate::geospatial::{Crs, EcefTransform};
        let points = [[0., 0., 123.], [10000., 20000., -30.]];
        for (definition, input) in [
            ("+proj=longlat +lon_0=10 +datum=WGS84", &points[..1]),
            ("+proj=latlong +lon_0=-10 +datum=WGS84", &points[..1]),
            (DMS_TMERC, &points[..]),
            (r#"+proj=tmerc +lat_0=49d30'0"N +datum=WGS84"#, &points[..]),
            (r#"+proj=merc +lat_ts=30d0'0"N +datum=WGS84"#, &points[..]),
            (
                r#"+proj=lcc +lat_1=33d0'0"N +lat_2=45 +datum=WGS84"#,
                &points[..],
            ),
            (
                r#"+proj=lcc +lat_1=33 +lat_2=45d0'0"N +datum=WGS84"#,
                &points[..],
            ),
        ] {
            let mut operation = Transform::new(definition, 7.).unwrap();
            assert!(matches!(operation, Transform::Native(_)), "{definition}");
            let expected = EcefTransform::new(Crs::from_definition(definition).unwrap(), Some(7.))
                .unwrap()
                .transform(input)
                .unwrap();
            for (actual, expected) in operation
                .transform(input)
                .unwrap()
                .into_iter()
                .zip(expected)
            {
                assert_mm(actual, expected, definition);
            }
        }
        // The original review reproducer lands at longitude 10 degrees, not 0.
        let mut operation = Transform::new("+proj=longlat +lon_0=10 +datum=WGS84", 7.).unwrap();
        assert_mm(
            operation.transform(&points[..1]).unwrap()[0],
            geodetic_to_ecef(Cartographic::new(10., 0., 130.)),
            "geographic +lon_0=10",
        );
    }

    #[cfg(feature = "native-geospatial")]
    #[test]
    fn grid_free_operations_match_strict_gdal_within_one_millimetre() {
        use crate::geospatial::{Crs, EcefTransform};
        let mut cases = vec![
            (
                "+proj=utm +zone=32 +datum=WGS84 +units=ft +type=crs".to_owned(),
                vec![[500000. / 0.3048, 5678901. / 0.3048, 123.]],
            ),
            (
                format!(
                    r#"PROJCS["Invented polar",{WGS_WKT},PROJECTION["Polar_Stereographic"],PARAMETER["latitude_of_origin",70],PARAMETER["central_meridian",-45],UNIT["metre",1]]"#
                ),
                vec![[200000., 567890., 123.], [-300000., 700000., -30.]],
            ),
            (
                "EPSG:4326".to_owned(),
                vec![
                    [153.02, -27.47, 123.],
                    [-179.9, 80., -30.],
                    [180., -90., 0.],
                ],
            ),
            (
                "EPSG:3857".to_owned(),
                vec![[17000000., -3100000., 123.], [-20000000., 19000000., -30.]],
            ),
            (
                "EPSG:3395".to_owned(),
                vec![[17000000., -3100000., 123.], [-20000000., 19000000., -30.]],
            ),
            (
                UTM_WKT2.to_owned(),
                vec![[700000., 7000000., 123.], [200000., 2000000., -30.]],
            ),
        ];
        for code in (32601..=32660).chain(32701..=32760) {
            cases.push((
                format!("EPSG:{code}"),
                vec![[200000., 5678901., 123.], [786543., 7000000., -30.]],
            ));
        }
        for definition in projection_wkts(
            "Lambert_Conformal_Conic_1SP",
            "Lambert Conic Conformal (1SP)",
            49.,
            10.,
            Some(0.99),
        ) {
            cases.push((definition, vec![[0., 0., 123.], [200000., 567890., -30.]]));
        }
        for projection in [
            "+proj=tmerc +lat_0=49 +lon_0=-2 +k=0.9996012717 +x_0=400000 +y_0=-100000",
            "+proj=lcc +lat_1=33 +lat_2=45 +lat_0=39 +lon_0=-96",
            "+proj=aea +lat_1=29.5 +lat_2=45.5 +lat_0=23 +lon_0=-96",
            "+proj=stere +lat_0=90 +lat_ts=70 +lon_0=-45",
            "+proj=sterea +lat_0=52 +lon_0=5 +k=0.9999",
            "+proj=merc +lat_ts=30 +lon_0=12",
        ] {
            cases.push((
                format!("{projection} +datum=WGS84 +units=m +type=crs"),
                vec![[200000., 567890., 123.], [-300000., 700000., -30.]],
            ));
        }
        for shift in [
            "12,-34,56",
            "446.448,-125.157,542.06,0.15,0.247,0.842,-20.489",
        ] {
            cases.push((
                format!("+proj=longlat +ellps=airy +towgs84={shift} +type=crs"),
                vec![[-2., 53., 123.], [1., 51., -30.]],
            ));
        }
        for (definition, points) in cases {
            // Native builds must still select the portable tier for these CRSs.
            assert!(
                matches!(Transform::new(&definition, 7.).unwrap(), Transform::Pure(_)),
                "{definition}"
            );
            let native_source = Crs::from_definition(&definition).unwrap();
            let native_wkt = native_source.wkt().unwrap();
            let expected = EcefTransform::new(native_source, Some(7.))
                .unwrap()
                .transform(&points)
                .unwrap();
            let actual = PureTransform::new(&definition, 7.)
                .unwrap()
                .transform(&points)
                .unwrap();
            for (actual, expected) in actual.into_iter().zip(&expected) {
                assert_mm(actual, *expected, &definition);
            }
            // Exported WKT independently exercises units, parameters and datum
            // preservation. PROJ may export Helmert CRSs as BoundCRS (native).
            if !native_wkt.starts_with("BOUNDCRS") {
                let transform = match PureTransform::new(&native_wkt, 7.) {
                    Ok(transform) => transform,
                    // Polar stereographic WKT can have two axes directed south
                    // along different meridians. Keep native axis semantics.
                    Err(Error::Environment(reason)) if reason.contains("WKT axis direction") => {
                        assert!(matches!(
                            Transform::new(&native_wkt, 7.).unwrap(),
                            Transform::Native(_)
                        ));
                        continue;
                    }
                    Err(error) => panic!("{native_wkt}: {error}"),
                };
                let actual = transform.transform(&points).unwrap();
                for (actual, expected) in actual.into_iter().zip(&expected) {
                    assert_mm(actual, *expected, &native_wkt);
                }
            }
        }
    }

    #[cfg(feature = "native-geospatial")]
    #[test]
    fn unsupported_pure_projection_automatically_uses_strict_native_tier() {
        let definition = "+proj=aeqd +lat_0=-27 +lon_0=153 +datum=WGS84 +type=crs";
        let mut operation = Transform::new(definition, 7.).unwrap();
        assert!(matches!(operation, Transform::Native(_)));
        assert_mm(
            operation.transform(&[[0., 0., 123.]]).unwrap()[0],
            geodetic_to_ecef(Cartographic::new(153., -27., 130.)),
            definition,
        );
    }
}

// Keep these numerical oracles separate from the native-comparison tests above.
// Neither forward coordinates nor expected ECEF call either CRS implementation
// or crate::georef: agreement between inverse libraries is not an accuracy proof.
#[cfg(test)]
mod accuracy_tests {
    use super::{PureTransform, Transform};
    use crate::Error;
    use std::f64::consts::FRAC_PI_4;

    const A: f64 = 6_378_137.;
    const RF: f64 = 298.257223563;
    const OFFSET: f64 = 7.;
    const UNITS: [(&str, f64); 4] = [
        ("m", 1.),
        ("ft", 0.3048),
        ("us-ft", 1200. / 3937.),
        ("km", 1000.),
    ];

    fn ecef(a: f64, rf: f64, point: [f64; 3]) -> [f64; 3] {
        let f = 1. / rf;
        let e2 = f * (2. - f);
        let (s, c) = point[1].to_radians().sin_cos();
        let (sl, cl) = point[0].to_radians().sin_cos();
        let n = a / (1. - e2 * s * s).sqrt();
        let h = point[2] + OFFSET;
        [(n + h) * c * cl, (n + h) * c * sl, (n * (1. - e2) + h) * s]
    }

    fn assert_mm(actual: [f64; 3], expected: [f64; 3], label: &str) {
        let error = actual
            .iter()
            .zip(expected)
            .map(|(a, e)| (a - e).powi(2))
            .sum::<f64>()
            .sqrt();
        assert!(
            error < 0.001,
            "{label}: {error} m; {actual:?} != {expected:?}"
        );
    }

    fn check(definition: &str, input: &[[f64; 3]], expected: &[[f64; 3]]) {
        let mut transform = Transform::new(definition, OFFSET).unwrap();
        assert!(matches!(transform, Transform::Pure(_)), "{definition}");
        let actual = transform.transform(input).unwrap();
        assert_eq!(actual.len(), expected.len());
        for (actual, expected) in actual.into_iter().zip(expected) {
            assert_mm(actual, *expected, definition);
        }
        #[cfg(feature = "native-geospatial")]
        {
            // Force native selection too: ordinary creation prefers Pure.
            let actual = Transform::native(definition, OFFSET)
                .unwrap()
                .transform(input)
                .unwrap();
            assert_eq!(actual.len(), expected.len());
            for (actual, expected) in actual.into_iter().zip(expected) {
                assert_mm(actual, *expected, definition);
            }
        }
    }

    #[test]
    fn geographic_and_mercator_positions_have_independent_ecef_truth() {
        let points = [[153., -27., 123.], [-179.5, 75., -30.], [12., 0., 4500.]];
        let expected = points.map(|p| ecef(A, RF, p));
        check("EPSG:4326", &points, &expected);
        for (projection, eccentricity) in [
            ("merc", (2. / RF - 1. / RF.powi(2)).sqrt()),
            ("webmerc", 0.),
        ] {
            for (units, factor) in UNITS {
                // Mercator's isometric latitude, including ellipsoidal correction.
                let projected = points.map(|p| {
                    let phi = p[1].to_radians();
                    let es = eccentricity * phi.sin();
                    let y = A
                        * ((FRAC_PI_4 + phi / 2.).tan()
                            * ((1. - es) / (1. + es)).powf(eccentricity / 2.))
                        .ln();
                    [
                        (A * p[0].to_radians() + 5000.) / factor,
                        (y - 6000.) / factor,
                        p[2],
                    ]
                });
                check(
                    &format!("+proj={projection} +x_0=5000 +y_0=-6000 +datum=WGS84 +units={units}"),
                    &projected,
                    &expected,
                );
            }
        }
    }

    fn utm_forward(point: [f64; 3], central: f64, south: bool) -> [f64; 3] {
        let e2 = (1. / RF) * (2. - 1. / RF);
        let ep2 = e2 / (1. - e2);
        let phi = point[1].to_radians();
        let (sin, cos) = phi.sin_cos();
        let n = A / (1. - e2 * sin * sin).sqrt();
        let t = phi.tan().powi(2);
        let c = ep2 * cos * cos;
        let alpha = (point[0] - central).to_radians() * cos;
        // Integrate meridional radius directly instead of using the library's
        // meridional coefficients. Simpson quadrature error here is below 1 um.
        let steps = 2048;
        let step = phi / steps as f64;
        let radius = |lat: f64| A * (1. - e2) / (1. - e2 * lat.sin().powi(2)).powf(1.5);
        let mut m = radius(0.) + radius(phi);
        for index in 1..steps {
            m += radius(index as f64 * step) * if index % 2 == 0 { 2. } else { 4. };
        }
        m *= step / 3.;
        // Snyder transverse Mercator series, restricted to <= 0.5 degree from
        // the meridian in this test (omitted terms contribute below 1 um).
        let x = 500000.
            + 0.9996
                * n
                * (alpha
                    + (1. - t + c) * alpha.powi(3) / 6.
                    + (5. - 18. * t + t * t + 72. * c - 58. * ep2) * alpha.powi(5) / 120.);
        let y = if south { 10000000. } else { 0. }
            + 0.9996
                * (m + n
                    * phi.tan()
                    * (alpha.powi(2) / 2.
                        + (5. - t + 9. * c + 4. * c * c) * alpha.powi(4) / 24.
                        + (61. - 58. * t + t * t + 600. * c - 330. * ep2) * alpha.powi(6) / 720.));
        [x, y, point[2]]
    }

    #[test]
    fn utm_in_both_hemispheres_preserves_horizontal_units_and_metre_heights() {
        for (zone, central, sign) in [(32, 9., 1.), (56, 153., -1.)] {
            let points = [
                [central - 0.5, sign * 27., 123.],
                [central + 0.5, sign * 52., -30.],
            ];
            let expected = points.map(|p| ecef(A, RF, p));
            let metres = points.map(|p| utm_forward(p, central, sign < 0.));
            check(
                &format!("EPSG:{}{zone:02}", if sign < 0. { 327 } else { 326 }),
                &metres,
                &expected,
            );
            for (units, factor) in UNITS {
                let input = metres.map(|p| [p[0] / factor, p[1] / factor, p[2]]);
                check(
                    &format!(
                        "+proj=utm +zone={zone} {} +datum=WGS84 +units={units}",
                        if sign < 0. { "+south" } else { "" }
                    ),
                    &input,
                    &expected,
                );
            }
        }
    }

    #[test]
    fn local_lcc_with_seven_parameter_helmert_has_independent_position_vector_truth() {
        let a = 6377563.396;
        let rf: f64 = 299.3249646;
        let f = 1. / rf;
        let e2 = f * (2. - f);
        let e = e2.sqrt();
        let m = |lat: f64| {
            let phi = lat.to_radians();
            phi.cos() / (1. - e2 * phi.sin().powi(2)).sqrt()
        };
        let t = |lat: f64| {
            let phi = lat.to_radians();
            let es = e * phi.sin();
            (FRAC_PI_4 - phi / 2.).tan() / ((1. - es) / (1. + es)).powf(e / 2.)
        };
        let n = (m(33.) / m(45.)).ln() / (t(33.) / t(45.)).ln();
        let constant = m(33.) / (n * t(33.).powf(n));
        let rho0 = a * constant * t(39.).powf(n);
        let points = [[-97., 37., 123.], [-92., 43., -30.]];
        let projected = points.map(|p| {
            let rho = a * constant * t(p[1]).powf(n);
            let (s, c) = (n * (p[0] + 96.).to_radians()).sin_cos();
            [5000. + rho * s, -6000. + rho0 - rho * c, p[2]]
        });
        let expected = points.map(|p| {
            let [x, y, z] = ecef(a, rf, p);
            // Explicit +towgs84 uses the position-vector convention. Rotations
            // are arcseconds and scale is ppm; height is applied before shift.
            let arcsecond = std::f64::consts::PI / (180. * 3600.);
            let (rx, ry, rz) = (0.15 * arcsecond, 0.247 * arcsecond, 0.842 * arcsecond);
            let scale = 1. - 20.489e-6;
            [
                446.448 + scale * (x - rz * y + ry * z),
                -125.157 + scale * (rz * x + y - rx * z),
                542.06 + scale * (-ry * x + rx * y + z),
            ]
        });
        for (units, factor) in UNITS {
            let input = projected.map(|p| [p[0] / factor, p[1] / factor, p[2]]);
            check(&format!("+proj=lcc +lat_1=33 +lat_2=45 +lat_0=39 +lon_0=-96 +x_0=5000 +y_0=-6000 +ellps=airy +towgs84=446.448,-125.157,542.06,0.15,0.247,0.842,-20.489 +units={units}"), &input, &expected);
        }
    }

    #[test]
    fn spherical_stereographic_fallback_retries_both_batch_orders_against_analytic_truth() {
        let radius = 6370997.;
        let definition = "+proj=stere +lat_0=45 +ellps=sphere +towgs84=12,-34,56";
        let points = [[15_f64, 50_f64, 123.], [-20., 85., -30.]];
        let origin = 45_f64.to_radians();
        let input = points.map(|p| {
            let lon = p[0].to_radians();
            let lat = p[1].to_radians();
            let k = 2. * radius
                / (1. + origin.sin() * lat.sin() + origin.cos() * lat.cos() * lon.cos());
            [
                k * lat.cos() * lon.sin(),
                k * (origin.cos() * lat.sin() - origin.sin() * lat.cos() * lon.cos()),
                p[2],
            ]
        });
        let expected = points.map(|p| {
            let mut xyz = ecef(radius, f64::INFINITY, p);
            for (v, shift) in xyz.iter_mut().zip([12., -34., 56.]) {
                *v += shift;
            }
            xyz
        });
        let safe = (input[0], expected[0]);
        for (input, expected) in [
            (input, expected),
            ([input[1], input[0]], [expected[1], expected[0]]),
        ] {
            let mut transform = Transform::new(definition, OFFSET).unwrap();
            assert!(matches!(transform, Transform::Pure(_)));
            assert!(matches!(
                PureTransform::new(definition, OFFSET)
                    .unwrap()
                    .transform(&input),
                Err(Error::Environment(_))
            ));
            #[cfg(not(feature = "native-geospatial"))]
            {
                let _ = expected;
                assert!(matches!(
                    transform.transform(&input),
                    Err(Error::Environment(_))
                ));
                assert!(matches!(transform, Transform::Pure(_)));
            }
            #[cfg(feature = "native-geospatial")]
            {
                let actual = transform.transform(&input).unwrap();
                assert_eq!(actual.len(), 2);
                for (a, e) in actual.into_iter().zip(expected) {
                    assert_mm(a, e, definition);
                }
                assert!(matches!(transform, Transform::Native(_)));
                assert_mm(
                    transform.transform(&[input[0]]).unwrap()[0],
                    expected[0],
                    "subsequent native batch",
                );
            }
            // A later invalid coordinate must fail the entire retry, without
            // committing Native state or poisoning a subsequent portable batch.
            let mut failed = Transform::new(definition, OFFSET).unwrap();
            let result = failed.transform(&[input[0], input[1], [f64::NAN, 0., 123.]]);
            #[cfg(not(feature = "native-geospatial"))]
            assert!(matches!(result, Err(Error::Environment(_))));
            #[cfg(feature = "native-geospatial")]
            assert!(matches!(result, Err(Error::Data(_))));
            assert!(matches!(failed, Transform::Pure(_)));
            assert_mm(
                failed.transform(&[safe.0]).unwrap()[0],
                safe.1,
                "portable batch after failed retry",
            );
            assert!(matches!(failed, Transform::Pure(_)));
        }
    }
}
