//! Private immutable-byte GLB/glTF/b3dm integrity owner. No paths or operation defaults.
use super::{json, FormatError, PayloadError, PayloadLimits};
use serde_json::value::RawValue;
use std::{borrow::Cow, collections::BTreeSet};
mod numbers;
mod records;
use numbers::{at, domain, off, uint, work};
use records::{array, boolean, optional_record, pairs, push, record, string, Raw};
type FormatResult<T> = std::result::Result<T, FormatError>;
#[derive(Clone, Copy, Debug)]
pub(crate) enum PayloadKind {
    Glb,
    LocalGltf,
    B3dm,
}
#[derive(Debug)]
pub(crate) struct PayloadInspection {
    pub schema_uri: Option<String>,
    pub accessors_checked: u64,
    pub primitives_checked: u64,
    pub vertices: u64,
    pub elements_checked: u64,
    pub not_inspected: BTreeSet<String>,
}
fn invalid(s: impl Into<String>) -> FormatError {
    FormatError::InvalidInput(s.into())
}
fn unsupported(s: impl Into<String>) -> FormatError {
    FormatError::Unsupported(s.into())
}
fn limit(s: impl Into<String>) -> FormatError {
    FormatError::ResourceLimit(s.into())
}
fn range(data: &[u8], offset: usize, length: usize) -> FormatResult<&[u8]> {
    let end = offset
        .checked_add(length)
        .ok_or_else(|| invalid("byte range overflow"))?;
    data.get(offset..end)
        .ok_or_else(|| invalid("byte range outside actual resource"))
}
fn word(data: &[u8], offset: usize) -> FormatResult<usize> {
    Ok(u32::from_le_bytes(range(data, offset, 4)?.try_into().unwrap()) as usize)
}
fn envelope(bytes: &[u8]) -> FormatResult<(&[u8], Option<&[u8]>)> {
    if range(bytes, 0, 4)? != b"glTF" || word(bytes, 4)? != 2 || word(bytes, 8)? != bytes.len() {
        return Err(invalid("invalid GLB v2 header/length"));
    }
    let mut cursor = 12usize;
    let mut json = None;
    let mut bin = None;
    let mut chunk = 0;
    while cursor < bytes.len() {
        let n = word(bytes, cursor)?;
        let kind = word(
            bytes,
            cursor
                .checked_add(4)
                .ok_or_else(|| invalid("chunk offset overflow"))?,
        )?;
        let start = cursor
            .checked_add(8)
            .ok_or_else(|| invalid("chunk offset overflow"))?;
        let data = range(bytes, start, n)?;
        if n % 4 != 0 {
            return Err(invalid("GLB chunk length is not aligned"));
        }
        if chunk == 0 && kind != 0x4e4f534a {
            return Err(invalid("first GLB chunk must be JSON"));
        }
        match kind {
            0x4e4f534a => {
                if json.is_some() {
                    return Err(invalid("duplicate JSON chunk"));
                }
                json = Some(data)
            }
            0x004e4942 => {
                if bin.is_some() || chunk != 1 {
                    return Err(invalid("BIN must be the second chunk, at most once"));
                }
                bin = Some(data)
            }
            _ => {}
        }
        cursor = start
            .checked_add(n)
            .ok_or_else(|| invalid("chunk range overflow"))?;
        chunk += 1;
    }
    Ok((json.ok_or_else(|| invalid("missing GLB JSON"))?, bin))
}
#[derive(Clone)]
struct Accessor {
    view: Option<usize>,
    offset: usize,
    stride: usize,
    count: usize,
    component: usize,
    components: usize,
    component_offsets: Vec<usize>,
    normalized: bool,
    shape: String,
    min: Vec<f64>,
    max: Vec<f64>,
    ordinary_max: f64,
    shortest_segment: usize,
    has_bounds: bool,
}
fn scalar(bytes: &[u8], component: usize) -> f64 {
    match component {
        5120 => i8::from_le_bytes([bytes[0]]) as f64,
        5121 => bytes[0] as f64,
        5122 => i16::from_le_bytes(bytes[..2].try_into().unwrap()) as f64,
        5123 => u16::from_le_bytes(bytes[..2].try_into().unwrap()) as f64,
        5125 => u32::from_le_bytes(bytes[..4].try_into().unwrap()) as f64,
        5126 => f32::from_le_bytes(bytes[..4].try_into().unwrap()) as f64,
        _ => unreachable!(),
    }
}
fn value(a: &Accessor, views: &[Cow<'_, [u8]>], i: usize, c: usize) -> f64 {
    a.view.map_or(0., |view| {
        scalar(
            &views[view][a.offset + i * a.stride + a.component_offsets[c]..],
            a.component,
        )
    })
}
fn numeric_set(name: &str, prefix: &str) -> bool {
    let suffix = &name[prefix.len()..];
    !suffix.is_empty()
        && (suffix == "0" || !suffix.starts_with('0'))
        && suffix.bytes().all(|b| b.is_ascii_digit())
}
fn normalized(v: f64, c: usize) -> f64 {
    match c {
        5120 => (v / 127.).max(-1.),
        5121 => v / 255.,
        5122 => (v / 32767.).max(-1.),
        5123 => v / 65535.,
        _ => v,
    }
}
fn minimum_version(raw: Raw<'_>) -> FormatResult<()> {
    let Some(raw) = raw else {
        return Ok(());
    };
    let version = string(Some(raw))?;
    let (major, minor) = version
        .split_once('.')
        .filter(|(major, minor)| {
            !major.is_empty()
                && !minor.is_empty()
                && major.bytes().all(|b| b.is_ascii_digit())
                && minor.bytes().all(|b| b.is_ascii_digit())
        })
        .ok_or_else(|| invalid("asset.minVersion must have digit major.minor syntax"))?;
    let major = major.trim_start_matches('0');
    let minor = minor.trim_start_matches('0');
    if !(major.is_empty() || major == "1" || (major == "2" && minor.is_empty())) {
        return Err(unsupported(format!("minimum glTF version {version}")));
    }
    Ok(())
}
fn allocation<T: Clone>(length: usize, fill: T) -> FormatResult<Vec<T>> {
    let requested = length
        .checked_mul(std::mem::size_of::<T>())
        .ok_or_else(|| limit("allocation byte overflow"))?;
    if requested > isize::MAX as usize {
        return Err(limit("allocation outside host range"));
    }
    let mut out = Vec::new();
    out.try_reserve_exact(length)
        .map_err(|_| limit("allocation failed"))?;
    out.resize(length, fill);
    Ok(out)
}
fn sextet(byte: u8) -> Option<u8> {
    match byte {
        b'A'..=b'Z' => Some(byte - b'A'),
        b'a'..=b'z' => Some(byte - b'a' + 26),
        b'0'..=b'9' => Some(byte - b'0' + 52),
        b'+' => Some(62),
        b'/' => Some(63),
        _ => None,
    }
}
fn decode_buffer_uri(uri: &str, individual: usize, remaining: usize) -> FormatResult<Vec<u8>> {
    use base64::Engine;
    let encoded = uri
        .strip_prefix("data:application/octet-stream;base64,")
        .or_else(|| uri.strip_prefix("data:application/gltf-buffer;base64,"))
        .ok_or_else(|| unsupported("data buffer media type or encoding"))?;
    // Preserve the existing conservative individual gate before malformed-data checks.
    let upper = encoded
        .len()
        .checked_add(3)
        .and_then(|n| (n / 4).checked_mul(3))
        .ok_or_else(|| limit("data buffer decoded size overflow"))?;
    if upper > individual {
        return Err(limit("data buffer exceeds individual decoded byte limit"));
    }
    let bytes = encoded.as_bytes();
    if !bytes.len().is_multiple_of(4) {
        return Err(invalid("invalid standard base64 length"));
    }
    let padding = bytes.iter().rev().take_while(|b| **b == b'=').count();
    if padding > 2
        || bytes[..bytes.len() - padding]
            .iter()
            .any(|b| sextet(*b).is_none())
    {
        return Err(invalid("invalid standard base64 alphabet/padding"));
    }
    if padding > 0 && bytes.len() < 4 {
        return Err(invalid("invalid standard base64 padding"));
    }
    if padding == 2 && sextet(bytes[bytes.len() - 3]).is_none_or(|v| v & 15 != 0)
        || padding == 1 && sextet(bytes[bytes.len() - 2]).is_none_or(|v| v & 3 != 0)
    {
        return Err(invalid("noncanonical base64 trailing bits"));
    }
    let actual = (bytes.len() / 4)
        .checked_mul(3)
        .and_then(|n| n.checked_sub(padding))
        .ok_or_else(|| invalid("invalid base64 size"))?;
    if actual > remaining {
        return Err(limit(
            "actual inline resource exceeds remaining resource allowance",
        ));
    }
    let mut out = allocation(actual, 0u8)?;
    let length = base64::engine::general_purpose::STANDARD
        .decode_slice(encoded, &mut out)
        .map_err(|e| invalid(format!("invalid base64 buffer: {e}")))?;
    if length != actual {
        return Err(invalid("base64 decoded size disagrees with preflight"));
    }
    Ok(out)
}
struct ViewPlan<'a> {
    buffer: usize,
    offset: usize,
    length: usize,
    stride: Option<usize>,
    target: Option<usize>,
    meshopt: Option<records::Meshopt<'a>>,
    meshopt_source: Option<usize>,
}
fn decode_meshopt(
    e: &records::Meshopt<'_>,
    source: usize,
    buffers: &[Cow<'_, [u8]>],
    ceiling: usize,
) -> FormatResult<(Vec<u8>, usize)> {
    let compressed = range(&buffers[source], off(e.offset)?, uint(e.length)?)?;
    let count = usize::try_from(work(e.count, ceiling as u64)?)
        .map_err(|_| limit("meshopt count outside host range"))?;
    let stride = uint(e.stride)?;
    if count == 0 {
        return Err(invalid("empty meshopt stream"));
    }
    let length = count
        .checked_mul(stride)
        .ok_or_else(|| limit("meshopt decoded size overflow"))?;
    if length > ceiling {
        return Err(limit("meshopt output exceeds remaining view allowance"));
    }
    let filter = e
        .filter
        .map_or_else(|| Ok("NONE".to_owned()), |v| string(Some(v)))?;
    if filter != "NONE" {
        return Err(unsupported("meshopt filters other than NONE"));
    }
    let mode = string(e.mode)?;
    match mode.as_str() {
        "ATTRIBUTES" if stride > 0 && stride <= 256 && stride % 4 == 0 => {}
        "TRIANGLES" if [2, 4].contains(&stride) && count % 3 == 0 => {}
        "INDICES" if [2, 4].contains(&stride) => {}
        "ATTRIBUTES" | "TRIANGLES" | "INDICES" => {
            return Err(invalid("invalid meshopt stride/count"))
        }
        _ => return Err(invalid("invalid meshopt mode")),
    }
    let mode = match mode.as_str() {
        "ATTRIBUTES" => super::meshopt::Mode::Attributes,
        "TRIANGLES" => super::meshopt::Mode::Triangles,
        _ => super::meshopt::Mode::Indices,
    };
    let out = super::meshopt::decode(compressed, count, stride, mode)?;
    Ok((out, stride))
}
fn unwrap_b3dm(bytes: &[u8], limits: super::JsonLimits) -> FormatResult<&[u8]> {
    if bytes.len() < 28 || &bytes[..4] != b"b3dm" {
        return Err(invalid("truncated or invalid b3dm header"));
    }
    let word = |offset| u32::from_le_bytes(bytes[offset..offset + 4].try_into().unwrap()) as usize;
    if word(4) != 1 {
        return Err(unsupported("b3dm version other than 1"));
    }
    if word(8) != bytes.len() || !bytes.len().is_multiple_of(8) {
        return Err(invalid("b3dm length/alignment invalid"));
    }
    let lengths = [word(12), word(16), word(20), word(24)];
    if lengths[0] == 0 || (lengths[2] == 0 && lengths[3] != 0) {
        return Err(invalid("b3dm table framing invalid"));
    }
    let mut offset = 28usize;
    for (index, length) in lengths.iter().enumerate() {
        let table = range(bytes, offset, *length)?;
        if index == 0 || (index == 2 && *length > 0) {
            let doc = json::admit(table, limits)?;
            let feature: records::FeatureTable<'_> = record(Some(doc.raw()))?;
            if index == 0 {
                domain(feature.batch_length, u32::MAX as u64)?;
            }
        }
        offset = offset
            .checked_add(*length)
            .ok_or_else(|| invalid("b3dm table length overflow"))?;
        if !offset.is_multiple_of(8) {
            return Err(invalid("b3dm table boundary must be eight-byte aligned"));
        }
    }
    let header = range(bytes, offset, 12)?;
    if &header[..4] != b"glTF" {
        return Err(invalid("b3dm does not contain GLB"));
    }
    let length = u32::from_le_bytes(header[8..12].try_into().unwrap()) as usize;
    let glb = range(bytes, offset, length)?;
    let end = offset
        .checked_add(length)
        .ok_or_else(|| invalid("embedded GLB length overflow"))?;
    if bytes.len() - end > 7 || bytes[end..].iter().any(|b| *b != 0) {
        return Err(invalid("invalid b3dm trailing padding"));
    }
    Ok(glb)
}
// Extension declarations are semantic checks. Opaque extras and extension bodies
// remain outside this traversal; JSON admission already checked their structure.
fn check_extensions(raw: &RawValue, used: &BTreeSet<String>, ceiling: usize) -> FormatResult<()> {
    let mut pending = Vec::new();
    push(&mut pending, raw, ceiling)?;
    while let Some(value) = pending.pop() {
        match value.get().as_bytes().first() {
            Some(b'[') => {
                for child in array::<&RawValue>(Some(value), ceiling)? {
                    push(&mut pending, child, ceiling)?;
                }
            }
            Some(b'{') => {
                for (key, child) in pairs(Some(value), ceiling)?.0 {
                    if key == "extensions" {
                        for (name, payload) in pairs(Some(child), ceiling)?.0 {
                            if !used.contains(&name) {
                                return Err(invalid(format!("undeclared extension {name}")));
                            }
                            if !payload.get().starts_with('{') {
                                return Err(invalid("extension payload must be object"));
                            }
                        }
                    } else if key != "extras" {
                        push(&mut pending, child, ceiling)?;
                    }
                }
            }
            _ => {}
        }
    }
    Ok(())
}
pub(crate) fn inspect<E>(
    kind: PayloadKind,
    bytes: &[u8],
    limits: PayloadLimits,
    remaining_components: u64,
    mut resolve: impl FnMut(&str, usize) -> std::result::Result<Vec<u8>, E>,
) -> std::result::Result<PayloadInspection, PayloadError<E>> {
    if bytes.len() > limits.member_bytes {
        return Err(limit("payload exceeds member byte allowance").into());
    }
    let (source, bin) = match kind {
        PayloadKind::B3dm => envelope(unwrap_b3dm(bytes, limits.json)?)?,
        PayloadKind::Glb => envelope(bytes)?,
        PayloadKind::LocalGltf => (bytes, None),
    };
    let document = json::admit(source, limits.json)?;
    let slots = document.value_nodes();
    let root: records::Root<'_> = record(Some(document.raw()))?;
    let asset: records::Asset<'_> = record(root.asset)?;
    if string(asset.version)? != "2.0" {
        return Err(invalid("glTF asset.version must be 2.0").into());
    }
    minimum_version(asset.minimum)?;
    let used: BTreeSet<String> = array::<String>(root.used, slots)?.into_iter().collect();
    let required: BTreeSet<String> = array::<String>(root.required, slots)?.into_iter().collect();
    let supported = [
        "KHR_mesh_quantization",
        "EXT_meshopt_compression",
        "KHR_materials_unlit",
        "KHR_mesh_primitive_restart",
    ];
    for ext in &required {
        if !used.contains(ext) {
            return Err(invalid("required extension absent from extensionsUsed").into());
        }
        if !supported.contains(&ext.as_str()) {
            return Err(unsupported(format!("required extension {ext}")).into());
        }
    }
    let restart = used.contains("KHR_mesh_primitive_restart");
    if restart && !required.contains("KHR_mesh_primitive_restart") {
        return Err(invalid("primitive restart extension must be required").into());
    }
    let accessor_docs = array::<records::Accessor<'_>>(root.accessors, slots)?;
    let element_cap = remaining_components.min(limits.accessor_components);
    let mut planned_elements = 0u64;
    let mut accessor_plans = Vec::new();
    for a in accessor_docs {
        if a.sparse.is_some() {
            return Err(unsupported("sparse accessor").into());
        }
        let count = work(a.count, element_cap.saturating_sub(planned_elements))?;
        let shape = string(a.shape)?;
        let components = match shape.as_str() {
            "SCALAR" => 1,
            "VEC2" => 2,
            "VEC3" => 3,
            "VEC4" | "MAT2" => 4,
            "MAT3" => 9,
            "MAT4" => 16,
            _ => return Err(invalid("invalid accessor shape").into()),
        };
        planned_elements = planned_elements
            .checked_add(
                count
                    .checked_mul(components)
                    .ok_or_else(|| limit("accessor element overflow"))?,
            )
            .ok_or_else(|| limit("accessor aggregate overflow"))?;
        if planned_elements > element_cap {
            return Err(limit("accessor elements exceed remaining document/archive budget").into());
        }
        let count =
            usize::try_from(count).map_err(|_| limit("accessor count outside host range"))?;
        push(&mut accessor_plans, (a, count, shape), slots)?;
    }
    let mut not_inspected = used
        .iter()
        .filter(|s| !supported.contains(&s.as_str()))
        .cloned()
        .collect::<BTreeSet<_>>();
    if matches!(kind, PayloadKind::B3dm) {
        not_inspected.insert("b3dm feature and batch table semantics".into());
    }
    check_extensions(document.raw(), &used, slots)?;
    let buffer_docs = array::<records::Buffer<'_>>(root.buffers, slots)?;
    let mut lengths = Vec::new();
    let mut declared_bytes = 0usize;
    // Complete declared-buffer and accessor plans precede the first resolver call.
    for b in &buffer_docs {
        let n = usize::try_from(work(
            b.length,
            limits.member_bytes.saturating_sub(declared_bytes) as u64,
        )?)
        .map_err(|_| limit("buffer length outside host range"))?;
        if n == 0 {
            return Err(invalid("empty buffer").into());
        }
        declared_bytes = declared_bytes
            .checked_add(n)
            .ok_or_else(|| limit("buffer budget overflow"))?;
        push(&mut lengths, n, slots)?;
    }
    let view_docs = array::<records::View<'_>>(root.views, slots)?;
    let mut plans = Vec::new();
    let mut placeholder_covered = allocation(buffer_docs.len(), true)?;
    for v in view_docs {
        let buffer = at(&buffer_docs, v.buffer)?;
        // Declared fallback range is a format range gate, before native loss.
        let offset = off(v.offset)?;
        let length = usize::try_from(domain(v.length, lengths[buffer] as u64)?)
            .map_err(|_| invalid("view length outside host range"))?;
        if length == 0 {
            return Err(invalid("empty bufferView").into());
        }
        if offset
            .checked_add(length)
            .is_none_or(|end| end > lengths[buffer])
        {
            return Err(invalid("bufferView exceeds declared buffer byteLength").into());
        }
        let stride = v.stride.map(|v| uint(Some(v))).transpose()?;
        if stride.is_some_and(|s| !(4..=252).contains(&s) || s % 4 != 0) {
            return Err(invalid("invalid bufferView byteStride").into());
        }
        let target = v.target.map(|v| uint(Some(v))).transpose()?;
        if target.is_some_and(|t| ![34962, 34963].contains(&t)) {
            return Err(invalid("invalid bufferView target").into());
        }
        let extensions: records::Extensions<'_> = optional_record(v.extensions)?;
        let meshopt = extensions
            .meshopt
            .map(|e| record::<records::Meshopt<'_>>(Some(e)))
            .transpose()?;
        let meshopt_source = meshopt
            .as_ref()
            .map(|e| at(&buffer_docs, e.buffer))
            .transpose()?;
        if let Some(source) = meshopt_source {
            placeholder_covered[source] = false;
        } else {
            placeholder_covered[buffer] = false;
        }
        push(
            &mut plans,
            ViewPlan {
                buffer,
                offset,
                length,
                stride,
                target,
                meshopt,
                meshopt_source,
            },
            slots,
        )?;
    }
    let meshopt_required = required.contains("EXT_meshopt_compression");
    let mut buffers = Vec::new();
    let mut resource_bytes = 0usize;
    for (i, b) in buffer_docs.into_iter().enumerate() {
        let extensions: records::Extensions<'_> = optional_record(b.extensions)?;
        if let Some(e) = extensions.meshopt {
            let marker: records::Fallback<'_> = record(Some(e))?;
            if let Some(raw) = marker.fallback {
                if boolean(Some(raw))? && !placeholder_covered[i] {
                    return Err(
                        invalid("meshopt fallback buffer has non-fallback references").into(),
                    );
                }
            }
        }
        let remaining = limits
            .member_bytes
            .checked_sub(resource_bytes)
            .ok_or_else(|| limit("resource byte allowance exhausted"))?;
        let mut data = if let Some(raw) = b.uri {
            let uri = string(Some(raw))?;
            if uri.starts_with("data:") {
                Cow::Owned(decode_buffer_uri(&uri, limits.decoded_bytes, remaining)?)
            } else {
                Cow::Owned(resolve(&uri, remaining).map_err(PayloadError::Resolver)?)
            }
        } else if let (0, Some(bin)) = (i, bin) {
            Cow::Borrowed(bin)
        } else if placeholder_covered[i] && meshopt_required {
            push(&mut buffers, Cow::Borrowed(&[][..]), slots)?;
            continue;
        } else {
            return Err(invalid("buffer has no resource").into());
        };
        if data.len() > remaining {
            return Err(
                limit("actual buffer resources exceed remaining resource allowance").into(),
            );
        }
        resource_bytes = resource_bytes
            .checked_add(data.len())
            .ok_or_else(|| limit("actual buffer byte overflow"))?;
        let n = lengths[i];
        if data.len() < n
            || (b.uri.is_none() && (data.len() - n > 3 || data[n..].iter().any(|v| *v != 0)))
        {
            return Err(invalid("buffer byteLength does not match actual resource").into());
        }
        match &mut data {
            Cow::Owned(bytes) => bytes.truncate(n),
            Cow::Borrowed(bytes) => *bytes = &bytes[..n],
        }
        push(&mut buffers, data, slots)?;
    }
    drop(placeholder_covered);
    drop(lengths);
    let mut views = Vec::new();
    let mut view_budget = 0usize;
    for plan in &mut plans {
        let remaining = limits
            .decoded_bytes
            .checked_sub(view_budget)
            .ok_or_else(|| limit("view byte allowance exhausted"))?;
        if plan.length > remaining {
            return Err(limit("views exceed decoded byte allowance").into());
        }
        if let Some(e) = plan.meshopt.take() {
            if !used.contains("EXT_meshopt_compression") {
                return Err(invalid("meshopt extension not declared").into());
            }
            let (decoded, stride) =
                decode_meshopt(&e, plan.meshopt_source.unwrap(), &buffers, remaining)?;
            if decoded.len() != plan.length {
                return Err(invalid("meshopt decoded size differs from bufferView").into());
            }
            if plan.stride.is_some_and(|s| s != stride) {
                return Err(invalid("meshopt and bufferView stride disagree").into());
            }
            push(&mut views, Cow::Owned(decoded), slots)?;
        } else {
            push(
                &mut views,
                Cow::Borrowed(range(&buffers[plan.buffer], plan.offset, plan.length)?),
                slots,
            )?;
        }
        // A view is charged once whether it borrows or owns decoded storage.
        view_budget = view_budget
            .checked_add(plan.length)
            .ok_or_else(|| limit("view byte sum overflow"))?;
    }
    let mut accessors = Vec::new();
    let mut elements = 0usize;
    for (a, count, shape) in accessor_plans {
        if count == 0 {
            return Err(invalid("empty accessor").into());
        }
        let component = uint(a.component)?;
        let size = match component {
            5120 | 5121 => 1,
            5122 | 5123 => 2,
            5125 | 5126 => 4,
            _ => return Err(invalid("invalid componentType").into()),
        };
        let (rows, cols) = match shape.as_str() {
            "SCALAR" => (1, 1),
            "VEC2" => (2, 1),
            "VEC3" => (3, 1),
            "VEC4" => (4, 1),
            "MAT2" => (2, 2),
            "MAT3" => (3, 3),
            "MAT4" => (4, 4),
            _ => return Err(invalid("invalid accessor shape").into()),
        };
        let normalized = a.normalized.map_or(Ok(false), |v| boolean(Some(v)))?;
        if normalized && [5125, 5126].contains(&component) {
            return Err(invalid("invalid normalized component type").into());
        }
        elements = elements
            .checked_add(
                count
                    .checked_mul(rows * cols)
                    .ok_or_else(|| limit("accessor element overflow"))?,
            )
            .ok_or_else(|| limit("accessor aggregate overflow"))?;
        if elements as u64 > element_cap {
            return Err(limit("accessor components exceed allowance").into());
        }
        let column = if cols > 1 {
            (rows * size).next_multiple_of(4)
        } else {
            rows * size
        };
        let element = column * cols;
        let occupied = column * (cols - 1) + rows * size;
        let mut offsets = Vec::new();
        for c in 0..cols {
            for r in 0..rows {
                push(&mut offsets, c * column + r * size, 16)?;
            }
        }
        let view = a.view.map(|v| at(&views, Some(v))).transpose()?;
        let offset = off(a.offset)?;
        let stride = view.map_or(element, |v| plans[v].stride.unwrap_or(element));
        if offset % size != 0 || stride < element || (cols > 1 && size < 4 && offset % 4 != 0) {
            return Err(invalid("accessor alignment/stride invalid").into());
        }
        if let Some(v) = view {
            let absolute = plans[v]
                .offset
                .checked_add(offset)
                .ok_or_else(|| invalid("accessor absolute offset overflow"))?;
            if absolute % size != 0 || (cols > 1 && size < 4 && absolute % 4 != 0) {
                return Err(invalid("accessor absolute alignment invalid").into());
            }
            let length = (count - 1)
                .checked_mul(stride)
                .and_then(|n| n.checked_add(occupied))
                .ok_or_else(|| invalid("accessor range overflow"))?;
            range(&views[v], offset, length)?;
        } else if a.offset.is_some() {
            return Err(invalid("accessor without view must omit byteOffset").into());
        }
        let mut parsed = Accessor {
            view,
            offset,
            stride,
            count,
            component,
            components: rows * cols,
            component_offsets: offsets,
            normalized,
            shape,
            min: Vec::new(),
            max: Vec::new(),
            ordinary_max: f64::NEG_INFINITY,
            shortest_segment: usize::MAX,
            has_bounds: a.min.is_some() && a.max.is_some(),
        };
        let mut min = allocation(parsed.components, f64::INFINITY)?;
        let mut max = allocation(parsed.components, f64::NEG_INFINITY)?;
        let reserved = match component {
            5121 => 255.,
            5123 => 65535.,
            5125 => 4294967295.,
            _ => f64::INFINITY,
        };
        let mut segment = 0usize;
        for i in 0..count {
            for c in 0..parsed.components {
                let n = value(&parsed, &views, i, c);
                if !n.is_finite() {
                    return Err(invalid("nonfinite accessor float").into());
                }
                if parsed.components == 1 && c == 0 {
                    if n == reserved {
                        parsed.shortest_segment = parsed.shortest_segment.min(segment);
                        segment = 0;
                    } else {
                        parsed.ordinary_max = parsed.ordinary_max.max(n);
                        segment += 1;
                    }
                }
                min[c] = min[c].min(n);
                max[c] = max[c].max(n);
            }
        }
        parsed.shortest_segment = parsed.shortest_segment.min(segment);
        for (bound, actual) in [(a.min, &min), (a.max, &max)] {
            if let Some(bound) = bound {
                let declared = array::<&RawValue>(Some(bound), slots)?;
                if declared.len() != parsed.components {
                    return Err(invalid("accessor bounds shape mismatch").into());
                }
                for (raw, actual) in declared.iter().zip(actual) {
                    let n = serde_json::from_str::<f64>(raw.get())
                        .ok()
                        .filter(|n| n.is_finite())
                        .ok_or_else(|| invalid("accessor bound not finite number"))?;
                    let n = if component == 5126 {
                        f64::from(n as f32)
                    } else {
                        n
                    };
                    if !n.is_finite() {
                        return Err(invalid("accessor bound outside finite component range").into());
                    }
                    // Published arbitrary-bound exemption applies only to viewless nonsparse storage.
                    if view.is_some() && n != *actual {
                        return Err(
                            invalid("declared accessor bounds differ from decoded values").into(),
                        );
                    }
                }
            }
        }
        parsed.min = min;
        parsed.max = max;
        push(&mut accessors, parsed, slots)?;
    }
    let mut view_roles = allocation(views.len(), 0u8)?;
    let mut attribute_accessors = allocation(views.len(), None)?;
    let mut accessor_views = allocation(views.len(), false)?;
    for accessor in &accessors {
        if let Some(view) = accessor.view {
            accessor_views[view] = true;
        }
    }
    let mut index_accessors = allocation(accessors.len(), false)?;
    let mut vertices = 0u64;
    let mut primitives = 0u64;
    let materials = array::<&RawValue>(root.materials, slots)?;
    for mesh in array::<&RawValue>(root.meshes, slots)? {
        let mesh: records::Mesh<'_> = uninspected_record(Some(mesh))?;
        for p in array::<records::Primitive<'_>>(mesh.primitives, slots)? {
            primitives = primitives
                .checked_add(1)
                .ok_or_else(|| limit("primitive count overflow"))?;
            let mode = p.mode.map_or(Ok(4), |v| {
                usize::try_from(domain(Some(v), 6)?)
                    .map_err(|_| invalid("primitive mode outside host range"))
            })?;
            if ![0, 1, 3, 4].contains(&mode) {
                return Err(unsupported(format!("primitive mode {mode}")).into());
            }
            let extensions: records::Extensions<'_> = optional_record(p.extensions)?;
            if extensions.draco.is_some() {
                return Err(unsupported("Draco primitive").into());
            }
            if let Some(polygon) = extensions.polygon {
                if mode != 4 || p.indices.is_none() {
                    return Err(invalid("polygon extension requires indexed TRIANGLES").into());
                }
                let polygon: records::Polygon<'_> = record(Some(polygon))?;
                let polygons = usize::try_from(work(polygon.count, element_cap)?)
                    .map_err(|_| limit("polygon count outside host range"))?;
                if polygons == 0 {
                    return Err(invalid("polygon count must be positive").into());
                }
                for (raw, is_loop) in [
                    (polygon.offsets, false),
                    (polygon.loops, true),
                    (polygon.loop_offsets, false),
                ] {
                    let index = at(&accessors, raw)?;
                    let accessor = &accessors[index];
                    if accessor.shape != "SCALAR"
                        || ![5121, 5123, 5125].contains(&accessor.component)
                        || accessor.normalized
                    {
                        return Err(
                            invalid("polygon reference must be unsigned SCALAR accessor").into(),
                        );
                    }
                    if !is_loop && accessor.count != polygons {
                        return Err(invalid("polygon offset accessor count differs").into());
                    }
                    if let Some(view) = accessor.view {
                        if view_roles[view] == 1
                            || plans[view].stride.is_some()
                            || plans[view].target.is_some_and(|target| target != 34963)
                        {
                            return Err(invalid("polygon accessor view mixes vertex attributes or has invalid stride/target").into());
                        }
                        view_roles[view] = 2;
                    }
                    index_accessors[index] = true;
                }
            }
            let mut attrs = Vec::new();
            for (name, raw) in pairs(p.attributes, slots)?.0 {
                let index = at(&accessors, Some(raw))?;
                push(&mut attrs, (name, index), slots)?;
            }
            let position = attrs
                .iter()
                .find(|(name, _)| name == "POSITION")
                .ok_or_else(|| invalid("primitive POSITION missing"))?
                .1;
            for prefix in ["COLOR_", "TEXCOORD_"] {
                let mut sets = Vec::new();
                for (semantic, _) in attrs.iter().filter(|(name, _)| name.starts_with(prefix)) {
                    if !numeric_set(semantic, prefix) {
                        return Err(invalid("invalid indexed attribute suffix").into());
                    }
                    let set = semantic[prefix.len()..]
                        .parse::<usize>()
                        .map_err(|_| invalid("attribute set index exceeds supported range"))?;
                    push(&mut sets, set, slots)?;
                }
                sets.sort_unstable();
                if sets
                    .iter()
                    .enumerate()
                    .any(|(expected, actual)| expected != *actual)
                {
                    return Err(invalid(
                        "indexed attribute sets must start at zero and be consecutive",
                    )
                    .into());
                }
            }
            let count = accessors[position].count;
            if !accessors[position].has_bounds {
                return Err(invalid("POSITION accessor requires min and max").into());
            }
            vertices = vertices
                .checked_add(count as u64)
                .ok_or_else(|| limit("vertex count overflow"))?;
            for (semantic, index) in attrs {
                let a = &accessors[index];
                if a.count != count {
                    return Err(invalid("primitive attribute counts differ").into());
                }
                if let Some(v) = a.view {
                    if view_roles[v] == 2 {
                        return Err(
                            invalid("bufferView mixes indices and vertex attributes").into()
                        );
                    }
                    if attribute_accessors[v].is_some_and(|previous| previous != index)
                        && plans[v].stride.is_none()
                    {
                        return Err(invalid(
                            "distinct vertex attributes sharing a bufferView require byteStride",
                        )
                        .into());
                    }
                    attribute_accessors[v] = Some(index);
                    view_roles[v] = 1;
                    let absolute = plans[v]
                        .offset
                        .checked_add(a.offset)
                        .ok_or_else(|| invalid("attribute absolute offset overflow"))?;
                    if absolute % 4 != 0 || a.stride % 4 != 0 {
                        return Err(invalid("vertex attribute is not four-byte aligned").into());
                    }
                    if plans[v].target.is_some_and(|target| target != 34962) {
                        return Err(invalid("attribute view target mismatch").into());
                    }
                }
                let quant = used.contains("KHR_mesh_quantization");
                let valid = match semantic.as_str() {
                    "POSITION" => {
                        a.shape == "VEC3"
                            && ((a.component == 5126 && !a.normalized)
                                || (quant && [5120, 5121, 5122, 5123].contains(&a.component)))
                    }
                    "NORMAL" => {
                        a.shape == "VEC3"
                            && ((a.component == 5126 && !a.normalized)
                                || (quant && [5120, 5122].contains(&a.component) && a.normalized))
                    }
                    "TANGENT" => {
                        a.shape == "VEC4"
                            && ((a.component == 5126 && !a.normalized)
                                || (quant && [5120, 5122].contains(&a.component) && a.normalized))
                    }
                    s if s.starts_with("TEXCOORD_") => {
                        if !numeric_set(s, "TEXCOORD_") {
                            return Err(invalid("invalid TEXCOORD set suffix").into());
                        }
                        a.shape == "VEC2"
                            && ((a.component == 5126 && !a.normalized)
                                || ([5121, 5123].contains(&a.component) && a.normalized)
                                || (quant && [5120, 5121, 5122, 5123].contains(&a.component)))
                    }
                    s if s.starts_with("COLOR_") => {
                        if !numeric_set(s, "COLOR_") {
                            return Err(invalid("invalid COLOR set suffix").into());
                        }
                        ["VEC3", "VEC4"].contains(&a.shape.as_str())
                            && ((a.component == 5126 && !a.normalized)
                                || ([5121, 5123].contains(&a.component) && a.normalized))
                    }
                    s if s.starts_with('_') => a.component != 5125,
                    _ => return Err(unsupported(format!("attribute semantic {semantic}")).into()),
                };
                if !valid {
                    return Err(invalid(format!("invalid {semantic} accessor type")).into());
                }
                if a.component != 5126
                    && (["POSITION", "NORMAL", "TANGENT"].contains(&semantic.as_str())
                        || (semantic.starts_with("TEXCOORD_")
                            && !([5121, 5123].contains(&a.component) && a.normalized)))
                    && quant
                    && !required.contains("KHR_mesh_quantization")
                {
                    return Err(invalid(
                        "quantized attribute requires KHR_mesh_quantization in extensionsRequired",
                    )
                    .into());
                }
                if semantic.starts_with("COLOR_") {
                    for raw in a.min.iter().chain(&a.max) {
                        let n = if a.normalized {
                            normalized(*raw, a.component)
                        } else {
                            *raw
                        };
                        if !(0.0..=1.0).contains(&n) {
                            return Err(invalid("COLOR value outside zero to one").into());
                        }
                    }
                }
            }
            let n = if let Some(raw) = p.indices {
                let i = at(&accessors, Some(raw))?;
                index_accessors[i] = true;
                let a = &accessors[i];
                if a.shape != "SCALAR" || ![5121, 5123, 5125].contains(&a.component) || a.normalized
                {
                    return Err(invalid("invalid index accessor type").into());
                }
                if let Some(v) = a.view {
                    if view_roles[v] == 1 {
                        return Err(
                            invalid("bufferView mixes indices and vertex attributes").into()
                        );
                    }
                    view_roles[v] = 2;
                    if plans[v].stride.is_some()
                        || plans[v].target.is_some_and(|target| target != 34963)
                    {
                        return Err(invalid("index view stride/target invalid").into());
                    }
                }
                let reserved = match a.component {
                    5121 => 255.,
                    5123 => 65535.,
                    _ => 4294967295.,
                };
                let has_restart = a.max[0] == reserved;
                if has_restart && !(restart && mode == 3) {
                    return Err(invalid(
                        "primitive restart index is not allowed for this core mode",
                    )
                    .into());
                }
                if has_restart && a.shortest_segment < 2 {
                    return Err(unsupported("restart LINE_STRIP profile requires at least two vertices in every segment").into());
                }
                if a.ordinary_max >= count as f64 {
                    return Err(invalid("primitive index outside POSITION count").into());
                }
                a.count
            } else {
                count
            };
            if (mode == 1 && n % 2 != 0) || (mode == 3 && n < 2) || (mode == 4 && n % 3 != 0) {
                return Err(invalid("primitive element count incompatible with mode").into());
            }
            if let Some(material) = p.material {
                at(&materials, Some(material))?;
            }
        }
    }
    for (index, accessor) in accessors.iter().enumerate() {
        if accessor.component == 5125 && !index_accessors[index] {
            return Err(
                invalid("UNSIGNED_INT accessor must be referenced by primitive indices").into(),
            );
        }
    }
    let images = array::<records::Image<'_>>(root.images, slots)?;
    let image_count = images.len();
    for image in images {
        if image.uri.is_some() && image.view.is_some() {
            return Err(invalid("image has URI and bufferView").into());
        }
        if let Some(raw) = image.uri {
            let uri = string(Some(raw))?;
            if uri.starts_with("data:") {
                return Err(unsupported("image data URI").into());
            }
            let remaining = limits
                .member_bytes
                .checked_sub(resource_bytes)
                .ok_or_else(|| limit("resource byte allowance exhausted"))?;
            let data = resolve(&uri, remaining).map_err(PayloadError::Resolver)?;
            if data.len() > remaining {
                return Err(
                    limit("actual image resource exceeds remaining resource allowance").into(),
                );
            }
            resource_bytes = resource_bytes
                .checked_add(data.len())
                .ok_or_else(|| limit("image resource byte overflow"))?;
        } else {
            let view = at(&views, image.view)?;
            if accessor_views[view] {
                return Err(invalid("bufferView mixes image data and accessor data").into());
            }
            view_roles[view] = 3;
        }
        not_inspected.insert("image decoding".into());
    }
    let textures = array::<&RawValue>(root.textures, slots)?;
    let texture_count = textures.len();
    let samplers = array::<&RawValue>(root.samplers, slots)?;
    for texture in textures {
        let texture: records::Texture<'_> = uninspected_record(Some(texture))?;
        if let Some(source) = texture.source {
            reference_count(image_count, Some(source))?;
        }
        if let Some(sampler) = texture.sampler {
            at(&samplers, Some(sampler))?;
        }
    }
    drop(samplers);
    for material in materials {
        // Preserve the existing fixed five material texture-info roles; null is absent.
        let material: records::Material<'_> = uninspected_record(Some(material))?;
        let pbr: records::Pbr<'_> = uninspected_record(material.pbr)?;
        for info in [
            pbr.base,
            pbr.metallic,
            material.normal,
            material.occlusion,
            material.emissive,
        ] {
            if info.is_some() && !records::is_null(info) {
                let info: records::TextureInfo<'_> = record(info)?;
                reference_count(texture_count, info.index)?;
            }
        }
    }
    let extensions: records::Extensions<'_> = optional_record(root.extensions)?;
    let schema_uri = if let Some(metadata) = extensions.metadata {
        let metadata: records::Metadata<'_> = record(Some(metadata))?;
        metadata.uri.map(|uri| string(Some(uri))).transpose()?
    } else {
        None
    };
    not_inspected
        .insert("scene transforms, skins, animations, morph targets and rendered bounds".into());
    not_inspected.insert("metadata and feature semantics".into());
    Ok(PayloadInspection {
        schema_uri,
        accessors_checked: accessors.len() as u64,
        primitives_checked: primitives,
        vertices,
        elements_checked: elements as u64,
        not_inspected,
    })
}
fn reference_count(count: usize, raw: Raw<'_>) -> FormatResult<usize> {
    let value = domain(raw, count.saturating_sub(1) as u64)?;
    if count == 0 {
        return Err(invalid("reference out of range"));
    }
    usize::try_from(value).map_err(|_| invalid("reference outside host range"))
}

// Uninspected object shapes retain Value::get's absence behavior; consumed fields
// still apply their exact own type/range checks.
fn uninspected_record<'a, T: serde::Deserialize<'a> + Default>(raw: Raw<'a>) -> FormatResult<T> {
    if raw.is_some_and(|v| v.get().starts_with('{')) {
        record(raw)
    } else {
        Ok(T::default())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{json, Value};

    fn fixture_limits() -> PayloadLimits {
        PayloadLimits {
            member_bytes: 64 * 1024 * 1024,
            decoded_bytes: 64 * 1024 * 1024,
            accessor_components: 4_000_000,
            json: super::super::JsonLimits {
                bytes: 8 * 1024 * 1024,
                depth: 64,
                value_nodes: 65_536,
            },
        }
    }
    fn inspect(
        name: &str,
        bytes: &[u8],
        remaining: u64,
        mut resolve: impl FnMut(&str) -> FormatResult<Vec<u8>>,
    ) -> FormatResult<PayloadInspection> {
        let kind = if name.ends_with(".b3dm") {
            PayloadKind::B3dm
        } else if name.ends_with(".gltf") {
            PayloadKind::LocalGltf
        } else {
            PayloadKind::Glb
        };
        super::inspect(kind, bytes, fixture_limits(), remaining, |uri, _| {
            resolve(uri)
        })
        .map_err(|e| match e {
            PayloadError::Format(e) | PayloadError::Resolver(e) => e,
        })
    }
    fn minimum_version(asset: &Value) -> FormatResult<()> {
        let bytes = serde_json::to_vec(asset).unwrap();
        let admitted = json::admit(&bytes, fixture_limits().json)?;
        let asset: records::Asset<'_> = record(Some(admitted.raw()))?;
        super::minimum_version(asset.minimum)
    }
    fn frame(doc: &Value, bin: &[u8]) -> Vec<u8> {
        let mut json = serde_json::to_vec(doc).unwrap();
        while !json.len().is_multiple_of(4) {
            json.push(b' ')
        }
        let mut bin = bin.to_vec();
        while !bin.len().is_multiple_of(4) {
            bin.push(0)
        }
        let mut result = b"glTF".to_vec();
        result.extend(2u32.to_le_bytes());
        result.extend((28u32 + json.len() as u32 + bin.len() as u32).to_le_bytes());
        result.extend((json.len() as u32).to_le_bytes());
        result.extend(b"JSON");
        result.extend(json);
        result.extend((bin.len() as u32).to_le_bytes());
        result.extend(b"BIN\0");
        result.extend(bin);
        result
    }
    fn triangle() -> Value {
        json!({"asset":{"version":"2.0"},"buffers":[{"byteLength":36}],"bufferViews":[{"buffer":0,"byteLength":36}],"accessors":[{"bufferView":0,"componentType":5126,"type":"VEC3","count":3,"min":[0,0,0],"max":[0,0,0]}],"meshes":[{"primitives":[{"attributes":{"POSITION":0}}]}]})
    }
    fn b3dm(glb: &[u8]) -> Vec<u8> {
        let mut table = br#"{"BATCH_LENGTH":0}"#.to_vec();
        while !(28 + table.len()).is_multiple_of(8) {
            table.push(b' ');
        }
        let mut result = b"b3dm".to_vec();
        result.extend(1u32.to_le_bytes());
        result.extend(0u32.to_le_bytes());
        result.extend((table.len() as u32).to_le_bytes());
        result.extend([0u8; 12]);
        result.extend(table);
        result.extend(glb);
        while !result.len().is_multiple_of(8) {
            result.push(0);
        }
        let length = result.len() as u32;
        result[8..12].copy_from_slice(&length.to_le_bytes());
        result
    }
    #[test]
    fn modern_b3dm_checks_tables_and_embedded_length() {
        let good = b3dm(&frame(&triangle(), &[0; 36]));
        assert_eq!(
            inspect("a.b3dm", &good, 4_000_000, |_| panic!())
                .unwrap()
                .vertices,
            3
        );
        for offset in [8, 12, 16, 20, 24] {
            let mut bad = good.clone();
            bad[offset..offset + 4].copy_from_slice(&u32::MAX.to_le_bytes());
            assert!(matches!(
                super::unwrap_b3dm(&bad, fixture_limits().json),
                Err(FormatError::InvalidInput(_))
            ));
        }
        assert!(matches!(
            super::unwrap_b3dm(b"b3dm", fixture_limits().json),
            Err(FormatError::InvalidInput(_))
        ));
        let mut bad = good.clone();
        let start = 28 + u32::from_le_bytes(bad[12..16].try_into().unwrap()) as usize;
        bad[start + 8..start + 12].copy_from_slice(&u32::MAX.to_le_bytes());
        assert!(matches!(
            super::unwrap_b3dm(&bad, fixture_limits().json),
            Err(FormatError::InvalidInput(_))
        ));
        let mut bad = good.clone();
        bad[30] = b'X';
        assert!(matches!(
            super::unwrap_b3dm(&bad, fixture_limits().json),
            Err(FormatError::InvalidInput(_))
        ));
    }
    #[test]
    fn matrix_padding_and_actual_ranges_are_checked() {
        let mut doc = triangle();
        doc["buffers"][0]["byteLength"] = json!(48);
        doc["bufferViews"]
            .as_array_mut()
            .unwrap()
            .push(json!({"buffer":0,"byteOffset":36,"byteLength":12}));
        doc["accessors"]
            .as_array_mut()
            .unwrap()
            .push(json!({"bufferView":1,"componentType":5121,"type":"MAT3","count":1}));
        let good = frame(&doc, &[0; 48]);
        let facts = inspect("a.glb", &good, 4_000_000, |_| panic!("self-contained")).unwrap();
        assert_eq!(facts.accessors_checked, 2);
        assert_eq!(facts.elements_checked, 18);
        doc["bufferViews"][1]["byteLength"] = json!(9);
        assert!(matches!(
            inspect("a.glb", &frame(&doc, &[0; 48]), 4_000_000, |_| panic!()),
            Err(FormatError::InvalidInput(_))
        ));
    }
    #[test]
    fn remaining_budget_is_checked_before_external_reads_or_decoding() {
        let mut doc = triangle();
        doc["buffers"][0]["uri"] = json!("external.bin");
        let bytes = serde_json::to_vec(&doc).unwrap();
        assert!(matches!(
            inspect("a.gltf", &bytes, 8, |_| panic!(
                "budget must fail before resource callback"
            )),
            Err(FormatError::ResourceLimit(_))
        ));
        doc["accessors"][0]["count"] = json!(u64::MAX);
        assert!(matches!(
            inspect(
                "a.gltf",
                &serde_json::to_vec(&doc).unwrap(),
                4_000_000,
                |_| panic!()
            ),
            Err(FormatError::ResourceLimit(_))
        ));
    }
    #[test]
    fn sparse_is_unsupported_before_resource_reads() {
        let mut doc = triangle();
        doc["buffers"][0]["uri"] = json!("external.bin");
        doc["accessors"][0]["sparse"] = json!({});
        assert!(matches!(
            inspect(
                "a.gltf",
                &serde_json::to_vec(&doc).unwrap(),
                4_000_000,
                |_| panic!("unsupported admission before reads")
            ),
            Err(FormatError::Unsupported(_))
        ));
    }
    #[test]
    fn final_matrix_padding_may_be_omitted_in_published_profile() {
        let doc = json!({"asset":{"version":"2.0"},"buffers":[{"byteLength":11}],"bufferViews":[{"buffer":0,"byteLength":11}],"accessors":[{"bufferView":0,"componentType":5121,"type":"MAT3","count":1}]});
        let facts = inspect("a.glb", &frame(&doc, &[0; 11]), 4_000_000, |_| panic!()).unwrap();
        assert_eq!(facts.elements_checked, 9);
    }
    #[test]
    fn mixed_view_roles_and_invalid_attribute_sets_are_rejected() {
        let mut doc = triangle();
        doc["buffers"][0]["byteLength"] = json!(40);
        doc["bufferViews"][0]["byteLength"] = json!(40);
        doc["accessors"][0]["byteOffset"] = json!(4);
        doc["accessors"]
            .as_array_mut()
            .unwrap()
            .push(json!({"bufferView":0,"componentType":5121,"type":"SCALAR","count":3}));
        doc["meshes"][0]["primitives"][0]["indices"] = json!(1);
        assert!(matches!(
            inspect("a.glb", &frame(&doc, &[0; 40]), 4_000_000, |_| panic!()),
            Err(FormatError::InvalidInput(_))
        ));
        assert!(!numeric_set("COLOR_foo", "COLOR_"));
        assert!(!numeric_set("TEXCOORD_", "TEXCOORD_"));
        assert!(numeric_set("COLOR_12", "COLOR_"));
    }
    #[test]
    fn unsigned_int_accessor_requires_index_ownership() {
        let mut doc = triangle();
        doc["accessors"]
            .as_array_mut()
            .unwrap()
            .push(json!({"componentType":5125,"type":"SCALAR","count":3}));
        assert!(matches!(
            inspect("a.glb", &frame(&doc, &[0; 36]), 4_000_000, |_| panic!()),
            Err(FormatError::InvalidInput(_))
        ));
        doc["meshes"][0]["primitives"][0]["indices"] = json!(1);
        let facts = inspect("a.glb", &frame(&doc, &[0; 36]), 4_000_000, |_| panic!()).unwrap();
        assert_eq!(facts.primitives_checked, 1);
    }
    #[test]
    fn pinned_line_strip_restart_checks_actual_segments() {
        let mut doc = triangle();
        doc["extensionsUsed"] = json!(["KHR_mesh_primitive_restart"]);
        doc["extensionsRequired"] = doc["extensionsUsed"].clone();
        doc["buffers"][0]["byteLength"] = json!(41);
        doc["bufferViews"]
            .as_array_mut()
            .unwrap()
            .push(json!({"buffer":0,"byteOffset":36,"byteLength":5}));
        doc["accessors"]
            .as_array_mut()
            .unwrap()
            .push(json!({"bufferView":1,"componentType":5121,"type":"SCALAR","count":5}));
        doc["meshes"][0]["primitives"][0]["indices"] = json!(1);
        doc["meshes"][0]["primitives"][0]["mode"] = json!(3);
        let mut bin = vec![0; 36];
        bin.extend([0, 1, 255, 1, 2]);
        assert!(inspect("a.glb", &frame(&doc, &bin), 4_000_000, |_| panic!()).is_ok());
        doc["meshes"][0]["primitives"][0]["mode"] = json!(0);
        assert!(matches!(
            inspect("a.glb", &frame(&doc, &bin), 4_000_000, |_| panic!()),
            Err(FormatError::InvalidInput(_))
        ));
        doc["meshes"][0]["primitives"][0]["mode"] = json!(3);
        bin[36] = 255;
        assert!(matches!(
            inspect("a.glb", &frame(&doc, &bin), 4_000_000, |_| panic!()),
            Err(FormatError::Unsupported(_))
        ));
    }
    #[test]
    fn bounded_base64_buffers_are_checked_without_resource_callback() {
        let mut doc = triangle();
        doc["buffers"][0]["uri"] = json!(format!(
            "data:application/octet-stream;base64,{}",
            "A".repeat(48)
        ));
        assert!(inspect(
            "a.gltf",
            &serde_json::to_vec(&doc).unwrap(),
            4_000_000,
            |_| panic!()
        )
        .is_ok());
        doc["buffers"][0]["uri"] = json!("data:application/gltf-buffer;base64,???");
        assert!(matches!(
            inspect(
                "a.gltf",
                &serde_json::to_vec(&doc).unwrap(),
                4_000_000,
                |_| panic!()
            ),
            Err(FormatError::InvalidInput(_))
        ));
        assert!(matches!(
            decode_buffer_uri("data:text/plain;base64,AAAA", 64, 64),
            Err(FormatError::Unsupported(_))
        ));
    }
    #[test]
    fn minimum_version_gate_is_bounded_and_typed() {
        for version in ["2.0", "1.999999999999999999999999", "0002.000"] {
            assert!(minimum_version(&json!({"minVersion":version})).is_ok());
        }
        for version in [
            "2.1",
            "3.0",
            "2.999999999999999999999999",
            "99999999999999999999.0",
        ] {
            assert!(matches!(
                minimum_version(&json!({"minVersion":version})),
                Err(FormatError::Unsupported(_))
            ));
        }
        for version in [
            json!(2),
            json!("2"),
            json!("2."),
            json!("2.0.0"),
            json!("-2.0"),
        ] {
            assert!(matches!(
                minimum_version(&json!({"minVersion":version})),
                Err(FormatError::InvalidInput(_))
            ));
        }
    }
    #[test]
    fn extras_are_opaque_and_external_buffers_are_declared_length_bounded() {
        let mut doc = triangle();
        doc["extras"] = json!({"extensions":42,"nested":{"extensions":{"unknown":true}}});
        assert!(inspect("a.glb", &frame(&doc, &[0; 36]), 4_000_000, |_| panic!()).is_ok());
        doc["buffers"][0]["uri"] = json!("a.bin");
        let facts = inspect(
            "a.gltf",
            &serde_json::to_vec(&doc).unwrap(),
            4_000_000,
            |_| Ok(vec![0; 40]),
        )
        .unwrap();
        assert_eq!(facts.vertices, 3);
    }
    #[test]
    fn independent_meshopt_placeholder_and_declared_range_controls() {
        use std::io::{Cursor, Read};
        let mut archive = zip::ZipArchive::new(Cursor::new(include_bytes!(
            "../../tests/fixtures/c1/meshopt_none_golden.3tz"
        )))
        .unwrap();
        let name = (0..archive.len())
            .map(|i| archive.by_index(i).unwrap().name().to_owned())
            .find(|name| name.ends_with(".glb"))
            .unwrap();
        let mut original = Vec::new();
        archive
            .by_name(&name)
            .unwrap()
            .read_to_end(&mut original)
            .unwrap();
        let (json, bin) = envelope(&original).unwrap();
        let mut doc = serde_json::from_slice::<Value>(json).unwrap();
        let compressed = bin.unwrap().to_vec();
        doc["buffers"] = json!([{"byteLength":48},{"byteLength":85,"uri":"a.bin"}]);
        doc["bufferViews"][0]["buffer"] = json!(0);
        doc["bufferViews"][0]["extensions"]["EXT_meshopt_compression"]["buffer"] = json!(1);
        assert!(inspect(
            "a.gltf",
            &serde_json::to_vec(&doc).unwrap(),
            4_000_000,
            |_| Ok(compressed.clone())
        )
        .is_ok());
        let mut json_only_glb = frame(&doc, &[]);
        json_only_glb.truncate(json_only_glb.len() - 8);
        let length = json_only_glb.len() as u32;
        json_only_glb[8..12].copy_from_slice(&length.to_le_bytes());
        assert!(inspect("a.glb", &json_only_glb, 4_000_000, |_| Ok(
            compressed.clone()
        ))
        .is_ok());
        let mut missing_required = doc.clone();
        missing_required
            .as_object_mut()
            .unwrap()
            .remove("extensionsRequired");
        assert!(matches!(
            inspect(
                "a.gltf",
                &serde_json::to_vec(&missing_required).unwrap(),
                4_000_000,
                |_| Ok(compressed.clone())
            ),
            Err(FormatError::InvalidInput(_))
        ));
        let mut uncovered = doc.clone();
        uncovered["bufferViews"]
            .as_array_mut()
            .unwrap()
            .push(json!({"buffer":0,"byteLength":4}));
        assert!(matches!(
            inspect(
                "a.gltf",
                &serde_json::to_vec(&uncovered).unwrap(),
                4_000_000,
                |_| Ok(compressed.clone())
            ),
            Err(FormatError::InvalidInput(_))
        ));
        doc["buffers"][1]["byteLength"] = json!(84);
        assert!(matches!(
            inspect(
                "a.gltf",
                &serde_json::to_vec(&doc).unwrap(),
                4_000_000,
                |_| Ok(compressed.clone())
            ),
            Err(FormatError::InvalidInput(_))
        ));
    }
    #[test]
    fn quantized_uv_required_only_when_core_type_is_insufficient() {
        let mut doc = triangle();
        doc["extensionsUsed"] = json!(["KHR_mesh_quantization"]);
        doc["buffers"][0]["byteLength"] = json!(60);
        doc["bufferViews"]
            .as_array_mut()
            .unwrap()
            .push(json!({"buffer":0,"byteOffset":36,"byteLength":24,"byteStride":8}));
        doc["accessors"].as_array_mut().unwrap().push(
            json!({"bufferView":1,"componentType":5123,"type":"VEC2","count":3,"normalized":true}),
        );
        doc["meshes"][0]["primitives"][0]["attributes"]["TEXCOORD_0"] = json!(1);
        for component in [5121, 5123] {
            doc["accessors"][1]["componentType"] = json!(component);
            assert!(inspect("a.glb", &frame(&doc, &[0; 60]), 4_000_000, |_| panic!()).is_ok());
        }
        for (component, normalized) in [(5122, true), (5123, false)] {
            doc["accessors"][1]["componentType"] = json!(component);
            doc["accessors"][1]["normalized"] = json!(normalized);
            assert!(matches!(
                inspect("a.glb", &frame(&doc, &[0; 60]), 4_000_000, |_| panic!()),
                Err(FormatError::InvalidInput(_))
            ));
            doc["extensionsRequired"] = json!(["KHR_mesh_quantization"]);
            assert!(inspect("a.glb", &frame(&doc, &[0; 60]), 4_000_000, |_| panic!()).is_ok());
            doc.as_object_mut().unwrap().remove("extensionsRequired");
        }
    }
    #[test]
    fn missing_position_is_a_failure_instead_of_a_panic() {
        let mut doc = triangle();
        doc["meshes"][0]["primitives"][0]["attributes"] = json!({});
        assert!(matches!(
            inspect("a.glb", &frame(&doc, &[0; 36]), 4_000_000, |_| panic!()),
            Err(FormatError::InvalidInput(_))
        ));
    }
    #[test]
    fn independent_meshopt_golden_stream_decodes() {
        use std::io::{Cursor, Read};
        let mut archive = zip::ZipArchive::new(Cursor::new(include_bytes!(
            "../../tests/fixtures/c1/meshopt_none_golden.3tz"
        )))
        .unwrap();
        let name = (0..archive.len())
            .map(|i| archive.by_index(i).unwrap().name().to_owned())
            .find(|name| name.ends_with(".glb"))
            .unwrap();
        let mut bytes = Vec::new();
        archive
            .by_name(&name)
            .unwrap()
            .read_to_end(&mut bytes)
            .unwrap();
        let facts = inspect(&name, &bytes, 4_000_000, |_| {
            panic!("embedded golden buffer")
        })
        .unwrap();
        assert_eq!(facts.vertices, 4);
        assert_eq!(facts.accessors_checked, 1);
        assert_eq!(facts.primitives_checked, 1);
    }
}

#[cfg(test)]
mod controls;
