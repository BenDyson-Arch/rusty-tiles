# Coarse proxy identity and appearance audit

Audit source: `a5f618e7cf8e3ba82a4b3e9afcfa55c101384da1`.
This records static source observations, representation choices and outstanding
proofs. No conversion, browser experiment, build or acceptance execution was
performed for this audit. Existing implementation is evidence to examine, not
the specification. This document does not close #121 or authorize release.

## Source observations and dispositions

- `src/mesh_archive/source.rs:32-54`: `SourceIdentity` is one original
  node/mesh/primitive/triangle tuple, and every `Triangle` requires one. The
  decoder fixes the original triangle ordinal at lines 1417-1497, after choosing
  the selected instance and before material regrouping. Reflection changes
  corner order without changing that ordinal. Retain this representation for
  full detail; do not invent a tuple for a synthetic proxy face.
- `src/mesh_archive/encode/identity.rs:11-92`: the leaf plan assumes unique
  original tuples, rejects duplicate identity multiplicity, and always emits
  `source_primitive` and `source_triangle` feature sets. Retain for full detail;
  a separate finite proxy identity plan is required. Relabeling its triangle
  table or assigning each synthetic face a representative original is incorrect.
- `src/mesh_archive/encode.rs:153-174,268-361`: full-detail groups preserve material
  and companion layout; each channel follows the same expanded corner order.
  Retain that full-detail behavior with independent replay. Proxy encoding can
  share low-level attribute/buffer writing, but source associations must come
  from a prepared proxy plan rather than `source::Triangle.source`.
- `src/mesh_archive/source/material.rs:27-105,163-247`: one private finite plan
  owns all five texture roles and preserves admitted factor/field omission while
  remapping resource indices. This owner should resolve proxy material
  eligibility. Orchestration/codec must not rediscover alpha/default semantics.
- `src/mesh_archive/source/texture.rs` validates bounded images and preserves
  encoded bytes. It provides no proxy texture-bake or visual approximation
  guarantee. Keep unchanged source-faithful delivery for full detail.
- Legacy `src/hlod.rs:106-186,255-318` merges into an IR without material/identity
  ownership, generates missing normals, welds by approximate position, averages
  normals and retains the first UV. `src/texture.rs:834-856` chart unwraps and
  nearest-surface bakes. These are static incompatibilities with the proposed
  contract, not newly executed defects. Replace this path for the first proxy
  slice; no atlas, nearest-surface identity inference or legacy fallback.

## Minimal first proxy contract

An explicitly requested single root proxy represents all unchanged full-detail
leaves. First candidate groups are selected source primitive instances, keyed
by `(node_index, mesh_index, primitive_index)`. Material grouping must never
merge source instances. Mesh simplification can propose new faces; it cannot
create source identities or certify approximation error.

Use private `ProxyGeometry`, `ProxyRegion`, and `ProxyTriangle` representations.
A proxy triangle carries prepared positions and a region ID, not a mandatory
original `SourceIdentity`. A region owns the complete sorted, unique list of
original triangle tuples represented by its source group. Membership includes
every original triangle, including removed faces and coincident/repeated
triangles with distinct ordinals. Groups partition the selected source triangle
set exactly. Their association is region-level provenance, not an assertion
that every member contributes to each proxy face or a nearest-source query.
Preserve disconnected components; do not join them merely because their
positions coincide or because they share a material.

The first profile has one region per source primitive instance. Its exact source
node name remains an optional label, never identity. The four-column membership
representation below also handles several source primitive identities honestly
if a later, separately specified algorithm combines them. Such an extension
must settle multi-node name behavior and component/material ownership before
admission; it must not assign a representative primitive to the entire region.

## Finite generated codec and consumer query

Proxy GLB has one generated class/table, `proxy_region`, under a new fixed schema
ID. Required membership properties are variable-length `SCALAR`, `UINT32`
arrays named `source_node_indices`, `source_mesh_indices`,
`source_primitive_indices`, and `source_triangle_indices`. The four arrays in
each row have equal nonzero lengths; column-wise zipping yields sorted complete
original tuples. All columns can share one `UINT32` array-offset view with
table-count plus one entries; offsets count elements, not bytes. Values are
little-endian and the existing eight-byte buffer alignment is retained. No
normalization, numeric transforms, generic extras or business metadata.

For this single-node-per-region profile, required scalar `source_node_name`
STRING and `source_node_name_present` UINT8 preserve absent versus empty labels
exactly. The finite name budget counts both proxy rows and leaf primitive rows,
and is checked before workspace creation. Membership totals equal selected
source triangles, at most 100,000; four numeric arrays therefore cost at most
1.6 MB, before bounded offsets/names/alignment/schema. This is a payload bound,
not an RSS measurement. Region and emitted proxy face counts are also bounded
by the selected triangle ceiling.

Every expanded proxy face has three identical region IDs in `_FEATURE_ID_0`,
SCALAR nonnormalized FLOAT. `EXT_mesh_features` references its proxy table with
label `proxy_region` and counts distinct IDs present in the encoded primitive.
Grouping faces by material may use a sparse subset of the shared table; IDs
must be below table count, not necessarily below that group's featureCount.
Do not emit a `source_triangle` feature set on synthetic geometry. Full-detail
contents retain their current two tables and both exact source feature labels.
Both metadata extensions remain optional for rendering.

