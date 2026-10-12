# H8 leaf producer and remaining acceptance gates

This branch adds owned full-attribute and image capture over the merged H1
tree, a frozen geometry lattice, crop/pack atlases and an actual in-process ML1
caller. `mesh-leaf-payloads` publishes leaf GLBs and a diagnostic manifest;
it does not publish a tileset hierarchy. Producer acceptance is unfinished.
The encoder API is on draft #163 and remains coordinated with the completed
H8 producer. The user owns CI monitoring and merges.

`src/mesh_large/full.rs` captures attributes, every declared image, material
bindings and samplers before releasing source ownership. It checks the exact
H1 triangle-rank projection, joins every corner to H1's positional minimum
depth through bounded spills, then freezes the dataset-wide zero-origin
lattice. Logical image aliases remain distinct. Encoded-image hashes bind the
owned snapshots; source stability checks establish observed file identity,
rather than an atomic cryptographic snapshot of all source files.

Full capture preflights the larger 286-byte records, retained streams, corner
join, image chunks, metadata and serial decoder ownership. Its conservative
scratch allowance adds `863N + 286N + 189N` bytes to the existing H1 allowance,
plus images and physical allocation reserves. The known-memory check includes
the scratch allowance on memory-backed filesystems and applies to fixed job
counts too. These calculations and decoder limits still require measured
producer RSS/scratch acceptance; they are not an allocator-wide memory proof.

`leaves/geometry.rs` owns the immutable all-depth plan and aligned U16 affine
representation. Focused tests cover mixed incidence, packed/synthetic depths
through 31, negative positions, global lattice indices beyond `2^24`, exact
terms, rounded sums, identity flattening and outward displacement/bounds.
Actual source-derived snapped coordinates are exactly representable as f32.
The GLB writer promotes certified f32 scale/translation to f64 before
JSON serialization: a shortest f32 decimal need not preserve its exact value
when the independent validator parses it as f64. Geographic placement and
renderer RTC arithmetic remain separate proof gates.

`leaves/mips.rs` reduces colour in linear light with linear alpha, normal maps
in tangent space with per-level renormalization, and data channels linearly.
It covers odd dimensions and one-dimensional chains to 1x1. Normal cancellation
uses the greatest-overlap unit donor with stable ties. Its scalar MASK helper
reports attainable discrete coverage. Uniform image support is its domain;
sparse atlas support, gutters and variable vertex-alpha contexts require the
atlas owner described below.

The independent source-authored mip reference in `mip_oracle_tests.rs` does not
use the production filter arithmetic. Regular tests exercise class differences
and sensitive wrong-filter controls. Ignored delivery probes are deliberate
coordinator executions, not default-CI acceptance. The initial delivery
inventory fails 16 unchanged class/profile floors. Its fixtures and failures
remain preserved. Bounded probes investigate UASTC colour, meaningful-channel
occlusion replication and alpha guards. Success on those fixtures does not
establish a general encoder recipe. A coupled vertex-alpha counterexample
explicitly exposes where the proposed one-byte guard changes source coverage.

Bulk precode, source reviews, execution logs and consumer probes are held at
`~/.cache/rusty-tiles-evidence/h8-leaves-20261013/`. The tracked
`foundation-manifest.json` binds retained source, evidence and build identities.
The complete revised H12/H13 planning documents remain in their corresponding
`tiler-lod-design-20261013/` and `tiler-bake-design-20261013/` cache directories.
Planning GO is distinct from approximation/baking implementation acceptance.

The pinned GPU consumer probe accepts dense output `TEXCOORD_0..4` and two
feature attributes (11 attributes against its limit of 16), with independent
glTF validation and role-binding/raster controls. Its constant-UV, FLOAT,
base-PNG/NEAREST fixtures establish layout and sampling closure only. The
occlusion diagnostic checks sampled material occlusion separately because the
pinned standard PBR fixture is insensitive to occlusion. It proves neither
general-client support beyond two UV sets nor feature picking, atlas mip/LOD
equivalence, quantization or normal-frame preservation. Its loopback server and
private browser context are closed; no product viewer is installed.

The producer emits dense per-slot UVs, original material factors/normalScale,
source numeric picking metadata, required meshopt/quantization extensions and
decoded content bounds. Only actual byte-budget failures permit lossy density
retries; texture-fidelity failures abort publication. Physical crop/page planning
is frozen before the all-depth geometry lattice. The tree's later scratch check
includes retained full attributes and encoded images.

Lossless PNG preserves decoded RGBA8 at level zero only, including transparent
RGB and original normal bytes. Viewer-generated PNG mip levels are outside its
texel-exact guarantee. KTX2 profiles carry complete caller-authored class chains
and independently decode/check every level. Finite MASK checks preserve original
vertex alpha and report the fixed semantic attainable gap; an optional one-byte
alpha guard requires unchanged classifications at every checked geometry sample.
This is a finite sample guarantee, not continuous surface coverage certification.

Executed small published-artifact checks establish all five slots, complete KTX
chains and unchanged class floors on an opaque low-variance D0 fixture for all
three KTX profiles. PNG has an independent raw-texel check. Numeric GPU picking
works in the pinned consumer, including reflected geometry. These fixtures use
non-mip samplers. A new source-LOD counterexample exposes crop phase error and
coarse mixing between packed charts for mipmapped samplers; the composite-chain
quality oracle alone cannot certify source minification correspondence. The corrected mip-sampler path uses dedicated aligned pages and original-source
class chains, with finite decoded-geometry sampler checks. Independent artifact
checks have passed repeat, mirror and clamp fixtures; final checks remain pending.
The reflected PBR diagnostic was traced to reference winding: independently
flattening the source resolves its interior discrepancy. Original failed
measurements remain preserved, including fast/balanced MASK-edge differences.

Before H8 can complete, independently verify source sampler mip/LOD support,
normalScale/filter correspondence and MASK coverage with original COLOR_0.
Coverage discreteness must be established from an independent fixed semantic
mip, never from the compressed candidate itself. Bind all five slots, every
class/mip and each profile to decoded artifacts with unchanged quality floors.
Run the 10-million textured producer with encoding/assembly resource receipts,
mixed-depth/packed decoded seam witnesses, deterministic output controls and
final GPU acceptance. Future implicit hierarchy integration
must use actual decoded content bounds through the existing metadata override
contract, rather than treating centroid assignment cells as containing geometry.

The implementation is published as a draft while final-source checks run. The
first actual 10-million producer execution emitted 1,024 leaves in 60.95 seconds
with measured peak RSS of 200,511,488 bytes. Its independent geometry/provenance
check passed all triangles; this constant-texture fixture does not establish
high-frequency texture quality. The resource wrapper falsely flagged unrelated
concurrent cache directories, so its original failure is preserved and a final
isolated repeat is required. Mixed-depth, packed and synthetic-depth seam
artifacts pass all four profiles. Final resource, decoded texture and packaging
receipts will supersede these provisional results without deleting failures.
