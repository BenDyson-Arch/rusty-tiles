# A2 initial imported metadata and provenance decision

2026-10-10, preparation only. Coordinator-declared baseline
`e4d90897518d2b569fb5e99b876581523e4c418c` in
`/tmp/rusty-tiles-a2-semantics-settlement`. This lane changes only its new audit
subtree. No Git, Cargo, production, converter, validator or browser execution.
The accepted [separate preparation review](../a2_review/review.md) is the
starting evidence, not acceptance of this new profile. Capture/name ceilings
retain their prior preparation status; this decision introduces no allocator,
worker or generic conversion framework.

[36 independently authored model cases](results.json) cover effective inherited
REPLACE, declaration promotion, inline payload metadata, application extras
presence/value, root-level exact source snapshots, fixed-name collisions,
historical vector associations and incoming replaced-control references.
The same small `nice -n 10` process separately reads all 14 final portable and
14 final native source fixtures, verifying archive and non-index member hashes
against their exact originating receipts. It creates no replacement archive.
24 fixture inspections pass this metadata/provenance model and four refuse
unknown extensions. This is field/resource inspection, not independent payload
schema/type/geometry or production admission acceptance.

## Finite initial source policy

Resolve effective source values once before planning. Each private admitted node
owns source identity/path, effective REPLACE, finite validated local bounds/error/
transform from the separate spatial owner, ordered content headers and an
optional owned JSON extras value. Children inherit REPLACE when refine is absent;
root refine must exist. Effective ADD is valid outside this profile: Unsupported.
Missing required root refine, an invalid enum, both content/contents or ambiguous
uri/url are InvalidInput. No generated tree or codec resolves inheritance again.
The pinned [core refinement rules](https://raw.githubusercontent.com/CesiumGS/3d-tiles/4d781014b52294759834018a931223b98ac1ce47/specification/README.adoc)
require root refinement and define descendant inheritance.

Admit declarations 1.0 and 1.1 only; every generated main/external manifest
declares 1.1. Original source declaration remains in its exact snapshot and
report. Promotion admits only separately validated content/feature profiles;
changing a declaration does not certify that previously mislabeled content was
valid 1.0. Legacy b3dm remains possible within the previously exercised trivial
BATCH_LENGTH-zero, empty-feature-binary/batch-table, no-RTC profile. Broader b3dm,
Draco, sparse/rotated payloads and alternate CRS remain separate gates. This is
a declaration migration, not a format repair. Version governs the source
schema/base formats in the [primary core specification](https://raw.githubusercontent.com/CesiumGS/3d-tiles/4d781014b52294759834018a931223b98ac1ce47/specification/README.adoc).

Preserve unchanged GLB/b3dm payload bytes and same-parent aliases. Retain the
independently exercised mesh/features/structural metadata, inline payload schema,
known relative external payload schema, buffers/images, quantization, meshopt,
feature IDs/property rows and payload extras. Known resource references must
resolve to exact retained originals, with their original payload/document base.
This relies on the earlier independently decoded finite point/vector profiles,
not on interpreting every byte-identical payload as valid. The metadata model
does not add new extension capability: its known-name set is a gate mechanism,
not evidence for unused declared features.

Unused inline tileset schemas/classes and their JSON extras may be retained as
declarations if no source entity metadata uses them. They carry no active tile
semantics in this profile. Keep their class names/data unchanged, then add the
fixed generated `a2Tile` class after collision checks. A source `a2Tile` class is
Unsupported, without renaming fallback. A source class merely named `rustyTile`
can remain when unused: the old name ban is not inherited policy. A source schema
referenced by payload structural metadata remains owned by that payload's
metadata/resource admission; this is distinct from top-level tileset schema.

| Source capability | Initial disposition |
| --- | --- |
| Effective inherited/explicit REPLACE, contentless routes, ordered arrays/shared slots | Retain; one resolved node/header owner and separate source-order/spatial proof |
| Tile/content metadata entities, even apparently simple nonspatial values | Unsupported; their class/type/semantic/property-row mapping to subtrees is unproved |
| Tileset metadata/statistics and groups/content.group | Unsupported; scope, inherited meanings and ownership are not silently discarded |
| Content bounding volumes, viewerRequestVolume | Unsupported; effective culling/request semantics need dedicated mapping/consumer proof |
| External tileset schemaUri or external source tileset content | Unsupported; payload-relative external schemas remain supported in their proved domain |
| Unknown tile/tileset/content spatial/resource extensions and unknown GLB extensions | Unsupported, even optional or pointing at an already known resource |
| Known content `3DTILES_content_gltf_vector: {vector:true}` header | Retain exact flag with slot header; pinned draft/consumer support remains qualified by the prior source audit |
| Top-level properties/asset fields and opaque extras without unsupported active fields | Preserve owned values under their declared application policy; no implicit geometric/resource meaning is inferred |

All Unsupported decisions happen before output planning/staging. Valid but
unproved entity metadata is not InvalidInput merely because no mapping exists.
Malformed structures remain InvalidInput at their owning structural gate. Unknown
extensions need a demonstrated owner/type/resource/spatial contract before
admission; copying their JSON is insufficient. In particular, the prior
`opaque-extension-known-resource` legacy success establishes byte and known-image
retention only. It is deliberately **not** grandfathered into this initial
profile. `opaque-extension-private-resource` remains Unsupported, not repaired
by retaining its orphan member. Known feature metadata on actual current sources
is unaffected by these tile/content entity-metadata exclusions.

## Extras: explicit application mapping

The [primary extras definition](https://raw.githubusercontent.com/CesiumGS/3d-tiles/4d781014b52294759834018a931223b98ac1ce47/specification/README.adoc)
provides application JSON, not a standardized implicit tile-extras semantic.
STRING property rows are an A2 application mapping; they do not automatically
reproduce `tile.extras` for arbitrary clients. Do not rely on the production
expander reconstructing JSON as a format proof.

The minimal generated `a2Tile` class has the standard validated bound/error
properties and three application properties: `sourceNodeId` (UINT32),
`sourceTileExtrasPresent` (BOOLEAN), and `sourceTileExtrasJson` (STRING containing
the source JSON value). Preserve absence separately from present null, scalar,
array and empty object. Row order/rank comes from the same admitted tile inventory
as effective bounds/error; every available tile has an authoritative row.
Source-node IDs refer to preorder node identities recorded in the source snapshot
mapping, not to inferred centers or reused build paths. Generated terminal
external links are address/ownership rows; do not falsely attribute source-node
extras to them. Their `sourceNodeId` is the declared UINT32 noData sentinel
4,294,967,295 (outside the admitted source-node range),
`sourceTileExtrasPresent` is false and `sourceTileExtrasJson` is `"null"`, while
the child's actual emitted root owns that child's source extras. One generated
class therefore covers source rows and terminal link rows without inventing a
second row model.

The application interpretation base is the root-level `a2-source-tileset.json`,
not the binary subtree URI. Declare that base/mapping in the profile/report.
Preserve main manifest/asset/header extras at their equivalent top-level scopes;
content header extras stay attached to their ordered slots; payload extras remain
in exact payload bytes. Removing `vectorBuildStateSha256` from active asset extras
is the explicit exception below. Other application values are opaque: strings
such as `resourceHint` do not become standard URI edges because they resemble
paths. Neither A2 nor this model promises arbitrary private application-resource
or extension semantics. Original snapshots retain those values for applications
that know their interpretation. An application whose semantics require ordinary
`tile.extras` access needs the documented A2 mapping/consumer migration; it must
not be told the mapping is a standard restoration.

## Exact source provenance and its reference graph

Use fixed root-level `a2-source-tileset.json` and, if the source report exists,
`a2-source-conversion.json`. Their bytes are exactly the captured originals,
including whitespace/key order. The optional report's absence is recorded;
never invent an empty source report and claim it was captured. New live
`tileset.json` and required typed `conversion.json` have new operation meanings.
Root-level snapshots preserve the known relative outgoing base: original
content/schema/resource paths and `geometryReports: geometry-reports.jsonl`
still resolve to the original retained members. A subdirectory provenance copy
would change that base; the model detects it. Relative URI bases are document
relative under the [primary URI rules](https://raw.githubusercontent.com/CesiumGS/3d-tiles/4d781014b52294759834018a931223b98ac1ce47/specification/README.adoc).

No snapshot rename preserves an incoming reference to the overwritten old
`tileset.json`, `conversion.json` or `@3dtilesIndex1@`. Refuse known live incoming
content/buffer/image/schema/report edges to any of these names as Unsupported
before staging. A valid payload buffer can consume an old control's bytes, so
the index also belongs to this gate. Do not rewrite byte-identical payloads to
redirect them to snapshots, and do not assume that unknown extension/opaque
extras references have been enumerated. Unreferenced original non-control
members may remain exact bytes; this alone makes no unknown-reference closure
claim. Known outgoing provenance edges must resolve and consume the same finite
reference/name budgets; a missing required known member is InvalidInput.

The complete pre-staging inventory includes all retained non-control originals,
the two present fixed snapshots, assigned payload aliases, generated manifest/
external roots/subtrees, the new required report and exactly one generated
index. Check exact and both file/ancestor collisions, including source files
under `a2-source-conversion.json/…`. Valid source versus fixed generated-name
conflicts are Unsupported; generated-plan duplicates are InvalidState. There
is no fallback suffix or provenance directory. Charge snapshot bytes/names to
the operation's retained/generated/final budgets explicitly; copying source
JSON a second time consumes bytes even though it is logically provenance.

The required typed report gives source archive SHA256/bytes, source manifest and
optional source report snapshot URI/SHA256/bytes, optional historical vector-state
URI/SHA256/bytes, original index SHA256/bytes with `retained:false`, and the new
metadata/extras/resource/reuse policies. Source archive/index identity is evidence
about captured inputs, not a claim to retain/reconstruct the original ZIP layout.
Original index is replaced because it associates the original member layout;
new index describes the published inventory. No raw source archive is copied
merely to solve provenance. The report points to known provenance resources,
and independent admission/final inspection must understand these typed report
edges, not treat them as active rendering content or unknown orphan JSON.

## Vector build state is historical only

Current source marker `asset.extras.vectorBuildStateSha256` names the SHA256 of
raw `vector-build.json`. Reuse additionally compares `state.manifestSha256` with
the canonical manifest after removing that marker (and removing empty asset
extras). Those are two separate associations, implemented in
`src/vector/pipeline/reuse.rs`; canonicalization is owned by the vector reuse
profile, not a general source-truth oracle.

Preserve the exact source manifest marker in its snapshot and retain exact
`vector-build.json` as historical data. Every active generated header removes
that marker; preserve the other asset fields/extras. Do not regenerate state
from guessed old records or pretend the rewritten tree has its old reuse
certificate. Report the original raw marker/state comparison and the observed
original manifest binding separately, with explicit verified/not-matched/
unverified statuses. Optional historical state is not required source branding
for shape/subdivision/geometry admission. An inconsistent imported claim cannot
acquire a verified status; retaining the original evidence is not an endorsement
or a repair of that claim.

In 18 inspected vector archives the raw state association matches. Four edited
authored fixtures (`fragmented-array-leaf`, `shared-relative-resources`, in both
modes) differ from the source manifest digest; 14 direct-producer cases match.
The model uses a narrow exact compact-byte deletion recipe, avoiding a competing
float serializer. It labels other encodings unverified. This observation does
not require that state match the **rewritten** manifest, and does not reject
otherwise admitted rendered payloads because an optional reuse claim is stale.
Production may use the vector owner's proved canonical routine if that dependency
is warranted, or report the association unverified; no new canonical framework
is necessary for A2 admission.

Using an A2 output as a new vector reuseTileset is deliberately outside this
initial operation's product contract. Retained old state is historical, not a
live reuse input. The current reuser sees a missing active marker and rejects
its integrity gate; that is a static observation, not a newly executed typed
Unsupported response. A future vector adapter/domain gate must explicitly
identify A2 and return Unsupported if reuse is advertised there. No A2 domain
option silently promises reusability, and preservation of payload bytes does
not preserve producer build caches.

## Bounded C1 inventory integration

Addition after coordinator-declared PR #151 merge
`9d2973db06967f87d759c87c34d8fdd0921c86ed`; inspected production bytes are
unchanged. Current `validate.rs` marks original `retainedContentUris` used and
actually invokes `Check::payload` only when the untrusted report's `operation`
is `convert-to-implicit`. It marks `vector-build.json` through the live asset
marker. After that marker is removed, fixed source snapshots and historical
state have no current C1 reference route and the final unused-member check
refuses them. Adding their names to arbitrary report JSON does not implement
this contract; treating every report-named member as used would hide missing
payload checks and unknown-resource semantics.

The narrow private integration takes the consumer's already admitted expected
member inventory, original payload identities and fixed provenance evidence
alongside the same captured-byte/final-staging reader. The consumer owns roles
and known reference edges; private inspection verifies them. A typed immutable
plan, not bytes in candidate conversion.json, authorizes a role. No public
allow-unreferenced option, generic metadata role registry or new RunControl is
introduced. The expected inventory must cover exactly the actual candidate
members, with checked byte/hash associations; it cannot exempt unexpected
members from the unused check.

| Admitted member role | Private final-inspection responsibility and meaning |
| --- | --- |
| Generated render payload aliases | Existing `Check::payload` and known-resource traversal, in addition to admitted exact bytes and source-member identity association; live tree traversal still checks each selected alias |
| Original payloads retained outside the generated URI templates | Explicitly call the same `Check::payload` on each distinct original payload; neither `operation` spelling nor absence of live rendering edges can skip it. Charge reference visits and reads, using admitted cache/work bounds |
| Known source payload resources | Existing payload/schema resource checks own their proved interpretation; every known URI edge must resolve and retain the expected bytes and same base. A role does not replace metadata or resource inspection |
| `a2-source-tileset.json` | Exact captured original bytes/hash/length; bounded unique-key JSON object with original asset version and explicit root under the initial source import profile. Verify admitted known content edges against original payload IDs. This is evidence about the original source, not an active external rendering tileset; do not pass it to generated-tree traversal or reapply C1's stronger child-box predicate as source truth |
| Optional `a2-source-conversion.json` | Exact captured original bytes/hash/length; bounded unique-key source-report JSON object. Its known `geometryReports` edge resolves at the same root base and is checked. Its other imported fields are historical application data; they do not set final inspection budgets, operation kind, padding, used-member exemptions or generated payload counts |
| Historical `vector-build.json` with an admitted source association | Exact captured bytes/hash/length; bounded unique-key historical JSON object, with previously observed marker/state and original manifest hash statuses carried separately. Keep its original root base and no live generated marker. Do not execute reuse, import its config as admission policy, or treat its record names/application strings as standard resource declarations. Unknown historical application-reference meaning remains explicitly unclaimed |
| Other exact retained source bytes, including admitted orphans/auxiliary diagnostics | Verify original member identity/name/hash/length and finite byte accounting; classify as `retainedSourceBytes`, with `semanticChecks: []`. This proves preservation only. It cannot turn an unknown active resource extension into supported semantics or substitute for an original payload's role |
| Generated manifest/subtrees/report/index | Their own format/semantic/inventory checks remain required; the trusted inventory does not bypass report schema, binary availability/metadata, archive/index/CRC or active resource closure |

The consumer's original admitted source supplies the snapshot schema/profile
and known edge inventory once; lower inspection verifies immutable evidence,
not a second source policy. Physical byte/hash checks happen before a member
is marked accounted. Required role-specific checks also precede that mark.
An original payload cannot be mislabeled opaque bytes to avoid Check::payload:
source content identities determine the role. A `vector-build.json` with no
admitted source association is ordinary retained bytes, never a verified state
merely because of its filename. All fixed snapshots/new report/historical-state
edges consume normal visit/read/JSON budgets. Generated aliases and original
payloads share decoded-check caches only by verified identity, while reference
occurrences still consume the appropriate work budget.

Keep `used`/accounted truth distinct from semantic validation: the final set may
include verified original opaque bytes and historical documents, but a report
must not claim these were validated content or generic reference closure. After
the exact per-role checks, the unused-member difference must be empty. This
preserves the check's practical purpose: unexpected files and unowned references
fail, while planned historical evidence is included intentionally.

For a separately invoked public C1 inspection there is no trusted in-memory
source plan. Current public C1 therefore does **not** accept this new inventory.
Supporting standalone A2 inspection requires an explicit bounded A2 report
profile with fixed snapshot roles/paths, unique checked member inventory,
intrinsic hash/length checks, snapshot shape/known-edge reconstruction and the
same actual original-payload inspection. Untrusted JSON claims cannot skip those
checks, assign arbitrary new roles or certify a relationship to unavailable
original source bytes. Public inspection can check intrinsic archived
associations; capture-to-output fidelity still requires the original source
binding. Until that read-only profile is implemented/proven, label its absence
as an integration gate, rather than claiming current public validate accepts
new A2 artifacts or broadening generic report handling silently.

Missing expected members, malformed snapshot/report JSON, wrong candidate bytes/
lengths/hashes, missing known edges and unexpected actual members are InvalidInput
at inspection. An input source requiring unproved reference/metadata semantics is
Unsupported during initial import, before staging. A bug duplicating or
misclassifying an already admitted generated plan is InvalidState. Genuine
read/stat/seek infrastructure errors remain Io; corruption/UnexpectedEof is
InvalidInput through typed archive-read conversion. All errors stay under the
original Attempt's first-cause arbitration and required cleanup semantics; no
report string or fresh control can replace that cause.

[Eleven small inventory-model cases](inventory-results.json) use a trusted plan
separate from an untrusted candidate report. They detect changed original
payload/snapshot/orphan/state bytes, missing snapshot/known reference, unexpected
report-claimed members and missing expected edges. Positive controls inspect
original payloads even with no legacy operation string and account for historical
state without an active marker. They call only an independent GLB header model,
not production Check::payload/C1, and construct skeleton control JSON rather than
a valid implicit archive. Thus they prove the ownership/accounting distinction,
not inspector format/codec/lifecycle acceptance.

## Boundary of this settlement

This is the concrete initial decision available for the production contract:
resolved REPLACE and version promotion, separately admitted exact feature
payloads/resources, explicit extras application rows, unused inline tileset
schemas, fixed root-level exact provenance, known replaced-control incoming-edge
refusal, historical-only vector state and explicit valid-feature Unsupported
dispositions. The private representation has one imported owner for each value;
codecs consume validated effective rows/headers, not source JSON mutation.

Actual current rounded/quantized/meshopt point/vector cases, deep/tight routing,
fragmentation, arrays/shared references, nested BIN/image/external payload schema
and trivial b3dm remain within the initial metadata profile, subject to their
separate numerical/format gates. Optional unknown vendor-resource fixtures are
explicitly outside it. Tile/content metadata, groups, external tileset schemas,
content/request volumes, unknown spatial/resource extensions and general b3dm
are remaining noninitial capabilities with Unsupported dispositions, not
silently retained or accepted. Installed/API documentation must record these
changes before removing the advertised legacy entrypoints.

The models do not validate general JSON/schema/type ranges, subtree codecs,
resource ceilings, terminal-link format conformance, final report/CRC/index,
Rust lifecycle, viewer/picking or installed/platform behavior. Those remain
mandatory independent production acceptance. No foundation/release issue closes
here, and no unproved source use is waived by these model successes.
