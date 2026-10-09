# F1b2 resource binding evidence

The next bounded #121 slice admits local JSON glTF and GLB resources through the existing local mesh operation. Source parsing, metadata admission and payload decoding remain pure; the consumer owns relative file binding and one immutable capture per physical dependency. The encoder and F0 runtime do not discover source dependencies. See the [contract](../../../docs/architecture/f1b2-contract.md).

Production source is pinned to `ce8a89c58e2781d72c2ac5dff686de2f8d340850`, following baseline `641e1b461f3b11621fd528fb05bfba45a6f930d6`. [build-manifest.json](build-manifest.json) hashes all 88 compiled source/resource inputs, both uniquely named frozen binaries and the installed wheel/extension. Later commits add acceptance and evidence; every compiled input must still match this pin. Frozen binaries were copied only after their owning Cargo build completed, with no Cargo operation running at the copy. Packaging verification may overwrite the shared target binary; that path is never an acceptance input.

## Audit disposition

Retain F0 attempt/publication ownership, bounded geometry/image IR, full-detail partitioning, stored-geometry bounds and pure leaf encoding. Replace the single-file read with consumer-owned binding and capture. Rework source admission into parsed buffer/image/accessor/scene plans followed by borrowed payload decoding. Do not introduce a URI resolver into codecs, synthetic GLB flattening in production, source paths retained for later reads, generic converter registries, compatibility branches, legacy atlas, external encoders or coarse LOD.

Sol 6.1 agents separately audited resource binding, decoder geometry/layout, and independent acceptance before implementing their assigned files. A fourth Sol 6.1 reviewer authored neither production code nor the main oracle. [Final review](review_probes/final-review.md) records 57 independent manual cases and two sensitive checker controls against the exact final binary hash. It reproduced three P2 defects in the initial candidate: source-file output descendants classified as Io, contradictory accessor metadata reaching dependency I/O, and loose enclosing min/max admitted instead of exact binary extrema. All three are independently verified fixed. A final root inspection reproduced a fourth P2 defect: growth after file admission returned Unsupported and could read up to the larger profile cap. Root-run deterministic controls failed before the fix and pass in both final Rust suites. The reader now uses the admitted length plus one detection byte and reports growth as InvalidInput; the separate reviewer inspected that fix and reran its stable-source controls. See [growth-regression.json](growth-regression.json) for execution attribution. The latter was retained behavior from the earlier decoder; the official Khronos validator independently confirmed its invalidity.

The source audit also tightened raw reserved URI data to glTF's required percent encoding and refused terminal dot directory locations. The independent oracle later corrected its C1 Unicode control interpretation and added raw/encoded U+0085 controls. These are recorded refinements, not decoder defects. Initial receipts describe their older admission scope and are superseded by the final receipts.

## Final evidence

- Portable and native external-resource oracle: 69 conversions (23 variants at three leaf limits), 136 refusals, and independently sensitive URI/base/logical-buffer/payload controls. Physical identity counters include unused dependencies. [Portable](portable-oracle-final.json), [native](native-oracle-final.json).
- Embedded F1b1 regression: 84 conversions and 90 refusals in both builds, with all 21 existing corruption controls. F1a geometry regression also reruns against the final portable binary. [Portable embedded](portable-embedded-final.json), [native embedded](native-embedded-final.json), [geometry](portable-geometry-final.json).
- Thirteen committed filesystem fixture manifests independently replayed; [static receipt](static-fixture-final.json) and [author/tool/hash provenance](independent-provenance.json).
- Installed wheel: 37 API tests with PATH empty; 69 external CLI/Python conversions with full report and archive-member byte parity, 120 declaration refusals; 84 embedded parity conversions and 90 embedded refusals. Filesystem/alias refusals also run through the Rust facade and CLI. [External parity](wheel-parity-final.json), [embedded parity](wheel-embedded-final.json).
- Full portable/native Rust suites, Clippy for both profiles and Python binding, formatting and source-package verification are recorded in [verification.json](verification.json). Deterministic private capture tests cover observed earlier dependency changes, root/alias pathname replacement, cancellation precedence, and symlink/FIFO refusal. Public Rust/Python callbacks delete source/dependency files after preparation; nested and concurrent attempts retain separate state. Writer/archive/cleanup fault evidence from the consumer remains exercised by the full suite.
- CesiumJS 1.146.0 and Playwright Chromium 153.0.8010.12: [routing](routing-final.json), [embedded appearance](appearance-embedded-final.json), and [external-buffer/image appearance](appearance-external-final.json). Appearance controls deliberately corrupt UVs and alpha semantics and must fail independently.
- Eight external byte/file workloads and fifteen existing image/geometry workloads: [external resource measurements](resource-final.json), [embedded resource measurements](embedded-resource-final.json). Maximum observed converter wait4 RSS in the external workload is 88,532 KiB. Process descriptor/thread/scratch observations are samples, not established maxima. The 64-reference alias workload counts only two unique dependency files; the distinct workload counts 64. Exact 64 MiB unique capture and 32 MiB individual file boundaries succeed; one byte over is refused.

The independent reviewer did not execute Rust callback/mutation tests or installed Python tests; those are root-run evidence, separately identified above. The root authored the measurement probe and frontend parity adapters; they are not independent fidelity truth. Their checks use the separately authored source/output oracle.

## Replay

Use a locally built candidate, not the historical absolute artifact paths. The CLI must implement this contract; there is no release/tag authorization.

```sh
cargo test --locked
cargo clippy --locked --all-targets -- -D warnings
cargo build --locked --bin rusty-tiles
python3 -B tests/f1b2_oracle.py --binary target/debug/rusty-tiles --json-output /tmp/f1b2-resources.json
python3 -B tests/f1b_oracle.py --binary target/debug/rusty-tiles --json-output /tmp/f1b2-embedded.json
python3 -B tests/f1a_oracle.py --binary target/debug/rusty-tiles
python3 -B bench/architecture_audit/f1b2/resource_probe.py --binary target/debug/rusty-tiles --output /tmp/f1b2-measured.json
```

The measurement probe needs Linux `/proc`, `cc`, and wait4; it imports only the previous resource-measurement helpers. The independent source oracle runs with Python's standard library, including `python3 -S`, using the fixed independently decoded JPEG manifest. Arbitrary JPEG review probes need Pillow; reviewer receipts retain that provenance. Browser runners require preprovisioned pinned Cesium/Playwright/Chromium assets and download nothing. `tests/f1b_viewer.py --external-resources` uses the same analytic appearance fixture with separately authored local resource files.

Native checks require the established GDAL/PROJ/system-SQLite environment and `LIBSQLITE3_SYS_USE_PKG_CONFIG=1`; wheel parity uses `tests/f1b2_wheel.py --installed PATH --binary BIN --output JSON`. The reviewer script requires an explicit expected binary SHA so it cannot accidentally review a rebuilt shared-target binary. Historical initial and intermediate receipts retain their own source/binary identity.

Stable source files during preparation remain a precondition. The captured set is not an atomic multi-file view, a secure-openat sandbox against hostile parent replacement, or protection against adversarial metadata restoration. The CLI wrapper may emit its initial conversion progress before entering the consumer; domain observer callbacks begin only after capture/validation. No coarse approximation, additional PBR channels, placement, metadata/picking or implicit mesh acceptance is claimed. #121 and 0.4.0 remain open.
