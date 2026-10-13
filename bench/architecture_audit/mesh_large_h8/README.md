# H8 full-detail leaf producer

H8 produces actual full-detail GLBs through the in-process ML1 encoder. The
hidden `mesh-leaf-payloads` command and exported `mesh_to_leaf_payloads` Rust API
publish leaf payloads, decoded bounds/error certificates, source picking metadata
and diagnostic manifests. The recorded baseline establishes the bounded producer
acceptance below; reviewed fixes are recorded separately at the end. Implicit
hierarchy/archive assembly is a later integration slice. The branch includes
the caller-mip API commits from #163. The coordinator
plans to merge #187 alone and close #163 as superseded. The user owns CI
monitoring and merges.

The accepted baseline producer source is commit `b81dd9430039f00a5595ddb65e11cb688dbad10e`.
[producer-acceptance.json](producer-acceptance.json) binds its exact source and
binary identities, independent reviews, executed artifacts, controls and limits.
Bulk receipts live at `~/.cache/rusty-tiles-evidence/h8-leaves-20261013/`; the
manifest records their SHA-256 values. The earlier
[foundation-manifest.json](foundation-manifest.json) remains historical evidence.

Since H16 (#192) every profile's leaf budget is 8,192 triangles (CesiumJS
property-texture width), so `fast` and `balanced` trees changed shape and the
figures below describe the earlier 16,384-triangle `balanced` budget; the same
mesh now gives 1,959 balanced leaves.

The final balanced 10-million-triangle run emitted 1,024 leaves, totaling
1,334,467,104 GLB bytes, in 71.13 seconds. Its measured high-water RSS was
219,254,784 bytes and sampled physical scratch peak was 6,555,504,640 bytes.
Every member stays below 96 MiB; the largest is 1,313,348 bytes. All 10 million
triangles retain the independently accepted geometry/provenance proof through
strict byte identity of every complete GLB. Only two named allocation-accounting
receipt fields differ. All 5,120 KTX pages and 20,480 stored mip levels pass the
independent decoded-image gate. This scale fixture has constant five-slot
textures and establishes no general high-frequency appearance guarantee.

A separate reproducible five-image 2048x2048 fixture exercises ten physical
pages, including 4096-wide pages, with an explicitly planned density drop of two.
Its measured RSS is 558,587,904 bytes; all 124 decoded levels pass unchanged
class floors. Resource checks use an empty PATH, exact input/binary identities,
owned scratch cleanup and fresh isolated output parents. Periodic scratch
samples are non-atomic. Native workspace charges remain provisional policy,
rather than a universal allocator or worst-case resource proof.

Full capture owns attributes, every declared image, material bindings and
samplers before releasing source ownership. It checks the exact H1 triangle-rank
projection and joins every corner to its positional minimum incident depth
through bounded spills. Logical image aliases remain distinct. Physical page
planning precedes the immutable dataset-wide zero-origin lattice; leaf writes
cannot later change it. Exact decoded terms and rounded sums support large
lattice indices, packed and synthetic depths through 31. The writer promotes
certified f32 TRS terms to f64 before JSON serialization and bounds actual decoded
positions. Executed mixed-depth, packed and synthetic seam witnesses pass all
four profiles, with sensitive crack controls.

PNG lossless preserves decoded source RGBA8 at level zero, including transparent
RGB and original normal bytes, original factors/normalScale and vertex colours.
The independent raw-texel artifact check passes. Viewer-generated PNG mips are
outside the lossless guarantee. Fast, balanced and quality instead supply complete
caller-authored KTX2 chains: linear-light colour with linear alpha, tangent-space
normal filtering and per-level renormalization, and linear data filtering. The
encoder never regenerates these chains. Independent decoding checks every level
against fixed profile/class floors on meaningful material channels.

Mipmapped bindings use dedicated, aligned pages derived from original-source
semantic chains. Sixteen barycentric centroids, three vertices and three edge
midpoints per decoded triangle provide finite source/page sampler checks across
quarter LODs. Independent artifacts cover repeat, mirror, clamp and all four
published mip filter modes: 150 pages, 1,080 stored-level checks and 707,520
sampler evaluations, with seven controls per run. The exact rational reference
keeps discontinuous nearest boundaries and adjacent f32 UV controls. Its scope
is exact geometric sample points and finite isotropic footprints, not all camera
positions or GPU interpolation precision. The final binary reproduced all fifteen
witnesses with byte-identical GLBs.

MASK filtering uses an independent fixed semantic mip and original vertex alpha
to establish attainable coverage. A one-byte alpha guard is permitted only if
every checked geometry classification remains unchanged. Literal material
factors and normalScale are retained. Coverage is finite-sample certification;
continuous coverage and effective normal shading under arbitrary filtering are
separate claims. Only physical byte-budget failures permit lossy density retries;
fidelity failures abort publication. Executed refusal and worker-count/scratch
path determinism controls pass.

