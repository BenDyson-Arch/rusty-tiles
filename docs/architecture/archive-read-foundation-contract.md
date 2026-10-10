# Bounded stored-archive reader foundation

Status: proposed next production prerequisite under #125/#126. A2's full
rewrite is still held at its literal external-template conformance gate. This
slice does not substitute the different partial-implicit product or remove the
current converter. The existing C1 read-only inspector is the real consumer;
A2's captured bytes and completed staging file are the concrete next inputs.
The foundation gate in [README.md](README.md) applies.

## Evidence and finite decision

The coordinator's independently authored [two-archive control](../../bench/architecture_audit/implicit_rewrite/a2_settlement_coordinator/overlap-before.json)
executes accepted portable production `4553533` with exact binary/driver/archive
hashes. Both the ordinary disjoint archive and an archive whose GLB local
header/data lie inside another member's stored data return success. The latter
has coherent names, CRCs and a correct 3TZ index. This is an executed physical
admission gap, not a decoded scene defect or proof of an unbounded allocation.

The selected finite stored-3TZ reader requires disjoint physical member records:
local fixed/variable header, stored data and any admitted descriptor form one
extent. Zero-length data still has a header extent. Central directory order
need not equal physical order; adjacent extents may meet exactly. No member
extent may intersect another or the central directory. A nested ZIP file as
ordinary opaque payload remains allowed; its interior records are not separate
members of this archive. Distinct logical names cannot license shared physical
records or multiply the interpretation of one extent.

PKWARE APPNOTE 6.3.10 sections 4.3.6–4.3.9 describe repeated local-header/data/
descriptor records and descriptor lengths. The disjointness rule is this
reader's explicit finite profile inference, rather than a claim that every
ZIP implementation rejects the reproduced input.
[Primary ZIP specification](https://pkware.cachefly.net/webdocs/casestudies/APPNOTE.TXT).
A structurally consistent archive outside this physical profile is Unsupported;
malformed offsets, truncated records, inconsistent sizes/names/CRC fields and
invalid descriptors remain InvalidInput. Existing C1 ResourceLimit distinctions
remain separate. Descriptor ambiguity must be resolved against its next physical
boundary, not by choosing a length that overlaps another member.

## Ownership and smallest change

Move the existing bounded stored ZIP/ZIP64 envelope reader beneath the private
archive format owner. Its inputs are `Read + Seek`, caller-known byte length and
an explicit private `ReadLimits` containing only archive/central-directory/
entry/member/stored-byte limits actually consumed there. It owns no File path,
source stability, job control, converter, validation report or public limits.
C1 maps its settled public limits once at its boundary and continues using its
regular held file and before/after identity checks. No runtime or validation
import enters the archive codec. Remove the old decoder after integration;
do not retain two envelope interpretations.

The format owner checks source size before bounded tail allocation, declared
central size/count before directory materialization, and all local/extra/
descriptor extents before downstream ZIP/index allocation. Keep checked count,
byte and host-size arithmetic. The extent table is bounded by admitted entry
count; sort and verify it once. Every extra-field TLV is structurally checked even when no size/offset sentinel
requires ZIP64 expansion. Duplicate ZIP64 tags and truncated fields are refused;
unknown well-framed extra tags remain opaque. Required ZIP64 values are consumed
in their prescribed sentinel order; absence/truncation is malformed. Both local
and central extras share this interpretation, without copying a second parser.

Descriptor parsing returns the selected extent, including an optional signature.
The logical central-directory scan first admits each bounded header/data extent
and descriptor candidate. Physical sorting then checks candidates against the
next local-record start or central-directory start. No structurally matching descriptor (bad CRC/sizes or truncation), or an
extent crossing the central-directory/source boundary, is InvalidInput. At least
one structurally matching candidate whose possible ends all cross another
coherent local-record boundary is Unsupported under the disjointness profile.
Exactly one distinct fitting end wins; multiple distinct fitting ends are
Unsupported. Padding gaps are allowed and exact boundary equality is allowed;
do not require the extent to fill the gap or silently pick a different length. Charge/check any per-entry
candidate storage under the already admitted entry ceiling.

Read failures keep actual I/O causes. Typed
format errors distinguish malformed, unsupported and resource limit; C1 maps
those meanings without message matching. Existing public validation categories
and geometry/metadata proof limits remain truthful.

The private 3TZ index reader should consume the already selected `Read + Seek`
input rather than reopen or clone a pathname. File wrappers retain public
path-opening behavior. A missing tileset is identified only by the typed
not-found member variant; ZIP corruption or I/O during lookup is not converted
into missing manifest. Index cardinality, ordering and every offset/hash/name
association remain checked. Admit exact index length as 24 times the checked
non-index entry count before allocating its bytes. Read its admitted raw stored
range and verify CRC explicitly, so detected CRC mismatch is a typed malformed
condition while underlying seek/read InvalidData remains Io. C1 maps these
private codec errors directly instead of losing origin through its legacy
blanket archive-read mapping. Other C1 payload ZipFile read-cause classification
is outside this slice and must remain recorded as an unresolved ownership limit. CRC verification remains part of complete C1 member
reads; this envelope slice cannot claim every payload was read merely by
checking fields. Do not add a generic archive resource framework or a public
reader API.

## Independent acceptance and stop

Before retention, separately review this finite profile and the current source.
Production acceptance needs independently authored stored records for disjoint,
adjacent, reordered and nested extents; classic and ZIP64 lengths; signed and
unsigned 32/64-bit descriptors; descriptor-boundary/CRC-signature ambiguity;
header/data/directory overlap, duplicate local offsets, malformed names/sizes/
extras and exact one-over byte/count limits. Include a valid ordinary nested-ZIP
payload control so the rule does not become a payload magic scan.

Run the same admitted bytes through in-memory Cursor and a held file and prove
agreement. Inject seek/read failures so real I/O survives while truncation is
InvalidInput. An instrumented source proves the entry/byte admission gates fire
before ZIP directory/index allocation or an unbounded read. Replay the actual
CLI before/after control and the existing meaningful C1/archive suites with two
workers under the coordinator. Inspect resource scaling of the bounded extent
table; no whole-operation RSS claim follows from one bounded table.

Fresh separate nonauthor final review, exact source/artifact receipts and all
applicable PR/platform/wheel checks precede a develop merge. Do not claim A2
format/lifecycle/metadata/viewer acceptance, decoded content bounds, generic ZIP
conformance, legacy-operation removal or release completion from this slice.
