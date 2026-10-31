//! Shared 3D Tiles node pieces for hierarchies whose children sit at a pure
//! translation from their parent's centre, with origin-centred box bounds.
//!
//! It also owns the typed Tiles 1.1 records ([`TileRecord`], [`TileTree`],
//! [`TilesetDocument`] and their parts) and their deterministic serializer: the
//! one emitter of tileset and tile JSON for callers migrated onto typed records
//! (vector first). Records are validated at construction, so every emitter is a
//! pure function of validated values; generated JSON is never read back.
use crate::{implicit::SubdivisionScheme, vec3::Vec3, Error};
use serde::Serialize;
use serde_json::{Map, Value};
use std::collections::BTreeSet;

/// A translation-only column-major `transform`.
pub(crate) fn translation(delta: Vec3) -> Value {
    serde_json::json!([1, 0, 0, 0, 0, 1, 0, 0, 0, 0, 1, 0, delta[0], delta[1], delta[2], 1])
}

/// The offset of a node written with [`translation`].
pub(crate) fn translation_offset(node: &Value) -> Result<Vec3, Error> {
    let mut offset = [0.; 3];
    for (i, value) in offset.iter_mut().enumerate() {
        *value = node["transform"][12 + i]
            .as_f64()
            .ok_or_else(|| Error::Data("invalid child transform".into()))?;
    }
    Ok(offset)
}

/// An axis-aligned, origin-centred `box`. `zero` is written verbatim so each
/// converter keeps its published JSON form (`0` for vector, `0.0` for points).
pub(crate) fn box_json(zero: impl Into<Value>, half: Vec3) -> Value {
    let z: Value = zero.into();
    Value::Array(vec![
        z.clone(),
        z.clone(),
        z.clone(),
        half[0].into(),
        z.clone(),
        z.clone(),
        z.clone(),
        half[1].into(),
        z.clone(),
        z.clone(),
        z,
        half[2].into(),
    ])
}

/// Half extents of an axis-aligned `box` value.
pub(crate) fn box_half(value: &Value) -> Result<Vec3, Error> {
    let mut half = [0.; 3];
    for (i, h) in half.iter_mut().enumerate() {
        *h = value[3 + i * 4]
            .as_f64()
            .ok_or_else(|| Error::Data("invalid child bounds".into()))?;
    }
    Ok(half)
}

/// The exact rounding error of `a + b`: `(a + b) - fl(a + b)` over the reals,
/// itself an f64 (Knuth's TwoSum). Zero when the sum is exact.
pub(crate) fn sum_error(a: f64, b: f64) -> f64 {
    let sum = a + b;
    let b_part = sum - a;
    (a - (sum - b_part)) + (b - b_part)
}

/// `a + b` rounded up: the least f64 at or above the exact sum. A sum that
/// was rounded down moves up one ulp; every other sum is returned unchanged.
pub(crate) fn sum_up(a: f64, b: f64) -> f64 {
    let sum = a + b;
    if sum_error(a, b) > 0. {
        sum.next_up()
    } else {
        sum
    }
}

/// Grow `half` (the parent's own, already padded content box) to enclose
/// every child placed at `delta`, and return the largest of `error` and the
/// children's geometric errors. Children carry their own padding, so the
/// enclosure stays conservative; each `|delta| + child half` is rounded
/// outward, so the child box nests exactly (C2 decodedContentContainment
/// tests every ancestor in exact arithmetic). Malformed child bounds are data
/// errors.
pub(crate) fn enclose_children<'a>(
    half: &mut Vec3,
    mut error: f64,
    children: impl IntoIterator<Item = (Vec3, &'a Value)>,
) -> Result<f64, Error> {
    for (delta, child) in children {
        let child_half = box_half(&child["boundingVolume"]["box"])?;
        for i in 0..3 {
            half[i] = half[i].max(sum_up(delta[i].abs(), child_half[i]));
        }
        error = error.max(child["geometricError"].as_f64().unwrap_or(0.));
    }
    Ok(error)
}

/// An empty parent's geometric error is this multiple of its box diagonal: until its whole
/// box projects below about one pixel at CesiumJS's default maximum screen-space error of
/// 16, the viewer refines it towards the leaves that hold the geometry.
pub(crate) const EMPTY_PARENT_SCALE: f64 = 16.;

/// Tileset-level geometric error: at least 1 m, the root box `diagonal`, and
/// `root_margin` times the root's own error. A margin above 1 is a converter
/// policy that leaves an SSE interval in which the root itself renders.
pub(crate) fn top_level_error(diagonal: f64, root_error: f64, root_margin: f64) -> f64 {
    1f64.max(root_error * root_margin).max(diagonal)
}

// ---- Owned Tiles 1.1 records and their serializer ----
//
// Number forms are contract: structural constants are JSON integers (centred-box zeros,
// transform 0 and 1, and the empty root's error); measured values are f64, including
// measured zeros, written by serde_json's shortest round-trip formatter. Objects are
// `serde_json::Map`s, which sort their keys (the crate does not enable `preserve_order`).
// Every value is validated when its record is built, so emitters cannot write `null` for a
// non-finite number.

/// The vector content marker that makes CesiumJS decode a GLB as vector content.
pub(crate) const CONTENT_GLTF_VECTOR: &str = "3DTILES_content_gltf_vector";
/// The bound on `|a_i · a_j − δ_ij|` for the axes of a rigid placement (reuse restores
/// frames to the same bound).
const ORTHONORMAL: f64 = 1e-12;

fn invalid(message: impl std::fmt::Display) -> Error {
    Error::Data(format!("tileset record: {message}"))
}

fn finite3(value: Vec3, what: &str) -> Result<Vec3, Error> {
    if value.iter().all(|v| v.is_finite()) {
        Ok(value)
    } else {
        Err(invalid(format!("{what} {value:?} is not finite")))
    }
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// A tile's geometric error in metres: finite and non-negative.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct GeometricError {
    metres: f64,
    /// The structural constant `0`, written as a JSON integer ([`TileRecord::empty_root`]).
    structural: bool,
}

impl GeometricError {
    pub(crate) fn new(metres: f64) -> Result<Self, Error> {
        if metres.is_finite() && metres >= 0. {
            Ok(Self {
                metres,
                structural: false,
            })
        } else {
            Err(invalid(format!(
                "geometric error {metres} must be finite and non-negative"
            )))
        }
    }

    pub(crate) fn metres(self) -> f64 {
        self.metres
    }

    /// The larger of two errors (`f64::max`, as [`enclose_children`]), as a measured value.
    pub(crate) fn max(self, other: Self) -> Self {
        Self {
            metres: self.metres.max(other.metres),
            structural: false,
        }
    }

    fn to_value(self) -> Value {
        if self.structural {
            Value::from(0)
        } else {
            Value::from(self.metres)
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Refine {
    Replace,
    // Point clouds refine by ADD; they write through the JSON shim until they migrate.
    #[cfg_attr(not(test), allow(dead_code))]
    Add,
}

impl Refine {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Replace => "REPLACE",
            Self::Add => "ADD",
        }
    }
}

/// An axis-aligned `box` in the tile's own frame (z-up metres): finite, non-negative half
/// lengths, centred either on the frame origin (written as integer zeros) or at a measured
/// centre.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct TileBox {
    center: Option<Vec3>,
    half: Vec3,
}

fn checked_half(half: Vec3) -> Result<Vec3, Error> {
    if half.iter().all(|h| h.is_finite() && *h >= 0.) {
        Ok(half)
    } else {
        Err(invalid(format!(
            "box half lengths {half:?} must be finite and non-negative"
        )))
    }
}

impl TileBox {
    /// A box centred on the tile frame's origin (vector, points).
    pub(crate) fn centred(half: Vec3) -> Result<Self, Error> {
        Ok(Self {
            center: None,
            half: checked_half(half)?,
        })
    }

    /// A box with a measured centre (the wrapped model's box; the large-mesh tiler's boxes).
    pub(crate) fn offset(center: Vec3, half: Vec3) -> Result<Self, Error> {
        Ok(Self {
            center: Some(finite3(center, "box centre")?),
            half: checked_half(half)?,
        })
    }

    #[cfg(test)]
    pub(crate) fn half(&self) -> Vec3 {
        self.half
    }

    pub(crate) fn center(&self) -> Vec3 {
        self.center.unwrap_or([0.; 3])
    }

    /// The twelve `box` numbers.
    pub(crate) fn lanes(&self) -> [f64; 12] {
        let (c, h) = (self.center(), self.half);
        [c[0], c[1], c[2], h[0], 0., 0., 0., h[1], 0., 0., 0., h[2]]
    }

    /// This box with the half lengths of `lanes` (lanes 3, 7 and 11), as the implicit layout
    /// widens them. Every other lane must equal this box's.
    pub(crate) fn with_half_lanes(&self, lanes: &[f64; 12]) -> Result<Self, Error> {
        let own = self.lanes();
        if (0..12).any(|k| k % 4 != 3 && lanes[k] != own[k]) {
            return Err(invalid("a widened box must keep its centre and axes"));
        }
        Ok(Self {
            center: self.center,
            half: checked_half([lanes[3], lanes[7], lanes[11]])?,
        })
    }

    /// Grow this origin-centred box to enclose an origin-centred `child` box placed at
    /// `delta`. Each `|delta| + child half` is rounded outward ([`sum_up`]), exactly as
    /// [`enclose_children`], so the child box nests in exact arithmetic.
    pub(crate) fn grow_to_enclose(&mut self, delta: Vec3, child: &TileBox) -> Result<(), Error> {
        if self.center.is_some() || child.center.is_some() {
            return Err(invalid(
                "only origin-centred boxes enclose translated children",
            ));
        }
        let delta = finite3(delta, "child translation")?;
        let mut half = self.half;
        for i in 0..3 {
            half[i] = half[i].max(sum_up(delta[i].abs(), child.half[i]));
        }
        self.half = checked_half(half)?;
        Ok(())
    }

    /// The box diagonal, `2·|half|`.
    pub(crate) fn diagonal(&self) -> f64 {
        crate::vec3::norm(self.half) * 2.
    }

    fn to_value(self) -> Value {
        let mut lanes = box_json(0, self.half);
        if let (Some(center), Value::Array(values)) = (self.center, &mut lanes) {
            for (lane, c) in values.iter_mut().zip(center) {
                *lane = Value::from(c);
            }
        }
        lanes
    }
}

/// A tile's `transform`: none, a pure translation from the parent's frame (children), or a
/// rigid placement (roots).
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct NodePlacement(Placement);