Actual compressed GPU payloads pass stored and fractional-LOD checks with the
matching sRGB colour target; maximum meaningful error is two for fast/balanced
and one for quality on these fixtures. Their KTX colour DFDs correctly specify
sRGB. The pinned Cesium consumer chooses UNORM while ignoring the DFD transfer
function, so its high-contrast linear-light colour filtering fails. H19 reproduced
this on ASTC, BC7 and ETC2; a generic uncompressed KTX2 path also fails in a
diagnostic outside the legal Basis glTF binding. Matching sRGB targets remove the
measured texture-sampling error. A full material fix must coordinate hardware
transfer with Cesium's existing post-sample decode to avoid double conversion.
The draft investigation is retained at
`~/.cache/rusty-tiles-evidence/h19-cesium-srgb-scope-20261013/report.md`, SHA-256
`4ca7085ed6317de75709283d0eb9b9ea96640df4353b3705d1734ce4595d89c0`.
That compatibility failure is retained; no producer compensation or relaxed
floor was added. Numeric source picking and normalized reflected-source model
checks pass separately. No general renderer, geographic RTC, continuous normal
frame or all-camera appearance certification is claimed.

Final verification: 732 portable and 799 native library tests pass, both
all-target Clippy configurations pass with warnings denied, and the installed
wheel plus checksum-pinned official Blender 4.5.14 each pass 45 API tests with
an empty PATH. Wheel licence notices and the tracked-tree budget pass. These are
local checks; CI monitoring and merging remain with the user.

The independent oracles, witness generators and controls in this directory are
retained for later slices. Original failed codec, sampler-reference, resource,
GPU and receipt-construction runs remain in the cache. H12/H13 planning documents
are in their respective `tiler-lod-design-20261013/` and
`tiler-bake-design-20261013/` cache directories; their planning GO does not certify
LOD approximation or proxy baking. Future hierarchy integration must use actual
decoded content bounds through the metadata override contract, rather than
centroid assignment cells. No permanent viewer is installed.

The independent #187 review found missing ordinary-CI regression controls. The
review-fix candidate adds off-lattice decoded seam checks, typed fidelity-versus-
budget retry checks, five-slot KTX DFD checks and a nonignored current-production
128x128 per-class/profile/every-level codec gate. Passing policy probes now run
in CI; the abandoned 16-violation policy is explicitly a historical diagnostic.
The focused suite passes and executed seam, fidelity-retry and colour-transfer
mutants fail their ordinary tests.
[review-fixes-acceptance.json](review-fixes-acceptance.json) binds these fixes,
executed source `87cfb9b`, equivalent rebased source `efd084c`, and their exact
receipts. The rebase onto `820256b` adds only upstream bench files; all 264
recorded compiled source files retain identical hashes.

The native library suite passes 830 tests; both all-target Clippy configurations
pass. The full portable invocation records 762 passes and one spill VmHWM
assertion failure. That same test passes in isolation without a source or
threshold change; the failed invocation remains preserved. This is not reported
as an entirely green portable run. The installed wheel and checksum-pinned
official Blender 4.5.14 each pass 46 API tests. Licence notices pass. The new
binary reproduces all fifteen accepted explicit-filter artifacts with complete
GLB byte identity; a fresh authored-omission witness emits minFilter9987 and is
byte-identical to its accepted explicit9987 counterpart. The old scale/resource
runs remain the separate producer baseline and were not repeated with this
review-fix binary.

An unspecified source minFilter now resolves to explicit LINEAR_MIPMAP_LINEAR
(9987), so conforming viewers use authored chains. Explicit source filters remain
unchanged. This is a producer policy, not a claimed glTF default. For lossless
PNG, viewer-generated coarse mips can also blend neighbouring packed crops:
level-zero exactness does not prevent this distant minification bleed. The
nonconstant 128x128 codec witness extends current-CI coverage; nonconstant D2
and NPOT end-to-end artifacts remain a milestone-4 gate.

H4's source-bound metadata diagnosis identifies a separate consumer integration
gate: Cesium's structural-property texture width equals table rows and can exceed
the pinned device's 8192 limit at H8's maximum leaf size. UINT64 scalar columns
still use that GPU path through downcasting; CPU source provenance must remain
exact. Planned per-primitive/table partitions and actual property/picking checks
belong to the follow-up. This is a forward compatibility risk, rather than a new
executed H8 browser failure. H16 must consume frozen leaves without late splits
or lattice changes.
