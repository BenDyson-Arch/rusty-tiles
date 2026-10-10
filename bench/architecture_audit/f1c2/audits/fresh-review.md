# F1c2/W1/package removal: independent final review

Review date: 2026-10-10. Baseline:
`29b96e501b1426e08cf7d3f01ba93964c6618a57`. Reviewer was not a production author;
review changes are independent probes and this record. The coordinator alone
owned Cargo/builds. Review followed AGENTS.md, the architecture gate, and the
F1c2, W1 and package-removal contracts. This record supersedes neither the
preimplementation audits nor the broader foundation/release gates.

**Status: final source and independent portable controls reviewed; final source
commit and native/wheel/platform integration bindings pending.** No source defect
remains open in the reviewed corrected implementation. Final acceptance is not
claimed while those bindings remain pending.

## Exact candidate

The corrected portable executable is
`/home/bend/.cache/rusty-tiles-f1c2-evidence/rusty-tiles-portable-final`, SHA256
`92815505d581b8df3aa2a53bd388ca84b9cab394fb35cd6c925524d6e7fa6d79`.
The coordinator's final-source-artifacts.json records 91 production inputs,
aggregate SHA256
`8598d166ceb88796f02553cacfd28debc783d0b65131b2003145d794875ff7b0`.
The reviewer independently checked all 91 current file hashes against that
inventory; all matched. Probe drivers check executable identity before/after
their executions. Final source commit: pending coordinator pin.

Earlier failing executions used executable SHA256
`d461c38d88cdab3ae643a16bec8b261c982940b9392a362d93fee9986c7c9bbb`,
production aggregate
`669242056050f2e040f9f6e33e6557c92ed22fbcc57695256f80f4f2743dd7d7`.
Those failures and feasibility repairs do not constitute corrected-candidate
acceptance.

## Findings and closure

| Classification | Finding | Corrected owner and executed closure |
| --- | --- | --- |
| Executed defect | Standalone CLI reported a nonexistent `tileset.json/conversion.json` sidecar. | CLI now returns inline modelReport and null conversionReport. Independent [before](../probes/review-manifest-report-defect.json)/[after](../probes/review-manifest-report-fixed.json) probe checks the real output inventory. |
| Executed defect | `source.gltf/data.bin` or `source.glb/data.bin` collided with the synthetic archive source file as an ancestor; failure was Io after output-parent creation. | W1 inventory rejects equal or descendant source names as Unsupported before geometry/workspace/staging. Independent [before](../probes/review-synthetic-prefix-defect.json)/[after](../probes/review-synthetic-prefix-fixed.json) covers both envelopes, preserved sources and absent output parent. A sibling prefix without a slash remains admitted by the owner regression. |
| Executed design/consumer defect | Top-level tileset error zero caused ordinary Cesium traversal to visit/request/render no content. | W1 separately computes outward conservative box diameter with a one-metre floor as tileset omission error; root full-detail error stays zero. Final unchanged browser executes both placements and a zero-error control. The earlier failure receipt is preserved; manifest-only repairs are explicitly feasibility evidence. |
| Static format gap | W1 root omitted its refinement declaration. | Pure writer emits root refine REPLACE. Independent source/manifest reader and final browser inspect the emitted value. |
| Static documentation gaps | Current surface inventory still advertised removed public wrapping/pack exports; legacy attribute guidance overstated W1 admission. | Inventory distinguishes current typed APIs from historical declarations and private residuals. Guidance names the bounded wrapping profile and caller-authored tilesets for excluded attributes. |

