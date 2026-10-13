# VS-A: archive catalog at scale (author evidence, 2026-10-13)

Contract: [c1-archive-catalog-scale-amendment.md](../../../docs/architecture/c1-archive-catalog-scale-amendment.md).
This is author evidence. The nonauthor review and the coordinator's gate are pending.

Bulk evidence is kept out of the tree, in `~/.cache/rusty-tiles-evidence/vs-a-20261013/`:
- `parity/`: in-crate parity logs per catalog profile (`lib-resident.log`,
  `lib-forced.log`), the C1 CLI oracle outputs (`cli/`) and their runner scripts;
- `cli-identity/`: `--json validate` byte comparison against develop;
- `mutants/`: `report.json` and one log per mutant;
- `scale/`: binaries with `SHA256SUMS`, `run.sh`, per-run `wait4` records and
  `results.tsv`;
- `python_check.py`: the lazy-sequence check of the Python binding.

Scale archives are generated under `~/.cache/rusty-tiles-scale/archives/` by
`bench/scale_gates/gen_archive.py` (`implicit --levels A --subtree-levels A
--content-levels A` for `vsa-aA`: one triangle GLB per tile, so members = tiles + 3;
`combined` for the gate (d) archive).

## Controls

In `src/archive3tz/read.rs` (tests) and `src/validate/catalog_controls.rs`, run in CI:

- `index_merge_join_matches_the_sequential_reference`: 600 randomly defected indexes
  (swaps, wrong MD5s and offsets, repeated and self-referencing rows, reversed tails)
  against the pre-slice algorithm, kept in the test as the reference, under both
  profiles. Each verdict (admitted, "not ordered", "differs") occurs at least 50 times.
- `diagnosis_resolves_md5_collisions_by_offset`: the exact diagnosis on synthetic MD5
  collisions (valid UTF-8 names cannot collide), plus 2,000 random cases against the
  reference.
- `forced_spill_profile_uses_scratch_and_default_does_not`,
  `duplicate_and_overlap_failures_agree_across_profiles`: lookups, ranks, names and
  failures agree; scratch exists only in the forced profile and is removed.
- `payload_rows_are_in_uri_order_across_shared_long_prefixes`,
  `unreferenced_entries_list_sixteen_names_and_count_the_rest`,
  `profiles_agree_on_reports_and_failures`,
  `vector_build_digest_and_conversion_marker_survive_the_hash_pass`: both profiles,
  and the streamed CLI JSON equals the former `Value` output for every report.
- `spill::tests::deferred_scratch_exists_only_after_the_first_spill_file`.

CI also re-runs `validate::`, `archive3tz::` and both oracle parity modules with
`RUSTY_TILES_FORCED_SPILL=1`.

## Results

Sources: branch commits are listed in each run's `source.txt`/`SOURCES`; develop is
`957d252` (H7 merged) with the same test-only scale driver added.

**Parity (release, both profiles).** `c2_oracle_parity` (slice-1 witnesses and
controls, C2-WB classes, H11 fast-path controls, slice-3 controls, forest witnesses
and limit pairs, the S3-18 sweep), `i1_oracle_parity` (I1 witnesses and controls, W
corpus), the EM controls and every `validate::` and `archive3tz::` test passed with 0
failures in the resident profile and with `RUSTY_TILES_FORCED_SPILL=1` (158 tests).
Nothing was skipped. The C1 CI oracles (`c1_validation_oracle`, `c1/resource_probe`,
`c1_producer_alignment`, `archive_read_oracle`, `payload_integrity_oracle`,
`payload_meshopt_stride_oracle`) passed against a debug binary and against a
`validate-forced-spill` debug binary with the variable set (which was checked to
spill: pointing `TMPDIR` at a missing directory makes it fail with
"validation scratch: scratch directory does not exist").

**CLI byte identity.** `--json validate` stdout and exit status were byte-identical
to develop on all 599 distinct archives (by SHA-256) under the evidence and scale
trees: 249 accepted, 116 unsupported, 234 invalid or over a limit. The largest
report is 7.6 MB (87,381 payload rows). No archive in the corpus has more than 16
unreferenced members, so the capped message does not occur there; the control above
covers it.

