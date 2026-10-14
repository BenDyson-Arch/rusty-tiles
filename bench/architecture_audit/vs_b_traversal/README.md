# VS-B: traversal and decoded containment at scale (author evidence, 2026-10-14)

Contract: [validator-traversal-scale-amendment.md](../../../docs/architecture/validator-traversal-scale-amendment.md)
(VS-B-1 to VS-B-11; VS-B-11 integrates the H23 payload replay, branch
`perf/c1-single-inspection` `f623568`). This is author evidence. The nonauthor review and
the coordinator's gate are pending.

Bulk evidence is kept out of the tree, in `~/.cache/rusty-tiles-evidence/vs-b-20261013/`:
- `parity/`: in-crate parity logs and per-suite summaries per catalog profile, `run.sh`,
  `source.txt`; `cli/`: the C1 CLI oracles against plain and forced-spill debug binaries;
- `scale/`: driver and CLI binaries with `SHA256SUMS` and `SOURCE`, `run.sh`,
  `results.tsv` (scale), `replay.tsv` (replay against full re-inspection), per-run logs;
- `hostile/`: the external-tileset amplification archives;
- `perf/`: `ab.py`, `forest.tsv` (VS-A against VS-B), `replay.tsv`;
- `mutants/`: `report.json` and one log per mutant.

## Controls

`src/validate/traversal_controls.rs` (CI, both catalog profiles through the forced-spill
thread override):

- **Reference traversal.** A test-only switch keeps each passing reference's level chain in
  memory and streams after traversal with a full C1 re-inspection per reference, exactly as
  the old phase 2. Every differential compares the logged, replaying stream with it, with the memo off, with the forced-spill profile, and all
  three together: report or failure, C2 units and the C1 counters.
- **Precedence** (VS-B-4): an Outside followed by an exclusion (exclusion wins), by a
  node-depth precheck stop (`workLimit` wins), by a C1 failure or a closure failure (C1
  wins); a passing reference followed by an exclusion; the first failure in traversal order
  with a deferred reference after an external tileset.
- **Generated archives**: 160 deterministic archives (shared external documents, transforms,
  content volumes, repeat references, escapes, exclusions, node-depth stops, late C1
  failures) at two limit sets; every outcome class is reached.
- **Memo budget** (VS-B-6): 80 generated archives at budgets 0, 1 event, 8 events and 4 KiB
  give the unbudgeted result; small budgets re-traverse documents.
- **Live documents** (VS-B-7): the charge at the cap and one below, released at exit; the
  audit probe (one document placed shallow, then deep) gives the same verdict with the memo on
  and off, at the deep path's charge and one below, and the memo is used at the passing cap.
- **Replay composition** (VS-B-6): a document placed twice, the second time under a rotated
  external-content box, gives `rotatedBox` through the replayed child.
- **Labels** (VS-B-9): 64 kept, 70 reversed, a 256-byte and a 257-byte name.
- **Budgets** (VS-B-2): every crossover at `S − 1`, `S` and one κ step above; an end-to-end
  299,593-tile implicit archive refused at the fixed floor and accepted with `S/8` visits.
- **Schema** (VS-B-10): duplicate children give the library's message; `schema.rs` checks the
  keyword against the built-in on 24,000 generated arrays of both paths and the `±0` split.
- **Facts store** (VS-B-5): `facts.rs` round-trips every field and the receipt; the
  resident and spilled stores agree on strided and adjacent ranks; the spilled file's
  `peakBytes` is one record past the highest rank; a flipped byte anywhere in a record, or in
  the spilled file, is a typed `invalid_state`; the preflight estimate counts the file.
- **Spill** (`spill::tests`): `SlotFile` round trip in any order, holes, bounds and widths,
  accounting, and typed create, write and truncation faults through the fault seam.
- **Stream log** (`stream_log.rs`): resident and spilled logs read back in order; truncated or
  unknown records are corrupt scratch.
- **I1** (VS-B-8): the independent oracle models the index-table charge (amendment 7,
  separate author, `implicit_read/oracle/README.md`) with new at-cap pairs `VSB8.rank`,
  `VSB8.alias` and `VSB8.const`; the R2/R2b pairs were recomputed.

