### Changed

- Development: the validator oracle-parity fixture corpora under
  `bench/architecture_audit/` (I1 implicit read, C2 decoded containment and the C1
  context controls, 669 files) are generated at test time instead of committed.
  `cargo test` runs `scripts/test_corpora.py`, which needs Python 3.12 or newer, and
  checks every file against `bench/architecture_audit/generated-corpora.sha256`.
