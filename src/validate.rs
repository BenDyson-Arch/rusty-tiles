//! Read-only validation of self-contained explicit and native implicit 3TZ packages.
use crate::archive3tz::TZ_INDEX_NAME;
use crate::content_integrity::{
    json, payload, FormatError, JsonLimits, PayloadError, PayloadLimits,
};
mod containment;
mod types;
pub use types::{
    PayloadReport, ValidationFailure, ValidationLimits, ValidationReport, ValidationRequest,
};
type Error = ValidationFailure;
#[cfg(test)]
use serde_json::json;
use serde_json::Value;
use std::{
    collections::{BTreeSet, HashMap, HashSet},
    fs::File,
    io::Read,
    path::Path,
};

type Archive = crate::archive3tz::StoredArchive<File>;
fn invalid(message: impl Into<String>) -> Error {
    Error::InvalidInput(message.into())
}
// Select and host-admit the six consumed format facts once at the operation boundary.
fn content_limits(limits: &ValidationLimits) -> Result<PayloadLimits, Error> {
    let host = |value: u64| {
        usize::try_from(value).map_err(|_| Error::limit("content limit exceeds host range"))
    };
    Ok(PayloadLimits {
        member_bytes: host(limits.member_bytes)?,
        decoded_bytes: host(limits.document_decoded_bytes)?,
        accessor_components: limits.accessor_elements,
        json: JsonLimits {
            bytes: host(limits.json_bytes)?,
            depth: limits.json_depth,
            value_nodes: host(limits.document_items)?,
        },
    })
}
fn format_error(error: FormatError) -> Error {
    match error {
        FormatError::InvalidInput(message) => Error::InvalidInput(message),
        FormatError::Unsupported(message) => Error::Unsupported(message),
        FormatError::ResourceLimit(message) => Error::ResourceLimit(message),
    }
}
fn payload_error(error: PayloadError<Error>) -> Error {
    match error {
        PayloadError::Format(error) => format_error(error),
        PayloadError::Resolver(error) => error,
    }
}
// A real resolver I/O cause bypasses retained generic archive conversions.
fn implicit_error(error: crate::Error) -> Error {
    match error {
        crate::Error::Io(error) => Error::Io(error),
        crate::Error::Validation(error) => error,
        other => other.into(),
    }
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
// Native archive reads preserve their typed format/I/O origins before the
// retained generic crate/ZIP conversions above can reinterpret an I/O kind.
fn archive_admission_error(error: crate::archive3tz::ReadError) -> ValidationFailure {
    use crate::archive3tz::ReadError;
    match error {
        ReadError::InvalidInput(message) => Error::InvalidInput(message),
        ReadError::Unsupported(message) => Error::Unsupported(message),
        ReadError::ResourceLimit(message) => Error::ResourceLimit(message),
        ReadError::Io(error) => Error::Io(error),
        ReadError::MissingMember(name) => invalid(format!("missing archive member: {name}")),
    }
}
fn read_member(
    archive: &mut Archive,
    name: &str,
    bytes_read: &mut u64,
    limits: &ValidationLimits,
    maximum: u64,
) -> Result<Vec<u8>, Error> {
    let size = archive.member(name).map_err(archive_admission_error)?.size;
    if size > maximum {
        return Err(Error::limit(format!(
            "member exceeds admitted byte limit: {name}"
        )));
    }
    *bytes_read = bytes_read
        .checked_add(size)
        .filter(|&n| n <= limits.total_bytes_read)
        .ok_or_else(|| Error::limit("member reads exceed 2 GiB work limit"))?;
    archive
        .read_member(name, maximum)
        .map_err(archive_admission_error)
}
/// A C2 phase-2 re-read: identical members to phase 1, so the precheck's
/// `containmentBytesRead` total bounds it. Exceeding it means the archive changed.
fn read_rerun(
    archive: &mut Archive,
    name: &str,
    reread: &mut u64,
    cap: u64,
    maximum: u64,
) -> Result<Vec<u8>, Error> {
    let size = archive.member(name).map_err(archive_admission_error)?.size;
    *reread = reread
        .checked_add(size)
        .filter(|&n| n <= cap && size <= maximum)
        .ok_or_else(|| invalid("source archive changed during inspection"))?;
    archive
        .read_member(name, maximum)
        .map_err(archive_admission_error)
}
fn uri(base: &str, value: &str) -> Result<String, Error> {
    if value.is_empty() || value.contains('\\') {
        return Err(invalid(format!("invalid resource URI: {value}")));
    }
    if value.contains([':', '?', '#']) || value.starts_with('/') {
        return Err(Error::unsupported(format!(
            "URI outside archive-local profile: {value}"
        )));
    }
    let mut parts: Vec<String> = base.rsplit_once('/').map_or(vec![], |(dir, _)| {
        dir.split('/').map(str::to_owned).collect()
    });
    // Decode each segment once. Encoded separators must never become path
    // syntax, and a literal percent in a decoded filename stays literal.
    for raw in value.split('/') {
        let mut decoded = Vec::with_capacity(raw.len());
        let mut bytes = raw.bytes();
        while let Some(byte) = bytes.next() {
            decoded.push(if byte == b'%' {
                let digit = |byte: u8| (byte as char).to_digit(16).map(|n| n as u8);
                let high = bytes.next().and_then(digit);
                let low = bytes.next().and_then(digit);
                let (Some(high), Some(low)) = (high, low) else {
                    return Err(invalid("invalid URI percent escape"));
                };
                high * 16 + low
            } else {
                byte
            });
        }
        let part =
            String::from_utf8(decoded).map_err(|_| invalid("URI decoded path must be UTF-8"))?;
        if part.contains(['/', '\\']) || part.chars().any(char::is_control) {
            return Err(invalid("URI decoded segment contains a separator/control"));
        }
        match part.as_str() {
            "" | "." => (),
            ".." => {
                if parts.pop().is_none() {
                    return Err(invalid("URI escapes archive"));
                }
            }
            _ => parts.push(part),
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
// ---- C2 decoded-content containment: operation owner ----
use containment::{
    Arena, Exclusion, LevelFacts, Local, Normalization, RawVertex, Role, SpanCaps, UriBounds,
    Verdict, VolumeFacts, Word,
};
use payload::scene::{
    ContentExclusions, Instance, NodeTransform, PositionFormat, PositionWord, SceneConsumer,
};

/// The frozen 15-name not-inspected reason enum.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Reason {
    ImplicitTiling,
    TilesetExtension,
    TileMetadata,
    UpAxisOverride,
    RegionVolume,
    RotatedBox,
    RtcCenter,
    GltfExtension,
    SceneSelection,
    Skin,
    MorphTargets,
    Animation,
    QuaternionWindow,
    NumericSpan,
    WorkLimit,
}
const REASONS: [Reason; 15] = [
    Reason::ImplicitTiling,
    Reason::TilesetExtension,
    Reason::TileMetadata,
    Reason::UpAxisOverride,
    Reason::RegionVolume,
    Reason::RotatedBox,
    Reason::RtcCenter,
    Reason::GltfExtension,
    Reason::SceneSelection,
    Reason::Skin,
    Reason::MorphTargets,
    Reason::Animation,
    Reason::QuaternionWindow,
    Reason::NumericSpan,
    Reason::WorkLimit,
];
impl Reason {
    fn name(self) -> &'static str {
        match self {
            Self::ImplicitTiling => "implicitTiling",
            Self::TilesetExtension => "tilesetExtension",
            Self::TileMetadata => "tileMetadata",
            Self::UpAxisOverride => "upAxisOverride",
            Self::RegionVolume => "regionVolume",
            Self::RotatedBox => "rotatedBox",
            Self::RtcCenter => "rtcCenter",
            Self::GltfExtension => "gltfExtension",
            Self::SceneSelection => "sceneSelection",
            Self::Skin => "skin",
            Self::MorphTargets => "morphTargets",
            Self::Animation => "animation",
            Self::QuaternionWindow => "quaternionWindow",
            Self::NumericSpan => "numericSpan",
            Self::WorkLimit => "workLimit",
        }
    }
    fn bit(self) -> u16 {
        1 << self as u16
    }
}
const TILESET_ALLOWLIST: [&str; 1] = ["3DTILES_content_gltf_vector"];
fn content_reasons(facts: &ContentExclusions) -> u16 {
    [
        (facts.rtc_center, Reason::RtcCenter),
        (facts.gltf_extension, Reason::GltfExtension),
        (facts.scene_selection, Reason::SceneSelection),
        (facts.skin, Reason::Skin),
        (facts.morph_targets, Reason::MorphTargets),
        (facts.animation, Reason::Animation),
    ]
    .into_iter()
    .filter(|(applies, _)| *applies)
    .fold(0, |bits, (_, reason)| bits | reason.bit())
}
/// A capacity breach inside the exact arithmetic is a validator defect, never a verdict.
fn containment_fault() -> Error {
    Error::InvalidState("internal containment invariant: exact arena capacity exceeded".into())
}
fn refused() -> Error {
    Error::limit("containment storage allocation refused")
}
fn try_push<T>(values: &mut Vec<T>, value: T) -> Result<(), Error> {
    values.try_reserve(1).map_err(|_| refused())?;
    values.push(value);
    Ok(())
}
fn volume_facts(volume: &Volume) -> VolumeFacts {
    match volume {
        Volume::Box(b) => VolumeFacts::Box(std::array::from_fn(|i| b[i])),
        Volume::Sphere(s) => VolumeFacts::Sphere(std::array::from_fn(|i| s[i])),
        Volume::Region(_) => VolumeFacts::Region,
    }
}
fn local(transform: &NodeTransform) -> Local {
    match *transform {
        NodeTransform::Skip => Local::Skip,
        NodeTransform::Matrix(m) => Local::Matrix(m),
        NodeTransform::Trs { t, q, s } => Local::Trs { t, q, s },
    }
}
fn normalization(format: PositionFormat) -> Normalization {
    match format {
        PositionFormat::F32 => Normalization::None(Word::F32),
        PositionFormat::I8 { normalized: true } => Normalization::I8,
        PositionFormat::U8 { normalized: true } => Normalization::U8,
        PositionFormat::I16 { normalized: true } => Normalization::I16,
        PositionFormat::U16 { normalized: true } => Normalization::U16,
        PositionFormat::I8 { .. } | PositionFormat::U8 { .. } => Normalization::None(Word::Int8),
        PositionFormat::I16 { .. } | PositionFormat::U16 { .. } => Normalization::None(Word::Int16),
    }
}
fn raw_vertex(word: PositionWord) -> RawVertex {
    match word {
        PositionWord::F32(v) => RawVertex::F32(v),
        PositionWord::Int(v) => RawVertex::Int(v),
    }
}
fn scene_error(error: payload::SceneError<Error, Error>) -> Error {
    match error {
        payload::SceneError::Payload(error) => payload_error(error),
        payload::SceneError::Consumer(error) => error,
    }
}
fn payload_kind(name: &str) -> Result<payload::PayloadKind, Error> {
    match name.to_ascii_lowercase().rsplit_once('.') {
        Some((_, "glb")) => Ok(payload::PayloadKind::Glb),
        Some((_, "gltf")) => Ok(payload::PayloadKind::LocalGltf),
        Some((_, "b3dm")) => Ok(payload::PayloadKind::B3dm),
        _ => Err(Error::unsupported(
            "payload format outside GLB/local glTF 2.0",
        )),
    }
}

/// Per-URI static facts, aggregated over every instance during the first C1
/// inspection (D4). Never a verdict.
#[derive(Clone, Copy)]
struct UriFacts {
    /// Hull over every (instance, distinct POSITION accessor); None without instances.
    bounds: Option<UriBounds>,
    t: u64,
    /// None when the checked sum overflowed (counts as over the cap).
    nodes: Option<u64>,
    idx: u64,
    e: u64,
    dec: u64,
    b: u64,
    max_depth: u64,
    exclusions: u16,
}
/// One tile of the traversed hierarchy: f64 facts only.
struct TileFact {
    /// The tile's own transform into its parent frame, when present.
    transform: Option<[f64; 16]>,
    volume: VolumeFacts,
    /// For an external-tileset root: the referencing content's boundingVolume.
    external: Option<VolumeFacts>,
    parent: Option<usize>,
    depth: usize,
}
/// One payload content reference, in traversal order.
struct Reference {
    tile: usize,
    uri: usize,
    content: Option<usize>,
}
struct Containment {
    caps: SpanCaps,
    work_cap: u64,
    bytes_cap: u64,
    decoded_cap: u64,
    exclusions: u16,
    precheck: Option<Reason>,
    units: u64,
    bytes: u64,
    decoded: u64,
    tiles: Vec<TileFact>,
    references: Vec<Reference>,
    content_volumes: Vec<VolumeFacts>,
    uris: Vec<(String, UriFacts)>,
    uri_index: HashMap<String, usize>,
    levels: Vec<LevelFacts>,
    arena: Option<Arena>,
}
impl Containment {
    fn new(limits: &ValidationLimits) -> Self {
        Self {
            caps: SpanCaps {
                span_bits: limits.containment_span_bits,
                node_depth: limits.containment_node_depth,
            },
            work_cap: limits.containment_work_units,
            bytes_cap: limits.containment_bytes_read,
            decoded_cap: limits.containment_decoded_bytes,
            exclusions: 0,
            precheck: None,
            units: 0,
            bytes: 0,
            decoded: 0,
            tiles: Vec::new(),
            references: Vec::new(),
            content_volumes: Vec::new(),
            uris: Vec::new(),
            uri_index: HashMap::new(),
            levels: Vec::new(),
            arena: None,
        }
    }
    fn exclude(&mut self, reason: Reason) {
        self.exclusions |= reason.bit();
    }
    fn arena(arena: &mut Option<Arena>, caps: SpanCaps) -> Result<&mut Arena, Error> {
        if arena.is_none() {
            *arena = Some(Arena::reserve(caps).map_err(|_| refused())?);
        }
        Ok(arena.as_mut().unwrap())
    }
    fn tile(&mut self, fact: TileFact) -> Result<usize, Error> {
        try_push(&mut self.tiles, fact)?;
        Ok(self.tiles.len() - 1)
    }
    /// The level chain of a reference: own tile and content volume at level 0,
    /// then each ancestor through external-tileset roots (I1, I2).
    fn build_levels(&mut self, tile: usize, content: Option<usize>) -> Result<(), Error> {
        self.levels.clear();
        let content = content.map(|c| (Role::Content, self.content_volumes[c]));
        try_push(
            &mut self.levels,
            LevelFacts {
                to_parent: None,
                volumes: [Some((Role::Own, self.tiles[tile].volume)), content, None],
            },
        )?;
        let mut current = tile;
        while let Some(parent) = self.tiles[current].parent {
            let level = self.levels.len() as u32;
            let child = &self.tiles[current];
            let fact = LevelFacts {
                to_parent: child.transform,
                volumes: [
                    Some((Role::Ancestor(level), self.tiles[parent].volume)),
                    None,
                    child.external.map(|v| (Role::ExternalContent(level), v)),
                ],
            };
            try_push(&mut self.levels, fact)?;
            current = parent;
        }
        Ok(())
    }
    /// Collect chain exclusions and run the precheck step for one reference, in
    /// traversal order. The precheck stops at its first violation.
    fn reference(
        &mut self,
        tile: usize,
        content: Option<VolumeFacts>,
        uri: usize,
    ) -> Result<(), Error> {
        let content = match content {
            Some(volume) => {
                try_push(&mut self.content_volumes, volume)?;
                Some(self.content_volumes.len() - 1)
            }
            None => None,
        };
        try_push(&mut self.references, Reference { tile, uri, content })?;
        let facts = self.uris[uri].1;
        self.exclusions |= facts.exclusions;
        self.build_levels(tile, content)?;
        let mut chain = 0;
        for level in &self.levels {
            for (_, volume) in level.volumes.iter().flatten() {
                match containment::volume_exclusion(volume) {
                    Some(Exclusion::RegionVolume) => chain |= Reason::RegionVolume.bit(),
                    Some(Exclusion::RotatedBox) => chain |= Reason::RotatedBox.bit(),
                    Some(Exclusion::NumericSpan) | None => {}
                }
            }
        }
        self.exclusions |= chain;
        // Any exclusion anywhere overrides the precheck reason, so the precheck
        // only runs while the archive is still entirely in domain.
        if self.exclusions != 0 || self.precheck.is_some() {
            return Ok(());
        }
        self.precheck = self.precheck_step(&facts);
        Ok(())
    }
    fn precheck_step(&mut self, facts: &UriFacts) -> Option<Reason> {
        let vertex_units = match &facts.bounds {
            None => 0,
            Some(bounds) => match containment::level_spans(bounds, &self.levels, self.caps) {
                Err(Exclusion::NumericSpan) => return Some(Reason::NumericSpan),
                Err(Exclusion::RegionVolume) => return Some(Reason::RegionVolume),
                Err(Exclusion::RotatedBox) => return Some(Reason::RotatedBox),
                Ok(spans) => match containment::vertex_units(facts.t, &spans) {
                    Some(units) => units,
                    None => return Some(Reason::WorkLimit),
                },
            },
        };
        if facts.max_depth > self.caps.node_depth {
            return Some(Reason::WorkLimit);
        }
        let units = facts.nodes.and_then(|nodes| {
            vertex_units
                .checked_add(nodes)?
                .checked_add(facts.idx)?
                .checked_add(facts.e)
        });
        let within = |total: u64, add: Option<u64>, cap: u64| {
            add.and_then(|add| total.checked_add(add))
                .filter(|sum| *sum <= cap)
        };
        let (Some(units), Some(bytes), Some(decoded)) = (
            within(self.units, units, self.work_cap),
            within(self.bytes, Some(facts.b), self.bytes_cap),
            within(self.decoded, Some(facts.dec), self.decoded_cap),
        ) else {
            return Some(Reason::WorkLimit);
        };
        self.units = units;
        self.bytes = bytes;
        self.decoded = decoded;
        None
    }
}

/// Phase 1: per-URI static facts through M's bound functions, plus the
/// quaternion window over every selected node.
struct FactConsumer<'a> {
    arena: &'a mut Option<Arena>,
    caps: SpanCaps,
    /// Composite bound and the maximum `s_step` along the current path.
    stack: Vec<(containment::Composite, u32)>,
    bounds: Option<UriBounds>,
    t: u64,
    nodes: Option<u64>,
    idx: u64,
    max_depth: u64,
    quaternion_window: bool,
}
impl SceneConsumer for FactConsumer<'_> {
    type Error = Error;
    fn instances_only(&self) -> bool {
        false
    }
    fn enter(&mut self, transform: &NodeTransform, depth: u64) -> Result<(), Error> {
        self.max_depth = self.max_depth.max(depth);
        let l = local(transform);
        if let Local::Trs { q, .. } = l {
            let arena = Containment::arena(self.arena, self.caps)?;
            let outcome = containment::admit_rotation(q, arena);
            if arena.fault() {
                return Err(containment_fault());
            }
            match outcome {
                containment::RotationOutcome::Ok => {}
                containment::RotationOutcome::QuaternionWindow => self.quaternion_window = true,
                containment::RotationOutcome::InvalidZeroOrRange => {
                    return Err(invalid(
                        "glTF node rotation must be nonzero with components in [-1, 1]",
                    ))
                }
            }
        }
        if l == Local::Skip {
            self.nodes = self.nodes.and_then(|n| n.checked_add(1));
            return Ok(());
        }
        let (g, path) = match self.stack.last() {
            Some((g, path)) => (*g, *path),
            None => (containment::composite_identity(), 0),
        };
        let (g, step) = containment::node_step_bound(&g, &l);
        self.nodes = self
            .nodes
            .and_then(|n| n.checked_add(containment::node_units(step)));
        try_push(&mut self.stack, (g, path.max(step)))
    }
    fn leave(&mut self, transform: &NodeTransform) {
        if *transform != NodeTransform::Skip {
            self.stack.pop();
        }
    }
    fn instance(&mut self, instance: &Instance<'_, '_>) -> Result<(), Error> {
        let (g, s_inst) = match self.stack.last() {
            Some((g, path)) => (*g, *path),
            None => (containment::composite_identity(), 0),
        };
        for &accessor in instance.positions() {
            let (pair0, s_pos) =
                containment::position_bound(&g, normalization(instance.format(accessor)));
            let member = UriBounds {
                pair0,
                s_inst,
                s_pos,
            };
            self.bounds = Some(match &self.bounds {
                Some(bounds) => containment::hull(bounds, &member),
                None => containment::hull(&UriBounds::identity(), &member),
            });
        }
        self.t = self.t.saturating_add(instance.vertex_total());
        self.idx = self.idx.saturating_add(instance.index_total());
        Ok(())
    }
}

