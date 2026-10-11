# I1 precode review (fresh nonauthor N): implicit effective-volume read owner, contract v2

Reviewer: N (wrote no part of I1, the contract, the probes or O).
Date: 2026-10-11.

Reviewed: `/tmp/rusty-tiles-i1-contract-20261011/`, which contains:
- `contract-draft.md` (v2);
- `open-decisions.md`, `ownership-plan.md` and `producer-inventory.md`;
- `probes/` and `primary-fetch/`;
- `v1/`, compared where relevant.

Source checked: `origin/develop` = `f2ed077eecf3…` in `/tmp/rusty-tiles-c2`. Cited files were exported with
`git show origin/develop:<path>` to `/tmp/i1-precode-review-20261011/src/`.

Primary text checked:
- `/tmp/rusty-tiles-a2-format-primary/3d-tiles/specification/{ImplicitTiling,Metadata/Semantics,.}/README.adoc`. The
  hashes equal the receipt (`14a37a34…`, `02ac07db…`, `b841fade…`).
- The `primary-fetch/` files.

Reviewer probes, all in `/tmp/i1-precode-review-20261011/probes/`, single-threaded `python3 -I`, with no cargo and no
repo edits:
- `w2_caps.py`: on `samples/pc-perf-2000.3tz` (W2), measures live-path subtree bytes against C1 payload view bytes.
- `alias_budget.py` → `alias_budget.out.json`: arithmetic for a time-amplification subtree, checked against every
  default limit.

No release binary was needed. The findings are static or arithmetic, and the two pinned binaries implement today's
expander, not I1.

Settled context, not relitigated:
- `convert-to-implicit` is retired (branch `retire/convert-to-implicit`, `b59c7b3`/`4b35dd7`, pending merge).
- The producers stop writing JSON content templates. I1's strict reading (JSON template → Unsupported) and its
  merge-after-P gate stand.

## Verdict: **GO-WITH-CHANGES**

The architecture is sound:
- an owner under `content_integrity` that drives a depth-first walk;
- symbolic exact `Derived` cells;
- override by `semantic`;
- strict LSB-first availability, Morton order and rank;
- live-path subtree accounting;
- no new `ValidationLimits` fields;
- a validator migration gated on P.

Most citations check out. The contract is **not yet codable as written**, for these reasons:
- **The profile has a correctness hole.** `CONTENT_URI` is missing (RC-1).
- **The bounded-work section can be defeated in time** under every cap, by aliased availability views (RC-2).
- **The acceptance plan has an unconstructible at-cap control** (RC-6).
- **The removal plan breaks retained code** (RC-10).
- **The exit-status table and one tolerance claim are wrong** (RC-8, RC-9).

All of these are contract-text fixes; none changes the design.

**Conditions for GO-WITH-CHANGES.**
- Fold RC-1 to RC-20 into v3.
- A focused N delta check of v3 must confirm RC-1, RC-2, RC-3, RC-5 and RC-6 before O freezes and before A codes §1,
  §2 and §7. This is a text check, not a new full review.
- RC-21 to RC-24 are recommended, not blocking.

---

## 1. Citations and the OD-1b reading

### 1.1 Citations verified (f2ed077 and pinned primary)

These match:
- `validate.rs`:
  - `:26-41` `content_limits`; `:56-62` `implicit_error`; `:158-210` `uri()` (the function body starts at `:165`);
  - `:490-500` `payload_kind`; `:518-529` `TileFact`; `:590-614` `build_levels`;
  - `:898`, `:1600-1604` `expansion_budget`; `:1122-1124` active set; `:1160` expand call;
  - `:1166-1168` root-only exclusion; `:1206-1217` retained sources; `:1280-1284` nested refusal;
  - `:1408-1415` extras comparison; `:1716-1730` `checks`; `:2550-2556` test.
