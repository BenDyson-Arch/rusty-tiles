# C2 decoded containment: large-mesh tiler amendment (ML0, A-C2-1 to A-C2-4)

Status: **DRAFT for the full (C2-style) treatment.** Written by the ML0 author; not frozen. The
slices require this text to be agreed with the C2-WB and C2 slice-3 owners. Before code it needs
an independent oracle author (O), hashed mutant controls and a fresh nonauthor precode review.
Nothing here records acceptance.

This note amends [c2-decoded-containment-contract.md](c2-decoded-containment-contract.md) (v3.3,
the post-freeze amendments and the C2-WB amendment; sha256 `a0a43e4f…575c`) **as further amended
by C2 slice 3** (frozen v3.1, `/tmp/rusty-tiles-c2s3-contract-20261012/contract-draft.md`
`8bc53429…31d6`, branch `feat/c2-implicit-containment` at `ac1bf94`). Neither text is edited.

**Base:** `origin/develop` `1b1ff9d99fe5e66b844d2c0453abdbe6ed20dd26`, which contains the C2-WB
merge (PR #164) and its WB-6 calibration. **Design:** large-mesh v6.1 (`design.md` `0bf9fd19…441b`,
§3.1, §3.4, §3.5, §10.4, decision E44; ruling 4; U1, U4 (c)); reviews `698b3b61…c4ef` (v5) and
`3f2c0583…ae6d` (v6). **Umbrella:** [f1d3-large-mesh-contract.md](f1d3-large-mesh-contract.md)
§4, §6.4–§6.7.

## Composition with C2-WB and slice 3

- **Order of landing.** C2-WB (merged) → slice 3 → ML7a → ML7b → **ML7c implements A-C2-1**.
  Slice 3 consumes "C2-WB's formula and default cap unchanged" (its §5.2). A-C2-1 changes only how
  the default of three caps is computed and what `limits` serializes. It changes no formula, unit,
  fact, enum name, chain, predicate or precedence of slice 1, C2-WB or slice 3.
- **Slice 3's at-cap controls** (§9.D) use explicit `ValidationLimits` through `inspect_selected`.
  Under A-C2-1 an explicit value *replaces* the default, so those pairs keep their meaning.
- **Slice 3's coverage statement and performance obligation** (§5.2, §10) are measured at the
  fixed 2^29 default. A-C2-1 makes archives above `S₀ = 8 MiB` (below) stream where they reported
  `workLimit`, so validation time grows on them by design. ML7c re-derives the coverage statement
  and records every perf-lane `validate` case before and after. Growth on a case that moved from
  `workLimit` to streaming is an expected change: the perf-lane gate accepts it only through its
  `--explain` option, naming the case and this amendment (coordinator ruling O-C2-3).

## Unchanged

The claim (slice 1 and slice 3); the domain predicate and the 15-name reason enum (with slice 3's
`derivedImplicitVolume`); spans and `containmentSpanBits`; `containmentNodeDepth`; C2-WB's unit
formula, facts and floors; the precheck order and "first violation stops"; streaming order;
outcome precedence (R8); the Outside message; **no ResourceLimit from any C2 counter**; the
public field names `containmentWorkUnits`, `containmentBytesRead`, `containmentDecodedBytes`.

---

## A-C2-1. Archive-proportional defaults for three caps

**Why.** Rulings (b) and (c): validation never refuses for size and absolute volume caps are
opt-in. A fixed 2^29 units makes every large archive `workLimit`, so C2 would never inspect
large-mesh output.

### Rule

