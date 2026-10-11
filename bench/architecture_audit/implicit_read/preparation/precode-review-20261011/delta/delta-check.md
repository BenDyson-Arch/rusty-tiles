# I1 contract v3: focused N delta check

Reviewer: N, the same nonauthor as the precode review (`../review.md`, sha256 `92eee8f0cc79…`).
Date: 2026-10-11.

Reviewed: `/tmp/rusty-tiles-i1-contract-20261011/`:
- `contract-draft.md` v3;
- `open-decisions.md` v3;
- `ownership-plan.md` v3;
- `change-log.md`;
- compared against `v2/`.

Source: `origin/develop` `f2ed077`.

Probes (stdlib `python3 -I`, single-threaded, no cargo, no repo edits). All are in this directory.
- `delta_attacks.py` → `delta_attacks.out.json`. A transcription of `validate.rs:165-210` `uri()` and of v3's §3
  detection rule, plus attack arithmetic.
- `content_index.out.json`. The maximum live content-index size under v3 §2a step 10.
- `uri_probe/`. One timing measurement with the pinned develop binary
  `/home/bend/.cache/c2-corpus/rusty-tiles-f2238cd`, run as `taskset -c 0,1 nice -n 10` (see DC-1).

## Verdicts

| Author | Verdict | Meaning |
| --- | --- | --- |
| **A** (owner) | **GO-WITH-CHANGES** | A may start now on §1, §2 and §2a steps 1–9, 11 and 12, §4 and the Morton/rank code. The template parts of §3 (resolution and JSON detection), §2a step 10 and the §5 class handling start once DC-1 to DC-5 are in the contract text. If the coordinator folds them in as specified below, no further N round is needed; N verifies at final review. |
| **O** (oracle) | **GO-WITH-CHANGES** | O may start writing now. O must **not freeze** until DC-1 to DC-6 are folded in. O mirrors JSON detection, the `Scan`/`E` charges and the template charge, and those definitions change. |

The five priority items were each confirmed resolved. Four new problems were found, of the same kind as v2's:
- **DC-1, time.** Template length multiplies per-emission work. It is measured, and it defeats every cap.
- **DC-2, profile leak.** v3's static JSON detection is bypassable through `.`/`..` and empty segments.
- **DC-3, memory statement.** The content index is checked but not reserved, so it reaches 7.7× `references`.
- **DC-4, time.** Per-table class scans are not memoized: about 1.2·10⁹ property visits.

None of these changes the design. Each fix is a few lines of contract text with a control.

---

## Priority items

| RC | Status | Notes |
| --- | --- | --- |
| **RC-1** | **Resolved**, with one gap (DC-5) | Covered: CONTENT_URI and CONTENT_GROUP_ID; the full TILE_* and CONTENT_* override set; TILE_BOUNDING_BOX/TILE_GEOMETRIC_ERROR in content classes; controls C-12 and C-13; mutants; exit row X-6. Gap: the set is not applied to a content template's header `metadata`, which is copied to every derived content |
| **RC-2** | **Resolved for the v2 attack**, with the problems below | Verified against the reviewer construction: per-object passes go from 714 GiB to `Scan` = 2 × 19.17 MB, charged before any pass. The per-object-charge mutant would hit ResourceLimit and the no-memo mutant would time out, so both are sensitive. The constant forms in steps 5–8 are correct, and `C` is never enumerated. Problems: new attacks DC-1, DC-3 and DC-4 lie outside §2a's formula; the formula's constant is wrong (DC-7); R7b is infeasible under default limits (DC-6) |
| **RC-3** | **Resolved** | `admit_root` returns `ImplicitRoot` only; there is no JsonRoot and no pre-read; C-5 is Unsupported; the `:90`/`:96` argument is correct (raw lines verified) |
| **RC-5** | **Resolved** | No 0x20-only rule; trailing bytes are admitted per C1 `json::parse`; D-8 and its mutant are present |
| **RC-6** | **Resolved** (one verification item) | R2 synthetic: L = 1, A = 6, branching 2, 63 subtrees, 6 on the live path, no payloads. A cap of 6S−1 flips "current subtree only", and 63S flips "cumulative". R2b window `[190,944, 246,583]` on W2. Verification item: my 190,944 is a Python sum of `bufferView.byteLength`, not C1's actual budget accounting. R must confirm C1 refuses W2 at 190,943 and accepts at 190,944 before relying on the window (DC-8) |

