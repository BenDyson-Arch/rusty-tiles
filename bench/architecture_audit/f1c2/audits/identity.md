# F1c2 identity audit ledger

Baseline develop `29b96e501b1426e08cf7d3f01ba93964c6618a57`; audit date 2026-10-10.
Contract: [f1c2-contract.md](../../../../docs/architecture/f1c2-contract.md).
This ledger distinguishes static findings, profile decisions, feasibility probes
and implementation acceptance. No Cargo build is owned by this agent.

## Static observations

- Source `Instance` currently contains mesh/transform but loses source node index.
  Triangle carries attributes/material without source primitive/ordinal.
- Selected forest admission already ensures single parents, scene selection,
  finite admitted transforms and bounded expanded triangle count. Retain only
  this declared domain, not arbitrary glTF instances or graph semantics.
- Partition retains triangle indices. Encoder groups by material/channel layout;
  those indices are sufficient to carry provenance if source discovery owns it.
- Legacy metadata/node-feature builder exists, but current use groups primitives
  by node and does not establish original triangle identity or a new contract.
- Binding owns immutable source bytes, deduplicated capture storage and output
  exclusion. Wrapping additionally needs root/member spellings; those additions
  are coordinated with the root and wrapping owner.
- Original-node wrapper bounds cannot use node-baked f32 geometry, or blindly use
  producer-normalized quaternion matrices while preserving raw source TRS.

## Profile decisions

Root accepted two always-present labeled feature sets: default primitive-instance
features with node/mesh/primitive and exact optional name, plus triangle features
with four authored indices. Dense IDs are leaf-local table rows. No generic
extras/business metadata is admitted. Name caps are 4096 source UTF-8 bytes and
8 MiB archive-emitted UTF-8 bytes, with pre-staging Unsupported failures. These
are deliberate new limits, not reproduced defects.

Use f32 scalar feature IDs throughout the finite <=100,000-row domain. glTF does
not permit UINT32 vertex attributes; UINT32 metadata property-table columns are
appropriate. Use aligned binary columns and UTF-8/string-offset representation,
without inheriting arbitrary legacy metadata-building policy. Both referenced
extensions remain Draft and optional; output does not require them for rendering.

## Primary reference provenance

[EXT_mesh_features](https://github.com/CesiumGS/glTF/blob/7182f40bd2bb5c090188efc4f66f2404cba266f0/extensions/2.0/Vendor/EXT_mesh_features/README.md)
and [EXT_structural_metadata](https://github.com/CesiumGS/glTF/blob/7182f40bd2bb5c090188efc4f66f2404cba266f0/extensions/2.0/Vendor/EXT_structural_metadata/README.md)
are pinned by source hashes in [spec-provenance.json](../probes/spec-provenance.json).
The current 3D Tiles glTF section links these specifications on the Cesium branch;
Khronos main does not contain these draft extension folders. Consumer source
is locally installed Cesium 1.146.0 / engine 26.4.0, independently pinned in the
probe execution record.

## Feasibility and proof limits

[Consumer prototype](../probes/identity_feasibility.mjs) executed with Cesium
engine 26.4.0: eight real ModelFeature property queries passed, including duplicate
Unicode/NUL strings, absent versus empty labels and UINT32 columns. Four sensitive
association/table/ordinal controls failed as expected. FLOAT integer IDs at
0/65535/65536/99999 remained exact. See [execution](../probes/identity-feasibility.json).
This is independent authored parser/query feasibility, not rendered picking or
production-source acceptance. The parser logs that no GPU property texture can
be allocated with its no-context maximum texture size zero; no GPU was invoked.

[Directed interval prototype](../probes/original_bounds_prototype.py) executed
against 90-digit Decimal references for near-unit raw/normalized quaternions at
high magnitudes, reflection, a depth-128 chain and cancellation. A sensitive
normalized-only control differs from raw authored source by more than 1e-6m.
See [execution](../probes/original-bounds-feasibility.json). This establishes design
feasibility, not full production or arbitrary GPU float32 enclosure.

A primary-schema correction makes featureCount the distinct IDs in each encoded
primitive, independently from shared property-table count. The separate acceptance
agent executed [mixed-group browser feasibility](../probes/sparse-table-browser-feasibility.json)
with Cesium 1.146/Chromium 153: all twelve independently targeted triangles queried
correctly in default, source_primitive and source_triangle modes. Sparse IDs exceed
featureCount-1 but remain within shared table counts. Primitive/triangle ID-swap
controls retained twelve picks and yielded zero correct queries in their affected
mode, with unrelated modes retaining twelve correct queries. This is consumer
representation feasibility on independently authored GLBs, not candidate evidence.

Owned source/encoder changes are implemented, including source keys before winding,
exact name-presence handling, pre-staging repeated-name budget and a separate pure
authored-bounds evaluator. Focused unit cases are authored; coordinator-owned Cargo
execution and final source identity remain pending here.

Remaining gates: independent byte decoder and candidate actual browser picking;
exact-source final evidence and nonauthor review; root-owned platform/package
checks. Original-node bounds and URI/member alias controls remain separate shared
wrapping obligations. No broad legacy mesh deletion or release claim is earned.
