# F1d2 bounded adaptive complete-surface evidence

This evidence supports the [settled contract](../../../docs/architecture/f1d2-certificate-contract.md)
and [implementation](../../../docs/architecture/f1d2-implementation.md) for one
adaptive root certificate over unchanged full-detail leaves. Frozen production
source is `4553533e64c1494e706888a2c8e3a38a6867ce56`, after merged PR #149.
Final portable/native artifact, installed-wheel and placed-consumer executions
passed. Package production-byte identity and final separate nonauthor review
passed; [storage-index.json](storage-index.json) records the completed slots.
This bounded slice does not close #121/#113 or authorize release.

## Identity and receipt map

| Artifact | SHA256 |
| --- | --- |
| Frozen portable CLI | `aa71fd57614844488d336dbc991df551fd9294c6d31d82027e1b4a43aedbf64b` |
| Frozen native CLI | `c4c4a64ed25ddfd706e2dcaebb830c9323928dc299ebc2664569f563ca0a52e3` |
| Frozen optimized CLI | `d1e86e4f38b458262d5652b425df7a02c88924519089d505eda8da09ed5797d9` |
| Installed Linux abi3 wheel | `028cadd7f78de1beff544eb46f243108658759cab09d12ff1f75a5dc3d9c70c7` |

[independent-summary.json](independent-summary.json) gives compact artifact,
consumer and resource results; the [storage index](storage-index.json) maps each
original raw receipt path to retained exact bytes, sizes, SHA256s and available
receipt-authored driver/source pins. `final-source-artifacts.json` binds the
frozen production files, three binaries, wheel members and current execution
drivers. `execution-input-pins.json` binds the final artifact/consumer inputs.
[coordinator-checks.json](coordinator-checks.json) owns commands, exit statuses,
build/test/lint/package pins and the historical missing-log limitation.
[review.md](review.md) records fresh separate NONAUTHOR bounded acceptance, with
no production/evidence blocker in this slice. Its archived final bytes supersede
the earlier draft. The final reviewer receipts in `review_probes/resume-*`
independently bind source, executed drivers, artifacts, wheel members, package
and coordinator logs. The package/storage probe pins an earlier 49-entry
index snapshot; the final verifier checks all later immutable additions.

| Evidence | Stored original receipt name | Meaning |
| --- | --- | --- |
| Final portable and native | `portable-oracle.json`, `native-oracle.json` | Five baseline artifacts, two tighter artifacts, exact cover, corruption sensitivity and typed refusals per CLI |
| Final artifact binding | `final-artifact-summary.json` | Final execution inputs unchanged; binary/source pins and run outcomes |
| Installed Python | `wheel-api.json`, `wheel-api.log` | 44 API tests, zero failures/errors/skips, empty PATH; separate validation fixture suite in log |
| Local Cesium | `local-browser.json` | Natural public coarse/fine traversal and property queries |
| Placed Cesium | `placed-browser.json` | Independent Decimal80 WGS84 placement and public coarse/fine traversal |
| Near-limit resources | `resource-release.json` | Three optimized serial cases; requested wide budget proved, smaller published scalar unproved |
| Prior full-detail | `full-detail-replay.json`, `f1*-full-detail.json/log` | Six active profile drivers, including F1c2 large |
| Prior resource/profile replay | `f1b3-resource.json`, `f1c1-resource.json` | Four PBR and ten placement cases under schema 7 |
| Official glTF core | `khronos.json` | 51 entries and sensitive zero-position control; no metadata-extension proof |
| Root crate packaging | `package-checks.json`, `package.log` | Exact production bytes and normalized Cargo metadata; docs/oracle snapshot, no packaged-build verification or publication |

## Independently re-certified actual artifacts

Both final CLI receipts use fresh artifact directories and bind the final
driver hashes. Each passed five baseline cases: grid and materialless 32→15,
six-region instances and viewer 192→44, and coincident-key 64→14. Each baseline
case rejected 23 altered artifacts. CLI mesh reports equal published reports.
Eleven baseline typed refusal controls left output parents absent.

| Additional actual artifact | Faces | Requested m | Published certificate m | Tests / accepted patches / depth | Independent exact cover squared m² | Rejected mutations |
| --- | --- | ---: | ---: | --- | --- | ---: |
| Planar useful tightening | 32→15 | 0.5 | 0.4850712500726813 | 9912 / 260 / 3 | 4/17 | 24 |
| Nonzero bump | 32→2 | 0.5 | 0.25000000000000017 | 1408 / 64 / 2 | 1/16 | 25 |

The grid's historical unpartitioned face-pair bound was 2 m, and the bump's
squared bound was 5 m². Those old whole-face values measure the old proof's
looseness; they are not lower bounds after subdivision. The final checker
decodes original/root faces and uses exact Fraction point-to-closed-triangle
distances on its own complete canonical four-child cover. It receives no
producer witnesses or private proof geometry and does not infer geometry truth
from operational counters. The false-zero bump mutation is rejected.

Independent necessary tests/leaves/depth must fit the reported metrics. This
does not independently identify their exact operational values: compatible
inflated counters may pass. Source review and unit controls establish global
charging and cancellation semantics separately. Checker limits are 100,000
tests, depth 24 and cooperative 10 seconds; exhaustion leaves a scalar unproved.
Coincident positions cannot identify authored face/component origin, so original
key association also depends on production controls and source review.

