# F1c2/W1/API removal evidence

Date: 2026-10-10. Local Linux x86_64 acceptance is complete for the declared
F1c2/W1/package-removal scopes. Platform and official Blender CI remain merge
gates. This is not acceptance of arbitrary glTF, legacy mesh/CRS/LOD, all utility
exports, global Job/report deletion or release.

## Identity and artifacts

Production source commit: `4e1bda6de8d9f8be9ee4f776d51671420ff8faf5`.
The [source/artifact manifest](../../bench/architecture_audit/f1c2/receipts/source-artifacts.json)
records all 91 production input SHA256 values and aggregate
`22f4b82e6379d889441be2f0f9e364aa51570ec6f391b3c98c95be58ef0d8444`.
The coordinator checked them before and after builds; the separate reviewer
checked them independently. Later evidence/docs commits preserve these inputs.

| Final artifact | SHA256 |
| --- | --- |
| Portable CLI | `6f26363b4f9fd5270a5faf186fbaab8b3b139ebf13c9c788bb52d368eb1ea4f1` |
| Native geospatial/JPEG CLI | `9f7378c7bf2a81fb8f04a9fc667b2e55a4778f96bf51388262b995ea80a337b7` |
| Portable abi3 >=3.10 wheel | `3aa409052ab798588a9ca0056a307bb77662bddbd2af823593f89ecb26b35225` |

Builds used Rust 1.98, locked dependencies, one coordinator and isolated frozen
executables. The native environment used GDAL 3.13.3. The wheel was built with
maturin 1.15.0 in release mode. Its local linux tag is an integration artifact;
CI supplies the supported distribution wheels. No binary is committed here.

## Executed independent domain and consumer checks

| Proof | Final evidence and result | Limits |
| --- | --- | --- |
| Source identity/metadata | [Portable](../../bench/architecture_audit/f1c2/probes/identity-final-portable.json) and [native](../../bench/architecture_audit/f1c2/probes/identity-final-native.json): each 26 positive cases, 11 corrupt-output controls, six source refusals and 14 oracle self-controls. Includes 100000 IDs, 65535/65536, exact 8 MiB emitted names, absent/empty names, reflection/regrouping/partition and Local/Wgs84. | Document-local authored keys, finite source/schema; no imported business metadata or future proxy association semantics. |
| Original bytes and bounds | [Portable](../../bench/architecture_audit/f1c2/probes/wrapping-final-portable.json) and [native](../../bench/architecture_audit/f1c2/probes/wrapping-final-native.json): 22 cases with Decimal80 original-node/world references, exact resource aliases/bytes, omission-error diagonal/floor and sensitive controls. | Mathematical decoded source, raw/normalized admitted quaternion conventions; no arbitrary GPU f32 bound. |
| Separate original-source review | [Decimal100](../../bench/architecture_audit/f1c2/probes/review-original-source-final.json): five authored matrix/TRS/scene/cancellation cases and three sensitive geometry-bound controls. | Independent source evaluation rather than retiled f32 bounds. |
| W1 alias boundary | [Exact 64 MiB and +2 bytes](../../bench/architecture_audit/f1c2/probes/review-emitted-alias-final.json): admitted exact limit, Unsupported before scratch above limit, prior output preserved; sampled RSS/FD/scratch recorded. | Measurements are finite Linux observations, not universal whole-job resource guarantees. |
| Actual source picking | [Normal](../../bench/architecture_audit/f1c2/probes/picking-final.json) and [absent/empty names](../../bench/architecture_audit/f1c2/probes/empty-names-picking-final.json): each nine browser runs, 12 independently targeted triangles, both feature modes/default. Targeted swaps break only the affected association set. | Cesium1.146/Chromium153 actual picks; coincident surfaces do not acquire unique visual selection. |
| Official core glTF checks | [Khronos](../../bench/architecture_audit/f1c2/probes/khronos-final.json): 23 positives with zero errors/warnings; zero STRING view produces the expected error and four excess BIN pad bytes produce the expected warning. | Validator does not certify these extension semantics; independent decoder and consumer do. |
| Actual unchanged W1 consumer | [Browser](../../bench/architecture_audit/f1c2/probes/unchanged-model-browser-final.json): Local/Wgs84 both render two textured triangles and hit exact targets; original percent-URI buffers/image load, wrong-root/missing-alias/zero-top-error controls are sensitive. | Unmodified model/resources and ordinary SSE; finite loading/placement/render proof, no generalized shader-fidelity promise. |
| Retained geometry/resources/placement | F1a/F1b/F1b2/F1b3/F1c1 independent oracles replayed on final portable. [World placement](../../bench/architecture_audit/f1c2/receipts/f1c1-browser.json) passes four placements and all axis/order/height/root/error controls. [Lit PBR](../../bench/architecture_audit/f1c2/receipts/f1b3-browser.json) source/output RGB delta zero, ten sensitive controls detected. | Prior bounded domains retained; no approximation or general CRS acceptance. |

