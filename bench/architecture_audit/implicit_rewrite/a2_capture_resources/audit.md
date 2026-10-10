# A2 capture, resources, lifecycle and adapter audit

2026-10-10. Preparation only; no production implementation or A2 acceptance.
Coordinator identifies the inspected checkout as accepted F1d2 merge
`34a76152d18b02553691f472207225f116307a67` (same production tree as
`43d14b1`). [Source identities](source-identities.json) bind the particular
files inspected; no Git command or Cargo build was performed here. Read
AGENTS.md, the architecture gate, ordered scopes, draft implicit contract,
public inventory, prior ownership/fresh-review audits and baseline driver.

The independently authored [probe](probe.py) ran at `nice -n 10`, one small
process at a time, under Python 3.14.7/Linux. [Results](results.json) record
41 cases. These are Python admission/accounting/schedule **models**, with
actual POSIX filesystem operations for the capture/identity cases. No Rust,
CLI, installed-wheel or actual producer execution is reported by this audit.
Replay into a new directory:

```sh
nice -n 10 python3 -B bench/architecture_audit/implicit_rewrite/a2_capture_resources/probe.py /tmp/new-a2-capture-probe
```

The final execution used `/tmp/rusty-tiles-a2-capture-artifacts-final3`; the
initial run used `/tmp/rusty-tiles-a2-capture-artifacts`. Only the final receipt
is retained here. Artifact paths, PID and RSS can differ on replay; the ZIP
fixture has a fixed 1980 timestamp. The driver hash binds the executed model.
Never use `-O`.

## Required source owner and admission order

One consumer-owned `CapturedArchive` owns one bounded archive byte allocation.
Member values own validated archive names plus byte ranges into that allocation;
payload aliases point to the same member identity instead of duplicating
resident payload buffers. Source report/provenance is imported evidence,
distinct from the required new operation report. Member ranges, references,
counts and all plan arithmetic are checked before consumption. No public
source IR, allocator framework or worker pool is necessary.

The deterministic order is: claim F0 Attempt; validate request intent; bind
absolute source and destination; admit/open the regular source and exclude
native identities/path overlap; check source length before capture allocation;
capture bounded bytes with cooperative checks and final held-handle/path
rechecks; inspect the ZIP envelope before ZIP directory/index allocation;
admit stored-member ranges/names/count/aggregate bytes; parse bounded unique-key
documents; admit semantic tree/resource closure; build and bound the complete
output plan; then admit the first domain observer and create output workspaces.
Capturing up to an admitted archive-byte ceiling is allowed before ZIP envelope
inspection. It does not authorize an unbounded ZIP directory or member buffer.

3TZ's stored-member contract removes ZIP decompression from this operation.
Compressed GLB contents such as meshopt/Draco are opaque exact retained payloads
unless an admitted semantic check independently requires bounded decoding.
Valid general ZIP compression is outside the requested 3TZ input format; reject
it as InvalidInput before constructing a decompressor. Any separate expansion
in an inspector must have its own declared decoded-element/byte/work ceilings.
No claim of payload accuracy follows from preserving compressed bytes.

The pre-directory scanner must bound EOCD search (at most 65,557 bytes), source
bytes, central directory bytes, entry count, individual variable fields, ZIP64
records, local/data descriptor ranges, declared stored byte sum and index bytes.
Check overflow/ranges and local versus central flags/names/sizes. Reject duplicate
names and exact/file-ancestor collisions. Check intersecting local-header/payload
ranges or explicitly prove a narrower envelope; duplicated physical extents
must not defeat logical byte/work accounting. Validate index cardinality, order,
hash/name/offset association and CRC using admitted ranges. The model deliberately
handles classic stored ZIP only, omitting ZIP64, descriptors, CRC and 3TZ index
semantics; it proves the gate ordering, not complete archive admission.

