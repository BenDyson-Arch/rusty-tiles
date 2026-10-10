# Production authoring ownership after this gate

Use three distinct GPT-6.1 Sol author lanes, with the coordinator alone owning
shared heavy builds and integration. This review author remains a nonauthor.
The coordinator records the exact base and assigns these non-overlapping files
before fan-out; any additional shared path is assigned once before editing.

| Author lane | Files and concrete responsibility |
| --- | --- |
| Format/codec owner | `src/vector_encoding.rs` and its new private child files; actual physical Plan/admission/overlay/count/native/repack/identity/limit-default owner and focused source tests. Narrow exact-number and native-helper extraction in `src/content_integrity/payload.rs`, `payload/numbers.rs` and new actual-consumed common child files, preserving existing C1 callers. Own a small vector-selected framing helper here if both codec and bridge consume it. No paths/jobs/adapters or producer orchestration. |
| Captured-file/runtime/adapter owner | One new standalone operation file, `src/runtime.rs`/its test files where the small causal ResourceLimit constructor is needed, `src/error.rs`, `src/main.rs`, `src/lib.rs`, `bindings/python/src/lib.rs` and directly associated adapter tests. Own request/result/public reexports, hidden worker migration, path/capture/permissions/rechecks/Replace, typed cause categories and installed producer-Python mapping. No format-plan or producer geometry edits. |
| Producer bridge owner | `src/vector/pipeline/encoding.rs`, `reuse.rs` and directly required vector feature/error transport files and tests. Own bounded selected-framing raw bridge, existing meshopt choice resolved once, fitting/final/one-Attempt ResourceLimit transport, actual bridge lifetimes, reuse/inventory/max_bytes/report facts and portable/native generated witnesses. Use the format owner's private helper; do not silently alter unrelated generic writer consumers. |

The coordinator owns manifests/build configuration, shared documentation and
evidence retention, sequencing/helper handoff and review-ready combined source
freeze. Owners publish the smallest exact private helper contract needed by
the other two lanes, rather than parallel implementations or speculative APIs.
Module paths above are file ownership, not authorization for broad layout moves.
An export edit must preserve deliberate public API/error ownership; no public
Plan or IR is introduced. A formerly public standalone mutator can be deliberately
migrated under the existing foundation authority, with its new API documented.

Before root executes new substantive code, freeze actual source/ledger/test
expectations and reconcile changed type/error/extension/bridge owners against
the accepted representation. The coordinator serializes nice10/two-worker
Cargo/native/producer/resource work. After implementation, obtain a fresh
separate nonauthor actual final-source/artifact review and applicable CI/consumer/
platform checks before production acceptance or an evidence-backed develop merge.
The precode decision itself grants no such acceptance.