/// Phase 2: serial streaming of one reference over the live C1 views.
struct StreamConsumer<'a> {
    arena: &'a mut Arena,
    bitmap: &'a mut Vec<u64>,
    levels: &'a [LevelFacts],
    member: &'a str,
    depth: usize,
}
impl SceneConsumer for StreamConsumer<'_> {
    type Error = Error;
    fn instances_only(&self) -> bool {
        true
    }
    fn enter(&mut self, transform: &NodeTransform, _: u64) -> Result<(), Error> {
        let l = local(transform);
        if l != Local::Skip {
            containment::push_node(self.arena, &l);
            if self.arena.fault() {
                return Err(containment_fault());
            }
        }
        Ok(())
    }
    fn leave(&mut self, transform: &NodeTransform) {
        if *transform != NodeTransform::Skip {
            containment::pop_node(self.arena);
        }
    }
    fn instance(&mut self, instance: &Instance<'_, '_>) -> Result<(), Error> {
        for &accessor in instance.positions() {
            let words = instance.count(accessor).div_ceil(64);
            if self.bitmap.len() < words {
                self.bitmap
                    .try_reserve_exact(words - self.bitmap.len())
                    .map_err(|_| refused())?;
                self.bitmap.resize(words, 0);
            }
            let bitmap = &mut self.bitmap[..words];
            bitmap.fill(0);
            instance.mark(accessor, |v| bitmap[v / 64] |= 1 << (v % 64));
            let n = normalization(instance.format(accessor));
            for (w, &bits) in bitmap.iter().enumerate() {
                let mut bits = bits;
                while bits != 0 {
                    let vertex = w * 64 + bits.trailing_zeros() as usize;
                    bits &= bits - 1;
                    let raw = raw_vertex(instance.vertex(accessor, vertex));
                    let verdict = containment::walk(self.arena, n, raw, self.levels);
                    if self.arena.fault() {
                        return Err(containment_fault());
                    }
                    if let Verdict::Outside { role, .. } = verdict {
                        let volume = match role {
                            Role::Own => "own".to_owned(),
                            Role::Content => "content".to_owned(),
                            Role::ExternalContent(k) => format!("externalContent:{k}"),
                            Role::Ancestor(k) => format!("ancestor:{k}"),
                        };
                        let primitive = instance.primitive_drawing(accessor, vertex).unwrap_or(0);
                        return Err(invalid(format!(
                            "decoded content outside bounding volume: content={} depth={} volume={volume} node={} mesh={} primitive={primitive} vertex={vertex}",
                            self.member,
                            self.depth,
                            instance.node(),
                            instance.mesh(),
                        )));
                    }
                }
            }
        }
        Ok(())
    }
}

