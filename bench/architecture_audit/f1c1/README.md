# F1c1 explicit placement foundation

Implementation candidate for [#142](https://github.com/BenDyson-Arch/rusty-tiles/issues/142), following merged F1b3 #141. The [contract](../../../docs/architecture/f1c1-contract.md) defines placement before source I/O: a raw Local/Wgs84 choice resolves once to a finite rigid f64 root transform and actual normalized parameters. PreparedMesh owns resolved state, and one pure manifest writer consumes it. Leaf geometry and companions stay local; no ECEF f32 bake, compatibility branch or CRS fallback is introduced.

Three Sol 6.1 agents audited [spatial meaning](audits/spatial.md), [ownership](audits/ownership.md) and [acceptance](audits/acceptance.md) before implementation. The separate nonauthor [review](review_probes/review.json) inspects the final candidate and its evidence. Repository AGENTS.md and the architecture gate require this sequence for future foundation slices; green historical tests alone are insufficient.

Production is pinned to `3bd86c352c0a430731528040379ce1765fc11e67`. [build-manifest.json](build-manifest.json) binds all 90 compiled inputs to that Git tree and worktree, unique frozen portable/native binaries, and the installed wheel/extension. Each binary was copied after its build/test exited successfully, while no shared Cargo operation ran. Later commits contain acceptance, documentation and evidence only. Shared target binaries are never acceptance inputs.

## Disposition and clean ownership

| Existing/proposed component | Decision and evidence |
| --- | --- |
| Local source/resource capture, triangle/companion decoding and complete-triangle partition | Retain only within the independently replayed bounded profile. Source geometry/material truth precedes world evaluation; selected capture and F0 ownership remain intact. |
| Unchecked legacy Cartographic/HPR/CRS placement dependency | Replace in this operation with one pure validated placement owner. The declared cartographic frame defines pole meridians and elevated/negative anchors directly. |
| Raw request retained by prepared producer | Remove. PreparedMesh contains resolved output, policy, leaf ceiling and placement; redundant Snapshot.input is removed. |
| Duplicate Y-up conversion and consumer-built manifest | Rework into one concrete axis helper and one format writer. One leaf-name owner prevents manifest/write drift. No general matrix or producer framework is added. |
| File capability admission after production | Rework the existing platform check into one pure runtime function, called before this mesh consumer reads a source and reused by staging. Unsupported-target ordering is a static obligation; no unsupported OS execution is claimed. |
| F1b3 report/profile | Replace with schema4/profile `f1c1-placed-gltf-v1`, typed placement and exact emitted transform. Local uses the same writer with explicit identity. |
| Broader CRS, source identity/picking, approximation and implicit delivery | Defer with explicit gates. F1c2 keys originate before partition; no unused identity fields are added here. Advertised legacy routes still need supported-use replacement/removal decisions. |

[Spatial disposition](spatial-disposition.md) records mathematical and format choices and their limits. Design probes found that Cesium's convenient elevated/pole frame does not define the declared frame, and that direct f32 ECEF storage loses local detail. Independent budget analysis also found that a rounded f64 total could admit an immediately-above-limit request. The contract now uses a conservative representable remaining cap and an exact bounded five-term expansion, with no arbitrary epsilon. [Budget probe](probes/budget-results.json) passed 90,240 exact-rational comparisons; this is algorithm evidence, separately followed by compiled candidate boundary probes.

Two candidate replay failures were harness defects: duplicate planned output names, and treating Clap arity errors as producer job errors. [Corrections](probes/harness-corrections.json) retain numeric/combination expectations and distinguish usage transport. Source inventories and hashes are checked for every positive.

The first resource runner also inherited fixture-generation memory in wait4 RSS. [Control](probes/rss-launcher-results.json) demonstrated the contamination. A [fresh-worker control](probes/resource-worker-results.json) and final full replay replace that measurement. wait4 still includes the explicitly recorded small worker launch floor; sampled converter VmHWM is reported separately and may miss transients. The superseded receipt is retained in the coordinator cache, not promoted to final evidence.

## Final local evidence

Full receipts are gzip-compressed with original byte lengths/SHA256 in [receipts/index.json](receipts/index.json). [verification.json](verification.json) records completion and remaining CI gates.

| Proof | Execution and practical limits |
| --- | --- |
| Independent world semantics | Portable/native each206 positives and106 refusals. Decimal80/cardinal/Hamilton truth covers33 placements,34 source variants, three leaf ceilings, exact budget boundaries, degenerate boxes and source frames. Maximum added world-coordinate evaluation error about1.062e-8m, against a1e-6m limit. Source f32 storage error is assessed separately. |
| Actual placement consumer | Four fixed independent cameras/target sets in Cesium1.146/Chromium153, including pole137° and3cm geometry at50Mm height. Original root transform, identity modelMatrix, three leaves/triangles and all target picks pass. Wrong-axis/order/height/missing-root controls miss; far/near and GE0 controls are sensitive. GPU depth error is measured separately, not used as the offline arithmetic oracle. |
| Installed adapters | CPython3.14.7 wheel passes39 API tests with empty PATH. Placement CLI/wheel parity passes134 positives and100 typed refusals; complete member bytes/reports, prior output preservation and no scratch are checked. |
| Source/format/material regressions | Native prior oracles: embedded84 positives/90 refusals, local resources69/136, core PBR94/140; geometry controls also pass. Khronos2.0.0-dev.3.10 checks26 sources and104 emitted leaves with zero errors/warnings. Three0.180 lit source/leaf comparison has RGBdelta0 and ten sensitive controls. |
| Resource limits | Three Local/placed workload pairs:100k single/multiple leaves and4096 shared six-attribute primitives. GLB/image bytes are identical; four cap/request refusals create no work. These measurements support concrete workloads, not asymptotic or universal resource claims. |
| Lifecycle and platform references | Full portable/native Rust suites, workspace lint with warnings denied, Python Rust mappings and14 public mesh tests pass. Frozen independent21 cartographic frames and rational rotations produce42 calls in the Rust test run on each portable CI target. Hosted cross-platform runs remain pending. |
| Separate nonauthor review |109 handcrafted frozen-CLI invocations plus29 installed API calls, exact Fraction boundary distributions, independent Decimal source/world math, sensitivity controls, callback exception identity, SIGINT/KeyboardInterrupt preservation and reentrant placement isolation. See the review's own scope and artifact verification. |

The probe snapshots under `probes/` preserve authoring provenance and original local cache paths; they are not CI drivers or proof that those paths exist elsewhere. Portable acceptance drivers are `tests/f1c1_oracle.py`, `tests/f1c1_viewer.py`, `tests/f1c1_wheel.py`, the installed API suite and `resource_probe.py`. The resource probe requires Linux `/proc`, POSIX spawn/wait4 and an explicit binary SHA; no compiler or browser dependency is introduced by it.

Hosted platform/native/wheel/official Blender CI, broader source CRS/datum/epoch admission, identity/picking, approximation, implicit delivery and remaining utility-operation migration stay separate gates. This candidate does not authorize deletion of all legacy job/report APIs or a release.
