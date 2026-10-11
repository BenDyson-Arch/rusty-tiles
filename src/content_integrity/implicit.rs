//! I1: crate-private implicit effective-volume read owner (contract v3.1).
//!
//! [`admit_root`] admits one implicit root (§3 templates, profile and header
//! classes) and [`Walk::next`] visits its available tiles in depth-first
//! pre-order (§1–§5, §7). This module owns format meaning only. Callers own
//! archive paths, limit selection, report policy and causal I/O: every member
//! read goes through the caller's resolver. No floating-point tolerance is
//! applied here, and derived cells stay symbolic ([`Cell`]).
//!
//! Charging protocol (§2a, §7). [`ImplicitLimits`] carries the caller's
//! *remaining* allowances. Before each [`Walk::next`] the caller sets them from
//! its counters and afterwards writes them back. The owner reduces them by
//! exactly what it charges or reserves:
//! - `visits_remaining` by each subtree's tile popcount `P_t` (step 9);
//! - `refs_remaining` by each subtree's content entries `E` (step 10), and by
//!   one per resolver read (mirroring the caller's `reference()` charge);
//! - `scan_remaining` by each subtree's `Scan` (step 2), and by the length of
//!   every resolver read (mirroring the caller's `read_member` charge).
// Dead until the validator integration (author B) consumes it in the combined PR.
#![cfg_attr(not(test), allow(dead_code))]

use super::{FormatError, JsonLimits};
use serde_json::{Map, Value};
use std::collections::HashSet;

mod availability;
mod metadata;
mod subtree;
mod walk;

pub(crate) use metadata::ImplicitSchema;
pub(crate) use walk::Walk;

/// The resolver error type of calls that never resolve anything.
pub(crate) type Never = std::convert::Infallible;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Scheme {
    Quadtree,
    Octree,
}

impl Scheme {
    fn branches(self) -> u64 {
        match self {
            Self::Quadtree => 4,
            Self::Octree => 8,
        }
    }
}

/// Remaining validation-wide allowances, synchronized by the caller around
/// every [`Walk::next`] call (see the module documentation).
#[derive(Debug)]
pub(crate) struct ImplicitLimits {
    /// `hierarchy_visits − task_visits`.
    pub(crate) visits_remaining: u64,
    /// `references − reference_visits`.
    pub(crate) refs_remaining: u64,
    /// `total_bytes_read − bytes_read`.
    pub(crate) scan_remaining: u64,
    /// The live-path cap (`document_decoded_bytes`), not a remaining value.
    pub(crate) live_subtree_bytes: u64,
    pub(crate) json: JsonLimits,
    /// Subtree members resolved so far in this validation (OD-7). Every entry
    /// is an existing archive member, so this is bounded by the archive index.
    subtrees: HashSet<String>,
}

impl ImplicitLimits {
    /// One value per validation: it also carries the OD-7 repeated-subtree set.
    pub(crate) fn new(live_subtree_bytes: u64, json: JsonLimits) -> Self {
        Self {
            visits_remaining: 0,
            refs_remaining: 0,
            scan_remaining: 0,
            live_subtree_bytes,
            json,
            subtrees: HashSet::new(),
        }
    }
}

/// How the resolver interprets the name it is given.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Base<'a> {
    /// The name is already the confined member name: a subtree member, equal
    /// to `uri(<tileset JSON holding R>, substituted template)` (§3).
    Resolved,
    /// The name is a raw subtree buffer URI, to be resolved through `uri()`
    /// relative to the given subtree member (§1 step 4).
    Relative(&'a str),
}

/// The caller's resolver: `(name, base, maximum bytes)` to the member bytes.
/// It charges one reference and the bytes read; the owner mirrors both.
pub(crate) type Resolver<'a, E> = dyn FnMut(&str, Base<'_>, u64) -> Result<Vec<u8>, E> + 'a;

