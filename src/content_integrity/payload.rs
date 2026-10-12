//! Private immutable-byte GLB/glTF/b3dm integrity owner. No paths or operation defaults.
use super::{json, FormatError, PayloadError, PayloadLimits};
use serde_json::value::RawValue;
use std::{borrow::Cow, collections::BTreeSet};
mod numbers;
mod records;
pub(crate) mod scene;
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
    /// Bytes C1 materialized: owned external and data-URI buffers plus meshopt views.
    pub materialized_bytes: u64,
    pub not_inspected: BTreeSet<String>,
    /// C2 format-owned exclusion facts of this payload.
    pub exclusions: scene::ContentExclusions,
    // The three C2-WB facts below are read by the work formula in
    // `validate.rs`; drop this allowance when that consumer lands.
    /// `J(u)`: Σ `value_nodes()` over every JSON document admitted here: the
    /// glTF JSON or GLB JSON chunk (a b3dm's embedded GLB included) and the
    /// b3dm feature-table and batch-table JSON.
    #[allow(dead_code)]
    pub json_nodes: u64,
    /// `Res(u)`: external resources resolved through the resolver, one per
    /// buffer or image with a non-`data:` `uri`.
    #[allow(dead_code)]
    pub external_resources: u64,
    /// `Prim(u)`: Σ over the selected scene's instances (selected nodes with a
    /// mesh) of that mesh's primitive count; 0 without a selected scene.
    #[allow(dead_code)]
    pub instance_primitives: u64,
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
/// The embedded GLB, whether the feature-table JSON has the `RTC_CENTER` key
/// (JSON value or binary-body reference; the value and range are not consumed)
/// and the value nodes of the admitted feature-table and batch-table JSON.
fn unwrap_b3dm(bytes: &[u8], limits: super::JsonLimits) -> FormatResult<(&[u8], bool, u64)> {
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
    let mut rtc_center = false;
    let mut table_nodes = 0u64;
    for (index, length) in lengths.iter().enumerate() {
        let table = range(bytes, offset, *length)?;
        if index == 0 || (index == 2 && *length > 0) {
            let doc = json::admit(table, limits)?;
            table_nodes += doc.value_nodes() as u64;
            let feature: records::FeatureTable<'_> = record(Some(doc.raw()))?;
            if index == 0 {
                domain(feature.batch_length, u32::MAX as u64)?;
                rtc_center = feature.rtc_center.is_some();
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
    Ok((glb, rtc_center, table_nodes))
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
/// C1 integrity plus C2 scene/node admission and exclusion facts, without a
/// scene consumer (format controls and producer self-checks).
#[cfg(test)]
pub(crate) fn inspect<E>(
    kind: PayloadKind,
    bytes: &[u8],
    limits: PayloadLimits,
    remaining_components: u64,
    resolve: impl FnMut(&str, usize) -> std::result::Result<Vec<u8>, E>,
) -> std::result::Result<PayloadInspection, PayloadError<E>> {
    inspect_scene(
        kind,
        bytes,
        limits,
        remaining_components,
        resolve,
        &mut scene::NoScene,
    )
    .map_err(|error| match error {
        SceneError::Payload(error) => error,
        SceneError::Consumer(never) => match never {},
    })
}
/// A payload failure, or a scene-consumer failure with its own distinct type.
#[derive(Debug)]
pub(crate) enum SceneError<E, C> {
    Payload(PayloadError<E>),
    Consumer(C),
}
impl<E, C> From<FormatError> for SceneError<E, C> {
    fn from(error: FormatError) -> Self {
        Self::Payload(error.into())
    }
}
/// C1 integrity, then C2 scene/node admission and exclusion facts, then the
/// selected-scene walk into `consumer` over the live C1 views.
pub(crate) fn inspect_scene<E, C: scene::SceneConsumer>(
    kind: PayloadKind,
    bytes: &[u8],
    limits: PayloadLimits,
    remaining_components: u64,
    mut resolve: impl FnMut(&str, usize) -> std::result::Result<Vec<u8>, E>,
    consumer: &mut C,
) -> std::result::Result<PayloadInspection, SceneError<E, C::Error>> {
    if bytes.len() > limits.member_bytes {
        return Err(limit("payload exceeds member byte allowance").into());
    }
    let mut exclusions = scene::ContentExclusions::default();
    let mut json_nodes = 0u64;
    let (source, bin) = match kind {
        PayloadKind::B3dm => {
            let (glb, rtc_center, table_nodes) = unwrap_b3dm(bytes, limits.json)?;
            exclusions.rtc_center = rtc_center;
            json_nodes = table_nodes;
            envelope(glb)?
        }
        PayloadKind::Glb => envelope(bytes)?,
        PayloadKind::LocalGltf => (bytes, None),
    };
    let document = json::admit(source, limits.json)?;
    let slots = document.value_nodes();
    json_nodes += slots as u64;
    let mut external_resources = 0u64;
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
    let mut materialized = 0u64;
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
                external_resources += 1;
                Cow::Owned(
                    resolve(&uri, remaining)
                        .map_err(|e| SceneError::Payload(PayloadError::Resolver(e)))?,
                )
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
        if let Cow::Owned(owned) = &data {
            materialized = materialized.saturating_add(owned.len() as u64);
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
            materialized = materialized.saturating_add(decoded.len() as u64);
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
    let mut table = scene::MeshTable {
        meshes: Vec::new(),
        primitives: Vec::new(),
        positions: Vec::new(),
        groups: Vec::new(),
        grouped: Vec::new(),
    };
    let mut position_stamp = allocation(accessors.len(), usize::MAX)?;
    // Slot of a POSITION accessor within the current mesh's distinct range.
    let mut position_slot = allocation(accessors.len(), 0usize)?;
    for (mesh_index, mesh) in array::<&RawValue>(root.meshes, slots)?
        .into_iter()
        .enumerate()
    {
        let mesh: records::Mesh<'_> = uninspected_record(Some(mesh))?;
        let mut fact = scene::MeshFact {
            primitives: (table.primitives.len(), table.primitives.len()),
            positions: (table.positions.len(), table.positions.len()),
            weights: mesh.weights.is_some(),
        };
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
            let indices = p.indices.map(|raw| at(&accessors, Some(raw))).transpose()?;
            push(
                &mut table.primitives,
                scene::PrimitiveFact {
                    position,
                    indices,
                    targets: p.targets.is_some(),
                },
                slots,
            )?;
            if position_stamp[position] != mesh_index {
                position_stamp[position] = mesh_index;
                position_slot[position] = table.positions.len() - fact.positions.0;
                push(&mut table.positions, position, slots)?;
            }
        }
        fact.primitives.1 = table.primitives.len();
        fact.positions.1 = table.positions.len();
        table.group(&fact, &position_slot, slots)?;
        push(&mut table.meshes, fact, slots)?;
    }
    drop(position_stamp);
    drop(position_slot);
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
            external_resources += 1;
            let data = resolve(&uri, remaining)
                .map_err(|e| SceneError::Payload(PayloadError::Resolver(e)))?;
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
    not_inspected.insert("skins, animations, morph targets and rendered extent".into());
    // The operation removes this entry only when every reference is contained.
    not_inspected.insert("scene transforms".into());
    not_inspected.insert("metadata and feature semantics".into());
    // C2 admission follows C1 integrity: malformed scene/node records are
    // InvalidInput in every payload, selected or not.
    let graph = scene::admit(&root, &table, slots)?;
    exclusions.gltf_extension = used
        .iter()
        .any(|name| !scene::GLTF_ALLOWLIST.contains(&name.as_str()));
    let instance_primitives = graph.exclusions(&table, root.animations, slots, &mut exclusions)?;
    let draws = scene::Draws {
        accessors: &accessors,
        views: &views,
    };
    graph
        .walk(&table, draws, consumer, slots)?
        .map_err(SceneError::Consumer)?;
    Ok(PayloadInspection {
        schema_uri,
        accessors_checked: accessors.len() as u64,
        primitives_checked: primitives,
        vertices,
        elements_checked: elements as u64,
        materialized_bytes: materialized,
        not_inspected,
        exclusions,
        json_nodes,
        external_resources,
        instance_primitives,
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
                super::unwrap_b3dm(&bad, fixture_limits().json).map(|_| ()),
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
    fn scene_doc() -> Value {
        let mut doc = triangle();
        doc["scene"] = json!(0);
        doc["scenes"] = json!([{"nodes":[0]}]);
        doc["nodes"] = json!([{"mesh":0}]);
        doc
    }
    fn glb_facts(doc: &Value) -> FormatResult<PayloadInspection> {
        inspect("a.glb", &frame(doc, &[0; 36]), 4_000_000, |_| panic!())
    }
    #[test]
    fn c1_label_is_split_and_scene_transforms_is_separate() {
        let facts = glb_facts(&scene_doc()).unwrap();
        assert!(facts.not_inspected.contains("scene transforms"));
        assert!(facts
            .not_inspected
            .contains("skins, animations, morph targets and rendered extent"));
        assert!(!facts
            .not_inspected
            .iter()
            .any(|s| s.contains("rendered bounds")));
        assert_eq!(facts.exclusions, scene::ContentExclusions::default());
    }
    #[test]
    fn malformed_scene_and_node_records_are_invalid_in_any_payload() {
        let cases: Vec<(&str, Value)> = vec![
            ("scene index", json!({"scene":1})),
            ("scene root index", json!({"scenes":[{"nodes":[3]}]})),
            ("child index", json!({"nodes":[{"mesh":0,"children":[7]}]})),
            ("mesh index", json!({"nodes":[{"mesh":1}]})),
            (
                "two parents",
                json!({"scenes":[{"nodes":[0]}],"nodes":[{"children":[2]},{"children":[2]},{"mesh":0}]}),
            ),
            (
                "cycle",
                json!({"scenes":[{"nodes":[0]}],"nodes":[{"mesh":0},{"children":[2]},{"children":[1]}]}),
            ),
            (
                "self cycle",
                json!({"scenes":[{"nodes":[0]}],"nodes":[{"mesh":0},{"children":[1]}]}),
            ),
            (
                "root with parent",
                json!({"scenes":[{"nodes":[0,1]}],"nodes":[{"children":[1]},{"mesh":0}]}),
            ),
            ("duplicate root", json!({"scenes":[{"nodes":[0,0]}]})),
            (
                "matrix and TRS",
                json!({"nodes":[{"mesh":0,"matrix":[1,0,0,0,0,1,0,0,0,0,1,0,0,0,0,1],"scale":[1,1,1]}]}),
            ),
            (
                "matrix length",
                json!({"nodes":[{"mesh":0,"matrix":[1,0,0,0]}]}),
            ),
            (
                "matrix last row",
                json!({"nodes":[{"mesh":0,"matrix":[1,0,0,0,0,1,0,0,0,0,1,0,0,0,0,2]}]}),
            ),
            (
                "matrix projective",
                json!({"nodes":[{"mesh":0,"matrix":[1,0,0,1,0,1,0,0,0,0,1,0,0,0,0,1]}]}),
            ),
            (
                "zero quaternion",
                json!({"nodes":[{"mesh":0,"rotation":[0,0,0,-0.0]}]}),
            ),
            (
                "rotation range",
                json!({"nodes":[{"mesh":0,"rotation":[0,0,1.5,0]}]}),
            ),
            (
                "rotation type",
                json!({"nodes":[{"mesh":0,"rotation":"identity"}]}),
            ),
            (
                "translation length",
                json!({"nodes":[{"mesh":0,"translation":[1,2]}]}),
            ),
            (
                "scale string",
                json!({"nodes":[{"mesh":0,"scale":[1,"2",3]}]}),
            ),
            ("node object", json!({"nodes":[3]})),
            (
                "unselected node",
                json!({"nodes":[{"mesh":0},{"translation":[0,0]}]}),
            ),
        ];
        for (name, patch) in cases {
            let mut doc = scene_doc();
            for (key, value) in patch.as_object().unwrap() {
                doc[key] = value.clone();
            }
            assert!(
                matches!(glb_facts(&doc), Err(FormatError::InvalidInput(_))),
                "{name}"
            );
        }
        // A literal that overflows binary64 is InvalidInput, not a rounded infinity.
        let text = serde_json::to_string(&scene_doc())
            .unwrap()
            .replace(r#"{"mesh":0}"#, r#"{"mesh":0,"translation":[1e400,0,0]}"#);
        let mut json = text.into_bytes();
        while !json.len().is_multiple_of(4) {
            json.push(b' ');
        }
        let mut glb = b"glTF".to_vec();
        glb.extend(2u32.to_le_bytes());
        glb.extend((28 + json.len() as u32 + 36).to_le_bytes());
        glb.extend((json.len() as u32).to_le_bytes());
        glb.extend(b"JSON");
        glb.extend(json);
        glb.extend(36u32.to_le_bytes());
        glb.extend(b"BIN\0");
        glb.extend([0; 36]);
        assert!(matches!(
            inspect("a.glb", &glb, 4_000_000, |_| panic!()),
            Err(FormatError::InvalidInput(_))
        ));
    }
    #[test]
    fn scene_selection_and_selected_exclusions() {
        let facts = |patch: Value| {
            let mut doc = scene_doc();
            for (key, value) in patch.as_object().unwrap() {
                if value.is_null() {
                    doc.as_object_mut().unwrap().remove(key);
                } else {
                    doc[key] = value.clone();
                }
            }
            glb_facts(&doc).unwrap().exclusions
        };
        let none = scene::ContentExclusions::default();
        assert_eq!(facts(json!({"scene":null})), none, "sole scene");
        for patch in [
            json!({"scene":null,"scenes":null}),
            json!({"scene":null,"scenes":[]}),
            json!({"scene":null,"scenes":[{"nodes":[0]},{"nodes":[0]}]}),
        ] {
            assert!(facts(patch).scene_selection);
        }
        assert!(!facts(json!({"scenes":[{"nodes":[0]},{}]})).scene_selection);
        assert!(facts(json!({"nodes":[{"mesh":0,"skin":0}]})).skin);
        assert!(facts(json!({"nodes":[{"mesh":0,"weights":[0]}]})).morph_targets);
        assert!(
            facts(json!({"meshes":[{"weights":[],"primitives":[{"attributes":{"POSITION":0}}]}]}))
                .morph_targets
        );
        assert!(
            facts(json!({"meshes":[{"primitives":[{"attributes":{"POSITION":0},"targets":[]}]}]}))
                .morph_targets
        );
        // Unselected nodes and an empty animation list are not exclusions.
        let unselected =
            facts(json!({"nodes":[{"mesh":0},{"skin":0,"weights":[1]}],"animations":[]}));
        assert_eq!(unselected, none);
        assert!(facts(json!({"animations":[{}]})).animation);
        let cesium = facts(
            json!({"extensionsUsed":["CESIUM_RTC"],"extensions":{"CESIUM_RTC":{"center":[1,2,3]}}}),
        );
        assert!(cesium.gltf_extension);
        assert!(facts(json!({"extensionsUsed":["EXT_mesh_gpu_instancing"]})).gltf_extension);
        assert!(
            !facts(json!({"extensionsUsed":["KHR_materials_unlit","EXT_mesh_features"]}))
                .gltf_extension
        );
    }
    fn b3dm_with_table(glb: &[u8], table: &[u8], body: usize) -> Vec<u8> {
        let mut table = table.to_vec();
        while !(28 + table.len()).is_multiple_of(8) {
            table.push(b' ');
        }
        let mut result = b"b3dm".to_vec();
        result.extend(1u32.to_le_bytes());
        result.extend(0u32.to_le_bytes());
        result.extend((table.len() as u32).to_le_bytes());
        result.extend((body as u32).to_le_bytes());
        result.extend([0u8; 8]);
        result.extend(table);
        result.extend(vec![0u8; body]);
        result.extend(glb);
        while !result.len().is_multiple_of(8) {
            result.push(0);
        }
        let length = result.len() as u32;
        result[8..12].copy_from_slice(&length.to_le_bytes());
        result
    }
    #[test]
    fn b3dm_rtc_center_key_is_detected_in_json_and_binary_form() {
        let glb = frame(&scene_doc(), &[0; 36]);
        for (table, body, rtc) in [
            (&br#"{"BATCH_LENGTH":0}"#[..], 0, false),
            (&br#"{"BATCH_LENGTH":0,"RTC_CENTER":[1,2,3]}"#[..], 0, true),
            (
                &br#"{"BATCH_LENGTH":0,"RTC_CENTER":{"byteOffset":0}}"#[..],
                16,
                true,
            ),
            (&br#"{"BATCH_LENGTH":0,"RTC_CENTER":null}"#[..], 0, true),
        ] {
            let bytes = b3dm_with_table(&glb, table, body);
            let facts = inspect("a.b3dm", &bytes, 4_000_000, |_| panic!()).unwrap();
            assert_eq!(
                facts.exclusions.rtc_center,
                rtc,
                "{}",
                String::from_utf8_lossy(table)
            );
        }
    }
    #[derive(Default)]
    struct Recorder {
        events: Vec<String>,
        instances_only: bool,
    }
    impl scene::SceneConsumer for Recorder {
        type Error = ();
        fn instances_only(&self) -> bool {
            self.instances_only
        }
        fn enter(&mut self, t: &scene::NodeTransform, depth: u64) -> Result<(), ()> {
            let kind = match t {
                scene::NodeTransform::Skip => "skip",
                scene::NodeTransform::Matrix(_) => "matrix",
                scene::NodeTransform::Trs { .. } => "trs",
            };
            self.events.push(format!("enter {kind} {depth}"));
            Ok(())
        }
        fn leave(&mut self, _: &scene::NodeTransform) {
            self.events.push("leave".into());
        }
        fn instance(&mut self, i: &scene::Instance<'_, '_>) -> Result<(), ()> {
            for &a in i.positions() {
                let mut marks = Vec::new();
                i.mark(a, |v| marks.push(v));
                let words: Vec<_> = marks.iter().map(|v| i.vertex(a, *v)).collect();
                self.events.push(format!(
                    "instance node={} mesh={} accessor={a} format={:?} marks={marks:?} words={words:?} T={} Idx={}",
                    i.node(),
                    i.mesh(),
                    i.format(a),
                    i.vertex_total(),
                    i.index_total()
                ));
            }
            Ok(())
        }
    }
    fn walk_events(doc: &Value, bin: &[u8], instances_only: bool) -> Vec<String> {
        let mut recorder = Recorder {
            instances_only,
            ..Default::default()
        };
        super::inspect_scene(
            PayloadKind::Glb,
            &frame(doc, bin),
            fixture_limits(),
            4_000_000,
            |_, _| -> Result<Vec<u8>, ()> { panic!() },
            &mut recorder,
        )
        .map_err(|_| ())
        .unwrap();
        recorder.events
    }
    #[test]
    fn selected_walk_is_root_first_in_order_and_prunes_only_for_streaming() {
        let mut doc = scene_doc();
        doc["scenes"] = json!([{"nodes":[3]},{"nodes":[1,0]}]);
        doc["scene"] = json!(1);
        doc["nodes"] = json!([
            {"mesh":0,"translation":[1,0,0]},
            {"children":[2],"matrix":[1,0,0,0,0,1,0,0,0,0,1,0,0,0,0,1]},
            {"rotation":[0,0,0,1]},
            {"mesh":0}
        ]);
        let all = walk_events(&doc, &[0; 36], false);
        assert_eq!(
            all[..4],
            ["enter matrix 1", "enter trs 2", "leave", "leave"].map(String::from)
        );
        assert_eq!(all[4], "enter trs 1");
        assert!(all[5].starts_with("instance node=0 mesh=0 accessor=0 format=F32 marks=[0, 1, 2]"));
        assert_eq!(all[6], "leave");
        assert_eq!(all.len(), 7);
        let pruned = walk_events(&doc, &[0; 36], true);
        assert_eq!(pruned[0], "enter trs 1");
        assert_eq!(pruned.len(), 3);
    }
    #[test]
    fn drawn_indices_skip_restart_dedupe_per_accessor_and_ignore_polygon_loops() {
        // Two primitives share POSITION 0 (counted once in T); indices name 0 and 2
        // only, and the line strip has a restart value. Vertex 1 is undrawn.
        let mut doc = scene_doc();
        doc["extensionsUsed"] = json!(["KHR_mesh_primitive_restart", "EXT_mesh_polygon"]);
        doc["extensionsRequired"] = json!(["KHR_mesh_primitive_restart"]);
        doc["buffers"][0]["byteLength"] = json!(48);
        doc["bufferViews"] = json!([
            {"buffer":0,"byteLength":36},
            {"buffer":0,"byteOffset":36,"byteLength":5},
            {"buffer":0,"byteOffset":44,"byteLength":4}
        ]);
        doc["accessors"] = json!([
            {"bufferView":0,"componentType":5126,"type":"VEC3","count":3,"min":[1,2,3],"max":[7,8,9]},
            {"bufferView":1,"componentType":5121,"type":"SCALAR","count":5},
            {"bufferView":2,"componentType":5121,"type":"SCALAR","count":3},
            {"componentType":5121,"type":"SCALAR","count":1},
            {"bufferView":2,"componentType":5121,"type":"SCALAR","count":1,"byteOffset":3}
        ]);
        doc["meshes"] = json!([{"primitives":[
            {"attributes":{"POSITION":0},"indices":1,"mode":3},
            {"attributes":{"POSITION":0},"indices":2,"mode":4,"extensions":{"EXT_mesh_polygon":{"count":1,"indicesOffsets":3,"loopIndices":4,"loopIndicesOffsets":3}}}
        ]}]);
        let mut bin = Vec::new();
        for v in [1f32, 2., 3., 4., 5., 6., 7., 8., 9.] {
            bin.extend(v.to_le_bytes());
        }
        bin.extend([0, 2, 255, 2, 0, 0, 0, 0]);
        bin.extend([2, 0, 2, 1]);
        // Loop index 1 names vertex 1, which no core index draws: loops are not draws.
        let events = walk_events(&doc, &bin, true);
        let instance = events.iter().find(|e| e.starts_with("instance")).unwrap();
        assert!(
            instance.contains("marks=[0, 2, 2, 0, 2, 0, 2]"),
            "{instance}"
        );
        assert!(instance.contains("T=3 Idx=8"), "{instance}");
        assert!(instance.contains("F32([1.0, 2.0, 3.0])"), "{instance}");
    }
    #[test]
    fn quantized_words_stay_raw_and_viewless_positions_are_zero() {
        let mut doc = scene_doc();
        doc["extensionsUsed"] = json!(["KHR_mesh_quantization"]);
        doc["extensionsRequired"] = json!(["KHR_mesh_quantization"]);
        doc["buffers"][0]["byteLength"] = json!(24);
        doc["bufferViews"][0] = json!({"buffer":0,"byteLength":24,"byteStride":8});
        doc["accessors"][0] = json!({"bufferView":0,"componentType":5122,"normalized":true,"type":"VEC3","count":3,"min":[-32768,0,1],"max":[32767,0,1]});
        let mut bin = Vec::new();
        for v in [[-32768i16, 0, 1], [32767, 0, 1], [0, 0, 1]] {
            for c in v {
                bin.extend(c.to_le_bytes());
            }
            bin.extend([0, 0]);
        }
        let events = walk_events(&doc, &bin, true);
        assert!(
            events[1].contains("format=I16 { normalized: true }")
                && events[1].contains("Int([-32768, 0, 1]), Int([32767, 0, 1])"),
            "{events:?}"
        );
        let mut doc = scene_doc();
        doc["accessors"][0] =
            json!({"componentType":5126,"type":"VEC3","count":3,"min":[5,5,5],"max":[6,6,6]});
        let events = walk_events(&doc, &[0; 36], true);
        assert!(events[1].contains("F32([0.0, 0.0, 0.0])"), "{events:?}");
    }
    #[test]
    fn materialized_bytes_count_owned_buffers_only() {
        let doc = scene_doc();
        assert_eq!(glb_facts(&doc).unwrap().materialized_bytes, 0);
        let mut doc = scene_doc();
        doc["buffers"][0]["uri"] = json!("a.bin");
        let facts = inspect(
            "a.gltf",
            &serde_json::to_vec(&doc).unwrap(),
            4_000_000,
            |_| Ok(vec![0; 40]),
        )
        .unwrap();
        assert_eq!(facts.materialized_bytes, 40);
    }
    /// Value nodes as C1 counts them: the root, every container and every
    /// scalar; object keys are not nodes.
    fn count_value_nodes(v: &Value) -> u64 {
        1 + match v {
            Value::Array(items) => items.iter().map(count_value_nodes).sum(),
            Value::Object(members) => members.values().map(count_value_nodes).sum(),
            _ => 0,
        }
    }
    fn b3dm_tables(glb: &[u8], feature: &Value, batch: Option<&Value>) -> Vec<u8> {
        let mut ft = serde_json::to_vec(feature).unwrap();
        while !(28 + ft.len()).is_multiple_of(8) {
            ft.push(b' ');
        }
        let mut bt = batch.map_or(Vec::new(), |b| serde_json::to_vec(b).unwrap());
        while !bt.len().is_multiple_of(8) {
            bt.push(b' ');
        }
        let mut result = b"b3dm".to_vec();
        result.extend(1u32.to_le_bytes());
        result.extend(0u32.to_le_bytes());
        for n in [ft.len(), 0, bt.len(), 0] {
            result.extend((n as u32).to_le_bytes());
        }
        result.extend(ft);
        result.extend(bt);
        result.extend(glb);
        while !result.len().is_multiple_of(8) {
            result.push(0);
        }
        let length = result.len() as u32;
        result[8..12].copy_from_slice(&length.to_le_bytes());
        result
    }
    #[test]
    fn json_nodes_sum_value_nodes_of_every_admitted_document() {
        let doc = scene_doc();
        let doc_nodes = count_value_nodes(&doc);
        let doc_bytes = serde_json::to_vec(&doc).unwrap();
        assert_eq!(
            json::admit(&doc_bytes, fixture_limits().json)
                .unwrap()
                .value_nodes() as u64,
            doc_nodes,
            "the reference count agrees with C1's value_nodes()"
        );
        assert_eq!(glb_facts(&doc).unwrap().json_nodes, doc_nodes);
        // A local .gltf with a data: buffer counts its one document.
        let mut local = doc.clone();
        local["buffers"][0]["uri"] = json!(format!(
            "data:application/octet-stream;base64,{}",
            "A".repeat(48)
        ));
        let local_facts = inspect(
            "a.gltf",
            &serde_json::to_vec(&local).unwrap(),
            4_000_000,
            |_| panic!(),
        )
        .unwrap();
        assert_eq!(local_facts.json_nodes, count_value_nodes(&local));
        let glb = frame(&doc, &[0; 36]);
        let feature = json!({"BATCH_LENGTH":2,"RTC_CENTER":[1,2,3]});
        let batch = json!({"id":[1,2],"name":["a","b"],"nested":{"k":[{"x":null},true]}});
        for (batch, expected) in [
            (None, count_value_nodes(&feature) + doc_nodes),
            (
                Some(&batch),
                count_value_nodes(&feature) + count_value_nodes(&batch) + doc_nodes,
            ),
        ] {
            let bytes = b3dm_tables(&glb, &feature, batch);
            let facts = inspect("a.b3dm", &bytes, 4_000_000, |_| panic!()).unwrap();
            assert_eq!(facts.json_nodes, expected, "batch table {batch:?}");
        }
        // Hand count for the smallest b3dm: {"BATCH_LENGTH":0} is 2 nodes.
        let facts = inspect(
            "a.b3dm",
            &b3dm_tables(&glb, &json!({"BATCH_LENGTH":0}), None),
            4_000_000,
            |_| panic!(),
        )
        .unwrap();
        assert_eq!(facts.json_nodes, 2 + doc_nodes);
    }
    #[test]
    fn external_resources_count_resolved_buffer_and_image_uris() {
        assert_eq!(glb_facts(&scene_doc()).unwrap().external_resources, 0);
        let mut doc = scene_doc();
        doc["buffers"] = json!([
            {"uri":"a.bin","byteLength":36},
            {"uri":"data:application/octet-stream;base64,AAAA","byteLength":3},
            {"uri":"b.bin","byteLength":4}
        ]);
        doc["images"] = json!([{"uri":"t.png"}, {"uri":"u.png"}]);
        let mut resolved = Vec::new();
        let facts = inspect(
            "a.gltf",
            &serde_json::to_vec(&doc).unwrap(),
            4_000_000,
            |uri| {
                resolved.push(uri.to_owned());
                Ok(vec![0; if uri == "a.bin" { 36 } else { 4 }])
            },
        )
        .unwrap();
        // Two external buffers and two images; the data: buffer is not resolved.
        assert_eq!(resolved, ["a.bin", "b.bin", "t.png", "u.png"]);
        assert_eq!(facts.external_resources, 4);
        // An image stored in a bufferView is not an external resource.
        let mut doc = scene_doc();
        doc["buffers"][0]["byteLength"] = json!(40);
        doc["bufferViews"] = json!([
            {"buffer":0,"byteLength":36},
            {"buffer":0,"byteOffset":36,"byteLength":4}
        ]);
        doc["images"] = json!([{"bufferView":1,"mimeType":"image/png"}]);
        let facts = inspect("a.glb", &frame(&doc, &[0; 40]), 4_000_000, |_| panic!()).unwrap();
        assert_eq!(facts.external_resources, 0);
    }
    #[test]
    fn instance_primitives_sum_selected_instances_mesh_primitive_counts() {
        let mut doc = scene_doc();
        let p = json!({"attributes":{"POSITION":0}});
        doc["meshes"] = json!([
            {"primitives":[p.clone(), p.clone()]},
            {"primitives":[p.clone(), p.clone(), p.clone()]}
        ]);
        // Selected scene 0: node 0 (mesh 0, 2) with child 1 (mesh 1, 3), node 2
        // (mesh 1, 3), node 4 without a mesh. Node 3 (mesh 0) is unselected.
        doc["nodes"] = json!([
            {"mesh":0,"children":[1]},
            {"mesh":1},
            {"mesh":1,"translation":[1,0,0]},
            {"mesh":0},
            {}
        ]);
        doc["scenes"] = json!([{"nodes":[0,2,4]},{"nodes":[3]}]);
        doc["scene"] = json!(0);
        assert_eq!(glb_facts(&doc).unwrap().instance_primitives, 2 + 3 + 3);
        doc["scene"] = json!(1);
        assert_eq!(glb_facts(&doc).unwrap().instance_primitives, 2);
        // No selected scene: no instances.
        doc.as_object_mut().unwrap().remove("scene");
        assert_eq!(glb_facts(&doc).unwrap().instance_primitives, 0);
        assert_eq!(glb_facts(&triangle()).unwrap().instance_primitives, 0);
    }
    /// xorshift64*: a deterministic generator for the equivalence corpus.
    struct Rng(u64);
    impl Rng {
        fn below(&mut self, n: usize) -> usize {
            self.0 ^= self.0 >> 12;
            self.0 ^= self.0 << 25;
            self.0 ^= self.0 >> 27;
            (self.0.wrapping_mul(0x2545_f491_4f6c_dd1d) >> 32) as usize % n
        }
    }
    /// A random valid GLB: shared and repeated POSITION accessors across meshes,
    /// u8 index accessors with repeated values, and a random node forest.
    fn random_scene(rng: &mut Rng) -> (Value, Vec<u8>, usize) {
        let positions = 1 + rng.below(80);
        let index_count = rng.below(40);
        let mut bin = Vec::new();
        for _ in 0..4 {
            for v in [1f32, -1., 1.] {
                bin.extend(v.to_le_bytes());
            }
        }
        let mut accessors = Vec::new();
        let mut counts = Vec::new();
        for _ in 0..positions {
            let count = 1 + rng.below(4);
            counts.push(count);
            accessors.push(json!({"bufferView":0,"componentType":5126,"type":"VEC3","count":count,"min":[1,-1,1],"max":[1,-1,1]}));
        }
        let mut index_bytes = Vec::new();
        let mut index_max = Vec::new();
        for _ in 0..index_count {
            let n = 1 + rng.below(12);
            let top = rng.below(4);
            let offset = index_bytes.len();
            for _ in 0..n {
                index_bytes.push(rng.below(top + 1) as u8);
            }
            index_max.push(top);
            accessors.push(json!({"bufferView":1,"byteOffset":offset,"componentType":5121,"type":"SCALAR","count":n}));
        }
        let mut views = vec![json!({"buffer":0,"byteLength":48,"byteStride":12})];
        if !index_bytes.is_empty() {
            views.push(json!({"buffer":0,"byteOffset":48,"byteLength":index_bytes.len()}));
        }
        bin.extend(&index_bytes);
        let mut meshes = Vec::new();
        for _ in 0..1 + rng.below(6) {
            let mut primitives = Vec::new();
            for _ in 0..1 + rng.below(50) {
                // Half the primitives draw from a small pool, so groups repeat.
                let pool = if rng.below(2) == 0 {
                    positions.min(5)
                } else {
                    positions
                };
                let position = rng.below(pool);
                let mut p = json!({"attributes":{"POSITION":position},"mode":0});
                if index_count > 0 && rng.below(2) == 0 {
                    let i = rng.below(index_count);
                    if index_max[i] < counts[position] {
                        p["indices"] = json!(positions + i);
                    }
                }
                primitives.push(p);
            }
            meshes.push(json!({ "primitives": primitives }));
        }
        let node_count = 1 + rng.below(30);
        let mut nodes = Vec::new();
        let mut roots = Vec::new();
        for n in 0..node_count {
            let mut node = json!({});
            if rng.below(4) != 0 {
                node["mesh"] = json!(rng.below(meshes.len()));
            }
            nodes.push(node);
            if n > 0 && rng.below(3) == 0 {
                let parent = rng.below(n);
                let children = nodes[parent]
                    .as_object_mut()
                    .unwrap()
                    .entry("children")
                    .or_insert(json!([]));
                children.as_array_mut().unwrap().push(json!(n));
            } else {
                roots.push(n);
            }
        }
        let doc = json!({
            "asset":{"version":"2.0"},
            "buffers":[{"byteLength":bin.len()}],
            "bufferViews":views,
            "accessors":accessors,
            "meshes":meshes,
            "nodes":nodes,
            "scenes":[{"nodes":roots}],
            "scene":0
        });
        (doc, bin, positions + index_count)
    }
    /// Compares `mark` with the pre-C2-WB `mark_scan` for every accessor of
    /// every instance, in order, in reverse and over every accessor id.
    struct Equivalence {
        accessors: usize,
        calls: usize,
        marked: usize,
    }
    impl scene::SceneConsumer for Equivalence {
        type Error = ();
        fn instances_only(&self) -> bool {
            false
        }
        fn enter(&mut self, _: &scene::NodeTransform, _: u64) -> Result<(), ()> {
            Ok(())
        }
        fn leave(&mut self, _: &scene::NodeTransform) {}
        fn instance(&mut self, i: &scene::Instance<'_, '_>) -> Result<(), ()> {
            let order = i
                .positions()
                .iter()
                .copied()
                .chain(i.positions().iter().rev().copied())
                .chain(0..self.accessors + 2);
            for a in order {
                let (mut new, mut old) = (Vec::new(), Vec::new());
                i.mark(a, |v| new.push(v));
                i.mark_scan(a, |v| old.push(v));
                assert_eq!(new, old, "node {} mesh {} accessor {a}", i.node(), i.mesh());
                self.calls += 1;
                self.marked += new.len();
            }
            Ok(())
        }
    }
    #[test]
    fn grouped_mark_equals_the_scan_on_random_scenes() {
        let mut rng = Rng(0x9e37_79b9_7f4a_7c15);
        let (mut calls, mut marked, mut instances) = (0, 0, 0);
        for seed in 0..300 {
            let (doc, bin, accessors) = random_scene(&mut rng);
            let mut consumer = Equivalence {
                accessors,
                calls: 0,
                marked: 0,
            };
            let facts = super::inspect_scene(
                PayloadKind::Glb,
                &frame(&doc, &bin),
                fixture_limits(),
                4_000_000,
                |_, _| -> Result<Vec<u8>, ()> { panic!() },
                &mut consumer,
            )
            .unwrap_or_else(|e| panic!("scene {seed}: {e:?}"));
            calls += consumer.calls;
            marked += consumer.marked;
            instances += facts.instance_primitives;
        }
        eprintln!("mark equivalence: 300 scenes, {calls} mark calls, {marked} marks, {instances} instance-primitive pairs");
        assert!(calls > 10_000 && marked > 10_000 && instances > 10_000);
    }
    /// The review's many-accessor shape (`quadB` in the C2-WB probes): `a`
    /// POSITION accessors of one vertex on one strided view, one POINTS mesh
    /// whose primitive `i` uses accessor `i % a`, and `m` root instances.
    fn quad_shape(a: usize, p: usize, m: usize) -> Vec<u8> {
        let accessor = json!({"bufferView":0,"componentType":5126,"type":"VEC3","count":1,"min":[1,-1,1],"max":[1,-1,1]});
        let doc = json!({
            "asset":{"version":"2.0"},
            "buffers":[{"byteLength":12}],
            "bufferViews":[{"buffer":0,"byteLength":12,"byteStride":12}],
            "accessors":vec![accessor; a],
            "meshes":[{"primitives":(0..p).map(|i| json!({"attributes":{"POSITION":i % a},"mode":0})).collect::<Vec<_>>()}],
            "nodes":vec![json!({"mesh":0}); m],
            "scenes":[{"nodes":(0..m).collect::<Vec<_>>()}],
            "scene":0
        });
        let mut bin = Vec::new();
        for v in [1f32, -1., 1.] {
            bin.extend(v.to_le_bytes());
        }
        frame(&doc, &bin)
    }
    /// Times the streaming consumer's marking loop (bitmap per accessor).
    struct MarkTimer {
        scan: bool,
        nanos: u128,
        marked: u64,
        bitmap: Vec<u64>,
    }
    impl scene::SceneConsumer for MarkTimer {
        type Error = ();
        fn instances_only(&self) -> bool {
            true
        }
        fn enter(&mut self, _: &scene::NodeTransform, _: u64) -> Result<(), ()> {
            Ok(())
        }
        fn leave(&mut self, _: &scene::NodeTransform) {}
        fn instance(&mut self, i: &scene::Instance<'_, '_>) -> Result<(), ()> {
            let start = std::time::Instant::now();
            for &a in i.positions() {
                let words = i.count(a).div_ceil(64);
                self.bitmap.clear();
                self.bitmap.resize(words, 0);
                let bitmap = &mut self.bitmap;
                if self.scan {
                    i.mark_scan(a, |v| bitmap[v / 64] |= 1 << (v % 64));
                } else {
                    i.mark(a, |v| bitmap[v / 64] |= 1 << (v % 64));
                }
                self.marked += bitmap
                    .iter()
                    .map(|w| u64::from(w.count_ones()))
                    .sum::<u64>();
            }
            self.nanos += start.elapsed().as_nanos();
            Ok(())
        }
    }
    fn time_marks(bytes: &[u8], scan: bool) -> (u128, u64) {
        let mut limits = fixture_limits();
        limits.json.bytes = 32 << 20;
        limits.json.value_nodes = 1 << 22;
        let mut timer = MarkTimer {
            scan,
            nanos: 0,
            marked: 0,
            bitmap: Vec::new(),
        };
        super::inspect_scene(
            PayloadKind::Glb,
            bytes,
            limits,
            4_000_000,
            |_, _| -> Result<Vec<u8>, ()> { panic!() },
            &mut timer,
        )
        .unwrap();
        (timer.nanos, timer.marked)
    }
    /// Linearity control (C2-WB A3): marking time per instance·(A + P) on the
    /// many-accessor shapes against the shared-accessor class, before (the
    /// scan) and after (grouped). Release build:
    /// `cargo test --release --lib mark_linearity -- --ignored --nocapture`.
    #[test]
    #[ignore = "timing control; run in release"]
    fn mark_linearity_control() {
        let shapes = [
            ("shared-16000-200", 1, 16_000, 200),
            ("quad-6000-20", 6_000, 6_000, 20),
            ("quadB-5000-16000-40", 5_000, 16_000, 40),
            ("e2e-quadB-5000-15500-500", 5_000, 15_500, 500),
        ];
        let mut per_unit = Vec::new();
        for (name, a, p, m) in shapes {
            let bytes = quad_shape(a, p, m);
            let units = (m * (a + p)) as f64;
            // Grouped: minimum of five runs.
            let mut after = u128::MAX;
            let mut marked = 0;
            for _ in 0..5 {
                let (nanos, n) = time_marks(&bytes, false);
                after = after.min(nanos);
                marked = n;
            }
            let (before, before_marked) = time_marks(&bytes, true);
            assert_eq!(marked, before_marked);
            assert_eq!(marked, (m * a) as u64, "every accessor's vertex 0 is drawn");
            let (b, f) = (before as f64 / units, after as f64 / units);
            eprintln!(
                "mark-linearity {name}: A={a} P={p} instances={m} units={units} before={:.3}s ({b:.2} ns/unit) after={:.4}s ({f:.2} ns/unit) speedup={:.1}x",
                before as f64 / 1e9,
                after as f64 / 1e9,
                b / f
            );
            per_unit.push((name, b, f));
        }
        let shared = per_unit[0].2;
        for (name, before, after) in &per_unit[1..] {
            assert!(
                *after <= 4.0 * shared,
                "{name}: {after:.2} ns/unit against shared {shared:.2}"
            );
            assert!(*before >= 10.0 * after, "{name}: scan not slower");
        }
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