struct ArchiveResources<'a> {
    archive: &'a mut Archive,
    names: HashSet<String>,
    used: HashSet<String>,
    limits: ValidationLimits,
    content_limits: PayloadLimits,
    bytes_read: u64,
    reference_visits: u64,
}
impl ArchiveResources<'_> {
    fn read_json(&mut self, name: &str) -> Result<Value, Error> {
        let bytes = read_member(
            self.archive,
            name,
            &mut self.bytes_read,
            &self.limits,
            self.limits.json_bytes,
        )?;
        json::parse(&bytes, self.content_limits.json).map_err(format_error)
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
    c2: Containment,
}
/// The C2 level-chain link of a traversal task: the parent tile in the tile
/// arena, and for an external-tileset root the referencing content volume.
#[derive(Clone, Copy)]
struct Link {
    parent: Option<usize>,
    external: Option<VolumeFacts>,
}
enum ValidationTask {
    Tileset {
        name: String,
        parent: Option<Volume>,
        parent_error: Option<f64>,
        depth: usize,
        link: Link,
    },
    Node(NodeTask),
    ExitTileset(String),
    /// A C2 payload reference that follows an external-tileset content of the
    /// same tile: registered after that tileset, in pre-order (v3.3 item 4).
    /// Not a hierarchy traversal step, so not charged to `hierarchy_visits`.
    Reference {
        tile: usize,
        content: Option<VolumeFacts>,
        uri: usize,
    },
}
struct NodeTask {
    node: Value,
    base: String,
    parent: Option<Volume>,
    parent_error: f64,
    depth: usize,
    link: Link,
}
impl Check<'_> {
    fn payload(&mut self, name: &str) -> Result<PayloadReport, Error> {
        if let Some(report) = self.payloads.get(name) {
            return Ok(report.clone());
        }
        let bytes_before = self.resources.bytes_read;
        let bytes = read_member(
            self.resources.archive,
            name,
            &mut self.resources.bytes_read,
            &self.resources.limits,
            self.resources.limits.member_bytes,
        )?;
        let remaining = self.resources.limits.total_payload_elements - self.elements;
        let kind = payload_kind(name)?;
        let mut consumer = FactConsumer {
            arena: &mut self.c2.arena,
            caps: self.c2.caps,
            stack: Vec::new(),
            bounds: None,
            t: 0,
            nodes: Some(0),
            idx: 0,
            max_depth: 0,
            quaternion_window: false,
        };
        let resources = &mut self.resources;
        let facts = payload::inspect_scene(
            kind,
            &bytes,
            resources.content_limits,
            remaining,
            |value, maximum| {
                let resource = uri(name, value)?;
                resources.reference(resource.clone())?;
                read_member(
                    resources.archive,
                    &resource,
                    &mut resources.bytes_read,
                    &resources.limits,
                    maximum as u64,
                )
            },
            &mut consumer,
        )
        .map_err(scene_error)?;
        let mut exclusions = content_reasons(&facts.exclusions);
        if consumer.quaternion_window {
            exclusions |= Reason::QuaternionWindow.bit();
        }
        let uri_facts = UriFacts {
            bounds: consumer.bounds,
            t: consumer.t,
            nodes: consumer.nodes,
            idx: consumer.idx,
            e: facts.elements_checked,
            dec: facts.materialized_bytes,
            b: self.resources.bytes_read - bytes_before,
            max_depth: consumer.max_depth,
            exclusions,
        };
        self.c2.uri_index.try_reserve(1).map_err(|_| refused())?;
        try_push(&mut self.c2.uris, (name.to_owned(), uri_facts))?;
        self.c2
            .uri_index
            .insert(name.to_owned(), self.c2.uris.len() - 1);
        self.elements = self
            .elements
            .checked_add(facts.elements_checked)
            .filter(|&n| n <= self.resources.limits.total_payload_elements)
            .ok_or_else(|| Error::limit("payload elements exceed 16 million"))?;
        if let Some(value) = facts.schema_uri.as_deref() {
            self.schema_uri(name, value)?;
        }
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
    /// Phase 2 (outcome 4): serial per-reference streaming, in traversal order.
    /// Re-reads are charged only to `containmentBytesRead`; C1 counters are untouched.
    fn stream(&mut self) -> Result<(), Error> {
        let mut arena = match self.c2.arena.take() {
            Some(arena) => arena,
            None => Arena::reserve(self.c2.caps).map_err(|_| refused())?,
        };
        let mut bitmap = Vec::new();
        let mut reread = 0u64;
        let cap = self.resources.limits.containment_bytes_read;
        let limits = self.resources.content_limits;
        let references = std::mem::take(&mut self.c2.references);
        for reference in &references {
            self.c2.build_levels(reference.tile, reference.content)?;
            let name = self.c2.uris[reference.uri].0.as_str();
            let archive = &mut *self.resources.archive;
            let bytes = read_rerun(archive, name, &mut reread, cap, limits.member_bytes as u64)?;
            let mut consumer = StreamConsumer {
                arena: &mut arena,
                bitmap: &mut bitmap,
                levels: &self.c2.levels,
                member: name,
                depth: self.c2.tiles[reference.tile].depth,
            };
            payload::inspect_scene(
                payload_kind(name)?,
                &bytes,
                limits,
                limits.accessor_components,
                |value, maximum| {
                    read_rerun(
                        archive,
                        &uri(name, value)?,
                        &mut reread,
                        cap,
                        maximum as u64,
                    )
                },
                &mut consumer,
            )
            .map_err(scene_error)?;
        }
        Ok(())
    }
    fn metadata_schema(&mut self, base: &str, doc: &Value) -> Result<(), Error> {
        if let Some(value) = doc.get("schemaUri") {
            let value = value
                .as_str()
                .ok_or_else(|| invalid("schemaUri must be a string"))?;
            self.schema_uri(base, value)?;
        }
        Ok(())
    }
    fn schema_uri(&mut self, base: &str, value: &str) -> Result<(), Error> {
        let name = uri(base, value)?;
        self.resources.reference(name.clone())?;
        self.resources.read_json(&name)?;
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
            link: Link {
                parent: None,
                external: None,
            },
        }];
        while let Some(task) = pending.pop() {
            if let ValidationTask::Reference { tile, content, uri } = task {
                self.c2.reference(tile, content, uri)?;
                continue;
            }
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
                    link,
                } => {
                    if depth > 128 {
                        return Err(Error::limit("hierarchy exceeds validation depth limit"));
                    }
                    if !self.active.insert(name.clone()) {
                        return Err(invalid("cyclic external tileset reference"));
                    }
                    let (mut doc, error) = self.prepare_tileset(&name)?;
                    pending.push(ValidationTask::ExitTileset(name.clone()));
                    pending.push(ValidationTask::Node(NodeTask {
                        node: doc["root"].take(),
                        base: name,
                        parent,
                        parent_error: parent_error.map_or(error, |parent| parent.min(error)),
                        depth,
                        link,
                    }));
                }
                ValidationTask::Node(task) => {
                    self.node(task, &mut pending)?;
                }
                ValidationTask::ExitTileset(name) => {
                    self.active.remove(&name);
                }
                ValidationTask::Reference { .. } => unreachable!("handled before charging"),
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
        self.tileset_exclusions(&doc);
        let doc = self.expand_implicit(name, doc)?;
        Ok((doc, error))
    }

    /// Document-level C2 exclusion facts, before implicit expansion.
    fn tileset_exclusions(&mut self, doc: &Value) {
        if doc["root"].get("implicitTiling").is_some() {
            self.c2.exclude(Reason::ImplicitTiling);
        }
        if let Some(axis) = doc["asset"].get("gltfUpAxis") {
            if axis.as_str() != Some("Y") {
                self.c2.exclude(Reason::UpAxisOverride);
            }
        }
        for key in ["extensionsUsed", "extensionsRequired"] {
            if let Some(names) = doc.get(key) {
                let listed = names.as_array().is_none_or(|names| {
                    names.iter().any(|name| {
                        name.as_str()
                            .is_none_or(|name| !TILESET_ALLOWLIST.contains(&name))
                    })
                });
                if listed {
                    self.c2.exclude(Reason::TilesetExtension);
                }
            }
        }
        self.extension_object(doc);
    }
    /// An `extensions` object naming anything outside the tileset allowlist. A
    /// `3DTILES_content_gltf_vector` object with `clip: true` permits clipped
    /// content outside the volume, so it is a non-allowlisted use (v3.3 item 3).
    fn extension_object(&mut self, owner: &Value) {
        if let Some(extensions) = owner.get("extensions") {
            if extensions.as_object().is_none_or(|e| {
                e.iter().any(|(name, body)| {
                    !TILESET_ALLOWLIST.contains(&name.as_str()) || body["clip"] == Value::Bool(true)
                })
            }) {
                self.c2.exclude(Reason::TilesetExtension);
            }
        }
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
            let json_limits = resources.content_limits.json;
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
                        resources.archive,
                        &source,
                        &mut resources.bytes_read,
                        &resources.limits,
                        maximum,
                    )
                    .map_err(crate::Error::from)?;
                    if source.ends_with(".json") {
                        let doc = json::parse(&bytes, json_limits)
                            .map_err(format_error)
                            .map_err(crate::Error::from)?;
                        schema.validate(&doc).map_err(|e| {
                            crate::Error::from(invalid(format!("tileset schema: {e}")))
                        })?;
                    }
                    Ok(bytes)
                },
                &mut self.expansion_budget,
                &|bytes| {
                    json::parse(bytes, json_limits)
                        .map_err(format_error)
                        .map_err(crate::Error::from)
                },
            )
            .map_err(implicit_error)?;
        }
        Ok(doc)
    }
    fn node(&mut self, task: NodeTask, pending: &mut Vec<ValidationTask>) -> Result<(), Error> {
        let NodeTask {
            mut node,
            base,
            parent,
            parent_error,
            depth,
            link,
        } = task;
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
        if node.get("metadata").is_some() {
            self.c2.exclude(Reason::TileMetadata);
        }
        self.extension_object(&node);
        self.extension_object(&node["boundingVolume"]);
        let tile = self.c2.tile(TileFact {
            transform: node.get("transform").map(|_| std::array::from_fn(|i| m[i])),
            volume: volume_facts(&bounds),
            external: link.external,
            parent: link.parent,
            depth,
        })?;
        if let Some(parent) = &parent {
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
        let mut external_seen = false;
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
            let name = uri(&base, value)?;
            self.resources.reference(name.clone())?;
            self.contents += 1;
            if content.get("metadata").is_some() {
                self.c2.exclude(Reason::TileMetadata);
            }
            self.extension_object(&content);
            let content_volume = match content.get("boundingVolume") {
                Some(v) => {
                    self.extension_object(v);
                    let cb = volume(v)?;
                    if !contains(&bounds, &cb, &IDENTITY)? {
                        return Err(invalid("content bounds escape tile"));
                    }
                    Some(cb)
                }
                None => None,
            };
            bytes = bytes
                .checked_add(
                    self.resources
                        .archive
                        .member(&name)
                        .map_err(archive_admission_error)?
                        .size,
                )
                .ok_or_else(|| invalid("tile byte count overflow"))?;
            if name.ends_with(".json") {
                external_seen = true;
                descendants.push(ValidationTask::Tileset {
                    name,
                    parent: Some(bounds.clone()),
                    parent_error: Some(error),
                    depth: depth + 1,
                    link: Link {
                        parent: Some(tile),
                        external: content_volume.as_ref().map(volume_facts),
                    },
                });
                continue;
            }
            let inspected = self.payload(&name)?;
            let uri = self.c2.uri_index[&name];
            let content = content_volume.as_ref().map(volume_facts);
            // C1 inspects inline; the C2 reference keeps contents in array order
            // with each external tileset expanded in place.
            if external_seen {
                descendants.push(ValidationTask::Reference { tile, content, uri });
            } else {
                self.c2.reference(tile, content, uri)?;
            }
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
                descendants.push(ValidationTask::Node(NodeTask {
                    node: child,
                    base: base.clone(),
                    parent: Some(bounds.clone()),
                    parent_error: error,
                    depth: depth + 1,
                    link: Link {
                        parent: Some(tile),
                        external: None,
                    },
                }));
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
    let mut options = std::fs::OpenOptions::new();
    options.read(true);
    // Inspect the selected handle's type without waiting for a FIFO writer.
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NONBLOCK);
    }
    let mut file = options.open(path).map_err(|error| {
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
    inspect_selected(request, ValidationLimits::default())
}

// C1 selects its profile above. Explicit limits also permit bounded internal
// controls to exercise the actual operation and every resource/parser consumer.
pub(crate) fn inspect_selected(
    request: ValidationRequest,
    limits: ValidationLimits,
) -> Result<ValidationReport, Error> {
    inspect_facts(request, limits).map(|(report, _)| report)
}

/// Per-URI C2 work facts of one successful inspection (v3.3 item 7). Test-only:
/// acceptance reconciles them with the oracle's model; no public report change.
#[cfg(test)]
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct UriWork {
    pub uri: String,
    pub t: u64,
    pub nodes: Option<u64>,
    pub idx: u64,
    pub e: u64,
    pub dec: u64,
    pub b: u64,
    pub max_node_depth: u64,
}
#[cfg(test)]
pub(crate) fn containment_work_facts(
    request: ValidationRequest,
    limits: ValidationLimits,
) -> Result<Vec<UriWork>, Error> {
    let (_, uris) = inspect_facts(request, limits)?;
    Ok(uris
        .into_iter()
        .map(|(uri, f)| UriWork {
            uri,
            t: f.t,
            nodes: f.nodes,
            idx: f.idx,
            e: f.e,
            dec: f.dec,
            b: f.b,
            max_node_depth: f.max_depth,
        })
        .collect())
}

fn inspect_facts(
    request: ValidationRequest,
    limits: ValidationLimits,
) -> Result<(ValidationReport, Vec<(String, UriFacts)>), Error> {
    let path = request.input();
    let source = open_archive(path)?;
    let before = source.metadata()?;
    let identity = same_file::Handle::from_file(source.try_clone()?)?;
    let content_limits = content_limits(&limits)?;
    let archive_limits = crate::archive3tz::ReadLimits {
        source_archive_bytes: limits.source_archive_bytes,
        central_directory_bytes: limits.central_directory_bytes,
        archive_entries: limits.archive_entries,
        member_bytes: limits.member_bytes,
        archive_stored_bytes: limits.archive_stored_bytes,
    };
    let mut archive = Archive::new(source.try_clone()?, before.len(), &archive_limits)
        .map_err(archive_admission_error)?;
    // Count the exact index bytes already consumed by the format owner. Later
    // index/member hashing and repeated content reads consume this same budget.
    let mut bytes_read = archive.index_read_bytes();
    let mut names = HashSet::new();
    let mut hashes = HashMap::new();
    for i in 0..archive.len() {
        let member = archive.member_at(i).map_err(archive_admission_error)?;
        let name = member.name.clone();
        let size = member.size;
        if name.split('/').any(|v| matches!(v, "" | "." | ".."))
            || name.starts_with('/')
            || name.contains('\\')
        {
            return Err(invalid(format!("unsafe archive path: {name}")));
        }
        names.insert(name.clone());
        bytes_read = bytes_read
            .checked_add(size)
            .filter(|&n| n <= limits.total_bytes_read)
            .ok_or_else(|| Error::limit("archive hash reads exceed work limit"))?;
        let digest = archive.hash_member(i).map_err(archive_admission_error)?;
        let hash = format!("{:x}", sha2::digest::Output::<sha2::Sha256>::from(digest));
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
        json::parse(
            &read_member(
                &mut archive,
                "conversion.json",
                &mut bytes_read,
                &limits,
                limits.json_bytes,
            )?,
            content_limits.json,
        )
        .map_err(format_error)?
    } else {
        Value::Null
    };
    let c2 = Containment::new(&limits);
    let mut check = Check {
        expansion_budget: crate::implicit::ExpansionBudget::new(
            usize::try_from(limits.hierarchy_visits)
                .map_err(|_| Error::limit("hierarchy limit exceeds host range"))?,
            content_limits.decoded_bytes,
        ),
        resources: ArchiveResources {
            archive: &mut archive,
            names,
            used: HashSet::from([TZ_INDEX_NAME.into()]),
            limits,
            content_limits,
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
        c2,
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
    // C2 outcome precedence after C1 success: exclusions, then the precheck,
    // then streaming (Outside or allocation refusal), otherwise Contained.
    let reasons = if check.c2.exclusions != 0 {
        check.c2.exclusions
    } else {
        check.c2.precheck.map_or(0, Reason::bit)
    };
    let contained = reasons == 0;
    if contained {
        check.stream()?;
        check.not_inspected.remove("decodedContentBounds");
        check.not_inspected.remove("scene transforms");
    } else {
        for reason in REASONS.iter().filter(|r| reasons & r.bit() != 0) {
            check
                .not_inspected
                .insert(format!("decodedContentBounds.{}", reason.name()));
        }
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
    let uris = std::mem::take(&mut check.c2.uris);
    let report = ValidationReport {
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
            "archiveStoredRecordLayout",
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
        .chain(contained.then_some("decodedContentContainment"))
        .map(String::from)
        .collect(),
        not_inspected: check.not_inspected.into_iter().collect(),
    };
    Ok((report, uris))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn content_bridges_preserve_causal_io_and_typed_format_categories() {
        #[derive(Debug)]
        struct Cause;
        impl std::fmt::Display for Cause {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.write_str("resource limit: duplicate JSON key; actual resolver cause")
            }
        }
        impl std::error::Error for Cause {}
        for kind in [
            std::io::ErrorKind::InvalidData,
            std::io::ErrorKind::UnexpectedEof,
        ] {
            for implicit in [false, true] {
                let io = std::io::Error::new(kind, Cause);
                let failure = if implicit {
                    implicit_error(crate::Error::Io(io))
                } else {
                    payload_error(PayloadError::Resolver(Error::Io(io)))
                };
                let Error::Io(io) = failure else {
                    panic!("causal I/O must stay I/O")
                };
                assert_eq!(io.kind(), kind);
                assert!(io.get_ref().unwrap().downcast_ref::<Cause>().is_some());
                assert_eq!(
                    io.to_string(),
                    "resource limit: duplicate JSON key; actual resolver cause"
                );
            }
        }
        for (format, category) in [
            (
                FormatError::InvalidInput("resource limit: lookalike".into()),
                "invalid_input",
            ),
            (
                FormatError::ResourceLimit("duplicate key lookalike".into()),
                "resource_limit",
            ),
            (
                FormatError::Unsupported("invalid syntax lookalike".into()),
                "unsupported",
            ),
        ] {
            assert_eq!(
                payload_error(PayloadError::Format(format)).category().0,
                category
            );
        }
    }
    #[derive(Debug)]
    struct InjectedReadFailure;
    impl std::fmt::Display for InjectedReadFailure {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.write_str("injected reader failure")
        }
    }
    impl std::error::Error for InjectedReadFailure {}

    fn map_reader_io(error: std::io::Error) -> ValidationFailure {
        archive_admission_error(crate::archive3tz::ReadError::Io(error))
    }
    #[test]
    fn archive_reader_mapping_preserves_real_os_io_causes() {
        let work = tempfile::tempdir().unwrap();
        {
            let error = File::open(work.path().join("missing.3tz")).unwrap_err();
            let kind = error.kind();
            let os_code = error.raw_os_error();
            let message = error.to_string();
            assert_eq!(kind, std::io::ErrorKind::NotFound);
            assert!(os_code.is_some());
            let failure = map_reader_io(error);
            assert_eq!(failure.category(), ("io", 1));
            let Error::Io(error) = failure else {
                panic!("lost actual I/O origin");
            };
            assert_eq!(error.kind(), kind);
            assert_eq!(error.raw_os_error(), os_code);
            assert_eq!(error.to_string(), message);
        }
    }
    #[test]
    fn archive_reader_mapping_preserves_injected_invalid_data_and_eof_io() {
        for kind in [
            std::io::ErrorKind::InvalidData,
            std::io::ErrorKind::UnexpectedEof,
        ] {
            {
                let failure = map_reader_io(std::io::Error::new(kind, InjectedReadFailure));
                assert_eq!(failure.category(), ("io", 1));
                let Error::Io(error) = failure else {
                    panic!("reclassified actual {kind:?} as malformed input");
                };
                assert_eq!(error.kind(), kind);
                assert!(error
                    .get_ref()
                    .is_some_and(|cause| cause.is::<InjectedReadFailure>()));
            }
        }
    }
    #[test]
    fn archive_admission_mapping_keeps_typed_categories_without_message_matching() {
        use crate::archive3tz::ReadError;
        for (error, category) in [
            (
                ReadError::InvalidInput("I/O-looking malformed input".into()),
                ("invalid_input", 3),
            ),
            (
                ReadError::Unsupported("CRC-looking unsupported profile".into()),
                ("unsupported", 2),
            ),
            (
                ReadError::ResourceLimit("package missing tileset.json".into()),
                ("resource_limit", 3),
            ),
        ] {
            assert_eq!(archive_admission_error(error).category(), category);
        }
    }
    #[test]
    fn archive_reader_mapping_distinguishes_missing_member_and_detected_crc() {
        use crate::archive3tz::ReadError;
        let missing = archive_admission_error(ReadError::MissingMember("tileset.json".into()));
        assert_eq!(missing.category(), ("invalid_input", 3));
        assert_eq!(missing.to_string(), "missing archive member: tileset.json");
        let crc = archive_admission_error(ReadError::InvalidInput(
            "detected index CRC mismatch".into(),
        ));
        assert_eq!(crc.category(), ("invalid_input", 3));
        assert_eq!(crc.to_string(), "detected index CRC mismatch");
    }
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
        crate::package::package(
            crate::package::PackageRequest::directory(&data, &output),
            &crate::RunControl::default(),
        )
        .unwrap();
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
#[test]
fn archive_uri_decodes_segments_once_and_confines_paths() {
    assert_eq!(
        uri("model/source.gltf", "data/a%20b.bin").unwrap(),
        "model/data/a b.bin"
    );
    assert_eq!(
        uri("model/source.gltf", "a%2520b.bin").unwrap(),
        "model/a%20b.bin"
    );
    assert_eq!(
        uri("model/source.gltf", "%2e%2e/data.bin").unwrap(),
        "data.bin"
    );
    for value in [
        "%2e%2e/%2e%2e/data.bin",
        "%2fdata.bin",
        "a%5cb.bin",
        "a%00.bin",
        "%ff.bin",
        "bad%",
        "bad%2g",
    ] {
        assert!(uri("model/source.gltf", value).is_err(), "{value}");
    }
}

/// C2 operation controls through the real `inspect_selected` entry point.
#[cfg(test)]
mod containment_controls {
    use super::*;
    use serde_json::json;
    use std::path::PathBuf;

    fn glb(doc: &Value, bin: &[u8]) -> Vec<u8> {
        let mut json = serde_json::to_vec(doc).unwrap();
        while !json.len().is_multiple_of(4) {
            json.push(b' ');
        }
        let mut bin = bin.to_vec();
        while !bin.len().is_multiple_of(4) {
            bin.push(0);
        }
        let mut out = b"glTF".to_vec();
        out.extend(2u32.to_le_bytes());
        let chunks = if bin.is_empty() { 0 } else { 8 + bin.len() };
        out.extend((20 + json.len() as u32 + chunks as u32).to_le_bytes());
        out.extend((json.len() as u32).to_le_bytes());
        out.extend(b"JSON");
        out.extend(json);
        if !bin.is_empty() {
            out.extend((bin.len() as u32).to_le_bytes());
            out.extend(b"BIN\0");
            out.extend(bin);
        }
        out
    }
    /// POINTS over FLOAT positions, one selected scene, the given node list.
    fn points_doc(positions: &[[f32; 3]], nodes: Value) -> (Value, Vec<u8>) {
        let mut bin = Vec::new();
        for p in positions {
            for c in p {
                bin.extend(c.to_le_bytes());
            }
        }
        let bound = |f: fn(f32, f32) -> f32, start: f32| -> Vec<f64> {
            (0..3)
                .map(|c| f64::from(positions.iter().map(|p| p[c]).fold(start, f)))
                .collect()
        };
        let doc = json!({
            "asset":{"version":"2.0"},"scene":0,"scenes":[{"nodes":[0]}],"nodes":nodes,
            "buffers":[{"byteLength":bin.len()}],"bufferViews":[{"buffer":0,"byteLength":bin.len()}],
            "accessors":[{"bufferView":0,"componentType":5126,"type":"VEC3","count":positions.len(),
                "min":bound(f32::min, f32::INFINITY),"max":bound(f32::max, f32::NEG_INFINITY)}],
            "meshes":[{"primitives":[{"attributes":{"POSITION":0},"mode":0}]}]
        });
        (doc, bin)
    }
    fn points(positions: &[[f32; 3]]) -> Vec<u8> {
        let (doc, bin) = points_doc(positions, json!([{"mesh":0}]));
        glb(&doc, &bin)
    }
    fn boxed(center: [f64; 3], half: [f64; 3]) -> Value {
        json!({"box":[center[0],center[1],center[2],half[0],0,0,0,half[1],0,0,0,half[2]]})
    }
    fn tile(volume: Value) -> Value {
        json!({"boundingVolume":volume,"geometricError":0})
    }
    fn content_tile(volume: Value, uri: &str) -> Value {
        let mut tile = tile(volume);
        tile["content"] = json!({"uri":uri});
        tile
    }
    fn tileset(root: Value) -> Vec<u8> {
        serde_json::to_vec(&json!({"asset":{"version":"1.1"},"geometricError":0,"root":root}))
            .unwrap()
    }
    fn package(files: &[(&str, Vec<u8>)]) -> (tempfile::TempDir, PathBuf) {
        let work = tempfile::tempdir().unwrap();
        let data = work.path().join("data");
        for (name, bytes) in files {
            let file = data.join(name);
            std::fs::create_dir_all(file.parent().unwrap()).unwrap();
            std::fs::write(file, bytes).unwrap();
        }
        let output = work.path().join("test.3tz");
        crate::package::package(
            crate::package::PackageRequest::directory(&data, &output),
            &crate::RunControl::default(),
        )
        .unwrap();
        (work, output)
    }
    fn run_with(
        files: &[(&str, Vec<u8>)],
        limits: ValidationLimits,
    ) -> Result<ValidationReport, Error> {
        let (_work, path) = package(files);
        inspect_selected(ValidationRequest::new(&path), limits)
    }
    fn run(files: &[(&str, Vec<u8>)]) -> Result<ValidationReport, Error> {
        run_with(files, ValidationLimits::default())
    }
    fn single(volume: Value, payload: Vec<u8>) -> Vec<(&'static str, Vec<u8>)> {
        vec![
            ("tileset.json", tileset(content_tile(volume, "a.glb"))),
            ("a.glb", payload),
        ]
    }
    fn contained(report: &ValidationReport) -> bool {
        let listed = report
            .checks
            .iter()
            .any(|c| c == "decodedContentContainment");
        let bounds = report
            .not_inspected
            .iter()
            .any(|c| c == "decodedContentBounds");
        assert_eq!(listed, !bounds, "{report:?}");
        if !report.payloads.is_empty() {
            assert_eq!(
                listed,
                !report.not_inspected.iter().any(|c| c == "scene transforms"),
                "{report:?}"
            );
        }
        listed
    }
    /// The `decodedContentBounds.<reason>` entries, in report order.
    fn reasons(report: &ValidationReport) -> Vec<&str> {
        report
            .not_inspected
            .iter()
            .filter_map(|c| c.strip_prefix("decodedContentBounds."))
            .collect()
    }
    fn not_inspected(result: Result<ValidationReport, Error>) -> Vec<String> {
        let report = result.unwrap();
        assert!(!contained(&report), "{report:?}");
        reasons(&report).into_iter().map(String::from).collect()
    }
    fn outside(result: Result<ValidationReport, Error>) -> String {
        match result {
            Err(Error::InvalidInput(message)) => message,
            other => panic!("expected Outside, got {other:?}"),
        }
    }

    #[test]
    fn invalid_state_category_is_invalid_state_exit_one() {
        let failure = containment_fault();
        assert!(matches!(failure, Error::InvalidState(_)));
        assert_eq!(failure.category(), ("invalid_state", 1));
        assert_eq!(
            crate::Error::Validation(failure).category(),
            ("invalid_state", 1)
        );
    }

    #[test]
    fn boundary_vertex_is_contained_and_one_ulp_inside_box_is_outside() {
        // glTF (1, 2, 3) is (1, -3, 2) in the z-up tile frame after U.
        let payload = points(&[[1., 2., 3.]]);
        let report = run(&single(boxed([0.; 3], [1., 3., 2.]), payload.clone())).unwrap();
        assert!(contained(&report), "{report:?}");
        assert!(report
            .not_inspected
            .iter()
            .any(|c| c == "skins, animations, morph targets and rendered extent"));
        // Non-integer centre, vertex exactly on the face.
        let report = run(&single(
            boxed([0.5, -1.5, 1.], [0.5, 1.5, 1.]),
            payload.clone(),
        ));
        assert!(contained(&report.unwrap()));
        for half in [
            [1f64.next_down(), 3., 2.],
            [1., 3f64.next_down(), 2.],
            [1., 3., 2f64.next_down()],
        ] {
            assert_eq!(
                outside(run(&single(boxed([0.; 3], half), payload.clone()))),
                "decoded content outside bounding volume: content=a.glb depth=0 volume=own node=0 mesh=0 primitive=0 vertex=0"
            );
        }
        // Omitting U would accept a box that only holds the y-up point.
        assert!(outside(run(&single(boxed([0.; 3], [1., 2., 3.]), payload))).contains("volume=own"));
    }

    #[test]
    fn sphere_membership_is_exact() {
        let payload = points(&[[3., 0., 4.]]);
        let sphere = |r: f64| json!({"sphere":[0,0,0,r]});
        assert!(contained(
            &run(&single(sphere(5.), payload.clone())).unwrap()
        ));
        assert!(outside(run(&single(sphere(5f64.next_down()), payload))).contains("volume=own"));
    }

    #[test]
    fn undrawn_vertices_are_not_tested_and_drawn_ones_are() {
        let (mut doc, mut bin) = points_doc(
            &[[1., 1., 1.], [1000., -1000., 1000.], [0., 0., 0.]],
            json!([{"mesh":0}]),
        );
        bin.extend([0u8, 2, 0, 0]);
        doc["buffers"][0]["byteLength"] = json!(40);
        doc["bufferViews"]
            .as_array_mut()
            .unwrap()
            .push(json!({"buffer":0,"byteOffset":36,"byteLength":3}));
        doc["accessors"]
            .as_array_mut()
            .unwrap()
            .push(json!({"bufferView":1,"componentType":5121,"type":"SCALAR","count":3}));
        doc["meshes"][0]["primitives"][0]["indices"] = json!(1);
        let volume = boxed([0.; 3], [2., 2., 2.]);
        assert!(contained(
            &run(&single(volume.clone(), glb(&doc, &bin))).unwrap()
        ));
        bin[37] = 1;
        assert_eq!(
            outside(run(&single(volume, glb(&doc, &bin)))),
            "decoded content outside bounding volume: content=a.glb depth=0 volume=own node=0 mesh=0 primitive=0 vertex=1"
        );
    }

    #[test]
    fn content_ancestor_and_external_content_roles_are_reported() {
        let payload = points(&[[5., 0., 0.]]);
        // Content volume.
        let mut root = content_tile(boxed([0.; 3], [10.; 3]), "a.glb");
        root["content"]["boundingVolume"] = boxed([0.; 3], [4.; 3]);
        let message = outside(run(&[
            ("tileset.json", tileset(root)),
            ("a.glb", payload.clone()),
        ]));
        assert!(message.contains("depth=0 volume=content "), "{message}");
        // Ancestor (I1): the child volume passes C1's tolerant hierarchy check.
        let edge = f32::from_bits(10f32.to_bits() + 1);
        let mut root = tile(boxed([0.; 3], [10.; 3]));
        root["children"] = json!([content_tile(boxed([0.; 3], [10.000002, 10., 10.]), "a.glb")]);
        let message = outside(run(&[
            ("tileset.json", tileset(root)),
            ("a.glb", points(&[[edge, 0., 0.]])),
        ]));
        assert!(message.contains("depth=1 volume=ancestor:1 "), "{message}");
        // External-reference content volume (I2), tested in the referencing frame.
        let mut root = tile(boxed([0.; 3], [100.; 3]));
        root["content"] = json!({"uri":"ext.json","boundingVolume":boxed([0.; 3], [1.; 3])});
        let message = outside(run(&[
            ("tileset.json", tileset(root)),
            (
                "ext.json",
                tileset(content_tile(boxed([0.; 3], [50.; 3]), "a.glb")),
            ),
            ("a.glb", payload),
        ]));
        assert!(
            message.contains("depth=1 volume=externalContent:1 "),
            "{message}"
        );
    }

    #[test]
    fn external_root_transform_is_a_chain_step() {
        let mut root = tile(boxed([3., 0., 0.], [1.; 3]));
        root["content"] = json!({"uri":"ext.json"});
        let mut external = content_tile(boxed([0.; 3], [1.; 3]), "a.glb");
        let mut m = IDENTITY;
        m[12] = 3.;
        external["transform"] = json!(m);
        let files = |x: f32| {
            vec![
                ("tileset.json", tileset(root.clone())),
                ("ext.json", tileset(external.clone())),
                ("a.glb", points(&[[x, 0., 0.]])),
            ]
        };
        // Skipping the external root transform would put -0.9 outside [2, 4].
        assert!(contained(&run(&files(-0.9)).unwrap()));
        assert!(contained(&run(&files(0.9)).unwrap()));
    }

    #[test]
    fn same_uri_is_tested_per_reference() {
        let mut first = content_tile(boxed([0.; 3], [1.; 3]), "a.glb");
        let mut m = IDENTITY;
        m[12] = 5.;
        first["transform"] = json!(m);
        let second = content_tile(boxed([0.; 3], [0.4, 1., 1.]), "a.glb");
        let mut root = tile(boxed([0.; 3], [10.; 3]));
        root["children"] = json!([first.clone(), second]);
        let message = outside(run(&[
            ("tileset.json", tileset(root.clone())),
            ("a.glb", points(&[[0.5, 0., 0.]])),
        ]));
        assert!(message.contains("depth=1 volume=own "), "{message}");
        let mut again = first.clone();
        again["refine"] = json!("ADD");
        root["children"] = json!([first, again]);
        assert!(contained(
            &run(&[
                ("tileset.json", tileset(root)),
                ("a.glb", points(&[[0.5, 0., 0.]])),
            ])
            .unwrap()
        ));
    }

    fn with_doc(patch: impl Fn(&mut Value)) -> Vec<u8> {
        let (mut doc, bin) = points_doc(&[[0., 0., 0.]], json!([{"mesh":0}]));
        patch(&mut doc);
        glb(&doc, &bin)
    }
    fn wide() -> Value {
        boxed([0.; 3], [10.; 3])
    }

    #[test]
    fn each_exclusion_reason_is_reported_without_streaming() {
        let payload = points(&[[0., 0., 0.]]);
        type Files = Vec<(&'static str, Vec<u8>)>;
        let mut cases: Vec<(&str, Files)> = Vec::new();
        let mut doc = json!({"asset":{"version":"1.1"},"geometricError":0,
            "extensionsUsed":["3DTILES_bounding_volume_S2"],"root":content_tile(wide(), "a.glb")});
        cases.push((
            "tilesetExtension",
            vec![
                ("tileset.json", serde_json::to_vec(&doc).unwrap()),
                ("a.glb", payload.clone()),
            ],
        ));
        doc = json!({"asset":{"version":"1.1"},"geometricError":0,"root":content_tile(wide(), "a.glb")});
        doc["root"]["extensions"] = json!({"ACME_tile":{}});
        cases.push((
            "tilesetExtension",
            vec![
                ("tileset.json", serde_json::to_vec(&doc).unwrap()),
                ("a.glb", payload.clone()),
            ],
        ));
        let mut root = content_tile(wide(), "a.glb");
        root["metadata"] = json!({"class":"c"});
        cases.push((
            "tileMetadata",
            vec![("tileset.json", tileset(root)), ("a.glb", payload.clone())],
        ));
        let mut root = content_tile(wide(), "a.glb");
        root["content"]["metadata"] = json!({"class":"c"});
        cases.push((
            "tileMetadata",
            vec![("tileset.json", tileset(root)), ("a.glb", payload.clone())],
        ));
        doc = json!({"asset":{"version":"1.1","gltfUpAxis":"Z"},"geometricError":0,"root":content_tile(wide(), "a.glb")});
        cases.push((
            "upAxisOverride",
            vec![
                ("tileset.json", serde_json::to_vec(&doc).unwrap()),
                ("a.glb", payload.clone()),
            ],
        ));
        cases.push((
            "regionVolume",
            single(json!({"region":[-0.1,-0.1,0.1,0.1,0,10]}), payload.clone()),
        ));
        cases.push((
            "rotatedBox",
            single(json!({"box":[0,0,0,1,1,0,-1,1,0,0,0,1]}), payload.clone()),
        ));
        cases.push((
            "gltfExtension",
            single(
                wide(),
                with_doc(|d| {
                    d["extensionsUsed"] = json!(["CESIUM_RTC"]);
                    d["extensions"] = json!({"CESIUM_RTC":{"center":[1,2,3]}});
                }),
            ),
        ));
        cases.push((
            "sceneSelection",
            single(
                wide(),
                with_doc(|d| {
                    d.as_object_mut().unwrap().remove("scene");
                    d["scenes"] = json!([{"nodes":[0]}, {"nodes":[0]}]);
                }),
            ),
        ));
        cases.push((
            "skin",
            single(wide(), with_doc(|d| d["nodes"][0]["skin"] = json!(0))),
        ));
        cases.push((
            "morphTargets",
            single(wide(), with_doc(|d| d["nodes"][0]["weights"] = json!([0]))),
        ));
        cases.push((
            "animation",
            single(
                wide(),
                with_doc(|d| d["animations"] = json!([{"channels":[],"samplers":[]}])),
            ),
        ));
        cases.push((
            "quaternionWindow",
            single(
                wide(),
                with_doc(|d| d["nodes"][0]["rotation"] = json!([0, 0, 0, 0.999])),
            ),
        ));
        for (reason, files) in cases {
            assert_eq!(not_inspected(run(&files)), [reason], "{reason}");
        }
    }

    fn b3dm(glb: &[u8], table: &[u8], body: usize) -> Vec<u8> {
        let mut table = table.to_vec();
        while !(28 + table.len()).is_multiple_of(8) {
            table.push(b' ');
        }
        let mut out = b"b3dm".to_vec();
        out.extend(1u32.to_le_bytes());
        out.extend(0u32.to_le_bytes());
        out.extend((table.len() as u32).to_le_bytes());
        out.extend((body as u32).to_le_bytes());
        out.extend([0u8; 8]);
        out.extend(table);
        out.extend(vec![0u8; body]);
        out.extend(glb);
        while !out.len().is_multiple_of(8) {
            out.push(0);
        }
        let length = out.len() as u32;
        out[8..12].copy_from_slice(&length.to_le_bytes());
        out
    }

    #[test]
    fn rtc_center_in_json_or_binary_form_excludes_and_plain_b3dm_streams() {
        let payload = points(&[[0., 0., 0.]]);
        let files = |b3dm: Vec<u8>| {
            vec![
                ("tileset.json", tileset(content_tile(wide(), "a.b3dm"))),
                ("a.b3dm", b3dm),
            ]
        };
        let plain = b3dm(&payload, br#"{"BATCH_LENGTH":0}"#, 0);
        assert!(contained(&run(&files(plain)).unwrap()));
        for (table, body) in [
            (&br#"{"BATCH_LENGTH":0,"RTC_CENTER":[1,2,3]}"#[..], 0),
            (
                &br#"{"BATCH_LENGTH":0,"RTC_CENTER":{"byteOffset":0}}"#[..],
                16,
            ),
        ] {
            assert_eq!(
                not_inspected(run(&files(b3dm(&payload, table, body)))),
                ["rtcCenter"]
            );
        }
    }

    #[test]
    fn implicit_archives_are_not_inspected() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/c1/implicit_no_content.3tz");
        let reasons = not_inspected(inspect(ValidationRequest::new(path)));
        assert!(
            reasons.contains(&"implicitTiling".to_owned()),
            "{reasons:?}"
        );
    }

    #[test]
    fn allowlisted_and_default_values_do_not_exclude() {
        let doc = json!({"asset":{"version":"1.1","gltfUpAxis":"Y"},"geometricError":0,
            "extensionsUsed":["3DTILES_content_gltf_vector"],"root":content_tile(wide(), "a.glb")});
        let payload = with_doc(|d| {
            d["extensionsUsed"] = json!(["KHR_materials_unlit", "EXT_mesh_features"]);
            d["nodes"][0]["rotation"] = json!([0, 0, 0, 1]);
            d["animations"] = json!([]);
        });
        assert!(contained(
            &run(&[
                ("tileset.json", serde_json::to_vec(&doc).unwrap()),
                ("a.glb", payload),
            ])
            .unwrap()
        ));
        // |q|^2 - 1 exactly 2^-18 is inside the window; one ulp more is not.
        let edge = 2f64.powi(-9);
        let q = |x: f64| {
            single(
                wide(),
                with_doc(|d| d["nodes"][0]["rotation"] = json!([x, 0, 0, 1])),
            )
        };
        assert!(contained(&run(&q(edge)).unwrap()));
        assert_eq!(not_inspected(run(&q(edge.next_up()))), ["quaternionWindow"]);
    }

    #[test]
    fn all_reasons_are_listed_sorted_and_deduplicated() {
        let skinned = with_doc(|d| {
            d["nodes"][0]["skin"] = json!(0);
            d["animations"] = json!([{"channels":[],"samplers":[]}]);
        });
        let mut root = tile(wide());
        root["children"] = json!([content_tile(wide(), "a.glb"), content_tile(wide(), "b.glb")]);
        let doc =
            json!({"asset":{"version":"1.1","gltfUpAxis":"X"},"geometricError":0,"root":root});
        assert_eq!(
            not_inspected(run(&[
                ("tileset.json", serde_json::to_vec(&doc).unwrap()),
                ("a.glb", skinned.clone()),
                ("b.glb", skinned),
            ])),
            ["animation", "skin", "upAxisOverride"]
        );
    }

    #[test]
    fn outcome_precedence() {
        // Malformed node beats an exclusion (here rtcCenter).
        let malformed = with_doc(|d| {
            d["nodes"][0]["matrix"] = json!([1, 0, 0, 0, 0, 1, 0, 0, 0, 0, 1, 0, 0, 0, 0, 1]);
            d["nodes"][0]["scale"] = json!([1, 1, 1]);
        });
        let failure = run(&[
            ("tileset.json", tileset(content_tile(wide(), "a.b3dm"))),
            (
                "a.b3dm",
                b3dm(&malformed, br#"{"BATCH_LENGTH":0,"RTC_CENTER":[0,0,0]}"#, 0),
            ),
        ])
        .unwrap_err();
        assert!(
            matches!(&failure, Error::InvalidInput(m) if m.contains("matrix must not coexist")),
            "{failure:?}"
        );
        // An exclusion beats the precheck and a would-be Outside.
        let far = points(&[[100., 0., 0.]]);
        let (mut doc, bin) = points_doc(&[[100., 0., 0.]], json!([{"mesh":0}]));
        doc["animations"] = json!([{"channels":[],"samplers":[]}]);
        let limits = ValidationLimits {
            containment_span_bits: 1,
            ..ValidationLimits::default()
        };
        assert_eq!(
            not_inspected(run_with(&single(wide(), glb(&doc, &bin)), limits.clone())),
            ["animation"]
        );
        // The precheck beats a would-be Outside.
        assert_eq!(
            not_inspected(run_with(&single(wide(), far.clone()), limits)),
            ["numericSpan"]
        );
        assert!(outside(run(&single(wide(), far.clone()))).contains("volume=own"));
        // A C1 failure beats every C2 outcome.
        let failure = run(&[
            ("tileset.json", tileset(content_tile(wide(), "a.glb"))),
            ("a.glb", far),
            ("stray.bin", vec![0; 4]),
        ])
        .unwrap_err();
        assert!(failure.to_string().contains("unreferenced archive entries"));
    }

    #[test]
    fn precheck_records_only_the_first_violation_in_traversal_order() {
        // `deep` violates the node-depth cap; `float` violates the span cap
        // (FLOAT position pair spans 277 bits) while `small` u8 words do not.
        let deep = {
            let (mut doc, bin) = points_doc(&[[0., 0., 0.]], json!([{"children":[1]},{"mesh":0}]));
            doc["extensionsUsed"] = json!(["KHR_mesh_quantization"]);
            doc["extensionsRequired"] = json!(["KHR_mesh_quantization"]);
            doc["accessors"][0]["componentType"] = json!(5121);
            doc["accessors"][0]["min"] = json!([0, 0, 0]);
            doc["accessors"][0]["max"] = json!([0, 0, 0]);
            doc["buffers"][0]["byteLength"] = json!(4);
            doc["bufferViews"][0] = json!({"buffer":0,"byteLength":4,"byteStride":4});
            drop(bin);
            glb(&doc, &[0; 4])
        };
        let float = points(&[[0., 0., 0.]]);
        let limits = ValidationLimits {
            containment_span_bits: 100,
            containment_node_depth: 1,
            ..ValidationLimits::default()
        };
        let files = |first: &str, second: &str| {
            let mut root = tile(wide());
            root["children"] = json!([content_tile(wide(), first), content_tile(wide(), second)]);
            vec![
                ("tileset.json", tileset(root)),
                ("deep.glb", deep.clone()),
                ("float.glb", float.clone()),
            ]
        };
        assert_eq!(
            not_inspected(run_with(&files("deep.glb", "float.glb"), limits.clone())),
            ["workLimit"]
        );
        assert_eq!(
            not_inspected(run_with(&files("float.glb", "deep.glb"), limits.clone())),
            ["numericSpan"]
        );
        // Each alone, at caps that admit it, is contained.
        let relaxed = ValidationLimits {
            containment_node_depth: 2,
            ..limits
        };
        assert!(contained(
            &run_with(&single(wide(), deep.clone()), relaxed).unwrap()
        ));
    }

    #[test]
    fn identity_node_forms_share_spans_at_the_cap_and_one_over() {
        // FLOAT pair (-149, 127) spans 277 bits; centre and half-lengths add none.
        for node in [
            json!({"mesh":0}),
            json!({"mesh":0,"translation":[0,0,0],"rotation":[0,0,0,1],"scale":[1,1,1]}),
            json!({"mesh":0,"matrix":[1,0,0,0,0,1,0,0,0,0,1,0,0,0,0,1]}),
        ] {
            let (doc, bin) = points_doc(&[[0., 0., 0.]], json!([node.clone()]));
            let files = single(wide(), glb(&doc, &bin));
            let limits = |bits| ValidationLimits {
                containment_span_bits: bits,
                ..ValidationLimits::default()
            };
            assert!(contained(&run_with(&files, limits(277)).unwrap()), "{node}");
            assert_eq!(
                not_inspected(run_with(&files, limits(276))),
                ["numericSpan"],
                "{node}"
            );
        }
    }

    #[test]
    fn per_uri_hull_decides_numeric_span_for_multi_instance_uris() {
        // Each instance alone spans 278 bits (FLOAT pair scaled by 2^-100 or
        // 2^100); the per-URI Pair0 hull spans 478 (contract v3.1, delta2 H1).
        let (tiny, huge) = (2f64.powi(-100), 2f64.powi(100));
        let files = |children: Value| {
            let nodes = json!([
                {"children":children},
                {"mesh":0,"scale":[tiny,tiny,tiny]},
                {"mesh":0,"scale":[huge,huge,huge]}
            ]);
            let (doc, bin) = points_doc(&[[0., 0., 0.]], nodes);
            single(wide(), glb(&doc, &bin))
        };
        let limits = ValidationLimits {
            containment_span_bits: 300,
            ..ValidationLimits::default()
        };
        for alone in [json!([1]), json!([2])] {
            assert!(contained(&run_with(&files(alone), limits.clone()).unwrap()));
        }
        assert_eq!(
            not_inspected(run_with(&files(json!([1, 2])), limits)),
            ["numericSpan"]
        );
    }

    #[test]
    fn work_bytes_decoded_and_depth_caps_at_the_sum_and_one_below() {
        // One FLOAT vertex, a skipped {"mesh":0} node, one level of span 277:
        // units = T·⌈277/64⌉² + Nodes + Idx + E = 1·25 + 1 + 1 + 3 = 30.
        let (mut doc, bin) = points_doc(&[[0., 0., 0.]], json!([{"children":[1]},{"mesh":0}]));
        doc["buffers"][0]["uri"] = json!("a.bin");
        let gltf = serde_json::to_vec(&doc).unwrap();
        let files = vec![
            ("tileset.json", tileset(content_tile(wide(), "a.gltf"))),
            ("a.gltf", gltf.clone()),
            ("a.bin", bin.clone()),
        ];
        let read = (gltf.len() + bin.len()) as u64;
        let units = 25 + 2 + 1 + 3;
        for (field, at_cap) in [
            ("units", units),
            ("bytes", read),
            ("decoded", bin.len() as u64),
            ("depth", 2),
        ] {
            let limits = |cap: u64| {
                let mut limits = ValidationLimits::default();
                match field {
                    "units" => limits.containment_work_units = cap,
                    "bytes" => limits.containment_bytes_read = cap,
                    "decoded" => limits.containment_decoded_bytes = cap,
                    _ => limits.containment_node_depth = cap,
                }
                limits
            };
            assert!(
                contained(&run_with(&files, limits(at_cap)).unwrap()),
                "{field} at cap"
            );
            assert_eq!(
                not_inspected(run_with(&files, limits(at_cap - 1))),
                ["workLimit"],
                "{field} one below"
            );
        }
        let (_work, path) = package(&files);
        assert_eq!(
            containment_work_facts(ValidationRequest::new(&path), ValidationLimits::default())
                .unwrap(),
            [UriWork {
                uri: "a.gltf".into(),
                t: 1,
                nodes: Some(2),
                idx: 1,
                e: 3,
                dec: bin.len() as u64,
                b: read,
                max_node_depth: 2,
            }]
        );
    }

    #[test]
    fn worked_example_is_in_domain_at_29_and_numeric_span_at_28() {
        // Contract "Worked example": TRS node, u16 normalized POSITION, own box
        // c=(1.5,0,2) h=(100,0,50), child transform (1000.25,-20,0), parent h=2048.
        let doc = json!({
            "asset":{"version":"2.0"},"extensionsUsed":["KHR_mesh_quantization"],
            "extensionsRequired":["KHR_mesh_quantization"],"scene":0,"scenes":[{"nodes":[0]}],
            "nodes":[{"mesh":0,"translation":[10.5,0,-3],"scale":[2,4,0.25]}],
            "buffers":[{"byteLength":8}],"bufferViews":[{"buffer":0,"byteLength":8,"byteStride":8}],
            "accessors":[{"bufferView":0,"componentType":5123,"normalized":true,"type":"VEC3",
                "count":1,"min":[0,0,0],"max":[0,0,0]}],
            "meshes":[{"primitives":[{"attributes":{"POSITION":0},"mode":0}]}]
        });
        let mut child = content_tile(json!({"box":[1.5,0,2,100,0,0,0,0,0,0,0,50]}), "a.glb");
        let mut m = IDENTITY;
        m[12] = 1000.25;
        m[13] = -20.;
        child["transform"] = json!(m);
        let mut root = tile(boxed([0.; 3], [2048.; 3]));
        root["children"] = json!([child]);
        let files = vec![
            ("tileset.json", tileset(root)),
            ("a.glb", glb(&doc, &[0; 8])),
        ];
        let limits = |bits| ValidationLimits {
            containment_span_bits: bits,
            ..ValidationLimits::default()
        };
        // In domain at 29: streamed, and the vertex (10.5, 3, 0) misses the
        // zero-height own box row, so the verdict is a proven Outside.
        assert_eq!(
            outside(run_with(&files, limits(29))),
            "decoded content outside bounding volume: content=a.glb depth=1 volume=own node=0 mesh=0 primitive=0 vertex=0"
        );
        assert_eq!(not_inspected(run_with(&files, limits(28))), ["numericSpan"]);
        // v3.3 item 1: O's adjusted geometry (own-box row-1 centre 3) keeps the
        // spans [23, 29] and is Contained at the cap.
        let mut root: Value = serde_json::from_slice(&files[0].1).unwrap();
        root["root"]["children"][0]["boundingVolume"]["box"][1] = json!(3);
        let files = vec![
            ("tileset.json", serde_json::to_vec(&root).unwrap()),
            files[1].clone(),
        ];
        assert!(contained(&run_with(&files, limits(29)).unwrap()));
        assert_eq!(not_inspected(run_with(&files, limits(28))), ["numericSpan"]);
    }

    #[test]
    fn vector_clip_extension_object_is_not_allowlisted() {
        for (owner, clip, excluded) in [
            ("tile", true, true),
            ("content", true, true),
            ("tileset", true, true),
            ("tile", false, false),
        ] {
            let mut doc = json!({"asset":{"version":"1.1"},"geometricError":0,
                "extensionsUsed":["3DTILES_content_gltf_vector"],
                "root":content_tile(wide(), "a.glb")});
            let extension = json!({"3DTILES_content_gltf_vector":{"clip":clip}});
            match owner {
                "tile" => doc["root"]["extensions"] = extension,
                "content" => doc["root"]["content"]["extensions"] = extension,
                _ => doc["extensions"] = extension,
            }
            let result = run(&[
                ("tileset.json", serde_json::to_vec(&doc).unwrap()),
                ("a.glb", points(&[[0., 0., 0.]])),
            ]);
            if excluded {
                assert_eq!(not_inspected(result), ["tilesetExtension"], "{owner}");
            } else {
                assert!(contained(&result.unwrap()), "{owner}");
            }
        }
    }

    #[test]
    fn references_follow_preorder_with_external_tilesets_expanded_in_place() {
        // contents [ext.json -> b.glb, a.glb]: b.glb is the first reference.
        let mut root = tile(wide());
        root["contents"] = json!([{"uri":"ext.json"}, {"uri":"a.glb"}]);
        let far = points(&[[20., 0., 0.]]);
        let files = vec![
            ("tileset.json", tileset(root.clone())),
            ("ext.json", tileset(content_tile(wide(), "b.glb"))),
            ("a.glb", far.clone()),
            ("b.glb", far),
        ];
        let message = outside(run(&files));
        assert!(message.contains("content=b.glb depth=1 "), "{message}");
        root["contents"] = json!([{"uri":"a.glb"}, {"uri":"ext.json"}]);
        let message = outside(run(&[
            ("tileset.json", tileset(root)),
            files[1].clone(),
            files[2].clone(),
            files[3].clone(),
        ]));
        assert!(message.contains("content=a.glb depth=0 "), "{message}");
    }

    #[test]
    fn each_contents_entry_is_a_reference_with_its_own_volume() {
        let mut root = tile(wide());
        root["contents"] = json!([
            {"uri":"a.glb","boundingVolume":boxed([0.; 3], [2.; 3])},
            {"uri":"a.glb","boundingVolume":boxed([0.; 3], [0.5; 3])}
        ]);
        let files = vec![
            ("tileset.json", tileset(root.clone())),
            ("a.glb", points(&[[1., 0., 0.]])),
        ];
        assert!(outside(run(&files)).contains("depth=0 volume=content "));
        root["contents"][1]["boundingVolume"] = boxed([0.; 3], [1.; 3]);
        assert!(contained(
            &run(&[("tileset.json", tileset(root)), files[1].clone()]).unwrap()
        ));
    }
}
