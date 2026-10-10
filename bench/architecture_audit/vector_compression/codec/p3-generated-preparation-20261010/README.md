# P3 generated-vector source preparation

Static preparation only. No producer, codec, consumer, Cargo, target or Git
execution occurred in this lane. The accepted producer source is unchanged:
all 97 production hashes in the stride-final-build source pin match the current
checkout. The binaries are accepted before-codec artifacts; their vector writer
has not gained codec acceptance from C1. Root owns scheduled executions.

`prepare.py` authors nine typed metadata rows, separate point/multiple-line/
restart/polygon-hole/multiple-polygon geometries, and one oversized polygon
with a hole expected to reach genuine filled b3dm fragments. It emits exact
commands for portable/native raw, quantized, meshopt, quantized-meshopt runs.
The optional 65537-identical-point source raises genuine producer limits to
70000 features/vertices and 32MiB bytes, so the f32 feature-ID branch can be
reached without geometric hierarchy partitioning. This is a scheduled larger
case; no claim that its actual runtime or admission is established yet.

The metadata oracle starts from authored typed states: BOOLEAN including final
bit padding, signed INT64 extremes and integers above 2^53, FLOAT64 quarters,
UTF8 and UINT32 byte offsets, typed null/noData, and JSON list strings containing
ordered nested lists, exact large integers, UTF8 and null. It does not prescribe
the producer's missing-value sentinel. The binary reader recovers physical
values and independently maps the declared noData back to missing values. No
nullable BOOLEAN or producer UINT64/structural-array claim is made. A separate
independently authored external GLB contains UINT64 and variable INT64 arrays;
its exact raw views and typed states are a preservation fixture, never a
generated-source claim.

The root can begin with these lightweight input-only commands:

```sh
nice -n 10 python3 -B /tmp/rusty-tiles-vector-p3-generated-preparation/prepare.py --work /tmp/rusty-tiles-vector-p3-inputs-small
```

The emitted `manifest.json` binds inputs, expectations, all 97 source files,
binary hashes and exact argv. `root-producer-commands.sh` is a command inventory;
run selected lines serially under an actual two-CPU taskset. Each producer has
`--jobs 2`, `RAYON_NUM_THREADS=2`, `PROJ_NETWORK=OFF`, and nice10. Wrap each small
command with `timeout --signal=TERM --kill-after=5s 90s` under the coordinator.
Start with typed-metadata portable raw/meshopt, then geometry portable
quantized/quantized-meshopt, then native counterparts and genuine fill cases.
Do not run the entire command inventory as an unscheduled batch. Preserve
stdout/stderr, exact argv/status, binary hash, source manifest hash, input and
output hashes, monotonic elapsed time, and before/after source identities in
the coordinator execution receipt. This package does not launch processes or
manufacture execution receipts.

Extraction commands are supplied per run. For a compressed output, first
extract framing/encoded streams, then run the supplied consumer using the
coordinator's already-pinned upstream CommonJS MeshoptDecoder module:

```sh
nice -n 10 python3 -B /tmp/rusty-tiles-vector-p3-generated-preparation/extract.py --archive OUTPUT.3tz --expected CASE.expected.json --out OUTPUT.extracted
nice -n 10 node /tmp/rusty-tiles-vector-p3-generated-preparation/consumer.mjs OUTPUT.extracted /path/to/pinned/meshopt_decoder.js DECODER_SHA256
nice -n 10 python3 -B /tmp/rusty-tiles-vector-p3-generated-preparation/extract.py --archive OUTPUT.3tz --expected CASE.expected.json --out OUTPUT.checked --decoded-root OUTPUT.extracted
nice -n 10 python3 -B /tmp/rusty-tiles-vector-p3-generated-preparation/compare.py RAW.extracted COMPRESSED.checked
```

The decoder path/hash and pinned upstream revision must come from root before
execution. No install or fetch is authorized here. Upstream decoder and Rust
meshopt share algorithm/kernel lineage: this tests independent consumption and
exact preservation, not a second mathematical codec implementation. Compare
raw versus meshopt and quantized versus quantized-meshopt separately; lossy
quantization is never treated as byte-preserving compression.

`extract.py` uses stdlib ZIP solely as transport, checks CRC and unique names,
then reads literal GLB chunks/views/accessors/metadata without product imports.
It retains original inner GLB bytes, raw JSON, BIN, raw value lexemes at metadata,
node, primitive-extension, material and extras associations, decoded view bytes,
integer IDs and topology streams. `compare.py` matches preserved association
graphs independently of hashed content filenames and compares all logical view
bytes, including padding, plus preserved raw association lexemes. No C1 success
or old vector oracle supplies expected bits.

Framing is an observation, not repaired input. The current ordinary generated
GLB writer uses core four-byte chunk padding; fill wrapping separately calls
`align_glb_eight`, so b3dm inner framing may differ. Reports record actual JSON
absolute end, BIN absolute start/end, declared buffer slack and all three
absolute-eight conditions. The codec draft selects absolute-eight metadata
framing from the pinned structural metadata draft's Binary Data Storage lines
523–528; identity input cannot silently be rewritten to meet it. Whether every
actual generated source is admitted unchanged remains an executed P3 gate.

Exact primary bytes remain in the checkout's
`bench/architecture_audit/vector_compression/codec/primary/`: core glTF, structural
metadata, 3D Metadata, mesh features, polygon and primitive-restart drafts, with
`primary-receipt.json` revisions and hashes. In particular, maximum U32 indices
are legal line restarts only under the required pinned restart extension;
polygon loop restart semantics belong to the separate pinned polygon draft.
The extractor checks those declarations and raw streams, but does not certify
polygon winding, triangulation coverage/area, scene/world geometry accuracy,
quantization rounding/error bounds, consumer rendering, or full extension
conformance. Those are independent remaining gates. Source-root transform/CRS
and global accuracy are deliberately outside this compression-preservation
oracle; node/raw association retention is checked separately.

Before calling generated acceptance complete, root must observe every required
branch (the extractor requires aggregate identity/mode/restart/polygon coverage), independently
check geometry topology against source rings/segments, run sensitive wrong-byte/
wrong-offset/wrong-association controls, bind all receipts to frozen scripts and
source/artifacts, and obtain nonauthor review. Current scripts are syntax-only
preparation; no oracle self-test or producer execution is claimed.