**Python.** `report["payloads"]` is a `ValidationPayloads`; its length, indexing
(negative and out of range), slices and iteration equal the CLI rows on a
21,845-row archive, with 7 KB of traced Python memory while reading them. The
validation tests of `bindings/python/tests/test_api.py` pass.

**Scale** (release, wait4 peak RSS, CPUs 12-15, warm page cache, the second of two
runs; volume caps raised, visits and all other limits at defaults):

| Archive | Members | Stored bytes | develop RSS | VS-A RSS | develop s | VS-A s | Outcome |
| --- | --- | --- | --- | --- | --- | --- | --- |
| `vsa-e14` | 21,848 | 15 MB | 111 MiB | 97 MiB | 0.36 | 0.33 | accepted |
| `vsa-a9` | 87,384 | 60 MB | 219 MiB | 178 MiB | 1.36 | 1.21 | accepted, full C2 |
| `vsa-a10` | 349,528 | 239 MB | 190 MiB | 104 MiB | 1.44 | 0.89 | visit limit at subtree admission |
| `vsa-a11` | 1,398,104 | 960 MB | 541 MiB | 132 MiB | 7.6 | 4.6 | visit limit |
| `vsa-a12` | 5,592,408 | 3.8 GB | 1,945 MiB | 133 MiB | 38.6 | 18.6 | visit limit |
| `implicit-2p22` (d) | 4,100 | 1.2 MB | 235 MiB | 227 MiB | 0.15 | 0.15 | visit limit |
| `combined-d` (d) | 4,184 | 5.0 GB | 241 MiB | 231 MiB | 2.79 | 2.93 | visit limit |
| `large-4p5g` | 41 | 4.9 GB | 194 MiB | 187 MiB | 3.7–3.8 | 3.7–3.9 | accepted |

`vsa-a10` and smaller keep the catalog resident; `vsa-a11` and `vsa-a12` spill it.
From 1.4 M to 5.6 M members the peak RSS is flat (132 to 133 MiB): both build sorters
are at their run caps and lookups use fences and the page cache. develop grows by
about 350 bytes per member. The visit-limited runs stop at I1 subtree admission, so
they measure the catalog, the index check and the hash pass of every stored byte.
The (d) archives have few members, so their peak is the traversal's, which VS-B
bounds.

Times for `vsa-a10` to `vsa-a12` are the minimum of three alternating runs
(`timing.tsv`); the host was shared (load 11 to 22 on 20 CPUs), so single runs vary
by up to 1.6×. Per member, VS-A takes 2.6, 3.3 and 3.3 µs at 0.35, 1.4 and 5.6 M
members: linear from 1.4 M up (1.02× per member for 4× the members), with the spill
taking over above 96 MiB. develop takes 4.1, 5.4 and 6.9 µs (1.27× per member for
4×). `large-4p5g` alternated four times on each binary: the medians agree (3.8 s).

## Mutants

`python3 -I bench/architecture_audit/vs_a_catalog/mutants.py OUT_DIR` from the
repository root with `src/` committed and `TMPDIR` set away from `/tmp`. It makes one
release build per mutant, under the shared Cargo lock.

All 16 rows at `15d9213` (`mutants/report.json`, sha256 `6652b4ad…`): `clean` failed
nothing and each of the 15 mutants was caught by at least one control. At the final
source `2e6b446`, `clean`, `index-order-tie`, `vector-digest-dropped` (whose first
form did not compile, so it is not counted from the first run) and
`page-cache-stale` were run again and caught (`mutants-final/report.json`, sha256
`d745f21b…`; `page-cache-stale` was re-run in `mutants-final-rerun/` after a rustc
SIGSEGV aborted its first build).

