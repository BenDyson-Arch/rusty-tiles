//! Finite glTF document/resource admission and decoding from borrowed snapshots.
//! No filesystem, jobs, source dependency discovery, or legacy mesh IR.
use crate::{JobError, JobErrorKind};
use serde_json::{Map, Value};
use std::ops::Range;
mod json;
mod material;
mod original_bounds;
mod texture;
pub(crate) use material::Material;

pub(crate) const MAX_SOURCE_BYTES: usize = 32 * 1024 * 1024;
pub(crate) const MAX_TRIANGLES: usize = 100_000;
pub(crate) const MAX_LEAVES: usize = 4096;
const MAX_JSON_BYTES: usize = 1024 * 1024;
/// Local-metre source and transformed position domain shared by both admission profiles.
const MAX_LOCAL_COORDINATE: f64 = 1_000_000.;

/// Explicit admission ceilings of one document profile. `None` means no total ceiling: the
/// streaming large-mesh profile bounds memory by configuration, not by input size (D12), and
/// keeps only explicit JSON count caps (design v6.1 §2.5).
#[derive(Clone, Copy, Debug)]
pub(crate) struct Limits {
    /// Message prefix of ceilings that name the profile.
    pub label: &'static str,
    pub root_bytes: Option<usize>,
    pub json_bytes: usize,
    pub declared_buffer_bytes: Option<usize>,
    pub buffers: usize,
    pub buffer_views: usize,
    pub accessors: usize,
    pub accessor_elements: Option<usize>,
    pub meshes: usize,
    pub primitives: usize,
    pub nodes: usize,
    pub materials: usize,
    pub images: usize,
    pub triangles: Option<usize>,
}

impl Limits {
    /// The bounded in-memory F1a/F1c2 profile.
    pub(crate) const F1A: Self = Self {
        label: "F1a",
        root_bytes: Some(MAX_SOURCE_BYTES),
        json_bytes: MAX_JSON_BYTES,
        declared_buffer_bytes: Some(MAX_SOURCE_BYTES),
        buffers: 32,
        buffer_views: usize::MAX,
        accessors: usize::MAX,
        accessor_elements: Some(1_000_000),
        meshes: usize::MAX,
        primitives: 4096,
        nodes: 4096,
        materials: 256,
        images: 32,
        triangles: Some(MAX_TRIANGLES),
    };
    /// The streaming large-mesh profile: JSON within `J` = 16 MiB and the explicit count caps
    /// of design v6.1 §2.5; no byte, element or triangle total.
    pub(crate) const LARGE: Self = Self {
        label: "large-mesh",
        root_bytes: None,
        json_bytes: 16 << 20,
        declared_buffer_bytes: None,
        buffers: 4096,
        buffer_views: 262_144,
        accessors: 262_144,
        accessor_elements: None,
        meshes: 65_536,
        primitives: 65_536,
        nodes: 65_536,
        materials: 4096,
        images: 4096,
        triangles: None,
    };
}
type Result<T> = std::result::Result<T, JobError>;
pub(crate) type Matrix = [[f64; 4]; 4];
struct Instance {
    node: usize,
    mesh: usize,
    world: Matrix,
    determinant: f64,
    normal_matrix: [[f64; 3]; 3],
}
const IDENTITY: Matrix = [
    [1., 0., 0., 0.],
    [0., 1., 0., 0.],
    [0., 0., 1., 0.],
    [0., 0., 0., 1.],
];

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct SourceIdentity {
    pub node_index: u32,
    pub mesh_index: u32,
    pub primitive_index: u32,
    pub triangle_index: u32,
}
impl SourceIdentity {
    pub fn primitive(self) -> (u32, u32, u32) {
        (self.node_index, self.mesh_index, self.primitive_index)
    }
}

#[derive(Clone, Debug)]
pub(crate) struct Triangle {
    pub source: SourceIdentity,
    /// Authored POSITION accessor keys, in the same reflected corner order.
    pub vertex_indices: [u32; 3],
    pub positions: [[f32; 3]; 3],
    pub normals: Option<[[f32; 3]; 3]>,
    pub tangents: Option<[[f32; 4]; 3]>,
    pub texcoords: [Option<[[f32; 2]; 3]>; 2],
    pub colors: Option<[[f32; 4]; 3]>,
    pub material: Option<usize>,
}

pub(crate) struct Image {
    pub bytes: Vec<u8>,
    pub mime_type: &'static str,
    pub extension: &'static str,
    pub width: u32,
    pub height: u32,
}

/// Tile-basis intervals enclosing unchanged authored source-node transforms.
pub(crate) struct OriginalBounds {
    pub min: [f64; 3],
    pub max: [f64; 3],
}

pub(crate) fn original_bounds(
    document: &Document,
    supplied_buffers: &[&[u8]],
    check: impl FnMut() -> Result<()>,
) -> Result<OriginalBounds> {
    original_bounds::evaluate(document, supplied_buffers, check)
}

pub(crate) struct Geometry {
    pub node_names: Vec<Option<String>>,
    pub triangles: Vec<Triangle>,
    pub materials: Vec<Material>,
    pub images: Vec<Image>,
    pub textures: Vec<Value>,
    pub samplers: Vec<Value>,
}

fn invalid(message: impl Into<String>) -> JobError {
    JobError::new(JobErrorKind::InvalidInput, message)
}
fn unsupported(message: impl Into<String>) -> JobError {
    JobError::new(JobErrorKind::Unsupported, message)
}
fn object<'a>(v: &'a Value, allowed: &[&str]) -> Result<&'a Map<String, Value>> {
    let o = v
        .as_object()
        .ok_or_else(|| invalid("expected GLB JSON object"))?;
    for key in o.keys() {
        if !allowed.contains(&key.as_str()) {
            return Err(unsupported(format!(
                "F1a does not support JSON field {key}"
            )));
        }
    }
    if o.get("name").is_some_and(|v| !v.is_string()) {
        return Err(invalid("object name must be a string"));
    }
    Ok(o)
}
fn array(v: &Value) -> Result<&[Value]> {
    v.as_array()
        .map(Vec::as_slice)
        .ok_or_else(|| invalid("expected GLB JSON array"))
}
fn list<'a>(v: &'a Value, key: &str) -> Result<&'a [Value]> {
    v.get(key).map_or(Ok(&[]), |v| {
        let values = array(v)?;
        if values.is_empty() {
            return Err(invalid(format!("present {key} array must not be empty")));
        }
        Ok(values)
    })
}
fn uint(v: &Value) -> Result<usize> {
    v.as_u64()
        .and_then(|n| usize::try_from(n).ok())
        .ok_or_else(|| invalid("expected nonnegative integer in host range"))
}
fn number(v: &Value) -> Result<f64> {
    v.as_f64()
        .filter(|n| n.is_finite())
        .ok_or_else(|| invalid("expected finite number"))
}
fn field(v: &Value, key: &str) -> Result<usize> {
    uint(&v[key])
}
fn offset(v: &Value, key: &str) -> Result<usize> {
    v.get(key).map_or(Ok(0), uint)
}
fn vector<const N: usize>(v: &Value) -> Result<[f64; N]> {
    let a = array(v)?;
    if a.len() != N {
        return Err(invalid("incorrect numeric vector length"));
    }
    let mut out = [0.; N];
    for (out, v) in out.iter_mut().zip(a) {
        *out = number(v)?;
    }
    Ok(out)
}
fn reference<'a>(values: &'a [Value], v: &Value) -> Result<&'a Value> {
    values
        .get(uint(v)?)
        .ok_or_else(|| invalid("GLB reference out of range"))
}

/// GLB v2 container layout: the JSON chunk and the optional BIN chunk as byte ranges.
pub(crate) struct GlbLayout {
    pub json: Range<u64>,
    pub bin: Option<Range<u64>>,
}

fn word(read_at: &mut dyn FnMut(u64, &mut [u8]) -> Result<()>, at: u64) -> Result<u32> {
    let mut bytes = [0; 4];
    read_at(at, &mut bytes)?;
    Ok(u32::from_le_bytes(bytes))
}

