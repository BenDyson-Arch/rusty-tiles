# A2 separate nonauthor preparation review

2026-10-10. This is an independent review of the preparation contract and three
fresh audit lanes against coordinator-provided accepted merge
`34a76152d18b02553691f472207225f116307a67`. The evidence supports retaining this
bounded preparation record. It does **not** accept a production rewrite,
advertised legacy-operation removal, broader implicit conformance or release.
Unresolved engineering gates below remain blocking before those actions.
The frozen preparation commit is coordinator-provided
`bca778a67a8062ccdb983f9b2507c10529a604f9`; its 35 reviewed file snapshots and
93 frozen production inputs were rechecked unchanged after the commit.

The reviewer owns only this new subtree. No production/contract/other-audit
editing, Git mutation, Cargo, converter execution or broad suite occurred.
Small replay/decoding processes run at `nice -n 10`. Source identity is established
by recorded file hashes, not by treating the old converter or shared validator
as specifications. The branch/merge identities are coordinator-provided; this
review independently checks their byte-bound production input manifest.

## Independently executed checks

[probe.py](probe.py) copies the authored semantic model, fixtures and eight
inspected source files into a fresh `/tmp` skeleton, then executes that copy.
All four five-node QUADTREE/OCTREE fixtures and 22 corruption controls pass
their intended checks. A fresh capture execution reproduces all 41 deterministic
cases, including actual POSIX identity operations and the mixed-byte capture
limit. Python RSS/PID/path-dependent receipt fields are not compared as invariant
facts. These executions remain models; they establish no Rust lifecycle or
resource acceptance.

The review adds a separate exact ancestor-containment oracle for the authored
axis-aligned local-box fixtures, with rational inverse/composed frames. Every
authored payload is checked against its own box and every ancestor. Removing
either authoritative bounds or error metadata is rejected in both schemes.
Moving a descendant consistently in source and emitted documents passes the
authored source/output pair oracle, but our ancestor oracle rejects it. That
control is a design/model result, **not an executed legacy converter defect**.
It proves why row/frame preservation and actual content coherence are distinct
mandatory checks; neither check may silently stand in for the other.

The reviewer separately downloaded all 14 pinned primary specification/schema
and Cesium implementation files and verified their lengths/SHA256s. This agrees
with the semantic lane's exact upstream identities. All 26 F1d2 remote compressed
records match their storage-index compressed and expanded hashes/lengths and
the actual externally retained original files. That preserves the earlier
bounded evidence; it does not rerun or broaden F1d2 acceptance.

[bind_real.py](bind_real.py) binds the final retained portable/native raw receipts
to the tracked compressed receipts, current driver/wrapper, actual frozen
binaries/reference meshopt decoder, production manifest, every generated
artifact and every recorded non-index source member. It performs read-only
replay of the authored geometry/metadata/ownership decoder and a separate exact
ancestor check over its decoded coordinates. It runs no producer or converter.
Final receipt identities are recorded in [real-bindings.json](real-bindings.json).
Both final runs contain 14 cases, 19 controls and eight legacy successes; all
93 frozen production-manifest entries match. The separate replay makes 1,031
portable and 1,046 native ancestor-containment checks. Model/primary-source/F1d2
storage results are retained in [model-results.json](model-results.json); final
reviewed file identities are recorded in [receipt.json](receipt.json).

## Contract and evidence findings

The selected finite source-array `ChildOrdinal` profile is a reasonable bounded
inference. It requires at most four/eight children, authoritative effective
bounds/error rows for **every** available tile, exact source identities/frames,
and decoded own/ancestor content coherence. No center or nominal-cell fit may
replace those requirements. Coordinates represent availability/resource
addresses under this profile; the contract correctly avoids a grid-spatial-query
equivalence claim. The independent source-order fixtures preserve coincident
box/center identities and detect swapped owners.