#[derive(Clone, Copy, Debug, PartialEq)]
enum Placement {
    Inherit,
    Translation(Vec3),
    Rigid { axes: [Vec3; 3], origin: Vec3 },
}

impl NodePlacement {
    /// No `transform`: the tile shares its parent's frame.
    pub(crate) const INHERIT: Self = Self(Placement::Inherit);

    /// A translation-only `transform` (integer 0 and 1, f64 offsets), always written, even
    /// for a zero offset.
    pub(crate) fn translation(delta: Vec3) -> Result<Self, Error> {
        Ok(Self(Placement::Translation(finite3(delta, "translation")?)))
    }

    /// A rotation by right-handed orthonormal `axes` (columns, for example ENU in ECEF) and
    /// a translation to `origin`.
    pub(crate) fn rigid(axes: [Vec3; 3], origin: Vec3) -> Result<Self, Error> {
        for axis in axes {
            finite3(axis, "axis")?;
        }
        finite3(origin, "origin")?;
        for i in 0..3 {
            for j in 0..3 {
                let expected = if i == j { 1. } else { 0. };
                let dot = crate::vec3::dot(axes[i], axes[j]);
                if (dot - expected).abs() > ORTHONORMAL {
                    return Err(invalid(format!(
                        "rigid placement axes {axes:?} are not orthonormal"
                    )));
                }
            }
        }
        let [a, b, c] = axes;
        let cross = [
            a[1] * b[2] - a[2] * b[1],
            a[2] * b[0] - a[0] * b[2],
            a[0] * b[1] - a[1] * b[0],
        ];
        if crate::vec3::dot(cross, c) <= 0. {
            return Err(invalid(format!(
                "rigid placement axes {axes:?} are not right-handed"
            )));
        }
        Ok(Self(Placement::Rigid { axes, origin }))
    }

    /// The written column-major `transform` values, `None` without a transform.
    pub(crate) fn matrix(&self) -> Option<[f64; 16]> {
        match self.0 {
            Placement::Inherit => None,
            Placement::Translation(d) => Some([
                1., 0., 0., 0., 0., 1., 0., 0., 0., 0., 1., 0., d[0], d[1], d[2], 1.,
            ]),
            Placement::Rigid {
                axes: [e, n, u],
                origin: o,
            } => Some([
                e[0], e[1], e[2], 0., n[0], n[1], n[2], 0., u[0], u[1], u[2], 0., o[0], o[1], o[2],
                1.,
            ]),
        }
    }

    /// The offset from the parent's frame for a child: zero without a transform, `None` for
    /// a rigid placement.
    pub(crate) fn child_translation(&self) -> Option<Vec3> {
        match self.0 {
            Placement::Inherit => Some([0.; 3]),
            Placement::Translation(delta) => Some(delta),
            Placement::Rigid { .. } => None,
        }
    }

    fn to_value(self) -> Option<Value> {
        match self.0 {
            Placement::Inherit => None,
            Placement::Translation(delta) => Some(translation(delta)),
            Placement::Rigid {
                axes: [e, n, u],
                origin: o,
            } => Some(serde_json::json!([
                e[0], e[1], e[2], 0, n[0], n[1], n[2], 0, u[0], u[1], u[2], 0, o[0], o[1], o[2], 1
            ])),
        }
    }
}

/// `name` as a relative archive member reference: `/`-separated non-empty segments of URI
/// unreserved ASCII (`A-Z a-z 0-9 - . _ ~`), none `.` or `..`, not starting with `.`.
/// That excludes schemes, queries, fragments, escapes, backslashes and absolute paths.
fn check_relative(name: &str, what: &str) -> Result<(), Error> {
    let fail = |why: &str| Err(invalid(format!("{what} {name:?} {why}")));
    if name.is_empty() {
        return fail("is empty");
    }
    if name.starts_with('.') {
        return fail("must not start with '.'");
    }
    if let Some(c) = name
        .chars()
        .find(|c| !(c.is_ascii_alphanumeric() || "-._~/".contains(*c)))
    {
        return fail(&format!(
            "has the character {c:?}; members are relative paths"
        ));
    }
    if name
        .split('/')
        .any(|segment| segment.is_empty() || segment == "." || segment == "..")
    {
        return fail("must be relative, with no empty, '.' or '..' segment");
    }
    Ok(())
}

/// A validated relative archive member name, or ([`MemberName::escaped`]) a relative URI
/// reference whose percent escapes name a file beside the document.
#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub(crate) struct MemberName {
    uri: String,
    /// The file the URI names, when it differs from the URI text (percent escapes).
    decoded: Option<String>,
}

impl MemberName {
    pub(crate) fn new(name: impl Into<String>) -> Result<Self, Error> {
        let name = name.into();
        check_relative(&name, "member name")?;
        Ok(Self {
            uri: name,
            decoded: None,
        })
    }

    /// A relative URI reference in canonical percent-encoded form, as
    /// `binding::filename_uri` writes a source filename for a sibling manifest:
    /// `/`-separated segments of URI unreserved ASCII and `%HH` escapes (upper-case hex) of
    /// every other byte. Each decoded segment is UTF-8, non-empty, not `.` or `..`, and has
    /// no separator or control character, and the whole URI passes the portable path rules
    /// of admitted resource references (`binding::relative_uri`); unlike
    /// [`MemberName::new`], it may start with `.`, because the source file keeps its
    /// admitted name.
    pub(crate) fn escaped(uri: impl Into<String>) -> Result<Self, Error> {
        let uri = uri.into();
        let fail = |why: &str| invalid(format!("escaped URI {uri:?} {why}"));
        let unreserved = |b: u8| b.is_ascii_alphanumeric() || b"-._~".contains(&b);
        let hex = |b: u8| match b {
            b'0'..=b'9' => Some(b - b'0'),
            b'A'..=b'F' => Some(b - b'A' + 10),
            _ => None,
        };
        let mut decoded = Vec::with_capacity(uri.len());
        for segment in uri.split('/') {
            let start = decoded.len();
            let bytes = segment.as_bytes();
            let mut at = 0;
            while at < bytes.len() {
                if bytes[at] == b'%' {
                    let Some(byte) = bytes
                        .get(at + 1..at + 3)
                        .and_then(|pair| Some(hex(pair[0])? * 16 + hex(pair[1])?))
                    else {
                        return Err(fail(
                            "has a percent escape that is not two upper-case hex digits",
                        ));
                    };
                    if unreserved(byte) {
                        return Err(fail("escapes an unreserved character"));
                    }
                    decoded.push(byte);
                    at += 3;
                } else if unreserved(bytes[at]) {
                    decoded.push(bytes[at]);
                    at += 1;
                } else {
                    return Err(fail(&format!(
                        "has the character {:?}; it must be escaped",
                        char::from(bytes[at])
                    )));
                }
            }
            match &decoded[start..] {
                b"" | b"." | b".." => {
                    return Err(fail("must be relative, with no empty, '.' or '..' segment"))
                }
                part if part
                    .iter()
                    .any(|b| *b == b'/' || *b == b'\\' || b.is_ascii_control()) =>
                {
                    return Err(fail("encodes a separator or control character"))
                }
                _ => {}
            }
            decoded.push(b'/');
        }
        decoded.pop();
        let decoded = String::from_utf8(decoded).map_err(|_| fail("does not decode to UTF-8"))?;
        if decoded.chars().any(char::is_control) {
            return Err(fail("encodes a control character"));
        }
        // The admitted resource reference rules: length and component ceilings and the
        // portable filename profile (no `<>:"|?*`, no trailing dot or space).
        crate::gltf_source::binding::relative_uri(&uri)
            .map_err(|error| fail(&format!("is not a portable relative path: {error}")))?;
        Ok(Self {
            decoded: (decoded != uri).then_some(decoded),
            uri,
        })
    }

    /// The URI text, as written.
    pub(crate) fn as_str(&self) -> &str {
        &self.uri
    }

    /// The relative file path the name references: the URI with its escapes decoded.
    pub(crate) fn path(&self) -> &str {
        self.decoded.as_deref().unwrap_or(&self.uri)
    }

    /// Whether the name is a plain archive member name ([`MemberName::new`]): no escapes and
    /// no leading `.`.
    pub(crate) fn is_member(&self) -> bool {
        self.decoded.is_none() && check_relative(&self.uri, "member name").is_ok()
    }
}

/// A validated implicit URI template: a relative member name whose only placeholders are
/// exactly its scheme's `{level}`, `{x}`, `{y}` (and `{z}` for octrees), each present.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct UriTemplate {
    template: String,
    scheme: SubdivisionScheme,
}

impl UriTemplate {
    pub(crate) fn new(
        template: impl Into<String>,
        scheme: SubdivisionScheme,
    ) -> Result<Self, Error> {
        let template = template.into();
        let allowed: &[&str] = match scheme {
            SubdivisionScheme::Quadtree => &["level", "x", "y"],
            SubdivisionScheme::Octree => &["level", "x", "y", "z"],
        };
        let mut seen = BTreeSet::new();
        let mut expanded = String::with_capacity(template.len());
        let mut rest = template.as_str();
        while let Some(open) = rest.find(['{', '}']) {
            expanded.push_str(&rest[..open]);
            let close = rest[open..]
                .find('}')
                .filter(|_| rest.as_bytes()[open] == b'{')
                .map(|close| open + close)
                .ok_or_else(|| invalid(format!("template {template:?} has an unmatched brace")))?;
            let name = &rest[open + 1..close];
            if !allowed.contains(&name) {
                return Err(invalid(format!(
                    "template {template:?} has the placeholder {{{name}}}, outside the {} scheme",
                    scheme.as_str()
                )));
            }
            seen.insert(name);
            expanded.push('0');
            rest = &rest[close + 1..];
        }
        expanded.push_str(rest);
        if let Some(missing) = allowed.iter().find(|name| !seen.contains(*name)) {
            return Err(invalid(format!(
                "template {template:?} lacks the placeholder {{{missing}}}"
            )));
        }
        check_relative(&expanded, "template")?;
        Ok(Self { template, scheme })
    }

    pub(crate) fn as_str(&self) -> &str {
        &self.template
    }

