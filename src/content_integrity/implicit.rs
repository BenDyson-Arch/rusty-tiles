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

#[cfg(test)]
mod test_support {
    //! Small in-memory fixtures shared by the owner's unit tests.
    use super::*;
    use serde_json::json;
    use std::collections::HashMap;

    pub(super) fn format_only(error: ImplicitError<Never>) -> FormatError {
        match error {
            ImplicitError::Format(error) => error,
            ImplicitError::InvalidState(message) => panic!("invalid state: {message}"),
            ImplicitError::Resolver(never) => match never {},
        }
    }
    fn check<T: std::fmt::Debug>(
        result: Result<T, FormatError>,
        kind: &str,
        needle: &str,
    ) -> String {
        let (actual, message) = match &result {
            Err(FormatError::InvalidInput(m)) => ("InvalidInput", m),
            Err(FormatError::Unsupported(m)) => ("Unsupported", m),
            Err(FormatError::ResourceLimit(m)) => ("ResourceLimit", m),
            Ok(_) => panic!("expected {kind} containing {needle:?}, got {result:?}"),
        };
        assert!(
            actual == kind && message.contains(needle),
            "expected {kind} containing {needle:?}, got {actual}: {message}"
        );
        message.clone()
    }
    pub(super) fn assert_invalid<T: std::fmt::Debug>(r: Result<T, FormatError>, needle: &str) {
        check(r, "InvalidInput", needle);
    }
    pub(super) fn assert_unsupported<T: std::fmt::Debug>(r: Result<T, FormatError>, needle: &str) {
        check(r, "Unsupported", needle);
    }
    pub(super) fn assert_limit<T: std::fmt::Debug>(r: Result<T, FormatError>, needle: &str) {
        check(r, "ResourceLimit", needle);
    }

    /// A verbatim copy of `validate.rs::uri()` at `f2ed077` (`:165-210`), used
    /// only as the exactness oracle of §3 normalization.
    pub(super) fn reference_uri(base: &str, value: &str) -> Result<String, String> {
        if value.is_empty() || value.contains('\\') {
            return Err(format!("invalid resource URI: {value}"));
        }
        if value.contains([':', '?', '#']) || value.starts_with('/') {
            return Err(format!("URI outside archive-local profile: {value}"));
        }
        let mut parts: Vec<String> = base.rsplit_once('/').map_or(vec![], |(dir, _)| {
            dir.split('/').map(str::to_owned).collect()
        });
        for raw in value.split('/') {
            let mut decoded = Vec::with_capacity(raw.len());
            let mut bytes = raw.bytes();
            while let Some(byte) = bytes.next() {
                decoded.push(if byte == b'%' {
                    let digit = |byte: u8| (byte as char).to_digit(16).map(|n| n as u8);
                    let high = bytes.next().and_then(digit);
                    let low = bytes.next().and_then(digit);
                    let (Some(high), Some(low)) = (high, low) else {
                        return Err("invalid URI percent escape".into());
                    };
                    high * 16 + low
                } else {
                    byte
                });
            }
            let part = String::from_utf8(decoded).map_err(|_| "URI decoded path must be UTF-8")?;
            if part.contains(['/', '\\']) || part.chars().any(char::is_control) {
                return Err("URI decoded segment contains a separator/control".into());
            }
            match part.as_str() {
                "" | "." => (),
                ".." => {
                    if parts.pop().is_none() {
                        return Err("URI escapes archive".into());
                    }
                }
                _ => parts.push(part),
            }
        }
        Ok(parts.join("/"))
    }

    /// LSB-first bits of length `count` with `set` bits on.
    pub(super) fn bits(count: usize, set: &[usize]) -> Vec<u8> {
        let mut bytes = vec![0u8; count.div_ceil(8)];
        for &i in set {
            assert!(i < count);
            bytes[i / 8] |= 1 << (i % 8);
        }
        bytes
    }

