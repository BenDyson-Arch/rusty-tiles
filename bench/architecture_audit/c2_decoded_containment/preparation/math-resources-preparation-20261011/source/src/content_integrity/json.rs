//! Shared structural admission of immutable JSON, without materializing numbers.
//!
//! Nodes include the root (depth zero), containers and scalar values, but not
//! object keys. Immediate children and unique decoded keys are checked before
//! descending into any child. Only one serde decoder survives at a time.
//!
//! Explicit slot/key storage uses checked arithmetic and fallible reservation.
//! Pending and immediate-child lengths together are bounded by admitted nodes;
//! their independently grown capacities are each source/node bounded. Serde's
//! private grammar/string scratch is source bounded, including the root skip
//! before descendant depth admission. Serde uses infallible internal Vec growth:
//! allocator abort there (and in the ordinary Value adapter) is not recoverable
//! through its public API. These are O(bytes + nodes) requested-storage bounds,
//! with growth overlap, not allocator-overhead or process-RSS guarantees.
use super::{FormatError, JsonLimits};
use serde::de::{self, DeserializeSeed, MapAccess, SeqAccess, Visitor};
use serde::{Deserialize, Deserializer};
use serde_json::{value::RawValue, Value};
use std::{borrow::Cow, cell::Cell, collections::HashSet, mem::size_of};

#[derive(Debug)]
pub(crate) struct CheckedDocument<'a> {
    raw: &'a RawValue,
    value_nodes: usize,
}

impl<'a> CheckedDocument<'a> {
    pub(crate) fn raw(&self) -> &'a RawValue {
        self.raw
    }

    pub(crate) fn value_nodes(&self) -> usize {
        self.value_nodes
    }
}

#[derive(Clone, Copy)]
enum AdmissionFailure {
    Depth,
    Nodes,
    Storage,
}

impl AdmissionFailure {
    fn message(self) -> &'static str {
        match self {
            Self::Depth => "JSON exceeds depth limit",
            Self::Nodes => "JSON exceeds value-node limit",
            Self::Storage => "JSON storage capacity or allocation failed",
        }
    }
}

struct State {
    nodes: Cell<usize>,
    failure: Cell<Option<AdmissionFailure>>,
    limits: JsonLimits,
    // Each actual value needs at least one source byte. Do not reserve the
    // caller's potentially huge limit for a tiny document.
    max_slots: usize,
}

impl State {
    fn fail<E: de::Error>(&self, failure: AdmissionFailure) -> E {
        self.failure.set(Some(failure));
        E::custom(failure.message())
    }

    fn charge<E: de::Error>(&self, depth: u64) -> Result<(), E> {
        if depth > self.limits.depth {
            return Err(self.fail(AdmissionFailure::Depth));
        }
        if self.nodes.get() >= self.limits.value_nodes {
            return Err(self.fail(AdmissionFailure::Nodes));
        }
        let next = self
            .nodes
            .get()
            .checked_add(1)
            .ok_or_else(|| self.fail(AdmissionFailure::Storage))?;
        self.nodes.set(next);
        Ok(())
    }

    fn error(&self, error: serde_json::Error) -> FormatError {
        match self.failure.get() {
            Some(failure) => FormatError::ResourceLimit(failure.message().into()),
            None => FormatError::InvalidInput(format!("invalid JSON: {error}")),
        }
    }

    fn reserve<T, E: de::Error>(&self, values: &mut Vec<T>, additional: usize) -> Result<(), E> {
        let required = values
            .len()
            .checked_add(additional)
            .filter(|n| *n <= self.max_slots)
            .ok_or_else(|| self.fail(AdmissionFailure::Storage))?;
        if required <= values.capacity() {
            return Ok(());
        }
        let target = values
            .capacity()
            .checked_mul(2)
            .unwrap_or(self.max_slots)
            .max(required)
            .min(self.max_slots);
        checked_storage::<T>(target).ok_or_else(|| self.fail(AdmissionFailure::Storage))?;
        values
            .try_reserve_exact(target - values.len())
            .map_err(|_| self.fail(AdmissionFailure::Storage))
    }
}

