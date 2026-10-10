This is frozen P2 preparation, not an implementation or accepted proof package.
Read proof-plan.md, inputs/pins.json and expected-records.json. Inputs retain both
historical freezes and current candidate source observations; only the latter may
be bound to the eventual accepted C1 merge. No Cargo/compiler/native/target/model
probe has been run by the author. Preparation generation/syntax/hash/shape checks
are recorded in static-preparation.json.

Root owns every compilation and actual execution, serially at nice10/affinity2/
two-worker environment. Do not run a native/target probe until corrected C1 is
independently accepted and merged into develop. Templates are deliberately invalid:
accepted-c1-gate-TEMPLATE.json and coordinator-artifact-receipt-TEMPLATE.json must
be replaced by NEW coordinator-owned exact merge/source/artifact records outside
this immutable package. No confirmation from the user is required; this is the
explicit task prerequisite, not an approval workflow.

After P0, coordinator commands (substitute actual pinned paths; fresh output dirs):

```sh
python3 /tmp/rusty-tiles-next-vector-compression-audit/codec/p2-proof-preparation-20261010/coordinator_runner.py --lane models --c1-gate /tmp/P2-accepted-c1.json --out-dir /tmp/P2-model-results
python3 /tmp/rusty-tiles-next-vector-compression-audit/codec/p2-proof-preparation-20261010/coordinator_runner.py --lane raw --c1-gate /tmp/P2-accepted-c1.json --artifacts /tmp/P2-exact-artifacts.json --out-dir /tmp/P2-raw-results
python3 /tmp/rusty-tiles-next-vector-compression-audit/codec/p2-proof-preparation-20261010/coordinator_runner.py --lane ffi --c1-gate /tmp/P2-accepted-c1.json --artifacts /tmp/P2-exact-artifacts.json --out-dir /tmp/P2-ffi-results
python3 /tmp/rusty-tiles-next-vector-compression-audit/codec/p2-proof-preparation-20261010/evaluate_scaffold_records.py --run-dir /tmp/P2-raw-results
```

The runner compiles isolated source with explicit prebuilt hash-pinned rlibs; it
never invokes Cargo or edits the repository. Dependency receipts must identify
all consumed serde/serde_json/meshopt/proc-macro/compiler/std artifacts and feature
closure matching the frozen lock, not select a random existing rlib by filename.
The root-supplied merge marker/evidence pin is a coordinator attestation, not an
independent merge verifier. A changed helper/dependency/toolchain holds the old
probe inputs for separate repin/review; do not silently regenerate expectations.

Model lane runs only independent integer/byte/oracle selftests. Raw/FFI lanes
produce actual scaffold execution evidence when root runs them. Rust prototypes
are presently uncompiled; compilation failures are preparation defects to fix in
a separately hashed revision, never hidden by substituting production helpers.
Compiler warnings do not establish or refute acceptance. No production code is
included automatically from a working repository; json.rs is a frozen copy bound
to the accepted source at execution.

Allocation calibration may return2 and retain HELD: a late escaped key may exceed
the proposed E_PLAN2J term. Compare stage terms and whole scheduled envelope
separately; a stage gap is not automatically an observed full-operation budget
violation. Correct the concrete representation/term/lifetime at its owner and
repin before review. A passing scaffold still does not establish absent actual
production plan/path/receipt capacities, numeric defaults or graceful serde OOM.

Full generated-domain raw witnesses and future actual corpus gates are in
fixtures/generated-domain/manifest.json. These are independent role/association
inputs, not a producer. Future candidate comparison requires independently pinned
extraction/decode artifacts; do not use their source bytes as manufactured target
outputs. Producer whole-state/bridge/fitting/final one-Attempt evidence and platform
publication remain their owners' P4/P5 tasks. No hidden fallback/partialA2 or release
approval results from this preparation.
