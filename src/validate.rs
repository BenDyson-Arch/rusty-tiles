//! Read-only validation of self-contained explicit and native implicit 3TZ packages.
use crate::archive3tz::TZ_INDEX_NAME;
use crate::content_integrity::implicit::{
    admit_root, declares_implicit, Base, EffectiveError, EffectiveVolume, ImplicitError,
    ImplicitLimits, ImplicitRoot, ImplicitSchema,
};
use crate::content_integrity::{
    json, payload, FormatError, JsonLimits, PayloadError, PayloadLimits,
};
#[cfg(test)]
mod catalog_controls;
mod containment;
#[cfg(test)]
mod memo_controls;
mod rows;
mod types;
pub use rows::{PayloadRows, PayloadRowsIter};
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
    rc::Rc,
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
// I1: the resolver's own failure (including real I/O) passes through unchanged.
fn implicit_error(error: ImplicitError<Error>) -> Error {
    match error {
        ImplicitError::Format(error) => format_error(error),
        ImplicitError::Resolver(error) => error,
        ImplicitError::InvalidState(message) => Error::InvalidState(message.into()),
    }
}
/// `limit − used` for the I1 counter sync; `used` never exceeds its limit.
fn remaining(limit: u64, used: u64) -> Result<u64, Error> {
    limit
        .checked_sub(used)
        .ok_or_else(|| Error::InvalidState("validation counter exceeds its limit".into()))
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
        ReadError::Scratch(error) => spill_failure(error),
    }
}
/// The validator's own scratch failures (design §1.3 error mapping): never a verdict on
/// the archive.
fn spill_failure(error: crate::spill::SpillError) -> Error {
    use crate::spill::SpillError;
    let message = format!("validation scratch: {error}");
    match error {
        SpillError::Io { source, .. } => Error::Io(std::io::Error::new(source.kind(), message)),
        SpillError::InsufficientSpace(_) => Error::Io(std::io::Error::other(message)),
        SpillError::Overflow => Error::limit(message),
        SpillError::CorruptRun { .. } | SpillError::Poisoned | SpillError::InvalidConfig(_) => {
            Error::InvalidState(message)
        }
    }
}
/// One bit per catalog rank: whether a reference reached the member.
struct Bits(Vec<u64>);
impl Bits {
    fn new(len: u64) -> Result<Self, Error> {
        let words = usize::try_from(len.div_ceil(64))
            .map_err(|_| Error::limit("archive entry count exceeds host range"))?;
        let mut bits = Vec::new();
        bits.try_reserve_exact(words).map_err(|_| refused())?;
        bits.resize(words, 0);
        Ok(Self(bits))
    }
    fn set(&mut self, rank: u64) {
        self.0[(rank / 64) as usize] |= 1 << (rank % 64);
    }
    fn get(&self, rank: u64) -> bool {
        self.0[(rank / 64) as usize] & (1 << (rank % 64)) != 0
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
        return checked_box(array(v, 12, "boundingVolume.box")?);
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
/// A finite box with orthogonal half axes, as every explicit tile box is checked.
fn checked_box(b: Vec<f64>) -> Result<Volume, Error> {
    for i in 0..3 {
        for j in i + 1..3 {
            let a = [b[3 + 3 * i], b[4 + 3 * i], b[5 + 3 * i]];
            let c = [b[3 + 3 * j], b[4 + 3 * j], b[5 + 3 * j]];
            if dot(a, c).abs() > 1e-8 * norm(a) * norm(c) + 1e-12 {
                return Err(invalid("box half axes must be orthogonal"));
            }
        }
    }
    Ok(Volume::Box(b))
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
    Arena, Credit, Exclusion, LevelFacts, Local, Normalization, RawVertex, ReferenceFacts, Role,
    SpanCaps, UriBounds, Verdict, VolumeFacts, Word,
};
use payload::scene::{
    ContentExclusions, Instance, NodeTransform, PositionFormat, PositionWord, SceneConsumer,
};

/// The frozen not-inspected reason enum (14 names; H7 removed `rtcCenter`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Reason {
    DerivedImplicitVolume,
    TilesetExtension,
    TileMetadata,
    UpAxisOverride,
    RegionVolume,
    RotatedBox,
    GltfExtension,
    SceneSelection,
    Skin,
    MorphTargets,
    Animation,
    QuaternionWindow,
    NumericSpan,
    WorkLimit,
}
const REASONS: [Reason; 14] = [
    Reason::DerivedImplicitVolume,
    Reason::TilesetExtension,
    Reason::TileMetadata,
    Reason::UpAxisOverride,
    Reason::RegionVolume,
    Reason::RotatedBox,
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
            Self::DerivedImplicitVolume => "derivedImplicitVolume",
            Self::TilesetExtension => "tilesetExtension",
            Self::TileMetadata => "tileMetadata",
            Self::UpAxisOverride => "upAxisOverride",
            Self::RegionVolume => "regionVolume",
            Self::RotatedBox => "rotatedBox",
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
/// H7: legacy 3D Tiles 1.0 content is Unsupported, never partially inspected.
fn legacy(format: &str, name: &str, magic: bool) -> Error {
    let magic = if magic {
        format!(" starts with {format} magic")
    } else {
        String::new()
    };
    Error::unsupported(format!(
        "legacy {format} content is unsupported (glTF 2.0 content only): {name}{magic}"
    ))
}
fn payload_kind(name: &str) -> Result<payload::PayloadKind, Error> {
    match name.to_ascii_lowercase().rsplit_once('.') {
        Some((_, "glb")) => Ok(payload::PayloadKind::Glb),
        Some((_, "gltf")) => Ok(payload::PayloadKind::LocalGltf),
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
    /// `B`, `Dec`, `J`, `Res`, `Prim`: charged again on every reference (C2-WB §3.1).
    per_reference: ReferenceFacts,
    max_depth: u64,
    exclusions: u16,
}
/// One tile of the traversed hierarchy: f64 facts only.
struct TileFact {
    /// The tile's own transform into its parent frame, when present.
    transform: Option<[f64; 16]>,
    /// `None` only for an implicit tile at level ≥ 1 without a row: its
    /// `Derived` cell is never evaluated (slice 3 I5, `derivedImplicitVolume`).
    volume: Option<VolumeFacts>,
    /// For an external-tileset root: the referencing content's boundingVolume.
    external: Option<VolumeFacts>,
    parent: Option<usize>,
    depth: usize,
    /// Inherited chain exclusions (C2-WB §3.3): the caller's own bits, ORed at
    /// registration with the shape exclusions of `volume` and `external` (or
    /// `derivedImplicitVolume` for a `None` volume, slice 3 §4 step 5) and
    /// the parent's `chain`. Every level of a reference's chain except its
    /// content volume is covered, so `Containment::reference` needs no walk.
    chain: u16,
}
/// The reason bit of a volume's shape exclusion, if any.
fn shape_bit(volume: &VolumeFacts) -> u16 {
    match containment::volume_exclusion(volume) {
        Some(Exclusion::RegionVolume) => Reason::RegionVolume.bit(),
        Some(Exclusion::RotatedBox) => Reason::RotatedBox.bit(),
        Some(Exclusion::NumericSpan) | None => 0,
    }
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
    /// Register a tile; its `chain` gains its own shape bits and its parent's.
    /// A rowless implicit tile (`volume: None`) carries `derivedImplicitVolume`
    /// here, so any reference through it is excluded in O(1), whether or not
    /// the precheck has already stopped (slice 3 §4 step 5).
    fn tile(&mut self, mut fact: TileFact) -> Result<usize, Error> {
        fact.chain |= fact
            .volume
            .as_ref()
            .map_or(Reason::DerivedImplicitVolume.bit(), shape_bit)
            | fact.external.as_ref().map_or(0, shape_bit)
            | fact.parent.map_or(0, |p| self.tiles[p].chain);
        try_push(&mut self.tiles, fact)?;
        Ok(self.tiles.len() - 1)
    }
    /// A tile's volume on a level chain. A `None` (derived) volume never
    /// reaches a chain in domain: its chain bit excludes every reference
    /// through it before any level is built (slice 3 §4 step 5).
    fn chain_volume(&self, tile: usize) -> Result<VolumeFacts, Error> {
        self.tiles[tile].volume.ok_or_else(|| {
            Error::InvalidState(
                "internal containment invariant: derived implicit volume on a level chain".into(),
            )
        })
    }
    /// The level chain of a reference: own tile and content volume at level 0,
    /// then each ancestor through external-tileset roots (I1, I2). Implicit
    /// levels have no transform; an implicit root's transform steps out of
    /// its own level (slice 3 §4 table).
    fn build_levels(&mut self, tile: usize, content: Option<usize>) -> Result<(), Error> {
        self.levels.clear();
        let content = content.map(|c| (Role::Content, self.content_volumes[c]));
        let own = self.chain_volume(tile)?;
        try_push(
            &mut self.levels,
            LevelFacts {
                to_parent: None,
                volumes: [Some((Role::Own, own)), content, None],
            },
        )?;
        let mut current = tile;
        while let Some(parent) = self.tiles[current].parent {
            let level = self.levels.len() as u32;
            let volume = self.chain_volume(parent)?;
            let child = &self.tiles[current];
            let fact = LevelFacts {
                to_parent: child.transform,
                volumes: [
                    Some((Role::Ancestor(level), volume)),
                    None,
                    child.external.map(|v| (Role::ExternalContent(level), v)),
                ],
            };
            try_push(&mut self.levels, fact)?;
            current = parent;
        }
        Ok(())
    }
    /// Collect chain exclusions in O(1) and run the precheck step for one
    /// reference, in traversal order. The precheck stops at its first violation;
    /// the level chain is built only when the precheck step runs (C2-WB §3.3).
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
        // Chain bits are ORed even after the precheck has stopped: any exclusion
        // anywhere overrides the precheck reason.
        self.exclusions |= facts.exclusions
            | self.tiles[tile].chain
            | content.map_or(0, |c| shape_bit(&self.content_volumes[c]));
        // The precheck only runs while the archive is still entirely in domain.
        if self.exclusions != 0 || self.precheck.is_some() {
            return Ok(());
        }
        self.build_levels(tile, content)?;
        self.precheck = self.precheck_step(&facts);
        Ok(())
    }
    /// One reference's charge: `T·V + Nodes + Idx + E` plus the per-reference
    /// terms (C2-WB §3.1), against the work cap; `B` and `Dec` also against
    /// their own caps. Requires `self.levels` built for this reference.
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
        let per_reference = &facts.per_reference;
        let units = facts.nodes.and_then(|nodes| {
            vertex_units
                .checked_add(nodes)?
                .checked_add(facts.idx)?
                .checked_add(facts.e)?
                .checked_add(containment::reference_units(
                    self.levels.len(),
                    per_reference,
                )?)
        });
        let within = |total: u64, add: Option<u64>, cap: u64| {
            add.and_then(|add| total.checked_add(add))
                .filter(|sum| *sum <= cap)
        };
        let (Some(units), Some(bytes), Some(decoded)) = (
            within(self.units, units, self.work_cap),
            within(self.bytes, Some(per_reference.b), self.bytes_cap),
            within(self.decoded, Some(per_reference.dec), self.decoded_cap),
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
    /// Maximum path `s_step` over every instance, with or without POSITION.
    s_inst_all: u32,
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
        // sInst ranges over all instances (contract "Per-URI facts").
        self.s_inst_all = self.s_inst_all.max(s_inst);
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
    /// The stream's corner-walk budget (H11 V7).
    credit: &'a mut Credit,
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
            let first =
                containment::walk_marked(self.arena, n, self.levels, bitmap, self.credit, |v| {
                    raw_vertex(instance.vertex(accessor, v))
                });
            if self.arena.fault() {
                return Err(containment_fault());
            }
            if let Some((vertex, Verdict::Outside { role, .. })) = first {
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
        Ok(())
    }
}

struct ArchiveResources<'a> {
    archive: &'a mut Archive,
    /// Members reached by a reference, by catalog rank.
    used: Bits,
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
            .ok_or_else(|| {
                Error::limit(format!(
                    "archive exceeds {} reference visits",
                    self.limits.references
                ))
            })?;
        self.mark(&name)
    }
    /// Records use of an existing member; a missing one is InvalidInput.
    fn mark(&mut self, name: &str) -> Result<(), Error> {
        match self.archive.find(name).map_err(archive_admission_error)? {
            Some(member) => {
                self.used.set(member.rank);
                Ok(())
            }
            None => Err(invalid(format!("missing archive entry: {name}"))),
        }
    }
    /// An emitted implicit content (§2a step 10, DC-3): its reference was
    /// reserved at subtree admission, so this records use without a charge.
    fn reference_reserved(&mut self, name: &str) -> Result<(), Error> {
        self.mark(name)
    }
}
/// A metadata schema document: inline schemas are keyed by their tileset
/// member, `schemaUri` schemas by the schema member (DC-4).
#[derive(PartialEq, Eq, Hash)]
enum SchemaKey {
    Inline(String),
    Uri(String),
}
/// One visited implicit tile's facts, for the parity seam (RC-16).
#[cfg(test)]
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct ImplicitTileFact {
    /// The tileset member holding R.
    pub doc: String,
    pub level: u32,
    pub coords: [u32; 3],
    pub depth: usize,
    /// The path parent's level; `None` for I_0, whose parent is R.
    pub parent_level: Option<u32>,
    pub volume: EffectiveVolume,
    pub error: EffectiveError,
    pub refine: Option<&'static str>,
    pub contents: Vec<(u16, String)>,
}
struct Check<'a> {
    resources: ArchiveResources<'a>,
    reports: Value,
    tiles: usize,
    contents: usize,
    /// Link tiles: explicit tiles whose contents are all external tilesets.
    links: usize,
    active: HashSet<String>,
    schema: jsonschema::Validator,
    task_visits: u64,
    elements: u64,
    /// Report rows of first-inspected payloads, in inspection order until the report.
    rows: rows::RowSink<'a>,
    /// Counts of each first-inspected payload, by its C2 URI index (repeat references).
    payload_counts: Vec<[u64; 3]>,
    not_inspected: BTreeSet<String>,
    /// One per validation: the I1 allowances and the OD-7 subtree set.
    implicit: ImplicitLimits,
    /// Implicit roots read in this validation (report labels, OD-4).
    implicit_roots: u64,
    schemas: HashMap<SchemaKey, Rc<ImplicitSchema>>,
    #[cfg(test)]
    implicit_facts: Vec<ImplicitTileFact>,
    c2: Containment,
    /// EM: one summary per completed, implicit-free external-tileset document.
    memo: HashMap<String, Rc<Summary>>,
    /// EM: the expansions being traversed in full, innermost last.
    recording: Vec<Recorder>,
    /// First-inspection payload charges (EM-2): never steady charges.
    once_references: u64,
    once_bytes: u64,
    /// The reference traversal switch of the controls (EM terms).
    #[cfg(test)]
    memoize: bool,
    /// Documents prepared (parsed and schema-checked), for the engagement control.
    #[cfg(test)]
    prepared: u64,
}
/// EM: the memoized expansion of one external-tileset placement.
struct Summary {
    /// Steady charges of one placement (EM-2).
    visits: u64,
    references: u64,
    bytes: u64,
    /// Report counts of one expansion.
    tiles: usize,
    contents: usize,
    links: usize,
    /// Deepest tileset task and tile of the expansion, below the placement depth.
    tileset_depth: usize,
    node_depth: usize,
    /// The placement-dependent root facts (EM-5.2, EM-5.3).
    root_bounds: Volume,
    root_transform: [f64; 16],
    root_error: f64,
    document_error: f64,
    /// The document's own C2 registrations, in traversal order (EM-5.4).
    events: Vec<Event>,
}
/// One C2 registration of a document's expansion. Indices into `locals` are
/// the document's own tiles in registration order; `first` indices name the
/// first placement's facts in the C2 arena and reference list.
enum Event {
    /// A tile; `parent: None` is the root, linked to the placement.
    Tile {
        first: usize,
        parent: Option<usize>,
        depth: usize,
    },
    Reference {
        tile: usize,
        first: usize,
    },
    Place(Box<Placement>),
}
struct Placement {
    summary: Rc<Summary>,
    parent: usize,
    external: Option<VolumeFacts>,
    depth: usize,
}
/// EM: one placement traversed in full, measured for its summary.
struct Recorder {
    depth: usize,
    link: Link,
    visits: u64,
    references: u64,
    bytes: u64,
    once_references: u64,
    once_bytes: u64,
    tiles: usize,
    contents: usize,
    links: usize,
    tileset_depth: usize,
    node_depth: usize,
    document_error: f64,
    root: Option<(Volume, [f64; 16], f64)>,
    /// Arena indices of the document's own tiles, ascending.
    locals: Vec<usize>,
    events: Vec<Event>,
    /// An implicit root in the expansion (EM-3), or an unmapped registration.
    opaque: bool,
}
impl Recorder {
    fn local(&self, tile: Option<usize>) -> Option<usize> {
        self.locals.binary_search(&tile?).ok()
    }
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
        #[cfg(test)]
        refine: Option<&'static str>,
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
    /// The metadata schema of the document holding this tile, kept only for
    /// documents that hold an implicit root.
    schema: Option<Rc<ImplicitSchema>>,
    /// The inherited `refine`, recorded only in the parity facts.
    #[cfg(test)]
    refine: Option<&'static str>,
}
/// The explicit facts of an implicit root R that its walk consumes.
struct ImplicitParent {
    #[cfg(test)]
    base: String,
    /// R's index in the C2 tile arena: the parent of I_0's `TileFact`.
    tile: usize,
    bounds: Volume,
    error: f64,
    depth: usize,
    #[cfg(test)]
    refine: Option<&'static str>,
}
impl Check<'_> {
    /// The single payload classifier (H7 INV-4): every payload caller comes here.
    fn payload(&mut self, name: &str) -> Result<PayloadReport, Error> {
        if let Some(&uri) = self.c2.uri_index.get(name) {
            let [accessors_checked, primitives_checked, vertices] = self.payload_counts[uri];
            return Ok(PayloadReport {
                uri: name.into(),
                accessors_checked,
                primitives_checked,
                vertices,
            });
        }
        // L0 is the caller's: every caller has already established that the
        // member exists (`Resources::reference` or the catalog lookup for the
        // tile byte count), so a missing member stays InvalidInput.
        // L1: a legacy suffix is Unsupported before any payload byte is read.
        if let Some(format) = payload::legacy_suffix(name) {
            return Err(legacy(format, name, false));
        }
        let references_before = self.resources.reference_visits;
        let bytes_before = self.resources.bytes_read;
        let report = self.inspect_payload(name, bytes_before)?;
        // EM-2: a payload's first inspection is charged once per validation.
        self.once_references += self.resources.reference_visits - references_before;
        self.once_bytes += self.resources.bytes_read - bytes_before;
        Ok(report)
    }
    fn inspect_payload(&mut self, name: &str, bytes_before: u64) -> Result<PayloadReport, Error> {
        let bytes = read_member(
            self.resources.archive,
            name,
            &mut self.resources.bytes_read,
            &self.resources.limits,
            self.resources.limits.member_bytes,
        )?;
        // L2: legacy magic in the buffer just read wins over any other suffix.
        if let Some(format) = payload::legacy_magic(&bytes) {
            return Err(legacy(format, name, true));
        }
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
            s_inst_all: 0,
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
            // A URI with no POSITION-bearing instance has no Pair0 and streams
            // nothing; otherwise sInst covers every instance's node path.
            bounds: consumer.bounds.map(|mut bounds| {
                bounds.s_inst = bounds.s_inst.max(consumer.s_inst_all);
                bounds
            }),
            t: consumer.t,
            nodes: consumer.nodes,
            idx: consumer.idx,
            e: facts.elements_checked,
            per_reference: ReferenceFacts {
                b: self.resources.bytes_read - bytes_before,
                dec: facts.materialized_bytes,
                json_nodes: facts.json_nodes,
                external_resources: facts.external_resources,
                instance_primitives: facts.instance_primitives,
            },
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
        self.record_payload(&report)?;
        Ok(report)
    }
    /// The report row and repeat counts of a first-inspected payload, whose C2 URI facts
    /// were just registered.
    fn record_payload(&mut self, report: &PayloadReport) -> Result<(), Error> {
        let member = self
            .resources
            .archive
            .member(&report.uri)
            .map_err(archive_admission_error)?;
        self.rows.push(&report.uri, member.name_ref, report)?;
        self.payload_counts.try_reserve(1).map_err(|_| refused())?;
        self.payload_counts.push([
            report.accessors_checked,
            report.primitives_checked,
            report.vertices,
        ]);
        debug_assert_eq!(self.payload_counts.len(), self.c2.uris.len());
        Ok(())
    }
    /// Phase 2 (outcome 4): serial per-reference streaming, in traversal order.
    /// Re-reads are charged only to `containmentBytesRead`; C1 counters are untouched.
    fn stream(&mut self) -> Result<(), Error> {
        let mut arena = match self.c2.arena.take() {
            Some(arena) => arena,
            None => Arena::reserve(self.c2.caps).map_err(|_| refused())?,
        };
        let mut bitmap = Vec::new();
        let mut credit = Credit::default();
        let mut reread = 0u64;
        let cap = self.resources.limits.containment_bytes_read;
        let limits = self.resources.content_limits;
        let references = std::mem::take(&mut self.c2.references);
        for reference in &references {
            self.c2.build_levels(reference.tile, reference.content)?;
            credit.accrue(self.c2.uris[reference.uri].1.t);
            let name = self.c2.uris[reference.uri].0.as_str();
            let archive = &mut *self.resources.archive;
            let bytes = read_rerun(archive, name, &mut reread, cap, limits.member_bytes as u64)?;
            let mut consumer = StreamConsumer {
                arena: &mut arena,
                bitmap: &mut bitmap,
                levels: &self.c2.levels,
                member: name,
                depth: self.c2.tiles[reference.tile].depth,
                credit: &mut credit,
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
    /// The `schemaUri` document of a tileset, read once per preparation.
    fn metadata_schema(
        &mut self,
        base: &str,
        doc: &Value,
    ) -> Result<Option<(String, Value)>, Error> {
        let Some(value) = doc.get("schemaUri") else {
            return Ok(None);
        };
        let value = value
            .as_str()
            .ok_or_else(|| invalid("schemaUri must be a string"))?;
        self.schema_uri(base, value).map(Some)
    }
    fn schema_uri(&mut self, base: &str, value: &str) -> Result<(String, Value), Error> {
        let name = uri(base, value)?;
        self.resources.reference(name.clone())?;
        let schema = self.resources.read_json(&name)?;
        Ok((name, schema))
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
            #[cfg(test)]
            refine: None,
        }];
        while let Some(task) = pending.pop() {
            if let ValidationTask::Reference { tile, content, uri } = task {
                self.c2_reference(tile, content, uri)?;
                continue;
            }
            if let ValidationTask::Tileset {
                name,
                parent,
                parent_error,
                depth,
                link,
                ..
            } = &task
            {
                if let Some(summary) = self.summary(name, *depth) {
                    self.place(&summary, parent.as_ref(), *parent_error, *depth, *link)?;
                    continue;
                }
            }
            self.task_visits = self
                .task_visits
                .checked_add(1)
                .filter(|&n| n <= self.resources.limits.hierarchy_visits)
                .ok_or_else(|| {
                    Error::limit(format!(
                        "hierarchy exceeds {} traversal tasks",
                        self.resources.limits.hierarchy_visits
                    ))
                })?;
            match task {
                ValidationTask::Tileset {
                    name,
                    parent,
                    parent_error,
                    depth,
                    link,
                    #[cfg(test)]
                    refine,
                } => {
                    if depth > 128 {
                        return Err(Error::limit("hierarchy exceeds validation depth limit"));
                    }
                    if !self.active.insert(name.clone()) {
                        return Err(invalid("cyclic external tileset reference"));
                    }
                    self.begin(depth, link)?;
                    let (mut doc, error, schema) = self.prepare_tileset(&name)?;
                    if let Some(recorder) = self.recording.last_mut() {
                        recorder.document_error = error;
                    }
                    pending.push(ValidationTask::ExitTileset(name.clone()));
                    pending.push(ValidationTask::Node(NodeTask {
                        node: doc["root"].take(),
                        base: name,
                        parent,
                        parent_error: parent_error.map_or(error, |parent| parent.min(error)),
                        depth,
                        link,
                        schema,
                        #[cfg(test)]
                        refine,
                    }));
                }
                ValidationTask::Node(task) => {
                    self.node(task, &mut pending)?;
                }
                ValidationTask::ExitTileset(name) => {
                    self.active.remove(&name);
                    self.finish(name)?;
                }
                ValidationTask::Reference { .. } => unreachable!("handled before charging"),
            }
        }
        Ok(())
    }

    /// EM-6: the document's summary, if this placement may use it: the document
    /// is not on the active path, and every steady charge and depth bound fits.
    fn summary(&self, name: &str, depth: usize) -> Option<Rc<Summary>> {
        #[cfg(test)]
        if !self.memoize {
            return None;
        }
        if self.active.contains(name) {
            return None;
        }
        let summary = self.memo.get(name)?;
        let limits = &self.resources.limits;
        let within =
            |used: u64, add: u64, cap: u64| used.checked_add(add).is_some_and(|n| n <= cap);
        let deep = |add: usize, cap: u64| {
            depth
                .checked_add(add)
                .is_some_and(|d| u64::try_from(d).is_ok_and(|d| d <= cap))
        };
        (within(self.task_visits, summary.visits, limits.hierarchy_visits)
            && within(
                self.resources.reference_visits,
                summary.references,
                limits.references,
            )
            && within(
                self.resources.bytes_read,
                summary.bytes,
                limits.total_bytes_read,
            )
            && deep(summary.tileset_depth, 128)
            && deep(summary.node_depth, limits.hierarchy_depth))
        .then(|| summary.clone())
    }
    /// EM-5: one memoized placement. Its steady charges fit (EM-6), so only the
    /// root's parent-dependent checks can fail; then its C2 facts are registered.
    fn place(
        &mut self,
        summary: &Rc<Summary>,
        parent: Option<&Volume>,
        parent_error: Option<f64>,
        depth: usize,
        link: Link,
    ) -> Result<(), Error> {
        self.task_visits += summary.visits;
        self.resources.reference_visits += summary.references;
        self.resources.bytes_read += summary.bytes;
        if let Some(parent) = parent {
            if !contains(parent, &summary.root_bounds, &summary.root_transform)? {
                return Err(invalid(format!(
                    "child bounds escape parent at depth {depth}"
                )));
            }
        }
        let bound = parent_error.map_or(summary.document_error, |parent| {
            parent.min(summary.document_error)
        });
        if summary.root_error > bound + 1e-8 * (1. + bound) {
            return Err(invalid(format!(
                "geometricError increases at depth {depth}"
            )));
        }
        self.tiles += summary.tiles;
        self.contents += summary.contents;
        self.links += summary.links;
        if let Some(recorder) = self.recording.last_mut() {
            recorder.tileset_depth = recorder.tileset_depth.max(depth + summary.tileset_depth);
            recorder.node_depth = recorder.node_depth.max(depth + summary.node_depth);
            match recorder.local(link.parent) {
                Some(parent) => {
                    let placement = Placement {
                        summary: summary.clone(),
                        parent,
                        external: link.external,
                        depth: depth - recorder.depth,
                    };
                    recorder.events.try_reserve(1).map_err(|_| refused())?;
                    recorder.events.push(Event::Place(Box::new(placement)));
                }
                None => recorder.opaque = true,
            }
        }
        self.replay(summary, link, depth)
    }
    /// EM-5.4: register a summary's C2 facts again under this placement's link
    /// and depth, in the reference traversal's order. Not recorded: an
    /// enclosing recorder holds the placement itself.
    fn replay(&mut self, summary: &Rc<Summary>, link: Link, depth: usize) -> Result<(), Error> {
        struct Frame {
            summary: Rc<Summary>,
            next: usize,
            locals: Vec<usize>,
            link: Link,
            depth: usize,
        }
        let mut frames = vec![Frame {
            summary: summary.clone(),
            next: 0,
            locals: Vec::new(),
            link,
            depth,
        }];
        while let Some(frame) = frames.last_mut() {
            let Some(event) = frame.summary.events.get(frame.next) else {
                frames.pop();
                continue;
            };
            frame.next += 1;
            let nested = match event {
                Event::Tile {
                    first,
                    parent,
                    depth,
                } => {
                    let (parent, external) = match parent {
                        None => (frame.link.parent, frame.link.external),
                        Some(local) => (Some(frame.locals[*local]), None),
                    };
                    let fact = &self.c2.tiles[*first];
                    let index = self.c2.tile(TileFact {
                        transform: fact.transform,
                        volume: fact.volume,
                        external,
                        parent,
                        depth: frame.depth + depth,
                        chain: 0,
                    })?;
                    try_push(&mut frame.locals, index)?;
                    None
                }
                Event::Reference { tile, first } => {
                    let Reference { uri, content, .. } = self.c2.references[*first];
                    let content = content.map(|c| self.c2.content_volumes[c]);
                    self.c2.reference(frame.locals[*tile], content, uri)?;
                    None
                }
                Event::Place(placement) => Some(Frame {
                    summary: placement.summary.clone(),
                    next: 0,
                    locals: Vec::new(),
                    link: Link {
                        parent: Some(frame.locals[placement.parent]),
                        external: placement.external,
                    },
                    depth: frame.depth + placement.depth,
                }),
            };
            if let Some(nested) = nested {
                try_push(&mut frames, nested)?;
            }
        }
        Ok(())
    }
    /// EM: start measuring a placement traversed in full. Its `Tileset` visit
    /// is already charged.
    fn begin(&mut self, depth: usize, link: Link) -> Result<(), Error> {
        let recorder = Recorder {
            depth,
            link,
            visits: self.task_visits - 1,
            references: self.resources.reference_visits,
            bytes: self.resources.bytes_read,
            once_references: self.once_references,
            once_bytes: self.once_bytes,
            tiles: self.tiles,
            contents: self.contents,
            links: self.links,
            tileset_depth: depth,
            node_depth: depth,
            document_error: 0.,
            root: None,
            locals: Vec::new(),
            events: Vec::new(),
            opaque: false,
        };
        try_push(&mut self.recording, recorder)
    }
    /// EM: a placement traversed in full has completed: summarize its document
    /// unless its expansion is opaque (EM-3), and enter it in the enclosing one.
    fn finish(&mut self, name: String) -> Result<(), Error> {
        let Some(recorder) = self.recording.pop() else {
            return Err(Error::InvalidState(
                "external tileset exit without a recorder".into(),
            ));
        };
        let summary = match recorder.root {
            Some((root_bounds, root_transform, root_error)) if !recorder.opaque => {
                let once_references = self.once_references - recorder.once_references;
                let once_bytes = self.once_bytes - recorder.once_bytes;
                Some(Rc::new(Summary {
                    visits: self.task_visits - recorder.visits,
                    references: self.resources.reference_visits
                        - recorder.references
                        - once_references,
                    bytes: self.resources.bytes_read - recorder.bytes - once_bytes,
                    tiles: self.tiles - recorder.tiles,
                    contents: self.contents - recorder.contents,
                    links: self.links - recorder.links,
                    tileset_depth: recorder.tileset_depth - recorder.depth,
                    node_depth: recorder.node_depth - recorder.depth,
                    root_bounds,
                    root_transform,
                    root_error,
                    document_error: recorder.document_error,
                    events: recorder.events,
                }))
            }
            _ => None,
        };
        if let Some(summary) = &summary {
            self.memo.try_reserve(1).map_err(|_| refused())?;
            self.memo.insert(name, summary.clone());
        }
        if let Some(outer) = self.recording.last_mut() {
            outer.tileset_depth = outer.tileset_depth.max(recorder.tileset_depth);
            outer.node_depth = outer.node_depth.max(recorder.node_depth);
            match (summary, outer.local(recorder.link.parent)) {
                (Some(summary), Some(parent)) => {
                    let placement = Placement {
                        summary,
                        parent,
                        external: recorder.link.external,
                        depth: recorder.depth - outer.depth,
                    };
                    outer.events.try_reserve(1).map_err(|_| refused())?;
                    outer.events.push(Event::Place(Box::new(placement)));
                }
                _ => outer.opaque = true,
            }
        }
        Ok(())
    }
    /// A C2 tile registration of the full traversal, recorded for the summary
    /// of the innermost document being traversed.
    fn c2_tile(&mut self, fact: TileFact) -> Result<usize, Error> {
        let (parent, external, depth) = (fact.parent, fact.external, fact.depth);
        let index = self.c2.tile(fact)?;
        if let Some(recorder) = self.recording.last_mut() {
            // The first registration is the root, linked to the placement;
            // every other tile's parent is one of the document's own tiles.
            let parent = if recorder.locals.is_empty() {
                (parent == recorder.link.parent && external == recorder.link.external)
                    .then_some(None)
            } else {
                recorder
                    .local(parent)
                    .filter(|_| external.is_none())
                    .map(Some)
            };
            match parent {
                Some(parent) => {
                    recorder.events.try_reserve(1).map_err(|_| refused())?;
                    recorder.events.push(Event::Tile {
                        first: index,
                        parent,
                        depth: depth - recorder.depth,
                    });
                    try_push(&mut recorder.locals, index)?;
                }
                None => recorder.opaque = true,
            }
        }
        Ok(index)
    }
    /// A C2 reference registration of the full traversal, recorded likewise.
    fn c2_reference(
        &mut self,
        tile: usize,
        content: Option<VolumeFacts>,
        uri: usize,
    ) -> Result<(), Error> {
        self.c2.reference(tile, content, uri)?;
        if let Some(recorder) = self.recording.last_mut() {
            match recorder.local(Some(tile)) {
                Some(tile) => {
                    recorder.events.try_reserve(1).map_err(|_| refused())?;
                    recorder.events.push(Event::Reference {
                        tile,
                        first: self.c2.references.len() - 1,
                    });
                }
                None => recorder.opaque = true,
            }
        }
        Ok(())
    }

    fn prepare_tileset(
        &mut self,
        name: &str,
    ) -> Result<(Value, f64, Option<Rc<ImplicitSchema>>), Error> {
        #[cfg(test)]
        {
            self.prepared += 1;
        }
        self.resources.reference(name.into())?;
        let mut doc = self.resources.read_json(name)?;
        self.schema
            .validate(&doc)
            .map_err(|error| invalid(format!("tileset schema: {error}")))?;
        if !matches!(doc["asset"]["version"].as_str(), Some("1.0" | "1.1")) {
            return Err(invalid("asset.version must be 1.0 or 1.1"));
        }
        let error = number(&doc["geometricError"], "tileset.geometricError")?;
        let schema_uri = self.metadata_schema(name, &doc)?;
        self.tileset_exclusions(&doc);
        // One ImplicitSchema per schema document, kept only for documents that
        // hold an implicit root (§5, DC-4). `schema` and `schemaUri` are
        // mutually exclusive in the tileset schema.
        let schema = if holds_implicit_root(&doc["root"]) {
            let (key, value) = match schema_uri {
                Some((member, value)) => (SchemaKey::Uri(member), Some(value)),
                None => (
                    SchemaKey::Inline(name.to_owned()),
                    doc.get_mut("schema").map(Value::take),
                ),
            };
            match value {
                Some(value) => {
                    self.schemas.try_reserve(1).map_err(|_| refused())?;
                    Some(
                        self.schemas
                            .entry(key)
                            .or_insert_with(|| Rc::new(ImplicitSchema::new(value)))
                            .clone(),
                    )
                }
                None => None,
            }
        } else {
            None
        };
        if doc["root"].get("implicitTiling").is_some() {
            if let Some(recorder) = self.recording.last_mut() {
                recorder.opaque = true;
            }
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
        }
        Ok((doc, error, schema))
    }

    /// Document-level C2 exclusion facts.
    fn tileset_exclusions(&mut self, doc: &Value) {
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
    /// Report-level recorded budgets of one tile with content (OD-4: these
    /// apply to explicit and implicit tiles alike).
    fn tile_budgets(&self, bytes: u64, vertices: u64) -> Result<(), Error> {
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
        Ok(())
    }
    fn node(&mut self, task: NodeTask, pending: &mut Vec<ValidationTask>) -> Result<(), Error> {
        let NodeTask {
            mut node,
            base,
            parent,
            parent_error,
            depth,
            link,
            schema,
            #[cfg(test)]
            refine,
        } = task;
        if depth as u64 > self.resources.limits.hierarchy_depth {
            return Err(Error::limit("hierarchy exceeds validation depth limit"));
        }
        if let Some(recorder) = self.recording.last_mut() {
            recorder.node_depth = recorder.node_depth.max(depth);
        }
        if !node.is_object() {
            return Err(invalid("tile must be an object"));
        }
        // I1 root admission precedes R's explicit checks (contract precedence):
        // fields, scheme, levels, volume kind, shape, templates, header classes.
        let implicit = if declares_implicit(&node) {
            Some(
                admit_root(&node, &base, schema.as_deref())
                    .map_err(|e| implicit_error(e.widen()))?,
            )
        } else {
            None
        };
        if implicit.is_some() {
            // EM-3: an implicit root is walked once per validation (OD-7).
            if let Some(recorder) = self.recording.last_mut() {
                recorder.opaque = true;
            }
        }
        if let Some(refine) = node.get("refine") {
            if !matches!(refine.as_str(), Some("ADD" | "REPLACE")) {
                return Err(invalid("invalid refine value"));
            }
        }
        #[cfg(test)]
        let refine = match node["refine"].as_str() {
            Some("ADD") => Some("ADD"),
            Some(_) => Some("REPLACE"),
            None => refine,
        };
        let bounds = volume(&node["boundingVolume"])?;
        let m = transform(&node)?;
        if node.get("metadata").is_some() {
            self.c2.exclude(Reason::TileMetadata);
        }
        self.extension_object(&node);
        self.extension_object(&node["boundingVolume"]);
        let tile = self.c2_tile(TileFact {
            transform: node.get("transform").map(|_| std::array::from_fn(|i| m[i])),
            volume: Some(volume_facts(&bounds)),
            external: link.external,
            parent: link.parent,
            depth,
            chain: 0,
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
        if let Some(recorder) = self.recording.last_mut() {
            // The document's root is its first registered tile (EM-5.2, EM-5.3).
            if recorder.root.is_none() && recorder.locals.first() == Some(&tile) {
                recorder.root = Some((bounds.clone(), std::array::from_fn(|i| m[i]), error));
            }
        }
        self.tiles += 1;
        if let Some(root) = implicit {
            // R's content entries are templates: never explicit references.
            // A header `metadata` entity is copied to every emitted content, so
            // it is "content has `metadata`" (slice 3 I6, §4 step 7).
            for slot in 0..root.slots() {
                let header = root.header(slot);
                if header.get("metadata").is_some() {
                    self.c2.exclude(Reason::TileMetadata);
                }
                self.extension_object(header);
            }
            return self.walk_implicit(
                &root,
                ImplicitParent {
                    #[cfg(test)]
                    base,
                    tile,
                    bounds,
                    error,
                    depth,
                    #[cfg(test)]
                    refine,
                },
            );
        }
        let mut bytes = 0u64;
        let mut vertices = 0u64;
        let mut descendants = Vec::new();
        if node.get("content").is_some() && node.get("contents").is_some() {
            return Err(invalid("tile has both content and contents"));
        }
        let mut external_seen = false;
        let mut payload_seen = false;
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
            if name.ends_with(".json") {
                // Amendment D-2: an external tileset document is not tile
                // payload, so it is not charged to the per-tile byte budget
                // (it stays charged to the archive and JSON read limits).
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
                    #[cfg(test)]
                    refine,
                });
                continue;
            }
            bytes = bytes
                .checked_add(
                    self.resources
                        .archive
                        .member(&name)
                        .map_err(archive_admission_error)?
                        .size,
                )
                .ok_or_else(|| invalid("tile byte count overflow"))?;
            payload_seen = true;
            let inspected = self.payload(&name)?;
            let uri = self.c2.uri_index[&name];
            let content = content_volume.as_ref().map(volume_facts);
            // C1 inspects inline; the C2 reference keeps contents in array order
            // with each external tileset expanded in place.
            if external_seen {
                descendants.push(ValidationTask::Reference { tile, content, uri });
            } else {
                self.c2_reference(tile, content, uri)?;
            }
            vertices = vertices
                .checked_add(inspected.vertices)
                .ok_or_else(|| invalid("tile vertex count overflow"))?;
        }
        // Amendment D-1: a link tile (external tileset content only) is
        // reported but not compared against the recorded tile budget.
        if external_seen && !payload_seen {
            self.links += 1;
        }
        if bytes > 0 {
            self.tile_budgets(bytes, vertices)?;
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
                    schema: schema.clone(),
                    #[cfg(test)]
                    refine,
                }));
            }
        }
        pending.extend(descendants.into_iter().rev());
        Ok(())
    }
    /// The I1 walk of one admitted root R, after R's explicit checks. Each
    /// visited tile gets the explicit-tile checks: `hierarchyBounds` and
    /// `geometricErrorOrder` against its path parent (R for I_0), C1 payloads
    /// and report-level budgets. C2 (slice 3 §4 steps 3-4): each visited tile
    /// is a `TileFact` whose parent is R for I_0 and its path parent otherwise,
    /// and each emitted content is a C2 reference, in walk order.
    fn walk_implicit(&mut self, root: &ImplicitRoot<'_>, r: ImplicitParent) -> Result<(), Error> {
        self.implicit_roots += 1;
        let mut walk = root.walk();
        // Level-indexed path stack (RC-21): entry `l` is the tile at level `l`,
        // with its C1 volume, its error and its C2 tile index.
        let mut path: Vec<(Volume, f64, usize)> = Vec::new();
        loop {
            // Counter sync around every `Walk::next` (owner charging protocol).
            let limits = &self.resources.limits;
            self.implicit.visits_remaining = remaining(limits.hierarchy_visits, self.task_visits)?;
            self.implicit.refs_remaining =
                remaining(limits.references, self.resources.reference_visits)?;
            self.implicit.scan_remaining =
                remaining(limits.total_bytes_read, self.resources.bytes_read)?;
            let resources = &mut self.resources;
            let next = walk.next(
                &mut self.implicit,
                &mut |name: &str, base: Base<'_>, maximum: u64| -> Result<Vec<u8>, Error> {
                    let member = match base {
                        Base::Resolved => name.to_owned(),
                        Base::Relative(from) => uri(from, name)?,
                    };
                    resources.reference(member.clone())?;
                    let maximum = maximum.min(resources.limits.member_bytes);
                    read_member(
                        resources.archive,
                        &member,
                        &mut resources.bytes_read,
                        &resources.limits,
                        maximum,
                    )
                },
            );
            let limits = &self.resources.limits;
            self.task_visits = limits.hierarchy_visits - self.implicit.visits_remaining;
            self.resources.reference_visits = limits.references - self.implicit.refs_remaining;
            self.resources.bytes_read = limits.total_bytes_read - self.implicit.scan_remaining;
            let Some(tile) = next.map_err(implicit_error)? else {
                break;
            };
            let level = tile.level as usize;
            let depth = r.depth + level;
            if depth as u64 > self.resources.limits.hierarchy_depth {
                return Err(Error::limit("hierarchy exceeds validation depth limit"));
            }
            let volume = match &tile.volume {
                EffectiveVolume::Row(b) => checked_box(b.to_vec())?,
                EffectiveVolume::DerivedRoot => r.bounds.clone(),
                // OD-3: the labelled rounded evaluation, for this check only.
                EffectiveVolume::Derived(cell) => {
                    Volume::Box(cell.rounded_for_hierarchy_bounds().to_vec())
                }
            };
            let (EffectiveError::Row(error) | EffectiveError::Derived(error)) = tile.error;
            let (parent, parent_error, parent_tile) = match level {
                0 => (&r.bounds, r.error, r.tile),
                _ => path
                    .get(level - 1)
                    .map(|(volume, error, index)| (volume, *error, *index))
                    .ok_or_else(|| {
                        Error::InvalidState("implicit tile without a path parent".into())
                    })?,
            };
            if !contains(parent, &volume, &IDENTITY)? {
                return Err(invalid(format!(
                    "child bounds escape parent at depth {depth}"
                )));
            }
            if error > parent_error + 1e-8 * (1. + parent_error) {
                return Err(invalid(format!(
                    "geometricError increases at depth {depth}"
                )));
            }
            // C2 facts: rows exactly (FLOAT32 already widened by I1), V_R for
            // a rowless I_0, and no volume for a derived cell (I5).
            let index = self.c2.tile(TileFact {
                transform: None,
                volume: match &tile.volume {
                    EffectiveVolume::Row(b) => Some(VolumeFacts::Box(*b)),
                    EffectiveVolume::DerivedRoot => Some(volume_facts(&r.bounds)),
                    EffectiveVolume::Derived(_) => None,
                },
                external: None,
                parent: Some(parent_tile),
                depth,
                chain: 0,
            })?;
            path.truncate(level);
            try_push(&mut path, (volume, error, index))?;
            // R and I_0 are one tile for counting.
            if level > 0 {
                self.tiles += 1;
            }
            let mut bytes = 0u64;
            let mut vertices = 0u64;
            for (_, name) in &tile.contents {
                self.resources.reference_reserved(name)?;
                self.contents += 1;
                bytes = bytes
                    .checked_add(
                        self.resources
                            .archive
                            .member(name)
                            .map_err(archive_admission_error)?
                            .size,
                    )
                    .ok_or_else(|| invalid("tile byte count overflow"))?;
                // JSON detection at admission leaves only payload members here.
                let inspected = self.payload(name)?;
                // C2 registration only: C1 already counted this reference at
                // subtree admission (`reference_reserved` above).
                let uri = self.c2.uri_index[name.as_str()];
                self.c2.reference(index, None, uri)?;
                vertices = vertices
                    .checked_add(inspected.vertices)
                    .ok_or_else(|| invalid("tile vertex count overflow"))?;
            }
            if bytes > 0 {
                self.tile_budgets(bytes, vertices)?;
            }
            #[cfg(test)]
            try_push(
                &mut self.implicit_facts,
                ImplicitTileFact {
                    doc: r.base.clone(),
                    level: tile.level,
                    coords: tile.coords,
                    depth,
                    parent_level: tile.level.checked_sub(1),
                    volume: tile.volume,
                    error: tile.error,
                    refine: r.refine,
                    contents: tile.contents,
                },
            )?;
        }
        walk.finish().map_err(|e| implicit_error(e.widen()))
    }
}
/// Whether a document's tile tree holds a tile declaring an implicit root.
fn holds_implicit_root(root: &Value) -> bool {
    let mut pending = vec![root];
    while let Some(tile) = pending.pop() {
        if declares_implicit(tile) {
            return true;
        }
        if let Some(children) = tile.get("children").and_then(Value::as_array) {
            pending.extend(children);
        }
    }
    false
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
    inspect_facts(request, limits).map(|inspection| inspection.report)
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
    pub json_nodes: u64,
    pub external_resources: u64,
    pub instance_primitives: u64,
    pub max_node_depth: u64,
}
#[cfg(test)]
pub(crate) fn containment_work_facts(
    request: ValidationRequest,
    limits: ValidationLimits,
) -> Result<Vec<UriWork>, Error> {
    let uris = inspect_facts(request, limits)?.uris;
    Ok(uris
        .into_iter()
        .map(|(uri, f)| UriWork {
            uri,
            t: f.t,
            nodes: f.nodes,
            idx: f.idx,
            e: f.e,
            dec: f.per_reference.dec,
            b: f.per_reference.b,
            json_nodes: f.per_reference.json_nodes,
            external_resources: f.per_reference.external_resources,
            instance_primitives: f.per_reference.instance_primitives,
            max_node_depth: f.max_depth,
        })
        .collect())
}
/// The precheck's accumulated work units of one successful inspection
/// (C2-WB §3.1). Test-only: 0 when an exclusion stopped the precheck before
/// its first reference; the units before the violation when it stopped.
#[cfg(test)]
pub(crate) fn containment_work_total(
    request: ValidationRequest,
    limits: ValidationLimits,
) -> Result<u64, Error> {
    inspect_facts(request, limits).map(|inspection| inspection.units)
}

