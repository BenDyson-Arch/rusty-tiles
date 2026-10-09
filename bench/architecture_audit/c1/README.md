# C1 payload inspection evidence

Final functional source: `0a1cc94e467f92f615215ddc6327d59d7b3a64c0`, based on develop
`00a9d1c34c3017a9475e6099c2805d6d2366d70a`.
Frozen native binary SHA-256: `d4f2bae195067d7b93aec1e98cb82085553c1410f17f9bac3163b5120f5b7eef`.
[Build manifest](build-manifest.json) pins 83 compiled source hashes and fixture provenance.

Two fresh reviewers who authored none of the production change reviewed the PR:
[fresh payload review](fresh-payload-review.md) and
[fresh integration review](fresh-integration-review.md). They found five payload
correctness gaps, inconsistent local ZIP headers, and FIFO opening that could block.
Their independently executed final controls verify the repairs. The integration
reviewer authored added tests; the reviews identify authorship and execution separately.
The [initial cross-review](independent-review.md) is historical and was insufficient
for these boundaries. Initial acceptance records remain available at PR commit `d9305a7`.

Final acceptance records:

- `cli.json`: 125 fresh independent cases; exact checked-corpus regeneration and unchanged inputs.
- `fresh-payload-findings.json`: 15 focused probes from the fresh reviewer.
- `resources.json`: 20 fresh-process measurements, with unavailable descriptor samples reported explicitly.
- `producer-alignment.json`: 147 actual producer b3dm framing controls.
- `historical-final.json`: original missing-BIN defect rejected.
- `official-positive.json`: 16 independently decoded Khronos positives, zero errors.
- `wheel-parity.json`: 125 isolated installed-wheel controls with complete success-report and failure-class parity.
- `wheel-api.json`: 33 fresh-venv API tests with empty PATH.
- `verification.json`: full default/native Rust suites, lint, Python and packaging status.

The portable release wheel excludes native geospatial/JPEG support. Its installed
extension hash matches the wheel ZIP entry. Baseline and intermediate-candidate
counterexamples are retained as historical evidence, not claims about the freeze.
Resource observations are not universal RSS or time guarantees. The finite profile
and uninspected material/metadata, implicit addressing, decoded bounds and source/LOD
semantics are defined in [VALIDATION.md](../../../docs/VALIDATION.md).
Remote CI remains the merge gate; no release/publication is authorized.
