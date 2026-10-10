# Independent final-source review of the stored-archive owner

**Decision: accept the bounded local stored-archive owner source and artifacts.**
No unresolved material source defect remains. This decision covers the finite
reader contract and current C1 integration. Applicable exact-head CI, installed
wheel and official Blender checks remain required before a develop merge; it
does not grant A2 or release acceptance. [Final acceptance pins](accepted-source-artifact-pins.json)
bind the exact reviewed source, three frozen executables and durable receipts.

This is a fresh nonauthor lane. It owns only this new review directory and has made no
production edits, Cargo/build/install runs, producer runs, network requests or
candidate CLI executions. The coordinator owns serialized heavy verification.
[Initial source pins](initial-source-pins.json) record the first observed bytes;
they are not a build receipt or an acceptance pin. The initial worktree was dirty
over `ce971b6ce59e46d6a9ea39c3e20c187f6b7f6e37`.

The governing inputs are `AGENTS.md`, the bounded archive reader contract, the
dated selected-directory owner adjudication and the recorded finite decisions.
Source, tests and historical successes were evaluated as candidates rather than
format specifications. Full A2, decoded geometry/metadata, legacy removal and
release authorization remain outside this slice.

## Material finding SR-1: actual read EOF errors lose their cause

At initial pinned `src/archive3tz/read.rs`, `read_exact` calls the trait's
`read_exact` and maps every `UnexpectedEof` to `InvalidInput`. This conflates an
actual `Read::read` returning `Err(UnexpectedEof)` with a reader returning `Ok(0)`
before the admitted range ends. The former's original I/O object and cause are
discarded. The `hash_streams_at_most_64k_and_stops_at_exact_admitted_size` test
explicitly expected `InvalidInput` for an injected real error; the direct C1
error-mapping test preserved an EOF I/O error but never traversed this helper.

This is a static, source-established defect against the contract's causal raw
I/O versus malformed truncation distinction, not a reviewer-executed failure.
The coordinator agreed and assigned the original source author a bounded manual
read loop: `Ok(0)` is malformed premature EOF, `Interrupted` is retried and other
actual errors propagate unchanged as `Io`. That correction and sensitive controls
must be reviewed and executed before this finding can be marked resolved.

The author's correction was separately read at
`b16df290cccbaa03d28fa5e60cf10c1c631d06f1a5bcc3f856ac39cc8badac41`,
recorded in [corrected source pins](corrected-source-pins.json). The manual loop
implements that causal distinction without delegating to an overriding trait
`read_exact`. New constructor controls use an actual injected EOF error with a
recognizable cause, observed zero-byte exhaustion and a single interrupted read.
The member range control now compares both `hash_member` and `read_member` with
actual injected EOF errors and then actual source truncation. This resolves the
source defect; execution of these exact source bytes remains pending. No other
material source defect was found in the reviewed bounded owner.

The additional final test-only change was independently read at `read.rs` hash
`01870325ae4e9cea52f466cab1b6b30cc14c6ef79ba0cc51a3e95c03751b4838`.
Its selected-directory guard separately arms actual fixed-header, name and
nonempty framed-extra offsets in read and seek modes; all six assert original
error kind and recognizable cause. [Final reviewed source pins](final-source-pins.json)
bind this source alongside the unchanged facade and C1 adapter. Execution is
still pending.

## Source observations

The candidate representation has one private `StoredArchive<R>` with an owned
source, central-order catalog and sorted name ordinals. Its scanner selects one
end record and never retries an earlier one. C1 and the legacy validation facade
both construct that owner. The previous admission module is absent. Listing and
writers still use their separate legacy helpers and receive no reader acceptance.
No downstream ZIP directory constructor, synthetic source or metadata exemption
is present in the reviewed C1/index/member call graph.

The scanner checks the caller-known source size before bounded tail allocation.
Entry and central byte ceilings, checked directory extents and
`entry_count * 46 <= directory_bytes` precede the catalog, extent and name-ordinal
reservations. Host conversions and reservations fail through `ResourceLimit`.
Names are retained once in the catalog; cumulative name bytes are charged to the
admitted directory. Local name verification uses a temporary bounded copy. Local
and central extra fields share a TLV parser; framing and duplicate ZIP64 tags are
checked without requiring a sentinel. Sentinel-selected fields, raw UTF-8 identity,
Unicode Path/AES refusal and surplus ZIP64 policy follow the finite recorded
profile.

