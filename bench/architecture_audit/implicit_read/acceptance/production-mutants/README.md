# I1 production implementation mutants (contract §D; final review E1)

Each `<name>.patch` is a `git diff` against `387a7bb`. It introduces one implementation error from
the I1 contract's §D table into the production owner (`src/content_integrity/implicit*`) or B's
integration (`src/validate.rs`). The oracle lists no mutants of its own, so these 35 are the
contract's complete list. `scripts/i1_mutants.py` generates the patches from exact-string edits,
and `scripts/run_i1_mutants.sh` runs them.

`report.json` records, for each mutant:
- the patch sha256 and the paired control;
- every control and corpus artifact the parity harness flags (`controlsFlipped`,
  `corpusFlipped`);
- whether its paired control flipped;
- the I1 unit tests that fail (`content_integrity::implicit` and `validate::tests::i1_*`);
- for the three timing mutants, the timed case's wall time under the contract's timeout;
- a diagnosis wherever the paired control does not flip.

The clean tree gives 181/181 controls, 41/41 corpus artifacts and all unit tests passing. Clean
timed runs: R7 0.16 s, R7c 0.42 s, R8a 0.69 s.

## Results

| Outcome | Mutants |
| --- | --- |
| Paired control flips in parity (28) | inclusive-rank, drop-child-implies-tile, drop-available-count, key-by-name, ignore-override, swap-x-y, error-not-halved, float32-with-float64-stride, case-sensitive-suffix, read-scale, ignore-content-uri, ignore-content-group-id, drop-repeated-subtree-check, scan-per-object, per-visit-charging, live-path-current-subtree-only, cumulative-instead-of-live, refine-forced-replace, padding-0x20-only, skip-post-walk-content-rule, absolute-1e-6, detect-on-raw-last-segment, no-variable-survival-check, escape-guard-last-segment-only, check-only-content-index, header-metadata-not-classified, and the timing mutants no-source-memoization (R7 > 60 s) and resolve-raw-template-per-emission (R8a > 300 s) |
| Detected in parity, but not by the paired control (3) | **msb-first**: A-3 stays InvalidInput with another reason; 78 controls and 18 corpus artifacts flip. **ignore-trailing-bits**: A-5's trailing bit also changes the popcount, so `availableCount` still refuses it; A-12c.trailing flips. **admit-json-templates**: C-6 and W5u stay Unsupported through C1's payload refusal; C-4 and the never-available C-7 forms flip |
| **Flip nothing in parity (4)** | **drop-content-implies-tile** (A-6): the stray content bit is indexed to a neighbouring tile, so A-6 stays InvalidInput as "missing archive entry". **ge-vs-gt-on-A** (A-8): the child subtree it admits is absent, so A-8 stays InvalidInput as "missing archive entry"; A-9 exercises the 32-level cap, a different comparison. **reservation-after-rank-table** (R5b): the verdict is unchanged; only the rank-table allocation differs. **per-table-class-scan** (R7c): stays Accepted in 17.4 s against 0.42 s clean, which breaks R7c's 5 s bound but not the contract's 60 s mutant timeout |

The contract counts a mutant that flips nothing as a failed control, so those four are **failed
parity controls**. Each is caught by the in-crate unit tests and probes:
- `content_layers_and_content_implies_tile`;
- `levels_rule`;
- the `RANK_ALLOCATIONS` probe;
- the `CLASS_SCANS` probe.

33 of 35 mutants fail at least one unit test. absolute-1e-6 and resolve-raw-template-per-emission
fail none, but parity catches both. **No mutant escapes both.**

Each failed parity control can be made discriminating by its owner:
- **A-6 and A-8**: ship the members the defect would name, so the only refusal left is the
  availability rule.
- **R5b**: production exposes no allocation count in the report, so only the probe can see it.
- **R7c**: tighten the timed bound to the contract's 5 s.

## Replay

From a checkout of `387a7bb`, with `witnesses/generate.py --large` run and the W corpus present:

```sh
python3 -I scripts/i1_mutants.py check . /tmp/m                     # every anchor occurs once
scripts/run_i1_mutants.sh "$PWD/bench/architecture_audit/implicit_read/acceptance/production-mutants" /tmp/i1m
```

The runner's repository path (`R=`) points at a dedicated worktree. It applies each patch, builds
the release lib-test binary under the shared cargo lock, runs it on CPUs 0 and 1, and reverts
`src`. Check a patch with `sha256sum <name>.patch` against `report.json`.
