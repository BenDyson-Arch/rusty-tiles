use super::raw::{self, Arena, Field, Raw, Span};
use super::{
    add, align, exact, invalid, limit, mul, unsupported, CodecError, CompressionReceipt, Result,
    ValidatedCompressionLimits, Working,
};
use crate::content_integrity::{json, meshopt};
use serde_json::value::RawValue;
use std::mem::size_of;
#[derive(Clone, Copy, PartialEq)]
#[repr(u8)]
pub(super) enum Known {
    Features,
    Metadata,
    Polygon,
    Restart,
    Quantization,
    Unlit,
    Meshopt,
}
pub(super) fn known(name: &str) -> Result<Known> {
    Ok(match name {
        "EXT_mesh_features" => Known::Features,
        "EXT_structural_metadata" => Known::Metadata,
        "EXT_mesh_polygon" => Known::Polygon,
        "KHR_mesh_primitive_restart" => Known::Restart,
        "KHR_mesh_quantization" => Known::Quantization,
        "KHR_materials_unlit" => Known::Unlit,
        "EXT_meshopt_compression" => Known::Meshopt,
        _ => return Err(unsupported("unknown actual extension or declaration")),
    })
}
#[derive(Clone, Copy)]
pub(super) struct Declaration<'a> {
    pub raw: &'a RawValue,
    pub kind: Known,
}
pub(super) struct Declarations<'a> {
    pub used: [Option<Declaration<'a>>; 7],
    pub required: [Option<Declaration<'a>>; 7],
    pub used_len: usize,
    pub required_len: usize,
}
fn declarations<'a>(raw: Raw<'a>) -> Result<([Option<Declaration<'a>>; 7], usize)> {
    let mut refs = [None; 7];
    let mut n = 0;
    if raw.is_some() {
        raw::each(raw, |r| {
            if n == 7 {
                return Err(unsupported("too many extension declarations"));
            }
            raw::shape(Some(r), b'"')?;
            refs[n] = Some(r);
            n += 1;
            Ok(())
        })?;
    }
    let mut out = [None; 7];
    let mut bits = 0u8;
    for i in 0..n {
        let r = refs[i].unwrap();
        let kind = raw::text(Some(r), known)??;
        let bit = 1u8 << (kind as u8);
        if bits & bit != 0 {
            return Err(invalid("duplicate extension declaration"));
        }
        bits |= bit;
        out[i] = Some(Declaration { raw: r, kind });
    }
    Ok((out, n))
}
#[derive(Clone, Copy)]
pub(super) struct Range {
    pub offset: usize,
    pub len: usize,
}
#[derive(Clone, Copy)]
pub(super) enum Agreement {
    Absent,
    Agreed(usize),
    Conflict,
}
#[derive(Clone, Copy)]
pub(super) enum Action {
    Raw,
    Encode {
        stride: usize,
        count: usize,
        bound: usize,
    },
    Existing {
        encoded: Range,
        stride: usize,
        count: usize,
    },
}
pub(super) struct View {
    pub fields: Span,
    pub extensions: Option<Span>,
    pub source: Range,
    pub parent_buffer: usize,
    pub parent_stride: Option<usize>,
    pub agreement: Agreement,
    pub raw_role: bool,
    pub action: Action,
}
#[derive(Clone, Copy)]
enum Context {
    Core,
    Accessor,
    Primitive,
    Attributes,
    Image,
    Metadata,
    Table,
    Properties,
    Property,
    Features,
    FeatureId,
    Polygon,
    Extensions,
    Texture,
    TextureInfo,
    Node,
    Scene,
    Skin,
    AnimationSampler,
    AnimationTarget,
}
#[derive(Clone, Copy)]
struct Task<'a> {
    raw: &'a RawValue,
    context: Context,
    index: usize,
}
#[derive(Clone, Copy)]
struct Accessor {
    unsigned_scalar: bool,
}
#[derive(Clone, Copy, PartialEq)]
enum Shape {
    Scalar,
    Vec2,
    Vec3,
    Vec4,
    Mat2,
    Mat3,
    Mat4,
}
struct Counts {
    nodes: usize,
    meshes: usize,
    images: usize,
    textures: usize,
    samplers: usize,
    materials: usize,
    skins: usize,
    cameras: usize,
    scenes: usize,
    tables: usize,
}
pub(crate) struct Plan<'a> {
    pub(super) source: &'a [u8],
    pub(super) bin: &'a [u8],
    pub(super) arena: Arena<'a>,
    pub(super) root: Span,
    pub(super) buffers: [Span; 2],
    pub(super) views: Vec<View>,
    pub(super) declarations: Declarations<'a>,
    pub(super) metadata: bool,
    pub(super) identity: bool,
    pub(super) fallback_bytes: usize,
    pub(super) bin_bound: usize,
    pub(super) json_bound: usize,
    pub(super) limits: ValidatedCompressionLimits,
    pub(super) working: Working,
    pub(super) receipt: CompressionReceipt,
}
fn bytes(source: &[u8], r: Range) -> Result<&[u8]> {
    let end = add(r.offset, r.len).map_err(|_| invalid("source byte range overflow"))?;
    source
        .get(r.offset..end)
        .ok_or_else(|| invalid("source byte range outside actual buffer"))
}
fn word(source: &[u8], o: usize) -> Result<usize> {
    Ok(u32::from_le_bytes(
        bytes(source, Range { offset: o, len: 4 })?
            .try_into()
            .unwrap(),
    ) as usize)
}
fn envelope(source: &[u8]) -> Result<(&[u8], &[u8], usize)> {
    if source.len() < 20
        || &source[..4] != b"glTF"
        || word(source, 4)? != 2
        || word(source, 8)? != source.len()
    {
        return Err(invalid("invalid GLB v2 envelope"));
    }
    let j = word(source, 12)?;
    if j % 4 != 0 || word(source, 16)? != 0x4e4f534a {
        return Err(invalid("invalid first JSON chunk"));
    }
    let json = bytes(source, Range { offset: 20, len: j })?;
    let h = add(20, j)?;
    if h == source.len() {
        return Err(unsupported("GLB requires embedded BIN chunk"));
    }
    let n = word(source, h)?;
    if n % 4 != 0 {
        return Err(invalid("unaligned BIN chunk"));
    }
    let kind = word(source, h + 4)?;
    if kind == 0x4e4f534a {
        return Err(invalid("duplicate JSON chunk"));
    }
    if kind != 0x004e4942 {
        return Err(unsupported("GLB requires one second BIN chunk"));
    }
    let origin = add(h, 8)?;
    let bin = bytes(
        source,
        Range {
            offset: origin,
            len: n,
        },
    )?;
    if add(origin, n)? != source.len() {
        return Err(unsupported("extra GLB chunks"));
    }
    Ok((json, bin, origin))
}
fn idx(raw: Raw<'_>, n: usize) -> Result<usize> {
    let i = raw::uint(raw)?;
    if i >= n {
        return Err(invalid("consumed reference outside domain"));
    }
    Ok(i)
}
fn cap(n: usize, max: usize) -> Result<()> {
    if n > max {
        Err(limit("compression cardinality/logical limit"))
    } else {
        Ok(())
    }
}
fn enqueue<'a>(
    queue: &mut Vec<Task<'a>>,
    raw: &'a RawValue,
    context: Context,
    index: usize,
) -> Result<()> {
    if queue.len() == queue.capacity() {
        return Err(limit("core scan slots exhausted"));
    }
    queue.push(Task {
        raw,
        context,
        index,
    });
    Ok(())
}
fn array<'a>(queue: &mut Vec<Task<'a>>, raw: Raw<'a>, context: Context) -> Result<usize> {
    let mut i = 0;
    if raw.is_none() {
        return Ok(0);
    }
    raw::each(raw, |r| {
        enqueue(queue, r, context, i)?;
        i += 1;
        Ok(())
    })
}
fn array_refs(raw: Raw<'_>, n: usize) -> Result<()> {
    if raw.is_some() {
        raw::each(raw, |r| {
            idx(Some(r), n)?;
            Ok(())
        })?;
    }
    Ok(())
}
fn view_roles(views: &mut [View], raw: Raw<'_>) -> Result<()> {
    if raw.is_some() {
        let i = idx(raw, views.len())?;
        views[i].raw_role = true;
    }
    Ok(())
}
fn extension_fields<'a>(arena: &mut Arena<'a>, raw: Raw<'a>) -> Result<Option<Span>> {
    if raw.is_none() {
        return Ok(None);
    }
    let s = arena.object(raw)?;
    for f in arena.fields(s) {
        known(f.key.as_ref())?;
        raw::shape(Some(f.value), b'{')?;
    }
    Ok(Some(s))
}
fn mark_extensions<'a>(
    queue: &mut Vec<Task<'a>>,
    arena: &Arena<'a>,
    span: Option<Span>,
    used: u8,
) -> Result<()> {
    if let Some(s) = span {
        for f in arena.fields(s) {
            let k = known(f.key.as_ref())?;
            if used & (1 << (k as u8)) == 0 {
                return Err(invalid("actual extension missing used declaration"));
            }
            match k {
                Known::Metadata => enqueue(queue, f.value, Context::Metadata, 0)?,
                Known::Features => enqueue(queue, f.value, Context::Features, 0)?,
                Known::Polygon => enqueue(queue, f.value, Context::Polygon, 0)?,
                Known::Meshopt => {
                    return Err(unsupported(
                        "meshopt extension outside admitted buffer/view",
                    ))
                }
                _ => enqueue(queue, f.value, Context::Core, 0)?,
            }
        }
    }
    Ok(())
}
fn shape(raw: Raw<'_>) -> Result<Shape> {
    raw::text(raw, |s| match s {
        "SCALAR" => Ok(Shape::Scalar),
        "VEC2" => Ok(Shape::Vec2),
        "VEC3" => Ok(Shape::Vec3),
        "VEC4" => Ok(Shape::Vec4),
        "MAT2" => Ok(Shape::Mat2),
        "MAT3" => Ok(Shape::Mat3),
        "MAT4" => Ok(Shape::Mat4),
        _ => Err(invalid("unknown accessor shape")),
    })?
}
fn component(c: usize) -> Result<usize> {
    match c {
        5120 | 5121 => Ok(1),
        5122 | 5123 => Ok(2),
        5125 | 5126 => Ok(4),
        _ => Err(invalid("unknown accessor component type")),
    }
}
fn dimensions(s: Shape, width: usize) -> Result<(usize, usize)> {
    let (cols, rows) = match s {
        Shape::Scalar => (1, 1),
        Shape::Vec2 => (1, 2),
        Shape::Vec3 => (1, 3),
        Shape::Vec4 => (1, 4),
        Shape::Mat2 => (2, 2),
        Shape::Mat3 => (3, 3),
        Shape::Mat4 => (4, 4),
    };
    let column = if cols > 1 {
        align(rows * width, 4)?
    } else {
        rows * width
    };
    Ok((cols * column, (cols - 1) * column + rows * width))
}
fn accessor(arena: &Arena<'_>, span: Span, views: &mut [View]) -> Result<Accessor> {
    if arena.get(span, "sparse").is_some() {
        return Err(unsupported("sparse accessor representation"));
    }
    let count = raw::uint(arena.get(span, "count"))?;
    if count == 0 {
        return Err(invalid("empty accessor"));
    }
    let c = raw::uint(arena.get(span, "componentType"))?;
    let width = component(c)?;
    let shape = shape(arena.get(span, "type"))?;
    let (natural, last) = dimensions(shape, width)?;
    let offset = raw::off(arena.get(span, "byteOffset"))?;
    if offset % width != 0 {
        return Err(invalid("accessor component offset alignment"));
    }
    if let Some(n) = arena.get(span, "normalized") {
        let normalized = raw::boolean(Some(n))?;
        if normalized && matches!(c, 5125 | 5126) {
            return Err(invalid("normalized accessor component type"));
        }
    }
    let view = arena
        .get(span, "bufferView")
        .map(|r| idx(Some(r), views.len()))
        .transpose()?;
    if let Some(i) = view {
        let v = &mut views[i];
        let stride = v.parent_stride.unwrap_or(natural);
        if stride < natural || !stride.is_multiple_of(width) {
            return Err(invalid("accessor stride/alignment"));
        }
        if offset > v.source.len
            || last > v.source.len - offset
            || (count - 1) > (v.source.len - offset - last) / stride
        {
            return Err(invalid("accessor window outside logical view"));
        }
        let absolute = add(v.source.offset, offset)?;
        if absolute % width != 0
            || matches!(shape, Shape::Mat2 | Shape::Mat3 | Shape::Mat4) && absolute % 4 != 0
        {
            return Err(invalid("absolute accessor component alignment"));
        }
        v.agreement = match v.agreement {
            Agreement::Absent => Agreement::Agreed(stride),
            Agreement::Agreed(s) if s == stride => Agreement::Agreed(s),
            _ => Agreement::Conflict,
        };
    } else if offset != 0 {
        return Err(invalid("viewless accessor byte offset"));
    }
    Ok(Accessor {
        unsigned_scalar: shape == Shape::Scalar && matches!(c, 5121 | 5123 | 5125),
    })
}
pub(super) fn bound(count: usize, stride: usize) -> Result<usize> {
    if count == 0 || stride == 0 || !stride.is_multiple_of(4) || stride > 256 {
        return Err(invalid("native ATTRIBUTES dimensions"));
    }
    let block = (8192 / stride / 16 * 16).min(256);
    let groups = add(block, 63)? / 64;
    let chunks = add(count, block - 1)? / block;
    let body = mul(mul(chunks, stride)?, add(add(stride / 4, groups)?, block)?)?;
    let b = add(add(1, body)?, (stride + stride / 4).max(32))?;
    if b > isize::MAX as usize {
        return Err(limit("native bound outside host range"));
    }
    Ok(b)
}
fn reference_accessor(raw: Raw<'_>, accessors: &[Accessor], indices: bool) -> Result<()> {
    let i = idx(raw, accessors.len())?;
    let a = accessors[i];
    if indices && !a.unsigned_scalar {
        return Err(invalid("index accessor requires unsigned SCALAR"));
    }
    Ok(())
}
fn scan<'a>(
    queue: &mut Vec<Task<'a>>,
    arena: &mut Arena<'a>,
    views: &mut [View],
    accessors: &mut [Accessor],
    counts: &mut Counts,
    used: u8,
) -> Result<()> {
    while let Some(t) = queue.pop() {
        let s = arena.object(Some(t.raw))?;
        match t.context {
            Context::Accessor => {
                accessors[t.index] = accessor(arena, s, views)?;
            }
            Context::Attributes => {
                for f in arena.fields(s) {
                    reference_accessor(Some(f.value), accessors, false)?;
                }
            }
            Context::Image => {
                if let Some(uri) = arena.get(s, "uri") {
                    raw::shape(Some(uri), b'"')?;
                    return Err(unsupported("external image URI"));
                }
                view_roles(views, arena.get(s, "bufferView"))?;
            }
            Context::Property => {
                for key in ["values", "arrayOffsets", "stringOffsets"] {
                    view_roles(views, arena.get(s, key))?;
                }
            }
            Context::Metadata => {
                if let Some(uri) = arena.get(s, "schemaUri") {
                    raw::shape(Some(uri), b'"')?;
                    return Err(unsupported("external metadata schema URI"));
                }
                counts.tables = arena
                    .get(s, "propertyTables")
                    .map(|r| raw::count(Some(r)))
                    .transpose()?
                    .unwrap_or(0);
            }
            Context::FeatureId => {
                if let Some(r) = arena.get(s, "propertyTable") {
                    idx(Some(r), counts.tables)?;
                }
                if let Some(r) = arena.get(s, "featureCount") {
                    raw::uint(Some(r))?;
                }
            }
            Context::Polygon => {
                for key in ["indicesOffsets", "loopIndices", "loopIndicesOffsets"] {
                    reference_accessor(arena.get(s, key), accessors, true)?;
                }
                raw::uint(arena.get(s, "count"))?;
            }
            Context::Primitive => {
                if let Some(r) = arena.get(s, "indices") {
                    reference_accessor(Some(r), accessors, true)?;
                }
                if let Some(r) = arena.get(s, "material") {
                    idx(Some(r), counts.materials)?;
                }
                if let Some(r) = arena.get(s, "mode") {
                    if raw::uint(Some(r))? > 6 {
                        return Err(invalid("primitive mode outside domain"));
                    }
                }
            }
            Context::Texture => {
                if let Some(r) = arena.get(s, "source") {
                    idx(Some(r), counts.images)?;
                }
                if let Some(r) = arena.get(s, "sampler") {
                    idx(Some(r), counts.samplers)?;
                }
            }
            Context::TextureInfo => {
                idx(arena.get(s, "index"), counts.textures)?;
            }
            Context::Node => {
                for (key, n) in [
                    ("mesh", counts.meshes),
                    ("skin", counts.skins),
                    ("camera", counts.cameras),
                ] {
                    if let Some(r) = arena.get(s, key) {
                        idx(Some(r), n)?;
                    }
                }
                array_refs(arena.get(s, "children"), counts.nodes)?;
            }
            Context::Scene => {
                array_refs(arena.get(s, "nodes"), counts.nodes)?;
            }
            Context::Skin => {
                array_refs(arena.get(s, "joints"), counts.nodes)?;
                if let Some(r) = arena.get(s, "skeleton") {
                    idx(Some(r), counts.nodes)?;
                }
                if let Some(r) = arena.get(s, "inverseBindMatrices") {
                    reference_accessor(Some(r), accessors, false)?;
                }
            }
            Context::AnimationSampler => {
                reference_accessor(arena.get(s, "input"), accessors, false)?;
                reference_accessor(arena.get(s, "output"), accessors, false)?;
            }
            Context::AnimationTarget => {
                if let Some(r) = arena.get(s, "node") {
                    idx(Some(r), counts.nodes)?;
                }
            }
            _ => {}
        }
        // Every enqueued object is a source containment occurrence; references never enqueue targets.
        for f in arena.fields(s) {
            let key = f.key.as_ref();
            if !matches!(t.context, Context::Properties | Context::Attributes)
                && matches!(key, "extras" | "schema")
            {
                continue;
            }
            if matches!(t.context, Context::Attributes) {
                continue;
            }
            let child = match (t.context, key) {
                (Context::Properties, _) => Some(Context::Property),
                (Context::Metadata, "propertyTables") => {
                    array(queue, Some(f.value), Context::Table)?;
                    None
                }
                (Context::Metadata, "propertyTextures" | "propertyAttributes") => {
                    array(queue, Some(f.value), Context::Core)?;
                    None
                }
                (Context::Table, "properties") => Some(Context::Properties),
                (Context::Features, "featureIds") => {
                    array(queue, Some(f.value), Context::FeatureId)?;
                    None
                }
                (_, "extensions") => Some(Context::Extensions),
                (_, "primitives") => {
                    array(queue, Some(f.value), Context::Primitive)?;
                    None
                }
                (Context::Primitive, "attributes") => Some(Context::Attributes),
                (Context::Primitive, "targets") => {
                    array(queue, Some(f.value), Context::Attributes)?;
                    None
                }
                (_, "pbrMetallicRoughness" | "perspective" | "orthographic") => Some(Context::Core),
                (
                    _,
                    "baseColorTexture"
                    | "metallicRoughnessTexture"
                    | "normalTexture"
                    | "occlusionTexture"
                    | "emissiveTexture",
                ) => Some(Context::TextureInfo),
                (_, "channels") => {
                    array(queue, Some(f.value), Context::Core)?;
                    None
                }
                (_, "samplers") => {
                    array(queue, Some(f.value), Context::AnimationSampler)?;
                    None
                }
                (_, "target") => Some(Context::AnimationTarget),
                _ => None,
            };
            if let Some(context) = child {
                if matches!(context, Context::Extensions) {
                    raw::shape(Some(f.value), b'{')?;
                }
                enqueue(queue, f.value, context, 0)?;
            }
        }
        if matches!(t.context, Context::Extensions) {
            for f in arena.fields(s) {
                let k = known(f.key.as_ref())?;
                if used & (1 << (k as u8)) == 0 {
                    return Err(invalid("actual extension missing declaration"));
                }
                let context = match k {
                    Known::Metadata => Context::Metadata,
                    Known::Features => Context::Features,
                    Known::Polygon => Context::Polygon,
                    Known::Meshopt => {
                        return Err(unsupported("meshopt at unsupported core position"))
                    }
                    _ => Context::Core,
                };
                enqueue(queue, f.value, context, 0)?;
            }
        }
    }
    Ok(())
}
pub(super) fn construction_storage(j: usize, k: usize, vu: usize, au: usize) -> Result<usize> {
    let bytes = add(
        add(
            add(
                add(
                    add(add(mul(j, 3)?, 8)?, mul(k, size_of::<Field>())?)?,
                    mul(k, size_of::<Task>())?,
                )?,
                mul(vu, size_of::<View>())?,
            )?,
            mul(au, size_of::<Accessor>())?,
        )?,
        mul(vu.max(2), size_of::<&RawValue>())?,
    )?;
    add(bytes, size_of::<Plan>())
}
pub(super) fn start_arena<'a>(
    j: usize,
    k: usize,
    vu: usize,
    au: usize,
    working: &mut Working,
) -> Result<Arena<'a>> {
    working.admit(construction_storage(j, k, vu, au)?)?;
    Arena::new(k)
}
#[cfg(test)]
pub(super) fn owner_sizes() -> [usize; 6] {
    [
        size_of::<Field>(),
        size_of::<Task>(),
        size_of::<View>(),
        size_of::<Accessor>(),
        size_of::<super::rewrite::Edit>(),
        size_of::<Plan>(),
    ]
}
pub(crate) fn prepare<'a, E>(
    source: &'a [u8],
    retained_working_bytes: usize,
    limits: &ValidatedCompressionLimits,
    checkpoint: &mut impl FnMut() -> std::result::Result<(), E>,
) -> std::result::Result<Plan<'a>, CodecError<E>> {
    checkpoint().map_err(CodecError::Checkpoint)?;
    let l = limits.values;
    cap(source.len(), l.source_bytes)?;
    if retained_working_bytes < source.len() {
        return Err(invalid("working baseline omits captured source").into());
    }
    let (j, bin, bin_origin) = envelope(source)?;
    cap(j.len(), l.json_bytes)?;
    let mut working = Working::new(retained_working_bytes, limits)?;
    working.admit(add(
        add(mul(j.len(), 8)?, mul(j.len().min(l.json_value_nodes), 256)?)?,
        65_536,
    )?)?;
    let checked = json::admit(j, limits.json())?;
    checkpoint().map_err(CodecError::Checkpoint)?;
    let k = checked.value_nodes();
    let vu = k.min(l.buffer_views);
    let au = k.min(l.accessors);
    let mut arena = start_arena(j.len(), k, vu, au, &mut working)?;
    let root = arena.object(Some(checked.raw()))?;
    let (used, used_len) = declarations(arena.get(root, "extensionsUsed"))?;
    let (required, required_len) = declarations(arena.get(root, "extensionsRequired"))?;
    let declarations = Declarations {
        used,
        required,
        used_len,
        required_len,
    };
    let used_bits = declarations.used[..used_len]
        .iter()
        .fold(0u8, |b, d| b | 1 << (d.unwrap().kind as u8));
    for d in &declarations.required[..required_len] {
        if used_bits & (1 << (d.unwrap().kind as u8)) == 0 {
            return Err(invalid("required extension missing used declaration").into());
        }
    }
    let asset_raw = arena.get(root, "asset");
    let asset = arena.object(asset_raw)?;
    if !raw::is(arena.get(asset, "version"), "2.0")? {
        return Err(unsupported("glTF version other than 2.0").into());
    }
    if let Some(r) = arena.get(asset, "minVersion") {
        raw::text(Some(r), |version| {
            let Some((major, minor)) = version.split_once('.') else {
                return Err(invalid("minimum version syntax"));
            };
            if major.is_empty()
                || minor.is_empty()
                || !major.bytes().all(|b| b.is_ascii_digit())
                || !minor.bytes().all(|b| b.is_ascii_digit())
            {
                return Err(invalid("minimum version syntax"));
            }
            let major = major.trim_start_matches('0');
            let minor = minor.trim_start_matches('0');
            if major.is_empty() || major == "1" || major == "2" && minor.is_empty() {
                Ok(())
            } else {
                Err(unsupported("minimum version exceeds glTF 2.0"))
            }
        })??;
    }
    let buffer_raw = arena.get(root, "buffers");
    let bn = raw::count(buffer_raw)?;
    if !(1..=2).contains(&bn) {
        return Err(unsupported("embedded buffer profile").into());
    }
    let refs = raw::refs(buffer_raw, bn)?;
    let buffer_values = [refs.first().copied(), refs.get(1).copied()];
    drop(refs);
    let mut buffers = [Span::default(); 2];
    let mut lengths = [0usize; 2];
    let mut buffer_extensions = [None; 2];
    for i in 0..bn {
        buffers[i] = arena.object(buffer_values[i])?;
        if let Some(uri) = arena.get(buffers[i], "uri") {
            raw::shape(Some(uri), b'"')?;
            return Err(unsupported("buffer URI source").into());
        }
        lengths[i] = raw::uint(arena.get(buffers[i], "byteLength"))?;
        let extension_raw = arena.get(buffers[i], "extensions");
        buffer_extensions[i] = extension_fields(&mut arena, extension_raw)?;
    }
    if lengths[0] > bin.len() {
        return Err(invalid("declared embedded buffer exceeds BIN").into());
    }
    let view_raw = arena.get(root, "bufferViews");
    let v = if view_raw.is_some() {
        raw::count(view_raw)?
    } else {
        0
    };
    cap(v, l.buffer_views)?;
    let refs = if v > 0 {
        raw::refs(view_raw, v)?
    } else {
        Vec::new()
    };
    let mut views = exact(v)?;
    let mut logical = 0usize;
    let mut existing = 0usize;
    for r in refs {
        let fields = arena.object(Some(r))?;
        let buffer = idx(arena.get(fields, "buffer"), bn)?;
        let range = Range {
            offset: raw::off(arena.get(fields, "byteOffset"))?,
            len: raw::uint(arena.get(fields, "byteLength"))?,
        };
        if range.len == 0
            || range
                .offset
                .checked_add(range.len)
                .is_none_or(|end| end > lengths[buffer])
        {
            return Err(invalid("view range outside declared buffer").into());
        }
        logical = add(logical, range.len)?;
        cap(logical, l.logical_bytes)?;
        if let Some(r) = arena.get(fields, "target") {
            if !matches!(raw::uint(Some(r))?, 34962 | 34963) {
                return Err(invalid("buffer view target domain").into());
            }
        }
        let parent_stride = arena
            .get(fields, "byteStride")
            .map(|r| raw::uint(Some(r)))
            .transpose()?;
        if parent_stride.is_some_and(|s| !(4..=252).contains(&s) || !s.is_multiple_of(4)) {
            return Err(invalid("core view stride domain").into());
        }
        let extension_raw = arena.get(fields, "extensions");
        let ext = extension_fields(&mut arena, extension_raw)?;
        let mesh = ext.and_then(|s| arena.get(s, "EXT_meshopt_compression"));
        let action = if let Some(raw) = mesh {
            existing += 1;
            let e = arena.object(Some(raw))?;
            if idx(arena.get(e, "buffer"), bn)? != 0 {
                return Err(unsupported("canonical meshopt source buffer").into());
            }
            let encoded = Range {
                offset: raw::off(arena.get(e, "byteOffset"))?,
                len: raw::uint(arena.get(e, "byteLength"))?,
            };
            bytes(&bin[..lengths[0]], encoded)?;
            let count = raw::uint(arena.get(e, "count"))?;
            let stride = raw::uint(arena.get(e, "byteStride"))?;
            if count == 0
                || stride == 0
                || stride > 256
                || stride % 4 != 0
                || !range.len.is_multiple_of(stride)
                || count != range.len / stride
                || parent_stride.is_some_and(|s| s != stride)
            {
                return Err(invalid("compressed parent/count/stride agreement").into());
            }
            if !raw::is(arena.get(e, "mode"), "ATTRIBUTES")?
                || arena
                    .get(e, "filter")
                    .map(|r| raw::is(Some(r), "NONE"))
                    .transpose()?
                    .is_some_and(|b| !b)
            {
                return Err(unsupported("meshopt mode/filter outside canonical identity").into());
            }
            Action::Existing {
                encoded,
                stride,
                count,
            }
        } else {
            if buffer != 0 {
                return Err(invalid("uncovered virtual fallback view").into());
            }
            bytes(&bin[..lengths[0]], range)?;
            Action::Raw
        };
        views.push(View {
            fields,
            extensions: ext,
            source: range,
            parent_buffer: buffer,
            parent_stride,
            agreement: Agreement::Absent,
            raw_role: false,
            action,
        });
    }
    let a_raw = arena.get(root, "accessors");
    let a = if a_raw.is_some() {
        raw::count(a_raw)?
    } else {
        0
    };
    cap(a, l.accessors)?;
    let mut accessors = exact(a)?;
    accessors.resize(
        a,
        Accessor {
            unsigned_scalar: false,
        },
    );
    let mut queue = exact(k)?;
    let mut counts = Counts {
        nodes: 0,
        meshes: 0,
        images: 0,
        textures: 0,
        samplers: 0,
        materials: 0,
        skins: 0,
        cameras: 0,
        scenes: 0,
        tables: 0,
    };
    for (key, ctx) in [
        ("nodes", Context::Node),
        ("meshes", Context::Core),
        ("images", Context::Image),
        ("textures", Context::Texture),
        ("samplers", Context::Core),
        ("materials", Context::Core),
        ("skins", Context::Skin),
        ("cameras", Context::Core),
        ("scenes", Context::Scene),
        ("animations", Context::Core),
    ] {
        let n = array(&mut queue, arena.get(root, key), ctx)?;
        match key {
            "nodes" => counts.nodes = n,
            "meshes" => counts.meshes = n,
            "images" => counts.images = n,
            "textures" => counts.textures = n,
            "samplers" => counts.samplers = n,
            "materials" => counts.materials = n,
            "skins" => counts.skins = n,
            "cameras" => counts.cameras = n,
            "scenes" => counts.scenes = n,
            _ => {}
        }
    }
    if let Some(r) = arena.get(root, "scene") {
        idx(Some(r), counts.scenes)?;
    }
    let extension_raw = arena.get(root, "extensions");
    let root_ext = extension_fields(&mut arena, extension_raw)?;
    let metadata = root_ext.is_some_and(|s| arena.get(s, "EXT_structural_metadata").is_some());
    mark_extensions(&mut queue, &arena, root_ext, used_bits)?;
    let asset_extension = arena.get(asset, "extensions");
    if let Some(r) = asset_extension {
        enqueue(&mut queue, r, Context::Extensions, 0)?;
    }
    for span in buffer_extensions
        .into_iter()
        .flatten()
        .chain(views.iter().filter_map(|v| v.extensions))
    {
        for f in arena.fields(span) {
            let kind = known(f.key.as_ref())?;
            if used_bits & (1 << (kind as u8)) == 0 {
                return Err(invalid("actual buffer/view extension missing declaration").into());
            }
            if kind != Known::Meshopt {
                let ctx = match kind {
                    Known::Metadata => Context::Metadata,
                    Known::Features => Context::Features,
                    Known::Polygon => Context::Polygon,
                    _ => Context::Core,
                };
                enqueue(&mut queue, f.value, ctx, 0)?;
            }
        }
    }
    array(&mut queue, a_raw, Context::Accessor)?;
    scan(
        &mut queue,
        &mut arena,
        &mut views,
        &mut accessors,
        &mut counts,
        used_bits,
    )?;
    #[cfg(test)]
    super::tests::scan_owners(queue.capacity(), accessors.capacity());
    drop(queue);
    drop(accessors);
    #[cfg(test)]
    super::tests::scan_dropped();
    if metadata && (20 + j.len()) % 8 != 0
        || metadata && bin_origin % 8 != 0
        || metadata && bin.len() % 8 != 0
    {
        return Err(invalid("metadata GLB requires absolute eight-byte framing").into());
    }
    let slack = bin.len() - lengths[0];
    if slack > 3 || bin[lengths[0]..].iter().any(|b| *b != 0) {
        return Err(invalid("BIN padding outside selected profile").into());
    }
    if existing > 0 {
        if bn != 2
            || used_bits & (1 << (Known::Meshopt as u8)) == 0
            || !declarations.required[..required_len]
                .iter()
                .any(|d| d.unwrap().kind == Known::Meshopt)
        {
            return Err(unsupported("canonical compressed identity declarations/buffers").into());
        }
        let fallback = buffer_extensions[1]
            .and_then(|s| arena.get(s, "EXT_meshopt_compression"))
            .ok_or_else(|| invalid("missing virtual fallback marker"))?;
        let fs = arena.object(Some(fallback))?;
        if !raw::boolean(arena.get(fs, "fallback"))? {
            return Err(invalid("invalid virtual fallback marker").into());
        }
        for view in &views {
            if matches!(view.action, Action::Existing { .. }) && view.parent_buffer != 1 {
                return Err(unsupported("canonical compressed parent buffer").into());
            }
        }
    } else if bn != 1 {
        return Err(unsupported("uncompressed embedded buffer profile").into());
    }
    let mut fallback_bytes = 0usize;
    let mut bin_bound = 0usize;
    let mut compressed = 0usize;
    for view in &mut views {
        if existing == 0 && !view.raw_role {
            if let Agreement::Agreed(stride) = view.agreement {
                if stride > 0 && stride % 4 == 0 && stride <= 256 && view.source.len % stride == 0 {
                    let count = view.source.len / stride;
                    view.action = Action::Encode {
                        stride,
                        count,
                        bound: bound(count, stride)?,
                    };
                    compressed += 1;
                }
            }
        }
        fallback_bytes = align(fallback_bytes, 8)?;
        fallback_bytes = add(fallback_bytes, view.source.len)?;
        bin_bound = align(bin_bound, 8)?;
        bin_bound = add(
            bin_bound,
            match view.action {
                Action::Encode { bound, .. } => bound,
                _ => view.source.len,
            },
        )?;
    }
    let identity = existing > 0 || compressed == 0;
    if !identity {
        let mut output_nodes = add(k, 5)?;
        for view in &views {
            if matches!(view.action, Action::Encode { .. }) {
                output_nodes = add(output_nodes, 8 + usize::from(view.extensions.is_none()))?;
            }
            if arena.get(view.fields, "byteOffset").is_none() {
                output_nodes = add(output_nodes, 1)?;
            }
        }
        for (key, names) in [
            ("extensionsUsed", &declarations.used[..used_len]),
            ("extensionsRequired", &declarations.required[..required_len]),
        ] {
            if !names.iter().any(|d| d.unwrap().kind == Known::Meshopt) {
                output_nodes = add(
                    output_nodes,
                    1 + usize::from(arena.get(root, key).is_none()),
                )?;
            }
        }
        cap(output_nodes, l.json_value_nodes)?;
        if l.json_depth < 5 {
            return Err(limit("rewritten JSON depth limit").into());
        }
    }
    let json_bound = add(add(j.len(), mul(v, 512)?)?, 1024)?;
    let retained = add(
        add(
            mul(arena.fields.capacity(), size_of::<Field>())?,
            mul(views.capacity(), size_of::<View>())?,
        )?,
        arena.owned_keys(),
    )?;
    if identity {
        cap(source.len(), l.candidate_bytes)?;
        for view in &views {
            if let Action::Existing {
                encoded,
                stride,
                count,
            } = view.action
            {
                checkpoint().map_err(CodecError::Checkpoint)?;
                working.admit(add(
                    add(retained, size_of::<Plan>())?,
                    meshopt::scratch(view.source.len)?,
                )?)?;
                let decoded = meshopt::decode(
                    bytes(&bin[..lengths[0]], encoded)?,
                    count,
                    stride,
                    meshopt::Mode::Attributes,
                )?;
                drop(decoded);
            }
        }
        working.admit(source.len())?;
    } else {
        let (_, _, upper) = super::framing::size(json_bound, bin_bound, metadata)?;
        cap(upper, l.candidate_bytes)?;
        let edits = mul(v, size_of::<super::rewrite::Edit>())?;
        working.admit(add(
            add(add(add(retained, size_of::<Plan>())?, edits)?, bin_bound)?,
            upper,
        )?)?;
    }
    checkpoint().map_err(CodecError::Checkpoint)?;
    let receipt = CompressionReceipt {
        before_bytes: source.len(),
        views: v,
        raw_views: v - compressed - existing,
        compressed_views: compressed,
        existing_views: existing,
        accessors: a,
        logical_bytes: logical,
        estimated_peak_bytes: working.peak,
        ..CompressionReceipt::default()
    };
    Ok(Plan {
        source,
        bin: &bin[..lengths[0]],
        arena,
        root,
        buffers,
        views,
        declarations,
        metadata,
        identity,
        fallback_bytes,
        bin_bound,
        json_bound,
        limits: *limits,
        working,
        receipt,
    })
}