Each local fixed/variable header and stored data extent is checked against the
central start before variable local reads. Descriptor parsing collects at most
two matching signed/unsigned ends using the declared local width; it cannot
switch width to make a boundary fit. Physical sorting checks the next admitted
local start or central start once. Adjacent records and padding fit; coherent
member overlap or multiple fitting descriptor ends are `Unsupported`; malformed
framing, inconsistent fields, absent matching descriptors and central/source
crossings are `InvalidInput`. Duplicate logical names are checked before index
construction. The scanner does not search opaque payloads for ZIP signatures.

The constructor derives the exact index length as checked `24 * (count - 1)`,
requires the last central member to be the index and explicitly verifies its
stored CRC. Index association uses only admitted catalog local offsets and raw
name MD5. Exact row cardinality plus the duplicate-row seen table establishes
coverage; nondecreasing hash keys are checked independently of physical order.
Exact lookup never normalizes names, and missing lookup is a separate typed
outcome. Complete member reads and streamed hashes use admitted data offsets and
lengths, stop at those lengths and verify CRC, including empty members.

C1 maps its existing five archive limits once. The constructor's already-read
index bytes, complete hash reads and later repeated reads enter the existing
member-work meter. Actual native reader errors map directly by variant. Its held
regular file and before/after length, mtime and pathname identity checks remain
the existing path boundary; this does not create an atomic snapshot. The legacy
validation facade uses the same catalog with natural input-length bounds and a
separate legacy error bridge, without selecting new engineering defaults.

## Review gate and proof limits

The reviewed gate required the corrected final read loop and exact source pins,
the coordinator's compiled source/build/artifact receipts, candidate fixture
executions and meaningful C1/archive tests. It required raw injected read/seek causes
at selected-directory, index and member boundaries, natural truncation controls,
Cursor/held-file agreement, exact range-read guards, before-allocation resource
gates and layout/scaling evidence. Replay the earlier ownership controls as well
as the independent physical/descriptor/name/index matrix.

Resident costs must include retained catalog/String/name ordinals and overlapping
temporary extents/candidate storage, bounded tail/local fields, index bytes,
offset ordinals and coverage bytes. Source inspection establishes O(entries)
table structure, not accepted resident-memory constants or a CLI RSS bound.
C1's later copied names and hash maps are separate additional resident costs.
CLI scaling alone must not be presented as private allocation instrumentation.
Original source pins and historical receipts remain frozen; later source
or artifact bytes have their own receipts. The execution/artifact gates below
are now verified; the broader merge and release gates remain separate.

## Verification integration reviewed

At `5fc28be272c6bf2ecc080611a537d85e01f3ab67`, the ordinary-checkout runner
`tests/archive_read_oracle.py` imports the independent fixture/reference module
and executes every one of its 45 final cases serially. It defines no new format
truth. CI invokes the runner after its validation controls and the existing
`c1-*.json` artifact pattern includes its receipt. The runner's commit/hash fields
are observations; a compiled-source identity receipt is still needed to bind an
acceptance run. A suggested evidence-only improvement is retaining receipts for
malformed CLI output/timeouts and uploading the C1 evidence on failure: the first
runner version can raise before writing output, and the upload step initially
has the default success condition. This does not change archive policy.

The independently authored `catalog_resource_probe.py` constructs literal records
at 2, 3, 1024, 65536 and 65537 total entries. It includes real names, sorted index
bytes, central fields and ZIP64 count metadata. Cases through 65536 reach the
actual C1 hashing/semantic boundary rather than a Python memory model; 65537 must
hit the entry ceiling. It measures one fresh serial Linux child using `wait4`
and records source/output identities. The source correctly limits its claims to
whole-C1 RSS observations; the opaque received source pin is not independent
proof that it names the binary's compiled input. Execution remains pending.

The coordinator supplied logs of 323 passing library tests and passing C1,
package and frontend tests on the earlier SR-1-corrected source. These were read
and confirm actual test execution, including the lower actual-error/exhaustion
control. The final test-only variable-field extension, complete candidate CLI
matrix and exact build/artifact bindings remain separate pending gates. Earlier
passing logs are not silently rebound to the later final source.

## Independently verified portable execution evidence

[Verified portable evidence](verified-portable-evidence.json) binds the
coordinator's completed portable build to source
`5c68f7b52b72e2f6e72096e930f85d27f2897448`, tree
`9b813ec13fb232b857f545b22f0b69e201b58201`. This lane independently checked all
93 production and four acceptance input hashes against current file bytes and
that commit's Git blobs. The frozen 21,033,912-byte binary hashes to
`3328487c2920e07b291eebec8072954f11b0d20a39181ccbc0c4287e2f765524`;
the source-pin JSON hashes to
`913e15540cfe4b387905fdaf00aff270614285ba88d9e12acfe4ffccaa18aa13`.
The build command/log and two-CPU, two-worker, absolute nice-10 wrapper identities
also agree. This verifies the coordinator's compiler/artifact/execution receipt;
it is not a separate reproducible-build experiment.

