//! Finite glTF document/resource admission and decoding from borrowed snapshots.
//! No filesystem, jobs, source dependency discovery, or legacy mesh IR.
use crate::{JobError, JobErrorKind};
use serde_json::{Map, Value};
use std::ops::Range;
mod json;
mod texture;

pub(super) const MAX_SOURCE_BYTES: usize = 32 * 1024 * 1024;
pub(super) const MAX_TRIANGLES: usize = 100_000;
pub(super) const MAX_LEAVES: usize = 4096;
const MAX_JSON_BYTES: usize = 1024 * 1024;
type Result<T> = std::result::Result<T, JobError>;
type Matrix = [[f64; 4]; 4];
type Instance = (usize, Matrix, f64, [[f64; 3]; 3]);
const IDENTITY: Matrix = [
    [1., 0., 0., 0.],
    [0., 1., 0., 0.],
    [0., 0., 1., 0.],
    [0., 0., 0., 1.],
];

#[derive(Clone, Debug)]
pub(super) struct Triangle {
    pub positions: [[f32; 3]; 3],
    pub normals: Option<[[f32; 3]; 3]>,
    pub texcoords: Option<[[f32; 2]; 3]>,
    pub material: Option<usize>,
}

pub(super) struct Image {
    pub bytes: Vec<u8>,
    pub mime_type: &'static str,
    pub extension: &'static str,
    pub width: u32,
    pub height: u32,
}

pub(super) struct Geometry {
    pub triangles: Vec<Triangle>,
    pub materials: Vec<Value>,
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
fn u32_at(bytes: &[u8], at: usize) -> Result<u32> {
    let data = bytes
        .get(at..at + 4)
        .ok_or_else(|| invalid("truncated GLB envelope"))?;
    Ok(u32::from_le_bytes(data.try_into().unwrap()))
}

fn envelope(bytes: &[u8]) -> Result<(&[u8], Option<Range<usize>>)> {
    if bytes.len() > MAX_SOURCE_BYTES {
        return Err(unsupported("source exceeds 32 MiB F1a ceiling"));
    }
    if bytes.get(..4) != Some(b"glTF")
        || u32_at(bytes, 4)? != 2
        || u32_at(bytes, 8)? as usize != bytes.len()
    {
        return Err(invalid("invalid GLB v2 header or file length"));
    }
    let json_len = u32_at(bytes, 12)? as usize;
    if json_len > MAX_JSON_BYTES {
        return Err(unsupported("GLB JSON exceeds 1 MiB F1a ceiling"));
    }
    if !json_len.is_multiple_of(4) || u32_at(bytes, 16)? != 0x4e4f534a {
        return Err(invalid("first GLB chunk must be aligned JSON"));
    }
    let json_end = 20usize
        .checked_add(json_len)
        .ok_or_else(|| invalid("GLB length overflow"))?;
    let json = bytes
        .get(20..json_end)
        .ok_or_else(|| invalid("truncated JSON chunk"))?;
    if json_end == bytes.len() {
        return Ok((json, None));
    }
    let bin_len = u32_at(bytes, json_end)? as usize;
    if u32_at(bytes, json_end + 4)? != 0x004e4942 {
        return Err(unsupported("F1a requires one BIN chunk after JSON"));
    }
    let bin_start = json_end + 8;
    let bin_end = bin_start
        .checked_add(bin_len)
        .ok_or_else(|| invalid("GLB length overflow"))?;
    if !bin_len.is_multiple_of(4) || bin_end > bytes.len() {
        return Err(invalid("invalid BIN length"));
    }
    if bin_end < bytes.len() {
        let mut cursor = bin_end;
        while cursor < bytes.len() {
            let len = u32_at(bytes, cursor)? as usize;
            let kind = u32_at(bytes, cursor + 4)?;
            cursor = cursor
                .checked_add(8)
                .and_then(|n| n.checked_add(len))
                .ok_or_else(|| invalid("GLB chunk length overflow"))?;
            if !len.is_multiple_of(4)
                || cursor > bytes.len()
                || [0x4e4f534a, 0x004e4942].contains(&kind)
            {
                return Err(invalid("invalid extra GLB chunk"));
            }
        }
        return Err(unsupported("unknown GLB chunks outside F1a"));
    }
    Ok((json, Some(bin_start..bin_end)))
}

#[derive(Clone, Copy)]
struct Accessor<'a> {
    bytes: &'a [u8],
    stride: usize,
    count: usize,
    component: usize,
}
impl Accessor<'_> {
    fn scalar(&self, i: usize, c: usize) -> f64 {
        let size = match self.component {
            5121 => 1,
            5123 => 2,
            _ => 4,
        };
        let start = i * self.stride + c * size;
        match self.component {
            5121 => self.bytes[start] as f64,
            5123 => u16::from_le_bytes(self.bytes[start..start + 2].try_into().unwrap()) as f64,
            5125 => u32::from_le_bytes(self.bytes[start..start + 4].try_into().unwrap()) as f64,
            5126 => f32::from_le_bytes(self.bytes[start..start + 4].try_into().unwrap()) as f64,
            _ => unreachable!("admitted component type"),
        }
    }
    fn vec3(&self, i: usize) -> [f64; 3] {
        std::array::from_fn(|c| self.scalar(i, c))
    }
    fn texcoord(&self, i: usize) -> [f32; 2] {
        let divisor = match self.component {
            5121 => u8::MAX as f64,
            5123 => u16::MAX as f64,
            _ => 1.,
        };
        std::array::from_fn(|c| (self.scalar(i, c) / divisor) as f32)
    }
}

