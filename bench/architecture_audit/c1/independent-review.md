# C1 independent archive and payload implementation review

Status: functional source review at latest functional commit
`64122fd3085347ea95c2d2458bc5c109bfd06a59`; frozen native binary identified,
final corpus/resource evidence recorded; installed-wheel evidence recorded; source-package verification recorded; CI pending.
Review baseline branch is `feat/c1-payload-validation`, based on merged develop
`00a9d1c34c3017a9475e6099c2805d6d2366d70a`. This source commit is the functional freeze recorded in [build-manifest.json](build-manifest.json),
not a resource acceptance claim. Later evidence-only edits require their own recorded provenance.

## Reviewer scope and independence

The reviewer authored public validation types, CLI/Python adapters, the Error
bridge, supporting documentation and migration tests. Those files are not
independently certified here. Nonauthor review covered `src/validate.rs`,
`src/validate/admission.rs`, `src/validate/payload.rs`, the shared bounded implicit
presentation helper and the opened-file 3TZ index codec change.

The review is source inspection plus inspection of separately recorded probe
results. This reviewer ran no Cargo suite and did not independently execute the
reported corpus; the execution records and logs were independently read. Final evidence must identify its own immutable source,
binary, toolchain/features and fixture hashes before acceptance.

## Findings and candidate resolutions

- The original content reader validated JSON declarations without actual BIN,
  accessor or index payload checks. Independent baseline probes reproduced
  missing BIN, unsafe stride/offset/index, NaN position and required-unknown
  extension acceptance, and a debug overflow panic. The new inspector checks
  actual envelope/resource/view/accessor/index bytes under finite limits. Those
  baseline defects are executed evidence supplied by the independent oracle
  owner, rather than new executions by this reviewer.
- Nonauthor review identified mixed index/attribute bufferViews and malformed
  attribute set suffixes. The independent
  [candidate role probes](candidate-role-defects.json) reproduced acceptance in
  the earlier candidate. The payload owner implemented role and semantic-set
  checks; final pinned controls must demonstrate rejection.
- Reusing a large index accessor across many primitives amplified index scanning
  work beyond the accessor-value count admission. The separate
  [work-amplification probe](candidate-work-amplification.json) timed out after
  three seconds on 1,000 primitives sharing one million indices. This does not
  establish a memory or universal runtime failure; it demonstrates the earlier
  counter did not bound that repeated work. Final cached/admitted primitive
  inspection must retain its own regression and resource evidence.
- ZIP admission initially checked only central directory extents. The revised
  preflight parses bounded ZIP64 extras and validates local header/name/payload
  extents before ZIP-library seeks. Central and local names/flags/methods agree;
  stored lengths and aggregate sizes are checked. Arithmetic uses checked
  addition. Source cap, entry count and central-directory size are admitted
  before allocating the ZIP directory. Malformed EOF/InvalidData now map to
  InvalidInput, while other source read errors retain Io.
- The original hierarchy-depth rejection used InvalidInput despite being a
  resource ceiling. The candidate uses ResourceLimit and exposes depth 128 in
  typed limits.
- Per-document implicit expansion limits did not imply an archive-wide retained
  presentation bound. The revised Check owns one ExpansionBudget and passes
  its mutable reference through every external expansion and nested implicit
  link; it is not recreated inside each document. Node/presentation-byte charges
  occur before retaining child results. Native subtree caches remain bounded
  within an expansion; this is not a claim that all archive resources are cached.
- The member read counter originally appeared to cover every read. Documentation
  now explicitly limits that counter to member-hash and payload/document work;
  separately bounded ZIP metadata/index preflight is outside it. This is an
  engineering work limit, not a total-process RSS or wall-clock guarantee.

## Ownership and truthful result scope

ArchiveResources owns ZIP resource authority, reference admission and explicit
member reads. Payload resolution receives archive-owned bytes and does not open
paths, fetch remote URIs, own publication policy or import converter-specific
source acceptance. The index check and archive reader use clones of the selected
open file, avoiding a pathname reopen for payload authority. Final source
metadata/identity checks detect the documented changes, but do not establish a
hostile concurrent-writer snapshot guarantee.

Required unsupported extensions fail explicitly, rather than becoming opaque
successful payloads. Supported quantization and bounded meshopt NONE decoding
have concrete handling. The candidate now admits modern b3dm v1 with a strict
28-byte header/table framing and the same embedded GLB proof, needed by an actual
fragmented vector producer. Feature/batch table meaning remains uninspected.
Restart support is explicitly a bounded indexed LINE_STRIP subset of the
experimental CesiumGS/glTF draft `9811e8407d4533500cfc6b10e3bc408345035a6f`.
Its maximum-component sentinel requires both Used/Required declarations and
remains invalid in ordinary core modes. This does not claim ratification or
full draft draw-mode support. The narrow admitted profile requires at least
two vertices in each delimited segment; leading/trailing/consecutive or singleton
segments are Unsupported rather than invented draft InvalidInput failures.
Segment edge behavior and cache reuse across core and restart modes require
independent controls; this reviewer read the pinned
primary draft and supplied that contract to the payload owner.
The published glTF 2.0.1 registry revision is
`8e798b02d254cea97659a333cfcb20875b62bdd4`; permitted accessors without bufferViews
have zero-initialized values rather than missing stored payloads. Optional metadata/material semantics, image decoding,
scene transforms/skins/animations, decoded content bounds, geometric-error
accuracy and implicit addressing/availability remain explicitly uninspected.
Finite stored accessor values and matching declared extrema do not prove unit
normals, feature identities, rendered material meaning or source/LOD fidelity.
Hierarchy bounds are checks on declared volumes; they are not a decoded-world
content-containment oracle.

