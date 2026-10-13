//! The bundled tileset schema with a linear `uniqueItems` (VS-B-10, design W6).
//!
//! The schema library's `uniqueItems` compares or hashes every item in full, so on the
//! tileset's recursive `children` arrays it costs `O(bytes × nesting depth)`. This keyword
//! takes its place in the compiled schema, so it is evaluated at the same point, and decides
//! exactly what the built-in decides:
//!
//! - at most 15 items: whether two items are equal under `jsonschema::ext::cmp::equal`;
//! - more items: whether two items are equal and also have the same value-hash input, in which
//!   numbers enter as their `f64` bits. This is the built-in's hash-set path; it does not
//!   identify `0` with `-0.0`, unlike the pairwise path.
//!
//! Candidate pairs come from two keyed structural hashes per array or object, each computed
//! once per document check and memoized by address, then are confirmed by exact comparisons.
use jsonschema::{paths::Location, Keyword, ValidationError};
use serde_json::{Map, Number, Value};
use std::{
    collections::HashMap,
    hash::{BuildHasher, Hash, Hasher, RandomState},
    sync::{Arc, Mutex},
};

/// Above this many items the built-in uses its hash set (`unique_items.rs`).
const PAIRWISE_ITEMS: usize = 15;

/// Two hashes of one value: `exact` agrees with `cmp::equal` (numbers by exact value);
/// `bits` follows the built-in's hash input (numbers by `as_f64` bits).
#[derive(Clone, Copy, PartialEq, Eq)]
struct Hashes {
    exact: u64,
    bits: u64,
}

/// Memoized hashes of the containers of the document being checked, keyed by address.
struct Memo {
    keys: RandomState,
    containers: HashMap<usize, Hashes>,
}

impl Memo {
    fn hasher(&self) -> impl Hasher {
        self.keys.build_hasher()
    }

    fn of(&mut self, value: &Value) -> Hashes {
        let mut exact = self.hasher();
        let mut bits = self.hasher();
        match value {
            Value::Null => {
                0u8.hash(&mut exact);
                0u8.hash(&mut bits);
            }
            Value::Bool(b) => {
                (1u8, *b).hash(&mut exact);
                (1u8, *b).hash(&mut bits);
            }
            Value::Number(n) => {
                2u8.hash(&mut exact);
                exact_number(n, &mut exact);
                (2u8, n.as_f64().map(f64::to_bits)).hash(&mut bits);
            }
            Value::String(s) => {
                (3u8, s).hash(&mut exact);
                (3u8, s).hash(&mut bits);
            }
            Value::Array(items) => {
                let key = value as *const Value as usize;
                if let Some(hashes) = self.containers.get(&key) {
                    return *hashes;
                }
                (4u8, items.len()).hash(&mut exact);
                (4u8, items.len()).hash(&mut bits);
                for item in items {
                    let h = self.of(item);
                    h.exact.hash(&mut exact);
                    h.bits.hash(&mut bits);
                }
                return self.keep(key, exact.finish(), bits.finish());
            }
            Value::Object(map) => {
                let key = value as *const Value as usize;
                if let Some(hashes) = self.containers.get(&key) {
                    return *hashes;
                }
                // Keys iterate in sorted order (no `preserve_order`), as `cmp::equal` zips them.
                (5u8, map.len()).hash(&mut exact);
                (5u8, map.len()).hash(&mut bits);
                for (k, v) in map {
                    let h = self.of(v);
                    (k, h.exact).hash(&mut exact);
                    (k, h.bits).hash(&mut bits);
                }
                return self.keep(key, exact.finish(), bits.finish());
            }
        }
        Hashes {
            exact: exact.finish(),
            bits: bits.finish(),
        }
    }

    fn keep(&mut self, key: usize, exact: u64, bits: u64) -> Hashes {
        let hashes = Hashes { exact, bits };
        self.containers.insert(key, hashes);
        hashes
    }
}

