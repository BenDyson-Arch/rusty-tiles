# H1: streamed local mesh capture and cell tree

Status: implementation contract, 2026-10-12; acceptance pending. This is the
bounded capture/sort/tree slice of roadmap milestone 3. It produces private
scratch-backed records, not tiles, LOD, atlases or certificates. The diagnostic
driver exercises the production pipeline without advertising a premature tiler
command or public API. The handover's H1 scope governs; old ML2 text supplies
algorithm inputs only.

## Source and ownership

Admit one static glTF 2.0/GLB scene in local metres through the retained finite
source profile, with explicit JSON/count/per-item limits and no total payload
byte or triangle limit. Coordinate and transform eligibility remain the F1a
profile, including its local coordinate range. Bake selected-scene positions
once using its node matrices and reflected corner order; canonicalize signed
zero. Preserve authored primitive and vertex references for later attributes.
Validate every declared accessor and authored primitive, including unselected
meshes. Attribute payloads are validated, but the H1 spill stores positions and
source/vertex references, not NORMAL/TANGENT/UV/COLOR bytes. Full attribute
capture is a downstream obligation: extend the capture pass before ending
source stability when the tiler begins consuming those attributes. References
alone cannot recover attributes after source removal. Image references are
admitted; image payload capture and encoding are outside H1.
Stable input files are a precondition; change detection does not
promise an atomic multi-file snapshot or a hostile-filesystem sandbox.

`gltf_source` owns pure document/layout/transform rules and local input binding.
The old archive converter and H1 consume it independently. `mesh_large::input`
owns bounded reads and source change checks; `capture` owns triangle assembly;
`spill` owns records, scratch, sorting and I/O failures; `tree` owns spatial and
synthetic hierarchy mathematics. The coordinator owns resolved configuration,
resource accounting and pass orchestration.

Retain finite source semantics, Morton keys and provenance. Rework the WIP's
parser ownership and path/handle change checks. Replace the missing tree and
unbounded sorter run inventory. Correct live scratch accounting. Do not retain
code solely because the interrupted WIP or a historical suite contained it.

## Deterministic records and geometry budgeting

Every selected-scene source triangle appears in exactly one leaf. Retain and
flag degenerate triangles; later adjacency may skip them. This explicitly
supersedes old ML2 prose about dropping degenerates. Node/mesh/primitive source
indices follow their explicit document caps; triangle ordinals, expanded scene
instance ordinals and authored nonindexed vertex ordinals use u64.

The ordering is (63-bit centroid Morton key, exact source tuple). The isotropic
root cube covers finite baked f32 positions; key depth is 21. Record encodings
are fixed-width little endian. Binary triangle/node/weld-depth dumps are
independent of worker count, run size, merge fan-in and scratch path.

H1 projected geometry bytes per triangle are 20 provenance bytes plus three
times (12 POSITION bytes, optionally 12 NORMAL, 16 TANGENT, 8 per UV set and 16
COLOR). This is a geometry planning charge, not an encoded member-size bound.
Texture projection and final member-budget acceptance belong to subsequent
slices. H1 does not claim its geometry-only tree is the final textured tiler tree.

## Hierarchy and future seam support

Consume sorted records with an O(key depth + synthetic height) stack. A cell
within both triangle and geometry-byte budgets is a candidate. Over-budget
cells become internal. Under one internal cell, greedily pack consecutive leaf
octants while both budgets hold; empty octants do not break a run, internal
octants do. Packed leaves occupy the first octant's slot and a contiguous rank
range, with bounds unioned over actual triangles.
Morton assignment uses centroids without clipping: triangle bounds need not fit
the implied spatial cell cube. Containment applies to actual ancestor union
bounds, not the centroid-addressing cell. This corrects the old design prose.

An over-budget key-depth-21 cell splits into consecutive budgeted runs. Group
these eight at a time into a left-filled synthetic hierarchy. Identity is
(cell depth, prefix, synthetic path length, synthetic path); every sibling has
a distinct slot and every parent has at most eight children. Synthetic levels
count toward the height cap. Spill postorder output and sort into preorder
(first triangle rank, depth); avoid resident whole-node inventories.

After leaves are known, sort every triangle corner by its canonical f32
position-bit key and reduce the minimum leaf depth over ALL incidents. Store
the exact global welded minimum; no 16-entry incidence truncation. These
records and retained vertex/source references prepare for later seam snapping;
H1 does not snap or quantize positions.

Diagnostic wire schema 1 uses these little-endian records:

