# Validator limits surface: amendment VS-C

Status: author amendment for validator-scale slice VS-C, written before the code. The
nonauthor final-source review and the coordinator's gate are still pending. Nothing here
records acceptance.

This note amends the limit parts of the C1 validator
([c1-payload-integrity-contract.md](c1-payload-integrity-contract.md), which stays
byte-identical because the C1 production-control driver pins its hash), the
[archive read foundation](archive-read-foundation-contract.md) (A-C1-1, A-C1-2), the
[C2 contract](c2-decoded-containment-contract.md) (A-C2-1, A-C2-3), the
[I1 contract](i1-implicit-read-contract.md) (A-I1-1), the
[VS-A catalog amendment](c1-archive-catalog-scale-amendment.md) and the
[VS-B traversal amendment](validator-traversal-scale-amendment.md). If the texts disagree, this
note governs. Plan: `~/.cache/rusty-tiles-evidence/vs-c-plan-20261014/plan.md` with the
coordinator's answers; design: `validator-scale-design-20261013/design.md` §3 and §4.

## Defect

After VS-B the default budgets scale with the archive, but four absolute volume caps do not:
`sourceArchiveBytes` (1 GiB), `archiveStoredBytes` (1 GiB), `archiveEntries` (2^19) and
`centralDirectoryBytes` (32 MiB). A well-formed 1.4 GB tiler output is refused with
`resource_limit: source archive exceeds byte limit`. No surface lets a user tighten or set a
limit, the report does not say how a limit was derived, the tmpfs scratch refusal of
`validate_3tz` is an untyped `io` error, budget refusals are untyped strings, and a validation
cannot be cancelled.

## Terms

- **S**: the sum, over every admitted central-directory record (the `@3dtilesIndex1@` index
  included), of its stored size. It is fixed after catalog admission and before the hash pass;
  nothing found later changes it. Every byte of S is hashed.
- **Model**: the rule table of VS-C-1 (floors and per-stored-byte ratios), versioned as
  `limitDerivation.model`.
- **Override**: a user-given value for one of the eleven configurable keys (Class B or C).

## Invariants

**VS-C-1. Three classes.** Every key of `limits` belongs to exactly one class.

*Class A, per-item format limits.* Fixed; never overridable (a user may not loosen them). Always
numbers.

| key | value |
| --- | --- |
| `jsonBytes` | 8,388,608 |
| `jsonDepth` | 64 |
| `memberBytes` | 134,217,728 (the index member is exempt) |
| `accessorElements` | 4,000,000 |
| `documentDecodedBytes` | 134,217,728 |
| `documentItems` | 131,072 |
| `hierarchyDepth` | 128 |
| `containmentSpanBits` | 4,096 |
| `containmentNodeDepth` | 64 |
| `liveDocumentBytes` | 268,435,456 (VS-B-7; now serialized) |

*Class B, absolute volume caps.* Opt-in: unset by default, serialized as `null`.
`sourceArchiveBytes`, `archiveEntries`, `centralDirectoryBytes`, `archiveStoredBytes`.

*Class C, amplification budgets.* The effective value is the override when one is given;
otherwise `max(floor, ⌊S·p/q⌋)`, computed in `u128` with floor division and saturating at
`u64::MAX`.

| key | floor | p/q | first S above the floor |
| --- | --- | --- | --- |
| `hierarchyVisits` | 262,144 | 1/8 | 2,097,160 |
| `references` | 1,048,576 | 1/8 | 8,388,616 |
| `totalBytesRead` | 2,147,483,648 | 4/1 | 536,870,913 |
| `totalPayloadElements` | 16,000,000 | 77/1 | 207,793 |
| `containmentWorkUnits` | 536,870,912 | 128/1 | 4,194,305 |
| `containmentBytesRead` | 2,147,483,648 | 4/1 | 536,870,913 |
| `containmentDecodedBytes` | 2,147,483,648 | 16/1 | 134,217,729 |

This is model 1. κ (the ratios) stays provisional; VS-D freezes it and bumps the model when a
ratio or floor changes.

