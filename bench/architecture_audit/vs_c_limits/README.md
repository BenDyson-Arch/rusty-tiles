# VS-C limits oracle

Independent oracle and fixture generator for validator slice VS-C
(`docs/architecture/validator-limits-amendment.md`).

Independence: both scripts were authored from the amendment, the two archive-format
notes it links, and the ZIP/ZIP64/3TZ format only. The author did not read `src/`,
`tests/` or `bindings/`. Python 3 stdlib only; run with `python3 -I`.

## limits_oracle.py

- `parse(path)` returns `(N, S)`: entry count (index included) and the sum of stored
  sizes from the central directory (ZIP64 aware). No member data is read. S is not the
  file length.
- `expected(S, overrides)` returns `(limits, limitDerivation)` per VS-C-1, VS-C-3 and
  VS-C-4 (21 `limits` keys; model 1).
- `python3 -I limits_oracle.py self-test` runs the unit checks (crossovers, saturation,
  overrides, in-memory and hand-built ZIP64 parsing).
- `python3 -I limits_oracle.py check --binary BIN [--maxVolume KEY=N ...] [--list FILE]
  [--jobs K] [--timeout SECS] [ARCHIVE ...]` runs `BIN --json validate ARCHIVE
  [--maxVolume KEY=N ...]` per archive and compares `storedBytes`, `limits` and
  `limitDerivation` (exit 0) or the structured `resource_limit` error (nonzero exit)
  with the oracle. Prints one JSON line per archive and a final
  `{"archives","compared","disagreements"}` line; exit 1 on any disagreement.
  `FILE` is a JSON array of path strings or objects with an `archive` key.
- `python3 -I limits_oracle.py parse ARCHIVE` prints N and S.

## sparse_archive.py

`python3 -I sparse_archive.py OUT.3tz [--members K] [--member-bytes B]
[--force-zip64-eocd]` writes a valid stored 3TZ (default 9 GLB members of 120 MiB,
about 1.05 GiB) whose GLB payloads are filesystem holes. GLBs are glTF 2.0 POINTS
meshes; tile boxes are huge so containment cannot fail; members are `tiles/tN.glb`.
CRCs are computed over the real (zero) bytes by streaming 1 MiB blocks; ZIP64
records are written where sizes, offsets or the total require them. Prints one JSON
line with `path`, `length`, `N`, `S`. Peak memory is small; delete the file after use.

Author correction (VS-C author, 2026-10-14): `sparse_archive.py` first ordered the 3TZ index by
MD5 byte order, which the validator refuses ("3TZ index hashes are not ordered"); 3TZ orders it
by the MD5 read as two little-endian u64s, as the producer writes it. Only that sort key changed.

## mutants.py

The VS-C production mutants (plan m1–m16 plus four structural ones), each a set of exact
string edits run against its suite (`lib`: release unit controls; `cli`: the CLI contract
test; `py`: the Python binding test against a freshly built extension). `clean*` must pass;
every other mutant must be caught. `python3 -I bench/architecture_audit/vs_c_limits/mutants.py
OUT_DIR [name ...]` from a clean checkout; it writes `OUT_DIR/report.json` with each patch and
its SHA-256.

## Evidence (bulk, not in git)

`~/.cache/rusty-tiles-evidence/vs-c-20261014/` (README there), at source `4e55659`:

- `oracle/corpus.jsonl` (sha256 `40f5afae…`): this oracle against the release CLI on the 599
  distinct archives of the VS-A identity corpus: 275 compared (263 accepted, 12 structured
  refusals), 0 disagreements.
- `parity/parity.json` (`b7c6ae21…`): CLI against the VS-B base `3c424f4`: 591 identical after
  removing the VS-C keys and the suffix; 7 differ only by the renamed message "member reads
  exceed work limit"; `vsa-a11` (over 2^19 entries) is accepted instead of refused.
- `real/` (VSC-REAL, the 8.1 GB H16 archive at defaults): `report.json` (`3498fca2…`),
  `resources.json`, `oracle.jsonl`, `scale-driver.txt`, `python.json`, the sensitive pair
  (`pre-vsc.json`, `capped.json`).
- `k4-sparse/`, `k5-hostile/`, `mutants/report.json` (`2d7958ee…`: 21/21 mutants caught, the three clean suites pass).