The [primary implicit text](https://raw.githubusercontent.com/CesiumGS/3d-tiles/4d781014b52294759834018a931223b98ac1ce47/specification/ImplicitTiling/README.adoc)
allows metadata overrides but also describes spatial subdivision and forbids
external tileset URIs at an implicit root. The selected interpretation of an
unavailable external root slot remains qualified. Neither schema/model success
nor Cesium's direct use of the override establishes unconditional conformance.
Actual selection/culling/refinement/picking and independent format acceptance
remain required. The explicit-skeleton/implicit-leaf alternative is correctly
identified as a separate partial-implicit product.

The real-source lane exercises actual portable and native point/vector source
archives, rounded/quantized/meshopt/combined payloads, deep/tight trees, duplicate
midpoint branches, fragmented line and circle arrays, shared payload slots,
relative buffers/images/schema resources and opaque metadata/extensions.
Eight legacy successes per mode preserve checked effective frames, rows,
authored errors, ordered content headers and exact original non-control/alias
bytes. Six source refusals per mode remain refusals, not proof that the valid
use cases should be retired. The source-order plans admit these small decoded
geometries but create no replacement archive. Sensitive output/resource models
remain distinguishable from actual CLI controls.

Executed b3dm coverage is the specific `BATCH_LENGTH: 0` wrapper with empty
feature binary and batch tables and no `RTC_CENTER`; metadata decoded by this
lane comes from the inner GLB. It does not establish general b3dm feature-table,
batch-table or RTC behavior. Meshopt is decoded by the pinned reference decoder;
there is no Draco or general sparse/rotated glTF decode proof. Primitive restart
handling follows a pinned draft profile and still needs target consumer support.

Known-resource closure interprets GLB buffers/images and structural-metadata
schema URIs; it reports unknown extension names. Success for an unknown
extension referring to an already known image does not prove unknown-reference
semantics. An unknown-only private resource is refused as unreferenced by the
legacy path. Source report/control bytes are deliberately excluded from the
legacy byte oracle; required new report, original provenance bytes and references
to replaced controls remain a concrete preservation blocker.

Capture/ZIP/JSON/accounting/lifecycle evidence is correctly labelled. The
41-case Python model omits ZIP64, descriptors, CRC/index semantics and Rust
resource acceptance. Identity/length/mtime checks observe changes but do not
provide an atomic snapshot against restored metadata; stable input during
capture is a real precondition. Proposed byte/count/RSS/scratch limits remain
unaccepted engineering proposals. The small actual-source census is useful
fit evidence, not a maximum-resource stress or production-admission result.

Error categories correctly distinguish malformed source paths/collisions
(`InvalidInput`), source/generated-name conflicts in a valid source
(`Unsupported`), duplicate generated names (`InvalidState`), invalid destination
syntax (`InvalidRequest`), infrastructure (`Io`) and final installation races
(`Conflict`). Model categories are not adapter parity acceptance. Collision
receipts retain source/forced-prior-output preservation and cleanup; exact final
workspace-prefix checks must bind to the final executed driver.

## Blocking next engineering gates

1. Settle the terminal external-template interpretation with its finite rationale
   and independent format/actual consumer proof, or select and name a different
   representation. Source-order addressing must keep its authoritative row and
   decoded-content prerequisites. Cover proxy/descendant ownership, contentless
   routes, multiple slots and culling/picking through coarse/fine selection.
2. Resolve admitted effective metadata types/semantics, inherited REPLACE,
   ADD refusal, version promotion, tile/content bounds/groups/schemas, extras
   application mapping and exact source-report/provenance bytes/references.
   Give each currently advertised source capability a support/rework/retirement
   disposition before removing its legacy entrypoint. No producer branding or
   silent epsilon is admission truth.
3. Rework the existing inspector dependency explicitly: `validate.rs::node`
   currently requires child-box containment with its own tolerance, which is
   stricter than the selected content-coherence policy. It is supplementary
   structural evidence, not a decoded-geometry oracle. Reuse only a private
   captured-byte/final-staging-handle core with caller-owned budgets, causal
   errors and a justified semantic profile; no public path recapture or new
   default control.
4. Implement complete bounded ZIP/index/CRC/name/resource admission before
   directory/tree allocation, and complete output inventory before workspace
   creation. Prove aliases per emitted occurrence, repeated headers, reports,
   final names/index/archive overhead and at/one-beyond ceilings. Measure actual
   Rust RSS, descriptors and scratch under coordinated CPU limits.
5. Replace legacy JSON sentinel mutation and operation lifecycle together.
   `convert_implicit.rs` currently calls public `package` with a fresh default
   RunControl, inspects a pathname candidate and later repacks; these are source
   observations, not newly executed lifecycle bugs. One Attempt must prepare
   before callbacks, write/inspect/seal/publish the same candidate and complete
   required cleanup before ready. Add real fault/cancellation/reentry/race and
   first-cause probes plus exact typed-report/inventory checks.
6. Require final-source independent production acceptance, external Rust
   consumers, CLI/freshly installed wheel parity, applicable platform checks and
   final artifact/consumer bindings. No missing evidence is waived by these
   preparation models. #126/#125/#113 and release gates remain open.

Replay without `-O`, using a fresh nonexistent `/tmp` directory:

```sh
nice -n 10 python3 -B bench/architecture_audit/implicit_rewrite/a2_review/probe.py /tmp/new-a2-nonauthor-review
nice -n 10 python3 -B bench/architecture_audit/implicit_rewrite/a2_review/bind_real.py /tmp/new-a2-real-bindings.json /tmp/rusty-tiles-a2-probe-artifacts-real-portable-final2 /tmp/rusty-tiles-a2-probe-artifacts-real-native-final2
```

The second command requires exact externally retained artifacts. The reviewer
receipt binds the final inspected evidence paths and hashes. No merge into
`main`, tag, publication, release or production acceptance is authorized here.