Public Cesium picking can read each membership array using
`feature.getProperty(propertyName)`; the application zips corresponding array
entries. A coarse hit queries the represented region, while a full-detail hit
can query the exact original triangle. Source tuple scope remains the captured
document; no durable cross-document identifier is introduced. Requesting the
full-detail `source_triangle` label must not silently query coarse provenance.
The browser must establish the behavior of an absent label and the chosen
coarse/fine query procedure; parser feasibility alone does not establish it.

Primary format basis is the pinned extension revision
`7182f40bd2bb5c090188efc4f66f2404cba266f0`:
[class property schema](https://github.com/CesiumGS/glTF/blob/7182f40bd2bb5c090188efc4f66f2404cba266f0/extensions/2.0/Vendor/EXT_structural_metadata/schema/class.property.schema.json)
defines `array:true` without count as variable-length;
[property table schema](https://github.com/CesiumGS/glTF/blob/7182f40bd2bb5c090188efc4f66f2404cba266f0/extensions/2.0/Vendor/EXT_structural_metadata/schema/propertyTable.property.schema.json)
defines element offsets and count+1 storage.
[Cesium 1.146 MetadataTableProperty source](https://github.com/CesiumGS/cesium/blob/1.146/packages/engine/Source/Scene/MetadataTableProperty.js)
reads variable-array offsets and returns their scalar arrays. This is static
consumer feasibility, pending actual browser execution against pinned bytes.

## Appearance policy and current textured uses

First explicit proxy eligibility is selected positions-only geometry with no
NORMAL, TANGENT, UV0, UV1 or COLOR channels, using omitted/OPAQUE alpha mode and
no material texture bindings. Missing material remains missing; admitted core
PBR factors, field omissions, emissive factor, alpha fields and doubleSided are
preserved exactly for each group. No material defaults are invented. Excluded
selected companions, MASK or any of the five texture bindings are Unsupported
for an explicit proxy request before workspace creation. Eligibility belongs
to source/material preparation once, not codec checks against raw JSON.

This is a deliberate narrow approximation profile. It retains the existing
full-detail admission of textured core PBR, authored normals/tangents, both UV
sets, vertex colors and MASK. It neither retires those typed full-detail uses nor
establishes replacement of legacy textured HLOD/atlases. An explicitly requested
unsupported proxy does not silently become a full-detail-only archive.

Preserving factors does not establish unchanged shading after changing geometry:
normals derived by a renderer may change with synthetic faces. Certified
geometric distance is separate from normal deviation, UV/color interpolation,
material texture signals, alpha coverage and final rendered pixels. Do not put
texel size, sampled color deltas or normal deviation into a metres geometric
error field. A geometric metric also cannot guarantee preservation of textured
appearance: arbitrarily small displacements can move a UV discontinuity or
alpha-mask silhouette. Any later textured proxy needs a declared appearance
contract with independently proved seam/binding/tangent/alpha behavior. An atlas
or unbounded generic appearance IR is not necessary for this first slice.

## Independent checks and API/report consequences

- Author independent sources with shared meshes in distinct nodes, reflected
  winding, duplicate and absent/empty names, repeated triangles, material
  regrouping, disconnected components and enough topology for real reduction.
  Decode output without production source/metadata plans. Prove root memberships
  partition the selected original set and full-detail tuples/geometry/materials
  are unchanged. Check each proxy face's region against its candidate group.
- Sensitive controls must swap region IDs/tuple columns, omit removed source
  triangles, insert an unselected instance, alter original ordinals, misalign
  array offsets, corrupt names/presence and expose a false source_triangle label
  while leaving rendered positions plausible. Check finite caps before staging.
- Execute browser coarse and fine camera states with natural traversal. Pick
  independently targeted regions, call public getProperty for all four arrays,
  verify exact membership and name presence, then refine and query original
  source_triangle identities. Two distinct nodes sharing one mesh must remain
  distinguishable. Coincident surfaces cannot prove which surface the renderer
  selects; record that consumer limit. Include corrupt controls.
- Replay the current full-detail textured/attribute/material/image and picking
  oracles on final sources. Proxy positives prove exact factor/field retention
  and useful reduced geometry, not general visual equivalence. Exercise each
  selected-only proxy refusal with no published output or fallback.

Keep `MeshRequest` as one intentional operation with a finite explicit
approximation choice: full detail by default, or one root proxy with its validated
geometric-error/target policy. Core owns all domain validation. Rust, CLI and
Python adapters collect equivalent choices; no material-policy switches or
public proxy IR. Existing `triangles` continues to count selected full-detail
source triangles. A typed approximation report should separately record proxy
faces/regions, target and achieved count, certified geometric error and the
declared opaque-factor appearance profile. Count shared images once. Root
proxy inventory and identity/name budgets belong to preparation; serializer
receives validated plans and publication retains F0 lifecycle ownership.

Geometric certificate, target-budget failure semantics, bounds and hierarchy
error policy remain owned by the approximation/spatial audit and must be settled
before implementation. This identity/appearance audit does not claim those
proofs, browser success, performance, broad acceptance or release authorization.