#[derive(Debug)]
pub(crate) enum ImplicitError<E> {
    Format(FormatError),
    Resolver(E),
    InvalidState(&'static str),
}

impl<E> From<FormatError> for ImplicitError<E> {
    fn from(error: FormatError) -> Self {
        Self::Format(error)
    }
}

impl ImplicitError<Never> {
    pub(crate) fn widen<E>(self) -> ImplicitError<E> {
        match self {
            Self::Format(error) => ImplicitError::Format(error),
            Self::InvalidState(message) => ImplicitError::InvalidState(message),
            Self::Resolver(never) => match never {},
        }
    }
}

/// A tile's effective volume (§4).
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum EffectiveVolume {
    /// A TILE_BOUNDING_BOX row, widened exactly.
    Row([f64; 12]),
    /// Level 0 without a row: exactly the root's `boundingVolume.box`.
    DerivedRoot,
    /// Level ≥ 1 without a row: the exact symbolic subdivision cell.
    Derived(Cell),
}

/// An exact rational cell of the root box (§4). Centre lane `j` is
/// `c_j + Σ_k a_kj·((2·i_k + 1)/2^level − 1)`; half-axis `k` is `a_k/2^level`.
/// QUADTREE keeps axis 2. Nothing rounded is produced as truth.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Cell {
    pub(crate) root: [f64; 12],
    pub(crate) level: u32,
    pub(crate) coords: [u32; 3],
    pub(crate) scheme: Scheme,
}

impl Cell {
    /// The OD-3 *labelled rounded* evaluation. It exists only for the legacy
    /// `hierarchyBounds` tolerance check, and must never enter C2 facts.
    pub(crate) fn rounded_for_hierarchy_bounds(&self) -> [f64; 12] {
        let scale = power_of_two(self.level);
        let axes = match self.scheme {
            Scheme::Quadtree => 2,
            Scheme::Octree => 3,
        };
        let mut out = self.root;
        for k in 0..axes {
            let factor = f64::from(2 * self.coords[k] + 1) * scale - 1.;
            for j in 0..3 {
                out[j] += self.root[3 + 3 * k + j] * factor;
                out[3 + 3 * k + j] = self.root[3 + 3 * k + j] * scale;
            }
        }
        out
    }
}

/// A tile's effective geometric error (§4).
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum EffectiveError {
    /// A TILE_GEOMETRIC_ERROR row, widened exactly.
    Row(f64),
    /// `e_R·2^−level`, exact (level 0 is `e_R` itself).
    Derived(f64),
}

/// One available tile, in pre-order. Its parent is the tile at `level − 1` on
/// the caller's level-indexed path stack (RC-21).
#[derive(Debug, PartialEq)]
pub(crate) struct VisitedTile {
    pub(crate) level: u32,
    /// Global `(x, y, z)`; `z` is 0 for QUADTREE.
    pub(crate) coords: [u32; 3],
    pub(crate) volume: EffectiveVolume,
    pub(crate) error: EffectiveError,
    /// Emitted contents in slot order: `(slot, resolved member name)`. The name
    /// already equals `uri(base, substituted template)` (§3).
    pub(crate) contents: Vec<(u16, String)>,
}

/// One admitted implicit root R (§3, root admission).
#[derive(Debug)]
pub(crate) struct ImplicitRoot<'s> {
    scheme: Scheme,
    subtree_levels: u32,
    available_levels: u32,
    root_box: [f64; 12],
    root_error: f64,
    contents: Vec<Template>,
    headers: Vec<Value>,
    subtrees: Template,
    schema: Option<&'s ImplicitSchema>,
}

impl<'s> ImplicitRoot<'s> {
    pub(crate) fn walk(&self) -> Walk<'_> {
        Walk::new(self)
    }
    /// The content header of `slot`: the template object without `uri`
    /// (Cesium `Implicit3DTileContent.js:453`), copied to each derived content.
    pub(crate) fn header(&self, slot: u16) -> &Value {
        &self.headers[usize::from(slot)]
    }
    pub(crate) fn slots(&self) -> u16 {
        // Bounded at admission.
        self.contents.len() as u16
    }
    pub(crate) fn scheme(&self) -> Scheme {
        self.scheme
    }
}

/// Whether a tile declares an implicit root in any form, including the
/// Unsupported `3DTILES_implicit_tiling` (1.0) extension. Callers route every
/// such tile through [`admit_root`].
pub(crate) fn declares_implicit(tile: &Value) -> bool {
    tile.get("implicitTiling").is_some()
        || tile
            .get("extensions")
            .and_then(|e| e.get("3DTILES_implicit_tiling"))
            .is_some()
}

