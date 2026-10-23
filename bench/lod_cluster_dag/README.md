| Full model, production octree | This HLOD | #260 baseline |
|---|---:|---:|
| Whole CPU / wall | 788 / 546.1 s | 10,000 / 3,612 s historical |
| Coarse / total CPU-s per Mtri | 72.00 / 78.80 | unavailable / 1,000 |
| Peak RSS | 6.06 GiB | 9.37 GiB historical |
| Cells / exact leaves | 2,806 / 1,959 | identical |
| Root triangles | 8,192 | 7,680 |
| Root mean geometric error | 0.1401 m | 0.3333 m |
| Root PSNR proxy→source / source→proxy | 20.89 / 18.85 dB | 17.00 / 16.59 dB |
| Output unpacked | 1,144.24 MB | 1,541.63 MB |
| First view 1600×1000, range 3 | 5.44 MB | 35.01 MB |
| First view 2560×1600, range 3 | 119.28 MB | 63.37 MB |
| First view 2560×1600, range .75 | 197.52 MB | 230.81 MB |
| Mixed / same-depth gaps | zero across 15,536 / 6,859 pairs | not rerun |

**Verdict: retain the production-octree HLOD geometry design as a 0.4 implementation
candidate; do not adopt this delivery configuration unchanged.** Geometry, colour,
leaf budgets and two of three first views improve. The wide first view remains
1.88× baseline: six selected leaves require 69 original images (78.22 MB), while
proxy images add 21.80 MB. Localized leaf texture delivery is an untested next step.
832 of 847 proxies hit the 1024² cap; the GE/16 target is often not achieved.
Coarse cost is above the original low-tens CPU-s/Mtri aspiration. Historical timing
compares unequal pipelines, and creating the reused baseline partition is excluded.

The final tex3 rehearsal passed first: 156 exact leaves, 74 CPU-s total, 1.67 GiB
peak, root PSNR 27.41 dB, zero gaps across 778 mixed / 466 same-depth pairs.
The new full model was built exactly once, after preflight review corrections.
Source and artifact manifests bind the final tex3 and m2048 runs.

## Implementation and acceptance

`baseline_tree.py` expands actual implicit availability and metadata bounds, then
`octree_partition.py` reads original triangle ownership from baseline leaf metadata.
Every occupied cell is retained. Exact source leaf attributes/provenance and a cap
of 8,192 triangles are independently checked against the baseline partition.

`octree_hlod.py` uses local position-welded geometry, explicit outer-interface
locks and no UV/material attributes. Source junction ownership is unchanged.
Three full-model parents required two protected faces each; none used the merged
children fallback. All 847 parents fit 8,192 triangles. The simplifier is budget-led
with an effectively unbounded error threshold; all saved simplifier errors are
below their corresponding baseline GE. Published GE independently samples source
surfaces in both directions and takes the child maximum; it is not certified.

Every nonleaf gets a fresh chart atlas at desired spacing max(GE/16, 1e-5 m), with
128–1024 power-of-two square pages. The actual spacing and cap status are recorded.
Leaf image lists are pruned to used dependencies, retaining original pixels/UVs.
The result is glTF/meshopt plus JPEG, not production KTX2/mip delivery.

`evaluate_octree.py` checks complete decoded interface segments across all
source-shared-edge adjacency, including all incident nonmanifold owner pairs.
It checks exact leaf membership/attributes/provenance, valid all-corner feature IDs
and representative coarse primitive identity. Coarse identities are not full
ancestry. Independent 200k-per-direction quality samples cover cuts 0, 2, 4 and 6,
carrying terminal leaves forward. Root max 2.8369 m exceeds published GE 2.8106 m,
illustrating the sampling limit. Unrelated nearby boundaries and actual Cesium
refinement/picking remain untested.

17 full-model GLBs and 13 tex3 GLBs have zero validator errors/warnings; native and
WASM decoding agree. Metadata acceptance is from the independent oracle, since
the glTF validator does not validate those extensions. All output tiles pass exact
source-attribute, bounds and monotonic-GE checks. Independent review is recorded in
the external report; acceptance is bounded research, not a release authorization.

## Output and reproduction

Ready-to-serve tileset (same baseline transform / requested anchor, local metres Y-up):
`/home/bend/scratch/rusty-tiles-scratch/lod-bakeoff/cluster-dag/hlod-octree-m2048/tileset.json`.
No viewer or server was started.

`run_octree.sh {tex3,m2048}` records the build invocation and enforces a no-clobber
sentinel. The approved full run is already spent; do not remove that sentinel and
rerun without further authorization. It runs detached through the caller's
`setsid -f`, systemd 20 GiB/no-swap, idle scheduler/nice 19, measure.py slot 2 and
a 60 GiB disk watchdog. `assess_octree.sh` and `extra_octree.sh` inspect existing
outputs; validation scratch must be fresh for another assessment.

Full report, per-level tables, identity manifests and review records:
`~/.cache/rusty-tiles-evidence/lod-bakeoff-20261024/cluster-dag/README.md`.
`octree-evidence.sha256` binds the external execution/artifact manifests and report.
Historical reports there are `README-binary-m2048.md`, `README-crops.md` and
`README-locked-uv.md`; their source identities refer to the commits recorded in
those reports. This is standalone research code, not integrated Rust production tiling.
