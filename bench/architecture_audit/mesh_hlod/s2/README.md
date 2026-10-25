# S2 decoded source-error evidence — on develop (S1 #275)

Measured production commit: `b12ffa149d6356477485d01aec3c4964fea09885`, the merge of
develop `072ea8ac` (S1 with exact seams, upward omission, hard cap, `ScratchLedger`)
into S2. Contract: [`hlod-s2-error.md`](../../../../docs/architecture/hlod-s2-error.md).
Release binary SHA-256 `7eeadcca3cb03bff23b1d709079c90683f6e40f251ca108dbbb2e61b489c5fe6`.

Bulk evidence: `~/.cache/rusty-tiles-evidence/hlod-s2-rebase-20261025/`; its
`SHA256SUMS` (SHA-256 `96f42e1e80a6f08096c4be30ea18d50932bc594541c2df47ae748a0210ec217e`)
pins every log, report, census and both archives. The superseded campaign
(`hlod-s2-s1-final-20261024`, against S1 `85bdd4f6`) and its independent cut-audit
helpers (`audit.py`, `decode_geometry.mjs`, `sparse_oracle_control.py`, `unpack.py`;
moved out of the tree, pinned in `retired-bench/SHA256SUMS`) are historical.

## Crop bakes

Release, balanced, `--jobs 2 --lod hlod --memory-budget 8GiB`, measured slot 2
(not a quiet timing lane), user scope `MemoryMax=10G`, `MemorySwapMax=0`, idle
priority; zero cgroup max/OOM events. Both inputs: 788,425 source triangles, 156
leaves, 61 encoded parents, 2 pass-through cells, 0 omitted cells.

| input | S2 CPU-s | S2 CPU-s/Mtri | coarse stage CPU-s/Mtri | max RSS GiB | scope peak bytes |
| --- | ---: | ---: | ---: | ---: | ---: |
| tex3 | 7.756 | 9.837 | 12.350 | 1.25 | 1723125760 |
| crop3 | 7.808 | 9.903 | 12.516 | 0.22 | 470577152 |

S2 CPU is index construction plus summed worker CPU (tex3: index 0.34,
representatives 1.08, proxy-to-source 4.11, source-to-proxy 2.02). The proposed 5
CPU-s/Mtri S2 allocation is still missed (a follow-up before S8). The coarse stage
excludes S3/S4.

**Scratch (walk ledger):** the source index is the only S2 scratch component.

| input | `s2-source-index` admitted | index actual | incidence walk base | stage peak | stage bound |
| --- | ---: | ---: | ---: | ---: | ---: |
| tex3 | 127196576 | 97988368 | 216794271 | 314782639 | 343990847 |
| crop3 | 127196576 | 97988368 | 166799291 | 264787659 | 293995867 |

Admission is `160n + 1 MiB`: 88-byte compact triangles plus at most one 72-byte
BVH node per triangle in a block and one upper node per block (about 124 B per
triangle in full blocks).

**Nearest search:** budget 4,096 steps; 5,077,341 searches per input, 0 exhausted,
122 seeded, at most 501 steps. G is unchanged from exact search.

**G by encoded depth** (both inputs, metres) is bit-identical to the previous S2
receipt:

| encoded depth | parents | max G | max sampled M | area samples/direction |
| --- | ---: | ---: | ---: | ---: |
| 0 | 1 | 1.42904584107 | 1.36099603912 | 200000 |
| 3 | 1 | 1.42904584107 | 1.36099603912 | 200000 |
| 4 | 8 | 0.268756481952 | 0.25595855424 | 200003 |
| 5 | 24 | 0.0706435486373 | 0.0672795701308 | 200013 |
| 6 | 23 | 0.0119894619624 | 0.0114185352023 | 200014 |
| 7 | 4 | 0.00234853407257 | 0.00223669911673 | 200002 |

**Census** (S1's `evidence_census` lane, 8 GiB budget): identical to S1 round 8 on
both inputs. Results: 75,737 checked segments, 0 missing segments or vertices, 0
perpendicular displaced, 0 reversed coarse faces (winding), 0 duplicate chain tiles,
max 8,192 coarse triangles. crop3 leaf members are byte-identical to S1 round 8
(156/156). tex3 leaves differ because S1 round 8 (`98a1bec5`) predates develop's
ETC1S colour default (#277); S2 changes no leaf code.

## Bounded nearest probe

`nearest_searches_are_bounded_on_oblique_adversaries` adapts round-3 review probe
(#278): R parallel oblique triangles whose boxes contain all 32 queries.

| path | R | exact `closest` calls | budgeted calls | exhausted |
| --- | ---: | ---: | ---: | ---: |
| resident | 32,768 | 1,048,607 | 87,204 | 32/32 |
| resident | 131,072 | 4,194,335 | 87,172 | 32/32 |
| paged index | 32,768 | 1,048,607 | 84,832 | 32/32 |

Every budgeted distance is finite and at least the exact one.

## Verification

- `cargo clippy --all-targets -- -D warnings` (no `too_many_arguments` allow in S2),
  `cargo fmt --check`.
- `cargo test --lib mesh_large`: 366 passed, 7 evidence-only ignores.
- `h26_mesh_core`, `h26_mesh_frontends`, `golden`, `output_digests`: 24 passed,
  1 recipe-exporter ignore.
- New controls: omission (cap 12: nothing measured; cap 30: each published tile
  measured once, monotone G, floored content-less cells, valid archive), admitted
  index scratch refusal, budgeted vs exhaustive nearest, delivery refusal of an
  inflated depth schedule.
