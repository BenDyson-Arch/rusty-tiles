# H17 certified geometry LOD (coarse tiles)

> **Superseded direction (draft, effective when the legacy path is removed).** The
> 0.4 owner decision replaces H17's certified geometry and H27's per-texel proxy
> bake with classic octree HLOD (`docs/architecture/hlod-040-decision.md`). Until
> HLOD slice S6 deletes the code, this document describes the shipped default;
> afterwards it is historical. Certified bounds, `proxy_source` and the bake ledger
> are not successor requirements; see the [mesh guide](../MESH.md#coarse-levels-hlod).


Status: implemented on `ml/h17-lod`, light process (tests with controls, one
nonauthor review, perf/memory evidence, consumer proof, CI). Design:
`~/.cache/rusty-tiles-evidence/tiler-lod-design-20261013/design.md` Revision 2
and its recorded user decisions; this document states what was built and where
it departs. Evidence: `~/.cache/rusty-tiles-evidence/h17-20261013/`.

H17 gives every internal tile of H1's tree a certified, **untextured** proxy.
Texture baking for proxies is H13: proxies here carry flat material factors
(every texture binding removed, `MASK` made opaque) and smooth normals. A viewer
therefore shows untextured coarse levels until the textured H8 leaves load.

## Where it runs

Inside the H8 producer (`leaves::producer::produce`), after every leaf is
published and while the capture scratch (baked source, corner depths, frozen
lattice) still exists: the direct certificate terms need the baked source, which
H16 never sees. The leaf directory gains:

- `proxies/f<first>-d<depth>.glb`, one per internal tile;
- `proxies.jsonl`, one row per proxy in POSTORDER (`end` ascending, deeper
  first), which is the order H16 closes tiles;
- `manifest.json` `proxyManifest: "proxies.jsonl"` and the `lod` report.

H16 reads the rows in lockstep while closing tiles (`plan::Builder::
with_proxies`): the identity `(first, end, depth)` must match the closing tile,
the error must be at least every child's, the proxy's decoded bounds join the
tile box, and the proxy becomes the tile's content (explicit
`content/f<first>-d<depth>.glb`, or implicit content with content availability).
Its error replaces the empty-parent `16·diagonal`. A leaf directory without
`proxyManifest` publishes as before.

## Stages per depth (deepest internal depth first)

1. **Pre-pass (once).** One external join over every source corner by welded
   position (normalized f32 bits): per welded vertex `c2` (least depth at which
   its triangles lie in two tiles: a seam vertex), `c3` (three tiles: a seam
   endpoint) and `special` (incident to a degenerate triangle, a source boundary
   or non-manifold edge, or an overflowing incidence group). Joined back to rank
   order with H8's exact `l(v)` as the slim `LodSource` sequence (81 bytes per
   triangle). Welded positions are named by capture's vertex ordinal (the
   index in the position-sorted weld table, joined to rank order with the
   corner depths), so the sort record is 30 bytes per corner: ordinal, the two
   neighbour ordinals, a 40-bit rank and the packed leaf address (at most 31
   digits), corner and degenerate flag. Inputs past those ranges (more than 2^40
   triangles, or more than 2^32 welded vertices) are refused, never truncated:
   capture checks the triangle count before reading geometry and the welded
   vertex count right after the weld.
2. **Children from published bytes.** Leaf GLBs are decoded (meshopt, client
   f32 order) and each triangle is matched to its source by provenance; every
   decoded corner must equal its frozen lattice point. Proxy GLBs are decoded and
   must equal their scratch sidecar (welded keys and facts). `T` is the union.
3. **Islands.** Connected components (welded-vertex connectivity) with no seam
   vertex, an area share below `1/P`, and every vertex within the tile's spacing
   `diagonal/sqrt(P)` of the surface that stays vanish, recorded in
   `lod_vanished` rows (primitive, component key, faces, the child slots where it
   reopens). The largest component always stays. Embedded (non-isolated) small
   primitives are simplified, not vanished.
4. **Seam chains.** A seam vertex is kept iff `l(v) <= d` (a shallower leaf
   shares it), `c3 <= d`, `special`, its frozen hash has at least `k_d` trailing
   zeros (`k_d = round(log4(mean source count / P))` by exact integer
   comparisons, monotone in depth), or it is in the depth's keep set. Every other
   seam vertex inside one seam chain (exactly two border edges) is removed by a
   valid collapse along the chain, so the chain becomes the kept vertices in
   order on both sides.
