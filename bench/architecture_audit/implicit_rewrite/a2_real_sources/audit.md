# A2 actual point/vector source and resource audit

This is bounded preimplementation evidence for draft mandatory probes 3 and 4.
It does not accept a replacement, certify arbitrary source formats or close a
release gate. Source inspection follows the repository architecture gate and
the draft implicit rewrite contract. The coordinator supplied accepted head
`43d14b1` and reports that merged baseline
`34a76152d18b02553691f472207225f116307a67` has the identical production tree.
Both frozen `4553533` executable artifacts are independently hashed; every
production input in their frozen source manifest matches this checkout.

## Executed source observations

[Results](results.json) retain compact observations; the portable/native raw
gzip files preserve complete commands, source/fixture/archive/member hashes,
exact rational envelopes, metadata tables, ordinal plans and stdout/stderr.
[Source/artifact pins](source-artifacts.json) include the driver, codec wrapper,
upstream codec and all frozen production inputs. No generated archive or binary
is committed. Fourteen small cases and nineteen controls run separately on
portable and native binaries. Each run performs 44 serial CLI commands. Eight
legacy conversions succeed, with no raw owned-output oracle failure. Five real
producer hierarchies are refused as irregular; one authored unknown-resource
case is refused as unreferenced. A probe completing successfully means these
observations were collected, not that the old or replacement operation passes
the proposed contract.

Literal LAS 1.2 format-0 fixtures and independently authored GeoJSON produce
actual current explicit sources. The fresh decoder imports no production
parser, encoder, implicit expander or existing acceptance oracle. It reads ZIP
CRC/framing, GLB and modern b3dm framing, accessor ranges/stride/normalization,
selected scene translations/scales, feature IDs, structural metadata strings
and scalars, binary subtree availability/ranks, boxes/errors/extras and external
root fields. Exact rational arithmetic interprets decoded numbers and matrix
composition for source geometry. Meshopt bytes are supplied to the independently
pinned meshoptimizer 1.3.0 JavaScript reference codec; geometry, metadata and
expected source identities remain owned by this driver. This is third-party
codec evidence, not a second independently implemented compression codec.

These executed b3dm wrappers have `BATCH_LENGTH: 0`, no `RTC_CENTER`, and empty
batch tables. Their embedded GLB feature metadata is covered; general b3dm
feature/batch-table interpretation and RTC placement are not proved here.

Full-detail source identity/multiplicity and authored metadata match for the
point fixtures and the four rounded/compressed/quantized vector-point fixtures.
Source coordinates are separately enumerated from literal LAS bytes; vector
IDs, label and weight come from authored GeoJSON. Parent representatives are
not counted as extra full-detail source identities. All fourteen sources have
zero exact decoded own-payload and descendant-payload escape from their original
owner boxes. This tests geometry coherence in the exercised translated local
frames; it does not infer coherence from a producer label or report.

