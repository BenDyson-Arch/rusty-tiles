# H16 streamed hierarchy and 3TZ delivery: acceptance record

Contract: [h16-delivery-contract.md](../../../docs/architecture/h16-delivery-contract.md).
Light process: tests, one nonauthor review, perf/memory evidence, a real-mesh
consumer proof, exact-head CI. Bulk evidence (binaries, inputs, archives,
screenshots, logs) stays out of the tree in
`~/.cache/rusty-tiles-evidence/h16-20261013/`; [acceptance.json](acceptance.json)
binds it by SHA-256.

## Tests

`cargo test --lib mesh_large::delivery` (10 tests) plus the M1 and H8 additions
(`payload_at_its_final_member_that_needs_a_rebase_is_refused`, the archive-root
document refusal, `property_tables_stay_within_the_consumer_row_limit`). They
cover determinism, the ceiling refusal, slot and availability correctness from
H1 addresses (independent subtree-bitstream reader), validator acceptance with
`decodedContentContainment`, the 8,192-row cap and planner memory.

## Real mesh (10 M triangles)

Input: `mgal_detail_offset_2048.glb` (Agisoft Metashape, 9,999,999 triangles,
89 meshes, 89 JPEG 2048 atlases), local metres offset from EPSG:3857
E 14,812,000 / N -1,384,000 / A 100, anchored at lon 133.058460, lat -12.336251,
height 100 (identity orientation, no offset). Develop's H8 refuses these atlases
(KTX2 colour gates; `ml/h8-throughput` is not merged) and the merged validator
refuses KTX2 leaves, so the proof input bakes each 2048 atlas into per-vertex
`COLOR_0` (nearest texel at each vertex UV) and adds area-weighted normals
(`tools/bake_vertex_colours.py`). Positions, indices, nodes and triangle order are
unchanged, so source identities are the textured source's.

| run (release, 4 CPUs) | wall | peak RSS | output |
|---|---:|---:|---|
| `mesh-large-to-3tz --profile balanced` (H8 then H16) | 193.6 s | 186 MiB | 1,959 leaves, 2,806 tiles, 200 implicit roots, 7 documents, 2,167 members, 1.40 GB |
| H16 stage (in the run above) | 6.7 s | | |
| `mesh-leaves-to-3tz` alone on kept leaves | 9.5 s | 20.8 MiB | same bytes |

The archive passes an independent read (`tools/check_archive.py`): all 1,959
payloads reachable once, availability closed and equal to the members, no
problems. Its 1.40 GB exceeds the validator's fixed 1 GiB archive limit
(`resource_limit`); a real-mesh crop of 162,869 triangles (28 leaves, 4 implicit
roots, packed leaves) is accepted with `decodedContentContainment`,
`hierarchyBounds`, `implicitAddressingAndAvailability` and `geometricErrorOrder`.
A 462,281-triangle crop exceeds the validator's payload-element budget. The crop
archive is byte-identical across H8 jobs 1, 2, 3 and 4, two scratch directories
and two output locations. The full archive (SHA-256 `5a672d99…`) is
byte-identical across H8 jobs 4 and 2, two scratch directories, two output
locations, and the one-command and two-step routes.

## Synthetic scale (H16 alone)

A 10-level chain over a complete octree, one triangle per leaf
(`tools/synth_leaves.py`):

| leaves | tiles | documents | peak held records | wall | peak RSS | validator |
|---:|---:|---:|---:|---:|---:|---|
| 4,096 | 4,691 | 3 | 4,680 | 0.76 s | 24.3 MiB | accepted, containment |
| 32,768 | 37,459 | 10 | 4,680 | 1.24 s | 27.5 MiB | accepted, containment |
| 262,144 | 299,603 | 67 | 4,687 | 9.8 s | 37.2 MiB | `resource_limit` (2^18 hierarchy visits) |
| 2,097,152 | 2,396,755 | 522 | 4,687 | 360 s | 48.4 MiB | not run (1.75 GB exceeds the 1 GiB archive limit) |

Peak held records stay flat. The residual RSS growth is the index sorter's
in-memory run (capped at 16 MiB) and the serializer's name per explicit leaf and
document. The 2.1 M-member ZIP64 archive reads back with Python's `zipfile`; its
index is sorted, holds `24·(N-1)` bytes, and 20,001 sampled offsets match
(`synth/l7-index-check.json`). Archives of 2^22 tiles or more need the
validator-scale slices before they can be validated.

Nonauthor review (sonnet, fresh): GO-with-changes; its required changes (an
honest acceptance record, a same-length tamper test after staging begins with
`InvalidInput`, proof that the ceiling precedes staging) and most nits (run-leaf
`lastOctant`, address cases, a JSON-chunk cap, the staging path on I/O errors,
multi-document determinism, a ZIP64 and spilled-index archive test, the CLI
help) are addressed.

## Consumer proof (user's Chromium 153, CesiumJS 1.146, RTX 4080)

One tab opened over CDP and left open on `http://127.0.0.1:8793/index.html`,
served by `tools/serve_3tz.py`, which reads each member straight from the `.3tz`.
With the first run's 3 GiB cache plus 1 GiB overflow, Cesium's memory use
reached that budget (3.8 to 4.0 GiB) and 42 dense sub-metre leaves were never
requested, so a hole showed (kept as `proof/run1-budget-4GiB/`; the cause is
inferred from Cesium's memory-adjusted screen-space error, which run 1 did not
record). With 12 + 4 GiB, all 1,959
leaves are selected at the overview (H1 depth 9 drawn). Settling takes 28.9 s
and uses 12.2 GiB of Cesium memory (1.26 GiB geometry), with 2,016 GLB,
200 subtree and 8 JSON requests (1.37 GB). There are no errors and no
property-table warnings. Full detail without coarse levels makes far views load
every leaf until H17.

Four `source_triangle` picks name real source triangles: authored vertex indices
match, and each centroid, placed through the published root transform, is
within 4 to 5 cm of the picked surface point (triangles 0.17 to 0.57 m). The
`source_primitive` pick at each pixel names the same node, mesh and primitive
(`tools/check_picks.py`). Orientation, scale and placement are therefore
correct. The info box shows the exact identity.

## Perf lane (processing, develop `e691a03` vs head)

Every payload and subtree of vector, point-cloud, mesh-local, glb-to-3tz and
legacy mesh output is byte-identical. The vector build-state digest changes
because its encoder identity hashes `implicit/tileset.rs`, which gained a test.
Wall time and RSS are within noise.

## Findings outside this slice

- H8 capture failed once with `--jobs 2` on the full mesh ("ranked corner depth
  order mismatch", `InvalidState`, after 56 s). The same command succeeded on
  the second try, and its leaves give the same archive as jobs 4: an
  intermittent H8 capture defect (`full.rs` corner-depth join), not reproduced
  on the crop with jobs 1 to 4.
- The C1 payload profile does not admit a required `KHR_texture_basisu`, so KTX2
  leaves cannot be validated yet.
