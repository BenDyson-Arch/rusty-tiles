# I1: independent implicit effective-volume read owner (contract draft v1)

Status: **draft for a fresh nonauthor precode review**. Nothing is implemented. Owners:
[#125](https://github.com/BenDyson-Arch/rusty-tiles/issues/125) (implicit traversal), with
#126/A2 and C2 slice 3 as consumers. Base: `origin/develop` `f2ed077`. The tree is identical to
`/tmp/rusty-tiles-c2` `3563d53` and includes C2 (PR #157), so the recommendation's "#157 must
merge first" gate is already met. The scope comes from
`/tmp/next-slice-selection-20261011/recommendation.md` §2.

Every line reference below is to `f2ed077`.

Evidence for this draft:
- [producer-inventory.md](producer-inventory.md), [open-decisions.md](open-decisions.md) and
  [ownership-plan.md](ownership-plan.md);
- `probes/receipt.json`, which pins the binaries, inputs, samples, scripts and primary texts;
- the raw probe outputs in `probes/results/*.json`.

The probes are fact-finding by the contract author. They are not the acceptance oracle.

The draft claims nothing about:
- decoded content containment (C2 slice 3);
- standards conformance of JSON content templates on implicit roots (OD-1);
- regions or S2;
- viewer selection;
- metadata semantics beyond the two interpreted semantics;
- memory or RSS;
- release readiness.

## Corrections to the selection record (found while preparing)

1. **"Reading what point and vector already emit does not depend on the A2 template verdict" is
   only half true.**
   - At realistic sizes, both default producers write the representation the A2 gate is about:
     an implicit root whose `contents` includes an external-JSON template
     (`implicit-tileset-{prefix}-{level}-{x}-{y}[-{z}].json`, `implicit/tileset.rs:449-451`).
     The JSON slot is available only at non-root, childless tiles.
   - Executed observations:
     - `pc-deep.3tz` (point-cloud default, `--maxPoints 40`) has 208 implicit documents and 207
       JSON links.
     - `vec-perf.3tz` (vector default on the perf-lane input) has 85 documents, external nesting
       to depth 3 and 84 JSON links.
     - Today's validator accepts both.
   - The reader therefore has to adopt a reading of those templates; this is OD-1. A *literal*
     InvalidInput reading would turn default producer output red. It would also turn
     `convert-to-implicit` output red, because that operation validates its own candidate before
     publishing (`convert_implicit.rs:205`).
2. **The producers also emit child subtrees.** `pc-perf-2000.3tz` (the 200k-point perf input at
   `--maxPoints 2000`) has 234 subtrees and 233 child-subtree bits, with no JSON links.
   - Which form the writer uses depends on the data (`implicit/tileset.rs:284-308`).
   - Both forms are in the supported domain.
3. **C2's dyadic arithmetic cannot be reused without touching `src/validate/containment*`.**
   `dyadic.rs` is `pub(super)` (`containment/dyadic.rs:10-200`), and `LevelFacts` carries `f64`
   volumes only (`containment.rs:42-53`). The exact derived box is therefore represented
   symbolically (see Invariants: effective volume and error).
4. **The primary texts were incomplete.**
   - `3d-tiles-core.adoc` only cross-references implicit tiling (`:837-877`) and metadata
     (`:1003`).
   - The implicit README and the Semantics README were pinned by A2 (`a2_format_settlement/
     primary-sources.json`). Byte copies exist at `/tmp/rusty-tiles-a2-format-primary/` and match
     those pins.
   - These were not pinned anywhere: `AVAILABILITY.adoc`, `Metadata/README.adoc`, the
     PropertyTable README, and the Subtree, PropertyTable, class-property, `subtrees` and
     `tile.implicitTiling` schemas. They were fetched at the same commit `4d781014` into
     `primary-fetch/`; their hashes are in the receipt.
5. **The #151 design fixtures are not valid as committed.** Their metadata `schema` lacks the
   required `id` (Schema/schema.schema.json). Today's validator rejects them with
   `tileset schema: "id" is a required property`. They are reusable only with that recorded
   correction (`probes/materialize_a2_fixture.py`).

## Claim

When `checks` contains `implicitAddressingAndAvailability`, the archive contains at least one tile
with `implicitTiling`, and every such *implicit root* reached from `tileset.json` was read. This
includes implicit roots on any explicit tile and roots reached through external tilesets. For
each implicit root, all of the following hold:

1. **Subtrees.** Every subtree reachable through available child-subtree bits was resolved from
   its template, read from a confined archive member, and admitted under the subtree-container
   invariants.
2. **Availability.** Tile, content and child-subtree availability satisfy the availability
   invariants, evaluated with LSB-first bit order and Morton level order.
3. **Visiting.** Every available tile was visited once, in the defined order, at its exact global
   coordinates.
   - Its effective bounding volume and geometric error follow the effective-volume rules.
   - Its refinement is the implicit root's effective `refine`.
   - Its available contents resolve to confined, existing members.
4. **Same checks as explicit tiles.** Each visited tile passed the checks today's validator
   applies to explicit tiles:
   - C1 payload integrity for payload contents;
   - external-tileset traversal for JSON contents;
   - `hierarchyBounds` and `geometricErrorOrder` against its parent;
   - `resourceReferences`;
   - recorded budgets, subject to OD-4.
5. **Metadata.** Every tile-metadata row that supplies an interpreted semantic was decoded at the
   tile's availability rank and type-checked.

The claim does not cover:
- decoded content containment (`decodedContentBounds.implicitTiling` remains until C2 slice 3);
- the meaning of non-interpreted metadata columns, content metadata or subtree metadata
  (`metadataSemantics` remains notInspected);
- the standards status of JSON content templates on implicit roots (OD-1);
- viewer culling or selection.

Normative basis:
- `ImplicitTiling/README.adoc` (sha256 `14a37a34…`): implicit root constraints `:72-76`,
  subdivision rules `:104-126`, template URIs `:158-162`, availability `:171-207`, tile metadata
  `:214-230`, buffers and views `:249-255`, availability encoding `:257-261`, binary format
  `:560-589`. Line numbers count the file after removing blank and `image::` lines, as printed
  by the probe.
- `AVAILABILITY.adoc`: Morton interleave (x is the least significant bit), level offsets, and
  global and local coordinates.
- `Metadata/README.adoc`: binary little-endian, tightly packed; booleans LSB-first;
  offset/scale; noData/default.
- `Semantics/README.adoc`: TILE_* precedence over derived values, and the TILE_BOUNDING_BOX and
  TILE_GEOMETRIC_ERROR types.
- `class.property.schema.json`: "The semantic cannot be used by other properties in the class".
- `subtree`, `availability`, `buffer` and `bufferView` schemas: `bufferView.byteOffset` is
  required; `tileAvailability.constant: 0` is disallowed.
- Core `README.adoc:731-748`: an external-tileset tile has no children; no cycles; transforms
  compose.

## Supported domain

### Admitted (what producers emit; inventory rows P1–P4, V1–V2, M1–M2 and C1–C2)

- **Tiling.** `QUADTREE` or `OCTREE`, with `subtreeLevels` and `availableLevels` each in
  `[1, 32]`.
- **Root volume.** A `box` with any half-axes, including rotated and degenerate ones. C2 decides
  what it can test.
- **Placement.** An implicit root on any explicit tile of any tileset document, including tiles
  inside external tilesets. Its `transform` and `refine` follow core rules. When `refine` is
  absent it is inherited; the producers always write it.
- **Content templates.** A single `content` or a `contents` array of templates. Each template
  object may carry other properties, such as `extensions` and `group`. Those properties are a
  *content header* applied to every derived content, as Cesium does
  (`Implicit3DTileContent.js:453`).
- **Subtrees.** Binary subtrees with:
  - one internal buffer, and external buffers that resolve to confined members (decided in OD-6);
  - bitstream or constant availability, with or without `availableCount`;
  - any number of child subtrees.
- **Tile metadata.** A `tileMetadata` property table of any class. TILE_BOUNDING_BOX and
  TILE_GEOMETRIC_ERROR are interpreted when the class declares them **by `semantic`**:
  - component type FLOAT32 or FLOAT64;
  - no offset, scale, noData or default on those two properties.
  All other columns are opaque, including the producers' `extras` STRING column.
- **External JSON content.** JSON content in a slot that is available only at level ≥ 1, on a
  tile with no available implicit child and no other available content slot. This is the
  terminal-occupancy reading; it is pending OD-1.
- **Unused metadata entries.** `contentMetadata` and `subtreeMetadata` are structurally counted
  but otherwise opaque. They are admitted unless a CONTENT_BOUNDING_* or CONTENT_*_HEIGHT
  semantic is present.

### Unsupported (valid, outside this profile; exit 2)

- **Root volumes.** `region` roots (#120); `3DTILES_bounding_volume_S2` roots.
- **Subtree format and extensions.**
  - JSON-format subtrees: the template resolves to a member whose bytes do not start with
    `subt`. Producers never write them.
  - The 3D Tiles 1.0 `3DTILES_implicit_tiling` tile extension.
  - Any `extensions` object inside subtree JSON.
- **Tile metadata semantics** other than the two interpreted ones, if they affect volume,
  transform or refinement:
  - TILE_BOUNDING_SPHERE, TILE_BOUNDING_REGION, TILE_BOUNDING_S2_CELL;
  - TILE_MINIMUM_HEIGHT, TILE_MAXIMUM_HEIGHT;
  - TILE_TRANSFORM (A2 showed the pinned Cesium ignores it);
  - TILE_REFINE.
  TILE_HORIZON_OCCLUSION_POINT and application semantics are opaque.
- **Modifiers on interpreted properties.** offset, scale, noData or default on an interpreted
  property, whether in the class or the property table.
- **Content semantics.** CONTENT_BOUNDING_* or CONTENT_*_HEIGHT semantics in `contentMetadata`.
- **Level limits.** `availableLevels > 32` or `subtreeLevels > 32`.
- **Repeated subtrees.** A subtree member resolved twice in one validation (OD-7). Producers never
  repeat them.
- **Mixed JSON and payload content.** A tile with both an available JSON content and any other
  available content.

### InvalidInput (malformed per pinned text; exit 3)

All rules in the Invariants section below. In addition:
- **Sphere roots.** A sphere root (`:85` "disallowed").
- **Implicit root shape.** An implicit root with `children`, with `metadata`, or with
  `content.boundingVolume` on a template (`:72-76`).
- **JSON at level 0.** An available JSON content at level 0 (`:75`).
- **Children under JSON content.** An available JSON content on a tile that has available implicit
  children or an available child subtree at that position (core `:735-737`).

## Invariants

### 1. Subtree container

Each subtree member is read through the validator's resolver:
- confined `uri()` (`validate.rs:158-210`) relative to the tileset JSON that holds the implicit
  root;
- `ArchiveResources::reference`;
- `read_member` with `member_bytes`.

Admission order:

1. **Header.** At least 24 bytes; magic `subt`; version `1`.
   - `jsonByteLength > 0`, and both `jsonByteLength` and `binaryByteLength` are multiples of 8.
   - `24 + json + bin == member length`, checked with u64 arithmetic.
2. **JSON chunk.** Parsed by C1 `json::parse` with `JsonLimits`, from `content_limits()`
   (`validate.rs:26-41`). Trailing padding must be `0x20` only; any other trailing byte is a JSON
   parse failure. The top level must be an object.
3. **Buffers.**
   - `buffers[i].byteLength ≥ 1`.
   - A buffer without `uri` must be index 0, and the BIN chunk must exist. In that case
     `byteLength ≤ bin` and `bin − byteLength < 8`, and the padding bytes are zero.
   - A buffer with `uri` must not be a data URI (InvalidInput). It resolves relative to the
     **subtree member**. Its member length must be `≥ byteLength`, and the first `byteLength`
     bytes are used. This follows the C1 glTF rule (`payload.rs:627-633`).
   - A non-empty BIN chunk with no internal buffer is InvalidInput.
4. **Buffer views.**
   - `buffer` must be an index into `buffers`.
   - `byteOffset` is **required** and must be a multiple of 8. Today it defaults to 0
     (`implicit/tileset.rs:912-916`).
   - `byteLength ≥ 1`.
   - `byteOffset + byteLength ≤ buffer.byteLength`, using checked arithmetic.
   - Every view is validated, including views that nothing references.
5. **Availability objects**, with the semantics in §2.
6. **Property tables**, with the semantics in §5.

Numbers that must be integers must be non-negative JSON integers that fit in u64; anything else is
InvalidInput.

An `extensions` object anywhere in the subtree JSON is Unsupported. Unknown non-extension
properties are ignored.

### 2. Availability

Notation:
- `N` is 4 for QUADTREE and 8 for OCTREE.
- `L` is `subtreeLevels`.
- The tile and content bit count is `T = (N^L − 1)/(N − 1)`.
- The child-subtree bit count is `C = N^L`.

Both counts are computed in u128. `L ≤ 32` keeps `C ≤ 2^96`.

- **Encoding.**
  - Exactly one of `bitstream` and `constant` is present.
  - `constant` must be 0 or 1.
  - `tileAvailability.constant == 0` is InvalidInput.
- **Bitstream length.** A bitstream view has `byteLength == ⌈bits/8⌉`. Bits beyond the count must
  be 0.
  - Bit `i` is `(byte[i/8] >> (i%8)) & 1` (Metadata "Booleans").
  - MSB-first reading is a sensitive control: every producer witness with a bitstream reads
    differently under MSB-first (probe field `anyMsbFirstWouldDiffer`).
- **availableCount.** When present it must equal the popcount (for a constant: `0` or the
  count). Checked with no allocation, in O(view bytes).
- **Root tile.** Tile bit 0 must be 1.
- **Parents.** For each level `l ≥ 1` and Morton index `m`, `tile[off(l)+m] ⇒ tile[off(l−1)+⌊m/N⌋]`,
  where `off(l) = (N^l − 1)/(N − 1)`.
- **Content layers.**
  - The number of `contentAvailability` layers equals the number of templates.
  - The array is absent when the root has no content.
  - `content[k][i] ⇒ tile[i]` for every layer `k`.
- **Child-subtree bits.** `child[m] ⇒ tile[off(L−1)+⌊m/N⌋]`.
- **Available levels.** Let `A` be `availableLevels`.
  - An available tile at global level `≥ A` is InvalidInput.
  - A set child-subtree bit whose child root would be at global level `≥ A` is InvalidInput.
  - Declaring `A` larger than the deepest available level is admitted; producers are tight.
- **Child subtree roots.** For each set child bit, the child subtree must resolve and be admitted,
  and its own tile bit 0 is already required. A missing member is InvalidInput
  (`missing archive entry`).
- **Unset child bits.** Nothing is read. An orphan member falls to the existing unreferenced-entry
  check, `validate.rs` "unreferenced archive entries" (InvalidInput).

### 3. Addressing and traversal

- **Morton order.** Child index `c` in `[0, N)` sets `x = 2x + (c&1)`, `y = 2y + ((c>>1)&1)`,
  and, for octrees only, `z = 2z + ((c>>2)&1)`. Within a level, Morton order interleaves with `x`
  as the least significant bit (`AVAILABILITY.adoc`).
- **Coordinates.** Global coordinates are `level < 32` and `x, y, z < 2^level`. Local coordinates
  inside a subtree are `global − (subtreeRoot << localLevel)`.
- **Templates.**
  - Substitution covers `{level}`, `{x}`, `{y}` and, for octrees, `{z}`, in decimal.
  - A template lacking any required variable is InvalidInput (`:161` "shall include").
  - `{z}` in a QUADTREE template is InvalidInput.
  - Any other `{` or `}` left after substitution is InvalidInput.
  - Substitution happens before percent-decoding. Resolution uses the existing `uri()`, relative to
    the tileset JSON that contains the implicit root (`:162`). Subtree buffers resolve relative to
    the subtree.
  - Scheme, query, fragment and absolute paths keep today's `uri()` mapping to Unsupported.
- **Traversal order.** Depth-first pre-order: a tile, then its available contents in slot order,
  then its available children in ascending child index. An available JSON content is expanded in
  place, as C2 v3.3 requires (C2 amendments, "Traversal order"). This order fixes C2 reference
  order and first-failure order.
- **Revisits and cycles.**
  - No tile coordinate is visited twice within a root; this follows from the parent rule.
  - A subtree member resolved twice anywhere in one validation is Unsupported (OD-7).
  - An external-tileset cycle through implicit JSON content is InvalidInput through the
    validator's existing active set (`validate.rs:1122-1124`).
  - Repeated *payload* members remain admitted, as for explicit tiles, through the URI cache.

### 4. Effective volume and error

**Implicit root.** The tile `R` that carries `implicitTiling` keeps its JSON `boundingVolume`
`V_R`, its `transform` and its `geometricError` `e_R`.

**Level-0 tile.** The level-0 implicit tile `I_0` is the same tile for counting. Its effective
volume `V_0` is the TILE_BOUNDING_BOX row of `I_0` if present, otherwise `V_R`.

**Two volumes.** `V_R` and `V_0` are both volumes of that tile.
- Content and descendants must lie in both; Cesium tests the placeholder first.
- `hierarchyBounds` checks `V_0 ⊆ V_R` with the existing `contains()`. This replaces the
  expander's separate 1e-6 "semantic root inside placeholder" check (`implicit/tileset.rs:759-772`)
  with the same tolerance and a different message.
- For C2 slice 3 they are two `LevelFacts` levels joined by `to_parent: None`. This uses the
  existing interface (`containment.rs:47-53`); see "Interface supplying C2 level chains".

**Deeper tiles.** For a tile at level `l` with coordinates `(x, y, z)`:

- **Effective volume.** The TILE_BOUNDING_BOX row (`Row(f64×12)`) if the tile has one. Otherwise
  the **derived cell**, represented symbolically and exactly as
  `Derived(Cell { root: V_R.box, level: l, coords, scheme })`.
  - Its meaning is exact rational arithmetic. Centre lane `j`:
    `c_j + Σ_k a_kj·((2·i_k + 1)/2^l − 1)`. Half-axis `k`: `a_k / 2^l`. Here `i = (x, y, z)`,
    and only axes 0 and 1 subdivide for QUADTREE.
  - No rounded `f64` centre is produced or compared as truth. The informative "min + size·i"
    formula (`:123-124`) is the same rational.
  - At level 0, `Derived` equals `V_R` exactly, and the owner reports it as `Row(V_R.box)`.
- **Effective error.** The TILE_GEOMETRIC_ERROR row if present. Otherwise `e_R · 2^−l`.
  - This is exact in binary64 when `e_R = 0` or `e_R·2^−l` is normal.
  - If `e_R·2^−l` would be subnormal and nonzero, the result is Unsupported ("derived geometric
    error underflows binary64"). No producer approaches this.
- **Refinement.** `refine` is constant: the implicit root's own `refine`, or the value inherited
  from its parent. The expander's per-descendant `"REPLACE"` (`implicit/tileset.rs:1135`) is a
  defect and is not ported.
- **Transforms.** Implicit tiles at level ≥ 0 carry no transform; only `R` does. TILE_TRANSFORM
  is Unsupported.
  - An external tileset reached through JSON content contributes its own root `transform`, applied
    once in the chain, as for explicit external tilesets (C2 I1/I2).
  - Implicit templates cannot carry `content.boundingVolume`, so there is no external content
    volume (`Link.external = None`).
- **Row values.** Twelve finite values, and an error that is finite and `≥ 0`; otherwise
  InvalidInput.
  - FLOAT32 rows widen exactly to `f64`.
  - Rows are never rounded, padded or compared with a tolerance by I1.

The legacy `hierarchyBounds` check needs an `f64` box for a `Derived` cell (OD-3). If it is given
one, that box is labelled as a *rounded evaluation for the tolerance-based legacy check only* and
never enters C2 facts.

### 5. Metadata rows

The schema is the tileset JSON's inline `schema`, or the `schemaUri` member, of the document that
holds `R`. `tileMetadata` indexes `propertyTables`; the table's `class` must exist in the schema
(InvalidInput otherwise).

**Interpreted properties.** These are the class properties whose `semantic` is TILE_BOUNDING_BOX
or TILE_GEOMETRIC_ERROR, matched **by semantic**. The class name and the property names are not
used: today `rustyTile`, `boundingBox`, `geometricError` and `extras` are required
(`implicit/tileset.rs:718-735`, `:1005`). That gating made the executed #151 fixtures InvalidInput.
- A semantic used twice in one class is InvalidInput.
- **TILE_BOUNDING_BOX** must be `type SCALAR`, `componentType FLOAT32|FLOAT64`, `array true`,
  `count 12`, and not normalized.
- **TILE_GEOMETRIC_ERROR** must be `type SCALAR`, `componentType FLOAT32|FLOAT64`, and not an
  array.
- Any other shape for these two semantics is InvalidInput (Semantics table).
- offset, scale, noData or default is Unsupported. Today they are ignored silently
  (`implicit/tileset.rs:723-731` checks only type, componentType, array, count and semantic). That
  is a static wrong-meaning defect, not executed.

**Rows.**
- `table.count` must equal the tile popcount (`:217` "Each available tile shall have a value").
- The row of a tile at bit index `i` is `rank(i) = popcount(tile[0..i))`.
- The column's `values` view must have `byteLength == count · 12 · size` (or `count · size` for
  the error) exactly; anything else is InvalidInput.
- Views must be aligned to the component size, which is already implied by the 8-byte rule.
- Values are read little-endian in place.

**Omitted columns.** A `required` interpreted property missing from the table is InvalidInput
(propertyTable schema). An optional one is simply not an override.

**Other columns.** A non-interpreted column's `values`, `arrayOffsets` and `stringOffsets` must
index existing validated views. Their contents are opaque. The producers' `extras` STRING column
is not parsed: today it is parsed as JSON (`:1048`).

**Content and subtree metadata.**
- `contentMetadata`, when present, has one entry per template, and table `count` equals that
  layer's popcount. A layer with no available content may have no table.
- CONTENT_BOUNDING_* or CONTENT_*_HEIGHT semantics are Unsupported.
- `subtreeMetadata` is opaque.

### 6. Profile exclusions (decided)

These are listed under Supported domain. Each one has a single-defect control (see Acceptance)
and a stable message prefix: `implicit: unsupported <reason>`.

### 7. Bounded work (caps before the allocations they guard)

There are **no new `ValidationLimits` fields** (OD-5). Every I1 bound uses an existing field,
selected once in `inspect_selected`.

| Quantity | Bound | Checked before | Outcome when exceeded |
| --- | --- | --- | --- |
| Implicit tile visits | `hierarchy_visits` (65,536), shared with explicit tasks. At subtree admission the subtree's tile popcount is **reserved** (charged at once to `task_visits`); visiting those tiles does not charge again | the rank table and before any tile of that subtree is visited | ResourceLimit |
| Hierarchy depth | `hierarchy_depth` (128). Depth of an implicit tile is depth(`R`) + level | visiting the tile | ResourceLimit (existing) |
| Subtree member bytes | `member_bytes` per member; `total_bytes_read` overall; plus **per implicit root** `Σ subtree member bytes ≤ document_decoded_bytes` (64 MiB). The last mirrors today's per-expansion cache cap (`implicit/tileset.rs:1099-1103`); it counts cumulative reads, with no eviction credit, so it is deterministic | reading the member (size comes from the archive index) | ResourceLimit |
| External buffer bytes | Same as subtree members, charged to the same per-root sum | read | ResourceLimit |
| Subtree JSON | C1 `JsonLimits`: `json_bytes`, `json_depth`, `document_items` | parse | as C1 (ResourceLimit or Unsupported, per C1 contract) |
| Content and subtree references | `references` (262,144) through `ArchiveResources::reference` | registration | ResourceLimit (existing) |
| Rank table | `4·⌈T/64⌉` B for bitstream tile availability only (none for constant). It is at most `member_bytes/2`, allocated with `try_reserve_exact` | allocation | ResourceLimit (host refusal) |
| Levels | `availableLevels ≤ 32`, `subtreeLevels ≤ 32` (profile constants) | root admission | **Unsupported**; not a limit |

What this table means for failures:

- **New ResourceLimit sources.** I1 adds only two:
  - the pre-allocation reservation and byte checks above, which are applications of existing
    limits;
  - fallible allocation refusal.
- **Never a verdict.** A ResourceLimit is never a verdict about the archive.
- **No allocation scales with declared counts.** `T`, `C`, `availableCount` and `table.count` are
  declared counts. They size nothing until popcount ≤ the remaining reservation, or a view length
  has bounded them.
- **Constant availability.** For a constant tile availability of 1 with large `L`, the
  reservation fails before any allocation. Resource control R3 requires this to be measured.

**Time.** The time is O(subtree bytes read + tiles visited + JSON items + Σ template length per
content). The popcount, availableCount and trailing-bit passes are O(view bytes). There is no pass
over `C` for constant child availability: popcount is `C` or 0, and enumeration visits only set
bits. A bitstream with `C` bits has `⌈C/8⌉` bytes, already bounded by `member_bytes`.

**Memory held.**
- Subtrees are retained per open implicit root until the root's traversal ends, within the per-root
  byte cap.
- There is no serialized presentation tree. Today's `serde_json` expansion, budgeted at 64 MiB of
  serialized nodes (`implicit/tileset.rs:1160-1168`), disappears.
- This is not an RSS claim.

**InvalidState.** A breach of an internal invariant is `ValidationFailure::InvalidState`
(`invalid_state`, exit 1) and never a verdict. Examples: a rank at or above `count` after the
count check passed; a visit beyond the reservation; a Morton decode mismatch.

## Failure semantics and precedence

| Kind | Meaning in I1 | Exit |
| --- | --- | --- |
| InvalidInput | Malformed per the pinned text or the invariants above | 3 |
| Unsupported | Valid but outside the profile in "Supported domain" | 2 |
| ResourceLimit | An existing limit checked before the allocation or read it guards, or host allocation refusal. Never a verdict | 3 |
| InvalidState | Internal invariant breach | 1 |
| Io | Archive I/O through the existing `archive_admission_error` and `read_member` (unchanged) | 1 |

**Precedence.** The first failure in traversal order wins, matching C1 and C2.
- Root admission comes before any subtree read: `implicitTiling` fields, volume kind, templates,
  schema semantics and levels.
- Within a subtree, the order is §1 items 1–6, then the reservation, then the metadata rows.
- Rules within the same step are not ordered normatively. Controls are single-defect, so category
  parity does not depend on that order. Three double-defect controls pin the step order (A-12).
- The owner never classifies by message string.

## Interface supplying C2 level chains

I1 provides typed facts; it does not import `validate::containment`.

The owner lives at `src/content_integrity/implicit.rs` (plus submodules if size warrants). It is
crate-private and has no `serde` derive. It consumes JSON values admitted by C1. It knows no
`ValidationLimits`, `Archive` or report types.

```text
pub(crate) enum Scheme { Quadtree, Octree }
pub(crate) struct ImplicitLimits { visits_remaining: u64, root_subtree_bytes: u64, json: JsonLimits }
pub(crate) struct ImplicitRoot { /* admitted implicitTiling, V_R, e_R, refine, templates (uri + header), semantics */ }
pub(crate) fn admit_root(tile: &Value, schema: Option<&Value>) -> Result<ImplicitRoot, ImplicitError<Never>>
pub(crate) struct TileAddr { level: u32, x: u32, y: u32, z: u32 }
pub(crate) enum EffectiveVolume { Row([f64; 12]), Derived(Cell) }
pub(crate) struct Cell { root: [f64; 12], level: u32, coords: [u32; 3], scheme: Scheme }
pub(crate) enum EffectiveError { Row(f64), Derived(f64) }   // Derived = e_R·2^-l, exact by §4
pub(crate) struct VisitedTile { addr: TileAddr, volume: EffectiveVolume, error: EffectiveError,
                                contents: SmallVec<(slot: u16, uri: String /* substituted, unresolved */)>,
                                children: ArrayVec<TileAddr, 8> /* ascending child index */ }
impl ImplicitRoot {
  pub(crate) fn visit<E>(&mut self, addr: TileAddr, limits: &mut ImplicitLimits,
        read: &mut dyn FnMut(&str /*relative uri*/, Base /*Tileset|Subtree(name)*/, u64 /*max*/) -> Result<Vec<u8>, E>)
        -> Result<VisitedTile, ImplicitError<E>>;   // lazily admits the containing subtree
  pub(crate) fn header(&self, slot: u16) -> &Value;     // content header for C2 exclusions and extension checks
}
pub(crate) enum ImplicitError<E> { Format(FormatError), Resolver(E), InvalidState(&'static str) }
```

The validator (author B) maps these facts as follows. `containment.rs` is untouched.

**The tile `R`.** The existing `TileFact { transform: R.transform, volume: V_R, external: link
from referencing content, parent: explicit parent, depth }` (`validate.rs:518-529`).

**`I_0`.** `TileFact { transform: None, volume: VolumeFacts::Box(V_0), external: None, parent:
R, depth: depth(R) }`.
- `build_levels` (`validate.rs:589-614`) then yields `[own V_0] → (to_parent None) [ancestor
  V_R] → …`.
- Nothing is inverted, and no step exists between the two levels.

**A tile at level ≥ 1.** `TileFact { transform: None, volume, external: None, parent: tile of
parent addr }`.
- `Row(b)` maps to `VolumeFacts::Box(b)`, exactly.
- `Derived(cell)` has **no `VolumeFacts`**. In the I1 PR, implicit archives keep the C2 exclusion
  `implicitTiling`, so no C2 reference is registered for implicit tiles (B must skip
  `c2.reference` for them).
- C2 slice 3 decides how to consume `Cell`: either an exact predicate (its own contract delta) or
  exclusion with a new reason (a frozen-enum delta). Producers never need it: every available
  producer tile has a row (`noRow = 0` in every probe).

**JSON content on a tile `t`.** It becomes `ValidationTask::Tileset { link: Link { parent: t,
external: None } }`, unchanged from explicit handling (`validate.rs:1366-1378`).

**Payload content.** It goes through `Check::payload` and the C1 cache, unchanged. In I1 the
`content: Option<VolumeFacts>` it carries is `None`.

**Consequence.** C2 slice 3 can consume every producer archive with **no change to
`containment.rs`**. It only:
- stops setting `implicitTiling` (`validate.rs:1166-1168`);
- registers the implicit references.

The only open interface item is `Derived` cells for third-party tilesets without rows.

## Validator migration

Author B makes these changes after the owner exists. The C2 merge has already happened.

1. **`prepare_tileset`.** It no longer expands (`validate.rs:1160`). The vector
   `rustyTilesSourceContents` handling (`:1206-1217`) moves into `prepare_tileset` unchanged.
2. **`node()` with `implicitTiling` on any tile.**
   - The nested-root refusal (`:1280-1284`) is removed.
   - Explicit-tile checks run on `R`: volume, transform, refine, metadata exclusion, extension
     objects, and `hierarchyBounds` or `geometricErrorOrder` against its parent.
   - `admit_root` is called. The content templates are **not** treated as references.
   - `R`'s template headers go through `extension_object` once each.
   - The C2 exclusion `ImplicitTiling` is set here, for any tile. Today it is set for document
     roots only.
   - `ValidationTask::Implicit { root: usize, addr: (0,0,0,0), parent_tile, parent_volume:
     V_R, parent_error: e_R, depth }` is pushed.
3. **`ValidationTask::Implicit`.**
   - It charges no new visit, because the reservation covers it.
   - It calls `visit()`.
   - It applies `hierarchyBounds` (`contains(parent, V)` with today's tolerance, OD-3 for
     `Derived`) and `geometricErrorOrder`.
   - It registers C1 payloads and JSON external tilesets per content.
   - It checks per-tile recorded budgets: actual bytes, vertices and `maxPoints` are unchanged,
     and extras follow OD-4.
   - It pushes children in reverse so that they pop in ascending order.
   - `Check` holds `roots: Vec<ImplicitRoot>`. A root is dropped when its last task completes; this
     drop is an implementation detail and is not observable.
4. **Removals.** `expansion_budget` (`:898`, `:1600-1604`) and `implicit_error` (`:56-62`, test
   `:1758-1761`) are removed from the validator. `implicit::expand_tileset_bounded` and
   `ExpansionBudget` (`implicit/tileset.rs:619-660`) lose their only caller and are removed. That
   also removes `implicit → validate::ValidationFailure` (`:656-660`).
   `implicit::expand_tileset` stays for `vector/pipeline/store.rs:1006` and the legacy tests.
5. **Counters.**
   - `tiles` counts explicit JSON tiles plus available implicit tiles, with `R` and `I_0` counted
     once.
   - A JSON-linked external root counts as its own tile, as for explicit external tilesets.
   - `content_references` counts every available content, including JSON.

### Exactly what the report says

For an archive with at least one implicit root that I1 read successfully:

- **`checks`** gains `"implicitAddressingAndAvailability"`, inserted after `"hierarchyBounds"`
  in the fixed list (`validate.rs:1716-1730`). The rest of the list is unchanged.
  `decodedContentContainment` is still absent, because implicit is still a C2 exclusion.
- **`notInspected`:**
  - loses `"implicitAddressingAndAvailability"`;
  - keeps `"decodedContentBounds"`, `"decodedContentBounds.implicitTiling"`,
    `"metadataSemantics"` and the others;
  - gains `"implicitExternalContentTemplates"` when any implicit root has a JSON template, pending
    OD-1. This states that the terminal-occupancy reading was applied without a standards
    verdict;
  - gains `"recordedBudgets.implicitTileExtras"` if OD-4 chooses to drop the per-tile extras
    comparison.
- **`tiles` and `contentReferences`** change for chunked outputs (executed today versus expected):

  | Archive | `tiles` | `contentReferences` |
  | --- | --- | --- |
  | `pc-deep.3tz` | 2233 → 2440 | 2233 → 2440 |
  | `vec-perf.3tz` | 547 → 631 | 544 → 628 |

  Unchunked outputs are unchanged: `pointcloud-implicit` stays at 73/73 and `pc-perf-2000` at
  428/428. The new counts equal the probe's `availableTiles` and `contentTiles`.
- **Explicit-only archives** have neither label in `checks` or `notInspected`. That matches
  today, where the label appears only for implicit archives (`:1218-1220`).
- **Rust, CLI and Python** serialize the same `ValidationReport`; the struct does not change.
  Python parity is tested in `bindings/python/tests/test_api.py`.

### Exit-status changes versus today

The list is complete for the I1 PR. **E** means executed in this preparation; **S** means static.

| Change | Archives | Basis |
| --- | --- | --- |
| InvalidInput → success | Valid semantic-by-`semantic` tilesets whose class or property names are not `rustyTile`, `boundingBox`, `geometricError` or `extras`; FLOAT32 semantic rows | E: #151 fixtures `a2fix/*` |
| InvalidInput → read | Implicit roots on non-root explicit tiles | S: `validate.rs:1280-1284` |
| InvalidInput → core semantics | JSON-linked external roots whose box, error or transform differs, or whose asset is not 1.1. The expander required equality (`implicit/tileset.rs:810-819`). `geometricErrorOrder` and `hierarchyBounds` now decide | S. The `native_implicit.rs:23` "link" case still fails, through `geometricErrorOrder` |
| InvalidInput → success | Malformed JSON in an opaque `extras` STRING column | S: `:1048` |
| InvalidInput → read | External subtree buffers | S: `:895-897`, pending OD-6 |
| success → InvalidInput | Missing `bufferView.byteOffset` | S: `:912-916` |
| success → InvalidInput | Templates lacking `{x}`, `{y}` or `{z}` | S: `:838-851` |
| success → InvalidInput | JSON content available at level 0 | S |
| success → InvalidInput | `contentMetadata` count mismatch | S: ignored today |
| success → Unsupported | Repeated subtree member (OD-7); JSON and payload content on one tile; unsupported TILE_* or CONTENT_* semantics; offset, scale, noData or default on interpreted properties (today misread silently) | S |
| InvalidInput → Unsupported | Region or S2 implicit roots; `availableLevels > 32` | S: `:684`, `:690`, `:710` |
| ResourceLimit → success | Sparse subtrees with `N^L > 65,536` but few available tiles, for example OCTREE `subtreeLevels` 6 | S: `:695-699` |
| (Not a change) | `3DTILES_implicit_tiling` (1.0) | It becomes Unsupported. Today it is not read, and its members fall to the unreferenced-entry check (S, inferred) |
| Report | `checks` and `notInspected` as above; `tiles` and `contentReferences` for chunked outputs | E: counts |

There are no public Rust API changes: no new `ValidationLimits` fields and no new variants.

## Disposition of existing code

| Component (`f2ed077`) | Decision | Basis |
| --- | --- | --- |
| `validate.rs:1203-1263` `expand_implicit` | **Replace** with the I1 owner and `ValidationTask::Implicit` | The production expander is not an independent truth (#125); presentation tree |
| `validate.rs:1206-1217` retained vector sources | **Retain**, moved | Not implicit semantics |
| `validate.rs:1166-1168` root-only `ImplicitTiling` exclusion | **Rework**: set it for any tile with `implicitTiling` | I1 admits roots anywhere |
| `validate.rs:1280-1284` nested implicit refusal | **Remove** | Spec `:28` "may be added to any tile" |
| `validate.rs:898`, `:1600-1604` `expansion_budget`; `:56-62` `implicit_error` | **Remove** | Replaced by the reservation and per-root byte cap |
| `validate.rs:1265-1436` `node()` | **Rework**: implicit-root branch; tile checks shared by JSON and implicit tiles | — |
| `validate.rs:1408-1415` per-tile recorded extras | **Decide (OD-4)** | Reads the opaque `extras` |
| `validate.rs:2550-2556` test | **Rework**: also assert the `checks` label | — |
| `validate.rs:197-338` `transform`, `inside`, `contains`, `scale_bound` | **Retain** for `hierarchyBounds` only (D4 retires) | Tolerance-based; not I1 truth |
| `validate/containment*` | **Untouched** | The interface fits `LevelFacts`/`TileFact` |
| `implicit/tileset.rs:619-660` `expand_tileset_bounded`, `ExpansionBudget`, `expansion_limit` | **Remove** after migration | Only caller is the validator |
| `implicit/tileset.rs:606-617`, `:662-1196` `expand_tileset` and the reader | **Retain** for `store.rs:1006` and the tests and public export (Inv:100) only, **not as truth**. Removal belongs to export narrowing (G6) | Defects found are not ported: class-name gating `:718-735` and `:1005` (E); descendant `REPLACE` `:1135` (S); `byteOffset` default `:912-916`; no required-variable check `:838-851`; link equality `:810-819`; prefix-based linking `:801-804`; extras parsed as JSON `:1048`; single internal buffer `:895-897`; f64 `derived_box` `:403-418` and `:1133` |
| `implicit.rs:68-332` `Subtree` writer; `implicit/tileset.rs:229-602` writers | **Untouched** (producers, A2) | Out of scope; the oracle checks their output |
| `content_integrity.rs` | **One line**: `pub(crate) mod implicit;` | Owner placement |
| `content_integrity/json.rs`, `numbers.rs`, `payload*` | **Retain, untouched** | Consumed through the existing API |
| `tests/p1_point_oracle.py:279-287` Morton/rank | **Reference only** for O | It merges links and keys rows by property name (`:305-306`, `:331-339`), so it is producer-shaped, not a truth source |
| `tests/native_implicit.rs:23-122` | **Retain**; R re-verifies each case | "root" fails by `hierarchyBounds` (`V_0 ⊄ V_R`); "link" fails by `geometricErrorOrder`; header, availability, metadata and missing still fail |
| `docs/VALIDATION.md:143` | **Rewrite** (R) | It currently describes the presentation budget |
| Inventory `:40`, `:100` | **Update** (R) | Validate row; `implicit` keeps the expander export only for non-validator callers |

## Independent acceptance plan

### A. Oracle O-implicit (separate author; never reads `src/`)

- **Implementation.** Pure Python 3 standard library: `zipfile`, `json`, `struct`, `fractions`. No
  repository imports.
- **Sources.** Written from the pinned primary text (the five retained and twelve fetched files,
  re-pinned under `bench/architecture_audit/implicit_read/primary/`) and from this contract.
  - O may read `tests/p1_point_oracle.py:279-287` for Morton bit layout only.
  - O may read the #151 fixture *data*.
- **Self-test.**
  - the `AVAILABILITY.adoc` interleave examples, bit-exact;
  - exhaustive Morton round trips for level ≤ 4;
  - a derived-cell identity: the children's union equals the parent along each axis, in `Fraction`.
- **Output per archive.** One canonical record:
  - `verdict`: Accepted, InvalidInput, Unsupported or ResourceLimit (with the limit name);
  - for Accepted, a canonical tile list in traversal order. Each entry has `doc`, `level`,
    `x`, `y`, `z`, `depth`, `parentIndex`, `volume` (`row:` 12 f64 hex bit patterns, or
    `derived:` root hex plus level and coordinates), `error` (hex), `refine`, `contents` (slot and
    resolved member name) and `kind` (payload or JSON);
  - its SHA-256.

  Production exposes the same list through a `#[cfg(test)] implicit_tile_facts(request, limits)`
  seam in `validate.rs`, following the C2 `containment_work_facts` pattern (`validate.rs:1494-1527`).
  Parity means equal hashes.

### B. Witnesses (R regenerates on the final source; hashes handed to O)

| ID | Producer and command | Shape (probed on f2238cd) |
| --- | --- | --- |
| W1 | point-cloud default, `cloud.las --sourceCrs local --maxPoints 1000` | OCTREE, 1 doc, 1 subtree, 73 tiles |
| W2 | point-cloud default, perf `points.las --maxPoints 2000` | 234 subtrees, 233 child bits, 428 tiles |
| W3 | point-cloud default, `cloud.las --maxPoints 40` | 208 docs, 207 JSON links, 2440 tiles, constant tile availability present |
| W4 | vector default ×6 (`vector.geojson` and countries: plain, `--quantize`, `--meshopt`) | QUADTREE, 1 tile |
| W5 | vector default, perf `vector.geojson --jobs 2` | 85 docs, nesting depth 3, 631 tiles, `3DTILES_content_gltf_vector` content header |
| W6 | legacy `mesh-to-3tz split.gltf --maxTriangles 512 --maxBytes 0` | 73 tiles; 72 rows outside derived cells |
| W7 | legacy mesh small (`legacy-mesh-small-implicit`) | refine **ADD**, `.gltf` content |
| W8 | `convert-to-implicit`, corpus single-tile; plus multi-node on the A2 real sources (`vector-flat-rounded.3tz` etc.) | external roots **with transforms**, JSON-only root template |
| W9 | #151 QUADTREE and OCTREE fixtures with the `schema.id` correction (`probes/materialize_a2_fixture.py`) | semantic on class `a2Tile` |
| W10 | Native and portable builds of W4 and W5 | Both build profiles |

The expected verdict is Accepted with O's facts. Production must equal O on every row. Any
disagreement needs a recorded disposition before merge.

### C. Sensitive data controls (O authors; base and mutant, both hashed; single defect each)

Each control flips the verdict in both production and O. "Facts" means the verdict stays Accepted
but the tile-fact hash must change identically in both.

| # | Control | Expected |
| --- | --- | --- |
| A-1 | One tile bit flipped 1→0 under an available child (one-bit) | InvalidInput (parent rule) |
| A-2 | One tile bit flipped 0→1 at a leaf whose parent is available | Facts (an extra tile; rows misaligned: InvalidInput by count) |
| A-3 | A bitstream rewritten MSB-first | InvalidInput or facts differ (W1, W5) |
| A-4 | `availableCount` ±1 | InvalidInput |
| A-5 | Trailing padding bit set (one-bit) | InvalidInput |
| A-6 | Content bit without its tile bit | InvalidInput |
| A-7 | Child-subtree bit under an unavailable tile | InvalidInput |
| A-8 | Child-subtree bit set beyond `availableLevels` | InvalidInput |
| A-9 | `availableLevels` 32 versus 33 | Accepted versus Unsupported |
| A-10 | Missing child subtree member | InvalidInput |
| A-11 | Repeated subtree member, through a template collision | Unsupported |
| A-12 | Three double-defect controls pinning step order | Specified category |
| B-1 | Two TILE_BOUNDING_BOX rows swapped | Facts |
| B-2 | One row component ±1 ulp (one-ulp) | Facts; exact hex in both |
| B-3 | Interpreted property renamed, and class renamed, semantic unchanged | Facts unchanged |
| B-4 | Semantic removed from the class | Facts change to `derived` |
| B-5 | Semantic duplicated in the class | InvalidInput |
| B-6 | FLOAT32 rows | Accepted; facts equal the widened values |
| B-7 | `scale` added on TILE_BOUNDING_BOX | Unsupported |
| B-8 | `table.count` ±1 | InvalidInput |
| B-9 | TILE_GEOMETRIC_ERROR row −1 or NaN | InvalidInput |
| B-10 | Derived error at subnormal | Unsupported |
| C-1 | Morton x and y swapped in the template coordinates (rename members) | InvalidInput (missing member) or facts |
| C-2 | Template without `{y}` | InvalidInput |
| C-3 | `{z}` in a QUADTREE template | InvalidInput |
| C-4 | JSON slot available at level 0 | InvalidInput |
| C-5 | JSON tile with an available child | InvalidInput |
| C-6 | JSON and payload available on one tile | Unsupported |
| C-7 | External cycle (link → ancestor document) | InvalidInput |
| C-8 | Implicit root on a non-root explicit tile | Accepted |
| C-9 | Region root and sphere root | Unsupported and InvalidInput |
| D-1 | Truncated BIN (−8 bytes) | InvalidInput |
| D-2 | `byteOffset` +1, and absent | InvalidInput |
| D-3 | Non-zero BIN padding | InvalidInput |
| D-4 | Bad magic or version | InvalidInput |
| D-5 | External buffer that is a data URI | InvalidInput |
| D-6 | External buffer that is valid | Accepted (OD-6) |
| D-7 | JSON-format subtree | Unsupported |

### D. Hashed implementation mutants (R; one patch against the final production source and a separate patch against O)

Each mutant must flip its paired control in its own implementation. A mutant that flips nothing is
a failed control.

| Mutant | Paired control |
| --- | --- |
| MSB-first bit order | A-3 |
| Inclusive rank (off by one) | B-1, W1 facts |
| Drop the content ⇒ tile check | A-6 |
| Drop the child-under-unavailable check | A-7 |
| Drop the `availableCount` check | A-4 |
| Key semantics by class or property name | B-3 |
| Ignore the override and always derive | B-4 and W6 facts |
| Swap x and y in Morton | C-1 and W5 facts |
| Error not halved per level | B-4 facts |
| `≥` versus `>` on `availableLevels` | A-8, A-9 |
| Accept FLOAT32 as FLOAT64 stride | B-6 |
| Ignore trailing bits | A-5 |
| Accept JSON at level 0 | C-4 |
| Read `scale` silently | B-7 |
| Drop the repeated-subtree check | A-11 |
| Reserve visits after allocation, or not at all | R2, R3 |
| Refine forced to `REPLACE` | W7 facts |

### E. Resource controls (R, through `inspect_selected` with explicit `ValidationLimits`)

- **R1.** `hierarchy_visits` equal to W2's exact visit count gives Accepted; one less gives
  ResourceLimit. Repeat with W3, which includes JSON tasks.
- **R2.** The per-root `document_decoded_bytes` set to the exact subtree byte sum of a W2 root
  gives Accepted; one less gives ResourceLimit, raised before the member read.
- **R3.** OCTREE `subtreeLevels` 8 with constant tile availability 1 (2,396,745 tiles) gives
  ResourceLimit. Peak RSS under `/usr/bin/time -v` must stay within 16 MiB of the W1 validation
  baseline, which proves the refusal happens before allocation. The same holds for
  `subtreeLevels` 32 with constants.
- **R4.** `hierarchy_depth` at exact depth gives Accepted; one less gives ResourceLimit.
- **R5.** An injected rank-table allocation refusal through a test seam gives ResourceLimit, and
  never a verdict.
- **R6.** Bitstream `C` at the `member_bytes` boundary.

### F. Adapters, CI and review

- Rust, CLI and Python report parity on W1, W3 and W5 (`checks`, `notInspected`, counts).
- Installed wheel; official Blender as applicable to validate.
- Exact-head CI.
- A fresh nonauthor final-source review (see the ownership plan).

## Performance obligation (standing perf-lane rule)

The lane is `/tmp/rusty-tiles-perf/bench/perf_lane/`, README "Gate". It is **not yet merged to
develop** (`git status`: untracked), so I1 pins the scripts by hash:
- `process.py` `513bec79…`
- `corpus.py` `a6cd738c…`
- `README.md` `25b94921…`

If the lane merges first, I1 uses the merged copy instead.

**The rule.** A case is flagged when head/base exceeds 1.25 **and** the absolute growth exceeds the
floor (0.1 s best wall time, or 16 MiB peak RSS). It is also flagged when head fails a case that
base passes. There are no `--explain` waivers for I1's validate cases.

**Before** = develop `f2ed077`, release build. **After** = the I1 head. Both run serially on the
coordinator's two pinned CPUs, `--repeat 3`.

1. **Standing cases.** `validate:vector` and `validate:point-cloud` from the lane corpus.
   Recorded baseline (`results/baseline-process.json`): 0.166 s and 0.058 s best wall. With the
   0.1 s floor these can trip only above about 0.27 s and 0.16 s.
2. **I1 cases**, added through the lane's corpus mechanism or recorded with the same
   `process.py` metrics:
   - `validate:pc-deep` (W3): today about 0.22 s;
   - `validate:pc-perf-2000` (W2): today about 0.10 s;
   - `validate:vec-perf` (W5): today about 0.17 s.

   These are rough single-host numbers (CPUs 12–13, best of 3) recorded in `probes/receipt.json`.
   They are not lane measurements.
3. **Amplified case.** A point cloud near 60,000 tiles, so that the reader dominates and the
   floor cannot mask a regression: perf `points.las --maxPoints` lowered until the tile count is
   between 50,000 and 65,000. R records the exact flag value and the tile count.
4. **Report and gate.** Report wall time, CPU time and peak RSS for all cases. A gated regression
   blocks merge. The expectation, unproven, is lower RSS, because the serialized presentation tree
   disappears.

C2 slice 3 will later add streaming cost to implicit archives. `validate:vector-explicit` grew
10× under C2 (`results/c2-process.json`). That cost is slice 3's to measure, not I1's.

## Proof limits and remaining gates

The following are not provided:
- decoded containment for implicit tiles (C2 slice 3, which also needs the `Derived` cell
  disposition);
- standards conformance of JSON templates on implicit roots (OD-1; the A2 gate);
- regions and S2 (#120);
- JSON subtrees;
- interpretation of TILE_* semantics other than the two;
- content metadata;
- viewer selection;
- whole-process memory.

Not removed:
- `implicit::expand_tileset` (vector self-read, tests, public export);
- producers;
- `convert_implicit.rs`.

Parents #113, #120, #125 and #126 stay open. There is no main merge, tag or publication.
