# F1c2: source identity, metadata and picking

Issue #144, within #121/#125/#126/#113. Baseline is accepted F1c1 develop
`29b96e501b1426e08cf7d3f01ba93964c6618a57`. This contract defines the producer
change before implementation. The adjacent typed wrapping/direct-manifest work
has its own contract; opaque wrapping does not acquire generated picking by
assertion. Implementation and acceptance remain separate.

## Supported domain and identity ownership

Retain the independently accepted F1b3 local metre/Y-up static core-PBR source
and F1c1 Local/Wgs84 rigid placement profiles and limits. Every generated full
detail leaf carries source identities by default, with no legacy feature mode.
A selected node index identifies an instance in the admitted single-parent
forest. Shared meshes in distinct nodes remain distinct instances. Group nodes
without geometry do not become features; unused scenes/nodes do not become
selected features. Names are exact optional labels, never IDs.

Source discovery fixes `(node_index, mesh_index, primitive_index,
triangle_index)` before partition, material/channel regrouping and reflected
corner winding. `triangle_index` is the original TRIANGLES ordinal within its
source primitive, for indexed and nonindexed primitives alike. Coincident or
repeated triangles retain distinct ordinals. All IDs are zero-based authored
indices scoped to the captured document, not stable database/business IDs,
filesystem identities, content hashes or output feature row numbers. Different
captured documents may use the same index tuple. No durable cross-document
identity or whole-closure digest is promised.

Source node names remain byte-exact UTF-8 strings. Missing and authored-empty
names remain distinguishable. Each source node name is at most 4096 UTF-8 bytes;
above that is Unsupported during pure source admission. Source JSON's existing
1 MiB ceiling also bounds total source labels. Source extras, imported feature
extensions and arbitrary business metadata remain Unsupported: this slice
creates a finite generated provenance schema and does not silently copy them.
That is an explicit source-domain decision, not a claim of general metadata
migration. Existing accepted sources with larger names newly exceed this profile.

## Generated representation and consumer query contract

