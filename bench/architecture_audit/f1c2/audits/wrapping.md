# W1 preimplementation wrapping/manifest audit

Baseline: merged F1c1 #143, `29b96e501b1426e08cf7d3f01ba93964c6618a57`.
Owner: #145 under #126/#120/#125/#113. Audit is read-only source review;
the contract was drafted before production implementation. No Cargo, builds,
defect probes or GitHub writes were performed by this auditor. Source locations
below identify static observations, not executed defect claims.

Contract: [model-wrapping-contract.md](../../../../docs/architecture/model-wrapping-contract.md).

## Static findings and dispositions

| Component | Observation at baseline | Decision / proof before implementation |
| --- | --- | --- |
| `src/tileset.rs:29-82` direct writer | Checks output then fs::write publishes directly; emitted URIs derive from source basename/root rather than actual final manifest base. | Replace with completed F0 file publication and explicit product/resource authority, or deliberately retire. Proposed retained product is one model and fixed sibling tileset.json. Coordinator selected this concrete replacement before production implementation. |
| `src/tileset.rs:93-121` collection | WalkDir errors are dropped and path names use lossy conversion; file/directory and multiple-model products share implicit policy. | Remove directory/multiple-model enumeration from W1; name lost capability and migration. No generic collection replacement required by single-model wrapper. |
| `src/tileset.rs:17-20,53-72` errors/hierarchy | 512/4096 constants come from old tool comparison; one/many content shape changes error values. | Replace W1 with one full-detail root. The initial zero top-level error design failed independent actual-consumer review; corrected contract keeps root error zero and defines a separate positive top-level omission/visibility error. No approximation bound is inferred. |
| `src/tileset.rs:143-165` wrapping | Legacy Job begins before resource resolution; capture/resource admission and bounds read source through different owners. | Replace operation with one Attempt, read-only admitted immutable capture, prepared inventory and required report before staging/commit. |
| `src/tileset/resources.rs:18-80` resources | Reads live mmap/document; enumerates core buffer/image URIs and structural schemaUri only; returns source paths to pack. | Replace W1 use with accepted binding/source owner. Arbitrary extension-owned dependencies remain excluded, not proved by preserved JSON. Preserve every admitted URI-to-member association, including aliases sharing capture bytes. |
| `src/bbox.rs:15-24,108-128` bounds | Parses separately, may import buffers on fallback, uses f32 node matrices, may trust accessor min/max rather than decoded coordinates. | Do not retain at W1 boundary. Use one independently specified original-node evaluator over admitted captured bytes. Retiled f32-bound evidence is insufficient. |
| `src/mesh_archive/source.rs:621-716` source TRS | Admits near-unit quaternion norm within 1e-6 then normalizes; unchanged source consumers retain raw node TRS. | Original-node outward raw/normalized quaternion interval evaluator was selected after focused independent Decimal feasibility probes; document.instances alone cannot establish unchanged-content bounds. Conservatively enclose authored/normalized interpretations and reject conservative endpoints outside the placement local magnitude profile. |
| `src/mesh_archive/binding.rs:35-67,552-655` capture | Owns root and per-request immutable resources; detects aliases and shares Arc byte owners, but Snapshot does not preserve exposed member spellings. | Retain only accepted capture domain; minimally extend private result to expose root bytes and decoded per-request member associations. Root coordinates file ownership. Do not rediscover URIs or reopen source during encoding. |
| `src/mesh_archive/placement.rs` | Pure resolved placement supplies actual matrix/report and F1c1 finite numerical profile. | Retain bounded owner for W1; share private visibility, not duplicated formulas. Original local bound evaluation must fit its magnitude assumptions. |
| `src/validate.rs:114-117` URI admission | Current local URI validation rejects percent escapes; exact wrapper source bytes can contain admitted percent-encoded resource paths previously rewritten by mesh encoding. | Owning C1 URI reader must gain independently proved admitted decoding, or wrapper exclusion must be deliberate and documented. No fake green validation from source rewrite or bypass. |
| `src/tile.rs:226-234`, `tileset.rs:156-195` shortcuts | Old mesh uses glb_job/implicit_glb_job and legacy Job/options for small unchanged sources. | Record exact residual private ownership. Migrating public wrapping cannot delete or certify these internal paths; no W1 fallback from old mesh. |
| CLI/Python/public facade | Main imports CreateTilesetOptions and direct/wrapping entry points; Python glb_to_3tz uses run_conversion and ConversionResult. | Coordinator migrates adapters/exports to operation-specific result/JobFailure. Remove dead public compatibility aliases; broader mesh/implicit/raster consumers keep their own legacy gate. |