/// Reads the GLB header and chunk headers of a `len`-byte root through `read_at`, which fails
/// with "truncated GLB envelope" outside the root. One implementation serves captured roots and
/// streamed files, so both apply the same header, alignment and trailing-chunk rules.
pub(crate) fn glb_layout(
    len: u64,
    limits: &Limits,
    read_at: &mut dyn FnMut(u64, &mut [u8]) -> Result<()>,
) -> Result<GlbLayout> {
    const JSON: u32 = 0x4e4f534a;
    const BIN: u32 = 0x004e4942;
    if let Some(max) = limits.root_bytes {
        if len > max as u64 {
            return Err(unsupported(format!(
                "source exceeds {} MiB {} ceiling",
                max >> 20,
                limits.label
            )));
        }
    }
    let mut magic = [0; 4];
    let magic_ok = len >= 4 && {
        read_at(0, &mut magic)?;
        &magic == b"glTF"
    };
    if !magic_ok || word(read_at, 4)? != 2 || u64::from(word(read_at, 8)?) != len {
        return Err(invalid("invalid GLB v2 header or file length"));
    }
    let json_len = u64::from(word(read_at, 12)?);
    if json_len > limits.json_bytes as u64 {
        return Err(unsupported(format!(
            "GLB JSON exceeds {} MiB {} ceiling",
            limits.json_bytes >> 20,
            limits.label
        )));
    }
    if !json_len.is_multiple_of(4) || word(read_at, 16)? != JSON {
        return Err(invalid("first GLB chunk must be aligned JSON"));
    }
    let json_end = 20 + json_len;
    if json_end > len {
        return Err(invalid("truncated JSON chunk"));
    }
    if json_end == len {
        return Ok(GlbLayout {
            json: 20..json_end,
            bin: None,
        });
    }
    let bin_len = u64::from(word(read_at, json_end)?);
    if word(read_at, json_end + 4)? != BIN {
        return Err(unsupported(format!(
            "{} requires one BIN chunk after JSON",
            limits.label
        )));
    }
    let bin_start = json_end + 8;
    let bin_end = bin_start + bin_len;
    if !bin_len.is_multiple_of(4) || bin_end > len {
        return Err(invalid("invalid BIN length"));
    }
    if bin_end < len {
        let mut cursor = bin_end;
        while cursor < len {
            let chunk = u64::from(word(read_at, cursor)?);
            let kind = word(read_at, cursor + 4)?;
            cursor = cursor + 8 + chunk;
            if !chunk.is_multiple_of(4) || cursor > len || [JSON, BIN].contains(&kind) {
                return Err(invalid("invalid extra GLB chunk"));
            }
        }
        return Err(unsupported(format!(
            "unknown GLB chunks outside {}",
            limits.label
        )));
    }
    Ok(GlbLayout {
        json: 20..json_end,
        bin: Some(bin_start..bin_end),
    })
}

fn envelope(bytes: &[u8]) -> Result<(&[u8], Option<Range<usize>>)> {
    let layout = glb_layout(bytes.len() as u64, &Limits::F1A, &mut |at, out| {
        let at = at as usize;
        out.copy_from_slice(
            bytes
                .get(at..at + out.len())
                .ok_or_else(|| invalid("truncated GLB envelope"))?,
        );
        Ok(())
    })?;
    let range = |r: Range<u64>| r.start as usize..r.end as usize;
    Ok((&bytes[range(layout.json)], layout.bin.map(range)))
}

#[derive(Clone, Copy)]
struct Accessor<'a> {
    bytes: &'a [u8],
    stride: usize,
    count: usize,
    component: usize,
    normalized: bool,
    width: usize,
}
/// Byte size of an admitted component type.
pub(crate) fn component_size(component: usize) -> usize {
    match component {
        5121 => 1,
        5123 => 2,
        _ => 4,
    }
}

/// Little-endian value of one admitted component at the start of `bytes`.
pub(crate) fn component_scalar(component: usize, bytes: &[u8]) -> f64 {
    match component {
        5121 => bytes[0] as f64,
        5123 => u16::from_le_bytes(bytes[..2].try_into().unwrap()) as f64,
        5125 => u32::from_le_bytes(bytes[..4].try_into().unwrap()) as f64,
        5126 => f32::from_le_bytes(bytes[..4].try_into().unwrap()) as f64,
        _ => unreachable!("admitted component type"),
    }
}

/// Observed per-component extrema of one accessor, compared with its declared `min`/`max`.
pub(crate) struct Extrema {
    min: [f64; 4],
    max: [f64; 4],
}
impl Extrema {
    pub(crate) fn new() -> Self {
        Self {
            min: [f64::INFINITY; 4],
            max: [f64::NEG_INFINITY; 4],
        }
    }
    pub(crate) fn observe(&mut self, c: usize, x: f64) -> Result<()> {
        if !x.is_finite() {
            return Err(invalid("nonfinite accessor data"));
        }
        self.min[c] = self.min[c].min(x);
        self.max[c] = self.max[c].max(x);
        Ok(())
    }
    pub(crate) fn verify(&self, view: &AccessorView<'_>) -> Result<()> {
        if view
            .min
            .is_some_and(|values| values != &self.min[..view.width])
            || view
                .max
                .is_some_and(|values| values != &self.max[..view.width])
        {
            return Err(invalid("accessor bounds disagree with actual extrema"));
        }
        Ok(())
    }
}

/// Source POSITION domain of local metres.
pub(crate) fn check_source_position(position: [f64; 3]) -> Result<()> {
    if position.iter().any(|v| v.abs() > MAX_LOCAL_COORDINATE) {
        return Err(unsupported("source position outside local mesh domain"));
    }
    Ok(())
}

/// A triangle index must address a vertex and must not be the primitive-restart value.
pub(crate) fn check_index(index: usize, vertices: usize, component: usize) -> Result<()> {
    let reserved = match component {
        5121 => u8::MAX as usize,
        5123 => u16::MAX as usize,
        _ => u32::MAX as usize,
    };
    if index >= vertices || index == reserved {
        return Err(invalid(
            "triangle index out of range or reserved restart value",
        ));
    }
    Ok(())
}

/// Bakes a source position through its accumulated node transform: evaluated in f64, rounded
/// once to f32, and kept inside the local-metre domain.
pub(crate) fn bake_position(world: &[[f64; 4]; 4], position: [f64; 3]) -> Result<[f32; 3]> {
    let mut baked = [0.; 3];
    for (r, out) in baked.iter_mut().enumerate() {
        let x = world[r][3] + (0..3).map(|c| world[r][c] * position[c]).sum::<f64>();
        if !x.is_finite() || x.abs() > MAX_LOCAL_COORDINATE {
            return Err(unsupported("transformed position outside local F1a domain"));
        }
        *out = x as f32;
    }
    Ok(baked)
}

impl Accessor<'_> {
    fn scalar(&self, i: usize, c: usize) -> f64 {
        let start = i * self.stride + c * component_size(self.component);
        component_scalar(self.component, &self.bytes[start..])
    }
    fn vec3(&self, i: usize) -> [f64; 3] {
        std::array::from_fn(|c| self.scalar(i, c))
    }
    fn value(&self, i: usize, c: usize) -> f64 {
        let divisor = if self.normalized {
            match self.component {
                5121 => u8::MAX as f64,
                5123 => u16::MAX as f64,
                _ => unreachable!("admitted normalized component"),
            }
        } else {
            1.
        };
        self.scalar(i, c) / divisor
    }
    fn decoded<const N: usize>(&self, i: usize) -> [f32; N] {
        std::array::from_fn(|c| self.value(i, c) as f32)
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum AccessorShape {
    Scalar,
    Vec2,
    Vec3,
    Vec4,
    Mat2,
    Mat3,
    Mat4,
}
impl AccessorShape {
    fn parse(value: &Value) -> Result<Self> {
        match value.as_str() {
            Some("SCALAR") => Ok(Self::Scalar),
            Some("VEC2") => Ok(Self::Vec2),
            Some("VEC3") => Ok(Self::Vec3),
            Some("VEC4") => Ok(Self::Vec4),
            Some("MAT2") => Ok(Self::Mat2),
            Some("MAT3") => Ok(Self::Mat3),
            Some("MAT4") => Ok(Self::Mat4),
            _ => Err(invalid("missing or invalid accessor type")),
        }
    }
    fn columns(self) -> Option<usize> {
        match self {
            Self::Mat2 => Some(2),
            Self::Mat3 => Some(3),
            Self::Mat4 => Some(4),
            _ => None,
        }
    }
    fn width(self) -> usize {
        match self {
            Self::Scalar => 1,
            Self::Vec2 => 2,
            Self::Vec3 => 3,
            Self::Vec4 | Self::Mat2 => 4,
            Self::Mat3 => 9,
            Self::Mat4 => 16,
        }
    }
    /// Matrix columns have four-byte alignment. The final column's trailing
    /// padding may be absent while successive elements retain the full stride.
    fn element_layout(self, component_size: usize) -> (usize, usize) {
        if let Some(columns) = self.columns() {
            let column_bytes = columns * component_size;
            let column_stride = (column_bytes + 3) & !3;
            (
                columns * column_stride,
                (columns - 1) * column_stride + column_bytes,
            )
        } else {
            let bytes = self.width() * component_size;
            (bytes, bytes)
        }
    }
}

/// Read-only layout of one admitted accessor, for streaming readers outside this module.
#[derive(Clone, Copy, Debug)]
pub(crate) struct AccessorView<'a> {
    pub index: usize,
    pub buffer: usize,
    /// Byte offset of element zero within its buffer.
    pub start: usize,
    pub stride: usize,
    pub count: usize,
    pub component: usize,
    pub width: usize,
    pub normalized: bool,
    pub min: Option<&'a [f64]>,
    pub max: Option<&'a [f64]>,
}
impl AccessorView<'_> {
    /// Bytes of one element's components (without stride padding).
    pub(crate) fn element_bytes(&self) -> usize {
        self.width * component_size(self.component)
    }
}