struct AccessorLayout {
    buffer: usize,
    start: usize,
    length: usize,
    stride: usize,
    count: usize,
    component: usize,
    width: usize,
    min: Option<Vec<f64>>,
    max: Option<Vec<f64>>,
}

fn accessor_layouts(
    doc: &Value,
    buffer_lengths: &[usize],
    check: &mut impl FnMut() -> Result<()>,
) -> Result<Vec<AccessorLayout>> {
    let views = list(doc, "bufferViews")?;
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
        if total > 1_000_000 {
            return Err(unsupported("F1a accessor count ceiling exceeded"));
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
            5121 => 1,
            5123 => 2,
            5125 | 5126 => 4,
            5120 | 5122 => return Err(unsupported("accessor component outside F1a")),
            _ => return Err(invalid("invalid accessor component type")),
        };
        let width = match value["type"].as_str() {
            Some("SCALAR") => 1,
            Some("VEC2") => 2,
            Some("VEC3") => 3,
            Some("VEC4" | "MAT2" | "MAT3" | "MAT4") => {
                return Err(unsupported("accessor shape outside F1a"))
            }
            _ => return Err(invalid("missing or invalid accessor type")),
        };
        let supported = match width {
            1 => [5121, 5123, 5125].contains(&component) && !normalized,
            2 => {
                (component == 5126 && !normalized)
                    || ([5121, 5123].contains(&component) && normalized)
            }
            3 => component == 5126 && !normalized,
            _ => false,
        };
        if !supported {
            return Err(unsupported(
                "accessor component/shape/normalization outside the supported mesh profile",
            ));
        }
        let view = reference(views, &value["bufferView"])?;
        let relative = offset(value, "byteOffset")?;
        let start = offset(view, "byteOffset")?
            .checked_add(relative)
            .ok_or_else(|| invalid("accessor offset overflow"))?;
        let stride = view.get("byteStride").map_or(Ok(size * width), uint)?;
        let length = (count - 1)
            .checked_mul(stride)
            .and_then(|n| n.checked_add(size * width))
            .ok_or_else(|| invalid("accessor range overflow"))?;
        if !relative.is_multiple_of(size)
            || !start.is_multiple_of(size)
            || stride < size * width
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
                                    let max = match component {
                                        5121 => u8::MAX as f64,
                                        5123 => u16::MAX as f64,
                                        _ => u32::MAX as f64,
                                    };
                                    if n.fract() != 0. || !(0.0..=max).contains(&n) {
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
            width,
            min,
            max,
        });
    }
    Ok(result)
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
        };
        let mut actual_min = [f64::INFINITY; 3];
        let mut actual_max = [f64::NEG_INFINITY; 3];
        for i in 0..layout.count {
            if i.is_multiple_of(1024) {
                check()?;
            }
            for c in 0..layout.width {
                let x = accessor.scalar(i, c);
                if !x.is_finite() {
                    return Err(invalid("nonfinite accessor data"));
                }
                actual_min[c] = actual_min[c].min(x);
                actual_max[c] = actual_max[c].max(x);
            }
        }
        if layout
            .min
            .as_ref()
            .is_some_and(|values| values.as_slice() != &actual_min[..layout.width])
            || layout
                .max
                .as_ref()
                .is_some_and(|values| values.as_slice() != &actual_max[..layout.width])
        {
            return Err(invalid("accessor bounds disagree with actual extrema"));
        }
        result.push(accessor);
    }
    Ok(result)
}