fn checked_storage<T>(count: usize) -> Option<usize> {
    count
        .checked_mul(size_of::<T>())
        .filter(|bytes| *bytes <= isize::MAX as usize)
}

type Slot<'a> = (&'a RawValue, u64);

struct Child<'a, 'de> {
    state: &'a State,
    parent_depth: u64,
    children: &'a mut Vec<Slot<'de>>,
}

impl<'de> DeserializeSeed<'de> for Child<'_, 'de> {
    type Value = ();

    fn deserialize<D: de::Deserializer<'de>>(self, decoder: D) -> Result<(), D::Error> {
        let depth = self
            .parent_depth
            .checked_add(1)
            .ok_or_else(|| self.state.fail(AdmissionFailure::Depth))?;
        self.state.charge(depth)?;
        // Admission and a retained slot are secured before serde scans/skips
        // the child's complete raw substring. It is never charged again.
        self.state.reserve(self.children, 1)?;
        let raw = <&RawValue>::deserialize(decoder)?;
        self.children.push((raw, depth));
        Ok(())
    }
}

struct Container<'a> {
    state: &'a State,
    depth: u64,
}

impl<'de> Visitor<'de> for Container<'_> {
    type Value = Vec<Slot<'de>>;

    fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("JSON container with unique decoded keys")
    }

    fn visit_seq<A: SeqAccess<'de>>(self, mut access: A) -> Result<Self::Value, A::Error> {
        let mut children = Vec::new();
        while access
            .next_element_seed(Child {
                state: self.state,
                parent_depth: self.depth,
                children: &mut children,
            })?
            .is_some()
        {}
        Ok(children)
    }

    fn visit_map<A: MapAccess<'de>>(self, mut access: A) -> Result<Self::Value, A::Error> {
        let mut keys = HashSet::new();
        let mut children = Vec::new();
        while let Some(key) = access.next_key_seed(Key(self.state))? {
            if keys.contains(&key) {
                return Err(de::Error::custom("duplicate decoded JSON key"));
            }
            let count = keys
                .len()
                .checked_add(1)
                .ok_or_else(|| self.state.fail(AdmissionFailure::Storage))?;
            checked_storage::<Cow<'de, str>>(count)
                .ok_or_else(|| self.state.fail(AdmissionFailure::Storage))?;
            // HashSet's fallible API checks its bucket/control-byte capacity
            // arithmetic too; the key bytes are separately bounded by source.
            keys.try_reserve(1)
                .map_err(|_| self.state.fail::<A::Error>(AdmissionFailure::Storage))?;
            keys.insert(key);
            access.next_value_seed(Child {
                state: self.state,
                parent_depth: self.depth,
                children: &mut children,
            })?;
        }
        Ok(children)
    }
}

struct Key<'a>(&'a State);

impl<'de> DeserializeSeed<'de> for Key<'_> {
    type Value = Cow<'de, str>;

    fn deserialize<D: de::Deserializer<'de>>(self, decoder: D) -> Result<Self::Value, D::Error> {
        decoder.deserialize_str(self)
    }
}

impl<'de> Visitor<'de> for Key<'_> {
    type Value = Cow<'de, str>;

    fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("valid Unicode JSON key")
    }

    fn visit_borrowed_str<E: de::Error>(self, value: &'de str) -> Result<Self::Value, E> {
        Ok(Cow::Borrowed(value))
    }

    fn visit_str<E: de::Error>(self, value: &str) -> Result<Self::Value, E> {
        checked_storage::<u8>(value.len()).ok_or_else(|| self.0.fail(AdmissionFailure::Storage))?;
        let mut key = String::new();
        key.try_reserve_exact(value.len())
            .map_err(|_| self.0.fail::<E>(AdmissionFailure::Storage))?;
        key.push_str(value);
        Ok(Cow::Owned(key))
    }
}

