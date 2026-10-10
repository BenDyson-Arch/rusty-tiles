# F1c2/W1/package removal: independent final review

Review date: 2026-10-10. Baseline:
`29b96e501b1426e08cf7d3f01ba93964c6618a57`. Reviewer was not a production author;
review changes are independent probes and this record. The coordinator alone
owned Cargo/builds. Review followed AGENTS.md, the architecture gate, and the
F1c2, W1 and package-removal contracts. This record supersedes neither the
preimplementation audits nor the broader foundation/release gates.

**Status: local PASS; final S3 source, independent controls, native tests,
consumer replays and installed-wheel acceptance reviewed.** No blocking source
or local evidence finding remains open in the reviewed implementation. The final lint cleanup binds the
private identity Plan to its original immutable Geometry and moves the model
test module after production functions; static review found no representation
change. S2 receipts are preserved separately with an `-s2` suffix.
Official Blender and platform CI are explicit PR gates, not local executions.

## Exact candidate

The corrected portable executable is
`/home/bend/.cache/rusty-tiles-f1c2-evidence/rusty-tiles-portable-verified`, SHA256
`6f26363b4f9fd5270a5faf186fbaab8b3b139ebf13c9c788bb52d368eb1ea4f1`.
The coordinator's verified-source-artifacts.json records 91 production inputs,
aggregate SHA256
`22f4b82e6379d889441be2f0f9e364aa51570ec6f391b3c98c95be58ef0d8444`.
The reviewer independently checked all 91 current file hashes against that
inventory; all matched. Probe drivers check executable identity before/after
their executions. Final source commit:
`4e1bda6de8d9f8be9ee4f776d51671420ff8faf5`. The reviewer checked HEAD and all
production input hashes before replay. Review/provenance additions after that
commit do not modify the frozen production inputs.

The final native-geospatial/native-jpeg executable is
`/home/bend/.cache/rusty-tiles-f1c2-evidence/rusty-tiles-native-verified`, SHA256
`9f7378c7bf2a81fb8f04a9fc667b2e55a4778f96bf51388262b995ea80a337b7`,
with the same production-input identity. The installed Linux x86_64 abi3 wheel
has SHA256 `3aa409052ab798588a9ca0056a307bb77662bddbd2af823593f89ecb26b35225`.
The reviewer independently checked the portable, native and wheel artifact hashes
against the [durable source/artifact manifest](../receipts/source-artifacts.json).
All matched. The packaged Rust source archive has SHA256
`5cc5f6e6c0da5a8fc19e79261f6e951e039b8cebf05f2610d5e4abe2fd502619`;
the reviewer independently checked all 87 production core source, preview and
build files against the worktree. Cargo-normalized package metadata and the
separate wheel binding are outside that exact-core-file comparison.

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
  Linux observed RSS is 62,904/62,480 KiB, sampled descriptors 6/4, sampled scratch
  129,566,617/0 bytes. These are executions with 10 ms sampling, not total memory,
  scratch or descriptor promises for every admitted model.
- [Actual unchanged textured browser](../probes/unchanged-model-browser-final.json):
  eight ordinary Cesium 1.146.0/Chromium 153.0.8010.12 Local/Wgs84 runs. Both good
  models render two unchanged triangles and pick both independent interiors;
  exact source/buffer/image bytes and percent-escaped/hard-link URI closure pass.
  Wrong-root, missing-alias and zero-omission-error controls render/pick none.
  Independent Decimal100 checks the omission error against all eight signed
  box-axis combinations. No shader or modelMatrix repair is used.
- [Final identity reader](../probes/identity-final-portable.json) and its
  [native replay](../probes/identity-final-native.json): 26 positives, 11 sensitive
  artifact controls, six source refusals and independent decoder self-controls.
  Includes 100,000 authored triangles, IDs above 65535, exact 8 MiB names,
  selected/sole scenes and empty-name sentinel semantics.
- [Normal picking](../probes/picking-final.json) and
  [all-empty-label picking](../probes/empty-names-picking-final.json): each nine
  mode/variant runs with twelve independent triangle targets. Affected set
  swaps retain geometry and picks while producing zero correct associations;
  unaffected sets retain all twelve correct queries. Sparse rows exceed each
  primitive's featureCount while remaining within the shared table count.
- [Khronos](../probes/khronos-final.json): 23 admitted source/leaf/synthetic
  positives have zero errors/warnings; two negative padding/STRING controls
  produce their expected diagnostic. This checks core format, not extension
  association truth.
- [W1 independent author reader](../probes/wrapping-final-portable.json) and
  [native replay](../probes/wrapping-final-native.json): 22 cases, Decimal80
  original geometry/placement, exact complete resource bytes/aliases, report
  agreement, sibling URI authority and explicit exclusions. Its omission-error,
  refinement, bounds, transform, alias and collision controls are sensitive.

The [final consumer ledger](../probes/final-browser-evidence-ledger.json), SHA256
`ddd06b601cd62dad4c14986542d688bc728e3f0da2694b61f03e8820c2ee3c36`,
binds all six final consumer executions and current drivers to the source commit,
91 input hashes and portable executable. It also records the replayed F1c1
four-world-placement Cesium controls and F1b3 pinned Three PBR comparison:
source/output RGB delta zero with ten deliberately changed channel/companion
controls producing deltas 28–143. Those finite appearance/placement checks retain
their original scopes.

Coordinator-owned final-source tests passed: portable 275 library cases plus
four CLI, fourteen mesh and fourteen W1 cases; native 319 library cases plus
five CLI, fourteen mesh and fourteen W1 cases. Both workspace/all-target
default/native Clippy runs passed with warnings denied; two Python Rust
error/recovery cases passed. The reviewer read the logs and verified the
source/artifact binding. Full portable/native S2 integration suites also passed;
their earlier executions remain labeled separately from final-source focused
tests and independent replays.

[Installed Python](../receipts/wheel-python.json) passed all 42 API cases under
Python 3.12.13 with empty PATH; the installation also passed the independent C1
fixture runner. [Installed distro Blender](../receipts/wheel-blender.json) passed
the same 42 cases under Blender 5.2.2 LTS, build `d13f752e3b9c`, Python 3.14.7,
with empty PATH. Both receipts bind the final wheel hash and record fresh
installed package locations. The reviewer inspected the successful receipts
and compressed logs. Distro Blender is supplemental local evidence and does
not satisfy the official Blender 4.5.14 CI gate.

The reviewer also checked the final implementation and evidence documents,
current public-surface inventory, migration/removal contracts and their gate
statements against the implementation and receipts. The records distinguish
removed public APIs from retained private legacy paths and preserve the broader
release gates.

## Proof limits and final disposition

Numerical containment establishes the finite decoded-f64 authored/raw-normalized
mathematical profile, not arbitrary GPU f32 precision. Browser proof establishes
finite unchanged-resource loading, rendering/target picking and control
sensitivity, not general photometric shader fidelity or unique selection among
coincident surfaces. W1 adds no generated feature metadata to unchanged sources.
Imported extras/extensions, animation and other excluded source capabilities do
not gain acceptance by exact byte copying. Sampled resource evidence is not a
whole-job bound. Reference manifests require stable live resources.

Official Blender and platform CI must pass
on the final PR candidate; they have not been executed by this local review.
The broader #113/#120/#121/#125/#126 gates remain
open. This review authorizes no release, general legacy deletion or acceptance
outside the declared slices.