| Current use case | Executed observation | Disposition for A2 |
| --- | --- | --- |
| Flat, rounded point leaves | OCTREE source rewrites; source box includes a `1e-6` metre minimum half thickness on zero-width Z; nominal child cell excess is exactly the serialized `1e-6` value | Retain, with authoritative actual tile boxes; an exact nominal-cell profile would remove this supported case |
| Rounded vector leaves | Rewrite succeeds; source box minimum half thickness is `0.001` metre; tiny positive exact nominal-cell excess exists | Retain source boxes and geometry, without a fixed epsilon |
| Quantized vector leaves | Rewrite succeeds; maximum observed authored-coordinate error is `2.746108165911428e-7` metre; independently computed per-leaf source-extent UINT16 half-step contains every decoded point | Retain unchanged payloads; report preservation does not turn quantization into lossless source geometry |
| Meshopt and quantized meshopt | Reference-codec decoding reproduces source IDs/metadata/geometry; rewrite preserves exact payload bytes | Retain the exercised codec profiles and bounded closure |
| Tight deeper point/vector trees | Legacy global midpoint walk refuses actual sources; exact decoded descendant geometry remains inside original boxes | Rework address planning to authored child ordinals; do not retire these sources to make a nominal-cell pilot pass |
| Coincident points at a midpoint | Legacy refuses two children in the same geometric slot; original source identities remain distinct | Retain through authored child ordinals, with geometry coherence checked independently |
| Fragmented line | Actual source includes validly declared draft primitive restart and preserved feature rows; legacy refuses its hierarchy | Retain original content bytes and authored topology; rework address planning |
| Fragmented polygon fill/outline | Actual producers emit b3dm/GLB arrays, up to two slots per node; portable has 25 nodes and native 27; legacy refuses the full hierarchy | Retain arrays and b3dm bytes; a separately packaged actual two-slot producer leaf rewrites successfully |
| Duplicate payload references and nested external resources | Two source slots share one GLB; BIN, image and nested schema resources remain at their original archive parents; rewrite and raw byte/base checks pass | Retain independent ordered slots, shared original inventory and distinct aliases |
| Imported feature metadata, inline schema and opaque extras | Exact payload/schema resources and source inline class definitions/extras survive the successful resource fixture; box/error/extras rank rows match source | Retain these exercised values without claiming broader metadata semantics |
| Optional unknown extension referencing an already known resource | Opaque extension bytes survive; the oracle explicitly lists the unknown extension | Preserve opaque bytes; no semantic interpretation claim |
| Unknown-only resource, external tileset schema, tile metadata or content volume | Unknown-only resource is refused as unreferenced; the other features have distinct legacy admission refusals | Unsupported initial source profiles unless their owning semantic/resource admission is separately proved; not executed source-invalidity findings |

The FLOAT32 point/vector fixtures have maximum observed source-coordinate error
`4.768452299686032e-10` metre. The raw receipt records exact coordinate error
fractions and exact envelopes, not an assertion that a universal tolerance is
acceptable. The per-leaf quantization half-step comes from authored coordinates,
not `quantizationErrorMetres`. These fixtures do not reconstruct every possible
producer rounding path, consumer FLOAT32 transform implementation or arbitrary
geographic root frame. A2 can preserve the stored source geometry without
certifying the source producer's broader accuracy.

