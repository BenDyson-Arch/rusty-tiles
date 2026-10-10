# Architecture foundations for 0.4.0

Status: ongoing architecture gate; F0 is accepted and merged in PR #116. Tracking issue: [#113](https://github.com/BenDyson-Arch/rusty-tiles/issues/113).
Baseline: `8dfd74bd87dd23c86278e98d736c3f5912c246cf` on `develop`.

0.4.0 promotion, tagging and publication are blocked by #113. Existing release
acceptance describes the previous candidate, not the redesigned library. F0 changes only packaging and completed-file publication; it does not complete
that release gate.

The audit is replacement-first. Treat the current implementation as untrusted
and potentially incorrect throughout; it is a source of observations and
fixtures, not the specification for its successor. General redesign/rewrite is
the planning baseline. Retaining a component is an evidence-backed exception,
not the default because it already exists or passes its current tests.

Define the required behavior independently, then require each component to meet
that contract. Current tests, output digests and benchmarks may encode current
mistakes: audit the oracle before using them as acceptance criteria. A failed
proof obligation or absent evidence leaves a component unproven and a candidate
for replacement. This is an engineering evidence standard, not a claim that
unexamined functions have been experimentally shown to fail.

The goal is clear ownership, validated contracts and dependable failure behavior
for future development. Intentional public API, algorithm and output changes
are acceptable when the desired contract calls for them. Geographic accuracy,
source fidelity and valid format/resource semantics are requirements to justify
independently; old public exports and historical output bytes are not automatic
compatibility obligations.

## Foundation-first scope

Every later foundation slice follows the same gate: independently define the
required invariants and domain, assign one owner to each decision, audit the
existing code and oracle, record retain/rework/replace/remove dispositions, and
settle the representation and evidence plan before implementation. Resolve a
failed proof by correcting its owner or explicitly narrowing the declared
domain, rather than adding a permissive fallback. Build the resulting vertical
slice, then require an independent nonauthor review and final-artifact checks.
These are engineering steps within authorized work, not extra approval pauses.

Future development should build on validated representations and clear
dependencies. A feature field or module move is insufficient when an underlying
coordinate, identity, failure or ownership contract remains implicit. Bounded
acceptance is useful only when its remaining obligations are explicit; passing
an old suite must not silently promote those obligations to accepted behavior.

