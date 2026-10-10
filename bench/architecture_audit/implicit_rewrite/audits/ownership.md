# A2 implicit rewrite ownership audit

Read-only source audit against
`e3d222a4c27a86e1f06e1e4b47db4e6a4fa5e24e`, after PR #147. No Cargo/build,
production execution, fixture regeneration, Git mutation or external message was
performed. These findings are static observations and proof obligations, not
newly reproduced defects. The
[draft contract](../../../../docs/architecture/implicit-rewrite-contract.md)
settles ownership while leaving semantic admission unproved.

## Concrete observations

| Source ranges | Observation and implication |
| --- | --- |
| `src/convert_implicit.rs:11-15,44-57` | Legacy force option, unit/report entrypoints, stderr default and generic ConversionResult. No owned operation request/report/control. |
| `src/main.rs:1127-1133`; `bindings/python/src/lib.rs:932-948`; `src/lib.rs:44-46` | Actual CLI, Python and root export callers still use that family. Python uses run_conversion with no callback, rather than run_job. |
| `src/convert_implicit.rs:58-70,143-145` | Source/output preflight, archive codec validation, ZIP open and later C1 inspection are separate pathname reads. There is no one captured archive/source identity shared by semantic admission and extraction. Canonical path equality is not a hardlink identity contract. |
| `src/convert_implicit.rs:25-32,68-70` | ZIP is allocated and initial documents parsed before bounded C1 admission. The local JSON limit is 64 MiB, with no cumulative parse/member/copy ceiling. Every by_name error becomes a missing-member Data error, including possible I/O. |
| `src/convert_implicit.rs:89-119`; `src/point_cloud.rs:75-94,628-640`; `src/vector/pipeline/reuse.rs:44,250` | Report encoder labels and vector build state select scheme. Current explicit point output still supplies the expected label. Labels/hash checks prove provenance consistency at most, not hierarchy or payload accuracy. |
| `src/convert_implicit.rs:125-129,304-312` | Point rounding defaults to zero; vector minimum tolerance is 0.001; child rounding/quantization defaults to zero. No finite/nonnegative/bounded validation of padding sums. Source extras are not an independent rounding oracle; checked arithmetic/range policy is absent. |
| `src/convert_implicit.rs:209-238,283-333` | Local boxes and child translation syntax are checked, but cumulative coordinates and subdivision arithmetic use unconstrained finite f64 inputs. Child slots use recursively subdivided root-derived cells, and are written into `_rustyImplicitChildIndex` in imported JSON. |
| `src/convert_implicit.rs:422-442,447-455,485-498` | Each emitted root uses the original node's own local box, yet child addressing consumes the earlier global-cell slot. Admission and generation do not share one cell/frame plan. Metadata overrides might justify some differences, but require independent semantics and geometry proof. |
| `src/convert_implicit.rs:267-276,379-403` | Tile metadata, viewer request volumes and content bounding volumes are refused before staging; URI/type/alias eligibility is deferred into generation after staging. Not every admission failure is preparation-only. |
| `src/convert_implicit.rs:74-79,154-157,395-403,480-483` | Reserved prefix checks cover top-level `t/owned-`, not nested alias names or exact ancestor member `subtrees`. Nested exact aliases fail later via exists; an admitted source file ancestor can fail during mkdir. Full generated/source inventory must be planned before writing. These collision cases were not executed here. |
| `src/convert_implicit.rs:340-345,407-479` | Owned external roots deliberately keep parent payloads and replacement descendants together; JSON links occur only at terminal level one. This is a meaningful candidate semantic design, not accepted merely because comments explain it. |
| `src/convert_implicit.rs:393-404`; `tests/convert_implicit.rs:836-909` | Aliases retain the original payload parent directory, preserving relative resource bases. Original payload members are also retained, so alias bytes can multiply with repeated references. The test inspects exact nested payload/schema preservation. |
| `src/convert_implicit.rs:485-515` | Root/header copy preserves imported values without a finite extension/resource disposition. Asset version is copied, not resolved for generated implicit output; source admission/C1 accepts 1.0 or 1.1. A 1.0 explicit input therefore leaves a declaration proof gap for generated implicit content. External header extras are selectively removed, while broader header semantics are copied. |
| `src/convert_implicit.rs:181-191`; `src/validate.rs:826-839` | Report is raw JSON, claiming byte preservation and retaining source provenance. C1 knows this operation's retainedContentUris to mark duplicate originals reachable. New report/inventory semantics need an explicit independent inspector disposition; changing field spelling alone can break closure checks. |
| `src/convert_implicit.rs:195-206`; `src/output.rs:111-133` | A candidate is publicly packaged with fresh default RunControl, inspected, then a second archive is packed/published from the tree by legacy Job. Candidate validation is not tied to the final completed file; there is no operation-wide cancellation/observer admission gate. |
| `src/validate.rs:810-815,518-522`; `src/implicit/tileset.rs:604-624` | C1 explicitly excludes decoded content bounds, metadata semantics, geometric error accuracy and implicit addressing/availability. Production expansion is structural presentation, not proof of this rewrite. Old converter comments at120-121 and193-194 overstate the validator's current semantic coverage. |

