# F1b3 core PBR and authored companions

Bounded implementation of [#140](https://github.com/BenDyson-Arch/rusty-tiles/issues/140), under #121/#113, following merged F1b2 #139. The [contract](../../../docs/architecture/f1b3-contract.md) advances the existing local mesh operation; there is no compatibility route, legacy fallback, atlas or generated tangent basis. This does not close broader mesh migration or authorize a release.

Production source is pinned to `92c7e02433efc2031df835290cd05d72a392ebdc`. [build-manifest.json](build-manifest.json) records all 89 compiled source/resource inputs, unique frozen portable/native binaries and the installed wheel/extension. Inputs match the Git pin and final worktree. Each binary was copied after its owning build finished, with no Cargo operation running. Later commits add tests, documentation and evidence. Shared target artifacts are never acceptance inputs.

## Ownership and audit

Three Sol 6.1 agents separately audited material/resource semantics, authored vertex/frame semantics and independent acceptance. [Audit notes](audits/) distinguish static findings from executions. The pure decoder owns validated material bindings and prepared corner attributes. One fixed five-slot material plan supplies selected and leaf closure and remapping; one channel writer encodes companions. Existing consumer capture and F0 publication ownership remain unchanged. No general extension registry or second pipeline is introduced.

The new conformal accumulated-transform restriction is a declared producer policy for tangent-bearing primitives. Reflections preserve companion corner association and reverse tangent handedness. Normal maps require authored normals and tangents. Finite colors outside [0,1] are explicitly Unsupported without clamping; this does not claim that every excluded source is otherwise valid glTF. No added orthogonality requirement is imposed on authored normal/tangent pairs.

The first candidate misclassified malformed unnormalized integer COLOR as Unsupported. The root executed the failing control and retained its expected InvalidInput disposition. The correction validates semantic encoding before the unused-accessor profile guard and before dependency I/O. [Regression](regressions/vertex-encoding-classification.json) preserves the actual initial log and both source/binary identities.

A separate nonauthor Sol 6.1 reviewer found no blocker in [27 handcrafted final runtime cases and eight sensitive controls](review_probes/final-review.json). [Source verification](review_probes/source-verification.json) independently checks every manifest input against Git and the tree. [Acceptance/CI review](review_probes/acceptance-ci-review.json) separately inspects the final oracle, installed API, lit renderer and complete emitted-leaf validator integration. It records its lightweight executions and does not claim the coordinator's full Rust, wheel or resource runs.

## Final evidence

| Proof | Final execution and limits |
| --- | --- |
| Independent geometry/material/resource semantics | [Portable](portable-oracle-full.json) and [native](native-oracle-full.json): each 94 positives and 140 typed refusal checks; 25 independent sensitivity controls. Oriented corners carry all companions, logical image identities and material semantics. |
| Prior profile regressions | Portable/native embedded: each 84 positives/90 refusals; external: each 69 positives/136 refusals. [Geometry](portable-geometry-final.json) and [26 static fixtures](static-final.json) replay separately. |
| Lit material behavior | [Three 0.180.0](independent-lit-viewer-final.json): original source versus every extracted output leaf, maximum sampled RGB difference zero; all ten wrong UV/slot/handedness/channel/scalar controls exceed unchanged tolerance 12. This is glTF material proof, not 3D Tiles traversal. |
| Routing and prior appearance | [Cesium routing](routing-final.json), [embedded](appearance-embedded-final.json) and [external](appearance-external-final.json) unlit appearance regressions. Cesium 1.146.0 cannot provide sensitive occlusion proof in this setup; the independent runner used a second standard renderer without custom shaders or weakened thresholds. |
| Official format validation | [Khronos](independent-khronos-final.json): 26 independently authored sources and all 104 emitted leaves at ceiling 3, external resources resolved, zero errors/warnings. This supplements rather than replaces source-fidelity inspection. |
| Installed Python | [Empty-PATH API](wheel-api-final.json): 38 tests, including the full new acceptance matrix. [Core](wheel-core-final.json) and [local-resource](wheel-external-final.json) adapters compare complete CLI/Python reports and archive member bytes. [Embedded](wheel-embedded-final.json) preserves its prior independent semantic/report proof. |
| Lifecycle and checks | [Verification](verification.json): full portable/native Rust suites (largest suites 262/306 tests), binding tests, Clippy for both profiles and binding, formatting. Existing snapshot, source mutation, observer/reentry, cancellation and publication/fault checks replay in those suites. |
| Finite workloads | [Four new core cases](resource-core-final.json), [15 embedded](resource-embedded-final.json) and [eight external](resource-external-final.json). New all-companion/all-slot workloads include 100k triangles in 1/128 leaves, 4096 shared-accessor primitives and 100001-triangle refusal. Single-leaf observed wait4 RSS is 98,644 KiB. Descriptor/thread/child/workspace observations are samples, not established maxima or performance guarantees. |

[Independent provenance](independent-provenance.json) pins authored fixtures, drivers, renderer/validator and their receipts. [Verification](verification.json) separately records coordinator executions and adapter hashes. CI repeats the new oracle/resource matrix, complete source/output Khronos harness and lit controls while keeping earlier regressions. Hosted platform/native/wheel/Blender checks remain pending PR CI; local Linux evidence is not a distribution-platform claim.

## Replay

Build a candidate from the checkout and use it explicitly; historical absolute cache paths need not exist.

```sh
cargo test --locked
cargo build --locked
python3 -S -B tests/f1b3_oracle.py --binary target/debug/rusty-tiles --json-output /tmp/f1b3.json
python3 -S -B tests/f1b3_khronos.py --binary target/debug/rusty-tiles \
  --validator-modules /existing/node_modules --json-output /tmp/f1b3-khronos.json
python3 -S -B tests/f1b3_viewer.py --binary target/debug/rusty-tiles \
  --three-dir /existing/node_modules/three --node-modules /existing/node_modules \
  --chromium /existing/chrome --json-output /tmp/f1b3-lit.json
python3 -B bench/architecture_audit/f1b3/resource_probe.py \
  --binary target/debug/rusty-tiles --output /tmp/f1b3-resources.json
```

The oracle uses Python's standard library. Browser assets must already be provisioned at their pins; runners download nothing. Resource measurement requires Linux `/proc`, `cc` and wait4. Installed parity reuses `tests/f1b2_wheel.py --core-pbr --installed PATH --binary BIN --output JSON`. Native checks use the established GDAL/PROJ environment and system SQLite.

Placement and identity, independently justified approximation/error, implicit delivery and remaining supported-use decisions still precede deletion of all legacy mesh/job/report APIs.