    pub(crate) fn scheme(&self) -> SubdivisionScheme {
        self.scheme
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ContentKind {
    Glb,
    /// A glTF JSON document (an unchanged wrapped `.gltf` source).
    Gltf,
}

impl ContentKind {
    pub(crate) fn suffix(self) -> &'static str {
        match self {
            Self::Glb => "glb",
            Self::Gltf => "gltf",
        }
    }

    /// The kind of a payload by its leading bytes: GLB magic, otherwise glTF JSON.
    pub(crate) fn of(bytes: &[u8]) -> Self {
        if bytes.starts_with(b"glTF") {
            Self::Glb
        } else {
            Self::Gltf
        }
    }

    /// The least byte length of a well-formed payload of this kind, and what it is.
    fn minimum(self) -> (u64, &'static str) {
        match self {
            Self::Glb => (12, "a GLB header"),
            Self::Gltf => (2, "a JSON object"),
        }
    }
}

/// Whose features a content holds, for accounting and oracle checks. It is never
/// serialized: `_source_id` in the GLB property table is the identity carrier.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum SourceIdentity {
    /// `count` source features, with a digest of their identities.
    Features { count: u32, digest: [u8; 32] },
    /// Content that carries no per-feature source identity: aggregates, or an unchanged
    /// wrapped source model, whose content digest is its identity.
    Aggregate,
    /// `count` source features in a payload reused from a previous build. Its content
    /// address pins the identities, which the previous build state does not record.
    Reused { count: u32 },
}

/// A tile content: a content-addressed payload member and how it is declared.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct ContentRef {
    uri: MemberName,
    sha256: [u8; 32],
    byte_length: u64,
    kind: ContentKind,
    /// Declares `3DTILES_content_gltf_vector {vector: true}`.
    vector: bool,
    sources: SourceIdentity,
}

fn content_header(vector: bool) -> Map<String, Value> {
    let mut header = Map::new();
    if vector {
        let mut marker = Map::new();
        marker.insert("vector".into(), Value::Bool(true));
        let mut extensions = Map::new();
        extensions.insert(CONTENT_GLTF_VECTOR.into(), Value::Object(marker));
        header.insert("extensions".into(), Value::Object(extensions));
    }
    header
}

impl ContentRef {
    pub(crate) fn new(
        uri: MemberName,
        sha256: [u8; 32],
        byte_length: u64,
        kind: ContentKind,
        vector: bool,
        sources: SourceIdentity,
    ) -> Result<Self, Error> {
        let suffix = kind.suffix();
        if !uri
            .as_str()
            .rsplit_once('.')
            .is_some_and(|(_, s)| s == suffix)
        {
            return Err(invalid(format!(
                "content {:?} must be a .{suffix} member",
                uri.as_str()
            )));
        }
        let (minimum, what) = kind.minimum();
        if byte_length < minimum {
            return Err(invalid(format!(
                "content {:?} has {byte_length} bytes, less than {what}",
                uri.as_str()
            )));
        }
        if matches!(
            sources,
            SourceIdentity::Features { count: 0, .. } | SourceIdentity::Reused { count: 0 }
        ) {
            return Err(invalid(format!(
                "content {:?} declares no source features",
                uri.as_str()
            )));
        }
        Ok(Self {
            uri,
            sha256,
            byte_length,
            kind,
            vector,
            sources,
        })
    }

    /// An unchanged source model a wrapper references, of `kind` by its bytes
    /// ([`ContentKind::of`]). The name keeps the admitted file name, whatever its suffix
    /// ([`MemberName::escaped`] for a sibling file); it carries no per-feature identity.
    pub(crate) fn source_model(
        uri: MemberName,
        sha256: [u8; 32],
        byte_length: u64,
        kind: ContentKind,
    ) -> Result<Self, Error> {
        let (minimum, what) = kind.minimum();
        if byte_length < minimum {
            return Err(invalid(format!(
                "source model {:?} has {byte_length} bytes, less than {what}",
                uri.as_str()
            )));
        }
        Ok(Self {
            uri,
            sha256,
            byte_length,
            kind,
            vector: false,
            sources: SourceIdentity::Aggregate,
        })
    }

    pub(crate) fn uri(&self) -> &MemberName {
        &self.uri
    }

    #[cfg(test)]
    pub(crate) fn byte_length(&self) -> u64 {
        self.byte_length
    }

    pub(crate) fn kind(&self) -> ContentKind {
        self.kind
    }

    pub(crate) fn vector(&self) -> bool {
        self.vector
    }

    pub(crate) fn sources(&self) -> SourceIdentity {
        self.sources
    }

    /// The content object without `uri`: the header an implicit content slot shares.
    pub(crate) fn header(&self) -> Map<String, Value> {
        content_header(self.vector)
    }

    /// Read this content's staged payload under `directory`. A missing member is a
    /// [`StagedContentError::Missing`].
    pub(crate) fn read_staged(&self, directory: &std::path::Path) -> Result<Vec<u8>, Error> {
        match std::fs::read(directory.join(self.uri.path())) {
            Ok(bytes) => Ok(bytes),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                Err(StagedContentError::Missing {
                    member: self.uri.as_str().to_owned(),
                }
                .into())
            }
            Err(error) => Err(error.into()),
        }
    }

    /// Check a staged payload of `length` bytes and digest `sha256` against the declared
    /// length and digest, before the payload is published.
    pub(crate) fn check_staged(
        &self,
        length: u64,
        sha256: &[u8; 32],
    ) -> Result<(), StagedContentError> {
        let member = || self.uri.as_str().to_owned();
        if length != self.byte_length {
            return Err(StagedContentError::Length {
                member: member(),
                declared: self.byte_length,
                staged: length,
            });
        }
        if *sha256 != self.sha256 {
            return Err(StagedContentError::Digest {
                member: member(),
                declared: self.sha256,
                staged: *sha256,
            });
        }
        Ok(())
    }

    /// Read, hash and check this content's staged payload; returns its length.
    pub(crate) fn verify_staged(&self, directory: &std::path::Path) -> Result<u64, Error> {
        use sha2::Digest;
        let bytes = self.read_staged(directory)?;
        let length = bytes.len() as u64;
        self.check_staged(length, &sha2::Sha256::digest(&bytes).into())?;
        Ok(length)
    }

    fn to_value(&self) -> Value {
        let mut object = self.header();
        object.insert("uri".into(), Value::from(self.uri.as_str()));
        Value::Object(object)
    }
}

/// A staged payload that does not match its [`ContentRef`] at publication. Converted to
/// [`Error::Data`]; nothing is published.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum StagedContentError {
    /// The member is not in the staging directory.
    Missing { member: String },
    /// The staged payload's byte length differs from the declared length.
    Length {
        member: String,
        declared: u64,
        staged: u64,
    },
    /// The staged payload's sha256 differs from the declared digest.
    Digest {
        member: String,
        declared: [u8; 32],
        staged: [u8; 32],
    },
}

impl std::fmt::Display for StagedContentError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Missing { member } => write!(f, "staged content {member:?} is missing"),
            Self::Length {
                member,
                declared,
                staged,
            } => write!(
                f,
                "staged content {member:?} has {staged} bytes; its record declares {declared}"
            ),
            Self::Digest {
                member,
                declared,
                staged,
            } => write!(
                f,
                "staged content {member:?} has sha256 {}; its record declares {}",
                hex(staged),
                hex(declared)
            ),
        }
    }
}

impl From<StagedContentError> for Error {
    fn from(error: StagedContentError) -> Self {
        Error::Data(format!("tileset record: {error}; nothing was published"))
    }
}

/// An implicit root's content template, one per slot in the layout's slot order.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ContentTemplate {
    uri: UriTemplate,
    vector: bool,
}

impl ContentTemplate {
    pub(crate) fn new(uri: UriTemplate, vector: bool) -> Self {
        Self { uri, vector }
    }

    fn to_value(&self) -> Value {
        let mut object = content_header(self.vector);
        object.insert("uri".into(), Value::from(self.uri.as_str()));
        Value::Object(object)
    }
}

/// An implicit root's `implicitTiling` with a single subtree:
/// `subtreeLevels == availableLevels`.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ImplicitTiling {
    scheme: SubdivisionScheme,
    levels: u32,
    subtrees: UriTemplate,
}

impl ImplicitTiling {
    pub(crate) fn single_subtree(
        scheme: SubdivisionScheme,
        levels: u32,
        subtrees: UriTemplate,
    ) -> Result<Self, Error> {
        if !(1..=32).contains(&levels) {
            return Err(invalid(format!(
                "an implicit subtree has {levels} levels; it admits 1 to 32"
            )));
        }
        if subtrees.scheme != scheme {
            return Err(invalid("the subtree template does not match the scheme"));
        }
        Ok(Self {
            scheme,
            levels,
            subtrees,
        })
    }

    fn to_value(&self) -> Value {
        serde_json::json!({
            "subdivisionScheme": self.scheme.as_str(),
            "subtreeLevels": self.levels,
            "availableLevels": self.levels,
            "subtrees": {"uri": self.subtrees.as_str()},
        })
    }
}

/// Where an explicit tile object sits in its document.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ObjectRole {
    /// A tile in a hierarchy, with its `transform`.
    Tile,
    /// The root of an external document: no `transform` (the link carries it).
    DocumentRoot,
}

/// One tile of a producer's hierarchy: placement, bounds, error, refinement, contents, and
/// the producer's typed `extras`, which must serialize to a JSON object of finite values.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct TileRecord<X> {
    placement: NodePlacement,
    bounds: TileBox,
    geometric_error: GeometricError,
    refine: Refine,
    contents: Vec<ContentRef>,
    extras: Option<X>,
}

/// The `extras` object of a typed value: refuses non-objects, non-finite numbers (which
/// serde_json would write as `null`) and `implicitChildIndex` (slots are structural:
/// [`TileTree::with_slots`]). An
/// empty object is `None`: it is never written.
fn extras_object<X: Serialize>(extras: Option<&X>) -> Result<Option<Map<String, Value>>, Error> {
    let Some(extras) = extras else {
        return Ok(None);
    };
    extras
        .serialize(finite::Probe)
        .map_err(|error| invalid(format!("extras: {error}")))?;
    match serde_json::to_value(extras)? {
        Value::Object(object) if object.contains_key("implicitChildIndex") => Err(invalid(
            "extras must not carry implicitChildIndex; child slots are structural (TileTree::with_slots)",
        )),
        Value::Object(object) if object.is_empty() => Ok(None),
        Value::Object(object) => Ok(Some(object)),
        other => Err(invalid(format!("extras must be an object, not {other}"))),
    }
}

fn set_contents(object: &mut Map<String, Value>, mut contents: Vec<Value>) {
    if contents.len() == 1 {
        object.insert("content".into(), contents.remove(0));
    } else if !contents.is_empty() {
        object.insert("contents".into(), Value::Array(contents));
    }
}