Glanced at:
- **RC-8.** X-1 to X-14 match my rule-by-rule spot check of `implicit/tileset.rs:662-1196`:
  - X-3 `:684` scheme;
  - X-3 `:869` JSON subtree;
  - X-4 `checked_pow`;
  - X-7 `:895-897` and `:909-911`.
  X-1 needs DC-2's bypass forms added after the fix.
- **RC-9.** The tolerance table is now correct:
  - absolute 1e-6 (`bbox.rs:382-389`) → scaled (`validate.rs:297`);
  - strict (`:766`) → `1e-8·(1+e)` (`:1312`);
  - R's parent check is stricter.
  T-1's choice of ‖h‖ ≥ 10 does separate the two rules.
- **RC-17.** The combined-PR argument is coherent:
  - today's validator refuses roots on explicit tiles (`:1280-1284`);
  - `store.rs:1006` cannot expand them;
  - I1 alone would make default outputs Unsupported.
  The only shared file is `src/implicit.rs` (line 20 for B, writer code for P), applied as separate commits.
- **RC-19.** Coherent:
  - #159 lands first;
  - W8m and W8s replace W8/W8u;
  - X-1 covers already-written multi-node archives;
  - the `VALIDATION.md:143` sentence goes to the coordinator on top of #159.

Raw-line conversions verified at `:90`, `:92-97`, `:116`, `:182`, `:231`, `:233`, `:299`, `:386`, `:392`, `:405` and `:770`.

---

## Delta changes

### DC-1 (blocking for A's §3 template code and for O): template length multiplies per-emission work

v3 resolves each emitted content, and each subtree, by substituting into the **raw** template and calling `uri()`.
RC-24 bounds the *number* of substitutions by `references`, but not their *length*.

`uri()` is linear in the template length, and `.` / `..` segments collapse. So an 8 MB template padded with `./` still
names a short, existing member.

**Measured** with the develop binary on an explicit tileset (`uri_probe/`, 3 runs each, wall time):

| Variant | Wall time |
| --- | --- |
| `content.uri = "./"×4,000,000 + "a.glb"` | 0.24–0.36 s |
| Same 8 MB string as unused `extras` (control) | 0.11–0.18 s |

So one `uri()` call on that template costs about **0.12–0.2 s**. It validates OK, so explicit content tolerates it.

**I1-admissible construction** (every cap holds):
- OCTREE, L = 6, tile `Const1` (37,449 tiles), one subtree, A = 6;
- one content slot `Const1`, with template `"./"×~4M + "c/{level}/../{x}/../{y}/../{z}/../a.glb"`. All variables
  are present, and every tile resolves to the single member `c/a.glb`, decoded once through the C1 cache;
- tileset under 8 MiB and 65,536 nodes; 37,449 visits; 37,449 references; one payload.

**Cost:** about 37,449 × 0.12 s ≈ **75 minutes**, and the subtree template can be padded the same way.

**Today** the 64 MiB presentation budget (`implicit/tileset.rs:1160-1168`) refuses this after about 8 nodes, and I1
removes that budget. This is a regression I1 would introduce.

**Required.**
1. **Normalize every content and subtree template once, at root admission.** Use `uri()` semantics, with `{…}`
   variables as opaque tokens: resolve `.`/`..` and empty segments, and percent-decode the static parts.
   - This is exact. Variables substitute only decimal digits, which can never form `.`, `..`, `/` or `%`, and can
     never complete or break UTF-8.
   - It requires one guard: any `%` escape within two characters before a `{` is Unsupported, **in every segment**,
     not only the last. Give it its own reason ("escape adjacent to template variable"); it is not a JSON verdict.
