# C2 finite-owner schedules — preparation only

The source snapshots pin the existing validation/C1 owners. This ledger records
observed lifetimes and the proposed rework needed for C2. It does not assert
actual Rust type sizes, capacity equality, allocator overhead or process RSS.

## Existing observed owners and dispositions

| Owner/source | Static observation | C2 disposition |
| --- | --- | --- |
| validate.rs transform/volume/inside/contains/scale_bound | Heap Vec matrices/volumes, optional-child transform only, relative spatial slack, norm overflow/underflow and induced-norm sphere-radius overestimate | Replace decoded point/frame semantics with the fixed numerical primitive; do not retain these heuristics as C2 authority |
| Check::payload URI HashMap | Caches PayloadReport per unique URI; it establishes no scene/frame verdict | Retain bounded unique integrity report meaning only; every content reference streams/checks actual instances separately |
| ValidationTask pending queue | Owns Value nodes, cloned parent volumes/base Strings and child/content arrays; external documents remain related through queued clones | Rework actual consumed instance traversal to explicit bounded raw/current document or fully accounted Value lifetime; an unexamined existing queue is not a new joint storage proof |
| payload::inspect CheckedDocument + typed records | Borrowed admitted RawValue; source bytes stay live, no full payload Value | Retain C1 source meaning and raw numeric/profile admission; extend with a private actual geometry event consumer |
| buffers Cow | GLB BIN borrowed from source; external/inline buffers owned; resource_bytes logical lengths retain owned capacity after truncate | Charge each owned allocation capacity and source baseline, never count borrowed BIN/view aliases as extra backing |
| views Cow | Ordinary views borrow buffers; meshopt views own decoded Vec | Retain decoder meaning; add actual joint survivor/native temporary admission |
| accessors/plans/names/materials/records | Fallible Vec growth with source/node ceiling; shapes and extrema allocate; old/new growth overlap can exist | Pin actual sizes/capacities/growth source in authored ledger, reserve from actual admitted facts, never from huge caller caps |
| meshopt::decode/copy_words | u32 aligned native destination lives while final u8 output copy allocates; logical peak<=2D+3 before returning output | Retain narrow decoder; source/read buffers/prior views/record/math survivors are additional owners. try_reserve_exact is not a universal capacity==request promise |
| external image resolver | Image bytes are inspected/charged then drop each iteration, while source/buffers/views/accessors survive | Retain separate C1 image integrity semantics; include live temporary capacity and cause; no image-render/C2 claim |
| json::admit | Source-derived RawValue/key/slot grammar storage, with serde private scratch and infallible internals | Retain matched C1 requested-owner claim only. Value adapter, startup/RSS and later callbacks are not included |
| archive/name/hash/report/schema storage | Archive directory/index/name facts, repeated String/hash/map owners and conversion Value/schema persist | Explicit joint baseline or honest excluded foundation owner. Interned member IDs avoid per-reference whole-name clones; no duplicated member-byte cache |

## Chosen policy proposal

Retain existing16 C1 ValidationLimits values for integrity/resource interpretation.
Add a private selected C2 finite profile, resolved once at the owning check boundary:

| C2 quantity | Proposed default | What is charged |
| --- | ---: | --- |
| node_instance_visits |262144| Each actual selected node occurrence/enter, not only unique meshes |
| position_instance_visits |16000000| Every drawn POSITION occurrence including repeated indices/mesh instances/content references; restart commands are not positions |
| containment_tests |64000000| Every drawn position against each applicable own tile/content/ancestor bound |
| node_frame_depth |128| Actual selected glTF path; distinct from Tiles hierarchy_depth128 |
| working_bytes |536870912| Requested live owned backing and charged growth/native/parser overlap under an implementation-specific matched-source ledger |

These are finite engineering refusal defaults, not empirically approved values
or guarantees that all inputs satisfying individual C1 limits fit the joint
policy. The intended public selection/default/report migration remains the domain
author/coordinator's combined-gate choice. Do not add unused public fields or
frameworks as preparation. Counts and all sums/products are checked before the
corresponding callback/scan/reservation/decode; host/isize conversion refusal is
ResourceLimit. Avoid allocations sized from configured maxima for a tiny input.

Actual drawn occurrences are simplest to charge, not deduplicated positions.
One primitive referenced by two nodes creates two streams. One POSITION index
repeated100 times costs100 visits; no index-set allocation is needed. The work
cap also bounds callback/predicate count; archive/reference/hierarchy counters
remain independent. A URI report cache does not reduce these instance counters.
Repeated resources either reopen/redecode serially and charge actual reads/work,
or retain their actual backing under an explicitly charged cache; initial choice
is serial re-streaming with only integrity report facts cached.

## Actual-owner schedule to author and verify

