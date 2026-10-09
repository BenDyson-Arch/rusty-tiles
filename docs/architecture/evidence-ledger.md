# Component evidence and disposition ledger

Status: open audit at production baseline `8dfd74b` for
[#113](https://github.com/BenDyson-Arch/rusty-tiles/issues/113).

The working assumption is that the current implementation is incorrect until it
provides adequate evidence for an independently defined contract. Plan a general
redesign/rewrite; a component earns retention through its evidence, not its age,
existing use or a green test run. This does not label an untested suspicion as an
observed defect. Both confirmed violations and counterexamples to our suspicions
must be recorded accurately.

## Evidence gate

For each component, record:

1. **Required contract:** what the user/data/format needs, independent of the
   current code and its tests. Define supported input classes, failure behavior,
   precision/fidelity, resource limits and observable results.
2. **Oracle provenance:** explain why the expected result is trustworthy. Use
   independently computed coordinates, decoded source-to-output invariants,
   applicable format requirements, or a separately validated reference. A reader
   and writer sharing the same bug do not validate each other.
3. **Positive and adversarial evidence:** ordinary and boundary inputs, malformed
   inputs, numerical conditioning, concurrency, partial failure, resource
   exhaustion and relevant platform/backend variation.
4. **Architecture fitness:** ownership, lifetime/thread constraints, dependency
   direction and integration through the desired API. Correct output alone does
   not justify keeping an unsuitable boundary.
5. **Disposition and residual limits:** retain, rework or replace, with evidence
   links and acceptance boundaries. Mark unresolved/untested cases explicitly.

`Unproven` is the default status. `Observed violation` means an executed case
violated a stated requirement. `Supported case` means evidence for only that
case, not general correctness. `Retain` requires a reviewed evidence gate for
the declared scope. No subsystem has earned unconditional retention in this
pass. An unsupported proof obligation defaults to replacement planning; it does
not become a release waiver because reproduction is inconvenient.

Existing tests are candidates for reuse. Characterization digests and snapshots
are useful change detectors but cannot establish that the historical output is
correct. Audit fixtures and oracles before carrying tests into the replacement.
The format/CRS standard review and broad numerical/performance proof work are
still outstanding; this pass does not claim to have independently verified them.

## Post-F0 update

The initial table below is historical evidence at `8dfd74b`; it is not a claim
that F0 remains unimplemented. The [post-F0 audit and scopes](next-foundation-scopes.md)
assign every remaining subsystem and operation a proof/migration owner.

| Area | Updated evidence/disposition | Follow-on owner |
| --- | --- | --- |
| Opaque package/file lifecycle/3TZ codec | Implemented in #116 with independent readers, fault/concurrency/platform tests; final review found relative-output redirection, fixed in `c777180` with isolated regression. Bounded F0 acceptance only, never scene certification | #115 / [current evidence](f0-evidence.md) |
| Mesh orchestration and source boundary | Concrete early-staging, size-dependent admission, raw URI/resource ownership and global-metric problems; one local static milestone before broader support | #117; fidelity #121; spatial #120 |
| Directory installation/recovery | D1/D2 accepted through #129/#130, including source-overlap correction, exclusive install and typed recovery; bounded single RGB consumer only | #118 complete; broader #124 |
| Vector acceptance | V1/V2 replaced the defective acceptance/report boundary and passed independent portable/native replay and final platform/wheel checks in #131; feature ownership only | #119 complete; geometry/identity #123 |
| CRS/math and decoded fidelity oracles | Analytic/frozen references and small independent decoders are bounded retain candidates; historical shared-kernel encoders do not certify whole subsystems | #120–#124 per operation |
| Validator and sampled LOD claims | Missing actual BIN payload accepted; content containment not checked by narrower child-bound validation; constructed sampled estimator can miss a 100 m spike. Distinguish those observations from unproved end-to-end defects | #125 and #121 |
| Remaining operations/public surfaces | Wrapping, convertImplicit, in-place transforms, support tools and broad exported building blocks need explicit lifecycle/API disposition; codec proof alone is insufficient | #126 |

Before release, every advertised operation earns scoped support or receives a
recorded deliberate scope/migration decision. The issue list is not evidence of
completion. See the [raw evidence audit](../../bench/architecture_audit/post_f0/evidence-audit.md)
for per-oracle provenance and counterexamples to broad historical claims.

## Initial ledger

| Component | Required contract | Evidence at this pass | Status / planned disposition |
| --- | --- | --- | --- |
| Public API and adapters | No ignored options; consistent core defaults, validation and error meanings; explicit capabilities | Executed offset/rotation/default-codec probes disagree across Rust/CLI/Python | Observed violations. Replace option/validation boundary and migrate all callers |
| Job events and callbacks | Job-local observations; before-commit abort; explicit core committed outcome | Early and post-publication Python exceptions both leave valid archives; global timing counters found statically | Observed callback-contract problem; metric isolation untested. Redesign lifecycle/context; retain pieces only after failure/concurrency evidence |
| Publication and recovery | Create-new never clobbers a concurrent target; documented replacement/recovery limits | Exact-source directory race and archive comparison are isolated controlled probes; see runtime evidence | Directory guarantee violated in injected schedule; archive case supports a narrow property. Replace/rework publisher according to platform evidence |
| Vector ingestion transactions | Accepted feature rows, counters and diagnostics agree; infrastructure failures never become skippable features | Natural skipped polygon leaves success-like reporting; isolated accept-callback I/O injection exercises classification | Observed violation / controlled fault evidence. Redesign transaction and error boundaries |
| Vector candidate/reuse publication | Final artifacts match selected hierarchy and dependencies; reuse preserves semantics | Rejected byte-budget candidate control has zero orphan content; cleanup found in `reuse.rs` | Supported case only. Initial leak suspicion withdrawn; broader cleanup/reuse remains unproven |
| CRS interpretation/transforms | Explicit axes/units/heights; declared supported operations; independent accuracy and failure behavior | Existing `tests/mesh_crs.rs` and CRS/native regressions are potential evidence, not rerun/oracle-audited here | Unproven. Audit references, conditioning, fallback and batch behavior; replace parser/adapters/transform selection as needed |
| Mesh load/placement/model | Correct scene transforms, source coordinates, normals, identities and resource references | Offset/rotation probes expose boundary violations; geometry fidelity suites not reassessed here | Unproven beyond reproduced violations. Redesign models/placement; no algorithm retention granted |
| Mesh partition/LOD/textures | Defined leaf fidelity, coverage, error bounds, material/alpha behavior and budgets | Existing fidelity/digest/benchmark corpus has not passed the new oracle review | Unproven. Define independent decoded geometry/texel/error checks; retain or replace by measured result |
| Point-cloud ingestion/sampling/hierarchy | Every required source point/attribute represented according to explicit LOD semantics; accurate placement and bounds | Existing source/LOD tests identified; no new point-cloud proof in this pass | Unproven. Assess sampling coverage/error and chunk/worker invariance before reuse |
| Vector geometry/LOD/metadata | Source topology and identity rules preserved; repair/aggregation explicit; independent error bounds | Shared owned model/backend separation found statically; numerical/topological oracle not reassessed | Unproven. Architecture is a candidate only; audit native/portable geometry and simplification separately |
| Raster and terrain | Explicit band/nodata/alpha/height semantics; correct sampling, tiling, bounds and error | Native handle lifetime/bounded-work patterns found statically; no new native execution | Unproven. Redesign where contract/evidence fails; native implementation is not grandfathered |
| GLB/glTF encoding and resources | Valid content, complete resource closure, meaningful extension preservation and accurate bounds | Callback artifacts pass local validator; independent reader/schema/extension proof not run | Narrow local validation only. Assess independent conformance and numerical representation before retention |
| Tileset/implicit hierarchy | Valid availability/ranks/relationships, meaningful bounds/errors and correct traversal | Existing subtree/metadata suites identified; generated JSON coupling found statically | Unproven. New typed model must meet independent traversal/schema/fidelity evidence |
| 3TZ archive/index | Correct offsets/checksums/ZIP framing, resource naming and collision rules; unchanged payload where required | Archive race control supports no-clobber for one case; format/index correctness not newly assessed | Unproven overall. Separate codec from job; prove codec independently or replace |
| Bounds/math/spatial primitives | Correct finite results within stated numerical domains and conditioning limits | Current math and glTF ingestion are coupled; no new numerical proof | Unproven. Define analytical/metamorphic references; move/rewrite only against those contracts |
| Error/report/capability models | Distinguish request/source/feature/infrastructure failures; reports describe committed truth | Frontend classification drift and vector reporting/infrastructure findings | Redesign required; schema and mappings need independent contract tests |
| Native wrappers and dependencies | Correct ownership/close/thread rules; no weaker implicit coordinate operation | Rust lifetimes and explicit finalizers observed in source; fault/ABI matrix not executed | Candidate only. Lifetimes support a limited ownership property, not all FFI correctness |
| Validation and release harnesses | Reject relevant corruptions and require actual platform/viewer behavior; no false passing skips | Existing harnesses identified; not rerun as final-candidate acceptance | Unproven for redesigned candidate. Audit validator independence and gate failure handling |
| Packaging/install/preview/doctor | Installed supported operations work; capabilities/readiness honest; bounded local serving behavior | Direct Linux binding probes only; no wheel/install/browser run in this pass | Unproven. Revalidate against new API and packaging, not historical candidate evidence |

## Foundation-first proof and rewrite slices

The executable API/runtime probes expose immediate required rewrites, but they
must not cause the rest of the library to be treated as correct by omission.
The bounded [F0 package/job slice](foundation-contracts.md) is first; full mesh,
directory publication and vector transactions are later scoped work. Alongside
that foundation, continue independent contract/oracle reviews for:

- CRS, placement, bounds and numerical domains.
- Mesh geometry, partitioning, LOD error and texture fidelity.
- Point-cloud/vector sampling, topology, identity and hierarchy semantics.
- Raster/terrain data meaning and native resource ownership.
- Format/resource/archive correctness and independent validators.

Each workstream produces a retain/rework/replace decision with its evidence.
A clean module diagram is not proof. Replacing a component also needs to pass
its evidence gate; new code receives no exemption merely because it is new.


## Post-vector migration update

#131 merged into develop as `5df15e7`; bounded vector V1/V2 acceptance is recorded
in #119 and its contract/evidence. The historical initial ledger above preserves
the original observations rather than claiming those defects remain in the new
facade. #132 replaces point lifecycle orchestration next; its source and numerical
algorithms earn only the support covered by the P1 independent ledger. #133 payload
validation is a separate slice. The public-surface inventory records all remaining
operation and export owners; neither pending issue is accepted by this update.
