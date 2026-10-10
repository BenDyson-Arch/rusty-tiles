# P1 point-cloud acceptance and ownership contract

Tracking: [#132](https://github.com/BenDyson-Arch/rusty-tiles/issues/132), under
[#122](https://github.com/BenDyson-Arch/rusty-tiles/issues/122) and the release gate
[#113](https://github.com/BenDyson-Arch/rusty-tiles/issues/113). Baseline `5df15e7`.
This document defines the required contract; execution evidence and final review
must establish acceptance. It does not accept all point-cloud algorithms.

## Request, coordinates and source profile

Rust uses `point_cloud::point_cloud_to_archive(PointCloudRequest, &RunControl)`.
The request requires source, output, an explicit `PointCloudCoordinates` value and
domain options. `with_policy` selects `CreateNew` or `Replace`; domain options do
not contain `force`. `PointCloudResult` contains the canonical output identity,
a typed serializable report and cleanup diagnostics.

`LocalMetres` means source XYZ metres without geographic placement. glTF content
uses Y-up coordinates, with tileset transforms carrying the declared XYZ frame.
`Horizontal` requires a header or explicit horizontal CRS definition and a finite
metre offset from source Z to ellipsoidal height. Header interpretation is an
explicit choice, never an inference from file size, absent options or a failed
transform. Duplicate relevant GeoTIFF keys are ambiguous and refused. A sole WKT
declaration takes precedence over GeoTIFF keys; this is the declared source
selection policy, not proof of arbitrary source CRS semantics. Portable/native capability failures remain explicit; this migration
does not certify every operation accepted by a CRS parser. Geographic reference
tests use analytical WGS84 coordinates or frozen references with provenance.

The admitted decompressed LAS point formats are 0, 1, 2, 3, 6, 7 and 8, including
LAS and LAZ delivery. Waveforms, array/untyped Extra Bytes and undocumented record
tails are unsupported. Source XYZ scales must be positive finite and offsets
finite. Required standard dimensions, raw X/Y/Z, scaled source XYZ, source ordinal
and declared scalar Extra Bytes are retained. Empty sources, ambiguous CRS/field
declarations, reserved field collisions and nonfinite admitted numeric values are
refused. Metadata declarations receive finite allocation/extent checks before
the LAS library materializes them. The admitted metadata profile permits at most
65,536 VLR/EVLR records and 16 MiB of metadata payload/headers; header-to-point
padding is also limited to 16 MiB. Point data itself is streamed without that
metadata-size limit.

Unscaled signed/unsigned 64-bit scalar values retain their integer bytes. Scaled
64-bit integer Extra Bytes are explicitly unsupported in P1: converting an exact
integer through binary64 can silently change it. This is a documented support
change, not a permanent compatibility branch. Other admitted scaled scalar fields require positive finite scales and
use binary64 multiplication followed by addition, with that rounding contract
declared in the report. Extra Bytes no-data/min/max declarations must survive in
the property schema with their admitted value interpretation; accepted record
values alone do not justify discarding declarations. No-data is a schema sentinel,
not permission to drop source points. FLOAT32 declaration slots must be exactly
widened FLOAT32 values; a finite declaration outside that profile is explicitly
unsupported, rather than coerced into a different sentinel or interval. RGB encoding retains its source values in
metadata alongside the normalized display attribute.

The core options default to implicit output, no additional vertex property
attributes, 50,000 points per leaf and 100,000 records per ingestion batch. These
are workload choices, not whole-job memory limits. Explicit coordinate intent is
mandatory in Rust, CLI and Python. Python retains one `point_cloud_to_3tz` adapter
spelling and requires the `source_crs` keyword; the two old Rust overloads and
producer-owned publication options are removed after caller migration.

## Acceptance, sampling and inventory

Every admitted source record has a stable source ordinal for that run. Identical
records remain distinct records: full-detail leaf coverage is a multiset, not a
set of coordinate values or IDs. Every required source field is independently
enumerated before comparison with output; checking only output schema names
cannot prove completeness. Rendered POSITION is decoded through glTF node and
tileset transforms and compared with independent source coordinates. Source XYZ
metadata alone cannot prove placement.

Leaf contents retain full detail. Parent representatives select the first source
record per voxel, ordered by voxel key; source reorder invariance is not promised.
The stated cell-diagonal distance bound is checked against every descendant point
in adversarial fixtures. Float32 position rounding is measured per tile and
included where required by emitted bounds/error. Triangle-style surface error or
general optimal point sampling is not a claim of this profile.

Explicit and implicit hierarchy support have independent coverage, reachable
resource, transform, bounds and finite error checks. Implicit traversal must not
use the production expansion routine as its acceptance oracle. Writer/schema
round trips and the production validator are supplemental checks. The separate
[#133](https://github.com/BenDyson-Arch/rusty-tiles/issues/133) payload validator
scope does not replace these source-to-output oracles.

The producer accepts exact generated-member receipts. Source spool and partition
files are scratch, never archive members. Explicit content and manifest/report
receipts compose directly; implicit conversion returns its generated-content and
subtree receipts. The archive codec consumes those accepted members and knows no
coordinate, source-enumeration or sampling policy.

## One lifecycle and resource owner

One F0 attempt covers validation/preparation, events, execution, serialization and
installation. Preparation binds source/output identities and owns the open source
reader, header/layout and coordinate transform. Invalid options, resolvable source
profile/capability failures and unsafe output identities precede observation and
private output work. Relative paths are resolved before callbacks can change CWD.
Source/output equality and aliases cannot authorize replacing source data.

Execution owns the workspace, ingestion spool, partitioning, synchronous producer,
accepted member inventory, metrics and required reports. There is no point worker
pool to join in this implementation. File handles finish before their scratch is
removed. Checkpoints between batches and bounded producer work cooperate with
cancellation; no claim of immediate interruption of a LAS/native call is made.

Source/storage/report/flush/finalizer failures are fatal. Cancellation and fallible
observation arbitrate through the same first-cause gate. Required producer work,
reports, archive serialization and workspace removal finish before event closure,
sealing and publication. The final ready observation may abort; there is no required
postcommit success callback. Failed cleanup reports retained paths and secondary
causes without replacing an already accepted primary cause. Runtime owns no point
policy, report schema or converter registry.

Physical fault controls target actual spool/member/archive write paths. Shared F0
sync/install tests remain bounded infrastructure evidence; they do not imply every
physical source or storage fault was injected end to end in this consumer.

Source/chunk/leaf/schema/hierarchy RSS, scratch and descriptors are measured at
fixed options as record counts and record widths increase. Spooling and bounded
batch counts are not an asymptotic bound on all hierarchy or metadata state.

## Disposition and stop boundary

Replace the legacy point `output::Job`/`Reporter` facade and its overload family.
Retain/rework the raw record layout, synchronous partitioning, voxel sampler,
metadata encoding and implicit services only for independently exercised cases.
The migration does not stabilize all publicly exposed utility modules. Their
owners and removal gates are in the [surface inventory](public-surface-inventory.md).

Remaining point algorithm/source/CRS breadth stays under #122/#120/#125. Terrain
and broader mesh migration follow their own slices; the legacy job/report paths
remain only while their named consumers are being migrated. P1 completion alone
does not close #122 or the 0.4.0 architecture gate.