**`totalPayloadElements` ratio, measured basis (2026-10-30, still model 1).** The ratio rose
from 16 to 77. Two things changed together. HLOD coarse tiles became indexed: their validation
work is unchanged, but their stored bytes fell. And the native HLOD delivery check now charges
its scratch work at its measured cost, so a unit costs about the same in every work class:

- each SQLite scratch-store row written, and each ancestor-walk or placement-frame lookup:
  128 units;
- each seek of a keyed lookup into a complete leaf table (descendant incidences, edge faces and
  coincident faces of a coarse tile, a coarse drop's border contacts): 320 units, and each further
  row it reads: 32 units. Coarse ancestors read the same descendant rows again, so these are
  charged per row, not per write;
- each original vertex row appended to the ingest spill sequence: 16 units;
- each placed original key (placement, external sort and ordered alias scan): 32 units.

The original-vertex alias check no longer goes through SQLite. Its rows are appended to a spill
sequence, and their placed keys are checked by one bounded external sort (the catalog's side-sort
memory) and one ordered scan. This cut the hostile leaf class c5 from 46.9 s to 15.2 s of CPU.

*Constant-cost scratch rows (2026-10-30).* A constant charge per row needs a constant cost per
row. Random-key B-tree inserts do not have one: once a table outgrows SQLite's 8 MiB page cache,
each insert reads and writes back pages that miss it, more often as the table grows. So no row
is inserted out of key order any more:

- Rows whose keys arrive in payload order go to one bounded external sort at a time (the
  catalog's side-sort configuration, the alias pass's memory bound) and are loaded in primary-key
  order, so every insert appends. These are the leaf points, segments and faces, ancestor
  requirements, coarse border images, coincident-face twins and multi-edge leaf faces. Rows with
  one key are merged as the former upserts merged them (points once, incidences summed with the
  same overflow refusal, faces counted, directions joined), so every table holds what it held.
- Two lookups per row became merge joins in key order: the multi-edge filter of the edge-face
  replay, and the open-segment census's shared-endpoint checks (lower endpoints in scan order,
  higher ones after a sort).
- Secondary indexes on references, tiles and placements are built once, by sort, after their rows
  are in.
- The lookups that remain read tables that are complete. A coarse tile's descendant incidences,
  edge faces and coincident faces, and a coarse drop's border contacts (now one seek per side of
  the tile's range), are charged 320 units per seek and 32 per further row. Every row such a
  lookup reads is a row it charges: edge faces are indexed by (key, direction, tile, third
  corner), so a lookup of one direction reads none of the other's rows. That matters because each
  coarse ancestor repeats it, up to 127 of them. Ancestor walks (the tiles table, mostly hot)
  stay at 128 per step.
- A seek into a large leaf table reads pages the 8 MiB cache does not hold, so its cost grows
  slowly with the table. Measured on the P-core (random keys into a bulk-loaded `leaf_segments`,
  200,000 seeks each):

  | rows | table | seek, key present | seek, key absent | ns/unit at 320 + 32 per row |
  | --- | --- | --- | --- | --- |
  | 1 M | 39 MB | 1.85 µs | 1.69 µs | 5.2–5.3 |
  | 4 M | 159 MB | 2.30 µs | 2.14 µs | 6.5–6.7 |
  | 16 M | 642 MB | 2.60 µs | 2.37 µs | 7.4 |
  | 64 M | 2.6 GB | 3.02 µs | 2.87 µs | 8.6–9.0 |

  A least-squares fit gives 0.38 µs per fourfold growth. A 1 GB archive's budget writes at most
  77·S/128 ≈ 600 M rows, where a seek costs about 3.44 µs (key absent) to 3.58 µs (present):
  10.2–10.8 ns per unit at 320 units. The weight is set so this extrapolation stays within the
  P-core τ; at 256 units it would be 13.4 ns. At the former 128 units the measured seeks cost
  11.5 to 22.4 ns per unit. The load of
  those rows (sort and append) cost 0.93 µs per row at 1 M rows and 1.12 µs at 64 M, 7.2 to 8.7
  ns per charged unit.
