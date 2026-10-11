//! §1 subtree container: header, JSON chunk, arrays, buffers and views, then
//! §2a availability and §5 tables. Only typed facts survive admission: the
//! parsed subtree JSON is dropped before the walk descends (RC-11).
use super::availability::{Availability, Shape};
use super::metadata::{self, TileColumns};
use super::{
    invalid, refused, try_push, uint, unsupported, Base, FormatError, ImplicitError,
    ImplicitLimits, ImplicitRoot, Resolver,
};
use crate::content_integrity::json;
use serde_json::{Map, Value};

#[derive(Clone, Copy, Debug)]
enum Backing {
    Member,
    External(usize),
}

#[derive(Clone, Copy, Debug)]
struct Span {
    backing: Backing,
    start: usize,
    length: usize,
}

/// Member bytes, external buffer bytes and the view table of one subtree.
#[derive(Debug)]
pub(super) struct Storage {
    member: Vec<u8>,
    external: Vec<Vec<u8>>,
    views: Vec<Span>,
}

impl Storage {
    pub(super) fn view(&self, view: usize) -> &[u8] {
        let span = self.views[view];
        let bytes = match span.backing {
            Backing::Member => &self.member,
            Backing::External(i) => &self.external[i],
        };
        &bytes[span.start..span.start + span.length]
    }

    /// A reference to an existing validated view.
    pub(super) fn view_index(&self, value: &Value, label: &str) -> Result<usize, FormatError> {
        let index = uint(Some(value), label)?;
        usize::try_from(index)
            .ok()
            .filter(|i| *i < self.views.len())
            .ok_or_else(|| invalid(format!("{label} names no bufferView")))
    }
}

/// One admitted subtree: typed facts only.
#[derive(Debug)]
pub(super) struct Subtree {
    pub(super) storage: Storage,
    pub(super) availability: Availability,
    pub(super) columns: TileColumns,
    /// Member plus external buffer bytes, released with the subtree.
    pub(super) live_bytes: u64,
}

/// One resolver read, charged to the live path and mirrored onto `limits`
/// exactly as the caller charges it (one reference, the bytes read).
pub(super) fn read_charged<E>(
    limits: &mut ImplicitLimits,
    live: &mut u64,
    name: &str,
    base: Base<'_>,
    read: &mut Resolver<'_, E>,
) -> Result<Vec<u8>, ImplicitError<E>> {
    let maximum =
        limits
            .live_subtree_bytes
            .checked_sub(*live)
            .ok_or(ImplicitError::InvalidState(
                "live subtree bytes exceed their cap",
            ))?;
    let bytes = read(name, base, maximum).map_err(ImplicitError::Resolver)?;
    let size = bytes.len() as u64;
    if size > maximum {
        return Err(ImplicitError::InvalidState(
            "resolver returned more than the admitted maximum",
        ));
    }
    limits.scan_remaining =
        limits
            .scan_remaining
            .checked_sub(size)
            .ok_or(ImplicitError::InvalidState(
                "resolver read exceeds the synchronized byte budget",
            ))?;
    limits.refs_remaining =
        limits
            .refs_remaining
            .checked_sub(1)
            .ok_or(ImplicitError::InvalidState(
                "resolver read exceeds the synchronized references",
            ))?;
    *live += size;
    Ok(bytes)
}

/// `extensions` anywhere in subtree JSON is Unsupported; application-specific
/// `extras` values are opaque and not searched.
fn has_extensions(value: &Value) -> Result<bool, FormatError> {
    let mut pending = vec![value];
    while let Some(value) = pending.pop() {
        match value {
            Value::Object(object) => {
                if object.contains_key("extensions") {
                    return Ok(true);
                }
                pending.try_reserve(object.len()).map_err(|_| refused())?;
                pending.extend(
                    object
                        .iter()
                        .filter(|(k, _)| *k != "extras")
                        .map(|(_, v)| v),
                );
            }
            Value::Array(values) => {
                pending.try_reserve(values.len()).map_err(|_| refused())?;
                pending.extend(values);
            }
            _ => {}
        }
    }
    Ok(false)
}

