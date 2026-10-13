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

- **Reference traversal.** A test-only switch streams after traversal with a full C1
  re-inspection per reference, exactly as the old phase 2. Every differential compares the
  inline replaying stream with it, with the memo off, with the forced-spill profile, and all
  three together: report or failure, C2 units and the C1 counters.
- **Precedence** (VS-B-4): an Outside followed by an exclusion (exclusion wins), by a
  node-depth precheck stop (`workLimit` wins), by a C1 failure or a closure failure (C1
  wins); a passing stream followed by an exclusion; the first failure in traversal order
  with a deferred reference after an external tileset.
- **Generated archives**: 160 deterministic archives (shared external documents, transforms,
  content volumes, repeat references, escapes, exclusions, node-depth stops, late C1
  failures) at two limit sets; every outcome class is reached.
- **Memo budget** (VS-B-6): 80 generated archives at budgets 0, 1 event, 8 events and 4 KiB
  give the unbudgeted result; small budgets re-traverse documents.
- **Live documents** (VS-B-7): the charge at the cap and one below, released at exit.
- **Labels** (VS-B-9): 64 kept, 70 reversed, a 256-byte and a 257-byte name.
- **Budgets** (VS-B-2): every crossover at `S − 1`, `S` and one κ step above; an end-to-end
  299,593-tile implicit archive refused at the fixed floor and accepted with `S/8` visits.
- **Schema** (VS-B-10): duplicate children give the library's message; `schema.rs` checks the
  keyword against the built-in on 24,000 generated arrays of both paths and the `±0` split.
- **Facts store** (VS-B-5): `facts.rs` round-trips every field and the receipt, and the
  resident and spilled stores agree.
- **I1** (VS-B-8): the independent oracle models the index-table charge (amendment 7,
  separate author, `implicit_read/oracle/README.md`) with new at-cap pairs `VSB8.rank`,
  `VSB8.alias` and `VSB8.const`; the R2/R2b pairs were recomputed.

## Results

Source: see `parity/source.txt` and `scale/bin/SOURCE`.

**Parity (release, both profiles, 0 disagreements).** C2 slice-1 witnesses and controls
261/261 (large fixtures generated), C2-WB classes 90/90 (with `@large`), H11 16/16,
slice-3 controls 80/80 (with `C2S3_EVIDENCE_DIR`), S3-18 sweep 188/188, forest 34/34 (with
`C2S3_FOREST_DIR`, all W twins and mf2-lod3), I1 controls 187/187 (large controls
generated), W corpus 27/27 run, 14 skipped because their artifacts no longer exist
(`/tmp` paths of the I1 preparation). Every `validate::`, `archive3tz::`,
`content_integrity::` and `spill::` test passes in both profiles (332 resident, 317 forced;
the two pinned context/stride acceptance modules run resident only, as their reports
gain `scratch` when forced).

**C1 CLI oracles.** All six pass against the plain debug binary. Against the
`validate-forced-spill` binary, `payload_integrity_oracle` (73) and
`payload_meshopt_stride_oracle` (4) differ only by the report's `scratch` member, which
VS-A adds whenever anything spilled (`cli/forced-scratch-only.txt`); nothing else differs.

**Scale** (release scale driver: volume caps raised, budgets scaled from S, seams off;
CPUs 4-7, host shared with an H17 run, load 5–11):

| Archive | Members | Tiles | Peak RSS | Seconds | Outcome |
| --- | --- | --- | --- | --- | --- |
| `vsa-e14` | 21,848 | 21,845 | 87 MiB | 0.32 | accepted |
| `vsa-a9` | 87,384 | 87,381 | 111 MiB | 1.24 | accepted |
| `vsa-a10` | 349,528 | 349,525 | 213 MiB | 5.0 | accepted (catalog and facts resident) |
| `vsa-a11` | 1,398,104 | 1,398,101 | 241 MiB | 24.6 | accepted (catalog and facts spilled) |
| `vsa-a12` | 5,592,408 | 5,592,405 | 232 MiB | 96.1 | accepted |
| `combined-d` (gate d) | 4,184 | 5,592,405 | 135 MiB | 8.2 | accepted |
| `large-4p5g` | 41 | 40 | 198 MiB | 9.4 | accepted, contained |
| H16 `mgal_balanced_fullres` (8.1 GB) | | 2,812 | 183 MiB | 19.3 | accepted, contained |
| `implicit-2p22` (1.2 MB) | 4,100 | | 78 MiB | 0.12 | visit budget (S/8 is below its 5.6 M tiles) |

Peak RSS is flat from 1.4 M to 5.6 M tiles (241 → 232 MiB) and time per tile is linear
(17.6 and 17.2 µs). Before VS-B every such run stopped at 2^18 visits (VS-A README); the
first run of this table, with the test-only per-tile seams still collecting, grew to 987 MiB
at 1.4 M tiles, so the driver now switches them off. The `vsa-a*` and `combined-d` archives
have rowless deeper implicit tiles (`derivedImplicitVolume`), so they exercise traversal,
C1 and the facts store at scale but not the containment stream; `large-4p5g` and the H16
archive stream every reference.

**Hostile archives** (`ext-amplification-20261013`, release CLI, default budgets): every
output is byte-identical to the EM memo run; the slowest is 2.15 s (61–63 s before EM) at
70–73 MiB. The `workLimit` archives (`c2-d15-m5`, `-m6`, `-m28`) take 2.1 s instead of
0.16 s: inline streaming streams the references before the precheck stop, which the old
phase 2 never streamed. That work was charged in units before it was done, so it stays
within the default cap's bound (2^29 units, at most about 7.9 s on an E-core).

**Performance.** VS-A against VS-B (release CLI, minimum of 3 alternating runs, every
forest archive): mf2-lod3 1.533 → 1.524 s, 87 → 81 MiB; the other archives within ±5%
except W2 and W2-explicit (+11%, 0.105 → 0.118 s); outputs equal except `limits` above the
crossovers. Replay against full re-inspection in the same source (5 alternating runs):
mf2-lod3 1.56 → 1.51 s (−3%), W5-vec-perf −7%, W2 −4%, W3 −1%; byte-heavy `large-4p5g`
(4.9 GB) 8.5–9.0 → 8.5–8.7 s and H16 19.0–19.9 → 17.9–18.4 s. So H23's receipt hashing does
not slow byte-heavy payloads, but replay removes only a few percent here: the containment
walk, not the C1 value pass, dominates the stream on these archives.

## Not established

- C2 streaming at millions of references (no generated archive with tile rows at that
  scale; VS-D's generators).
- κ: producer units against `κ_u·S` on the gate archives is VS-D's.
- The W corpus rows whose artifacts are gone.
- `liveDocumentBytes`, `storedBytes` and opt-in volume caps in the report: VS-C.