/// One mesh primitive instanced by one node of the selected scene, in scene traversal order.
pub(crate) struct ScenePrimitive<'a> {
    pub node: u32,
    pub mesh: u32,
    pub primitive: u32,
    pub world: &'a Matrix,
    /// Negative for a mirroring transform; corners 1 and 2 are then swapped.
    pub determinant: f64,
    pub position: AccessorView<'a>,
    pub indices: Option<AccessorView<'a>>,
    pub triangles: usize,
    pub has_normal: bool,
    pub has_tangent: bool,
    pub texcoord_sets: usize,
    pub has_color: bool,
}

/// Borrowed metadata needed to check an authored primitive through bounded readers.
pub(crate) struct PayloadPrimitive<'a> {
    pub position: AccessorView<'a>,
    pub indices: Option<AccessorView<'a>>,
    pub tangent: Option<AccessorView<'a>>,
    pub count: usize,
    primitive: &'a Primitive,
    accessors: &'a [AccessorLayout],
}
impl<'a> PayloadPrimitive<'a> {
    pub(crate) fn attributes(&self) -> impl Iterator<Item = (VertexRole, AccessorView<'a>)> + '_ {
        self.primitive
            .attributes()
            .map(|(role, i)| (role, self.accessors[i].view(i)))
    }
}

struct AccessorLayout {
    buffer: usize,
    start: usize,
    length: usize,
    stride: usize,
    count: usize,
    component: usize,
    normalized: bool,
    shape: AccessorShape,
    width: usize,
    min: Option<Vec<f64>>,
    max: Option<Vec<f64>>,
}

impl AccessorLayout {
    fn view(&self, index: usize) -> AccessorView<'_> {
        AccessorView {
            index,
            buffer: self.buffer,
            start: self.start,
            stride: self.stride,
            count: self.count,
            component: self.component,
            width: self.width,
            normalized: self.normalized,
            min: self.min.as_deref(),
            max: self.max.as_deref(),
        }
    }
}

fn accessor_layouts(
    doc: &Value,
    buffer_lengths: &[usize],
    limits: &Limits,
    check: &mut impl FnMut() -> Result<()>,
) -> Result<Vec<AccessorLayout>> {
    let views = list(doc, "bufferViews")?;
    if views.len() > limits.buffer_views || list(doc, "accessors")?.len() > limits.accessors {
        return Err(unsupported(format!(
            "bufferView/accessor count exceeds {}/{}",
            limits.buffer_views, limits.accessors
        )));
    }
    for view in views {
        check()?;
        object(
            view,
            &[
                "buffer",
                "byteOffset",
                "byteLength",
                "byteStride",
                "target",
                "name",
            ],
        )?;
        let start = offset(view, "byteOffset")?;
        let size = field(view, "byteLength")?;
        let buffer_length = *buffer_lengths
            .get(field(view, "buffer")?)
            .ok_or_else(|| invalid("bufferView buffer reference out of range"))?;
        if size == 0
            || start
                .checked_add(size)
                .is_none_or(|end| end > buffer_length)
        {
            return Err(invalid("bufferView range outside declared buffer"));
        }
        if let Some(stride) = view.get("byteStride") {
            let stride = uint(stride)?;
            if !(4..=252).contains(&stride) || !stride.is_multiple_of(4) {
                return Err(invalid("invalid bufferView stride"));
            }
        }
        if let Some(target) = view.get("target") {
            if ![34962, 34963].contains(&uint(target)?) {
                return Err(invalid("invalid bufferView target"));
            }
        }
    }
    let mut result = Vec::new();
    let mut total = 0usize;
    for value in list(doc, "accessors")? {
        check()?;
        object(
            value,
            &[
                "bufferView",
                "byteOffset",
                "componentType",
                "normalized",
                "count",
                "type",
                "min",
                "max",
                "name",
            ],
        )?;
        let count = field(value, "count")?;
        total = total
            .checked_add(count)
            .ok_or_else(|| unsupported("accessor count overflow"))?;
        if limits.accessor_elements.is_some_and(|max| total > max) {
            return Err(unsupported(format!(
                "{} accessor count ceiling exceeded",
                limits.label
            )));
        }
        if count == 0 {
            return Err(invalid("empty accessor"));
        }
        let normalized = value
            .get("normalized")
            .map(|v| {
                v.as_bool()
                    .ok_or_else(|| invalid("normalized must be boolean"))
            })
            .transpose()?
            .unwrap_or(false);
        let component = field(value, "componentType")?;
        let size = match component {
            5120 | 5121 => 1,
            5122 | 5123 => 2,
            5125 | 5126 => 4,
            _ => return Err(invalid("invalid accessor component type")),
        };
        if normalized && ![5120, 5121, 5122, 5123].contains(&component) {
            return Err(invalid(
                "only byte/short accessor components may be normalized",
            ));
        }
        let shape = AccessorShape::parse(&value["type"])?;
        let width = shape.width();
        let (element_stride, element_length) = shape.element_layout(size);
        let view = reference(views, &value["bufferView"])?;
        let relative = offset(value, "byteOffset")?;
        let start = offset(view, "byteOffset")?
            .checked_add(relative)
            .ok_or_else(|| invalid("accessor offset overflow"))?;
        let stride = view.get("byteStride").map_or(Ok(element_stride), uint)?;
        let length = (count - 1)
            .checked_mul(stride)
            .and_then(|n| n.checked_add(element_length))
            .ok_or_else(|| invalid("accessor range overflow"))?;
        if !relative.is_multiple_of(size)
            || !start.is_multiple_of(size)
            || (shape.columns().is_some() && !start.is_multiple_of(4))
            || stride < element_stride
            || relative
                .checked_add(length)
                .is_none_or(|end| end > field(view, "byteLength").unwrap_or(0))
        {
            return Err(invalid("accessor alignment/range/stride invalid"));
        }
        let bounds =
            [value.get("min"), value.get("max")].map(|bound| -> Result<Option<Vec<f64>>> {
                bound
                    .map(|v| {
                        let a = array(v)?;
                        if a.len() != width {
                            return Err(invalid("accessor bounds shape mismatch"));
                        }
                        a.iter()
                            .map(|value| {
                                let n = number(value)?;
                                if component == 5126 {
                                    let n = n as f32;
                                    if !n.is_finite() {
                                        return Err(invalid(
                                            "accessor bound outside component range",
                                        ));
                                    }
                                    Ok(f64::from(n))
                                } else {
                                    let (min, max) = match component {
                                        5120 => (i8::MIN as f64, i8::MAX as f64),
                                        5121 => (0., u8::MAX as f64),
                                        5122 => (i16::MIN as f64, i16::MAX as f64),
                                        5123 => (0., u16::MAX as f64),
                                        _ => (0., u32::MAX as f64),
                                    };
                                    if n.fract() != 0. || !(min..=max).contains(&n) {
                                        return Err(invalid(
                                            "accessor bound outside integer component range",
                                        ));
                                    }
                                    Ok(n)
                                }
                            })
                            .collect()
                    })
                    .transpose()
            });
        let [min, max] = bounds;
        let (min, max) = (min?, max?);
        if min
            .as_ref()
            .zip(max.as_ref())
            .is_some_and(|(min, max)| min.iter().zip(max).any(|(min, max)| min > max))
        {
            return Err(invalid("accessor minimum exceeds maximum"));
        }
        result.push(AccessorLayout {
            buffer: field(view, "buffer")?,
            start,
            length,
            stride,
            count,
            component,
            normalized,
            shape,
            width,
            min,
            max,
        });
    }
    Ok(result)
}