Follow the primary draft [EXT_mesh_features specification](https://github.com/CesiumGS/glTF/blob/7182f40bd2bb5c090188efc4f66f2404cba266f0/extensions/2.0/Vendor/EXT_mesh_features/README.md)
and [EXT_structural_metadata specification](https://github.com/CesiumGS/glTF/blob/7182f40bd2bb5c090188efc4f66f2404cba266f0/extensions/2.0/Vendor/EXT_structural_metadata/README.md).
Pinned source digests are in the audit evidence. Both extensions are optional
and appear in `extensionsUsed`, not `extensionsRequired`.

Each leaf has exactly two property tables. Table 0, class `source_primitive`,
has one row for each selected source primitive instance represented in that
leaf, sorted by `(node_index, mesh_index, primitive_index)`. Required columns
are UINT32 `node_index`, `mesh_index`, `primitive_index`, UINT8
`node_name_present` (exactly 0 or 1), and STRING `node_name`. Missing name uses
presence zero and empty stored text; authored empty uses presence one. No
synthetic fallback name, Unicode normalization, trimming or replacement occurs.

Table 1, class `source_triangle`, has one row per original selected triangle
represented in the leaf, sorted by the complete four-index tuple. Required
columns are UINT32 `node_index`, `mesh_index`, `primitive_index`,
`triangle_index`. This table intentionally contains no repeated node-name text.
A client can associate the primitive/name table through the three-index key.
There is no automatic property inheritance between these tables.

Every encoded primitive carries `_FEATURE_ID_0` and `_FEATURE_ID_1`. Both
accessors are SCALAR FLOAT, nonnormalized; each expanded corner receives its
primitive-instance row index and triangle row index respectively. IDs are dense
leaf-local rows, never source IDs. The existing 100,000 selected-triangle ceiling
keeps their integer values exactly representable in f32, including above 65535.
All three corners of each triangle have identical IDs, so interpolation or
nearest-vertex interpretation cannot change association within its interior.
`EXT_mesh_features.featureIds` contains set 0 with label `source_primitive`,
attribute 0/propertyTable 0, then set 1 with label `source_triangle`, attribute
1/propertyTable 1. Each `featureCount` is the number of distinct IDs actually present in that
encoded primitive's attribute. Shared leaf tables can contain rows used by other
material/channel groups; a primitive may use a sparse subset whose row IDs exceed
featureCount-1. Every ID is below the referenced property-table count. IDs are
dense over the leaf tables, not necessarily over each primitive. No null feature
ID is emitted.

Default positional `featureId_0` picking therefore queries the source primitive
instance and provides node/mesh/primitive IDs plus exact optional name through
`node_name_present`. Selecting `featureIdLabel = "source_triangle"` queries the
exact authored triangle tuple. Feature IDs may change across partitions/leaves;
the stored source tuple does not. Cesium runtime picking acceptance requires an
executed browser proof; a metadata parser probe alone establishes only decoding
and query feasibility. Extension-unaware viewers may render unchanged geometry
without supporting these queries.

Metadata is appended after existing geometry/companion views. Regrouping remains
by material and channel layout, without splitting by feature identity. Every
POSITION, NORMAL, TANGENT, UV0/UV1 and COLOR value follows the same existing
corner order; textures/materials/samplers and image closure keep their accepted
meaning. No coordinates, placement, appearance or approximation policy changes.
Metadata numeric columns are little-endian, component-aligned; strings use exact
UTF-8 concatenation and count+1 UINT32 byte offsets. When all stored strings are
empty, the values view contains one unused zero byte to keep glTF's positive
bufferView.byteLength requirement; all string offsets remain zero. All new views start on
8-byte boundaries. JSON chunk end and BIN payload start are at absolute 8-byte
boundaries; BIN uses zero padding to 8 bytes and JSON uses spaces. Declared
buffer.byteLength includes terminal zero alignment bytes; no excess BIN padding
outside the declared buffer is needed.
No 64-bit metadata components, BigInt queries or external schemas are added.

## Finite resource and failure policy

Across all leaf primitive tables, the sum of emitted node-name UTF-8 bytes is
at most 8 MiB. Calculate it after partition and before output workspace/staging;
above the cap is Unsupported. A long label repeated across many primitive
instances or leaves can therefore newly exceed this profile even if its source
JSON fits. This cap bounds deliberate repeated STRING storage; it is not a total
RSS promise.

Total primitive table rows across the archive cannot exceed total triangles:
each nonempty primitive-instance/leaf association accounts for at least one
triangle, and every triangle belongs to one leaf. Triangle table rows equal
selected triangles. At 100,000 triangles, the two f32 per-corner ID streams cost
2.4 MB; UINT32 identity columns cost at most 2.8 MB; presence and string offsets
cost at most about 0.51 MB; names cost at most 8 MiB, plus bounded view alignment
and schema/JSON. This is a bounded payload calculation, not executed RSS evidence.
Only one leaf metadata buffer is materialized at a time. Do not retain a second
archive-wide encoded-metadata copy.

Malformed names remain InvalidInput, semantic/name-size and emitted-name limits
Unsupported. Metadata budget/cancellation checks occur before staging and during
encoding; infrastructure failures use existing typed F0 errors. Report schema
advances to 5, profile `f1c2-source-identity-gltf-v1`. Root owns report counts and
frontend parity. All required report/observer work finishes before publication.

## Ownership, retain/rework/replace/remove decisions

Retain bounded source semantics, immutable capture, placement, partition and
geometry/image encoding only with their prior evidence replayed. Rework source
instance/triangle representation and leaf encoding to carry exact keys. Add one
private finite metadata serializer; do not retain the legacy mutable generic
metadata builder without independent evidence. Source decode owns interpretation;
publication owns paths/lifecycle; the metadata codec receives prepared values.

Wrapping shares capture/source semantics, but unchanged models retain authored
node transforms rather than the producer's f32 bake. Its source-owner interface
must evaluate conservative original-node bounds from the original authored
chain, including raw admitted quaternion semantics. A normalized producer matrix
alone does not establish exact wrapping bounds. Bounds and URI/member-alias proof
belong to that contract; no filesystem access in pure source interpretation.
Source::original_bounds evaluates directed outward f64 intervals for original
matrix/TRS arithmetic, hulling raw and normalized admitted quaternion conventions,
then composing ancestor matrices and applying original POSITION values before
any f32 bake. Global parent walks are valid because admission rejects every scene
root that has a parent. The proof covers mathematical authored chains and ordinary
f64 evaluation within these interval operations; arbitrary GPU f32 transform
precision is separate. Nonfinite intervals are Unsupported. Wrapping must also
admit the resulting intervals against its placement magnitude profile and prove
outward serialized box center/half-axis construction.

This slice replaces new-operation picking absence. It does not delete the
broader legacy mesh route: its projected sources, LOD/atlas, implicit delivery,
compression and encoder options need replacements or explicit removals. Legacy
node-feature implementation removal requires all retained legacy callers to
migrate first. Breaking API changes are allowed; lost capabilities must be
recorded explicitly. #121/#125/#126/#113 remain open beyond bounded acceptance.

## Independent acceptance

Independently author source cases with shared meshes/distinct nodes, duplicate
and Unicode names, absent versus empty names, indexed/nonindexed/repeated and
coincident triangles, unselected scenes, reflected winding, multiple materials
and differing channel layouts, forced partition boundaries and Local/Wgs84.
Decode output bytes independently and check complete source tuples, row mappings,
exact strings/presence, extension references, feature counts, offset/alignment,
and unchanged companions/appearance. Add 65535/65536 IDs and finite name budget
boundaries with refusing cases before staging.

Sensitive controls must swap feature IDs/columns while leaving geometry intact,
swap table references, corrupt string offsets/presence, change original triangle
ordinal after reflection, and make invalid accessor types/counts. Execute Cesium
render/pick queries in both feature modes using independently targeted features
and prove the controls fail. Regrouping/partition controls must reveal wrong
associations even for coincident geometry; coincidence itself cannot prove which
surface a renderer will choose. Record that consumer limit explicitly.

Final acceptance needs exact source/artifact identities, independent nonauthor
review, lifecycle/resource/frontend/installed-wheel/Blender and platform evidence.
A green prior suite or prototype is not implementation or release acceptance.
