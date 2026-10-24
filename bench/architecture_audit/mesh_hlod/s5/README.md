# S5 validator and independent HLOD acceptance

Sampled G is finite, nonnegative and ordered; it is not a certified maximum.
C1 still owns resources, hashes, codecs and accessor admission, C2 owns decoded
bounds, and I1 owns implicit availability. No H17 certificate is required for
HLOD. `geometricErrorAccuracy` remains explicitly uninspected. Source-distance
sampling is the separately invoked `../s2/audit.py`, never an archive certificate.

Retain the bounded validator/facts/replay owners. Add source-identity admission
and a disk-backed decoded delivery relation; replace ad-hoc seam probes with
permanent independent controls. The census harvests E2 `816d0670`'s decoded leaf
incidence/ancestor cross-product and extends it to corner and T contacts. No
producer facts, lock files, reports or producer imports supply independent truth.

## Archive validation

`src/validate/identity.rs` projects only owned C1-admitted raw JSON, preserving
opaque overflowing extras and exact integer lexemes. It validates every present
identity column, unsigned scalar representation, counts/ranges, null IDs and
triangle-constant feature IDs. Unsupported metadata modifiers and declared local
`.gltf` identity fail closed. Views are cached within the document decode limit;
repeated scans are preflighted through the shared payload-element budget.

`src/validate/hlod_delivery.rs` stores hierarchy, references, original vertex
correspondence and decoded boundary incidences in scratch SQLite. A partial
identity strip is rejected regardless of traversal order once source identity is
expected. Original triangle IDs must address the matching primitive identity and
use rows one-to-one. Coarse `source_triangle` is refused under D4; same-payload
repeated cells do not change the payload's role. Semantic NORMAL/TEXCOORD accessor
shapes are checked. Archive correspondence does not establish external source truth.

The native hierarchy includes the explicit walk, external-document placements,
I1's expanded availability walk and memo replays. `replaceContentContinuity`
rejects ordinary contentless cells between a REPLACE renderable ancestor and its
renderable descendants. Leading empty roots and external-link placeholders are
valid, as are S1's repeated coarse-content cells. An empty ADD override cannot
clear a barrier owned by a renderable REPLACE ancestor.

Only open/nonmanifold decoded edges survive each bounded payload reduction.
A disk R-tree joins leaf contacts. Ancestor checks require consecutive border
subsegments, corner/T points and matching interface incidences; root exterior
incidences also have descendant orientation witnesses. Keeping just a T vertex
cannot conceal a detour. Every candidate/interval visit and SQLite VM work is
charged; dense queries refuse rather than truncate. Decoded quantization spacing
and transforms supply tolerances, never producer error summaries. Native winding
checks cover interfaces/coincident exterior witnesses, not every isolated interior.
Fresh coarse UV/source-pixel correspondence cannot be inferred from leaf UVs.

## Independent real-archive oracle

`archive.py` reads directories or `.3tz` directly and expands explicit, external
and implicit trees, including subtree availability/metadata. `continuity.py`
uses that same independent expansion and produces a named verdict for implicit
and legacy archives. `decode.mjs` accepts paths or GLB bytes on stdin, decodes
POSITION/NORMAL/TEXCOORD_0 and selected-scene matrix/TRS transforms, and derives
position/normal/UV tolerances independently from scalar storage, codec bits and
transforms. UINT64 columns cross JSON as decimal strings and become Python exact
integers. URI-shadow buffers and unknown BIN replacements are refused; required
meshopt virtual fallback buffers are supported.

`meshopt.py` uses the retained independent Khronos-spec codec in
`tests/ext_meshopt_spec.py`, plus specification-based OCTAHEDRAL, QUATERNION,
EXPONENTIAL and index decoding. No npm package or producer decoder is required.
Unsupported scene extensions, animation, skin, morph or UV1 source expectations
fail closed.

`source_manifest.py` independently reads the input GLB with mmap and streams
original geometry, vertex IDs, normals, UVs, material classes/constants and source
order to SQLite. Source keys and ordinals remain exact. It imports no producer.
The default root-relative glTF frame removes common archive placement and applies
relative tile transforms; explicit world frames require a documented 4x4 source
transform. Source manifests can also pin a separately accepted baseline, but the
real campaign uses the original input GLBs.

`oracle.py` checks source leaf multiplicity/IDs/geometry/normals, materials and
nearest coarse representatives, with D4 ties in source order. Analytic manifests
check exact UVs/materials. Real textured leaves may legitimately crop/repack UVs
and texture bindings: their material class/PBR constants/slot presence and
rendered UV structure are checked, while UV/source-pixel correspondence is
explicitly counted as uninspected. No large epsilon substitutes for image proof.

`spatial.py` keeps archive-sized geometry/indexes on disk. R-tree candidate rows,
leaf-box broadphase visits, descendant interval tests, segment-chain candidates
and nearest-source queries have charges and caps. The T census requires all
original split-side subsegments even when another contact point interleaves.
It enumerates disjoint same/mixed-depth ancestor pairs without retaining an
archive-sized pair cross-product. Full 2048 CPU/RSS and pair counts remain S8
measurements, not extrapolated acceptance claims.

## Reproduction and evidence

All builds, tests and bakes use the required scope:

```sh
systemd-run --user --scope -q -p MemoryMax=10G -p MemorySwapMax=0 -- chrt -i 0 nice -n 19 COMMAND
```

Set `CARGO_TARGET_DIR=/home/bend/.cache/rt-wt/target-hlod-s5`,
`CARGO_BUILD_JOBS=3`, and both dev/test debug settings to zero for Cargo.
`cargo test --test hlod_acceptance` regenerates the permanent corpus and checks
all generated members against `controls-sha256.json`; positives assert that
`hlodFeatureIds` and continuity actually ran. Rust/CLI verdicts must agree.
Python 3 and Node are needed. `python3 controls.py SCRATCH` runs the independent
named controls without Rust. The final native and real-archive receipts follow
in the execution record below.

From the repository root, inside that scope:

```sh
python3 bench/architecture_audit/mesh_hlod/s5/source_manifest.py INPUT.glb SOURCE.sqlite
python3 bench/architecture_audit/mesh_hlod/s5/oracle.py OUTPUT.3tz SOURCE.sqlite --scratch SCRATCH --profile hlod
python3 bench/architecture_audit/mesh_hlod/s5/continuity.py OUTPUT.3tz
```

Use `--profile h17` to audit retained legacy archives without applying D4's new
coarse representative identity policy. Source leaves and geometry/material/
winding/interface checks remain active. A named failed geometry check is a
verdict, not a decoder crash or legacy certificate interpretation.

The earlier S5 receipts at `33ce60c5` are historical and superseded by this
review revision. Their cache remains
`/home/bend/.cache/rusty-tiles-evidence/hlod-s5-20261024/`, bound by execution
manifest `afd9e6fa265fc9c2e1659186304a9f22bce41d6a6436b77196435a9f77a3cb72`
and review manifest `6f94c1c7f748799bcc273f8fddc1a178f2b6e0670ee3596882e0c19247034b13`.
The old retained tex3's eight continuity failures remain reproducible; fresh
latest-S1 output is measured separately. No browser or 2048 bake runs in S5.