Let `S` be the sum of stored data lengths over the selected central-directory catalog (the same
`S` as A-C1-4's read budget; the stored-3TZ profile has no compression). `S` is known before
traversal, so the caps are fixed before the precheck starts.

| Field | Default after A-C2-1 | Today |
| --- | --- | --- |
| `containmentWorkUnits` | `max(B0, κ_u·S)`, `B0` = 2^29 (WB-6) | 2^29 |
| `containmentBytesRead` | `max(2^31, κ_r·S)` | 2^31 |
| `containmentDecodedBytes` | `max(2^31, κ_d·S)` | 2^31 |

- Products saturate at `u64::MAX` (C2 already treats overflow of a running total as exceeding).
- **Explicit values replace the default.** `--max-volume containment-units=N` (and
  `containment-bytes`, `containment-decoded`; A-C1-4's option) sets that cap to `N`. It is not
  combined with `κ·S`. Over the cap is still `workLimit`, never ResourceLimit.
- **Serialized `limits` carry the effective value** applied to this archive, as numbers. The
  report shape does not change.
- Private controls (`inspect_selected`) may also set `B0` and each `κ` explicitly.

### Constants (WB-6 numbers appended)

- `B0` = 2^29 = 536,870,912, from C2-WB WB-6: the largest power of two with `cap × τ_E ≤ 8 s`,
  with `τ_E` = 14.65 ns/unit (E-core) and `τ_P` = 8.10 (P-core) on the measurement host.
- **`κ_u` = 64 units per stored byte (provisional).** Rule: `κ_u ≥ 2 ×` the largest measured
  producer rate `units / S`, including ML6 outputs. Measured so far: mf2-lod3 5.36 and W5 14.25
  units per stored byte [E-rev, C2-WB probes]; the mesh tiler is projected at about 24 at depth 6
  (non-indexed leaves, D1). Frozen in ML9 (design §13, ML9 "κ_u frozen").
- **`κ_r` = 2 and `κ_d` = 16** (coordinator ruling 2026-10-12: design v6.1 gives no value, so
  this amendment's proposal is adopted; provisional like `κ_u` until ML9).
  Same rule: at least twice the largest measured `Σ bytes(r) / S` and `Σ decoded(r) / S` over the
  producer corpus. A payload referenced once is read once, so `Σ bytes / S ≤ 1` for forest output;
  `Dec` counts meshopt-decoded views, whose ratio to stored bytes is not yet measured on
  large-mesh output; ML7c and ML9 measure it.
- **Threshold.** `κ_u·S > B0` exactly when `S > S₀ = B0/κ_u` = 8,388,608 bytes (8 MiB). Archives
  of at most 8 MiB keep the default 2^29 and its serialized value. For bytes and decoded the
  thresholds are 1 GiB and 128 MiB.

### Time bound (replaces the "about 7.9 s" sentence in VALIDATION.md for large archives)

At the cap, phase-2 time ≤ `τ × cap`, as C2-WB states: on the measurement host's E-core,
`max(7.87 s, κ_u·S·14.65 ns)`. With `κ_u` = 64 that is 937.6 ns per stored byte, about **938 s
per 10^9 stored bytes** (P-core 518 s). The design's 929/502 s/GB used the pre-WB-6 τ of
14.51/7.85 and is corrected here. This is a calibration ceiling at the cap, not a typical time:
at the measured producer rates (5–24 units per byte) the projected cost is 73–352 s per 10^9
bytes on an E-core. The per-archive bound now grows with `S` instead of staying at 8 s.

### Exit-status and report changes (complete)

| # | Change | Archives |
| --- | --- | --- |
| XC-1 | `notInspected` `decodedContentBounds.workLimit` → `checks` `decodedContentContainment` | `S` above the threshold, precheck totals between today's cap and the new one, no Outside |
| XC-2 | `workLimit` → **InvalidInput: Outside** | The same, with a proven escape |
| XC-3 | `workLimit` → ResourceLimit (host allocation refusal only) | The same, when the fallible reservations of streaming fail |
| XC-4 | Report: `limits.containmentWorkUnits`, `containmentBytesRead`, `containmentDecodedBytes` | Every archive above the respective threshold |

### Acceptance criteria (O computes `S`, the units and the expected caps from the archive bytes)

- **AC-C2.1 Effective value.** For archives with `S` = 2^23 − 1, 2^23, 2^23 + 1 and one large `S`,
  the serialized `containmentWorkUnits` equals O's `max(2^29, 64·S)` (that is 2^29, 2^29,
  2^29 + 64, and O's value). The same sweep for the other two fields around 1 GiB and 128 MiB.
- **AC-C2.2 Large-archive inspection.** An archive with `S` > 8 MiB whose O units exceed 2^29 but
  are ≤ `64·S` reports `decodedContentContainment` (or the Outside that O predicts). A mutant
  keeping the fixed 2^29 default reports `workLimit`.
- **AC-C2.3 Default function at-cap pairs** (private controls): with `κ_u` = 0 and `B0` = O's
  units: Contained; `B0` = units − 1: `workLimit`. With `B0` = 0 and `κ_u` chosen by O so that
  `κ_u·S ≥ units > (κ_u − 1)·S`: Contained at `κ_u`, `workLimit` at `κ_u − 1`.
- **AC-C2.4 Explicit replaces.** `containment-units=N` with `N` = units − 1 on a large-`S`
  archive whose `64·S` exceeds units: `workLimit`, exit 0. With `N` = units: Contained.
- **AC-C2.5 No ResourceLimit.** Every cap violation yields `notInspected`, never ResourceLimit,
  for each of the three fields.
- **AC-C2.6 Parity.** Every slice-1, C2-WB and slice-3 corpus archive with `S` ≤ 8 MiB gives an
  identical report. The amended C1 limits pins
  (`bench/architecture_audit/c1_payload_integrity/production_controls/c2wb_cap_amendment/2026-10-12/`,
  which expect 536,870,912) stay valid; O lists each pinned fixture's `S`. For larger `S`, the verdict changes
  only from `workLimit`, and only where O's totals are within the new caps.
- **AC-C2.7 Producer headroom (ML7c, ML9).** On every producer witness, including ML6 outputs,
  units, bytes and decoded are each ≤ ½ of the default caps.
- **Mutants:** fixed default kept; `κ_u` applied to entries or to file length instead of `S`;
  `min` instead of `max`; explicit value combined by `max` with the default; serialized value
  shows `B0` instead of the effective cap; product wrapping instead of saturating.

---

## A-C2-2. KTX2 textures: no rule change

`KHR_texture_basisu` is already on the C2 glTF allowlist (texture sources only). Textures have no
effect on drawn POSITION or node transforms. Its C1 admission (A-C1-1) is what turns large-mesh
archives from Unsupported into C2 candidates.

**Acceptance.** AC-C2.8: an in-domain explicit archive whose payloads use required
`KHR_texture_basisu` reports `decodedContentContainment`; the same archive with one drawn vertex
moved outside its tile box reports the Outside message naming that vertex.

---

## A-C2-3. The snapped per-depth quantization grid

**What the producer publishes** (umbrella §4, §6.4; design v6.1 §3.1, §3.4). Fast and balanced
store POSITION as unnormalized `KHR_mesh_quantization` u16 codes `u`, with node `scale` = `s_d`
(a power of two, one per depth, monotone, nested) and node `translation` = an integer multiple of
`s_d`. Every vertex v with `ℓ(v) ≤ d` is stored at `q(v, s_ℓ(v))`, a point of a coarser lattice
that is exact on `s_d`. The lattice origin is in the root tile's transform; translation-only,
lattice-aligned transforms appear only on explicit tiles and implicit roots; implicit tiles below
an implicit root have none (δ ≡ 0). Quality and lossless store f32 relative to the tile frame's
origin.

**C2 rule change: none.** C2's frame chain already decodes these values exactly: integer codes
stay integers, node T and S are exact dyadic rationals (frame chain steps 1–2), tile transforms
are exact steps, and nothing is inverted. Spans stay small: dyadic values of a few limbs, far
below 4,096 bits (slice 3 §5.1 observed ≤ 290 bits on forest output). The snap is just where the
producer chose to put a vertex; C2 tests where it actually is.

**What C2 then proves, and what it does not.**
- C2 proves that every drawn, **snapped** vertex lies in its own tile's row box, the root's box,
  and every ancestor's box (I1 of slice 1, I4 of slice 3), exactly.
- C2 does **not** prove that a shared vertex decodes identically in two tiles (seam
  watertightness), nor anything about GPU f32 dequantization (excluded by the claim). Those are
  the output auditor's checks (umbrella §14).

**Producer obligations** (checked by C2 on producer output; ML4, ML5 and ML6 tests):
1. **Boxes after the snap.** A snapped vertex can move up to `s_ℓ(v)/2` per axis from its source
   position, outside the tile's source box. Each tile's box is computed from its snapped, decoded
   positions (design v6.1: box, offset and range after the snap).
2. **Exact box arithmetic.** Rows store a center `c` and half-axes `h` in f64, and C2 evaluates
   `c ± h` exactly. Naive `c = (min+max)/2`, `h = (max−min)/2` can round so that `c + h < max`.
   The producer chooses `c`, `h` and verifies `c − h ≤ min` and `c + h ≥ max` exactly (or with
   directed rounding).
3. **Ancestors contain descendants.** A leaf's snapped vertex can lie outside its parent proxy's
   own content box. Each tile's box is the hull of its own decoded content and every
   descendant's box, so every ancestor test (slice 1 I1) holds.
4. **Row = root box at level 0**, bitwise (P3), so slice 3's I4 tests two identical volumes.
5. **Axis-aligned rows on every implicit tile** (A-I1-3), so neither `rotatedBox` nor
   `derivedImplicitVolume` applies.

**Acceptance (O builds the fixtures from this text and the C2 contract; production must agree).**
- **AC-C2.9 Snapped vertex inside a post-snap box.** A two-depth forest fixture: a leaf at depth
  2 with `s_2` = 2^-4 and a vertex `v` with `ℓ(v)` = 1, `s_1` = 2^-3, stored at
  `q(v, s_1)`, which lies `s_1/2` outside the leaf's source box on +x. With the box computed after
  the snap: `decodedContentContainment`. With the box from source positions (the bug the
  obligation prevents): InvalidInput, `volume=own`, naming that vertex.
- **AC-C2.10 Ancestor hull.** The same snapped vertex inside the leaf's box but outside its parent
  proxy's own-content box: with the hull rule, Contained; without it, Outside with
  `volume=ancestor:1`.
- **AC-C2.11 Lattice-aligned implicit-root transform.** A forest whose implicit root carries a
  translation of `k·s_0` and whose payload offsets are relative to it: Contained, with O's span
  totals equal to production's. Moving one code by one unit past the row's face: Outside.
- **AC-C2.12 Exact box rounding.** A tile whose maximum decoded coordinate is `m` and whose naive
  f64 center and half-axis give `c + h < m`: the naive row is Outside; the producer-style row
  (verified exactly) is Contained.
- **AC-C2.13 f32 path.** A quality-style fixture (FLOAT positions relative to a lattice-aligned
  frame origin): Contained; one position moved by one f32 ulp past the face: Outside.
- **Mutants (producer-side test helpers, each must flip its control):** box before snap; no
  ancestor hull; naive center and half-axis; offset taken relative to `O` instead of the implicit
  root's translation.

---

## A-C2-4. Published vanished and filled sets, and filled holes

**What the producer publishes** (umbrella §6.5–§6.7; U1, U4 (c)). A proxy may omit vanished
regions and islands, and may contain fan triangles over the kept vertices of a small hole's
chain. Each proxy lists these in an `EXT_structural_metadata` property table
`rustyLodChanges` inside its own GLB (kind, primitive, key). Leaves have no table.

**C2 rule change: none.** The rules below are consequences of the existing contract, stated so
that the oracle checks them on this output.
1. **Fill triangles are drawn and are tested.** A fan has no new vertex: its vertices are the
   chain's kept vertices, which are vertices of the proxy anyway, quantized and snapped like any
   other. C2 marks every vertex the fan's indices reference and tests it (stream algorithm step 3),
   and boxes are convex, so the fan's interior is contained whenever its vertices are.
2. **Vanished geometry is not drawn** and is not tested in the proxy. It is still drawn and
   tested in the descendants that keep it, and the proxy's box must still contain those
   descendants' boxes (A-C2-3 obligation 3). Removing geometry never makes C2 fail.
3. **The published table is not geometry.** `EXT_structural_metadata` is on the glTF allowlist;
   the property table's views are C1-admitted buffer views with no POSITION role, and C2 reads
   nothing from them. So publishing V and F neither excludes the archive nor changes any vertex
   set.
4. **The publication form is part of the domain.** Publishing the sets as tile or content
   `metadata` in tileset JSON would set `tileMetadata` (slice 1; slice 3 I6) and exclude the whole
   archive. Publishing them as a separate archive member that nothing references would be C1
   InvalidInput (unreferenced entry). Publishing them through a new glTF extension would set
   `gltfExtension`. The umbrella's form (§6.7) avoids all three.
5. **C2 does not verify the sets.** Whether each missing boundary is accounted for by a
   `FILLED_HOLE` or `VANISHED_*` row is the output auditor's check (umbrella §14), not a validator
   rule. A future validator check of the sets would need its own contract.

**Acceptance (O builds the fixtures; production must agree).**
- **AC-C2.14 Fill contained.** A proxy with a filled hole (a 6-vertex rim, fan from the
  minimum-key vertex), with `rustyLodChanges` holding one `FILLED_HOLE` row: Contained, and C2's
  `Idx` and `T` facts count the fan's index occurrences and its (already present) vertices once.
- **AC-C2.15 Fill escape is caught.** The same proxy with one rim vertex's code moved outside the
  box (the fan then reaches it): Outside naming that vertex. A fan with an apex that is **not** a
  rim vertex and lies outside the box (a producer bug the fan rule forbids): Outside.
- **AC-C2.16 Vanished island.** A proxy that omits an island which its child keeps, with a
  `VANISHED_ISLAND` row: Contained when the proxy's box is the hull; Outside at `ancestor:1` for the
  child's island vertex when the proxy's box covers only its own content.
- **AC-C2.17 Publication form.** The AC-C2.14 archive gives the same C2 outcome and the same
  `notInspected` with and without the property table. The same sets written as tile `metadata`
  instead: `decodedContentBounds.tileMetadata`. Written as an unreferenced member: C1
  InvalidInput, "unreferenced archive entries".
- **AC-C2.18 Empty table.** A proxy with no record has no table, and a proxy whose table has zero
  rows is not produced (a producer test; C2's outcome is the same either way).
- **Mutants (producer-side):** fan apex chosen off the rim; box computed before vanished
  descendants are hulled; sets written as tile `metadata`.

## Open questions and settled items

- **Settled (coordinator, 2026-10-12):** `κ_r` = 2, `κ_d` = 16 (A-C2-1); by-design slower
  validation of large archives passes the perf-lane gate only through `--explain` (O-C2-3); the
  lattice origin in the root transform with δ ≡ 0 (umbrella §4), which A-C2-3 assumes.
- **O-C2-2 (C2-WB owner).** Confirm `S` as the sum of stored data lengths over the selected
  catalog (not the archive file length, which includes headers and the central directory).
- **O-C2-4 (slice-3 owner).** Add a large-mesh forest witness to slice 3's witness table once ML4
  produces one. Slice 3's corpus has none today.
- **O-C2-5 (C2 owners).** A-C2-3 and A-C2-4 change no rule. Their acceptance criteria are C2
  sensitivity controls on this output's shapes (snapped vertices, hulls, fans, the property
  table); confirm they belong in the C2 corpus rather than only in the producer tests.
