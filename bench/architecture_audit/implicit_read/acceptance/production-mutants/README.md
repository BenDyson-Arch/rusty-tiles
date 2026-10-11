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
- `boundBreaches` (a timed control past its own bound) and `probeClausesFailed`;
- a diagnosis (`diagnoses.json`) where it is not obvious.

The clean tree gives 181/181 controls, 41/41 corpus artifacts and all unit tests passing. Clean
timed runs: R7 0.16 s, R7c 0.42 s, R8a 0.69 s.

## Results

**How a control counts as flipped.** `scripts/build_report.py` decides this, re-applied to the
same runs after the re-check (`i1-recheck.md`). A control flips when any of these holds:
- production disagrees with the oracle;
- a timed control misses its own contract bound: R7, R7a, R7b, R7c and R8 within 5 s; R8a within
  2× R8a.nopad and within 60 s;
- for R5b, its allocation-probe clause fails
  (`availability::tests::visit_reservation_precedes_rank_table_and_tables`).

The runner's 60 s and 300 s timeouts only cap a run. Under these rules:

| Outcome | Mutants |
| --- | --- |
| Paired control flips (30 of 35) | all mutants except the five below, including:<br>- **per-table-class-scan**: R7c is Accepted in 17.4 s against its 5 s bound<br>- **reservation-after-rank-table**: R5b's probe records a rank-table allocation<br>- **no-source-memoization**: R7 exceeds the 60 s cap<br>- **resolve-raw-template-per-emission**: R8a exceeds the 300 s cap |
| Detected only by other controls (3) | **msb-first**: A-3 stays InvalidInput with another reason; 78 controls and 18 corpus artifacts flip.<br>**ignore-trailing-bits**: A-5's trailing bit also changes the popcount; A-12c.trailing flips.<br>**admit-json-templates**: C-6 and W5u stay Unsupported through C1's payload refusal; C-4 and the never-available C-7 forms flip |
| No control flips (2) | **drop-content-implies-tile** (A-6) and **ge-vs-gt-on-A** (A-8). The member each defect would name was absent, so both stay InvalidInput as "missing archive entry". The oracle amends A-6 and A-8 so that only the content ⇒ tile rule and the levels rule refuse them; see "A-6 and A-8 after the oracle amendment" |

33 of 35 mutants fail at least one unit test.

## Replay

From a checkout of `387a7bb`, with `witnesses/generate.py --large` run and the W corpus present:

```sh
python3 -I scripts/i1_mutants.py check . /tmp/m                     # every anchor occurs once
scripts/run_i1_mutants.sh "$PWD/bench/architecture_audit/implicit_read/acceptance/production-mutants" /tmp/i1m [NAME,...]
```

The optional third argument reruns only the named mutants; the clean baselines always run. The
runner ends with `scripts/build_report.py`, which writes `report.json`.

The runner's repository path (`R=`) points at a dedicated worktree. It applies each patch, builds
the release lib-test binary under the shared cargo lock, runs it on CPUs 0 and 1, and reverts
`src`. Check a patch with `sha256sum <name>.patch` against `report.json`.