    /// An availability object in a fixture.
    #[derive(Clone, Debug)]
    pub(super) enum Av {
        Const(u64),
        Bits(Vec<u8>),
        /// A bitstream naming an existing view index (aliasing).
        View(usize),
    }

    /// Builds binary subtrees: one internal buffer with 8-aligned views.
    #[derive(Clone, Debug)]
    pub(super) struct Sub {
        pub(super) bin: Vec<u8>,
        pub(super) views: Vec<Value>,
        pub(super) json: Map<String, Value>,
    }

    impl Sub {
        pub(super) fn new() -> Self {
            Self {
                bin: Vec::new(),
                views: Vec::new(),
                json: Map::new(),
            }
        }
        pub(super) fn view(&mut self, data: &[u8]) -> usize {
            while !self.bin.len().is_multiple_of(8) {
                self.bin.push(0);
            }
            self.views.push(json!({
                "buffer": 0, "byteOffset": self.bin.len(), "byteLength": data.len()
            }));
            self.bin.extend_from_slice(data);
            self.views.len() - 1
        }
        fn availability(&mut self, av: &Av) -> Value {
            match av {
                Av::Const(c) => json!({"constant": c}),
                Av::Bits(data) => json!({"bitstream": self.view(data)}),
                Av::View(v) => json!({"bitstream": v}),
            }
        }
        pub(super) fn tile(mut self, av: Av) -> Self {
            let value = self.availability(&av);
            self.json.insert("tileAvailability".into(), value);
            self
        }
        pub(super) fn child(mut self, av: Av) -> Self {
            let value = self.availability(&av);
            self.json.insert("childSubtreeAvailability".into(), value);
            self
        }
        pub(super) fn content(mut self, layers: &[Av]) -> Self {
            let values = layers.iter().map(|a| self.availability(a)).collect();
            self.json
                .insert("contentAvailability".into(), Value::Array(values));
            self
        }
        pub(super) fn set(mut self, key: &str, value: Value) -> Self {
            self.json.insert(key.into(), value);
            self
        }
        pub(super) fn build(&self) -> Vec<u8> {
            let mut json = self.json.clone();
            if !self.views.is_empty() {
                json.entry("buffers")
                    .or_insert_with(|| json!([{"byteLength": self.bin.len()}]));
                json.entry("bufferViews")
                    .or_insert_with(|| Value::Array(self.views.clone()));
            }
            container(
                &serde_json::to_vec(&Value::Object(json)).unwrap(),
                &self.bin,
                b' ',
            )
        }
    }

    /// A binary subtree container: JSON padded with `pad`, BIN with zeros.
    pub(super) fn container(json: &[u8], bin: &[u8], pad: u8) -> Vec<u8> {
        let mut json = json.to_vec();
        while !json.len().is_multiple_of(8) {
            json.push(pad);
        }
        let mut bin = bin.to_vec();
        while !bin.len().is_multiple_of(8) {
            bin.push(0);
        }
        let mut out = b"subt".to_vec();
        out.extend(1u32.to_le_bytes());
        out.extend((json.len() as u64).to_le_bytes());
        out.extend((bin.len() as u64).to_le_bytes());
        out.extend(json);
        out.extend(bin);
        out
    }

    /// In-memory members plus a record of every resolver call.
    #[derive(Default)]
    pub(super) struct Archive {
        pub(super) members: HashMap<String, Vec<u8>>,
        /// `(resolved name, maximum)` per read.
        pub(super) reads: Vec<(String, u64)>,
    }

    impl Archive {
        pub(super) fn with(mut self, name: &str, bytes: Vec<u8>) -> Self {
            self.members.insert(name.into(), bytes);
            self
        }
        pub(super) fn read(
            &mut self,
            name: &str,
            base: Base<'_>,
            maximum: u64,
        ) -> Result<Vec<u8>, String> {
            let name = match base {
                Base::Resolved => name.to_owned(),
                Base::Relative(from) => reference_uri(from, name)?,
            };
            self.reads.push((name.clone(), maximum));
            let bytes = self
                .members
                .get(&name)
                .ok_or_else(|| format!("missing archive entry: {name}"))?;
            if bytes.len() as u64 > maximum {
                return Err(format!("member exceeds admitted byte limit: {name}"));
            }
            Ok(bytes.clone())
        }
    }

