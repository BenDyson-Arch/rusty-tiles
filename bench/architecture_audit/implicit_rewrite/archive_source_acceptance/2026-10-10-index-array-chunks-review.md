# Independent index-row iterator supplement — 2026-10-10

Decision: the reviewed one-line source correction preserves the bounded index
reader's behavior. Corrected-source runtime acceptance remains pending its exact
CI head and artifact evidence. The original [accepted review](review.md),
[artifact pins](accepted-source-artifact-pins.json) and 19 historical retained
receipts are preserved; they are not rebound to the corrected source.
[Supplement pins](2026-10-10-index-array-chunks-pins.json) bind this separate phase.

At PR #152 head `ca6030245b02ce81d32afbaf10c129992045233e`, the retained CPython
3.10 CI log reports Clippy `chunks_exact_to_as_chunks` at
`src/archive3tz/read.rs:153`, promoted to an error by `-D warnings` on Rust 1.99.
That observed failure is a bindings-lint failure, not an executed archive defect.
This lane did not run Cargo, builds, product probes or heavy verification.

The coordinator changes only:

```rust
for row in bytes.chunks_exact(24) {
```

to:

```rust
for row in bytes.as_chunks::<24>().0 {
```

The reviewer checked exact source bytes against the PR head, not merely the
displayed diff. All 93 production inputs were compared with the earlier source
manifest; only this source file differs. All four verification inputs are
unchanged. The recorded old and corrected source hashes distinguish the phases.

Before this loop, `validate_index` requires at least two members, computes
checked `24 * (count - 1)`, requires the last central member to be the named index
and requires its stored length to equal that value. `read_member` then allocates
exactly that admitted size, completes the bounded read and checks its CRC before
returning. Consequently the loop's successful input length is divisible by 24,
and the remainder returned by `as_chunks::<24>()` is empty.

Both expressions borrow the same complete 24-byte rows in the same order.
The new row reference has array type; its coercion to the existing byte-slice
helpers preserves the same reads at offsets 0, 8 and 16. Hash ordering, exact
catalog-local-offset lookup, raw-name MD5 comparison and the coverage table are
unchanged. The delta adds no allocation, source read, seek, normalization,
fallback, alternate catalog, resource policy or error mapping. The positive
constant chunk width introduces no new zero-width condition. Existing production
modules already use `as_chunks`, so this change adds no new repository method
dependency. No non-equivalence was found.

This static equivalence review supports the correction without replaying broad
local builds solely for the iterator spelling. The coordinator's capped bindings
Clippy result and the corrected exact-head CI's archive, resource, wheel and
official Blender artifacts must establish that phase's executed acceptance.
Earlier portable/native/test executable hashes remain historical. Applicable
merge gates, full A2 conformance, legacy migration and release parents remain
separate and open wherever their required evidence is unfinished.
