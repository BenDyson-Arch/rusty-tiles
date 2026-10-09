//! Read-only validation of self-contained explicit and native implicit 3TZ packages.
use crate::pack::TZ_INDEX_NAME;
mod admission;
mod json;
mod payload;
mod types;
pub use types::{
    PayloadReport, ValidationFailure, ValidationLimits, ValidationReport, ValidationRequest,
};
type Error = ValidationFailure;
#[cfg(test)]
use serde_json::json;
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeSet, HashMap, HashSet},
    fs::File,
    io::Read,
    path::Path,
};

type Archive = zip::ZipArchive<File>;
fn invalid(message: impl Into<String>) -> Error {
    Error::InvalidInput(message.into())
}
fn number(value: &Value, label: &str) -> Result<f64, Error> {
    value
        .as_f64()
        .filter(|n| n.is_finite() && *n >= 0.)
        .ok_or_else(|| invalid(format!("{label} must be finite and nonnegative")))
}
fn array(value: &Value, length: usize, label: &str) -> Result<Vec<f64>, Error> {
    let values = value
        .as_array()
        .filter(|v| v.len() == length)
        .ok_or_else(|| invalid(format!("{label} must have {length} numbers")))?;
    values
        .iter()
        .map(|v| {
            v.as_f64()
                .filter(|n| n.is_finite())
                .ok_or_else(|| invalid(format!("{label} contains a nonfinite/non-numeric value")))
        })
        .collect()
}
impl From<serde_json::Error> for ValidationFailure {
    fn from(e: serde_json::Error) -> Self {
        Self::invalid(format!("invalid JSON: {e}"))
    }
}
impl From<zip::result::ZipError> for ValidationFailure {
    fn from(e: zip::result::ZipError) -> Self {
        match e {
            zip::result::ZipError::Io(e) => archive_read_error(e),
            other => Self::invalid(other.to_string()),
        }
    }
}
impl From<crate::Error> for ValidationFailure {
    fn from(e: crate::Error) -> Self {
        match e {
            crate::Error::Validation(e) => e,
            crate::Error::Io(e) => archive_read_error(e),
            crate::Error::Zip(e) => e.into(),
            other => Self::invalid(other.to_string()),
        }
    }
}
fn archive_read_error(e: std::io::Error) -> ValidationFailure {
    match e.kind() {
        std::io::ErrorKind::UnexpectedEof | std::io::ErrorKind::InvalidData => {
            ValidationFailure::invalid(format!("malformed archive bytes: {e}"))
        }
        _ => ValidationFailure::Io(e),
    }
}
fn read_member(
    zip: &mut Archive,
    name: &str,
    bytes_read: &mut u64,
    limits: &ValidationLimits,
    maximum: u64,
) -> Result<Vec<u8>, Error> {
    let mut entry = zip.by_name(name)?;
    let size = entry.size();
    if size > maximum {
        return Err(Error::limit(format!(
            "member exceeds admitted byte limit: {name}"
        )));
    }
    *bytes_read = bytes_read
        .checked_add(size)
        .filter(|&n| n <= limits.total_bytes_read)
        .ok_or_else(|| Error::limit("member reads exceed 2 GiB work limit"))?;
    let mut bytes = Vec::new();
    bytes
        .try_reserve_exact(
            usize::try_from(size).map_err(|_| Error::limit("member exceeds host address range"))?,
        )
        .map_err(|_| Error::limit("member allocation unavailable"))?;
    entry
        .by_ref()
        .take(size + 1)
        .read_to_end(&mut bytes)
        .map_err(archive_read_error)?;
    if bytes.len() as u64 != size {
        return Err(invalid(format!(
            "member length differs from directory: {name}"
        )));
    }
    Ok(bytes)
}
fn uri(base: &str, value: &str) -> Result<String, Error> {
    if value.is_empty() || value.contains('\\') {
        return Err(invalid(format!("invalid resource URI: {value}")));
    }
    if value.contains([':', '?', '#', '%']) || value.starts_with('/') {
        return Err(Error::unsupported(format!(
            "URI outside archive-local profile: {value}"
        )));
    }
    let mut parts: Vec<&str> = base
        .rsplit_once('/')
        .map_or(vec![], |(dir, _)| dir.split('/').collect());
    for part in value.split('/') {
        match part {
            "" | "." => (),
            ".." => {
                if parts.pop().is_none() {
                    return Err(invalid("URI escapes archive"));
                }
            }
            other => parts.push(other),
        }
    }
    Ok(parts.join("/"))
}
use crate::vec3::{dot, norm, sub};
const IDENTITY: [f64; 16] = [
    1., 0., 0., 0., 0., 1., 0., 0., 0., 0., 1., 0., 0., 0., 0., 1.,
];
fn transform(node: &Value) -> Result<Vec<f64>, Error> {
    let m = if node.get("transform").is_some() {
        array(&node["transform"], 16, "transform")?
    } else {
        IDENTITY.to_vec()
    };
    if m[3] != 0. || m[7] != 0. || m[11] != 0. || m[15] != 1. {
        return Err(invalid("transform must be affine"));
    }
    Ok(m)
}
fn point(m: &[f64], p: [f64; 3]) -> [f64; 3] {
    std::array::from_fn(|i| m[i] * p[0] + m[4 + i] * p[1] + m[8 + i] * p[2] + m[12 + i])
}
fn scale_bound(m: &[f64]) -> f64 {
    let row = (0..3)
        .map(|i| (0..3).map(|j| m[4 * j + i].abs()).sum::<f64>())
        .fold(0., f64::max);
    let col = (0..3)
        .map(|j| (0..3).map(|i| m[4 * j + i].abs()).sum::<f64>())
        .fold(0., f64::max);
    (row * col).sqrt()
}
#[derive(Clone)]
enum Volume {
    Box(Vec<f64>),
    Sphere(Vec<f64>),
    Region(Vec<f64>),
}
fn volume(value: &Value) -> Result<Volume, Error> {
    let o = value
        .as_object()
        .ok_or_else(|| invalid("boundingVolume must be an object"))?;
    if ["box", "sphere", "region"]
        .iter()
        .filter(|k| o.contains_key(**k))
        .count()
        != 1
    {
        return Err(invalid(
            "boundingVolume requires exactly one box, sphere or region",
        ));
    }
    if let Some(v) = o.get("box") {
        let b = array(v, 12, "boundingVolume.box")?;
        for i in 0..3 {
            for j in i + 1..3 {
                let a = [b[3 + 3 * i], b[4 + 3 * i], b[5 + 3 * i]];
                let c = [b[3 + 3 * j], b[4 + 3 * j], b[5 + 3 * j]];
                if dot(a, c).abs() > 1e-8 * norm(a) * norm(c) + 1e-12 {
                    return Err(invalid("box half axes must be orthogonal"));
                }
            }
        }
        return Ok(Volume::Box(b));
    }
    if let Some(v) = o.get("sphere") {
        let s = array(v, 4, "boundingVolume.sphere")?;
        if s[3] < 0. {
            return Err(invalid("sphere radius must be nonnegative"));
        }
        return Ok(Volume::Sphere(s));
    }
    let r = array(&value["region"], 6, "boundingVolume.region")?;
    if r[0].abs() > std::f64::consts::PI
        || r[2].abs() > std::f64::consts::PI
        || r[1].abs() > std::f64::consts::FRAC_PI_2
        || r[3].abs() > std::f64::consts::FRAC_PI_2
        || r[1] > r[3]
        || r[4] > r[5]
    {
        return Err(invalid("invalid geographic region ranges"));
    }
    Ok(Volume::Region(r))
}
fn inside(parent: &Volume, p: [f64; 3], radius: f64) -> bool {
    match parent {
        Volume::Sphere(s) => norm(sub(p, [s[0], s[1], s[2]])) + radius <= s[3] + 1e-6 * (1. + s[3]),
        Volume::Box(b) => {
            let mut residual = sub(p, [b[0], b[1], b[2]]);
            let d = residual;
            let tolerance = 1e-6 * (1. + b[3..].iter().map(|v| v * v).sum::<f64>().sqrt());
            let mut rank = 0;
            for i in 0..3 {
                let axis = [b[3 + 3 * i], b[4 + 3 * i], b[5 + 3 * i]];
                let length = norm(axis);
                if length > 0. {
                    rank += 1;
                    let projection = dot(d, axis) / length;
                    if projection.abs() + radius > length + tolerance {
                        return false;
                    }
                    for k in 0..3 {
                        residual[k] -= projection * axis[k] / length;
                    }
                }
            }
            norm(residual) <= tolerance && (rank == 3 || radius <= tolerance)
        }
        _ => false,
    }
}
fn contains(parent: &Volume, child: &Volume, m: &[f64]) -> Result<bool, Error> {
    match (parent, child) {
        (Volume::Region(p), Volume::Region(c)) => {
            let width = |r: &Vec<f64>| (r[2] - r[0]).rem_euclid(2. * std::f64::consts::PI);
            let offset = (c[0] - p[0]).rem_euclid(2. * std::f64::consts::PI);
            let pw = if p[2] - p[0] >= 2. * std::f64::consts::PI - 1e-12 {
                2. * std::f64::consts::PI
            } else {
                width(p)
            };
            Ok(offset + width(c) <= pw + 1e-12
                && c[1] >= p[1] - 1e-12
                && c[3] <= p[3] + 1e-12
                && c[4] >= p[4] - 1e-6
                && c[5] <= p[5] + 1e-6)
        }
        (Volume::Region(_), _) | (_, Volume::Region(_)) => Err(Error::unsupported(
            "mixed region/Cartesian containment is outside the admitted profile",
        )),
        (_, Volume::Sphere(s)) => Ok(inside(
            parent,
            point(m, [s[0], s[1], s[2]]),
            s[3] * scale_bound(m),
        )),
        (_, Volume::Box(b)) => Ok((0..8).all(|mask| {
            let p = std::array::from_fn(|k| {
                b[k] + (0..3)
                    .map(|i| {
                        if mask & (1 << i) == 0 {
                            -b[3 + 3 * i + k]
                        } else {
                            b[3 + 3 * i + k]
                        }
                    })
                    .sum::<f64>()
            });
            inside(parent, point(m, p), 0.)
        })),
    }
}
struct ArchiveResources<'a> {
    zip: &'a mut Archive,
    names: HashSet<String>,
    used: HashSet<String>,
    limits: ValidationLimits,
    bytes_read: u64,
    reference_visits: u64,
}
impl ArchiveResources<'_> {
    fn read_json(&mut self, name: &str) -> Result<Value, Error> {
        let bytes = read_member(
            self.zip,
            name,
            &mut self.bytes_read,
            &self.limits,
            self.limits.json_bytes,
        )?;
        json::parse(&bytes)
    }
    fn reference(&mut self, name: String) -> Result<(), Error> {
        self.reference_visits = self
            .reference_visits
            .checked_add(1)
            .filter(|&n| n <= self.limits.references)
            .ok_or_else(|| Error::limit("archive exceeds 262,144 reference visits"))?;
        if !self.names.contains(&name) {
            return Err(invalid(format!("missing archive entry: {name}")));
        }
        self.used.insert(name);
        Ok(())
    }
}
struct Check<'a> {
    resources: ArchiveResources<'a>,
    reports: Value,
    tiles: usize,
    contents: usize,
    active: HashSet<String>,
    schema: jsonschema::Validator,
    task_visits: u64,
    elements: u64,
    payloads: HashMap<String, PayloadReport>,
    not_inspected: BTreeSet<String>,
    expansion_budget: crate::implicit::ExpansionBudget,
}
enum ValidationTask {
    Tileset {
        name: String,
        parent: Option<Volume>,
        parent_error: Option<f64>,
        depth: usize,
    },
    Node {
        node: Value,
        base: String,
        parent: Option<Volume>,
        parent_error: f64,
        depth: usize,
    },
    ExitTileset(String),
}
impl Check<'_> {
    fn payload(&mut self, name: &str) -> Result<PayloadReport, Error> {
        if let Some(report) = self.payloads.get(name) {
            return Ok(report.clone());
        }
        let bytes = read_member(
            self.resources.zip,
            name,
            &mut self.resources.bytes_read,
            &self.resources.limits,
            self.resources.limits.member_bytes,
        )?;
        let remaining = self.resources.limits.total_payload_elements - self.elements;
        let facts = payload::inspect(name, &bytes, remaining, |value| {
            let resource = uri(name, value)?;
            self.resources.reference(resource.clone())?;
            read_member(
                self.resources.zip,
                &resource,
                &mut self.resources.bytes_read,
                &self.resources.limits,
                self.resources.limits.member_bytes,
            )
        })?;
        self.elements = self
            .elements
            .checked_add(facts.elements_checked)
            .filter(|&n| n <= self.resources.limits.total_payload_elements)
            .ok_or_else(|| Error::limit("payload elements exceed 16 million"))?;
        self.metadata_schema(
            name,
            &facts.document["extensions"]["EXT_structural_metadata"],
        )?;
        self.not_inspected.extend(facts.not_inspected);
        let report = PayloadReport {
            uri: name.into(),
            accessors_checked: facts.accessors_checked,
            primitives_checked: facts.primitives_checked,
            vertices: facts.vertices,
        };
        self.payloads.insert(name.into(), report.clone());
        Ok(report)
    }
    fn metadata_schema(&mut self, base: &str, doc: &Value) -> Result<(), Error> {
        if let Some(value) = doc.get("schemaUri") {
            let value = value
                .as_str()
                .ok_or_else(|| invalid("schemaUri must be a string"))?;
            let name = uri(base, value)?;
            self.resources.reference(name.clone())?;
            self.resources.read_json(&name)?;
        }
        Ok(())
    }
    fn tileset(
        &mut self,
        name: &str,
        parent: Option<&Volume>,
        parent_error: Option<f64>,
        depth: usize,
    ) -> Result<(), Error> {
        let mut pending = vec![ValidationTask::Tileset {
            name: name.into(),
            parent: parent.cloned(),
            parent_error,
            depth,
        }];
        while let Some(task) = pending.pop() {
            self.task_visits = self
                .task_visits
                .checked_add(1)
                .filter(|&n| n <= self.resources.limits.hierarchy_visits)
                .ok_or_else(|| Error::limit("hierarchy exceeds 65,536 traversal tasks"))?;
            match task {
                ValidationTask::Tileset {
                    name,
                    parent,
                    parent_error,
                    depth,
                } => {
                    if depth > 128 {
                        return Err(Error::limit("hierarchy exceeds validation depth limit"));
                    }
                    if !self.active.insert(name.clone()) {
                        return Err(invalid("cyclic external tileset reference"));
                    }
                    let (mut doc, error) = self.prepare_tileset(&name)?;
                    pending.push(ValidationTask::ExitTileset(name.clone()));
                    pending.push(ValidationTask::Node {
                        node: doc["root"].take(),
                        base: name,
                        parent,
                        parent_error: parent_error.map_or(error, |parent| parent.min(error)),
                        depth,
                    });
                }
                ValidationTask::Node {
                    node,
                    base,
                    parent,
                    parent_error,
                    depth,
                } => {
                    self.node(
                        node,
                        &base,
                        parent.as_ref(),
                        parent_error,
                        depth,
                        &mut pending,
                    )?;
                }
                ValidationTask::ExitTileset(name) => {
                    self.active.remove(&name);
                }
            }
        }
        Ok(())
    }

    fn prepare_tileset(&mut self, name: &str) -> Result<(Value, f64), Error> {
        self.resources.reference(name.into())?;
        let doc = self.resources.read_json(name)?;
        self.schema
            .validate(&doc)
            .map_err(|error| invalid(format!("tileset schema: {error}")))?;
        if !matches!(doc["asset"]["version"].as_str(), Some("1.0" | "1.1")) {
            return Err(invalid("asset.version must be 1.0 or 1.1"));
        }
        let error = number(&doc["geometricError"], "tileset.geometricError")?;
        self.metadata_schema(name, &doc)?;
        let doc = self.expand_implicit(name, doc)?;
        Ok((doc, error))
    }

    fn expand_implicit(&mut self, name: &str, mut doc: Value) -> Result<Value, Error> {
        if doc["root"].get("implicitTiling").is_some() {
            // Raw immutable vector payloads remain reachable through the native
            // reuse state; display content uses implicit URI templates.
            if let Some(sources) = doc["extras"]["rustyTilesSourceContents"].as_array() {
                for source in sources {
                    let source = uri(
                        name,
                        source
                            .as_str()
                            .ok_or_else(|| invalid("invalid cached content URI"))?,
                    )?;
                    self.resources.reference(source.clone())?;
                    self.payload(&source)?;
                }
            }
            self.not_inspected
                .insert("implicitAddressingAndAvailability".into());
            let resources = &mut self.resources;
            let schema = &self.schema;
            doc = crate::implicit::expand_tileset_bounded(
                &doc,
                |value| {
                    let source = uri(name, value).map_err(crate::Error::from)?;
                    resources
                        .reference(source.clone())
                        .map_err(crate::Error::from)?;
                    let maximum = if source.ends_with(".json") {
                        resources.limits.json_bytes
                    } else {
                        resources.limits.member_bytes
                    };
                    let bytes = read_member(
                        resources.zip,
                        &source,
                        &mut resources.bytes_read,
                        &resources.limits,
                        maximum,
                    )
                    .map_err(crate::Error::from)?;
                    if source.ends_with(".json") {
                        let doc = json::parse(&bytes).map_err(crate::Error::from)?;
                        schema.validate(&doc).map_err(|e| {
                            crate::Error::from(invalid(format!("tileset schema: {e}")))
                        })?;
                    }
                    Ok(bytes)
                },
                &mut self.expansion_budget,
            )?;
        }
        Ok(doc)
    }
    fn node(
        &mut self,
        mut node: Value,
        base: &str,
        parent: Option<&Volume>,
        parent_error: f64,
        depth: usize,
        pending: &mut Vec<ValidationTask>,
    ) -> Result<(), Error> {
        if depth as u64 > self.resources.limits.hierarchy_depth {
            return Err(Error::limit("hierarchy exceeds validation depth limit"));
        }
        if !node.is_object() {
            return Err(invalid("tile must be an object"));
        }
        if node.get("implicitTiling").is_some() {
            return Err(invalid(
                "nested implicit roots require an external tileset JSON",
            ));
        }
        if let Some(refine) = node.get("refine") {
            if !matches!(refine.as_str(), Some("ADD" | "REPLACE")) {
                return Err(invalid("invalid refine value"));
            }
        }
        let bounds = volume(&node["boundingVolume"])?;
        let m = transform(&node)?;
        if let Some(parent) = parent {
            if !contains(parent, &bounds, &m)? {
                return Err(invalid(format!(
                    "child bounds escape parent at depth {depth}"
                )));
            }
        }
        let error = number(&node["geometricError"], "tile.geometricError")?;
        if error > parent_error + 1e-8 * (1. + parent_error) {
            return Err(invalid(format!(
                "geometricError increases at depth {depth}"
            )));
        }
        self.tiles += 1;
        let mut bytes = 0u64;
        let mut vertices = 0u64;
        let mut descendants = Vec::new();
        if node.get("content").is_some() && node.get("contents").is_some() {
            return Err(invalid("tile has both content and contents"));
        }
        let contents = if let Some(c) = node.get("contents") {
            c.as_array()
                .ok_or_else(|| invalid("contents must be an array"))?
                .clone()
        } else {
            node.get("content").cloned().into_iter().collect()
        };
        for content in contents {
            let value = content["uri"]
                .as_str()
                .or_else(|| content["url"].as_str())
                .ok_or_else(|| invalid("content.uri must be a string"))?;
            let name = uri(base, value)?;
            self.resources.reference(name.clone())?;
            self.contents += 1;
            if let Some(v) = content.get("boundingVolume") {
                let cb = volume(v)?;
                if !contains(&bounds, &cb, &IDENTITY)? {
                    return Err(invalid("content bounds escape tile"));
                }
            }
            bytes = bytes
                .checked_add(self.resources.zip.by_name(&name)?.size())
                .ok_or_else(|| invalid("tile byte count overflow"))?;
            if name.ends_with(".json") {
                descendants.push(ValidationTask::Tileset {
                    name,
                    parent: Some(bounds.clone()),
                    parent_error: Some(error),
                    depth: depth + 1,
                });
                continue;
            }
            let inspected = self.payload(&name)?;
            vertices = vertices
                .checked_add(inspected.vertices)
                .ok_or_else(|| invalid("tile vertex count overflow"))?;
        }
        if bytes > 0 {
            if let Some(limit) = self.reports["budgets"]["bytes"].as_u64() {
                if bytes > limit {
                    return Err(invalid("tile exceeds recorded encoded byte budget"));
                }
            }
            if let Some(limit) = self.reports["budgets"]["vertices"].as_u64() {
                if vertices > limit {
                    return Err(invalid("tile exceeds recorded vertex budget"));
                }
            }
            if let Some(limit) = self.reports["maxPoints"].as_u64() {
                if vertices > limit {
                    return Err(invalid("tile exceeds recorded point budget"));
                }
            }
            for (key, actual) in [("encodedBytes", bytes), ("vertices", vertices)] {
                if let Some(recorded) = node["extras"][key].as_u64() {
                    if recorded != actual {
                        return Err(invalid(format!("recorded {key} differs from payload")));
                    }
                }
            }
        }
        if let Some(children) = node.as_object_mut().unwrap().remove("children") {
            let Value::Array(children) = children else {
                return Err(invalid("children must be an array"));
            };
            for child in children {
                descendants.push(ValidationTask::Node {
                    node: child,
                    base: base.into(),
                    parent: Some(bounds.clone()),
                    parent_error: error,
                    depth: depth + 1,
                });
            }
        }
        pending.extend(descendants.into_iter().rev());
        Ok(())
    }
}
/// Reject inputs that are not 3TZ (ZIP) archives before opening them, so a
/// directory or other file gets an actionable message instead of an OS error.
fn open_archive(path: &Path) -> Result<File, Error> {
    const HINT: &str = "validate currently checks .3tz archives only; raster and terrain \
        output directories are not validated yet. Pass a .3tz written by vector, \
        point-cloud, mesh-to-3tz, glb-to-3tz or convert";
    if path.is_dir() {
        return Err(invalid(format!(
            "{} is a directory: {HINT}",
            path.display()
        )));
    }
    let mut file = File::open(path).map_err(|error| {
        if error.kind() == std::io::ErrorKind::NotFound {
            Error::Io(std::io::Error::new(
                error.kind(),
                format!("input not found: {}", path.display()),
            ))
        } else {
            Error::Io(error)
        }
    })?;
    if !file.metadata()?.is_file() {
        return Err(Error::unsupported(
            "validation requires a regular .3tz file",
        ));
    }
    let mut magic = [0u8; 4];
    let read = file.read(&mut magic)?;
    if read < 4 || !matches!(&magic, b"PK\x03\x04" | b"PK\x05\x06") {
        return Err(invalid(format!(
            "{} is not a ZIP/.3tz archive: {HINT}",
            path.display()
        )));
    }
    Ok(file)
}