At bump budget 0.125, meshopt does not achieve the requested actual reduction.
Both final drivers label this typed proposal refusal correctly and verify an
existing destination and directory are unchanged under `--force`. The request
budget affects proposal behavior. No actual CLI depth-exhaustion case is
accepted. Private depth-cap controls, actual optimized dynamic-cap refusal and
exploratory depth feasibility are distinct evidence.

## Consumer and installed adapters

Local and placed public probes used Cesium 1.146.0, Playwright 1.63.0 and
Chromium 153, identity modelMatrix and natural maximumScreenSpaceError 16.
Both observed 44 coarse root faces and 192 full-detail faces. Six coarse region
membership/name queries and six fine exact source-triangle queries matched.
The opposite-level label combinations exposed no matching invented properties.
No page errors or failed/external requests occurred.

The placed probe used independently calculated Decimal80 WGS84 ECEF/ENU cameras
at the Brisbane anchor, rather than deriving expected placement from the
emitted matrix. Maximum root-origin component discrepancy was
8.845845404077495e-10m; three independently checked matrix corruptions were
rejected. Its two false `frozen_requested_*_match` fields compare historical
F1d1 constants and are not current-source failures; the supplied F1d2 source and
binary match the current external pins. These finite observations do not prove
world Hausdorff distance, pixel equivalence, textured appearance or unique
selection among coincident supports.

The installed abi3 wheel's 44 API tests include actual adaptive grid/bump 0.5 m
results, typed schema 7 certificates, independent decoding and published-report
equality. The runner used an empty PATH. Native Python CLI 127 tests passed with
two skips. Portable/native core tests and lints and Python-binding checks are
coordinator evidence; the historical portable test had no retained full log,
which must not be reconstructed as if captured. A final retained portable
replay has its own indexed log and coordinator result.

The root crate package passed exact production-byte checks. Cargo's normalized
manifest retains a byte-identical `Cargo.toml.orig`; the package lock omits only
the workspace Python binding and seven Python-only dependency entries, with no
new or changed remaining package. Binding source is checked separately through
the installed wheel. The package includes a pinned contract, implementation,
README and oracle snapshot. Evidence-only review/coordinator additions and
final receipt documentation follow packaging to avoid circular receipt hashes;
the package does not claim those later document bytes or a packaged build check.

## Resource observations and controls

[resource-method.md](resource-method.md) records fresh optimized execution,
not inherited F1d1 timing. The accepted 8192→1023 grid performed 16,760,832 tests,
99.90234375% of the cap, with 9215 patches/depth 0 in 17.439 seconds. wait4 peak
RSS was 26388 KiB; 10 ms samples saw 14088 KiB RSS, one thread, three descriptors
and 475920 scratch bytes. Actual typed base and dynamic work refusals preserved
absent output parents, and all three children were reaped.

An exact common 64 m square proves the requested 100 m budget. The published
7.000000000000003m scalar is explicitly unproved independently at this size.
wait4 can include inherited launcher residency; sampling may miss brief peaks.
These observations establish neither total-RSS limits nor cancellation/latency
guarantees. The source finite profile bounds evaluations and added DFS state,
not all process memory or individual meshopt call responsiveness.

`math_probes`, `pipeline_probes` and `probes` contain labelled preimplementation
exact-cover, lattice, midpoint and scheduler controls; they do not execute the
final producer. `certificate-controls.json` and `oracle-self-test.json` contain
exact/synthetic sensitivity controls. `review_probes` preserves separate
reviewer interval/checker/actual-artifact receipts; final review is a further
source/evidence disposition. Finite witness precision, work/depth exhaustion,
recursive hierarchy, appearance and broader world/resource coverage remain
explicit gates.

## Failed preliminary attempts and storage

The preliminary broad portable/native drivers completed positives then stopped
on an incorrect expectation that bump 0.125 reaches a depth gate. The retained
`preliminary-native-oracle.log` shows that assertion failure; no completed
preliminary broad JSON exists or is accepted. The corrected final runs use
distinct fresh directories. `preliminary-local-browser.log` and
`preliminary-placed-browser.log` show a driver `str()` editing TypeError before
browser start. Corrected local/placed receipts supersede those attempts; the
preliminary logs remain classified `nonacceptance`. These are driver defects,
not demonstrated production defects. Exploratory depth timeouts/proposal
refusals in `depth-fixture-feasibility.json` are not accepted depth-gate evidence.

[storage_receipts.py](storage_receipts.py) stores exact original bytes without
JSON normalization. Files at least 16384 bytes use gzip level 9, mtime 0 and no
filename; smaller receipts/logs stay raw. It refuses to overwrite an existing
receipt identity with different bytes. Every entry retains the original source
path, raw/stored byte lengths and SHA256s, classification, bounded scope and
available original driver/source provenance. Compression is not another run.

```sh
python3 bench/architecture_audit/mesh_approximation_f1d2/storage_receipts.py --verify
```

Verification hashes stored bytes, decompresses each gzip, hashes recovered raw
bytes and verifies deterministic recompression. Original raw receipts remain
in the external old cache and final `/tmp` evidence directory. No binaries,
wheels or generated 3tz archives are committed. Historical F1d1 receipt bytes
and profile/source pins remain unchanged.

To rerun final bounded artifact evidence with an already frozen CLI:

```sh
PYTHONDONTWRITEBYTECODE=1 python3 tests/f1d2_oracle.py --self-test \
  --binary /absolute/frozen/rusty-tiles --artifact-dir /absolute/new/artifacts \
  --json-output /absolute/new/portable-oracle.json
```

Use new artifact directories because publication is CreateNew. Receipts must
bind the executed drivers and exact frozen artifacts; successful compression
or regression alone cannot expand this bounded domain.