JSON byte ceilings must apply before text/Value allocation. A unique-key seeded
parser counts values and depth before admitting each next value, keeps a typed
limit marker separate from syntax error text, and rejects trailing data and
nonfinite numbers. Per-document limits alone are insufficient: charge aggregate
distinct parsed JSON bytes/items, source nodes, traversal visits and resource
edges across manifests, schemas and source reports. Cache admitted document
identities once; repeated references still consume reference/work budgets.
Recursive external documents require cycle detection and depth across files.

## Identity, capture stability and callbacks

Reject source and existing destination symlink leaves and nonregular files.
Use nonblocking/nofollow opens where available so FIFOs and leaf replacement do
not block or escape the regular-file contract. Resolve parent aliases once,
preserve absolute native path values, and compare an existing destination's
native file identity with the held source handle. Canonical string equality
cannot reject hardlinks. Reject destination equality, destination below a
regular source path and parent aliases to it before workspace creation. Check
the nearest existing destination ancestor is a directory without creating
parents. OutputPolicy owns the later installation race.

Check source identity/length/mtime on the opened file before/after capture and
against the selected pathname at the final sweep. An observed replacement,
growth, shrink, symlink substitution or disappearance is InvalidInput. Genuine
open/read/stat infrastructure errors remain Io. The source must remain stable
during capture; no adversarial/atomic snapshot guarantee is offered. The model
rejects observed growth/shrink/identity replacement and demonstrates the limit:
an in-place second-half rewrite with length and mtime restored admits mixed
bytes. Stronger guarantees require a different source contract, not a claim
that a second stat proves atomicity.

Close preparation's source handles once the owned archive is complete. Every
later semantic check, payload write and report count reads owned values, never
the input pathname. The executed POSIX model changes CWD, replaces the source
and creates the destination after capture; owned bytes and absolute output stay
unchanged. It does not execute F0's final CreateNew/Replace race. Initial overlap
refusal does not claim to prevent an adversary creating a new hardlink/symlink
after binding. Runtime publication must install the completed file according
to its platform-tested policy and must never follow a replacement leaf to
mutate the captured source object.

Destination nonregular/symlink syntax is InvalidRequest in the proposed A2
boundary, while an inadmissible source leaf is InvalidInput. The existing mesh
binder's destination InvalidInput choice is not automatically retained.

## Complete resource/name plan and amplification

Resolve known archive-local URIs relative to each document or payload's archive
parent, with one percent-decoding interpretation, portable bounded names and
confinement. Same-parent payload aliases preserve the unchanged payload's base
for external buffers/images/schemas. A nested alias in another parent is not
semantically equivalent merely because its GLB bytes match.

Before staging, inventory retained source names, generated aliases, the main
manifest, external-root documents, subtrees, required report, admitted provenance
documents and one generated index. Use a trie or file/ancestor set, not adjacent
sorted comparisons: `a`, `a-`, `a/x` has an ancestor collision that an adjacent
predecessor test misses. The model rejects exact collisions, both ancestor
orders, a source `subtrees`, nested aliases, generated-document ancestors and
duplicate report/index entries. Source control entries being deliberately
replaced must be explicit dispositions, not excluded from collision checks
before their semantic roles are resolved. All generated names are assigned
from the admitted plan, and writes consume only that inventory.

Retaining every non-control original preserves orphan bytes without needing to
infer which arbitrary resource is unused. It does not validate unknown extension
references, metadata interpretation or old report references. Unknown reference
semantics require an explicit supported/excluded disposition from the semantic
owner. Renaming original `conversion.json` to a provenance member can change
references to it; embedding parsed source JSON is not preservation of its exact
bytes. Required new `conversion.json`, original report byte retention and any
references to control documents need a concrete rule before implementation.
Do not silently overwrite a source file, choose a fallback name after staging,
or use source branding as resource admission.