/// Run after primitive roles distinguish malformed core encodings from valid
/// generic accessors outside this producer's finite storage profile.
fn validate_accessor_profile(
    layouts: &[AccessorLayout],
    check: &mut impl FnMut() -> Result<()>,
) -> Result<()> {
    for layout in layouts {
        check()?;
        let supported = match layout.shape {
            AccessorShape::Scalar => {
                [5121, 5123, 5125].contains(&layout.component) && !layout.normalized
            }
            AccessorShape::Vec2 | AccessorShape::Vec3 | AccessorShape::Vec4 => {
                (layout.component == 5126 && !layout.normalized)
                    || ([5121, 5123].contains(&layout.component) && layout.normalized)
            }
            AccessorShape::Mat2 | AccessorShape::Mat3 | AccessorShape::Mat4 => false,
        };
        if !supported {
            return Err(unsupported(
                "accessor component/shape/normalization outside the supported mesh profile",
            ));
        }
    }
    Ok(())
}

fn decode_accessors<'a>(
    layouts: &[AccessorLayout],
    buffers: &[&'a [u8]],
    check: &mut impl FnMut() -> Result<()>,
) -> Result<Vec<Accessor<'a>>> {
    let mut result = Vec::with_capacity(layouts.len());
    for layout in layouts {
        check()?;
        let end = layout
            .start
            .checked_add(layout.length)
            .ok_or_else(|| invalid("accessor range overflow"))?;
        let bytes = buffers
            .get(layout.buffer)
            .and_then(|buffer| buffer.get(layout.start..end))
            .ok_or_else(|| invalid("accessor outside actual buffer"))?;
        let accessor = Accessor {
            bytes,
            stride: layout.stride,
            count: layout.count,
            component: layout.component,
            normalized: layout.normalized,
            width: layout.width,
        };
        let mut extrema = Extrema::new();
        for i in 0..layout.count {
            if i.is_multiple_of(1024) {
                check()?;
            }
            for c in 0..layout.width {
                extrema.observe(c, accessor.scalar(i, c))?;
            }
        }
        extrema.verify(&layout.view(usize::MAX))?;
        result.push(accessor);
    }
    Ok(result)
}

fn multiply(a: Matrix, b: Matrix) -> Matrix {
    std::array::from_fn(|r| std::array::from_fn(|c| (0..4).map(|k| a[r][k] * b[k][c]).sum()))
}
fn length(v: [f64; 3]) -> f64 {
    v.into_iter().map(|x| x * x).sum::<f64>().sqrt()
}
fn linear(m: Matrix) -> Result<(f64, [[f64; 3]; 3])> {
    if m.iter().flatten().any(|x| !x.is_finite()) {
        return Err(unsupported("nonfinite accumulated transform"));
    }
    let [[a, b, c, _], [d, e, f, _], [g, h, i, _], _] = m;
    let cof = [
        [e * i - f * h, f * g - d * i, d * h - e * g],
        [c * h - b * i, a * i - c * g, b * g - a * h],
        [b * f - c * e, c * d - a * f, a * e - b * d],
    ];
    let det: f64 = (0..3).map(|c| m[0][c] * cof[0][c]).sum();
    let scale: f64 = (0..3)
        .map(|c| length(std::array::from_fn(|r| m[r][c])))
        .product();
    if !det.is_finite() || !scale.is_finite() || scale == 0. || det.abs() / scale <= 1e-12 {
        return Err(unsupported(
            "singular or severely conditioned node transform",
        ));
    }
    Ok((det, cof.map(|row| row.map(|v| v / det))))
}

/// A baked tangent basis is supported only when the accumulated linear map
/// preserves angles and relative axis lengths. Scale first to avoid overflow
/// in the Gram matrix; reflections are allowed and adjust tangent handedness.
fn conformal(m: Matrix) -> Result<()> {
    let magnitude = (0..3)
        .flat_map(|r| (0..3).map(move |c| m[r][c].abs()))
        .fold(0.0_f64, f64::max);
    let gram: [[f64; 3]; 3] = std::array::from_fn(|a| {
        std::array::from_fn(|b| {
            (0..3)
                .map(|r| (m[r][a] / magnitude) * (m[r][b] / magnitude))
                .sum()
        })
    });
    let squared_scale = (0..3).map(|i| gram[i][i]).sum::<f64>() / 3.;
    if !squared_scale.is_finite()
        || squared_scale <= 0.
        || (0..3).any(|r| {
            (0..3).any(|c| {
                let expected = if r == c { squared_scale } else { 0. };
                (gram[r][c] - expected).abs() > squared_scale * 1e-10
            })
        })
    {
        return Err(unsupported(
            "authored TANGENT requires a conformal accumulated transform",
        ));
    }
    Ok(())
}

fn transform(node: &Value) -> Result<Matrix> {
    object(
        node,
        &[
            "name",
            "mesh",
            "children",
            "matrix",
            "translation",
            "rotation",
            "scale",
        ],
    )?;
    let result = if let Some(matrix) = node.get("matrix") {
        if ["translation", "rotation", "scale"]
            .iter()
            .any(|key| node.get(key).is_some())
        {
            return Err(invalid("node has both matrix and TRS"));
        }
        let values = vector::<16>(matrix)?;
        let m: Matrix = std::array::from_fn(|r| std::array::from_fn(|c| values[c * 4 + r]));
        if m[3] != [0., 0., 0., 1.] {
            return Err(invalid("node matrix must be affine"));
        }
        for a in 0..3 {
            for b in a + 1..3 {
                let dot: f64 = (0..3).map(|r| m[r][a] * m[r][b]).sum();
                let scale = length(std::array::from_fn(|r| m[r][a]))
                    * length(std::array::from_fn(|r| m[r][b]));
                if dot.abs() > scale * 1e-6 {
                    return Err(unsupported("sheared local matrix outside F1a"));
                }
            }
        }
        m
    } else {
        let t = node
            .get("translation")
            .map(vector::<3>)
            .transpose()?
            .unwrap_or([0.; 3]);
        let s = node
            .get("scale")
            .map(vector::<3>)
            .transpose()?
            .unwrap_or([1.; 3]);
        let mut q = node
            .get("rotation")
            .map(vector::<4>)
            .transpose()?
            .unwrap_or([0., 0., 0., 1.]);
        if q.iter().any(|v| !(-1.0..=1.0).contains(v)) {
            return Err(invalid("rotation quaternion components must be in [-1,1]"));
        }
        let norm = q.iter().map(|v| v * v).sum::<f64>().sqrt();
        if !norm.is_finite() || (norm - 1.).abs() > 1e-6 {
            return Err(invalid("rotation quaternion must be unit length"));
        }
        for v in &mut q {
            *v /= norm;
        }
        let [x, y, z, w] = q;
        let rotation = [
            [
                1. - 2. * (y * y + z * z),
                2. * (x * y - z * w),
                2. * (x * z + y * w),
            ],
            [
                2. * (x * y + z * w),
                1. - 2. * (x * x + z * z),
                2. * (y * z - x * w),
            ],
            [
                2. * (x * z - y * w),
                2. * (y * z + x * w),
                1. - 2. * (x * x + y * y),
            ],
        ];
        let mut m = IDENTITY;
        for r in 0..3 {
            for (c, scale) in s.iter().enumerate() {
                m[r][c] = rotation[r][c] * scale;
            }
            m[r][3] = t[r];
        }
        m
    };
    linear(result)?;
    Ok(result)
}

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum VertexRole {
    Position,
    Normal,
    Tangent,
    Texcoord(usize),
    Color,
}
impl VertexRole {
    /// Payload semantics independent of whether accessor bytes were captured or streamed.
    pub(crate) fn check_payload(self, values: &[f64]) -> Result<()> {
        match self {
            Self::Position => check_source_position([values[0], values[1], values[2]])?,
            Self::Normal | Self::Tangent => {
                if (length([values[0], values[1], values[2]]) - 1.).abs() > 1e-4 {
                    return Err(invalid("source NORMAL/TANGENT XYZ must be unit length"));
                }
                if self == Self::Tangent && ![-1., 1.].contains(&values[3]) {
                    return Err(invalid("source TANGENT W must be exactly -1 or +1"));
                }
            }
            Self::Texcoord(_) => {
                if values.iter().any(|v| v.abs() > 1_000_000.) {
                    return Err(unsupported("TEXCOORD outside supported finite magnitude"));
                }
            }
            Self::Color => {
                if values.iter().any(|v| !(0.0..=1.0).contains(v)) {
                    return Err(unsupported("COLOR_0 outside bounded [0,1] profile"));
                }
            }
        }
        Ok(())
    }
    fn validate(self, accessor: &AccessorLayout) -> Result<()> {
        let float = accessor.component == 5126 && !accessor.normalized;
        let normalized = [5121, 5123].contains(&accessor.component) && accessor.normalized;
        let valid = match self {
            Self::Position | Self::Normal => accessor.shape == AccessorShape::Vec3 && float,
            Self::Tangent => accessor.shape == AccessorShape::Vec4 && float,
            Self::Texcoord(_) => accessor.shape == AccessorShape::Vec2 && (float || normalized),
            Self::Color => {
                matches!(accessor.shape, AccessorShape::Vec3 | AccessorShape::Vec4)
                    && (float || normalized)
            }
        };
        if !valid {
            return Err(invalid(
                "vertex accessor encoding contradicts its core semantic",
            ));
        }
        Ok(())
    }
}
struct Primitive {
    position: usize,
    normal: Option<usize>,
    tangent: Option<usize>,
    texcoords: [Option<usize>; 2],
    color: Option<usize>,
    indices: Option<usize>,
    count: usize,
    material: Option<usize>,
}
impl Primitive {
    fn attributes(&self) -> impl Iterator<Item = (VertexRole, usize)> + '_ {
        std::iter::once((VertexRole::Position, self.position))
            .chain(self.normal.map(|i| (VertexRole::Normal, i)))
            .chain(self.tangent.map(|i| (VertexRole::Tangent, i)))
            .chain(
                self.texcoords
                    .iter()
                    .enumerate()
                    .filter_map(|(set, &i)| i.map(|i| (VertexRole::Texcoord(set), i))),
            )
            .chain(self.color.map(|i| (VertexRole::Color, i)))
    }
    fn vertices(&self, data: &[Accessor<'_>], start: usize) -> [usize; 3] {
        let vertices = [start, start + 1, start + 2];
        self.indices.map_or(vertices, |a| {
            vertices.map(|i| data[a].scalar(i, 0) as usize)
        })
    }
}
#[derive(Clone, Copy, PartialEq, Eq)]
enum BufferViewUse {
    Vertex(usize),
    Indices,
}