The primary [tileset schema](https://github.com/CesiumGS/3d-tiles/blob/4d781014b52294759834018a931223b98ac1ce47/specification/schema/tileset.schema.json)
distinguishes the error of omitting a tileset from the [tile schema](https://github.com/CesiumGS/3d-tiles/blob/4d781014b52294759834018a931223b98ac1ce47/specification/schema/tile.schema.json)'s
refinement error. [Pinned source evidence](../probes/review-geometric-error-spec.json)
records both exact source files/hashes. The chosen diameter/floor is an explicit
finite visibility policy, not a new LOD or arbitrary-distance visibility claim.

## Source, ownership and domains

The reviewer inspected final public exports, Rust/CLI/Python routes, source
admission and discovery, binding/capture associations, partition and encoding,
original-node interval mathematics, W1 inventory/materialization/reporting,
publication/error/cleanup paths, validator URI decoding and package/implicit
caller migration.

Source discovery owns authored node/mesh/primitive/triangle indices before
partition, regrouping or reflection. The private metadata codec consumes those
keys and exact optional UTF-8 labels; names do not become IDs. Sorted leaf tables,
sparse primitive subsets, per-primitive distinct featureCount, float corner IDs,
STRING offsets/empty sentinel and declared GLB padding agree with the specified
finite representation. Name-budget admission happens before workspace creation;
one leaf metadata payload is materialized at a time.

W1 shares admitted source/capture/placement owners. Original authored-node bounds
are evaluated before f32 baking with directed intervals covering raw and
normalized quaternion conventions, intermediate compositions and decoded
referenced positions. The emitted box encloses those intervals; bounds outside
the finite local profile are refused. Captured byte owners are shared while
distinct admitted member spellings and emitted-byte charges are retained. Root
envelope kind, complete unused declared-resource capture, URI authority and
source/destination exclusion remain owned at their existing boundaries.

Both W1 products have one Attempt, required prepared reports, precommit events,
typed failures and completed-file publication. No public package invocation,
second RunControl, string-based failure classifier or legacy Reporter bridge is
introduced. Archive production consumes immutable captured bytes. A sibling
manifest references caller-owned live files; stability through the entire call,
callbacks/aliases and subsequent use is an explicit precondition, not an atomic
directory snapshot promise.

The old public wrapping/direct-manifest and pack mutation/options exports are
removed with repository callers migrated. Read-only pack utilities remain
provisionally public. Private tileset shortcuts, broader mesh, implicit outer
Job/Reporter, raster and other legacy operations remain explicitly outside this
slice. Migration docs record capability reductions rather than silently routing
unsupported input elsewhere.

## Independent corrected-artifact execution

- [Decimal100 original-source reader](../probes/review-original-source-final.json):
  five cases covering authored matrix/nonselected scene, raw near-unit quaternion,
  f32 translation distinction, indexed repetition and high-magnitude cancellation.
  Exact archived source bytes and raw/normalized point containment pass. Three
  controls reject f32-only bounds, normalized-only bounds and wrong-scene bounds.
- [Alias/resource boundary](../probes/review-emitted-alias-final.json): exactly
  64 MiB emitted payload passes; two bytes above fails Unsupported before scratch.
  Linux observed RSS is 57,780/56,868 KiB, sampled descriptors 5/4, sampled scratch
  133,105,561/0 bytes. These are executions with 10 ms sampling, not total memory,
  scratch or descriptor promises for every admitted model.
- [Actual unchanged textured browser](../probes/unchanged-model-browser-final.json):
  eight ordinary Cesium 1.146.0/Chromium 153.0.8010.12 Local/Wgs84 runs. Both good
  models render two unchanged triangles and pick both independent interiors;
  exact source/buffer/image bytes and percent-escaped/hard-link URI closure pass.
  Wrong-root, missing-alias and zero-omission-error controls render/pick none.
  Independent Decimal100 checks the omission error against all eight signed
  box-axis combinations. No shader or modelMatrix repair is used.

Final identity, Khronos, primitive/triangle picking, source/placement replay and
coordinator-owned lifecycle/frontend/package test bindings are being consolidated
with final native/wheel evidence. Their earlier passed receipts remain separate
from this corrected-artifact review.

## Proof limits and final disposition

Numerical containment establishes the finite decoded-f64 authored/raw-normalized
mathematical profile, not arbitrary GPU f32 precision. Browser proof establishes
finite unchanged-resource loading, rendering/target picking and control
sensitivity, not general photometric shader fidelity or unique selection among
coincident surfaces. W1 adds no generated feature metadata to unchanged sources.
Imported extras/extensions, animation and other excluded source capabilities do
not gain acceptance by exact byte copying. Sampled resource evidence is not a
whole-job bound. Reference manifests require stable live resources.

Final native/wheel, official Blender, platform CI and source-commit bindings:
pending coordinator evidence. The broader #113/#120/#121/#125/#126 gates remain
open. This review authorizes no release, general legacy deletion or acceptance
outside the declared slices.