Charge alias bytes per **emitted occurrence**, not per distinct original. The
model permits 4,096 aliases of a 16-KiB payload exactly at 64 MiB and refuses
4,097. Charge each repeated document header/schema/extras occurrence. A 64-KiB
header repeated 4,096 times requires 256 MiB and exceeds the proposed 16-MiB
generated-document envelope, even though subtree availability bits are small.
Streaming serialization must charge/stop before a buffer/write grows beyond
its budget. Checked arithmetic covers count increments, aliases, header copies,
subtree arrays/property rows, index length, names and final output overhead.

## Proposed finite resource envelope and its proof limits

These are engineering admission proposals, not accepted product limits. Apply
the tighter remaining aggregate bound at each allocation/read/write. Do not
permit all independent maxima if their checked combined bound would overflow.

| Quantity | Proposed starting ceiling and independent reason |
| --- | --- |
| Captured archive bytes and aggregate stored member bytes | 64 MiB each; one immutable source owner, no resident alias payload copies |
| Source/final entry count | 16,384, including output index for final count; bounds directory/index/name metadata independently of payload bytes |
| Source central directory | 2 MiB; 16,384 classic fixed headers plus at most 1 MiB names fit, extra/comment bytes consume remaining budget |
| Per-name bytes / aggregate source or final name bytes | 4,096 / 1 MiB; bounds path depth/string/header amplification; add a 128-component ceiling |
| One tileset JSON / all distinct parsed JSON bytes | 1 MiB / 8 MiB; source reports/schemas must count, not just main manifest |
| JSON values per document / aggregate values | 65,536 / 262,144; bounds small-item AST amplification before allocation |
| JSON depth / semantic source tree depth | 64 / 31; distinct purposes, source tree depth continues across external documents |
| Source nodes / content slots per node | 4,096 / 16; finite tree and one-level subtree/slot plan; total references need a separate checked 65,536 cap |
| Resource/reference traversal visits | 65,536; cycles/shared references cannot create unlimited repeated work |
| Alias bytes / generated JSON+subtree bytes | 64 MiB / 16 MiB; checked occurrence costs, independently motivated by amplification controls |
| Required serialized report bytes | 64 KiB before any imported provenance embedding; provenance has a separate admitted owner/budget |
| Final completed archive bytes | 160 MiB; actual retained originals + aliases + generated data + report + index + local/central/name/end overhead must fit |

Under the present archive writer's classic/no-extra envelope, for N selected
members excluding index: index bytes are `24*N`; each entry including index
costs `76 + 2*UTF8(name).len()` header/name bytes; EOCD costs 22 bytes. Source
index is discarded. ZIP64 extras/end records must be included if that envelope
is admitted. With 16,383 short names and maxima of 64+64+16 MiB plus a 64-KiB
report, the independently computed bound is 152,906,050 bytes. This is an
accounting example, not proof that all legal name maxima fit: aggregate names
and the final byte ceiling govern the actual plan. Refuse a valid oversized
plan as Unsupported before output allocation/staging.

The small measured model stores 4,096 generated documents totaling 4,242,346
bytes, with Python tracked peak 4,412,483 bytes. Process high-water RSS is
recorded in results.json. Four descriptors were visible at end. This says nothing about Rust
AST allocator overhead, inspector caches, producer codecs or maximum scratch.
The capture model temporarily holds chunks plus joined bytes; the proposed Rust
owner should reserve a single bounded buffer. No accepted hard RSS cap follows.

[Independent source envelope counts](source-envelopes.json) read the sibling
real-source agent's 14 final small portable archives after verifying their source and
every non-index member hash against its exact execution receipt. Observed
maxima are 96,342 archive bytes, 30 members, 90,352 aggregate stored bytes,
3,224 directory bytes, 13,516 manifest/member bytes, 29,740 distinct JSON bytes,
1,791 JSON values, 368 compact original header bytes, 25 nodes/content
references and two content slots. All fit the proposed envelopes; this does
not establish maximum production memory or broader semantic eligibility.
The [counter](count_source_envelopes.py) executes no producer and records the
originating portable execution pins without relabeling that evidence as ours.

