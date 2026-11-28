# Architecture audit evidence for #113

> Bulk evidence was moved out of the tree on 2026-10-12; each subdirectory README
> gives its archive SHA-256 and the last commit that contained it
> (`e29cd19d2f236dc688e91a80a45170d4a573e20b`). Policy: [AGENTS.md](../../AGENTS.md).

## Generated test corpora

The fixtures that the I1, C2 and C1-context oracle-parity tests read are generated, not committed
(since 0.4.2; the last commit that held them is the parent of the commit that added
`generated-corpora.sha256`). Their generators are tracked, and
[`generated-corpora.sha256`](generated-corpora.sha256) pins every generated file: one
`<sha256>  <path>` line per file, paths relative to this directory (`sha256sum -c` format).
[`scripts/test_corpora.py`](../../scripts/test_corpora.py) runs the generators and writes the
files in place, where `.gitignore` ignores them:

```sh
python3 scripts/test_corpora.py           # write every missing or different file, then verify all 669
python3 scripts/test_corpora.py check     # verify only
python3 scripts/test_corpora.py update    # rerun every generator, rewrite the manifest
```

Tests that read the files call it first (`src/test_corpora.rs`, once per test process), and CI
runs it before `cargo test`. A file that is missing or differs after generation fails the run; nothing
is skipped. A cold run takes about 2 CPU seconds and a warm one about 0.05 s. Run it yourself
before the Python oracle, mutant and replay scripts in these directories.

| Directory | Generator (run with `--fixtures-only`: fixtures only, committed records unchanged) |
| --- | --- |
| `implicit_read/controls/fixtures/` | `implicit_read/witnesses/generate.py` |
| `c2_decoded_containment/witnesses/fixtures/`, `controls/fixtures/` | `c2_decoded_containment/witnesses/generate.py` |
| `c2_decoded_containment/wb/fixtures/` | `c2_decoded_containment/wb/generate.py` (no oracle run) |
| `c2_decoded_containment/h11/fixtures/` | `c2_decoded_containment/h11/generate.py` (no oracle run) |
| `c2_decoded_containment/controls/slice3/fixtures/` | `controls/slice3/generate.py --pre-amendment`, fed by `controls/slice3/pre_amendment6/run.py` (the I1 generator frozen at 92e8c041 for the S3-19 controls) |
| `c1_payload_integrity/.../context_controls/2026-10-10/fixtures/*/` | `context_controls/2026-10-10/generate.py --generate` into a temporary directory; its `manifest.json` is not copied |

The records the tests compare against (each generator's `manifest.json`, `report.json`,
`report.json.gz`, oracle results) stay committed. After an intended generator change, run the
generator as before to update its records, then `scripts/test_corpora.py update`, and commit both
diffs.

The bytes are the same on Linux, macOS and Windows and from Python 3.12 on:

- Every archive is a stored ZIP (no zlib output) with fixed timestamps. The driver runs each
  generator under a shim that sets `ZipInfo.create_system` to 3 (Unix), the one header field the
  standard library sets per platform, and in UTF-8 mode.
- `.gitattributes` turns off line-ending conversion here, so pinned inputs check out byte for byte.
- Python 3.12 made the built-in `sum()` of floats compensated, and the C2 witness builder sums
  float products with it; 3.10 and 3.11 change six rotated-TRS controls. The driver refuses older
  versions. Checked with CPython 3.12.16, 3.13.18 and 3.14.4 on Linux, each from an empty tree;
  macOS and Windows first run in the portable CI matrix.

### Kept committed

These sit beside generated files but cannot be regenerated exactly:

| File | Bytes | Reason |
| --- | ---: | --- |
| `c2_decoded_containment/wb/fixtures/nodes-rotated.3tz` | 6,924 | Quaternions from `random.gauss`, which calls the platform libm (`log`, `cos`, `sin`). glibc reproduces it, but no libm guarantees the last bit; a correctly rounded rewrite matches this file but changes the large twin. The generator's `COMMITTED` set leaves it alone |
| `c2_decoded_containment/controls/slice3/fixtures/F-W4-polygons.3tz`, `S3-2.W4-polygons.3tz`, `S3-2.W4-polygons.base.3tz` | 3 x 37,304 | Derived from R's forest producer archive W4-polygons, which is not in the repository (`controls/slice3/forest.py` `INPUTS`) |
| `c2_decoded_containment/controls/slice3/fixtures/F-W7.3tz`, `S3-4.W7.3tz`, `S3-5.W7.3tz` | 3,666 + 3,666 + 3,678 | Derived from forest archive W7, likewise external |
| `c1_payload_integrity/.../context_controls/2026-10-10/fixtures/manifest.json` | 53,508 | Hand-amended record (labels), compiled in with `include_str!` |

The rest of `c1_payload_integrity/` stays committed. Its 16 `.tar.gz`/`.json.gz` bundles (571,058
bytes) were compressed by a different deflate implementation from the zlib that CPython ships: their
tar and JSON content regenerates exactly, but the compressed bytes do not, and each amendment pins its
input bundle's compressed SHA-256. The C2 amendment's corrected-205 bundle no longer reproduces at all,
because the C2 oracle has since reclassified four b3dm cases. The stride, stride-parent and
meshopt-header lane fixtures (124,013 bytes) regenerate exactly on Linux, but their generators write in
place through git symlinks (which Windows checkouts do not create) and record their own SHA-256, so
generating them needs those lane scripts reworked first. `production_controls/fixtures/meshopt-none-placeholder-gltf/tile.gltf`
(675 bytes) is compiled in with `include_bytes!`. Primary references, receipts, decisions and runner
copies are records.

The `--large` controls (`implicit_read/controls/large/`, `c2_decoded_containment/controls/large/`,
`wb/large/`) were never committed and are not in the manifest; tests skip them when absent, as before.

## Audit

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
| [API probes](https://github.com/BenDyson-Arch/rusty-tiles/blob/112044668e8a39394a96832833a0ebaa17c4c0db/bench/architecture_audit/api/README.md) / [results](https://github.com/BenDyson-Arch/rusty-tiles/blob/e29cd19d2f236dc688e91a80a45170d4a573e20b/bench/architecture_audit/api/results.json) | Build unmodified portable CLI/core/Python binding, generate invented GLBs, compare decoded archive members and inspect output after exceptions | Geographic offsets, rotation-only requests, omitted texture choice, callback failure/publication |
| [Runtime probes](https://github.com/BenDyson-Arch/rusty-tiles/blob/112044668e8a39394a96832833a0ebaa17c4c0db/bench/architecture_audit/runtime/README.md) / [results](https://github.com/BenDyson-Arch/rusty-tiles/blob/e29cd19d2f236dc688e91a80a45170d4a573e20b/bench/architecture_audit/runtime/evidence/publication-vector.json) | Natural vector conversions plus clearly identified isolated source fault injection | Feature rollback/reporting, source callback I/O failure, directory publication race, orphan-content control |
| [Foundation design model](foundation/README.md) / [results](../MOVED_EVIDENCE.md) | Exhaustive finite state/history exploration with seven negative controls | Proposed gate, seal/drain, primary failure, publication and recovery contracts; no production execution |

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

## Foundation-first follow-up

[Issue #115](https://github.com/BenDyson-Arch/rusty-tiles/issues/115) scopes the
first implementation to one real packaging consumer and completed-file
publication through Rust/CLI/Python. The bounded model passes 14 contract cases
and detects seven designated broken variants (2,354 state/history pairs; 4,258
edges). Its assumptions and limits are explicit: a logical safety result is not
proof of OS primitives, actual synchronization, FFI or crash durability. The
coordinator reran it after integration and reproduced the same counts.
