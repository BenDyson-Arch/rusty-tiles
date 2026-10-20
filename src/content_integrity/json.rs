//! Shared structural admission of immutable JSON, without materializing numbers.
//!
//! Nodes include the root (depth zero), containers and scalar values, but not
//! object keys. The failure order is fixed: the byte ceiling, the root node, one
//! whole-document grammar skip, then containers in document order. For each
//! container, every member key is decoded and checked unique and every immediate
//! child is charged (depth, node) before any child is descended into; scalar
//! strings are decoded when the walk reaches them.
//!
//! Work is O(bytes). After the grammar skip, one structural scan records the value
//! tree (per value: source offset, member-key offset and recorded-subtree extent),
//! and the per-container walk replays that record instead of re-deserializing each
//! container's raw text. A document with more values than `value_nodes` cannot be
//! admitted; for it, one counting scan first fixes how many children of each
//! container the walk can reach before the node ceiling, and a second recording
//! scan keeps only those, so the record never exceeds `value_nodes + 1` values.
//!
//! Explicit record/slot/key storage uses checked arithmetic and fallible
//! reservation, bounded by min(admitted nodes + 1, source bytes). Serde's private
//! grammar/string scratch is source bounded, including the root skip before
//! descendant depth admission, and only one serde decoder survives at a time.
//! Serde uses infallible internal Vec growth: allocator abort there (and in the
//! ordinary Value adapter) is not recoverable through its public API. These are
//! O(bytes + nodes) requested-storage bounds, with growth overlap, not
//! allocator-overhead or process-RSS guarantees.
use super::{FormatError, JsonLimits};
use serde::de::{self, Visitor};
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

/// Every failure is carried as a serde error; `State::error` classifies it.
type Fault = serde_json::Error;

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

    fn error(&self, error: Fault) -> FormatError {
        match self.failure.get() {
            Some(failure) => FormatError::ResourceLimit(failure.message().into()),
            None => FormatError::InvalidInput(format!("invalid JSON: {error}")),
        }
    }

    fn reserve<T>(&self, values: &mut Vec<T>, additional: usize) -> Result<(), Fault> {
        self.reserve_within(values, additional, self.max_slots)
    }

    fn reserve_within<T>(
        &self,
        values: &mut Vec<T>,
        additional: usize,
        max: usize,
    ) -> Result<(), Fault> {
        let required = values
            .len()
            .checked_add(additional)
            .filter(|n| *n <= max)
            .ok_or_else(|| self.fail(AdmissionFailure::Storage))?;
        if required <= values.capacity() {
            return Ok(());
        }
        let target = values
            .capacity()
            .checked_mul(2)
            .unwrap_or(max)
            .max(required)
            .min(max);
        checked_storage::<T>(target).ok_or_else(|| self.fail(AdmissionFailure::Storage))?;
        values
            .try_reserve_exact(target - values.len())
            .map_err(|_| self.fail(AdmissionFailure::Storage))
    }

    /// A broken record invariant. Unreachable; typed rather than a panic.
    fn inconsistent(&self) -> Fault {
        debug_assert!(false, "JSON admission record invariant broken");
        self.fail(AdmissionFailure::Storage)
    }
}

fn checked_storage<T>(count: usize) -> Option<usize> {
    count
        .checked_mul(size_of::<T>())
        .filter(|bytes| *bytes <= isize::MAX as usize)
}

/// A recorded value. The children of a container node `n` are the nodes from
/// `n + 1` to `nodes[n].end`, each followed by its own recorded subtree.
#[derive(Clone, Copy, Debug)]
struct Node {
    /// Offset of the value's first byte in the admitted root text.
    start: usize,
    /// Offset of the member key's opening quote; `NO_KEY` outside objects.
    key: usize,
    /// One past the last node of this value's recorded subtree.
    end: usize,
}

const NO_KEY: usize = usize::MAX;

/// A pending walk slot: a recorded node and its depth.
type Slot = (usize, u64);

#[derive(Clone, Copy)]
enum Token {
    Open(usize),
    Close,
    Key(usize),
    Value(usize),
}

/// Visits the structural tokens of grammar-admitted JSON text in document
/// order. It keeps no nesting state: a string followed by `:` is a member key.
fn scan<E>(source: &[u8], mut visit: impl FnMut(Token) -> Result<(), E>) -> Result<(), E> {
    let mut at = 0;
    while let Some(&byte) = source.get(at) {
        match byte {
            b'[' | b'{' => {
                visit(Token::Open(at))?;
                at += 1;
            }
            b']' | b'}' => {
                visit(Token::Close)?;
                at += 1;
            }
            b',' | b':' | b' ' | b'\t' | b'\n' | b'\r' => at += 1,
            b'"' => {
                let next = whitespace_end(source, string_end(source, at + 1));
                if source.get(next) == Some(&b':') {
                    visit(Token::Key(at))?;
                    at = next + 1;
                } else {
                    visit(Token::Value(at))?;
                    at = next;
                }
            }
            _ => {
                visit(Token::Value(at))?;
                at = scalar_end(source, at + 1);
            }
        }
    }
    Ok(())
}

/// One past the closing quote of a string whose body starts at `at`.
fn string_end(source: &[u8], mut at: usize) -> usize {
    while let Some(&byte) = source.get(at) {
        match byte {
            b'"' => return at + 1,
            b'\\' => at += 2,
            _ => at += 1,
        }
    }
    source.len()
}

fn whitespace_end(source: &[u8], mut at: usize) -> usize {
    while matches!(source.get(at), Some(b' ' | b'\t' | b'\n' | b'\r')) {
        at += 1;
    }
    at
}

fn scalar_end(source: &[u8], mut at: usize) -> usize {
    while !matches!(
        source.get(at),
        None | Some(b',' | b']' | b'}' | b' ' | b'\t' | b'\n' | b'\r')
    ) {
        at += 1;
    }
    at
}

enum Halt {
    /// An unlimited record would exceed `value_nodes` values.
    Full,
    Fault(Fault),
}

impl From<Fault> for Halt {
    fn from(fault: Fault) -> Self {
        Self::Fault(fault)
    }
}

/// Records the value tree in document order. Without `quotas` every value is
/// recorded, stopping with `Halt::Full` past `value_nodes` values. With `quotas`
/// (per container in document order), a container records at most its quota of
/// children; values below an unrecorded value are not recorded.
fn record(state: &State, text: &str, quotas: Option<&[usize]>) -> Result<Vec<Node>, Halt> {
    let limit = state.limits.value_nodes;
    let cap = match quotas {
        None => limit,
        Some(_) => limit.saturating_add(1),
    }
    .min(text.len());
    let mut nodes: Vec<Node> = Vec::new();
    // Recorded open containers with their remaining child quota, then a count of
    // unrecorded open containers, which always nest inside the recorded ones.
    let mut open: Vec<(usize, usize)> = Vec::new();
    let mut hidden = 0usize;
    let mut containers = 0usize;
    let mut key = NO_KEY;
    scan(text.as_bytes(), |token| {
        let (start, container) = match token {
            Token::Key(at) => {
                key = at;
                return Ok(());
            }
            Token::Close => {
                if hidden > 0 {
                    hidden -= 1;
                } else if let Some((node, _)) = open.pop() {
                    let end = nodes.len();
                    nodes[node].end = end;
                }
                return Ok(());
            }
            Token::Value(at) => (at, false),
            Token::Open(at) => (at, true),
        };
        // A key token always immediately precedes its member value.
        let key = std::mem::replace(&mut key, NO_KEY);
        let ordinal = containers;
        containers += usize::from(container);
        let recorded = hidden == 0
            && match open.last_mut() {
                None => nodes.is_empty(),
                Some((_, quota)) => {
                    let within = *quota > 0;
                    *quota = quota.saturating_sub(1);
                    within
                }
            };
        if !recorded {
            hidden += usize::from(container);
            return Ok(());
        }
        if nodes.len() >= cap {
            return Err(Halt::Full);
        }
        state.reserve_within(&mut nodes, 1, cap)?;
        let node = nodes.len();
        nodes.push(Node {
            start,
            key,
            end: node + 1,
        });
        if container {
            let quota = quotas.map_or(usize::MAX, |q| q.get(ordinal).copied().unwrap_or(0));
            state.reserve_within(&mut open, 1, cap)?;
            open.push((node, quota));
        }
        Ok(())
    })?;
    Ok(nodes)
}

