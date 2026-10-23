# Classic HLOD cluster-tree experiment

| tex3 result | Classic HLOD | Current #260 | v0.3.0 |
|---|---:|---:|---:|
| Root triangles | 15,970 | 4,696 | 19,968 |
| Root proxy→source PSNR | 26.61 dB | 19.53 dB | 29.06 dB |
| Coarse CPU-s/Mtri | 22.89 | 173.62 historical | not isolated |
| First view far / near | 1.192 / 6.460 MB | 4.472 / 12.297 MB | 13.924 / 25.982 MB |
| Decoded mixed-depth max / p99 | 0 / 0 m | 0.377 / 0.025 m diagnostic | 1.07e−14 / 0 m |

**Verdict: adopt this architecture as the 0.4 direction, with production gates open.**
Crop3 also passes, at 19.18 coarse CPU-s/Mtri and 29.13 dB root colour.
This is a standalone research prototype, not production tiler integration.

Full tables, caveats and evidence:
`~/.cache/rusty-tiles-evidence/lod-bakeoff-20261024/cluster-dag/README.md`.
Prior UV-preserving results are archived there as `README-locked-uv.md` and
remain reproducible with `build.py`. They stalled at 183,551 root triangles.
The new implementation is `hlod.py` plus `charts.cpp`.

## Contract and decisions

Retain exact source triangles, all leaf attributes/winding and per-triangle
picking metadata. Retain a nested binary ownership tree rather than unrestricted
DAG regrouping; each parent's source domain is the union of its children.
Replace per-primitive attribute simplification with one position-welded parent
mesh. Lock only original vertices incident to source faces outside the parent,
plus true open geometry borders. Child-interior borders and UV/material/colour
seams can collapse. Publish original float32 positions through lossless packing.

Before implementation, the follow-up contract required fresh charts and texture,
source-relative measured error, explicit failures and independent decoded
acceptance. Investigation found that locked endpoints alone can lose a tiny
nonmanifold sheet. Rework leaf ownership so incident triangles on each
nonmanifold edge stay in one leaf; 16 triangles change ownership on these inputs,
with all data exact. Assert every outer interface segment survives simplification.
Do not add interior locks, skirts or boundary collars.

Fresh planar charts use connected normal-cone growth, interior-overlap rejection
and shelf packing. Parent texels sample the original source region via a BVH.
Source images decode once. Parent source-relative error is sampled in both
directions and made monotonic over children. It is not a certified bound.
Exact leaves retain triangle provenance and all-corner feature IDs. Proxy feature
IDs identify the nearest source primitive at each coarse face centroid: valid
representative identity, not complete ancestry after seam collapse.

Supported domain: the two supplied static triangle GLBs, identity mesh nodes,
float32 positions, opaque JPEG/PNG or vertex colour. The implementation rejects
unsupported node transforms, nonfinite measurements, failed chart packs and
missing outer segments. Own only an explicit output directory; no production
state is changed. Exact source leaf ownership seeds are an input, not a source of
coarse geometry. The reproduction path can build them from the source.

Independent acceptance decodes every output, checks full leaf attributes,
provenance uniqueness, all-corner picking IDs, coarse primitive references,
bounds, monotonic GE and every source-shared-edge adjacent pair across all depths.
Geometry/colour evaluation uses 200k area samples per direction per level.
Native/WASM buffer bytes agree; 126 GLBs validate without errors or warnings.
The planar control releases child-interior vertices, preserves outer segments
and rejects a missing-border mutant. Final-source nonauthor review is required.

Proof limits: no live Cesium/browser test, no KTX2/mips, no lit-material fidelity
claim, no full-model scaling result. Adjacency means shared original edges;
unrelated nearby surfaces are not welded by the oracle. Sampling can miss extrema;
independent root max 0.390762 m exceeds published GE 0.385378 m. Source-image/BVH
residency is not bounded for large models. Current baseline uses fewer root
triangles and a different pipeline; the historical coarse comparison is 7.59×,
not a demonstrated 10× end-to-end improvement. Current baseline border projection
outliers include ambiguous source holes; its same-depth outlier is not a proven gap.

## Reproduce

Use the pinned local paths/packages in `reproduce.sh` to build the exact seed
leaves/tools if absent. Then detach `setsid -f bash bench/lod_cluster_dag/reproduce_hlod.sh`.
It builds the chart helper and runs tex3/crop3 serially at idle priority, with
single-thread numerical libraries, measured builds, decoded quality/seam checks,
first-view simulation, format validation, independent decoding and analytic controls.
No 2048 input, browser or server is used. Scratch and evidence stay under the
experiment's assigned directories. The final timing repeat's rebind manifests
prove all published bytes match the earlier evaluated/validated bytes.

The complete external evidence is bound by manifests below; final production
adoption still needs bounded Rust integration, KTX2/mips, a sampled-error policy,
coarse identity semantics and broader nonmanifold/thin-sheet/viewer validation.

- `hlod-execution-source.sha256`: `febcf37169b9009cdd44ff2a2191e35055842ad1df426c717d6ddae7732e162e`

- `hlod-artifact-manifest.sha256`: `1a5c38e2c48b2abc6acf3701d1bacf915bbb8b4876bb9c6dfbeb3c925e01fbff`

- External dependency snapshot `hlod-external-dependencies.sha256`: `994ac01d0bf38c80c935cc267759c7681f626af90aad14288476a1e04438f780`

Independent final-source review accepted bounded research submission; all 13
execution sources and 419 artifact identities verified. Its nonblocking external
dependency gap is addressed by the separate simulator, tree helper, JS/WASM,
Python package and executable snapshot above. This is a local identity snapshot,
not a hermetic environment or broader production acceptance.
