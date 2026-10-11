# I1: independent implicit effective-volume read owner (contract draft v2)

Status: **draft for a fresh nonauthor precode review**. Nothing is implemented. Owners:
[#125](https://github.com/BenDyson-Arch/rusty-tiles/issues/125) (implicit traversal), with
#126/A2 and C2 slice 3 as consumers. Base: `origin/develop` `f2ed077`. The tree is identical to
`/tmp/rusty-tiles-c2` `3563d53` and includes C2 (PR #157). The scope comes from
`/tmp/next-slice-selection-20261011/recommendation.md` §2. Every line reference is to `f2ed077`.

Evidence for this draft:
- [producer-inventory.md](producer-inventory.md), [open-decisions.md](open-decisions.md) and
  [ownership-plan.md](ownership-plan.md);
- `probes/receipt.json`, which pins the binaries, inputs, samples, scripts and primary texts;
- the raw probe outputs in `probes/results/*.json`.

The probes are fact-finding by the contract author. They are not the acceptance oracle. v1 is
preserved in `v1/`.

The draft claims nothing about:
- decoded content containment (C2 slice 3);
- regions or S2;
- viewer selection;
- metadata semantics beyond the two interpreted semantics;
- memory or RSS;
- release readiness.

## Changes from v1

The user decided OD-1 (relayed by the coordinator): the producers change now, and I1 adopts the
strict reading of JSON content templates.

1. **The profile excludes JSON templates.**
   - An implicit root that has any content template able to name a JSON resource, in any slot,
     is **Unsupported**.
   - The one exception is a JSON slot that is available at level 0. The text directly prohibits
     that case, so it is **InvalidInput** (argued under "OD-1 resolution"; still open for the
     reviewer as OD-1b).
   - The v1 terminal-occupancy reading and the `implicitExternalContentTemplates` label are
     withdrawn.
2. **Consequence and merge order.** Today's default point and vector outputs at realistic size
   would validate as Unsupported, and so would the deep legacy `mesh-to-3tz`, multi-node
   `convert-to-implicit` and the #151 fixtures. The validator migration therefore merges after the
   producer layout change, or together with it. This is now a start gate, along with a recorded
   disposition for `convert-to-implicit`.
3. **Simplifications, because implicit tiles can no longer reach JSON content.**
   - The owner drives the whole depth-first traversal of one implicit root itself.
   - No external tileset is expanded inside an implicit tree.
   - The link-tile counting change, the JSON-tile rules and the in-place interleaving are removed.
4. **Bounded work.** v1 charged cumulative subtree bytes per root. This is replaced by **live-path
   subtree bytes**. Measured in the child-subtree layout: about 1,054 B per subtree, so a
   cumulative 64 MiB cap would bind near 63,700 subtrees, the same scale as the 65,536-visit cap
   (see §7). OD-5 is revised accordingly.
5. **OD-2 to OD-8 re-checked** (see open-decisions.md). OD-7 is narrowed. OD-2, OD-3, OD-4, OD-6
   and OD-8 are unchanged in substance.

## Corrections to the selection record (found while preparing; still valid)

1. **The A2 template gate is unavoidable for the reader.** At realistic size, both default
   producers write an external-JSON template in an implicit root's `contents`
   (`implicit-tileset-{prefix}-{level}-{x}-{y}[-{z}].json`, `implicit/tileset.rs:449-451`).
   - Executed observations:
     - `pc-deep.3tz` (point-cloud default, `--maxPoints 40`): 208 documents and 207 links.
     - `vec-perf.3tz` (vector default on the perf-lane input): 85 documents, external nesting to
       depth 3.
   - The reader therefore had to choose a reading. That choice is now settled as above.
2. **The producers also emit child subtrees.** `pc-perf-2000.3tz` has 234 subtrees, 233 child bits
   and no JSON. Which layout the writer uses depends on the data (`implicit/tileset.rs:284-308`).
   This form stays in the profile.
3. **C2's dyadic arithmetic cannot be reused without touching `src/validate/containment*`.**
   `dyadic.rs` items are `pub(super)` (`containment/dyadic.rs:10-200`), and `LevelFacts` carries
   `f64` volumes only (`containment.rs:42-53`). The derived box is therefore symbolic.
4. **The primary pins were incomplete.** `3d-tiles-core.adoc` only cross-references implicit tiling
   (`:837-877`). The missing texts were fetched at `4d781014` into `primary-fetch/` and hashed in
   the receipt.
5. **The #151 fixtures lack the required `schema.id`.** They are reusable only with the recorded
   correction (`probes/materialize_a2_fixture.py`).

## OD-1 resolution (user decision) and the one mandated InvalidInput

**The rule.** An implicit root is outside the I1 profile if any of its content templates (`content`
or any `contents[k]`) can name a JSON resource. "Can name" is decided statically, at root
admission:
- the template's path, after removing any `?` or `#` part, ends in `.json`, ASCII
  case-insensitive;
- this is the same suffix role rule the validator uses for explicit content
  (`validate.rs:1366`, `payload_kind` `:490-500`).

Outcomes:
- **Unsupported** (`implicit: unsupported external tileset content template`), whether or not
  any tile makes that slot available.
- **Exception: InvalidInput** when that JSON slot's availability is 1 at level 0.
- **Non-JSON suffix.** A template without a `.json` suffix whose resolved member holds tileset
  JSON fails C1 payload decoding (InvalidInput, wrong magic). Today's validator does the same
  for explicit content, so role is never inferred from bytes.

**Why level 0 is InvalidInput, mandated by the text.**
- `ImplicitTiling/README.adoc` `:75`: "The `content.uri` shall not point to an external tileset",
  stated for the implicit root tile.
- `:130`: "`level` is 0 for the implicit root tile".
- `:198-201`: content availability determines which tiles have a content resource located
  through the template.

So a level-0 content bit of 1 on a JSON slot means the implicit root tile's own content is an
external tileset. That is a direct "shall not" violation, not an interpretation.

**Why other availability is only Unsupported.** For bits at level ≥ 1 the text neither permits nor
forbids JSON content explicitly. A template is not a single `content.uri` value, and the A2 audit
records the official validator's TODO at this exact rule. So, per the user's decision, no
validity verdict is given.

**The cost of the exception.**
- The InvalidInput case needs the root subtree, so the root subtree (§1–§5) is admitted before
  the Unsupported result is returned.
- The precedence is: root admission, then root-subtree admission, then the level-0 JSON bit
  check, then Unsupported.
- A malformed root subtree in such a tileset is therefore InvalidInput.
- If the reviewer prefers uniform Unsupported without reading any subtree, the change is one rule
  plus controls C-4 and C-5 (OD-1b).

**Consequence (recorded).**
- Until the producer layout change lands, these will fail `validate` with Unsupported (exit 2):
  - default point-cloud and vector outputs above the chunking threshold;
  - deep legacy `mesh-to-3tz` outputs (shared writer);
  - every multi-node `convert-to-implicit` output;
  - the #151 fixtures.
- `convert-to-implicit` validates its own candidate (`convert_implicit.rs:205`), so it can no
  longer publish multi-node conversions.
- Existing tests that assert these outputs validate will fail:
  - `tests/native_implicit.rs:23-122`;
  - `tests/convert_implicit.rs:536-571`, and any other multi-node convert case.

Hence the start gates in ownership-plan.md.

## Claim

When `checks` contains `implicitAddressingAndAvailability`, the archive contains at least one tile
with `implicitTiling`, and every such *implicit root* reached from `tileset.json` was read. This
includes implicit roots on any explicit tile and roots inside explicit external tilesets. For each
implicit root, all of the following hold:

1. **No JSON templates.** It has no JSON content template.
2. **Subtrees.** Every subtree reachable through available child-subtree bits was resolved from
   its template, read from a confined archive member, and admitted under the subtree-container
   invariants.
3. **Availability.** Tile, content and child-subtree availability satisfy the availability
   invariants, evaluated with LSB-first bit order and Morton level order.
4. **Visiting.** Every available tile was visited once, in the defined order, at its exact global
   coordinates.
   - Its effective bounding volume and geometric error follow the effective-volume rules.
   - Its refinement is the implicit root's effective `refine`.
   - Each available content resolves to a confined, existing payload member.
5. **Same checks as explicit tiles.** Each visited tile passed the checks today's validator
   applies to explicit tiles:
   - C1 payload integrity;
   - `hierarchyBounds` and `geometricErrorOrder` against its parent;
   - `resourceReferences`;
   - recorded budgets, subject to OD-4.
6. **Metadata.** Every tile-metadata row that supplies an interpreted semantic was decoded at the
   tile's availability rank and type-checked.

The claim does not cover:
- decoded content containment (`decodedContentBounds.implicitTiling` remains until C2 slice 3);
- the meaning of non-interpreted metadata columns, content metadata or subtree metadata
  (`metadataSemantics` remains notInspected);
- viewer culling or selection.

Normative basis:
- `ImplicitTiling/README.adoc` (sha256 `14a37a34…`): implicit root constraints `:72-76`,
  subdivision rules `:104-126`, tile coordinates `:128-131`, template URIs `:158-162`,
  availability `:171-207`, tile metadata `:214-230`, buffers and views `:249-255`, availability
  encoding `:257-261`, binary format `:560-589`. Line numbers count the file after removing blank
  and `image::` lines.
- `AVAILABILITY.adoc`: Morton interleave (x is the least significant bit), level offsets, and
  global and local coordinates.
- `Metadata/README.adoc`: binary little-endian, tightly packed; booleans LSB-first;
  offset/scale; noData/default.
- `Semantics/README.adoc`: TILE_* precedence and types.
- `class.property.schema.json`: a semantic may not be reused within a class.
- `subtree`, `availability`, `buffer` and `bufferView` schemas.
- Core `README.adoc:731-748`: external tilesets, which matter only around implicit roots.

## Supported domain

### Admitted

What producers will emit after the layout change, and already emit below the chunking threshold:

- **Tiling.** `QUADTREE` or `OCTREE`, with `subtreeLevels` and `availableLevels` each in
  `[1, 32]`.
- **Root volume.** A `box` with any half-axes, including rotated and degenerate ones.
- **Placement.** An implicit root on any explicit tile, including tiles inside explicit external
  tilesets. Its `transform` and `refine` follow core rules, and `refine` is inherited when absent.
  - This deliberately admits the plausible new producer layouts, so the design lane is not
    constrained to one:
    - a single implicit tree with child subtrees;
    - an explicit skeleton with implicit roots at explicit tiles;
    - explicit external tilesets whose roots are implicit.
- **Content templates.** Payload templates only (`content` or `contents`). Other template
  properties, such as `extensions` and `group`, form a *content header* applied to every derived
  content (`Implicit3DTileContent.js:453`).
- **Subtrees.** Binary subtrees with:
  - one internal buffer, and external buffers that resolve to confined members (OD-6);
  - bitstream or constant availability, with or without `availableCount`;
  - any number of child subtrees.
- **Tile metadata.** A `tileMetadata` property table of any class. TILE_BOUNDING_BOX and
  TILE_GEOMETRIC_ERROR are interpreted **by `semantic`**:
  - component type FLOAT32 or FLOAT64;
  - no offset, scale, noData or default on those properties.
  All other columns are opaque.
- **Unused metadata entries.** `contentMetadata` and `subtreeMetadata` are counted structurally
  but otherwise opaque. They are admitted unless a CONTENT_BOUNDING_* or CONTENT_*_HEIGHT semantic
  is present.

### Unsupported (valid or undecided, outside this profile; exit 2)

- **JSON content templates.** Any content template naming JSON, in any slot (OD-1 resolution).
- **Root volumes.** `region` roots (#120); `3DTILES_bounding_volume_S2` roots.
- **Subtree format and extensions.**
  - JSON-format subtrees: a member that does not start with `subt`.
  - The 3D Tiles 1.0 `3DTILES_implicit_tiling` tile extension.
  - Any `extensions` object in subtree JSON.
- **Tile metadata semantics** that affect volume, transform or refinement:
  - TILE_BOUNDING_SPHERE, TILE_BOUNDING_REGION, TILE_BOUNDING_S2_CELL;
  - TILE_MINIMUM_HEIGHT, TILE_MAXIMUM_HEIGHT;
  - TILE_TRANSFORM;
  - TILE_REFINE.
  Other semantics are opaque.
- **Modifiers on interpreted properties.** offset, scale, noData or default on an interpreted
  property, whether in the class or the property table.
- **Content semantics.** CONTENT_BOUNDING_* or CONTENT_*_HEIGHT semantics in `contentMetadata`.
- **Level limits.** `availableLevels > 32` or `subtreeLevels > 32`.
- **Repeated subtrees.** A subtree member resolved twice in one validation (OD-7).

### InvalidInput (malformed per pinned text; exit 3)

All rules in the Invariants section. In addition:
- **JSON at level 0.** A JSON template slot that is available at level 0 (OD-1 resolution).
- **Sphere roots.** A sphere root (`:85`).
- **Implicit root shape.** An implicit root with `children`, with `metadata`, or with
  `content.boundingVolume` on a template (`:72-76`).

## Invariants

### 1. Subtree container

Each subtree member is read through the validator's resolver:
- confined `uri()` (`validate.rs:158-210`) relative to the tileset JSON that holds the implicit
  root;
- `ArchiveResources::reference`;
- `read_member` with `member_bytes`.

Admission order:

1. **Header.** At least 24 bytes; magic `subt`; version `1`.
   - `jsonByteLength > 0`, and both lengths are multiples of 8.
   - `24 + json + bin == member length`, in u64.
2. **JSON chunk.** Parsed by C1 `json::parse` with the `JsonLimits` from `content_limits()`
   (`validate.rs:26-41`). Trailing padding is `0x20` only. The top level is an object.
3. **Buffers.**
   - `byteLength ≥ 1`.
   - A buffer without `uri` must be index 0, and the BIN chunk must exist. In that case
     `byteLength ≤ bin` and `bin − byteLength < 8`, and the padding is zero.
   - A buffer with `uri` must not be a data URI (InvalidInput). It resolves relative to the
     subtree member. Its member length must be `≥ byteLength`, matching C1 (`payload.rs:627-633`).
   - A non-empty BIN chunk with no internal buffer is InvalidInput.
4. **Buffer views.**
   - `buffer` must be a valid index.
   - `byteOffset` is **required** and must be a multiple of 8. Today it defaults to 0
     (`implicit/tileset.rs:912-916`).
   - `byteLength ≥ 1`.
   - `byteOffset + byteLength ≤ buffer.byteLength`, using checked arithmetic.
   - Every view is validated.
5. **Availability**, per §2.
6. **Property tables**, per §5.

Integer fields must be non-negative JSON integers that fit in u64; otherwise InvalidInput.
`extensions` anywhere in subtree JSON is Unsupported, and unknown non-extension properties are
ignored.

### 2. Availability

Notation:
- `N` is 4 for QUADTREE and 8 for OCTREE.
- `L` is `subtreeLevels`.
- The tile and content bit count is `T = (N^L − 1)/(N − 1)`.
- The child-subtree bit count is `C = N^L`.

Both counts are computed in u128; `L ≤ 32` keeps `C ≤ 2^96`.

- **Encoding.**
  - Exactly one of `bitstream` and `constant` is present.
  - `constant` must be 0 or 1.
  - `tileAvailability.constant == 0` is InvalidInput.
- **Bitstream length.** A bitstream view has `byteLength == ⌈bits/8⌉`. Bits beyond the count must
  be 0.
  - Bit `i` is `(byte[i/8] >> (i%8)) & 1`.
  - MSB-first reading differs on every producer witness with a bitstream (probe field
    `anyMsbFirstWouldDiffer`), so the bit-order control is sensitive on real data.
- **availableCount.** When present it equals the popcount (for a constant: 0 or the count). It is
  checked in O(view bytes), with no allocation.
- **Root tile.** Tile bit 0 is 1.
- **Parents.** `tile[off(l)+m] ⇒ tile[off(l−1)+⌊m/N⌋]`.
- **Content layers.** The number of `contentAvailability` layers equals the number of templates;
  the array is absent when the root has no content. `content[k][i] ⇒ tile[i]`.
- **Child-subtree bits.** `child[m] ⇒ tile[off(L−1)+⌊m/N⌋]`.
- **Available levels.** Let `A` be `availableLevels`.
  - An available tile at global level `≥ A` is InvalidInput.
  - A set child-subtree bit whose child root would be at level `≥ A` is InvalidInput.
  - A looser declared `A` is admitted (OD-8).
- **Child subtree roots.** For each set child bit the child subtree must resolve and be admitted.
  A missing member is InvalidInput.
- **Unset child bits.** Nothing is read. Orphan members fall to the existing "unreferenced archive
  entries" check.

### 3. Addressing and traversal

- **Morton order.** Child index `c` sets `x = 2x + (c&1)`, `y = 2y + ((c>>1)&1)` and, for octrees
  only, `z = 2z + ((c>>2)&1)`. Within a level, Morton order interleaves with `x` as the least
  significant bit.
- **Coordinates.** Global `level < 32` and `x, y, z < 2^level`. Local coordinates are
  `global − (subtreeRoot << localLevel)`.
- **Templates.**
  - Substitution covers `{level}`, `{x}`, `{y}` and, for octrees, `{z}`, in decimal.
  - A missing required variable is InvalidInput (`:161`).
  - `{z}` in a QUADTREE template is InvalidInput.
  - A leftover `{` or `}` is InvalidInput.
  - Substitution happens before percent-decoding; resolution uses the existing `uri()`, relative
    to the tileset JSON that holds the implicit root. Subtree buffers resolve relative to the
    subtree.
  - Scheme, query, fragment and absolute paths keep today's Unsupported mapping.
- **Traversal order.** The owner drives a depth-first pre-order walk of one implicit root: a tile,
  then its available payload contents in slot order, then its available children in ascending
  child index.
  - Implicit tiles can no longer reach JSON content, so no external tileset is expanded inside an
    implicit tree.
  - The validator consumes the owner's tile sequence in order. This fixes the C2 reference order
    and the first-failure order.
  - Explicit external tilesets *around* implicit roots keep the existing traversal, unchanged.
- **Revisits and cycles.**
  - No coordinate is visited twice within a root; this follows from the parent rule.
  - A subtree member resolved twice in one validation is Unsupported (OD-7).
  - Cycles can arise only through explicit external tilesets, and the existing active set
    (`validate.rs:1122-1124`) catches them.
  - Repeated payload members remain admitted, as for explicit tiles.

### 4. Effective volume and error

**Implicit root.** The tile `R` carrying `implicitTiling` keeps its JSON `boundingVolume` `V_R`,
its `transform` and its `geometricError` `e_R`.

**Level-0 tile.** The level-0 implicit tile `I_0` is the same tile for counting. Its effective
volume `V_0` is the TILE_BOUNDING_BOX row of `I_0` if present, otherwise `V_R`.

**Two volumes.** Both are volumes of that tile.
- `hierarchyBounds` checks `V_0 ⊆ V_R` with the existing `contains()`, replacing the expander's
  1e-6 placeholder check (`implicit/tileset.rs:759-772`). The tolerance is the same; the message
  differs.
- For C2 they are two levels joined by `to_parent: None`.

**Deeper tiles.** For a tile at level `l` with coordinates `(x, y, z)`:

- **Effective volume.** The row (`Row(f64×12)`) if present. Otherwise the **derived cell**,
  represented symbolically and exactly as `Derived(Cell { root: V_R.box, level: l, coords,
  scheme })`.
  - Its meaning is exact rational arithmetic. Centre lane `j`:
    `c_j + Σ_k a_kj·((2·i_k + 1)/2^l − 1)`. Half-axis `k`: `a_k / 2^l`. Only axes 0 and 1
    subdivide for QUADTREE.
  - No rounded `f64` centre is produced or compared as truth.
  - At level 0 it equals `V_R` exactly and is reported as `Row(V_R.box)`.
- **Effective error.** The row if present. Otherwise `e_R · 2^−l`, which is exact in binary64
  when `e_R = 0` or the result is normal. A subnormal nonzero result is Unsupported.
- **Refinement.** Constant: the root's own `refine`, or the inherited value. The expander's
  descendant `"REPLACE"` (`implicit/tileset.rs:1135`) is not ported.
- **Transforms.** No implicit tile carries a transform; only `R` does. TILE_TRANSFORM is
  Unsupported.
  - Implicit trees no longer contain external roots.
  - An explicit external tileset whose root `R'` is implicit composes `R'.transform` once, through
    the existing explicit chain (C2 I1/I2).
- **Row values.** Twelve finite values, and an error that is finite and `≥ 0`; otherwise
  InvalidInput.
  - FLOAT32 rows widen exactly.
  - No rounding, padding or tolerance is applied by I1.

The legacy `hierarchyBounds` check needs an `f64` box for `Derived` cells (OD-3). Any such box is a
labelled rounded evaluation for that tolerance check only, and never enters C2 facts.

### 5. Metadata rows

The schema is the inline `schema`, or the `schemaUri` member, of the document holding `R`.
`tileMetadata` indexes `propertyTables`, and the table's `class` must exist (InvalidInput
otherwise).

**Interpreted properties.** These are the class properties whose `semantic` is TILE_BOUNDING_BOX
or TILE_GEOMETRIC_ERROR, matched **by semantic**. Class and property names are never used: today
the expander requires `rustyTile`, `boundingBox`, `geometricError` and `extras`
(`implicit/tileset.rs:718-735`, `:1005`), which made the #151 fixtures InvalidInput (executed).
- A semantic used twice in one class is InvalidInput.
- **TILE_BOUNDING_BOX** must be `SCALAR`, `FLOAT32|FLOAT64`, `array true`, `count 12`, and not
  normalized.
- **TILE_GEOMETRIC_ERROR** must be `SCALAR`, `FLOAT32|FLOAT64`, and not an array.
- Any other shape for these semantics is InvalidInput.
- offset, scale, noData or default is Unsupported. Today they are ignored silently (static,
  `:723-731`).

**Rows.**
- `count` must equal the tile popcount.
- The row of a tile at bit index `i` is `rank(i) = popcount(tile[0..i))`.
- The `values` view must have `byteLength == count · 12 · size` (or `count · size` for the error)
  exactly.
- Values are read little-endian, in place.

**Omitted columns.** A `required` interpreted property missing from the table is InvalidInput; an
optional one is simply not an override.

**Other columns.** They must reference existing validated views. Their contents are opaque; the
`extras` STRING is not parsed (today: `:1048`).

**Content and subtree metadata.**
- `contentMetadata` has one entry per template, and `count` equals that layer's popcount.
- CONTENT_BOUNDING_* or CONTENT_*_HEIGHT semantics are Unsupported.
- `subtreeMetadata` is opaque.

### 6. Profile exclusions (decided)

These are listed under Supported domain. Each has a single-defect control and a stable message
prefix: `implicit: unsupported <reason>`.

### 7. Bounded work (caps before the allocations they guard)

There are no new `ValidationLimits` fields (OD-5). Limits are selected once in `inspect_selected`.

| Quantity | Bound | Checked before | Outcome |
| --- | --- | --- | --- |
| Implicit tile visits | `hierarchy_visits` (65,536), shared with explicit tasks. At subtree admission the tile popcount is **reserved** (charged at once to `task_visits`); visits do not charge again | the rank table and the first visit of that subtree | ResourceLimit |
| Hierarchy depth | `hierarchy_depth` (128). An implicit tile's depth is depth(`R`) + level | the visit | ResourceLimit (existing) |
| Subtree and external buffer member bytes | `member_bytes` per member; `total_bytes_read` overall | the read (size from the archive index) | ResourceLimit (existing) |
| **Live subtree bytes** | `Σ` member bytes of the subtrees on the current depth-first path, plus their external buffers, `≤ document_decoded_bytes` (64 MiB). The path is the subtree containing the tile being visited, plus its ancestor subtrees in this root. A subtree is released when its last tile's descendants are done | the read of each subtree or buffer | ResourceLimit |
| Subtree JSON | C1 `JsonLimits` | the parse | as C1 |
| Content and subtree references | `references` (262,144) | registration | ResourceLimit (existing) |
| Rank table | `4·⌈T/64⌉` B, for bitstream tile availability only; at most `member_bytes/2`; allocated with `try_reserve_exact` | allocation | ResourceLimit (host refusal) |
| Levels | `availableLevels ≤ 32`, `subtreeLevels ≤ 32` (profile constants) | root admission | Unsupported |

**Why the live path replaces v1's per-root cumulative sum.**
- Without JSON links, one implicit root holds every subtree of a large output.
- Measured in the child-subtree layout (`pc-perf-2000`): 234 subtrees, 246,584 B, about 1,054 B
  each.
- A cumulative 64 MiB cap would bind near 63,700 subtrees. That is the same scale as the visit cap,
  so valid producer archives could hit ResourceLimit under default limits.
- The live path holds at most `⌈32/L⌉` subtrees. It is deterministic given the traversal order, so
  O computes the same sums and at-cap controls are exact.
- Depth-first ownership by the owner is what makes the release point well defined. This is the
  main simplification that dropping JSON templates allows.

What this means for failures:
- **New ResourceLimit sources.** I1 adds only two: the pre-allocation reservation and byte
  checks, which apply existing limits; and fallible allocation refusal.
- **Never a verdict.** A ResourceLimit is never a verdict about the archive.
- **No allocation scales with declared counts.** `T`, `C`, `availableCount` and `table.count`
  size nothing until popcount ≤ the remaining reservation, or a view length has bounded them.

**Time.** O(subtree bytes read + tiles visited + JSON items + Σ template length per content).
Bit passes are O(view bytes). Constant child availability enumerates only set bits.

**Memory held.** Live-path subtrees plus one cursor stack of at most `32·N` entries per root. The
serialized presentation tree disappears (today budgeted at 64 MiB, `implicit/tileset.rs:1160-1168`).
This is not an RSS claim.

**InvalidState.** An internal invariant breach, for example rank ≥ count after the count check, a
visit beyond the reservation, or a Morton mismatch, is `ValidationFailure::InvalidState` (exit 1)
and never a verdict.

## Failure semantics and precedence

| Kind | Meaning in I1 | Exit |
| --- | --- | --- |
| InvalidInput | Malformed per the pinned text or the invariants; also a JSON slot available at level 0 | 3 |
| Unsupported | Valid or undecided, outside the profile; this includes every JSON template | 2 |
| ResourceLimit | An existing limit checked before the allocation or read it guards, or host allocation refusal. Never a verdict | 3 |
| InvalidState | Internal invariant breach | 1 |
| Io | Archive I/O (unchanged) | 1 |

**Precedence.** The first failure in traversal order wins.
- Root admission comes first: `implicitTiling` fields, volume kind, templates (variable rules),
  schema semantics, levels, and the presence of a JSON template.
- If a JSON template is present: admit the root subtree, then apply the level-0 JSON check, then
  return Unsupported.
- Otherwise, within a subtree: §1 items 1–6, then the reservation, then the metadata rows.
- Rules within the same step are not ordered normatively. Controls are single-defect, and three
  double-defect controls pin the step order.
- The owner never classifies by message string.

## Interface supplying C2 level chains

The owner is `src/content_integrity/implicit.rs`. It is crate-private, has no `serde` derive and
consumes C1-admitted JSON. It knows no `ValidationLimits`, `Archive` or report types, and it does
not import `validate::containment`.

```text
pub(crate) enum Scheme { Quadtree, Octree }
pub(crate) struct ImplicitLimits { visits_remaining: u64, live_subtree_bytes: u64, json: JsonLimits }
pub(crate) struct ImplicitRoot { /* admitted implicitTiling, V_R, e_R, refine, payload templates (uri + header), semantics */ }
pub(crate) fn admit_root(tile: &Value, schema: Option<&Value>) -> Result<RootAdmission, ImplicitError<Never>>
pub(crate) enum RootAdmission { Read(ImplicitRoot), JsonTemplates(JsonRoot) }  // JsonRoot only offers check_level0()
pub(crate) enum EffectiveVolume { Row([f64; 12]), Derived(Cell) }
pub(crate) struct Cell { root: [f64; 12], level: u32, coords: [u32; 3], scheme: Scheme }
pub(crate) enum EffectiveError { Row(f64), Derived(f64) }      // Derived = e_R·2^-l, exact by §4
pub(crate) struct VisitedTile { level: u32, coords: [u32; 3], parent: Option<u32> /* index in this walk */,
                                volume: EffectiveVolume, error: EffectiveError,
                                contents: SmallVec<(slot: u16, uri: String /* substituted, unresolved */)> }
pub(crate) struct Walk<'r> { /* owns the cursor stack and live-path subtrees */ }
impl ImplicitRoot { pub(crate) fn walk(&self) -> Walk<'_>; pub(crate) fn header(&self, slot: u16) -> &Value; }
impl Walk<'_> {
  pub(crate) fn next<E>(&mut self, limits: &mut ImplicitLimits,
        read: &mut dyn FnMut(&str /*relative uri*/, Base /*Tileset|Subtree(name)*/, u64 /*max*/) -> Result<Vec<u8>, E>)
        -> Result<Option<VisitedTile>, ImplicitError<E>>;   // depth-first pre-order; lazily admits, releases subtrees
}
pub(crate) enum ImplicitError<E> { Format(FormatError), Resolver(E), InvalidState(&'static str) }
```

The validator maps these facts as follows. `containment.rs` is untouched.

**The tile `R`.** The existing `TileFact { transform: R.transform, volume: V_R, external, parent,
depth }` (`validate.rs:518-529`). `external` is set only when `R` is an explicit external tileset's
root, as today.

**`I_0`.** `TileFact { transform: None, volume: Box(V_0), external: None, parent: R, depth:
depth(R) }`. `build_levels` (`validate.rs:590-614`) yields `[own V_0] → (None) [ancestor V_R] → …`.

**A tile at level ≥ 1.** `TileFact { transform: None, volume, external: None, parent: tile of
VisitedTile.parent }`.
- `Row(b)` maps to `VolumeFacts::Box(b)`, exactly.
- `Derived(cell)` has no `VolumeFacts`.
- In the I1 PR, implicit archives keep the C2 exclusion `implicitTiling`, and B registers no C2
  reference for implicit tiles.
- C2 slice 3 decides how to consume `Cell`: either an exact predicate (its own delta) or a new
  reason (a frozen-enum delta).
- Producers row-override every tile (`noRow = 0` in every probe). The design lane is asked to keep
  that property (ownership-plan start gates).

**Payload contents** go through `Check::payload` and the C1 cache. There is never JSON content and
never a content volume for implicit tiles.

**Consequence.** C2 slice 3 can consume every row-overriding archive with no change to
`containment.rs`. It only:
- stops setting `implicitTiling` (`validate.rs:1166-1168`, and the new any-tile site);
- registers implicit references.

## Validator migration

Author B; it merges after, or together with, the producer layout change (start gates).

1. **`prepare_tileset`.** It no longer expands (`validate.rs:1160`). The vector
   `rustyTilesSourceContents` handling (`:1206-1217`) moves into `prepare_tileset` unchanged.
2. **`node()` with `implicitTiling` on any tile.**
   - The nested-root refusal (`:1280-1284`) is removed.
   - The explicit-tile checks run on `R`.
   - `admit_root` is called. For `JsonTemplates`, B reads the root subtree through the resolver,
     calls `check_level0()`, and returns InvalidInput or Unsupported.
   - Otherwise templates are not treated as references, and template headers go through
     `extension_object` once each.
   - The C2 exclusion `ImplicitTiling` is set for any tile.
   - B then **loops over `walk().next()`** inside the same task. For each tile it applies:
     - `hierarchyBounds` against the parent volume (`contains`, OD-3 for `Derived`);
     - `geometricErrorOrder`;
     - C1 payload inspection per content;
     - per-tile recorded budgets (OD-4).
   - B keeps a parent-volume and parent-error stack indexed by `VisitedTile.parent`.
   - There is no new `ValidationTask` variant and no root registry, because no JSON content can
     interleave.
3. **Removals.** `expansion_budget` (`:898`, `:1600-1604`), `implicit_error` (`:56-62`, test
   `:1758-1761`), and `implicit::expand_tileset_bounded`, `ExpansionBudget` and `expansion_limit`
   (`implicit/tileset.rs:619-660`) are removed. `implicit::expand_tileset` stays for
   `vector/pipeline/store.rs:1006`, the tests and the export.
4. **Counters.** `tiles` counts explicit JSON tiles plus available implicit tiles, with `R` and
   `I_0` counted once. `content_references` counts available payload contents.
   - Both equal today's numbers for every non-chunked output.
   - Verified by probe: `pointcloud-implicit` 73/73 and `pc-perf-2000` 428/428 equal
     `availableTiles` and `contentTiles`.
   - v1's link-tile count change no longer exists.

### Exactly what the report says

For an archive whose implicit roots were all read successfully:

- **`checks`** gains `"implicitAddressingAndAvailability"`, inserted after `"hierarchyBounds"`
  (`validate.rs:1716-1730`). The rest of the list is unchanged.
- **`notInspected`** loses `"implicitAddressingAndAvailability"`. It keeps
  `"decodedContentBounds"`, `"decodedContentBounds.implicitTiling"`, `"metadataSemantics"` and the
  others. It gains `"recordedBudgets.implicitTileExtras"` only if OD-4 drops the extras
  comparison.
- **`tiles` and `contentReferences`** are unchanged for every admitted producer shape.
- **Explicit-only archives:** no change.
- **An archive with a JSON template** produces no report: the result is `ValidationFailure::Unsupported`
  (`unsupported`, exit 2). The message starts
  `implicit: unsupported external tileset content template` and names the template's document
  member, not the template text. A JSON slot available at level 0 gives InvalidInput instead.
- **Rust, CLI and Python** serialize the same `ValidationReport`, which is unchanged. Python parity
  is tested in `bindings/python/tests/test_api.py`.

### Exit-status changes versus today

The list is complete for the I1 PR. **E** means executed in this preparation; **S** means static.

| Change | Archives | Basis |
| --- | --- | --- |
| **success → Unsupported** | Any implicit root with a `.json` content template. Today this includes default point-cloud and vector outputs above the chunking threshold (W3, W5), deep legacy `mesh-to-3tz` (`native_implicit.rs:23`), and every multi-node `convert-to-implicit` output (W8, `tests/convert_implicit.rs:536-571`). `convert-to-implicit` then refuses to publish them (`convert_implicit.rs:205`) | E: today W3 and W5 validate OK (`probes/receipt.json`) |
| **success → InvalidInput** | A JSON slot available at level 0 | S |
| InvalidInput → Unsupported | The #151 fixtures as corrected: they have JSON templates. Today they are InvalidInput through class-name gating | E: `a2fix/*` |
| InvalidInput → success | Valid semantic-by-`semantic` tilesets without JSON templates whose class or property names differ from the producer's; FLOAT32 semantic rows | S, plus O witness W9' |
| InvalidInput → read | Implicit roots on non-root explicit tiles | S: `validate.rs:1280-1284` |
| InvalidInput → success | Malformed JSON in an opaque `extras` STRING column | S: `:1048` |
| InvalidInput → read | External subtree buffers | S: `:895-897`, OD-6 |
| success → InvalidInput | Missing `bufferView.byteOffset`; templates missing a required variable; `contentMetadata` count mismatch | S: `:912-916`, `:838-851` |
| success → Unsupported | Repeated subtree member (OD-7); unsupported TILE_* or CONTENT_* semantics; offset, scale, noData or default on interpreted properties | S |
| InvalidInput → Unsupported | Region or S2 roots; `availableLevels > 32` | S: `:684`, `:690-691`, `:710` |
| ResourceLimit → success | Sparse subtrees with `N^L > 65,536` but few available tiles | S: `:695-699` |
| (Not a change) | `3DTILES_implicit_tiling` (1.0) | Becomes Unsupported. Today it is not read, and its members fall to the unreferenced-entry check (S, inferred) |
| Report | `checks` and `notInspected` labels as above; counts unchanged | — |

There are no public Rust API changes.

## Disposition of existing code

| Component (`f2ed077`) | Decision | Basis |
| --- | --- | --- |
| `validate.rs:1203-1263` `expand_implicit` | **Replace** with the owner walk inside `node()` | The production expander is not independent truth; presentation tree |
| `validate.rs:1206-1217` retained vector sources | **Retain**, moved | — |
| `validate.rs:1166-1168` root-only exclusion | **Rework**: set it for any tile | — |
| `validate.rs:1280-1284` nested refusal | **Remove** | `:28` |
| `validate.rs:898`, `:1600-1604`, `:56-62` | **Remove** | Replaced by §7 |
| `validate.rs:1265-1436` `node()` | **Rework**: implicit-root branch and walk loop; shared per-tile checks | — |
| `validate.rs:1366-1378` JSON content for explicit tiles | **Retain**, unchanged | No longer reachable from implicit tiles |
| `validate.rs:1408-1415` recorded extras | **Decide (OD-4)** | — |
| `validate.rs:2550-2556` test | **Rework**: also assert the label | — |
| `validate.rs:197-358` legacy volume helpers | **Retain** for `hierarchyBounds` only | D4 retires them |
| `validate/containment*` | **Untouched** | — |
| `implicit/tileset.rs:619-660` | **Remove** | Only caller is the validator |
| `implicit/tileset.rs:606-617`, `:662-1196` `expand_tileset` and reader | **Retain** for non-validator callers only, **not truth** | Defects not ported: class-name gating (E); descendant `REPLACE`; `byteOffset` default; no variable check; `implicit-tileset-` prefix linking and link equality `:801-829` (moot: the profile excludes links); extras parsed as JSON; single internal buffer; f64 `derived_box` |
| `implicit.rs` `Subtree` writer; `implicit/tileset.rs:229-602` writers | **Not touched by I1.** The **producer layout lane** changes `write_chunks` (`:473-602`) and its trigger (`:284-308`) | Producer change (start gate) |
| `convert_implicit.rs:347-516` | **Not touched by I1.** It needs a recorded disposition before I1 merges (start gate) | Every multi-node output has a JSON slot by design (`:404-421`) |
| `content_integrity.rs` | One line: `pub(crate) mod implicit;` | — |
| `tests/p1_point_oracle.py:279-287` | **Reference only** for O | Producer-shaped; merges links |
| `tests/native_implicit.rs:23-122`, `tests/convert_implicit.rs:536-571` | Changed by the **producer lane** and convert disposition, not by B | They assert that JSON-link outputs validate |
| `docs/VALIDATION.md:143`; inventory `:40`, `:100` | **Rewrite or update** (R) | — |

## Independent acceptance plan

### A. Oracle O-implicit (separate author; never reads `src/`)

- **Implementation.** Pure Python 3 standard library: `zipfile`, `json`, `struct`, `fractions`; no
  repository imports.
- **Sources.** Written from the pinned primary text, re-pinned under
  `bench/architecture_audit/implicit_read/primary/`, and from this contract.
  - O may read `tests/p1_point_oracle.py:279-287` for Morton layout only.
  - O may read the #151 fixture data.
- **Self-test.**
  - the `AVAILABILITY.adoc` interleave examples, bit-exact;
  - exhaustive Morton round trips for level ≤ 4;
  - a `Fraction` identity: the union of the derived children equals the parent along each axis;
  - live-path byte sums on a hand-built tree.
- **Output per archive.** A canonical record:
  - `verdict`: Accepted, InvalidInput, Unsupported or ResourceLimit (with the limit name);
  - for Accepted, a tile list in traversal order. Each entry has `doc`, `level`, `x`, `y`, `z`,
    `depth`, `parentIndex`, `volume` (`row:` hex of 12 f64, or `derived:` root hex plus level and
    coordinates), `error` hex, `refine` and `contents` (slot and member);
  - its SHA-256.

  Production exposes the same list through `#[cfg(test)] implicit_tile_facts(request, limits)` in
  `validate.rs`, on the pattern of `containment_work_facts` (`:1494-1527`). Parity means equal
  hashes.

### B. Witnesses (R regenerates on the final source and hands O the hashes)

| ID | Producer and command | Expected under I1 |
| --- | --- | --- |
| W1 | point-cloud default, `cloud.las --sourceCrs local --maxPoints 1000` (73 tiles, 1 subtree) | Accepted and facts equal |
| W2 | point-cloud default, perf `points.las --maxPoints 2000` (234 subtrees, 233 child bits, 428 tiles) | Accepted |
| W3 | point-cloud default, `cloud.las --maxPoints 40`, **on the producer-change head** | Accepted. On f2238cd (JSON links): Unsupported, the negative witness W3u |
| W4 | vector default ×6 (single tile) | Accepted |
| W5 | vector default, perf `vector.geojson --jobs 2`, **on the producer-change head** | Accepted. On f2238cd: Unsupported (W5u) |
| W6 | legacy `mesh-to-3tz split.gltf --maxTriangles 512 --maxBytes 0` (73 tiles; rows outside cells) | Accepted |
| W6d | legacy deep mesh (`native_implicit.rs:23` recipe) on the producer-change head | Accepted, if the shared-writer change covers it |
| W7 | legacy small mesh (refine ADD, `.gltf`) | Accepted |
| W8 | `convert-to-implicit`: corpus single tile (no JSON slot) | Accepted. Multi-node outputs: Unsupported (W8u), until the convert disposition |
| W9u | #151 QUADTREE and OCTREE fixtures, schema id corrected | Unsupported (JSON templates) |
| W9' | O-authored variants of the #151 data with link templates removed: leaf roots with semantics on class `a2Tile`, FLOAT32 variant | Accepted; semantic-by-semantic witness |
| W10 | Native and portable builds of W4 and W5 | Same as W4 and W5 |
| W11 | The design lane's new-layout outputs for every changed producer | Accepted, with production equal to O. **Merge gate** |

### C. Sensitive data controls (O; base and mutant both hashed; single defect)

"Facts" means the verdict stays Accepted but the tile-fact hash must change identically in
production and O.

| # | Control | Expected |
| --- | --- | --- |
| A-1 | Tile bit 1→0 under an available child (one-bit) | InvalidInput |
| A-2 | Tile bit 0→1 at a leaf whose parent is available | InvalidInput (row count) |
| A-3 | Bitstream rewritten MSB-first | InvalidInput, or facts differ (W1, W2) |
| A-4 | `availableCount` ±1 | InvalidInput |
| A-5 | Trailing padding bit set | InvalidInput |
| A-6 | Content bit without its tile bit | InvalidInput |
| A-7 | Child bit under an unavailable tile | InvalidInput |
| A-8 | Child bit beyond `availableLevels` | InvalidInput |
| A-9 | `availableLevels` 32 versus 33 | Accepted versus Unsupported |
| A-10 | Missing child subtree | InvalidInput |
| A-11 | Repeated subtree member (template collision) | Unsupported |
| A-12 | Three double-defect step-order controls | Specified |
| B-1 | Rows swapped | Facts |
| B-2 | Row component ±1 ulp | Facts (exact hex) |
| B-3 | Interpreted property and class renamed | Facts unchanged |
| B-4 | Semantic removed | Facts change to `derived` |
| B-5 | Semantic duplicated | InvalidInput |
| B-6 | FLOAT32 rows | Accepted, facts widened |
| B-7 | `scale` on TILE_BOUNDING_BOX | Unsupported |
| B-8 | `count` ±1 | InvalidInput |
| B-9 | Error −1 or NaN | InvalidInput |
| B-10 | Derived error subnormal | Unsupported |
| C-1 | Morton x and y swapped in member names | InvalidInput (missing member) or facts |
| C-2 | Template without `{y}` | InvalidInput |
| C-3 | `{z}` in a QUADTREE template | InvalidInput |
| C-4 | A JSON template whose slot is unavailable everywhere | Unsupported |
| C-5 | A JSON slot available at level 0 | InvalidInput (OD-1b) |
| C-6 | A JSON slot available only at level ≥ 1 (the producer pattern) | Unsupported |
| C-7 | `.JSON` uppercase suffix, and a suffix followed by `?v=1` | Unsupported, and Unsupported through the existing `uri()` mapping |
| C-8 | Implicit root on a non-root explicit tile | Accepted |
| C-9 | Region root and sphere root | Unsupported and InvalidInput |
| C-10 | `.glb` template whose member is tileset JSON bytes | InvalidInput (C1 magic) |
| C-11 | Explicit external tileset whose root is implicit, with a root transform | Accepted; the transform is composed once |
| D-1 | Truncated BIN | InvalidInput |
| D-2 | `byteOffset` +1, and absent | InvalidInput |
| D-3 | Non-zero BIN padding | InvalidInput |
| D-4 | Bad magic or version | InvalidInput |
| D-5 | External buffer as a data URI | InvalidInput |
| D-6 | Valid external buffer | Accepted (OD-6) |
| D-7 | JSON-format subtree | Unsupported |

### D. Hashed implementation mutants (R; separate patches for production and O; each must flip its paired control)

| Mutant | Paired control |
| --- | --- |
| MSB-first bit order | A-3 |
| Inclusive rank | B-1, W1 |
| Drop the content ⇒ tile check | A-6 |
| Drop the child-under-unavailable check | A-7 |
| Drop the `availableCount` check | A-4 |
| Key semantics by name | B-3 |
| Ignore the override | B-4, W6 |
| Swap x and y | C-1, W2 |
| Error not halved | B-4 facts |
| `≥` versus `>` on `availableLevels` | A-8, A-9 |
| FLOAT32 read with FLOAT64 stride | B-6 |
| Ignore trailing bits | A-5 |
| **Admit JSON templates** | C-6, W5u |
| **Level-0 JSON as Unsupported** | C-5 |
| **Case-sensitive suffix** | C-7 |
| Read `scale` silently | B-7 |
| Drop the repeated-subtree check | A-11 |
| Reservation after allocation, or none | R2, R3 |
| Live-path release never happens (cumulative) | R2b |
| Refine forced to `REPLACE` | W7 |

A mutant that flips nothing is a failed control.

### E. Resource controls (R, through `inspect_selected` with explicit `ValidationLimits`)

- **R1.** `hierarchy_visits` equal to W2's exact count gives Accepted; one less gives
  ResourceLimit.
- **R2.** `document_decoded_bytes` equal to W2's **maximum live-path sum**, computed by O, gives
  Accepted; one less gives ResourceLimit before the member read.
- **R2b.** The same archive with the limit set between the maximum live-path sum and the
  cumulative sum gives Accepted. This catches a cumulative mutant.
- **R3.** OCTREE `subtreeLevels` 8 with constant availability 1 (2,396,745 tiles) gives
  ResourceLimit, with peak RSS (`/usr/bin/time -v`) within 16 MiB of the W1 baseline. The same
  for `subtreeLevels` 32.
- **R4.** `hierarchy_depth` exact versus one less.
- **R5.** An injected rank-table allocation refusal gives ResourceLimit, never a verdict.
- **R6.** Bitstream `C` at the `member_bytes` boundary.

### F. Adapters, CI and review

- Rust, CLI and Python parity on W1, W2, W5 and W5u.
- Installed wheel; official Blender as applicable.
- Exact-head CI.
- A fresh nonauthor final-source review.

## Performance obligation (standing perf-lane rule)

The lane is `/tmp/rusty-tiles-perf/bench/perf_lane/`, README "Gate". It is not merged to develop,
so I1 pins the scripts by hash (`process.py` `513bec79…`, `corpus.py` `a6cd738c…`, `README.md`
`25b94921…`) unless the lane merges first.

**The rule.** A case is flagged when head/base exceeds 1.25 **and** the growth exceeds the floor
(0.1 s best wall, or 16 MiB peak RSS). It is also flagged when head fails a case that base passes.
There are no `--explain` waivers for I1's validate cases.

**Before and after must use the same archives.**
- **Before** = the producer-change head, without I1. That is develop after the layout lane, or the
  combined branch with I1 reverted.
- **After** = the I1 head.
- Today's chunked outputs cannot serve as a pair, because after I1 they fail fast with Unsupported.

Both run serially on the coordinator's two pinned CPUs, `--repeat 3`.

1. **Standing cases.** `validate:vector` and `validate:point-cloud`. Recorded baselines are
   0.166 s and 0.058 s, but they will be re-baselined on the new layout.
2. **I1 cases.**
   - `validate:pc-perf-2000` (W2; child subtrees; today about 0.10 s, comparable as is);
   - `validate:pc-deep` and `validate:vec-perf` on the new layout (W3, W5).

   The rough single-host numbers are in `probes/receipt.json` and are not lane measurements.
3. **Amplified case.** A point cloud with 50,000–65,000 tiles in the new layout. R records the
   flag value and the tile count.
4. **Report and gate.** Report wall time, CPU time and peak RSS; a gated regression blocks merge.
   The expectation, unproven, is lower RSS.

C2 slice 3 will later add streaming cost to implicit archives (`validate:vector-explicit` grew
10× under C2). That cost is slice 3's to measure.

## Proof limits and remaining gates

The following are not provided:
- decoded containment for implicit tiles (C2 slice 3, which also needs the `Derived` disposition);
- regions and S2 (#120);
- JSON subtrees;
- interpretation of other TILE_* semantics;
- content metadata;
- viewer selection;
- whole-process memory.

The JSON-template question is excluded by profile, not answered: no conformance verdict is given
for level ≥ 1 JSON slots.

Not removed:
- `implicit::expand_tileset`;
- the producers (changed by their own lane);
- `convert_implicit.rs` (disposition pending).

Parents #113, #120, #125 and #126 stay open. There is no main merge, tag or publication.
