# F0 implementation evidence (#115)

Initial implementation source: `36db8b2941db8d93a66c81c49efdd917711e6e41`.
Destination-binding fix and full verification: `c777180f178df5cc2f524f7b20f4251ba9d522e0`.
CLI fixture follow-up: `4b51b32bc63e789296f9eaaaaaa2fc0b2dee8e1d` (3 portable / 4 native CLI contract tests pass; production unchanged).
Merged into `develop` in PR #116 at `519e8c105dfb9afcdcf6447b82adc91ff0dba630`;
the merged tree exactly matches accepted head `dd0e0af`.
The [implementation contract](f0-implementation.md) defines the supported scope
and deliberate migrations. This evidence does not close #113 or release 0.4.0.

## Local verification

Linux x86-64, Rust 1.98.0, Python 3.14.7, GDAL 3.13.3. Builds use the dev profile,
no debug information, two build jobs and disabled incremental compilation.
[Machine-readable results](../../bench/architecture_audit/foundation/implementation-results.json)
record source/toolchain identity, test totals, log hashes and wheel identity.

| Check | Result |
| --- | --- |
| `cargo test --locked --no-fail-fast` | 309 passed; 8 existing ignored tests |
| `cargo test --locked --features native-geospatial,native-jpeg --no-fail-fast` | 334 passed; 9 existing ignored tests |
| `cargo clippy --locked --workspace --all-targets -- -D warnings` | Passed |
| Native-feature all-target clippy with warnings denied | Passed |
| `cargo fmt --all --check`, `git diff --check` | Passed |
| Source packaging (`cargo package --locked --no-verify`) | Passed; generated Python bytecode excluded |
| Installed abi3 wheel, isolated Python 3.14 environment, empty `PATH` | 22 passed; no skips |
| Foundation lifecycle model | 14 contract cases pass; 8 designated negative controls detected |
| Independent monitor traces | 3 tests pass; pre-permission fatal causes, late producer work, post-permission publisher failure |

The locally installed wheel is a Linux dev artifact, not a manylinux/release
qualification. Its production source is `c777180`; the final Python output-path
assertion at `dd0e0af` compares filesystem identity, including Windows verbatim
path spellings. Ignored tests are not counted as passing. The hosted results below are separate from the local runs; no platform result
is inferred from the local environment.

The final review found and fixed publication through an unresolved relative
destination: a callback changing CWD could redirect Replace into a selected
source. The operation now retains its checked absolute destination for staging,
publication and result identity. An isolated subprocess regression proves source
preservation and intended installation; the rebuilt wheel verifies the resolved
output contract. See the [final review](../../bench/architecture_audit/post_f0/f0-acceptance-review.md).

## Hosted acceptance

At head `dd0e0afc9b927f43944cc185aa8b806c6032fbde`, all active jobs in
[PR CI](https://github.com/BenDyson-Arch/rusty-tiles/actions/runs/37882662620)
and [wheel/Blender acceptance](https://github.com/BenDyson-Arch/rusty-tiles/actions/runs/37882662617)
pass. [Recorded CI metadata](../../bench/architecture_audit/foundation/ci-acceptance.json)
identifies the exact jobs, source, merge and equal tree hashes.

- Five portable CLI targets: Linux x86-64/ARM64, macOS x86-64/ARM64, Windows x86-64.
- Rust/Python checks, native GDAL 3.12/3.13, and the Python-free installed container
  suite plus network-disabled runtime capability check.
- Five candidate wheel platforms, each tested on CPython 3.10 and 3.14, plus four
  official Blender 4.5.14 LTS bundles (Linux/Windows x86-64 and both macOS targets):
  **14 sessions, 308 tests, zero failures/errors/skipped tests**. The
  [installed-platform evidence](../../bench/architecture_audit/foundation/installed-platform-results.json)
  retains report/wheel/distribution hashes and environment identity.

The optional distribution-Blender job and workflow-dispatch-only release route
were skipped, not passed. No PyPI installation/publication, current strict browser
run or completed #113 architecture gate is claimed. Fresh merged-head CI starts
separately; the accepted PR and merged source trees are identical.

## Proof boundaries

The 20 runtime tests exercise actual temporary files and installation primitives,
including simultaneous create-new publishers, a competing destination,
actual replacement failure against a competing directory, replacement, pre/post-permission cancellation,
producer/observer first cause, callback reentry, in-flight admission, run reuse,
duplicate publication, sync/install faults, cleanup failure with retained paths,
postcommit reoccupied temporary names and Unix permissions. Faults and schedules
are deterministic test injection, not observed disk-full or natural race events.
Artifact ownership ties each candidate to its creating attempt by lifetime.

Twelve Linux package integration tests and four CLI tests cover pure validation, read-only
resolution, exact byte/receipt inventory, safe names, root/member/directory
symlinks, nonregular sources, non-UTF8 member rejection, long/oversized members,
source/output aliases, changed/deleted sources, observer failure, competing
outputs, replacement and CLI path display. Three codec tests cover persistent
payload/finalization/flush faults, the 65,535-entry ZIP64 sentinel, and an actual
sparse file with an offset at `u32::MAX`. A package producer-fault test drives
those codec failures through abort and cleanup while preserving previous output.

Raw-byte non-UTF8 filename fixtures run on Linux because APFS rejects their
creation. Unix symlink/socket coverage remains enabled on macOS. Source mutation
fixtures change size, avoiding timestamp-resolution assumptions; deleted-source
errors are checked against canonical resolved paths.

Container oracles include a separate ZIP reader and direct local-header, CRC,
size, name, index and offset parsing. Installed Python tests use `zipfile`,
`struct` and `hashlib`, including corrupt hash/order/offset negative controls.
They do not depend on the production 3TZ validator accepting its writer's output.
Python also checks original callback exception identity, early/final callback
failure with replacement, and nested/concurrent/repeated operations.

The model correction addresses the independent #113 review. Its monitor records
permission independently of producer state and rejects all accepted fatal causes
before that point. The producer-revival negative control is detected at permission
grant. Additional external traces reject producer work after seal. A separate DAG
review confirms all 22 scenarios are acyclic and grant permission at most once:
2,455 state/history pairs and 4,382 edges. This remains bounded design evidence.

## Resource behavior

The [replay script](../../bench/architecture_audit/foundation/package_resources.py)
and [measurements](../../bench/architecture_audit/foundation/package-resources.json)
record a fresh process per case, exact CLI binary hash and source commit.
With two selected members, 8/128/512 MiB payloads used 19.6–19.9 MiB maximum RSS.
With 129/4,097 small members, observed RSS was 20.3/21.5 MiB. These are Linux
measurements with sparse zero-filled sources, not worst-case bounds or timing
regression thresholds.

The encoder copies through one 64 KiB buffer with one open source at a time.
Resolved paths, sorting, ZIP directory metadata and the index grow with member
count. The completed archive occupies temporary space beside its destination;
replacement needs space for both the existing file and candidate. Failed cleanup
can retain work and is reported explicitly. Sources must remain stable; size/mtime
checks do not constitute a snapshot.

## Remaining limits

The F0 runtime is used only by packaging. Other converters retain their old job,
callback/reporting and writer behavior, including the old ZIP writer's possible
stderr output during failed destruction. Their implementation remains unproven
under #113. No directory recovery, scene resource closure, semantic tileset
certification, power-loss durability, Blender/browser acceptance or complete
release qualification is claimed here.
