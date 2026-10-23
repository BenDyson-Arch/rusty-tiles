# Locked cluster tree experiment

Research-only prototype for the 2026-10-24 LOD bake-off. Evidence lives at
`~/.cache/rusty-tiles-evidence/lod-bakeoff-20261024/cluster-dag/README.md`.

Preimplementation contract: retain exact source leaf triangles, attribute values,
winding and per-triangle identity. Partition meshlets into nested spatial groups;
replace DAG regrouping with a binary ownership tree. Retain meshoptimizer's
index-only simplifier, lock group outer borders, simplify materials separately,
and re-meshletize each result inside its owning group. Never update positions.
Store shared positions as original float32, avoiding independent tile grids.
Children replace their whole parent's domain; bounding volumes contain full
triangles and descendant bounds, not just centroid partition cells.

Supported domain: indexed triangle GLBs with identity mesh nodes, float32
positions and ordinary vertex attributes, embedded JPEG/PNG or vertex colours.
Reject unsupported transforms, instancing and sparse accessors. The prototype
owns an output directory, not production tiler state. Leaf picking uses mesh
feature IDs and structural metadata; coarse identity remains source primitive.

Independent acceptance: decode output GLBs afresh; compare exact leaf provenance
and all topological interfaces between disjoint tree nodes, including every
mixed-depth pairing; sample >=200k surface points in EACH direction per level
with exact nearest-triangle distances; sample rendered base colours using
interpolated UVs or vertex colours. Compare current tex3 output and run the
provided first-view simulator. Sampled errors are evidence, not a certified
Hausdorff bound. A static 3D Tiles mapping argument is not a browser test.

Tradeoff/proof obligations: permanent fine borders may throttle reduction;
material cuts add locks; spatial binary meshlet ownership may have overlapping
AABBs but must not duplicate faces. Generic DAG transitions are deliberately
removed. Shared external source/downscaled images trade bake time for transfer
bytes; report dependencies separately because the provided simulator counts
GLBs, not external images. KTX2 conversion and production integration are outside
this prototype. Source holes/non-manifold edges are not repaired or hidden.

Final refinements: explicit locks derived from original welded vertex/leaf
ownership protect non-manifold junctions as well as manifold borders. Lossless
EXT_meshopt_compression covers geometry and metadata; leaves remain bit-exact.
Coarse UVs are transformed affinely into shared, padded source-image crops,
resized to at most 1024 pixels per dimension (JPEG quality 85). Border aprons
honor source repeat/clamp/mirror addressing. Source images and UVs on leaves
remain unchanged. Permissive simplification was probed and rejected after its
small triangle saving increased colour error. The top-level tileset error is the
asset's bounding diagonal (omission error); tile errors are the accumulated
meshoptimizer estimates. Placement copies the baseline's root transform.

Run `setsid -f bash bench/lod_cluster_dag/reproduce.sh > RUN.log 2>&1` from this
worktree. The script enforces idle scheduling, one numerical worker, dedicated
scratch and Cargo target directories, and the 60-GiB free-space floor. It replaces
only its own generated tex3/crop3 directories. Dependencies are pinned in that
script. The cached meshoptimizer native source comes from Rust meshopt 0.6.2;
its local C++ API includes partitionClusters, though this spatial ownership-tree
variant deliberately does not repartition across tree ancestry. Native and WASM
decoders are compared byte-for-byte in `decode_agreement.py`.

`evaluate.py` saves individual records for every disjoint tree-node pair sharing
source edges, including non-manifold incident face pairs. Candidate acceptance
requires the complete original interface segments to survive in both decoded
meshes. Its nonzero baseline fallback projects to nearest open boundaries and
is a conservative diagnostic where unrelated hole boundaries are nearby, not
an exact reconstruction of a rendered gap. The original proxy seam census
supplies the separate baseline proxy-to-leaf witness. Colour measurements use
bilinear level-0 base colour in linear light and CIE76 D65, without lighting or
mip selection. Neither the standalone GLB validator nor the simulator proves a
live Cesium rendering; the house rules prohibit a browser run.

Evidence binding (files were never stored in Git; starting checkout commit `8971c9c3`):

- `README.md`: SHA-256 `49eaabeca94cb021b19d58c82ca7e855a0607c89e2b48c20e1b76e4e8fe4ca48`
- `artifact-manifest.sha256`: SHA-256 `74315d1c3386164810f78702cadf10a6c8bb127ae0ff13484ab3e768a0a7bf80`
- `execution-source.sha256`: SHA-256 `3de128f896edad08ab556b01f8ffe9db059cc6c8e08d17a4e0da043cdbfaf927`
- `review-r2.json`: SHA-256 `24a243fcd6e931730999fb966ca3d01e7e15cc2699c4085f10365f3a0d8ecbb7`
