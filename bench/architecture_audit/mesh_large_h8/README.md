# H8 leaf foundations and remaining producer gates

This branch adds owned full-attribute and image capture over the merged H1
tree, a frozen geometry lattice, and class-correct whole-image mip helpers.
It does not yet emit leaf GLBs or constitute the first production ML1 caller.
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
A future GLB writer must promote certified f32 scale/translation to f64 before
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

Before H8 can complete, implement and independently verify actual crop/pack
atlases, per-slot UV remapping, source sampler support, sparse class mips,
normalScale/filter correspondence and MASK coverage with original COLOR_0.
Coverage discreteness must be established from an independent fixed semantic
mip, never from the compressed candidate itself. Bind all five slots, every
class/mip and each profile to decoded artifacts with unchanged quality floors.
Resolve the lossless PNG/authored-mip representation policy, emit complete
GLBs with provenance/metadata and actual decoded bounds, charge encoding and
assembly resources, then run the 10-million textured producer, deterministic
output controls and final GPU acceptance. Future implicit hierarchy integration
must use actual decoded content bounds through the existing metadata override
contract, rather than treating centroid assignment cells as containing geometry.