The smallest initial implementation can materialize a private bounded workspace
from captured member ranges and serialize it with the existing file-member
archive writer. It needs no generalized source abstraction. Resident aliases
are streamed from captured bytes, not retained as duplicated vectors. Required
cleanup applies to that workspace. Workspace logical bytes are at most the
plan's member-byte envelope and candidate bytes at most 160 MiB; with 144 MiB
workspace members the coarse simultaneous scratch bound is 304 MiB, plus
filesystem allocation/metadata overhead to measure. Private borrowed-byte
serialization could remove the workspace, but is a separate representation
decision requiring demonstrated benefit and codec evidence, not a framework to
introduce preemptively.

Before these numbers become the declared profile: obtain actual admitted
point/vector source counts/sizes; measure final Rust RSS, descriptors, scratch,
wall/work boundaries for large single member, many tiny members, maximum JSON
items/depth, repeated headers, shared payload alias amplification and output
names. Exercise exactly at/one beyond each limit and corruption near the same
gate. Coordinate sustained stress with the build owner. Serialize heavy work,
use the standing thread/nice limits, and do not imply unconstrained concurrent
jobs are bounded by one job's resource measurement.

## Reusable foundations and concrete dispositions

| Actual current component | Disposition and proof limit |
| --- | --- |
| `runtime::{Attempt,Staging,SealedArtifact}`, JobFailure/RunControl | Retain accepted F0 control/publication machinery. `begin/check/emit/close_events/fail/seal/publish` supplies first-cause arbitration, observer closure and completed-file installation. Does not capture/admit A2 source, clean its domain workspace or certify implicit meaning. |
| `output_path::resolve` | Retain native absolute destination resolution technique. Does not validate nonregular/symlink output leaves, nearest parent syntax or native source overlap by itself. |
| `mesh_archive::binding` | Rework/copy only proved regular/nofollow/nonblocking open, bounded capture, identity and final-path sweep techniques. It is private, mesh-specific and owns glTF dependency policy; do not import all of its profile/defaults into archive A2 or call mesh preparation. |
| `validate::admission::admit` | Candidate reuse of bounded EOCD/ZIP64/local descriptor scanning after source-byte admission. Currently private, File-only and fixed C1 limits; it does not establish A2 resource/name/plan accounting or whole-archive nonoverlapping extents. Generalize only its reader and explicit budgets if needed, without a public validation framework. |
| `validate::json::parse` | Better parser retention candidate than ordinary serde_json: seeded value/depth counts, unique keys, typed limit marker and trailing checks. Currently defaults reset for each document and no A2 aggregate owner. Inject one operation-owned remaining budget; map typed outcomes deliberately. |
| `mesh_archive::source::json::parse` | Unique-key/padding technique only. glTF/GLB domain and no aggregate item ceiling; C1's seeded parser is the more direct candidate for imported tileset JSON. |
| `archive3tz::serialize` | Retain private bounded streaming serializer for an admitted finite workspace inventory: one 64-KiB payload buffer, per-count metadata/index allocations, caller-owned checkpoints, explicit finalization/flush. It reads file paths and checks length/mtime, so no later original-source pathname may be a member source. Does not prove source/native identity, subtree semantics or final A2 candidate. |
| `archive3tz::validate_open_3tz` | Candidate bounded index check **after** admission; it allocates ZipArchive/index without its own envelope gate. Current `by_name(...).is_err()` converts all manifest lookup failures into missing-manifest. Preserve actual I/O/corruption causality when used by A2. |
| `validate::inspect(ValidationRequest)` | Supplementary C1 structural/payload checks only. Path/File-only, opens its own source and uses its own limits; does not inspect A2's captured source or bind final inspection to the staging handle. Its documented exclusions include metadata/implicit semantics and content/world bound accuracy. Minimal private read-only inspection from captured bytes or already-open final staging handle is required; do not call the public path API and recapture. |
| `convert_implicit` tree/lifecycle/options/report adaptation | Replace together, then delete operation entrypoints/options. Current source/report reads, semantic sentinel mutation, public nested package, fresh default control, inspected candidate and second repack do not supply the new contract. |