/// For a document with more than `value_nodes` values: per container in document
/// order, how many of its children the legacy walk reaches, including the child
/// whose key check or charge is the last event before the node ceiling.
///
/// The walk processes containers in document order and charges all of a
/// container's children when it reaches the container, so `1 + Σ children` of the
/// earlier containers have been charged when container `g` is reached. Only a
/// document-order prefix of containers can be reached (a later container sees at
/// least as many earlier children), so counting stops tracking new containers
/// once the counted children alone reach the ceiling.
fn reachable_children(state: &State, text: &str) -> Result<Vec<usize>, Fault> {
    let limit = state.limits.value_nodes;
    let max = limit.min(text.len());
    let mut counts = Vec::new();
    let mut open = Vec::new();
    let mut hidden = 0usize;
    let mut counted = 0usize;
    scan(text.as_bytes(), |token| {
        let container = match token {
            Token::Key(_) => return Ok(()),
            Token::Close => {
                if hidden > 0 {
                    hidden -= 1;
                } else {
                    open.pop();
                }
                return Ok(());
            }
            Token::Value(_) => false,
            Token::Open(_) => true,
        };
        if hidden > 0 {
            hidden += usize::from(container);
            return Ok(());
        }
        if let Some(&parent) = open.last() {
            counts[parent] += 1;
            counted += 1;
        }
        if container {
            // `1 + counted` values precede this container's children in the walk.
            if counted < limit {
                state.reserve_within(&mut counts, 1, max)?;
                state.reserve_within(&mut open, 1, max)?;
                open.push(counts.len());
                counts.push(0);
            } else {
                hidden += 1;
            }
        }
        Ok(())
    })?;
    let mut charged = 1usize;
    for count in &mut counts {
        let children = *count;
        *count = children.min(limit.saturating_add(1).saturating_sub(charged));
        charged = charged.saturating_add(children);
    }
    Ok(counts)
}

/// The legacy per-container walk over the record: pop a value; for a container
/// decode and check each member key, then charge each child, before any child is
/// pushed; decode a scalar string. Children are pushed reversed (LIFO) so that
/// values are reached in document order.
fn replay(state: &State, text: &str, nodes: &[Node]) -> Result<(), Fault> {
    let source = text.as_bytes();
    let mut pending: Vec<Slot> = Vec::new();
    state.reserve(&mut pending, 1)?;
    pending.push((0, 0));
    let mut children: Vec<Slot> = Vec::new();
    while let Some((index, depth)) = pending.pop() {
        let node = nodes[index];
        match source[node.start] {
            open @ (b'[' | b'{') => {
                children.clear();
                let mut keys = HashSet::new();
                let mut child = index + 1;
                while child < node.end {
                    let member = nodes[child];
                    if open == b'{' {
                        let key = member_key(state, text, member.key)?;
                        if keys.contains(&key) {
                            return Err(de::Error::custom("duplicate decoded JSON key"));
                        }
                        let count = keys
                            .len()
                            .checked_add(1)
                            .ok_or_else(|| state.fail::<Fault>(AdmissionFailure::Storage))?;
                        checked_storage::<Cow<'_, str>>(count)
                            .ok_or_else(|| state.fail::<Fault>(AdmissionFailure::Storage))?;
                        // HashSet's fallible API checks its bucket/control-byte
                        // capacity arithmetic too; key bytes are source bounded.
                        keys.try_reserve(1)
                            .map_err(|_| state.fail::<Fault>(AdmissionFailure::Storage))?;
                        keys.insert(key);
                    }
                    let child_depth = depth
                        .checked_add(1)
                        .ok_or_else(|| state.fail::<Fault>(AdmissionFailure::Depth))?;
                    state.charge::<Fault>(child_depth)?;
                    state.reserve(&mut children, 1)?;
                    children.push((child, child_depth));
                    child = member.end;
                }
                drop(keys); // The key set never outlives its container.
                state.reserve(&mut pending, children.len())?;
                pending.extend(children.iter().rev().copied());
            }
            b'"' => {
                // Raw skip alone permits unpaired surrogate escapes. Decode
                // every scalar string, retaining no string, to enforce Unicode.
                let mut decoder = serde_json::Deserializer::from_str(&text[node.start..]);
                decoder.deserialize_str(ScalarString)?;
            }
            _ => {} // Numbers remain their actual borrowed source lexemes.
        }
    }
    Ok(())
}

fn member_key<'de>(state: &State, text: &'de str, at: usize) -> Result<Cow<'de, str>, Fault> {
    let source = text.get(at..).ok_or_else(|| state.inconsistent())?;
    let mut decoder = serde_json::Deserializer::from_str(source);
    decoder.deserialize_str(Key(state))
}

struct Key<'a>(&'a State);

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
    state.charge::<Fault>(0).map_err(|e| state.error(e))?;
    // (1) RawValue's root grammar skip is iterative in the pinned serde. It may
    // scan all source and allocate O(bytes) nesting scratch before logical
    // descendant depth admission. from_slice also rejects trailing tokens.
    let root = serde_json::from_slice::<&RawValue>(bytes).map_err(|e| state.error(e))?;
    let text = root.get();
    // (2) Record the value tree from the admitted grammar.
    let (nodes, truncated) = match record(&state, text, None) {
        Ok(nodes) => (nodes, false),
        Err(Halt::Fault(e)) => return Err(state.error(e)),
        Err(Halt::Full) => {
            let quotas = reachable_children(&state, text).map_err(|e| state.error(e))?;
            match record(&state, text, Some(&quotas)) {
                Ok(nodes) => (nodes, true),
                Err(Halt::Fault(e)) => return Err(state.error(e)),
                Err(Halt::Full) => return Err(state.error(state.inconsistent())),
            }
        }
    };
    // (3) Charge and check in the legacy per-container order.
    replay(&state, text, &nodes).map_err(|e| state.error(e))?;
    if truncated {
        // A cut record exists only for more than value_nodes values, so the
        // replay must already have failed.
        return Err(state.error(state.inconsistent()));
    }
    Ok(CheckedDocument {
        raw: root,
        value_nodes: state.nodes.get(),
    })
}

pub(crate) fn parse(bytes: &[u8], limits: JsonLimits) -> Result<Value, FormatError> {
    parse_counted(bytes, limits).map(|(value, _)| value)
}

/// [`parse`], with the admitted value-node count.
pub(crate) fn parse_counted(
    bytes: &[u8],
    limits: JsonLimits,
) -> Result<(Value, usize), FormatError> {
    let checked = admit(bytes, limits)?;
    let nodes = checked.value_nodes();
    // All source syntax/Unicode/duplicates/limits are already decided. A
    // subsequent failure is this finite owned representation's refusal, never
    // a reinterpretation of valid source as malformed (e.g. 1e400).
    let mut decoder = serde_json::Deserializer::from_str(checked.raw().get());
    decoder.disable_recursion_limit();
    let value = Value::deserialize(&mut decoder).map_err(|_| {
        FormatError::Unsupported("JSON exceeds finite owned-document representation".into())
    })?;
    Ok((value, nodes))
}