- `implicit/tileset.rs`:
  - `:9` `LEVELS`; `:284-308` chunk trigger; `:449-451` link template; `:452-455` root;
  - `:473-602` `write_chunks`; `:606-617` `expand_tileset`; `:619-660` bounded expander and budget;
  - `:684`, `:690-691`, `:710`; `:718-735` `rustyTile` gating; `:759-772` placeholder check;
  - `:801-829` links; `:838-851` template; `:895-897` single buffer; `:912-916` `byteOffset` default;
  - `:1005` class gate; `:1048` extras JSON parse; `:1135` `REPLACE`; `:1160-1168` presentation budget.
- `convert_implicit.rs:205` (self-validate), `:404-421`, `:485-503`.
- `containment.rs:42-53` (`VolumeFacts` and `LevelFacts`, f64 only); `containment/dyadic.rs` (24 `pub(super)` items).
- `payload.rs:627-633`: `data.len() < n` is InvalidInput.
- ImplicitTiling, with filtered numbering:
  - `:72-76`, `:85`, `:104-126`, `:128-131`, `:158-162`;
  - `:171-207`, `:214-230`, `:249-255`, `:257-261`, `:560-589`.
- AVAILABILITY.adoc interleave examples. All five were reproduced with x as the least significant bit.
- Metadata Booleans: `(buffer[i/8] >> (i%8)) & 1`.
- Semantics: TILE_BOUNDING_BOX is SCALAR, FLOAT32/64, array, count 12. TILE_GEOMETRIC_ERROR is SCALAR, FLOAT32/64.
- `class.property.schema.json`: "The semantic cannot be used by other properties in the class".

Inaccurate, but not load-bearing:
- `validate.rs:1366` is the `Tileset` push. The JSON test is `name.ends_with(".json")` at `:1365`.
- `containment_work_facts` is at `:1513-1531`, not `:1494-1527`.
- Core `README.adoc:731-748` uses **raw** line numbers, while ImplicitTiling uses blank- and image-stripped numbers.
  State the convention per file.
- `Implicit3DTileContent.js:453` is unpinned. Pin it or drop it.

Wrong and load-bearing (see RC-4, RC-9 and RC-20):
- **"the same suffix role rule the validator uses for explicit content".** Explicit content uses a **case-sensitive**
  `ends_with(".json")` on the **percent-decoded** name (`:1365`). `.JSON` reaches `payload_kind` and becomes
  Unsupported "payload format outside GLB…". The outcome for `.JSON` agrees; the claimed rule does not.
