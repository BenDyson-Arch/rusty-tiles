# C1 independent archive and payload implementation review

Status: candidate review; final source/binary freeze and final test pins pending.
Review baseline branch is `feat/c1-payload-validation`, based on merged develop
`00a9d1c34c3017a9475e6099c2805d6d2366d70a`. This document does not identify the
working tree as an immutable accepted artifact.

## Reviewer scope and independence

The reviewer authored public validation types, CLI/Python adapters, the Error
bridge, supporting documentation and migration tests. Those files are not
independently certified here. Nonauthor review covered `src/validate.rs`,
`src/validate/admission.rs`, `src/validate/payload.rs`, the shared bounded implicit
presentation helper and the opened-file 3TZ index codec change.

The review is source inspection plus inspection of separately recorded probe
results. This reviewer ran no Cargo suite and did not independently execute the
reported 39-control suite. Final evidence must identify its own immutable source,
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

Archive and payload JSON now share one bounded duplicate-key-rejecting parser;
a typed resource-limit flag prevents user error-message strings from changing
the failure category. URI handling separates valid references outside the
archive-local profile (Unsupported) from malformed/archive-escaping references
(InvalidInput). These are inspected candidate corrections pending final pins.

The typed InvalidInput/Unsupported/ResourceLimit/Io outcomes have shared CLI and
Python mappings. External validator subprocess execution is deliberately
removed. A separately run official validator owns its own evidence and cannot
upgrade an incomplete built-in result invisibly.

## Evidence and remaining gate

The coordinator reports 39 independent Rust/CLI controls passing and is running
broader default/native and installed-wheel checks. These are attributed results,
not independently rerun here. Baseline and intermediate candidate failures are
retained as counterexamples; they are not evidence that the revised candidate
still contains those defects. Conversely, a source fix is not an executed pass.

Final acceptance requires frozen source/binary/fixture provenance, malformed
payload and ZIP controls, primitive-work and implicit-presentation resource
regressions, actual supported producer outputs, typed Rust/CLI/installed-wheel
parity, immutable input preservation and required full checks. Unavailable
checks remain unverified. C1's finite payload profile does not certify every
glTF extension, metadata/implicit semantics, decoded world-space containment,
source fidelity or the complete #133/#125/#113 release gate.
