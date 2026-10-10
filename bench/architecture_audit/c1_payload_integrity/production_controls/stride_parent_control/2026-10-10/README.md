Single unmasked parent-stride control — frozen independent prediction

This is a new lane, separate from the unchanged six-case lane and compiled Rust
controls. It contains only one public negative, no Rust module. No target binary
or project encoder/decoder/numeric helper authored its bytes or expectations.

The root-authored requirement is isolated as follows: parent view stride8;
extension ATTRIBUTES/NONE stride12/count4 gives48 decoded bytes. POSITION is a
viewless FLOAT VEC3 count4 with zero bounds and implied zero values. The only
accessor referencing the compressed view is an UNUSED FLOAT SCALAR count4,
offset0. Under the current C1 profile its parentstride8 window occupies28 bytes,
fits48, has component/absolute alignment4 and finite independently read values.
It is neither an index accessor nor a second vertex attribute, so the current
index restrictions and shared-attribute stride requirement do not mask the gate.
The one view charges48 physical decoded bytes and components total12+4=16.

Pinned meshopt specification lines73–87 requires parent byteStride equality WHEN
DEFINED, independently of accessor role. Parent8 versus extension12 is the sole
intended refusal, and the runner asserts the COMPLETE typed error response:
InvalidInput, exit3, exact message 'meshopt and bufferView stride disagree', empty
stderr. The manifest records the complete hypothetical admitted report (two
accessors, one primitive, four vertices, all15 defaults/13 checks/6 exclusions)
for mutation analysis; it is a prediction, not observed acceptance.

Primary scope nuance: core line923 reserves byteStride for vertex data unless an
extension enables other layout; line975 describes other assigned semantic data
as tight. These texts do not establish a universal unused/non-vertex stride
acceptance guarantee. This negative intentionally tests the existing C1 profile,
which applies parent stride to unused accessor windows. Its hypothetical
admission is only evidence of removing the parent-agreement guard in that
profile, not external normative conformance of arbitrary strided non-vertex
assets. Even interpreted as tight4, the unused scalar would fit16 bytes and stay
finite; primary parent/extension mismatch still independently invalidates it.
A valid positive with a second vertex attribute would instead need agreedstride12;
that is already covered separately and is not changed here.

Retained literal primary Uint8Array arrays from zeux/meshoptimizer pinned commit
3d8e9b8a2a2b9a5becfc6fbc512307b04207ab5d supply85 compressed bytes and48 expected
bytes. Python struct independently reads scalar offsets0/8/16/24. Exact primary
URLs/revisions/hashes are in primary/pins.json. Generator uses independent stdlib
GLB framing, deterministic ZIP_STORED and digest-sorted3TZ index construction.
Public report constants repeat documented CLI schema; they are not semantic
numeric oracles. No dependency on production code or the old fixture manifest.

Run generator.py only into a fresh fixtures directory; it refuses overwrite.
Coordinator-owned runner.py requires explicit --binary, --binary-sha256,
--source-pin, --source-pin-sha256 and a fresh --run-dir. Root owns compilation and
all target execution. The lane author performed only bounded syntax/hash/struct/
JSON/ZIP checks. Nothing in this directory records target acceptance yet.
