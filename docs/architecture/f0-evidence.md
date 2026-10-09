# F0 implementation evidence (#115)

Implementation source: `36db8b2941db8d93a66c81c49efdd917711e6e41`.
The [implementation contract](f0-implementation.md) defines the supported scope
and deliberate migrations. This evidence does not close #113 or release 0.4.0.

## Local verification

Linux x86-64, Rust 1.98.0, Python 3.14.7, GDAL 3.13.3. Builds use the dev profile,
no debug information, two build jobs and disabled incremental compilation.
[Machine-readable results](../../bench/architecture_audit/foundation/implementation-results.json)
record source/toolchain identity, test totals, log hashes and wheel identity.

| Check | Result |
| --- | --- |
| `cargo test --locked --no-fail-fast` | 305 passed; 8 existing ignored tests |
| `cargo test --locked --features native-geospatial,native-jpeg --no-fail-fast` | 330 passed; 9 existing ignored tests |
| `cargo clippy --locked --workspace --all-targets -- -D warnings` | Passed |
| Native-feature all-target clippy with warnings denied | Passed |
| `cargo fmt --all --check`, `git diff --check` | Passed |
| Source packaging (`cargo package --locked --no-verify`) | Passed; generated Python bytecode excluded |
| Installed abi3 wheel, isolated Python 3.14 environment, empty `PATH` | 22 passed; no skips |
| Foundation lifecycle model | 14 contract cases pass; 8 designated negative controls detected |
| Independent monitor traces | 3 tests pass; pre-permission fatal causes, late producer work, post-permission publisher failure |

The installed wheel is a local Linux dev artifact, not a manylinux/release
qualification. Ignored tests are not counted as passing. Windows/macOS and other
Python versions require CI evidence; no local claim is made for them.

## Proof boundaries

The 18 runtime tests exercise actual temporary files and installation primitives,
including a competing destination, replacement, pre/post-permission cancellation,
producer/observer first cause, callback reentry, in-flight admission, run reuse,
duplicate publication, sync/install faults, cleanup failure with retained paths,
postcommit reoccupied temporary names and Unix permissions. Faults and schedules
are deterministic test injection, not observed disk-full or natural race events.
Artifact ownership ties each candidate to its creating attempt by lifetime.

Ten package integration tests and four CLI tests cover pure validation, read-only
resolution, exact byte/receipt inventory, safe names, root/member/directory
symlinks, nonregular sources, non-UTF8 member rejection, long/oversized members,
source/output aliases, changed/deleted sources, observer failure, competing
outputs, replacement and CLI path display. Three codec tests cover persistent
payload/finalization/flush faults, the 65,535-entry ZIP64 sentinel, and an actual
sparse file with an offset at `u32::MAX`. A package producer-fault test drives
those codec failures through abort and cleanup while preserving previous output.

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