Inline buffer support is deliberately limited to standard base64 data URIs
using application/octet-stream or application/gltf-buffer. Declared/actual decoded
sizes remain bounded before allocation; resulting bytes receive the same
accessor proof. Other media/encodings and image data URIs remain Unsupported.
This addition serves actual opaque glTF resource preservation, not a generic
fetch/decode fallback.

The shared GLB eight-byte alignment helper inserts JSON whitespace and updates
GLB total/JSON lengths, preserving BIN bytes and buffer-relative offsets. The
vector b3dm producer's encoded-byte estimate includes the extra alignment bytes
and wrapper/table lengths; its actual writer and the implicit b3dm rewrite use
the same helper. This reviewer inspected that producer correction but has not
independently executed its final frozen replay.

The validator's single shared ExpansionBudget installs the bounded JSON parser
callback. Code inspection traced that callback into external implicit documents,
binary subtree JSON and every retained metadata-row extras JSON slice. Those
paths no longer use the unbounded default serde parser under C1. The remaining
raw parser in the separate implicit payload rewrite is a producer operation,
not a C1 inspection path; this review does not broaden that producer's claim.

Archive and payload JSON now share one bounded duplicate-key-rejecting parser;
a typed resource-limit flag prevents user error-message strings from changing
the failure category. URI handling separates valid references outside the
archive-local profile (Unsupported) from malformed/archive-escaping references
(InvalidInput). These are inspected corrections at the recorded functional freeze; executed
regression outcomes remain separately attributed.

The typed InvalidInput/Unsupported/ResourceLimit/Io outcomes have shared CLI and
Python mappings. External validator subprocess execution is deliberately
removed. A separately run official validator owns its own evidence and cannot
upgrade an incomplete built-in result invisibly.

The final minimum-version gate was inspected independently: digit-only
major/minor components are compared without integer parsing or overflow; future
requirements are Unsupported and malformed/non-string values InvalidInput.
The alignment lint delta uses is_multiple_of with identical divisibility meaning.
No new nonauthor blocker was found in these final changes.

## Evidence and remaining gate

This reviewer independently recomputed all 83 source-file SHA256 values in
[build-manifest.json](build-manifest.json), with no mismatches, and the frozen
binary SHA256:
`775267c80696e0df575437ee5e1ef5c634d579ea2aa2b637d50746d8cc4d5beb`.
The manifest records native-geospatial, dev/debug=0/incremental=0, Rust 1.98.0
and GDAL 3.13.3. Hash verification establishes artifact identity; it does not
substitute for executing the pending corpus/resource controls. The bounded
implicit JSON callback call chain was rechecked against these exact source bytes.

This reviewer independently read the final execution records, all naming binary
`775267c80696e0df575437ee5e1ef5c634d579ea2aa2b637d50746d8cc4d5beb`:
[cli.json](cli.json) contains 77 fresh CLI controls and records unchanged inputs;
[resources.json](resources.json) contains 20 fresh-process observations;
[producer-alignment.json](producer-alignment.json) records 147 actual b3dm members
and unchanged producer source; [historical-final.json](historical-final.json)
records typed rejection of the unchanged historical missing-BIN fixture.
Execution and exact fixture regeneration passes are attributed to the oracle
owner; these are not reruns by this reviewer. Resource observations are finite
Linux measurements, not a universal RSS/runtime guarantee.

The reviewer read the latest coordinator logs under
`/home/bend/.cache/rusty-tiles-c1-`: default-tests records 239 library tests plus
all integration tests passing; native-tests records 283 library tests plus all
integration tests passing. python-tests records 124 tests, 2 skips, with the
coordinator identifying the same final binary; binding-tests records 2 passing
tests. native-clippy and python-clippy record successful completion. Cargo,
Python and Clippy were not rerun by this reviewer.

The recorded [official-positive.json](official-positive.json) contains 13
Khronos-validator positive reports with zero errors, including the 2.0 minimum
version case. This reviewer inspected those recorded results, not executed the
official tool. Baseline and intermediate failures remain counterexamples to
previous implementations rather than claims about this freeze. The reviewer also read [wheel-parity.json](wheel-parity.json): 77 cases passed
under isolated Python with empty PATH, full CLI report/failure-class parity,
unchanged inputs and installed-extension hash matching the wheel ZIP entry.
Wheel SHA256 is `babce1ca2e533b50287137806a6e39902cea87f2e6401ba0b4e7b1607513a17a`;
extension SHA256 is `2f5af46358fc6b9854fe015576ffa7fa7b7ef94e476fa0af70e433b40f725b37`.
[wheel-api.json](wheel-api.json) records a separate fresh-venv empty-PATH run
of 33 API tests, zero failures/errors/skips, against the same wheel. These are
owner-executed records, not reviewer reruns. All 83 functional source hashes
were rechecked after recording wheel evidence and still match the manifest.
Source-package verification is recorded in verification.json; CI remains pending. Native and installed-wheel
controls are complete.

Final acceptance requires frozen source/binary/fixture provenance, malformed
payload and ZIP controls, primitive-work and implicit-presentation resource
regressions, actual supported producer outputs, typed Rust/CLI/installed-wheel
parity, immutable input preservation and required full checks. Unavailable
checks remain unverified. C1's finite payload profile does not certify every
glTF extension, metadata/implicit semantics, decoded world-space containment,
source fidelity or the complete #133/#125/#113 release gate.

Coordinator packaging evidence: `cargo package --locked` verified the clean
`97313d6` archive (479 files), including compilation after unpacking. The
archive hash and log hash are in [verification.json](verification.json). This
packaging run is attributed to the coordinator, not executed by this reviewer.
Two equivalent portable-test bool assertions were adjusted for Clippy after the
functional freeze; all 83 compiled-source manifest hashes remain unchanged.