fn primitives(
    doc: &Value,
    data: &[AccessorLayout],
    materials: &[Material],
    limits: &Limits,
    check: &mut impl FnMut() -> Result<()>,
) -> Result<Vec<Vec<Primitive>>> {
    if list(doc, "meshes")?.len() > limits.meshes {
        return Err(unsupported(format!("mesh count exceeds {}", limits.meshes)));
    }
    let get = |i: usize| {
        data.get(i)
            .ok_or_else(|| invalid("primitive accessor out of range"))
    };
    let mut view_uses = vec![None; list(doc, "bufferViews")?.len()];
    let mut use_view = |accessor: usize, usage: BufferViewUse| -> Result<&Value> {
        let index = field(&doc["accessors"][accessor], "bufferView")?;
        let slot = &mut view_uses[index];
        match (*slot, usage) {
            (Some(BufferViewUse::Vertex(first)), BufferViewUse::Vertex(next)) => {
                if first != next && doc["bufferViews"][index].get("byteStride").is_none() {
                    return Err(invalid("shared vertex bufferView requires byteStride"));
                }
            }
            (None, _) | (Some(BufferViewUse::Indices), BufferViewUse::Indices) => {
                *slot = Some(usage);
            }
            _ => {
                return Err(invalid(
                    "bufferView cannot mix vertex attributes and indices",
                ))
            }
        }
        Ok(&doc["bufferViews"][index])
    };
    let mut meshes = Vec::new();
    let mut total = 0usize;
    let mut primitive_count = 0;
    let mut used_indices = vec![false; data.len()];
    for mesh in list(doc, "meshes")? {
        object(mesh, &["name", "primitives"])?;
        let values = array(&mesh["primitives"])?;
        if values.is_empty() {
            return Err(invalid("mesh has no primitives"));
        }
        let mut prepared = Vec::new();
        for p in values {
            check()?;
            primitive_count += 1;
            if primitive_count > limits.primitives {
                return Err(unsupported(format!(
                    "{} primitive ceiling exceeded",
                    limits.label
                )));
            }
            object(p, &["attributes", "indices", "material", "mode"])?;
            if p.get("mode").map(uint).transpose()?.unwrap_or(4) != 4 {
                return Err(unsupported("F1a supports TRIANGLES only"));
            }
            let attributes = object(
                &p["attributes"],
                &[
                    "POSITION",
                    "NORMAL",
                    "TANGENT",
                    "TEXCOORD_0",
                    "TEXCOORD_1",
                    "COLOR_0",
                ],
            )?;
            let attribute = |name| attributes.get(name).map(uint).transpose();
            let mut primitive = Primitive {
                position: field(&p["attributes"], "POSITION")?,
                normal: attribute("NORMAL")?,
                tangent: attribute("TANGENT")?,
                texcoords: [attribute("TEXCOORD_0")?, attribute("TEXCOORD_1")?],
                color: attribute("COLOR_0")?,
                indices: p.get("indices").map(uint).transpose()?,
                count: 0,
                material: p.get("material").map(uint).transpose()?,
            };
            let pos = get(primitive.position)?;
            for (role, i) in primitive.attributes() {
                let a = get(i)?;
                role.validate(a)?;
                if a.count != pos.count {
                    return Err(invalid("vertex attribute/POSITION count mismatch"));
                }
                let raw = &doc["accessors"][i];
                let view = use_view(i, BufferViewUse::Vertex(i))?;
                if !offset(raw, "byteOffset")?.is_multiple_of(4)
                    || !a.stride.is_multiple_of(4)
                    || view
                        .get("target")
                        .map(uint)
                        .transpose()?
                        .is_some_and(|n| n != 34962)
                {
                    return Err(invalid("invalid vertex accessor alignment/target"));
                }
            }
            if doc["accessors"][primitive.position].get("min").is_none()
                || doc["accessors"][primitive.position].get("max").is_none()
            {
                return Err(invalid("POSITION requires min/max"));
            }
            if primitive.texcoords[1].is_some() && primitive.texcoords[0].is_none() {
                return Err(invalid("TEXCOORD_1 requires consecutive TEXCOORD_0"));
            }
            if primitive.tangent.is_some() && primitive.normal.is_none() {
                return Err(unsupported(
                    "authored TANGENT requires authored NORMAL in this profile",
                ));
            }
            primitive.count = if let Some(i) = primitive.indices {
                let a = get(i)?;
                used_indices[i] = true;
                if a.shape != AccessorShape::Scalar
                    || ![5121, 5123, 5125].contains(&a.component)
                    || a.normalized
                {
                    return Err(invalid("indices must be non-normalized unsigned SCALAR"));
                }
                let view = use_view(i, BufferViewUse::Indices)?;
                if view.get("byteStride").is_some()
                    || view
                        .get("target")
                        .map(uint)
                        .transpose()?
                        .is_some_and(|n| n != 34963)
                {
                    return Err(invalid("indices cannot be strided or vertex-targeted"));
                }
                a.count
            } else {
                pos.count
            };
            if !primitive.count.is_multiple_of(3) {
                return Err(invalid("TRIANGLES count must be divisible by three"));
            }
            total = total
                .checked_add(primitive.count / 3)
                .ok_or_else(|| unsupported("declared triangle count overflow"))?;
            if limits.triangles.is_some_and(|max| total > max) {
                return Err(unsupported("declared triangle ceiling exceeded"));
            }
            if let Some(i) = primitive.material {
                let material = materials
                    .get(i)
                    .ok_or_else(|| invalid("material reference out of range"))?;
                for binding in material.bindings() {
                    if primitive
                        .texcoords
                        .get(binding.texcoord)
                        .is_none_or(Option::is_none)
                    {
                        return Err(invalid(
                            "material texture requires its bound primitive TEXCOORD set",
                        ));
                    }
                }
                if material.has_normal_texture()
                    && (primitive.normal.is_none() || primitive.tangent.is_none())
                {
                    return Err(unsupported(
                        "normalTexture requires authored NORMAL and TANGENT in this profile",
                    ));
                }
            }
            prepared.push(primitive);
        }
        meshes.push(prepared);
    }
    if data
        .iter()
        .zip(used_indices)
        .any(|(a, used)| a.component == 5125 && a.shape == AccessorShape::Scalar && !used)
    {
        return Err(invalid(
            "u32 accessor must be referenced by primitive indices",
        ));
    }
    Ok(meshes)
}

