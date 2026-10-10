//! One bounded JSON interpretation shared by package and payload inspection.
use super::{ValidationFailure, ValidationLimits};
use serde::de::{self, DeserializeSeed, MapAccess, SeqAccess, Visitor};
use serde_json::{Map, Value};
use std::cell::Cell;

pub(super) fn parse(bytes: &[u8]) -> Result<Value, ValidationFailure> {
    let limits = ValidationLimits::default();
    if bytes.len() > limits.json_bytes as usize {
        return Err(ValidationFailure::ResourceLimit(
            "JSON exceeds byte limit".into(),
        ));
    }
    let nodes = Cell::new(0usize);
    let limited = Cell::new(false);
    let mut decoder = serde_json::Deserializer::from_slice(bytes);
    let result = Seed {
        nodes: &nodes,
        limited: &limited,
        depth: 0,
        max_depth: limits.json_depth,
        limit: limits.document_items as usize,
    }
    .deserialize(&mut decoder)
    .and_then(|value| {
        decoder.end()?;
        Ok(value)
    });
    result.map_err(|error| {
        let message = error.to_string();
        if limited.get() {
            ValidationFailure::ResourceLimit(message)
        } else {
            ValidationFailure::InvalidInput(format!("invalid JSON: {message}"))
        }
    })
}
#[derive(Clone, Copy)]
struct Seed<'a> {
    nodes: &'a Cell<usize>,
    limited: &'a Cell<bool>,
    depth: usize,
    max_depth: u64,
    limit: usize,
}
impl<'a> Seed<'a> {
    fn child(self) -> Self {
        Self {
            depth: self.depth + 1,
            ..self
        }
    }
}
impl<'de> DeserializeSeed<'de> for Seed<'_> {
    type Value = Value;
    fn deserialize<D: de::Deserializer<'de>>(self, decoder: D) -> Result<Value, D::Error> {
        if self.depth as u64 > self.max_depth || self.nodes.get() >= self.limit {
            self.limited.set(true);
            return Err(de::Error::custom("resource limit: JSON depth/items"));
        }
        self.nodes.set(self.nodes.get() + 1);
        decoder.deserialize_any(self)
    }
}
impl<'de> Visitor<'de> for Seed<'_> {
    type Value = Value;
    fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("bounded JSON with unique keys")
    }
    fn visit_bool<E: de::Error>(self, v: bool) -> Result<Value, E> {
        Ok(v.into())
    }
    fn visit_i64<E: de::Error>(self, v: i64) -> Result<Value, E> {
        Ok(v.into())
    }
    fn visit_u64<E: de::Error>(self, v: u64) -> Result<Value, E> {
        Ok(v.into())
    }
    fn visit_f64<E: de::Error>(self, v: f64) -> Result<Value, E> {
        serde_json::Number::from_f64(v)
            .map(Value::Number)
            .ok_or_else(|| E::custom("nonfinite JSON"))
    }
    fn visit_str<E: de::Error>(self, v: &str) -> Result<Value, E> {
        Ok(v.into())
    }
    fn visit_string<E: de::Error>(self, v: String) -> Result<Value, E> {
        Ok(v.into())
    }
    fn visit_unit<E: de::Error>(self) -> Result<Value, E> {
        Ok(Value::Null)
    }
    fn visit_seq<A: SeqAccess<'de>>(self, mut access: A) -> Result<Value, A::Error> {
        let mut values = Vec::new();
        while let Some(value) = access.next_element_seed(self.child())? {
            values.push(value);
        }
        Ok(Value::Array(values))
    }
    fn visit_map<A: MapAccess<'de>>(self, mut access: A) -> Result<Value, A::Error> {
        let mut values = Map::new();
        while let Some(key) = access.next_key::<String>()? {
            if values.contains_key(&key) {
                return Err(de::Error::custom(format!("duplicate JSON key: {key}")));
            }
            let value = access.next_value_seed(self.child())?;
            values.insert(key, value);
        }
        Ok(Value::Object(values))
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rejects_duplicate_trailing_and_depth() {
        assert!(matches!(
            parse(br#"{"a":1,"a":2}"#),
            Err(ValidationFailure::InvalidInput(_))
        ));
        assert!(parse(b"{} {}").is_err());
        let deep = format!("{}0{}", "[".repeat(66), "]".repeat(66));
        assert!(matches!(
            parse(deep.as_bytes()),
            Err(ValidationFailure::ResourceLimit(_))
        ));
    }
    #[test]
    fn duplicate_key_cannot_spoof_resource_limit_category() {
        for key in ["resource limit:", "recursion limit exceeded"] {
            let bytes = format!("{{\"{key}\":1,\"{key}\":2}}");
            assert!(matches!(
                parse(bytes.as_bytes()),
                Err(ValidationFailure::InvalidInput(_))
            ));
        }
    }
    #[test]
    fn rejects_total_nodes_before_constructing_entire_document() {
        let large = format!("[{}]", vec!["0"; 65536].join(","));
        assert!(matches!(
            parse(large.as_bytes()),
            Err(ValidationFailure::ResourceLimit(_))
        ));
    }
}