The narrow inspector change is to separate the public file-opening/stability
boundary from the existing Check/resource traversal, and permit its private
read-only core to consume `Read + Seek` with caller-owned explicit remaining
budgets. This has two concrete A2 inputs: `Cursor<&[u8]>` over the owned captured
archive and the already-open final Staging File. Keep the public inspect API
and its source-path/stability checks for its existing callers; A2 neither calls
that path wrapper nor changes general tooling ownership. Reuse the existing
Check logic only for its independently established finite profile, not as a
second source admission owner.

There is a semantic dependency in that Check logic: `validate.rs::node` checks
all child bounding volumes against the parent through `contains`, and checks
content bounding volumes as well. Box `inside` uses a `1e-6*(1+axis norm)`
tolerance; errors have another monotonicity tolerance. This is an existing C1
finite-validator-profile constraint, not independent proof of ancestor decoded
content coherence or a universal implicit/source padding rule. If A2 admits
box overlap beyond that predicate, mandatory C1 invocation can refuse an
otherwise admitted A2 source/candidate. The semantic owner must settle that
dependency explicitly: retain an independently justified common profile or
rework the predicate/check selection and prove it. Do not add a silent bypass,
claim C1 established decoded content bounds, or import C1 tolerance as A2 truth.

The prepared values should be private `CapturedArchive`, `SourceMemberId`,
`AdmittedSourceTree`, `ImplicitPlan` and a complete `PlannedMember` inventory.
`PlannedMember` distinguishes retained captured range, alias of a payload member,
and bounded generated bytes. The semantic owner supplies validated cells,
transforms, content slots and metadata; path/name accounting consumes those
values without reinterpreting hierarchy or guessing scheme. A `PreparedImplicit`
owns resolved output/policy, capture, admitted tree/plan and the typed report.
Do not expose imported mutable JSON or use sentinel keys as node identity.

## One candidate, failure causality and required report

One Attempt covers validation through publication. Prepare without domain
callbacks; emit admitted progress; create owned workspace; write only planned
members and required typed report; serialize exactly one final archive into
Staging; inspect that same completed staging handle/file using bounded private
checks and the independent semantic candidate oracle; remove required workspace;
emit `ready_to_publish`; close event admission; seal/sync; publish. No public
package call, nested RunControl, tree rewalk/repack or final pathname source
reread. A sequential cloned staging handle is sufficient for private File-based
inspection once its checked API exists; no copying candidate is required.

Preserve first Attempt-selected cause. ObserverFailure/Cancelled recorded first
remain primary when a write/check later fails; subsequent errors and cleanup
diagnostics remain secondary. Malformed source/required missing member/capture
change are InvalidInput. Valid unsupported shape or finite engineering ceiling
is Unsupported. Request conflict/overlap is InvalidRequest; infrastructure
open/read/write/finalize/report/inspection/cleanup failure is Io; actual final
installation collision is Conflict. Distinguish missing-member, invalid ZIP,
UnexpectedEof and other I/O by typed variants at the boundary, never message
searches. Do not zero unavailable padding or classify missing provenance as
archive corruption if provenance is an optional profile input.

Proposed collision categories distinguish gates: unsafe/duplicate/conflicting
file paths inside the source archive are InvalidInput; a valid admitted source
name conflicting with a fixed generated output name is Unsupported for this
finite naming profile; duplicate names generated by the producer after an
admitted plan are InvalidState. The name-control models choose Unsupported at
the source-plus-generated planning gate. Final contract review must preserve
this distinction rather than reuse one generic collision error everywhere.

Required report serialization/write failure prevents commit. Required workspace
cleanup failure prevents `ready_to_publish` and reports retained owned paths.
Candidate cleanup failures augment the causal JobFailure. Successful publication
returns its typed report already in memory and F0 cleanup diagnostics; no
fallible domain reread/serialization/observer/signal check follows commit.
Runtime's postcommit temporary-name cleanup diagnostics do not change success.
The 14-stage model checks schedule constraints and causal/cleanup combinations;
it is not actual F0 or A2 fault injection acceptance.