fn validate_primitive_payload(
    meshes: &[Vec<Primitive>],
    data: &[Accessor<'_>],
    check: &mut impl FnMut() -> Result<()>,
) -> Result<()> {
    let mut attributes = std::collections::HashSet::new();
    let mut indices = std::collections::HashSet::new();
    for p in meshes.iter().flatten() {
        check()?;
        let pos = &data[p.position];
        for (role, accessor) in p.attributes() {
            // UV sets share identical payload semantics even when an accessor
            // is bound to both. Other semantic roles remain independently checked.
            let cached_role = match role {
                VertexRole::Texcoord(_) => VertexRole::Texcoord(0),
                role => role,
            };
            if !attributes.insert((cached_role, accessor)) {
                continue;
            }
            let a = &data[accessor];
            for i in 0..a.count {
                if i.is_multiple_of(1024) {
                    check()?;
                }
                let values: [f64; 4] =
                    std::array::from_fn(|c| if c < a.width { a.value(i, c) } else { 0. });
                role.check_payload(&values[..a.width])?;
            }
        }
        if let Some(index_accessor) = p.indices.filter(|i| indices.insert((*i, pos.count))) {
            let a = &data[index_accessor];
            for i in 0..a.count {
                if i.is_multiple_of(1024) {
                    check()?;
                }
                check_index(a.scalar(i, 0) as usize, pos.count, a.component)?;
            }
        }
        if let Some(tangent) = p.tangent {
            for start in (0..p.count).step_by(3) {
                if start.is_multiple_of(3072) {
                    check()?;
                }
                let vertices = p.vertices(data, start);
                let sign = data[tangent].scalar(vertices[0], 3);
                if vertices[1..]
                    .iter()
                    .any(|&i| data[tangent].scalar(i, 3) != sign)
                {
                    return Err(unsupported(
                        "mixed triangle TANGENT W has undefined tangent space",
                    ));
                }
            }
        }
    }
    Ok(())
}

#[derive(Clone, Debug)]
pub(crate) enum BufferSource {
    Embedded(Range<usize>),
    External(usize),
}
#[derive(Clone, Debug)]
pub(crate) enum ImageSource {
    BufferView(usize),
    External(usize),
}
/// A GLB BIN chunk located in the root, with its last (at most three) bytes for the padding
/// check, so the payload itself need not be resident.
pub(crate) struct BinChunk {
    pub range: Range<usize>,
    pub tail: Vec<u8>,
}
#[derive(Clone, Debug)]
pub(crate) struct ResourceRequest {
    pub uri: String,
}

/// Pure parsed metadata and layout plans. Root and dependency byte owners stay
/// in the consumer; embedded data is a range rather than a self-reference.
pub(crate) struct Document {
    value: Value,
    buffers: Vec<BufferSource>,
    buffer_lengths: Vec<usize>,
    images: Vec<ImageSource>,
    resources: Vec<ResourceRequest>,
    accessors: Vec<AccessorLayout>,
    meshes: Vec<Vec<Primitive>>,
    instances: Vec<Instance>,
    triangle_count: usize,
    materials: Vec<Material>,
    textures: Vec<Value>,
    samplers: Vec<Value>,
}
impl Document {
    pub fn parse(bytes: &[u8], mut check: impl FnMut() -> Result<()>) -> Result<Self> {
        check()?;
        if bytes.len() > MAX_SOURCE_BYTES {
            return Err(unsupported("root document exceeds 32 MiB"));
        }
        if bytes.starts_with(b"glTF") {
            let (json, bin) = envelope(bytes)?;
            let bin = bin.map(|range| BinChunk {
                tail: bytes[range.end - range.len().min(3)..range.end].to_vec(),
                range,
            });
            Self::parse_parts(json, true, bin, &Limits::F1A, check)
        } else {
            Self::parse_parts(bytes, false, None, &Limits::F1A, check)
        }
    }

    /// Parses a root given as its JSON bytes and, for GLB, its BIN chunk position. Streaming
    /// consumers read only these bytes; buffer payloads stay on disk.
    pub(crate) fn parse_parts(
        json: &[u8],
        glb: bool,
        bin: Option<BinChunk>,
        limits: &Limits,
        mut check: impl FnMut() -> Result<()>,
    ) -> Result<Self> {
        if !glb && json.len() > limits.json_bytes {
            return Err(unsupported(format!(
                "glTF JSON exceeds {} MiB",
                limits.json_bytes >> 20
            )));
        }
        let doc = json::parse(json, glb)?;
        validate_document(&doc)?;
        let mut resources = Vec::new();
        let mut buffers = Vec::new();
        let mut buffer_lengths = Vec::new();
        let declarations = list(&doc, "buffers")?;
        if declarations.is_empty() {
            return Err(invalid("mesh document requires buffers"));
        }
        if declarations.len() > limits.buffers {
            return Err(unsupported(format!(
                "buffer count exceeds {}",
                limits.buffers
            )));
        }
        let mut total_declared = 0usize;
        for (i, buffer) in declarations.iter().enumerate() {
            check()?;
            object(buffer, &["byteLength", "name", "uri"])?;
            let declared = field(buffer, "byteLength")?;
            if declared == 0 {
                return Err(invalid("buffer byteLength must be positive"));
            }
            total_declared = total_declared
                .checked_add(declared)
                .ok_or_else(|| unsupported("declared buffer byte sum overflow"))?;
            if let Some(max) = limits.declared_buffer_bytes {
                if total_declared > max {
                    return Err(unsupported(format!(
                        "declared buffers exceed {} MiB",
                        max >> 20
                    )));
                }
            }
            buffer_lengths.push(declared);
            if let Some(uri) = buffer.get("uri") {
                let uri = uri
                    .as_str()
                    .ok_or_else(|| invalid("buffer URI must be a string"))?;
                if i == 0 && bin.is_some() {
                    return Err(invalid("GLB BIN requires URI-less buffer zero"));
                }
                buffers.push(BufferSource::External(resources.len()));
                resources.push(ResourceRequest { uri: uri.into() });
            } else {
                if i != 0 {
                    return Err(unsupported("only GLB buffer zero may omit URI"));
                }
                let chunk = bin
                    .as_ref()
                    .ok_or_else(|| invalid("URI-less buffer requires GLB BIN"))?;
                let length = chunk.range.len();
                if declared > length
                    || length - declared > 3
                    || chunk.tail[chunk.tail.len() - (length - declared)..]
                        .iter()
                        .any(|&b| b != 0)
                {
                    return Err(invalid(
                        "BIN payload does not match buffer byteLength/padding",
                    ));
                }
                buffers.push(BufferSource::Embedded(
                    chunk.range.start..chunk.range.start + declared,
                ));
            }
        }
        let accessors = accessor_layouts(&doc, &buffer_lengths, limits, &mut check)?;
        let materials = material::parse(&doc, limits.materials)?;
        let (images, textures, samplers) =
            texture::metadata(&doc, &mut resources, limits.images, &mut check)?;
        let meshes = primitives(&doc, &accessors, &materials, limits, &mut check)?;
        validate_accessor_profile(&accessors, &mut check)?;
        let (instances, triangle_count) = scene_plan(&doc, &meshes, limits, &mut check)?;
        check()?;
        Ok(Self {
            value: doc,
            buffers,
            buffer_lengths,
            images,
            resources,
            accessors,
            meshes,
            instances,
            triangle_count,
            materials,
            textures,
            samplers,
        })
    }
    pub fn resources(&self) -> &[ResourceRequest] {
        &self.resources
    }
    pub fn buffer_sources(&self) -> &[BufferSource] {
        &self.buffers
    }
    pub fn image_sources(&self) -> &[ImageSource] {
        &self.images
    }
    /// Declared byte lengths of the buffers, in declaration order.
    pub(crate) fn buffer_lengths(&self) -> &[usize] {
        &self.buffer_lengths
    }
    /// Expanded triangle count of the selected scene.
    pub(crate) fn triangle_count(&self) -> usize {
        self.triangle_count
    }
    /// Every declared accessor, including those in unselected meshes.
    pub(crate) fn accessor_views(&self) -> impl Iterator<Item = AccessorView<'_>> {
        self.accessors.iter().enumerate().map(|(i, a)| a.view(i))
    }
    /// Every authored primitive is validated before any selected-scene capture is accepted.
    pub(crate) fn payload_primitives(&self) -> impl Iterator<Item = PayloadPrimitive<'_>> {
        self.meshes.iter().flatten().map(|p| PayloadPrimitive {
            position: self.accessors[p.position].view(p.position),
            indices: p.indices.map(|i| self.accessors[i].view(i)),
            tangent: p.tangent.map(|i| self.accessors[i].view(i)),
            count: p.count,
            primitive: p,
            accessors: &self.accessors,
        })
    }
    /// Every instanced primitive in scene traversal order, then primitive order.
    pub(crate) fn scene_primitives(&self) -> impl Iterator<Item = ScenePrimitive<'_>> + '_ {
        self.instances.iter().flat_map(move |instance| {
            self.meshes[instance.mesh]
                .iter()
                .enumerate()
                .map(move |(primitive, p)| ScenePrimitive {
                    node: instance.node as u32,
                    mesh: instance.mesh as u32,
                    primitive: primitive as u32,
                    world: &instance.world,
                    determinant: instance.determinant,
                    position: self.accessors[p.position].view(p.position),
                    indices: p.indices.map(|i| self.accessors[i].view(i)),
                    triangles: p.count / 3,
                    has_normal: p.normal.is_some(),
                    has_tangent: p.tangent.is_some(),
                    texcoord_sets: p.texcoords.iter().flatten().count(),
                    has_color: p.color.is_some(),
                })
        })
    }
}