fn invalid(message: impl std::fmt::Display) -> FormatError {
    FormatError::InvalidInput(format!("implicit: {message}"))
}
fn unsupported(reason: impl std::fmt::Display) -> FormatError {
    FormatError::Unsupported(format!("implicit: unsupported {reason}"))
}
fn limit(message: impl std::fmt::Display) -> FormatError {
    FormatError::ResourceLimit(format!("implicit: {message}"))
}
fn refused() -> FormatError {
    limit("storage allocation refused")
}
fn try_push<T>(values: &mut Vec<T>, value: T) -> Result<(), FormatError> {
    values.try_reserve(1).map_err(|_| refused())?;
    values.push(value);
    Ok(())
}
/// Exactly `2^−level` for `level ≤ 1022`.
fn power_of_two(level: u32) -> f64 {
    f64::from_bits(u64::from(1023 - level) << 52)
}
/// A non-negative JSON integer that fits u64 (§1); anything else is InvalidInput.
fn uint(value: Option<&Value>, label: &str) -> Result<u64, FormatError> {
    value
        .and_then(Value::as_u64)
        .ok_or_else(|| invalid(format!("{label} must be a non-negative integer")))
}
fn finite(value: &Value, label: &str) -> Result<f64, FormatError> {
    value
        .as_f64()
        .filter(|v| v.is_finite())
        .ok_or_else(|| invalid(format!("{label} must be a finite number")))
}

/// Root admission (§3, precedence "Root admission"). `base` is the member name
/// of the tileset JSON holding R; `schema` is that document's metadata schema.
pub(crate) fn admit_root<'s>(
    tile: &Value,
    base: &str,
    schema: Option<&'s ImplicitSchema>,
) -> Result<ImplicitRoot<'s>, ImplicitError<Never>> {
    Ok(root(tile, base, schema)?)
}