/// The implicit tiles of one successful inspection, in traversal order (RC-16).
/// Test-only parity seam: acceptance compares them with the oracle's facts.
#[cfg(test)]
pub(crate) fn implicit_tile_facts(
    request: ValidationRequest,
    limits: ValidationLimits,
) -> Result<Vec<ImplicitTileFact>, Error> {
    Ok(inspect_facts(request, limits)?.implicit)
}

#[cfg(test)]
thread_local! {
    /// Off selects the reference traversal (EM terms) on this thread.
    static MEMOIZE: std::cell::Cell<bool> = const { std::cell::Cell::new(true) };
}
/// Names shown in an unreferenced-entries message; the rest are counted.
const UNREFERENCED_SHOWN: usize = 16;
/// Closure: every member not reached by a reference, scanned by rank. The message lists
/// the first 16 names in byte order and counts any others (A-C1-3).
fn unreferenced(archive: &mut Archive, used: &Bits) -> Result<(), Error> {
    let mut first = std::collections::BinaryHeap::new();
    let mut count = 0u64;
    for rank in 0..archive.len() {
        if used.get(rank) {
            continue;
        }
        count += 1;
        let name = archive.name(rank).map_err(archive_admission_error)?;
        if first.len() < UNREFERENCED_SHOWN {
            first.push(name);
        } else if first.peek().is_some_and(|last| name < *last) {
            first.pop();
            first.push(name);
        }
    }
    if count == 0 {
        return Ok(());
    }
    let shown = first.into_sorted_vec().join(", ");
    let rest = count - UNREFERENCED_SHOWN.min(count as usize) as u64;
    Err(invalid(if rest == 0 {
        format!("unreferenced archive entries: {shown}")
    } else {
        format!("unreferenced archive entries: {shown} and {rest} more")
    }))
}
/// One successful inspection: the report and its test-only facts.
struct Inspection {
    report: ValidationReport,
    #[cfg(test)]
    uris: Vec<(String, UriFacts)>,
    #[cfg(test)]
    units: u64,
    #[cfg(test)]
    prepared: u64,
    /// `task_visits`, `reference_visits` and `bytes_read` at the end.
    #[cfg(test)]
    counters: [u64; 3],
    #[cfg(test)]
    implicit: Vec<ImplicitTileFact>,
}