impl<X: Serialize> TileRecord<X> {
    pub(crate) fn new(
        placement: NodePlacement,
        bounds: TileBox,
        geometric_error: GeometricError,
        refine: Refine,
        contents: Vec<ContentRef>,
        extras: Option<X>,
    ) -> Result<Self, Error> {
        extras_object(extras.as_ref())?;
        Ok(Self {
            placement,
            bounds,
            geometric_error,
            refine,
            contents,
            extras,
        })
    }

    /// The empty dataset's root: a 1 mm origin-centred box, `REPLACE`, no content, and the
    /// structural error `0` (a JSON integer).
    pub(crate) fn empty_root(extras: Option<X>) -> Result<Self, Error> {
        Self::new(
            NodePlacement::INHERIT,
            TileBox::centred([0.001; 3])?,
            GeometricError {
                metres: 0.,
                structural: true,
            },
            Refine::Replace,
            Vec::new(),
            extras,
        )
    }

    pub(crate) fn placement(&self) -> NodePlacement {
        self.placement
    }

    pub(crate) fn bounds(&self) -> &TileBox {
        &self.bounds
    }

    pub(crate) fn geometric_error(&self) -> GeometricError {
        self.geometric_error
    }

    pub(crate) fn refine(&self) -> Refine {
        self.refine
    }

    pub(crate) fn contents(&self) -> &[ContentRef] {
        &self.contents
    }

    pub(crate) fn extras(&self) -> Option<&X> {
        self.extras.as_ref()
    }

    pub(crate) fn set_placement(&mut self, placement: NodePlacement) {
        self.placement = placement;
    }

    pub(crate) fn set_bounds(&mut self, bounds: TileBox) {
        self.bounds = bounds;
    }

    pub(crate) fn set_geometric_error(&mut self, geometric_error: GeometricError) {
        self.geometric_error = geometric_error;
    }

    pub(crate) fn set_refine(&mut self, refine: Refine) {
        self.refine = refine;
    }

    pub(crate) fn set_extras(&mut self, extras: Option<X>) -> Result<(), Error> {
        extras_object(extras.as_ref())?;
        self.extras = extras;
        Ok(())
    }

    /// The written `extras` object, if any (absent and empty extras are not written).
    pub(crate) fn extras_object(&self) -> Result<Option<Map<String, Value>>, Error> {
        extras_object(self.extras.as_ref())
    }

    fn base(&self, bounds: &TileBox, role: ObjectRole) -> Map<String, Value> {
        let mut object = Map::new();
        let mut volume = Map::new();
        volume.insert("box".into(), bounds.to_value());
        object.insert("boundingVolume".into(), Value::Object(volume));
        object.insert("geometricError".into(), self.geometric_error.to_value());
        object.insert("refine".into(), Value::from(self.refine.as_str()));
        if role == ObjectRole::Tile {
            if let Some(transform) = self.placement.to_value() {
                object.insert("transform".into(), transform);
            }
        }
        object
    }

    /// The explicit tile object without `children`, bounded by `bounds`: the record's own,
    /// or as the implicit layout widened them.
    pub(crate) fn explicit_object(
        &self,
        bounds: &TileBox,
        role: ObjectRole,
    ) -> Result<Map<String, Value>, Error> {
        let mut object = self.base(bounds, role);
        set_contents(
            &mut object,
            self.contents.iter().map(ContentRef::to_value).collect(),
        );
        if let Some(extras) = self.extras_object()? {
            object.insert("extras".into(), Value::Object(extras));
        }
        Ok(object)
    }

    /// The implicit root tile object: content templates in slot order and `implicitTiling`.
    /// It carries no `extras`; the root's are in its subtree row.
    pub(crate) fn implicit_root_object(
        &self,
        bounds: &TileBox,
        templates: &[ContentTemplate],
        tiling: &ImplicitTiling,
    ) -> Result<Map<String, Value>, Error> {
        if templates.iter().any(|t| t.uri.scheme != tiling.scheme) {
            return Err(invalid("a content template does not match the scheme"));
        }
        let mut object = self.base(bounds, ObjectRole::Tile);
        set_contents(
            &mut object,
            templates.iter().map(ContentTemplate::to_value).collect(),
        );
        object.insert("implicitTiling".into(), tiling.to_value());
        Ok(object)
    }

    /// An explicit link tile to the external document that this tile roots.
    pub(crate) fn link_object(
        &self,
        bounds: &TileBox,
        document: &MemberName,
    ) -> Map<String, Value> {
        let mut object = self.base(bounds, ObjectRole::Tile);
        let mut content = Map::new();
        content.insert("uri".into(), Value::from(document.as_str()));
        object.insert("content".into(), Value::Object(content));
        object
    }
}

/// The largest implicit child slot: an octree's Morton index `x + 2y + 4z` (a quadtree's
/// slots are 0 to 3).
pub(crate) const MAX_CHILD_SLOT: u8 = 7;

/// A producer's explicit hierarchy H: each tile's children in build order, which is also
/// the order of an explicit `children` array. A child's implicit slot (its Morton index in
/// the parent's subdivision, from which subtree availability is derived) is its index among
/// its siblings ([`TileTree::new`]) or explicit ([`TileTree::with_slots`]).
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct TileTree<X> {
    record: TileRecord<X>,
    children: Vec<TileTree<X>>,
    /// Explicit child slots, parallel to `children`; `None` for sibling-index slots.
    slots: Option<Vec<u8>>,
}

impl<X: Serialize> TileTree<X> {
    pub(crate) fn new(record: TileRecord<X>, children: Vec<TileTree<X>>) -> Self {
        Self {
            record,
            children,
            slots: None,
        }
    }

    /// A tile whose children occupy explicit implicit slots (sparse or packed octants), in
    /// the given order. A slot above [`MAX_CHILD_SLOT`] or a slot shared by two children is
    /// refused; the forest also refuses a slot outside its scheme (4 to 7 in a quadtree).
    #[cfg_attr(not(test), allow(dead_code))]
    pub(crate) fn with_slots(
        record: TileRecord<X>,
        children: Vec<(u8, TileTree<X>)>,
    ) -> Result<Self, Error> {
        let mut seen = 0u8;
        for (slot, _) in &children {
            if *slot > MAX_CHILD_SLOT {
                return Err(invalid(format!(
                    "child slot {slot} is outside every subdivision scheme (0 to {MAX_CHILD_SLOT})"
                )));
            }
            if seen & (1 << slot) != 0 {
                return Err(invalid(format!("duplicate child slot {slot}")));
            }
            seen |= 1 << slot;
        }
        let (slots, children) = children.into_iter().unzip();
        Ok(Self {
            record,
            children,
            slots: Some(slots),
        })
    }

    pub(crate) fn record(&self) -> &TileRecord<X> {
        &self.record
    }

    pub(crate) fn record_mut(&mut self) -> &mut TileRecord<X> {
        &mut self.record
    }

    pub(crate) fn children(&self) -> &[TileTree<X>] {
        &self.children
    }

    #[cfg(test)]
    pub(crate) fn children_mut(&mut self) -> &mut [TileTree<X>] {
        &mut self.children
    }

    /// The implicit slot of child `index`.
    pub(crate) fn slot(&self, index: usize) -> usize {
        self.slots
            .as_ref()
            .map_or(index, |slots| usize::from(slots[index]))
    }

    /// The tiles in preorder (children in order), without recursion: each record with its
    /// parent's preorder index and its implicit slot.
    pub(crate) fn preorder(&self) -> Vec<(&TileRecord<X>, Option<usize>, usize)> {
        let mut out = Vec::new();
        let mut stack = vec![(self, None, 0)];
        while let Some((tree, parent, slot)) = stack.pop() {
            let id = out.len();
            out.push((&tree.record, parent, slot));
            for (index, child) in tree.children.iter().enumerate().rev() {
                stack.push((child, Some(id), tree.slot(index)));
            }
        }
        out
    }

    /// H as an explicit tile object, children nested.
    pub(crate) fn to_object(&self) -> Result<Map<String, Value>, Error> {
        let mut object = self
            .record
            .explicit_object(&self.record.bounds, ObjectRole::Tile)?;
        if !self.children.is_empty() {
            let children = self
                .children
                .iter()
                .map(|child| child.to_object().map(Value::Object))
                .collect::<Result<_, Error>>()?;
            object.insert("children".into(), Value::Array(children));
        }
        Ok(object)
    }

    pub(crate) fn to_value(&self) -> Result<Value, Error> {
        self.to_object().map(Value::Object)
    }
}

/// A tileset-level extension a document may declare in `extensionsUsed`.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub(crate) enum TilesetExtension {
    ContentGltfVector,
}

impl TilesetExtension {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::ContentGltfVector => CONTENT_GLTF_VECTOR,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
enum DocumentKind {
    /// `tileset.json`, optionally with the producer's build-state digest and its HLOD
    /// declaration (`rustyTiles.lod`) in `asset.extras`.
    Root {
        build_state: Option<[u8; 32]>,
        hlod: bool,
    },
    /// An external document `tileset-d{m}.json`.
    External,
}

/// A tileset document's members other than `root`, which the layout assembles from tile
/// objects emitted by [`TileRecord`].
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct TilesetDocument {
    kind: DocumentKind,
    geometric_error: GeometricError,
    extensions_used: BTreeSet<TilesetExtension>,
    schema: bool,
}

/// The tile metadata schema of implicit subtrees (`metadata::tile_schema`).
pub(crate) fn tile_schema() -> Value {
    serde_json::to_value(crate::metadata::tile_schema()).expect("tile schema serializes")
}

impl TilesetDocument {
    /// `tileset.json`, whose tileset-level error is `geometric_error` ([`top_level_error`]).
    pub(crate) fn root(
        geometric_error: GeometricError,
        extensions_used: impl IntoIterator<Item = TilesetExtension>,
    ) -> Self {
        Self {
            kind: DocumentKind::Root {
                build_state: None,
                hlod: false,
            },
            geometric_error,
            extensions_used: extensions_used.into_iter().collect(),
            schema: false,
        }
    }

    /// An external document, whose error is its root tile's.
    pub(crate) fn external(
        geometric_error: GeometricError,
        extensions_used: impl IntoIterator<Item = TilesetExtension>,
    ) -> Self {
        Self {
            kind: DocumentKind::External,
            geometric_error,
            extensions_used: extensions_used.into_iter().collect(),
            schema: false,
        }
    }