fn root<'s>(
    tile: &Value,
    base: &str,
    schema: Option<&'s ImplicitSchema>,
) -> Result<ImplicitRoot<'s>, FormatError> {
    let tile = tile
        .as_object()
        .ok_or_else(|| invalid("tile must be an object"))?;
    // implicitTiling fields, scheme, levels.
    if object_has(tile.get("extensions"), "3DTILES_implicit_tiling") {
        return Err(unsupported("3DTILES_implicit_tiling (1.0) extension"));
    }
    let tiling = tile
        .get("implicitTiling")
        .and_then(Value::as_object)
        .ok_or_else(|| invalid("implicitTiling must be an object"))?;
    let scheme = match tiling.get("subdivisionScheme").and_then(Value::as_str) {
        Some("QUADTREE") => Scheme::Quadtree,
        Some("OCTREE") => Scheme::Octree,
        Some(other) => return Err(unsupported(format!("subdivisionScheme {other:?}"))),
        None => return Err(invalid("subdivisionScheme must be a string")),
    };
    let levels = |key: &str| -> Result<u32, FormatError> {
        match uint(tiling.get(key), key)? {
            0 => Err(invalid(format!("{key} must be at least 1"))),
            n @ 1..=32 => Ok(n as u32),
            _ => Err(unsupported(format!("{key} above 32"))),
        }
    };
    let subtree_levels = levels("subtreeLevels")?;
    let available_levels = levels("availableLevels")?;
    let subtree_template = tiling
        .get("subtrees")
        .and_then(|s| s.get("uri"))
        .and_then(Value::as_str)
        .ok_or_else(|| invalid("missing subtree URI template"))?;
    // Volume kind and root shape.
    let volume = tile
        .get("boundingVolume")
        .and_then(Value::as_object)
        .ok_or_else(|| invalid("implicit root boundingVolume must be an object"))?;
    if object_has(volume.get("extensions"), "3DTILES_bounding_volume_S2") {
        return Err(unsupported(
            "3DTILES_bounding_volume_S2 implicit root volume",
        ));
    }
    if volume.contains_key("region") {
        return Err(unsupported("region implicit root volume"));
    }
    if volume.contains_key("sphere") {
        return Err(invalid("sphere implicit root volumes are disallowed"));
    }
    let root_box = match volume.get("box").and_then(Value::as_array) {
        Some(values) if values.len() == 12 => {
            let mut out = [0.; 12];
            for (lane, value) in out.iter_mut().zip(values) {
                *lane = finite(value, "implicit root box")?;
            }
            out
        }
        _ => return Err(invalid("implicit root box must have 12 numbers")),
    };
    if tile.contains_key("children") {
        return Err(invalid("implicit root cannot have children"));
    }
    if tile.contains_key("metadata") {
        return Err(invalid("implicit root cannot have metadata"));
    }
    let templates: Vec<&Map<String, Value>> = match (tile.get("content"), tile.get("contents")) {
        (Some(_), Some(_)) => return Err(invalid("tile has both content and contents")),
        (Some(content), None) => vec![content
            .as_object()
            .ok_or_else(|| invalid("content must be an object"))?],
        (None, Some(contents)) => contents
            .as_array()
            .ok_or_else(|| invalid("contents must be an array"))?
            .iter()
            .map(|c| {
                c.as_object()
                    .ok_or_else(|| invalid("content must be an object"))
            })
            .collect::<Result<_, _>>()?,
        (None, None) => Vec::new(),
    };
    if templates.len() > usize::from(u16::MAX) {
        return Err(limit("content templates exceed host slot range"));
    }
    if templates.iter().any(|c| c.contains_key("boundingVolume")) {
        return Err(invalid(
            "implicit content template cannot have a boundingVolume",
        ));
    }
    let root_error = tile
        .get("geometricError")
        .and_then(Value::as_f64)
        .filter(|e| e.is_finite() && *e >= 0.)
        .ok_or_else(|| invalid("implicit root geometricError must be finite and nonnegative"))?;
    // Templates: content slots in order, then the subtree template.
    let mut contents = Vec::new();
    contents
        .try_reserve_exact(templates.len())
        .map_err(|_| refused())?;
    for content in &templates {
        let raw = content
            .get("uri")
            .and_then(Value::as_str)
            .ok_or_else(|| invalid("content template uri must be a string"))?;
        contents.push(Template::normalize(raw, base, scheme, Some(base))?);
    }
    let subtrees = Template::normalize(subtree_template, base, scheme, None)?;
    // Template header `metadata` classes (DC-5).
    let mut headers = Vec::new();
    headers
        .try_reserve_exact(templates.len())
        .map_err(|_| refused())?;
    for content in &templates {
        if let Some(entity) = content.get("metadata") {
            let class = entity
                .get("class")
                .and_then(Value::as_str)
                .ok_or_else(|| invalid("content metadata class must be a string"))?;
            schema
                .ok_or_else(|| invalid(format!("unknown metadata class {class:?}")))?
                .admit(class, metadata::Use::Content)?;
        }
        let mut header = (*content).clone();
        header.remove("uri");
        headers.push(Value::Object(header));
    }
    Ok(ImplicitRoot {
        scheme,
        subtree_levels,
        available_levels,
        root_box,
        root_error,
        contents,
        headers,
        subtrees,
        schema,
    })
}

fn object_has(value: Option<&Value>, key: &str) -> bool {
    value.and_then(|v| v.get(key)).is_some()
}

// ---- Templates (§3): normalized once at root admission ----

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Var {
    Level,
    X,
    Y,
    Z,
}

impl Var {
    const ALL: [Var; 4] = [Var::Level, Var::X, Var::Y, Var::Z];
    fn token(self) -> &'static str {
        match self {
            Self::Level => "{level}",
            Self::X => "{x}",
            Self::Y => "{y}",
            Self::Z => "{z}",
        }
    }
    /// The recognized token at the start of `text` (which starts with `{`).
    fn parse(text: &str) -> Option<Var> {
        Self::ALL.into_iter().find(|v| text.starts_with(v.token()))
    }
}

#[derive(Debug, PartialEq)]
enum Piece {
    Text(String),
    Var(Var),
}

/// A normalized template: static text (already percent-decoded and resolved
/// against the base directory) interleaved with variables.
#[derive(Debug)]
pub(crate) struct Template {
    pieces: Vec<Piece>,
    /// Normalized UTF-8 length with variables counted as their token text.
    len: usize,
}

/// Normalized templates longer than this are Unsupported (DC-1).
const TEMPLATE_CAP: usize = 1024;

