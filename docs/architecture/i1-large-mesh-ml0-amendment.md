# I1 implicit read: large-mesh tiler amendment (ML0, A-I1-1 to A-I1-3)

Status: **DRAFT for the full (C2-style) treatment.** Written by the ML0 author; not frozen. It
needs the I1 owner's agreement, an independent oracle author (O) working from the pinned texts
and this note, hashed mutant controls and a fresh nonauthor precode review before code. Nothing
here records acceptance.

This note amends [i1-implicit-read-contract.md](i1-implicit-read-contract.md) (v3.1 with the v3.2
rulings and integration amendments; sha256 `bd931869…9ee2`), which stays byte-identical. Where
they disagree on an item below, this note governs that item. It is the I1 half of the cap work in
[the C1 amendment](c1-large-mesh-ml0-amendment.md) (A-C1-4), and it composes with C2 slice 3
(frozen v3.1, `contract-draft.md` `8bc53429…31d6`), which uses I1's walk unchanged.

**Base:** `origin/develop` `1b1ff9d99fe5e66b844d2c0453abdbe6ed20dd26`. **Design:** large-mesh v5
(`design.md` `70a1eeec…afd1`, `slices.md` `b0ddfd0c…aa7`); review `698b3b61…c4ef`.

## What the forest output needs from I1

The large-mesh forest (P, amended in [p-large-mesh-ml0-amendment.md](p-large-mesh-ml0-amendment.md))
uses nothing outside I1's admitted domain: OCTREE implicit roots with
`subtreeLevels == availableLevels ≤ 4`, one binary subtree each, `childSubtreeAvailability
{constant: 0}`, TILE_BOUNDING_BOX and TILE_GEOMETRIC_ERROR rows on every tile, one `.glb`
content slot, no CONTENT_URI and no JSON templates. **No semantic change is needed for
admission.** What it needs is scale: millions of implicit tiles. Today every archive above
262,144 visits, 1,048,576 references or 2 GiB of reads is ResourceLimit. Under A-C1-4 those
become proportional budgets. Once `references` is proportional, I1's content index (`offsets`,
one entry per emitted content) is no longer bounded by a constant, so A-I1-2 adds an item cap.

| Item | What | Slice |
| --- | --- | --- |
| A-I1-1 | I1's charges are compared against A-C1-4's budgets; I1 subtree availability bytes earn visit credit | ML7b |
| A-I1-2 | New item cap on available tiles and contents across the live subtrees of one path | ML7a (memory) and ML7b (default) |
| A-I1-3 | Forest-domain statement and producer obligations (no rule change) | ML3, ML5 acceptance |

## Unchanged

The claim; the admitted, Unsupported and InvalidInput domains; invariants §1–§6; the §2a step
order and passes; template normalization; the OD-7 repeated-subtree rule; the walk order; the
failure categories, messages and precedence; the C2 interface; `hierarchyBounds` tolerances.

---

## A-I1-1. I1 charges against the amplification budgets

The quantities I1 charges are **unchanged**; only the limit they are compared with changes.

| I1 charge (unchanged) | Compared today with | Compared after A-I1-1 with |
| --- | --- | --- |
| `task_visits` plus each subtree's `P_t` reservation (§2a step 9, §7 "Visits") | `hierarchy_visits` | budget V (A-C1-4), plus an opt-in absolute `visits` cap |
| `reference_visits` including each subtree's `E` reservation (§2a step 10) | `references` | budget R, plus an opt-in absolute `references` cap |
| `bytes_read`: subtree member reads, external buffers and the availability scan `Scan(s)` (§2a step 2, OD-10) | `total_bytes_read` | budget B, plus an opt-in absolute `reads` cap |

**Visit credit from subtrees.** V contains `32·Scan(s)` for each **distinct** subtree member
`s` (`4·8·availability bytes`). The credit is added in §2a step 2, immediately after `Scan(s)` is
charged to `bytes_read` and before step 9 reserves `P_t`. OD-7 already makes a subtree member
resolved twice Unsupported, so every admitted subtree is distinct and earns its credit once.

**Messages and order.** Every check happens at the same step as today, with today's message and
category. "Remaining" in step 9 and step 10 means the budget (with every credit earned so far in
traversal order) minus the charges so far.

**Monotone.** Each budget is at least today's default, and the charges are identical. An archive
that today's defaults do not refuse with ResourceLimit keeps its verdict, category, message and
report (except `limits`, A-C1-4).

**Why the credit is sound for producers.** A tile available through a `View` bitstream costs at
most one bit, so `P_t ≤ 8·|tile view|` and the subtree's own credit covers 4 visits per tile. A
`Const1` subtree earns no byte credit; for producer forests every implicit tile has a content
member, so `P_t ≤ entries` and the `4·entries` term covers it. A `Const1` subtree with no content
earns nothing and is refused once it exceeds the remaining budget. That refusal is an
amplification limit, not a size limit.

### Acceptance criteria (O computes every expected value from the archive bytes)

- **AC-I1.1 Parity.** Every I1 corpus archive, witness, control and mutant that is not
  ResourceLimit under today's defaults gives an identical verdict, category, message and report
  except `limits`. This includes the R9 control, which must still refuse at its reservation under
  explicit limits.
- **AC-I1.2 Credit at-cap pairs.** For a `View` subtree whose `P_t` exceeds the base term's
  remainder but is covered by its own `32·Scan(s)` credit: success at equality, ResourceLimit with
  today's "visits" message one tile over (one more available bit in a byte that already exists,
  so `Scan` is unchanged).
- **AC-I1.3 `Const1` without content.** A `Const1`, content-free subtree of `T` tiles, with
  the visit base term set through `inspect_selected` so that O's remaining V equals `T`, succeeds;
  with the base one lower it is ResourceLimit at step 9.