    /// Attach the implicit tile metadata [`tile_schema`].
    pub(crate) fn with_schema(mut self, schema: bool) -> Self {
        self.schema = schema;
        self
    }

    /// Record the producer's build state digest as `asset.extras.vectorBuildStateSha256`.
    pub(crate) fn with_build_state(mut self, sha256: [u8; 32]) -> Result<Self, Error> {
        match &mut self.kind {
            DocumentKind::Root { build_state, .. } => *build_state = Some(sha256),
            DocumentKind::External => {
                return Err(invalid("only tileset.json records the build state"))
            }
        }
        Ok(self)
    }

    /// Declare the archive an HLOD delivery (`asset.extras.rustyTiles.lod = "hlod"`), which
    /// makes a validator hold every payload to HLOD identity and continuity.
    pub(crate) fn declaring_hlod(mut self) -> Result<Self, Error> {
        match &mut self.kind {
            DocumentKind::Root { hlod, .. } => *hlod = true,
            DocumentKind::External => {
                return Err(invalid("only tileset.json declares the archive's LOD"))
            }
        }
        Ok(self)
    }

    #[cfg(test)]
    pub(crate) fn geometric_error(&self) -> GeometricError {
        self.geometric_error
    }

    pub(crate) fn is_root(&self) -> bool {
        matches!(self.kind, DocumentKind::Root { .. })
    }

    pub(crate) fn build_state(&self) -> Option<&[u8; 32]> {
        match &self.kind {
            DocumentKind::Root { build_state, .. } => build_state.as_ref(),
            DocumentKind::External => None,
        }
    }

    pub(crate) fn extensions_used(&self) -> &BTreeSet<TilesetExtension> {
        &self.extensions_used
    }

    /// Every member except `root`.
    pub(crate) fn head(&self) -> Map<String, Value> {
        let mut asset = Map::new();
        asset.insert("version".into(), Value::from("1.1"));
        if let DocumentKind::Root { build_state, hlod } = &self.kind {
            let mut extras = Map::new();
            if *hlod {
                extras.insert("rustyTiles".into(), serde_json::json!({"lod": "hlod"}));
            }
            if let Some(sha256) = build_state {
                extras.insert("vectorBuildStateSha256".into(), Value::from(hex(sha256)));
            }
            if !extras.is_empty() {
                asset.insert("extras".into(), Value::Object(extras));
            }
        }
        let mut head = Map::new();
        head.insert("asset".into(), Value::Object(asset));
        if !self.extensions_used.is_empty() {
            head.insert(
                "extensionsUsed".into(),
                self.extensions_used
                    .iter()
                    .map(|e| Value::from(e.as_str()))
                    .collect(),
            );
        }
        head.insert("geometricError".into(), self.geometric_error.to_value());
        if self.schema {
            head.insert("schema".into(), tile_schema());
        }
        head
    }

    /// The document around an assembled `root` tile object.
    pub(crate) fn to_value(&self, root: Map<String, Value>) -> Value {
        let mut document = self.head();
        document.insert("root".into(), Value::Object(root));
        Value::Object(document)
    }
}

/// A serializer that writes nothing and refuses non-finite floats, which serde_json would
/// otherwise write as `null`.
mod finite {
    use serde::ser::{self, Serialize};
    use std::fmt;

    #[derive(Debug)]
    pub(super) struct NonFinite(String);

    impl fmt::Display for NonFinite {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            f.write_str(&self.0)
        }
    }

    impl std::error::Error for NonFinite {}

    impl ser::Error for NonFinite {
        fn custom<T: fmt::Display>(message: T) -> Self {
            Self(message.to_string())
        }
    }

    pub(super) struct Probe;

    type Done = Result<(), NonFinite>;

    fn number(value: f64) -> Done {
        if value.is_finite() {
            Ok(())
        } else {
            Err(NonFinite(format!("{value} is not a finite number")))
        }
    }

    macro_rules! accept {
        ($($method:ident: $type:ty),* $(,)?) => {
            $(fn $method(self, _: $type) -> Done { Ok(()) })*
        };
    }

    impl ser::Serializer for Probe {
        type Ok = ();
        type Error = NonFinite;
        type SerializeSeq = Probe;
        type SerializeTuple = Probe;
        type SerializeTupleStruct = Probe;
        type SerializeTupleVariant = Probe;
        type SerializeMap = Probe;
        type SerializeStruct = Probe;
        type SerializeStructVariant = Probe;

        accept!(serialize_bool: bool, serialize_i8: i8, serialize_i16: i16, serialize_i32: i32,
            serialize_i64: i64, serialize_i128: i128, serialize_u8: u8, serialize_u16: u16,
            serialize_u32: u32, serialize_u64: u64, serialize_u128: u128, serialize_char: char,
            serialize_str: &str, serialize_bytes: &[u8], serialize_unit_struct: &'static str);

        fn serialize_f32(self, value: f32) -> Done {
            number(value.into())
        }
        fn serialize_f64(self, value: f64) -> Done {
            number(value)
        }
        fn serialize_none(self) -> Done {
            Ok(())
        }
        fn serialize_some<T: ?Sized + Serialize>(self, value: &T) -> Done {
            value.serialize(self)
        }
        fn serialize_unit(self) -> Done {
            Ok(())
        }
        fn serialize_unit_variant(self, _: &'static str, _: u32, _: &'static str) -> Done {
            Ok(())
        }
        fn serialize_newtype_struct<T: ?Sized + Serialize>(
            self,
            _: &'static str,
            value: &T,
        ) -> Done {
            value.serialize(self)
        }
        fn serialize_newtype_variant<T: ?Sized + Serialize>(
            self,
            _: &'static str,
            _: u32,
            _: &'static str,
            value: &T,
        ) -> Done {
            value.serialize(self)
        }
        fn serialize_seq(self, _: Option<usize>) -> Result<Probe, NonFinite> {
            Ok(self)
        }
        fn serialize_tuple(self, _: usize) -> Result<Probe, NonFinite> {
            Ok(self)
        }
        fn serialize_tuple_struct(self, _: &'static str, _: usize) -> Result<Probe, NonFinite> {
            Ok(self)
        }
        fn serialize_tuple_variant(
            self,
            _: &'static str,
            _: u32,
            _: &'static str,
            _: usize,
        ) -> Result<Probe, NonFinite> {
            Ok(self)
        }
        fn serialize_map(self, _: Option<usize>) -> Result<Probe, NonFinite> {
            Ok(self)
        }
        fn serialize_struct(self, _: &'static str, _: usize) -> Result<Probe, NonFinite> {
            Ok(self)
        }
        fn serialize_struct_variant(
            self,
            _: &'static str,
            _: u32,
            _: &'static str,
            _: usize,
        ) -> Result<Probe, NonFinite> {
            Ok(self)
        }
    }

    macro_rules! elements {
        ($($trait:ident :: $method:ident),* $(,)?) => {$(
            impl ser::$trait for Probe {
                type Ok = ();
                type Error = NonFinite;
                fn $method<T: ?Sized + Serialize>(&mut self, value: &T) -> Done {
                    value.serialize(Probe)
                }
                fn end(self) -> Done {
                    Ok(())
                }
            }
        )*};
    }

    elements!(
        SerializeSeq::serialize_element,
        SerializeTuple::serialize_element,
        SerializeTupleStruct::serialize_field,
        SerializeTupleVariant::serialize_field,
    );

    impl ser::SerializeMap for Probe {
        type Ok = ();
        type Error = NonFinite;
        fn serialize_key<T: ?Sized + Serialize>(&mut self, key: &T) -> Done {
            key.serialize(Probe)
        }
        fn serialize_value<T: ?Sized + Serialize>(&mut self, value: &T) -> Done {
            value.serialize(Probe)
        }
        fn end(self) -> Done {
            Ok(())
        }
    }

    macro_rules! fields {
        ($($trait:ident),* $(,)?) => {$(
            impl ser::$trait for Probe {
                type Ok = ();
                type Error = NonFinite;
                fn serialize_field<T: ?Sized + Serialize>(&mut self, _: &'static str, value: &T) -> Done {
                    value.serialize(Probe)
                }
                fn end(self) -> Done {
                    Ok(())
                }
            }
        )*};
    }

    fields!(SerializeStruct, SerializeStructVariant);
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn json_forms_match_published_tilesets() {
        let half = [1.5, 2., 0.25];
        assert_eq!(
            box_json(0, half).to_string(),
            json!([0, 0, 0, 1.5, 0, 0, 0, 2., 0, 0, 0, 0.25]).to_string()
        );
        assert_eq!(
            box_json(0., half).to_string(),
            json!([0., 0., 0., 1.5, 0., 0., 0., 2., 0., 0., 0., 0.25]).to_string()
        );
        let node = json!({"transform": translation([1., -2., 3.5])});
        assert_eq!(
            node["transform"].to_string(),
            "[1,0,0,0,0,1,0,0,0,0,1,0,1.0,-2.0,3.5,1]"
        );
        assert_eq!(translation_offset(&node).unwrap(), [1., -2., 3.5]);
    }

    #[test]
    fn children_grow_parent_bounds_and_errors_or_are_rejected() {
        let child = json!({"boundingVolume":{"box":box_json(0, [1., 1., 1.])},"geometricError":4.});
        let mut half = [0.5; 3];
        let error = enclose_children(&mut half, 2., [([-3., 0., 0.25], &child)]).unwrap();
        assert_eq!((half, error), ([4., 1., 1.25], 4.));
        let broken = json!({"boundingVolume":{"box":[0, 0, 0]}});
        assert!(matches!(
            enclose_children(&mut half, 0., [([0.; 3], &broken)]),
            Err(Error::Data(_))
        ));
        assert_eq!(top_level_error(0.5, 0.25, 2.), 1.);
    }

    #[test]
    fn child_enclosure_rounds_outward() {
        // 1 + 2^-53 is a tie that rounds to nearest (even) 1.0, half an ulp inside the child.
        let child = json!({"boundingVolume":{"box":box_json(0., [2f64.powi(-53); 3])}});
        let mut half = [0.; 3];
        enclose_children(&mut half, 0., [([1., -1., 0.], &child)]).unwrap();
        assert_eq!(half, [1f64.next_up(), 1f64.next_up(), 2f64.powi(-53)]);
        // Exact and rounded-up sums are unchanged.
        assert_eq!(sum_up(0.5, 0.25), 0.75);
        assert_eq!(
            sum_up(1., 2f64.powi(-52) + 2f64.powi(-53)),
            1. + 2f64.powi(-51)
        );
        assert_eq!(sum_up(0.1, 0.2), 0.1 + 0.2);
        assert_eq!(top_level_error(3., 2., 2.), 4.);
    }
}

#[cfg(test)]
mod record_tests {
    use super::*;
    use serde_json::json;