fn validate_document(doc: &Value) -> Result<()> {
    // Separate profile checks prevent a permissive schema parser from silently
    // ignoring extension, metadata, resource, or rendering semantics.
    object(
        doc,
        &[
            "asset",
            "scene",
            "scenes",
            "nodes",
            "meshes",
            "materials",
            "buffers",
            "bufferViews",
            "accessors",
            "images",
            "textures",
            "samplers",
        ],
    )?;
    object(
        &doc["asset"],
        &["version", "minVersion", "generator", "copyright"],
    )?;
    if !doc["asset"]["version"].is_string()
        || doc["asset"]
            .get("minVersion")
            .is_some_and(|v| !v.is_string())
    {
        return Err(invalid(
            "asset version fields must be strings; version is required",
        ));
    }
    if doc["asset"]["version"] != "2.0"
        || doc["asset"].get("minVersion").is_some_and(|v| v != "2.0")
    {
        return Err(unsupported("F1a requires glTF asset version 2.0"));
    }
    for key in ["generator", "copyright"] {
        if doc["asset"].get(key).is_some_and(|v| !v.is_string()) {
            return Err(invalid("asset label must be string"));
        }
    }
    Ok(())
}

fn scene_plan(
    doc: &Value,
    meshes: &[Vec<Primitive>],
    limits: &Limits,
    check: &mut impl FnMut() -> Result<()>,
) -> Result<(Vec<Instance>, usize)> {
    let nodes = list(doc, "nodes")?;
    if nodes.len() > limits.nodes {
        return Err(unsupported(format!(
            "{} node ceiling exceeded",
            limits.label
        )));
    }
    let mut locals = Vec::with_capacity(nodes.len());
    let mut parents = vec![None; nodes.len()];
    for (i, node) in nodes.iter().enumerate() {
        check()?;
        locals.push(transform(node)?);
        if node
            .get("name")
            .and_then(Value::as_str)
            .is_some_and(|name| name.len() > 4096)
        {
            return Err(unsupported("source node name exceeds 4096 UTF-8 bytes"));
        }
        if node
            .get("mesh")
            .map(uint)
            .transpose()?
            .is_some_and(|m| m >= meshes.len())
        {
            return Err(invalid("node mesh out of range"));
        }
        for child in list(node, "children")? {
            let c = uint(child)?;
            let parent = parents
                .get_mut(c)
                .ok_or_else(|| invalid("child node out of range"))?;
            if parent.replace(i).is_some() {
                return Err(invalid("node has duplicate/multiple parents"));
            }
        }
    }
    for start in 0..nodes.len() {
        let mut current = Some(start);
        let mut depth = 0;
        let mut ancestors = std::collections::HashSet::new();
        while let Some(i) = current {
            if !ancestors.insert(i) {
                return Err(invalid("node graph contains a cycle"));
            }
            depth += 1;
            if depth > 128 {
                return Err(unsupported("node hierarchy exceeds depth 128"));
            }
            current = parents[i];
        }
    }
    let scenes = list(doc, "scenes")?;
    for scene in scenes {
        object(scene, &["name", "nodes"])?;
        let mut roots = std::collections::HashSet::new();
        for root in list(scene, "nodes")? {
            let i = uint(root)?;
            if i >= nodes.len() || parents[i].is_some() || !roots.insert(i) {
                return Err(invalid("scene root invalid/duplicated or has parent"));
            }
        }
    }
    let selected = match doc.get("scene") {
        Some(v) => uint(v)?,
        None if scenes.len() == 1 => 0,
        _ => return Err(unsupported("F1a requires default scene or sole scene")),
    };
    let scene = scenes
        .get(selected)
        .ok_or_else(|| invalid("default scene out of range"))?;
    let mut stack: Vec<_> = list(scene, "nodes")?
        .iter()
        .rev()
        .map(|v| Ok((uint(v)?, IDENTITY)))
        .collect::<Result<_>>()?;
    let mut instances = Vec::new();
    let mut triangle_count = 0usize;
    while let Some((i, parent)) = stack.pop() {
        check()?;
        let world = multiply(parent, locals[i]);
        let (det, normal_matrix) = linear(world)?;
        if let Some(mesh) = nodes[i].get("mesh") {
            let mesh = uint(mesh)?;
            if meshes[mesh].iter().any(|p| p.tangent.is_some()) {
                conformal(world)?;
            }
            for p in &meshes[mesh] {
                triangle_count = triangle_count
                    .checked_add(p.count / 3)
                    .ok_or_else(|| unsupported("expanded triangle count overflow"))?;
            }
            if limits.triangles.is_some_and(|max| triangle_count > max) {
                return Err(unsupported(
                    "expanded selected-scene triangle ceiling exceeded",
                ));
            }
            instances.push(Instance {
                node: i,
                mesh,
                world,
                determinant: det,
                normal_matrix,
            });
        }
        for child in list(&nodes[i], "children")?.iter().rev() {
            stack.push((uint(child)?, world));
        }
    }
    if triangle_count == 0 {
        return Err(invalid("selected scene contains no triangles"));
    }
    Ok((instances, triangle_count))
}