/// The pre-C2-WB implementation (develop d1874ef, `json.rs:37-302`), retained
/// verbatim as the differential reference. It re-deserializes each container's
/// raw text when popped: O(depth x bytes).
#[cfg(test)]
mod legacy {
    use super::{CheckedDocument, FormatError, JsonLimits};
    use serde::de::{self, DeserializeSeed, MapAccess, SeqAccess, Visitor};
    use serde::{Deserialize, Deserializer};
    use serde_json::value::RawValue;
    use std::{borrow::Cow, cell::Cell, collections::HashSet, mem::size_of};

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

        fn reserve<T, E: de::Error>(
            &self,
            values: &mut Vec<T>,
            additional: usize,
        ) -> Result<(), E> {
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

        fn deserialize<D: de::Deserializer<'de>>(
            self,
            decoder: D,
        ) -> Result<Self::Value, D::Error> {
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
            checked_storage::<u8>(value.len())
                .ok_or_else(|| self.0.fail(AdmissionFailure::Storage))?;
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

    pub(crate) fn admit(
        bytes: &[u8],
        limits: JsonLimits,
    ) -> Result<CheckedDocument<'_>, FormatError> {
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
        assert!(checked_storage::<Slot>(usize::MAX).is_none());
        assert!(checked_storage::<Node>(isize::MAX as usize).is_none());
        let state = State {
            nodes: Cell::new(0),
            failure: Cell::new(None),
            limits: limits(usize::MAX, u64::MAX, usize::MAX),
            max_slots: 1,
        };
        let mut slots = Vec::<Slot>::new();
        let error = state.reserve(&mut slots, 2).unwrap_err();
        assert!(matches!(state.error(error), FormatError::ResourceLimit(_)));
        assert!(admit(b"0", limits(usize::MAX, u64::MAX, usize::MAX)).is_ok());
    }

    fn tokens(text: &str) -> Vec<String> {
        let mut out = Vec::new();
        scan::<()>(text.as_bytes(), |token| {
            out.push(match token {
                Token::Open(at) => format!("open@{at}"),
                Token::Close => "close".into(),
                Token::Key(at) => format!("key@{at}"),
                Token::Value(at) => format!("value@{at}"),
            });
            Ok(())
        })
        .unwrap();
        out
    }

