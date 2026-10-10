# F1d1: one certified root proxy

Implementation contract for #121, settled from the error, identity/appearance
and pipeline fan-out audits against `a5f618e7cf8e3ba82a4b3e9afcfa55c101384da1`.
Acceptance remains pending final-source execution and separate nonauthor review.
This slice does not close #121, #113 or authorize a release.

## Domain and ownership

`MeshRequest` has one finite approximation choice: `FullDetail` (the default)
or `RootProxy { triangle_limit, max_error_metres }`. Both limits are positive;
error must be finite. Core preparation validates this choice once. Full detail
retains its current source/attribute/material/resource profile. An explicitly
requested proxy admits only selected positions-only geometry, omitted/OPAQUE
alpha, and untextured core PBR factors. No normals, tangents, UVs or colors are
admitted for that choice. Eligibility is owned by source/material preparation.
Unsupported choices fail before workspace creation, with no full-detail fallback.

Preparation owns source capture, placement, unchanged leaf partitioning, proxy
candidate, provenance, certification and resource admission. Serialization
consumes these validated plans. Existing F0 owns the single run, cancellation,
workspace cleanup, archive sealing and completed-file publication. Approximation
math knows no paths, reports, runtime jobs or CLI/Python types.

## Geometry and certificate

Candidate generation uses meshopt only as an untrusted proposal. Its indices
must address original decoded f32 positions. Preserve authored corner vertex
keys through source decoding; never infer connectivity by position welding.
One region represents one selected `(node, mesh, primitive)` instance. Simplify
its authored connected components independently, retain every component, and
never create a face joining components. Require a nonempty candidate, actual
total reduction and total face count at most the requested limit. Failure is
`Unsupported`, not an optimizer fallback. No later position quantization occurs.

Certification compares complete decoded f32 triangle supports in local metres,
independently for each region, in both directions. For source face S and target
face T, choose three explicit witnesses q_i in T for S's corners. Convexity
gives a whole-face bound `b(S,T) = max_i ||s_i-q_i||`. Then
`max_S min_T b(S,T)` bounds directed surface distance; the maximum of the two
directions bounds symmetric Hausdorff distance. Degenerate faces remain in the
comparison. No sampling, skipped invalid queries or optimizer scores certify
error. Identical complete faces may return exact zero.

Witness weights are nonnegative integers summing exactly to 2^24, scaled by
2^-24. Ordinary projection arithmetic proposes them; interval reconstruction
and outward subtraction, squaring, summation and square root certify distance.
Vertices and projected edge/interior witnesses are candidates. Reject invalid
indices, empty supports or nonfinite arithmetic. The certificate is bound to
the exact positions serialized in the root and leaves.

Exhaustive work is admitted before workspace creation: checked sum of
`2 * original_region_faces * proxy_region_faces`, at most 16,777,216 face pairs.
Comparisons stream without a resident pair matrix. Cancellation checkpoints
must occur in bounded chunks. This ceiling bounds pair work, not total RSS.

This initial certificate uses no proof subdivision. It can be loose even when
the two surfaces coincide: an 8 m triangle and its four subdivisions can certify
4 m although their true distance is zero. The requested maximum fails closed
when the bound is too loose. Independent probes must demonstrate both this
limitation and a useful actual reduction. Later tightening requires a separately
proved complete-coverage subdivision, not a sampled fallback.

The emitted root geometric error is the requested positive `max_error_metres`,
only after the actual certificate is no greater. This is an explicit conservative
budget and permits refinement when actual surface error is zero; no implicit
epsilon/floor is invented. The report records actual certificate separately.
Leaf errors are zero. Root content uses REPLACE refinement over unchanged leaves.
Top-level omission error is separately owned and at least both the conservative
bounds diagonal (with the existing explicit 1 m routing minimum) and root budget.
Bounds enclose root plus leaves; original-position candidates permit the leaf
union to supply them. The bound is local to stored payload coordinates. 3D Tiles
1.1 transform scaling applies; this is not an independently proved exact world
Hausdorff certificate for a finite stored placement matrix.

## Identity and appearance

Synthetic faces carry a private region ID, never an invented `SourceIdentity`.
Each root `proxy_region` row owns all original triangle tuples in that region,
including removed, coincident and repeated faces. Membership partitions the
selected source set exactly. Four variable UINT32 arrays hold node, mesh,
primitive and triangle indices, equal-length and tuple-sorted. Exact optional
node labels retain absent/empty presence. `_FEATURE_ID_0` exposes `proxy_region`
with `EXT_mesh_features` and `EXT_structural_metadata`; synthetic faces expose no
`source_triangle` feature label. Full-detail metadata is unchanged. Existing
8 MiB emitted-name admission includes both proxy and leaf rows.

Untextured core PBR factors and field omissions remain exact per source group.
The metric proves neither topology, multiplicity, normal deviation, appearance
nor pixel equivalence. Renderer-generated normals can change. Textured HLOD,
atlases and companions require a later explicit appearance contract; no texel
error is inserted into a metres geometric field. Public Cesium queries and
natural coarse/fine traversal need artifact-bound browser evidence.

## Disposition and acceptance

Retain, subject to replay, current captured-source admission, full-detail leaf
encoding, identity, material factors, placement and F0 lifecycle. Replace the
leaf-only root/error construction with one prepared finite root plan. Replace
legacy sampled/grid error, approximate welding, texture-error mixing and epsilon
monotonicity for this path. Retain meshopt only for candidate generation. Keep
legacy HLOD/grid/atlas modules pending their remaining callers' migration; this
slice does not establish global legacy API removal.

Report schema becomes 6, profile `f1d1-root-proxy-gltf-v1`, with a typed
approximation report distinguishing full detail and root proxy, counts, declared
budget, actual bound and comparison work. Rust, CLI and Python expose equivalent
finite choices; adapter syntax requires both proxy arguments together.

Acceptance requires independent exact-rational geometry controls (spike,
interior hole, degenerate and mixed-magnitude faces), useful reduced proxies,
original-key component controls, full leaf/resource/PBR replay, complete proxy
membership and corrupted controls. Exercise invalid/error/work refusal before
staging, cancellation/observer and producer failures under the same F0 ownership,
Replace preservation, cleanup and installed adapters. Pin source, binaries,
wheels, oracle drivers and browser artifacts. Regression green alone is not
acceptance. Recursive HLOD, appearance approximation, general CRS, external
encoders, implicit delivery and global legacy API removal remain separate gates.

Proof references: [CGAL bounded Hausdorff processing](https://doc.cgal.org/6.2/Polygon_mesh_processing/index.html),
[OGC 3D Tiles 1.1](https://docs.ogc.org/cs/22-025r4/22-025r4.html), and the pinned
format/consumer references in the [identity audit](../../bench/architecture_audit/mesh_approximation/audits/identity.md).