pub(crate) fn decode(
    document: &Document,
    supplied_buffers: &[&[u8]],
    image_resources: &[Option<&[u8]>],
    mut check: impl FnMut() -> Result<()>,
) -> Result<Geometry> {
    check()?;
    if supplied_buffers.len() != document.buffers.len() {
        return Err(invalid(
            "buffer snapshot count disagrees with parsed document",
        ));
    }
    let buffers = supplied_buffers
        .iter()
        .zip(&document.buffer_lengths)
        .map(|(buffer, &length)| {
            buffer
                .get(..length)
                .ok_or_else(|| invalid("actual buffer bytes shorter than declared byteLength"))
        })
        .collect::<Result<Vec<_>>>()?;
    let data = decode_accessors(&document.accessors, &buffers, &mut check)?;
    validate_primitive_payload(&document.meshes, &data, &mut check)?;
    let images = texture::decode(
        &document.value,
        &buffers,
        &document.images,
        image_resources,
        &mut check,
    )?;
    let mut triangles = Vec::with_capacity(document.triangle_count);
    for instance in &document.instances {
        let Instance {
            node,
            mesh,
            world,
            determinant: det,
            normal_matrix,
        } = instance;
        for (primitive_index, p) in document.meshes[*mesh].iter().enumerate() {
            for start in (0..p.count).step_by(3) {
                if start.is_multiple_of(3072) {
                    check()?;
                }
                let mut vertices = p.vertices(&data, start);
                if *det < 0. {
                    vertices.swap(1, 2);
                }
                let mut positions = [[0f32; 3]; 3];
                let mut normals = p.normal.map(|_| [[0.; 3]; 3]);
                let mut tangents = p.tangent.map(|_| [[0.; 4]; 3]);
                let mut texcoords = p.texcoords.map(|a| a.map(|_| [[0.; 2]; 3]));
                let mut colors = p.color.map(|_| [[0.; 4]; 3]);
                for (corner, index) in vertices.into_iter().enumerate() {
                    positions[corner] = bake_position(world, data[p.position].vec3(index))?;
                    if let (Some(a), Some(output)) = (p.normal, &mut normals) {
                        let n = data[a].vec3(index);
                        let mapped: [f64; 3] = std::array::from_fn(|r| {
                            (0..3).map(|c| normal_matrix[r][c] * n[c]).sum()
                        });
                        let norm = length(mapped);
                        if !norm.is_finite() || norm == 0. {
                            return Err(unsupported("normal transform outside numeric domain"));
                        }
                        output[corner] = mapped.map(|v| (v / norm) as f32);
                    }
                    if let (Some(a), Some(output)) = (p.tangent, &mut tangents) {
                        let t = data[a].vec3(index);
                        let mapped: [f64; 3] =
                            std::array::from_fn(|r| (0..3).map(|c| world[r][c] * t[c]).sum());
                        let norm = length(mapped);
                        if !norm.is_finite() || norm == 0. {
                            return Err(unsupported("tangent transform outside numeric domain"));
                        }
                        let xyz = mapped.map(|v| (v / norm) as f32);
                        output[corner] = [
                            xyz[0],
                            xyz[1],
                            xyz[2],
                            (data[a].scalar(index, 3) * det.signum()) as f32,
                        ];
                    }
                    for (accessor, output) in p.texcoords.into_iter().zip(&mut texcoords) {
                        if let (Some(a), Some(output)) = (accessor, output) {
                            output[corner] = data[a].decoded(index);
                        }
                    }
                    if let (Some(a), Some(output)) = (p.color, &mut colors) {
                        let rgb = data[a].decoded::<3>(index);
                        let alpha = if data[a].width == 3 {
                            1.
                        } else {
                            data[a].value(index, 3) as f32
                        };
                        output[corner] = [rgb[0], rgb[1], rgb[2], alpha];
                    }
                }
                triangles.push(Triangle {
                    source: SourceIdentity {
                        node_index: *node as u32,
                        mesh_index: *mesh as u32,
                        primitive_index: primitive_index as u32,
                        triangle_index: (start / 3) as u32,
                    },
                    vertex_indices: vertices.map(|index| index as u32),
                    positions,
                    normals,
                    tangents,
                    texcoords,
                    colors,
                    material: p.material,
                });
            }
        }
    }
    Ok(Geometry {
        node_names: list(&document.value, "nodes")?
            .iter()
            .map(|node| node.get("name").and_then(Value::as_str).map(str::to_owned))
            .collect(),
        triangles,
        materials: document.materials.clone(),
        images,
        textures: document.textures.clone(),
        samplers: document.samplers.clone(),
    })
}

#[cfg(test)]
mod metadata_tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn reflected_authored_keys_follow_the_decoded_corner_order() {
        let value = json!({
            "asset":{"version":"2.0"},
            "buffers":[{"uri":"data.bin","byteLength":42}],
            "bufferViews":[{"buffer":0,"byteLength":36},
                {"buffer":0,"byteOffset":36,"byteLength":6}],
            "accessors":[{"bufferView":0,"componentType":5126,"count":3,
                "type":"VEC3","min":[0,0,0],"max":[1,1,0]},
                {"bufferView":1,"componentType":5123,"count":3,"type":"SCALAR"}],
            "meshes":[{"primitives":[{"attributes":{"POSITION":0},"indices":1}]}],
            "nodes":[{"mesh":0,"scale":[-1,1,1]}],"scenes":[{"nodes":[0]}]
        });
        let document = Document::parse(&serde_json::to_vec(&value).unwrap(), || Ok(())).unwrap();
        let mut buffer = Vec::new();
        for point in [[0.0_f32, 0., 0.], [1., 0., 0.], [0., 1., 0.]] {
            for component in point {
                buffer.extend(component.to_le_bytes());
            }
        }
        for index in [2_u16, 0, 1] {
            buffer.extend(index.to_le_bytes());
        }
        let geometry = decode(&document, &[&buffer], &[], || Ok(())).unwrap();
        assert_eq!(geometry.triangles[0].vertex_indices, [2, 1, 0]);
        assert_eq!(
            geometry.triangles[0].positions,
            [[0., 1., 0.], [-1., 0., 0.], [0., 0., 0.]]
        );
    }

    // Only metadata is supplied. The dependency deliberately does not exist;
    // Document::parse must classify declarations without resolving its URI.
    fn declared_attribute(component: usize, shape: &str, normalized: bool, used: bool) -> Vec<u8> {
        let mut attributes = json!({"POSITION": 0});
        if used {
            attributes["COLOR_0"] = json!(1);
        }
        serde_json::to_vec(&json!({
            "asset": {"version": "2.0"},
            "buffers": [{"uri": "deliberately-missing.bin", "byteLength": 228}],
            "bufferViews": [
                {"buffer": 0, "byteLength": 36},
                {"buffer": 0, "byteOffset": 36, "byteLength": 192}
            ],
            "accessors": [
                {"bufferView": 0, "componentType": 5126, "type": "VEC3", "count": 3,
                 "min": [0, 0, 0], "max": [1, 1, 0]},
                {"bufferView": 1, "componentType": component, "type": shape,
                 "normalized": normalized, "count": 3}
            ],
            "meshes": [{"primitives": [{"attributes": attributes}]}],
            "nodes": [{"mesh": 0}],
            "scenes": [{"nodes": [0]}]
        }))
        .unwrap()
    }

    #[test]
    fn malformed_color_encodings_fail_metadata_before_dependency_capture() {
        for (component, shape, normalized) in [
            (5121, "VEC4", false),
            (5123, "VEC3", false),
            (5120, "VEC4", true),
            (5122, "VEC3", true),
            (5126, "MAT2", false),
            (5126, "SCALAR", false),
        ] {
            let error = Document::parse(
                &declared_attribute(component, shape, normalized, true),
                || Ok(()),
            )
            .err()
            .expect("contradictory COLOR_0 encoding must fail");
            assert_eq!(error.kind(), JobErrorKind::InvalidInput);
        }
    }

    #[test]
    fn valid_unused_storage_outside_mesh_profile_stays_unsupported() {
        for (component, shape, normalized) in [
            (5120, "VEC3", true),
            (5122, "VEC3", false),
            (5126, "SCALAR", false),
            (5126, "MAT4", false),
            (5125, "VEC4", false),
        ] {
            let error = Document::parse(
                &declared_attribute(component, shape, normalized, false),
                || Ok(()),
            )
            .err()
            .expect("unused generic encoding must remain outside profile");
            assert_eq!(error.kind(), JobErrorKind::Unsupported);
        }
    }

    #[test]
    fn normalization_invalid_for_component_is_invalid_even_when_unused() {
        for component in [5125, 5126] {
            let error =
                Document::parse(&declared_attribute(component, "VEC4", true, false), || {
                    Ok(())
                })
                .err()
                .expect("FLOAT/u32 normalized flag must fail");
            assert_eq!(error.kind(), JobErrorKind::InvalidInput);
        }
    }

    #[test]
    fn source_node_name_cap_counts_utf8_bytes_without_changing_missing_or_empty() {
        for (name, accepted) in [
            ("a".repeat(4096), true),
            ("🦉".repeat(1024), true),
            ("🦉".repeat(1025), false),
        ] {
            let mut value: Value =
                serde_json::from_slice(&declared_attribute(5126, "VEC3", false, false)).unwrap();
            value["nodes"][0]["name"] = json!(name);
            let result = Document::parse(&serde_json::to_vec(&value).unwrap(), || Ok(()));
            if accepted {
                assert!(result.is_ok());
            } else {
                assert_eq!(result.err().unwrap().kind(), JobErrorKind::Unsupported);
            }
        }
    }
}