5. **Seam consensus.** A seam vertex a tile cannot remove validly is kept and
   reported; the depth then keeps it on every tile and re-runs every tile that
   removed it. After a round without such failures the frontier audit (stage 9)
   runs; an unmatched seam edge adds its tile's removed seam vertices to the keep
   set the same way. Keeping a vertex on every sharing tile always agrees, so the
   rounds converge (at most 16; otherwise `InvalidState`, never a hole). Rounds run
   stages 2-6 only; once the depth agrees, one finishing pass certifies, fills,
   encodes and writes every tile with the final keep set. The finishing pass
   reuses each tile's cached topology run, bound to a hash of its forced seam set;
   a missing or mismatched cache is `InvalidState` (a missed re-run), so the
   audited seam edges are the published ones (fills never touch a seam vertex).
6. **Simplification.** Half-edge collapse (QEM, strict cost order with key
   ties) onto existing source vertices to `P - P/16` faces (`P = 8192`; the rest
   is headroom for fills): labels preserved, no
   primitive loses its last face, no vertex is orphaned, link and fold checks,
   border/primitive-seam vertices only along their feature edges, and
   true-boundary decimation never reaches a non-boundary seam vertex. If the
   strict pass leaves more than `P`, a relaxed pass also moves topology-locked
   interior vertices (never onto a seam vertex). A tile still above `P` is reported
   (`splitProvenanceTiles`: its provenance needs several tables, so CesiumJS picks
   on it are unreliable); above `2P` it is over budget; above `4P` `Unsupported`.
7. **Encode and certify.** Positions are encoded on the frozen H8 lattice at the
   tile's depth with `l(v)` (the plan is recomputed and must equal H8's; its
   SHA-256 is bound into every row); the GLB is decoded again and certification
   reads only decoded bytes. Distances are outward-rounded interval f64 (TwoSum
   and FMA residuals, upward sqrt): a whole-patch witness is the maximum corner
   distance to ONE target triangle (distance to a convex set is convex), refined
   by dyadic midpoint subdivision with explicit rounding slack. Per direction:
   - composed: `S->A <= up(h(S->T) + max e_c^->)`,
     `A->S <= up(max e_c^<- + h(T->S))`, `T` including vanished faces;
   - direct against the baked source range, above children that include a proxy
     (above leaf-only children `T` is the decoded source itself): source->proxy
     makes one capped query per source triangle (parallel over fixed blocks; a
     budget that ends early keeps the node's source box against a real proxy
     point), so the composed term cannot add up level by level at shallow tiles;
     proxy->source keeps per-face minima over 65,536-triangle chunks (any
     complete chunk is a valid bound) and is attempted only for ranges of at
     most four chunks, since a whole-face witness against small source triangles
     is loose;
   - each direction takes `min(composed, direct)`; `e_raw` is the larger;
     published `e = max(e_raw, children's published)`.
   Per-primitive proxy->source bounds `e_A` (same-primitive witnesses) are
   exported to the sidecar for parents' fills.
8. **Fills.** Simple closed border loops of `S` with no seam vertex, one label,
   at most 64 vertices, fanned from the least key (orientation against the loop's
   faces, no existing diagonal, no degenerate decoded face, and the outline rule
   below) are filled only if
   `b_A(F) = up(h(F->T_A) + max_c e_A(c)) <= e_open`, the unfilled certificate,
   and only while the proxy stays within `P` faces. The seam-vertex exclusion is
   checked again on every accepted loop (`InvalidState` otherwise).
   The final proxy->source term is `max(e^->(S), b_A(F))`; source->proxy is
   unchanged. An accepted fill never raises the error (checked). Rows go to
   `lod_filled` (primitive, loop key, size, bound, reopening children).

   *Outline rule.* The outline of an open component (a sheet, sign or planar roof)
   looks like a hole, but its fan would be a back-facing duplicate of the surface. For
   each loop the proxy sums, in f64, the area vectors of the fan triangles and of every
   face of `S` with a directed edge on the loop (scanned in `s_order`), and rejects
   the loop (`fills_rejected_topology`) only if the dot product of the two sums is
   strictly negative. A zero dot (degenerate sums) is accepted. The `CrackAsHole` and
   `FillAcrossSeams` controls are exempt. Known blind spot: the rule is an aggregate,
   so border faces whose area vectors cancel can hide a back-facing fan. Near-tangent
   rims (pipe ends, vertical walls) are decided by a near-zero dot; a lid there is
   outward-consistent and bounded by the directed-distance check `b_A(F)`. The scan
   is O(loops x target faces); index border faces by edge if the target grows.