struct ScalarString;

impl<'de> Visitor<'de> for ScalarString {
    type Value = ();

    fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("valid Unicode JSON string")
    }

    fn visit_str<E: de::Error>(self, _: &str) -> Result<(), E> {
        Ok(())
    }
}

pub(crate) fn admit(bytes: &[u8], limits: JsonLimits) -> Result<CheckedDocument<'_>, FormatError> {
    if bytes.len() > limits.bytes {
        return Err(FormatError::ResourceLimit("JSON exceeds byte limit".into()));
    }
    let state = State {
        nodes: Cell::new(0),
        failure: Cell::new(None),
        limits,
        max_slots: limits.value_nodes.min(bytes.len()),
    };
    state
        .charge::<serde_json::Error>(0)
        .map_err(|e| state.error(e))?;
    // RawValue's root grammar skip is iterative in the pinned serde. It may
    // scan all source and allocate O(bytes) nesting scratch before logical
    // descendant depth admission. from_slice also rejects trailing tokens.
    let root = serde_json::from_slice::<&RawValue>(bytes).map_err(|e| state.error(e))?;
    let mut pending = Vec::new();
    state
        .reserve::<Slot<'_>, serde_json::Error>(&mut pending, 1)
        .map_err(|e| state.error(e))?;
    pending.push((root, 0));
    while let Some((raw, depth)) = pending.pop() {
        match raw.get().as_bytes()[0] {
            b'[' | b'{' => {
                let children = {
                    let mut decoder = serde_json::Deserializer::from_str(raw.get());
                    decoder.disable_recursion_limit();
                    let children = decoder
                        .deserialize_any(Container {
                            state: &state,
                            depth,
                        })
                        .map_err(|e| state.error(e))?;
                    decoder.end().map_err(|e| state.error(e))?;
                    children
                }; // Drop decoder, scalar scratch and key set before descent.
                state
                    .reserve::<Slot<'_>, serde_json::Error>(&mut pending, children.len())
                    .map_err(|e| state.error(e))?;
                pending.extend(children.into_iter().rev());
            }
            b'"' => {
                // Raw skip alone permits unpaired surrogate escapes. Decode
                // every scalar string, retaining no string, to enforce Unicode.
                let mut decoder = serde_json::Deserializer::from_str(raw.get());
                decoder
                    .deserialize_str(ScalarString)
                    .map_err(|e| state.error(e))?;
                decoder.end().map_err(|e| state.error(e))?;
            }
            _ => {} // Numbers remain their actual borrowed source lexemes.
        }
    }
    Ok(CheckedDocument {
        raw: root,
        value_nodes: state.nodes.get(),
    })
}