Fragmented lines use the pinned primary
[KHR_mesh_primitive_restart draft](https://github.com/CesiumGS/glTF/blob/9811e8407d4533500cfc6b10e3bc408345035a6f/extensions/2.0/Khronos/KHR_mesh_primitive_restart/README.md).
The draft permits maximal index commands for the admitted strip modes and
requires the extension declaration. The driver checks both used/required lists
before excluding restart commands from rendered positions. Its initial core-only
decoder halt was a corrected oracle profile gap, not a product defect. Draft
extension consumer support remains an explicit format gate.

## Concrete representation and ownership choices

Adopt authored source child-array order as `ChildOrdinal`, bounded by the chosen
QUADTREE/OCTREE fanout. Ordinal coordinates own availability and resource
identity; authoritative actual tile bounding-box/error metadata owns spatial
meaning. Do not select addresses from centers, nominal cells, producer branding,
padding claims or a default epsilon. The standalone ordinal design model runs
against all fourteen actual/authored sources, checks child-count admission,
full-detail descendant geometry coherence and availability/content row counts.
It generates no replacement archive and supplies no viewer or conformance
verdict. Terminal external links and semantic override interpretation remain
the independent semantics lane's proof and final consumer gates.

The consumer owns one captured member map, original source tree, semantic
admission and complete generated-name inventory. The subtree codec consumes
validated addresses and rows; it must not infer spatial slots or policy from
source extras. Each external root retains its original child translation once;
its link metadata box is in its parent's frame. Ordered payload slots remain
ordered even for duplicate references. Preserve authored errors as values,
without claiming that rewriting certifies approximation error.

Keep original payload/resource bytes under their original names and put each
alias in the payload's original archive parent. Source report values are
preserved in the legacy output's `sourceReport`, but its original JSON spelling
is overwritten. The replacement should retain captured original report bytes
under one generated root-level provenance name, so relative report resources
keep their base; collision-check that name with the full inventory. This is a
new ownership decision, not a claim that the old implementation already retains
the original report bytes. The report must distinguish copied opaque data from
interpreted resource closure.


Moving an original conversion report to a generated root-level provenance name
preserves its outgoing resource base only. Incoming URI references to the old
`conversion.json`, any other overwritten member, and imported header hash
associations require an explicit admission/retention mapping before production.
Opaque copying does not prove those associations. The finite initial domain
may refuse such references as Unsupported if it cannot preserve them; that is
a declared capability disposition, not a claim they are malformed.

Initially admit the known exercised GLB/b3dm, embedded or archive-relative
buffer/image/structural-schema closure and opaque extras. Optional unknown
extension bytes may be preserved when their referenced resources already belong
to the admitted closure; an unknown extension does not authorize interpreting a
private URI or classifying an unknown-only resource as validated. A broader
retain-all-originals resource policy needs its own admission decision rather
than treating C1's orphan result as a semantic proof. Top-level external schema,
explicit tile/content metadata and content bounds remain separately scoped.

## Resource inventory and sensitive controls

The independent inventory constructs originals, aliases, external documents and
subtree names before writing a conversion workspace. It checks exact and
file/ancestor collisions. Four authored fixtures establish actual legacy
outcomes: a referenced source file named `subtrees` reaches an I/O `File exists`
failure; nested alias exact and ancestor collisions reach a generated-alias
failure; a top-level reserved alias is refused by the existing prefix guard.
In each case the source and forced prior output bytes remain unchanged and the
correct `.tiles-work-` prefix check finds no residual workspace after return.
The `subtrees` case is an executed preparation defect relative to the draft
preflight inventory invariant. The alias cases demonstrate late legacy refusal;
final cleanup is not evidence that no workspace was created. Source inspection,
separately from execution, locates these collision checks after `Job::begin`.

Eleven independent semantic/resource mutations are rejected: availability swap,
box row, error row, extras row, external transform, alias bytes, relative alias
base, missing external buffer, missing schema, changed resource bytes and orphan
resource inventory. Wrong metadata/resource controls retain plausible framing so
their intended invariant supplies the failure. Four collision cases and four
distinct admission cases complete the nineteen controls per run. C1 source
admission is supplementary structural evidence only; the oracle never calls
`validate` or `implicit::expand_tileset` as its truth source.

The archive metrics in results.json are observed fixture sizes/counts, not
accepted ceilings. No near-limit workload or asymptotic/RSS/descriptor test runs
in this lane. Source capture/resource limits belong to the separate capture
audit. This evidence covers local translated frames, small bounded sources,
selected scalar/string metadata and one reference compression codec. It does
not exhaust all LAS formats, native OGR drivers, URI schemes, unknown extensions,
arbitrary metadata types, rotations or geometric validity/approximation.

## Replay and remaining gates

Use a new designated `/tmp` directory and the pinned meshoptimizer reference
decoder. Missing/different binary or codec identities fail; nothing is skipped.

```sh
python3 -B bench/architecture_audit/implicit_rewrite/a2_real_sources/probe.py \
  --mode portable \
  --binary /home/bend/.cache/rusty-tiles-f1d2-evidence/rusty-tiles-portable-4553533 \
  --work /tmp/rusty-tiles-a2-probe-artifacts-replay-portable
```

Native replay uses `--mode native` and the matching frozen native executable.
The driver applies `nice -n 10`, `RAYON_NUM_THREADS=2` and vector `--jobs 2`,
permits one converter at a time, sets bounded subprocess timeouts and reaps
children. It follows the current CPU-load authority and performs no Cargo/build
work. Replays change path-dependent receipts/archive bytes; fixture, source,
driver, codec and artifact identities remain independently comparable.

Before production acceptance: implement and independently decode the ordinal
plan on the final source/artifact; exercise all retained real-source cases through
Rust/CLI/installed Python; prove authoritative semantic overrides, external links
and actual coarse/fine viewer ownership/picking; prove source capture, limits,
all required fault/cleanup/publication paths and unknown-resource policy; then
obtain separate nonauthor final review. None of the retained use cases can be
silently removed by an exact-cell pilot. No legacy family removal or release is
authorized by this audit alone.
