# LOD refinement audit (2026-10-24)

Supports [the design recommendation](../../../docs/architecture/lod-refinement-budget.md).
No tiler, build, browser, server or production-code change was used. All work is
in `/home/bend/.cache/rt-wt/lod-refinement-design`, from source commit
`f97cd930fa36df0ca4213131f2e9d1026a5bcda0`. Inputs were read-only.

## Reproduce at idle priority

Run from this worktree root, one dataset at a time. Python uses only its standard
library; Node uses the already-installed meshoptimizer decoder. Set `decoder`
to another installation's `index.js` if necessary; this is an audit dependency,
not a product dependency. No network install is needed in the supplied environment.

```bash
audit=bench/architecture_audit/lod_refinement
dataset=m2048 # repeat with tex3
view=/home/bend/scratch/rusty-tiles-scratch/proxy-density/$dataset/view
decoder=/home/bend/.cache/rusty-tiles-117-browser-cache/node_modules/meshoptimizer/index.js
mkdir -p "$audit/local"
chrt -i 0 nice -n 19 python3 "$audit/analyze.py" index "$view" > "$audit/local/$dataset-index.json"
chrt -i 0 nice -n 19 node "$audit/measure.mjs" "$view" "$audit/local/$dataset-index.json" "$decoder" > "$audit/local/$dataset-tiles.jsonl"
chrt -i 0 nice -n 19 python3 "$audit/analyze.py" report "$view" "$audit/local/$dataset-tiles.jsonl" > "$audit/local/$dataset-report.json"
```

`index` expands explicit/external/implicit tiles; logical depths ignore document
and implicit-root wrappers. Proxy means a content tile with logical children,
not an arbitrary error threshold. `measure.mjs` decodes meshopt positions and
indices, applies each scene node's scale, sums cross-product triangle areas and
reads image buffer-view lengths. Its narrow H8/H17 scene domain rejects nested,
matrix/rotated/instanced scenes, sparse accessors and external buffers. The
measured outputs have only scale/translation nodes; translation does not affect
area. Sizes/hashes come from actual GLBs, not `conversion.json` counters.

The m2048 report's old `glb_bytes` counter is 1,152,800,660, but the current 2,806
GLBs total **1,540,854,160** bytes. That historical conversion report does not
describe current rebaked byte sizes. The traversal baseline instead matches the
owner's approximately 35 MB / 230 MB loads. tex3's 219 GLBs total 72,933,020 bytes
and its leaves sum to 788,425 triangles.

`first_view.py` is a frozen copy of the supplied `/tmp/simview/sim.py`, SHA-256
`8fcbc89ee5583b80abc22606576f934bf4182b5a1921854022b1b6d59ee86f07`.
It differs from the forest branch simulator only in environment-controlled
width, height and range. The forest design and original simulator were read
with `git show` at `e2c1f7d57d4eed3012520f087a18a97b097eb0d2`; they are absent
from this worktree's base. Original simulator SHA-256:
`802429075c8d625db1d058622ec9ec6610a89cafc43e705980e90fa12b3896a5`.
The area decoder was adapted from the supplied `texel_density.mjs`, SHA-256
`9b7d4ac85a98d773a14cd9833d3b7b750ed8b8cc7d04aa5f2cba0c62708810d9`.

`analyze.py` resets the layout for each scenario, changes only internal errors,
closes errors bottom-up against child errors, and reruns resource traversal.
The independent logical frontier walk checks every selected tile was requested.
For reduced-face scenarios, triangle count is `min(old, target)` and estimated
bytes are `old_images + old_JSON/chunks + old_geometry_BIN * new/old`.
Nonimage BIN includes provenance tables and padding, so the latter's scaling is
approximate. The 2,048 scenario is a final-face allowance, not a prediction of
the simplifier's exact 15/16 pre-fill result. Geometry floors and new image chart
layouts are unmodelled. The error-growth scenario uses `sqrt(old/new)` and is
neither an empirical law nor a rigorous worst case.

For the wider-level surrogate, odd internal depths are bypassed, and logical
traversal becomes directly explicit. The corresponding uncollapsed scenario's
metadata charge is retained. This is a topology cost model, not shippable
implicit addressing. Actual rebinning cannot be inferred without retiling.

For unchanged-geometry scenarios, resource bytes are exact **within the supplied
simulator**, including top `tileset.json` (which its original report excludes).
They omit HTTP overhead, archive packaging, index/conversion records that the
viewer does not request, and the proposed new certificate properties. Camera
projection uses 60° horizontal FOV; root rigid placement is factored out.
This is not new live-Cesium verification. No source deviation distributions,
occlusion, actual raster density, popping or cracks were measured.

## Artifact identity

Raw local inventories/reports are ignored under `local/` to respect both the
tree-size policy and the instruction to write only in this worktree. They are
reproducible rather than committed bulk evidence. Hash manifests below identify
their exact inputs; do not infer input identity from the scratch directory name.

The content-set digest is SHA-256 of sorted UTF-8 lines
`relative_glb_name + " " + sha256(payload) + "\n"`. The hierarchy digest uses the
same form for `tileset.json`, external `tileset-*.json` and all `.subtree` files.

