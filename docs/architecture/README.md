# Architecture foundations for 0.4.0

Status: proposed design and ongoing audit. Tracking issue: [#113](https://github.com/BenDyson-Arch/rusty-tiles/issues/113).
Baseline: `8dfd74bd87dd23c86278e98d736c3f5912c246cf` on `develop`.

0.4.0 promotion, tagging and publication are blocked by #113. Existing release
acceptance describes the previous candidate, not the redesigned library. This
proposal does not change production behavior or complete that release gate.

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

## Audit documents

- [Public API and mesh vertical slice](api-contract.md)
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
| `bbox.rs`, `georef.rs`, `crs.rs` | glTF ingestion mixed with bounds/math; coordinate definitions, transforms, strict eligibility and regression fixtures | Spatial primitives separated from format ingestion; existing CRS algorithms and strict policies retained |
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
    terrain/              sampling, simplification, quantized mesh
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

1. Define the desired contracts independently of current behavior and maintain
   the component evidence ledger. Add deterministic reproductions and assess
   oracle independence. Convert proven violations into desired-behavior
   regressions; never turn observations of defects into compatibility promises.
2. Implement the mesh request slice through Rust, CLI and Python, including
   placement, defaults, capability checks, reporting and pre-publication failure.
   Retain or replace algorithms only after their evidence gate; isolate contract
   and algorithm changes where useful for reviewing cause and effect.
3. Extract mesh models/helpers and archive serialization, make metrics job-local,
   and narrow public exports. Keep mechanical moves separate from behavior fixes.
4. Implement vector feature/candidate transactions and common publication/error
   contracts. Apply the request/result conventions to points, vector, raster,
   terrain and packaging without forcing identical processing stages.
5. Introduce typed generated hierarchy and finish module reorganization. Reuse
   independent validated oracles as checks, rather than sharing assumptions
   between writer and validator or trusting a test solely because it exists. Resolve CLI packaging only with tested install
   and release migration.
6. Run the acceptance matrix, update user-facing migration/default guidance and
   attach final-candidate evidence to #113. Merge design or probe work without
   closing the issue; required implementation and final acceptance close it.

The first implementation slice is specified in [the API proposal](api-contract.md).
The runtime contract records commit points and error guarantees that slice must
honor. No implementation schedule or performance improvement is asserted by
this audit.