## Results

Source: see `parity/source.txt` and `scale/bin/SOURCE`.

**Audit fixes (`5fd0f5a` onward).** After the nonauthor audit (`audit-217.md`, GO with four
SHOULD items): phase 2 streams again only after whole-archive admission, from a log of the
in-domain tiles and references (VS-B-4); summaries carry their live-document peak (VS-B-6,
VS-B-7); facts records carry a CRC-32 and are in the preflight estimate; `SlotFile` and
scratch accounting are tested. The numbers below are for the fixed source except where
marked.

**Parity (release, both profiles, 0 disagreements).** C2 slice-1 witnesses and controls
261/261 (large fixtures generated), C2-WB classes 90/90 (with `@large`), H11 16/16,
slice-3 controls 80/80 (with `C2S3_EVIDENCE_DIR`), S3-18 sweep 188/188, forest 34/34 (with
`C2S3_FOREST_DIR`, all W twins and mf2-lod3), I1 controls 187/187 (large controls
generated), W corpus 41/41, at `ee2c60f` (develop through #226 merged). Every
`validate::`, `archive3tz::`, `content_integrity::` and `spill::` test passes in both
profiles (341 resident, 326 forced;
the two pinned context/stride acceptance modules run resident only, as their reports
gain `scratch` when forced).

**Python** (at `84e37b4`, before the audit fixes). The installed release wheel passes `scripts/test_python_wheel.py` (46 tests), and
`python_check.py` finds Python's report equal to the CLI's, for list and lazy payloads, on
mf2-lod3, W5-vec-perf and `vsa-e14` (effective budgets, no `liveDocumentBytes`).

**C1 CLI oracles.** All six pass against the plain debug binary. Against the
`validate-forced-spill` binary, `payload_integrity_oracle` (73) and
`payload_meshopt_stride_oracle` (4) differ only by the report's `scratch` member, which
VS-A adds whenever anything spilled (`cli/forced-scratch-only.txt`); nothing else differs.

**Scale** (release scale driver at `e09801c`: volume caps raised, budgets scaled from S,
seams off; CPUs 4-7, host shared with H17 runs, load 6–8):

| Archive | Members | Tiles | Peak RSS | Seconds | C2 units / budget | Outcome |
| --- | --- | --- | --- | --- | --- | --- |
| `vsa-e14` | 21,848 | 21,845 | 85 MiB | 0.53 | | accepted |
| `vsa-a9` | 87,384 | 87,381 | 113 MiB | 2.0 | | accepted |
| `vsa-a10` | 349,528 | 349,525 | 213 MiB | 8.0 | | accepted (catalog and facts resident) |
| `vsa-a11` | 1,398,104 | 1,398,101 | 240 MiB | 25.0–50.5 | | accepted (catalog and facts spilled) |
| `vsa-a12` | 5,592,408 | 5,592,405 | 231 MiB | 168 | | accepted |
| `combined-d` (gate d) | 4,184 | 5,592,405 | 133 MiB | 9.2 | | accepted |
| `large-4p5g` | 41 | 40 | 196 MiB | 10.1 | 4.9e9 / 6.3e11 (0.8%) | accepted, contained |
| H16 `mgal_balanced_fullres` (8.1 GB) | | 2,812 | 182 MiB | 21.1 | 1.7e10 / 1.0e12 (1.7%) | accepted, contained |
| `implicit-2p22` (1.2 MB, first run) | 4,100 | | 78 MiB | 0.12 | | visit budget (S/8 is below its 5.6 M tiles) |

Peak RSS is flat from 1.4 M to 5.6 M tiles (240 → 231 MiB). Wall times on this run are
inflated by host load: alternating the pre-fix and fixed drivers on `vsa-a11` gave 35.0 and
34.9 s (pre-fix) against 25.0 and 28.3 s (fixed) in the same window, and the first, quieter
run of the pre-fix source (`results.tsv`) gave 24.6 s at 1.4 M tiles and 96.1 s at 5.6 M
(17.6 and 17.2 µs per tile, linear). Before VS-B every such run stopped at 2^18 visits
(VS-A README); an early run with the test-only per-tile seams still collecting grew to
987 MiB at 1.4 M tiles, so the driver now switches them off. The `vsa-a*` and `combined-d`
archives have rowless deeper implicit tiles (`derivedImplicitVolume`), so they exercise
traversal, C1 and the facts store at scale but not the stream; `large-4p5g` and the H16
archive stream every reference. Their C2 units use at most 1.7% of `128·S`.

**Hostile archives** (`ext-amplification-20261013`, release CLI at `e09801c`, default
budgets): every output is byte-identical to the EM memo run, and the whole corpus takes
9.43 s against 10.41 s (0.91×) at 69–76 MiB. The `workLimit` archives (`c2-d15-m5`, `-m6`,
`-m28`) take 0.17–0.19 s: an archive that leaves the domain streams nothing. (The first,
inline design streamed them for a discarded result: 2.1 s here, 3.7–3.9 s in the audit.)

**Performance.** VS-A (`6d3f279`) against VS-B at `e09801c` (release CLI, minimum of 3
alternating runs over every forest archive): total 4.034 → 4.017 s (0.996×); mf2-lod3
1.537 → 1.509 s and 83 → 77 MiB; every archive within ±3% except W2 and W2-explicit (+10%,
about 0.011 s, below the lane's 0.1 s floor); outputs equal except `limits` above the
crossovers. Replay against full re-inspection in the same pre-fix source (5 alternating
runs): mf2-lod3 −3%, W5-vec-perf −7%, W2 −4%, W3 −1%; byte-heavy `large-4p5g` (4.9 GB)
8.5–9.0 → 8.5–8.7 s and H16 19.0–19.9 → 17.9–18.4 s. So H23's receipt hashing does not slow
byte-heavy payloads, but replay removes only a few percent here: the containment walk, not
the C1 value pass, dominates the stream on these archives.

**Mutants.** Two drivers, both release, under the shared Cargo lock:
`mutants.py` (VS-B) and `legacy_mutants.py` (the existing C2, I1, EM, VS-A and H23 sets).
The legacy driver uses ports for mutants whose code VS-B removed. A port is the same defect
on the new code, named `port:<original>`, and it is listed in the driver. Logs and
`report.json` files are in `mutants-final/` and `mutants-legacy/final-*/`.

- **VS-B set.** Run on the audit-fixed source, `e09801c`. All 26 mutants are caught.
  - Two controls were added after the first run so that `memo-budget-ignored` and
    `replay-flat-parent` are caught.
  - Some single edits are equivalent, and the driver documents why:
    - the uniqueness hash-set checks: either the `bits` bucket or `same_bits` alone
      separates `0` from `-0.0`;
    - each of the two stream guards alone.
  - The recorded mutants therefore change both checks, or both guards.
- **Legacy sets.** Re-run on the audit-fixed source, `e09801c` through `55b78d6`, 101 mutants:
  - every C2 patch and port (slice 1, H11, slice 3);
  - every EM, H23 and port mutant;
  - the VS-A and I1 mutants not caught in the earlier run.

  Results:
  - 98 caught, after rerunning `vsa:scratch-ram-silent` with the spill tests included. They
    hold VS-A's placement control.
  - `c2:slice3/subtree-tileMetadata-is-tileMetadata` was applied with its arena context
    mapped, and is caught.
  - `em:memo-on-active-path` is equivalent; the EM amendment proves it.
  - `em:cached-placement-link` has no form without an arena. Its defect is now the VS-B
    mutants `replay-no-external` and `replay-flat-parent`.
  - 35 I1 and VS-A mutants were caught on the pre-fix source (`chunk-1` to `chunk-3`).
    The audit fixes do not touch their modules.
  - Every port is caught.
- **Not re-run after the `ee2c60f` merge of develop's #226.** That merge changes no
  VS-B-owned line. Its payload change is covered by parity and the payload tests.

## Not established

- C2 streaming at millions of references (no generated archive with tile rows at that
  scale; VS-D's generators).
- κ: producer units against `κ_u·S` on the gate archives is VS-D's.
- `liveDocumentBytes`, `storedBytes` and opt-in volume caps in the report: VS-C.
