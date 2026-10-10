//! Fixed consumed record views. Every field points into one admitted JSON source.
use super::{invalid, limit, FormatResult};
use serde::{
    de::{self, DeserializeSeed, MapAccess, SeqAccess, Visitor},
    Deserialize, Deserializer,
};
use serde_json::value::RawValue;
use std::{cell::Cell, fmt, marker::PhantomData};
pub(super) type Raw<'a> = Option<&'a RawValue>;

// Do not derive Option<&RawValue> deserialization: that would erase present null.
macro_rules! record {
    ($name:ident { $($field:ident => $key:literal),* $(,)? }) => {
        #[derive(Default)]
        pub(super) struct $name<'a> { $(pub $field: Raw<'a>,)* }
        impl<'de> Deserialize<'de> for $name<'de> {
            fn deserialize<D: de::Deserializer<'de>>(deserializer:D)->Result<Self,D::Error> {
                struct Fields;
                impl<'de> Visitor<'de> for Fields {
                    type Value=$name<'de>;
                    fn expecting(&self,f:&mut fmt::Formatter)->fmt::Result {f.write_str("object record")}
                    fn visit_map<M:MapAccess<'de>>(self,mut map:M)->Result<Self::Value,M::Error> {
                        let mut record=$name::default();
                        while let Some(key)=map.next_key::<String>()? {
                            // Raw admission has already checked duplicate decoded keys.
                            let value=map.next_value::<&'de RawValue>()?;
                            match key.as_str() { $($key=>record.$field=Some(value),)* _=>{} }
                        }
                        Ok(record)
                    }
                }
                deserializer.deserialize_map(Fields)
            }
        }
    }
}
record!(Root {asset=>"asset", used=>"extensionsUsed", required=>"extensionsRequired", buffers=>"buffers", views=>"bufferViews", accessors=>"accessors", meshes=>"meshes", images=>"images", textures=>"textures", samplers=>"samplers", materials=>"materials", extensions=>"extensions"});
record!(Asset {version=>"version", minimum=>"minVersion"});
record!(Buffer {length=>"byteLength", uri=>"uri", extensions=>"extensions"});
record!(View {buffer=>"buffer", length=>"byteLength", offset=>"byteOffset", stride=>"byteStride", target=>"target", extensions=>"extensions"});
record!(Meshopt {buffer=>"buffer", offset=>"byteOffset", length=>"byteLength", count=>"count", stride=>"byteStride", mode=>"mode", filter=>"filter"});
record!(Accessor {count=>"count", component=>"componentType", shape=>"type", view=>"bufferView", offset=>"byteOffset", normalized=>"normalized", min=>"min", max=>"max", sparse=>"sparse"});
record!(Mesh {primitives=>"primitives"});
record!(Primitive {mode=>"mode", indices=>"indices", material=>"material", attributes=>"attributes", extensions=>"extensions"});
record!(Polygon {count=>"count", offsets=>"indicesOffsets", loops=>"loopIndices", loop_offsets=>"loopIndicesOffsets"});
record!(Image {uri=>"uri", view=>"bufferView"});
record!(Texture {source=>"source", sampler=>"sampler"});
record!(Material {pbr=>"pbrMetallicRoughness", normal=>"normalTexture", occlusion=>"occlusionTexture", emissive=>"emissiveTexture"});
record!(Pbr {base=>"baseColorTexture", metallic=>"metallicRoughnessTexture"});
record!(TextureInfo {index=>"index"});
record!(Extensions {meshopt=>"EXT_meshopt_compression", polygon=>"EXT_mesh_polygon", draco=>"KHR_draco_mesh_compression", metadata=>"EXT_structural_metadata"});
record!(Fallback {fallback=>"fallback"});
record!(Metadata {uri=>"schemaUri"});
record!(FeatureTable {batch_length=>"BATCH_LENGTH"});

pub(super) fn record<'a, T: Deserialize<'a>>(raw: Raw<'a>) -> FormatResult<T> {
    let raw = raw.ok_or_else(|| invalid("missing object record"))?;
    serde_json::from_str(raw.get()).map_err(|e| invalid(format!("invalid object record: {e}")))
}
pub(super) fn optional_record<'a, T: Deserialize<'a> + Default>(raw: Raw<'a>) -> FormatResult<T> {
    raw.map_or_else(|| Ok(T::default()), |v| record(Some(v)))
}
pub(super) fn string(raw: Raw<'_>) -> FormatResult<String> {
    let raw = raw.ok_or_else(|| invalid("expected string"))?;
    serde_json::from_str(raw.get()).map_err(|_| invalid("expected string"))
}
pub(super) fn boolean(raw: Raw<'_>) -> FormatResult<bool> {
    match raw.map(RawValue::get) {
        Some("true") => Ok(true),
        Some("false") => Ok(false),
        _ => Err(invalid("expected boolean")),
    }
}
pub(super) fn is_null(raw: Raw<'_>) -> bool {
    raw.is_some_and(|v| v.get() == "null")
}

