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
