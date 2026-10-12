# I1 implicit read evidence

> **Bulk evidence moved out of the tree (2026-10-12).** 173 of 461 files remain
> (what CI or tests read); this README is new. The full directory last existed at
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

Removed from the tree: `preparation/` (contract drafts v1-v3, precode reviews, primary
fetches), `acceptance/` receipts and production-mutant runs, large generated controls
and witness outputs.
