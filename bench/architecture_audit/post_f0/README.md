# Post-F0 audit evidence (2026-10-09)

Contract/ownership/evidence audit baseline:
`d69ba2a7facb99b5b3f5a6efba4e4039bef26be5`. Three GPT-6.1 Sol agents conducted
separate reviews; coordinator consolidation and issue order are in
[the scoped roadmap](../../../docs/architecture/next-foundation-scopes.md).
These files preserve observations and limitations, not completed implementation
acceptance for the newly scoped work.

## Source reviews

- [API and directory/vector contracts](api-audit.md)
- [Ownership and resource disposition](ownership-audit.md)
- [Independent evidence and oracle provenance](evidence-audit.md)
- [Final F0 acceptance review and destination-binding defect](f0-acceptance-review.md)

The first three reviews' no-new-F0-blocker statements apply to their respective
limited reviews. The subsequent targeted acceptance review found and fixed the
relative-output/CWD issue in `c777180`; it takes precedence for F0 acceptance.
Raw reports retain their original temporary paths/commands and proposed scope;
this index and the consolidated roadmap identify durable files and adopted scope.

## Validator probe

[validator_probe.py](validator_probe.py) independently constructs one valid GLB,
a declared POSITION view without its binary bytes, and valid geometry outside
its declared tile box. It packages these through the intentionally opaque F0
operation, then records actual validator status/JSON. It also records the legacy
CLI geographic-offset refusal. It uses the standard-library fixture generator
from the earlier API audit, not the production scene writer. The generator's
hash and each fixture hash are recorded.

```sh
python3 bench/architecture_audit/post_f0/validator_probe.py \
  --binary /absolute/path/to/rusty-tiles \
  --output /tmp/validator-observations.json \
  --build-source-revision YOUR_COMMIT \
  --build-features YOUR_FEATURES \
  --build-provenance YOUR_BUILD_DETAILS
```

`--repo` is optional when the script is inside this checkout. The source revision
and feature metadata are supplied provenance, not extracted or authenticated from
the executable. The probe records the actual binary hash. A zero probe exit means
observations were collected, **not** that malformed inputs were rejected or that
the implementation passed an acceptance gate.

- [Baseline native observations](baseline-validator-observations.json): binary
  `738eebfb…`, source metadata `d69ba2a`, captured by the evidence agent.
- [Coordinator replay](validator-observations.json): separately built current
  binary/source provenance recorded in the JSON. All three validator inputs
  still return success, while the CLI offset request returns exit 3.

Missing required BIN data exposes payload-check coverage. The content-bound case
shows a limitation as a mesh oracle; the current CLI only promises child-bound
containment, so this is not automatically a violation of that narrower documented
contract. Neither invalid scene is an F0 opaque-packaging failure.

## Sampled LOD estimator probe

[baseline_lod_probe.rs](baseline_lod_probe.rs) records the original direct call with a flat
parent triangle and 3000 child triangles, including a 100 m spike skipped by the
bounded sampling stride. [Captured observation/provenance](lod-observations.json)
records 0 m estimated versus exactly 100 m independent vertex distance. The
source hash, library hash, compiler and exact original command are retained.
The coordinator [replay with controls](lod_probe.rs), recorded in
[lod-replay.json](lod-replay.json), returns 0 for flat geometry, about 66.67 m
when the same spike is at triangle zero, and 0 when it is at triangle one.
The independent spike distance remains 100 m; ordering affects the estimate.

Build the declared library first and select its matching rlib/dependency cache:

```sh
rustc --edition=2021 -C debuginfo=0 \
  bench/architecture_audit/post_f0/lod_probe.rs \
  --extern rusty_tiles=/absolute/path/to/matching/librusty_tiles-HASH.rlib \
  -L dependency=/absolute/path/to/matching/deps -o /tmp/lod-probe
/tmp/lod-probe
```

This establishes that the sampled estimator is not a universal upper bound. It
is **not** an observed end-to-end converter underbound: the supplied parent was
constructed independently, and existing format docs already disclaim a universal
visual-error guarantee. The next contract must decide what geometricError means.

## Destination binding regression

[f0_cwd_probe.rs](f0_cwd_probe.rs) preserves the isolated public Rust probe. It
changes CWD only in its own process, originally causing a relative `Replace` to
overwrite a stable selected source. The maintained regression in
`tests/package.rs::relative_output_is_bound_before_observer_changes_working_directory`
self-spawns to avoid contaminating parallel tests. After `c777180`, output stays
at its checked location and the selected source remains unchanged. Use the same
matching-rlib build pattern above to replay this standalone observation.

The [F0 evidence](../../../docs/architecture/f0-evidence.md) records final suites,
wheel identity, platform CI and scope limits. No real disk-full event, broad
numerical corpus, full public dataset audit or release promotion was performed
by these probes.
