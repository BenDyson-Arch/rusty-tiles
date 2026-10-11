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

Measured cost (Linux, release, 2 CPUs, best of 5 `validate --json` runs; develop f2238cd
compared with this branch). On the 20k-point explicit point cloud (4.7 MB) the time went from
48 ms to 108 ms, about 3 µs per tested vertex for the exact walk plus the re-read. Out-of-domain
archives (implicit) and small mesh/vector archives were unchanged within noise (26–42 ms).
Extrapolated to C1's 16M total payload elements, the exact pass could add roughly 50 s. That
is a projection, not a measurement. The contract permits an equivalent i128 fast path, which
is a later optimisation.

Not established here: platform byte identity, performance or RSS of the precheck caps (the cap
values are proposals), implicit/RTC/region/rotated-box coverage, and any rendered-extent
claim.