    #[test]
    fn structural_scan_needs_no_nesting_state() {
        assert_eq!(
            tokens(r#"{ "a" :"b\":", "c\\":[1,"d" ,true] }"#),
            [
                "open@0", "key@2", "value@7", "key@15", "open@21", "value@22", "value@24",
                "value@29", "close", "close"
            ]
        );
        assert_eq!(tokens("-1.5e+3"), ["value@0"]);
        assert_eq!(tokens(r#""\u0041""#), ["value@0"]);
        assert_eq!(tokens("[ ]"), ["open@0", "close"]);
    }

    fn state(value_nodes: usize, len: usize) -> State {
        State {
            nodes: Cell::new(0),
            failure: Cell::new(None),
            limits: limits(usize::MAX, u64::MAX, value_nodes),
            max_slots: value_nodes.min(len),
        }
    }

    /// Records a document the way `admit` does and returns the record length and
    /// whether it was cut.
    fn recorded(text: &str, value_nodes: usize) -> (usize, bool) {
        let state = state(value_nodes, text.len());
        match record(&state, text, None) {
            Ok(nodes) => (nodes.len(), false),
            Err(Halt::Full) => {
                let quotas = reachable_children(&state, text).ok().unwrap();
                match record(&state, text, Some(&quotas)) {
                    Ok(nodes) => (nodes.len(), true),
                    _ => panic!("quota record must fit"),
                }
            }
            Err(Halt::Fault(_)) => panic!("record fault"),
        }
    }

    #[test]
    fn records_hold_at_most_value_nodes_plus_one_values() {
        // Later root siblings are charged before the first child's children.
        assert_eq!(recorded("[[0,0,0],1,1]", 4), (5, true));
        assert_eq!(recorded("[[0,0,0],1,1]", 7), (7, false));
        let wide = format!("[{}]", ["0"; 10_000].join(","));
        assert_eq!(recorded(&wide, 10), (11, true));
        // Each nesting level gains its later children only after its deeper
        // levels close: lower-bound prefixes alone would record O(depth x V).
        let mut deep = String::new();
        for _ in 0..40 {
            deep.push('[');
        }
        deep.push('0');
        for _ in 0..40 {
            deep.push_str(&format!(",{}]", ["0"; 30].join(",")));
        }
        for value_nodes in [1, 2, 10, 39, 40, 41, 100, 1000] {
            let (len, cut) = recorded(&deep, value_nodes);
            assert!(cut && len <= value_nodes + 1, "{value_nodes}: {len}");
            let legacy = legacy::admit(deep.as_bytes(), limits(usize::MAX, 64, value_nodes));
            let new = admit(deep.as_bytes(), limits(usize::MAX, 64, value_nodes));
            assert_eq!(format!("{legacy:?}"), format!("{new:?}"));
        }
        let (len, cut) = recorded(&deep, 1241);
        assert_eq!((len, cut), (1241, false));
    }

    #[test]
    fn quotas_cover_exactly_the_reachable_children() {
        // Root has 3 children (charged 2..4); with 4 nodes the walk stops at the
        // first child of the inner array, after checking its key.
        let four = state(4, 64);
        let quotas = reachable_children(&four, "[[0,0,0],1,1]").unwrap();
        assert_eq!(quotas, [3, 1]);
        // Root: 2 children, charged 2 and 3; below, only the first member of
        // "a" is reached (its key, then its overflowing charge). "c" is never
        // reached, so counting does not track it.
        let three = state(3, 64);
        let quotas = reachable_children(&three, r#"{"a":{"b":0,"b":1},"c":[0]}"#).unwrap();
        assert_eq!(quotas, [2, 1]);
    }
}

/// Old/new differential over a generated grammar corpus, adversarial and
/// two-defect documents at depths 0-65, explicit token forms and committed or
/// supplied JSON corpora. Key: (outcome category, limit name or failure kind,
/// value_nodes and admitted raw span). ResourceLimit messages are compared
/// exactly; InvalidInput messages are compared without serde's "at line L column
/// C" suffix, which the old code reported relative to a re-parsed container.
#[cfg(test)]
mod differential {
    use super::{admit, legacy, scan, CheckedDocument, FormatError, JsonLimits, Token};
    use std::collections::{BTreeMap, HashSet};
    use std::hash::{Hash, Hasher};

    type Admit = for<'a> fn(&'a [u8], JsonLimits) -> Result<CheckedDocument<'a>, FormatError>;

    const DEPTH: &str = "JSON exceeds depth limit";
    const NODES: &str = "JSON exceeds value-node limit";
    const BYTES: &str = "JSON exceeds byte limit";
    const DUPLICATE: &str = "invalid JSON: duplicate decoded JSON key";

    fn limits(bytes: usize, depth: u64, value_nodes: usize) -> JsonLimits {
        JsonLimits {
            bytes,
            depth,
            value_nodes,
        }
    }

    const GENEROUS: JsonLimits = JsonLimits {
        bytes: usize::MAX,
        depth: u64::MAX,
        value_nodes: usize::MAX,
    };
    const PRODUCTION: JsonLimits = JsonLimits {
        bytes: 8 << 20,
        depth: 64,
        value_nodes: 65_536,
    };

    #[derive(Clone, Debug, PartialEq, Eq)]
    enum Outcome {
        Admitted {
            value_nodes: usize,
            start: usize,
            len: usize,
        },
        Refused {
            category: &'static str,
            message: String,
        },
    }

    impl Outcome {
        fn refused(&self, category: &str, message: &str) -> bool {
            matches!(self, Self::Refused { category: c, message: m } if *c == category && m == message)
        }
    }

    fn without_position(message: &str) -> String {
        if let Some(at) = message.rfind(" at line ") {
            let mut parts = message[at + 9..].splitn(2, " column ");
            let numeric = |s: Option<&str>| {
                s.is_some_and(|s| !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit()))
            };
            if numeric(parts.next()) && numeric(parts.next()) {
                return message[..at].to_string();
            }
        }
        message.to_string()
    }

    fn observe(
        bytes: &[u8],
        result: Result<CheckedDocument<'_>, FormatError>,
    ) -> (Outcome, Option<String>) {
        match result {
            Ok(document) => {
                let raw = document.raw().get();
                let outcome = Outcome::Admitted {
                    value_nodes: document.value_nodes(),
                    start: raw.as_ptr() as usize - bytes.as_ptr() as usize,
                    len: raw.len(),
                };
                (outcome, None)
            }
            Err(FormatError::InvalidInput(message)) => (
                Outcome::Refused {
                    category: "InvalidInput",
                    message: without_position(&message),
                },
                Some(message),
            ),
            Err(FormatError::ResourceLimit(message)) => (
                Outcome::Refused {
                    category: "ResourceLimit",
                    message: message.clone(),
                },
                Some(message),
            ),
            Err(FormatError::Unsupported(message)) => (
                Outcome::Refused {
                    category: "Unsupported",
                    message: message.clone(),
                },
                Some(message),
            ),
        }
    }

    fn preview(bytes: &[u8]) -> String {
        let text = String::from_utf8_lossy(&bytes[..bytes.len().min(400)]);
        format!("{text:?} ({} bytes)", bytes.len())
    }

    #[derive(Default)]
    struct Tally {
        documents: HashSet<u64>,
        cases: usize,
        outcomes: BTreeMap<String, usize>,
        exact_messages: usize,
        position_only: usize,
    }

    impl Tally {
        fn check(&mut self, bytes: &[u8], limits: JsonLimits) -> Outcome {
            let (old, old_message) = observe(bytes, legacy::admit(bytes, limits));
            let (new, new_message) = observe(bytes, admit(bytes, limits));
            assert_eq!(
                old,
                new,
                "old/new disagree on {} with {limits:?}",
                preview(bytes)
            );
            self.cases += 1;
            let mut hasher = std::collections::hash_map::DefaultHasher::new();
            bytes.hash(&mut hasher);
            self.documents.insert(hasher.finish());
            let key = match &new {
                Outcome::Admitted { .. } => "Admitted".to_string(),
                Outcome::Refused { category, message } if *category == "ResourceLimit" => {
                    format!("ResourceLimit({message})")
                }
                Outcome::Refused { category, .. } => category.to_string(),
            };
            *self.outcomes.entry(key).or_default() += 1;
            if old_message == new_message {
                self.exact_messages += 1;
            } else {
                self.position_only += 1;
            }
            new
        }

        fn profiles(&mut self, bytes: &[u8]) -> Vec<Outcome> {
            let (depth, nodes) = metrics(bytes);
            profiles(bytes.len(), depth, nodes)
                .into_iter()
                .map(|limits| self.check(bytes, limits))
                .collect()
        }

        fn merge(&mut self, other: Tally) {
            self.documents.extend(other.documents);
            self.cases += other.cases;
            self.exact_messages += other.exact_messages;
            self.position_only += other.position_only;
            for (key, count) in other.outcomes {
                *self.outcomes.entry(key).or_default() += count;
            }
        }

        fn report(&self, name: &str) {
            eprintln!(
                "json-differential {name}: documents={} cases={} disagreements=0 \
                 exact-messages={} position-only-message-differences={} outcomes={:?}",
                self.documents.len(),
                self.cases,
                self.exact_messages,
                self.position_only,
                self.outcomes
            );
        }
    }

    /// Maximum value depth and value count, from the structural scan. Used only
    /// to place limits at and one past a document's own extent.
    fn metrics(bytes: &[u8]) -> (u64, usize) {
        let (mut open, mut depth, mut nodes) = (0u64, 0u64, 0usize);
        let _ = scan::<()>(bytes, |token| {
            match token {
                Token::Open(_) | Token::Value(_) => {
                    depth = depth.max(open);
                    nodes += 1;
                    open += u64::from(matches!(token, Token::Open(_)));
                }
                Token::Close => open = open.saturating_sub(1),
                Token::Key(_) => {}
            }
            Ok(())
        });
        (depth, nodes)
    }

    /// Generous, production, exact, one past each of bytes/depth/value nodes
    /// (JsonLimits' value_nodes is the validator's `document_items`), one inside.
    fn profiles(len: usize, depth: u64, nodes: usize) -> Vec<JsonLimits> {
        let mut out = vec![GENEROUS, PRODUCTION, limits(len, depth, nodes)];
        if len > 0 {
            out.push(limits(len - 1, depth, nodes));
        }
        if depth > 0 {
            out.push(limits(len, depth - 1, nodes));
        }
        if nodes > 0 {
            out.push(limits(len, depth, nodes - 1));
        }
        out.push(limits(len + 1, depth + 1, nodes + 1));
        out
    }

    struct Rng(u64);

    impl Rng {
        fn next(&mut self) -> u64 {
            self.0 = self.0.wrapping_add(0x9e37_79b9_7f4a_7c15);
            let mut z = self.0;
            z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
            z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
            z ^ (z >> 31)
        }

        fn below(&mut self, n: u64) -> u64 {
            self.next() % n.max(1)
        }

        fn chance(&mut self, num: u64, den: u64) -> bool {
            self.below(den) < num
        }

        fn pick<'a, T: ?Sized>(&mut self, items: &[&'a T]) -> &'a T {
            items[self.below(items.len() as u64) as usize]
        }
    }

    const NUMBERS: &[&str] = &[
        "0",
        "-0",
        "1",
        "-1",
        "7",
        "1.0",
        "-0.5",
        "1e0",
        "1E2",
        "1e+2",
        "1e-2",
        "-1.5e10",
        "0e0",
        "-0.0e-0",
        "1.0000000000000001",
        "1e400",
        "-1e400",
        "123456789012345678901234567890",
        "18446744073709551615",
        "18446744073709551616",
        "-9223372036854775808",
        "-9223372036854775809",
        "0.0000001",
        "2.2250738585072014e-308",
        "5e-324",
        "1.7976931348623157e308",
    ];
    const BAD_NUMBERS: &[&str] = &[
        "01",
        "-01",
        "00",
        "1.",
        "-",
        ".5",
        "1e",
        "1e+",
        "+1",
        "NaN",
        "-NaN",
        "Infinity",
        "-Infinity",
        "0x1",
        "1.5.2",
        "1e5e5",
        "--1",
    ];
    const LITERALS: &[&str] = &["true", "false", "null"];
    const BAD_LITERALS: &[&str] = &["tru", "nul", "True", "NULL", "nan", "undefined"];
    /// Grammar- and Unicode-valid JSON string tokens.
    const STRINGS: &[&str] = &[
        r#""""#,
        r#""a""#,
        r#""\u0041""#,
        r#""\uD83D\uDE00""#,
        r#""\ud83d\ude00""#,
        r#""\uDBFF\uDFFF""#,
        r#""\uD800\uDC00""#,
        r#""\"\\\/\b\f\n\r\t""#,
        "\"é😀\"",
        r#""\u00e9""#,
        r#""\u0000""#,
        r#""\uFFFF""#,
        r#""a b""#,
        r#""[{]}:,""#,
        r#"" : ""#,
        r#""\\""#,
        r#""\\\"""#,
    ];
    /// Grammar-valid for the raw skip, Unicode-invalid when decoded.
    const BAD_STRINGS: &[&str] = &[
        r#""\uD800""#,
        r#""\uDC00""#,
        r#""\uD800\u0041""#,
        r#""\uD800x""#,
        r#""\uDBFF""#,
        r#""x\uDFFF""#,
        r#""\uD800\uD800""#,
        r#""\udc00\ud800""#,
        r#""\uD83D""#,
        r#""\uDE00\uD83D""#,
        r#""\uD800\n""#,
    ];
    /// Grammar-invalid string tokens.
    const SYNTAX_STRINGS: &[&[u8]] = &[
        br#""\x""#,
        br#""\u12""#,
        br#""\u12G4""#,
        b"\"a\x01b\"",
        b"\"tab\there\"",
        b"\"new\nline\"",
        b"\"\xff\"",
        b"\"\xc0\x80\"",
        b"\"\xe2\x82\"",
        b"\"\xed\xa0\x80\"",
        b"\"unterminated",
        br#""\""#,
    ];
    const WHITESPACE: &[&str] = &["", " ", "\n", "\t", "\r", "\r\n", "  \t\n "];
    const BAD_WHITESPACE: &[&[u8]] = &[
        b"\x0b",
        b"\x0c",
        b"\x00",
        "\u{a0}".as_bytes(),
        "\u{feff}".as_bytes(),
        "\u{2028}".as_bytes(),
    ];
    /// Keys that collide after decoding.
    const KEYS: &[&str] = &[
        r#""a""#,
        r#""\u0061""#,
        r#""b""#,
        r#""\u0062""#,
        "\"é\"",
        r#""\u00e9""#,
        r#""\uD83D\uDE00""#,
        "\"😀\"",
        r#""""#,
        r#""\u0000""#,
    ];

    #[derive(Clone, Copy)]
    struct Mix {
        duplicates: bool,
        unicode: bool,
        syntax: bool,
    }

    struct Gen<'r> {
        rng: &'r mut Rng,
        out: Vec<u8>,
        mix: Mix,
        keys: usize,
    }

    impl Gen<'_> {
        fn push(&mut self, token: &str) {
            self.out.extend_from_slice(token.as_bytes());
        }

        fn ws(&mut self) {
            let ws = self.rng.pick(WHITESPACE);
            self.push(ws);
        }

        fn scalar(&mut self) {
            let syntax = self.mix.syntax && self.rng.chance(1, 40);
            match self.rng.below(10) {
                0..=3 if syntax => {
                    let token = self.rng.pick(BAD_NUMBERS);
                    self.push(token);
                }
                0..=3 => {
                    let token = self.rng.pick(NUMBERS);
                    self.push(token);
                }
                4..=6 if syntax => {
                    let token = self.rng.pick(SYNTAX_STRINGS);
                    self.out.extend_from_slice(token);
                }
                4..=6 if self.mix.unicode && self.rng.chance(1, 6) => {
                    let token = self.rng.pick(BAD_STRINGS);
                    self.push(token);
                }
                4..=6 => {
                    let token = self.rng.pick(STRINGS);
                    self.push(token);
                }
                7 if syntax => {
                    let token = self.rng.pick(BAD_LITERALS);
                    self.push(token);
                }
                7 => {
                    let token = self.rng.pick(LITERALS);
                    self.push(token);
                }
                8 => self.push("[]"),
                _ => self.push("{}"),
            }
        }

        fn key(&mut self) {
            if self.mix.duplicates && self.rng.chance(1, 3) {
                let key = self.rng.pick(KEYS);
                self.push(key);
            } else if self.mix.unicode && self.rng.chance(1, 12) {
                let key = self.rng.pick(BAD_STRINGS);
                self.push(key);
            } else {
                self.keys += 1;
                let key = if self.rng.chance(1, 4) {
                    format!("\"\\u006b{}\"", self.keys)
                } else {
                    format!("\"k{}\"", self.keys)
                };
                self.push(&key);
            }
        }

        /// A value at `depth`; the spine reaches `target` exactly, other
        /// subtrees nest at most `budget` further levels (and never past it).
        fn value(&mut self, depth: u64, target: u64, spine: bool, budget: u64) {
            let nest = depth < target && (spine || budget > 0);
            if !nest || (!spine && self.rng.chance(1, 2)) {
                self.scalar();
                return;
            }
            let object = self.rng.chance(1, 2);
            let width = self.rng.below(4) + u64::from(spine);
            let on = if spine {
                self.rng.below(width)
            } else {
                u64::MAX
            };
            self.push(if object { "{" } else { "[" });
            for i in 0..width {
                if i > 0 {
                    self.push(",");
                }
                self.ws();
                if object {
                    self.key();
                    self.ws();
                    self.push(":");
                    self.ws();
                }
                let budget = match (i == on, spine) {
                    (true, _) => 0,
                    (false, true) => self.rng.below(3),
                    (false, false) => budget - 1,
                };
                self.value(depth + 1, target, i == on, budget);
                self.ws();
            }
            self.push(if object { "}" } else { "]" });
        }
    }

    fn generate(rng: &mut Rng, target: u64, mix: Mix) -> Vec<u8> {
        let mut gen = Gen {
            rng,
            out: Vec::new(),
            mix,
            keys: 0,
        };
        gen.ws();
        gen.value(0, target, true, 0);
        gen.ws();
        gen.out
    }

    fn mutate(rng: &mut Rng, bytes: &mut Vec<u8>) {
        let at = |rng: &mut Rng, bytes: &[u8]| rng.below(bytes.len() as u64 + 1) as usize;
        let find = |rng: &mut Rng, bytes: &[u8], set: &[u8]| {
            let hits: Vec<usize> = (0..bytes.len())
                .filter(|&i| set.contains(&bytes[i]))
                .collect();
            (!hits.is_empty()).then(|| hits[rng.below(hits.len() as u64) as usize])
        };
        match rng.below(10) {
            0 => {
                let i = at(rng, bytes.as_slice());
                bytes.truncate(i);
            }
            1 => {
                let i = at(rng, bytes.as_slice());
                let set = [
                    b',', b']', b'}', b'[', b'{', b'"', b':', b'x', 0xff, 0x01, b'\\',
                ];
                bytes.insert(i, set[rng.below(set.len() as u64) as usize]);
            }
            2 if !bytes.is_empty() => {
                let i = rng.below(bytes.len() as u64) as usize;
                bytes.remove(i);
            }
            3 => {
                let tail = rng.pick(&[" 1", "x", "{}", ",", "]", " ", "\u{0}"]);
                bytes.extend_from_slice(tail.as_bytes());
            }
            4 => {
                if let Some(i) = find(rng, bytes.as_slice(), b"0123456789") {
                    bytes.insert(i, b'0');
                }
            }
            5 => {
                let i = at(rng, bytes.as_slice());
                let ws = rng.pick(BAD_WHITESPACE);
                bytes.splice(i..i, ws.iter().copied());
            }
            6 => {
                if let Some(i) = find(rng, bytes.as_slice(), b"\"") {
                    bytes.insert(i + 1, 0xff);
                }
            }
            7 => {
                if let Some(i) = find(rng, bytes.as_slice(), b"\"") {
                    bytes.insert(i + 1, 0x01);
                }
            }
            8 => {
                if let Some(i) = find(rng, bytes.as_slice(), b"]}") {
                    bytes.insert(i, b',');
                }
            }
            _ => {
                if let Some(i) = find(rng, bytes.as_slice(), b":") {
                    bytes.remove(i);
                }
            }
        }
    }

    fn wrap(depth: usize, inner: &str) -> String {
        format!("{}{inner}{}", "[".repeat(depth), "]".repeat(depth))
    }

    #[test]
    fn random_documents_agree_at_depths_0_to_65() {
        let mut tally = Tally::default();
        let mut rng = Rng(0xc20b_5eed);
        let mixes = [
            (false, false, false),
            (true, false, false),
            (false, true, false),
            (false, false, true),
            (true, true, false),
            (true, true, true),
        ];
        for target in 0..=65u64 {
            for (duplicates, unicode, syntax) in mixes {
                let mix = Mix {
                    duplicates,
                    unicode,
                    syntax,
                };
                for _ in 0..4 {
                    let mut bytes = generate(&mut rng, target, mix);
                    if syntax && rng.chance(1, 2) {
                        mutate(&mut rng, &mut bytes);
                    }
                    let outcomes = tally.profiles(&bytes);
                    let (depth, nodes) = metrics(&bytes);
                    for _ in 0..4 {
                        let random = limits(
                            rng.below(bytes.len() as u64 + 2) as usize,
                            rng.below(depth + 2),
                            rng.below(nodes as u64 + 2) as usize,
                        );
                        tally.check(&bytes, random);
                    }
                    if !(duplicates || unicode || syntax) {
                        // Clean documents: the metrics are exact; at each limit
                        // they are admitted and one past each limit names it.
                        assert_eq!(depth, target);
                        let exact = &outcomes[2];
                        assert!(
                            matches!(exact, Outcome::Admitted { value_nodes, .. } if *value_nodes == nodes),
                            "{exact:?} {}",
                            preview(&bytes)
                        );
                        assert!(outcomes[3].refused("ResourceLimit", BYTES));
                        let mut next = 4;
                        if depth > 0 {
                            assert!(outcomes[next].refused("ResourceLimit", DEPTH));
                            next += 1;
                        }
                        assert!(outcomes[next].refused("ResourceLimit", NODES));
                    }
                }
            }
        }
        tally.report("random");
    }

    fn adversarial(d: usize) -> Vec<String> {
        let siblings = |inner: &str| {
            let mut s = inner.to_string();
            for _ in 0..d {
                s = format!("[1,{s},1]");
            }
            s
        };
        let objects = |inner: &str| format!("{}{inner}{}", r#"{"a":"#.repeat(d), "}".repeat(d));
        let alternate = |inner: &str| {
            let mut s = inner.to_string();
            for i in 0..d {
                s = if i % 2 == 0 {
                    format!("[{s}]")
                } else {
                    format!(r#"{{"k":{s}}}"#)
                };
            }
            s
        };
        let spaced = |inner: &str| format!("{}{inner}{}", " [ \n\t".repeat(d), " \r\n] ".repeat(d));
        let wide = format!("{}0{}{}", "[".repeat(d), ",0".repeat(200), "]".repeat(d));
        vec![
            wrap(d, "0"),
            wrap(d, "[]"),
            wrap(d, "{}"),
            wrap(d, r#""\uD800""#),
            wrap(d, r#""\uD83D\uDE00""#),
            wrap(d, r#"{"a":0,"a":1}"#),
            wrap(d, r#"{"\uD800":0}"#),
            objects("0"),
            objects(r#"{"a":0,"\u0061":1}"#),
            format!(r#"{{"a":{},"a":0}}"#, wrap(d, "0")),
            format!(r#"{{"a":{},"b":0,"\uDC00":1}}"#, objects("0")),
            alternate("1e400"),
            alternate(r#""\uDC00""#),
            siblings("0"),
            siblings(r#""\uD800""#),
            siblings(r#"{"a":0,"a":0}"#),
            spaced("-0.0e-0"),
            wide,
            format!("{} 01", wrap(d, "0")),
            format!("{},", wrap(d, r#"{"a":0,"a":1}"#)),
            wrap(d, "0")[..(2 * d + 1).saturating_sub(1)].to_string(),
        ]
    }

    #[test]
    fn adversarial_documents_agree_at_depths_0_to_65() {
        let mut tally = Tally::default();
        for d in 0..=65usize {
            for doc in adversarial(d) {
                let bytes = doc.as_bytes();
                tally.profiles(bytes);
                let (_, nodes) = metrics(bytes);
                for depth in [
                    0,
                    1,
                    d as u64 - u64::from(d > 0),
                    d as u64,
                    d as u64 + 1,
                    64,
                    65,
                ] {
                    for value_nodes in [
                        1,
                        2,
                        3,
                        nodes / 2,
                        nodes.saturating_sub(1),
                        nodes,
                        nodes + 1,
                    ] {
                        tally.check(bytes, limits(bytes.len(), depth, value_nodes));
                    }
                }
            }
        }
        tally.report("adversarial");
    }

    /// §9.A2 two-defect inputs, at every depth 0-65 (65 is one past `json_depth`).
    #[test]
    fn two_defect_documents_keep_the_legacy_order() {
        let mut tally = Tally::default();
        for d in 0..=65usize {
            // (a) An early duplicate key plus more root siblings than value_nodes
            // allows: the root's children are charged before any descent.
            let dup = wrap(d, r#"{"a":1,"a":2}"#);
            let a = format!("[{dup}{}]", ",1".repeat(8));
            for depth in [64, u64::MAX] {
                let outcome = tally.check(a.as_bytes(), limits(a.len(), depth, 9));
                assert!(outcome.refused("ResourceLimit", NODES), "{d}: {outcome:?}");
            }
            for value_nodes in [1, 2, 8, 10, 11, d + 9, d + 10, d + 11, d + 12, d + 13] {
                tally.check(a.as_bytes(), limits(a.len(), 64, value_nodes));
            }
            // The literal form, a duplicate in the first root child at depth 1.
            let literal = format!(r#"[{{"a":1,"a":2}}{}]"#, ",1".repeat(d + 1));
            let outcome = tally.check(literal.as_bytes(), limits(literal.len(), 64, d + 2));
            assert!(outcome.refused("ResourceLimit", NODES), "{d}: {outcome:?}");

            // (b) A late syntax error plus an early node, depth or duplicate-key
            // (or Unicode) defect: the grammar skip decides first.
            let earlies = [
                wrap(d, r#"{"a":1,"a":2}"#),
                wrap(d + 2, "0"),
                wrap(d, &["0"; 40].join(",")),
                wrap(d, r#""\uD800""#),
            ];
            let lates = [
                ",01]",
                ",[1,]]",
                "]x",
                ",\"\\x\"]",
                ",tru]",
                "",
                ",1 2]",
                ",{\"a\" 1}]",
            ];
            for early in &earlies {
                for late in lates {
                    let b = format!("[{early}{late}");
                    for (depth, value_nodes) in [(d as u64, 3), (0, 1), (64, 2), (u64::MAX, 4)] {
                        let outcome =
                            tally.check(b.as_bytes(), limits(b.len(), depth, value_nodes));
                        let Outcome::Refused { category, message } = &outcome else {
                            panic!("{d}: {outcome:?}");
                        };
                        assert_eq!(*category, "InvalidInput", "{d}: {b} {outcome:?}");
                        assert!(
                            message != DUPLICATE && !message.contains("surrogate"),
                            "{message}"
                        );
                    }
                }
            }

            // (c) A duplicate key in a container plus a depth violation below a
            // later sibling, and the reverse, the pair at depth d.
            let x = r#"{"a":0,"a":1}"#;
            let y = wrap(4, "0");
            let limit = d as u64 + 3;
            for (pair, expected) in [
                (format!("[{x},{y}]"), ("InvalidInput", DUPLICATE)),
                (format!("[{y},{x}]"), ("ResourceLimit", DEPTH)),
            ] {
                let c = wrap(d, &pair);
                let outcome = tally.check(c.as_bytes(), limits(c.len(), limit, 1 << 20));
                assert!(
                    outcome.refused(expected.0, expected.1),
                    "{d}: {c} {outcome:?}"
                );
                for value_nodes in [d + 2, d + 3, d + 4, d + 5, d + 6, d + 7] {
                    tally.check(c.as_bytes(), limits(c.len(), limit, value_nodes));
                }
            }

            // (d) Depth over the limit plus value nodes over the limit, in both
            // document orders.
            let deep = format!("[{}]", wrap(3, "0"));
            let broad = format!("[{}]", ["0"; 11].join(","));
            let value_nodes = d + 9;
            for (pair, expected) in [
                (format!("[{deep},{broad}]"), DEPTH),
                (format!("[{broad},{deep}]"), NODES),
            ] {
                let e = wrap(d, &pair);
                let outcome = tally.check(e.as_bytes(), limits(e.len(), d as u64 + 2, value_nodes));
                assert!(
                    outcome.refused("ResourceLimit", expected),
                    "{d}: {e} {outcome:?}"
                );
                for depth in [d as u64 + 1, d as u64 + 3, d as u64 + 4, d as u64 + 5] {
                    for value_nodes in [value_nodes - 1, value_nodes + 1, value_nodes + 12] {
                        tally.check(e.as_bytes(), limits(e.len(), depth, value_nodes));
                    }
                }
            }

            // (e) An invalid surrogate in a deep string plus a node-limit
            // overflow at a shallower level.
            let bad = wrap(d, r#""\uD800""#);
            let root_overflow = format!("[{bad}{}]", ",0".repeat(10));
            let outcome = tally.check(root_overflow.as_bytes(), limits(usize::MAX, 70, 10));
            assert!(outcome.refused("ResourceLimit", NODES), "{d}: {outcome:?}");
            let later_overflow = format!("[[{bad}],[{}]]", ["0"; 10].join(","));
            let earlier_overflow = format!("[[{}],[{bad}]]", ["0"; 10].join(","));
            for doc in [&root_overflow, &later_overflow, &earlier_overflow] {
                for value_nodes in [3, 4, 5, 6, 10, d + 3, d + 4, d + 5, d + 14, d + 15, d + 16] {
                    for depth in [70, d as u64, d as u64 + 1, d as u64 + 2] {
                        tally.check(doc.as_bytes(), limits(doc.len(), depth, value_nodes));
                    }
                }
            }
            let outcome = tally.check(later_overflow.as_bytes(), limits(usize::MAX, 70, d + 5));
            // serde reports a leading surrogate followed by `"` this way.
            let surrogate = "invalid JSON: unexpected end of hex escape";
            assert!(
                outcome.refused("InvalidInput", surrogate),
                "{d}: {outcome:?}"
            );
        }
        tally.report("two-defect");
    }

    /// Strings with surrogates, numbers, duplicate keys and whitespace forms,
    /// each at the root, in arrays and objects, as keys, and at depths 1-65.
    #[test]
    fn explicit_token_forms_agree() {
        let mut tally = Tally::default();
        let mut values: Vec<Vec<u8>> = Vec::new();
        for list in [
            NUMBERS,
            BAD_NUMBERS,
            LITERALS,
            BAD_LITERALS,
            STRINGS,
            BAD_STRINGS,
        ] {
            values.extend(list.iter().map(|t| t.as_bytes().to_vec()));
        }
        values.extend(SYNTAX_STRINGS.iter().map(|t| t.to_vec()));
        let mut docs: Vec<Vec<u8>> = Vec::new();
        for value in &values {
            let v =
                |prefix: &str, suffix: &str| [prefix.as_bytes(), value, suffix.as_bytes()].concat();
            docs.push(value.clone());
            docs.push(v("[", "]"));
            docs.push(v("[0,", ",0]"));
            docs.push(v(r#"{"k":"#, "}"));
            docs.push(v(r#"{"a":0,"k":"#, r#","b":[1]}"#));
            for depth in [1, 2, 63, 64, 65] {
                docs.push(v(&"[".repeat(depth), &"]".repeat(depth)));
                docs.push(v(&r#"{"k":"#.repeat(depth), &"}".repeat(depth)));
            }
        }
        let mut keys: Vec<&[u8]> = Vec::new();
        keys.extend(KEYS.iter().map(|k| k.as_bytes()));
        keys.extend(STRINGS.iter().map(|k| k.as_bytes()));
        keys.extend(BAD_STRINGS.iter().map(|k| k.as_bytes()));
        keys.extend(SYNTAX_STRINGS.iter().copied());
        for key in &keys {
            let k =
                |prefix: &str, suffix: &str| [prefix.as_bytes(), key, suffix.as_bytes()].concat();
            docs.push(k("{", ":0}"));
            docs.push(k(r#"{"a":0,"#, r#":[0]}"#));
            docs.push(k(r#"[[0],{"#, r#":{"x":"\uD800"}}]"#));
            docs.push(k(&r#"{"k":"#.repeat(63), &format!(":0{}", "}".repeat(63))));
        }
        // Every ordered pair of key forms: decoded-equality duplicates.
        for first in &keys {
            for second in &keys {
                docs.push([b"{", *first, b":0,", *second, b":1}"].concat());
                docs.push([b"[{", *first, b":[0],", *second, b":[[1]]},0,0]"].concat());
            }
        }
        for ws in WHITESPACE
            .iter()
            .map(|w| w.as_bytes())
            .chain(BAD_WHITESPACE.iter().copied())
        {
            // `~` marks every token boundary.
            let t = |s: &str| -> Vec<u8> {
                let parts: Vec<&[u8]> = s.split('~').map(str::as_bytes).collect();
                parts.join(ws)
            };
            docs.extend([
                t(r#"~[~1~,~{~"a"~:~2~,~"b"~:~[~]~}~,~"s"~]~"#),
                t(r#"~{~"k"~:~{~"\uD83D\uDE00"~:~"\uDE00"~}~}~"#),
                t(r#"[~"x"~:~1~]"#),
                t("~0~"),
                t(r#"~""~"#),
            ]);
        }
        for doc in &docs {
            tally.profiles(doc);
        }
        tally.report("token-forms");
    }

    fn le32(bytes: &[u8], at: usize) -> Option<usize> {
        Some(u32::from_le_bytes(bytes.get(at..at + 4)?.try_into().ok()?) as usize)
    }

    fn le64(bytes: &[u8], at: usize) -> Option<usize> {
        usize::try_from(u64::from_le_bytes(bytes.get(at..at + 8)?.try_into().ok()?)).ok()
    }

    fn slice(bytes: &[u8], at: usize, len: usize) -> Option<Vec<u8>> {
        Some(bytes.get(at..at.checked_add(len)?)?.to_vec())
    }

    fn glb_json(bytes: &[u8]) -> Option<Vec<u8>> {
        if bytes.get(..4)? != b"glTF" || bytes.get(16..20)? != b"JSON" {
            return None;
        }
        slice(bytes, 20, le32(bytes, 12)?)
    }

    /// Every JSON document in a file: JSON/glTF text, GLB JSON chunks, b3dm,
    /// i3dm and pnts tables (and a b3dm's GLB), subtree JSON, and recursively the
    /// members of 3tz/zip archives.
    fn documents(name: &str, bytes: &[u8], out: &mut Vec<Vec<u8>>) {
        let lower = name.to_ascii_lowercase();
        let extension = lower.rsplit('.').next().unwrap_or("");
        let table = |header: usize, out: &mut Vec<Vec<u8>>| -> Option<usize> {
            let (ftj, ftb, btj) = (le32(bytes, 12)?, le32(bytes, 16)?, le32(bytes, 20)?);
            let btb = le32(bytes, 24)?;
            out.push(slice(bytes, header, ftj)?);
            if btj > 0 {
                out.push(slice(bytes, header + ftj + ftb, btj)?);
            }
            Some(header + ftj + ftb + btj + btb)
        };
        match extension {
            "json" | "gltf" | "geojson" | "schema" => out.push(bytes.to_vec()),
            "glb" => out.extend(glb_json(bytes)),
            "b3dm" if bytes.starts_with(b"b3dm") => {
                if let Some(glb) = table(28, out) {
                    out.extend(bytes.get(glb..).and_then(glb_json));
                }
            }
            "pnts" if bytes.starts_with(b"pnts") => {
                table(28, out);
            }
            "i3dm" if bytes.starts_with(b"i3dm") => {
                table(32, out);
            }
            "subtree" if bytes.starts_with(b"subt") => {
                out.extend(le64(bytes, 8).and_then(|len| slice(bytes, 24, len)));
            }
            "3tz" | "zip" => {
                let Ok(mut archive) = zip::ZipArchive::new(std::io::Cursor::new(bytes)) else {
                    return;
                };
                for index in 0..archive.len() {
                    let Ok(mut member) = archive.by_index(index) else {
                        continue;
                    };
                    let member_name = member.name().to_string();
                    let mut data = Vec::new();
                    if std::io::Read::read_to_end(&mut member, &mut data).is_ok() {
                        documents(&member_name, &data, out);
                    }
                }
            }
            _ => {}
        }
    }

    /// Committed fixtures, plus any directories in RUSTY_TILES_JSON_DIFF_CORPUS
    /// (path-list separated): C1 controls, oracle work directories and the
    /// producer corpus.
    #[test]
    fn committed_and_supplied_json_corpora_agree() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
        let mut directories = vec![root.join("tests/fixtures")];
        if let Some(extra) = std::env::var_os("RUSTY_TILES_JSON_DIFF_CORPUS") {
            directories.extend(std::env::split_paths(&extra));
        }
        let mut total = Tally::default();
        let mut files = 0usize;
        for directory in &directories {
            let mut tally = Tally::default();
            for entry in walkdir::WalkDir::new(directory).sort_by_file_name() {
                let entry = entry.unwrap();
                if !entry.file_type().is_file() || entry.metadata().unwrap().len() > 256 << 20 {
                    continue;
                }
                let name = entry.file_name().to_string_lossy().into_owned();
                let mut docs = Vec::new();
                documents(&name, &std::fs::read(entry.path()).unwrap(), &mut docs);
                files += usize::from(!docs.is_empty());
                for doc in docs {
                    tally.profiles(&doc);
                }
            }
            tally.report(&format!("corpus {}", directory.display()));
            total.merge(tally);
        }
        eprintln!("json-differential corpus files-with-json={files}");
        total.report("corpus total");
        assert!(!total.documents.is_empty());
    }

    fn series(shape: &str, depth: usize, budget: usize) -> Vec<u8> {
        let mut body = String::new();
        let mut i = 0u64;
        let (open, close, item): (&str, &str, fn(u64) -> String) = match shape {
            // GLB JSON with an `extras` array of numbers nested `depth` deep.
            "glb-extras-numbers" => (r#"{"asset":{"version":"2.0"},"extras":"#, "}", |i| {
                format!("{}.{}", i % 1000, i % 7)
            }),
            // glTF buffer data URIs: long strings dominate the bytes.
            "gltf-data-uris" => (r#"{"asset":{"version":"2.0"},"buffers":"#, "}", |i| {
                let payload = "QUJD".repeat(4096);
                format!(
                    r#"{{"byteLength":{i},"uri":"data:application/gltf-buffer;base64,{payload}"}}"#
                )
            }),
            // A b3dm batch table: one property, nested string/number values.
            _ => (r#"{"name":"#, "}", |i| {
                if i % 2 == 0 {
                    format!("\"n{i}\\u00e9\"")
                } else {
                    format!("{i}")
                }
            }),
        };
        let fixed = open.len() + close.len() + 2 * depth;
        while fixed + body.len() < budget {
            if i > 0 {
                body.push(',');
            }
            body.push_str(&item(i));
            i += 1;
        }
        format!("{open}{}{close}", wrap(depth, &body)).into_bytes()
    }

    fn median(mut values: Vec<f64>) -> f64 {
        values.sort_by(f64::total_cmp);
        values[values.len() / 2]
    }

    /// Admission time per byte on a fixed byte budget at depths 1, 30 and 62
    /// must stay within 1.25x of depth 1. The legacy implementation is the
    /// sensitivity control: its re-scanning must fail the same bound.
    #[test]
    #[ignore = "timing control: run in a release build with --ignored --nocapture"]
    fn linearity_control_depth_series() {
        const BUDGET: usize = 1 << 20;
        const ROUNDS: usize = 15;
        let limits = limits(8 << 20, 64, 1 << 22);
        for shape in ["glb-extras-numbers", "b3dm-batch-table", "gltf-data-uris"] {
            let docs: Vec<(usize, Vec<u8>)> =
                [1, 30, 62].map(|d| (d, series(shape, d, BUDGET))).into();
            for (name, run) in [("new", admit as Admit), ("legacy", legacy::admit as Admit)] {
                let mut times = vec![Vec::new(); docs.len()];
                for round in 0..ROUNDS {
                    // Rotate the order per round so drift does not favour a depth.
                    for k in 0..docs.len() {
                        let index = (round + k) % docs.len();
                        let bytes = &docs[index].1;
                        let started = std::time::Instant::now();
                        let nodes = run(bytes, limits).unwrap().value_nodes();
                        let elapsed = started.elapsed().as_nanos() as f64;
                        std::hint::black_box(nodes);
                        times[index].push(elapsed / bytes.len() as f64);
                    }
                }
                let per_byte: Vec<f64> = times.iter().map(|t| median(t.clone())).collect();
                let minimum: Vec<f64> = times
                    .iter()
                    .map(|t| t.iter().copied().fold(f64::INFINITY, f64::min))
                    .collect();
                let ratios: Vec<f64> = per_byte.iter().map(|t| t / per_byte[0]).collect();
                for (k, (depth, bytes)) in docs.iter().enumerate() {
                    eprintln!(
                        "linearity {shape} {name} depth={depth} bytes={} median_ns_per_byte={:.3} \
                         min_ns_per_byte={:.3} ratio_to_depth1={:.3}",
                        bytes.len(),
                        per_byte[k],
                        minimum[k],
                        ratios[k]
                    );
                }
                let worst = ratios.iter().copied().fold(0.0, f64::max);
                if name == "new" {
                    assert!(
                        worst <= 1.25,
                        "{shape}: new admission grows with depth: {ratios:?}"
                    );
                } else {
                    assert!(worst > 1.25, "{shape}: control insensitive: {ratios:?}");
                }
            }
        }
    }
}
