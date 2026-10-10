# Independent selected-primary controls

This lane authored literal JSON fixtures without production encoders or another
author's fixture generator. It made fourteen serial executions against the
unchanged frozen portable CLI from source
`5c68f7b52b72e2f6e72096e930f85d27f2897448`. It did not edit production code,
build, install, run browsers, or run a broad corpus. Each execution had a ten
second timeout, absolute nice 10, CPU affinity `{0,1}` and Rayon two workers.
The longest observed execution was 0.044 seconds. All fixture bytes remained
unchanged across every execution.

The [receipt](receipt.json) preserves complete target responses, classifications,
archive/member/input hashes, source manifest, binary and executed driver pins.
The [adjudication](adjudication.json) records primary provenance and precise
decimal sensitivity. The deterministic [raw bundle](raw-evidence.tar.gz)
losslessly retains all literal JSON, GLBs/local glTF, actual 3TZ bytes, stdout,
stderr, execution controls, source pin, and fetched pinned primary bytes.
Its [manifest](evidence-manifest.json) binds each retained file. Original external
records are under `/tmp/rusty-tiles-c1-primary-controls-qrndemti`.

## Selected authority independently verified

The official [Khronos registry specification](https://registry.khronos.org/glTF/specs/2.0/glTF-2.0.html)
was opened independently through the web tool. Its header identifies glTF 2.0.1,
published 2021-10-11, source revision
`8e798b02d254cea97659a333cfcb20875b62bdd4`. A separate direct raw-registry download
received HTTP 403; no retained complete registry HTML snapshot is claimed.

The [exact pinned source](https://raw.githubusercontent.com/KhronosGroup/glTF/8e798b02d254cea97659a333cfcb20875b62bdd4/specification/2.0/Specification.adoc)
was fetched independently: 148,064 bytes, SHA256
`10aebb6a8362a155b196448912bf6250bf046f150431fa73c6cd657c44185aff`.
It is byte-identical to the preexisting local cache. The fetched pinned
[accessor schema](https://raw.githubusercontent.com/KhronosGroup/glTF/8e798b02d254cea97659a333cfcb20875b62bdd4/specification/2.0/schema/accessor.schema.json)
is 6,517 bytes, SHA256
`86f9aacb0b616e1f6a1d5b8ff81e92294602e9e3c98c7f13d2cefdf987c87e35`.
Both original files are retained in the bundle; provenance is selected published
source, not unpublished main or implementation agreement.

Section 2.7 allows schema integer properties to use zero-fraction decimal or
exponent notation and forbids nonzero fractional values. Section 3.6.2.5 permits
arbitrary declared min/max if neither sparse nor bufferView exists. Stored
binary extrema still require matching declared bounds. The pinned accessor
schema separately requires omitted bufferView to initialize accessor values to
zero; these fixtures declare no sparse values or extensions.

## Finite outcomes and precode decision

| Independent control | Primary-derived expected outcome | Frozen CLI outcome |
| --- | --- | --- |
| Stored f32 zero POINTS and viewless zero POINTS | Admitted | Admitted |
| Count `1.0`, componentType `5.126e3`, POSITION reference `0e0`, buffer byteLength `12.0` | Admitted | InvalidInput for each |
| Local glTF with integer-valued decimal/exponent count/type/length/view/node/scene/attribute references | Admitted | InvalidInput |
| Count `1.5`, POSITION reference `0.5`, componentType `5126.5` | InvalidInput | InvalidInput for each |
| Count `1.0000000000000001` and `10000000000000001e-16` | InvalidInput | InvalidInput for each |
| Viewless, non-sparse VEC3 with min=max `[17,-4,8]` | Admitted | InvalidInput |
| Identical arbitrary bounds on actual stored zero f32 bytes | InvalidInput | InvalidInput |

The six rejected valid controls are executed outcome defects against the
selected primary and the existing declared C1 core/viewless profile. They are
not evidence of malformed payloads. An intentional exclusion would require an
explicit finite Unsupported choice and justification; no such exclusion is
declared in the current profile. Recommend correcting these outcomes while
preserving the entire previously admitted C1 domain. This recommendation does
not authorize narrowing the profile to easier fixtures.

Exact decimal analysis of the two rounding-sensitive count controls establishes
nonzero fractional mathematical values even though binary f64 represents both
as `1.0`. A correction based only on `as_f64().fract() == 0` would incorrectly
admit them. Integer-property interpretation must retain exact numeric meaning
until integrality and range admission are settled.

Decoded zero POSITION values and declared bounds are separate facts. These
fixtures independently derive one zero VEC3 from twelve literal little-endian
zero bytes, or from the pinned viewless initialization rule. Arbitrary viewless
declared bounds do not change the zero values. This lane does not establish
decoded-world content containment, scene transform correctness or rendered
geometry meaning. For malformed fractional declarations, the zero byte facts
do not make the accessor declaration admissible.

Source SHA comparisons establish that payload, JSON, validation operation and
validation types match the frozen source manifest. They establish execution
identity only. These fourteen controls do not establish universal glTF
conformance, complete retained C1 profile acceptance, candidate implementation
acceptance, A2 acceptance or release authorization. Final correction requires
independent final-source controls and the complete profile's separate evidence.