impl Template {
    /// §3 steps 1–6, first failure wins. `json` names the document for a
    /// content template's JSON verdict; it is `None` for the subtree template.
    fn normalize(
        raw: &str,
        base: &str,
        scheme: Scheme,
        json: Option<&str>,
    ) -> Result<Template, FormatError> {
        // 1. Raw syntax.
        if raw.contains('\\') {
            return Err(invalid(format!("template contains a backslash: {raw}")));
        }
        let mut present = [false; 4];
        let bytes = raw.as_bytes();
        let mut i = 0;
        while i < bytes.len() {
            match bytes[i] {
                b'{' => {
                    let var = Var::parse(&raw[i..])
                        .filter(|v| *v != Var::Z || scheme == Scheme::Octree)
                        .ok_or_else(|| {
                            invalid(format!("unsupported template expression in {raw}"))
                        })?;
                    present[var as usize] = true;
                    i += var.token().len();
                }
                b'}' => return Err(invalid(format!("unsupported template expression in {raw}"))),
                _ => i += 1,
            }
        }
        let required = match scheme {
            Scheme::Quadtree => 3,
            Scheme::Octree => 4,
        };
        if present[..required].contains(&false) {
            return Err(invalid(format!(
                "template lacks a required variable: {raw}"
            )));
        }
        if raw.contains([':', '?', '#']) || raw.starts_with('/') {
            return Err(unsupported(format!(
                "URI outside archive-local profile: {raw}"
            )));
        }
        // 2. Escape guard, in every segment.
        for segment in raw.split('/') {
            let s = segment.as_bytes();
            if (0..s.len()).any(|p| s[p] == b'%' && s[p + 1..].iter().take(2).any(|b| *b == b'{')) {
                return Err(unsupported("escape adjacent to template variable"));
            }
        }
        // 3. Exact normalization, as uri(base, substituted) would resolve it.
        let normalized = Normalized::resolve(raw, base)?;
        // 4. JSON detection on the last normalized segment (content only).
        if let Some(document) = json {
            if normalized.last_segment_is_json() {
                return Err(unsupported(format!(
                    "external tileset content template in {document}"
                )));
            }
        }
        // 5. Variable survival.
        if (0..required).any(|v| !normalized.vars.iter().any(|(_, w)| *w as usize == v)) {
            return Err(unsupported("template variables eliminated by dot-segments"));
        }
        // 6. Length cap.
        let len = normalized.len();
        if len > TEMPLATE_CAP {
            return Err(unsupported("normalized template exceeds 1 KiB"));
        }
        normalized.into_template(len)
    }

    /// The member name of a tile or subtree root: digits substituted into the
    /// normalized template (RC-24). Never re-resolves the raw template.
    fn substitute(&self, level: u32, coords: [u32; 3]) -> Result<String, FormatError> {
        let value = |var: Var| match var {
            Var::Level => level,
            Var::X => coords[0],
            Var::Y => coords[1],
            Var::Z => coords[2],
        };
        let digits = |n: u32| n.checked_ilog10().unwrap_or(0) as usize + 1;
        let size = self.pieces.iter().fold(0usize, |size, piece| {
            size + match piece {
                Piece::Text(text) => text.len(),
                Piece::Var(var) => digits(value(*var)),
            }
        });
        let mut out = String::new();
        out.try_reserve_exact(size).map_err(|_| refused())?;
        for piece in &self.pieces {
            match piece {
                Piece::Text(text) => out.push_str(text),
                Piece::Var(var) => {
                    use std::fmt::Write;
                    let _ = write!(out, "{}", value(*var));
                }
            }
        }
        Ok(out)
    }

    #[cfg(test)]
    fn text(&self) -> String {
        self.pieces
            .iter()
            .map(|p| match p {
                Piece::Text(t) => t.as_str(),
                Piece::Var(v) => v.token(),
            })
            .collect()
    }
}

/// Resolution state of §3 step 3: decoded static bytes, variable positions and
/// segment starts. Popping truncates all three, so storage is O(raw).
struct Normalized {
    text: Vec<u8>,
    vars: Vec<(usize, Var)>,
    /// `(text start, vars start)` per retained segment.
    segments: Vec<(usize, usize)>,
}