- The D4 coarse-identity check compared every reference with every descendant reference, up to
  `hierarchyDepth` × `references` index steps, charged only at one unit per 10,000 SQLite VM
  operations. It is now one preorder pass that keeps each open tile's lowest and highest
  descendant leaf rank, charged one unit per reference.
- What SQLite still does uncharged is linear in rows charged when they were written: scans,
  sorts and index builds over them, and one seek per tile or reference. One unit per 10,000 VM
  operations stays as a backstop.

Verdicts are unchanged. Validation reports are identical, field for field, except two things:

- `observed` in a budget refusal, the running count at the refusing charge, moves by a few units;
- `scratch.peakBytes` includes the sort runs (tex3 and crop3: 254 to 310 MB).

Charges change only for the coarse lookups and the D4 pass, and by SQLite progress ticks:

- On large archives the change is small (Walem +0.009%).
- On small archives dominated by coarse lookups it is large. Over review 1's 154 adversarial
  validations (coincident and equal-direction groups under chains) the median is +0.003% and the
  largest +54% (12,876 to 19,872 units).
- All of those archives charge under 700,000 units, far below the 16,000,000-unit floor, so no
  default verdict changes.
- At an explicit budget that small (`--maxVolume`), an archive that was admitted, or refused for
  its content, can now be refused on budget. The reverse happens too, from develop's D4 progress
  ticks.

Work per stored byte (all C1, identity and delivery work against this budget) was logged for
every validation the lib and integration suites run (5,247), and for the archives below:

| archive | develop | indexed coarse |
| --- | --- | --- |
| border-control sheet (n = 80) | 30.34 | 36.52 |
| mesh-1e6 | 19.44 | 21.96 |
| tex3 | 14.62 | 16.74 |
| crop3 | 12.08 | 15.06 |
| Walem 2048 (dabca18b) | 13.85 | – |

The largest value over every logged validation, small archives included, is 50.75 (the
`plain` suite archives, 144,339 stored bytes, which stay under the floor). The largest above the
floor is the sheet's 36.52. The ratio is ⌈1.5 × 50.75⌉ = 77.

The constant-cost scratch rows leave this basis unchanged. Logged again with them, the named
archives' work moves by at most 0.01%. Walem gains 1,078,379 of 11.47 G units and tex3 and crop3
gain 19,543, for coarse lookups charged per seek and per row. The sheet and mesh-1e6 lose up to
3,449 (fewer SQLite progress ticks). The largest value over
5,546 logged suite validations is 50.74 (the `plain` archives, 144,389 stored bytes). The ratio
stays 77.

Two validator builds now report `limitDerivation.model: 1` with different ratios: 16 before
this amendment and 77 after it. κ stays provisional under model 1, and only VS-D freezes it and
bumps the model. Until then, read the ratio from `perStoredByte`, not from the model number.

Hostile expansion stays linear. Members are stored, never deflated, so S is the payload bytes.
The meshopt codecs decode at most 16 vertices per encoded byte (ATTRIBUTES), 3 indices
(TRIANGLES) or 1 index (INDICES). C1 charges each attribute vertex once per component, though,
so a VEC4 u8 stream is up to 64 charged units per encoded byte. The bound does not rest on the
codec ratio. It holds because every unit of work is charged before it runs: a validation does
at most max(16,000,000, 77·S) units plus one charge's overshoot. A 10-million-triangle
repeated-index stream is refused by the per-accessor element cap in 0.1 s.

*Time statement.* CPU time per unit was measured for nine adversarial classes, each run pinned
to one P-core (CPU 10) and one E-core (CPU 17) at idle priority, on a host at load average 15–56
(2026-10-30 and -31). c1–c4 run to a 500,000,000-unit budget. c5 holds 600 leaves of 8,192 triangles with
three new original vertices each and a valid root, and is validated in full (1,502,712,616 units,
32.0 per stored byte). c6–c8 run to their default budget of 77·S and are refused there.