Let A be live archive/member-name/index/hash/schema/report backing and its fixed
stack owner; T the current hierarchy document/task/ancestor bound owners; S the
active payload source Vec capacity; R all typed raw-record/plan/accessor/key
backing; B live owned external/inline buffer capacities; V prior owned decoded
view capacities; F fixed-size node/ancestor frame stack backing; E bounded
diagnostic/cause/report backing. Logical view lengths and aliased slice headers
are not additional backing byte totals.

1. Before payload source read, admit A+T+S+fixed read temporary. Reserve actual
   advertised member size after the existing maximum and aggregate read checks.
   Drop obsolete traversal/materialized document copies before active payload
   work if they are not deliberately part of T. Preserve only necessary raw
   current contexts and immutable bound/matrix facts.
2. JSON admission phase: A+T+S+F+E+P(J,K). A prior matched source-only parser
   review estimated P=8J+256*min(node_limit,J)+64KiB under its pinned64-bit Global,
   Slot/Cow/hashbrown/serde sources. Reuse only after current JSON source and
   compiled target machinery/actual size observations match that review. The
   Value adapter and source-sized typed serde error payloads are outside P.
3. After JSON admission drops parser scratch, typed records/buffer loading phase
   admits A+T+S+F+E+R+B+the next external/base64 temporary plus actual old/new
   capacity growth overlap. Borrowed source BIN is already S. Truncated external
   Vec capacity remains owned and charged. No second source/JSON tree for C2.
4. Before each meshopt decode, admit A+T+S+F+E+R+B+V+round4(D)+D, with checked
   round4(D)+D<=2D+3 logical request. Native aligned Vec and final byte Vec overlap;
   account actual capacities under matched allocator/source or fail an explicit
   post-reservation capacity guard without claiming a universal hard allocator
   theorem. Release aligned temporary before retaining the final view in V.
5. C1 integrity and C2 stream phase shares A+T+S+F+E+R+B+V. Existing views/accessors
   stay live through NodeEnter/DrawPosition/NodeExit. Each point/predicate uses
   fixed scalar/interval stack storage; no POSITION Vec, no decoded-per-instance
   matrix/position cache. Old/new growth for frame/graph records must be admitted
   or replace with a fixed bounded frame stack whose real size is included once.
6. External image C1 check temporarily adds actual image Vec capacity; release it
   before the next image. C2's scene stream cannot silently waive these existing
   integrity checks or weaken resolver failure meaning.
7. Complete one content reference: remove its active source/record/buffer/view
   owners before the next. Keep only bounded integrity counters/report/member ID
   facts. Append bounded per-reference C2 report facts or aggregates deliberately;
   do not grow an uncharged one-record-per-position report.

No numeric total is claimed before those actual source/types/capacities/error
branches exist. Existing C1 logical decoded64MiB does not become whole working
storage64MiB; the requested joint512MiB policy needs the complete live schedule.
If A/T ownership cannot be source-bounded without a broad unrelated rewrite,
freeze a narrower honest C2 added-owner claim and leave a whole-validation
working-memory guarantee held. Do not pretend these exclusions are charged.

Invalid consumed-shape errors should use finite stage/reason/index facts.
Precheck RawValue token shape before typed deserialize where serde invalid_type
could retain a source-sized string; otherwise explicitly ledger that dynamic
error overlap. Do not interpolate arbitrary source node/key/value strings into
new C2 errors. Keep original resolver/consumer errors causal and primary; no
message-string category matching.

## Required small future controls

After authored source, measure actual target size_of/capacities and source-ledger
matches, tiny versus huge configured limits, per-counter equal/one-below cases,
repeated URI/instance and deeply nested ancestor predicate work, all live decoder
allocation phases (base64/external/meshopt/images) including real refusal paths,
malformed-shape error storage, and observer/cancellation cause/drop behavior.
Use existing narrow native decoder tests where meaningful; no exhaustive native
toolkit/RSS/full-renderer proof is a prerequisite to authoring this bounded
slice. Fresh nonauthor actual implementation/resource/consumer acceptance is
mandatory afterward.

## Concrete combined-gate decisions still required

1. Settle the box represented-basis and rotated-degenerate dispositions, and the
   explicit approximate local quaternion/matrix admission meaning described in
   math-contract.md. No historical tolerance becomes normative by default.
2. Resolve actual C2 check/default/report selection and producer-domain inventory:
   region/no-scene/geometry-extension Unsupported must be intentionally exposed,
   not silently added to the existing C1 integrity success contract.
3. Freeze the source-specific A/T/R/F/error ownership plan and which owner carries
   joint working admission before implementation. Actual sizeof/capacity/native
   callback execution is later evidence; the representation/owner choice is the
   present prerequisite. Shared decoder extension needs its explicit actual
   consumer signature and causal consumer-error route.