- **"The tolerance is the same"** (§4, about replacing the expander's placeholder check):
  - the expander's check uses `Obb::contains(corner, 1e-6)`, an **absolute** 1e-6 (`bbox.rs:382-389`);
  - `contains()`/`inside()` use `1e-6·(1+‖half-axes‖)`, which is **scaled** (`validate.rs:297`);
  - the expander's level-0 error check is strict (`> root_error`, `:766`);
  - `geometricErrorOrder` allows `1e-8·(1+e)` (`:1312`).
- **perf `process.py` pin `513bec79…`.** This is commit `bb7d054`'s blob. The `/tmp/rusty-tiles-perf` worktree copy
  is modified, with sha `056bb1d8…`. The change is only `--explain` argument validation, but R must run the
  committed blob.

### 1.2 OD-1b: is InvalidInput for an available level-0 JSON slot mandated?

**No. Recommend uniform Unsupported (RC-3).**

- **The text supports it as a violation, but does not mandate the narrow reading.**
  - `:75` combined with `:130` does mean that a level-0 tile whose content is an external tileset violates a
    "shall not".
  - But `:71` (verified) says: "For content, the template URI is specified in the tile's `content.uri` property."
    So the subject of `:75`'s `content.uri` *is the template*.
  - The most literal reading is therefore that the template itself shall not point to an external tileset, at any
    availability. That reading makes every JSON template invalid, and the user deliberately chose not to give that
    verdict.
  - The contract's distinguishing sentence, "A template is not a single `content.uri` value", contradicts `:71`.
- **Level 0 has no textual privilege.** The level-0-only InvalidInput is a middle position that the text does not
  single out.
- **Unsupported is never a false verdict.** It is an under-claim, so nothing obliges I1 to issue InvalidInput here.
- **The cost is real and partly unspecified.**
  - It needs a second API (`RootAdmission::JsonTemplates`/`JsonRoot::check_level0`).
  - It needs a subtree read outside `Walk`.
  - Whether that read takes the visit reservation and the live-path charge is **unspecified**, so ResourceLimit
    versus Unsupported ordering is undefined for a large root subtree.
  - It needs an extra precedence step, two controls (C-4, C-5) and one mutant.
- **The benefit is nil in practice.** No producer sets a level-0 JSON bit (inventory: "never available at level 0").
  After P, no producer writes JSON templates at all.

---

## 2. Invariants against the spec texts

Correct as specified:
- the header, version and 8-alignment rules;
- internal and external buffers, and data URIs refused (`:252`);
- `byteOffset` required (bufferView schema `required`) and 8-aligned (`:255`);
- exact `⌈bits/8⌉` bitstream length (`:254`: "byteLength does not include any padding"), with trailing bits zero
  (`:261`);
- the `constant` rules; root bit 0 (follows from `:194` and the parent rule);
- parent, content ⇒ tile and child ⇒ tile at L−1;
- `availableLevels` bounds;
- Morton order with x as the LSB; level offsets; local coordinates;
- template variables (`:161`); `{z}` in a QUADTREE template;
- the derived-cell formula (centre `c + Σ a_k((2i_k+1)/2^l − 1)`, half-axis `a_k/2^l`, QUADTREE axis 2 unchanged per
  `:80`);
- error `e_R/2^l` from the root (`:118`, `:123`; CesiumJS does the same);
- strict rank (`:217`, `:221`);
- semantic uniqueness;
- omitted optional properties fall back (Semantics `:142`).

Missing or wrong:
- **`CONTENT_URI` and `CONTENT_GROUP_ID` are not excluded.** Semantics `:293`: `CONTENT_URI`, "Overrides the
  implicit tile's generated content uri". I1 treats `contentMetadata` as opaque except for bounds and height
  semantics. With `CONTENT_URI` present, I1 then:
  - (a) resolves the *template* member instead of the real one. If the template member is absent, the result is a
    **false InvalidInput** on a valid archive. If it is present, the "resolves to a payload member" claim is about
    the wrong member, and the real members fall to the unreferenced-entry InvalidInput;
  - (b) lets a `CONTENT_URI` row name `x.json`, which **bypasses the static JSON exclusion**. The claims "implicit
    tiles can no longer reach JSON content" and "never JSON content" are then false.

  RC-1.
- **The JSON chunk "trailing padding is 0x20 only" rule is not mandated, and gives false InvalidInput.**
  `jsonByteLength` is a multiple of 8, so a chunk whose JSON text ends in `\n` (or other JSON whitespace) with zero
  padding is valid. The text cannot be split into "JSON text" and "padding" in any way that forbids that. Today's
  C1-backed parser accepts it. RC-5.
- **Whole-tree content rule (`:198`, last sentence).** "If there are no tiles with a content resource, then
  `tile.content` and `tile.contents` shall be omitted." This is neither enforced nor listed as unchecked. It can only
  be checked after the walk. RC-15.
- **Schema `minItems: 1`** on `buffers`, `bufferViews`, `propertyTables`, `contentAvailability` and `contentMetadata`
  is not stated. RC-15.
- **The suffix test runs on the raw template.** A template such as `…/{level}-{x}-{y}.js%6Fn` escapes static
  detection, and C-7's `?v=1` case passes only through `uri()`. RC-4.
- **Row count for content metadata.** "count equals that layer's popcount" is correct per `:233`. Under RC-2 the
  popcount must be memoized per view.

---

## 3. Bounded work: adversarial constructions

**3.1 The time cap is defeated by aliased availability views (RC-2).**

Nothing forbids several availability objects naming the same `bufferView`. `:254` says "There may be multiple
buffer views referencing a single buffer", and views may likewise be shared.

§2 checks, per content layer and in O(view bytes):
- `availableCount` (popcount);
- trailing bits;
- content ⇒ tile.

Precedence puts all of §2 **before** the visit reservation.

Construction (`probes/alias_budget.out.json`):

| Parameter | Value |
| --- | --- |
| Tiling | OCTREE, `subtreeLevels` 10, `availableLevels` 1 |
| Tile bitstream | 19,173,962 B; only bit 0 set (popcount 1, so the reservation is 1) |
| Content layers | K = 20,000 `{"bitstream":1,"availableCount":0}`, all naming one all-zero view |
| Subtree JSON | 700,280 B, 60,019 nodes (8 MiB and 65,536 limits: OK) |
| Tileset `contents` | 40,001 nodes (OK) |
| Subtree member | 39.0 MB (64 MiB `member_bytes` and live path: OK) |

- **Contract-prescribed work: about 714 GiB of byte scanning** before any cap can trip. That is minutes to hours.
- **Per-tile slot scan.** Even without large views, each visited tile tests K layers. That is
  `hierarchy_visits × K ≈ 65,536 × 20,000 = 1.3·10⁹` tests.
- **The time line in §7 is therefore false.** It reads "O(subtree bytes read + tiles visited + JSON items + …)".
- **Today's validator does bound this case.** Its `slots × (templates+2) ≤ max_bytes` budget (`:700-703`) refuses
  it, and I1 deletes that budget.

**3.2 Constant availability needs closed forms (RC-2).**

§7 says "Constant child availability enumerates only set bits". For `childSubtreeAvailability.constant = 1` that is
all `N^L` bits, which is 2^93 at L = 31. These checks run before the reservation:
- child ⇒ tile at L−1;
- the `availableLevels` bound;
- the matching tile rules.

So they must be stated in closed form (for example, "all of level L−1 available" via an O(view) range check). Child
subtrees must be enumerated only lazily during the walk, after the reservation.

**3.3 Memory beyond the live-path byte charge (RC-11).**

- **Parsed JSON.** With L = 1, up to 32 live subtrees, each with ≤ 65,536 JSON nodes (unknown properties are parsed,
  then ignored). Retaining the parsed `Value`s holds about 200 MB that is **not charged**. The bytes themselves are
  charged.
- **Rank tables.** Up to half of the live bytes, not charged.

The contract's "Memory held" omits both. Either retain only compact typed facts per live subtree and drop the parsed
JSON before descending, or state the bound.

**3.4 What holds.**
- **Live-path depth.** It is at most ⌈32/L⌉ subtrees, 8 at L = 4, and it is deterministic. Release is well defined
  under owner-driven pre-order.
- **Live-path check before the read.** `read_member` refuses `size > maximum` with ResourceLimit before reading
  (`validate.rs:126-145`), so passing `max = min(member_bytes, live remaining)` enforces the cap ahead of the read.
- **Subtree count.** Each admitted subtree reserves ≥ 1 visit, so the count is at most 65,536.
- **Bytes read.** Total subtree bytes are at most `total_bytes_read`. OD-7 forbids rereads.
- **Rank table.** At most view/2 bytes, allocated with `try_reserve_exact`.
- **Visits.** Reserving the popcount gives the same total as per-visit charging for every valid archive.

**3.5 Determinism for O (RC-7).**
- **R1 needs the exact visit formula.**
  - `task_visits` charges one per `Tileset`, `ExitTileset` and explicit `Node` task (`validate.rs:1101-1110`).
  - Under I1 it also charges the popcount per admitted subtree, with R (a Node task) and I_0 (popcount bit 0) each
    charged once.
  - For W2 this gives 1 + 1 + 1 + 428 = 431.
  - O "never reads src", so the formula must be in the contract.
- **R3 needs `availableLevels` stated.** Constant-1 tiles with A < L are InvalidInput (§2) before the reservation,
  so `availableLevels` ≥ L is required. For L = 32 that is A = 32 with a constant-0 child.

---

## 4. Interface to C2 slice 3, and the symbolic `Derived` case

For **row-only** archives, slice 3 can indeed stream implicit references with `containment.rs` untouched:
- `Row(b)` → `VolumeFacts::Box(b)` exactly, with FLOAT32 widening exactly;
- I_0 → R carries `to_parent: None`, and R → parent carries `R.transform`. This is correct, because V_R and V_0 are
  both in R's frame.

The contract still leaves these open:
- **RC-12(a). The I1 PR's arena treatment is unspecified.**
  - `TileFact.volume` is a non-optional `VolumeFacts` (`validate.rs:518-529`), and `Derived` "has no
    `VolumeFacts`".
  - So either B does not register implicit tiles (I_0 and below) in the C2 tile arena in the I1 PR, or `TileFact`
    gains an optional volume.
  - The OD-3 rounded box must **not** fill it.
  - Recommendation: register nothing below R in the I1 PR. The archive is excluded anyway.
- **RC-12(b). "Slice 3 only stops setting `implicitTiling` and registers references" is incomplete.**
  - Under C2's I3 (all or nothing), an archive with any `Derived` tile still needs an exclusion.
  - That needs a new `Reason` (`validate.rs:369`), which is a frozen-enum delta in the C2 contract, or an exact
    predicate.
  - The "only" list should name this.
- **RC-12(c). C2's documented level-chain bound breaks.**
  - The C2 contract's B3 states "at most 129 levels".
  - I_0 is a separate fact at depth(R), so an implicit tile at depth d has a chain of d + 2 levels, up to 130.
  - `try_push` is fallible, so this is not unsafe, but the C2 bound statement must be amended (+1 per implicit root
    on the path).
- **Arena size is still bounded.** R and I_0 are each charged one visit, so the C2 arena stays at most
  `hierarchy_visits`.

---

## 5. Independent acceptance plan

- **Independence holds by construction.** O is stdlib Python, reads no `src/`, and uses the P1 oracle for Morton
  layout only.
- **Parity cannot run from an integration test.** The `#[cfg(test)] pub(crate) implicit_tile_facts` function cannot
  be called from `tests/i1_implicit_read.rs`. C2 runs `containment_work_facts` from unit tests inside `validate.rs`
  (`:2746`, `:2847`). RC-16.
- **R2 cannot be built on W2 (RC-6).**
  - `document_decoded_bytes` is also C1's per-payload view budget (`payload.rs:644-650`, `validate.rs:33`).
  - W2 measurements: maximum live-path sum **23,864 B**; cumulative 246,584 B; largest payload view sum
    **190,944 B**.
  - At the R2 cap, every payload fails with ResourceLimit first.
  - R2b is feasible on W2, for example with a limit of 200,000 B.
  - W2's path is only 2 subtrees deep, so it never exercises the ⌈32/L⌉ path.
- **Mutant pairing gaps (RC-14).**
  - "Reservation after allocation, or none" is paired with R2 and R3. R3 uses **constant** availability, which
    allocates no rank table, so the after-allocation mutant does not flip R3. A per-visit-charge mutant gives the
    same totals, so it does not flip R1.
  - No mutant charges only the current subtree, ignoring ancestors.
  - There is no control for `CONTENT_URI`, aliasing or `\n` padding.
  - A-1 and A-2 are not single-defect on producer data. Flipping a tile bit also breaks `availableCount`, the row
    `count` and possibly content ⇒ tile. Specify the co-adjustments, or which failure is expected first.
- **The perf obligation is only meaningful in the amplified case (RC-18).**
  - The floors are an absolute 0.1 s or 16 MiB, so every I1 case of about 0.1–0.2 s can double without being
    flagged.
  - The amplified case binds only if its base best wall time is ≥ 0.4 s (1.25 × base − base > 0.1 s).

---

## 6. Validator migration and the report

- **Report changes.** These are stated exactly and are consistent with today's code:
  - `checks` gains `implicitAddressingAndAvailability` after `hierarchyBounds`;
  - the label leaves `notInspected` (a BTreeSet, so ordering is stable);
  - counts are unchanged for non-chunked output;
  - retained vector sources move unchanged;
  - explicit-only archives are unchanged.
- **The OD-4 label must be made exact (RC-13).** The record must say exactly when
  `recordedBudgets.implicitTileExtras` is added: every archive with an implicit tile, or only those whose rows carry
  extras. It must also say that the report-level `budgets.bytes`, `budgets.vertices` and `maxPoints` still apply per
  implicit tile.
- **The exit-status table claims to be complete and is not (RC-8).** Missing rows:
  - JSON-format subtrees: today InvalidInput ("invalid binary subtree header", `tileset.rs:869`) → Unsupported.
  - `subtreeLevels > 32`: today ResourceLimit (`checked_pow`/`max_nodes`) → Unsupported.
  - `extensions` in subtree JSON: today ignored, so success → Unsupported.
  - V_0 ⊆ V_R: absolute 1e-6 → scaled tolerance, so InvalidInput → success for large roots. I_0 error against e_R:
    strict → 1e-8 relative.
  - V_R and e_R are now checked against R's explicit parent. Today the expanded root carries V_0, so a V_R escaping
    the parent of an external-tileset implicit root goes success → InvalidInput.
  - `CONTENT_URI` and `CONTENT_GROUP_ID` → Unsupported (after RC-1).
  - Archives that `convert-to-implicit` already wrote with multiple nodes: success → Unsupported. The retire branch's
    `docs/VALIDATION.md:143` says such archives keep validating.

---

## 7. Ownership and sequencing

- **B's removal breaks retained code (RC-10).**
  - B is to remove `implicit/tileset.rs:619-660`: `expand_tileset_bounded`, `ExpansionBudget`, `producer_json` and
    `expansion_limit`.
  - The retained `expand_tileset` (`:610-615`), `expand_document`/`Reader` (`:667-1168`) and
    `mod expansion_budget_tests` (`:1199+`) all use them.
  - Smallest coherent change: B removes only the validator's use and the `pub(crate) use` at `implicit.rs:20`. The
    bounded expander stays as `expand_tileset`'s private implementation.
  - B then never edits `implicit/tileset.rs`, and the P-overlap sequencing for that file disappears.
- **P's merge order depends on the layout P chooses (RC-17).**
  - If P chooses implicit roots on explicit tiles, today's validator rejects that layout (`validate.rs:1280-1284`).
  - Today's `expand_tileset`, used by `vector/pipeline/store.rs:1006` for `maximumTileBytes`, also cannot read it
    (`expand_document` handles only a manifest-root implicit root).
  - So for that layout "P merges first" is impossible; the combined PR is mandatory, and P owns the `store.rs`
    consumer.
  - The ownership plan should say so, so that the merge-order options are not presented as equivalent.
- **Convert retirement is not yet folded in (RC-19).** The OD "New: disposition" item, the gate, W8/W8u, the
  `tests/convert_implicit.rs` references, the disposition table rows and the exit-table wording still describe an
  open choice.
  - The retire branch also edits `src/validate.rs` (comment only) and `docs/VALIDATION.md:143`. B and R rebase after
    it.
- The rest is coherent:
  - A and O can start now.
  - B may write on a branch but merges with or after P.
  - W11 is a merge gate.
  - Before = P head without I1.

---

## Required changes

**RC-1. Content semantics.**
- `CONTENT_URI` in any `contentMetadata` class is **Unsupported**. Add `CONTENT_GROUP_ID` too, or justify why it is
  opaque.
- Add the semantics to the Unsupported list, the precedence step (root or subtree admission) and the exit-status
  table.
- Add control C-12 (a `CONTENT_URI` column, including one naming `.json`: Unsupported) and its mutant.

**RC-2. Bounded time for availability (§2, §7).**
1. Group availability objects by source (`constant 0` / `constant 1` / view index).
2. Compute popcount, trailing bits and content ⇒ tile **once per distinct view**, so the work is O(Σ distinct view
   bytes).
   - Alternatively, charge Σ (view `byteLength` over all availability objects, with aliases counted again) ≤
     `document_decoded_bytes` before any bit pass, giving ResourceLimit.
   - Choose one, and have O mirror it.
3. Per visited tile, test only non-constant-0 groups. Then per-tile work is at most (distinct views × T) plus
   emitted contents, which are charged to `references`.
4. Evaluate every constant-availability rule in closed form, before the reservation. Enumerate child subtrees only
   lazily in the walk.
5. Correct the §7 time statement.
6. Add resource control **R7**, the `alias_budget` construction (K ≈ 20,000 aliased layers, 19 MB view). It must
   finish within a stated wall bound, or reach ResourceLimit if the charge variant is chosen. Add a paired "no
   memoization" mutant.

**RC-3. OD-1b: adopt uniform Unsupported.**
- Delete `RootAdmission::JsonTemplates`/`JsonRoot::check_level0`, the root-subtree pre-read and the extra precedence
  step.
- C-5 becomes Unsupported. Drop the "Level-0 JSON as Unsupported" mutant.
- Replace the "OD-1 resolution" argument: `:71` makes `content.uri` the template, so the text gives no level-0-only
  rule, and the profile gives no verdict on JSON templates at any level.
- If the coordinator keeps InvalidInput anyway, the contract must:
  - specify whether the JsonRoot subtree read takes the visit reservation and the live-path charge;
  - fix its precedence against ResourceLimit;
  - still correct the `:71` argument.

**RC-4. Suffix rule.**
- Define detection as: strip `?`/`#`, percent-decode the last path segment exactly as `uri()` does, then test
  `.json` ASCII case-insensitively.
- Correct the claim that this is "the same rule" as explicit content (`:1365` is case-sensitive on the decoded name).
- Add a `%6A`-encoded case to C-7.

**RC-5. JSON chunk padding.** Remove "trailing padding is 0x20 only". Trailing bytes after the JSON value are
admitted when C1 `json::parse` admits them, as for GLB. Add a control: `\n`-terminated JSON with zero padding is
Accepted.

**RC-6. R2 must be constructible.**
- Run R2 and R2b on a **content-free (or tiny-payload) synthetic** fixture whose maximum live-path sum exceeds every
  payload's C1 view sum, with a path of at least 3 subtrees (for example L = 1 or 2).
- Keep R2b on W2 too (limit ∈ [190,944, 246,583]).
- State in §7 that the live-path cap shares `document_decoded_bytes` with C1's per-payload view budget.
- Add the mutant "charge the current subtree only" and pair it with R2.

**RC-7. Determinism for O.**
- State the exact `hierarchy_visits` formula: one per `Tileset`, `ExitTileset` and explicit `Node` task, plus the
  tile popcount per admitted subtree. R is a Node task and I_0 is in the popcount. W2 gives 431.
- State `availableLevels` for R3 (A ≥ L; L = 32 with A = 32 and constant-0 children).
- State that O's verdict domain excludes C1 payload integrity, so controls keep payloads valid.

**RC-8. Exit-status table.** Add the rows listed in §6 above. Then either remove "The list is complete" or
re-derive the table against `implicit/tileset.rs:860-1060` rule by rule.

**RC-9. Tolerances.**
- Replace "The tolerance is the same" with the actual change: absolute 1e-6 in R's frame → `inside()`'s scaled
  1e-6·(1+‖half-axes‖), and level-0 error strict → `geometricErrorOrder`'s 1e-8 relative.
- Record both in the exit table, with a control at each boundary.

**RC-10. Removal plan.** Restrict B's removal to `validate.rs`'s use (`:56-62`, `:898`, `:1203-1263`,
`:1600-1604`) and `implicit.rs:20`'s `pub(crate)` re-export. Leave `implicit/tileset.rs` untouched by I1, and update
the disposition and file-ownership tables.

**RC-11. Memory statement.**
- Specify that each live subtree retains only typed facts (views, grouped availability sources, table offsets) plus
  its bytes and rank table. The parsed subtree JSON is dropped before descending.
- Restate "Memory held" as: live bytes ≤ `document_decoded_bytes`, plus rank tables ≤ half of that, plus O(K + views)
  facts per live subtree.

**RC-12. C2 interface text.**
- (a) In the I1 PR, B registers no C2 `TileFact` below R, and the OD-3 rounded box never enters `TileFact`.
- (b) Add the "`Derived` present" exclusion reason, or the exact predicate, to slice 3's "only" list as a frozen-enum
  delta.
- (c) Note the +1 level per implicit root against C2 B3's 129-level bound, and have slice 3 amend it.

**RC-13. Record OD-4 exactly.** State exactly when `recordedBudgets.implicitTileExtras` appears, and that the
report-level budgets apply per implicit tile.

**RC-14. Mutant sensitivity.**
- Pair "reservation after allocation" with a **bitstream** tile-availability fixture whose popcount exceeds the
  remaining visits, observed through R5's allocation-refusal injection or a test-only allocation probe.
- Pair "per-visit charging instead of reservation" with **R1b**: `hierarchy_visits` one short, plus a corrupt payload
  in the first tile of the last subtree. Reservation gives ResourceLimit; per-visit gives InvalidInput.
- Make A-1 and A-2 single-defect by specifying the co-adjusted `availableCount`, row `count` and content bits.

**RC-15. Missing structural rules.**
- Enforce `:198` ("no tile has content ⇒ `content`/`contents` omitted") as a post-walk InvalidInput, or list it under
  proof limits.
- Enforce schema `minItems: 1` on the five subtree arrays.

**RC-16. Test placement.** The parity tests that call `implicit_tile_facts` live in `#[cfg(test)] mod tests` in
`src/validate.rs`, as C2 does. `tests/i1_implicit_read.rs` covers CLI and public-API controls only.

**RC-17. Sequencing.** State that a P layout with implicit roots on explicit tiles forces the combined PR. Today's
validator (`:1280-1284`) and `expand_tileset` (used by `store.rs:1006`) cannot read it, and P owns that consumer.

**RC-18. Perf.**
- Pin the lane scripts by commit `bb7d054` plus blob hashes, not by the dirty worktree.
- Require the amplified case's base best wall to be ≥ 0.4 s, so that the ratio rule can bind above the 0.1 s floor.

**RC-19. Fold in convert retirement.**
- Gate: `retire/convert-to-implicit` is merged before the I1 merge.
- Remove W8/W8u, the OD "New" item, the `tests/convert_implicit.rs` references and the "refuses to publish" wording.
- Add the exit row for already-written multi-node archives.
- R rewrites the retire branch's `docs/VALIDATION.md:143` sentence.

**RC-20. Citation hygiene.**
- Fix `:1366` → `:1365` and `:1494-1527` → `:1513-1531`.
- State the line-numbering convention per primary file.
- Pin or drop `Implicit3DTileContent.js:453`.

### Recommended (not blocking)

- **RC-21.** Expose the parent as the path level in `VisitedTile`. B's parent stack is naturally level-indexed in
  pre-order, which avoids storing every visited tile in a map keyed by walk index.
- **RC-22.** Report `Derived` at level 0 as a distinct `derived-root` tag in O records rather than `Row(V_R)`. Then
  B-4 on a level-0-only tileset still shows a facts change.
- **RC-23.** Add a control for a `required` interpreted property missing from the table (InvalidInput), and one for
  an optional property omitted (falls back to `Derived`).
- **RC-24.** Once RC-2 is adopted, state that template substitution happens only for available contents (it already
  does). That makes Σ template length bounded by `references`.

## Files

- `/tmp/i1-precode-review-20261011/review.md` (this file)
- `/tmp/i1-precode-review-20261011/probes/w2_caps.py`
- `/tmp/i1-precode-review-20261011/probes/alias_budget.py`, `alias_budget.out.json`
- `/tmp/i1-precode-review-20261011/src/` (f2ed077 exports used for the checks)
