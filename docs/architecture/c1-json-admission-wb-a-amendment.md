# C1 JSON admission: C2-WB work-bound amendment (WB-A)

Status: author (J) amendment for the C2-WB slice, draft v2.1 §3.5 WB-A and §9.A2.
Independent final-source review (V) and the coordinator's gate are still pending.
Nothing here records acceptance.

This note amends the "All actual JSON consumers" section of
[c1-payload-integrity-contract.md](c1-payload-integrity-contract.md). That file stays
byte-identical (sha256 `3cb5ad341a73cdda77a9dd4a1925cf9800af6e37e49b902eb6db0c4f0c3ee1e3`)
because the C1 production-control driver
(`bench/architecture_audit/c1_payload_integrity/production_controls/driver.py`, run mode)
asserts its hash through `referencePins` in the committed fixture manifest. If the two
texts disagree, this note governs the mechanism and the work bound. The C1 text still
governs everything else.

## Unchanged

The following stay exactly as before:

- the meaning of admission;
- the outcome categories;
- the ResourceLimit names (byte, depth, value-node, storage);
- the compound-fault order;
- `value_nodes()`;
- the borrowed root `RawValue`;
- the Unicode, duplicate-key and number rules;
- the owned `Value` adapter.

The order is: byte ceiling, root node, one whole-document grammar skip (`RawValue`,
which rejects trailing tokens), then containers in document order. For each container,
every member key is decoded and checked unique and every immediate child is charged
(depth, then node) before any child is descended into. Scalar strings are decoded when
the walk reaches them. A duplicate root key still wins over a depth-limited first
child. More root siblings than `value_nodes` allows still win over an earlier
container's duplicate key, and a late syntax error still wins over everything except
the byte and root-node ceilings. No C1 control is re-pinned.

## Replaced mechanism

The following replaces the C1 sentences from "Traverse with an iterative worklist of
borrowed raw/depth pairs" to "Drop the container decoder, scratch and key set before
descending":

1. **Record.** After the grammar skip, one structural scan of the admitted root text
   records the value tree. For each value it stores the source offset, the
   member-key offset and the extent of its recorded subtree. The scan keeps no
   nesting state: a string followed by `:` is a key.
2. **Replay.** The legacy per-container walk then runs over this record. It pops a
   value. If the value is a container, it decodes and checks each member key, then
   charges each child, and only then pushes the children in reverse order. If the
   value is a scalar string, it decodes it. Containers' raw text is never
   re-deserialized. Key and scalar-string decoding are unchanged: one serde decoder
   per token, and the key set is dropped before descent.
3. **Bounded record.** A document with more than `value_nodes` values cannot be
   admitted. For such a document, one counting scan fixes, per container, how many
   children the walk reaches before the node ceiling. This includes the child whose
   key check or charge is the last event. A second recording scan keeps only those
   children. The walk reaches containers in document order, and container *g*'s
   children are reached after `1 + Σ` children of the earlier containers. So only a
   document-order prefix of containers needs counting.

## Replaced work and storage bound

The following replaces "Sum of raw substring lengths skipped is <=(D+1)*B". Admission
does O(B) scanning work: the grammar skip, at most three structural scans, and the
key and scalar-string decoding. On top of that come O(N) record, replay and
duplicate-lookup work, with N ≤ min(`value_nodes` + 1, B). Work does not depend on D.

The following replaces the "pending/immediate child slots" storage clause.
Source-derived requested storage is:

- the grammar skip's nesting scratch;
- the record: 24 bytes per value on 64-bit, at most min(`value_nodes` + 1, B) values;
- the per-container child counts (counting scan only);
- the open-container stacks;
- the pending and child slots: 16 bytes each on 64-bit, at most min(`value_nodes`, B);
- one key set and one decoder scratch at a time.

All explicit storage uses checked arithmetic and fallible reservation, and its failure
is the storage ResourceLimit. The same representation-bound caveats as C1 apply.

## Evidence (author)

The in-crate tests in `src/content_integrity/json.rs` keep the pre-amendment
implementation verbatim under `#[cfg(test)]` as the reference. They compare old and
new on the key (category, limit name or failure kind, `value_nodes`, admitted span).
The comparison corpus covers:

- generated, adversarial and §9.A2 two-defect documents at depths 0–65;
- explicit string, number, key and whitespace forms;
- committed and supplied JSON corpora.

There is also a release-build linearity control at depths 1, 30 and 62. The figures
are in the C2-WB author record.
