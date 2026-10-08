//! Conservative, grid-free point-cloud CRS operations. EPSG definitions are an
//! explicit allowlist, not an old PROJ-string database with implicit datum shifts.
//! The WKT parser is used only for syntax: proj4wkt's formatter assumes a zero
//! datum shift and drops vertical/unknown metadata, so it is deliberately unused.
use crate::Error;
use proj4rs::Proj;
use proj4wkt::parser::{Attribute, Processor};
use std::collections::BTreeMap;

pub(super) enum Transform {
    Pure(Box<PureTransform>),
    #[cfg(feature = "native-geospatial")]
    Native(crate::geospatial::EcefTransform),
}

impl Transform {
    pub fn new(definition: &str, height_offset: f64) -> Result<Self, Error> {
        match PureTransform::new(definition, height_offset) {
            Ok(operation) => Ok(Self::Pure(Box::new(operation))),
            #[cfg(feature = "native-geospatial")]
            Err(Error::Environment(_)) => {
                let source = crate::geospatial::Crs::from_definition(definition)?;
                if !source.is_horizontal() {
                    return Err(horizontal_required());
                }
                crate::geospatial::EcefTransform::new(source, Some(height_offset)).map(Self::Native)
            }
            Err(error) => Err(error),
        }
    }

    pub fn transform(&mut self, points: &[[f64; 3]]) -> Result<Vec<[f64; 3]>, Error> {
        match self {
            Self::Pure(operation) => operation.transform(points),
            #[cfg(feature = "native-geospatial")]
            Self::Native(operation) => operation.transform(points),
        }
    }
}

fn unsupported(reason: &str) -> Error {
    Error::Environment(format!("pure-Rust point-cloud CRS transform unavailable: {reason}; use a build with --features native-geospatial (GDAL >= 3.12, PROJ >= 9.2) and the required local PROJ database/grids"))
}

fn horizontal_required() -> Error {
    Error::Data("use a 2D horizontal CRS and explicit ellipsoidal height offset; compound/geocentric CRS is unsupported".into())
}

pub(super) struct PureTransform {
    source: Proj,
    target: Proj,
    angular_units: f64,
    height_offset: f64,
}

impl PureTransform {
    fn new(definition: &str, height_offset: f64) -> Result<Self, Error> {
        if !height_offset.is_finite() {
            return Err(Error::Data("height offset must be finite".into()));
        }
        let definition = definition.trim();
        if definition.contains('\0') {
            return Err(Error::Data("CRS definition contains a NUL byte".into()));
        }
        let (definition, angular_units) = if definition.starts_with('+') {
            (validate_proj(definition)?, std::f64::consts::PI / 180.)
        } else if definition.to_ascii_uppercase().starts_with("EPSG:") {
            (epsg(definition)?, std::f64::consts::PI / 180.)
        } else if definition.contains('[') {
            wkt(definition)?
        } else {
            return Err(unsupported(
                "expected an EPSG code, WKT or PROJ CRS definition",
            ));
        };
        let source = Proj::from_proj_string(&definition)
            .map_err(|error| Error::Data(format!("invalid source CRS: {error}")))?;
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
        Ok(Self {
            source,
            target,
            angular_units,
            height_offset,
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
    if definition[5..].contains('+') {
        return Err(unsupported(
            "compound/vertical EPSG CRS may require a geoid grid",
        ));
    }
    let code = definition[5..]
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
        "sterea" | "laea" => Ok(&[]),
        "geocent" => Err(horizontal_required()),
        _ => Err(unsupported(
            "projection method is outside the verified grid-free tier",
        )),
    }
}

fn validate_proj(definition: &str) -> Result<String, Error> {
    let mut params = BTreeMap::new();
    for token in definition.split_whitespace() {
        let token = token
            .strip_prefix('+')
            .ok_or_else(|| Error::Data("invalid PROJ CRS parameter".into()))?;
        let (key, value) = token.split_once('=').unwrap_or((token, ""));
        if params.insert(key, value).is_some() {
            return Err(Error::Data(format!("duplicate PROJ CRS parameter: +{key}")));
        }
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
                | "zone"
        ) {
            finite(value)?;
        }
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
    let root = proj4wkt::parser::parse(definition, &Tree)
        .map_err(|error| Error::Data(format!("invalid source WKT CRS: {error}")))?;
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
            .replace([' ', '_', '-'], "");
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
            if key == "lat_0" {
                latitude_origin = value;
            }
            if key == "lat_ts" {
                latitude_parallel = Some(value);
            }
            if !seen.insert(key) {
                return Err(Error::Data("duplicate WKT projection parameter".into()));
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
        result = validate_proj(&result)?;
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
        for projection in [
            "+proj=tmerc +lat_0=49 +lon_0=-2 +k=0.9996012717 +x_0=400000 +y_0=-100000",
            "+proj=lcc +lat_1=33 +lat_2=45 +lat_0=39 +lon_0=-96",
            "+proj=lcc +lat_1=49 +lat_0=49 +lon_0=10 +k=0.99",
            "+proj=aea +lat_1=29.5 +lat_2=45.5 +lat_0=23 +lon_0=-96",
            "+proj=stere +lat_0=90 +lat_ts=70 +lon_0=-45",
            "+proj=sterea +lat_0=52 +lon_0=5 +k=0.9999",
            "+proj=laea +lat_0=52 +lon_0=10",
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
