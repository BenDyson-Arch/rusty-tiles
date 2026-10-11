# Implicit producer inventory (I1 preparation)

Source is `f2ed077`. Outputs were probed with
`probes/inventory_probe.py`, a stdlib decoder that imports nothing from the repository. All
identities are in `probes/receipt.json`, and the raw records in `probes/results/`.

The binary is `rusty-tiles-f2238cd` (`369c3c81…`). C2 changed no producer, and the probed implicit
shapes are writer-determined. R must regenerate every witness on the final I1 source.

## Shared writer: `implicit::write_tileset_recorded` (used by P, V and M)

**Constants and hierarchy.**
- `LEVELS = 4` (`implicit/tileset.rs:9`). Every subtree has `subtreeLevels: 4`.
- The explicit hierarchy is collected with child slot `extras.implicitChildIndex`, or the array
  index if absent (`:113-124`).
  - Point sets the slot from the octant branch (`point_cloud/tiles.rs:238-240`).
  - Legacy mesh sets it from `n.slot` (`tile.rs:855`).
  - Vector uses the array index.

**Translations.**
- Child `transform`s must be pure translations (`:126-136`). In explicit output, every non-root
  tile carries one: 1442 of 1443 tiles in point, 546 of 547 in vector.
- Translations accumulate into a delta that is added to the row box centre (`:92-95`).
- The delta is also baked into the GLB scene-root `translation` or `matrix` as `(d0, d2, −d1)`
  (`:149-224`). Implicit payload bytes therefore differ from the explicit ones.
- **Only the implicit root keeps a `transform`.** Probe: `rootTransform` is present at depth 0
  only, in every producer document.

**Effective rows.**
- Every available tile gets a `rustyTile` row: TILE_BOUNDING_BOX, TILE_GEOMETRIC_ERROR, and an
  `extras` STRING holding JSON (`implicit.rs:260-292`, schema `metadata.rs:174-180`, `id`
  `rustyTilesImplicit`).
- Rows are the producer's actual padded boxes and authored errors, **not** derived cells. Probe
  results:

  | Output | Rows equal to the derived cell | Strictly inside | Outside |
  | --- | --- | --- | --- |
  | `pc-deep` | 208 (roots only) | 1950 | 282 |
  | `vec-perf` | 85 (roots) | 0 | 546 |
  | legacy mesh | 1 | — | 72 |
  | `pc-perf-2000` | — | 427 | 0 |

  `noRow = 0` everywhere. TILE_GEOMETRIC_ERROR differs from root/2^l for every non-root row
  (`errorDiffersHalving`).

**Two layouts, chosen by the data** (`:284-308`). The writer chunks if any tile at a level that is
a multiple of 4 (and greater than 0) has a row escaping its derived cell, using `Obb::contains`
with `1e-6`.

1. **Single tree with child subtrees** (`:309-390`).
   - Templates are `implicit-content/{level}-{x}-{y}[-{z}]-{slot}.{glb|gltf|b3dm}` and
     `subtrees/{level}-{x}-{y}[-{z}].subtree`.
   - `availableLevels` equals the maximum level plus 1, so it is tight.
   - Child subtree bits are set for every level-4k tile (`:364-373`).
   - Executed on `pc-perf-2000.3tz`: 1 document, 234 subtrees, 233 child bits, 428 tiles. Each
     deeper subtree holds one tile.
2. **Chunked external roots** (`write_chunks`, `:473-602`).
   - Groups are formed with stride `LEVELS−1 = 3`.
   - A tile at a level multiple of 3 that has children becomes a **link**. In its parent's
     subtree it is available with only the extra JSON slot
     `implicit-tileset-{prefix}-{level}-{x}-{y}[-{z}].json` (`:449-451`, `:528-529`). The
     payload slots are unavailable.
   - The link tile is also the root of its own document `implicit-tileset-{parentKey}-{localKey}.json`
     (`:581-590`). That root has `boundingVolume` set to the row box, the same error, and **no
     transform** (`:452-455`).
   - Child-subtree availability is always constant 0. The JSON slot is never available at
     level 0. A JSON tile never has available implicit children.
   - Executed observations:

     | Output | Documents | JSON links | External nesting depth | Tiles |
     | --- | --- | --- | --- | --- |
     | `pc-deep.3tz` | 208 | 207 | 1 | 2440 |
     | `vec-perf.3tz` | 85 | 84 | 3 | 631 |

**Subtree encoding.**
- 24-byte header; JSON padded with spaces; one internal buffer with `byteLength` only.
- Views are 8-aligned and always carry `byteOffset` (`implicit.rs:312-331`, `:298-308`).
- Availability is a `constant` 0 or 1 when uniform, otherwise a `bitstream` with
  `availableCount` (`:321-331`).
- Constant tile availability occurs in `pc-deep`. Child availability is a constant 0, except a
  bitstream in `pc-perf-2000`.
- Trailing bits are zero, counts match, and there are no structural violations in any probed
  output.
- **MSB-first decoding differs from LSB-first in every output that has a bitstream**, so the
  bit-order control is sensitive on producer data.

**Other fields.**
- **Tile `extras`.** Vector rows carry `encodedBytes`, `vertices` and others (628 of 631 rows),
  plus `routing` and `routingReason` on 3 routing tiles. Point rows carry `{}`. Today's validator
  compares `encodedBytes` and `vertices` with actual payloads (`validate.rs:1408-1415`); see OD-4.
