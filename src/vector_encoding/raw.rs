//! One exact field arena and bounded raw cursors; no Value conversion.
use super::{exact, invalid, limit, Result};
use crate::content_integrity::numbers;
use serde::{
    de::{self, DeserializeSeed, MapAccess, SeqAccess, Visitor},
    Deserializer,
};
use serde_json::value::RawValue;
use std::{borrow::Cow, cell::Cell, fmt};
pub(super) type Raw<'a> = Option<&'a RawValue>;
#[derive(Clone, Copy, Default)]
pub(super) struct Span {
    pub start: usize,
    pub len: usize,
}
pub(super) struct Field<'a> {
    pub key: Cow<'a, str>,
    pub value: &'a RawValue,
}
pub(super) struct Arena<'a> {
    pub fields: Vec<Field<'a>>,
    pub ceiling: usize,
}
pub(super) fn shape(raw: Raw<'_>, token: u8) -> Result<&RawValue> {
    let r = raw.ok_or_else(|| invalid("missing consumed field"))?;
    if r.get().as_bytes().first() != Some(&token) {
        return Err(invalid("wrong consumed JSON token shape"));
    }
    Ok(r)
}
struct Key<'p>(&'p Cell<bool>);
impl<'de> DeserializeSeed<'de> for Key<'_> {
    type Value = Cow<'de, str>;
    fn deserialize<D: Deserializer<'de>>(self, d: D) -> std::result::Result<Self::Value, D::Error> {
        d.deserialize_str(self)
    }
}
impl<'de> Visitor<'de> for Key<'_> {
    type Value = Cow<'de, str>;
    fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.write_str("admitted object key")
    }
    fn visit_borrowed_str<E: de::Error>(self, s: &'de str) -> std::result::Result<Self::Value, E> {
        Ok(Cow::Borrowed(s))
    }
    fn visit_str<E: de::Error>(self, s: &str) -> std::result::Result<Self::Value, E> {
        let mut out = String::new();
        if out.try_reserve_exact(s.len()).is_err() || out.capacity() != s.len() {
            self.0.set(true);
            return Err(E::custom("key reservation failed"));
        }
        out.push_str(s);
        Ok(Cow::Owned(out))
    }
}
struct Fields<'p, 'a> {
    arena: &'p mut Arena<'a>,
    failed: &'p Cell<bool>,
}
impl<'de> Visitor<'de> for Fields<'_, 'de> {
    type Value = Span;
    fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.write_str("admitted object")
    }
    fn visit_map<M: MapAccess<'de>>(self, mut m: M) -> std::result::Result<Span, M::Error> {
        let start = self.arena.fields.len();
        while let Some(key) = m.next_key_seed(Key(self.failed))? {
            if self.arena.fields.len() >= self.arena.ceiling {
                self.failed.set(true);
                return Err(de::Error::custom("arena exhausted"));
            }
            let value = m.next_value::<&RawValue>()?;
            self.arena.fields.push(Field { key, value });
        }
        Ok(Span {
            start,
            len: self.arena.fields.len() - start,
        })
    }
}
impl<'a> Arena<'a> {
    pub fn new(k: usize) -> Result<Self> {
        Ok(Self {
            fields: exact(k)?,
            ceiling: k,
        })
    }
    pub fn object(&mut self, raw: Raw<'a>) -> Result<Span> {
        let raw = shape(raw, b'{')?;
        let failed = Cell::new(false);
        let mut d = serde_json::Deserializer::from_str(raw.get());
        d.disable_recursion_limit();
        d.deserialize_map(Fields {
            arena: self,
            failed: &failed,
        })
        .map_err(|_| {
            if failed.get() {
                limit("field/key storage exhausted")
            } else {
                invalid("invalid admitted consumed object")
            }
        })
    }
    pub fn get(&self, span: Span, key: &str) -> Raw<'a> {
        self.fields[span.start..span.start + span.len]
            .iter()
            .find(|f| f.key == key)
            .map(|f| f.value)
    }
    pub fn fields(&self, span: Span) -> &[Field<'a>] {
        &self.fields[span.start..span.start + span.len]
    }
    pub fn owned_keys(&self) -> usize {
        self.fields
            .iter()
            .map(|f| match &f.key {
                Cow::Borrowed(_) => 0,
                Cow::Owned(s) => s.capacity(),
            })
            .sum()
    }
}
struct Array<'p, F>(&'p mut F);
impl<'de, F: FnMut(&'de RawValue) -> Result<()>> Visitor<'de> for Array<'_, F> {
    type Value = Result<usize>;
    fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.write_str("admitted array")
    }
    fn visit_seq<M: SeqAccess<'de>>(self, mut m: M) -> std::result::Result<Self::Value, M::Error> {
        let mut n = 0usize;
        while let Some(raw) = m.next_element::<&RawValue>()? {
            if let Err(e) = (self.0)(raw) {
                return Ok(Err(e));
            }
            n += 1;
        }
        Ok(Ok(n))
    }
}
pub(super) fn each<'a>(
    raw: Raw<'a>,
    mut f: impl FnMut(&'a RawValue) -> Result<()>,
) -> Result<usize> {
    let r = shape(raw, b'[')?;
    let mut d = serde_json::Deserializer::from_str(r.get());
    d.disable_recursion_limit();
    d.deserialize_seq(Array(&mut f))
        .map_err(|_| invalid("invalid admitted consumed array"))?
}
pub(super) fn count(raw: Raw<'_>) -> Result<usize> {
    each(raw, |_| Ok(()))
}
pub(super) fn refs<'a>(raw: Raw<'a>, n: usize) -> Result<Vec<&'a RawValue>> {
    let mut out = exact(n)?;
    each(raw, |r| {
        if out.len() >= n {
            return Err(invalid("array count disagreement"));
        }
        out.push(r);
        Ok(())
    })?;
    Ok(out)
}
struct Text<F>(F);
impl<'de, T, F: FnOnce(&str) -> T> Visitor<'de> for Text<F> {
    type Value = T;
    fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.write_str("admitted string")
    }
    fn visit_borrowed_str<E: de::Error>(self, s: &'de str) -> std::result::Result<T, E> {
        Ok((self.0)(s))
    }
    fn visit_str<E: de::Error>(self, s: &str) -> std::result::Result<T, E> {
        Ok((self.0)(s))
    }
}
pub(super) fn text<T>(raw: Raw<'_>, f: impl FnOnce(&str) -> T) -> Result<T> {
    let r = shape(raw, b'"')?;
    let mut d = serde_json::Deserializer::from_str(r.get());
    d.deserialize_str(Text(f))
        .map_err(|_| invalid("invalid admitted consumed string"))
}
pub(super) fn is(raw: Raw<'_>, s: &str) -> Result<bool> {
    text(raw, |v| v == s)
}
pub(super) fn uint(raw: Raw<'_>) -> Result<usize> {
    usize::try_from(numbers::domain(raw, usize::MAX as u64)?)
        .map_err(|_| invalid("integer outside host domain"))
}
pub(super) fn off(raw: Raw<'_>) -> Result<usize> {
    raw.map_or(Ok(0), |r| uint(Some(r)))
}
pub(super) fn boolean(raw: Raw<'_>) -> Result<bool> {
    match raw.map(RawValue::get) {
        Some("true") => Ok(true),
        Some("false") => Ok(false),
        _ => Err(invalid("expected boolean")),
    }
}