/// A number by exact value: integers (including integral floats within `i128`) by value, other
/// floats by bits. `-0.0` is the integer 0, as `cmp::equal` compares it.
fn exact_number(n: &Number, state: &mut impl Hasher) {
    if let Some(v) = n.as_u64() {
        (0u8, i128::from(v)).hash(state);
    } else if let Some(v) = n.as_i64() {
        (0u8, i128::from(v)).hash(state);
    } else {
        let x = n.as_f64().unwrap_or(f64::NAN);
        if x.fract() == 0.0 && x.abs() < 2f64.powi(126) {
            (0u8, x as i128).hash(state);
        } else {
            (1u8, x.to_bits()).hash(state);
        }
    }
}

/// Structural equality with numbers compared by `as_f64` bits: equal hash input in the
/// built-in's value hash.
fn same_bits(a: &Value, b: &Value) -> bool {
    match (a, b) {
        (Value::Number(a), Value::Number(b)) => {
            a.as_f64().map(f64::to_bits) == b.as_f64().map(f64::to_bits)
        }
        (Value::Array(a), Value::Array(b)) => {
            a.len() == b.len() && a.iter().zip(b).all(|(a, b)| same_bits(a, b))
        }
        (Value::Object(a), Value::Object(b)) => {
            a.len() == b.len()
                && a.iter()
                    .zip(b)
                    .all(|((ka, va), (kb, vb))| ka == kb && same_bits(va, vb))
        }
        (a, b) => a == b,
    }
}

/// Whether `items` has no duplicate, exactly as the built-in `uniqueItems` decides.
fn unique(items: &[Value], memo: &mut Memo) -> bool {
    if items.len() <= 1 {
        return true;
    }
    let pairwise = items.len() <= PAIRWISE_ITEMS;
    let mut seen: HashMap<u64, Vec<usize>> = HashMap::with_capacity(items.len());
    for (i, item) in items.iter().enumerate() {
        let h = memo.of(item);
        let bucket = seen
            .entry(if pairwise { h.exact } else { h.bits })
            .or_default();
        for &j in bucket.iter() {
            let other = &items[j];
            if jsonschema::ext::cmp::equal(item, other) && (pairwise || same_bits(item, other)) {
                return false;
            }
        }
        bucket.push(i);
    }
    true
}

struct UniqueItems {
    location: Location,
    memo: Arc<Mutex<Memo>>,
}

impl UniqueItems {
    fn holds(&self, instance: &Value) -> bool {
        let Value::Array(items) = instance else {
            return true;
        };
        let mut memo = self.memo.lock().unwrap_or_else(|e| e.into_inner());
        unique(items, &mut memo)
    }
}

impl Keyword for UniqueItems {
    fn validate<'i>(
        &self,
        instance: &'i Value,
        location: &jsonschema::paths::LazyLocation,
    ) -> Result<(), ValidationError<'i>> {
        if self.holds(instance) {
            return Ok(());
        }
        Err(ValidationError::custom(
            self.location.clone(),
            location.into(),
            instance,
            format!("{instance} has non-unique elements"),
        ))
    }

    fn is_valid(&self, instance: &Value) -> bool {
        self.holds(instance)
    }
}

/// The keyword factory: `uniqueItems: true` is the linear check; any other value never fails
/// an instance (the bundled schema uses only `true`).
fn factory(
    memo: Arc<Mutex<Memo>>,
) -> impl for<'a> Fn(
    &'a Map<String, Value>,
    &'a Value,
    Location,
) -> Result<Box<dyn Keyword>, ValidationError<'a>>
       + Send
       + Sync
       + 'static {
    move |_, value, location| {
        if value != &Value::Bool(true) {
            return Ok(Box::new(Always) as Box<dyn Keyword>);
        }
        Ok(Box::new(UniqueItems {
            location,
            memo: memo.clone(),
        }) as Box<dyn Keyword>)
    }
}

/// The compiled bundled tileset schema.
pub(super) struct TilesetSchema {
    validator: jsonschema::Validator,
    memo: Arc<Mutex<Memo>>,
}