- **Refine.** `REPLACE` everywhere except the legacy small mesh: `implicit_glb_job`
  (`tileset.rs:153-177`) writes `refine: "ADD"` with `.gltf` content.
- **Asset version.** `"1.1"`.
- **No producer writes:** regions, spheres, child transforms in implicit trees, JSON subtrees,
  external buffers, `contentMetadata`, `subtreeMetadata`, offset, scale, noData or default, or
  FLOAT32 rows.

## Per producer

| ID | Producer (entry) | Scheme | Default? | Layouts reached | Root transform | Content header | Extras or retained sources | Admitted by I1? |
| --- | --- | --- | --- | --- | --- | --- | --- | --- |
| P1 | `point-cloud` (`point_cloud.rs:600-607`, `write_tileset_recorded`, retain=false) | OCTREE | **yes** (`--explicit` opts out, `main.rs:299-301`) | single tree (W1 73 tiles; W2 child subtrees), chunked (W3) | yes (origin, `:584-590`) | none | rows `{}` | **Yes**; chunked form only under OD-1 |
| V1 | `vector` (`store.rs:970-983`, `write_tileset`, retain=**true**) | QUADTREE | **yes** (`main.rs:335-337`) | single tree (W4 1 tile), chunked (W5, depth 3) | yes (frame, `store.rs:940-957`) | `3DTILES_content_gltf_vector: {vector: true}` on the payload template; none on the JSON template | rows with `encodedBytes` etc.; `extras.rustyTilesSourceContents` lists raw payloads (`implicit/tileset.rs:392-394`), which the validator inspects as payloads (`validate.rs:1206-1217`) | **Yes**; chunked under OD-1; per-tile extras under OD-4 |
| V2 | vector `--quantize` and `--meshopt` | QUADTREE | flags | same as V1 | yes | same | same | Yes (payload decode is C1's) |
| M1 | legacy `mesh-to-3tz` (`tile.rs:515-523`) | OCTREE | **yes** (`main.rs:468-470`) | single tree (W6 73 tiles); chunked in `native_implicit.rs:23` (`--maxTriangles 2`) | only when georeferenced (`tile.rs:512-514`) | none | — | Yes (legacy route; G1) |
| M2 | legacy small-model wrapper `implicit_glb_job` (`tileset.rs:153-177`, via `tile.rs:233`) | OCTREE | yes, for small models | 1 tile, refine **ADD**, `.gltf` plus resources | no | none | — | Yes |
| C1 | `convert-to-implicit` (`convert_implicit.rs:347-516`, its own `Subtree` use) | OCTREE (point) or QUADTREE (vector) | operation | Every source node becomes its own implicit root: `subtreeLevels` = `availableLevels` = 1 (leaf) or 2 (internal). Level 0 holds payload aliases `t/owned-{id}-{level}-{x}-{y}[-{z}]-{slot}.{glb,b3dm}`. The JSON slot `implicit-owned-{id}-{template}.json` is available only at level-1 cells (`:404-421`, `:444-446`). Child slot comes from `_rustyImplicitChildIndex` (`:432-443`) | yes; **external roots keep the original child translation as `transform`** (`:485-503`). Probed: all 3 documents carry a transform | source content headers kept | original extras kept on the root; rows' extras are `{}` for links | **Yes**, under OD-1. Validates its own candidate (`:205`), so I1 is on its publication path |
| C2 | convert on realistic sources | — | — | **Refused**: `vec-perf-explicit` and `pc-deep-explicit` both exit 3, "irregular explicit hierarchy" (executed). Only tiny or regular sources convert (corpus: 1 tile; A2 retained: 5 tiles, 3 documents) | — | — | — | n/a |

## Not producers, but witnesses

The #151 `a2_semantics` fixtures (`quadtree.json` `23d26bc0…`, `octree.json` `ec78244b…`) put
semantics on class `a2Tile` with properties `bounds` and `error`. They have external roots with
transforms and terminal JSON slots.

As committed they fail the bundled schema, because `schema.id` is missing. After that correction
(`probes/materialize_a2_fixture.py`), today's validator returns **InvalidInput "implicit subtree:
unsupported tile metadata schema"**. Renaming the class to `rustyTile` gives the same result,
because the property names also differ. This was executed on f2238cd; the outputs are
`samples/a2fix/*.3tz`.

Under I1 these fixtures are Accepted. Probe: 9 tiles, 5 documents, 4 JSON links, 5 rows equal to
derived cells and 4 strictly inside.

## Consequences for I1

1. Every producer row-overrides every tile. `Derived` cells are needed only for third-party data,
   so C2 slice 3 can run on all producer outputs with `LevelFacts` unchanged.
2. Both the child-subtree path and the external-JSON path are default behaviour, so both are in
   the minimal slice.
3. The JSON template on implicit roots is default behaviour for large point and vector outputs,
   and for every `convert-to-implicit` output with more than one node. OD-1 is therefore a
   release-relevant decision, not a third-party edge case.
4. No producer exercises:
   - FLOAT32 rows;
   - external buffers;
   - `contentMetadata`;
   - non-`rustyTile` classes;
   - ADD with depth greater than 1;
   - roots below explicit tiles.
   Coverage for these comes from O's synthetic controls only.