fn materials(doc: &Value) -> Result<Vec<Value>> {
    let input = list(doc, "materials")?;
    if input.len() > 256 {
        return Err(unsupported("F1a material ceiling exceeded"));
    }
    let unit = |value: &Value| -> Result<()> {
        if !(0.0..=1.0).contains(&number(value)?) {
            return Err(invalid("material factor outside [0,1]"));
        }
        Ok(())
    };
    for material in input {
        object(
            material,
            &[
                "name",
                "pbrMetallicRoughness",
                "emissiveFactor",
                "alphaMode",
                "alphaCutoff",
                "doubleSided",
            ],
        )?;
        if let Some(pbr) = material.get("pbrMetallicRoughness") {
            object(
                pbr,
                &[
                    "baseColorFactor",
                    "metallicFactor",
                    "roughnessFactor",
                    "baseColorTexture",
                ],
            )?;
            if let Some(info) = pbr.get("baseColorTexture") {
                object(info, &["index", "texCoord"])?;
                reference(list(doc, "textures")?, &info["index"])?;
                if info.get("texCoord").map(uint).transpose()?.unwrap_or(0) != 0 {
                    return Err(unsupported("baseColorTexture requires TEXCOORD_0"));
                }
            }
            if let Some(v) = pbr.get("baseColorFactor") {
                vector::<4>(v)?;
                for c in array(v)? {
                    unit(c)?;
                }
            }
            for key in ["metallicFactor", "roughnessFactor"] {
                if let Some(v) = pbr.get(key) {
                    unit(v)?;
                }
            }
        }
        if let Some(v) = material.get("emissiveFactor") {
            vector::<3>(v)?;
            for c in array(v)? {
                unit(c)?;
            }
        }
        if let Some(v) = material.get("alphaMode") {
            match v.as_str() {
                Some("OPAQUE" | "MASK") => {}
                Some("BLEND") => {
                    return Err(unsupported("F1a does not preserve blended draw order"))
                }
                _ => return Err(invalid("invalid alphaMode")),
            }
        }
        if material
            .get("alphaCutoff")
            .map(number)
            .transpose()?
            .is_some_and(|v| v < 0.)
        {
            return Err(invalid("negative alphaCutoff"));
        }
        if material.get("doubleSided").is_some_and(|v| !v.is_boolean()) {
            return Err(invalid("doubleSided must be boolean"));
        }
    }
    Ok(input
        .iter()
        .cloned()
        .map(|mut v| {
            v.as_object_mut().unwrap().remove("name");
            v
        })
        .collect())
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

struct Primitive {
    position: usize,
    normal: Option<usize>,
    texcoord: Option<usize>,
    indices: Option<usize>,
    count: usize,
    material: Option<usize>,
}
#[derive(Clone, Copy, PartialEq, Eq)]
enum BufferViewUse {
    Vertex(usize),
    Indices,
}

fn primitives(
    doc: &Value,
    data: &[AccessorLayout],
    check: &mut impl FnMut() -> Result<()>,
) -> Result<Vec<Vec<Primitive>>> {
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
    let mut total = 0;
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
            if primitive_count > 4096 {
                return Err(unsupported("F1a primitive ceiling exceeded"));
            }
            object(p, &["attributes", "indices", "material", "mode"])?;
            if p.get("mode").map(uint).transpose()?.unwrap_or(4) != 4 {
                return Err(unsupported("F1a supports TRIANGLES only"));
            }
            object(&p["attributes"], &["POSITION", "NORMAL", "TEXCOORD_0"])?;
            let position = field(&p["attributes"], "POSITION")?;
            let normal = p["attributes"].get("NORMAL").map(uint).transpose()?;
            let texcoord = p["attributes"].get("TEXCOORD_0").map(uint).transpose()?;
            let pos = get(position)?;
            for (i, is_texcoord) in std::iter::once((position, false))
                .chain(normal.map(|i| (i, false)))
                .chain(texcoord.map(|i| (i, true)))
            {
                let a = get(i)?;
                if is_texcoord {
                    if a.width != 2 || ![5121, 5123, 5126].contains(&a.component) {
                        return Err(unsupported(
                            "TEXCOORD_0 must be f32 or normalized unsigned VEC2",
                        ));
                    }
                } else if a.component != 5126 || a.width != 3 {
                    return Err(unsupported("POSITION/NORMAL must be f32 VEC3"));
                }
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
            if doc["accessors"][position].get("min").is_none()
                || doc["accessors"][position].get("max").is_none()
            {
                return Err(invalid("POSITION requires min/max"));
            }
            let indices = p.get("indices").map(uint).transpose()?;
            let count = if let Some(i) = indices {
                let a = get(i)?;
                used_indices[i] = true;
                if a.width != 1 || ![5121, 5123, 5125].contains(&a.component) {
                    return Err(unsupported("indices must be unsigned SCALAR"));
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
            if !count.is_multiple_of(3) {
                return Err(invalid("TRIANGLES count must be divisible by three"));
            }
            total += count / 3;
            if total > MAX_TRIANGLES {
                return Err(unsupported("declared triangle ceiling exceeded"));
            }
            let material = p.get("material").map(uint).transpose()?;
            if material.is_some_and(|i| i >= list(doc, "materials").unwrap_or(&[]).len()) {
                return Err(invalid("material reference out of range"));
            }
            if material.is_some_and(|i| {
                doc["materials"][i]["pbrMetallicRoughness"]
                    .get("baseColorTexture")
                    .is_some()
            }) && texcoord.is_none()
            {
                return Err(invalid("textured material requires primitive TEXCOORD_0"));
            }
            prepared.push(Primitive {
                position,
                normal,
                texcoord,
                indices,
                count,
                material,
            });
        }
        meshes.push(prepared);
    }
    if data
        .iter()
        .zip(used_indices)
        .any(|(a, used)| a.component == 5125 && !used)
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
    let mut positions = std::collections::HashSet::new();
    let mut normals = std::collections::HashSet::new();
    let mut texcoords = std::collections::HashSet::new();
    let mut indices = std::collections::HashSet::new();
    for p in meshes.iter().flatten() {
        check()?;
        let pos = &data[p.position];
        if positions.insert(p.position) {
            for i in 0..pos.count {
                if i.is_multiple_of(1024) {
                    check()?;
                }
                if pos.vec3(i).iter().any(|v| v.abs() > 1_000_000.) {
                    return Err(unsupported("source position outside local mesh domain"));
                }
            }
        }
        if let Some(n) = p.normal.filter(|n| normals.insert(*n)) {
            for i in 0..pos.count {
                if i.is_multiple_of(1024) {
                    check()?;
                }
                if (length(data[n].vec3(i)) - 1.).abs() > 1e-4 {
                    return Err(invalid("source NORMAL must be unit length"));
                }
            }
        }
        if let Some(t) = p.texcoord.filter(|t| texcoords.insert(*t)) {
            let a = &data[t];
            for i in 0..a.count {
                if i.is_multiple_of(1024) {
                    check()?;
                }
                if (0..2).any(|c| a.scalar(i, c).abs() > 1_000_000.) {
                    return Err(unsupported("TEXCOORD_0 outside supported finite magnitude"));
                }
            }
        }
        if let Some(index_accessor) = p.indices.filter(|i| indices.insert((*i, pos.count))) {
            let a = &data[index_accessor];
            let reserved = match a.component {
                5121 => u8::MAX as usize,
                5123 => u16::MAX as usize,
                _ => u32::MAX as usize,
            };
            for i in 0..a.count {
                if i.is_multiple_of(1024) {
                    check()?;
                }
                let index = a.scalar(i, 0) as usize;
                if index >= pos.count || index == reserved {
                    return Err(invalid(
                        "triangle index out of range or reserved restart value",
                    ));
                }
            }
        }
    }
    Ok(())
}

#[derive(Clone, Debug)]
pub(super) enum BufferSource {
    Embedded(Range<usize>),
    External(usize),
}
#[derive(Clone, Debug)]
pub(super) enum ImageSource {
    BufferView(usize),
    External(usize),
}
#[derive(Clone, Debug)]
pub(super) struct ResourceRequest {
    pub uri: String,
}

/// Pure parsed metadata and layout plans. Root and dependency byte owners stay
/// in the consumer; embedded data is a range rather than a self-reference.
pub(super) struct Document {
    value: Value,
    buffers: Vec<BufferSource>,
    buffer_lengths: Vec<usize>,
    images: Vec<ImageSource>,
    resources: Vec<ResourceRequest>,
    accessors: Vec<AccessorLayout>,
    meshes: Vec<Vec<Primitive>>,
    instances: Vec<Instance>,
    triangle_count: usize,
    materials: Vec<Value>,
    textures: Vec<Value>,
    samplers: Vec<Value>,
}
impl Document {
    pub fn parse(bytes: &[u8], mut check: impl FnMut() -> Result<()>) -> Result<Self> {
        check()?;
        if bytes.len() > MAX_SOURCE_BYTES {
            return Err(unsupported("root document exceeds 32 MiB"));
        }
        let (doc, bin) = if bytes.starts_with(b"glTF") {
            let (json, bin) = envelope(bytes)?;
            (json::parse(json, true)?, bin)
        } else {
            if bytes.len() > MAX_JSON_BYTES {
                return Err(unsupported("glTF JSON exceeds 1 MiB"));
            }
            (json::parse(bytes, false)?, None)
        };
        validate_document(&doc)?;
        let mut resources = Vec::new();
        let mut buffers = Vec::new();
        let mut buffer_lengths = Vec::new();
        let declarations = list(&doc, "buffers")?;
        if declarations.is_empty() {
            return Err(invalid("mesh document requires buffers"));
        }
        if declarations.len() > 32 {
            return Err(unsupported("buffer count exceeds 32"));
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
            if total_declared > MAX_SOURCE_BYTES {
                return Err(unsupported("declared buffers exceed 32 MiB"));
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
                let range = bin
                    .clone()
                    .ok_or_else(|| invalid("URI-less buffer requires GLB BIN"))?;
                if declared > range.len()
                    || range.len() - declared > 3
                    || bytes[range.start + declared..range.end]
                        .iter()
                        .any(|&b| b != 0)
                {
                    return Err(invalid(
                        "BIN payload does not match buffer byteLength/padding",
                    ));
                }
                buffers.push(BufferSource::Embedded(range.start..range.start + declared));
            }
        }
        let accessors = accessor_layouts(&doc, &buffer_lengths, &mut check)?;
        let materials = materials(&doc)?;
        let (images, textures, samplers) = texture::metadata(&doc, &mut resources, &mut check)?;
        let meshes = primitives(&doc, &accessors, &mut check)?;
        let (instances, triangle_count) = scene_plan(&doc, &meshes, &mut check)?;
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
    check: &mut impl FnMut() -> Result<()>,
) -> Result<(Vec<Instance>, usize)> {
    let nodes = list(doc, "nodes")?;
    if nodes.len() > 4096 {
        return Err(unsupported("F1a node ceiling exceeded"));
    }
    let mut locals = Vec::with_capacity(nodes.len());
    let mut parents = vec![None; nodes.len()];
    for (i, node) in nodes.iter().enumerate() {
        check()?;
        locals.push(transform(node)?);
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
            for p in &meshes[mesh] {
                triangle_count += p.count / 3;
            }
            if triangle_count > MAX_TRIANGLES {
                return Err(unsupported(
                    "expanded selected-scene triangle ceiling exceeded",
                ));
            }
            instances.push((mesh, world, det, normal_matrix));
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

pub(super) fn decode(
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
    for &(mesh, world, det, normal_matrix) in &document.instances {
        for p in &document.meshes[mesh] {
            for start in (0..p.count).step_by(3) {
                if start.is_multiple_of(3072) {
                    check()?;
                }
                let mut vertices = [start, start + 1, start + 2];
                if let Some(a) = p.indices {
                    vertices = vertices.map(|i| data[a].scalar(i, 0) as usize);
                }
                if det < 0. {
                    vertices.swap(1, 2);
                }
                let mut positions = [[0.; 3]; 3];
                let mut normals = p.normal.map(|_| [[0.; 3]; 3]);
                let mut texcoords = p.texcoord.map(|_| [[0.; 2]; 3]);
                for (corner, index) in vertices.into_iter().enumerate() {
                    let pos = data[p.position].vec3(index);
                    for r in 0..3 {
                        let x = world[r][3] + (0..3).map(|c| world[r][c] * pos[c]).sum::<f64>();
                        if !x.is_finite() || x.abs() > 1_000_000. {
                            return Err(unsupported(
                                "transformed position outside local F1a domain",
                            ));
                        }
                        positions[corner][r] = x as f32;
                    }
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
                    if let (Some(a), Some(output)) = (p.texcoord, &mut texcoords) {
                        output[corner] = data[a].texcoord(index);
                    }
                }
                triangles.push(Triangle {
                    positions,
                    normals,
                    texcoords,
                    material: p.material,
                });
            }
        }
    }
    Ok(Geometry {
        triangles,
        materials: document.materials.clone(),
        images,
        textures: document.textures.clone(),
        samplers: document.samplers.clone(),
    })
}
