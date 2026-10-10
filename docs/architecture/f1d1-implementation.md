# F1d1: certified root proxy over full-detail leaves

This is the first implementation slice of #121, under the #113 foundation work
blocking 0.4.0. It implements the [settled contract](mesh-approximation-contract.md)
without closing either issue. Full-detail conversion remains the default.

`MeshApproximation::RootProxy` opts into a reduced root GLB over the existing
full-detail leaves. The request supplies a positive triangle limit and finite
positive error budget in local metres. CLI and Python expose the same paired
parameters. Report schema 6 distinguishes full detail from the bounded
`f1d1-root-proxy-gltf-v1` profile; no legacy report adapter is introduced.

## Ownership and admission

The mesh consumer owns approximation, metadata and geometric-error policy. F0
still owns the single attempt, private workspace, cleanup and archive publication.
Runtime has no mesh policy. Eligibility, candidate construction, complete-pair
work admission and certification finish before workspace creation. Unsupported
appearance, an impossible reduction, excess work or a failed certificate returns
a typed failure without publishing or silently selecting full detail.

Original vertex indices now survive source decoding and reflected winding.
Connected components use those keys within each original node/mesh/primitive
instance. Positions alone cannot distinguish coincident components. Each
component must retain a nonempty proposal; each original primitive instance is
one metadata region. Meshopt proposes original-position geometry but supplies no
trusted error value. The private proxy type carries complete region membership
and the certificate into encoding. It never invents source-triangle identities.

The admitted proxy appearance is opaque, untextured, positions-only core PBR
factors, including exact material omission. Normals, tangents, UVs, colors,
textures and masked materials are unsupported in this mode. Full-detail leaves
retain their existing wider profile and exact source identity.

## Surface bound and hierarchy

For each source face, three explicit dyadic witnesses on a candidate face bound
the distance of the whole source face by convexity. Certification checks every
face pair in both directions, within its own region. Outward interval arithmetic
encloses witness reconstruction, distances and the final bound. Vertex sampling,
meshopt scores and ordinary floating-point projections cannot authorize output.
The checked work ceiling is 16,777,216 face comparisons; the loops check aborts,
but an individual meshopt call has no proved cancellation-latency bound.

The root's geometric error is the requested positive budget, after the measured
certificate passes it. The report records the actual certificate separately.
Root content uses REPLACE and leaf errors remain zero. Top-level omission error
encloses the root error and outward bounds diagonal, with an explicit routing
minimum. The certificate concerns decoded local f32 geometry; it does not claim
an exact world-space Hausdorff bound after finite placement arithmetic.

The scheme deliberately uses no proof subdivision in this slice. It can return
a conservative positive bound even for equal surfaces with different
triangulations. Rejection at a tight budget is preferable to an uncertified
fallback. A future tighter certificate should retain the same witness contract.

Proxy metadata uses `proxy_region` and four sorted variable-length UINT32 arrays
of every original source tuple in the region, plus exact optional node labels.
Fine content retains `source_primitive` and `source_triangle`. Public Cesium
queries establish the coarse-to-fine distinction; synthetic faces never claim
to be exact original triangles.

## Evidence and corrections

The [independent evidence](../../bench/architecture_audit/mesh_approximation/README.md)
pins production source, portable/native/release CLIs, installed wheel, artifacts,
drivers and consumers. Five authored positive cases, admission refusals and
sensitive corruptions cover geometry, materials, original-key components,
membership, count/work reports and bounds. A six-region fixture reduces 192 to
44 faces and naturally refines to all 192 leaf faces at Cesium SSE 16. Public
queries return six complete coarse memberships and six exact fine tuples, both
locally and at a WGS84 anchor with independently calculated ECEF cameras.

The strict rational child-box check found a 2^-52 m containment defect in the
first candidate. The correction outwardly encloses child endpoints before root
union; pinned corrective artifacts pass the same sensitive check. Separate
[nonauthor review](../../bench/architecture_audit/mesh_approximation/review.md)
also records checker corrections and the limits of interval and component proof.

Portable and native Rust tests/lints, full-detail independent profile replays,
127 Python CLI tests (two skips), 43 installed-wheel API tests with empty PATH,
and official Khronos core-format checks passed locally. Khronos does not validate
the metadata extension semantics. The near-limit optimized probe completed
16,760,832 comparisons in 14.81 seconds; sampled resources are observations on
one host, not memory, latency or performance guarantees. CI replays the smaller
geometry, format, traversal and placed-consumer probes.

## Remaining #121 gates

This establishes one certified coarse level. Recursive approximation, tighter
proof subdivision, textured/attribute-preserving proxies and measured appearance
budgets remain separate work. Broader malformed/resource stress and consumer
coverage must follow their own contracts and receipts. No current evidence
authorizes recursive error accumulation, topology guarantees, arbitrary placement
accuracy, appearance equivalence or release. These gates remain open in #121;
the bounded root implementation must not become a compatibility layer for the
remaining legacy job/report APIs.
