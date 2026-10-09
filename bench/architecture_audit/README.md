# Architecture audit evidence for #113

Baseline production revision: `8dfd74bd87dd23c86278e98d736c3f5912c246cf`.
See the [architecture proposal](../../docs/architecture/README.md) and
[release-blocking issue](https://github.com/BenDyson-Arch/rusty-tiles/issues/113).

The audit is replacement-first: current code must earn retention against an
independently defined contract. See the [component evidence ledger](../../docs/architecture/evidence-ledger.md).
These are executable observations, not tests that preserve undesirable behavior.
No production changes implement the proposed architecture in this audit. As a
fix lands, convert the corresponding scenario into a desired-behavior regression
in the ordinary Rust/Python suite. A probe's successful execution means it
collected evidence, not that the release gate passes.

| Evidence | Method | Scope |
| --- | --- | --- |
| [API probes](api/README.md) / [results](api/results.json) | Build unmodified portable CLI/core/Python binding, generate invented GLBs, compare decoded archive members and inspect output after exceptions | Geographic offsets, rotation-only requests, omitted texture choice, callback failure/publication |
| [Runtime probes](runtime/README.md) / [results](runtime/evidence/publication-vector.json) | Natural vector conversions plus clearly identified isolated source fault injection | Feature rollback/reporting, source callback I/O failure, directory publication race, orphan-content control |

The individual READMEs contain replay commands and prerequisites. Build targets
and generated inputs/outputs live outside tracked source. Results record the
production revision and toolchain; generated fixture hashes and injected-source
provenance distinguish actual baseline execution from controlled modifications.
The direct Python extension probe is not installed-wheel/Blender acceptance.
Linux observations do not establish Windows/macOS behavior or native backend
coverage. No full candidate acceptance was rerun for this documentation/probe
change.

## Findings supported by execution

- Legacy geographic source offsets are ignored by the exercised Rust/Python
  mesh path, with identical logical archive payloads to the no-offset controls;
  CLI rejects the request.
- Rotation without manual placement is similarly ignored by the exercised
  Rust/Python mesh/wrap paths; CLI rejects it.
- Forced texture processing with omitted codec selects PNG in Rust/Python and
  JPEG in CLI. The fixture bypasses unchanged-model wrapping, so this measures
  the encoder default rather than the source's embedded image format.
- Python callback exceptions, both at the first event and the existing
  post-publication event, leave archives that the CLI validator accepts.
- A naturally rejected oversized polygon can leave fragmentation counters and
  a success-like JSONL record even though its SQL rows were rolled back and the
  final conversion reports it skipped.

Controlled failure/race outcomes are documented separately in the runtime
report. They must not be described as naturally occurring failures or as a
measurement of how often a race happens.

## Correction to the first static pass

The initial suspicion that rejected candidate tiles leak into the final archive
missed the cleanup in `src/vector/pipeline/reuse.rs:424–429`. That cleanup removes
unreferenced generated tile files before publication. The exercised vector
control contains no orphan content. This one case is not proof of the whole cleanup/reuse subsystem;
speculative content still has temporary-space and lifecycle costs, but this
pass does not demonstrate a final-output leak.

The original issue audit comment has been corrected. Memory scaling, global
metric contamination, platform publication behavior beyond the exercised case,
and full native/frontend acceptance remain separate work; the architecture
proposal must not present them as measured results.