pub(super) fn reserve<T>(
    values: &mut Vec<T>,
    additional: usize,
    ceiling: usize,
) -> FormatResult<()> {
    let needed = values
        .len()
        .checked_add(additional)
        .ok_or_else(|| limit("record slot count overflow"))?;
    if needed > ceiling {
        return Err(limit("record slots exceed admitted node ceiling"));
    }
    if needed <= values.capacity() {
        return Ok(());
    }
    let requested = needed.max(values.capacity().saturating_mul(2)).min(ceiling);
    let bytes = requested
        .checked_mul(std::mem::size_of::<T>())
        .ok_or_else(|| limit("record slot byte overflow"))?;
    if bytes > isize::MAX as usize {
        return Err(limit("record slot allocation exceeds host range"));
    }
    values
        .try_reserve_exact(requested - values.len())
        .map_err(|_| limit("record slot allocation failed"))
}
pub(super) fn push<T>(values: &mut Vec<T>, value: T, ceiling: usize) -> FormatResult<()> {
    reserve(values, 1, ceiling)?;
    values.push(value);
    Ok(())
}

struct Array<'a, T> {
    ceiling: usize,
    limited: &'a Cell<bool>,
    marker: PhantomData<T>,
}
impl<'de, T: Deserialize<'de>> Visitor<'de> for Array<'_, T> {
    type Value = Vec<T>;
    fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.write_str("array")
    }
    fn visit_seq<S: SeqAccess<'de>>(self, mut seq: S) -> Result<Vec<T>, S::Error> {
        let mut values = Vec::new();
        while let Some(value) = seq.next_element::<T>()? {
            push(&mut values, value, self.ceiling).map_err(|_| {
                self.limited.set(true);
                de::Error::custom("record allocation admission")
            })?;
        }
        Ok(values)
    }
}
impl<'de, T: Deserialize<'de>> DeserializeSeed<'de> for Array<'_, T> {
    type Value = Vec<T>;
    fn deserialize<D: de::Deserializer<'de>>(self, d: D) -> Result<Vec<T>, D::Error> {
        d.deserialize_seq(self)
    }
}
pub(super) fn array<'a, T: Deserialize<'a>>(raw: Raw<'a>, ceiling: usize) -> FormatResult<Vec<T>> {
    let Some(raw) = raw else {
        return Ok(Vec::new());
    };
    let limited = Cell::new(false);
    let mut d = serde_json::Deserializer::from_str(raw.get());
    let result = Array {
        ceiling,
        limited: &limited,
        marker: PhantomData,
    }
    .deserialize(&mut d);
    result.map_err(|e| {
        if limited.get() {
            limit("record allocation admission failed")
        } else {
            invalid(format!("expected array: {e}"))
        }
    })
}

// Actual arbitrary attribute-name pairs are gathered in one object pass.
pub(super) struct Pairs<'a>(pub Vec<(String, &'a RawValue)>);
struct PairVisitor<'a> {
    ceiling: usize,
    limited: &'a Cell<bool>,
}
impl<'de> Visitor<'de> for PairVisitor<'_> {
    type Value = Pairs<'de>;
    fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.write_str("object")
    }
    fn visit_map<M: MapAccess<'de>>(self, mut map: M) -> Result<Self::Value, M::Error> {
        let mut pairs = Vec::new();
        while let Some(key) = map.next_key::<String>()? {
            let value = map.next_value::<&'de RawValue>()?;
            push(&mut pairs, (key, value), self.ceiling).map_err(|_| {
                self.limited.set(true);
                de::Error::custom("pair allocation admission")
            })?;
        }
        Ok(Pairs(pairs))
    }
}
pub(super) fn pairs<'a>(raw: Raw<'a>, ceiling: usize) -> FormatResult<Pairs<'a>> {
    let raw = raw.ok_or_else(|| invalid("expected object"))?;
    let limited = Cell::new(false);
    let mut d = serde_json::Deserializer::from_str(raw.get());
    d.deserialize_map(PairVisitor {
        ceiling,
        limited: &limited,
    })
    .map_err(|e| {
        if limited.get() {
            limit("pair allocation admission failed")
        } else {
            invalid(format!("expected object: {e}"))
        }
    })
}