- **AC-I1.4 Alias does not inflate credit.** K content layers aliasing one view earn
  `32·|view|` once (the §2a distinct-source rule). A mutant crediting per alias flips the pair.
- **AC-I1.5 Order.** A subtree that would be refused at step 9 without its own credit succeeds
  (credit before reservation). A mutant crediting after step 9 flips it.
- **AC-I1.6 Producer forest headroom.** For every large-mesh, point and vector forest witness, O
  reports `charges / budget` for V, R and B; each is ≤ 0.5. (Recorded at ML3 and ML9.)

---

## A-I1-2. Live-path cap on implicit tiles and contents

**Rule.** Let the live subtrees be the subtree of the current tile and its ancestor subtrees in
the same root (I1 §7, "Live-path subtree bytes"). With `C_sub` = 2^20:

- in §2a step 9, **before** the visit reservation: `Σ_live P_t ≤ C_sub`, counting the subtree
  being admitted; otherwise ResourceLimit, "implicit subtree tiles exceed limit";
- in §2a step 10, **before** the reference reservation: `Σ_live E ≤ C_sub`; otherwise
  ResourceLimit, "implicit subtree contents exceed limit".

`P_t` and `E` leave the sums when their subtree is released.

**Why on the live path and not per subtree.** I1's memory statement (§7, RC-11) bounds the
content-index entries by `Σ_live E ≤ references`. With `references` proportional, that bound
disappears. A per-subtree cap would still allow one cap's worth of entries in each of up to 32
live subtrees (about 256 MiB). The live-path sum keeps the index at most `C_sub` entries (about
8 MiB at 8 bytes per aligned `(u32, u16)` entry) in any archive.

**Monotone.** Today `Σ_live E ≤ references` = 2^20 holds at every moment, and
`Σ_live P_t ≤ hierarchy_visits` = 2^18. So no archive admitted today reaches either new check.

**Representation.** Proposed: a new `ValidationLimits` field `implicit_subtree_tiles`
(serialized `implicitSubtreeTiles`, default 1,048,576), carried in `ImplicitLimits` next to the
live-path byte cap. This supersedes OD-5 ("no new fields") and, like C2's O-b, breaks external
struct literals (O-I1-1). Producer forests use at most 585 tiles per subtree (OCTREE, S = 4) and
one subtree per path.

### Acceptance criteria

- **AC-I1.7 At-cap pairs** (through `inspect_selected` with explicit limits so fixtures stay
  small): with `C_sub` = 1,000, one subtree with `P_t` = 1,000 succeeds and 1,001 is ResourceLimit
  with the tiles message; a root subtree with `P_t` = 600 plus a child subtree with 400 succeeds,
  401 fails (the live-path sum); the same for `E` with the contents message.
- **AC-I1.8 Release.** Two sibling child subtrees of 600 tiles under a root subtree of 300
  succeed with `C_sub` = 1,000 (the first is released before the second is admitted).
- **AC-I1.9 Order.** A subtree that exceeds both `C_sub` and the remaining V reports the
  `C_sub` message (the item cap is checked first).
- **AC-I1.10 Memory.** The content index never holds more than `C_sub` entries (an
  instrumented test, plus ML7a's RSS-flatness gate).
- **Mutants:** per-subtree instead of live-path sum (flips AC-I1.7's second pair); `P_t` not
  released (flips AC-I1.8); cap checked after the reservation (flips AC-I1.9).

---

## A-I1-3. Forest domain and producer obligations (no rule change)

These are statements for the producer slices, so that the large-mesh forest stays inside I1 and
C2 slice 3 without new rules. Each is an ML3 or ML5 test, checked through `validate` within
today's caps and through the independent streaming reader at scale (slices ML3, RC16).

1. Every implicit tile has a TILE_BOUNDING_BOX and a TILE_GEOMETRIC_ERROR row (P3). So C2
   slice 3's `derivedImplicitVolume` never applies to producer output.
2. Rows are FLOAT64 boxes with signed-permutation half-axes (axis-aligned), so `rotatedBox` never
   applies. I1 admits any half-axes; the producer must not use them.
3. A packed leaf (design §2.4, blocked on B2) occupies one slot, its first octant, and its row
   box may exceed that octant's derived cell. I1 uses the row, never the cell, so this is admitted.
   `hierarchyBounds` compares the row with its parent's row (I1 §4, RC-9), so the producer must
   keep every row inside its parent's row (P6 and the C2 obligations in
   [the C2 amendment](c2-large-mesh-ml0-amendment.md)).
4. Level-0 row equals the root tile's JSON box, bitwise (P3), so C2 slice 3's I4 tests two
   identical volumes.
5. `subtreeLevels == availableLevels ≤ 4`, `childSubtreeAvailability {constant: 0}`; no
   subtree member is referenced twice (OD-7).
6. At scale, I1 parity is run only on archives within today's caps (`hierarchy_visits` ≤ 2^18);
   archives with ≥ 2^22 tiles get the independent streaming reader in ML3 and full `validate`
   in ML9, after ML7b.

---

## Open questions

- **O-I1-1 (I1 owner + coordinator).** Public field `implicit_subtree_tiles` (proposed), or a
  private profile constant that is not serialized? The public field is consistent with C2's O-b;
  the private constant avoids an API break but hides a refusal threshold from the report.
- **O-I1-2 (I1 owner).** Confirm the credit factor `4·8` per availability byte. It is the
  design's (§10.3); AC-I1.6 records producer headroom against it.
- **O-I1-3 (coordinator).** A-C1-4's serialization (O-C1-4) applies to `hierarchyVisits`,
  `references` and `totalBytesRead`, which I1 also reads.
