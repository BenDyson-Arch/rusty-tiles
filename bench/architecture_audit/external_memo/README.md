# EM: external-tileset per-document memo (author evidence, 2026-10-13)

Contract: [c1-external-tileset-memo-amendment.md](../../../docs/architecture/c1-external-tileset-memo-amendment.md).
This is author evidence. The nonauthor review and the coordinator's gate are pending.

Bulk evidence is kept out of the tree, in `~/.cache/rusty-tiles-evidence/ext-memo-20261013/`:
- `runs.tsv` and `deep.tsv`: the hostile archives;
- `parity/`: parity summaries and the run log;
- `c1/`: the C1 oracle outputs;
- `mutants/`: `report.json` and the per-mutant logs;
- `scripts/hostile.py`: the timing driver.

The hostile archives come from `ext-amplification-20261013/scripts/dag.py`.

## Controls

The author controls are in `src/validate/memo_controls.rs`. They run in CI (debug)
with a small corpus. The release evidence run used:

```
EM_SEEDS=300 EM_BYTE_POINTS=200 cargo test --release --lib validate::memo_controls
```

The EM-1 comparison checks, against the reference traversal (memo off):
- the serialized report, or the error category and message;
- the C2 work units;
- the final visit, reference and byte counters.

## Mutants

Run `python3 -I bench/architecture_audit/external_memo/mutants.py OUT_DIR` from the
repository root with `src/` committed. It makes one release build per mutant, under
the shared Cargo lock.

## Results

All results are at source `467a78d`. The release binary has SHA-256
`5dfe618d…3d58b55`. Timings were taken on CPUs 4-7 against the develop binary
`87b7078` from the findings.

**Hostile archives.** These are the 30 archives from the findings plus 6 deeper ones.
For every archive, the exit code and the output were byte-identical to `87b7078`
once the archive path was removed (`runs.tsv` sha256 `63901f69…`, `deep.tsv` sha256
`7a9ee01d…`).

| Archive | Develop | Memo | Verdict |
| --- | --- | --- | --- |
| `pad-keys-d15` (517,728 B) | 60.5 s | 0.58 s | accepted (both) |
| `combo-keys-d15-m4` | 67.6 s | 2.15 s | accepted |
| `pad-obj-d15` / `pad-num-d15` | 42.2 / 30.1 s | 0.58 / 0.57 s | accepted |
| `pad-obj-d16` / `pad-num-d16` | 41.7 / 25.9 s | 0.09 / 0.09 s | 2 GiB read limit |
| `pure-d15` / `pure-d16` | 0.89 / 0.51 s | 0.56 / 0.08 s | accepted / visit limit |
| `shape-keys-d11` | 3.66 s | 0.07 s | accepted |
| `pad-keys-d20`, `-d40`, `wide-k64-d3`, `wide-k512-d2` | 67-70 s | 0.05-0.19 s | 2 GiB read limit |
| `pad-keys-d100`, `c2-d17-m4` | 10.2 / 0.54 s | 0.31 / 0.09 s | visit limit |

The time left in the accepted d15 cases is the unchanged C2 phase-2 stream of
32,768 or more references. C2-WB bounds it.

**Parity suites (release).** All had 0 disagreements:

| Suite | Agree |
| --- | --- |
| C2 slice-1 | 261/261 |
| C2-WB | 102/102 |
| slice-3 | 80/80 |
| S3-18 sweep | 182 (181 cases) |
| forest | 34/34 + 5 pairs |
| I1 | 181/181 |
| W corpus | 41/41 |

The large fixtures and the `~/.cache/c2s3-evidence` slice-3 and forest-2d85208
inputs were present, so nothing was skipped. The C1 CI oracles all passed against
the memo binary: `c1_validation_oracle`, `c1/resource_probe`, `c1_producer_alignment`,
`archive_read_oracle`, `payload_integrity_oracle` and `payload_meshopt_stride_oracle`.

**EM controls (release, 300 generated archives).** All 8 tests pass, with 0
memo/reference differences
(`logs/memo-controls-release.log`, sha256 `53695432…`).

**Mutants (15 rows).** `clean` failed nothing, and 13 mutants were each caught by at
least one control. `memo-on-active-path` is equivalent by EM-5.5, because the active
check is defensive (`mutants/report.json`, sha256 `b4fcb598…`).

| Mutant | Caught by |
| --- | --- |
| `cached-placement-link` | the two C2 per-placement controls and the generated DAGs |
| `skip-root-bounds` | root-bounds control, `shared_external_tileset_is_checked_at_each_placement`, generated DAGs |
| `skip-root-error` | root-error control |
| `no-visit-charge`, `no-reference-charge`, `no-byte-charge`, `short-visit-charge` | 8 controls each, including every counter cap |
| `once-charges-repeated` | 5 controls, including the counter-cap sweep |
| `skip-node-depth-bound` | depth control, generated DAGs |
| `skip-tileset-depth-bound` | depth control (the 128 tileset-task bound) |
| `replay-without-references` | 4 controls, including both C2 per-placement controls |
| `nested-placement-unrecorded` | counter-cap sweep |
| `memo-disabled` | 8 controls, including the engagement control |
