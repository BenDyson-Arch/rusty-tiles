//! Bounded payload inspection. No filesystem access or converter/job ownership.
use super::ValidationFailure;
use serde_json::Value;
use std::{borrow::Cow, collections::BTreeSet};
type Result<T> = std::result::Result<T, ValidationFailure>;
fn caps() -> super::ValidationLimits {
    super::ValidationLimits::default()
}
pub(super) struct PayloadInspection {
    pub document: Value,
    pub accessors_checked: u64,
    pub primitives_checked: u64,
    pub vertices: u64,
    pub elements_checked: u64,
    pub not_inspected: BTreeSet<String>,
}
fn invalid(s: impl Into<String>) -> ValidationFailure {
    ValidationFailure::InvalidInput(s.into())
}
fn unsupported(s: impl Into<String>) -> ValidationFailure {
    ValidationFailure::Unsupported(s.into())
}
fn limit(s: impl Into<String>) -> ValidationFailure {
    ValidationFailure::ResourceLimit(s.into())
}
fn uint(v: &Value) -> Result<usize> {
    v.as_u64()
        .and_then(|n| usize::try_from(n).ok())
        .ok_or_else(|| invalid("expected host-range unsigned integer"))
}
fn field(v: &Value, k: &str) -> Result<usize> {
    uint(&v[k])
}
fn off(v: &Value, k: &str) -> Result<usize> {
    v.get(k).map_or(Ok(0), uint)
}
fn list<'a>(v: &'a Value, k: &str) -> Result<&'a [Value]> {
    v.get(k).map_or(Ok(&[]), |a| {
        a.as_array()
            .map(Vec::as_slice)
            .ok_or_else(|| invalid(format!("{k} must be an array")))
    })
}
fn at<T>(a: &[T], v: &Value) -> Result<usize> {
    let i = uint(v)?;
    if i >= a.len() {
        return Err(invalid("reference out of range"));
    }
    Ok(i)
}
fn range(data: &[u8], offset: usize, length: usize) -> Result<&[u8]> {
    let end = offset
        .checked_add(length)
        .ok_or_else(|| invalid("byte range overflow"))?;
    data.get(offset..end)
        .ok_or_else(|| invalid("byte range outside actual resource"))
}
fn word(data: &[u8], offset: usize) -> Result<usize> {
    Ok(u32::from_le_bytes(range(data, offset, 4)?.try_into().unwrap()) as usize)
}
fn envelope(bytes: &[u8]) -> Result<(&[u8], Option<&[u8]>)> {
    if bytes.len() > caps().member_bytes as usize {
        return Err(limit("payload exceeds 64 MiB"));
    }
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
fn decode_buffer_uri(uri: &str) -> Result<Vec<u8>> {
    use base64::Engine;
    let encoded = uri
        .strip_prefix("data:application/octet-stream;base64,")
        .or_else(|| uri.strip_prefix("data:application/gltf-buffer;base64,"))
        .ok_or_else(|| unsupported("data buffer media type or encoding"))?;
    let maximum = encoded
        .len()
        .checked_add(3)
        .and_then(|n| (n / 4).checked_mul(3))
        .ok_or_else(|| limit("data buffer decoded size overflow"))?;
    if maximum > caps().document_decoded_bytes as usize {
        return Err(limit("data buffer exceeds decoded byte limit"));
    }
    base64::engine::general_purpose::STANDARD
        .decode(encoded)
        .map_err(|error| invalid(format!("invalid base64 buffer: {error}")))
}
fn decode_meshopt(e: &Value, buffers: &[Cow<'_, [u8]>], budget: &mut usize) -> Result<Vec<u8>> {
    let b = at(buffers, &e["buffer"])?;
    let compressed = range(&buffers[b], off(e, "byteOffset")?, field(e, "byteLength")?)?;
    let count = field(e, "count")?;
    let stride = field(e, "byteStride")?;
    if count == 0 {
        return Err(invalid("empty meshopt stream"));
    }
    let length = count
        .checked_mul(stride)
        .ok_or_else(|| limit("meshopt decoded size overflow"))?;
    *budget = budget
        .checked_add(length)
        .ok_or_else(|| limit("decoded view budget overflow"))?;
    if *budget > caps().document_decoded_bytes as usize {
        return Err(limit("decoded views exceed 64 MiB"));
    }
    let filter = e.get("filter").map_or(Ok("NONE"), |v| {
        v.as_str()
            .ok_or_else(|| invalid("meshopt filter must be string"))
    })?;
    if filter != "NONE" {
        return Err(unsupported("meshopt filters other than NONE"));
    }
    let mode = e["mode"]
        .as_str()
        .ok_or_else(|| invalid("meshopt mode missing"))?;
    match mode {
        "ATTRIBUTES" if stride > 0 && stride <= 256 && stride % 4 == 0 => {}
        "TRIANGLES" if [2, 4].contains(&stride) && count % 3 == 0 => {}
        "INDICES" if [2, 4].contains(&stride) => {}
        "ATTRIBUTES" | "TRIANGLES" | "INDICES" => {
            return Err(invalid("invalid meshopt stride/count"))
        }
        _ => return Err(invalid("invalid meshopt mode")),
    }
    // u32 backing guarantees alignment for the native index decoder. Every byte
    // passed to the codec is in a checked input/output allocation; no filters run.
    let mut decoded = vec![0u32; length.div_ceil(4)];
    let code = unsafe {
        match mode {
            "ATTRIBUTES" => meshopt::ffi::meshopt_decodeVertexBuffer(
                decoded.as_mut_ptr().cast(),
                count,
                stride,
                compressed.as_ptr(),
                compressed.len(),
            ),
            "TRIANGLES" => meshopt::ffi::meshopt_decodeIndexBuffer(
                decoded.as_mut_ptr().cast(),
                count,
                stride,
                compressed.as_ptr(),
                compressed.len(),
            ),
            _ => meshopt::ffi::meshopt_decodeIndexSequence(
                decoded.as_mut_ptr().cast(),
                count,
                stride,
                compressed.as_ptr(),
                compressed.len(),
            ),
        }
    };
    if code != 0 {
        return Err(invalid("invalid meshopt compressed stream"));
    }
    let mut out = Vec::with_capacity(length);
    for word in decoded {
        out.extend(word.to_ne_bytes())
    }
    out.truncate(length);
    Ok(out)
}
// Modern b3dm v1 only: 3D Tiles Batched3DModel header/padding contract.
fn unwrap_b3dm(bytes: &[u8]) -> Result<&[u8]> {
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
            let document = super::json::parse(table)?;
            if !document.is_object() {
                return Err(invalid("b3dm table JSON must be object"));
            }
            if index == 0
                && document
                    .get("BATCH_LENGTH")
                    .and_then(Value::as_u64)
                    .is_none_or(|n| n > u32::MAX as u64)
            {
                return Err(invalid("b3dm requires uint32 BATCH_LENGTH"));
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
    if bytes.len() - end > 7 || bytes[end..].iter().any(|byte| *byte != 0) {
        return Err(invalid("invalid b3dm trailing padding"));
    }
    Ok(glb)
}
pub(super) fn inspect(
    name: &str,
    bytes: &[u8],
    max_elements: u64,
    mut resolve: impl FnMut(&str) -> Result<Vec<u8>>,
) -> Result<PayloadInspection> {
    if bytes.len() > caps().member_bytes as usize {
        return Err(limit("payload exceeds 64 MiB"));
    }
    let is_b3dm = name.to_ascii_lowercase().ends_with(".b3dm");
    let (json, bin) = if is_b3dm {
        envelope(unwrap_b3dm(bytes)?)?
    } else if name.to_ascii_lowercase().ends_with(".glb") {
        envelope(bytes)?
    } else if name.to_ascii_lowercase().ends_with(".gltf") {
        (bytes, None)
    } else {
        return Err(unsupported("payload format outside GLB/local glTF 2.0"));
    };
    let doc = super::json::parse(json)?;
    if !doc.is_object() || doc["asset"]["version"] != "2.0" {
        return Err(invalid("glTF asset.version must be 2.0"));
    }
    let used: BTreeSet<_> = list(&doc, "extensionsUsed")?
        .iter()
        .map(|v| {
            v.as_str()
                .ok_or_else(|| invalid("extension name must be string"))
        })
        .collect::<Result<_>>()?;
    let supported = [
        "KHR_mesh_quantization",
        "EXT_meshopt_compression",
        "KHR_materials_unlit",
        "KHR_mesh_primitive_restart",
    ];
    for ext in list(&doc, "extensionsRequired")? {
        let ext = ext
            .as_str()
            .ok_or_else(|| invalid("required extension must be string"))?;
        if !used.contains(ext) {
            return Err(invalid("required extension absent from extensionsUsed"));
        }
        if !supported.contains(&ext) {
            return Err(unsupported(format!("required extension {ext}")));
        }
    }
    let restart = used.contains("KHR_mesh_primitive_restart");
    if restart
        && !list(&doc, "extensionsRequired")?
            .iter()
            .any(|value| value == "KHR_mesh_primitive_restart")
    {
        return Err(invalid("primitive restart extension must be required"));
    }
    let element_cap = max_elements.min(caps().accessor_elements);
    let mut planned_elements = 0u64;
    for a in list(&doc, "accessors")? {
        if a.get("sparse").is_some() {
            return Err(unsupported("sparse accessor"));
        }
        let count = a["count"]
            .as_u64()
            .ok_or_else(|| invalid("accessor count must be unsigned integer"))?;
        let components = match a["type"].as_str() {
            Some("SCALAR") => 1,
            Some("VEC2") => 2,
            Some("VEC3") => 3,
            Some("VEC4" | "MAT2") => 4,
            Some("MAT3") => 9,
            Some("MAT4") => 16,
            _ => return Err(invalid("invalid accessor shape")),
        };
        planned_elements = planned_elements
            .checked_add(
                count
                    .checked_mul(components)
                    .ok_or_else(|| limit("accessor element overflow"))?,
            )
            .ok_or_else(|| limit("accessor aggregate overflow"))?;
        if planned_elements > element_cap {
            return Err(limit(
                "accessor elements exceed remaining document/archive budget",
            ));
        }
    }
    let mut not_inspected = used
        .iter()
        .filter(|e| !supported.contains(e))
        .map(|s| s.to_string())
        .collect::<BTreeSet<_>>();
    if is_b3dm {
        not_inspected.insert("b3dm feature and batch table semantics".into());
    }
    let mut nodes = vec![&doc];
    while let Some(value) = nodes.pop() {
        match value {
            Value::Array(a) => nodes.extend(a),
            Value::Object(o) => {
                if let Some(extensions) = o.get("extensions") {
                    let extensions = extensions
                        .as_object()
                        .ok_or_else(|| invalid("extensions must be an object"))?;
                    for (name, value) in extensions {
                        if !used.contains(name.as_str()) {
                            return Err(invalid(format!("undeclared extension {name}")));
                        }
                        if !value.is_object() {
                            return Err(invalid("extension payload must be object"));
                        }
                    }
                }
                nodes.extend(o.values());
            }
            _ => {}
        }
    }
    let mut buffers = Vec::new();
    let mut buffer_budget = 0usize;
    let mut resource_bytes = 0usize;
    for (i, b) in list(&doc, "buffers")?.iter().enumerate() {
        let n = field(b, "byteLength")?;
        if n == 0 {
            return Err(invalid("empty buffer"));
        }
        buffer_budget = buffer_budget
            .checked_add(n)
            .ok_or_else(|| limit("buffer budget overflow"))?;
        if buffer_budget > caps().member_bytes as usize {
            return Err(limit("buffer lengths exceed 64 MiB"));
        }
        let fallback =
            b["extensions"]["EXT_meshopt_compression"]["fallback"].as_bool() == Some(true);
        let data = if let Some(uri) = b.get("uri") {
            let uri = uri
                .as_str()
                .ok_or_else(|| invalid("buffer URI must be string"))?;
            if uri.starts_with("data:") {
                Cow::Owned(decode_buffer_uri(uri)?)
            } else {
                Cow::Owned(resolve(uri)?)
            }
        } else if i == 0 {
            Cow::Borrowed(bin.ok_or_else(|| invalid("missing embedded BIN buffer"))?)
        } else if fallback && used.contains("EXT_meshopt_compression") {
            buffers.push(Cow::Borrowed(&[][..]));
            continue;
        } else {
            return Err(invalid("buffer has no resource"));
        };
        resource_bytes = resource_bytes
            .checked_add(data.len())
            .ok_or_else(|| limit("actual buffer bytes overflow"))?;
        if resource_bytes > caps().member_bytes as usize {
            return Err(limit("actual buffer resources exceed 64 MiB"));
        }
        if data.len() < n
            || data.len() > caps().member_bytes as usize
            || (b.get("uri").is_none() && (data.len() - n > 3 || data[n..].iter().any(|&v| v != 0)))
        {
            return Err(invalid("buffer byteLength does not match actual resource"));
        }
        buffers.push(data);
    }
    let view_docs = list(&doc, "bufferViews")?;
    let mut views = Vec::new();
    let mut view_budget = 0usize;
    for v in view_docs {
        let n = field(v, "byteLength")?;
        if n == 0 {
            return Err(invalid("empty bufferView"));
        }
        let b = at(&buffers, &v["buffer"])?;
        if off(v, "byteOffset")?
            .checked_add(n)
            .is_none_or(|end| end > field(&doc["buffers"][b], "byteLength").unwrap_or(0))
        {
            return Err(invalid("bufferView exceeds declared buffer byteLength"));
        }
        if let Some(stride) = v.get("byteStride") {
            let s = uint(stride)?;
            if !(4..=252).contains(&s) || s % 4 != 0 {
                return Err(invalid("invalid bufferView byteStride"));
            }
        }
        if let Some(target) = v.get("target") {
            if ![34962, 34963].contains(&uint(target)?) {
                return Err(invalid("invalid bufferView target"));
            }
        }
        if let Some(e) = v["extensions"].get("EXT_meshopt_compression") {
            if !used.contains("EXT_meshopt_compression") {
                return Err(invalid("meshopt extension not declared"));
            }
            let decoded = decode_meshopt(e, &buffers, &mut view_budget)?;
            if decoded.len() != n {
                return Err(invalid("meshopt decoded size differs from bufferView"));
            }
            let end = off(v, "byteOffset")?
                .checked_add(n)
                .ok_or_else(|| invalid("fallback view range overflow"))?;
            if end > field(&doc["buffers"][b], "byteLength")? {
                return Err(invalid("fallback view outside declared buffer"));
            }
            if let Some(s) = v.get("byteStride") {
                if uint(s)? != field(e, "byteStride")? {
                    return Err(invalid("meshopt and bufferView stride disagree"));
                }
            }
            views.push(Cow::Owned(decoded));
        } else {
            view_budget = view_budget
                .checked_add(n)
                .ok_or_else(|| limit("view budget overflow"))?;
            if view_budget > caps().document_decoded_bytes as usize {
                return Err(limit("views exceed 64 MiB"));
            }
            views.push(Cow::Borrowed(range(&buffers[b], off(v, "byteOffset")?, n)?));
        }
    }
    let mut accessors = Vec::new();
    let mut elements = 0usize;
    for a in list(&doc, "accessors")? {
        if a.get("sparse").is_some() {
            return Err(unsupported("sparse accessor"));
        }
        let count = field(a, "count")?;
        if count == 0 {
            return Err(invalid("empty accessor"));
        }
        let component = field(a, "componentType")?;
        let size = match component {
            5120 | 5121 => 1,
            5122 | 5123 => 2,
            5125 | 5126 => 4,
            _ => return Err(invalid("invalid componentType")),
        };
        let shape = a["type"]
            .as_str()
            .ok_or_else(|| invalid("accessor type missing"))?;
        let (rows, cols) = match shape {
            "SCALAR" => (1, 1),
            "VEC2" => (2, 1),
            "VEC3" => (3, 1),
            "VEC4" => (4, 1),
            "MAT2" => (2, 2),
            "MAT3" => (3, 3),
            "MAT4" => (4, 4),
            _ => return Err(invalid("invalid accessor shape")),
        };
        let normalized = a.get("normalized").map_or(Ok(false), |v| {
            v.as_bool()
                .ok_or_else(|| invalid("normalized must be boolean"))
        })?;
        if normalized && [5125, 5126].contains(&component) {
            return Err(invalid("invalid normalized component type"));
        }
        elements = elements
            .checked_add(
                count
                    .checked_mul(rows * cols)
                    .ok_or_else(|| limit("accessor element overflow"))?,
            )
            .ok_or_else(|| limit("accessor aggregate overflow"))?;
        if elements as u64 > element_cap {
            return Err(limit("document exceeds four million accessor components"));
        }
        let column = if cols > 1 {
            (rows * size).next_multiple_of(4)
        } else {
            rows * size
        };
        let element = column * cols;
        // Published glTF 2.0.1 permits omitting padding after the last matrix column.
        let occupied = column * (cols - 1) + rows * size;
        let offsets = (0..cols)
            .flat_map(|c| (0..rows).map(move |r| c * column + r * size))
            .collect();
        let view = a.get("bufferView").map(|v| at(&views, v)).transpose()?;
        let offset = off(a, "byteOffset")?;
        let stride = view.map_or(element, |v| {
            view_docs[v]
                .get("byteStride")
                .and_then(Value::as_u64)
                .map_or(element, |s| s as usize)
        });
        if offset % size != 0 || stride < element || (cols > 1 && size < 4 && offset % 4 != 0) {
            return Err(invalid("accessor alignment/stride invalid"));
        }
        if let Some(v) = view {
            let absolute = off(&view_docs[v], "byteOffset")?
                .checked_add(offset)
                .ok_or_else(|| invalid("accessor absolute offset overflow"))?;
            if absolute % size != 0 || (cols > 1 && size < 4 && absolute % 4 != 0) {
                return Err(invalid("accessor absolute alignment invalid"));
            }
            let n = (count - 1)
                .checked_mul(stride)
                .and_then(|n| n.checked_add(occupied))
                .ok_or_else(|| invalid("accessor range overflow"))?;
            range(&views[v], offset, n)?;
        } else if a.get("byteOffset").is_some() {
            return Err(invalid("accessor without view must omit byteOffset"));
        }
        if let Some(v) = view {
            if let Some(e) = view_docs[v]["extensions"].get("EXT_meshopt_compression") {
                if field(e, "byteStride")? != stride {
                    return Err(invalid(
                        "accessor stride differs from meshopt decoded stride",
                    ));
                }
            }
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
            shape: shape.into(),
            min: Vec::new(),
            max: Vec::new(),
            ordinary_max: f64::NEG_INFINITY,
            shortest_segment: usize::MAX,
        };
        let mut min = vec![f64::INFINITY; parsed.components];
        let mut max = vec![f64::NEG_INFINITY; parsed.components];
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
                    return Err(invalid("nonfinite accessor float"));
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
        for (key, actual) in [("min", &min), ("max", &max)] {
            if let Some(bound) = a.get(key) {
                let declared = bound
                    .as_array()
                    .ok_or_else(|| invalid("accessor bounds must be array"))?;
                if declared.len() != parsed.components {
                    return Err(invalid("accessor bounds shape mismatch"));
                }
                for (v, actual) in declared.iter().zip(actual) {
                    let n = v
                        .as_f64()
                        .filter(|n| n.is_finite())
                        .ok_or_else(|| invalid("accessor bound not finite number"))?;
                    let n = if component == 5126 {
                        f64::from(n as f32)
                    } else {
                        n
                    };
                    if n != *actual {
                        return Err(invalid(
                            "declared accessor bounds differ from decoded values",
                        ));
                    }
                }
            }
        }
        parsed.min = min;
        parsed.max = max;
        accessors.push(parsed);
    }
    let mut view_roles = vec![0u8; views.len()];
    let mut attribute_accessors = vec![None; views.len()];
    let mut accessor_views = vec![false; views.len()];
    for accessor in &accessors {
        if let Some(view) = accessor.view {
            accessor_views[view] = true;
        }
    }
    let mut index_accessors = vec![false; accessors.len()];
    let mut vertices = 0u64;
    let mut primitives = 0u64;
    for mesh in list(&doc, "meshes")? {
        for p in list(mesh, "primitives")? {
            primitives += 1;
            let mode = p.get("mode").map_or(Ok(4), uint)?;
            if ![0, 1, 3, 4].contains(&mode) {
                return Err(unsupported(format!("primitive mode {mode}")));
            }
            if p["extensions"].get("KHR_draco_mesh_compression").is_some() {
                return Err(unsupported("Draco primitive"));
            }
            if let Some(polygon) = p["extensions"].get("EXT_mesh_polygon") {
                if mode != 4 || p.get("indices").is_none() {
                    return Err(invalid("polygon extension requires indexed TRIANGLES"));
                }
                let polygons = field(polygon, "count")?;
                if polygons == 0 {
                    return Err(invalid("polygon count must be positive"));
                }
                for key in ["indicesOffsets", "loopIndices", "loopIndicesOffsets"] {
                    let index = at(&accessors, &polygon[key])?;
                    let accessor = &accessors[index];
                    if accessor.shape != "SCALAR"
                        || ![5121, 5123, 5125].contains(&accessor.component)
                        || accessor.normalized
                    {
                        return Err(invalid(
                            "polygon reference must be unsigned SCALAR accessor",
                        ));
                    }
                    if key != "loopIndices" && accessor.count != polygons {
                        return Err(invalid("polygon offset accessor count differs"));
                    }
                    if let Some(view) = accessor.view {
                        if view_roles[view] == 1
                            || view_docs[view].get("byteStride").is_some()
                            || view_docs[view]
                                .get("target")
                                .is_some_and(|target| target != 34963)
                        {
                            return Err(invalid("polygon accessor view mixes vertex attributes or has invalid stride/target"));
                        }
                        view_roles[view] = 2;
                    }
                    index_accessors[index] = true;
                }
            }
            let attrs = p["attributes"]
                .as_object()
                .ok_or_else(|| invalid("primitive attributes missing"))?;
            let position = at(
                &accessors,
                attrs
                    .get("POSITION")
                    .ok_or_else(|| invalid("primitive POSITION missing"))?,
            )?;
            for prefix in ["COLOR_", "TEXCOORD_"] {
                let mut sets = Vec::new();
                for semantic in attrs.keys().filter(|name| name.starts_with(prefix)) {
                    if !numeric_set(semantic, prefix) {
                        return Err(invalid("invalid indexed attribute suffix"));
                    }
                    let set = semantic[prefix.len()..]
                        .parse::<usize>()
                        .map_err(|_| invalid("attribute set index exceeds supported range"))?;
                    sets.push(set);
                }
                sets.sort_unstable();
                if sets
                    .iter()
                    .enumerate()
                    .any(|(expected, actual)| expected != *actual)
                {
                    return Err(invalid(
                        "indexed attribute sets must start at zero and be consecutive",
                    ));
                }
            }
            let count = accessors[position].count;
            if doc["accessors"][position].get("min").is_none()
                || doc["accessors"][position].get("max").is_none()
            {
                return Err(invalid("POSITION accessor requires min and max"));
            }
            vertices = vertices
                .checked_add(count as u64)
                .ok_or_else(|| limit("vertex count overflow"))?;
            for (semantic, index) in attrs {
                let index = at(&accessors, index)?;
                let a = &accessors[index];
                if a.count != count {
                    return Err(invalid("primitive attribute counts differ"));
                }
                if let Some(v) = a.view {
                    if view_roles[v] == 2 {
                        return Err(invalid("bufferView mixes indices and vertex attributes"));
                    }
                    if attribute_accessors[v].is_some_and(|previous| previous != index)
                        && view_docs[v].get("byteStride").is_none()
                    {
                        return Err(invalid(
                            "distinct vertex attributes sharing a bufferView require byteStride",
                        ));
                    }
                    attribute_accessors[v] = Some(index);
                    view_roles[v] = 1;
                    let absolute = off(&view_docs[v], "byteOffset")? + a.offset;
                    if absolute % 4 != 0 || a.stride % 4 != 0 {
                        return Err(invalid("vertex attribute is not four-byte aligned"));
                    }
                    if view_docs[v].get("target").is_some_and(|v| v != 34962) {
                        return Err(invalid("attribute view target mismatch"));
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
                            return Err(invalid("invalid TEXCOORD set suffix"));
                        }
                        a.shape == "VEC2"
                            && ((a.component == 5126 && !a.normalized)
                                || ([5121, 5123].contains(&a.component) && a.normalized)
                                || (quant && [5120, 5121, 5122, 5123].contains(&a.component)))
                    }
                    s if s.starts_with("COLOR_") => {
                        if !numeric_set(s, "COLOR_") {
                            return Err(invalid("invalid COLOR set suffix"));
                        }
                        ["VEC3", "VEC4"].contains(&a.shape.as_str())
                            && ((a.component == 5126 && !a.normalized)
                                || ([5121, 5123].contains(&a.component) && a.normalized))
                    }
                    s if s.starts_with('_') => a.component != 5125,
                    _ => return Err(unsupported(format!("attribute semantic {semantic}"))),
                };
                if !valid {
                    return Err(invalid(format!("invalid {semantic} accessor type")));
                }
                if a.component != 5126
                    && ["POSITION", "NORMAL", "TANGENT"].contains(&semantic.as_str())
                    && quant
                    && !list(&doc, "extensionsRequired")?
                        .iter()
                        .any(|v| v == "KHR_mesh_quantization")
                {
                    return Err(invalid(
                        "quantized attribute requires KHR_mesh_quantization in extensionsRequired",
                    ));
                }
                if semantic.starts_with("COLOR_") {
                    for raw in a.min.iter().chain(&a.max) {
                        let n = if a.normalized {
                            normalized(*raw, a.component)
                        } else {
                            *raw
                        };
                        if !(0.0..=1.0).contains(&n) {
                            return Err(invalid("COLOR value outside zero to one"));
                        }
                    }
                }
            }
            let n = if let Some(index) = p.get("indices") {
                let i = at(&accessors, index)?;
                index_accessors[i] = true;
                let a = &accessors[i];
                if a.shape != "SCALAR" || ![5121, 5123, 5125].contains(&a.component) || a.normalized
                {
                    return Err(invalid("invalid index accessor type"));
                }
                if let Some(v) = a.view {
                    if view_roles[v] == 1 {
                        return Err(invalid("bufferView mixes indices and vertex attributes"));
                    }
                    view_roles[v] = 2;
                    if view_docs[v].get("byteStride").is_some()
                        || view_docs[v].get("target").is_some_and(|v| v != 34963)
                    {
                        return Err(invalid("index view stride/target invalid"));
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
                    ));
                }
                if has_restart && a.shortest_segment < 2 {
                    return Err(unsupported("restart LINE_STRIP profile requires at least two vertices in every segment"));
                }
                if a.ordinary_max >= count as f64 {
                    return Err(invalid("primitive index outside POSITION count"));
                }
                a.count
            } else {
                count
            };
            if (mode == 1 && n % 2 != 0) || (mode == 3 && n < 2) || (mode == 4 && n % 3 != 0) {
                return Err(invalid("primitive element count incompatible with mode"));
            }
            if let Some(m) = p.get("material") {
                at(list(&doc, "materials")?, m)?;
            }
        }
    }
    for (index, accessor) in accessors.iter().enumerate() {
        if accessor.component == 5125 && !index_accessors[index] {
            return Err(invalid(
                "UNSIGNED_INT accessor must be referenced by primitive indices",
            ));
        }
    }
    for image in list(&doc, "images")? {
        if image.get("uri").is_some() && image.get("bufferView").is_some() {
            return Err(invalid("image has URI and bufferView"));
        }
        if let Some(uri) = image.get("uri") {
            let uri = uri
                .as_str()
                .ok_or_else(|| invalid("image URI must be string"))?;
            if uri.starts_with("data:") {
                return Err(unsupported("image data URI"));
            }
            let data = resolve(uri)?;
            resource_bytes = resource_bytes
                .checked_add(data.len())
                .ok_or_else(|| limit("image resource bytes overflow"))?;
            if resource_bytes > caps().member_bytes as usize {
                return Err(limit("image resource exceeds 64 MiB"));
            }
        } else {
            let view = at(&views, &image["bufferView"])?;
            if accessor_views[view] {
                return Err(invalid("bufferView mixes image data and accessor data"));
            }
            view_roles[view] = 3;
        }
        not_inspected.insert("image decoding".into());
    }
    for texture in list(&doc, "textures")? {
        if let Some(source) = texture.get("source") {
            at(list(&doc, "images")?, source)?;
        }
        if let Some(sampler) = texture.get("sampler") {
            at(list(&doc, "samplers")?, sampler)?;
        }
    }
    for material in list(&doc, "materials")? {
        for info in [
            &material["pbrMetallicRoughness"]["baseColorTexture"],
            &material["pbrMetallicRoughness"]["metallicRoughnessTexture"],
            &material["normalTexture"],
            &material["occlusionTexture"],
            &material["emissiveTexture"],
        ] {
            if !info.is_null() {
                at(list(&doc, "textures")?, &info["index"])?;
            }
        }
    }
    not_inspected
        .insert("scene transforms, skins, animations, morph targets and rendered bounds".into());
    not_inspected.insert("metadata and feature semantics".into());
    Ok(PayloadInspection {
        document: doc,
        accessors_checked: accessors.len() as u64,
        primitives_checked: primitives,
        vertices,
        elements_checked: elements as u64,
        not_inspected,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
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
                unwrap_b3dm(&bad),
                Err(ValidationFailure::InvalidInput(_))
            ));
        }
        assert!(matches!(
            unwrap_b3dm(b"b3dm"),
            Err(ValidationFailure::InvalidInput(_))
        ));
        let mut bad = good.clone();
        let start = 28 + u32::from_le_bytes(bad[12..16].try_into().unwrap()) as usize;
        bad[start + 8..start + 12].copy_from_slice(&u32::MAX.to_le_bytes());
        assert!(matches!(
            unwrap_b3dm(&bad),
            Err(ValidationFailure::InvalidInput(_))
        ));
        let mut bad = good.clone();
        bad[30] = b'X';
        assert!(matches!(
            unwrap_b3dm(&bad),
            Err(ValidationFailure::InvalidInput(_))
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
            Err(ValidationFailure::InvalidInput(_))
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
            Err(ValidationFailure::ResourceLimit(_))
        ));
        doc["accessors"][0]["count"] = json!(u64::MAX);
        assert!(matches!(
            inspect(
                "a.gltf",
                &serde_json::to_vec(&doc).unwrap(),
                4_000_000,
                |_| panic!()
            ),
            Err(ValidationFailure::ResourceLimit(_))
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
            Err(ValidationFailure::Unsupported(_))
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
            Err(ValidationFailure::InvalidInput(_))
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
            Err(ValidationFailure::InvalidInput(_))
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
            Err(ValidationFailure::InvalidInput(_))
        ));
        doc["meshes"][0]["primitives"][0]["mode"] = json!(3);
        bin[36] = 255;
        assert!(matches!(
            inspect("a.glb", &frame(&doc, &bin), 4_000_000, |_| panic!()),
            Err(ValidationFailure::Unsupported(_))
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
            Err(ValidationFailure::InvalidInput(_))
        ));
        assert!(matches!(
            decode_buffer_uri("data:text/plain;base64,AAAA"),
            Err(ValidationFailure::Unsupported(_))
        ));
    }
    #[test]
    fn missing_position_is_a_failure_instead_of_a_panic() {
        let mut doc = triangle();
        doc["meshes"][0]["primitives"][0]["attributes"] = json!({});
        assert!(matches!(
            inspect("a.glb", &frame(&doc, &[0; 36]), 4_000_000, |_| panic!()),
            Err(ValidationFailure::InvalidInput(_))
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
