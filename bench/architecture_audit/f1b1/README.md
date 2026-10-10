# F1b1 embedded texture acceptance

First bounded slice of #121 after C1, audit baseline `fdc5d18`; final compiled
source `07928888e808cfd51e0daab9575023898ede894f`. The
[contract](../../../docs/architecture/f1b-contract.md) defines the supported
profile, ownership, image/geometry limits, deliberate breaking report change and
remaining mesh obligations. #121/#113 remain open; no release is authorized.

## Final evidence

`build-manifest.json` binds 87 source/config/embedded-asset hashes, both binaries,
the installed release-wheel extension and a clean verified source package to
the compiled source pin. `verification.json` summarizes checks and remaining
remote gates. Initial/intermediate receipts are explicitly historical.

- `portable-oracle-final.json` and `native-oracle-final.json`: independently
  authored geometry/UV/material/image/sampler/archive readers, 84 accepted
  conversions and 90 refused inputs per profile, 21 sensitive corruption controls.
- `final-source-review.json` and `fresh-review.md`: a separate reviewer authored
  neither production nor the main oracle. Three reproduced findings were fixed:
  repeated UV validation, 16-bit image admission, and incomplete image framing.
  Final independent replay verifies 16 controls against source-bound binary.
- `wheel-parity-final.json` and `wheel-api-final.json`: installed portable ABI3
  extension, full 84/90 report/failure parity under isolated Python and empty PATH;
  35 API tests. Installed extension bytes equal its exact wheel ZIP member.
- `resource-final.json`: 15 boundary workloads, including exact32MiB source,
  source-image count, edge/per-image/aggregate pixels, repeated-view byte
  amplification and 4096 leaves with shared300k-vertex accessors. RSS uses Linux
  wait4; descriptor/thread/child/workspace figures are sampled, not universal bounds.
- `appearance-final.json` and `routing-final.json`: pinned actual Cesium consumer,
  CI's exact Chromium build, analytical texel/alpha probes, wrong-UV/alpha controls,
  near/far/returned-near traversal. Appearance uses display-only unlit lighting
  to isolate base-color semantics and does not prove arbitrary PBR lighting,
  every sampler's rendering or every GPU/client.
- `static-oracle-final.json`: retained independent F1a geometry tests adapted to
  the operation's new report profile. No parallel compatibility operation exists.

Provenance and sensitivity descriptions:
[independent oracle](../../../tests/f1b_oracle.md),
`independent-provenance.json`, and
[separate review replay](review_probes/README.md). PNG pixels/CRC/filter handling
use independent standard-library authoring/reading. A prebuilt six-color JPEG
has Pillow12.3.0 author/decoder receipts; source bytes are forwarded exactly.
The Rust-job oracle can run without Pillow using the pinned decoded receipt.

## Replay

Use a freshly built candidate and an external temporary directory. No production
validator or legacy atlas provides the fidelity oracle.

```sh
python3 -B tests/f1b_oracle.py --binary /path/to/rusty-tiles --json-output /path/to/oracle.json
python3 -B bench/architecture_audit/f1b1/resource_probe.py --binary /path/to/rusty-tiles --output /path/to/resources.json
python3 -I -B tests/f1b_wheel.py --installed /path/to/installed-wheel --binary /path/to/rusty-tiles --output /path/to/parity.json
python3 -B tests/f1b_viewer.py --binary /path/to/rusty-tiles --cesium-dir /path/to/Cesium/Build/Cesium --node-modules /path/to/node_modules --chromium /path/to/chromium --json-output /path/to/appearance.json
python3 -B tests/f1a_viewer.py --binary /path/to/rusty-tiles --cesium-dir /path/to/Cesium/Build/Cesium --node-modules /path/to/node_modules --chromium /path/to/chromium --json-output /path/to/routing.json
```

Source and production changes are complete for this finite profile. External
resources, additional PBR channels/attributes, placement/identity, implicit
delivery and justified coarse error semantics remain later #121/#120/#125
slices. Legacy broad mesh code earns no retention from these results.
