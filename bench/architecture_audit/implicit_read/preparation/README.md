# I1 implicit read: retained preparation

Lossless copies of the I1 preparation, made on 2026-10-11 for the combined I1 + P branch
`feat/implicit-forest`. The published contract is
[docs/architecture/i1-implicit-read-contract.md](../../../../docs/architecture/i1-implicit-read-contract.md).

| Directory | Source | What it is |
| --- | --- | --- |
| [contract-20261011/](contract-20261011/) | `/tmp/rusty-tiles-i1-contract-20261011` | Contract v1 to v3.1 (`contract-draft.md` `9674e60b…`), change log with the v3.2 rulings, open decisions, ownership plan, producer inventory, `SHA256SUMS.v3.1`, primary-text fetches and probes (`probes/receipt.json`, `probes/results/`) |
| [precode-review-20261011/](precode-review-20261011/) | `/tmp/i1-precode-review-20261011` | Precode review `review.md` (`92eee8f0…`, GO-WITH-CHANGES), its probes (`w2_caps.py`, `alias_budget.*`), the reviewed source extract, and the N delta check `delta/delta-check.md` (`abd37595…`) with its probes |

Each directory has a `RETAINED.json` listing every copied file with its byte length and sha256,
and every file that was not copied. Everything not listed as dropped is byte-identical to the
source.

## Dropped files and how to regenerate them

Files over 5 MiB were not copied. Their sha256 is recorded in `RETAINED.json`.

- `contract-20261011/samples/*.3tz` (5 archives, 106 MB): the W samples `pc-deep`,
  `pc-deep-explicit`, `pc-perf-2000` (W2), `vec-perf` and `vec-perf-explicit`. Their sha256 are
  also pinned in `probes/receipt.json`, which lists the commands. Rebuild them with the
  develop binary `f2238cd` (`/home/bend/.cache/c2-corpus/rusty-tiles-f2238cd`, sha256
  `369c3c81…`) from the perf-lane inputs, for example
  `rusty-tiles --json point-cloud -i perf-lane/points.las --sourceCrs EPSG:32756 --heightOffset 0 --maxPoints 2000 -o pc-perf-2000.3tz`.
  The bytes depend on that binary; later builds write the forest layout instead.
- `precode-review-20261011/delta/uri_probe/{long,pad}.3tz` and
  `dir_{long,pad}/tileset.json` (4 files, 32 MB): the DC-1 timing probe. Each is an explicit
  tileset whose `content.uri` (long) or unused `extras` string (pad) is `"./"` repeated
  4,000,000 times followed by `a.glb`. The short variant, the logs and the timing results are
  retained beside them.
