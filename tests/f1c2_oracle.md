# Independent source identity acceptance

`f1c2_oracle.py` is a standard-library source/GLB decoder. It imports no Rust
encoder, production serializer, or older repository oracle. Source truth comes
from authored node, mesh, primitive and original TRIANGLES indices before
reflection. Emitted positions, material/color corners and both property-table
associations are compared against that truth. Leaf-local table rows never become
source identifiers in the reference calculation.

The finite positive domain covers shared meshes at distinct nodes, coincident
instances, duplicate names, missing and authored-empty names, exact Unicode
including decomposed text, nested transforms, reflections, indexed/unindexed
triangles, and material/color grouping. Default scene index1 with roots out of
index order and sole-scene selection are separate cases. Leaf ceilings1/5/100
and local/WGS84 placement retain the same source keys. Optional `--large` adds
100,000 authored triangle ordinals (including65535/65536) and exact8MiB emitted
name storage. Repeated geometrically identical triangles establish the complete
ID inventory/cardinality; geometry alone cannot distinguish permutations among
identical authored triangles. Distinct-geometry and reflection controls establish
source-to-output association sensitivity separately.

Eleven emitted-artifact controls exercise missing instances, crossed feature
streams/tables, plausible source-column and reflected-ordinal swaps, malformed
accessor representation, out-of-range IDs, invalid name presence and malformed
string offsets. The decoder self-test also checks the all-empty STRING sentinel:
all offsets zero and exactly one zero storage byte. Zero-length, nonzero and
oversized sentinel views are rejected. Six candidate source refusals verify typed
errors and absence of staging/output parents: oversized label, non-string label,
multiple parents, ambiguous scenes, four extra terminal BIN bytes and the emitted
name budget exceeded by4096bytes. Multiple-parent graphs are outside the admitted
source profile; shared meshes are supported through distinct source nodes.

`f1c2_viewer.py` and `fixtures/f1c2_viewer.cjs` use original emitted geometry and
manifest in Cesium1.146.0, with no custom shader or model-matrix repair. Actual
`scene.pick` queries twelve distinct triangle interiors under default,
`source_primitive` and `source_triangle` selection. Sparse shared-table rows
exceeding `featureCount-1` are deliberate. Primitive-row and triangle-row swaps
retain geometry, valid IDs and corner uniformity, and must break the respective
properties while preserving the other feature set. Both normal and all-empty
label candidates are exercised. This is a finite local camera proof; picking a
specific surface among coincident surfaces is not uniquely observable.

`f1c2_khronos.py` and `fixtures/f1c2_validate.cjs` independently submit four source
GLBs and every ceiling5 emitted leaf to pinned Khronos validator2.0.0-dev.3.10.
Every positive must have zero errors and warnings. A test-owned zero-length
STRING view must trigger `VALUE_NOT_IN_RANGE`; four terminal BIN bytes outside
the declared buffer must trigger `BUFFER_GLB_CHUNK_TOO_BIG`. The latter is an
expected warning on a negative only. Khronos core validation does not establish
extension/source semantics; the decoder and Cesium probes supply that evidence.

Executed receipts live under `bench/architecture_audit/f1c2/probes`. Each candidate
receipt identifies the frozen binary and independent drivers, source/artifact
hashes and exact controls. `receipts/source-artifacts.json` and
`probes/final-browser-evidence-ledger.json` record the production-input manifest
and final consumer receipt bindings. Historical cache paths and hashes retain
their original meaning; the storage index does not manufacture missing archived
cache manifests. Synthetic feasibility receipts
are explicitly distinct from candidate acceptance. Harness corrections and
uncaptured stale-driver identities are recorded honestly in separate receipts.
This evidence does not establish unrelated PBR/placement/lifecycle invariants or
whole legacy API retirement; those retain their separate gates.

```sh
python3 -S -B tests/f1c2_oracle.py --self-test --large --binary /path/to/frozen/rusty-tiles --json-output /path/to/identity.json
python3 -S -B tests/f1c2_viewer.py --binary /path/to/frozen/rusty-tiles --cesium-dir /path/to/Cesium --node-modules /path/to/node_modules --chromium /path/to/chrome --json-output /path/to/picking.json
python3 -S -B tests/f1c2_viewer.py --binary /path/to/frozen/rusty-tiles --empty-names --cesium-dir /path/to/Cesium --node-modules /path/to/node_modules --chromium /path/to/chrome --json-output /path/to/empty-picking.json
python3 -S -B tests/f1c2_khronos.py --binary /path/to/frozen/rusty-tiles --validator-modules /path/to/node_modules --json-output /path/to/khronos.json
```