[Browser ledger](../../bench/architecture_audit/f1c2/probes/final-browser-evidence-ledger.json)
verifies final executable/source identities and all current driver/consumer pins.
Exact extension primary sources, draft/optional disposition and numerical
feasibility remain in the preimplementation audit. Synthetic or manifest-repair
feasibility receipts are never counted as final candidate execution.

## Lifecycle, adapters, removal and packaging

Full portable and native Rust suites passed on corrected S2. Final S3 differs
only in an immutable Geometry borrow for the identity plan and test-module
ordering; focused final portable/native unit, mesh, W1 and CLI suites passed.
Both workspace/all-target Clippy runs passed with warnings denied; formatting
and Python binding Rust tests passed. Compressed command logs are retained in
[receipts](../../bench/architecture_audit/f1c2/receipts/).

Rust external-consumer tests exercise exact captured aliases, deletion of source
after preparation, fixed sibling references, source/output exclusion, callback
errors/reentry/cancellation, concurrent CreateNew winners, prior-output
preservation, original bounds and report parity. Producer injection covers
persistent partial writes, required report/archive failures and owned cleanup
residuals. Packaging fixture callers use the accepted typed facade; entry order,
ZIP64, receipt, malformed fixture sensitivity and legacy implicit candidate
validation remain covered. Deleted public mutation/options symbols have no
remaining production declarations/callers; historical audit sources stay
labeled as baseline artifacts.

[Installed Python](../../bench/architecture_audit/f1c2/receipts/wheel-python.json)
passed C1 validation tests and all 42 API tests under Python3.12 with an empty
PATH. [Installed distro Blender](../../bench/architecture_audit/f1c2/receipts/wheel-blender.json)
passed all 42 API tests under Blender5.2.2. Both pin the final wheel, record the installed package location and include new identity decoder, W1 exact capture/reference authority,
callback and typed refusal checks. Distro Blender is additional local evidence;
it does not replace pinned official Blender4.5.14 platform CI.

Cargo source packaging passed. All 87 core src/preview/build.rs files extracted
from its source archive match the production manifest exactly. Generated Cargo
metadata is normalized by Cargo; the separate Python wheel owns its binding
artifact checks. Historical converter digests were updated only for the two
intentionally replaced model products; opaque convert and both broader mesh
recipes retain their prior bytes.

## Independent review and remaining gates

[Separate nonauthor review](../../bench/architecture_audit/f1c2/audits/fresh-review.md)
records source/ownership review, three reproduced first-candidate defects and
corrected-artifact controls, documentation/format gaps, exact pins and limits.
Earlier S1 failures and S2 passes are retained separately so review history is
not relabeled as final S3 acceptance.

Main CI passed at `5509da98`, including supported portable/native platform
builds and consumer checks. Installed-wheel acceptance failed on macOS and
Windows because a new test compared canonical and caller path spellings.
[Path-identity correction evidence](../../bench/architecture_audit/f1c2/receipts/ci-path-identity/receipt.json)
records a local symlink-path reproduction (one failure among 42 tests), followed
by all 42 passing with filesystem identity assertions against the same wheel.
All 91 production inputs remain unchanged; separate nonauthor review passed.
Corrected platform wheel checks and pinned official Blender acceptance remain
CI gates. Do not merge on local checks alone.

Broader mesh, raster pyramid and implicit rewrite retain legacy Job/report
ownership; shared report helpers and provisional utilities have separate
disposition gates. Approximation/error precedes implicit delivery. See the
[implementation/removal map](f1c2-implementation.md). Parent foundation and
release blockers remain open; no release, tag or publication is authorized.