| class | P-core ns/unit | E-core ns/unit |
| --- | --- | --- |
| c1 comp64: unreferenced VEC4 u8 meshopt accessors | 5.80 | 6.47 |
| c2 hlodseq: distinct-vertex index streams | 7.61 | 8.57 |
| c3 hloddup: one triangle instanced by 100 nodes | 6.84 | 6.32 |
| c4 hlodleaf: 100k-triangle leaves, new originals per triangle | 10.28 | 11.88 |
| c5 hlodleaf: 600 × 8,192-triangle leaves | 9.85 | 10.72 |

The scratch-row classes (generators in `bench/architecture_audit/vs_c_limits/hostile/`):

- **c6 leafrand** (review 2): N leaves of 8,192 nondegenerate triangles. Every corner is a distinct
  pseudo-random lattice point and every leaf has a `source_triangle` table, so each point, edge
  and face is a random-key row. `leafrand.py N 8192 7` (N = 1,000: seed 11).
- **c7 twin:** the same leaves under two coarse cells, the second cell's at the first's
  positions, so every key is shared across the cells: ancestor requirements, multi-edge faces and
  coincident corner sets as well. `rowclasses.py twin N 8192 3`.
- **c8 chain:** leaves with every triangle doubled (every segment a multi edge, every face
  coincident) under a single-child chain of 24 coarse tiles sharing one payload that doubles 128
  of those triangles. `rowclasses.py chain N 8192 5 OUT 24 128`. At the default budget its
  admitted work is the leaf rows and the edge-face replay of every leaf; the chain tiles'
  lookups would come after the budget (its refusal counts are the same with any seek weight).
  The seek cost is measured directly instead (the probe above).
