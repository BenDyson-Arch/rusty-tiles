# C2 production implementation mutants

Each `<name>.patch` is a `git diff` against the C2 integration head `1c9783b`. It
introduces one plausible implementation error into the production C2 code
(`src/validate.rs`, `src/validate/containment*.rs`, `src/content_integrity/payload/scene.rs`).
Twelve patches mirror the oracle's same-named mutants in `../../oracle/mutants/patches/`.
The remaining seven are production-specific. `report.json` records, for each mutant, the
patch sha256 and the fixtures the oracle-parity acceptance test flags (`disagree`). It also
records whether the paired fixture flips. For an oracle mirror, the paired fixture is the
oracle mutant's `namedFixture`.

A mutant is detected when the parity test fails with a non-empty `disagree` list. The
clean tree must give 257/257 agree.

## Replay

From a checkout of `1c9783b` (repository root):

```sh
mkdir -p /tmp/c2m
for p in bench/architecture_audit/c2_decoded_containment/acceptance/production-mutants/*.patch; do
  n=$(basename "$p" .patch)
  git apply "$p"
  C2_PARITY_OUT=/tmp/c2m/$n.json cargo test --locked --release --lib c2_oracle_parity  # expected to FAIL
  git checkout -- src
  python3 -c "import json,sys; s=json.load(open(sys.argv[1])); print(sys.argv[2], len(s['disagree']))" /tmp/c2m/$n.json "$n"
done
# Clean tree: must pass with agree == 257.
C2_PARITY_OUT=/tmp/c2m/clean.json cargo test --locked --release --lib c2_oracle_parity
```

Verify a patch hash with `sha256sum <name>.patch` against `report.json`.

## C2-WB re-pin (2026-10-12)

Two slice-1 patches no longer applied at the C2-WB head, whose `src/validate.rs` moved. The
same edits are re-expressed against `feat/c2-work-bound` (`src/validate.rs` blob `b5c9dc0`) and
re-run with the release parity test (the three `--large` controls absent; clean 258/261 and
59/59 `wb/` CI rows). Each `report.json` entry keeps its slice-1 record and adds `rePin`.

| Mutant | sha256 | Flipped | Paired fixture |
| --- | --- | --- | --- |
| `precheck-depth-cap-off-by-one` (`>=` at the `containmentNodeDepth` check) | `02c8ce35612542979e8afa91f1437705ebec248b0de6c7711017b253ca8b2b52` | 1 | `work-containmentNodeDepth-at-sum` flips |
| `reverse-tile-order` (level `l` takes the root-side transform) | `cbf818dc63dad775739c715add462c23d21f9bdce017c7faea56da0b4b2dd1a9` | 19, plus `wb/` `terms-parent-step` and `terms-parent-sphere-step` | `W01-noRTC-variant` flips |