The first implementation scope is **F0: the shared job/publication plumbing used
by a real package-conversion path through Rust, CLI and Python**, tracked in
[implementation issue #115](https://github.com/BenDyson-Arch/rusty-tiles/issues/115). The full mesh
rewrite is downstream work. This narrows the earlier mesh-first plan: F0 must
prove its own contracts without inheriting geometry, CRS or LOD complexity.

See [foundation contracts](foundation-contracts.md), the
[boundary audit](boundary-audit.md), and [publication primitive evidence](platform-evidence.md).
These are the current sequencing authority; the larger API and runtime proposals
remain the target direction, not a checklist to squeeze into F0.

Clean means each decision has one owner, invalid states are not propagated,
and failure outcomes carry their meaning. It does not mean banning every `if`
or replacing branch statements with opaque policy tables. Real platform and
geometry alternatives remain explicit in their owning module.

The [post-F0 audit and ordered scopes](next-foundation-scopes.md) govern the next
work. They separate a bounded first mesh milestone, directory publication, vector
transactions and remaining subsystem proof obligations. Earlier API sketches
remain proposals wherever the concrete scope has not adopted them.

## Audit documents

- [F1d2 bounded adaptive surface certificate contract](f1d2-certificate-contract.md)
- [F1d2 implementation and remaining approximation gates](f1d2-implementation.md)
- [F1d2 independent artifact and consumer evidence](../../bench/architecture_audit/mesh_approximation_f1d2/README.md)
- [F1d1 certified root proxy contract](mesh-approximation-contract.md)
- [F1d1 implementation and remaining approximation gates](f1d1-implementation.md)
- [F1d1 independent artifact and consumer evidence](../../bench/architecture_audit/mesh_approximation/README.md)

- [Next #126 implicit rewrite contract draft](implicit-rewrite-contract.md)
- [Implicit rewrite baseline probes and proof limits](../../bench/architecture_audit/implicit_rewrite/README.md)
- [Lossless F1c2 evidence storage and verification](../../bench/architecture_audit/f1c2/README.md)
- [F1c2/W1 implementation and remaining removal gates](f1c2-implementation.md)
- [F1c2/W1 evidence](f1c2-evidence.md)
- [F1c2 source identity and picking contract](f1c2-contract.md)
- [Static model wrapping and sibling manifest contract](model-wrapping-contract.md)
- [Packaging facade removal contract](pack-api-removal-contract.md)

- [Post-F0 audit and ordered scopes](next-foundation-scopes.md)
- [F0 implementation and migrations](f0-implementation.md)
- [F0 acceptance evidence](f0-evidence.md)
- [Vector feature acceptance and publication contract](vector-acceptance-contract.md)
- [Foundation contracts and first implementation slice](foundation-contracts.md)
- [Boundary/conditional-complexity audit](boundary-audit.md)
- [Publication primitive evidence](platform-evidence.md)
- [Public API and subsequent mesh slice](api-contract.md)
- [Runtime contracts and acceptance matrix](runtime-contract.md)
- [Component evidence and disposition ledger](evidence-ledger.md)
- [Executable audit evidence](../../bench/architecture_audit/README.md)

The API sketches are proposed interfaces, not currently available APIs. Evidence
separates observations made with the unmodified converter from isolated failure
injection and findings established only by source inspection.

## Current ownership and dependency map

This is a source-level map of the relevant coupling, not a complete compiler
call graph. Paths and line numbers refer to the baseline above.

| Area | Current responsibilities and coupling | Proposed ownership |
| --- | --- | --- |
| `main.rs`, `bindings/python/src/lib.rs` | Parsing, construction of options, differing validation/defaults, presentation, error mapping | Thin adapters over one core request/validation contract |
| `tile.rs` | Mesh options, orchestration, partitioning, encoding, image/color helpers | Mesh request and pipeline; texture helpers owned by mesh texture processing |
| `mesh.rs`, `glb_write.rs` | Scene loading/model; encoder-owned `TilePrimitive` used by geometry/LOD | Domain-owned scene/primitive data; separate glTF source and serialization boundaries |
| `texture.rs`, `hlod.rs` | Texture calls tile helpers; texture and HLOD share global timing state | Texture algorithms independent of orchestration; per-job metrics |
| `mesh_crs.rs` | Coordinate bake depends on full `MeshTo3tzOptions` | Dedicated validated placement input, independent of output/codec settings |
| `bbox.rs`, `georef.rs`, `crs.rs` | glTF ingestion mixed with bounds/math; coordinate definitions, transforms, strict eligibility and regression fixtures | Spatial primitives separated from format ingestion; CRS algorithms and eligibility policies independently audited before retention |
| `pack.rs`, `output.rs` | `pack` invokes `Job`; `Job` invokes archive internals | Archive/index codec below publisher; package conversion above publisher |
| `tileset_node.rs`, `implicit/tileset.rs` | Generated hierarchy represented and repeatedly decoded as JSON | Typed generated nodes, bounds, transforms and errors; serialization at output boundary |
| `vector/pipeline/store.rs` | SQL spool, feature acceptance, side effects, hierarchy construction and reports | Feature transaction, spool, hierarchy and report responsibilities explicit |
| `vector/pipeline/encoding.rs` | Candidate encoding writes content before final acceptance | Private candidate artifacts; accepted inventory determines published resources |
| `vector/portable.rs`, `source_native.rs` | Source-specific decoding and common feature rejection/error classification | Format readers with owned feature output and explicit feature-vs-job failure types |
| `report.rs`, `error.rs` | Core events/results plus stderr defaults and CLI exit-code policy | Core result/error/event semantics; adapter-owned formatting and transport mappings |
| `doctor.rs`, `preview.rs` | Readiness and local server in library dependency graph | Core capability inspection distinct from application/server behavior |

Concrete reverse references include `texture.rs:917,918,1710,1732` into `tile`,
`texture.rs:21` into HLOD metrics, `mesh_crs.rs:7` into orchestration options,
and `pack.rs:2` / `output.rs:116` across the archive/job boundary. The migration
must eliminate these dependency cycles rather than merely relocate files.

## Proposed layout

```text
src/
  api/                    intentional requests, results, errors, capabilities
  runtime/                staging, publication, reports, job resources/metrics
  spatial/                bounds, math, frames, CRS policy and transforms
  formats/
    gltf/                 reading/writing, metadata, compression, URI resources
    tiles3d/              generated node types, manifests, implicit subtrees
    archive3tz/           archive/index reading and writing
  pipelines/
    mesh/                 model, source, placement, partition, LOD, textures
    point_cloud/          source, sampling, hierarchy, content encoding
    vector/               model, readers, geometry, spool, LOD, encoding, reuse
    raster/               source, display, tiling
    terrain/              owned sampling, bounded GLB terrain surfaces
  validation/             independent archive/content validation
  cli/                    parsing, application dispatch, presentation
  lib.rs                  deliberate public facade
  main.rs                 CLI entry point
bindings/python/          Python adapter and installed-wheel tests
```

The public facade may re-export domain-owned request/result types. It must not
become a dependency hub that lower-level algorithms import to reach each other.
Use Rust visibility to enforce ownership: default to private modules and
`pub(crate)` implementation types; make each public item an intentional API
commitment. Keep fixture generation and oracle constants in test support.

Allowed dependencies:

1. CLI/Python adapters invoke the public conversion/inspection API.
2. Converter orchestration uses its domain model/algorithms, runtime and format
   codecs. Domain algorithms use owned models and spatial primitives.
3. Format codecs use format types, spatial primitives and domain-neutral buffer
   inputs. They do not call converter entry points or depend on job options.
4. Publication uses archive serialization; archive serialization never uses a
   conversion job. Native source/transform handles stay in their owning backend
   and thread; worker payloads are owned Rust data.
5. Independent validation does not assume that documents came from our writer.
   Imported JSON retains unknown extension data; typed generated nodes do not
   authorize lossy deserialization of arbitrary external documents.

Keep one domain library and the existing Python binding crate initially. A
separate CLI/application crate is plausible, but moving the binary changes
`cargo install rusty-tiles`, workspace defaults, packaging and release scripts.
Map and test those consequences before choosing that package split. Logical
adapter separation and removing Clap derives from domain types are required;
per-format/per-converter crates and a universal conversion framework are not.

## Candidates for retention need evidence

No subsystem is grandfathered into the new architecture. The owned vector model,
native handle lifetimes, bounded terrain work units, CRS policy and staged Job
are plausible candidates for reuse, not accepted foundations. The
[evidence ledger](evidence-ledger.md) records their proof obligations and current
status. Existing fidelity/accuracy regressions become evidence only after their
oracle and coverage are assessed.

A reproduction correcting an initial suspicion is equally important: the
exercised vector cleanup removes unused candidates, contrary to the first
static suspicion. That result establishes this case, not the correctness of all
cleanup, reuse or implicit-resource behavior.

## Proposed pre-release scope

These are recommended implementation gates to resolve in #113, not completed
work. Each migration should produce a usable vertical slice against the desired
contract. Existing release routes are integration targets to redesign and test,
not constraints that force the old architecture to survive.

| Gate | Required outcome before 0.4.0 | Evidence to close it |
| --- | --- | --- |
| A: core requests | Canonical defaults, validated coordinate/output choices, no ignored options; Rust/CLI/Python share validation and domain error kinds | Shared invalid/valid request matrix exercised through all three interfaces; migration notes for deliberate changes |
| B: dependency ownership | Remove mesh and archive/job cycles; domain-owned intermediate model; private implementation surface; frontend-only argument parsing | Dependency review, external consumer compile examples, existing fidelity/digest checks |
| C: job contract | Per-job metrics/events, explicit callback/cancellation boundary, consistent publication/no-clobber/error semantics | Repeated/concurrent job probes, injected failures and directory/archive publication tests |
| D: vector acceptance | Feature state/diagnostics commit together, only feature failures skippable, accepted artifact inventory controls output | Rejected-fragment rollback, I/O/SQLite fault tests, rejected LOD/content inventory and reuse tests |
| E: generated documents | Typed finite bounds/transforms/errors and generated hierarchy; external document/resource validation remains independent | Existing implicit metadata/subtree and decoded hierarchy tests; unknown imported extension/resource preservation |
| F: capabilities/resources | Resolve support for the requested operation; explicit worker/resource ownership and documented resident-memory costs | Portable/native capability cases, fixed-worker memory/scratch/descriptor measurements |
| G: candidate validation | Documentation and adapters match the new contracts; applicable release acceptance rerun on final sources | Rust/frontend tests, platform CI, official wheel/Blender and strict browser acceptance |

A hard global memory allocator/scheduler is not assumed necessary for 0.4.0.
Worker/cache admission must be explicit and measured; distinguish bounded
payload batches from resident mesh/hierarchy state. Do not claim bounded total
memory from per-tile output limits. Likewise, crash-durable atomic replacement
of arbitrary directory trees is not promised without a concrete cross-platform
implementation. Define supported publication guarantees precisely.

Defer new formats, 3D Tiles 2.0 behavior, dynamic runtime backend plugins,
per-pipeline crates and a general plugin architecture unless the desired
contracts demonstrate a concrete need. Geometry algorithms, CRS parsing,
projection adapters and storage/publication logic are all in scope for
replacement. Their rewrite or retention is determined by independent accuracy,
fidelity, failure and resource evidence, not by the cost of changing them.
Do not weaken accuracy requirements to make an existing implementation pass.

## Migration sequence

1. Define foundation contracts and evidence before implementation. Resolve the
   limited F0 requirements, classify failures and publication states, and run
   the executable state model with negative controls. This is design evidence,
   not proof of current production or a real filesystem implementation.
2. Implement F0 using a real package-conversion path through Rust, CLI and Python:
   validated request, owned staging/sealed artifact, job-local observer/abort
   handling, completed-file publication and typed outcome/error mapping. The
   runtime knows no converter identities; the format encoder knows no jobs.
   Keep this scope small enough to review and prove independently.
3. Extend the foundation only with the next real consumer. Directory publication
   needs its own platform/recovery evidence. Vector feature transactions need
   their own accepted/rejected/fatal model. Do not add unused generic machinery
   to anticipate either during F0.
4. Implement the mesh request/algorithm slice against its independent coordinate,
   fidelity and resource contracts. Continue proof/disposition audits across
   every subsystem; untouched code does not earn retention by omission.
5. Introduce justified typed format/domain models and complete module ownership
   migrations. Resolve separate CLI packaging only with tested install/release
   consequences. Keep mechanical moves separate from semantic rewrites where
   that helps review.
6. Run final candidate acceptance, publish migration/default guidance and attach
   evidence to #113. Merging these audit documents or only F0 does not close the
   release gate. No implementation schedule or speedup is asserted by this audit.

## Discipline for the first implementation PR

- Fix ownership or representation when a branch is compensating for ambiguous
  state. Do not add another compatibility flag to a loosely related options bag.
- Normalize syntax once in adapters, validate domain choices once in the core,
  resolve execution capabilities once per operation, and preserve checks whose
  answer depends on actual streamed data. These are different boundaries.
- Use finite enums/owned values for real alternatives. Avoid a universal
  converter trait, plugin registry, policy dictionary or ever-growing context.
- Keep only the smallest interfaces required by the first real consumer and
  test seams. Add a shared abstraction when a concrete invariant or second
  consumer requires it, not merely because two functions look similar.
- No converter-name switches in runtime, no job imports in archive codecs,
  no CLI/Python types in domain validation, and no message-string classification
  of infrastructure failures. Enforce visibility/dependency rules in the F0 PR.
- Require independent failure/interleaving tests and end-to-end evidence. Lines
  moved, fewer `if` tokens, or agreement with historical outputs are not success
  criteria. Stop F0 when its declared slice is proven; broaden by a new scope.