## Capability reductions requiring explicit publication in migration guidance

The old wrapper is advertised as the fallback for animation/skins, unknown
source extensions, additional attributes, broader PBR/samplers and UV cases
in `src/tile.rs:1058-1100`. Existing docs also advertise data-URI sources and directory/multiple-model
createTilesetJson, large unchanged models and legacy Euler placement.

W1 explicitly admits the finite static core source domain, not every historical
escape-hatch use. Animation needs time-dependent conservative bounds; extension
bytes need their own dependency and meaning admission; imported feature metadata
is not F1c2 authored metadata. Every excluded use must be stated in help/docs and
the surface inventory when the old public wrapping publisher is removed.
Existing broader mesh use cases remain advertised pending their own decisions.

## Proposed representation and sharing

Wrapper owns `PreparedModel { snapshot, resolved_placement, original_bounds,
member_inventory, report }` and private completed captured-member artifacts.
These are operation implementation values, not public IR/typestate machinery.
The binding/source/placement modules remain concrete shared owners; expose only
the private methods needed by the two real consumers. Bound per-name emitted
payload at 64 MiB in addition to unique capture admission, because sharing one
captured file does not make distinct archive member copies free.

One-content manifest serialization can serve archive W1 and a retained
single-model sibling-manifest producer. Do not make either facade invoke the
other or public package; both compose F0 under their existing Attempt. If the
standalone product is retired, no unused manifest request/result is added.

## Unresolved proof obligations and focused probes

1. Complete final original-node acceptance: the chosen source-owned interval
   evaluator covers unchanged matrices/TRS, raw/normalized quaternion evaluation,
   outward composition/point enclosure, and wrapper endpoint admission ±1e6. Compare mixed magnitude,
   nested transform and near-unit quaternion cases against independent Decimal
   or frozen primary-consumer references. A single final-point next_up is not
   an established enclosure of accumulated rounding.
2. Prove decoded resource URI/member mapping without sharing a buggy test
   oracle. Test `%20`, encoded percent, non-ASCII UTF-8, literal dot segments,
   encoded separators/escape refusals, repeated resolved names, hard-link alias
   names, synthetic root collisions and emitted alias-byte budget boundaries.
   A control removes one alias while preserving the unique file capture.
3. The coordinator chose typed single-model sibling publication. Prove
   model/resource authority remains valid relative to final tileset.json.
   Source stability is an explicit precondition throughout the entire call,
   including observer callbacks, and throughout continued manifest use; it is
   not an atomic live-resource snapshot. Publish directory/multiple-model/
   arbitrary-output retirement and concrete W1/package guidance.
4. Independently compare archived root and every dependency against captured
   bytes, including unused declared resources; verify archive closure and model
   content without importing producer decoder or member-name helpers.
5. Exercise actual operation failure/cancellation/callback/replacement/concurrent
   writers/source aliases/CWD relocation/cleanup; freeze final identities and
   obtain separate nonauthor source/evidence review. Coordinator alone builds.

Acceptance records will identify actual executed results later. This document
does not accept a numerical evaluator, retained standalone product, wrapper
implementation, browser behavior, platform behavior, broad legacy deletion or
release readiness merely because their designs are listed here.

## Implementation handoff and remaining acceptance