fn inspect_facts(
    request: ValidationRequest,
    limits: ValidationLimits,
) -> Result<Inspection, Error> {
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
    // The hash pass stays in central-directory order, so its first failure is unchanged.
    // Only the `vector-build.json` digest is kept.
    let mut vector_build = None;
    let mut directory = archive.directory().map_err(archive_admission_error)?;
    while let Some(member) = directory.next_entry().map_err(spill_failure)? {
        let name = member.name;
        if name.split('/').any(|v| matches!(v, "" | "." | ".."))
            || name.starts_with('/')
            || name.contains('\\')
        {
            return Err(invalid(format!("unsafe archive path: {name}")));
        }
        bytes_read = bytes_read
            .checked_add(member.size)
            .filter(|&n| n <= limits.total_bytes_read)
            .ok_or_else(|| Error::limit("archive hash reads exceed work limit"))?;
        let digest = archive
            .hash_range(member.size, member.data_offset, member.crc32)
            .map_err(archive_admission_error)?;
        let hash = format!("{:x}", sha2::digest::Output::<sha2::Sha256>::from(digest));
        let stem = Path::new(&name)
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("");
        if stem.len() == 64 && stem.bytes().all(|b| b.is_ascii_hexdigit()) && stem != hash {
            return Err(invalid(format!("content checksum mismatch: {name}")));
        }
        if name == "vector-build.json" {
            vector_build = Some(hash);
        }
    }
    drop(directory);
    let conversion = archive
        .find("conversion.json")
        .map_err(archive_admission_error)?;
    let report = if conversion.is_some() {
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
    let entries = archive.len();
    let mut used = Bits::new(entries)?;
    used.set(
        archive
            .member(TZ_INDEX_NAME)
            .map_err(archive_admission_error)?
            .rank,
    );
    let scratch = archive.scratch();
    let names = archive.names();
    let config = *archive.config();
    let mut check = Check {
        implicit: ImplicitLimits::new(limits.document_decoded_bytes, content_limits.json),
        implicit_roots: 0,
        schemas: HashMap::new(),
        #[cfg(test)]
        implicit_facts: Vec::new(),
        memo: HashMap::new(),
        recording: Vec::new(),
        once_references: 0,
        once_bytes: 0,
        #[cfg(test)]
        memoize: MEMOIZE.with(std::cell::Cell::get),
        #[cfg(test)]
        prepared: 0,
        resources: ArchiveResources {
            archive: &mut archive,
            used,
            limits,
            content_limits,
            bytes_read,
            reference_visits: 0,
        },
        reports: report,
        tiles: 0,
        contents: 0,
        links: 0,
        active: HashSet::new(),
        task_visits: 0,
        elements: 0,
        rows: rows::RowSink::new(&scratch, &config)?,
        payload_counts: Vec::new(),
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
    if let Some(member) = conversion {
        check.resources.used.set(member.rank);
        if let Some(name) = check.reports["geometryReports"].as_str() {
            check.resources.reference(uri("conversion.json", name)?)?;
        }
        // The convert-to-implicit producer was removed in 0.4.0. Archives it
        // already wrote reference their retained original payloads only here,
        // so validation keeps reading this marker and checking those payloads.
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
        if vector_build.as_deref() != Some(expected) {
            return Err(invalid("vector build-state checksum mismatch"));
        }
    }
    check.tileset("tileset.json", None, None, 0)?;
    if let Some(limit) = check.reports["budgets"]["tiles"].as_u64() {
        // Amendment D-1: link tiles count in the report, not in the budget.
        if (check.tiles - check.links) as u64 > limit {
            return Err(invalid("hierarchy exceeds recorded tile budget"));
        }
    }
    unreferenced(check.resources.archive, &check.resources.used)?;
    // C2 outcome precedence after C1 success: exclusions, then the precheck,
    // then streaming (Outside or allocation refusal), otherwise Contained.
    let reasons = if check.c2.exclusions != 0 {
        check.c2.exclusions
    } else {
        check.c2.precheck.map_or(0, Reason::bit)
    };
    let contained = reasons == 0;
    // OD-4 (RC-13): per-tile `extras` are not compared below any implicit root.
    if check.implicit_roots > 0 {
        check
            .not_inspected
            .insert("recordedBudgets.implicitTileExtras".into());
    }
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
    let implicit = check.implicit_roots > 0;
    // The walk's working storage is released before the report rows are ordered: the
    // sorts and the walk are never live together (design §1.4).
    check.memo = HashMap::new();
    check.schemas = HashMap::new();
    check.payload_counts = Vec::new();
    check.recording = Vec::new();
    check.c2.tiles = Vec::new();
    check.c2.references = Vec::new();
    check.c2.content_volumes = Vec::new();
    check.c2.uri_index = HashMap::new();
    check.c2.levels = Vec::new();
    check.c2.arena = None;
    #[cfg(not(test))]
    {
        check.c2.uris = Vec::new();
    }
    let payloads = check.rows.finish(names, scratch.clone())?;
    let report = ValidationReport {
        ok: true,
        archive: path.into(),
        tiles: check.tiles as u64,
        content_references: check.contents as u64,
        entries,
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
        ]
        .into_iter()
        .chain(implicit.then_some("implicitAddressingAndAvailability"))
        .chain([
            "geometricErrorOrder",
            "resourceReferences",
            "recordedBudgets",
        ])
        .chain(contained.then_some("decodedContentContainment"))
        .map(String::from)
        .collect(),
        not_inspected: check.not_inspected.into_iter().collect(),
    };
    Ok(Inspection {
        report,
        #[cfg(test)]
        uris: std::mem::take(&mut check.c2.uris),
        #[cfg(test)]
        units: check.c2.units,
        #[cfg(test)]
        prepared: check.prepared,
        #[cfg(test)]
        counters: [
            check.task_visits,
            check.resources.reference_visits,
            check.resources.bytes_read,
        ],
        #[cfg(test)]
        implicit: check.implicit_facts,
    })
}

/// Small I1 archive builders shared by the in-crate tests.
#[cfg(test)]
mod i1_fixtures {
    use serde_json::{json, Value};
    use std::path::PathBuf;

    pub(super) type Files = std::collections::BTreeMap<String, Vec<u8>>;

    /// A one-point GLB at the origin.
    pub(super) fn point_glb() -> Vec<u8> {
        let doc = json!({
            "asset":{"version":"2.0"},"scene":0,"scenes":[{"nodes":[0]}],"nodes":[{"mesh":0}],
            "buffers":[{"byteLength":12}],"bufferViews":[{"buffer":0,"byteLength":12}],
            "accessors":[{"bufferView":0,"componentType":5126,"type":"VEC3","count":1,
                "min":[0,0,0],"max":[0,0,0]}],
            "meshes":[{"primitives":[{"attributes":{"POSITION":0},"mode":0}]}]
        });
        crate::glb::encode_glb(&doc, &[0; 12]).unwrap()
    }
    pub(super) fn bits(count: usize, set: &[usize]) -> Vec<u8> {
        let mut out = vec![0u8; count.div_ceil(8)];
        for &i in set {
            out[i / 8] |= 1 << (i % 8);
        }
        out
    }
    /// A binary subtree: JSON plus one internal buffer of 8-byte aligned views.
    #[derive(Clone)]
    pub(super) struct Sub {
        pub(super) doc: Value,
        bin: Vec<u8>,
    }
    impl Sub {
        pub(super) fn new(tile: Value, child: Value) -> Self {
            Self {
                doc: json!({"tileAvailability":tile,"childSubtreeAvailability":child}),
                bin: Vec::new(),
            }
        }
        pub(super) fn set(mut self, key: &str, value: Value) -> Self {
            self.doc[key] = value;
            self
        }
        pub(super) fn view(&mut self, bytes: &[u8]) -> usize {
            self.bin.resize(self.bin.len().next_multiple_of(8), 0);
            let offset = self.bin.len();
            self.bin.extend_from_slice(bytes);
            if self.doc.get("bufferViews").is_none() {
                self.doc["bufferViews"] = json!([]);
            }
            let views = self.doc["bufferViews"].as_array_mut().unwrap();
            views.push(json!({"buffer":0,"byteOffset":offset,"byteLength":bytes.len()}));
            views.len() - 1
        }
        pub(super) fn bitstream(&mut self, count: usize, set: &[usize]) -> Value {
            let view = self.view(&bits(count, set));
            json!({"bitstream":view,"availableCount":set.len()})
        }
        pub(super) fn floats(&mut self, values: &[f64], f32s: bool) -> usize {
            let bytes: Vec<u8> = values
                .iter()
                .flat_map(|v| {
                    if f32s {
                        (*v as f32).to_le_bytes().to_vec()
                    } else {
                        v.to_le_bytes().to_vec()
                    }
                })
                .collect();
            self.view(&bytes)
        }
        pub(super) fn build(&self) -> Vec<u8> {
            let mut doc = self.doc.clone();
            if !self.bin.is_empty() {
                doc["buffers"] = json!([{"byteLength":self.bin.len()}]);
            }
            let mut json = serde_json::to_vec(&doc).unwrap();
            json.resize(json.len().next_multiple_of(8), b' ');
            let mut bin = self.bin.clone();
            bin.resize(bin.len().next_multiple_of(8), 0);
            let mut out = b"subt".to_vec();
            out.extend(1u32.to_le_bytes());
            out.extend((json.len() as u64).to_le_bytes());
            out.extend((bin.len() as u64).to_le_bytes());
            out.extend(json);
            out.extend(bin);
            out
        }
    }
    pub(super) fn constant(value: u8) -> Value {
        json!({"constant":value})
    }
    pub(super) fn half(h: f64) -> Value {
        json!([0, 0, 0, h, 0, 0, 0, h, 0, 0, 0, h])
    }
    /// An implicit root R: QUADTREE, box half 8, error 16, the given templates.
    pub(super) fn implicit_root(levels: u32, available: u32, contents: &[&str]) -> Value {
        let mut root = json!({
            "boundingVolume":{"box":half(8.)},"geometricError":16,
            "implicitTiling":{"subdivisionScheme":"QUADTREE","subtreeLevels":levels,
                "availableLevels":available,"subtrees":{"uri":"s/{level}/{x}/{y}.subtree"}}
        });
        match contents {
            [] => {}
            [one] => root["content"] = json!({"uri":one}),
            many => root["contents"] = many.iter().map(|uri| json!({"uri":uri})).collect(),
        }
        root
    }
    pub(super) fn tileset(root: Value) -> Vec<u8> {
        serde_json::to_vec(&json!({"asset":{"version":"1.1"},"geometricError":32,"root":root}))
            .unwrap()
    }
    pub(super) fn package(files: &Files) -> (tempfile::TempDir, PathBuf) {
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
                    implicit_error(ImplicitError::Resolver(Error::Io(io)))
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

    // ---- I1 parity seam (RC-16) and resource controls ----
    use super::i1_fixtures::{
        bits, constant, half, implicit_root, package, point_glb, tileset as i1_tileset, Files, Sub,
    };
    use crate::content_integrity::implicit::{Cell, Scheme};

    fn i1_facts(files: &Files) -> (ValidationReport, Vec<ImplicitTileFact>) {
        let (_work, path) = package(files);
        let report = inspect(ValidationRequest::new(&path)).unwrap();
        let facts = implicit_tile_facts(ValidationRequest::new(&path), ValidationLimits::default())
            .unwrap();
        (report, facts)
    }
    fn i1_run(files: &Files, limits: ValidationLimits) -> Result<ValidationReport, Error> {
        let (_work, path) = package(files);
        inspect_selected(ValidationRequest::new(&path), limits)
    }

    /// A non-root R (X-8) with an inherited `refine`, a bitstream root subtree
    /// with error-only rows, and one FLOAT32 box-only child subtree.
    fn i1_walk_archive() -> Files {
        let mut root = Sub::new(Value::Null, Value::Null);
        let tiles = root.bitstream(5, &[0, 1, 4]);
        let contents = root.bitstream(5, &[1, 4]);
        let children = root.bitstream(16, &[15]);
        let errors = root.floats(&[16., 8., 7.], false);
        let root = root
            .set("tileAvailability", tiles)
            .set("contentAvailability", json!([contents]))
            .set("childSubtreeAvailability", children)
            .set(
                "propertyTables",
                json!([{"class":"t","count":3,"properties":{"err":{"values":errors}}}]),
            )
            .set("tileMetadata", json!(0));
        let mut child = Sub::new(Value::Null, constant(0));
        let tiles = child.bitstream(5, &[0]);
        let contents = child.bitstream(5, &[0]);
        let volume = child.floats(&[6., 6., 0., 2., 0., 0., 0., 2., 0., 0., 0., 8.], true);
        let child = child
            .set("tileAvailability", tiles)
            .set("contentAvailability", json!([contents]))
            .set(
                "propertyTables",
                json!([{"class":"u","count":1,"properties":{"vol":{"values":volume}}}]),
            )
            .set("tileMetadata", json!(0));
        let r = implicit_root(2, 3, &["c/{level}/{x}/{y}.glb"]);
        let doc = json!({"asset":{"version":"1.1"},"geometricError":32,
            "schema":{"id":"s","classes":{
                "t":{"properties":{"err":{"type":"SCALAR","componentType":"FLOAT64",
                    "semantic":"TILE_GEOMETRIC_ERROR"}}},
                "u":{"properties":{"vol":{"type":"SCALAR","componentType":"FLOAT32",
                    "array":true,"count":12,"semantic":"TILE_BOUNDING_BOX"}}}}},
            "root":{"boundingVolume":{"box":half(16.)},"geometricError":32,"refine":"ADD",
                "children":[r]}});
        let mut files = Files::new();
        files.insert("tileset.json".into(), serde_json::to_vec(&doc).unwrap());
        files.insert("s/0/0/0.subtree".into(), root.build());
        files.insert("s/2/3/3.subtree".into(), child.build());
        for name in ["1/0/0", "1/1/1", "2/3/3"] {
            files.insert(format!("c/{name}.glb"), point_glb());
        }
        files
    }

    #[test]
    fn i1_facts_follow_the_walk_exactly() {
        let (report, facts) = i1_facts(&i1_walk_archive());
        let root_box = [0., 0., 0., 8., 0., 0., 0., 8., 0., 0., 0., 8.];
        let cell = |level, x, y| {
            EffectiveVolume::Derived(Cell {
                root: root_box,
                level,
                coords: [x, y, 0],
                scheme: Scheme::Quadtree,
            })
        };
        let fact = |level: u32, xy: [u32; 2], volume, error, contents: &[&str]| ImplicitTileFact {
            doc: "tileset.json".into(),
            level,
            coords: [xy[0], xy[1], 0],
            depth: 1 + level as usize,
            parent_level: level.checked_sub(1),
            volume,
            error,
            refine: Some("ADD"),
            contents: contents.iter().map(|c| (0, c.to_string())).collect(),
        };
        assert_eq!(
            facts,
            [
                fact(
                    0,
                    [0, 0],
                    EffectiveVolume::DerivedRoot,
                    EffectiveError::Row(16.),
                    &[]
                ),
                fact(
                    1,
                    [0, 0],
                    cell(1, 0, 0),
                    EffectiveError::Row(8.),
                    &["c/1/0/0.glb"]
                ),
                fact(
                    1,
                    [1, 1],
                    cell(1, 1, 1),
                    EffectiveError::Row(7.),
                    &["c/1/1/1.glb"]
                ),
                fact(
                    2,
                    [3, 3],
                    EffectiveVolume::Row([6., 6., 0., 2., 0., 0., 0., 2., 0., 0., 0., 8.]),
                    EffectiveError::Derived(4.),
                    &["c/2/3/3.glb"]
                ),
            ]
        );
        // R and I_0 count once; one explicit root above R.
        assert_eq!((report.tiles, report.content_references), (5, 3));
        assert!(report
            .checks
            .contains(&"implicitAddressingAndAvailability".to_owned()));
    }

    #[test]
    fn i1_facts_match_the_retained_expander_on_writer_output() {
        // An explicit hierarchy encoded by the retained writer (rows on every
        // tile, payload templates), read by I1 and by `expand_tileset`.
        let work = tempfile::tempdir().unwrap();
        let data = work.path().join("data");
        std::fs::create_dir_all(&data).unwrap();
        let tile = |center: [f64; 2], h: f64, error: f64, uri: &str| {
            json!({"boundingVolume":{"box":[center[0],center[1],0,h,0,0,0,h,0,0,0,8]},
                "geometricError":error,"content":{"uri":uri}})
        };
        let mut children = Vec::new();
        for slot in 0..4 {
            let x = -4. + 8. * f64::from(slot & 1);
            let y = -4. + 8. * f64::from(slot >> 1);
            let uri = format!("b{slot}.glb");
            std::fs::write(data.join(&uri), point_glb()).unwrap();
            children.push(tile([x, y], 4., 8., &uri));
        }
        std::fs::write(data.join("g.glb"), point_glb()).unwrap();
        std::fs::write(data.join("a.glb"), point_glb()).unwrap();
        children[3]["children"] = json!([tile([2., 2.], 2., 4., "g.glb")]);
        let mut root = tile([0., 0.], 8., 16., "a.glb");
        root["refine"] = json!("REPLACE");
        root["children"] = json!(children);
        let mut manifest = json!({"asset":{"version":"1.1"},"geometricError":16,"root":root});
        crate::implicit::write_forest(
            &mut manifest,
            &data,
            crate::implicit::SubdivisionScheme::Quadtree,
            &mut || Ok(()),
        )
        .unwrap();
        std::fs::write(
            data.join("tileset.json"),
            serde_json::to_vec(&manifest).unwrap(),
        )
        .unwrap();
        let expanded =
            crate::implicit::expand_tileset(&manifest, |name| Ok(std::fs::read(data.join(name))?))
                .unwrap();
        let mut expected = Vec::new();
        let mut pending = vec![&expanded["root"]];
        while let Some(node) = pending.pop() {
            let volume: Vec<f64> =
                serde_json::from_value(node["boundingVolume"]["box"].clone()).unwrap();
            let uris: Vec<String> = node["content"]["uri"]
                .as_str()
                .map(String::from)
                .into_iter()
                .collect();
            expected.push((volume, node["geometricError"].as_f64().unwrap(), uris));
            if let Some(children) = node["children"].as_array() {
                pending.extend(children.iter().rev());
            }
        }
        let output = work.path().join("writer.3tz");
        crate::package::package(
            crate::package::PackageRequest::directory(&data, &output),
            &crate::RunControl::default(),
        )
        .unwrap();
        let facts =
            implicit_tile_facts(ValidationRequest::new(&output), ValidationLimits::default())
                .unwrap();
        let actual: Vec<_> = facts
            .iter()
            .map(|fact| {
                let EffectiveVolume::Row(volume) = &fact.volume else {
                    panic!("the writer records a row on every tile: {fact:?}");
                };
                let EffectiveError::Row(error) = fact.error else {
                    panic!("the writer records a row on every tile: {fact:?}");
                };
                let uris = fact.contents.iter().map(|(_, uri)| uri.clone()).collect();
                (volume.to_vec(), error, uris)
            })
            .collect();
        assert_eq!(actual.len(), 6);
        assert_eq!(actual, expected);
        let report = inspect(ValidationRequest::new(&output)).unwrap();
        assert_eq!((report.tiles, report.content_references), (6, 6));
    }

    /// The base archive: QUADTREE L = 2, A = 2, five tiles, five payloads.
    fn i1_base() -> Files {
        let mut files = Files::new();
        files.insert(
            "tileset.json".into(),
            i1_tileset(implicit_root(2, 2, &["c/{level}/{x}/{y}.glb"])),
        );
        files.insert(
            "s/0/0/0.subtree".into(),
            Sub::new(constant(1), constant(0))
                .set("contentAvailability", json!([constant(1)]))
                .build(),
        );
        for name in ["0/0/0", "1/0/0", "1/1/0", "1/0/1", "1/1/1"] {
            files.insert(format!("c/{name}.glb"), point_glb());
        }
        files
    }

    #[test]
    fn i1_visits_follow_the_reservation_formula() {
        // RC-7: Tileset + ExitTileset + R + P_t (5) = 8 tasks.
        let limits = |visits| ValidationLimits {
            hierarchy_visits: visits,
            ..ValidationLimits::default()
        };
        assert!(i1_run(&i1_base(), limits(8)).is_ok());
        let failure = i1_run(&i1_base(), limits(7)).unwrap_err();
        assert_eq!(failure.category().0, "resource_limit", "{failure}");
        // The reservation refuses the whole subtree before its first visit.
        let failure = i1_run(&i1_base(), limits(6)).unwrap_err();
        assert!(
            failure
                .to_string()
                .contains("exceeds remaining hierarchy visits"),
            "{failure}"
        );
    }

    #[test]
    fn i1_depth_is_r_depth_plus_level() {
        let limits = |depth| ValidationLimits {
            hierarchy_depth: depth,
            ..ValidationLimits::default()
        };
        assert!(i1_run(&i1_base(), limits(1)).is_ok());
        let failure = i1_run(&i1_base(), limits(0)).unwrap_err();
        assert!(failure.to_string().contains("depth limit"), "{failure}");
    }

    #[test]
    fn i1_live_path_cap_admits_the_path_sum_only() {
        // R2 in miniature: QUADTREE L = 1, A = 3, a chain of three payload-free
        // subtrees plus a sibling leaf at level 2. The cap is the deepest live
        // path, well below the cumulative sum.
        let leaf = Sub::new(constant(1), constant(0)).build();
        let mut files = Files::new();
        files.insert("tileset.json".into(), i1_tileset(implicit_root(1, 3, &[])));
        let mut top = Sub::new(constant(1), Value::Null);
        let children = top.bitstream(4, &[0]);
        let top = top.set("childSubtreeAvailability", children).build();
        let mut middle = Sub::new(constant(1), Value::Null);
        let children = middle.bitstream(4, &[0, 3]);
        let middle = middle.set("childSubtreeAvailability", children).build();
        let path = (top.len() + middle.len() + leaf.len()) as u64;
        files.insert("s/0/0/0.subtree".into(), top);
        files.insert("s/1/0/0.subtree".into(), middle);
        files.insert("s/2/0/0.subtree".into(), leaf.clone());
        files.insert("s/2/1/1.subtree".into(), leaf);
        let limits = |cap| ValidationLimits {
            document_decoded_bytes: cap,
            ..ValidationLimits::default()
        };
        let report = i1_run(&files, limits(path)).unwrap();
        assert_eq!(report.tiles, 4);
        let failure = i1_run(&files, limits(path - 1)).unwrap_err();
        assert!(
            failure
                .to_string()
                .starts_with("member exceeds admitted byte limit: s/2/0/0.subtree"),
            "{failure}"
        );
    }

    /// The smallest `limit` value admitting `files`, by bisection.
    fn threshold(files: &Files, set: impl Fn(u64) -> ValidationLimits, high: u64) -> u64 {
        let (_work, path) = package(files);
        let run = |value| inspect_selected(ValidationRequest::new(&path), set(value)).is_ok();
        assert!(run(high));
        let (mut low, mut high) = (0, high);
        while high - low > 1 {
            let middle = low + (high - low) / 2;
            if run(middle) {
                high = middle;
            } else {
                low = middle;
            }
        }
        high
    }

    #[test]
    fn i1_availability_scan_is_charged_to_total_bytes_read() {
        // X-12: a payload-free root whose last charge is its tile view's scan.
        let mut sub = Sub::new(Value::Null, constant(0));
        let tiles = sub.bitstream(87_381, &[0]);
        let sub = sub.set("tileAvailability", tiles);
        let mut files = Files::new();
        files.insert("tileset.json".into(), i1_tileset(implicit_root(9, 9, &[])));
        files.insert("s/0/0/0.subtree".into(), sub.build());
        let set = |total| ValidationLimits {
            total_bytes_read: total,
            ..ValidationLimits::default()
        };
        let needed = threshold(&files, set, 1 << 24);
        let failure = i1_run(&files, set(needed - 1)).unwrap_err();
        assert_eq!(failure.category().0, "resource_limit");
        assert!(
            failure
                .to_string()
                .contains("availability scan exceeds total_bytes_read work limit"),
            "{failure}"
        );
        // The scan is ⌈T/8⌉ = 10,923 bytes: refused within that window too.
        assert!(i1_run(&files, set(needed - 10_923)).is_err());
    }

    #[test]
    fn i1_content_reservation_and_emission_share_references() {
        // DC-3: E is reserved once at admission and emissions are not charged
        // again, so references are exactly the explicit charges plus E.
        let set = |references| ValidationLimits {
            references,
            ..ValidationLimits::default()
        };
        let needed = threshold(&i1_base(), set, 1 << 10);
        // tileset.json + the subtree member + E = 5 emitted contents.
        assert_eq!(needed, 1 + 1 + 5);
        let failure = i1_run(&i1_base(), set(needed - 1)).unwrap_err();
        assert!(
            failure
                .to_string()
                .contains("subtree content entries exceed remaining references"),
            "{failure}"
        );
    }

    #[test]
    fn i1_resolver_reads_external_buffers_relative_to_the_subtree() {
        // An external buffer is one more reference and its bytes; a missing
        // one is the resolver's InvalidInput.
        let mut files = Files::new();
        files.insert("tileset.json".into(), i1_tileset(implicit_root(2, 2, &[])));
        let mut sub = Sub::new(Value::Null, constant(0));
        sub.doc["buffers"] = json!([{"uri":"../avail.bin","byteLength":1}]);
        sub.doc["bufferViews"] = json!([{"buffer":0,"byteOffset":0,"byteLength":1}]);
        sub.doc["tileAvailability"] = json!({"bitstream":0,"availableCount":2});
        files.insert("s/0/0/0.subtree".into(), sub.build());
        let mut avail = bits(5, &[0, 1]);
        avail.resize(8, 0);
        files.insert("s/0/avail.bin".into(), avail);
        assert_eq!(
            i1_run(&files, ValidationLimits::default()).unwrap().tiles,
            2
        );
        files.remove("s/0/avail.bin");
        let failure = i1_run(&files, ValidationLimits::default()).unwrap_err();
        assert_eq!(failure.to_string(), "missing archive entry: s/0/avail.bin");
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
    pub(super) fn points(positions: &[[f32; 3]]) -> Vec<u8> {
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

    /// H11: declared accessor `min`/`max` never decide containment. The fast path reads the drawn
    /// words; a lying declaration over viewed storage is a C1 refusal, and over viewless (zero)
    /// storage, where C1 allows any declaration, the zero words are still tested.
    #[test]
    fn declared_accessor_bounds_cannot_hide_an_escape() {
        // 600 drawn vertices: enough corner credit (T/64 >= 2) for the fast path to run.
        let mut positions: Vec<[f32; 3]> = (0..600)
            .map(|i| [(i % 7) as f32 * 0.125, 0.0, (i % 5) as f32 * -0.125])
            .collect();
        positions[555] = [3.0, 0.0, 0.0];
        let volume = boxed([0.5, 0.5, 0.0], [1.0, 1.0, 1.0]);
        let (doc, bin) = points_doc(&positions, json!([{"mesh":0}]));
        let message = "decoded content outside bounding volume: content=a.glb depth=0 volume=own node=0 mesh=0 primitive=0 vertex=555";
        assert_eq!(
            outside(run(&single(volume.clone(), glb(&doc, &bin)))),
            message
        );
        // Declaring the escaping maximum away is refused before C2 runs.
        let mut lying = doc.clone();
        lying["accessors"][0]["max"] = json!([0.75, 0.0, 0.0]);
        match run(&single(volume.clone(), glb(&lying, &bin))) {
            Err(Error::InvalidInput(m)) => {
                assert!(m.contains("declared accessor bounds differ"), "{m}")
            }
            other => panic!("expected the C1 bounds refusal, got {other:?}"),
        }
        // Viewless storage: the declaration says [0.5, 0.75]^3, the words are zero, and the box
        // holds the declaration but not the origin.
        let viewless = json!({
            "asset":{"version":"2.0"},"scene":0,"scenes":[{"nodes":[0]}],"nodes":[{"mesh":0}],
            "accessors":[{"componentType":5126,"type":"VEC3","count":600,
                "min":[0.5,0.5,0.5],"max":[0.75,0.75,0.75]}],
            "meshes":[{"primitives":[{"attributes":{"POSITION":0},"mode":0}]}]
        });
        let payload = glb(&viewless, &[]);
        assert_eq!(
            outside(run(&single(boxed([0.625, -0.625, 0.625], [0.125; 3]), payload.clone()))),
            "decoded content outside bounding volume: content=a.glb depth=0 volume=own node=0 mesh=0 primitive=0 vertex=0"
        );
        assert!(contained(
            &run(&single(boxed([0.5, -0.5, 0.5], [0.75; 3]), payload)).unwrap()
        ));
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

    /// The H7 Unsupported message of a legacy verdict, or a panic.
    fn legacy_verdict(result: Result<ValidationReport, Error>) -> String {
        match result {
            Err(Error::Unsupported(message)) if message.starts_with("legacy ") => message,
            other => panic!("expected a legacy Unsupported verdict, got {other:?}"),
        }
    }

    #[test]
    fn legacy_content_is_unsupported_by_suffix_or_magic_and_never_inspected() {
        let glb = points(&[[0., 0., 0.]]);
        fn files(name: &str, bytes: Vec<u8>) -> Vec<(&str, Vec<u8>)> {
            vec![
                ("tileset.json", tileset(content_tile(wide(), name))),
                (name, bytes),
            ]
        }
        let after_magic = |format: &str, rest: &[u8]| [format.as_bytes(), rest].concat();
        for format in ["b3dm", "i3dm", "pnts", "cmpt"] {
            // L1: the suffix decides; nothing after the name is looked at, so a
            // GLB, garbage, an empty member and a valid-looking header agree.
            for name in [
                format!("a.{format}"),
                format!("a.{}", format.to_uppercase()),
            ] {
                for bytes in [glb.clone(), Vec::new(), after_magic(format, &[0xff; 24])] {
                    let message = legacy_verdict(run(&files(&name, bytes)));
                    assert_eq!(
                        message,
                        format!(
                            "legacy {format} content is unsupported (glTF 2.0 content only): {name}"
                        )
                    );
                }
            }
            // L2: the first four bytes decide under any other suffix (magic wins).
            for name in ["a.glb", "a.gltf", "a.bin", "a"] {
                for rest in [&[][..], &glb[4..], &[0xff; 3][..]] {
                    let message = legacy_verdict(run(&files(name, after_magic(format, rest))));
                    assert_eq!(
                        message,
                        format!(
                            "legacy {format} content is unsupported (glTF 2.0 content only): \
                             {name} starts with {format} magic"
                        )
                    );
                }
            }
        }
        // Not legacy: a non-final legacy suffix, magic off offset 0, short members.
        assert!(contained(
            &run(&files("b3dm.cmpt/x.glb", glb.clone())).unwrap()
        ));
        for (name, bytes) in [
            ("a.bin", glb.clone()),
            ("a.vctr", b"vctr".to_vec()),
            ("a.bin", b"b3d".to_vec()),
            ("a.bin", b" b3dm".to_vec()),
        ] {
            assert!(
                matches!(run(&files(name, bytes)), Err(Error::Unsupported(m))
                    if m == "payload format outside GLB/local glTF 2.0"),
                "{name}"
            );
        }
        assert!(matches!(
            run(&files("a.glb", b"B3DM".to_vec())),
            Err(Error::InvalidInput(_))
        ));
        // L0: a missing legacy member stays InvalidInput.
        assert!(matches!(
            run(&[("tileset.json", tileset(content_tile(wide(), "a.b3dm")))]),
            Err(Error::InvalidInput(m)) if m.contains("a.b3dm")
        ));
    }

    /// R-1: a legacy suffix is refused before the payload read. The GLB twin has
    /// the same names' lengths and member sizes, so its smallest admitting
    /// `total_bytes_read` minus the payload size is exactly the legacy archive's
    /// need; a classifier placed after the read would breach it.
    #[test]
    fn legacy_suffix_is_refused_before_the_payload_read() {
        let glb = points(&[[0., 0., 0.]]);
        let twin = vec![
            ("tileset.json", tileset(content_tile(wide(), "ab.glb"))),
            ("ab.glb", glb.clone()),
        ];
        let legacy_files = vec![
            ("tileset.json", tileset(content_tile(wide(), "a.b3dm"))),
            ("a.b3dm", [&b"b3dm"[..], &glb[4..]].concat()),
        ];
        let limited = |bytes| ValidationLimits {
            total_bytes_read: bytes,
            ..ValidationLimits::default()
        };
        let (mut low, mut high) = (0u64, 1 << 20);
        while low < high {
            let middle = (low + high) / 2;
            if run_with(&twin, limited(middle)).is_ok() {
                high = middle;
            } else {
                low = middle + 1;
            }
        }
        let need = low - glb.len() as u64;
        legacy_verdict(run_with(&legacy_files, limited(need)));
        assert!(matches!(
            run_with(&legacy_files, limited(need - 1)),
            Err(Error::ResourceLimit(_))
        ));
    }

    // ---- Slice 3: implicit archives (contract §4) ----
    use super::i1_fixtures::{constant, implicit_root, tileset as i1_tileset, Sub};

    /// One point at the origin of a node translated by `x` along +X (U leaves
    /// the X axis unchanged), so the drawn vertex is exactly `(x, 0, 0)`.
    fn point_at(x: f64) -> Vec<u8> {
        let (doc, bin) = points_doc(&[[0., 0., 0.]], json!([{"mesh":0,"translation":[x,0,0]}]));
        glb(&doc, &bin)
    }
    /// A box centred at the origin with half-lengths `(hx, 8, 8)`.
    fn row(hx: f64) -> [f64; 12] {
        [0., 0., 0., hx, 0., 0., 0., 8., 0., 0., 0., 8.]
    }
    /// A FLOAT64 TILE_BOUNDING_BOX schema for subtree rows (class `b`).
    fn row_schema() -> Value {
        json!({"id":"s","classes":{
            "b":{"properties":{"vol":{"type":"SCALAR","componentType":"FLOAT64",
                "array":true,"count":12,"semantic":"TILE_BOUNDING_BOX"}}},
            "label":{"properties":{"name":{"type":"STRING"}}}}})
    }
    /// The one subtree of a QUADTREE root (L = A = 2): available tiles and
    /// contents by subtree bit index (0 is I_0, 1 + Morton at level 1), and one
    /// FLOAT64 row per available tile when `rows` is given.
    fn subtree(tiles: &[usize], contents: &[usize], rows: Option<&[[f64; 12]]>) -> Vec<u8> {
        let mut sub = Sub::new(Value::Null, constant(0));
        let t = sub.bitstream(5, tiles);
        let c = sub.bitstream(5, contents);
        let mut sub = sub
            .set("tileAvailability", t)
            .set("contentAvailability", json!([c]));
        if let Some(rows) = rows {
            let flat: Vec<f64> = rows.iter().flatten().copied().collect();
            let values = sub.floats(&flat, false);
            sub = sub
                .set(
                    "propertyTables",
                    json!([{"class":"b","count":rows.len(),"properties":{"vol":{"values":values}}}]),
                )
                .set("tileMetadata", json!(0));
        }
        sub.build()
    }
    /// A tileset document with the row schema.
    fn implicit_doc(root: Value) -> Vec<u8> {
        serde_json::to_vec(&json!({"asset":{"version":"1.1"},"geometricError":32,
            "schema":row_schema(),"root":root}))
        .unwrap()
    }
    const NAMES: [&str; 5] = [
        "c/0/0/0.glb",
        "c/1/0/0.glb",
        "c/1/1/0.glb",
        "c/1/0/1.glb",
        "c/1/1/1.glb",
    ];
    /// An archive of `root` (holding R with template `c/{level}/{x}/{y}.glb`),
    /// its subtree, and one payload per emitted content.
    fn implicit_files(
        root: Value,
        sub: Vec<u8>,
        payloads: &[(usize, Vec<u8>)],
    ) -> Vec<(&'static str, Vec<u8>)> {
        let mut files = vec![
            ("tileset.json", implicit_doc(root)),
            ("s/0/0/0.subtree", sub),
        ];
        files.extend(payloads.iter().map(|(i, p)| (NAMES[*i], p.clone())));
        files
    }
    fn r() -> Value {
        implicit_root(2, 2, &["c/{level}/{x}/{y}.glb"])
    }

    #[test]
    fn implicit_archives_are_inspected() {
        // §4 step 1: no `implicitTiling` exclusion. Payload-free roots, on the
        // tileset root and below it, are Contained with the I1 check listed.
        let subtree = Sub::new(constant(1), constant(0)).build();
        for root in [
            implicit_root(1, 1, &[]),
            json!({"boundingVolume":{"box":[0,0,0,8,0,0,0,8,0,0,0,8]},"geometricError":16,
                "refine":"ADD","children":[implicit_root(1, 1, &[])]}),
        ] {
            let report = run(&[
                ("tileset.json", i1_tileset(root)),
                ("s/0/0/0.subtree", subtree.clone()),
            ])
            .unwrap();
            assert!(report
                .checks
                .contains(&"implicitAddressingAndAvailability".to_owned()));
            assert!(contained(&report), "{report:?}");
            assert!(!report
                .not_inspected
                .iter()
                .any(|c| c.contains("implicitTiling")));
        }
        // An implicit content's payload reasons come from its per-URI facts.
        let subtree = Sub::new(constant(1), constant(0))
            .set("contentAvailability", json!([constant(1)]))
            .build();
        let files = |payload: Vec<u8>| {
            vec![
                (
                    "tileset.json",
                    i1_tileset(implicit_root(1, 1, &["c/{level}/{x}/{y}.glb"])),
                ),
                ("s/0/0/0.subtree", subtree.clone()),
                ("c/0/0/0.glb", payload),
            ]
        };
        assert_eq!(
            not_inspected(run(&files(with_doc(|d| d["nodes"][0]["skin"] = json!(0))))),
            ["skin"]
        );
        // DerivedRoot only: I_0's own volume is exactly V_R (closed).
        assert!(contained(&run(&files(point_at(8.))).unwrap()));
        assert_eq!(
            outside(run(&files(point_at(8f64.next_up())))),
            "decoded content outside bounding volume: content=c/0/0/0.glb depth=0 volume=own node=0 mesh=0 primitive=0 vertex=0"
        );
    }

    #[test]
    fn implicit_chain_tests_own_implicit_ancestors_i0_and_v_r_in_order() {
        // §4 table and I4. R (depth 1, box half 8) under an explicit parent P
        // (box half 8 + 2e-7, within the hierarchyBounds tolerance). Rows widen
        // X by 2e-7 within the same tolerance; the vertex sits at 8 + 1e-7, so
        // each verdict names the first level whose X face is exactly 8.
        let (wide, edge) = (8. + 2e-7, 8. + 1e-7);
        let parent = |hx: f64, r_hx: f64| {
            let mut r = r();
            r["boundingVolume"]["box"][3] = json!(r_hx);
            json!({"boundingVolume":{"box":row(hx).to_vec()},"geometricError":32,
                "refine":"ADD","children":[r]})
        };
        let level_one = |p_hx, r_hx, rows: [f64; 2]| {
            implicit_files(
                parent(p_hx, r_hx),
                subtree(&[0, 1], &[1], Some(&[row(rows[0]), row(rows[1])])),
                &[(1, point_at(edge))],
            )
        };
        let message = |volume: &str| {
            format!("decoded content outside bounding volume: content=c/1/0/0.glb depth=2 volume={volume} node=0 mesh=0 primitive=0 vertex=0")
        };
        // Level l = 1: own, ancestor:1 = I_0, ancestor:2 = R (V_R), ancestor:3 = P.
        for (p_hx, r_hx, rows, volume) in [
            (wide, wide, [wide, 8.], "own"),
            (wide, wide, [8., wide], "ancestor:1"),
            (wide, 8., [wide, wide], "ancestor:2"),
            (8., wide, [wide, wide], "ancestor:3"),
        ] {
            assert_eq!(outside(run(&level_one(p_hx, r_hx, rows))), message(volume));
        }
        assert!(contained(
            &run(&level_one(wide, wide, [wide, wide])).unwrap()
        ));
        // Level l = 0: own = I_0's row, ancestor:1 = V_R.
        let level_zero = |r_hx, row_hx| {
            implicit_files(
                parent(wide, r_hx),
                subtree(&[0], &[0], Some(&[row(row_hx)])),
                &[(0, point_at(edge))],
            )
        };
        let message = |volume: &str| {
            format!("decoded content outside bounding volume: content=c/0/0/0.glb depth=1 volume={volume} node=0 mesh=0 primitive=0 vertex=0")
        };
        assert_eq!(outside(run(&level_zero(wide, 8.))), message("own"));
        assert_eq!(outside(run(&level_zero(8., wide))), message("ancestor:1"));
        assert!(contained(&run(&level_zero(wide, wide)).unwrap()));
    }

    #[test]
    fn implicit_root_transform_steps_out_of_v_r_before_the_parent() {
        // R carries translation (10, 0, 0) under P (centre 10, half 8): V_R is
        // tested in R's frame, then R.transform, then P (§4 table, level l+2).
        let parent = |r_hx: f64| {
            let mut r = r();
            r["boundingVolume"]["box"][3] = json!(r_hx);
            let mut m = IDENTITY;
            m[12] = 10.;
            r["transform"] = json!(m);
            json!({"boundingVolume":{"box":[10,0,0,8,0,0,0,8,0,0,0,8]},"geometricError":32,
                "refine":"ADD","children":[r]})
        };
        let files = |x: f64| {
            implicit_files(
                parent(8. + 2e-7),
                subtree(&[0], &[0], None),
                &[(0, point_at(x))],
            )
        };
        assert!(contained(&run(&files(8.)).unwrap()));
        assert_eq!(
            outside(run(&files(8. + 1e-7))),
            "decoded content outside bounding volume: content=c/0/0/0.glb depth=1 volume=ancestor:2 node=0 mesh=0 primitive=0 vertex=0"
        );
        // Both verdicts need the step: unstepped, 8 + 1e-7 would lie inside P
        // and -7 outside it; stepped, -7 is 3, inside P.
        assert!(contained(&run(&files(-7.)).unwrap()));
    }

    #[test]
    fn external_implicit_root_tests_the_referencing_content_after_r_transform() {
        // tileset.json links ext.json with a content volume centred at x = 5
        // (half 1); ext.json's root is R with translation (5, 0, 0).
        let mut r = implicit_root(1, 1, &["c/{level}/{x}/{y}.glb"]);
        let mut m = IDENTITY;
        m[12] = 5.;
        r["transform"] = json!(m);
        let link = json!({"boundingVolume":{"box":[0,0,0,20,0,0,0,20,0,0,0,20]},
            "geometricError":32,"contents":[{"uri":"ext.json",
                "boundingVolume":{"box":[5,0,0,1,0,0,0,8,0,0,0,8]}}]});
        let subtree = Sub::new(constant(1), constant(0))
            .set("contentAvailability", json!([constant(1)]))
            .build();
        let files = |x: f64| {
            vec![
                ("tileset.json", i1_tileset(link.clone())),
                ("ext.json", i1_tileset(r.clone())),
                ("s/0/0/0.subtree", subtree.clone()),
                ("c/0/0/0.glb", point_at(x)),
            ]
        };
        // 0 in R's frame is 5 in the link's: inside only after R.transform.
        assert!(contained(&run(&files(0.)).unwrap()));
        assert!(contained(&run(&files(1.)).unwrap()));
        assert_eq!(
            outside(run(&files(1f64.next_up()))),
            "decoded content outside bounding volume: content=c/0/0/0.glb depth=1 volume=externalContent:2 node=0 mesh=0 primitive=0 vertex=0"
        );
    }

    #[test]
    fn derived_implicit_volume_is_chain_scoped() {
        // §2, I5: rowless level-1 tiles with no content at or below them do
        // not exclude; a content on one does, through its chain bit.
        let all = [0, 1, 2, 3, 4];
        let origin = point_at(0.);
        assert!(contained(
            &run(&implicit_files(
                r(),
                subtree(&all, &[0], None),
                &[(0, origin.clone())]
            ))
            .unwrap()
        ));
        assert_eq!(
            not_inspected(run(&implicit_files(
                r(),
                subtree(&all, &[0, 3], None),
                &[(0, origin.clone()), (3, origin.clone())]
            ))),
            ["derivedImplicitVolume"]
        );
        // With rows, the same tiles are tested.
        let rows = [row(8.); 5];
        assert!(contained(
            &run(&implicit_files(
                r(),
                subtree(&all, &[0, 3], Some(&rows)),
                &[(0, origin.clone()), (3, origin)]
            ))
            .unwrap()
        ));
    }

    #[test]
    fn derived_implicit_volume_overrides_a_stopped_precheck() {
        // S3-20 in miniature: an explicit reference first stops the precheck
        // at workLimit; a later reference through a rowless tile still lists
        // derivedImplicitVolume, and only it.
        let origin = point_at(0.);
        let files = |contents: &[usize]| {
            let root = json!({"boundingVolume":{"box":[0,0,0,8,0,0,0,8,0,0,0,8]},
                "geometricError":32,"refine":"ADD",
                "children":[content_tile(boxed([0.; 3], [8.; 3]), "a.glb"), r()]});
            let mut files = implicit_files(
                root,
                subtree(&[0, 1], contents, None),
                &contents
                    .iter()
                    .map(|&i| (i, origin.clone()))
                    .collect::<Vec<_>>(),
            );
            files.push(("a.glb", origin.clone()));
            files
        };
        let limits = ValidationLimits {
            containment_work_units: 1,
            ..ValidationLimits::default()
        };
        assert_eq!(
            not_inspected(run_with(&files(&[0, 1]), limits.clone())),
            ["derivedImplicitVolume"]
        );
        assert_eq!(not_inspected(run_with(&files(&[0]), limits)), ["workLimit"]);
        assert_eq!(
            not_inspected(run(&files(&[0, 1]))),
            ["derivedImplicitVolume"]
        );
        assert!(contained(&run(&files(&[0])).unwrap()));
    }

    #[test]
    fn template_header_metadata_is_tile_metadata_and_subtree_tables_are_not() {
        // I6, §4 step 7: a header `metadata` entity is content metadata.
        let mut root = r();
        root["content"]["metadata"] = json!({"class":"label","properties":{"name":"x"}});
        let files = |root: Value| {
            implicit_files(
                root,
                subtree(&[0], &[0], Some(&[row(8.)])),
                &[(0, point_at(0.))],
            )
        };
        assert_eq!(not_inspected(run(&files(root))), ["tileMetadata"]);
        // The subtree's TILE_BOUNDING_BOX table alone excludes nothing.
        assert!(contained(&run(&files(r())).unwrap()));
    }

    #[test]
    fn implicit_references_stream_in_walk_order_at_r_position() {
        // §3 item 4: R's emitted contents are references in I1 walk order, at
        // R's place in the explicit pre-order. Every payload here is Outside.
        let far = point_at(100.);
        let files = |r_first: bool| {
            let explicit = content_tile(boxed([0.; 3], [8.; 3]), "z.glb");
            let children = if r_first {
                json!([r(), explicit])
            } else {
                json!([explicit, r()])
            };
            let root = json!({"boundingVolume":{"box":[0,0,0,8,0,0,0,8,0,0,0,8]},
                "geometricError":32,"refine":"ADD","children":children});
            let mut files = implicit_files(
                root,
                subtree(&[0, 1, 2], &[1, 2], Some(&[row(8.); 3])),
                &[(1, far.clone()), (2, far.clone())],
            );
            files.push(("z.glb", far.clone()));
            files
        };
        assert!(outside(run(&files(true))).contains("content=c/1/0/0.glb depth=2 volume=own "));
        assert!(outside(run(&files(false))).contains("content=z.glb depth=1 volume=own "));
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
        // Malformed node beats an exclusion (here gltfExtension).
        let malformed = with_doc(|d| {
            d["extensionsUsed"] = json!(["EXT_mesh_gpu_instancing"]);
            d["nodes"][0]["matrix"] = json!([1, 0, 0, 0, 0, 1, 0, 0, 0, 0, 1, 0, 0, 0, 0, 1]);
            d["nodes"][0]["scale"] = json!([1, 1, 1]);
        });
        let failure = run(&single(wide(), malformed)).unwrap_err();
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
    fn instances_without_position_bound_spans_and_are_never_streamed() {
        // Review B1: three nested in-window rotations [2^-1000, 0, 0, 1] end in
        // an empty mesh (static sInst 6008 > 4096). Alone it draws nothing and
        // has no Pair0: Contained, with no composite pushed (a push would breach
        // the arena). Beside a real triangle, its sInst joins the URI's
        // span: numericSpan. Expected verdicts are the oracle's.
        let rot = json!([2f64.powi(-1000), 0, 0, 1]);
        let nodes = json!([
            {"rotation":rot,"children":[1]},
            {"rotation":rot,"children":[2]},
            {"rotation":rot,"mesh":0}
        ]);
        let empty = json!({"asset":{"version":"2.0"},"scene":0,"scenes":[{"nodes":[0]}],
            "meshes":[{"primitives":[]}],"nodes":nodes});
        let files = single(boxed([0.; 3], [10.; 3]), glb(&empty, &[]));
        assert!(contained(&run(&files).unwrap()));
        let (_work, path) = package(&files);
        let facts =
            containment_work_facts(ValidationRequest::new(&path), ValidationLimits::default())
                .unwrap();
        // 128·(⌈2002/64⌉² + ⌈4005/64⌉² + ⌈6006/64⌉²), the oracle's precheck units.
        assert_eq!(facts[0].nodes, Some(1_770_112), "{facts:?}");
        let mut real = empty.clone();
        let mut bin = Vec::new();
        for v in [0f32, 0., 0., 1., 0., 0., 0., 1., 0.] {
            bin.extend(v.to_le_bytes());
        }
        real["buffers"] = json!([{"byteLength":36}]);
        real["bufferViews"] = json!([{"buffer":0,"byteLength":36}]);
        real["accessors"] = json!([{"bufferView":0,"componentType":5126,"type":"VEC3",
            "count":3,"min":[0,0,0],"max":[1,1,0]}]);
        real["meshes"]
            .as_array_mut()
            .unwrap()
            .push(json!({"primitives":[{"attributes":{"POSITION":0}}]}));
        real["nodes"]
            .as_array_mut()
            .unwrap()
            .push(json!({"mesh":1}));
        real["scenes"][0]["nodes"] = json!([0, 3]);
        let files = single(boxed([0.; 3], [10.; 3]), glb(&real, &bin));
        assert_eq!(not_inspected(run(&files)), ["numericSpan"]);
        // The span is the only obstacle: above 6008 bits the triangle streams.
        let limits = ValidationLimits {
            containment_span_bits: 8192,
            ..ValidationLimits::default()
        };
        assert!(contained(&run_with(&files, limits).unwrap()));
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
        // One FLOAT vertex, two skipped nodes, one level of span 277 (q = 25)
        // with the own box: T·V + Nodes + Idx + E = 1·(max(20, 25) + max(25, 25))
        // + 2 + 1 + 3, plus the per-reference terms (C2-WB §3.1): 1152 + 8·1 +
        // B + Dec + 16·J + 160·1 (a.bin) + 4·1 (one instance, one primitive).
        let (mut doc, bin) = points_doc(&[[0., 0., 0.]], json!([{"children":[1]},{"mesh":0}]));
        doc["buffers"][0]["uri"] = json!("a.bin");
        let gltf = serde_json::to_vec(&doc).unwrap();
        let files = vec![
            ("tileset.json", tileset(content_tile(wide(), "a.gltf"))),
            ("a.gltf", gltf.clone()),
            ("a.bin", bin.clone()),
        ];
        let read = (gltf.len() + bin.len()) as u64;
        let json_nodes = value_nodes(&doc);
        let units = 50 + 2 + 1 + 3 + 1152 + 8 + read + bin.len() as u64 + 16 * json_nodes + 160 + 4;
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
                json_nodes,
                external_resources: 1,
                instance_primitives: 1,
                max_node_depth: 2,
            }]
        );
        assert_eq!(
            containment_work_total(ValidationRequest::new(&path), ValidationLimits::default())
                .unwrap(),
            units
        );
    }

    /// `J`: every JSON value counts one node, containers included.
    fn value_nodes(v: &Value) -> u64 {
        1 + match v {
            Value::Array(items) => items.iter().map(value_nodes).sum(),
            Value::Object(members) => members.values().map(value_nodes).sum(),
            _ => 0,
        }
    }
    /// One unnormalized u8 POSITION vertex: every span stays within one limb (q = 1).
    fn quantized_doc(nodes: Value) -> Value {
        let (mut doc, _) = points_doc(&[[0., 0., 0.]], nodes);
        doc["extensionsUsed"] = json!(["KHR_mesh_quantization"]);
        doc["extensionsRequired"] = json!(["KHR_mesh_quantization"]);
        doc["accessors"][0]["componentType"] = json!(5121);
        doc["accessors"][0]["min"] = json!([0, 0, 0]);
        doc["accessors"][0]["max"] = json!([0, 0, 0]);
        doc["buffers"][0]["byteLength"] = json!(4);
        doc["bufferViews"][0] = json!({"buffer":0,"byteLength":4,"byteStride":4});
        doc
    }
    fn work_total(files: &[(&str, Vec<u8>)]) -> u64 {
        let (_work, path) = package(files);
        containment_work_total(ValidationRequest::new(&path), ValidationLimits::default()).unwrap()
    }
    fn work_facts(files: &[(&str, Vec<u8>)]) -> Vec<UriWork> {
        let (_work, path) = package(files);
        containment_work_facts(ValidationRequest::new(&path), ValidationLimits::default()).unwrap()
    }
    fn at_cap_pair(files: &[(&str, Vec<u8>)], units: u64) {
        let cap = |units| ValidationLimits {
            containment_work_units: units,
            ..ValidationLimits::default()
        };
        assert!(contained(&run_with(files, cap(units)).unwrap()));
        assert_eq!(
            not_inspected(run_with(files, cap(units - 1))),
            ["workLimit"]
        );
    }

    #[test]
    fn level_operations_are_charged_at_their_floors() {
        // T = 1 and q = 1 at every level, so each difference is one floor per
        // operation plus 8 per added level; the payload terms are identical.
        let payload = glb(&quantized_doc(json!([{"mesh":0}])), &[0; 4]);
        let facts = work_facts(&single(wide(), payload.clone()));
        assert_eq!(facts[0].t, 1, "{facts:?}");
        let base = work_total(&single(wide(), payload.clone()));
        let f = &facts[0];
        // Level 0: base 20 and the own box 25.
        assert_eq!(
            base,
            20 + 25
                + f.nodes.unwrap()
                + f.idx
                + f.e
                + 1152
                + 8
                + f.b
                + f.dec
                + 16 * f.json_nodes
                + 4
        );
        let with_root = |root: Value, extra: &[(&'static str, Vec<u8>)]| {
            let mut files = vec![("tileset.json", tileset(root)), ("a.glb", payload.clone())];
            files.extend_from_slice(extra);
            work_total(&files) - base
        };
        // An own sphere instead of the box: 32.
        let sphere = json!({"sphere":[0, 0, 0, 10]});
        assert_eq!(
            with_root(content_tile(sphere.clone(), "a.glb"), &[]),
            32 - 25
        );
        // A content box beside the own box at level 0: 25.
        let mut root = content_tile(wide(), "a.glb");
        root["content"]["boundingVolume"] = boxed([0.; 3], [5.; 3]);
        assert_eq!(with_root(root, &[]), 25);
        // A child tile without a transform: the parent's box at level 1, no step.
        let mut root = tile(wide());
        root["children"] = json!([content_tile(boxed([0.; 3], [5.; 3]), "a.glb")]);
        assert_eq!(with_root(root.clone(), &[]), 25 + 8);
        // With a transform the child also pays the step: 30.
        let mut m = IDENTITY;
        m[12] = 1.;
        root["children"][0]["transform"] = json!(m);
        assert_eq!(with_root(root, &[]), 30 + 25 + 8);
        // A sphere parent: 32 at level 1.
        let mut root = tile(sphere);
        root["children"] = json!([content_tile(boxed([0.; 3], [5.; 3]), "a.glb")]);
        assert_eq!(with_root(root, &[]), 32 + 8);
        // Through an external tileset the link's content box is tested at
        // level 1 beside the parent's own box.
        let mut root = tile(wide());
        root["contents"] = json!([{"uri":"ext.json","boundingVolume":boxed([0.; 3], [5.; 3])}]);
        let ext = tileset(content_tile(boxed([0.; 3], [5.; 3]), "a.glb"));
        assert_eq!(with_root(root, &[("ext.json", ext)]), 25 + 25 + 8);
    }

    #[test]
    fn per_reference_terms_are_charged_on_every_reference() {
        // Two instances of a two-primitive mesh, an external buffer: Prim = 4,
        // Res = 1; V = 20 + 25 per (instance, accessor) vertex.
        let mut doc = quantized_doc(json!([{"children":[1, 2]},{"mesh":0},{"mesh":0}]));
        doc["scenes"][0]["nodes"] = json!([0]);
        let primitive = doc["meshes"][0]["primitives"][0].clone();
        doc["meshes"][0]["primitives"] = json!([primitive.clone(), primitive]);
        doc["buffers"][0]["uri"] = json!("a.bin");
        let gltf = serde_json::to_vec(&doc).unwrap();
        let files = |root: Value, gltf: &[u8]| {
            vec![
                ("tileset.json", tileset(root)),
                ("a.gltf", gltf.to_vec()),
                ("a.bin", vec![0; 4]),
            ]
        };
        let one = files(content_tile(wide(), "a.gltf"), &gltf);
        let f = work_facts(&one).remove(0);
        let read = gltf.len() as u64 + 4;
        assert_eq!(
            (
                f.b,
                f.dec,
                f.json_nodes,
                f.external_resources,
                f.instance_primitives
            ),
            (read, 4, value_nodes(&doc), 1, 2 * 2),
            "{f:?}"
        );
        let units = 45 * f.t
            + f.nodes.unwrap()
            + f.idx
            + f.e
            + 1152
            + 8
            + read
            + 4
            + 16 * value_nodes(&doc)
            + 160
            + 4 * 4;
        assert_eq!(work_total(&one), units);
        at_cap_pair(&one, units);
        // A second reference to the same URI pays every term again.
        let mut root = tile(wide());
        root["contents"] = json!([{"uri":"a.gltf"}, {"uri":"a.gltf"}]);
        let two = files(root, &gltf);
        assert_eq!(work_total(&two), 2 * units);
        at_cap_pair(&two, 2 * units);
        // J: five more value nodes (an object, an array, three numbers) cost 16
        // each beside their bytes.
        let mut extra = doc.clone();
        extra["extras"] = json!({"k":[1, 2, 3]});
        let longer = serde_json::to_vec(&extra).unwrap();
        let more = files(content_tile(wide(), "a.gltf"), &longer);
        assert_eq!(work_facts(&more)[0].json_nodes, value_nodes(&doc) + 5);
        assert_eq!(
            work_total(&more) - units,
            16 * 5 + (longer.len() - gltf.len()) as u64
        );
        // Res: the same bytes as a data URI resolve nothing (Res = 0) and are
        // read with the JSON; Dec, J and Prim are unchanged.
        let mut inline = doc.clone();
        inline["buffers"][0]["uri"] = json!("data:application/octet-stream;base64,AAAAAA==");
        let inline = serde_json::to_vec(&inline).unwrap();
        let data = vec![
            ("tileset.json", tileset(content_tile(wide(), "a.gltf"))),
            ("a.gltf", inline.clone()),
        ];
        let g = work_facts(&data).remove(0);
        assert_eq!(
            (
                g.b,
                g.dec,
                g.json_nodes,
                g.external_resources,
                g.instance_primitives
            ),
            (inline.len() as u64, 4, value_nodes(&doc), 0, 4)
        );
        assert_eq!(
            i128::from(units) - i128::from(work_total(&data)),
            160 + i128::from(read) - inline.len() as i128
        );
    }

    #[test]
    fn worked_example_total_reproduces_the_c2wb_formula() {
        // §3.6: spans [23, 29], V = (20 + 25) + (30 + 25) = 100; one TRS node
        // with s_step = 5: 256; two levels: 1152 + 16; Res = 0, Prim = 1.
        let doc = json!({
            "asset":{"version":"2.0"},"extensionsUsed":["KHR_mesh_quantization"],
            "extensionsRequired":["KHR_mesh_quantization"],"scene":0,"scenes":[{"nodes":[0]}],
            "nodes":[{"mesh":0,"translation":[10.5,0,-3],"scale":[2,4,0.25]}],
            "buffers":[{"byteLength":8}],"bufferViews":[{"buffer":0,"byteLength":8,"byteStride":8}],
            "accessors":[{"bufferView":0,"componentType":5123,"normalized":true,"type":"VEC3",
                "count":1,"min":[0,0,0],"max":[0,0,0]}],
            "meshes":[{"primitives":[{"attributes":{"POSITION":0},"mode":0}]}]
        });
        let mut child = content_tile(json!({"box":[1.5,3,2,100,0,0,0,0,0,0,0,50]}), "a.glb");
        let mut m = IDENTITY;
        m[12] = 1000.25;
        m[13] = -20.;
        child["transform"] = json!(m);
        let mut root = tile(boxed([0.; 3], [2048.; 3]));
        root["children"] = json!([child]);
        let payload = glb(&doc, &[0; 8]);
        let files = vec![("tileset.json", tileset(root)), ("a.glb", payload.clone())];
        let f = work_facts(&files).remove(0);
        assert_eq!(
            (
                f.nodes,
                f.external_resources,
                f.instance_primitives,
                f.json_nodes
            ),
            (Some(256), 0, 1, value_nodes(&doc)),
            "{f:?}"
        );
        assert_eq!(f.b, payload.len() as u64);
        let units = f.t * 100 + 256 + f.idx + f.e + f.dec + f.b + 16 * f.json_nodes + 4 + 1152 + 16;
        assert_eq!(work_total(&files), units);
        at_cap_pair(&files, units);
    }

    #[test]
    fn chain_bits_are_inherited_at_registration_and_ored_per_reference() {
        let rotated = json!({"box":[0,0,0,1,1,0,-1,1,0,0,0,1]});
        let small = || boxed([0.; 3], [0.25; 3]);
        let payload = points(&[[0., 0., 0.]]);
        let with = |root: Value, extra: Option<(&'static str, Vec<u8>)>| {
            let mut files = vec![("tileset.json", tileset(root)), ("a.glb", payload.clone())];
            files.extend(extra);
            files
        };
        // An ancestor's rotated box two levels up.
        let mut mid = tile(small());
        mid["children"] = json!([content_tile(small(), "a.glb")]);
        let mut root = tile(rotated.clone());
        root["children"] = json!([mid]);
        assert_eq!(not_inspected(run(&with(root, None))), ["rotatedBox"]);
        // An external-tileset link's content volume, inherited by the external root.
        let mut root = tile(wide());
        root["contents"] = json!([{"uri":"ext.json","boundingVolume":rotated.clone()}]);
        let ext = tileset(content_tile(small(), "a.glb"));
        assert_eq!(
            not_inspected(run(&with(root, Some(("ext.json", ext))))),
            ["rotatedBox"]
        );
        // The reference's own content volume (not part of the tile's chain).
        let mut root = content_tile(wide(), "a.glb");
        root["content"]["boundingVolume"] = rotated.clone();
        assert_eq!(not_inspected(run(&with(root, None))), ["rotatedBox"]);
        // A rotated tile that no reference passes through excludes nothing.
        let mut root = content_tile(wide(), "a.glb");
        root["children"] = json!([tile(rotated.clone())]);
        assert!(contained(&run(&with(root, None)).unwrap()));
        // A rotated sibling's content volume excludes nothing for the other reference.
        let mut root = tile(wide());
        let mut sibling = tile(wide());
        sibling["contents"] = json!([{"uri":"ext.json","boundingVolume":rotated}]);
        root["children"] = json!([content_tile(wide(), "a.glb"), sibling]);
        let ext = tileset(tile(small()));
        assert!(contained(
            &run(&with(root, Some(("ext.json", ext)))).unwrap()
        ));
    }

    #[test]
    fn chain_bits_keep_precedence_after_the_precheck_stops() {
        // The first reference stops the precheck at workLimit; a later reference
        // through a rotated ancestor still lists rotatedBox, and only it.
        let payload = points(&[[0., 0., 0.]]);
        let mut excluded = tile(json!({"box":[0,0,0,1,1,0,-1,1,0,0,0,1]}));
        excluded["children"] = json!([content_tile(boxed([0.; 3], [0.25; 3]), "a.glb")]);
        let mut root = tile(wide());
        root["children"] = json!([content_tile(wide(), "a.glb"), excluded]);
        let files = vec![
            ("tileset.json", tileset(root.clone())),
            ("a.glb", payload.clone()),
        ];
        let limits = ValidationLimits {
            containment_work_units: 1,
            ..ValidationLimits::default()
        };
        assert_eq!(
            not_inspected(run_with(&files, limits.clone())),
            ["rotatedBox"]
        );
        // Control: without the later reference the same cap is workLimit.
        root["children"].as_array_mut().unwrap().pop();
        let files = vec![("tileset.json", tileset(root)), ("a.glb", payload)];
        assert_eq!(not_inspected(run_with(&files, limits)), ["workLimit"]);
    }

    #[test]
    fn levels_are_built_only_when_the_precheck_step_runs() {
        let limits = ValidationLimits::default();
        let mut c = Containment::new(&limits);
        let facts = UriFacts {
            bounds: None,
            t: 0,
            nodes: Some(0),
            idx: 0,
            e: 0,
            per_reference: ReferenceFacts::default(),
            max_depth: 0,
            exclusions: 0,
        };
        c.uris.push(("a.glb".into(), facts));
        let fact = |volume, parent, chain| TileFact {
            transform: None,
            volume,
            external: None,
            parent,
            depth: 0,
            chain,
        };
        let plain = VolumeFacts::Box([0., 0., 0., 1., 0., 0., 0., 1., 0., 0., 0., 1.]);
        let root = c.tile(fact(Some(plain), None, 0)).unwrap();
        let child = c.tile(fact(Some(plain), Some(root), 0)).unwrap();
        assert_eq!(c.tiles[child].chain, 0);
        c.reference(child, None, 0).unwrap();
        assert_eq!((c.levels.len(), c.units), (2, 1152 + 16));
        // Slice 3 §4 step 5: a rowless implicit tile (`None`) carries the
        // derived bit, inherited and ORed without building levels.
        c.levels.clear();
        let derived = c.tile(fact(None, Some(root), 0)).unwrap();
        assert_eq!(c.tiles[derived].chain, Reason::DerivedImplicitVolume.bit());
        let below = c.tile(fact(Some(plain), Some(derived), 0)).unwrap();
        assert_eq!(c.tiles[below].chain, Reason::DerivedImplicitVolume.bit());
        c.reference(below, None, 0).unwrap();
        assert_eq!(c.exclusions, Reason::DerivedImplicitVolume.bit());
        assert!(c.levels.is_empty());
        assert_eq!(c.units, 1152 + 16);
        // A caller-supplied chain bit is inherited too.
        let marked = c
            .tile(fact(Some(plain), Some(root), Reason::Skin.bit()))
            .unwrap();
        let below_marked = c.tile(fact(Some(plain), Some(marked), 0)).unwrap();
        assert_eq!(c.tiles[below_marked].chain, Reason::Skin.bit());
        // `build_levels` never sees a `None` in domain; reaching one, as own
        // volume or as an ancestor's, is an internal invariant failure.
        for tile in [derived, below] {
            assert!(matches!(
                c.build_levels(tile, None),
                Err(Error::InvalidState(_))
            ));
        }
        // A region tile's bit reaches its descendants' chains.
        let region = c
            .tile(fact(Some(VolumeFacts::Region), Some(root), 0))
            .unwrap();
        let leaf = c.tile(fact(Some(plain), Some(region), 0)).unwrap();
        assert_eq!(c.tiles[leaf].chain, Reason::RegionVolume.bit());
        assert_eq!(c.tiles[root].chain, 0);
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