- **c9 revfan** (review 1 of PR #309): 32 leaves each hold 2 base triangles doubled, plus
  1,360 copies of the reversed face on each base edge. A single-child chain of K coarse tiles,
  split into external tilesets every 25 levels, shares one payload that doubles the 64 base
  triangles. Every coarse edge is used twice forward, so each chain tile looks up its forward
  edge faces, with 1,360 reversed rows on the same key. Valid archives (34.1 MB);
  `rows_control.py revfan OUT K 64 1360 32 25`.

| class | leaves | S | P-core ns/unit | E-core ns/unit | develop, P-core | develop, E-core |
| --- | --- | --- | --- | --- | --- | --- |
| c6 leafrand | 10 | 1.52 MB | 8.38 | 8.87 | 8.37 | 9.47 |
| c6 leafrand | 40 | 6.08 MB | 8.22 | 8.90 | 10.27 | 12.24 |
| c6 leafrand | 150 | 22.81 MB | 8.75 | 9.79 | 17.19 | 20.79 |
| c6 leafrand | 400 | 60.83 MB | 7.27 | 10.15 | not run | 30.10 |
| c6 leafrand | 1000 | 152.06 MB | 7.61 | 10.64 | not run | not run |
| c7 twin | 10 | 1.52 MB | 9.68 | 10.57 | 10.04 | 11.16 |
| c7 twin | 40 | 6.09 MB | 9.97 | 10.59 | 12.99 | 15.37 |
| c7 twin | 150 | 22.81 MB | 9.73 | 11.60 | not run | 28.87 |
| c7 twin | 400 | 60.83 MB | 9.27 | 12.45 | not run | not run |
| c8 chain | 10 | 1.54 MB | 7.16 | 7.88 | 10.55 | 12.40 |
| c8 chain | 40 | 6.10 MB | 7.62 | 8.27 | 14.66 | 18.65 |
| c8 chain | 150 | 22.82 MB | 8.04 | 8.92 | not run | not run |
| c8 chain | 400 | 60.84 MB | 8.15 | 9.08 | not run | not run |

| c9 revfan | units | P-core ns/unit | E-core ns/unit | review 1 (c85639ef), P / E |
| --- | --- | --- | --- | --- |
| K = 1 | 243,896,125 | 9.07–9.35 | 10.03–10.20 | 6.2–6.6 / 8.8 |
| K = 100 | 277,462,749 | 9.09 | 10.21 | 11.4 / 17.5 |
| K = 122 (deepest admitted) | 284,921,993 | 9.14–9.24 | 9.99–10.60 | 12.7 (12.3–15.8) / 19.9 |
| K = 100, no reversed faces (16 M floor) | 16,000,043 | 11.03–11.48 | 14.12 | 7.5 / 11.6 |
| the same, unbounded | 34,150,400 | 8.84–9.08 | not run | |

Before the covering edge-face index, each chain tile read the 1,360 reversed rows of every edge
uncharged, so c9 cost more per unit the deeper the chain. Now K = 122 costs what K = 1 does. The
floor-bound run's higher figure is fixed start-up (about 40 ms) over a 0.18 s run.

E-core repeats of the larger archives gave 9.25 and 9.86 (c6, 150 and 400 leaves) and 11.51 and
12.55 (c7). Before this change, c6 rose on the P-core from 7.8 ns/unit at 1.52 MB to 26.7 at
60.8 MB (2,052 s per stored GB; review 2, same archives). In this session develop measures c6 at
17.19 (P) and 20.79 (E) ns/unit at 22.8 MB and 30.10 (E) at 60.8 MB, and c7 at 28.87 (E) at
22.8 MB. Now the cost per unit no longer grows with the tables, beyond a logarithmic term:

- On the P-core c6 and c7 stay within ±10% from 1.5 MB to 152 MB stored. c8 rises 14% over 40×
  (7.16 to 8.15 ns/unit).
- On the E-core c6 goes from 8.87 to 10.64 ns/unit over the 100× range, c7 from 10.57 to 12.55
  over 40×, and c8 from 7.88 to 9.08. The rise is logarithmic. Each external sort merges at most
  32 runs of 24 MiB at once, so a sort adds one sequential pass per 32-fold growth of its rows (past about 13 M rows,
  about 23 MB stored, and again past about 420 M rows, about 700 MB). The hot-path seeks also
  descend one more B-tree level per hundredfold growth.
- Extrapolating c7's E-core slope (about 0.5 ns per e-fold of S) from 61 MB to 1 GB gives
  14 ns/unit.

The stated bound is `(16,000,000 + 77·S)·τ`, with τ the worst of every measurement and every
extrapolation to 1 GB stored:

- **P-core: τ = 11.1 ns.**
  - The largest measured value is c9's floor-bound run, 11.03–11.48 ns (median 11.1).
  - Otherwise c4's 10.28 ns.
  - Extrapolated to 1 GB: a seek at about 600 M rows costs 10.8 ns at 320 units (the probe
    above), and c7's slope gives about 10.7 ns.
  - That is 0.18 s plus 855 s per stored GB.
- **E-core: τ = 14.1 ns.**
  - The largest measured values are c9's floor-bound run at 14.12 ns and c7's 12.55 ns at 61 MB.
  - c7 extrapolates to 14 ns at 1 GB.
  - That is 0.23 s plus 1,087 s per stored GB.
- Both are within the C2 statement below (1,037 and 1,875 s per stored GB).

A new work class, or a per-unit cost above 13.4 ns (P-core) or 24.3 ns (E-core), must re-weight
its charges or revisit the ratio.

**VS-C-2. Opt-in caps and natural bounds (A-C1-1, A-C1-2).** With no Class B cap set, the
archive is bounded by the format and the file alone, in this order (all VS-A, unchanged):

| key | natural bound when unset |
| --- | --- |
| `sourceArchiveBytes` | the file length |
| `archiveEntries` | the 3TZ member ceiling 178,956,970 (`unsupported`), then `46·N ≤` directory bytes (`invalid_input` "ZIP entry count exceeds directory length") |
| `centralDirectoryBytes` | the directory lies inside the source (`invalid_input`) |
| `archiveStoredBytes` | member extents are disjoint and inside the source, so S ≤ the file length |

A set cap is checked at today's site and in today's order: the source length, then the member
ceiling (a format rule, always first among the entry checks), then entries, then directory
bytes, then the running stored sum at each record. A cap refuses when the observed value is
strictly greater than the cap; a value equal to the cap passes. With no stored-bytes cap, an
overflow of the running u64 sum is `invalid_input` "stored member sizes overflow" (unreachable
for a real file; the check stays).

**VS-C-3. Overrides.** Only the four Class B and seven Class C keys may be given; any other key
(a Class A key included) is refused before the input is opened. Any u64 is legal, including 0
(refuses at the first charge) and values below the floor: users may tighten. A Class C override
replaces the rule entirely (it is neither raised to the floor nor to `⌊S·p/q⌋`). A Class B
override sets the cap. Overrides never change a Class A value.

**VS-C-4. Report.** A successful report (the CLI's `--json` object, Python's dict, Rust's
`ValidationReport::summary_json`) gains two top-level keys and one `limits` key:

1. `storedBytes`: S (u64).
2. `limits`: the same twenty keys as before plus `liveDocumentBytes`. Class A values are the
   fixed numbers, Class B values are the cap or `null`, Class C values are the effective
   numbers.
3. `limitDerivation`:

   ```json
   {"model": 1,
    "budgets": {"containmentBytesRead": {"floor": 2147483648, "perStoredByte": [4, 1], "source": "floor"},
                "...": "one entry per Class C key, seven in all"}}
   ```

   `source` is `"explicit"` when the key was overridden. Otherwise it is `"scaled"` iff
   `⌊S·p/q⌋ > floor`, and `"floor"` otherwise (a tie is `"floor"`). `floor` and
   `perStoredByte` always state the model's rule, even for an explicit value, so a reader can
   check `limits[k] == (explicit value) or max(floor, ⌊S·p/q⌋)` from the report alone. Class B
   needs no derivation: a number was set, `null` was not.

**Determinism.** `storedBytes`, `limits` and `limitDerivation` are a pure function of (S, the
overrides, the model). They do not depend on the host, the path, the scratch placement, the
catalog profile (resident or forced spill), threads, or the order in which overrides were given.
`scratch` remains the only host-dependent member and stays outside `limits`. Keys are written in
sorted order and `write_json` stays byte-identical to the `Display` of the serialized value.

**VS-C-5. Typed errors.**

| # | Failure | Category |
| --- | --- | --- |
| T1 | Every scratch candidate is memory-backed where memory-backed scratch is not allowed (`validate_3tz` and the other report-less archive readers): `ScratchRefusal::MemoryBackedOnly { candidates }`, carried by `Error::Scratch` | `environment`, exit 4 |
| T1' | Other scratch failures (`ScratchRefusal::InsufficientSpace { path, needed, available }`, `ScratchRefusal::Io`), carried by `Error::Scratch` and `ValidationFailure::Scratch` | `io`, exit 1, as before |
| T2 | Spill overflow, corrupt run, poisoned sorter | unchanged |
| T3 | A refused charge of a Class B cap or a Class C budget | `resource_limit`, exit 3, structured |
| T4 | Cancellation through `inspect_with` | `cancelled`, exit 1; observer failure `observer_failure`, exit 1 |
| T5 | A bad request: an unknown, Class A or duplicate key, a value that is not a decimal u64, a scratch directory that is missing or not a directory | `usage`, exit 2 (`ValidationFailure::InvalidRequest`), before the input is opened |

Scratch messages keep the `validation scratch:` prefix inside `inspect`. The facade refusal reads
`archive catalog scratch: every scratch candidate is memory-backed (tmpfs) …; set TMPDIR to a
disk-backed directory` and lists the candidates.

T3 refusals carry `LimitExceeded { limit, value, observed, derivation }`:

- `limit`: the camelCase key;
- `value`: the effective value of that key;
- `observed`: the total the refused charge would have reached (saturating at u64::MAX), or for
  a cap the observed quantity (file length, N, directory bytes, the running stored sum). For
  the accessor planner below it is the elements already charged plus the payload's planned
  total at the refusal (at a count-token refusal, a lower bound);
- `derivation`: `"floor"`, `"scaled"` or `"explicit"` for Class C, `"cap"` for Class B.

The message keeps its old text and gains the suffix ` (limit <key>=<value>, <derivation>)`, for
example `hierarchy exceeds 262144 traversal tasks (limit hierarchyVisits=262144, floor)`. Two
messages that named the old fixed values change: "member reads exceed 2 GiB work limit" is now
"member reads exceed work limit" and "payload elements exceed 16 million" is now "payload
elements exceed archive budget". The CLI JSON error (`{"ok": false, "error": {"code":
"resource_limit", "message": …}, "exitCode": 3}`) gains
`error.limit = {"limit": …, "value": …, "observed": …, "derivation": …}`; Python
`ResourceLimitError.limit` is the same dict (absent attribute value `None` for other resource
limits). The structured sites are every validator-owned charge of the eleven keys: the hash
pass, member reads, reference visits, traversal tasks, payload elements, and the three I1
reservations (scan → `totalBytesRead`, tile availability → `hierarchyVisits`, content entries →
`references`). The per-document accessor planner of a payload refuses with the archive budget
`totalPayloadElements` when the archive remainder was the binding allowance, and is otherwise the
Class A `accessorElements` refusal without structured data. The C2 budgets never refuse: an
exceeded C2 budget stays `notInspected: decodedContentBounds.workLimit`. Class A refusals keep
their messages and carry no structured data.

**VS-C-6. Cancellation and progress.** `inspect_with(request, &RunControl)` claims the control
once and checks it only at charge sites, so the work done after a cancellation is bounded by one
check interval:

- the hash pass: before each member (each member is at most `memberBytes`, or the index of
  `24·(N−1)` bytes);
- traversal: before each task popped from the pending stack (tilesets, tiles, deferred
  references); each task's own work is bounded by the Class A limits;
- the memo replay of a placement (VS-B EM-5.4), which registers a whole expansion inside one
  traversal task: before each replayed event (a tile, a reference or a nested placement);
- I1: before each implicit tile;
- the phase-2 stream (VS-B-4), which runs after whole-archive admission: before each logged
  tile or reference; each reference's stream is bounded by the Class A limits;
- the payload rows: before the sort.

Progress events (`RunEvent::Progress`) are emitted with phases `validate.hash` (done = members
hashed, total = N), `validate.traverse` (done = tasks, no total), `validate.stream` (done =
logged entries streamed, total = the logged tiles and references; only when the archive is in
the C2 domain and is streamed) and `validate.rows`, at the phase start and then every 1,024
members, tasks or entries, so a Python observer runs `check_signals` at the same interval. A
memo replay re-emits the current `validate.traverse` event (its task count unchanged) every 1,024
replayed events. An observer error aborts the run (`observer_failure`); a Python callback's own
exception (including `KeyboardInterrupt`) is re-raised unchanged. `inspect(request)` is
`inspect_with` with a silent control.

**VS-C-7. Scratch directory.** `ValidationRequest::scratch_dir(dir)` (CLI `--scratchDir`,
Python `scratch_dir`) places validation scratch inside `dir` exactly, still after the
free-space preflight. A missing `dir` or one that is not a directory is T5 before the archive is
opened. A memory-backed `dir` is used because the user chose it, and the report says
`scratch.memoryBacked: true`. Without it the VS-A default placement applies (disk-backed first,
memory-backed as a labelled last resort). `validate_3tz` keeps no scratch parameter and stays
refusal-only on tmpfs (T1).

## Surfaces

- **Rust.** `pub enum LimitKey` (eleven variants; `as_str`, `FromStr` with error
  `UnknownLimitKey`, `ALL`), `ValidationRequest::max_volume(LimitKey, u64)` (a later call for the
  same key replaces the earlier one), `ValidationRequest::scratch_dir(PathBuf)`,
  `inspect_with(request, &RunControl)`. `ValidationLimits`'s four cap fields become
  `Option<u64>`. `ValidationReport` gains `stored_bytes` and `limit_derivation`.
  `ValidationFailure` gains `Scratch`, `Cancelled`, `Observer` and `InvalidRequest`;
  `ResourceLimit` holds a `LimitRefusal { message, limit: Option<LimitExceeded> }`.
  `Error::Scratch(ScratchRefusal)` is new.
- **CLI.** `rusty-tiles validate IN [--maxVolume KEY=N]... [--scratchDir DIR]`, with the
  visible kebab aliases `--max-volume` and `--scratch-dir` (repository option convention). N is
  a decimal u64 with no sign, separator or suffix. `--progress json` emits the VS-C-6 events.
- **Python.** `validate(input, *, lazy_payloads=False, max_volume=None, scratch_dir=None,
  callback=None)`. `max_volume` is a mapping from key to int; an unknown or Class A key, a
  `bool`, a non-int, a negative value or one ≥ 2^64 raises `InvalidRequestError` before any I/O.
  `report["limits"]["archiveEntries"] is None` when the cap is unset.

## C2 time statement (A-C2-1, A-C2-3)

The stated C2 time bound becomes `(2^29 + 128·S)·τ` per validation at default limits: on the
measured E-core (τ = 14.65 ns) 7.87 s plus 1,875 s per stored GB; on a P-core (τ = 8.10 ns)
4.35 s plus 1,037 s per stored GB. The hostile JSON re-parse cost is charged to
`totalBytesRead` (4·S at about 29 ns/B, at most 116 s per stored GB). The τ obligation and the
mf2-lod3 ceilings of A-C2-3 are retained. If a real archive's C2 units exceed 64 per stored
byte (half of κ_u), VS-D must revisit κ before release.

## Memory bound

Unchanged from VS-A and VS-B. Uncapped entries are safe because the catalog, the facts store
and the payload rows spill by N (VS-A CAT-5, VS-B-5), and the member ceiling bounds N.

## Report and API changes (breaking)

- Default validation accepts archives over 1 GiB, over 2^19 entries and with directories over
  32 MiB. `zip64_count_at_ceiling` (a ZIP64 directory claiming 2^19+ entries in a tiny file)
  moves from `resource_limit` to `invalid_input` "ZIP entry count exceeds directory length";
  with `--maxVolume archiveEntries=524288` it keeps the old refusal.
- New report keys `storedBytes` and `limitDerivation`; `limits.liveDocumentBytes`; the four caps
  are `null` unless set.
- Budget and cap refusal messages gain the VS-C-5 suffix; the CLI error and Python exception gain
  `limit`.
- The `validate_3tz` tmpfs refusal moves from `io`/1 to `environment`/4.

## Acceptance plan

1. **Oracle** (`bench/architecture_audit/vs_c_limits/`, stdlib only, separate author who reads
   this note and the format, not `src/`): parses EOCD, ZIP64 and the central directory, computes
   N, S, `limits` and `limitDerivation` for given overrides, and is compared with the CLI on the
   VS-A identity corpus, the controls and the 8.1 GB archive: 0 disagreements.
2. **Controls K1–K10** (plan §6), in `src/validate/limits_controls.rs`, the CLI tests and the
   Python tests: crossovers at report level, at-cap pairs, overrides below the floor, opt-in caps
   (with a sparse archive over 1 GiB), determinism across profiles and flag order, surfaces,
   the public `validate_3tz` T1 test on tmpfs, the cancellation bound and `--scratchDir`.
3. **Mutants m1–m16** (plan §6), hashed, each killed.
4. **Parity**: every VS-A and VS-B suite in both catalog profiles and the six C1 CLI oracles;
   the only differences are the amended report keys and the listed `zip64_count_at_ceiling`
   verdict. The pinned C1 bundles are amended additively
   (`production_controls/vs_c_limits_amendment/`).
5. **VSC-REAL**: the 8.1 GB H16 archive validates with no flags, containment checked, peak RSS
   ≤ 1 GiB, `storedBytes`/`limits`/`limitDerivation` equal to the oracle; C2 units per stored
   byte reported.

## Unresolved obligations

- κ and the model number are provisional until VS-D.
- Class A limits are not overridable; a tiler output over `documentItems` or `jsonBytes` is
  refused (tiler risk R-5), by decision.
- `validate_3tz` has no scratch-directory parameter (OQ-C8).
