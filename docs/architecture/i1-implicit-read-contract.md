# I1 implicit read contract

Status: implemented on `feat/implicit-forest`, the combined I1 + P integration branch (I1 owner
`src/content_integrity/implicit*`, validator migration in `src/validate.rs`). Independent
acceptance (oracle parity, resource controls, perf lane and Cesium) is a separate step and is not
claimed here. This document is the frozen contract text v3.1 (`contract-draft.md`, sha256
`9674e60b…`), plus the coordinator's v3.2 rulings and the integration amendments listed at the end.
The preparation package (contract versions v1 to v3.1, change log, open decisions, ownership plan,
producer inventory and probes) and the precode review (`review.md` `92eee8f0…`) and delta check
(`delta-check.md` `abd37595…`) are not yet retained in the repository.

**Owners.** [#125](https://github.com/BenDyson-Arch/rusty-tiles/issues/125), with #126 and C2
slice 3 as consumers. The producer layout it was paired with is the
[P layout contract](p-impl-layout-contract.md).

**Line numbers.**
- Repository files: `origin/develop` `f2ed077`, the base the contract was written against.
- Primary text: raw line numbers of the pinned files: `ImplicitTiling/README.adoc` `14a37a34…`,
  `Semantics/README.adoc` `02ac07db…`, core `README.adoc` `b841fade…`, and Cesium
  `Implicit3DTileContent.js` `86109c4b…`, pinned by `a2_format_settlement/primary-sources.json`.

**Claims nothing about:**
- decoded content containment (C2 slice 3);
- regions or S2;
- viewer selection;
- metadata semantics beyond the two interpreted ones;
- the standards status of JSON content templates;
- memory or RSS;
- release readiness.

The contract text below calls the implementer of the owner "A", the validator migration author
"B", the oracle author "O" and the integration owner "R".

## Settled decisions this contract relies on

- **OD-1 (user).** JSON content templates are excluded by profile, and the producers change.
- **RC-3 (coordinator, accepted).** Exclusion is **uniform Unsupported**. There is no level-0
  InvalidInput case and no extra root-subtree read.
  - `:90` makes the root's `content.uri` *the template*: "For content, the template URI is
    specified in the tile's `content.uri` property".
  - So `:96`, "The `content.uri` shall not point to an external tileset", reads most literally as a
    rule about the template at every availability. The text has no level-0-only rule.
  - The user chose not to give that verdict. Unsupported is never a false verdict.
- **RC-17 (coordinator).** I1's validator migration and P land together in one combined integration
  PR. They may be developed on separate branches.
- **RC-19 (coordinator).** `convert-to-implicit` is retired: branch `retire/convert-to-implicit`
  (`b59c7b3`, `4b35dd7`), being folded into PR #159. It merges before the combined PR.
- **OD-9 (coordinator).** Each content and subtree template is **normalized once at root admission** (§3), and
  `.json` is detected on the last segment of the *normalized* template. This is exact: variables only ever become
  decimal digits, so substitution cannot create or remove `.`, `..`, `/`, `%` or an escape.
- **OD-10 (coordinator).** The availability scan charge goes to `total_bytes_read`, with its own message (§2a step 2).
  - DC-1 is resolved by a **length cap** (§3), not by a byte charge, so nothing else is charged there.
  - R updates `docs/VALIDATION.md:121`.
  - `types.rs` gets no new fields.

## Corrections to the selection record (still valid)

1. **The default producers chunk.** Above a data-dependent threshold
   (`implicit/tileset.rs:284-308`), they write external-JSON content templates.
   - `pc-deep` (point default, `--maxPoints 40`): 208 documents, 207 links.
   - `vec-perf` (vector default on the perf input): 85 documents, depth 3.
   P removes this.
2. **Child-subtree layout.** It is produced today (`pc-perf-2000`: 234 subtrees, 233 child bits)
   and is in the profile.
3. **C2's dyadic arithmetic is not reusable without touching `containment*`.** It is `pub(super)`,
   and `LevelFacts` is `f64` only (`containment.rs:42-53`). `Derived` cells are therefore symbolic.
4. **Primary pins were incomplete.** They were fetched at `4d781014` (`primary-fetch/`).
5. **The #151 fixtures lack `schema.id`.** They are reusable only with that correction.

## Claim

When `checks` contains `implicitAddressingAndAvailability`, the archive contains at least one tile
with `implicitTiling`, and every *implicit root* reached from `tileset.json` was read. This includes
roots on any explicit tile and roots inside explicit external tilesets. For each implicit root:

1. **Profile.** It has no JSON content template, and no metadata class it uses carries a semantic
   in the Unsupported set (RC-1).
2. **Subtrees.** Every subtree reachable through available child-subtree bits was resolved from its
   template, read from a confined member, and admitted under the subtree-container invariants.
3. **Availability.** Tile, content and child-subtree availability satisfy the availability
   invariants: LSB-first, Morton level order, evaluated per distinct source (§2a).
4. **Visiting.** Every available tile was visited once, in the defined order, at its exact global
   coordinates.
   - Effective volume and error follow the effective-volume rules.
   - `refine` is the root's effective `refine`.
   - Each available content resolves from its **normalized** template (§3; no CONTENT_URI) to a confined, existing
     payload member.
5. **Explicit-tile checks.** Each visited tile passed the checks today's validator applies to
   explicit tiles:
   - C1 payload integrity;
   - `hierarchyBounds` and `geometricErrorOrder`, with today's tolerances (§4, RC-9);
   - `resourceReferences`;
   - report-level recorded budgets (OD-4).
6. **Rows.** Every interpreted metadata row was decoded at the tile's availability rank and
   type-checked.
7. **Content rule.** If the root has content templates, at least one tile in the implicit tree has
   available content (`:299`).

**Not covered:**
- decoded containment (`decodedContentBounds.implicitTiling` remains);
- non-interpreted metadata, content and subtree metadata (`metadataSemantics` remains);
- viewer culling and selection.

**Normative basis (raw lines).** `ImplicitTiling/README.adoc`:

| Topic | Lines |
| --- | --- |
| Any tile may be an implicit root | `:34` |
| `availableLevels` | `:81` |
| Template is `content.uri` | `:90` |
| Root constraints | `:92-97` |
| Quadtree | `:104` |
| Spheres disallowed | `:116` |
| Subdivision rules | `:144-175` (`:164` error halving; `:170-172` direct computation) |
| Coordinates | `:178-184` (`:182` level 0 is the implicit root) |
| Templates | `:226-233` (`:231` required variables, `:233` resolution) |
| Availability | `:253-316` (`:291` at least one tile; `:299-304` content) |
| Tile metadata | `:328-352` (`:333`, `:338` rank) |
| Content metadata | `:357` |
| Buffers and views | `:381-392` (`:386` no data URIs, `:390`, `:392` 8-byte alignment) |
| Availability encoding | `:397-405` (`:405` trailing zeros) |
| Binary format | `:730-771` |

Other sources:
- `AVAILABILITY.adoc`: Morton (x is the LSB); offsets.
- `Metadata/README.adoc`: Booleans `:619-639`; offset and scale `:430-451`; noData and default
  `:472-485`.
- `Semantics/README.adoc`: TILE_* table; `CONTENT_URI` `:290-293` ("Overrides the implicit tile's
  generated content uri"); `CONTENT_GROUP_ID` `:295-299`.
- `class.property.schema.json`: semantic uniqueness.
- Subtree, `availability`, `buffer` and `bufferView` schemas: `minItems`, required `byteOffset`.
- Core `README.adoc:731-748`: external tilesets, which matter only around implicit roots.

## Supported domain

### Admitted

- **Tiling.** `QUADTREE` or `OCTREE`; `subtreeLevels` and `availableLevels` each in `[1, 32]`.
- **Root volume.** A `box` with any half-axes.
- **Placement.** An implicit root on any explicit tile, including inside explicit external
  tilesets. `transform` and `refine` follow core rules, and `refine` is inherited when absent.
- **Content templates.** Payload templates only (`content` or `contents`). Other template
  properties, such as `extensions` and `group`, form a content header copied to each derived
  content. This matches pinned Cesium `Implicit3DTileContent.js:453`
  (`combine(contentJson, implicitTileset.contentHeaders[i])`). A header `metadata` entity is copied too. Its class is
  classified at root admission and is subject to the Unsupported semantic set (DC-5).
- **Subtrees.** Binary subtrees: one internal buffer plus external buffers (OD-6); bitstream or
  constant availability; any number of child subtrees.
- **Tile metadata.** `tileMetadata` of any class. TILE_BOUNDING_BOX and TILE_GEOMETRIC_ERROR are
  interpreted **by `semantic`**, FLOAT32 or FLOAT64, with no offset, scale, noData or default.
- **Opaque.** Non-TILE_/CONTENT_ semantics, TILE_HORIZON_OCCLUSION_POINT and
  CONTENT_HORIZON_OCCLUSION_POINT, `contentMetadata` (structurally counted) and `subtreeMetadata`.

### Unsupported (exit 2)

- **JSON content templates.** Any template whose **normalized** form (§3) has a last segment ending in `.json`
  (ASCII case-insensitive), in any slot, at any availability. Decided at root admission, before any subtree read.
- **Template profile (DC-1).** Each of these is decided at root admission:
  - a `%` escape within two characters before a `{` in any segment: "escape adjacent to template variable";
  - a template whose required variables are **eliminated by normalization**, for example `{x}/..`: "template variables
    eliminated by dot-segments";
  - a normalized template longer than **1,024 bytes**: "normalized template exceeds 1 KiB".
- **Unsupported semantics (RC-1, DC-5).** These semantics are Unsupported in any class used by:
  - a `tileMetadata` or `contentMetadata` table of the walk; or
  - the `metadata` entity of a content **template header** (checked at root admission).

  The semantics are:
  - TILE_BOUNDING_SPHERE, TILE_BOUNDING_REGION, TILE_BOUNDING_S2_CELL;
  - TILE_MINIMUM_HEIGHT, TILE_MAXIMUM_HEIGHT;
  - TILE_TRANSFORM, TILE_REFINE;
  - **CONTENT_URI**, **CONTENT_GROUP_ID**;
  - CONTENT_BOUNDING_BOX, CONTENT_BOUNDING_REGION, CONTENT_BOUNDING_SPHERE,
    CONTENT_BOUNDING_S2_CELL;
  - CONTENT_MINIMUM_HEIGHT, CONTENT_MAXIMUM_HEIGHT;
  - TILE_BOUNDING_BOX and TILE_GEOMETRIC_ERROR when they appear in a *content* class.

  Why:
  - CONTENT_URI overrides the generated content URI. Ignoring it would check the wrong member and
    could reach JSON content.
  - CONTENT_GROUP_ID overrides group membership. I1 checks no group references, so it refuses the
    override rather than silently ignoring it.
- **Root volumes.** `region` and `3DTILES_bounding_volume_S2` roots.
- **Subdivision scheme.** Any `subdivisionScheme` other than `QUADTREE` or `OCTREE`. The schema
  admits other strings through `anyOf string`.
- **Subtree format.** JSON-format subtrees (the member does not start with `subt`).
- **Extensions.** `3DTILES_implicit_tiling` (1.0); `extensions` anywhere in subtree JSON.
- **Modifiers.** offset, scale, noData or default on an interpreted property.
- **Levels.** `availableLevels > 32` or `subtreeLevels > 32`.
- **Repeated subtrees.** A subtree member resolved twice in one validation (OD-7).
- **Derived error underflow.** A derived error that would be a nonzero subnormal (§4).

### InvalidInput (exit 3)

All invariant rules below. In addition:
- **Sphere roots** (`:116`).
- **Implicit root shape.** An implicit root with `children`, with `metadata`, or with
  `content.boundingVolume` on a template (`:92-97`).
- **Content rule.** The post-walk content rule (`:299`).

## Invariants

### 1. Subtree container

Each subtree member is read through the validator's resolver: confined `uri()`
(`validate.rs:165-210`), relative to the tileset JSON holding R; `ArchiveResources::reference`; and
`read_member` with maximum `min(member_bytes, live-path remaining)`. `read_member` refuses
`size > maximum` before reading (`validate.rs:126-145`).

Admission steps, in order:

1. **Header.** At least 24 bytes; magic `subt`; version 1.
   - `jsonByteLength > 0`, and both lengths are multiples of 8.
   - `24 + json + bin == member length` (u64).
2. **JSON chunk.** C1 `json::parse` with `JsonLimits` (`validate.rs:26-41`). The top level must be
   an object.
   - Bytes after the JSON value are admitted exactly when C1 `json::parse` admits them (JSON
     whitespace).
   - There is **no 0x20-only rule (RC-5).** `:770` asks writers to pad with spaces, but a chunk
     whose JSON text ends in `\n` with no padding is indistinguishable and valid.
3. **Arrays.** `buffers`, `bufferViews`, `propertyTables`, `contentAvailability` and
   `contentMetadata`, when present, have at least 1 item (schema `minItems`; RC-15). An empty
   array is InvalidInput.
4. **Buffers.**
   - `byteLength ≥ 1`.
   - A buffer without `uri` must be index 0, and the BIN chunk must exist. Then `byteLength ≤ bin`,
     `bin − byteLength < 8`, and the padding is zero.
   - A buffer with `uri` must not be a data URI (`:386`). It resolves relative to the subtree
     member, with member length `≥ byteLength` (as `payload.rs:627-633`).
   - A non-empty BIN with no internal buffer is InvalidInput.
5. **Buffer views.**
   - The `buffer` index must be valid.
   - `byteOffset` is **required** and a multiple of 8 (`:392`).
   - `byteLength ≥ 1`.
   - `byteOffset + byteLength ≤ buffer.byteLength` (checked).
   - All views are validated.
6. **Availability**, per §2 and §2a.
7. **Property tables**, per §5.

Integers must be non-negative and fit in u64; anything else is InvalidInput. `extensions` in
subtree JSON is Unsupported, and unknown other properties are ignored.

**Typed facts only (RC-11).** After step 7, the owner keeps for the live subtree only:
- the member bytes, plus external buffer bytes;
- a view table (offset and length);
- the grouped availability sources (§2a);
- the sparse content index;
- the rank table;
- typed metadata column descriptors.

The parsed subtree `Value` is dropped before the walk descends.

### 2. Availability (rules)

Notation: `N` is 4 or 8; `L` is `subtreeLevels`; `T = (N^L − 1)/(N − 1)`; `C = N^L` (u128;
`L ≤ 32` gives `C ≤ 2^96`). `g` is the subtree root's global level, and `A` is `availableLevels`.

- **Encoding.** Exactly one of `bitstream` and `constant`; `constant` ∈ {0, 1};
  `tileAvailability.constant == 0` is InvalidInput.
- **Bitstream length.** A bitstream view has `byteLength == ⌈bits/8⌉`, with bits beyond the count
  equal to 0 (`:405`). Bit `i` is `(byte[i/8] >> (i%8)) & 1`.
- **availableCount.** When present it equals the popcount (for a constant: 0 or the count).
- **Root and parents.** Tile bit 0 is 1. `tile[off(l)+m] ⇒ tile[off(l−1)+⌊m/N⌋]`.
- **Content.** Layer count equals the template count; the array is absent when the root has no
  content. `content[k][i] ⇒ tile[i]`.
- **Child bits.** `child[m] ⇒ tile[off(L−1)+⌊m/N⌋]`.
- **Levels.**
  - No available tile at global level `≥ A`.
  - No set child bit if `g + L ≥ A`.
  - A looser `A` is admitted (OD-8).
- **Child subtrees.** Each set child bit's subtree must resolve and be admitted when the walk
  reaches it; a missing member is InvalidInput. Unset bits read nothing, and orphan members fall to
  the existing unreferenced-entry check.

### 2a. Availability evaluation and work (normative; RC-2)

**Sources.** Each availability object resolves to one *source*: `Const0`, `Const1`, or
`View(v, bits)` (bufferView `v`, with `bits = T` for tile or content and `C` for child). Within a
subtree, sources are **deduplicated by `(kind, v, bits)`**. Any number of content layers that name
the same view form one source.

Within subtree admission, the steps run in this order. Each is mirrored by O.

1. **O(1) per availability object.** Encoding rules, view index validity, and
   `byteLength == ⌈bits/8⌉`.
2. **Work charge, before any bit pass.** `Scan(s) = Σ byteLength` over the **distinct**
   `View` sources of subtree `s`. It is added to the validator's byte-work counter that already
   holds member reads (`resources.bytes_read`, limit `total_bytes_read`, 2 GiB). Exceeding it is
   ResourceLimit, with its own message: `availability scan exceeds total_bytes_read work limit` (OD-10).
3. **One pass per distinct `View` source.** Popcount and the trailing-zero check, memoized.
4. **`availableCount` per object.** Compared with its source's memoized popcount, O(1).
5. **Tile source.** The root bit and parent rule, in one pass. `Const1` satisfies them in closed
   form.
6. **Levels.**
   - For `Const1` tiles: InvalidInput if `g + L − 1 ≥ A`.
   - For a `View`: one range pass over levels `[A − g, L)`.
   - Child: if the child source's popcount is greater than 0 and `g + L ≥ A`, InvalidInput, in
     closed form.
7. **content ⇒ tile, once per distinct content source.**
   - `Const0`: holds.
   - `Const1`: holds iff the tile source is `Const1` or its popcount is `T`.
   - `View`: one pass of `content & ¬tile == 0`, done byte-wise against the tile source.
8. **child ⇒ tile.**
   - `Const0`: holds.
   - `Const1`: holds iff every tile bit of level `L−1` is 1: the tile source is `Const1`, or a range
     pass over that level is all ones.
   - `View`: one pass.
9. **Visit reservation.** `P_t` = the tile popcount (`T` for `Const1`, in u128). `P_t` must be
   `≤` the remaining `hierarchy_visits`, else ResourceLimit. On success it is charged at once.
10. **Content index (a reservation; DC-3).**
    - `E = Σ_k popcount(source of slot k)`, from memoized values, counting aliases once per slot.
    - `E` must be `≤ references − reference_visits`, else ResourceLimit. This is checked at this moment, and
      `reference_visits` counts every earlier reservation.
    - On success, `E` is **reserved**: `reference_visits += E` at once, exactly as `P_t` is reserved from
      `hierarchy_visits`.
    - Each emitted implicit content is then registered through a reservation-consuming call, for example
      `ArchiveResources::reference_reserved`. That call records the member in `used`, checks that it exists, and
      does **not** increment `reference_visits` again.
    - Subtree members and external buffers keep the ordinary `reference()` charge of one each.
    - When the subtree's walk completes, emitted must equal `E`; otherwise the result is InvalidState.
    - So `references` keeps its meaning: one per reference over the validation. `Σ_live E ≤ references` holds at
      every moment.
    - Then, with fallible allocation, build a sparse index of `E` entries `(tileBit, slot)`, sorted by tile bit and
      then slot. Each distinct content source's set bits are extracted once and shared by its aliasing slots.
11. **Rank table.** For a `View` tile source only: `4·⌈T/64⌉` bytes, `try_reserve_exact`.
12. **Property tables**, per §5.

**Walk.** Visiting a tile costs O(1) for its bit, plus its own index entries, plus at most `N`
child-subtree candidates when it is at local level `L−1` (`Const1` means all `N`). Child subtrees
are read and admitted **only when the walk descends into them**. Nothing ever enumerates `C`.
`Const0` content sources are never consulted per tile.

Template substitution happens only for emitted contents and admitted subtrees (RC-24). It substitutes decimal
digits into the **normalized** template (§3), at cost O(|normalized| + 40) ≤ about 1.1 KB per emission. The raw
template is never re-resolved per emission (DC-1).

**Work formula (normative).** Over one validation:

- **Availability bytes read ≤ 5 · Σ_s Scan(s) ≤ 5 · `total_bytes_read`** (DC-7; corrects v3's "four"). Pass list per
  subtree, where |x| is the byte length of a distinct `View` source:
  - the tile source is read by:
    - step 3;
    - step 5;
    - step 6 (a range);
    - step 8 (the level `L−1` range);
    - step 11 (rank);
    - step 7 **once per distinct `View` content source**;
  - each content `View` source is read by steps 3, 7 and 10 (extraction);
  - the child `View` source is read by steps 3 and 8.

  Every content `View` source has the tile view's length ⌈T/8⌉. So the step-7 re-reads of the tile total exactly
  Σ|content|, and the sum is `5|tile| + 4Σ|content| + 2|child| ≤ 5·Scan(s)`.
  - Constant sources are never read.
  - Implementations may fuse passes; the bound is an upper limit.
  - Bit-level work is at most 8× the bytes, for example step 8's per-child-bit parent lookups.
- **Content-index construction** is O(Σ_s E_s), with Σ E_s ≤ `references` over the validation, because of the
  reservation.
- **Per-tile work** is O(1 + entries + N) over at most `hierarchy_visits` tiles.
- **Template work (DC-1).**
  - Normalization is O(raw template length), done once per template at root admission. Σ raw template lengths
    ≤ that document's `json_bytes`.
  - Per-emission substitution is O(normalized length ≤ 1,024 + 40) for each of ≤ `references` contents and
    ≤ `hierarchy_visits` subtrees: at most about 350 MB of character work.
- **Class classification (DC-4).** O(properties of the class), **once per `(schema document, class)`**. Table
  admission is O(1 + the table's own properties) (§5).
- **Subtree JSON** is bounded by C1 `JsonLimits` per subtree. The subtree count is ≤ `hierarchy_visits`, because
  every subtree reserves at least 1 visit.

The reviewer's construction (`alias_budget.out.json`: K = 20,000 layers aliasing one 19.17 MB view)
costs `Scan` = 2 × 19.17 MB, about 0.2 GB of passes. Under the v2 text it cost about 714 GiB. See
R7.

### 3. Addressing and traversal

- **Morton.** Child index `c` sets `x = 2x + (c&1)`, `y = 2y + ((c>>1)&1)` and, for octrees,
  `z = 2z + ((c>>2)&1)`. Within a level, x is the LSB.
- **Coordinates.** `level < 32`; `x, y, z < 2^level`. Local coordinates are
  `global − (subtreeRoot << localLevel)`.
- **Templates: normalized once at root admission (DC-1, DC-2, OD-9).** This applies to every content template and to
  the subtree template. The steps run in this order, and the first failure wins:
  1. **Raw syntax.**
     - A `\` is InvalidInput.
     - `{level}`, `{x}`, `{y}` and, for octrees, `{z}` must each occur (`:231`); a missing one is InvalidInput.
     - `{z}` in a QUADTREE template, or any other `{` or `}`, is InvalidInput.
     - `:`, `?`, `#` or a leading `/` is **Unsupported**, as today's `uri()` maps them (`validate.rs:165-210`).
  2. **Escape guard.** In **any** segment, a `%` within two characters before a `{` is **Unsupported** ("escape
     adjacent to template variable"). Substituted digits could otherwise complete an escape, for example
     `.j%7{x}on` with x = 3.
  3. **Normalize** exactly as `uri()` would resolve the substituted string, treating each `{…}` variable as an
     opaque, non-empty run of decimal digits.
     - Start from the directory of the tileset JSON holding R (`:233`).
     - Split on `/`. Percent-decode each segment's static parts. A malformed escape, a decoded `/` or `\`, or a
       control character is InvalidInput.
     - Drop empty and `.` segments; `..` pops, and popping past the archive root is InvalidInput.
     - A segment that contains a variable can never be empty, `.` or `..`, and digits can never form `/`, `%` or
       an escape. So the normalized template, with digits substituted, **equals** `uri(base, substituted raw
       template)` for every coordinate. Normalization is exact.
  4. **JSON detection.** If the last segment of the normalized template ends in `.json` (ASCII case-insensitive),
     the template is **Unsupported** (content templates only). This covers `x.json/.`, `x.json/`,
     `x.json/{…}/..` and `.JSON//./` (DC-2).
  5. **Variable survival.** Every required variable must still occur in the normalized template. If one was popped
     by a `..` (for example `c/{x}/../a.glb`), the template is **Unsupported** ("template variables eliminated by
     dot-segments"). It would name the same member for every tile.
  6. **Length cap.** A normalized template longer than **1,024 bytes** (UTF-8, variables counted as their token
     text) is **Unsupported** ("normalized template exceeds 1 KiB").

  **Why a length cap rather than a byte charge (DC-1).**
  - The cap is static and decided at root admission, like the JSON verdict. The outcome cannot depend on
    availability or on emission order.
  - O mirrors it trivially, and no second runtime counter shares `total_bytes_read`.
  - Producer templates are about 30–60 B; the longest observed is `implicit-content/0-0-0-0-{level}-{x}-{y}-{z}-0.glb`.
  - Normalization itself costs O(raw length), once, and so absorbs the reviewer's 8 MB `./` padding.

  Explicit content still uses `uri()` and a case-sensitive `ends_with(".json")` on the decoded name
  (`validate.rs:1365`). The two rules differ, but `.JSON` explicit content still reaches `payload_kind`
  (`:490-500`) as Unsupported. A template whose member is tileset JSON without a `.json` suffix fails C1 decoding
  (InvalidInput).

  Per emission, digits are substituted into the normalized template (§2a "Walk"). Subtree **buffer** URIs are not
  templates: each resolves once per admitted subtree through `uri()`, relative to the subtree member, and its length
  is bounded by that subtree's JSON.
- **Traversal.** The owner drives a depth-first pre-order walk of one root: a tile, its emitted
  payload contents in slot order, then its available children in ascending child index. No
  external tileset is ever reached from an implicit tile. Explicit external tilesets around roots
  keep the existing traversal.
- **Revisits and cycles.** No coordinate is visited twice. A subtree member resolved twice is
  Unsupported (OD-7). Cycles arise only through explicit external tilesets (existing active set,
  `validate.rs:1122-1124`). Repeated payload members are admitted.
- **Post-walk.** If R has content templates and no tile emitted content, the result is InvalidInput
  (`:299`, "If there are no tiles with a content resource, then `tile.content` and `tile.contents`
  shall be omitted"; RC-15).

### 4. Effective volume and error

**R and I_0.** R keeps its JSON `V_R`, `transform` and `e_R`. I_0, the level-0 tile, is the same
tile for counting. Its `V_0` is I_0's TILE_BOUNDING_BOX row if present, else `V_R`. Its `e_0` is the
TILE_GEOMETRIC_ERROR row if present, else `e_R`.

**Checks.** `hierarchyBounds` checks `V_R` against R's explicit parent and `V_0 ⊆ V_R`.
`geometricErrorOrder` checks `e_R` against the parent and `e_0 ≤ e_R`.

**Tolerances (RC-9), stated as the actual change.** I1 applies the validator's existing explicit-tile
rules to the implicit root. These are *looser* than today's expander checks:

| Check | Today (expander) | I1 |
| --- | --- | --- |
| `V_0 ⊆ V_R` | `Obb::contains(corner, 1e-6)`: an **absolute** 1e-6 per axis in R's frame (`implicit/tileset.rs:761-765`, `bbox.rs:382-389`) | `contains()`/`inside()`: **1e-6·(1 + ‖all half-axis components‖)** (`validate.rs:297`) |
| `e_0` vs `e_R` | **Strict** `> root_error` (`:766`) | `> e_R + 1e-8·(1 + e_R)` (`validate.rs:1312`) |
| Parent of R | Today the expanded root carries `V_0` and `e_0` | `V_R` and `e_R` themselves are now also checked against R's parent, which is stricter |

These are recorded in the exit table, with controls T-1 to T-3.

**Deeper tiles** (level `l`, coordinates `(x, y, z)`):

- **Effective volume.** `Row(f64×12)` if a row exists, else `Derived(Cell { root: V_R.box, level:
  l, coords, scheme })`.
  - The meaning is exact rational arithmetic. Centre lane `j`:
    `c_j + Σ_k a_kj·((2·i_k + 1)/2^l − 1)`. Half-axis `k`: `a_k/2^l`. QUADTREE keeps axis 2.
  - Nothing rounded is produced as truth.
  - Level 0 without a row is reported as `DerivedRoot` (RC-22). Its value is exactly `V_R`.
- **Effective error.** The row, else `e_R·2^−l`, which is exact when `e_R = 0` or the result is
  normal. A subnormal nonzero result is Unsupported.
- **Refinement.** Constant. The expander's descendant `"REPLACE"` (`:1135`) is not ported.
- **Transforms.** Only R carries one. TILE_TRANSFORM is Unsupported. An explicit external tileset
  whose root is implicit composes its root transform once, as today.
- **Row values.** Finite. Error finite and `≥ 0`. FLOAT32 rows widen exactly.

For `Derived` cells, `hierarchyBounds` uses the OD-3 labelled rounded evaluation only. It never
enters C2 facts (§Interface).

### 5. Metadata rows

The schema is the inline `schema` or `schemaUri` of the document holding R. A table's `class` must
exist (InvalidInput).

**Class classification, once per `(schema document, class)` (DC-4).** On first use, the owner scans the class's
properties once. "First use" means a template header `metadata` class at root admission, or a table's class at
table admission. The scan records:
- the **Unsupported-semantic flag**, for the set in "Supported domain";
- whether the class is used as a tile class or a content class (the set differs for TILE_BOUNDING_BOX and
  TILE_GEOMETRIC_ERROR);
- the interpreted-property descriptors (property id, component type);
- the duplicate-semantic check;
- the list of `required` interpreted properties.

The record is reused for every later table of the same class. **Table admission is O(1 + the table's own
properties).** It never rescans the class.

**Unsupported semantics.** The set is tested through the classification record:
- for the class of `tileMetadata` and of each `contentMetadata` entry, at table admission;
- for the class of each content **template header** `metadata` entity, at root admission (DC-5). A template's
  `metadata` is copied to every derived content, so a CONTENT_URI or CONTENT_GROUP_ID there has the same override
  hazard as one in `contentMetadata`.

Unknown classes are InvalidInput, as for tables.

**Interpreted properties.** These are the properties of the `tileMetadata` class whose `semantic`
is TILE_BOUNDING_BOX or TILE_GEOMETRIC_ERROR, **by semantic, never by class or property name**.
Today the expander gates on `rustyTile`, `boundingBox`, `geometricError` and `extras`
(`implicit/tileset.rs:718-735`, `:997-1006`).
- A duplicated semantic in one class is InvalidInput.
- TILE_BOUNDING_BOX: `SCALAR`, `FLOAT32|FLOAT64`, `array true`, `count 12`, not normalized.
- TILE_GEOMETRIC_ERROR: `SCALAR`, `FLOAT32|FLOAT64`, not an array.
- Any other shape is InvalidInput. offset, scale, noData or default is Unsupported.

**Rows.**
- `count` equals the tile popcount (memoized).
- A tile's row is `rank(i) = popcount(tile[0..i))`.
- `values.byteLength` is exactly `count·12·size` or `count·size`.
- Values are read in place, little-endian.

**Omitted columns.**
- A `required` interpreted property missing from the table is InvalidInput.
- An optional one falls back to `Derived` or `e_R·2^−l` (RC-23).

**Other columns.** References to existing validated views only; contents opaque. The `extras`
STRING is not parsed.

**`contentMetadata`.** One entry per template, and `count` equals that slot's popcount (memoized per source).

**DC-8: a slot with no content in this subtree.**
- Property-table `count` has schema `minimum: 1` (`propertyTable.schema.json`), and the spec gives no
  "ignore the entry" semantics for an empty slot.
- So a subtree whose slot `k` has popcount 0 **cannot carry `contentMetadata` at all**. Its `contentMetadata[k]`
  table would need `count` 0 (schema violation) or `count ≠ popcount`.
- Both are **InvalidInput**. The writer must omit `contentMetadata` from that subtree.
- The same `minimum: 1` applies to `tileMetadata`. It is always met, because the root tile bit is 1.

**`subtreeMetadata`.** Opaque.

### 6. Profile exclusions

These are listed under Supported domain. Messages start `implicit: unsupported <reason>`.

### 7. Bounded work and memory

There are no new `ValidationLimits` fields (OD-5). Limits are selected once in `inspect_selected`.

| Quantity | Bound | Checked before | Outcome |
| --- | --- | --- | --- |
| Visits | `hierarchy_visits` (65,536). **Formula (RC-7):** `task_visits` = one per `Tileset`, `ExitTileset` and explicit `Node` task (`validate.rs:1101-1110`), plus `P_t` per admitted subtree (§2a step 9). R is a Node task; I_0 is bit 0 of the root subtree. Example: W2 = 1 + 1 + 1 + 428 = **431** | rank table, index and first visit | ResourceLimit |
| Depth | `hierarchy_depth` (128); depth(implicit tile) = depth(R) + level | the visit | ResourceLimit |
| Member bytes | `member_bytes` per member; `total_bytes_read` | the read | ResourceLimit |
| Availability scan | `Σ Scan(s)` charged to the same `total_bytes_read` counter (§2a step 2), with its own message; R documents this at `docs/VALIDATION.md:121` (OD-10) | any bit pass | ResourceLimit |
| Live-path subtree bytes | Σ member bytes, including external buffers, of the subtrees on the current path, `≤ document_decoded_bytes` (64 MiB). The path is the subtree of the current tile plus its ancestor subtrees in this root. Subtrees are released when their last descendant finishes. **This limit is shared with C1, which uses it as each payload's view budget** (`payload.rs:644-650`, `validate.rs:33`); both apply independently (RC-6) | each subtree or buffer read | ResourceLimit |
| Content entries | `E` **reserved** from `references` at subtree admission (`reference_visits += E`). Emissions consume the reservation without a second charge (§2a step 10, DC-3) | index allocation | ResourceLimit |
| Template length | Normalized template ≤ 1,024 B (§3, DC-1) | root admission | Unsupported |
| Subtree JSON | C1 `JsonLimits` | parse | as C1 |
| Rank table, index | `try_reserve_exact` | allocation | ResourceLimit |
| Levels | `≤ 32` (profile constants) | root admission | Unsupported |

- **ResourceLimit.** It comes only from the pre-allocation and pre-scan checks of existing limits
  above, or from host allocation refusal. It is never a verdict.
- **No allocation scales with declared counts.** Declared values (`T`, `C`, `availableCount`,
  `count`) size nothing until they are bounded by a view length, a popcount and a reservation.

**Memory held (RC-11).**
- Live subtree bytes `≤ document_decoded_bytes`.
- Plus rank tables `≤ ½ ·` live bytes.
- Plus content-index entries. Σ over live subtrees ≤ Σ reserved ≤ `references` (262,144) per validation, each
  `(u32, u16)`, so at most about 2 MiB with alignment. With the DC-3 reservation this bound now holds; v3's
  check-only text allowed about 7.7× `references`.
- Plus one classification record per `(schema document, class)` used, O(class properties) each. Classes are
  bounded by the schema's JSON limits.
- Plus normalized templates, ≤ 1,024 B each, per open root.
- Plus O(views + slots + columns) typed facts per live subtree. There are at most `⌈32/L⌉` live
  subtrees, each with at most `document_items` views and slots.
- Parsed subtree JSON is not retained during the walk.
- The serialized presentation tree is gone (today budgeted at 64 MiB, `implicit/tileset.rs:1160-1168`).
- This is not an RSS claim.

**InvalidState.** An internal invariant breach is `ValidationFailure::InvalidState` (exit 1), never
a verdict. Examples: rank ≥ count after the count check; a visit beyond the reservation; an index
mismatch; emitted contents ≠ reserved `E` at subtree completion.

## Failure semantics and precedence

| Kind | Meaning | Exit |
| --- | --- | --- |
| InvalidInput | Malformed per the pinned text or the invariants | 3 |
| Unsupported | Valid or undecided, outside the profile; this includes every JSON template | 2 |
| ResourceLimit | An existing limit checked before the allocation, read or scan it guards, or host allocation refusal | 3 |
| InvalidState | Internal breach | 1 |
| Io | Archive I/O (unchanged) | 1 |

**Precedence.** First failure in traversal order.

- **Root admission** comes first:
  - `implicitTiling` fields, scheme, levels;
  - volume kind, root shape;
  - templates, each in slot order and then the subtree template, through §3 steps 1–6: raw syntax, escape guard,
    normalization, JSON detection (content templates only), variable survival, then the length cap. A JSON verdict
    is Unsupported here, with no subtree read;
  - template header `metadata` classes (classification, DC-5).
- **Per subtree:** §1 steps 1–5, then §2a steps 1–11 (including the step 10 reservation), then §5 (class
  classification on first use, then the table).
- **Then the walk.**
- **Then the post-walk content rule.**
- Rules within one step are not ordered normatively. Controls are single-defect, and three
  double-defect controls pin the step order.

## Interface supplying C2 level chains

The owner is `src/content_integrity/implicit.rs` (crate-private, no `serde` derive, consumes
C1-admitted JSON). It knows no `ValidationLimits`, `Archive` or report types and does not import
`validate::containment`.

```text
pub(crate) enum Scheme { Quadtree, Octree }
pub(crate) struct ImplicitLimits { visits_remaining: u64, refs_remaining: u64 /* reserved at step 10 */, scan_remaining: u64,
                                   live_subtree_bytes: u64, json: JsonLimits }
pub(crate) fn admit_root(tile: &Value, schema: Option<&Value>) -> Result<ImplicitRoot, ImplicitError<Never>>
     // Unsupported for JSON templates (§3), Unsupported semantics in the schema are tested per table (§5)
pub(crate) enum EffectiveVolume { Row([f64; 12]), DerivedRoot, Derived(Cell) }
pub(crate) struct Cell { root: [f64; 12], level: u32, coords: [u32; 3], scheme: Scheme }
pub(crate) enum EffectiveError { Row(f64), Derived(f64) }
pub(crate) struct VisitedTile { level: u32, coords: [u32; 3],
                                volume: EffectiveVolume, error: EffectiveError,
                                contents: SmallVec<(slot: u16, uri: String)> }  // parent = the tile at level-1 on the path (RC-21)
impl ImplicitRoot { pub(crate) fn walk(&self) -> Walk<'_>; pub(crate) fn header(&self, slot: u16) -> &Value; }
impl Walk<'_> { pub(crate) fn next<E>(&mut self, limits: &mut ImplicitLimits,
        read: &mut dyn FnMut(&str, Base, u64) -> Result<Vec<u8>, E>)
        -> Result<Option<VisitedTile>, ImplicitError<E>>;
        pub(crate) fn finish(self) -> Result<(), ImplicitError<Never>>; } // post-walk content rule
pub(crate) enum ImplicitError<E> { Format(FormatError), Resolver(E), InvalidState(&'static str) }
```

`ImplicitLimits` carries the remaining values, which B passes in and reads back.
- The visit and reference reservations made at steps 9 and 10 reduce `visits_remaining` and `refs_remaining`.
- B writes them back into `task_visits` and `reference_visits`.
- B registers each emitted implicit content with the reservation-consuming call (§2a step 10), never with the
  charging `reference()`.
- `admit_root` returns the normalized templates. `VisitedTile.contents[].uri` is the **resolved member name**,
  already equal to `uri(base, substituted)` (§3), so B does not call `uri()` per emission. B keeps the
parent volume and error in a **level-indexed path stack**: in pre-order, a tile at level `l` has the
entry at `l − 1` as its parent (RC-21).

**In the I1 PR (RC-12a).**
- B registers R in the C2 tile arena exactly as today: an explicit `TileFact`, with
  `ImplicitTiling` excluded.
- B registers **no** `TileFact` for I_0 or any implicit tile, and no C2 reference for implicit
  tiles. The archive is excluded anyway.
- The OD-3 rounded box never enters `TileFact`.

**Obligations recorded for C2 slice 3.** These are not I1's, but its interface is designed for them.

1. **Facts.**
   - I_0 is `TileFact { transform: None, volume: Box(V_0), external: None, parent: R }`. For
     `DerivedRoot` this is `Box(V_R)`, exact.
   - A tile at level `l ≥ 1` with `Row(b)` is `TileFact { transform: None, volume: Box(b), parent }`.
   - `build_levels` (`validate.rs:590-614`) then yields `[V_0] →(None) [V_R] →(R.transform) …`.
     This is correct because `V_R` and `V_0` share R's frame.
2. **RC-12b. `Derived` tiles.** A `Derived` cell at `l ≥ 1` has no `VolumeFacts`. Under C2's I3
   (all or nothing), slice 3 must either:
   - add a frozen-enum delta: a new `Reason`, for example `derivedImplicitVolume`
     (`validate.rs:369`, the C2 contract's 15-name enum); or
   - add an exact `Cell` predicate (its own contract delta).

   "Stop setting `implicitTiling` and register references" alone is **not** sufficient.
3. **RC-12c. Chain length.** I_0 adds one level per implicit root on the path. The C2 contract's B3
   bound, "at most 129 levels" (`hierarchy_depth + 1`), becomes `hierarchy_depth + 1 + (implicit
   roots on the path)`, which is at most 130 when roots cannot nest within one chain except through
   explicit external tilesets. Each external implicit root adds 1, bounded by `hierarchy_depth`.
   Slice 3 must amend B3 and the `SmallSpanList` capacity statement. `try_push` is fallible, so the
   code is not unsafe.
4. **Arena size.** It stays `≤ hierarchy_visits`, because R and I_0 are each charged one visit.
5. **Producers.** Rows on every tile: `noRow = 0` in every probe. P is asked to keep this (OD-3).

## Validator migration (author B; lands in the combined PR)

1. **`prepare_tileset`.** It no longer expands (`validate.rs:1160`). Retained vector sources
   (`:1206-1217`) move into it unchanged.
2. **`node()` with `implicitTiling` on any tile.**
   - The nested refusal (`:1280-1284`) is removed.
   - The explicit-tile checks run on R.
   - `admit_root` is called. A JSON template returns Unsupported here.
   - Template headers go through `extension_object` once each.
   - The C2 `ImplicitTiling` exclusion is set for any tile.
   - B then loops over `walk().next()`, applying per tile:
     - `hierarchyBounds`;
     - `geometricErrorOrder`;
     - C1 payloads;
     - report-level budgets.
   - B registers emitted implicit contents through a new `ArchiveResources::reference_reserved(name)`, a small
     addition in `validate.rs`. It records the member as used and checks that it exists, without charging
     (§2a step 10).
   - B calls `finish()`.
   - There is no new task variant and no root registry.
3. **Removals (RC-10).** In `validate.rs`: `:56-62` `implicit_error`, its test at `:1758-1761`,
   `:898` and `:1600-1604` `expansion_budget`, and `:1203-1263` `expand_implicit`. In `implicit.rs`,
   the **`pub(crate) use … expand_tileset_bounded, ExpansionBudget` re-export at `:20`**.
   **`implicit/tileset.rs` is untouched by I1.** `expand_tileset_bounded`, `ExpansionBudget`,
   `producer_json` and `expansion_limit` stay as the private implementation of the retained
   `expand_tileset` (`:610-615`, `:667-1168`, tests `:1199+`). Its `validate::ValidationFailure`
   dependency (`:656-660`) remains until `expand_tileset` itself is retired (G6 or P).
4. **Counters.** `tiles` = explicit tiles + available implicit tiles, with R and I_0 counted once.
   `content_references` = emitted contents. Both are unchanged for every non-chunked output
   (verified: W1 73/73, W2 428/428).

### Exactly what the report says

For an archive whose implicit roots were all read:

- **`checks`** gains `"implicitAddressingAndAvailability"` after `"hierarchyBounds"`
  (`validate.rs:1716-1730`).
- **`notInspected`:**
  - loses `"implicitAddressingAndAvailability"`;
  - keeps `"decodedContentBounds"`, `"decodedContentBounds.implicitTiling"`, `"metadataSemantics"`
    and the others;
  - **gains `"recordedBudgets.implicitTileExtras"` in every archive that contains at least one
    implicit root** (OD-4 as recommended; RC-13). It does not depend on whether rows carry extras.
- **Report-level budgets still apply per implicit tile with content:** `conversion.json`
  `budgets.bytes`, `budgets.vertices` and `maxPoints` (`validate.rs:1392-1407`). Only the per-tile
  `extras` comparison (`:1408-1415`) is dropped.
- **Counts** are unchanged for admitted shapes.
- **Explicit-only archives:** no change.
- **Archives with a JSON template:** `ValidationFailure::Unsupported`, exit 2, message
  `implicit: unsupported external tileset content template in <document member>`.
- **Rust, CLI and Python** serialize the unchanged `ValidationReport`.

### Exit-status changes versus today (re-derived rule by rule; RC-8)

Re-derived against `implicit/tileset.rs:662-1196` (expander) and `validate.rs:1203-1284` (callers).
Every expander rule not listed keeps its category:
- header, length and alignment;
- BIN padding;
- view alignment and range;
- the `constant` rules;
- bitstream length, trailing bits and `availableCount`;
- root bit, parent rule and child-parent rule;
- content count and content ⇒ tile;
- the row-count check and non-finite or negative rows;
- `availableLevels` 0 and the data-beyond-`A` checks;
- implicit root `children`;
- sphere root (serde failure → InvalidInput);
- missing subtree template;
- non-finite root box or error;
- `{z}` in a QUADTREE template, leftover braces;
- missing members.

**E** means executed in this preparation; **S** means static.

| # | Change | Archives | Basis |
| --- | --- | --- | --- |
| X-1 | success → **Unsupported** | Any content template whose **normalized** last segment ends in `.json`, including the DC-2 forms `….json/.`, `….json/`, `x.json/{…}/..` and `….JSON//./`: today's chunked point and vector defaults (W3, W5), deep legacy `mesh-to-3tz`, and **archives already written by the retired `convert-to-implicit` with more than one node**. Single-node convert archives have no JSON slot and keep validating | E: W3 and W5 validate OK today |
| X-2 | InvalidInput → Unsupported | The #151 fixtures (JSON templates) | E |
| X-3 | InvalidInput → Unsupported | JSON-format subtrees (`:869-871` "invalid binary subtree header"); other `subdivisionScheme` strings (`:684`); region or S2 roots (`:710`); `availableLevels > 32` (`:690-691`) | S |
| X-4 | ResourceLimit → Unsupported | `subtreeLevels > 32` (`checked_pow`/`max_nodes`, `:695-699`) | S |
| X-5 | ResourceLimit → success | Sparse subtrees with `N^L > 65,536` but few available tiles (`:695-699`); per-expansion cumulative cache over 64 MiB (`:1099-1103`), now live-path; presentation over 64 MiB (`:1163-1168`), removed | S |
| X-6 | success → Unsupported | `extensions` in subtree JSON; CONTENT_URI, CONTENT_GROUP_ID and the other Unsupported semantics, in table classes and in template header `metadata` classes (DC-5); offset, scale, noData or default on interpreted properties (today misread silently, `:723-731`); repeated subtree members | S |
| X-7 | InvalidInput → success | Semantics on other class or property names, and FLOAT32 rows (`:718-735`, `:997-1006`); a `rustyTile` schema with a subtree lacking `tileMetadata` ("missing semantic tile metadata", `:994-996`); extras offset type, offsets or unreferenced bytes, and JSON (`:1009-1014`, `:1033-1035`, `:1048`, `:1058-1060`); buffer index ≠ 0 and external buffers (`:895-897`, `:909-911`) | E (class gating, #151) and S |
| X-8 | InvalidInput → read | Implicit roots on non-root explicit tiles (`validate.rs:1280-1284`) | S |
| X-9 | success → InvalidInput | Missing `bufferView.byteOffset` (`:912-916`); a template missing a required variable (`:838-851`); a `contentMetadata` count mismatch, including `count` 0 for an empty slot (DC-8); empty `minItems` arrays; the post-walk content rule (`:299`) | S |
| X-10 | InvalidInput → success | **Tolerance (RC-9).** `V_0` outside `V_R` by δ with 1e-6 < δ ≤ 1e-6·(1+‖h‖) (absolute → scaled); `e_0` in (e_R, e_R + 1e-8·(1+e_R)] (strict → relative) | S, controls T-1, T-2 |
| X-11 | success → InvalidInput | `V_R` or `e_R` escaping or exceeding R's explicit parent while `V_0` and `e_0` do not. Today the expanded root carries the row values. This applies to implicit roots of explicit external tilesets, and to new non-root placements | S, control T-3 |
| X-12 | success → ResourceLimit | Availability scans now charged to `total_bytes_read`, so archives near 2 GiB of combined reads and scans refuse earlier; aliased or overlapping availability views (R7b) | S |
| X-15 | success → Unsupported | Template profile (DC-1): a `%` escape adjacent to a variable; variables eliminated by dot-segments (for example the reviewer's `"./"×4M + "c/{level}/../…/a.glb"`); a normalized template over 1 KiB. Today the first two validate, and the third is refused only by the 64 MiB presentation budget after about 8 nodes (`implicit/tileset.rs:1160-1168`) | S; R8 |
| X-16 | (category kept, earlier) | Content-index reservation (DC-3): archives whose live index would exceed `references` now get ResourceLimit at subtree admission rather than during emission | S; R9 |
| X-13 | (category kept) | `3DTILES_implicit_tiling` (1.0) becomes Unsupported. Today it is not read and its members fall to the unreferenced-entry check (S, inferred) | S |
| X-14 | report | `checks` and `notInspected` labels as above; counts unchanged | — |

No public Rust API changes.

## Disposition of existing code

| Component (`f2ed077`) | Decision |
| --- | --- |
| `validate.rs:1203-1263` `expand_implicit` | **Replace** with the walk in `node()` |
| `validate.rs:1206-1217` | **Retain**, moved |
| `validate.rs:1166-1168` | **Rework**: exclusion for any tile |
| `validate.rs:1280-1284` | **Remove** (`:34`) |
| `validate.rs:56-62`, `:898`, `:1600-1604`, test `:1758-1761` | **Remove** |
| `implicit.rs:20` (the `expand_tileset_bounded`/`ExpansionBudget` re-export) | **Remove** (B, the only I1 edit outside `validate.rs` and the owner) |
| `validate.rs:1265-1436` `node()` | **Rework** |
| `validate.rs:1365-1378` explicit JSON content | **Retain**, unchanged |
| `validate.rs:1408-1415` extras comparison | **Remove for implicit tiles** (OD-4); retained for explicit tiles |
| `validate.rs:2550-2556` test | **Rework** |
| `validate.rs:197-358` legacy volume helpers | **Retain** for `hierarchyBounds` (D4 retires) |
| `validate/containment*` | **Untouched** |
| `implicit/tileset.rs` (whole file, including `:619-660`) | **Untouched by I1** (RC-10). Writers belong to P. `expand_tileset` stays for `store.rs:1006`, tests and the export, **not as truth**. Its defects are not ported: class and property gating, descendant `REPLACE`, `byteOffset` default, missing variable check, link handling, extras parsing, single buffer, f64 `derived_box`, absolute and strict root checks |
| `content_integrity.rs` | One `mod` line |
| `tests/p1_point_oracle.py:279-287` | **Reference only** (O) |
| `tests/native_implicit.rs` | P's (the chunked recipe changes with the layout) |
| `docs/VALIDATION.md:143` | R rewrites it on top of #159's retirement sentence: the presentation-budget text goes, and already-written multi-node convert archives are Unsupported (X-1) |
| Inventory `:40`, `:100` | R |

## Independent acceptance plan

### A. Oracle O-implicit (separate author; never reads `src/`)

- **Implementation.** Stdlib Python (`zipfile`, `json`, `struct`, `fractions`); no repository
  imports.
- **Sources.** Pinned primary text (re-pinned under `bench/architecture_audit/implicit_read/primary/`)
  and this contract. O may read `tests/p1_point_oracle.py:279-287` for Morton layout only, and the
  #151 data.
- **Self-test.**
  - AVAILABILITY interleave examples;
  - Morton round trips for level ≤ 4;
  - `Fraction` child-union identities;
  - live-path sums;
  - §2a source dedup on aliasing.
- **Mirrors exactly:**
  - §2a step order, including the `E` **reservation** (DC-3);
  - `Scan`, `E` and the visit formula;
  - live-path release;
  - §3 normalization steps 1–6 (escape guard, exact normalization, JSON detection on the normalized last segment,
    variable survival, 1 KiB cap);
  - the semantic sets, applied through the once-per-class classification, including header `metadata`;
  - the DC-8 `count` rule.
- **Verdict domain (RC-7).** Accepted, InvalidInput, Unsupported, or ResourceLimit (with the limit
  name). **It excludes C1 payload integrity:** O treats payload members as opaque, and every
  control keeps payloads valid, except C-10 and R1b, which state the C1 outcome explicitly.
- **Facts record.** For Accepted: the tile list in traversal order. Each entry has `doc`, `level`,
  `x`, `y`, `z`, `depth`, path-parent level, `volume` (`row:` hex×12 | `derived-root` |
  `derived:` root hex, level, coordinates), `error` hex, `refine` and `contents` (slot, member). The
  record carries a SHA-256.

**Parity (RC-16).** The `#[cfg(test)] pub(crate) fn implicit_tile_facts(request, limits)` seam and
its parity tests live in `#[cfg(test)] mod tests` in `src/validate.rs`, as C2 does with
`containment_work_facts` (`:1513-1531`, used at `:2746` and `:2847`). `tests/i1_implicit_read.rs`
covers CLI and public-API outcomes only.

### B. Witnesses (R regenerates on the combined-PR head; hashes go to O)

| ID | Producer and command | Expected |
| --- | --- | --- |
| W1 | point-cloud default, `cloud.las --sourceCrs local --maxPoints 1000` (73 tiles) | Accepted |
| W2 | point-cloud default, perf `points.las --maxPoints 2000` (428 tiles, 234 subtrees) | Accepted |
| W3 | point-cloud, `cloud.las --maxPoints 40`, on P | Accepted. On f2238cd: Unsupported (W3u) |
| W4 | vector default ×6 | Accepted |
| W5 | vector default, perf `vector.geojson --jobs 2`, on P | Accepted. On f2238cd: Unsupported (W5u) |
| W6 | legacy `mesh-to-3tz split.gltf --maxTriangles 512 --maxBytes 0` | Accepted |
| W6d | legacy deep mesh (`native_implicit.rs:23` recipe), on P | Accepted |
| W7 | legacy small mesh (ADD, `.gltf`) | Accepted |
| W8s | An already-written single-node convert archive (corpus `convert-implicit-from-vector-points-lines-explicit.3tz`, `460c2505…`) | Accepted |
| W8m | An already-written multi-node convert archive (A2-retained `vector-flat-rounded-implicit.3tz`, rehashed by R) | Unsupported |
| W9u | #151 fixtures, `schema.id` corrected | Unsupported |
| W9' | O-authored link-free `a2Tile` variants, including FLOAT32 | Accepted |
| W10 | Native and portable builds of W4 and W5 | As W4 and W5 |
| W11 | P's new-layout outputs for every changed producer | Accepted, production equal to O (merge gate) |

### C. Sensitive data controls (O; base and mutant both hashed; single defect)

"Facts" means the verdict stays Accepted but the tile-fact hash changes identically in production
and O.

| # | Control | Expected |
| --- | --- | --- |
| A-1 | Clear tile bit `i` that has an available child in the same subtree. Co-adjust: `availableCount −1`, remove row `rank(i)` from every column with `count −1`, clear `i` in each content layer, and **delete** its payload members | InvalidInput (parent rule only) |
| A-2 | Set a tile bit at a leaf position whose parent is available. Co-adjust: `availableCount +1`; insert a row at its rank (copy of the parent row); no content | Accepted, facts (+1 tile) |
| A-3 | Bitstream rewritten MSB-first | InvalidInput or facts |
| A-4 | `availableCount ±1` | InvalidInput |
| A-5 | Trailing bit set | InvalidInput |
| A-6 | Content bit without its tile bit | InvalidInput |
| A-7 | Child bit under an unavailable tile | InvalidInput |
| A-8 | Child bit with `g + L ≥ A` | InvalidInput |
| A-9 | `A` = 32 versus 33 | Accepted versus Unsupported |
| A-10 | Missing child subtree | InvalidInput |
| A-11 | Repeated subtree member | Unsupported |
| A-12 | Three double-defect controls pinning the step order | Specified |
| A-13 | Content layers aliasing one view (K = 3), with a valid popcount | Accepted; facts equal to the unaliased form |
| A-14 | Child availability `Const1` with level `L−1` incomplete | InvalidInput, decided in closed form |
| B-1 | Rows swapped | Facts |
| B-2 | Row ±1 ulp | Facts |
| B-3 | Interpreted property and class renamed | Facts unchanged |
| B-4 | Semantic removed | Facts become `derived` (`derived-root` at level 0) |
| B-5 | Semantic duplicated | InvalidInput |
| B-6 | FLOAT32 rows | Accepted, widened |
| B-7 | `scale` on TILE_BOUNDING_BOX | Unsupported |
| B-8 | `count ±1` | InvalidInput |
| B-9 | Error −1 or NaN | InvalidInput |
| B-10 | Derived error subnormal | Unsupported |
| B-11 | A required interpreted property omitted from the table | InvalidInput (RC-23) |
| B-12 | An optional one omitted | Accepted; facts fall back to `derived` (RC-23) |
| C-1 | Morton x and y swapped in member names | InvalidInput or facts |
| C-2 | Template without `{y}` | InvalidInput |
| C-3 | `{z}` in a QUADTREE template | InvalidInput |
| C-4 | JSON template, slot unavailable everywhere | Unsupported |
| C-5 | JSON slot available at level 0 | **Unsupported (RC-3)** |
| C-6 | JSON slot available at level ≥ 1 (the producer pattern) | Unsupported |
| C-7 | `.JSON`; `.js%6Fn`; `.j%7{x}on` (escape guard); `.json?v=1`; **DC-2:** `c/{…}.json/.`, `c/{…}.json/`, `c/x.json/{…}/..`, `c/{…}.JSON//./` | Unsupported, every case, at root admission, also when the slot is never available |
| C-7b | Escape guard in a **non-last** segment: `c%2{x}/{level}-{y}-{z}.glb` | Unsupported ("escape adjacent to template variable") |
| C-7c | Variables eliminated: `c/{level}/../{x}/../{y}/../{z}/../a.glb` | Unsupported |
| C-7d | Normalization exactness: `./c//{level}/./{x}-{y}-{z}.glb` | Accepted; members equal those of `c/{level}/{x}-{y}-{z}.glb` |
| C-8 | Implicit root on a non-root explicit tile | Accepted |
| C-9 | Region root / sphere root / scheme `"HEXTREE"` | Unsupported / InvalidInput / Unsupported |
| C-10 | `.glb` template whose member is tileset JSON | InvalidInput (C1 magic) |
| C-11 | Explicit external tileset whose root is implicit, with a transform | Accepted |
| C-12 | `contentMetadata` class with **CONTENT_URI**, including a value naming `x.json` | Unsupported (RC-1) |
| C-13 | `contentMetadata` class with **CONTENT_GROUP_ID** | Unsupported (RC-1) |
| C-12b | A content **template header** `metadata` whose class has CONTENT_URI, and separately CONTENT_GROUP_ID | Unsupported at root admission (DC-5) |
| C-14 | Root content templates present, no tile with content | InvalidInput (post-walk) |
| D-1 | Truncated BIN | InvalidInput |
| D-2 | `byteOffset` +1 or absent | InvalidInput |
| D-3 | Non-zero BIN padding | InvalidInput |
| D-4 | Bad magic or version | InvalidInput |
| D-5 | External buffer as a data URI | InvalidInput |
| D-6 | Valid external buffer | Accepted |
| D-7 | JSON-format subtree | Unsupported |
| D-8 | JSON chunk ending in `\n` with zero padding | **Accepted (RC-5)** |
| D-9 | `"bufferViews": []` | InvalidInput (RC-15) |
| D-10 | `contentMetadata` present in a subtree where one slot has popcount 0: (a) that slot's table has `count` 0; (b) it has `count` 1 | InvalidInput in both cases (DC-8) |
| T-1 | `V_0` outside `V_R` by 2·10⁻⁶ when ‖h‖ ≥ 10, and by 2·(1e-6·(1+‖h‖)) | Accepted, then InvalidInput |
| T-2 | `e_0 = e_R·(1+10⁻⁹)`, and `e_R + 2·1e-8·(1+e_R)` | Accepted, then InvalidInput |
| T-3 | Implicit external root whose `V_R` escapes the referencing tile while `V_0` does not | InvalidInput |

### D. Hashed implementation mutants (R; separate production and oracle patches; each must flip its paired control)

| Mutant | Paired control |
| --- | --- |
| MSB-first | A-3 |
| Inclusive rank | B-1, W1 |
| Drop content ⇒ tile | A-6 |
| Drop child ⇒ tile | A-7 |
| Drop `availableCount` | A-4 |
| Key by name | B-3 |
| Ignore override | B-4, W6 |
| Swap x and y | C-1, W2 |
| Error not halved | B-12 |
| `≥` versus `>` on `A` | A-8, A-9 |
| FLOAT32 read with FLOAT64 stride | B-6 |
| Ignore trailing bits | A-5 |
| Admit JSON templates | C-6, W5u |
| Case-sensitive or undecoded suffix | C-7 |
| Read `scale` | B-7 |
| Ignore CONTENT_URI | C-12 |
| Ignore CONTENT_GROUP_ID | C-13 |
| Drop repeated-subtree check | A-11 |
| **No source memoization** (per-object passes) | R7 (time bound) |
| **Charge `Scan` per object rather than per distinct source** | R7 (gives ResourceLimit instead of InvalidInput) |
| **Reservation after the rank-table allocation** | R5b |
| **Per-visit charging instead of reservation** | R1b |
| **Live path counts the current subtree only** | R2 |
| **Cumulative instead of live path** | R2b |
| Refine forced to `REPLACE` | W7 |
| 0x20-only padding rule | D-8 |
| Skip post-walk content rule | C-14 |
| Absolute 1e-6 retained | T-1 |
| **Detect on the raw last segment** (v3 rule) | C-7 DC-2 forms (DC-2) |
| **Resolve the raw template per emission** | R8a (timeout; DC-1) |
| **No variable-survival check** | C-7c, R8 |
| **Escape guard on the last segment only** | C-7b |
| **Check-only content index (no reservation)** | R9 (DC-3) |
| **Per-table class scan** | R7c (timeout; DC-4) |
| **Header `metadata` class not classified** | C-12b (DC-5) |

A mutant that flips nothing is a failed control.

### E. Resource controls (R, through `inspect_selected` with explicit `ValidationLimits`)

- **R1.** `hierarchy_visits` = **431** on W2 gives Accepted; 430 gives ResourceLimit (formula in
  §7).
- **R1b (RC-14).** The last subtree has ≥ 2 tiles and a corrupt payload in its first tile;
  `hierarchy_visits` is one short of the full count. Reservation gives ResourceLimit at that
  subtree's admission. Per-visit charging would give InvalidInput.
- **R2 (RC-6).** An O-authored **content-free synthetic**: QUADTREE `L = 1`, `A = 6`, branching 2
  per level, root content omitted (`:299`).
  - Each subtree is padded with an unused 64 KiB view in BIN.
  - The deepest live path holds 6 subtrees, so its maximum live sum is about 6 × 64 KiB. The
    cumulative sum is much larger.
  - There are no payloads, so C1's use of `document_decoded_bytes` cannot fire first.
  - Expected: cap = maximum live sum gives Accepted; one less gives ResourceLimit before the read.
- **R2b.** The same fixture with any cap between the maximum live sum and the cumulative sum gives
  Accepted. On W2, the cap must lie in `[190,944, 246,583]`: above C1's largest payload view sum and
  below W2's cumulative 246,584 (reviewer `w2_caps.py`: maximum live sum 23,864). Expected:
  Accepted.
  - **The lower bound must be confirmed by execution (DC-8).** 190,944 is a Python sum of `bufferView.byteLength`,
    not C1's accounting.
  - Before fixing the window, R runs W2 through `inspect_selected` with `document_decoded_bytes` = 190,943
    (expected: C1 refuses with ResourceLimit) and = 190,944 (expected: accepted), and records both receipts.
  - If C1's actual threshold differs, the window's lower bound is the executed value, and the contract number is
    updated before O freezes R2b.
- **R3 (RC-7).** OCTREE `L = 8`, `A = 8`, tile `Const1` (2,396,745 tiles), child `Const0` gives
  ResourceLimit at step 9. Peak RSS (`/usr/bin/time -v`) must be within 16 MiB of W1. The same holds
  for `L = 32`, `A = 32`.
- **R4.** `hierarchy_depth` exact versus one less.
- **R5.** An injected rank-table allocation refusal gives ResourceLimit.
- **R5b (RC-14).** A **bitstream** tile source whose popcount exceeds the remaining visits gives
  ResourceLimit. A test-only allocation probe records zero rank-table allocation attempts.
- **R6.** Bitstream `C` at the `member_bytes` boundary.
- **R7 (RC-2).** The reviewer's `alias_budget` construction: OCTREE `L = 10`, `A = 1`, a 19.17 MB
  tile view with bit 0 set, and K = 20,000 content layers aliasing one all-zero view.
  - Expected: **InvalidInput by the post-walk content rule**, within **5 s wall** on the
    coordinator host (release build).
  - R7a: the same with one extra layer that has bit 0 set gives Accepted within 5 s.
  - The no-memoization mutant must exceed a 60 s timeout. The per-object-charge mutant gives
    ResourceLimit.
- **R7b (restated; DC-6).** OCTREE `L = 10`, `A = 1`, **K = 10,000** content layers, each naming its **own**
  distinct bufferView. The views overlap: each is 19,173,962 B at byte offsets 0, 8, 16, …, within one buffer of
  about 19.26 MB (member < 64 MiB).
  - About 6 subtree JSON nodes per layer (4 per view, 2 per availability object), so about 60,000 nodes, under
    `document_items` 65,536 and `json_bytes` 8 MiB. The tileset holds 10,000 templates (about 20,000 nodes).
  - `Scan` ≈ 10,001 × 19.17 MB ≈ 192 GB, far above 2 GiB.
  - Expected: **ResourceLimit at §2a step 2**, with the scan message, before any bit pass, within 5 s, under
    default limits.
- **R7c (DC-4).** OCTREE `L = 1`, `A = 6`: 37,449 single-tile subtrees. Child availability is `Const1` in subtrees
  at levels 0–4 and `Const0` at level 5, as required by §2a step 6. Each has one `tileMetadata`
  table with one opaque BOOLEAN column, `count` 1. Its class has 32,668 opaque `{"type":"BOOLEAN"}` properties (the
  tileset stays within 65,536 nodes and 8 MiB). The root has no content templates.
  - Expected: Accepted within 5 s.
  - The "per-table class scan" mutant costs about 1.22·10⁹ property visits and must exceed a 60 s timeout.
- **R8 (DC-1).** The reviewer's construction: OCTREE `L = 6`, tile `Const1` (37,449 tiles), one `Const1` slot, and
  the template `"./"×4,000,000 + "c/{level}/../{x}/../{y}/../{z}/../a.glb"`.
  - Expected: **Unsupported at root admission** ("template variables eliminated by dot-segments") within 5 s.
    Normalization is once, O(8 MB).
- **R8a (DC-1).** The same tree and padding, with the template `"./"×4,000,000 + "c/{level}-{x}-{y}-{z}.glb"`. The
  variables survive, and the normalized length is 27 B. It has 37,449 minimal payload members.
  - Expected: Accepted, with wall time recorded. Bound: within 2× the time of the same archive without the padding,
    and ≤ 60 s.
  - The "resolve the raw template per emission" mutant costs about 37,449 × 0.12–0.2 s ≈ 75 min, measured
    per-`uri()` cost (reviewer `uri_probe/`), and must exceed a 300 s timeout.
- **R8b.** Normalized template length exactly 1,024 B gives Accepted; 1,025 B gives Unsupported.
- **R9 (DC-3).** OCTREE `L = 3`, tile `Const1` (73 tiles), child `Const1`, `A = 6`. K = 2,500 content slots, all
  `Const1` with the identical template `c/{level}-{x}-{y}-{z}.glb`, so repeated payload members are admitted.
  - The root subtree reserves E = 182,500.
  - Expected: **ResourceLimit at the first child subtree's step 10** (182,500 + 182,500 > 262,144), before its index
    allocation.
  - The "check-only" mutant passes the check and allocates. The R5b allocation probe observes more than 262,144
    live index entries.

### F. Adapters, CI and review

Rust, CLI and Python parity on W1, W2, W5 and W5u; installed wheel; official Blender as applicable;
exact-head CI of the combined PR; fresh nonauthor final review.

## Performance obligation (perf lane pinned at `bb7d054`; RC-18)

**The lane.** Branch `perf/lane`, commit `bb7d0549b0f7a35f1127a431865140715904d933`. R runs the
committed files, not the modified worktree copy:

| File | Blob | sha256 |
| --- | --- | --- |
| `bench/perf_lane/process.py` | `beee98cd…` | `513bec79…` |
| `corpus.py` | `85581217…` | `a6cd738c…` |
| `README.md` | `1f07edee…` | `25b94921…` |

**The rule** (README "Gate"). A case is flagged when head/base exceeds 1.25 **and** the growth
exceeds the floor (0.1 s wall or 16 MiB RSS), or when head fails a case that base passes. There are
no `--explain` waivers for I1's validate cases.

**Base and head.**
- **Before** = the combined branch with I1's validator commits reverted (P only).
- **After** = the combined head.
- Same archives, serial runs, coordinator's two pinned CPUs, `--repeat 3`.

**Cases.**
1. **Standing:** `validate:vector`, `validate:point-cloud`.
2. **I1:** `validate` on W2, W3 and W5. These are reported; they can double without flagging below
   the 0.1 s floor.
3. **Amplified, the gating case.** A point cloud in P's layout tuned so that **base best wall is ≥
   0.4 s**. Only then can the 1.25× ratio exceed the 0.1 s floor. Use perf `points.las` with
   `--maxPoints` lowered, or a stdlib-generated larger LAS pinned by hash. R records the flag, the
   tile count and the base wall. If the base cannot reach 0.4 s within 65,536 visits, R records that
   and pins the closest case. The gate then rests on RSS only.
4. **Reporting.** Wall time, CPU time and peak RSS; a gated regression blocks the combined PR.

C2 slice 3 owns the later streaming cost.

## Proof limits

The following are not provided:
- decoded containment for implicit tiles (C2 slice 3, with obligations RC-12b and RC-12c);
- regions and S2;
- JSON subtrees;
- other TILE_* and CONTENT_* semantics;
- group references;
- viewer selection;
- whole-process memory.

There is no conformance verdict on JSON templates. Nothing is removed beyond the validator's use of
the expander and the `implicit.rs:20` re-export.

Parents #113, #120, #125 and #126 stay open. There is no main merge, tag or publication.

## Amendments adopted after the interface freeze

### v3.2: coordinator rulings after A's implementation (2026-10-11)

1. **Subtree magic.** A member whose first non-whitespace byte is `{` (UTF-8 BOM allowed) is a JSON-format subtree and is Unsupported. Any other member not starting with `subt` is InvalidInput (D-4). A bad version is InvalidInput. This resolves the Supported-domain/D-4 conflict.
2. **Interface amendments**, accepted:
   - `admit_root(tile, base: &str, schema: Option<&ImplicitSchema>)`. `ImplicitSchema` is per schema document, which gives DC-4's once-per-(document, class) classification across roots.
   - `ImplicitLimits::new(live_cap, json)` carries the validation-wide OD-7 repeated-subtree set.
   - `Base = Resolved | Relative(member)` with a `Resolver` callback. The resolver charges exactly one reference and `bytes.len()` bytes, which the owner mirrors.
   - `VisitedTile.contents: Vec<(u16, String)>`.
   - Additions: `declares_implicit`, `ImplicitRoot::{header, slots, scheme, walk}`, `Walk::{next, finish}`, `ImplicitError::widen`, `Cell::rounded_for_hierarchy_bounds` (OD-3, labelled, `hierarchyBounds` only).
3. **Interpretations**, accepted:
   - `extensions` "anywhere" excludes values inside `extras`.
   - Table-level `offset`/`scale` on an interpreted column is Unsupported, as for class-level ones.
   - The subnormal derived-error rule applies at level ≥ 1; level 0 is `e_R`.
   - The escape guard is per segment, so a malformed escape such as `c%/{level}` is InvalidInput.
   - Implicit integer fields are read as JSON integers, so `8.0` and `1e1` are InvalidInput. This matches the previous expander.
   - A duplicated semantic of any kind in one class is InvalidInput.
   - Row values (finite, error ≥ 0) are checked eagerly at table admission.
   - OD-7 is validation-wide: an explicit tileset that references the same implicit-root external tileset twice is Unsupported.

### Integration amendments (combined I1 + P branch)

- **Link tiles and external documents in recorded budgets (P delta review D-1, D-2; PD-13).**
  `budgets.tiles` is compared against the producer hierarchy, so tiles whose only content is an
  external tileset document (P's link tiles) are not counted. External tileset `.json` members do
  not count toward per-tile `bytes`.
- **C1 no-content fixture (X-9).** The C1 fixture `implicit_no_content.3tz` declared
  `"contentAvailability": []`, which is InvalidInput under this contract (empty `minItems` array).
  The fixture now omits `contentAvailability` and stays a valid no-content implicit case; its
  digest and that of `implicit_duplicate_subtree_key.3tz` changed in `tests/fixtures/c1/manifest.json`.
- **Pinned C1 context controls (X-14).** The 2026-10-10 context-control manifest's two implicit
  expected reports were amended additively for the label change only; hashes and reason are in
  [C2-PIN-AMENDMENT.md](../../bench/architecture_audit/c1_payload_integrity/production_controls/context_controls/2026-10-10/C2-PIN-AMENDMENT.md).
- **Validator caps (#160, merged from `develop` `766b92b`).** The default ceilings this text
  cites were raised: `hierarchy_visits` 65,536 to 262,144, `references` 262,144 to 1,048,576,
  `archive_entries` to 524,288, `document_items` to 131,072, and `member_bytes` and
  `document_decoded_bytes` (and so the live-path subtree cap) from 64 to 128 MiB. I1's formulas
  and outcomes are unchanged; only the default values move. The R9 control in
  `tests/i1_implicit_read.rs` is rescaled from K = 2,500 to K = 7,200 `Const1` slots, so the
  first child subtree's reservation (2 × 525,600) still exceeds the default `references`. The
  W2 visit example (431) and the DC-8 receipt use explicit limits and are unaffected.
- **DC-8 receipt (R2b lower bound).** Executed on the integration branch through
  `inspect_selected` with only `document_decoded_bytes` changed, on W2 (`pc-perf-2000.3tz`,
  sha256 `964e714f5f36f3b926f0af5f82150083c3e3c7ee28a70ceb6920f554630ed2b8`):
  - 190,943: ResourceLimit (`resource_limit`, exit 3), "views exceed decoded byte allowance",
    C1's per-payload view budget;
  - 190,944: accepted, `tiles` 428, `contentReferences` 428, with `implicitAddressingAndAvailability`
    in `checks` and `recordedBudgets.implicitTileExtras` in `notInspected`.

  The executed threshold equals the contract value, so the R2b window `[190,944, 246,583]` is
  unchanged.