## Tests, schemas and historical evidence

`tests/convert_implicit.rs:237-347,348-394,395-472,473-571` covers invented
point/vector conversions, payload/world observations, force/no-publication,
irregular/corrupt cases, deeper trees and padded cells. The geometry audit at
128-235 independently decodes GLB buffers and composes transforms, but obtains
implicit hierarchy through production expand_tileset at163-166 and213-215.
Consequently shared expansion/writer mistakes can pass its hierarchy comparison.

`tests/convert_implicit.rs:572-740` adds fragmented/quantized/compressed arrays,
GLB/b3dm byte retention and malformed content refusal, partly by editing a
current-producer fixture to the intended leaf shape. `:742-834` examines binary
availability/templates but still uses production expansion for structure.
`:836-909` checks nested payload-relative schemas. These are useful fixtures,
not independently defined operation admission or comprehensive failure/control
oracles. Existing refusal tests chiefly classify errors by message fragments.

`tests/implicit_subtree.rs:1-69,71-110` contains an independent byte/availability
decoder without writer internals or Morton library; this is a strong retention
candidate for the primitive oracle. `tests/implicit_metadata.rs:1-30` uses the
production writer/expander for metadata fixtures, requiring independent property
rank/frame/meaning checks before promoting it to semantic acceptance.

The bundled `docs/schema/tileset.schema.json:774-803,805-885` establishes property
shapes, mutual exclusions and minimum error, not refinement selection, decoded
bound accuracy or all version-dependent implicit restrictions. C1 uses it at
`src/validate.rs:816-819,487-499`. Primary specification/version pinning and an
independent semantic oracle are separate prerequisites.

`bench/convert_implicit_browser_results.json:1-52,977-1006` records historical
Cesium point/vector coarse/fine, placement and picking observations, exact
fixture hashes, and a sensitive previously failed sibling-proxy design. It
expressly limits coverage to invented regular fixtures and separates browser
behavior from specification conformance. No source revision binds those
observations to this audit's source. `tests/fixtures/convert_implicit.cjs:35-42,
62-80,101-122,129-136` checks actual selection and independently known source
positions, providing a candidate viewer oracle to rebind to final artifacts.
`tests/fixtures/implicit_validator.cjs:16-30` patches the upstream multiple-content
traverser at exact versions; disclose that workaround when using its receipts.

## Disposition ledger

| Component | Decision before implementation | Required proof/removal gate |
| --- | --- | --- |
| Legacy operation lifecycle, force/Reporter, public package candidate and double pack | Replace | One Attempt, one final candidate, explicit inventory, failure/cancellation/observer/source identity/publication probes. |
| Public old options and two entrypoints, CLI/Python routes | Remove after replacement domain is proven | Every advertised point/vector use case receives support/rewrite/retirement disposition; migrate actual callers and documentation; installed-wheel parity. |
| Mutable JSON hierarchy/check_tree sentinel indexes | Replace | Owned admitted source tree and per-emitted-root local child-slot/frame plan. |
| Provenance-driven scheme and permissive padding defaults | Replace policy ownership; domain choice unresolved | Explicit intent or independently justified inference; real producer padding/source accuracy evidence; finite checked arithmetic. |
| Owned-root terminal-link construction | Retain only as a candidate design; rewrite representation | Independent spec, transform/bounds/error/availability/refinement proof and viewer controls. |
| Same-parent byte aliases and exact original bytes | Retain only as a resource principle | Full closure/base/metadata equivalence, complete preplanned exact/ancestor collision checks and alias amplification bounds. |
| Subtree byte writer, availability/metadata structs and tile schema | Candidate bounded reuse, not broad retention | Independent bytes/ranks/schema/semantic evidence for only used levels/slots; no converter/path/job ownership. Other consumers prevent global removal. |
| Current C1 inspection | Retain as supplementary read-only structural/payload check | Never claim excluded semantics; prepare one captured input and tie candidate inspection to final file; explicit new report closure contract. |
| Original/header arbitrary JSON copying | Replace with admitted import/generation boundary | Unknown extension/metadata/group/statistic/resource meaning and version dispositions before lossless preservation claims. |
| Historical Rust/browser/upstream receipts | Retain as observations/fixture candidates | Rebind source/artifact identities, independent controls and finite actual producer profile; do not relabel historical success as A2 acceptance. |

## Blocking proof order

First settle primary implicit/metadata/external-link semantics and the shared
local frame/slot representation. Then compare exact independent fixtures with
actual current padded point/vector sources; resolve shape/subdivision/provenance
policy and every existing use case. Settle closure, alias/name collision and
header/version/metadata disposition. Finally settle bounded captured-source
admission and measure resource ceilings. Only then implement a typed production
vertical slice and run independent lifecycle/final-artifact/frontend acceptance.

No execution defect, A2 acceptance, universal implicit support, release closure
or permission to preserve/remove an unproved semantic path is claimed here.
