# Additive P3 source topology and sensitivity preparation

This package does not change the frozen `b2990c` generated-source preparation.
Only Python syntax and static identities were checked by its author. Actual
producer, decoder, compiler and oracle execution belongs to root, with two
actual CPU cores, nice10 and serialized workloads. These scripts perform no
producer/decoder/Cargo/install/fetch or child-process launches.

`pair_by_address.py` is frozen at
`6009de84ed3c3d53efab26e7c47a903e23e6c7a4ba1c46c2b56f1d744fbe84fd`.
It corrects the executed nonunique-association-selector failure while retaining
the original failure receipt. It independently reads authored tileset root,
child ordinal, content/contents slot and external-root-hop addresses. It pairs
resources at these addresses, then compares hierarchy/frame facts, wrapper
kind, semantic and raw-value associations, and every full logical view byte.
Member filenames locate resources within each archive; they never establish
cross-archive correspondence. No candidate-byte-based match selection occurs.
Root has separately executed its seven sensitive model controls and the actual
filled portable pair (16 content slots, 160 logical views). Those are root
receipts, not author executions; their exact paths and hashes must accompany
final acceptance. The old comparator and its nonunique-match failure remain
immutable.

The selected metadata framing requires **producer writer REWORK**. The actual
root-run typed portable raw GLB has JSON length2696, absolute JSON end2716,
BIN start2724 and GLB end4272: the first two fail absolute-eight boundaries.
The existing ordinary writer's core4 mechanism is mechanically suitable for
core GLB; it is not grandfathered into the selected structural metadata8
profile and C1 did not retroactively certify that profile. Current fill wrapping
separately invokes eight-byte alignment. Original bytes/identities must stay
retained while a newly authored producer bridge satisfies the selected policy;
identity compression cannot silently repair an old source's framing.

`decoder-location.json` records the existing cached CommonJS artifact usable by
frozen `consumer.mjs`. Its exact observed SHA and NPM package integrity are
separate from primary source revision evidence. The prior exact A2 pin refers
to an ESM JavaScript reference implementation, not this CommonJS WASM module.
Installed golden-test bytes match the primary revision; that does not prove a
Git revision for the entire cached decoder artifact. Do not erase this lineage
limit when recording actual root runs.

`author_controls.py` contains literal authored point, two-line restart,
polygon-with-hole, multiple-polygon and core-fill controls. The hole has eight
literal vertices and eight literal triangles covering area60; the source's
outer area64 minus hole area4 is independent truth. Raw integer streams, loops,
triangle offsets, padded IDs, table source identities and absolute8 framing are
authored without a production encoder. Nine mutations change a restart into a
joined line, remove the required restart declaration, alter feature ID/table
association, alter a finite POSITION or its min/max, scramble a loop vertex,
misassociate triangle offsets, or omit a triangle through degeneracy. Expected
failures are source-oracle failures. A preservation-only codec need not inspect
these rendered semantic values; it must preserve admitted values exactly.

`topology.py` independently reads actual GLB/b3dm, raw view bytes and decoder
outputs, source identity STRING offsets, POSITION/ID/index accessors and draft
topology associations. It checks authored point/line cardinality and partition,
per-topology source ownership, polygon loop/triangle offset associations,
declared min/max against decoded values, exact rational edge metrics, selected
planar triangle area and each polygon's own triangle/loop area. It admits only
the generated positive diagonal node scale/translation profile, no rotations,
matrices, sparse accessors or arbitrary plane arithmetic. For quantized sources,
coordinate comparison with original source metrics is disabled: counts,
associations and triangle-to-loop consistency remain distinct from quantization
accuracy. Ring edge metrics are reversal-invariant. No absolute winding,
triangle union/overlap/complete coverage, CRS/world-frame accuracy, global scene
truth or complete extension conformance is certified. Degeneracy or an unproved
source profile fails this focused oracle; it is not a proposed product error
classification.

Root materialization and small literal source-control commands:

```sh
nice -n 10 python3 -B /tmp/rusty-tiles-vector-p3-sensitive-preparation/author_controls.py --work /tmp/rusty-tiles-vector-p3-topology-controls-root
nice -n 10 python3 -B /tmp/rusty-tiles-vector-p3-sensitive-preparation/topology.py --input /tmp/rusty-tiles-vector-p3-topology-controls-root/hole.glb --source /tmp/rusty-tiles-vector-p3-topology-controls-root/hole.source.geojson
```

The manifest lists five positive and nine negative cases; root should execute
them serially and retain status/stdout/stderr/hashes. Against actual generated
geometry, use `--extracted ACTUAL.extracted --source geometry.geojson`; for
compressed geometry use the separately pinned-consumer `.checked` directory.
Use `--quantized` only with quantized source. Full logical view bits/padding and
raw association lexemes are checked by the address comparator independently of
these metric checks. Do not use the source oracle to authorize hidden rewriting
or to turn quantization into a preservation guarantee.

`typed_sensitivity.py` mutates an actual uncompressed nine-row typed raw GLB and
tests the frozen independent extractor against literal source states. Seven
controls alter an INT64 bit, unused BOOLEAN tail bit, UTF8 byte offset, exact
large integer within a JSON list string, source identity, padded U16 feature ID
bit, or feature/property-table association. The source input is never modified;
mutated GLBs and rejection receipts go to a fresh external directory. It keeps
core4 framing intentionally because these are typed semantic controls, not
selected metadata8 profile acceptance controls. Root's command is:

```sh
nice -n 10 python3 -B /tmp/rusty-tiles-vector-p3-sensitive-preparation/typed_sensitivity.py --input /tmp/rusty-tiles-vector-p3-inputs-small/typed-metadata-portable-raw.extracted/payload-0000/source.glb --expected /tmp/rusty-tiles-vector-p3-inputs-small/typed-metadata.expected.json --work /tmp/rusty-tiles-vector-p3-typed-controls-root
```

Use the actual payload folder from the extraction receipt if its ZIP ordinal
differs from0000. No success or rejection is claimed before root executes and
binds receipts. Root already proved the unmodified typed values/offsets and
source identities on the raw/compressed pair; the mutations establish that
specific independent acceptance is sensitive to these changes.

`large_ids_revision.py` is an additive argv-only correction to the retained
original large-ID recipe. Root executed that original32MiB recipe: both targets
completed, but four leaf tables held16385/16384/16384/16384rows with U16 IDs,
and the independent f32 expectation failed. `Feature::estimate` includes2048
bytes per feature;65537*2048=134219776 exceeds its2*32MiB source partition
guard, even without coordinate/property costs. Even2*64MiB misses that lower
bound by2048. The correction raises the **existing producer** maxBytes option
to128MiB, leaving source/expected bytes/count/70000feature+vertex limits intact.
It bounds this literal point source's estimate above by155191616 using at most
one rendered and one intrinsic point plus256property bytes, below the revised
268435456guard. This is neither codec default/source/working-budget widening
nor RAM/time acceptance. Actual encoded source is expected to remain small;
root must still execute and observe the real one-leaf65537f32 branch and exact
raw/meshopt preservation. Original results are never relabeled or overwritten.

```sh
nice -n 10 python3 -B /tmp/rusty-tiles-vector-p3-sensitive-preparation/large_ids_revision.py --source ORIGINAL-large-ids-65537.geojson --expected ORIGINAL-large-ids-65537.expected.json --work /tmp/rusty-tiles-vector-p3-large-ids-revision-root
```

The fresh receipt binds original source/expectation hashes, source estimate
owner hashes, binary identities and four exact corrected producer argv. Root
chooses/schedules these commands serially; the revision script launches none.