pub(crate) fn parse(bytes: &[u8], limits: JsonLimits) -> Result<Value, FormatError> {
    let checked = admit(bytes, limits)?;
    // All source syntax/Unicode/duplicates/limits are already decided. A
    // subsequent failure is this finite owned representation's refusal, never
    // a reinterpretation of valid source as malformed (e.g. 1e400).
    let mut decoder = serde_json::Deserializer::from_str(checked.raw().get());
    decoder.disable_recursion_limit();
    Value::deserialize(&mut decoder).map_err(|_| {
        FormatError::Unsupported("JSON exceeds finite owned-document representation".into())
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn limits(bytes: usize, depth: u64, value_nodes: usize) -> JsonLimits {
        JsonLimits {
            bytes,
            depth,
            value_nodes,
        }
    }

    fn small() -> JsonLimits {
        limits(1024, 8, 64)
    }

    #[test]
    fn inclusive_bytes_depth_and_root_nodes() {
        let bytes = br#"{"a":[0]}"#;
        let exact = limits(bytes.len(), 2, 3);
        let checked = admit(bytes, exact).unwrap();
        assert_eq!(checked.value_nodes(), 3);
        for one_over in [
            limits(bytes.len() - 1, 2, 3),
            limits(bytes.len(), 1, 3),
            limits(bytes.len(), 2, 2),
        ] {
            assert!(matches!(
                admit(bytes, one_over),
                Err(FormatError::ResourceLimit(_))
            ));
        }
        assert_eq!(admit(b"0", limits(1, 0, 1)).unwrap().value_nodes(), 1);
        assert!(matches!(
            admit(b"0", limits(1, 0, 0)),
            Err(FormatError::ResourceLimit(_))
        ));
        // No child exists: calculating its depth must not overflow/reject.
        assert_eq!(admit(b"[]", limits(2, 0, 1)).unwrap().value_nodes(), 1);
        assert!(matches!(
            admit(b"[0]", limits(3, 0, 2)),
            Err(FormatError::ResourceLimit(_))
        ));
        assert!(admit(b"[0]", limits(3, u64::MAX, 2)).is_ok());
    }

    #[test]
    fn typed_invalid_grammar_and_duplicate_keys() {
        for bytes in [
            &b"{} {}"[..],
            b"NaN",
            b"1e+",
            b"01",
            b"[0,]",
            b"{\"a\":0,}",
            br#"{"a":0,"a":1}"#,
            br#"{"a":0,"\u0061":1}"#,
            br#"{"resource limit:":0,"resource limit:":1}"#,
            br#"{"recursion limit exceeded":0,"recursion limit exceeded":1}"#,
            br#"{"a":[{"b":0,"b":1}]}"#,
        ] {
            assert!(
                matches!(admit(bytes, small()), Err(FormatError::InvalidInput(_))),
                "{bytes:?}"
            );
        }
        for bytes in [&b""[..], b" ", b"{", b"[", b"nullx"] {
            assert!(matches!(
                admit(bytes, small()),
                Err(FormatError::InvalidInput(_))
            ));
        }
    }

    #[test]
    fn validates_unicode_in_keys_and_scalar_strings() {
        for bytes in [
            &br#""\uD800""#[..],
            br#""\uDC00""#,
            br#""\uD800\u0041""#,
            br#"{"extras":"\uD800"}"#,
            br#"{"\uD800":0}"#,
            &b"\"\xff\""[..],
        ] {
            assert!(matches!(
                admit(bytes, small()),
                Err(FormatError::InvalidInput(_))
            ));
        }
        for bytes in [
            &br#""\uD83D\uDE00""#[..],
            br#""\"\\\n\t\u0041""#,
            br#"{"\uD83D\uDE00":"\uD83D\uDE00"}"#,
            "{\"é\":\"😀\"}".as_bytes(),
        ] {
            assert!(admit(bytes, small()).is_ok(), "{bytes:?}");
        }
        // Decoded equality applies across borrowed UTF-8 and escaped keys.
        assert!(matches!(
            admit("{\"é\":0,\"\\u00e9\":1}".as_bytes(), small()),
            Err(FormatError::InvalidInput(_))
        ));
    }

    #[test]
    fn raw_numeric_grammar_and_owned_representation_are_distinct() {
        for token in ["1e400", "-1e400"] {
            assert!(admit(token.as_bytes(), small()).is_ok());
            assert!(matches!(
                parse(token.as_bytes(), small()),
                Err(FormatError::Unsupported(_))
            ));
        }
        let bytes = br#"[1,-1,1.0,1e0,{"$serde_json::private::Number":"1"}]"#;
        let value = parse(bytes, small()).unwrap();
        let values = value.as_array().unwrap();
        assert!(values[0].as_number().unwrap().is_u64());
        assert!(values[1].as_number().unwrap().is_i64());
        assert!(values[2].as_number().unwrap().is_f64());
        assert!(values[3].as_number().unwrap().is_f64());
        assert!(values[4].is_object());
        assert_eq!(values[4]["$serde_json::private::Number"], "1");
    }

    #[test]
    fn root_and_retained_children_borrow_source_and_preserve_presence() {
        let bytes =
            br#"{"absentElsewhere":0,"present":null,"fraction":1.0000000000000001,"opaque":1e400}"#;
        let checked = admit(bytes, small()).unwrap();
        let records: std::collections::BTreeMap<String, &RawValue> =
            serde_json::from_str(checked.raw().get()).unwrap();
        assert!(!records.contains_key("absent"));
        assert_eq!(records["present"].get(), "null");
        assert_eq!(records["fraction"].get(), "1.0000000000000001");
        assert_eq!(records["opaque"].get(), "1e400");
        let start = bytes.as_ptr() as usize;
        let end = start.checked_add(bytes.len()).unwrap();
        for raw in std::iter::once(checked.raw()).chain(records.values().copied()) {
            let pointer = raw.get().as_ptr() as usize;
            assert!(pointer >= start);
            assert!(pointer.checked_add(raw.get().len()).unwrap() <= end);
        }
    }

    #[test]
    fn immediate_container_faults_precede_descendant_faults() {
        // The first child's depth-two value is never descended into: the
        // duplicate in the already admitted root container wins first.
        assert!(matches!(
            admit(br#"{"a":[0],"a":0}"#, limits(64, 1, 8)),
            Err(FormatError::InvalidInput(_))
        ));
        // Both immediate children are charged before scalar-string descent.
        assert!(matches!(
            admit(br#"["\uD800",0]"#, limits(64, 1, 2)),
            Err(FormatError::ResourceLimit(_))
        ));
        // Reversing the LIFO insertion preserves left-to-right descent.
        assert!(matches!(
            admit(br#"["\uD800",[0]]"#, limits(64, 1, 4)),
            Err(FormatError::InvalidInput(_))
        ));
        assert!(matches!(
            admit(br#"[[0],"\uD800"]"#, limits(64, 1, 4)),
            Err(FormatError::ResourceLimit(_))
        ));
        // Initial root grammar admission still comes before container work.
        assert!(matches!(
            admit(br#"{"a":[0],"a":01}"#, limits(64, 0, 1)),
            Err(FormatError::InvalidInput(_))
        ));
    }

    #[test]
    fn iterative_depth_and_width_are_limited_without_recursive_descent() {
        let deep = format!("{}0{}", "[".repeat(4096), "]".repeat(4096));
        assert!(matches!(
            admit(deep.as_bytes(), limits(deep.len(), 64, 4097)),
            Err(FormatError::ResourceLimit(_))
        ));
        let depth64 = format!("{}0{}", "[".repeat(64), "]".repeat(64));
        assert_eq!(
            admit(depth64.as_bytes(), limits(depth64.len(), 64, 65))
                .unwrap()
                .value_nodes(),
            65
        );
        let wide = format!("[{}]", ["0"; 1024].join(","));
        assert_eq!(
            admit(wide.as_bytes(), limits(wide.len(), 1, 1025))
                .unwrap()
                .value_nodes(),
            1025
        );
        assert!(matches!(
            admit(wide.as_bytes(), limits(wide.len(), 1, 1024)),
            Err(FormatError::ResourceLimit(_))
        ));
    }

    #[test]
    fn storage_overflow_is_typed_and_small_sources_do_not_reserve_huge_limits() {
        assert!(checked_storage::<Slot<'_>>(usize::MAX).is_none());
        assert!(checked_storage::<Slot<'_>>(isize::MAX as usize).is_none());
        let state = State {
            nodes: Cell::new(0),
            failure: Cell::new(None),
            limits: limits(usize::MAX, u64::MAX, usize::MAX),
            max_slots: 1,
        };
        let mut slots = Vec::<Slot<'_>>::new();
        let error = state
            .reserve::<Slot<'_>, serde_json::Error>(&mut slots, 2)
            .unwrap_err();
        assert!(matches!(state.error(error), FormatError::ResourceLimit(_)));
        assert!(admit(b"0", limits(usize::MAX, u64::MAX, usize::MAX)).is_ok());
    }
}