9. **Frontier audit.** Every border edge of a depth-`d` proxy with both
   endpoints seam vertices, and every such edge of a leaf at depth `<= d`, must
   appear in another frontier tile with bit-identical decoded endpoints, or have
   two `special` endpoints (a true boundary). A decoded mismatch is
   `InvalidState`; an unmatched edge feeds consensus (stage 5).

## Published records and provenance

Each proxy GLB has one scene node and mesh per distinct flat material, holding
one glTF primitive of at most 8,192 non-indexed triangles (more only on a tile
above `P`). CesiumJS counts a model's property-table bytes once per glTF
primitive, so primitives are merged by material rather than kept per source
primitive. Its `EXT_structural_metadata` tables, each at most 8,192 rows
(CesiumJS's per-table limit):

- `source_primitive` (feature-ID set 0, shared): every present primitive, with
  the leaves' columns (node, mesh, primitive, expanded ordinal, material);
- `proxy_source` (feature-ID set 1, one table per proxy within `P` faces): for
  each triangle, `triangle_index` (the source triangle it descends from within
  its set-0 primitive, the collapse survivor; UINT32 unless an index needs 64
  bits) and `fill` (1 for fill faces, whose `triangle_index` is 0). Node, mesh
  and primitive come from the triangle's set-0 row. A later annotate pass can map
  per-source-triangle metadata onto proxies through the pair (set-0 row,
  `triangle_index`) without re-tiling; picking resolves through it as through a
  leaf's `source_triangle`;
- `lod_vanished` and `lod_filled` record tables (split at 8,192 rows).

Readers must not assume a fixed number of feature-ID sets: each glTF primitive
declares its own in `EXT_mesh_features.featureIds`, and H17's decoder reads
whatever is declared.

Accounting: every primitive of a tile's children is present in its
`source_primitive` table or named by a `lod_vanished` row (reason 0 island,
1 lost to a forced collapse), and every vanished row's primitive is present in a
child.

## Bounds, determinism and memory

Tiles of one depth run in parallel, and inside a tile only the direct
source->proxy scan runs in parallel (fixed blocks, nested in the same pool, max
taken in block order); every output is keyed (GLBs by name, rows external-sorted), so
output is independent of jobs and scratch location (tests compare archives
across jobs 1 and 2). Resident state per job is one tile: at most eight
children plus the proxy, their BVHs, and one 65,536-triangle direct chunk;
seam edges, removed-vertex logs and rows are spilled. Scratch adds the pre-pass
(two external sorts) and the slim source, preflighted as stage `lod`.

## Known limits

- Proxies are untextured (H13).
- Mixed proxy depths under REPLACE are not bit-watertight: a seam between a
  depth-`d` proxy and a refined neighbour's depth-`d+1` content can gap by at
  most the two tiles' certified errors. Same-depth frontiers (including
  shallower leaves, packed leaves and synthetic depths) are bit-identical.
- Above leaf-only children no direct term runs (it would equal the composed
  term up to the leaves' snap bound), and the proxy->source direct term runs
  only for small ranges; the composed terms certify the rest.
- Profile constants (`P`, retention, budgets, refinement depth) are provisional.

## Acceptance

`cargo test --lib mesh_large::lod` (16 tests, including the public `mesh_to_tileset` path): exact-rational reference for the
interval witness and the inward-rounding control; refinement against dense
samples; simplifier, seam-removal and fill topology units; terrain (three
primitives, islands, holes) and synthetic-depth (> 21) runs through H8 and H16
checked by an independent auditor (own GLB decoder; sampled distances in both
directions against the fixture's exact source never exceed the published error;
frontier seams on decoded bytes; accounting, reopening and provenance; validator
`decodedContentContainment`; identical archives across jobs; one glTF primitive
per material and one slim provenance table per proxy; no fill corner on a seam
vertex); a seam-consensus stress run (forced removals fail on one side, so
neighbours re-run; audited crack-free and identical across jobs); the controls
"removed-region term skipped", "crack relabelled as hole" and "fills across
seams" (refused by the border-hole check). The real-mesh
consumer proof, performance and memory figures are in the PR and evidence.
