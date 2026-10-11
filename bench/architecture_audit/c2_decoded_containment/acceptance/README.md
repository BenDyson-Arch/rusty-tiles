# C2 acceptance (coordinator-owned)

Production is compared only against the independent oracle's committed expectations. No
verdict here is derived from production code.

| Evidence | What it shows | Where it runs |
| --- | --- | --- |
| `oracle_parity.rs`, compiled as `--lib c2_oracle_parity` | Production `inspect_selected` equals the oracle on every witness and control (outcome, reasons, Outside depth/level/role/node, C1 refusal kind) | CI and local. The 24 MB `controls/large/` archives are gitignored, so CI skips only those three ids |
| `parity-3676e74.json` | Local release run with the large controls: 259 cases, 259 agree, 0 skipped, 0 C1-precondition refusals | Local, Linux |
| `corpus_parity.py` and `corpus-parity-e22db61-release.json` | `validate --json` on the 27 producer artifacts (develop f2238cd release producers, manifest sha256 `28e4bee6…`) equals the oracle corpus manifest: 17 Contained, 10 NotInspected(implicitTiling), 0 Outside | Local, Linux |
| `production-mutants/` | 19 hashed production mutants (12 mirroring the oracle's). Initially 17 flipped at least one fixture. The 2 survivors exposed corpus gaps, which the oracle closed in `122a63e`; `rerun-after-gap-fixtures.json` shows both now flip | Local, Linux |

Replay:

```sh
python3 -I bench/architecture_audit/c2_decoded_containment/witnesses/generate.py --large
C2_PARITY_OUT=parity.json cargo test --locked --release --lib c2_oracle_parity
```

The corpus replay needs the producer artifacts: regenerate them with the corpus generator
recorded in the corpus manifest, then run `corpus_parity.py BINARY RUN_DIR
corpus/manifest.json OUT`.

Measured cost (Linux, release, 2 CPUs; develop f2238cd compared with this branch).

| Unit class | Measured on | Cost |
| --- | --- | --- |
| Exact vertex unit (`T·Σ⌈s/64⌉²`) | 20k-point explicit point cloud. The oracle computes 5.99M units; validate goes from 48 ms to 108 ms | about 10 ns |
| Index unit (`Idx`) | C1 `legal_shared_indices`: 1000 primitives share a 1M-index accessor, about 1e9 units. Release 2.85 s (debug 44 s, which exceeded the C1 finite-work probe's 30 s) | about 2.8 ns |

The work cap, not C1's element ceiling, bounds C2 time: instancing, shared index accessors and
repeated references all multiply work. The original default of 2^32 units therefore allowed
about 43 s in release (minutes in debug). After the nonauthor review, the coordinator set the
default to 2^28. That bounds C2 work at the defaults to about 2.7 s at the most expensive
measured unit cost, and `legal_shared_indices` becomes `workLimit`. Re-reads are separately
capped by `containmentBytesRead`. The nonauthor review noted that per-reference C1
re-validation and drawn-vertex marking across many primitives have no dedicated unit; that
cost is bounded only indirectly, and timing archives with many primitives is open work.

Not established here: platform byte identity, performance or RSS of the precheck caps (the cap
values are proposals), implicit/RTC/region/rotated-box coverage, and any rendered-extent
claim.