All 45 independent candidate cases and all 45 ordinary-checkout runner cases
agree on categories, actual process exit codes and independently checked archive
bytes. Ten original ownership/physical controls also agree, and their archive
hashes exactly match the earlier historical receipts. Thus the chosen-directory
fallback, hidden duplicate name, raw Unicode/ZIP64 override and physical nesting
defects were replayed with the original bytes. The ordinary disjoint control and
opaque nested payload controls remain admitted. The 32 final focused Rust archive
tests passed, including raw variable-field read/seek faults, actual EOF causes
versus observed exhaustion, empty CRC, exact range reads, impossible-count
admission and all five exact/one-over private limit controls.

The actual catalog scaling receipt has five fresh serial CLI children. Through
65536 entries, the unused-entry semantic diagnostic shows the owner catalog,
index and complete hash loop finished; source control flow establishes that
implication. At 65536 the input contains 1,114,105 raw name bytes, 4,128,761
central bytes and a 1,572,840-byte index. Whole-C1 Linux `wait4` peak RSS was
93,704 KiB and elapsed time 0.221 seconds. At 65537, entry admission refused the
input with `resource_limit` before the semantic boundary, at 59,376 KiB and
0.020 seconds. These are specific observed costs including catalog, names,
lookup/extents/index work and C1's later copies; they establish no universal
memory ceiling or isolated allocator constants. The 20 existing C1 resource
executions also match their categories and exit codes. Source/output hashes and
read-only observations were checked independently.

Local portable evidence supports the declared finite reader slice with SR-1
resolved. Native artifact and durable receipt verification was completed next,
as recorded below. Platform CI and installed-wheel/official Blender gates remain
coordinator-owned prerequisites to merge, separately from this bounded source
review. No A2 or release acceptance follows.

## Final local acceptance binding

The reviewed repository head is
`17dddb4de4620368a8fde663cc89b70d16e7a339`. All 93 production inputs agree with
the compiled-source pin at `5c68f7b`, the library-test compile at `5fc28be` and
the reviewed head; the intervening amendments changed runner/CI evidence and
documentation. All four verification inputs also match the source pin and
reviewed head. The updated implementation document accurately reports the
executed local scope while keeping the broader merge and A2 gates open.

The frozen native-geospatial/native-jpeg executable hashes to
`604688645e1e4d037922c261bcc575376cb628b2b076fa10862beadf9fe57c78`.
Its separately recorded build includes local GDAL 3.13.3 and libturboJPEG 3.2.0.
All 45 native executions agree with portable categories, actual process exit
codes and literal fixture bytes. The frozen Rust test executable hashes to
`172947f0afa82c2651d768bebbb22b69b0a16a9b88bf5b8c0603f75d69b80cfd`.
The exact artifact passed the final 32 focused archive tests and then all 323
library tests in an unfiltered replay; its manifest binds the actual replay
argv and log hash. No additional compiler run was needed to establish this
test-artifact association. The negative `/usr/bin/false` runner control retains
45 failed records and a read-only receipt; it tests evidence retention, not the
archive product.

The [durable candidate index](../archive_candidate_evidence/index.json) hashes to
`6249714fd558bfc2ce59003d1d93e3558307d59060fd8dd26f6ea261a07b753d`.
All 19 retained records were checked against their compressed SHA256, expanded
SHA256, expanded byte length and original receipt/log bytes where applicable.
The retained base64 archives were separately decoded and all 45 literal byte
sequences match the actual executed candidate archives and independent receipt
hashes. Historical baseline and preimplementation pins were not rebound.

This evidence supports retaining the one-catalog owner for the declared finite
stored ZIP/ZIP64 profile and current C1 consumer. Count/byte gates combine real
Rust fault controls with reviewed reservation order; resource acceptance is
limited to the observed Linux costs stated above. There is no claim of universal
ZIP conformance, an atomic mutable-source snapshot, universal or isolated memory
bounds, accepted legacy-facade RAM policy, decoded content containment, full A2
conformance, legacy-operation removal or release completion. Exact-head CI and
installed-wheel/official Blender checks remain the coordinator's merge gates.