2. **Bound per-emission work.** Choose one, and have O mirror it:
   - (a) charge `Σ |normalized template with digits substituted|` per content and per subtree to the OD-10 counter,
     before substitution; or
   - (b) a profile constant on normalized template length (for example 1 KiB): longer templates are Unsupported at
     root admission.
   Producer templates are about 50 B.
3. **Add control R8**, the construction above. It completes within 5 s: Accepted under (a) for this size, or
   Unsupported/ResourceLimit as specified.
4. **Add mutant** "resolve the raw template per emission", which must time out against R8.
5. **Amend the §2a work formula and §7 Time** to include template bytes.

### DC-2 (OD-9; blocking for A's §3 and for O): detection must use the normalized path

v3's rule tests the raw last `/`-segment. `uri()` drops `.` and empty segments and pops `..`. Verified with the
transcribed `uri()` (`delta_attacks.out.json`):

| Template | v3 detects? | Resolves to |
| --- | --- | --- |
| `c/{level}-{x}-{y}-{z}.json/.` | no | `c/1-0-1-0.json` |
| `c/{level}-{x}-{y}-{z}.json/` | no | `c/1-0-1-0.json` |
| `c/x.json/{level}-{x}-{y}-{z}/..` | no | `c/x.json` (all variables present) |
| `c/{level}-{x}-{y}-{z}.JSON//./` | no | `c/1-0-1-0.JSON` |

**Effect.**
- The outcome becomes availability-dependent: an available slot reaches `payload_kind`, which gives Unsupported with
  a different message.
- A slot that is **never available is Accepted.** That contradicts "Unsupported at any availability, decided at root
  admission", and C-4's intent.
- No false InvalidInput results, and JSON is never followed, so this is a profile leak rather than a soundness hole.

**My view on OD-9.** Keep the static, admission-time rule, but apply step 5 (the `.json`, ASCII case-insensitive
test) to the **last segment of the DC-1 normalized template**. That is exact, and it fixes both the bypass and v3's
alternative's availability dependence. Steps 1–4 of v3's rule become part of the normalization (with the escape guard
from DC-1.1). Add the four forms above to C-7.

### DC-3 (blocking for A's §2a step 10 and for O): reserve the content index, do not just check it

v3 step 10 checks `E ≤ references − reference_visits` and then allocates. Entries are charged to `reference_visits`
only when they are emitted. So the live subtrees on one path each pass the check against nearly the full remaining
budget.

Worst case found (`content_index.out.json`): OCTREE L = 3, tile `Const1`, K = 2,500 `Const1` slots, with the `/..`
trick resolving each slot to one member. That gives 11 live subtrees, about **2,007,500 index entries live: 7.7×
`references`, about 16 MB.**

This contradicts §7's "at most 262,144 per validation" and §2a's "Σ E_s ≤ references".

**Required.**
- Make `E` a **reservation**, like `P_t`: `refs_remaining −= E` at admission. Specify how B's per-content
  `ArchiveResources::reference` charge consumes or releases that reservation, so that `reference_visits` is not
  double-counted and the `references` total keeps its meaning. O mirrors this.
- Then `Σ_live E ≤ references` holds, and the statements become true.
- Add a control: the construction above gives ResourceLimit at the second or third subtree's step 10. The mutant
  "check only" must allocate past it, observed through the R5b allocation probe.

### DC-4 (blocking for A's §5 and for O): classify classes once, not per table

§5 tests the Unsupported semantic set "when each table is admitted". The natural implementation scans the class's
properties for every table.

**Construction:**
- OCTREE, L = 1, child `Const1`, A = 6: 37,449 single-tile subtrees, each with one `tileMetadata` table (one opaque
  BOOLEAN column, `count` 1);
- class `c` with 32,668 opaque `{"type":"BOOLEAN"}` properties, which fits the tileset's 65,536-node and 8 MiB limits;
- about 19 MB of subtrees, 37,449 visits.

**Cost:** about **1.22·10⁹ property visits** (`delta_attacks.out.json`), which is not covered by the work formula.