    #[derive(Clone, Debug, PartialEq, Serialize)]
    #[serde(rename_all = "camelCase")]
    struct Extras {
        build_path: &'static str,
        vertices: u32,
        tolerance_metres: f64,
        #[serde(skip_serializing_if = "Option::is_none")]
        routing_reason: Option<Option<&'static str>>,
    }

    fn extras(build_path: &'static str) -> Option<Extras> {
        Some(Extras {
            build_path,
            vertices: 3,
            tolerance_metres: 0.5,
            routing_reason: None,
        })
    }

    fn content(uri: &str) -> ContentRef {
        ContentRef::new(
            MemberName::new(uri).unwrap(),
            [7; 32],
            64,
            ContentKind::Glb,
            true,
            SourceIdentity::Features {
                count: 2,
                digest: [1; 32],
            },
        )
        .unwrap()
    }

    fn record(half: Vec3, error: f64, contents: Vec<ContentRef>) -> TileRecord<Extras> {
        TileRecord::new(
            NodePlacement::INHERIT,
            TileBox::centred(half).unwrap(),
            GeometricError::new(error).unwrap(),
            Refine::Replace,
            contents,
            extras("0"),
        )
        .unwrap()
    }

    /// A right-handed orthonormal frame with exact components: E = +y, N = −x, U = +z.
    const AXES: [Vec3; 3] = [[0., 1., 0.], [-1., 0., 0.], [0., 0., 1.]];

    fn is_data<T: std::fmt::Debug>(result: Result<T, Error>) -> String {
        match result {
            Err(Error::Data(message)) => message,
            other => panic!("expected a typed data error, got {other:?}"),
        }
    }

    #[test]
    fn maps_sort_keys_whatever_the_insertion_order() {
        // A guard against `preserve_order` arriving through feature unification.
        let mut map = Map::new();
        map.insert("b".into(), json!(1));
        map.insert("a".into(), json!(2));
        assert_eq!(Value::Object(map).to_string(), r#"{"a":2,"b":1}"#);
    }

    #[test]
    fn golden_tree_and_documents() {
        let mut child = record([0.5, 0.5, 0.25], 0., vec![content("t/child.glb")]);
        child.set_placement(NodePlacement::translation([1., -2., 0.5]).unwrap());
        let mut root = record([1.5, 2., 0.25], 4., vec![content("t/root.glb")]);
        root.set_placement(NodePlacement::rigid(AXES, [4_500_000., 600_000., 4_400_000.]).unwrap());
        let tree = TileTree::new(root, vec![TileTree::new(child, Vec::new())]);
        let marker = r#"{"extensions":{"3DTILES_content_gltf_vector":{"vector":true}},"uri":"#;
        let extras = r#""extras":{"buildPath":"0","toleranceMetres":0.5,"vertices":3}"#;
        let expected_child = format!(
            r#"{{"boundingVolume":{{"box":[0,0,0,0.5,0,0,0,0.5,0,0,0,0.25]}},"content":{marker}"t/child.glb"}},{extras},"geometricError":0.0,"refine":"REPLACE","transform":[1,0,0,0,0,1,0,0,0,0,1,0,1.0,-2.0,0.5,1]}}"#
        );
        let expected = format!(
            r#"{{"boundingVolume":{{"box":[0,0,0,1.5,0,0,0,2.0,0,0,0,0.25]}},"children":[{expected_child}],"content":{marker}"t/root.glb"}},{extras},"geometricError":4.0,"refine":"REPLACE","transform":[0.0,1.0,0.0,0,-1.0,0.0,0.0,0,0.0,0.0,1.0,0,4500000.0,600000.0,4400000.0,1]}}"#
        );
        assert_eq!(tree.to_value().unwrap().to_string(), expected);
        // The preorder records the parent's index and the child's slot.
        let order: Vec<_> = tree
            .preorder()
            .into_iter()
            .map(|(_, p, i)| (p, i))
            .collect();
        assert_eq!(order, [(None, 0), (Some(0), 0)]);

        let document = TilesetDocument::root(
            GeometricError::new(9.).unwrap(),
            [TilesetExtension::ContentGltfVector],
        );
        assert_eq!(
            document.to_value(tree.to_object().unwrap()).to_string(),
            format!(
                r#"{{"asset":{{"version":"1.1"}},"extensionsUsed":["3DTILES_content_gltf_vector"],"geometricError":9.0,"root":{expected}}}"#
            )
        );
        let stamped = document.clone().with_build_state([0xab; 32]).unwrap();
        assert_eq!(
            Value::Object(stamped.head())["asset"].to_string(),
            format!(
                r#"{{"extras":{{"vectorBuildStateSha256":"{}"}},"version":"1.1"}}"#,
                "ab".repeat(32)
            )
        );
        let external = TilesetDocument::external(
            GeometricError::new(2.5).unwrap(),
            [TilesetExtension::ContentGltfVector],
        );
        assert!(!external.head().contains_key("schema"));
        let head = external.clone().with_schema(true).head();
        assert_eq!(
            head["schema"],
            serde_json::to_value(crate::metadata::tile_schema()).unwrap()
        );
        assert_eq!(head["geometricError"].to_string(), "2.5");
        assert!(is_data(external.clone().with_build_state([0; 32])).contains("build state"));
        let declared = document.clone().declaring_hlod().unwrap();
        assert_eq!(
            Value::Object(declared.head())["asset"].to_string(),
            r#"{"extras":{"rustyTiles":{"lod":"hlod"}},"version":"1.1"}"#
        );
        assert!(is_data(external.declaring_hlod()).contains("declares"));
        let empty = TilesetDocument::root(GeometricError::new(1.).unwrap(), []);
        assert!(!empty.head().contains_key("extensionsUsed"));
    }

    #[test]
    fn role_objects_have_their_exact_key_sets() {
        let mut record = record([2., 3., 1.], 1.25, vec![content("t/a.glb")]);
        record.set_placement(NodePlacement::translation([0.; 3]).unwrap());
        let bounds = *record.bounds();
        let scheme = SubdivisionScheme::Quadtree;
        let template =
            UriTemplate::new("implicit-content/r3-{level}-{x}-{y}-0.glb", scheme).unwrap();
        let tiling = ImplicitTiling::single_subtree(
            scheme,
            4,
            UriTemplate::new("subtrees/r3-{level}-{x}-{y}.subtree", scheme).unwrap(),
        )
        .unwrap();
        let object = record
            .implicit_root_object(&bounds, &[ContentTemplate::new(template, true)], &tiling)
            .unwrap();
        assert_eq!(
            Value::Object(object).to_string(),
            r#"{"boundingVolume":{"box":[0,0,0,2.0,0,0,0,3.0,0,0,0,1.0]},"content":{"extensions":{"3DTILES_content_gltf_vector":{"vector":true}},"uri":"implicit-content/r3-{level}-{x}-{y}-0.glb"},"geometricError":1.25,"implicitTiling":{"availableLevels":4,"subdivisionScheme":"QUADTREE","subtreeLevels":4,"subtrees":{"uri":"subtrees/r3-{level}-{x}-{y}.subtree"}},"refine":"REPLACE","transform":[1,0,0,0,0,1,0,0,0,0,1,0,0.0,0.0,0.0,1]}"#
        );
        let link = record.link_object(&bounds, &MemberName::new("tileset-d1.json").unwrap());
        assert_eq!(
            Value::Object(link).to_string(),
            r#"{"boundingVolume":{"box":[0,0,0,2.0,0,0,0,3.0,0,0,0,1.0]},"content":{"uri":"tileset-d1.json"},"geometricError":1.25,"refine":"REPLACE","transform":[1,0,0,0,0,1,0,0,0,0,1,0,0.0,0.0,0.0,1]}"#
        );
        let root = record
            .explicit_object(&bounds, ObjectRole::DocumentRoot)
            .unwrap();
        let keys: Vec<_> = root.keys().map(String::as_str).collect();
        assert_eq!(
            keys,
            [
                "boundingVolume",
                "content",
                "extras",
                "geometricError",
                "refine"
            ]
        );
        // Several contents are a `contents` array; none is no member; a widened box keeps
        // its integer zeros.
        let mut many = record.clone();
        many.contents = vec![content("t/a.glb"), content("t/b.glb")];
        let widened = bounds
            .with_half_lanes(&[0., 0., 0., 2f64.next_up(), 0., 0., 0., 3., 0., 0., 0., 1.])
            .unwrap();
        let object = many.explicit_object(&widened, ObjectRole::Tile).unwrap();
        assert_eq!(object["contents"].as_array().unwrap().len(), 2);
        assert!(!object.contains_key("content"));
        assert_eq!(
            object["boundingVolume"]["box"].to_string(),
            "[0,0,0,2.0000000000000004,0,0,0,3.0,0,0,0,1.0]"
        );
        many.contents.clear();
        let object = many.explicit_object(&bounds, ObjectRole::Tile).unwrap();
        assert!(!object.contains_key("content") && !object.contains_key("contents"));
        // An offset box writes its measured centre as f64, its other zeros as integers.
        let offset = TileBox::offset([0., -1.5, 2.], [1., 1., 1.]).unwrap();
        assert_eq!(
            offset.to_value().to_string(),
            "[0.0,-1.5,2.0,1.0,0,0,0,1.0,0,0,0,1.0]"
        );
        assert_eq!(
            offset.lanes(),
            [0., -1.5, 2., 1., 0., 0., 0., 1., 0., 0., 0., 1.]
        );
    }

    #[test]
    fn empty_root_keeps_the_structural_integer_error() {
        // The literal `store.rs` writes for an empty dataset, then its placement.
        let mut root = TileRecord::<Extras>::empty_root(None).unwrap();
        root.set_placement(NodePlacement::rigid(AXES, [1., 2., 3.]).unwrap());
        let mut legacy = json!({"boundingVolume":{"box":[0,0,0,0.001,0,0,0,0.001,0,0,0,0.001]},"geometricError":0,"refine":"REPLACE"});
        legacy["transform"] = json!([
            AXES[0][0], AXES[0][1], AXES[0][2], 0, AXES[1][0], AXES[1][1], AXES[1][2], 0,
            AXES[2][0], AXES[2][1], AXES[2][2], 0, 1., 2., 3., 1
        ]);
        let tree = TileTree::new(root, Vec::new());
        assert_eq!(
            serde_json::to_vec(&tree.to_value().unwrap()).unwrap(),
            serde_json::to_vec(&legacy).unwrap()
        );
        assert_eq!(tree.record().geometric_error().metres(), 0.);
        // A measured zero is f64; `max` is measured.
        let zero = GeometricError::new(0.).unwrap();
        assert_eq!(zero.to_value().to_string(), "0.0");
        assert_eq!(
            tree.record()
                .geometric_error()
                .max(zero)
                .to_value()
                .to_string(),
            "0.0"
        );
    }

    #[test]
    fn number_forms_equal_the_legacy_writers() {
        let values = [
            0.,
            -0.,
            1.,
            0.1 + 0.2,
            1e-7,
            2f64.powi(-17),
            1e21,
            123_456_789.123,
            f64::MIN_POSITIVE,
            5e-324,
            f64::MAX,
            0.001 + 2f64.powi(-20),
        ];
        for v in values {
            let half = [v, v.abs(), v];
            assert_eq!(
                TileBox::centred(half).unwrap().to_value().to_string(),
                box_json(0, half).to_string(),
                "{v:e}"
            );
            let delta = [v, -v, v];
            assert_eq!(
                NodePlacement::translation(delta)
                    .unwrap()
                    .to_value()
                    .unwrap()
                    .to_string(),
                translation(delta).to_string()
            );
            assert_eq!(
                GeometricError::new(v.abs()).unwrap().to_value().to_string(),
                serde_json::json!(v.abs()).to_string()
            );
        }
        assert_eq!(
            GeometricError::new(0.1 + 0.2)
                .unwrap()
                .to_value()
                .to_string(),
            "0.30000000000000004"
        );
        assert_eq!(
            GeometricError::new(1e-7).unwrap().to_value().to_string(),
            "1e-7"
        );
        assert_eq!(
            GeometricError::new(1e21).unwrap().to_value().to_string(),
            "1e+21"
        );
        assert_eq!(
            NodePlacement::translation([-0., 0., 0.])
                .unwrap()
                .to_value()
                .unwrap()
                .to_string(),
            "[1,0,0,0,0,1,0,0,0,0,1,0,-0.0,0.0,0.0,1]"
        );
        assert_eq!(NodePlacement::INHERIT.to_value(), None);
        assert_eq!(NodePlacement::INHERIT.child_translation(), Some([0.; 3]));
        let rigid = NodePlacement::rigid(AXES, [0.; 3]).unwrap();
        assert_eq!(rigid.child_translation(), None);
    }

    #[test]
    fn enclosure_is_the_legacy_enclosure() {
        // xorshift64*: deterministic inputs over many magnitudes, with ties and exact sums.
        let mut state = 0x9E37_79B9_7F4A_7C15u64;
        let mut next = move || {
            state ^= state >> 12;
            state ^= state << 25;
            state ^= state >> 27;
            state.wrapping_mul(0x2545_F491_4F6C_DD1D)
        };
        let sample = |next: &mut dyn FnMut() -> u64| {
            let bits = next();
            let magnitude = 2f64.powi((bits % 80) as i32 - 50);
            let mantissa = (bits >> 11) as f64 / (1u64 << 53) as f64;
            match bits % 7 {
                0 => 0.,
                1 => magnitude,
                2 => 2f64.powi(-53),
                _ => mantissa * magnitude,
            }
        };
        for case in 0..4000 {
            let half: Vec3 = std::array::from_fn(|_| sample(&mut next));
            let error = sample(&mut next);
            let children: Vec<(Vec3, Vec3, f64)> = (0..1 + case % 3)
                .map(|_| {
                    let sign = |next: &mut dyn FnMut() -> u64| {
                        if next().is_multiple_of(2) {
                            1.
                        } else {
                            -1.
                        }
                    };
                    let delta: Vec3 = std::array::from_fn(|_| sign(&mut next) * sample(&mut next));
                    let child: Vec3 = std::array::from_fn(|_| sample(&mut next));
                    (delta, child, sample(&mut next))
                })
                .collect();
            let values: Vec<Value> = children
                .iter()
                .map(|(_, child, error)| {
                    json!({"boundingVolume":{"box":box_json(0, *child)},"geometricError":error})
                })
                .collect();
            let mut legacy_half = half;
            let legacy_error = enclose_children(
                &mut legacy_half,
                error,
                children.iter().map(|c| c.0).zip(values.iter()),
            )
            .unwrap();
            let mut bounds = TileBox::centred(half).unwrap();
            let mut typed_error = GeometricError::new(error).unwrap();
            for (delta, child, child_error) in &children {
                bounds
                    .grow_to_enclose(*delta, &TileBox::centred(*child).unwrap())
                    .unwrap();
                typed_error = typed_error.max(GeometricError::new(*child_error).unwrap());
            }
            assert_eq!(
                bounds.half().map(f64::to_bits),
                legacy_half.map(f64::to_bits),
                "case {case}"
            );
            assert_eq!(typed_error.metres().to_bits(), legacy_error.to_bits());
        }
        // The tie from `child_enclosure_rounds_outward`.
        let mut bounds = TileBox::centred([0.; 3]).unwrap();
        let child = TileBox::centred([2f64.powi(-53); 3]).unwrap();
        bounds.grow_to_enclose([1., -1., 0.], &child).unwrap();
        assert_eq!(
            bounds.half(),
            [1f64.next_up(), 1f64.next_up(), 2f64.powi(-53)]
        );
        assert_eq!(bounds.diagonal(), crate::vec3::norm(bounds.half()) * 2.);
    }

    #[test]
    fn non_finite_and_invalid_values_are_typed_errors() {
        for bad in [f64::NAN, f64::INFINITY, -1., -f64::MIN_POSITIVE] {
            assert!(is_data(GeometricError::new(bad)).contains("geometric error"));
            assert!(is_data(TileBox::centred([1., bad, 1.])).contains("half lengths"));
        }
        for bad in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
            assert!(is_data(TileBox::offset([bad, 0., 0.], [1.; 3])).contains("centre"));
            assert!(is_data(NodePlacement::translation([0., bad, 0.])).contains("not finite"));
            assert!(is_data(NodePlacement::rigid(AXES, [bad, 0., 0.])).contains("not finite"));
            let mut axes = AXES;
            axes[2][2] = bad;
            assert!(is_data(NodePlacement::rigid(axes, [0.; 3])).contains("not finite"));
        }
        let mut stretched = AXES;
        stretched[0][1] = 1. + 1e-9;
        assert!(is_data(NodePlacement::rigid(stretched, [0.; 3])).contains("orthonormal"));
        let mut skewed = AXES;
        skewed[1] = [-1., 1e-9, 0.];
        assert!(is_data(NodePlacement::rigid(skewed, [0.; 3])).contains("orthonormal"));
        let mut mirrored = AXES;
        mirrored[2] = [0., 0., -1.];
        assert!(is_data(NodePlacement::rigid(mirrored, [0.; 3])).contains("right-handed"));
        // Within the 1e-12 bound is admitted.
        let mut close = AXES;
        close[0][1] = 1. + 2e-13;
        NodePlacement::rigid(close, [0.; 3]).unwrap();

        // Enclosure that overflows, or of offset boxes, is refused.
        let mut huge = TileBox::centred([f64::MAX; 3]).unwrap();
        let child = TileBox::centred([f64::MAX; 3]).unwrap();
        assert!(is_data(huge.grow_to_enclose([f64::MAX, 0., 0.], &child)).contains("finite"));
        let mut centred = TileBox::centred([1.; 3]).unwrap();
        let offset = TileBox::offset([1.; 3], [1.; 3]).unwrap();
        assert!(is_data(centred.grow_to_enclose([0.; 3], &offset)).contains("origin-centred"));
        assert!(is_data(centred.grow_to_enclose([f64::NAN, 0., 0.], &child)).contains("finite"));
        let lanes = [1., 0., 0., 2., 0., 0., 0., 2., 0., 0., 0., 2.];
        assert!(is_data(centred.with_half_lanes(&lanes)).contains("centre"));
        let mut lanes = centred.lanes();
        lanes[7] = f64::NAN;
        assert!(is_data(centred.with_half_lanes(&lanes)).contains("half lengths"));
    }

