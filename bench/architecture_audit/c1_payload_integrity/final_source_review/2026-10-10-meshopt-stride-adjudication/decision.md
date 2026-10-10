# Separate meshopt stride adjudication before correction, 2026-10-10

**Correct the per-accessor codec-stride equality before merge.** A newly executed
control demonstrates false InvalidInput on the frozen 2eb portable CLI for a
valid, admitted meshopt NONE view. This is a production outcome defect inherited
from the accepted reader baseline, not a malformed fixture or a justified
Unsupported profile choice. Earlier local/package reviews remain immutable
historical decisions on 2eb/2ef; they do not supply acceptance of a corrected
source. Merge acceptance is held until correction and fresh evidence.

Reviewer `/root/payload_final_source_review` is separate from the contract,
prerequisite, implementation and independent-control authors. This phase reads
actual code, retained pinned primary rules, literal fixture bytes and retained
coordinator execution. The reviewer performs no Cargo, target run, production
edit or heavy workload. The root-authored four-case probe is an executed
sensitivity demonstration, not a replacement for the independently authored
acceptance lane or a universal meshopt oracle.

## Authority and consumed invariants

The completed precode contract SHA256
`3cb5ad341a73cdda77a9dd4a1925cf9800af6e37e49b902eb6db0c4f0c3ee1e3`
preserves the complete bounded C1 profile, all unused accessors, meshopt NONE and
required placeholders. Existing code is a candidate to retain; independent
format rules and explicit profile choices govern acceptance. Unproved behavior
is not grandfathered. The prerequisite approval SHA256
`d94d50b8f53fc81da6532053e1ba29258e7846ca421e62791f0592cbd42bc15f`
conditionally retains checked algorithms subject to independent proof. No
approved exclusion narrows meshopt to one accessor element layout per codec
group, and no new geometry profile is authorized.

The selected published glTF 2.0.1 source revision is
`8e798b02d254cea97659a333cfcb20875b62bdd4`. Retained
`pinned-Specification.adoc` is 148,064 bytes, SHA256
`10aebb6a8362a155b196448912bf6250bf046f150431fa73c6cd657c44185aff`.
Its accessor/data-alignment rules say absent bufferView.byteStride means tightly
packed accessor elements. The shared-view byteStride requirement applies to two
or more vertex attribute accessors. This fixture has only one vertex attribute
accessor; the new SCALAR accessor is unused. Its own count/element size, offset,
alignment, finite values and actual range still require inspection.

Selected `EXT_meshopt_compression` revision is
`5ccac1cb0f85197c4f78c3ddff0b12cdccd79da3`. Retained primary is 35,576 bytes,
SHA256 `602608cb4e3402f189e8c4b235be680093ea07c68aff87ab5d974992283555b0`.
The extension works at bufferView level and is agnostic of the data's use. Its
layout invariants require parent byteStride agreement **when that parent field
is defined**, and parent byteLength equal codec byteStride times codec count.
After decompression, referencing accessors interpret the resulting view under
their own layout. A codec group is not a universal accessor element.

The independently sourced upstream golden pair remains exact: 85 compressed
bytes SHA256 `c1c311df6033a0900eb605c2ad3e5ca631e076f7cf120595fd41d4ba157eab6a`,
48 decoded bytes SHA256
`820994b9ed8bf83e8c38f9f6ce0c6b0afb889ecb5e07f4e580add9ae462eab89`.
The reviewer verifies these literal sources and uses independent little-endian
FLOAT decoding to confirm all twelve packed scalar values are finite. No new
network retrieval or execution of the codec is claimed by the reviewer.

## Actual defect and sensitivity

The coordinator's receipt binds original source
`2ebfb749e9d71fdb72507fd1203472b226f23156` and frozen portable executable SHA256
`104a60c7ac278f6cc6932b560136be0f63aa6b7cc8464e6cf64d9b4f122a07f2`.
The reviewer rehashes that executable and all four archives, decodes the retained
stdout/stderr strictly from base64, checks the member bytes against the golden,
and reads every literal document. Outcomes are:

| Control | Independent disposition | Actual 2eb outcome |
| --- | --- | --- |
| Absent parent stride, original VEC3 FLOAT count 4 | Admit | Admit, exit 0 |
| Same document plus unused packed FLOAT SCALAR count 12 | Admit | InvalidInput, exit 3: accessor/codec stride equality |
| Explicit parent stride 12, unused SCALAR count 4 | Admit | Admit, exit 0 |
| Absent parent stride, SCALAR count 13 | InvalidInput: actual range exceeds view | InvalidInput, exit 3: actual range |

Codec stride 12 times codec count 4 gives exactly the 48-byte parent view.
Without parent byteStride, the unused scalar's element size/effective stride is
4; count 12 occupies exactly 48 bytes. Count 13 requires 52 bytes and is correctly
rejected by the ordinary range check. The POSITION accessor still has count 4,
shape VEC3, tightly packed stride 12 and independently correct declared bounds.
The second accessor is not another vertex attribute, and primitive/report counts
are unchanged except that successful accessors checked should increase from one
to two. Successful scalar-component work increases from 12 to 24, remaining far
below the selected default ceilings.

The actual owner first checks decoded view length and any defined parent stride
agreement correctly. It then independently computes ordinary accessor stride
from parent stride or element size and checks its actual range. The additional
`plans[v].meshopt_stride != stride` rejection at payload.rs:733 incorrectly
equates codec grouping with accessor element layout. Source inspection confirms
the same rule in baseline `0eebdeebf013591460e6ac77485ad91b7401aada` at the former
validator owner. Historical parity cannot justify it. Relabeling this admitted
input Unsupported or designing a future codec to avoid it would not satisfy the
contract.

The original probe's separate lossless index hashes to
`78cb677034fa90ed4f528a15906bc08dd8be90e89bcb8347074ab01c2faf0f0c`.
All thirteen gzip records (four archives, eight original output streams and the
receipt) match compressed/expanded lengths, hashes and original bytes. This
retains the executed defect before any owner correction.

## Permitted minimal owner correction

Remove the per-accessor meshopt codec-stride equality rejection and the now-unused
ViewPlan meshopt_stride field, initializer and assignment. Preserve the earlier
defined-parent-stride agreement and decoded-length equality exactly. Keep normal
accessor stride selection, alignment, range, finite-value/bounds checks, unused
accessor scanning, component counters and all resource admissions unchanged.
Do not infer accessor stride from meshopt grouping, add a profile exclusion,
change codec mode/filter support, alter fallback roles or rewrite frozen inputs.
This narrow correction is authorized by the settled byte-meaning ownership and
primary rules; it creates no public context/limit/policy option.

The independently authored additive lane must retain the original four execution
records and prepare sensitive corrected controls from literal/golden bytes. It
must include absent-parent equality admission, the real one-over accessor-range
negative, and a defined parent stride disagreement negative that still reaches
the existing parent gate. Positives must assert the complete report, selected
limits, resource/cache counts and archive immutability; raw/scalar interpretation
must not use target output as its oracle.

Before a new bounded local or merge decision, bind the corrected source and
selected inputs separately, review the exact minimal diff and additive input
independence, and supply completed corrected library/context/private/public
controls with frozen artifact/log identities. Recheck applicable native Clippy,
portable/native C1/archive/resource/producer records and corrected extracted
source-package compilation against the new source. Exact-head remote CI,
wheel/installed-wheel and official Blender/platform gates remain required.
No existing 2eb or 2ef artifact/run may be rebound to the correction, and this
adjudication provides no A2, release or wider issue acceptance.