    pub(super) fn json_limits() -> JsonLimits {
        JsonLimits {
            bytes: 1 << 20,
            depth: 64,
            value_nodes: 1 << 16,
        }
    }

    pub(super) fn limits() -> ImplicitLimits {
        let mut limits = ImplicitLimits::new(1 << 26, json_limits());
        limits.visits_remaining = 65_536;
        limits.refs_remaining = 262_144;
        limits.scan_remaining = 1 << 31;
        limits
    }

    /// An implicit root tile JSON.
    pub(super) fn root_tile(scheme: &str, levels: u32, available: u32, contents: &[&str]) -> Value {
        let mut tile = json!({
            "boundingVolume": {"box": [0,0,0, 16,0,0, 0,16,0, 0,0,16]},
            "geometricError": 64,
            "implicitTiling": {
                "subdivisionScheme": scheme,
                "subtreeLevels": levels,
                "availableLevels": available,
                "subtrees": {"uri": if scheme == "OCTREE" {
                    "s/{level}/{x}/{y}/{z}.subtree"
                } else {
                    "s/{level}/{x}/{y}.subtree"
                }}
            }
        });
        match contents {
            [] => {}
            [one] => tile["content"] = json!({"uri": one}),
            many => {
                tile["contents"] = many.iter().map(|uri| json!({"uri": uri})).collect();
            }
        }
        tile
    }

