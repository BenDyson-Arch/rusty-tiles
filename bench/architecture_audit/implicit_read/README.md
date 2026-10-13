# I1 implicit read evidence

> **Bulk evidence moved out of the tree (2026-10-12).** 227 of 461 files remain
> (what CI or tests read, plus the oracle, witnesses, self-test and mutant set);
> this README is new. The full directory last existed at
> `e29cd19d2f236dc688e91a80a45170d4a573e20b`
> (`git archive e29cd19d2f23 bench/architecture_audit/implicit_read | tar -x`). Local archive:
> `~/.cache/rusty-tiles-evidence/architecture_audit/implicit_read.tar.zst`, SHA-256
> `dbdf618cf551ff5be3cbdf0905e120087327dd9e6683c58409abfeae2e30491b`.

Contract: [I1 implicit read](../../../docs/architecture/i1-implicit-read-contract.md).
`acceptance/oracle_parity.rs` is compiled into the library tests (`src/lib.rs`) and
compares the production implicit reader with the frozen independent oracle
(`oracle/i1_oracle.py`) over `controls/` and `corpus/`.

Conclusions recorded in the removed `acceptance/README.md`:

- At combined head `e3a67bf` the reader failed on one production defect
  (B-10.subnormal); every other case agreed or differed for a diagnosed oracle-side reason.
- After the owner fix `5ffbf71` and oracle amendment `ce9838e`: 179/180 controls agree,
  1 diagnosed oracle-side difference (C-11b), 0 production disagreements; corpus 41/41.
- Final (`3c2f81a` + oracle amendment 4 `1d2b5e9`): **PASS, 181/181 controls agree,
  0 oracle-side differences, 0 production disagreements, 0 skipped** (release build,
  large controls generated).

`acceptance/scripts/produce.py` also writes an `--explicit` twin of every case. H6 (2026-10-13)
removed `--explicit` from `vector` and `point-cloud`, so producing those twins needs a converter
built at or before develop `7fad70c`; `mesh-to-3tz` keeps the flag.

Kept as live tooling: `oracle/` with its self-test, `witnesses/`, `acceptance/scripts/`,
the production-mutant set (patches, `mutants.json`, scripts) and `corpus/run.py`.
Removed from the tree: `preparation/` (contract drafts v1-v3, precode reviews, primary
fetches), `acceptance/` receipts and mutant reports, and generated `corpus/facts/`.

## W corpus archives live under `~/.cache/rusty-tiles-w-corpus`

`corpus/manifest.json` pins the 41 W archives by sha256; `oracle_parity.rs` skips any that is
missing. Seven of them (14 manifest entries: `pc-perf-2000`, `pc-deep`, `vec-perf`, and the four
`a2fix/*` archives) were built into `/tmp`, a RAM tmpfs, and a reboot lost them (H7 then ran 27/27).
They now live in `~/.cache/rusty-tiles-w-corpus/samples/`, and `corpus/regenerate.py` rebuilds any
that is missing or re-hashed (`python3 -I corpus/regenerate.py`; it refuses `/tmp`). It needs the
pre-forest producer binary `f2238cd` (`~/.cache/c2-corpus/rusty-tiles-f2238cd`, sha256 `369c3c81...`),
the pinned perf-lane inputs (`bench/perf_lane/corpus.py`) and the retained `cloud.las`.
Six archives regenerate byte-identically. `vec-perf` embeds the PROJ version, and the PROJ 9.8.1 ->
9.9.0 upgrade of 2026-10-13 changed its bytes, so W5u was re-pinned (verdict unchanged:
Unsupported). Check: `cargo test --lib production_matches_the_oracle_on_the_w_corpus` on develop `df3f7a5`
runs 41 of 41, with none skipped.