| File | Bytes per record | Fields in wire order |
| --- | ---: | --- |
| triangles.bin | 93 | key u64; node, mesh, primitive u32 each; triangle u64; three vertex ordinals u64; nine position f32; projected bytes u32; degenerate u8 |
| nodes.bin | 72 | cell_depth u8; cell u64; path_len u8; path u64; last_octant u8; kind u8 (0 internal, 1 leaf); children u8; first, end, bytes u64 each; min/max three f32 each; three zero padding bytes |
| vertex_depth.bin | 13 | three canonical position-bit u32; minimum leaf depth u8 |

`info.json` contains schema 1, invariant tree totals, per-depth actual union
bounds and maximum individual-node extent, source bounds, cube and geometry
charge. `report.json` additionally contains requested/resolved resources and
elapsed time. Resource decisions and elapsed time do not belong in deterministic
dump comparisons. The ignored `mesh_large::tests::external_driver` accepts
H1_INPUT/H1_OUTPUT/H1_SCRATCH and explicitly named configuration environment
variables; its limit overrides are test seams, not production options.

## Resources and failure semantics

Resolve and validate configuration once. Auto workers are at least one and
limited by available cores and memory after base and memory-backed scratch
allowances; report the decision. Explicit workers remain explicit. Sorter run
inventories use bounded leveled compaction, preserving chronological stable
merge order. Include descriptor/path storage, typed plan, process runtime,
reader/writer buffers, stable-sort scratch, thread stacks and tree stack in
the declared resident bound. Resource arithmetic must not wrap. Measure total
RSS on a spilled case; an algebraic payload bound alone is insufficient.

Preflight each H1 stage from admitted counts before scratch creation or the
stage's writes. Report memory-backed scratch separately; it is not charged as
resident payload. Account incomplete sequence/merge files in live and peak
scratch. Preflight is not a reservation: subsequent ENOSPC remains typed I/O.
Owned scratch is removed on refusal, cancellation and I/O failure; explicit
successful close reports cleanup failure.

For the node scratch bound, let X=floor(N/(L+1))+floor(B/(B_tile+1)). At each of
22 spatial depths at most X disjoint cells exceed a budget, giving 22X spatial
internal cells. Pairing adjacent maximal coincident-cell runs bounds their
total by 3X. At accepted height <=32, synthetic internals excluding their
already-counted spatial roots add at most (3/7+9)X. Thus fewer than 32X
internals imply at most 256X+1 nodes, since fan-out is at most eight. Use
256X+16 conservatively and check all byte multiplications. Scratch preflight
includes both generations of every corner/position/triangle/weld sort and
four node/span inventories, avoiding assumptions about iterator drop timing.
These are logical record bytes. Physical free-space preflight adds filesystem
allocation rounding using the actual fragment/cluster unit, a 64 KiB metadata
reserve per live file, 1 MiB for directory metadata and an additional logical
generation allowance for extent overhead. Live files are bounded by 256F+32
and by a conservative count-derived initial-run bound; use the smaller bound.
Report logical peak separately from this full allowance. These metadata
reserves are explicit conservative policy, requiring measured filesystem
acceptance, not a universal theorem about filesystem metadata, compression,
quotas or concurrent writes. A stopped-process Linux allocation witness must
cover many tiny files, including the negative control that exposed the old
logical-only preflight. Other platforms require their actual unit query and
applicable platform checks; unmeasured filesystem allocation behavior remains
a proof limit.

Use a sound tile lower bound before capture and the actual geometry tree count
after construction to refuse at the retained 178,956,970-member 3TZ ceiling,
including minimum required non-tile members. This proves a necessary early
refusal, not the future delivery writer's complete document/subtree count.
Refuse a single over-budget item and excessive hierarchy height with typed
Unsupported. Invalid source, invalid configuration, arithmetic exhaustion,
cancellation and I/O retain distinct error kinds. Check cancellation between
passes and within bounded record batches; one configured sort run may complete
before the next checkpoint.

## Acceptance and unfinished obligations

Independent source truth and a separately implemented recursive reference must
check exact-once provenance, baked positions, Morton order, packed and synthetic
hierarchies, identities, bounds and global welded minimum depths. Include
wrong-provenance, dropped/duplicated triangle, wrong-frame, missing-resource,
wrong hierarchy and capped-weld sensitive controls. Exercise degenerate cases,
resource/height/member refusals, cancellation and injected I/O/cleanup behavior.

Require deterministic binary dumps across workers, run sizes, fan-ins and
scratch locations; a spilled >=10^7-triangle run with wall time, total peak RSS
and scratch evidence outside CI; final-source independent nonauthor review;
and applicable exact-head CI/package/platform checks. Existing output paths
affected by parser extraction require their own retained tests and performance
comparison. Bulk evidence lives under ~/.cache/rusty-tiles-evidence/ with hashes.

Tile publication, geographic placement, adjacency, rendered picking, texture
capture, encoding, LOD, certificates and the larger final 0.4 scale gates remain
unfinished. Merge authority extends only to develop; #113/#121 remain open.