    pub(super) fn walk_with<'s>(
        root: &ImplicitRoot<'s>,
        archive: &mut Archive,
        limits: &mut ImplicitLimits,
    ) -> Result<Vec<VisitedTile>, ImplicitError<String>> {
        let mut walk = root.walk();
        let mut tiles = Vec::new();
        while let Some(tile) = walk.next(limits, &mut |n, b, m| archive.read(n, b, m))? {
            tiles.push(tile);
        }
        walk.finish().map_err(ImplicitError::widen)?;
        Ok(tiles)
    }

    /// Admit with no schema and walk with generous limits.
    pub(super) fn walk(
        tile: &Value,
        archive: &mut Archive,
    ) -> Result<Vec<VisitedTile>, ImplicitError<String>> {
        let root = admit_root(tile, "tileset.json", None).map_err(ImplicitError::widen)?;
        walk_with(&root, archive, &mut limits())
    }

    /// Only the format failure of a walk (panics on resolver/state errors).
    pub(super) fn format<T>(result: Result<T, ImplicitError<String>>) -> Result<T, FormatError> {
        result.map_err(|error| match error {
            ImplicitError::Format(error) => error,
            other => panic!("expected a format failure, got {other:?}"),
        })
    }

    /// `(level, x, y, z)` per tile in walk order.
    pub(super) fn coords(tiles: &[VisitedTile]) -> Vec<(u32, u32, u32, u32)> {
        tiles
            .iter()
            .map(|t| (t.level, t.coords[0], t.coords[1], t.coords[2]))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::test_support::*;
    use super::*;
    use serde_json::json;

    fn normalize(raw: &str, scheme: Scheme) -> Result<Template, FormatError> {
        Template::normalize(
            raw,
            "tiles/tileset.json",
            scheme,
            Some("tiles/tileset.json"),
        )
    }
    fn octree(raw: &str) -> Result<Template, FormatError> {
        normalize(raw, Scheme::Octree)
    }
    fn quadtree(raw: &str) -> Result<Template, FormatError> {
        normalize(raw, Scheme::Quadtree)
    }

    #[test]
    fn raw_syntax_rules_and_controls() {
        assert!(quadtree("c/{level}/{x}/{y}.glb").is_ok());
        assert!(octree("c/{level}/{x}/{y}/{z}.glb").is_ok());
        // C-2: missing {y}; octree missing {z}.
        assert_invalid(quadtree("c/{level}/{x}.glb"), "required variable");
        assert_invalid(octree("c/{level}/{x}/{y}.glb"), "required variable");
        // C-3: {z} in a QUADTREE template.
        assert_invalid(quadtree("c/{level}/{x}/{y}/{z}.glb"), "template expression");
        for raw in [
            "c/{level}/{x}/{y}/{w}.glb",
            "c/{level}/{x}/{y}}.glb",
            "c/{level}/{x}/{y.glb",
            "c/{{level}/{x}/{y}.glb",
        ] {
            assert_invalid(quadtree(raw), "template expression");
        }
        assert_invalid(quadtree("c\\{level}/{x}/{y}.glb"), "backslash");
        for raw in [
            "c:/{level}/{x}/{y}.glb",
            "c/{level}/{x}/{y}.glb?v=1",
            "c/{level}/{x}/{y}.glb#a",
            "/c/{level}/{x}/{y}.glb",
        ] {
            assert_unsupported(quadtree(raw), "archive-local");
        }
    }

    #[test]
    fn escape_guard_applies_in_every_segment() {
        // C-7: `.j%7{x}on` would let x = 3 complete `%73`.
        assert_unsupported(
            quadtree("c/{level}-{y}.j%7{x}on"),
            "escape adjacent to template variable",
        );
        // C-7b: a non-last segment.
        assert_unsupported(
            octree("c%2{x}/{level}-{y}-{z}.glb"),
            "escape adjacent to template variable",
        );
        assert_unsupported(
            quadtree("c/%{level}-{x}-{y}.glb"),
            "escape adjacent to template variable",
        );
        // Three characters before a variable is a complete static escape.
        let t = quadtree("c/%41{level}-{x}-{y}.glb").unwrap();
        assert_eq!(t.substitute(1, [1, 0, 0]).unwrap(), "tiles/c/A1-1-0.glb");
        // Across a separator the guard does not apply; the escape is malformed.
        assert_invalid(quadtree("c%/{level}/{x}/{y}.glb"), "percent escape");
    }

    #[test]
    fn normalization_is_exact_against_uri() {
        // C-7d and pops into the base directory.
        let cases = [
            (
                "./c//{level}/./{x}-{y}-{z}.glb",
                "c/{level}/{x}-{y}-{z}.glb",
            ),
            (
                "../up/{level}-{x}-{y}-{z}.glb",
                "../up/{level}-{x}-{y}-{z}.glb",
            ),
            (
                "a/b/../%63/{level}/{x}{y}{z}.b%2Ein",
                "a/c/{level}/{x}{y}{z}.b.in",
            ),
            (
                "%2E/c/%2e%2e/d/{level}/{x}/{y}/{z}",
                "d/{level}/{x}/{y}/{z}",
            ),
            (
                "c/{level}%20{x}/{y}/{z}/é.glb",
                "c/{level} {x}/{y}/{z}/é.glb",
            ),
        ];
        for (raw, plain) in cases {
            let t = octree(raw).unwrap();
            let p = octree(plain).unwrap();
            for (level, coords) in [(0, [0, 0, 0]), (3, [5, 1, 7]), (31, [u32::MAX >> 1, 0, 9])] {
                let name = t.substitute(level, coords).unwrap();
                assert_eq!(name, p.substitute(level, coords).unwrap(), "{raw}");
                let substituted = raw
                    .replace("{level}", &level.to_string())
                    .replace("{x}", &coords[0].to_string())
                    .replace("{y}", &coords[1].to_string())
                    .replace("{z}", &coords[2].to_string());
                assert_eq!(
                    name,
                    reference_uri("tiles/tileset.json", &substituted).unwrap()
                );
            }
        }
        assert_eq!(
            octree("./c//{level}/./{x}-{y}-{z}.glb").unwrap().text(),
            "tiles/c/{level}/{x}-{y}-{z}.glb"
        );
        // Popping past the archive root, malformed escapes, decoded separators
        // and controls are InvalidInput, as in uri().
        assert_invalid(quadtree("../../{level}/{x}/{y}"), "escapes archive");
        assert_invalid(quadtree("c%2/{level}/{x}/{y}"), "percent escape");
        assert_invalid(quadtree("c%zz/{level}/{x}/{y}"), "percent escape");
        assert_invalid(quadtree("c%2Fd/{level}/{x}/{y}"), "separator/control");
        assert_invalid(quadtree("c%5Cd/{level}/{x}/{y}"), "separator/control");
        assert_invalid(quadtree("c%0A/{level}/{x}/{y}"), "separator/control");
        assert_invalid(quadtree("c%C3/{level}/{x}/{y}"), "UTF-8");
        assert_invalid(quadtree("c%C3{level}%A9/{x}/{y}"), "UTF-8");
    }

    #[test]
    fn json_detection_on_the_normalized_last_segment() {
        // C-7 and the DC-2 bypass forms.
        for raw in [
            "c/{level}-{x}-{y}-{z}.json",
            "c/{level}-{x}-{y}-{z}.JSON",
            "c/{level}-{x}-{y}-{z}.js%6Fn",
            "c/{level}-{x}-{y}-{z}.json/.",
            "c/{level}-{x}-{y}-{z}.json/",
            "c/x.json/{level}-{x}-{y}-{z}/..",
            "c/{level}-{x}-{y}-{z}.JSON//./",
            "c/{level}/{x}/{y}/{z}/%2Ejson",
        ] {
            assert_unsupported(
                octree(raw),
                "external tileset content template in tiles/tileset.json",
            );
        }
        // `.json?v=1` is refused even earlier, still Unsupported.
        assert_unsupported(octree("c/{level}-{x}-{y}-{z}.json?v=1"), "archive-local");
        // Digits never complete the suffix; non-last segments do not count.
        for raw in [
            "c/{level}-{x}-{y}-{z}.json{x}",
            "c/{level}.json/{x}-{y}-{z}.glb",
            "c/{level}-{x}-{y}-{z}.jso",
            "c/{level}-{x}-{y}-{z}json",
        ] {
            assert!(octree(raw).is_ok(), "{raw}");
        }
        // Subtree templates are not content templates.
        let subtree = Template::normalize(
            "s/{level}/{x}/{y}.json",
            "tileset.json",
            Scheme::Quadtree,
            None,
        );
        assert!(subtree.is_ok());
    }

    #[test]
    fn variable_survival_and_length_cap() {
        // C-7c and the R8 construction at small scale.
        assert_unsupported(
            octree("c/{level}/../{x}/../{y}/../{z}/../a.glb"),
            "eliminated by dot-segments",
        );
        let padded = format!(
            "{}c/{{level}}/../{{x}}/../{{y}}/../{{z}}/../a.glb",
            "./".repeat(4000)
        );
        assert_unsupported(octree(&padded), "eliminated by dot-segments");
        // R8a: surviving variables normalize to a short template.
        let padded = format!("{}c/{{level}}-{{x}}-{{y}}-{{z}}.glb", "./".repeat(4000));
        let t = octree(&padded).unwrap();
        assert_eq!(t.text(), "tiles/c/{level}-{x}-{y}-{z}.glb");
        assert_eq!(t.len, 31);
        // R8b: 1,024 normalized bytes (base directory and tokens included).
        let fill = |n: usize| format!("{}/{{level}}/{{x}}/{{y}}.glb", "a".repeat(n));
        let fixed = "tiles/".len() + "/{level}/{x}/{y}.glb".len();
        assert_eq!(quadtree(&fill(1024 - fixed)).unwrap().len, 1024);
        assert_unsupported(quadtree(&fill(1025 - fixed)), "exceeds 1 KiB");
        // Padding that normalizes away does not count.
        let padded = format!("{}{}", "x/../".repeat(1000), fill(1024 - fixed));
        assert_eq!(quadtree(&padded).unwrap().len, 1024);
    }

    #[test]
    fn substitution_uses_decimal_digits() {
        let t = octree("c/{level}/{x}_{y}_{z}_{x}.glb").unwrap();
        assert_eq!(
            t.substitute(31, [2147483647, 10, 0]).unwrap(),
            "tiles/c/31/2147483647_10_0_2147483647.glb"
        );
    }

    fn tile(content: Value) -> Value {
        let mut tile = json!({
            "boundingVolume": {"box": [0,0,0, 8,0,0, 0,8,0, 0,0,8]},
            "geometricError": 16,
            "refine": "ADD",
            "implicitTiling": {
                "subdivisionScheme": "QUADTREE",
                "subtreeLevels": 2,
                "availableLevels": 4,
                "subtrees": {"uri": "s/{level}/{x}/{y}.subtree"}
            }
        });
        if !content.is_null() {
            tile["content"] = content;
        }
        tile
    }
    fn admit(tile: &Value) -> Result<ImplicitRoot<'static>, FormatError> {
        admit_root(tile, "tileset.json", None).map_err(format_only)
    }

    #[test]
    fn root_admission_profile_and_shape() {
        let base = tile(json!({"uri": "c/{level}/{x}/{y}.glb"}));
        let root = admit(&base).unwrap();
        assert_eq!(root.slots(), 1);
        assert_eq!(root.scheme(), Scheme::Quadtree);
        let set = |path: &[&str], value: Value| {
            let mut t = base.clone();
            let mut at = &mut t;
            for key in &path[..path.len() - 1] {
                at = &mut at[*key];
            }
            at[path[path.len() - 1]] = value;
            t
        };
        assert_unsupported(
            admit(&set(
                &["extensions"],
                json!({"3DTILES_implicit_tiling": {}}),
            )),
            "3DTILES_implicit_tiling",
        );
        // C-9.
        assert_unsupported(
            admit(&set(
                &["implicitTiling", "subdivisionScheme"],
                json!("HEXTREE"),
            )),
            "subdivisionScheme",
        );
        assert_invalid(
            admit(&set(&["implicitTiling", "subdivisionScheme"], json!(4))),
            "subdivisionScheme",
        );
        // A-9 and the level profile.
        assert!(admit(&set(&["implicitTiling", "availableLevels"], json!(32))).is_ok());
        assert_unsupported(
            admit(&set(&["implicitTiling", "availableLevels"], json!(33))),
            "availableLevels above 32",
        );
        assert!(admit(&set(&["implicitTiling", "subtreeLevels"], json!(32))).is_ok());
        assert_unsupported(
            admit(&set(&["implicitTiling", "subtreeLevels"], json!(33))),
            "subtreeLevels above 32",
        );
        assert_invalid(
            admit(&set(&["implicitTiling", "subtreeLevels"], json!(0))),
            "at least 1",
        );
        assert_invalid(
            admit(&set(&["implicitTiling", "availableLevels"], json!(-1))),
            "non-negative integer",
        );
        assert_invalid(
            admit(&set(&["implicitTiling", "subtrees"], json!({}))),
            "subtree URI template",
        );
        assert_unsupported(
            admit(&set(
                &["boundingVolume"],
                json!({"region": [0, 0, 1, 1, 0, 1]}),
            )),
            "region",
        );
        assert_invalid(
            admit(&set(&["boundingVolume"], json!({"sphere": [0, 0, 0, 1]}))),
            "sphere",
        );
        assert_unsupported(
            admit(&set(
                &["boundingVolume", "extensions"],
                json!({"3DTILES_bounding_volume_S2": {}}),
            )),
            "S2",
        );
        assert_invalid(
            admit(&set(&["boundingVolume", "box"], json!([0, 1]))),
            "12 numbers",
        );
        assert_invalid(admit(&set(&["children"], json!([]))), "children");
        assert_invalid(
            admit(&set(&["metadata"], json!({"class": "c"}))),
            "metadata",
        );
        assert_invalid(
            admit(&set(
                &["content", "boundingVolume"],
                json!({"box": vec![0; 12]}),
            )),
            "boundingVolume",
        );
        assert_invalid(
            admit(&set(&["geometricError"], json!(-1))),
            "geometricError",
        );
        // Precedence: content slots in order, then the subtree template.
        let mut t = base.clone();
        t.as_object_mut().unwrap().remove("content");
        t["contents"] = json!([{"uri": "a/{level}/{x}/{y}.json"}, {"uri": "a/{level}/{x}.glb"}]);
        t["implicitTiling"]["subtrees"]["uri"] = json!("s/{level}");
        assert_unsupported(
            admit(&t),
            "external tileset content template in tileset.json",
        );
        t["contents"][0]["uri"] = json!("a/{level}/{x}/{y}.glb");
        assert_invalid(admit(&t), "required variable: a/{level}/{x}.glb");
        t["contents"][1]["uri"] = json!("b/{level}/{x}/{y}.glb");
        assert_invalid(admit(&t), "required variable: s/{level}");
        // Every JSON template is Unsupported at admission, with no subtree read.
        let json_root = tile(json!({"uri": "c/{level}/{x}/{y}.json"}));
        assert_unsupported(admit(&json_root), "external tileset content template");
        // Headers are copied without `uri`.
        let header = tile(json!({"uri": "c/{level}/{x}/{y}.glb", "group": 0}));
        assert_eq!(admit(&header).unwrap().header(0), &json!({"group": 0}));
        assert!(declares_implicit(&base));
        assert!(declares_implicit(
            &json!({"extensions": {"3DTILES_implicit_tiling": {}}})
        ));
        assert!(!declares_implicit(&json!({"content": {}})));
    }

    #[test]
    fn header_metadata_classes_are_classified_at_admission() {
        // C-12b (DC-5): CONTENT_URI and CONTENT_GROUP_ID in a header class.
        let schema = |semantic: &str| {
            ImplicitSchema::new(json!({"classes": {"h": {"properties": {
                "p": {"type": "STRING", "semantic": semantic}
            }}}}))
        };
        let root = tile(json!({"uri": "c/{level}/{x}/{y}.glb", "metadata": {"class": "h"}}));
        for semantic in [
            "CONTENT_URI",
            "CONTENT_GROUP_ID",
            "TILE_BOUNDING_BOX",
            "TILE_REFINE",
        ] {
            let schema = schema(semantic);
            assert_unsupported(
                admit_root(&root, "tileset.json", Some(&schema)).map_err(format_only),
                semantic,
            );
        }
        let schema = schema("NAME");
        assert!(admit_root(&root, "tileset.json", Some(&schema)).is_ok());
        assert_invalid(admit(&root), "unknown metadata class");
        let other = tile(json!({"uri": "c/{level}/{x}/{y}.glb", "metadata": {"class": "x"}}));
        assert_invalid(
            admit_root(&other, "tileset.json", Some(&schema)).map_err(format_only),
            "unknown metadata class",
        );
    }

    #[test]
    fn rounded_evaluation_is_labelled_and_matches_the_cell() {
        let cell = Cell {
            root: [10., 20., 30., 8., 0., 0., 0., 4., 0., 0., 0., 2.],
            level: 2,
            coords: [3, 0, 1],
            scheme: Scheme::Octree,
        };
        assert_eq!(
            cell.rounded_for_hierarchy_bounds(),
            [16., 17., 29.5, 2., 0., 0., 0., 1., 0., 0., 0., 0.5]
        );
        let quad = Cell {
            scheme: Scheme::Quadtree,
            ..cell
        };
        assert_eq!(
            quad.rounded_for_hierarchy_bounds(),
            [16., 17., 30., 2., 0., 0., 0., 1., 0., 0., 0., 2.]
        );
        assert_eq!(power_of_two(0), 1.);
        assert_eq!(power_of_two(31), 1. / 2147483648.);
    }
}
