# F1a independent fixture and acceptance oracles

Current replay uses schema4/profile `f1c1-placed-gltf-v1` with an explicit identity root transform and typed Local placement report under the [F1c1 contract](../docs/architecture/f1c1-contract.md). This remains local geometry/resource regression evidence. The separate [F1c1 oracle](f1c1_oracle.md) proves Earth placement and the full world transform chain; historical receipts below retain their original scope.

These scripts inspect the bounded profile in [the contract](../docs/architecture/f1a-contract.md). They use Python's standard library to write and read GLB/ZIP bytes and calculate transforms; they never import the Rust loader, writer, validator, hierarchy reader or coordinate helpers. Passing proves the exercised cases, not the entire glTF/3D Tiles universe.

The mathematical checks follow the primary [glTF2.0 specification](https://registry.khronos.org/glTF/specs/2.0/glTF-2.0.html): column-major transforms, local T*R*S and parent*local composition, and global-determinant front-face orientation. The raw reader checks actual buffer/view/accessor ranges and index references. Authored normals use an independently written cofactor inverse transpose. Literal analytical goldens check reflected nonuniform and nested positions, normals and orientation before the reader serves as a source oracle.

[3D Tiles1.1](https://docs.ogc.org/cs/22-025r4/22-025r4.html) applies glTF node transforms, Y-up to Z-up `(x,-z,y)`, then tile transforms. GE drives screen-space selection; the empty root uses the contract's extent-derived routing metric, not a measured approximation bound. [Primary empty-placeholder discussion](https://github.com/CesiumGS/3d-tiles/issues/609) and [Cesium traversal](https://github.com/CesiumGS/cesium/blob/1.146/packages/engine/Source/Scene/Cesium3DTilesetTraversal.js) explain why zero-error empty nodes cannot simply be assumed to descend. Viewer evidence establishes behavior for one pinned implementation and defined cameras only.

## Standard-library oracle

```sh
python3 -B tests/f1a_oracle.py --self-test
python3 -B tests/f1a_oracle.py --binary target/debug/rusty-tiles
```

`--binary` runs self-tests and eleven analytical source variants with leaf limits1,3,1000 (33 conversions), including forced multileaf results, then independently inspects each output and compares CLI `meshReport` to embedded `conversion.json`. `--json-output PATH` stores a machine-readable summary including binary SHA. Every required test runs; absent binaries fail rather than skip.

It checks the complete oriented triangle multiset, normals including absence, exact material semantic presence/omission excluding names, declared component storage tolerances, decoded content bounds, child-box/root containment, the flat explicit hierarchy and routing GE formula, exact member closure, report facts and independent 3TZ MD5/index/local offsets. ZIP CRCs use stdlib zipfile; 3TZ index parsing uses struct/hashlib. Synthetic archive controls use stdlib ZIP, independently of the production codec. Corruptions exercise missing triangles, orientation, positions, missing/wrong normals, materials/default-valued declaration omission, BIN/accessor ranges, bounds, root metric, report, orphan and missing members.

Generate a reusable fixture without a converter:

```sh
python3 -B tests/f1a_oracle.py --fixture /tmp/f1a-source.glb --triangles 8 --variant nested
python3 -B tests/f1a_oracle.py --source /tmp/f1a-source.glb \
  --archive /tmp/f1a-output.3tz --leaf-limit 3
```

Variants: standard reflected matrix, coincident, degenerate, nested, instanced, default-scene selection (`scenes`), interleaved, normal-absent, default-material, empty-material, nonindexed. `fixture(triangles=N, variant=...)` returns bytes for integration callers. The generator's maximum100,000-triangle standard case stays inside the magnitude and accessor ceilings. The geometry matcher intentionally uses a simple quadratic multiset match for small analytical fixtures; it is not a large-data benchmark. Boundary generators may be used for RSS/scratch/FD sampling without this full matcher.

Resource evidence should distinguish input count/bytes, expanded instances and output member count. Use e.g.100,100,000triangles with a large leaf limit, and4096coincidenttriangles with leaf limit1 for the member ceiling. Use4097coincidenttriangles/limit1 as refusal control. Measure admitted source/JSON/accessor/instance boundaries separately; these triangle cases do not establish the complete32MiB source ceiling. `/usr/bin/time -v` process maximum RSS and sampled `/proc` descriptors/threads/scratch are different measurements; sampled maxima do not prove true peaks or a general memory bound.

## Real viewer routing

Already installed pinned Cesium/Playwright and Chromium are required; the scripts do not download packages. Replay:

```sh
python3 -B tests/f1a_viewer.py \
  --binary target/debug/rusty-tiles \
  --cesium-dir "$CESIUM_DIR" --node-modules "$NODE_PATH" \
  --chromium /usr/bin/chromium --json-output /tmp/f1a-viewer.json
```

Local executed assets: `/home/bend/.cache/rusty-tiles-117-browser-cache/node_modules/cesium/Build/Cesium` and its parent node_modules; recovered from existing npm offline cache with `npm install --prefix /home/bend/.cache/rusty-tiles-117-browser-cache --offline --ignore-scripts --no-audit --no-fund playwright@1.63.0 cesium@1.146.0`. Existing Chromium runs headless with SwiftShader. This setup changes no repository dependencies.

The viewer runner produces twelve source triangles/four leaves, verifies the archive independently, serves only local assets, and places local coordinates in a display-only Earth-fixed ENU frame. This is a test presentation transform, not source CRS inference or a new conversion feature. Viewport1000x800, SSE8, heading0, pitch-π/3, near distance4r and far10,000r are recorded. It checks actual post-render selected leaves, render commands, rendered triangle count, a scene pick, far suppression and return-to-near. A separate manifest with root GE0 is a negative control. Initial tilesLoaded alone never passes the check. External requests, request failures and page errors fail acceptance. All temporary fixtures, servers and browser processes are cleaned up.

Current viewer evidence is Linux Chromium153.0.8010.52, Cesium1.146.0/Playwright1.63.0. It does not prove other viewers, all distances, material appearance accuracy, GPU numerical fidelity, CRS or crash durability. Stdlib analytical checks and viewer traversal complement each other.