fn array<'a>(subtree: &'a Map<String, Value>, key: &str) -> Result<&'a [Value], FormatError> {
    match subtree.get(key) {
        None => Ok(&[]),
        Some(Value::Array(values)) if !values.is_empty() => Ok(values),
        Some(_) => Err(invalid(format!(
            "{key} must be an array with at least 1 item"
        ))),
    }
}

/// Resolve, read and admit the subtree rooted at global `(level, coords)`.
pub(super) fn admit<E>(
    root: &ImplicitRoot<'_>,
    level: u32,
    coords: [u32; 3],
    limits: &mut ImplicitLimits,
    live: &mut u64,
    read: &mut Resolver<'_, E>,
) -> Result<Subtree, ImplicitError<E>> {
    let name = root.subtrees.substitute(level, coords)?;
    // OD-7: a subtree member resolved twice in one validation.
    if limits.subtrees.contains(&name) {
        return Err(unsupported(format!("repeated subtree member {name}")).into());
    }
    let start = *live;
    let member = read_charged(limits, live, &name, Base::Resolved, read)?;
    limits.subtrees.try_reserve(1).map_err(|_| refused())?;
    limits.subtrees.insert(name.clone());
    // 1. Header.
    if !member.starts_with(b"subt") {
        return Err(unsupported(format!("JSON-format subtree {name}")).into());
    }
    if member.len() < 24 {
        return Err(invalid(format!("subtree header is truncated: {name}")).into());
    }
    let word = |at: usize| u64::from_le_bytes(member[at..at + 8].try_into().unwrap());
    if member[4..8] != 1u32.to_le_bytes() {
        return Err(invalid(format!("subtree version must be 1: {name}")).into());
    }
    let (json_length, bin_length) = (word(8), word(16));
    if json_length == 0
        || !json_length.is_multiple_of(8)
        || !bin_length.is_multiple_of(8)
        || 24u128 + u128::from(json_length) + u128::from(bin_length) != member.len() as u128
    {
        return Err(invalid(format!(
            "invalid subtree chunk lengths or alignment: {name}"
        ))
        .into());
    }
    let bin_start = 24 + json_length as usize;
    // 2. JSON chunk, admitted exactly as C1 admits it.
    let document = json::parse(&member[24..bin_start], limits.json)?;
    let subtree = document
        .as_object()
        .ok_or_else(|| invalid("subtree JSON must be an object"))?;
    if has_extensions(&document)? {
        return Err(unsupported(format!("extensions in subtree JSON {name}")).into());
    }
    // 3. Arrays: schema minItems.
    let buffers = array(subtree, "buffers")?;
    let views = array(subtree, "bufferViews")?;
    array(subtree, "propertyTables")?;
    array(subtree, "contentAvailability")?;
    let content_metadata = array(subtree, "contentMetadata")?;
    // 4. Buffers.
    let bin = &member[bin_start..];
    let internal = buffers
        .first()
        .and_then(Value::as_object)
        .is_some_and(|b| !b.contains_key("uri"));
    if !bin.is_empty() && !internal {
        return Err(invalid("non-empty BIN chunk without an internal buffer").into());
    }
    let mut spans: Vec<Span> = Vec::new();
    spans
        .try_reserve_exact(buffers.len())
        .map_err(|_| refused())?;
    let mut external = Vec::new();
    for (index, buffer) in buffers.iter().enumerate() {
        let buffer = buffer
            .as_object()
            .ok_or_else(|| invalid("subtree buffer must be an object"))?;
        let length = uint(buffer.get("byteLength"), "buffer byteLength")?;
        if length == 0 {
            return Err(invalid("buffer byteLength must be at least 1").into());
        }
        let span = match buffer.get("uri") {
            None => {
                if index != 0 {
                    return Err(invalid("only buffer 0 may be the internal buffer").into());
                }
                if bin.is_empty() {
                    return Err(invalid("internal buffer requires a BIN chunk").into());
                }
                let length = usize::try_from(length)
                    .ok()
                    .filter(|n| *n <= bin.len() && bin.len() - *n < 8)
                    .ok_or_else(|| invalid("internal buffer byteLength differs from BIN"))?;
                if bin[length..].iter().any(|b| *b != 0) {
                    return Err(invalid("nonzero BIN chunk padding").into());
                }
                Span {
                    backing: Backing::Member,
                    start: bin_start,
                    length,
                }
            }
            Some(uri) => {
                let uri = uri
                    .as_str()
                    .ok_or_else(|| invalid("buffer uri must be a string"))?;
                if uri
                    .get(..5)
                    .is_some_and(|scheme| scheme.eq_ignore_ascii_case("data:"))
                {
                    return Err(invalid("subtree buffers cannot be data URIs").into());
                }
                let data = read_charged(limits, live, uri, Base::Relative(&name), read)?;
                let length = usize::try_from(length)
                    .ok()
                    .filter(|n| *n <= data.len())
                    .ok_or_else(|| invalid("external buffer is shorter than its byteLength"))?;
                try_push(&mut external, data)?;
                Span {
                    backing: Backing::External(external.len() - 1),
                    start: 0,
                    length,
                }
            }
        };
        spans.push(span);
    }
    // 5. Buffer views, all validated.
    let mut table = Vec::new();
    table
        .try_reserve_exact(views.len())
        .map_err(|_| refused())?;
    for view in views {
        let view = view
            .as_object()
            .ok_or_else(|| invalid("bufferView must be an object"))?;
        let buffer = uint(view.get("buffer"), "bufferView buffer")?;
        let buffer = usize::try_from(buffer)
            .ok()
            .and_then(|b| spans.get(b))
            .ok_or_else(|| invalid("bufferView names no buffer"))?;
        let offset = uint(view.get("byteOffset"), "bufferView byteOffset")?;
        if !offset.is_multiple_of(8) {
            return Err(invalid("bufferView byteOffset must be a multiple of 8").into());
        }
        let length = uint(view.get("byteLength"), "bufferView byteLength")?;
        if length == 0 {
            return Err(invalid("bufferView byteLength must be at least 1").into());
        }
        if offset
            .checked_add(length)
            .is_none_or(|end| end > buffer.length as u64)
        {
            return Err(invalid("bufferView exceeds its buffer").into());
        }
        table.push(Span {
            backing: buffer.backing,
            start: buffer.start + offset as usize,
            length: length as usize,
        });
    }
    let storage = Storage {
        member,
        external,
        views: table,
    };
    // 6. Availability (§2, §2a).
    let shape = Shape {
        branches: root.scheme.branches(),
        levels: root.subtree_levels,
        available: root.available_levels,
        global: level,
        slots: root.contents.len(),
    };
    let availability = Availability::admit(subtree, &storage, shape, limits)?;
    // 7. Property tables (§5).
    let tables = subtree.get("propertyTables");
    let columns = match subtree.get("tileMetadata") {
        Some(index) => metadata::tile_table(
            root.schema,
            tables,
            index,
            availability.tile_popcount,
            &storage,
        )?,
        None => TileColumns::default(),
    };
    if !content_metadata.is_empty() && content_metadata.len() != root.contents.len() {
        return Err(invalid("contentMetadata must have one entry per content template").into());
    }
    for (slot, index) in content_metadata.iter().enumerate() {
        metadata::content_table(
            root.schema,
            tables,
            index,
            availability.slot_popcount(slot),
            &storage,
        )?;
    }
    drop(document);
    Ok(Subtree {
        storage,
        availability,
        columns,
        live_bytes: *live - start,
    })
}