impl Normalized {
    fn resolve(raw: &str, base: &str) -> Result<Normalized, FormatError> {
        let mut state = Normalized {
            text: Vec::new(),
            vars: Vec::new(),
            segments: Vec::new(),
        };
        // Start from the directory of the tileset JSON holding R, exactly as
        // `uri()` does (its parts are pushed as-is).
        if let Some((dir, _)) = base.rsplit_once('/') {
            for part in dir.split('/') {
                try_push(&mut state.segments, (state.text.len(), state.vars.len()))?;
                state.text.try_reserve(part.len()).map_err(|_| refused())?;
                state.text.extend_from_slice(part.as_bytes());
            }
        }
        for segment in raw.split('/') {
            let start = (state.text.len(), state.vars.len());
            state
                .text
                .try_reserve(segment.len())
                .map_err(|_| refused())?;
            let s = segment.as_bytes();
            let mut run = state.text.len();
            let mut i = 0;
            while i < s.len() {
                match s[i] {
                    b'{' => {
                        // Every brace is a recognized variable after step 1.
                        let var = Var::parse(&segment[i..])
                            .ok_or_else(|| invalid("unsupported template expression"))?;
                        check_run(&state.text[run..])?;
                        try_push(&mut state.vars, (state.text.len(), var))?;
                        run = state.text.len();
                        i += var.token().len();
                    }
                    b'%' => {
                        let digit = |b: Option<&u8>| b.and_then(|b| (*b as char).to_digit(16));
                        let (Some(high), Some(low)) = (digit(s.get(i + 1)), digit(s.get(i + 2)))
                        else {
                            return Err(invalid("invalid URI percent escape"));
                        };
                        state.text.push((high * 16 + low) as u8);
                        i += 3;
                    }
                    byte => {
                        state.text.push(byte);
                        i += 1;
                    }
                }
            }
            check_run(&state.text[run..])?;
            if state.vars.len() == start.1 {
                // A variable segment is never empty, `.` or `..`.
                match &state.text[start.0..] {
                    b"" | b"." => {
                        state.text.truncate(start.0);
                        continue;
                    }
                    b".." => {
                        let (text, vars) = state
                            .segments
                            .pop()
                            .ok_or_else(|| invalid("URI escapes archive"))?;
                        state.text.truncate(text);
                        state.vars.truncate(vars);
                        continue;
                    }
                    _ => {}
                }
            }
            try_push(&mut state.segments, start)?;
        }
        Ok(state)
    }

    fn len(&self) -> usize {
        self.text.len()
            + self
                .vars
                .iter()
                .map(|(_, v)| v.token().len())
                .sum::<usize>()
            + self.segments.len().saturating_sub(1)
    }

    fn last_segment_is_json(&self) -> bool {
        let Some(&(start, vars)) = self.segments.last() else {
            return false;
        };
        // Digits can never complete ".json": only the static tail counts.
        let tail = self.vars[vars..].last().map_or(start, |(at, _)| *at);
        let tail = &self.text[tail..];
        tail.len() >= 5 && tail[tail.len() - 5..].eq_ignore_ascii_case(b".json")
    }

    fn into_template(self, len: usize) -> Result<Template, FormatError> {
        let mut pieces = Vec::new();
        let mut text = Vec::new();
        let flush = |text: &mut Vec<u8>, pieces: &mut Vec<Piece>| -> Result<(), FormatError> {
            if !text.is_empty() {
                let run = String::from_utf8(std::mem::take(text))
                    .map_err(|_| invalid("URI decoded path must be UTF-8"))?;
                try_push(pieces, Piece::Text(run))?;
            }
            Ok(())
        };
        for (n, &(start, vars)) in self.segments.iter().enumerate() {
            let (end, vars_end) = self
                .segments
                .get(n + 1)
                .copied()
                .unwrap_or((self.text.len(), self.vars.len()));
            if n > 0 {
                text.push(b'/');
            }
            let mut at = start;
            for &(position, var) in &self.vars[vars..vars_end] {
                text.extend_from_slice(&self.text[at..position]);
                flush(&mut text, &mut pieces)?;
                try_push(&mut pieces, Piece::Var(var))?;
                at = position;
            }
            text.extend_from_slice(&self.text[at..end]);
        }
        flush(&mut text, &mut pieces)?;
        Ok(Template { pieces, len })
    }
}

/// `uri()`'s per-segment decoded checks, applied to one static run. Digits are
/// ASCII, so the runs are valid exactly when every substituted segment is.
fn check_run(run: &[u8]) -> Result<(), FormatError> {
    let run = std::str::from_utf8(run).map_err(|_| invalid("URI decoded path must be UTF-8"))?;
    if run.contains(['/', '\\']) || run.chars().any(char::is_control) {
        return Err(invalid("URI decoded segment contains a separator/control"));
    }
    Ok(())
}