/// Inspect a bounded archive using only declared archive-local resources.
/// Success certifies the named checks, not uninspected scene/source semantics.
pub fn inspect(request: ValidationRequest) -> Result<ValidationReport, Error> {
    let path = request.input();
    let mut source = open_archive(path)?;
    let before = source.metadata()?;
    let identity = same_file::Handle::from_file(source.try_clone()?)?;
    let limits = ValidationLimits::default();
    admission::admit(&mut source, &limits)?;
    crate::archive3tz::validate_open_3tz(source.try_clone()?).map_err(crate::Error::from)?;
    let mut zip = Archive::new(source.try_clone()?)?;
    let mut bytes_read = 0u64;
    let mut names = HashSet::new();
    let mut hashes = HashMap::new();
    for i in 0..zip.len() {
        let mut entry = zip.by_index(i)?;
        let name = entry.name().to_owned();
        if name.split('/').any(|v| matches!(v, "" | "." | ".."))
            || name.starts_with('/')
            || name.contains('\\')
            || !names.insert(name.clone())
        {
            return Err(invalid(format!("unsafe or duplicate archive path: {name}")));
        }
        if entry.size() > limits.member_bytes {
            return Err(Error::limit("archive member exceeds 64 MiB"));
        }
        bytes_read = bytes_read
            .checked_add(entry.size())
            .filter(|&n| n <= limits.total_bytes_read)
            .ok_or_else(|| Error::limit("archive hash reads exceed work limit"))?;
        let mut digest = Sha256::new();
        let mut buffer = [0u8; 65536];
        loop {
            let n = entry.read(&mut buffer).map_err(archive_read_error)?;
            if n == 0 {
                break;
            }
            digest.update(&buffer[..n]);
        }
        let hash = format!("{:x}", digest.finalize());
        let stem = Path::new(&name)
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("");
        if stem.len() == 64 && stem.bytes().all(|b| b.is_ascii_hexdigit()) && stem != hash {
            return Err(invalid(format!("content checksum mismatch: {name}")));
        }
        hashes.insert(name, hash);
    }
    let report = if names.contains("conversion.json") {
        json::parse(&read_member(
            &mut zip,
            "conversion.json",
            &mut bytes_read,
            &limits,
            limits.json_bytes,
        )?)?
    } else {
        Value::Null
    };
    let mut check = Check {
        expansion_budget: crate::implicit::ExpansionBudget::new(
            limits.hierarchy_visits as usize,
            limits.document_decoded_bytes as usize,
        )
        .with_json_parser(|bytes| json::parse(bytes).map_err(crate::Error::from)),
        resources: ArchiveResources {
            zip: &mut zip,
            names,
            used: HashSet::from([TZ_INDEX_NAME.into()]),
            limits,
            bytes_read,
            reference_visits: 0,
        },
        reports: report,
        tiles: 0,
        contents: 0,
        active: HashSet::new(),
        task_visits: 0,
        elements: 0,
        payloads: HashMap::new(),
        not_inspected: BTreeSet::from([
            "decodedContentBounds".into(),
            "metadataSemantics".into(),
            "materialAndImageSemantics".into(),
            "geometricErrorAccuracy".into(),
        ]),
        schema: jsonschema::validator_for(&serde_json::from_str(include_str!(
            "../docs/schema/tileset.schema.json"
        ))?)
        .map_err(|error| invalid(format!("bundled schema compilation failed: {error}")))?,
    };
    if check.resources.names.contains("conversion.json") {
        check.resources.used.insert("conversion.json".into());
        if let Some(name) = check.reports["geometryReports"].as_str() {
            check.resources.reference(uri("conversion.json", name)?)?;
        }
        if check.reports["operation"] == "convert-to-implicit" {
            if let Some(sources) = check.reports["retainedContentUris"].as_array().cloned() {
                for source in sources {
                    let source = uri(
                        "conversion.json",
                        source
                            .as_str()
                            .ok_or_else(|| invalid("invalid retained content URI"))?,
                    )?;
                    check.resources.reference(source.clone())?;
                    check.payload(&source)?;
                }
            }
        }
    }
    let manifest = check.resources.read_json("tileset.json")?;
    if let Some(expected) = manifest["asset"]["extras"]["vectorBuildStateSha256"].as_str() {
        check.resources.reference("vector-build.json".into())?;
        if hashes.get("vector-build.json").map(String::as_str) != Some(expected) {
            return Err(invalid("vector build-state checksum mismatch"));
        }
    }
    check.tileset("tileset.json", None, None, 0)?;
    if let Some(limit) = check.reports["budgets"]["tiles"].as_u64() {
        if check.tiles as u64 > limit {
            return Err(invalid("hierarchy exceeds recorded tile budget"));
        }
    }
    let mut unused: Vec<_> = check
        .resources
        .names
        .difference(&check.resources.used)
        .cloned()
        .collect();
    unused.sort();
    if !unused.is_empty() {
        return Err(invalid(format!(
            "unreferenced archive entries: {}",
            unused.join(", ")
        )));
    }
    let after = source.metadata()?;
    if before.len() != after.len()
        || before.modified()? != after.modified()?
        || identity != same_file::Handle::from_path(path)?
    {
        return Err(invalid("source archive changed during inspection"));
    }
    let mut payloads: Vec<_> = check.payloads.into_values().collect();
    payloads.sort_by(|a, b| a.uri.cmp(&b.uri));
    Ok(ValidationReport {
        ok: true,
        archive: path.into(),
        tiles: check.tiles as u64,
        content_references: check.contents as u64,
        entries: check.resources.names.len() as u64,
        payloads,
        limits: check.resources.limits,
        checks: [
            "archiveIndex",
            "archiveCrc",
            "contentHashes",
            "tilesetSchema",
            "payloadEnvelope",
            "bufferRanges",
            "accessorValues",
            "primitiveIndices",
            "hierarchyBounds",
            "geometricErrorOrder",
            "resourceReferences",
            "recordedBudgets",
        ]
        .into_iter()
        .map(String::from)
        .collect(),
        not_inspected: check.not_inspected.into_iter().collect(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    fn tile(radius: f64, error: f64) -> Value {
        json!({"boundingVolume":{"sphere":[0.,0.,0.,radius]},"geometricError":error,"refine":"REPLACE"})
    }
    fn tileset(root: Value, error: f64) -> Value {
        json!({"asset":{"version":"1.1"},"geometricError":error,"root":root})
    }
    fn check_documents(documents: Vec<(String, Value)>) -> Result<Value, Error> {
        let work = tempfile::tempdir().unwrap();
        let data = work.path().join("data");
        for (name, value) in documents {
            let file = data.join(name);
            std::fs::create_dir_all(file.parent().unwrap()).unwrap();
            std::fs::write(file, serde_json::to_vec(&value).unwrap()).unwrap();
        }
        let output = work.path().join("test.3tz");
        crate::pack::convert_to_3tz(&data, &output, &Default::default()).unwrap();
        inspect(ValidationRequest::new(&output)).map(|r| serde_json::to_value(r).unwrap())
    }
    #[test]
    fn external_roots_obey_referring_bounds_and_error() {
        let mut root = tile(1., 0.);
        root["content"] = json!({"uri":"external.json"});
        let mut outside = tile(0.5, 50.);
        outside["boundingVolume"]["sphere"][0] = json!(10000.);
        let mut translated = tile(0.5, 0.);
        let mut m = IDENTITY;
        m[12] = 10000.;
        translated["transform"] = json!(m);
        for (child, message) in [
            (outside, "bounds escape"),
            (translated, "bounds escape"),
            (tile(0.5, 50.), "geometricError increases"),
        ] {
            let error = check_documents(vec![
                ("tileset.json".into(), tileset(root.clone(), 0.)),
                ("external.json".into(), tileset(child, 50.)),
            ])
            .unwrap_err();
            assert!(error.to_string().contains(message), "{error}");
        }
    }
    #[test]
    fn shared_external_tileset_is_checked_at_each_placement() {
        let mut children = vec![];
        for x in [-10., 10.] {
            let mut child = tile(1., 0.);
            let mut m = IDENTITY;
            m[12] = x;
            child["transform"] = json!(m);
            child["content"] = json!({"uri":"nested/external.json"});
            children.push(child);
        }
        let mut root = tile(20., 0.);
        root["children"] = json!(children);
        let docs = |root| {
            vec![
                ("tileset.json".into(), tileset(root, 0.)),
                ("nested/external.json".into(), tileset(tile(0.5, 0.), 0.)),
            ]
        };
        let report = check_documents(docs(root.clone())).unwrap();
        assert_eq!(report["tiles"], 5);
        assert_eq!(report["contentReferences"], 2);
        root["children"][1]["boundingVolume"]["sphere"][3] = json!(0.1);
        assert!(check_documents(docs(root))
            .unwrap_err()
            .to_string()
            .contains("bounds escape"));
    }
    #[test]
    fn external_cycles_fail_and_depth_continues_across_files() {
        std::thread::Builder::new()
            .stack_size(1024 * 1024)
            .spawn(check_external_cycles_and_depth)
            .unwrap()
            .join()
            .unwrap();
    }
    fn check_external_cycles_and_depth() {
        let mut root = tile(1., 0.);
        root["content"] = json!({"uri":"nested/external.json"});
        let mut child = tile(1., 0.);
        child["content"] = json!({"uri":"../tileset.json"});
        assert!(check_documents(vec![
            ("tileset.json".into(), tileset(root, 0.)),
            ("nested/external.json".into(), tileset(child, 0.)),
        ])
        .unwrap_err()
        .to_string()
        .contains("cyclic external"));
        for length in [128, 129] {
            let mut docs = vec![];
            for depth in 0..=length {
                let name = if depth == 0 {
                    "tileset.json".into()
                } else {
                    format!("nested/{depth}.json")
                };
                let mut node = tile(1., 0.);
                if depth < length {
                    node["content"] = json!({"uri":if depth == 0 { "nested/1.json".into() } else { format!("{}.json", depth+1) }});
                }
                docs.push((name, tileset(node, 0.)));
            }
            let result = check_documents(docs);
            if length == 128 {
                assert_eq!(result.unwrap()["tiles"], 129);
            } else {
                assert!(result.unwrap_err().to_string().contains("depth limit"));
            }
        }
    }
    #[test]
    fn metadata_schemas_are_resolved_relative_to_each_document() {
        let schema = json!({"id":"fixture","classes":{}});
        let mut doc = tileset(tile(1., 0.), 0.);
        doc["schemaUri"] = json!("schemas/schema.json");
        let missing = check_documents(vec![("tileset.json".into(), doc.clone())]).unwrap_err();
        assert!(missing
            .to_string()
            .contains("missing archive entry: schemas/schema.json"));
        assert!(check_documents(vec![
            ("tileset.json".into(), doc.clone()),
            ("schemas/schema.json".into(), schema.clone()),
        ])
        .is_ok());
        doc["root"]["content"] = json!({"uri":"nested/external.json"});
        let mut external = tileset(tile(0.5, 0.), 0.);
        external["schemaUri"] = json!("../schemas/schema.json");
        external["root"]["content"] = json!({"uri":"../models/content.gltf"});
        let gltf = json!({"asset":{"version":"2.0"},"extensionsUsed":["EXT_structural_metadata"],
            "extensions":{"EXT_structural_metadata":{"schemaUri":"../schemas/schema.json"}}});
        assert!(check_documents(vec![
            ("tileset.json".into(), doc),
            ("nested/external.json".into(), external),
            ("models/content.gltf".into(), gltf.clone()),
            ("schemas/schema.json".into(), schema),
        ])
        .is_ok());
        let mut root = tile(1., 0.);
        root["content"] = json!({"uri":"models/content.gltf"});
        assert!(check_documents(vec![
            ("tileset.json".into(), tileset(root, 0.)),
            ("models/content.gltf".into(), gltf),
        ])
        .unwrap_err()
        .to_string()
        .contains("missing archive entry: schemas/schema.json"));
    }
    #[test]
    fn containment_uses_child_transform_and_rotated_half_axes() {
        let parent = Volume::Box(vec![0., 0., 0., 0., 10., 0., -2., 0., 0., 0., 0., 1.]);
        let child = Volume::Box(vec![0., 0., 0., 0.5, 0., 0., 0., 0.5, 0., 0., 0., 0.5]);
        let mut m = IDENTITY;
        m[12] = 1.;
        m[13] = 8.;
        assert!(contains(&parent, &child, &m).unwrap());
        m[12] = 2.;
        assert!(!contains(&parent, &child, &m).unwrap());
    }
    #[test]
    fn region_containment_crosses_antimeridian_without_transforming_regions() {
        let p = Volume::Region(vec![3., -0.5, -3., 0.5, 0., 100.]);
        let child = Volume::Region(vec![3.1, -0.1, -3.1, 0.1, 10., 90.]);
        assert!(contains(&p, &child, &IDENTITY).unwrap());
        let outside = Volume::Region(vec![2.9, -0.1, -3.1, 0.1, 10., 90.]);
        assert!(!contains(&p, &outside, &IDENTITY).unwrap());
    }
    #[test]
    fn degenerate_box_checks_out_of_plane_and_sphere_extent() {
        let flat = Volume::Box(vec![0., 0., 0., 1., 0., 0., 0., 1., 0., 0., 0., 0.]);
        assert!(inside(&flat, [0.5, 0.5, 0.], 0.));
        assert!(!inside(&flat, [0., 0., 0.01], 0.));
        assert!(!inside(&flat, [0., 0., 0.], 0.01));
        assert!(inside(
            &Volume::Sphere(vec![0., 0., 0., 2.]),
            [1., 0., 0.],
            1.
        ));
    }
}
