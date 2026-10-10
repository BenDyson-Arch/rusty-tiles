# Post-F0 foundation scopes and disposition

Tracking gate: [#113](https://github.com/BenDyson-Arch/rusty-tiles/issues/113).
Audit baseline: `d69ba2a7facb99b5b3f5a6efba4e4039bef26be5`, 2026-10-09.
Three GPT-6.1 Sol agents reviewed API/contracts, ownership/resources, and
independent evidence. The coordinator reviewed their findings, reproduced the
validator observations and consolidated the scopes below. The source reviews
are not complete call graphs or whole-library correctness certification.

The current implementation must earn retention against independently specified
contracts. A component with useful bounded evidence may retain that evidence;
this does not accept its entire subsystem. New code has the same obligation.
These issues record work still required; creating them does not complete it.
No scope here unblocks 0.4.0, changes `main`, or authorizes publication.

## F0 acceptance finding

The final review found a high-severity destination-binding defect in packaging:
read-only resolution checked an absolute output path, but staging/publication
used the caller's original relative path. An observer changing process CWD could
redirect `Replace` into a selected source. An isolated public Rust reproduction
returned success while replacing that source with ZIP bytes. The focused fix
retains the checked resolved path for all destination side effects and the
returned output identity. The subprocess regression avoids mutating the CWD of
parallel tests. This is a required F0 fix, not a new mesh feature.

See [F0 contract](f0-implementation.md), [acceptance evidence](f0-evidence.md)
and [final review](../../bench/architecture_audit/post_f0/f0-acceptance-review.md).
The earlier agents' statements that their narrower audits found no F0 blocker
precede this targeted review; they are not contrary evidence. The fixed head `dd0e0af` passed all active PR and wheel/Blender checks and merged
in #116 at `519e8c1`. The merged tree exactly matches the accepted tree. This
acceptance is bounded by the F0 contract; all later scopes below remain open.

## Ordered work and dependencies

| Scope | Concrete deliverable | Prerequisites / stop boundary |
| --- | --- | --- |
| [#117 F1 mesh foundation](https://github.com/BenDyson-Arch/rusty-tiles/issues/117) | F1a: explicit local static GLB -> real multileaf mesh -> explicit tileset -> archive through Rust/CLI/Python | Accepted F0 and the local coordinate/format oracle subset from #120/#125. Stop at the declared profile; do not call all mesh migrated |
| [#125 format/validator proof](https://github.com/BenDyson-Arch/rusty-tiles/issues/125) | Independent payload/resource/hierarchy corruption controls and truthful validator claims | Start alongside F1a design; its minimal decoder/oracle subset precedes mesh acceptance. Full validator rewrite need not precede every independent oracle |
| [#120 spatial foundation](https://github.com/BenDyson-Arch/rusty-tiles/issues/120) | Domain/accuracy matrix and evidence for transforms, placement, bounds and numerical primitives | Local transforms/bounds needed by F1a first; horizontal/native breadth follows without guessing CRS |
| [#118 directory publication](https://github.com/BenDyson-Arch/rusty-tiles/issues/118) | D1: true no-clobber with one directory consumer; D2: typed replacement/recovery | F0 lifecycle. Independent of archive-only F1a; precedes raster/terrain safety claims |
| [#119 vector acceptance](https://github.com/BenDyson-Arch/rusty-tiles/issues/119) | V1: portable feature acceptance delta and typed rejection/fatal boundary; V2: native/accepted-content integration | F0 and one actual vector archive consumer. Does not certify geometry algorithms |
| [#121 mesh fidelity](https://github.com/BenDyson-Arch/rusty-tiles/issues/121) | Textures/materials, resource closure, approximation/LOD and optimization support decisions | F1a ownership plus relevant #120/#125 contracts; no silent waiver of current use cases |
| [#122 point-cloud proof](https://github.com/BenDyson-Arch/rusty-tiles/issues/122) | Required source fields/points, placement, sampling/hierarchy and bounded execution | F0 and relevant spatial/format contracts. Source enumeration must detect omitted attributes |
| [#123 vector fidelity](https://github.com/BenDyson-Arch/rusty-tiles/issues/123) | Topology, identities/properties, repair, LOD and reuse/resource closure | Integrates #119 with #120/#125. Native/portable agreement alone is insufficient |
| [#126 remaining operations/API](https://github.com/BenDyson-Arch/rusty-tiles/issues/126) | Wrapping/createTilesetJson, convertImplicit, in-place transforms, support tools and public building-block disposition | Inventory alongside F1a; one operation at a time, with F0/D and relevant format/numerical evidence |
| [#124 raster/terrain/native proof](https://github.com/BenDyson-Arch/rusty-tiles/issues/124) | Analytic source-data fidelity, native lifetimes/resource limits and safe real directory consumer | #118 and relevant numerical/format contracts. GDAL-free raster remains separate #82 |

Contract/oracle work can run in parallel. Implementation remains bounded by
these dependencies, with a concrete real consumer and stop condition per slice.
A broad layout move or a growing generic framework is not a substitute. D1 and
a minimal raster/terrain pilot from #124 develop together; only the broader
raster/terrain acceptance depends on completed directory proof, avoiding a
prerequisite cycle.

## F1a: deliberately bounded first mesh milestone

The first profile is an explicitly interpreted local-metre/Y-up static embedded
GLB, triangles, POSITION/optional NORMAL and a declared untextured core material
subset. It performs real geometry processing and a forced multileaf case; simply
wrapping the input does not demonstrate a mesh producer. Every accepted triangle
and supported attribute/material meaning must survive within declared storage
tolerances. There is no automatic CRS inference, small-file eligibility bypass,
parent approximation, atlas, external encoder or directory publisher in F1a.

The API/ownership audit recommended this narrower first profile; the evidence
agent also described a broader mesh milestone including UV/texture coverage.
The consolidation chooses the narrow profile to isolate producer ownership and
independent leaf fidelity. Texture/material breadth remains required follow-on
work in #121, not an implicit removal or a hidden fallback to old behavior.

Before coding #117, finish its finite contract decisions: exact source allowlist,
default-or-unambiguous-sole scene selection, node transform/reflection rules,
accepted material factors, coordinate/storage tolerance and magnitude domain,
resource admission limits and a positive explicit leaf-triangle limit. That
limit is neither a memory limit nor a geometric-error promise. A convenience
default needs a measured domain decision, shared by all adapters. Unsupported
semantics are refused consistently, independent of source size or leaf count.

Private consumer stages may be `ValidatedMesh`, `PreparedMesh` and
`CompletedMesh`; these are not a public typestate framework. Read-only preparation
resolves metadata and capabilities, binds source/destination identities and
owns no final-output staging. Execution owns materialization or spooling,
workers, scratch and accepted member records. All required report, producer and
observer work finishes before seal. A typed mesh report is prepared once and
serialized before publication; no fallible required postcommit work remains.

### One run, clean dependencies

`package()` begins its own single-use control. Mesh must not begin a run and then
call public packaging with that control, or start a second control to evade reuse
checks. Compose the private archive codec and runtime under the existing mesh
attempt. Extract a private prepared-member/existing-attempt helper only if actual
duplication warrants it; it must not import opaque-source selection/report policy
or make the codec depend on jobs.

Keep one crate initially. The concrete direction is:

- Adapters -> mesh facade and domain request; adapters own syntax/presentation.
- Mesh orchestration -> source preparation, geometry/texture/format operations;
  orchestration owns workspace, worker lifetime, metrics and accepted inventory.
- Mesh/package orchestration -> archive codec and runtime as separate services.
- Runtime owns no scene, converter identity, CRS, report schema or skip policy.
- Codecs own no final destination, worker pool, frontend or feature acceptance.

Use module privacy and a small external facade consumer to enforce actual seams.
Do not invent a universal context, registry, converter trait, error whitelist,
platform-policy table or custom dependency analyser. Physical layout changes
follow ownership changes; no subsystem earns retention by being moved.

### Acceptance beyond matching old output

F1a needs independent raw-input/output decoding for transformed positions,
triangle multiplicity/winding, normals, material factors and bounds. Deliberate
corruptions must show those oracles fail. Generated member inventory must match
an independent reference walk, excluding scratch and rejected candidates. The
forced multileaf hierarchy needs explicitly justified refinement/error behavior.

Inject decode/encode/spill/report/archive/finalizer/observer failures; cancellation
and first-cause handling span the same run. Join work before sealing. Test callback
reentry, concurrent/repeated runs and stable worker-result acceptance. Measure
geometry/member/worker scaling, descriptors and scratch on both success and
failure. Image/atlas scaling belongs to the textured follow-on in #121. Materializing a declared bounded profile may be acceptable, but tile or
batch counts alone do not establish a byte/RSS bound.

F1a completion does not settle textures, placement/horizontal CRS, external glTF
resources, LOD/implicit hierarchy/metadata or optional codecs. Before release,
each advertised current use case needs verified replacement/retention or an
explicit documented unsupported/removal decision. A narrow milestone does not
authorize silently dropping those use cases or indefinitely routing them through
unproven legacy exceptions.

## Directory and vector ownership decisions

Directory CreateNew must use a proven no-replace primitive, not check-then-rename.
Replacement has distinct hold-old/install/restore states; choose the supported
platform/mode guarantees and visibility interval before implementation. Failed
restoration returns a typed previous-output recovery location. Backup cleanup
cannot erase the only surviving original or rewrite a committed outcome.
Competitors inserted after hold remain protected; Replace explicitly authorizes
retiring the entry present at hold, not a preparation-time snapshot. No process
mutex implies universal writer serialization. D1/D2 require an actual consumer
and OS evidence. D1 is accepted in #129; the
[D2 contract](d2-directory-contract.md) specifies current-at-hold replacement,
conditional restoration and machine-readable recovery.

Vector acceptance owns all effects of a feature: SQL/fragments, counters,
success diagnostics and any mutable frame/cache/identity contribution. Readers
return typed rejection or fatal failure; only the former can be skipped.
Source/storage/report/rollback/release errors abort. Decide whether a frame comes
from source preparation or first accepted geometry; a rejected first feature must
not accidentally choose it. Stage an accepted delta and merge it only when the
feature is accepted. A failure committing required state aborts the job. V1
starts portable; V2 must carry the same contract through native/reuse paths.

The natural rejected-polygon/report mismatch and injected I/O classification
failure remain confirmed evidence. The old candidate-orphan suspicion was
withdrawn after cleanup inspection and controls; this audit does not resurrect it.

## New observations and oracle limits

The [replay evidence](../../bench/architecture_audit/post_f0/README.md) records:

- **Missing GLB payload:** `validate` accepts a POSITION bufferView declaring 36
  bytes with an actual zero-byte BIN. This is a payload-check coverage gap. F0
  opaque packaging correctly preserves these bytes and does not certify them.
- **Content outside bounds:** geometry >=1000 passes a box near the origin. The
  CLI documents child-bound containment, not decoded-content containment, so this
  is not automatically a violation of its narrower documented contract. It does
  prove that success cannot serve as F1's content-bound oracle. Check labels and
  release claims must be precise.
- **Sampled LOD estimate:** a direct constructed parent/child call returns 0 m
  despite a 100 m off-sample spike. It disproves a universal upper-bound claim for
  that estimator; it does not establish an end-to-end converter underbound. Decide
  the geometricError contract rather than calling a sampled estimate a bound.
- **Point field completeness:** a large audit checks output schema fields and can
  miss an omitted source field. Stronger small tests independently enumerate source
  dimensions. Retain the stronger pattern and add omission/incorrect-POSITION
  negative controls before extending large-dataset claims.

Frozen analytic/PROJ references, small decoded fidelity fixtures and independent
ZIP/subtree readers offer bounded useful evidence. Several historical Python
encoders share native kernels or design lineage; differential agreement alone
cannot establish semantic correctness. Reports identify these distinctions per
component. No whole subsystem receives unconditional retention.

## Release gate and next action

F0, bounded F1a, D1/D2 and vector V1/V2 are accepted through #116, #128,
#129/#130 and #131. Vector merge `5df15e7` passed final-head CI, installed-wheel
and official-Blender checks; independent Sol 6.1 review replayed 27 portable and
30 native acceptance cases. This is lifecycle/feature acceptance, not full
vector geometry/CRS/LOD certification.

The current implementation candidate is [#132 P1](https://github.com/BenDyson-Arch/rusty-tiles/issues/132):
a real LAS/LAZ archive consumer under one F0 run, with explicit coordinate/source
preparation, independent record/attribute/POSITION multiplicity, accepted member
receipts and required reports, then deletion of the replaced point-cloud paths.
The [P1 contract](point-acceptance-contract.md) defines the finite profile.
Production `12d98ab` has completed local portable/native, independent decoded
LAS/LAZ, lifecycle, installed-wheel and resource checks; the
[evidence ledger](../../bench/architecture_audit/point_acceptance/README.md)
records exact identities and remaining CI/release gates.

[#133 C1](https://github.com/BenDyson-Arch/rusty-tiles/issues/133) can proceed in
parallel: actual GLB payload/accessor/index ranges and truthful validator claims.
The [#126 surface inventory](public-surface-inventory.md) records intentional
facades, incidental exports and migration/removal owners. Neither a shared
production validator nor a file move substitutes for independent acceptance.

### Current mesh follow-on after C1

C1 merged through #137. The first bounded #121 slice is
[F1b1](f1b-contract.md): the existing local mesh producer now carries embedded
PNG/JPEG base-color textures, complete UV/material/sampler associations, and a
shared image closure under F0. It forwards source bytes without invoking legacy
atlas, LOD or encoder paths. This deliberately advances the existing operation
and report profile rather than maintaining an F1a compatibility mode.

[F1b2](f1b2-contract.md) merged through #139 with bounded local glTF/GLB
resource binding, source stability, a pure decoder and consumer-owned capture.
[F1b3](f1b3-contract.md) merged through #141 with five core PBR texture bindings
and authored UV1/tangent/color companions, exact resource closure and the bounded
tangent-frame bake profile.

The current mesh implementation candidate is [F1c1 #142](f1c1-contract.md):
explicit Local/Wgs84 placement of that local metre/Y-up source, one normalized
ENU quaternion and post-node Y-up metre offset, and one f64 rigid root transform.
Pure resolution precedes source I/O; PreparedMesh owns the resolved map and one
format writer consumes it. Schema 4/profile `f1c1-placed-gltf-v1` records the
source/output frames, normalized parameters and exact emitted matrix. Independent
world-position/bounds/consumer, frontend and final-platform evidence plus separate
nonauthor review remain acceptance gates; implementation alone does not pass them.

F1c2 will implement source identity/metadata/picking. In the current admitted
forest, source node index identifies an instance; mesh/primitive indices and
original TRIANGLES ordinal identify its authored triangle association within
the captured document. Preserve those keys through partition and regrouping;
do not derive identity from names, output leaves or material grouping. Later
approximation needs proved associations for geometry combining multiple source
objects. F1c1 adds no unused metadata/provenance framework or picking claim.

Independently justified coarse approximation/error follows identity, then
implicit delivery follows accepted geometry and resource contracts. General
source CRS/axes/E/N/A shifts and vertical/epoch/grid operations require separate
admission evidence; manual WGS84 placement does not prove them. These remain
#121/#120/#125 obligations. Broader legacy mesh/report routes stay advertised
pending explicit supported-use replacement/removal decisions, with wrapping,
implicit rewriting and in-place operations still under #126. This bounded slice
does not close those parent gates or authorize release.

After P1 acceptance, scope terrain's real directory migration under #124 using
D1/D2, then broader mesh/resources/LOD under #121 and remaining utility operations
under #126. #120 supplies each needed numerical subset; #123 and the remaining
#125 format work stay open. #82 informs raster/terrain boundaries without making
GDAL replacement a prerequisite. Each implementation has a bounded audit,
independent final review and relevant final-candidate checks.

Issue #126 owns the otherwise uncovered operation/API inventory: wrapping and
createTilesetJson, convertImplicit, in-place compression, preview/doctor/fixtures,
and public metadata/implicit/grid/math/mesh/HLOD utilities. Format proof alone
does not migrate their orchestration or establish their public contracts.

Before closing #113, all advertised operations must have explicit supported
contracts, ownership and evidence or deliberate documented scope changes. Then
rerun the redesigned candidate's portable/native, installed wheel, official
Blender, strict browser and resource acceptance. Historical release evidence,
this fan-out, and F0 by themselves do not satisfy that gate. 3D Tiles 2.0 features
remain in their existing roadmap, downstream of the foundations.

## Current next operation: #126 A2 after #147

F1c1 merged through #143 and F1c2/W1/package facade removal merged through
#147 at `e3d222a4c27a86e1f06e1e4b47db4e6a4fa5e24e`. The latter's accepted
head passed main CI, all five installed-wheel platforms and all four pinned
official Blender platforms. Those bounded migrations do not accept broader
mesh approximation, legacy raster pyramid or implicit rewrite semantics.

User directed continued work on #126. Three Sol 6.1 audits cover the current
explicit-to-implicit operation's ownership/representation, independent
availability/resource evidence and lossless audit-storage compaction. The
[implicit rewrite contract draft](implicit-rewrite-contract.md) records the
next real operation and its unresolved proof gates. Existing checks share the
production implicit expander and cannot independently establish semantic
metadata or availability correctness. Source provenance labels cannot supply
topology, coordinate or padding truth. A replacement must resolve those owners
before adapting Rust/CLI/Python to one F0 run and deleting the legacy family.

This operation rewrites eligible point/vector archives. It is independent of
future mesh implicit delivery, which still follows justified approximation
under #121. Existing retained point/vector use cases, including padded cells,
translations and content arrays, need evidence-backed support or explicit
disposition; an exact-cell pilot alone does not replace the advertised route.
Parent #113/#120/#121/#125/#126 and release gates remain open.