| Mutant | Caught by |
| --- | --- |
| `index-skip-diagnosis` | reference index check, index CRC/association tests |
| `index-order-tie` | reference index check |
| `diagnosis-no-repeats`, `diagnosis-absent-late` | collision diagnosis, reference index check |
| `duplicates-undetected` | duplicate tests (both profiles) |
| `extents-ignore-next` | nested-record overlap test |
| `fence-page-late`, `page-cache-stale` | forced-spill lookups, profile agreement, URI order |
| `memo-stale` | 31 tests |
| `closure-shows-seventeen`, `closure-keeps-largest` | closure cap control |
| `conversion-unmarked`, `vector-digest-dropped` | hash-pass control |
| `rows-run-start` | URI order across shared prefixes |
| `json-row-separator` | streamed-JSON identity in the report controls |

## Not established

- Whole-validation RSS is not flat in tiles: the C2 tile arena, reference list and
  per-URI table, the external-document memo and live documents are VS-B's.
- The `used` bitmap is `8·⌈N/64⌉` bytes (21.3 MiB at the member ceiling).
- Timings are from a shared host, release builds, warm page cache.
- `spill::tests::spill_memory_is_flat_in_input_size` fails on this host on develop
  `957d252` as well as on this branch (VmHWM growth 11.3-11.6 MiB against an 11 MiB
  allowance in a debug build); it is not changed here.

## Audit fixes (`85f5c16` and later)

The nonauthor audit (`~/.cache/rusty-tiles-evidence/vs-a-20261013/audit.md`) was
GO-with-fixes. Changes:

- **F1:** `validate::rows::tests::deeper_passes_order_rows_whose_log_order_is_not_uri_order`
  (a log in shuffled, reversed order, ties at depths 1 and 2, names ending on a chunk
  boundary, both profiles) and
  `catalog_controls::payload_rows_follow_uri_order_when_the_directory_does_not` (a
  hand-built 3TZ with the central directory in reverse URI order). The audit's surviving
  mutant is `rows-tie-previous` in `mutants.py`.
- **F2:** the duplicate scan reads names only inside an equal-key run.
- **F3:** each MSD pass reads only the 64-byte chunk it orders, so a name of length ℓ
  costs O(ℓ) in all.
- **F5 (scratch placement):** the first disk-backed candidate with room among `$TMPDIR`,
  the user cache directory (`rusty-tiles/scratch`) and the system temporary directory.
  Memory-backed candidates are skipped. When only those exist, `validate` uses one and
  reports `scratch.memoryBacked: true`; `validate_3tz` refuses. The report's `scratch`
  member appears whenever anything spilled. A run with `TMPDIR` unset created
  `~/.cache/rusty-tiles/scratch` and left it empty afterwards.
- **F4:** the bitmap at the ceiling is 21.3 MiB.
- **F6:** the later passes and the placed rows of the report sort use the 24 MiB side
  runs; the contract's memory table states the report-time total.
- **F7:** `catalog_controls::scratch_failures_map_to_io_resource_limit_and_invalid_state`,
  `spill::tests::disk_placement_skips_memory_backed_candidates_and_reports_a_last_resort`
  and the Python lazy-sequence assertions in `test_api.py`
  (`test_validate_reports_decoded_containment…`). The changelog names the
  `==`/`json.dumps` breaks.
- **Audit 5:** the alignment of `payload_counts` with the C2 URI facts is a hard
  `InvalidState` check.
- **Flaky test:** `spill_memory_is_flat_in_input_size` now uses the sorter's declared
  `memory_bound` as its allowance, and compares growth over each child's own baseline
  for flatness. It passed 8 of 8 runs.

At `85f5c16`, release parity (`c2_oracle_parity`, `i1_oracle_parity`, `validate::`,
`archive3tz::`, `spill::`) passed 182/182 in the resident profile and 182/182 in the
forced-spill profile (`parity/audit/`). The Python containment and implicit tests pass
against a release extension built from `85f5c16`.

Audit mutants (`mutants-audit/report.json`, sha256 `19ebcfe5…`, lock A, `-j 2`):
`clean` failed nothing; `rows-tie-previous` (the audit's survivor), `rows-run-start`,
`scratch-ram-silent`, `scratch-report-dropped`, `page-cache-stale` and
`json-row-separator` were each caught.