| Artifact | SHA-256 |
| --- | --- |
| m2048 content set (2,806) | `6b5243430e0b08cb1d3bf036aa1a320866822a3641e50e4d26c68c5f4bb799d6` |
| m2048 hierarchy set (207) | `c6eaaa13f3da0ec1839ee8075455268b8489fea72dadece0bd2f5db75ba6d4fc` |
| m2048 tileset.json | `23c4652d5e73a7473d12e1af2602a81660ed4be6c7ace80b3952020f4b906efa` |
| m2048-tiles.jsonl | `fbad6517b14d20f227c7944a48cd1f1e15c33f7d24c292c408ead2e473b3e157` |
| m2048-report.json | `e031bc7dc8a37497a667d6e042f44498b61994f6cd796742bd8f0eded6e18c2b` |
| tex3 content set (219) | `c37be4b9696ea0eb3df19924225c4a82ef0dc52b2e728216184319ec84998f66` |
| tex3 hierarchy set (16) | `d66a87b4bef1966bc9e705c6a365c529c318763670c52dc26ae0f1becb0e09c1` |
| tex3 tileset.json | `1ebc896c4777affc5f9bb7e0b85e08809796e20204770783ce435979f8e0c3f5` |
| tex3-tiles.jsonl | `a02be1ddc0daa6c5ac04047d275b12dafa07724fc14751a4e2113401e80a6a00` |
| tex3-report.json | `a36d21ee7f554f292466e68c3f05958acfb6d1185da8f879f791f7dd52aa0b58` |

## Additional measured leaf data

Medians; leaves have no children, so a hypothetical own-error switch does not
trigger any further detail. All full distributions (p10/p50/p90) are in the
reproducible report. Proxy statistics and all m2048 scenarios are in the design.

| Depth | Leaves | Triangles | Error, m | Longest extent, m | Area, m² |
| --- | ---: | ---: | ---: | ---: | ---: |
| 2 | 3 | 3,459 | 0.000805 | 22.6 | 191 |
| 3 | 33 | 4,862 | 0.000812 | 13.8 | 214 |
| 4 | 63 | 3,927 | 0.000639 | 7.19 | 56.3 |
| 5 | 160 | 4,140 | 0.000332 | 3.57 | 13.6 |
| 6 | 601 | 5,655 | 0.000208 | 1.95 | 4.64 |
| 7 | 751 | 5,870 | 0.000188 | 1.65 | 1.62 |
| 8 | 316 | 5,876 | 0.0000993 | 0.822 | 0.359 |
| 9 | 32 | 5,873 | 0.0000432 | 0.391 | 0.0953 |

## tex3 cross-check

Same views, MB / million requested triangles. This is a second dataset, not
independent evidence about new geometry quality.

| Policy | 1600×1000, 3r | 2560×1600, 3r | 2560×1600, 0.75r |
| --- | ---: | ---: | ---: |
| Baseline | 4.47 / 0.078 | 4.47 / 0.078 | 12.30 / 0.236 |
| 2,048 cap, fixed error | 2.54 / 0.025 | 2.54 / 0.025 | 7.08 / 0.088 |
| 2,048 cap, square-root growth | 2.54 / 0.025 | 5.41 / 0.064 | 9.92 / 0.128 |
| Four px²/tri, fixed error | 4.37 / 0.076 | 4.37 / 0.076 | 12.20 / 0.233 |
| E/2 | 0.32 / 0.005 | 1.46 / 0.022 | 9.90 / 0.191 |
| E/4 | 0.32 / 0.005 | 0.32 / 0.005 | 4.47 / 0.078 |
| E/8 | 0.32 / 0.005 | 0.32 / 0.005 | 4.47 / 0.078 |
| Bypass odd proxy depths | 3.67 / 0.066 | 3.67 / 0.066 | 29.11 / 0.488 |
| E/4 + 2,048 cap | 0.23 / 0.002 | 0.23 / 0.002 | 2.54 / 0.025 |
| E/4 + 2,048, error growth | 0.23 / 0.002 | 0.46 / 0.004 | 5.41 / 0.064 |
| E/4 + four px²/tri | 0.32 / 0.005 | 0.32 / 0.005 | 4.47 / 0.078 |
| E/4 + bypass odd depths | 0.32 / 0.005 | 0.32 / 0.005 | 3.67 / 0.066 |

## Executed checks and limits

- All six baseline request counts, bytes and triangle sums agree with the frozen
  original simulator plus top-level JSON accounting. Expected GLB counts were
  70/144/510 (m2048) and 14/14/41 (tex3).
- An 8,192-face identity-cap control reproduces every baseline field; fixed-error
  2,048-cap runs preserve the requested GLB count. Each inventory's byte classes
  sum exactly to its file sizes; leaf triangle sums match the source totals.
- A hand-constructed indexed, normalized SHORT right triangle with scale
  `(2,3,4)` and translation `(10,20,30)` decodes to one triangle, three vertices
  and the independently known area **3 m²**. Fixture/output remain in
  `local/analytic/`. The real fixtures exercise meshopt; this analytic control
  isolates area, accessor normalization, indices and transform handling.
- Recomputing both reports after sorting summation order produced byte-identical
  reports with the SHA-256 values above. Python compilation, Node syntax,
  changelog validation, staged whitespace and repository tree-budget checks pass.

These are audit checks, not production or visual acceptance. The frontier walk
shares box/SSE math with the simulator, and its agreement is a consistency check,
not independent proof of Cesium rendering. No CI polling or source-aware
certificate revalidation was performed.
