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

/// A JSON-format subtree: its first non-whitespace byte, after an optional
/// UTF-8 BOM, is `{`. Any other member without `subt` magic is malformed.
fn is_json_format(member: &[u8]) -> bool {
    let text = member.strip_prefix(b"\xEF\xBB\xBF").unwrap_or(member);
    text.iter()
        .find(|b| !matches!(b, b' ' | b'\t' | b'\n' | b'\r'))
        .is_some_and(|b| *b == b'{')
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
        if is_json_format(&member) {
            return Err(unsupported(format!("JSON-format subtree {name}")).into());
        }
        return Err(invalid(format!("invalid subtree magic: {name}")).into());
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

#[cfg(test)]
mod tests {
    use super::super::test_support::*;
    use super::super::{admit_root, ImplicitError, VisitedTile};
    use super::FormatError;
    use serde_json::{json, Value};

    const ROOT: &str = "s/0/0/0.subtree";

    /// QUADTREE `L = 1`, `A = 1`, no content: one subtree, one tile.
    fn one(member: Vec<u8>) -> Result<Vec<VisitedTile>, FormatError> {
        let tile = root_tile("QUADTREE", 1, 1, &[]);
        format(walk(&tile, &mut Archive::default().with(ROOT, member)))
    }
    fn minimal() -> Sub {
        Sub::new().tile(Av::Const(1)).child(Av::Const(0))
    }
    /// A one-byte tile bitstream: BIN of 8 bytes, buffer byteLength 1.
    fn with_bin() -> Sub {
        Sub::new().tile(Av::Bits(bits(1, &[0]))).child(Av::Const(0))
    }
    fn patch(sub: &Sub, key: &str, value: Value) -> Vec<u8> {
        sub.clone().set(key, value).build()
    }

    #[test]
    fn header_and_json_chunk() {
        assert_eq!(one(minimal().build()).unwrap().len(), 1);
        let good = minimal().build();
        // D-4 (version) and truncation.
        let mut bad = good.clone();
        bad[4] = 2;
        assert_invalid(one(bad), "version must be 1");
        assert_invalid(one(b"subt\x01\0\0\0".to_vec()), "header is truncated");
        // D-7: JSON-format subtrees, also after a UTF-8 BOM and whitespace.
        let json =
            br#"{"tileAvailability":{"constant":1},"childSubtreeAvailability":{"constant":0}}"#;
        assert_unsupported(one(json.to_vec()), "JSON-format subtree s/0/0/0.subtree");
        let prefixed = [&b"\xEF\xBB\xBF \t\r\n"[..], json].concat();
        assert_unsupported(one(prefixed), "JSON-format subtree");
        // D-4: any other member without `subt` magic is malformed.
        let mut magic = good.clone();
        magic[3] = b'x';
        assert_invalid(one(magic), "invalid subtree magic: s/0/0/0.subtree");
        for other in [
            &b""[..],
            b"sub",
            b" [1]",
            b"\xEF\xBB\xBF[",
            b"\xEF\xBB{}",
            b"x{}",
        ] {
            assert_invalid(one(other.to_vec()), "invalid subtree magic");
        }
        // Chunk lengths.
        for (at, value) in [(8, 0u64), (8, 12), (16, 4)] {
            let mut bad = good.clone();
            bad[at..at + 8].copy_from_slice(&value.to_le_bytes());
            assert_invalid(one(bad), "chunk lengths");
        }
        let mut longer = good.clone();
        longer.extend([0; 8]);
        assert_invalid(one(longer), "chunk lengths");
        // D-8: a JSON chunk ending in `\n` with no padding is admitted (RC-5).
        let mut text = serde_json::to_vec(&json!({
            "tileAvailability": {"constant": 1},
            "childSubtreeAvailability": {"constant": 0}
        }))
        .unwrap();
        while !(text.len() + 1).is_multiple_of(8) {
            text.insert(1, b' ');
        }
        text.push(b'\n');
        assert_eq!(one(container(&text, &[], b' ')).unwrap().len(), 1);
        // Non-whitespace padding is not JSON; the top level must be an object.
        let short =
            br#"{"tileAvailability":{"constant":1},"childSubtreeAvailability":{"constant":0}}"#;
        assert_invalid(one(container(short, &[], 0)), "invalid JSON");
        assert_invalid(one(container(b"[1]", &[], b' ')), "must be an object");
    }

    #[test]
    fn arrays_have_min_items() {
        // D-9 and the other schema `minItems` arrays (RC-15).
        for key in [
            "buffers",
            "bufferViews",
            "propertyTables",
            "contentAvailability",
            "contentMetadata",
        ] {
            assert_invalid(one(patch(&minimal(), key, json!([]))), "at least 1 item");
        }
    }

    #[test]
    fn internal_buffer_and_bin_chunk() {
        let sub = with_bin();
        assert_eq!(one(sub.build()).unwrap().len(), 1);
        // D-1: truncated BIN.
        assert_invalid(
            one(patch(&sub, "buffers", json!([{"byteLength": 9}]))),
            "differs from BIN",
        );
        // BIN padding must be shorter than 8 and zero (D-3).
        let mut two = with_bin();
        two.view(&[0; 9]);
        assert_invalid(
            one(patch(&two, "buffers", json!([{"byteLength": 8}]))),
            "differs from BIN",
        );
        let mut bad = sub.build();
        *bad.last_mut().unwrap() = 1;
        assert_invalid(one(bad), "nonzero BIN chunk padding");
        assert_invalid(
            one(patch(&sub, "buffers", json!([{"byteLength": 0}]))),
            "at least 1",
        );
        // A non-empty BIN needs an internal buffer, which must be buffer 0.
        assert_invalid(
            one(patch(
                &sub,
                "buffers",
                json!([{"uri": "b.bin", "byteLength": 1}]),
            )),
            "without an internal buffer",
        );
        let member = with_bin()
            .set("buffers", json!([{"byteLength": 1}, {"byteLength": 1}]))
            .build();
        assert_invalid(one(member), "only buffer 0");
        let no_bin = minimal().set("buffers", json!([{"byteLength": 1}])).build();
        assert_invalid(one(no_bin), "requires a BIN chunk");
    }

    #[test]
    fn buffer_views() {
        let sub = with_bin();
        let view = |value: Value| one(patch(&sub, "bufferViews", json!([value])));
        assert!(view(json!({"buffer": 0, "byteOffset": 0, "byteLength": 1})).is_ok());
        // D-2: byteOffset absent or unaligned.
        assert_invalid(view(json!({"buffer": 0, "byteLength": 1})), "byteOffset");
        let mut wide = with_bin();
        wide.view(&[0; 16]);
        let wide_view = |value: Value| {
            one(patch(
                &wide,
                "bufferViews",
                json!([{"buffer": 0, "byteOffset": 0, "byteLength": 1}, value]),
            ))
        };
        assert_invalid(
            wide_view(json!({"buffer": 0, "byteOffset": 1, "byteLength": 1})),
            "multiple of 8",
        );
        assert!(wide_view(json!({"buffer": 0, "byteOffset": 8, "byteLength": 16})).is_ok());
        assert_invalid(
            wide_view(json!({"buffer": 0, "byteOffset": 8, "byteLength": 17})),
            "exceeds its buffer",
        );
        assert_invalid(
            wide_view(json!({"buffer": 0, "byteOffset": 8, "byteLength": 0})),
            "at least 1",
        );
        assert_invalid(
            wide_view(json!({"buffer": 1, "byteOffset": 8, "byteLength": 1})),
            "names no buffer",
        );
        // Integers are non-negative and fit u64.
        assert_invalid(
            wide_view(json!({"buffer": 0, "byteOffset": -8, "byteLength": 1})),
            "non-negative integer",
        );
        assert_invalid(
            wide_view(json!({"buffer": 0, "byteOffset": 8.5, "byteLength": 1})),
            "non-negative integer",
        );
    }

    #[test]
    fn external_buffers() {
        let external = |uri: &str, length: u64| {
            Sub::new()
                .set("tileAvailability", json!({"bitstream": 0}))
                .set("childSubtreeAvailability", json!({"constant": 0}))
                .set("buffers", json!([{"uri": uri, "byteLength": length}]))
                .set(
                    "bufferViews",
                    json!([{"buffer": 0, "byteOffset": 0, "byteLength": 1}]),
                )
                .build()
        };
        // D-6: a valid external buffer, resolved relative to the subtree member.
        let member = external("b.bin", 1);
        let size = member.len() as u64;
        let tile = root_tile("QUADTREE", 1, 1, &[]);
        let mut archive = Archive::default()
            .with(ROOT, member)
            .with("s/0/0/b.bin", vec![1, 0]);
        let root = admit_root(&tile, "tileset.json", None).unwrap();
        let mut limits = limits();
        let (scan, refs) = (limits.scan_remaining, limits.refs_remaining);
        assert_eq!(
            walk_with(&root, &mut archive, &mut limits).unwrap().len(),
            1
        );
        // The buffer read is charged to the live path after the member.
        assert_eq!(
            archive.reads,
            vec![
                (ROOT.to_owned(), 1 << 26),
                ("s/0/0/b.bin".to_owned(), (1 << 26) - size)
            ]
        );
        // Reads are mirrored (two references, member + buffer bytes) and the
        // one-byte tile view is the whole scan.
        assert_eq!(limits.refs_remaining, refs - 2);
        assert_eq!(limits.scan_remaining, scan - size - 2 - 1);
        // Shorter than byteLength.
        let mut archive = Archive::default()
            .with(ROOT, external("b.bin", 3))
            .with("s/0/0/b.bin", vec![1, 0]);
        assert_invalid(
            format(walk(&tile, &mut archive)),
            "shorter than its byteLength",
        );
        // D-5: data URIs, any case.
        for uri in ["data:application/octet-stream;base64,AQ==", "DATA:,%01"] {
            assert_invalid(one(external(uri, 1)), "cannot be data URIs");
        }
        // A missing external member is the resolver's failure.
        let mut archive = Archive::default().with(ROOT, external("b.bin", 1));
        assert!(matches!(
            walk(&tile, &mut archive),
            Err(ImplicitError::Resolver(message)) if message.contains("missing archive entry")
        ));
    }

    #[test]
    fn extensions_are_unsupported_anywhere_but_extras() {
        assert_unsupported(
            one(patch(&minimal(), "extensions", json!({}))),
            "extensions in subtree JSON",
        );
        assert_unsupported(
            one(patch(
                &minimal(),
                "tileAvailability",
                json!({"constant": 1, "extensions": {"EXT_x": {}}}),
            )),
            "extensions in subtree JSON",
        );
        assert!(one(patch(&minimal(), "extras", json!({"extensions": {}}))).is_ok());
        // Unknown other properties are ignored.
        assert!(one(patch(&minimal(), "unknown", json!({"extensions": 1}))).is_err());
        assert!(one(patch(&minimal(), "unknown", json!(1))).is_ok());
    }
}