impl TilesetSchema {
    pub(super) fn new(schema: &Value) -> Result<Self, String> {
        let memo = Arc::new(Mutex::new(Memo {
            keys: RandomState::new(),
            containers: HashMap::new(),
        }));
        let validator = jsonschema::options()
            .with_keyword("uniqueItems", factory(memo.clone()))
            .build(schema)
            .map_err(|error| error.to_string())?;
        Ok(Self { validator, memo })
    }

    /// The first schema failure of `doc`, as the library reports it.
    pub(super) fn validate(&self, doc: &Value) -> Result<(), String> {
        self.clear();
        let result = self.validator.validate(doc).map_err(|e| e.to_string());
        self.clear();
        result
    }

    fn clear(&self) {
        let mut memo = self.memo.lock().unwrap_or_else(|e| e.into_inner());
        memo.containers = HashMap::new();
    }
}

struct Always;
impl Keyword for Always {
    fn validate<'i>(
        &self,
        _: &'i Value,
        _: &jsonschema::paths::LazyLocation,
    ) -> Result<(), ValidationError<'i>> {
        Ok(())
    }
    fn is_valid(&self, _: &Value) -> bool {
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn builtin(items: &Value) -> bool {
        jsonschema::validator_for(&json!({"uniqueItems": true}))
            .unwrap()
            .is_valid(items)
    }
    fn linear(items: &Value) -> bool {
        TilesetSchema::new(&json!({"uniqueItems": true}))
            .unwrap()
            .validate(items)
            .is_ok()
    }

    /// A deterministic family of near-duplicate values, including `0` and `-0.0`.
    fn value(seed: u64, depth: u32) -> Value {
        let pick = seed % 9;
        match (depth, pick) {
            (0, _) | (_, 0) => json!(seed % 3),
            (_, 1) => json!(-0.0),
            (_, 2) => json!(0.0),
            (_, 3) => json!(1.0),
            (_, 4) => json!(format!("s{}", seed % 2)),
            (_, 5) => Value::Null,
            (_, 6) | (_, 7) => Value::Array(
                (0..seed % 3)
                    .map(|k| value(seed / 3 + k, depth - 1))
                    .collect(),
            ),
            _ => {
                let mut map = Map::new();
                for k in 0..seed % 3 {
                    map.insert(
                        format!("k{}", (seed + k) % 2),
                        value(seed / 5 + k, depth - 1),
                    );
                }
                Value::Object(map)
            }
        }
    }

    #[test]
    fn agrees_with_the_builtin_on_both_paths() {
        let mut checked = [0u32; 2];
        for seed in 0..4000u64 {
            for len in [2usize, 3, 7, 15, 16, 24] {
                let items = Value::Array(
                    (0..len as u64)
                        .map(|i| value((seed * 31 + i * 7) % 97, 3))
                        .collect(),
                );
                let expected = builtin(&items);
                assert_eq!(linear(&items), expected, "seed {seed} len {len}: {items}");
                checked[usize::from(expected)] += 1;
            }
        }
        assert!(checked[0] > 1000 && checked[1] > 1000, "{checked:?}");
    }

    #[test]
    fn signed_zero_splits_the_two_paths() {
        let mut short: Vec<Value> = vec![json!(0), json!(-0.0)];
        assert!(!builtin(&Value::Array(short.clone())));
        assert!(!linear(&Value::Array(short.clone())));
        short.extend((1..=14).map(|i| json!(i)));
        assert!(builtin(&Value::Array(short.clone())));
        assert!(linear(&Value::Array(short)));
    }

    #[test]
    fn message_matches_the_builtin() {
        let schema = json!({"properties": {"children": {"type": "array", "uniqueItems": true}}});
        let doc = json!({"children": [{"a": [1, 2]}, {"a": [1, 2.0]}]});
        let builtin = jsonschema::validator_for(&schema)
            .unwrap()
            .validate(&doc)
            .unwrap_err()
            .to_string();
        assert_eq!(
            TilesetSchema::new(&schema)
                .unwrap()
                .validate(&doc)
                .unwrap_err(),
            builtin
        );
    }
}