    #[test]
    fn member_names_and_templates_are_relative_and_closed() {
        for good in [
            "t/0123abcdef.glb",
            "implicit-content/r0-1-0-0-0.glb",
            "tileset-d1.json",
            "a/b~c/d_e.f",
        ] {
            assert_eq!(MemberName::new(good).unwrap().as_str(), good);
        }
        for bad in [
            "",
            "/abs.glb",
            "./x.glb",
            "../x.glb",
            "a/../b.glb",
            "a/./b.glb",
            "a//b.glb",
            "a/",
            ".hidden",
            "http://host/x.glb",
            "c:/x.glb",
            "a\\b.glb",
            "a b.glb",
            "a?b.glb",
            "a#b.glb",
            "a%2eb.glb",
            "a{level}.glb",
            "é.glb",
        ] {
            assert!(
                is_data(MemberName::new(bad)).contains("member name"),
                "{bad:?}"
            );
        }
        let quad = SubdivisionScheme::Quadtree;
        let oct = SubdivisionScheme::Octree;
        UriTemplate::new("subtrees/r0-{level}-{x}-{y}.subtree", quad).unwrap();
        UriTemplate::new("c/{z}/{y}/{x}/{level}.glb", oct).unwrap();
        for (bad, scheme, why) in [
            ("s/{level}-{x}.subtree", quad, "lacks"),
            ("s/{level}-{x}-{y}.subtree", oct, "lacks"),
            ("s/{level}-{x}-{y}-{z}.subtree", quad, "outside"),
            ("s/{level}-{x}-{y}-{slot}.glb", quad, "outside"),
            ("s/{level-{x}-{y}.glb", quad, "outside"),
            ("s/{level}-{x}-{y}{.glb", quad, "unmatched"),
            ("s/}{level}-{x}-{y}.glb", quad, "unmatched"),
            ("../{level}-{x}-{y}.glb", quad, "must not start"),
            ("s/../{level}-{x}-{y}.glb", quad, "segment"),
            ("/{level}-{x}-{y}.glb", quad, "segment"),
            ("s?{level}-{x}-{y}.glb", quad, "character"),
        ] {
            let message = is_data(UriTemplate::new(bad, scheme));
            assert!(message.contains(why), "{bad:?}: {message}");
        }
        assert!(is_data(ImplicitTiling::single_subtree(
            oct,
            3,
            UriTemplate::new("s/{level}-{x}-{y}.subtree", quad).unwrap()
        ))
        .contains("scheme"));
        for levels in [0, 33] {
            assert!(is_data(ImplicitTiling::single_subtree(
                quad,
                levels,
                UriTemplate::new("s/{level}-{x}-{y}.subtree", quad).unwrap()
            ))
            .contains("1 to 32"));
        }
        let tiling = ImplicitTiling::single_subtree(
            quad,
            2,
            UriTemplate::new("s/{level}-{x}-{y}.subtree", quad).unwrap(),
        )
        .unwrap();
        let octree_template = ContentTemplate::new(
            UriTemplate::new("c/{level}-{x}-{y}-{z}.glb", oct).unwrap(),
            true,
        );
        let record = record([1.; 3], 1., Vec::new());
        assert!(
            is_data(record.implicit_root_object(record.bounds(), &[octree_template], &tiling))
                .contains("scheme")
        );
    }