Proposed public request: source, destination, OutputPolicy and one required
`ImplicitSubdivision::{Quadtree,Octree}` until the semantic/source audit proves
an inference alternative. A constructor plus `with_policy` is enough; keep
engineering ceilings private and versioned. Proposed result: absolute output,
required `ImplicitReport`, cleanup diagnostics. Report fields have checked u64
meanings: profile/schema version, declared subdivision, preservation policy,
captured archive bytes, admitted original member bytes/count, source explicit
nodes, source content references, distinct original payloads, external-root
count excluding main root, subtree count, aliases bytes/count, published member
count including index. Count facts from inventory once, not serialization
recounts or saturating arithmetic. Final archive length belongs in the result
if including it in conversion.json would require a self-dependent rewrite.
No unconditional `contentBytesPreserved: true` substitutes for byte/resource
acceptance. Provenance disposition remains a semantic blocker above.

## Rust, CLI, installed API and deletion gates

The existing Rust public `convert_implicit` module/root aliases expose
ConvertToImplicitOptions and both unit/reported entrypoints. Replace with one
intentional request/result/report facade, then remove these declarations and
the operation's `output::Job`, Reporter, generic ConversionResult, public package
adaptation and candidate repack. Operation-specific tests/imports must migrate.
Other mesh/raster callers still use shared legacy Job/Reporter/ConversionResult,
so this slice cannot delete the shared families globally.

CLI `Command::ConvertToImplicit(IoArgs)` currently produces
`Outcome::Converted` via the old reported function. Give it explicit scheme
grammar if required by the settled profile, map `--force` once to OutputPolicy,
construct one RunControl and return the typed implicit outcome. Existing CLI
job_category already owns all F0 categories/statuses; reuse it, with required
typed summary/report fields and stable event semantics.

Python `convert_to_implicit(input, output, *, force=False)` currently returns
ConversionResult through `run_conversion(py, None, ...)`, without callback
control. Preserve the useful spelling if chosen, but replace that adapter
route with the single F0 `run_job`; expose a typed result/report, explicit
scheme intent and `callback=None` matching the new control boundary. `force`
may remain adapter spelling only. Existing job_failure_to_python maps F0 kinds
and retains original callback/signal exceptions. Do not leave compatibility
wrappers calling the deleted semantic/lifecycle implementation.

Acceptance must use external Rust compile consumers plus actual CLI and freshly
installed wheel: admitted point/vector source profiles, defaults/required scheme,
InvalidRequest/InvalidInput/Unsupported/Io/Conflict parity, callback mutation,
observer exceptions/cancellation/reentry/concurrent attempts, source hardlinks/
symlinks, destination races and exact typed report/resource byte inventory.
Inject actual source/write/report/archive finalize/flush/inspection/required
cleanup faults; prove no publication and exact prior target preservation. On
success independently inspect the exact sealed/published archive hash and
report, viewer/picking semantics under the separate owner's oracle, and platform
publication behavior. Rebind all final execution receipts to final source and
artifact identities. Update docs/CLI.md, docs/CONVERTERS.md, Python README and
public inventory for deliberate changed domain and removed Rust API. Independent
nonauthor review and applicable installed/platform CI remain mandatory.

This audit settles ownership proposals and supplies focused controls. Semantic
resource/provenance disposition, actual-source envelope fit, final production
resource measurements and lifecycle/installed acceptance remain blockers before
the replacement can retire the advertised old operation. It closes none of
#126/#125/#113 or release gates.


The coordinator replayed the read-only envelope census against the stabilized
final portable source receipt at `/tmp/rusty-tiles-a2-probe-artifacts-real-portable-final2`.
The retained census now binds all fourteen source archives and that final
execution's exact receipt/driver/member identities; observed envelope maxima
remain unchanged. This adds no producer or stress acceptance to this lane.
