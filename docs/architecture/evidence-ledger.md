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

## First proof and rewrite slices

The executable API/runtime probes expose immediate required rewrites, but they
must not cause the rest of the library to be treated as correct by omission.
After the API/job slice, fan out independent contract/oracle reviews for:

- CRS, placement, bounds and numerical domains.
- Mesh geometry, partitioning, LOD error and texture fidelity.
- Point-cloud/vector sampling, topology, identity and hierarchy semantics.
- Raster/terrain data meaning and native resource ownership.
- Format/resource/archive correctness and independent validators.

Each workstream produces a retain/rework/replace decision with its evidence.
A clean module diagram is not proof. Replacing a component also needs to pass
its evidence gate; new code receives no exemption merely because it is new.
