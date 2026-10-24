# H23 payload replay preparation

This checkpoint prepares a private C1 replay API. The public validator still
calls full inspection in both phases. Opus / VS-B owns caller integration,
bounded receipt storage and the final full validator acceptance; this checkpoint
does not claim integrated parity, a speedup, or release readiness.

`payload::inspect_scene` keeps full first admission and produces a 64-byte
`ReplayAdmission` only on success. `payload::replay_scene` verifies the original
member and the ordered external-resource bytes before any scene-consumer event.
The same parser, bounded view/decoder and graph remain; only repeated accessor
value/extrema/restart/colour checks and KTX2 container inspection are skipped.
Resolver call order, limits, materialization and inspection facts are preserved
for unchanged admitted bytes. Changed resources can encounter a retained
layout/decoder/I/O error before the final digest comparison. This is not an
atomic source snapshot or a promise of identical concurrent-mutation errors.

The ordinary payload tests include independent `replay_controls.rs`, with
raw scene-event comparisons, first-admission failure controls, changed-member /
resource / trailing-byte / transcript-framing controls, callback limits/errors
and direct observations that replay removes the targeted duplicate work.
The receipt hashes establish byte identity; they do not define format meaning.

Preparation source is develop `df3f7a5ff8c4bb4a28231e650490232f0276390d`.
Bulk plan, nonauthor reviews, focused extraction build/test receipts and handback
are in `~/.cache/rusty-tiles-evidence/h23-single-inspection-20261013/`.
Its final `manifest.json` pins every retained small document and execution receipt.
The focused harness compiles the actual private format owners against identified
existing dependency artifacts. It does not build or exercise the public CLI,
native/wheel/Blender adapters, traversal integration or the perf lane.

Final local payload extraction: 60 tests passed, no failures, one existing
release-only timing control ignored and 59 non-payload tests filtered out.
All 12 new replay controls passed. All eight focused mutants compiled and were
caught at runtime. Focused warnings-denied Clippy passed, with only the
extraction's crate-level dead-code allowance; this is not all-target Cargo Clippy.
Formatting, whitespace and the staged tree budget checks passed.

Final bounded nonauthor source/evidence review: `source-review.md`, SHA-256
`dacd2a3edb4d38799c690a41321628dfd3fa40936e645ab5a54c81754b10a7ce`.
Final test receipt: `runs/focused-lint-clean/receipt.json`, SHA-256
`47784b3846cd16823f24605b4f8a367ceefacb92b65c2e74db8da63a04b0dc2c`.
Final sensitivity matrix: `mutants-final.json`, SHA-256
`cf30214a888b63e4b391ab2aa29f122da0136401f6878e6a1015f13ce8544fed`.
Earlier harness and test-lint failures, corrections and successful earlier
executions remain retained under their original evidence identities.

Before an accepted H23 PR, integrate the caller after its existing C1/exclusion/
precheck gates, amend the normative C2 stream-step-1 and receipt allocation ledger,
run full C1/C2/I1/forest/W parity and sensitive mutants, check fault/limit/accounting
controls, calibrate byte-heavy and accessor-heavy work and arrange the separate
final-source audit through Opus. Existing timing constants and global resident
memory bounds do not follow from these payload-only tests.