The concrete W1 producer is mesh_archive/model.rs, reusing the accepted binding,
pure source and resolved placement owners. Binding exposes immutable per-request
member associations; the wrapper resolves its owned Arc inventory once before
materialization. Source original_bounds uses directed interval arithmetic over
authored node chains before f32 baking. Old public wrapping options/functions
are removed; tileset.rs remains a private legacy mesh shortcut island.

Known Rust/CLI resource, golden and user-model tests now target W1's finite
contract. Fixed-error/Euler npx equality tests were deliberately retired;
frozen bench/architecture_audit/api probe sources remain historical baseline
records, not runnable new-public-API consumers or retention evidence. Existing
legacy mesh small-input resource tests remain distinct regression cases.

New independent entry point: `python3 tests/model_wrapping_oracle.py --binary
<BINARY> --json-output <EVIDENCE>`. It authors its own GLB/glTF, decodes original
source buffers and raw/normalized TRS in Decimal80, independently maps archive
URIs and compares bytes/counts/world placement, and demands sensitive-control
failures. New public-consumer lifecycle/resource cases are in
`tests/model_wrapping.rs`; persistent partial writer and required report/archive/
cleanup failure injection lives beside the producer. Their existence alone is
not an executed acceptance result. Coordinator owns all Cargo builds and the
final evidence/CI record; final nonauthor review remains required.

## Executed portable W1 evidence

On 2026-10-10 the author ran the stdlib independent reader against the
coordinator's frozen portable binary, SHA256
`d461c38d88cdab3ae643a16bec8b261c982940b9392a362d93fee9986c7c9bbb`.
The [receipt](../model-wrapping-oracle.json) binds the executed oracle SHA256
`3c3b9235f6447ac614a609764a2c9c9e76c615ea38be300506983d255502d707`
and verifies binary identity unchanged across execution. This execution passed
15 Decimal80 authored-node/placement combinations and one external-resource
association case. The largest independently measured world-coordinate
difference was approximately `4.116e-10` metres, within the declared `1e-6`
allowance. Raw and normalized source quaternion interpretations both enclosed
all selected source points.

The external case proved exact root/buffer/image bytes, percent-escaped UTF-8,
spaces and percent characters, dot normalization, distinct hard-link member
names, unique capture counts and emitted alias-byte counts. Standalone sibling
URI authority, source preservation and zero copied-payload reporting passed.
Deliberately shrunken boxes, wrong placement matrices and an omitted hard-link
alias were detected. Generated source-name collisions, data URIs, escapes,
encoded separators, animations and imported extensions produced their declared
refusals; retired HPR and arbitrary manifest output syntax were refused.

The initial reader run exposed a reader-only assumption that the CLI nested
the model report at `report`; the actual declared frontend field is
`modelReport`. Correcting that assumption and rerunning produced this complete
receipt. It did not identify a production defect. These author executions do
not replace final nonauthor review, browser appearance/picking, platform/wheel
checks, finite-resource/lifecycle Rust tests or broader legacy removal gates.

## Independent recovery-review corrections

The first frozen candidate passed byte/bounds oracles but failed ordinary Cesium
traversal: top-level geometricError zero caused no root visits or content requests.
The primary 3D Tiles specification distinguishes top-level omission error from
root refinement error. The corrected contract retains root error zero, requires
root refine REPLACE, and uses a positive conservative emitted-box diagonal
(minimum one metre) for top-level selection. This is a declared visibility
policy, not an approximation or universal visual-error bound. A sensitive control
sets only the top-level error to zero. Final corrected-candidate execution must
replace any claim inferred from the first candidate's numerical checks alone.

Review also reproduced a generated-root ancestor collision misclassified as Io
after creating an output parent, and a CLI manifest summary advertising an
impossible conversion.json path. Both require pre-work collision refusal and
honest inline-only report presentation respectively. Old failing receipts remain
pinned separately from corrected-candidate acceptance.