    #[test]
    fn contents_and_extras_are_validated() {
        let name = |uri: &str| MemberName::new(uri).unwrap();
        let features = SourceIdentity::Features {
            count: 1,
            digest: [0; 32],
        };
        let make = |uri: &str, length: u64, sources: SourceIdentity| {
            ContentRef::new(name(uri), [0; 32], length, ContentKind::Glb, true, sources)
        };
        assert!(is_data(make("t/a.b3dm", 64, features)).contains(".glb"));
        assert!(is_data(make("t/glb", 64, features)).contains(".glb"));
        assert!(is_data(make("t/a.glb", 11, features)).contains("GLB header"));
        let none = SourceIdentity::Features {
            count: 0,
            digest: [0; 32],
        };
        assert!(is_data(make("t/a.glb", 64, none)).contains("no source features"));
        let aggregate = make("t/a.glb", 12, SourceIdentity::Aggregate).unwrap();
        assert_eq!(
            (
                aggregate.byte_length(),
                aggregate.kind(),
                aggregate.vector()
            ),
            (12, ContentKind::Glb, true)
        );
        assert_eq!(aggregate.sources(), SourceIdentity::Aggregate);
        // Source identity is never serialized.
        assert_eq!(
            aggregate.to_value().to_string(),
            r#"{"extensions":{"3DTILES_content_gltf_vector":{"vector":true}},"uri":"t/a.glb"}"#
        );
        let plain = ContentRef::new(
            name("t/a.glb"),
            [0; 32],
            64,
            ContentKind::Glb,
            false,
            features,
        )
        .unwrap();
        assert_eq!(plain.to_value().to_string(), r#"{"uri":"t/a.glb"}"#);

        let build = |extras: Option<Value>| {
            TileRecord::new(
                NodePlacement::INHERIT,
                TileBox::centred([1.; 3]).unwrap(),
                GeometricError::new(1.).unwrap(),
                Refine::Add,
                Vec::new(),
                extras,
            )
        };
        assert!(is_data(build(Some(json!(1)))).contains("must be an object"));
        assert!(
            is_data(build(Some(json!({"implicitChildIndex": 1})))).contains("implicitChildIndex")
        );
        // Null members (for example a routing tile's absent reason) are admitted.
        let routing = build(Some(json!({"routing": true, "routingReason": null}))).unwrap();
        assert_eq!(
            Value::Object(routing.extras_object().unwrap().unwrap()).to_string(),
            r#"{"routing":true,"routingReason":null}"#
        );
        assert_eq!(routing.refine().as_str(), "ADD");
        // Empty extras are not written.
        let empty = build(Some(json!({}))).unwrap();
        assert!(!empty
            .explicit_object(empty.bounds(), ObjectRole::Tile)
            .unwrap()
            .contains_key("extras"));

        // Non-finite numbers anywhere in typed extras are refused, not written as `null`.
        #[derive(Debug, Serialize)]
        struct Nested {
            values: Vec<f64>,
            pair: (f32, Option<f64>),
            inner: Option<Box<Nested>>,
        }
        let nested = |bad: f64, depth: u8| {
            let mut value = Nested {
                values: vec![1., 2.],
                pair: (1., Some(2.)),
                inner: None,
            };
            match depth {
                0 => value.values[1] = bad,
                1 => value.pair.0 = bad as f32,
                2 => value.pair.1 = Some(bad),
                _ => {
                    value.inner = Some(Box::new(Nested {
                        values: vec![bad],
                        pair: (0., None),
                        inner: None,
                    }))
                }
            }
            value
        };
        for bad in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
            for depth in 0..4 {
                let record = TileRecord::new(
                    NodePlacement::INHERIT,
                    TileBox::centred([1.; 3]).unwrap(),
                    GeometricError::new(1.).unwrap(),
                    Refine::Replace,
                    Vec::new(),
                    Some(nested(bad, depth)),
                );
                assert!(is_data(record).contains("not a finite number"), "{depth}");
            }
        }
        let mut good = TileRecord::new(
            NodePlacement::INHERIT,
            TileBox::centred([1.; 3]).unwrap(),
            GeometricError::new(1.).unwrap(),
            Refine::Replace,
            Vec::new(),
            Some(nested(3., 3)),
        )
        .unwrap();
        assert!(is_data(good.set_extras(Some(nested(f64::NAN, 0)))).contains("finite"));
        // A refused update keeps the previous extras.
        assert_eq!(good.extras().unwrap().inner.as_ref().unwrap().values, [3.]);
    }

    #[test]
    fn escaped_sibling_names_and_source_models() {
        // As binding::filename_uri writes a source filename: canonical escapes decode.
        for (uri, path) in [
            ("source.glb", "source.glb"),
            (".hidden.GLB", ".hidden.GLB"),
            ("my%20model%20%CE%B2.glb", "my model β.glb"),
            ("noext", "noext"),
            ("d/a%25b.gltf", "d/a%b.gltf"),
        ] {
            let name = MemberName::escaped(uri).unwrap();
            assert_eq!((name.as_str(), name.path()), (uri, path));
            assert_eq!(name.is_member(), MemberName::new(uri).is_ok(), "{uri}");
        }
        for (bad, why) in [
            ("", "segment"),
            ("a//b", "segment"),
            ("%2E%2E", "unreserved"),
            ("..", "segment"),
            ("a%2Fb", "separator"),
            ("a%5Cb", "separator"),
            ("a%0Ab", "separator or control"),
            ("a%C2%85b", "control"),
            ("a%2", "two upper-case"),
            ("a%2f", "two upper-case"),
            ("a%41", "unreserved"),
            ("a b", "must be escaped"),
            ("a%FF", "UTF-8"),
            ("a%3F.glb", "portable"),
            ("trailing.", "portable"),
            ("space%20", "portable"),
            ("con%3A.glb", "portable"),
        ] {
            let message = is_data(MemberName::escaped(bad));
            assert!(message.contains(why), "{bad:?}: {message}");
        }
        assert_eq!(ContentKind::of(b"glTF\x02\0\0\0"), ContentKind::Glb);
        assert_eq!(ContentKind::of(b"{\"asset\":{}}"), ContentKind::Gltf);
        // A source model keeps its admitted name whatever its suffix; it is never vector.
        let model = ContentRef::source_model(
            MemberName::escaped("my%20model").unwrap(),
            [3; 32],
            2,
            ContentKind::Gltf,
        )
        .unwrap();
        assert_eq!(model.to_value().to_string(), r#"{"uri":"my%20model"}"#);
        assert_eq!(
            (model.kind(), model.vector(), model.sources()),
            (ContentKind::Gltf, false, SourceIdentity::Aggregate)
        );
        let short = |kind, length| {
            ContentRef::source_model(MemberName::new("m.x").unwrap(), [0; 32], length, kind)
        };
        assert!(is_data(short(ContentKind::Gltf, 1)).contains("a JSON object"));
        assert!(is_data(short(ContentKind::Glb, 11)).contains("a GLB header"));
        // The typed glTF kind still requires its suffix through ContentRef::new.
        let gltf = |uri: &str| {
            ContentRef::new(
                MemberName::new(uri).unwrap(),
                [0; 32],
                64,
                ContentKind::Gltf,
                false,
                SourceIdentity::Aggregate,
            )
        };
        assert!(gltf("m/a.gltf").is_ok());
        assert!(is_data(gltf("m/a.glb")).contains(".gltf"));
        // A staged escaped name is read at its decoded path.
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("a b.glb"), b"glTF-bytes").unwrap();
        let staged = ContentRef::source_model(
            MemberName::escaped("a%20b.glb").unwrap(),
            [0; 32],
            12,
            ContentKind::Glb,
        )
        .unwrap();
        assert_eq!(staged.read_staged(dir.path()).unwrap(), b"glTF-bytes");
    }

    #[test]
    fn placement_matrices_are_the_written_transforms() {
        assert_eq!(NodePlacement::INHERIT.matrix(), None);
        let translation = NodePlacement::translation([1., -2., 3.5]).unwrap();
        let rigid = NodePlacement::rigid(AXES, [4_500_000., 600_000., 4_400_000.]).unwrap();
        for placement in [translation, rigid] {
            let written = placement.to_value().unwrap();
            let matrix = placement.matrix().unwrap();
            for (lane, value) in matrix.iter().enumerate() {
                assert_eq!(written[lane].as_f64().unwrap().to_bits(), value.to_bits());
            }
        }
    }

    #[test]
    fn explicit_child_slots_order_and_validate() {
        let leaf = || TileTree::new(record([1.; 3], 0., Vec::new()), Vec::new());
        let tree = TileTree::with_slots(
            record([2.; 3], 1., Vec::new()),
            vec![(6, leaf()), (1, leaf())],
        )
        .unwrap();
        let order: Vec<_> = tree
            .preorder()
            .into_iter()
            .map(|(_, p, s)| (p, s))
            .collect();
        assert_eq!(order, [(None, 0), (Some(0), 6), (Some(0), 1)]);
        assert_eq!((tree.slot(0), tree.slot(1)), (6, 1));
        let message = |result: Result<TileTree<Extras>, Error>| is_data(result);
        assert!(message(TileTree::with_slots(
            record([2.; 3], 1., Vec::new()),
            vec![(3, leaf()), (3, leaf())]
        ))
        .contains("duplicate child slot 3"));
        assert!(message(TileTree::with_slots(
            record([2.; 3], 1., Vec::new()),
            vec![(MAX_CHILD_SLOT + 1, leaf())]
        ))
        .contains("outside every subdivision scheme"));
    }
}