**Required.**
- Classify each `(schema document, class)` once, on first use or at root admission. Record: the Unsupported-semantic
  flag, the interpreted descriptors, the duplicate-semantic check, and the required interpreted properties.
- Table admission is then O(1 + the table's own properties).
- Add control **R7c** (the construction above: Accepted within 5 s) and mutant "per-table class scan" (timeout).

### DC-5 (RC-1 gap): template header `metadata`

A content template may carry `metadata` (a metadata entity with a `class`). v3 makes it part of the copied header,
treated as opaque. A CONTENT_URI or CONTENT_GROUP_ID semantic there carries the same override hazard RC-1 closed for
`contentMetadata`.

**Required.** Apply the Unsupported semantic set to the class of any template header `metadata`, at root admission
(it costs O(1) with DC-4's per-class classification). Extend C-12 with that case.

### DC-6: R7b is infeasible under the default limits

Each distinct view needs at least 4 JSON nodes, and each referencing content layer needs at least 2 more, while each
tileset template needs 2. 65,536 views cannot fit in `document_items` = 65,536.

**Required.** Restate R7b as about K = 10,000 layers, each naming its own overlapping 19.17 MB view (about 6 nodes per
layer). `Scan` ≈ 192 GB is far above 2 GiB, giving ResourceLimit at step 2. Alternatively, state the raised
`document_items` and `json_bytes` explicitly.

### DC-7: the work-formula constant

"Each distinct source is read by at most four passes" is false:
- the tile source is read in step 3, steps 5–6, **step 7 once per distinct content source**, step 8, and step 11
  (rank);
- content sources are read in steps 3, 7 and 10 (extraction).

The total is still linear, because every content view has the tile view's length (both cover `T` bits), so the
step-7 re-reads are at most `Σ` content `Scan`. Restate the formula as `≤ 7·Σ Scan` (or `O(Σ Scan)`) with the pass
list. This is not blocking, but the text is labelled normative and R7's timing rationale rests on it.

### DC-8 (minor; fold in with the rest)

- **Property-table `count` has schema `minimum: 1`.** v3's `contentMetadata` rule (`count` = that slot's popcount)
  allows 0 for a slot that is empty in a subtree. State the outcome: InvalidInput per the schema, or
  `contentMetadata` index semantics for empty slots. Add a D-control.
- **R2b window.** R confirms C1's actual view budget on W2 (refused at 190,943, accepted at 190,944) before fixing
  the window.

### DC-9 (OD-10, my view): which counter takes the scan charge

**Agree with the recommendation:** `total_bytes_read`, shared. My v2 alternative (per subtree against
`document_decoded_bytes`) bounds only one subtree; across up to 65,536 subtrees it allows up to 4 TiB of passes.

Conditions:
- **The DC-1(a) template charge, if chosen, goes to the same counter.**
- **A distinct message.** For example "availability scan exceeds total_bytes_read work limit", not today's "member
  reads exceed 2 GiB work limit". It is never classified by message, but users read it.
- **Documentation.** R updates `docs/VALIDATION.md` (`:121`, "2 GiB explicit member-hash and …") to say the field
  now also bounds implicit scan (and template) work. `ValidationLimits` fields carry no doc comments, so `types.rs`
  needs no edit.
- **Settling evidence.** R records W2, W5 and W11 `Scan` sums (expected < 0.3 MB) next to the R7, R7a, R7b and R7c
  timings.

---

## What A and O may do now

- **A:** §1 container and typed facts; §2 rules and §2a steps 1–9, 11 and 12 (memoized sources, `Scan` charge,
  closed forms, reservation, rank); §4 effective volumes, including `DerivedRoot`; Morton, rank and walk skeleton.
  A waits for DC-1 to DC-5 text before writing template resolution, JSON detection, step 10, or §5 class handling.
- **O:** everything except the freeze. Draft the DC-1, DC-2, DC-3, DC-4 and DC-5 controls now, and fix R7b's numbers
  (DC-6).
