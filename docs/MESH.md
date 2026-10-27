# Mesh tiling

`rusty-tiles mesh` and Python's `rusty_tiles.mesh_to_tileset` stream one admitted
static glTF/GLB scene, in local metres or a declared source CRS, into a 3D Tiles
1.1 implicit forest in one `.3tz`.
Content is glTF 2.0. The encoder is bundled: meshopt and KTX2/Basis need no
external executable or system geospatial library in the standard CLI or wheel.

The pipeline captures the source, writes exact full-detail leaf GLBs, builds
every coarse parent tile from its children as a classic octree level of detail
(HLOD: [coarse levels](#coarse-levels-hlod)), and publishes the hierarchy. A coarse
tile's geometric error is a **sampled, measured** distance to the source surface,
not a certificate. A successful conversion does not establish the final tiler's
resource, fidelity, scale or consumer acceptance.

## Convert a local scene

```sh
rusty-tiles doctor --command mesh
rusty-tiles mesh -i local.glb -o local.3tz --jobs auto
rusty-tiles validate local.3tz
```

Input and output are filesystem paths. Output must end in `.3tz`; directory
output and `.3dtiles.zip` are not tiler products. An existing destination is a
conflict unless `--force` (or `-f`) is supplied. Replacement installs a completed
archive after required work and callbacks finish. Failure before installation
preserves the previous destination. The output cannot overwrite or alias the
source or its dependencies. Installation does not promise power-loss durability.
The archive declares itself an HLOD delivery (`asset.extras.rustyTiles.lod` is
`"hlod"` in `tileset.json`), so `validate` applies every HLOD check without
being asked to, and refuses a copy whose payload labels were stripped.

## Source and placement

By default the source is **local metres with glTF Y up**. Node transforms are
evaluated as part of capture. Each source POSITION component and each component
after the accumulated node transform must lie within ±1,000,000 metres (source
units with a [source CRS](#source-crs)), before the final f32 rounding. This
finite source domain is separate from the placement precision domain.
Coordinates are never interpreted as geographic or projected because of their
magnitudes, file names or an offset file; a source CRS is always declared.

The admitted source has a default or sole scene, TRIANGLES, bounded local
resources and core PBR materials. It supports authored normals/tangents, UV0/UV1,
COLOR_0, and all five core PBR texture slots. Source PNG/JPEG images remain
supported. Normal maps require authored normals and tangents. Tangent-bearing
geometry requires a conformal accumulated transform, including supported
reflections. Animation, skins, morph targets, alpha blending, imported
extensions/extras/metadata, remote/data image resources and unadmitted semantics
are refused; they are not silently discarded. Dependencies must stay confined
to the source directory, without escaping/absolute/network URIs or symlinks.
Source files must remain stable through capture; change detection is
observational, not an atomic filesystem snapshot.

Without an anchor the tileset stays in its local frame. To place it on Earth:

```sh
rusty-tiles mesh -i local.glb -o placed.3tz \
  --anchor 153.02 -27.47 25 \
  --orientation-xyzw 0 0 0 1 --scene-offset 10 2 -5
```

`--anchor` takes longitude and latitude in WGS84 degrees and **ellipsoidal height
in metres**, including an explicit zero when intended. Longitude must be in
[-180,180], latitude in [-90,90], and all placement numbers must be finite.
`--orientation-xyzw` is an active right-handed ENU quaternion with scalar W last;
its norm must be within 1e-12 of one and is normalized once. `--scene-offset` is
a metre translation in source glTF Y-up coordinates after node transforms and
before orientation. Both require an anchor. Their defaults with an anchor are
identity rotation and zero translation. `--orientationXyzw` and `--sceneOffset`
are equivalent camelCase spellings.

Placement applies one rigid transform at the tileset root. It never converts a
projected CRS, infers source axes, applies an E/N/A shift or converts geoid
heights; use a [source CRS](#source-crs) for projected or geographic sources. A
large finite anchor/offset outside the shared precision budget is
`unsupported`; an invalid number, range or quaternion is `invalid_request`.

## Source CRS

Projected or geographic sources, such as Metashape GLB exports in EPSG:3857
with an `offset.txt` shift, declare their CRS instead of a rigid placement:

```sh
rusty-tiles mesh -i site.glb -o site.3tz \
  --source-crs EPSG:3857 --source-offset-file site_offset.txt
# the same request, spelled out:
rusty-tiles mesh -i site.glb -o site.3tz \
  --source-crs EPSG:3857 --source-axes y-up \
  --source-offset 14812000 -1384000 100 --height-offset 0
```

| Option | Meaning |
| --- | --- |
| `--source-crs` (`--sourceCrs`) | `local` (the default) or a 2D horizontal EPSG, WKT or PROJ definition, resolved by the shared [conservative CRS resolver](FORMATS.md#point-clouds) |
| `--source-axes` (`--sourceAxes`) | Default `y-up` (glTF is Y up): X=E, Y=height, Z=−N. `xyz` is X=E, Y=N, Z=height. Axes are read after node transforms and never inferred from data |
| `--source-offset E N [A]` (`--sourceOffset`) | Added in double precision: E/N in horizontal CRS units, A metres (default 0). Default `0 0 0` |
| `--source-offset-file` (`--sourceOffsetFile`) | A Metashape offset file holding exactly the lines `E: <number>`, `N: <number>`, `A: <number>` in that order (optional final LF or CRLF, at most 4 KiB). Anything else is `invalid_request`; excludes `--source-offset` |
| `--height-offset` (`--heightOffset`) | Default `0`: metres added to source height plus A to give WGS84 **ellipsoidal** height. The default is right only when source height plus A is already ellipsoidal |

A source CRS cannot be combined with `--anchor`, `--orientation-xyzw` or
`--scene-offset` (`invalid_request`), and `local` admits none of the source-CRS
options. Legacy `auto`, `geographic` and Web Mercator keywords and bare codes are
not accepted: name the CRS, for example `EPSG:4326` or `EPSG:3857`. Horizontal
feet never rescale source height or A, and no geoid model is applied: Metashape
heights that are orthometric need an explicit `--height-offset` to the
ellipsoid.

Capture maps each post-node source position plus the offset through the CRS to
WGS84 ECEF and into one local east-north-up frame in f64, then rounds once to
f32; positions are mapped exactly for every vertex. The frame's origin is the
mapped centre of the used post-node source bounds; the root transform carries
ENU to ECEF. Authored normals are transformed by the inverse transpose of the
Jacobian at the vertex and renormalized. Tangents are transformed by the
Jacobian, then re-orthonormalized against the transformed normal (Gram–Schmidt)
because a CRS map is not conformal in 3D (horizontal scale differs from
vertical); the W handedness sign is kept (it would flip only for an
orientation-reversing Jacobian). Jacobians use central differences through the
same CRS operation (one-sided at a projection-domain edge), evaluated exactly at
the nodes of a lattice over the source bounds (at most 32 cells per axis, each
nominally 1 km horizontally or 0.01° geographic and 100 m vertically) and
interpolated trilinearly. A cell whose interpolated Jacobian at its centre
differs from the exact one by more than 1e-7 (relative), or that touches the
CRS domain edge, uses per-vertex Jacobians instead. The resulting normal error
is about 1e-7 times the Jacobian's condition number (for EPSG:3857 about
1/cos(latitude), so about 2e-7 rad at 60°); the centre check estimates the
quadratic interpolation term, not higher-order terms. Measured errors in the
tests are below 1e-7 rad, including a 150 km site at 60° whose cells sit at the
check's edge. The report states the lattice and its
per-vertex cells. Sorting, leaves, coarse levels and delivery then work in local
metres. Every source coordinate must lie in the CRS's verified domain; a vertex
outside it is `invalid_input`, and a mapped position beyond ±1,000,000 local
metres is `unsupported`.

EPSG:4326, EPSG:3857, EPSG:3395, UTM (EPSG:326xx/327xx) and the grid-free PROJ
methods listed for point clouds work in the standard CLI and wheel. Other EPSG
codes, coordinate epochs and Lambert azimuthal equal area need the
`native-geospatial` build and its PROJ database; without it they are
`unsupported` before source I/O. One operation serves the whole capture: a
source that would need native PROJ for only some vertices is refused.

## Coarse levels (HLOD)

Leaves are the exact tiles; an occupied cell above them publishes one coarse tile
built from its children, unless it is [omitted](#hierarchy-and-cracks) because its
locked interface cannot fit the triangle cap. This is the only coarse-tile method; it
has no selector, and a plain `mesh` run uses it.

### Hierarchy and cracks

- **Leaves are exact.** Each leaf holds its source triangles unchanged (at most
  8,192) with original vertex references and full provenance. Coarse levels never
  alter a leaf.
- **Coarse tiles are bounded.** A parent welds its children's vertices by decoded
  position and reduces them with meshopt to at most 8,192 triangles. The triangle
  cap leads; there is no error target.
- **Crack-free by construction.** The vertices, segments and points on a tile's
  *outer interface*, where its surface meets geometry outside its own subtree, are
  locked: simplification never moves or deletes them, and every tile that carries
  one decodes it to the same position at every depth. Edges between children inside
  the parent are free to collapse. An interface is never thinned. Published bytes are decoded and
  checked before publication, and the acceptance campaign counts decoded interface
  segments independently. Crack-free means no gap along a locked interface between
  tiles of any depth mix; it does not make a coarse level watertight or hide its
  silhouette change.
- **Omission, not refusal.** 8,192 triangles stays the hard published cap. If a
  parent still needs more triangles than the cap after simplification (its exact locked
  interface, or topology that simplification cannot reduce, is too large), its coarse
  tile is **omitted**; the run does not fail. Omission propagates upward: every ancestor to the root is omitted
  too, because a parent cannot be built with a hole where the omitted tile was. An
  omitted cell publishes no content and holds no geometry, and the capped descendant
  frontier (coarse tiles and leaves below it) is what the archive publishes there. An
  omitted cell takes the empty-parent error (at least 16 times its box diagonal), so it
  refines whenever it is visible and no content-bearing ancestor sits above a
  content-less one, and delivery and `validate` refuse a coarse tile above an omitted
  cell. Omission shows as content-less tiles in the published tileset; its counts are
  kept in the internal leaf manifest, not in `tilerReport` or `diagnostics`. Omission
  is a bounded last resort and costs client requests near the omitted region (about 8
  times the depth in content-less cells plus the children's tiles). It is distinct from resource refusals: a budget or scratch/disk shortfall is
  still `resource_limit`, and an unreadable or unsupported source is still refused.
- **Single-child chains.** A cell whose only child is a coarse tile publishes that
  tile once, at the top of the chain; the cells below carry no content and the same
  error, so refinement stays monotone. A cell whose only child is a leaf publishes
  a reduced copy of it.

### Geometric error: sampled, not certified

A coarse tile's `geometricError` is `G = max(max child G, 1.05 × M)`, where `M` is
the larger of two directed distances measured on the **decoded positions a viewer
sees** against the captured source surface in the tile's source range: coarse to
source and source to coarse. Sampling is deterministic and area-stratified, with
extra controls at patch boundaries, one vertex per connected component and sliver
triangles. At least 2,048 samples per direction are taken for each tile and at
least 200,000 per direction for each occupied depth; scheduling is seeded by tile,
so `--jobs` never changes the result. `G` is monotone down the tree and is the
value in `tileset.json` and the implicit-subtree metadata.

This is **not** certification. A sampled maximum is neither a Hausdorff bound nor a
guarantee against unsampled outliers (the 5% margin exceeds the roughly 1% maximum
exceedance seen in prototype audits; it is not a covering radius), and no universal
bound is claimed.
Incomplete required sampling is a `resource_limit` refusal, never a zero error,
and a non-finite measurement fails the run. `diagnostics.hlod.error_measurement`
reports the policy version, sample counts and per-direction mean/p95/p99/max.
`validate` checks that errors are finite, non-negative, ordered and consistent
between explicit and implicit metadata; it does not recompute source distances.

### Coarse appearance

Each material class of a coarse tile gets fresh planar charts on its final decoded
positions (UV seams of the source are ignored; positions never move), packed into
one content-sized page per bound slot: the least-area rectangle of whole 4×4 codec
blocks, at most 1,024 texels a side, that holds the charts, with a 2-texel gutter. Texel spacing is
`max(G / 16, 1e-5 m)`, but never finer than the source's own texel spacing over
that surface, so a page never carries more texels than the source has. If the
content cannot fit 1,024², the page uses the finest spacing that does.

Each covered texel takes the nearest captured source point and samples the source
images at the texel's footprint level in linear light. Base colour (multiplied by
`COLOR_0`), emissive, metallic-roughness and occlusion are resampled; normal maps
are re-expressed in the coarse tile's tangent frame. Factors, `alphaMode`, MASK
cutoff and double-sidedness are kept, so one H28 material class stays one class. The
source samplers (wrap, filters) apply while baking, when texels are sampled from the
source images; they are not carried onto the coarse page, which publishes newly
authored atlas samplers (clamp to edge; linear with mip-linear minification for a
KTX2 page, linear without mips for a PNG page). Untextured classes keep flat class factors, plus a per-vertex `COLOR_0`
taken from the nearest source surface when the source carries colours. `NORMAL` is
published only when the source carries normals, so leaves and coarse tiles of one
source shade the same way. Coarse pages are not required to reproduce leaf texels:
they are an appearance approximation of the surface, with no appearance
certificate.

Coarse pages go through the same delivery encoder and decoded-image gates as leaf
pages, so a profile chooses one colour codec for both (the table under
[Profiles](#profiles-and-resources)). A tile's pages together are bounded to about
2²⁰ texels per slot, shared by surface area among its material classes, so a parent
with many classes splits one page's worth instead of taking a full page for each.

The coarse ladder differs from the leaf ladder below at its end. A coarse page that
misses a gate is re-encoded (UASTC level 2, then 3) as a leaf page is, but a page that
no codec step can deliver within its gates never fails the run: it is published as its
exact level-0 PNG and sampled without mips. Pages escalated or delivered this way are
counted per depth in `diagnostics.hlod.appearance_by_depth` (`escalatedPages`,
`pngPages`); the report's
`escalated_pages` and `escalation_diagnostics` cover leaf pages.

Normal, metallic-roughness and other data pages, MASK coverage and ID pages follow
their own class rules in every profile (see [Profiles](#profiles-and-resources)).
Leaf atlases keep only each leaf's needed source regions, with aligned crops, wrap,
gutters and per-level mips; a leaf never depends on a whole source image merely
because one triangle samples it.

### Identity, annotation and editability

- **Leaves** keep exact `source_primitive` and `source_triangle` feature IDs for
  every triangle, with original corner order.
- **Coarse tiles** carry one `source_primitive` feature ID per triangle, duplicated
  at all three corners: the source primitive nearest to the triangle's decoded
  centroid within the tile's source range (ties by distance, then source order).
  It is a **representative**, not ancestry or a triangle mapping, and a primitive
  that no longer shows in a coarse tile need not appear there. Coarse tiles have no
  per-triangle source IDs and no source-triangle table. Each tile keeps one property
  table per ID set.
- **Annotation.** Primitive-level labels may be shown on coarse tiles as
  representative picks. Exact annotation maps source-triangle annotations onto
  **leaves**. An arbitrary region cannot propagate exactly through coarse IDs; it
  needs an explicit later aggregation or rebake policy.
- **Texture editing.** A leaf is exact in texture space when no two triangles'
  decoded UVs share interior area on a page (`unique`); overlapping, mirrored or
  repeated source UVs make it `shared`, editable in vertex space only. Coarse charts
  never overlap, so a coarse page is editable, but editing it changes only that
  coarse tile, not its leaves or other levels, and does not make its provenance exact.

## Profiles and resources

`--profile` accepts `fast`, `balanced` (the default), `quality` or `lossless`.
These are **provisional policies**: leaf sizing, texture resolution, encoder
effort and other constants will be calibrated together as HLOD implementation
and acceptance progress. Do not build scripts around today's numerical constants.
Every present PBR slot follows its colour/data class and the chosen policy.
Lossy output uses in-process KTX2/Basis; lossless leaf textures use PNG.
`lossless` describes the agreed leaf texel policy, not exact source GLB bytes,
arbitrary viewer filtering, or lossless coarse geometry. Leaf coordinate
encoding and provenance still follow the tiler contract.

Colour-class pages (base colour and emissive), in **both leaves and coarse
tiles**, use the following initial codec. The single policy owner is
`src/mesh_large/leaves/delivery.rs::codec_settings`; coarse delivery calls it
through the shared authored-chain encoder.

| Profile | Colour codec | ETC1S quality / effort |
| --- | --- | --- |
| `fast` | KTX2 ETC1S (BasisLZ) | 50 / 0 (lean) |
| `balanced` (default) | KTX2 ETC1S (BasisLZ) | 90 / 0 (good) |
| `quality` | KTX2 UASTC + Zstd | unchanged |
| `lossless` | PNG | unchanged |

Quality uses the wrapper's 1–100 scale; effort 0 maps to native ETC1S
compression level 0. The initial effort-3 setting mapped to native level 2;
the [effort sweep](../bench/architecture_audit/etc1s_default/effort.md) records
the processing cost and byte/error tradeoffs behind the revision. Colour remains
sRGB with class-correct authored mips. Normal and metallic-roughness remain UASTC;
occlusion remains scalar ETC1S q90/effort 0 in fast/balanced and UASTC in
quality. ID pages remain exact PNG in every profile.

The processing tradeoff is explicit: the matched two-worker tex3 sweep took
47.1/56.5 CPU-s for fast/balanced ETC1S, versus 16.1/18.2 CPU-s for their
previous UASTC pack 0 codec. Effort 0 cuts the initial effort 3 cost substantially
and balanced meets the 91.2 CPU-s quality-reference budget, while costing about
3× the previous same-profile codec's encode CPU. Full tables, byte savings,
measurement context and resource limits are in the linked effort record.

Decoded RGB floors are support-weighted and checked on **every mip**, including
1×1. The leaf/delivery gate selects a band by the count of texels with positive
support, not their summed weights or the padded level area. The middle band is
64–1,023 supported texels; tiny means fewer than 64.
A page must meet all applicable criteria; PSNR is in dB, errors in RGBA8 byte units.

| Colour profile | PSNR ≥1,024 / middle / tiny | Large p99.9 cap | Worst supported 4×4 block RMSE cap |
| --- | --- | --- | --- |
| `fast` | 20 / 16 / 13 | 112 | 96 |
| `balanced` | 20 / 16 / 13 | 80 | 80 |
| `quality` | 27 / 23 / 23 | 64 | 48 |

These fast/balanced RGB gates are looser than develop's previous UASTC floors
of 24 dB at ≥1,024 supported texels and 20 dB below that, and their local and
decoded sampler caps are wider. The accepted byte saving carries an explicit
reduction in reconstruction accuracy.
Tiny fast/balanced levels additionally require **combined RGB PSNR ≥16 dB**,
computed from the mean of the three channel MSEs, never the mean of their dB
values. This catches correlated RGB damage while admitting a shared-palette
tail concentrated in one channel. The sampler applies this joint floor only
to decoded colour in its own tiny area band; authored correspondence remains
strict and separate.

The percentile cap is omitted below 1,024 supported texels; the block guard stays
active. Alpha keeps PSNR/max-error floors of 30/80 (fast), 30/64 (balanced),
34/32 (quality). Normal RGB keeps 30/112 plus normalized-length mean/max caps
0.08/0.20, 0.05/0.15, 0.03/0.10 respectively, and angular mean/p99 caps
3°/12°. Metallic-roughness keeps 30/32 on G/B; occlusion keeps 30/80 on R
(quality 30/8). MASK decoded coverage keeps the attainable semantic gap plus
0.02/0.01/0.005. Non-MASK alpha and data source-sampler floors are unchanged.
Decoded fast/balanced colour source sampling uses the same RGB PSNR bands,
selected by the sampled level's **area** (width × height), including padding,
not its positive-support count. Nearest-mip filtering rounds to the selected
level; a fractional mip blend uses the coarser participating level. Non-mip
filters use level zero. Thus a sparsely supported 32×32 level uses the large
sampler band even when its leaf gate uses the tiny band. Maximum errors are
160/144 respectively. Quality keeps 34/24. Pre-encode source correspondence
keeps its original 22/56, 22/32, 34/24 limits; codec error does not authorize
weaker chart correspondence. These errors are in sRGB byte space after
linear-light sampling.

The q50/q90 photo, alpha-photo and odd-size fixture bases were authored into
valid colour-correct mip pages (POT padding has zero support). They measured
minimum channel PSNR 20.28/20.67 dB at ≥1,024 texels, 18.57/18.62 dB in the
middle band and 13.30/13.30 dB in tiny mips at effort 0. Their minimum joint
tiny RGB PSNR is 16.85 dB. The effort-0 smooth 64² photo control
reaches 17.05/17.16 dB in the middle band. Shared palettes and joint-channel
reconstruction explain the coarse/tiny channel tails, motivating the initial
20/16/12 bands. The processing-cost revision raises tiny channel PSNR to 13 dB
and adds the 16 dB joint RGB floor. The former 12 dB admitted a light flat grey 200
page wrongly converted to linear bytes 147 (13.65 dB in all channels). Raising
the channel floor to 14 dB would also reject valid 13.30 dB fixture tails; the
joint guard separates those cases. The existing codec byte-domain
pyramids were measured separately; they do not define the colour-mip filtering contract.

A legal ETC1S decode is not automatically acceptable. The effort-0 128² and 256²
smooth-photo stress controls miss the large-level floors and require escalation; the
256² q90 case reaches 17.52 dB and p99.9 95 on its 32² mip. Noise/chroma-noise also miss the RGB floors; they do not set
acceptance floors. Percentile/block caps retain local damage detection, with
flat/low-noise and saturated checker controls measured separately. Source max
caps cover photo tails with margin; alpha/MASK guards remain independent and
are never relaxed to admit RGB error.

This brings forward D7 by the owner's 2026-10-24 decision. Calibration and
bounded tex3/crop3 measurements, corruption controls and proof limits are
recorded in [the ETC1S acceptance record](../bench/architecture_audit/etc1s_default/README.md).
The [processing-cost revision](../bench/architecture_audit/etc1s_default/effort.md)
records the effort sweep, tighter tiny gate and default-lossy archive golden.
Black pages, inverted channels and an upsampled coarse checker mip fail the
decoded gate. Smooth wrong-mip content can still pass even a 20 dB floor; the
revision records a >40 dB example and proposes a separately calibrated spatial
derivative/band-pass check. PSNR does not establish mip provenance.

In a lossy profile, a **leaf** texture page that misses any decoded-image gate at the
profile's own encoder setting (per-level channel and normal floors, MASK
coverage, the source-sampler check) is re-encoded one step stronger instead of
failing the run: UASTC pack level 2, then UASTC level 3. Every step is checked
again by the same gates; gates are never loosened. If level 3 also misses and
the page's sampler reads level 0 only (no mipmap minification), the page is
delivered as an exact level-0 PNG, which keeps the sampler. A mip-sampled page
that still misses at level 3 fails the run, because glTF PNG cannot carry its
authored coarse levels. (Coarse pages never fail this way; see
[coarse appearance](#coarse-appearance).) Escalation costs output size: an ETC1S page re-encoded
as UASTC is several times larger, and a PNG fallback page is typically the
largest form of the page (up to 4 bytes per texel). The ladder depends only on
the page, so output is identical for any worker count. Cancellation is polled
before every re-encode step, so a run can stop between steps of one page.

Leaf normals are a profile property. `lossless` keeps the f32 source normal
(after the node transform, renormalized at capture). `fast`, `balanced` and
`quality` store 12-bit octahedral codes through the `EXT_meshopt_compression`
OCTAHEDRAL filter; viewers decode them to normalized 16-bit `NORMAL`
(`KHR_mesh_quantization`). The angle to the source normal is at most about 0.06°
(0.0596° maximum and 0.020° mean over the 73 million leaf corners of a
24-million-triangle scene). The bit count is provisional, like the other
profile constants. The per-triangle provenance tables (`source_primitive`,
`source_triangle`) are losslessly meshopt-compressed in every profile. Their
schema, columns and decoded values are unchanged.

`--jobs auto` is memory-aware. A positive `--jobs N` is a **maximum**, also capped
by available cores and each stage's memory plan. The diagnostics report the
requested choice and actual workers independently for capture and leaf
delivery. When available memory cannot be queried, the conservative policy
uses one worker. A one-worker floor does not authorize an item that cannot fit;
required memory or scratch exhaustion returns `resource_limit`.

`--scratch-dir PATH` (`--scratchDir`) selects the parent for owned working data.
Without it, capture uses the platform scratch default and private leaf/forest
intermediates use the output parent. Final archive staging remains on the output
filesystem beside the destination. Disk-backed scratch is usually
preferable for large sources; memory-backed scratch consumes memory in the
worker plan and is reported. Admission requires free scratch for the largest
declared stage working set (files retained from earlier stages, the HLOD
incidence pass and source index, and each external sort's runs). Sort runs are 8 MiB chunk files removed as soon as
they are read, so a sorted stream and the output it feeds share space, with at
most `F` partly read chunks of slack. A 10% margin and per-file rounding are
added: about 0.5 KB per source triangle at scale with every vertex attribute present.
Capture stores only the vertex attributes some selected primitive carries, so sources
with fewer need less: about 0.3 KB with positions and one texture coordinate set.
It also reserves output-side space for leaf staging, archive
staging and a replaced destination: three copies of an estimate of 119 bytes per
triangle (measured before the archive-size reduction, which brought it to about 47; kept as a conservative estimate) plus the encoded image bytes at admission, raised once image dimensions
are known to 4 bytes per source texel for `lossless` PNG pages or 1 byte per
texel for KTX2 profiles, each with a 4/3 mip allowance (a texture-stage
recheck). These are estimates, not bounds. Output demands are added to the
scratch figure when they share a filesystem, told apart by device id: btrfs
subvolumes of one pool report different ids, so their demands are checked
separately against the same free space. Memory-backed (tmpfs) scratch, work
directory and output staging are charged to the memory budget
([Memory budget](#memory-budget)).
Each capture stage rechecks free space for its own working set on entry and
fails closed if its measured peak exceeds its declared bound; the diagnostics
report both (`scratch_stage_bytes`, `scratch_stage_peak_bytes`). The HLOD
incidence pass is measured the same way (`leaf_delivery.hlod_incidence_scratch_peak_bytes`), and the
whole-run spill peak (`run_scratch_peak_bytes`) must stay within the declared need.
The coarse walk's own scratch is admitted through its memory leases
(`diagnostics.hlod.scratch_components`) and is not in the capture figures. A preflight is
not a reservation against unrelated writers. Owned intermediate files are
cleaned on success and failure. Any cleanup failure carries actionable paths
and does not replace the original cause.

There is no total source-byte or triangle-size refusal. JSON, hierarchy,
individual-item and format limits still apply. The 3TZ member ceiling is
178,956,970, including the root document, reports and index; exceeding it returns
`unsupported`. Per-tile budgets are derived from policy. There is no
`--leaf-triangles`, coarse-tile triangle/error flag, explicit/full-detail selector,
texture-format option, external encoder fallback or compression bypass on
`mesh`.

### Memory budget

One declared ceiling governs the whole run, with one rule on every surface:

| Surface | Spelling |
| --- | --- |
| CLI | `--memory-budget BYTES` (`--memoryBudget`) |
| Python | `mesh_to_tileset(..., memory_budget_bytes=None)` |
| Rust | `TilerRequest::with_memory_budget(bytes)` |

- **Values.** A positive integer number of bytes. The CLI also takes the binary
  suffixes `B`, `KiB`, `MiB`, `GiB` and `TiB` (`2GiB`); Python takes an `int`, and
  Rust a `u64`. Zero, signs, fractions, exponents, decimal-unit (`GB`) or lowercase
  suffixes, booleans (Python: `TypeError`) and values above 2⁶⁴−1 are
  `invalid_request`, refused before any source or output is touched, including an
  offset file (a CLI usage error carries exit 2 and `error.code="usage"`, like
  `--jobs`).
- **Default.** Without one, the lesser of **4 GiB** and half the detected
  host/cgroup headroom; 2 GiB when headroom cannot be detected. An explicit budget,
  2 GiB for example, is a ceiling that is never silently raised: every stage plans
  against the lesser of the budget and the detected headroom.
- **What it covers.** Capture, the leaf stage and the coarse stage admit their
  indivisible work against it before allocating: source and JSON metadata, sort
  buffers, interface incidence, the child tiles held for their parents and the
  active parents, decoded source images, chart/page/codec workspace, memory-backed
  capture scratch and the archive stage's resident buffers (`archive_minimum_bytes`).
  A source without images reserves no texture, chart, mip or codec workspace.
- **Memory-backed storage.** Files on memory-backed (tmpfs) storage are charged too.
  Capture scratch there is charged at its declared need. After capture, each other
  owner on tmpfs is charged one copy of the output-side reservation (see scratch
  admission above) plus 32 MiB for manifests, documents and archive scratch that do
  not scale with triangles: the private work directory (leaf and coarse payloads, forest
  staging and the archive's scratch) and the archive staged beside the output. Every
  later stage plans against the budget less that charge. Every write into a charged
  owner is admitted before it lands, in whole allocation units plus 1 KiB per file,
  link or directory (the archive's index, central directory and end records are
  admitted together, at their exact size, before any of them is written); a write
  that would pass the charge is a `resource_limit` refusal (`use a disk-backed scratch
  directory` or `stage the output on disk`), so tmpfs never holds more than was
  charged. An overlay filesystem is classified by its writable upper layer (read from
  `/proc/self/mountinfo`); one whose upper layer cannot be resolved, as inside most
  containers, is charged as memory-backed and reported as `memory_backed_assumed`. The estimate is generous for textured
  sources and tighter for untextured ones (about 1.4× a measured untextured
  0.79M-triangle crop), so disk remains the better place for large outputs.
- **Pressure never changes output.** It serializes work (`--jobs` stays a maximum
  under the same budget), and what cannot fit even alone is a `resource_limit`
  refusal naming the stage and the bytes it needs, before that stage allocates and
  before anything is published. Archives are identical for every budget that
  admits the run.
- **Reporting.** `diagnostics.memory` holds the `budget` (`requested_bytes`,
  `resolved_bytes`, `source`: `explicit`, `default_ceiling`, `default_headroom` or
  `default_undetected`, and the detected headroom), the least budget that admits each
  stage (`minimum_bytes`, `capture_minimum_bytes`, `leaf_minimum_bytes`,
  `archive_minimum_bytes`), the coarse stage's plan (`hlod`) and the memory-backed
  storage charges with what was measured against them (`memory_backed_storage`; their
  sum is `budget.memory_backed_storage_bytes`);
  `diagnostics.hlod.lease_peak_bytes` is its highest simultaneous lease. The minima
  include charged request facts such as the lengths of the input, output and scratch
  paths (sort descriptors carry them), and the detected headroom is a host observation,
  so the same conversion reports slightly different minima from differently named
  paths (a few hundred bytes to about 130 kB on a 0.79M-triangle crop). This is
  deterministic accounting: the archive does not depend on it.
- **What it is not.** It is an admission and planning ceiling, not an operating
  system limit or a measured peak-RSS guarantee; measured peaks come from the
  acceptance campaign.

### Primitives, atlases and editability

Each tile publishes as few glTF primitives as glTF allows
([H28 contract](architecture/h28-tile-batching-contract.md)). Source primitives
share one node, mesh, primitive, material and texture page set when their
materials belong to one class and they carry the same vertex attributes. A class
means equal factors, `alphaMode`, MASK `alphaCutoff`, `doubleSided`, the same bound
slots with equal normal scale and occlusion strength, and equal source min/mag
filters. An omitted value equals its glTF default; texture identity and wrap
modes do not count. A leaf's
merged positions are rebased onto one shared quantization offset without being
quantized again, so every decoded vertex is bit-identical to a per-primitive
encoding. Primitives split only where those properties differ, where a mipmapped
source sampler keeps a dedicated leaf page in a lossy profile, or where a class
overflows one page. Every leaf triangle keeps its `source_primitive` and
`source_triangle` feature IDs; a coarse triangle carries one representative
`source_primitive` ID ([identity](#identity-annotation-and-editability)). Each
tile has one property table per feature-ID set. Feature-ID attributes carry no
baked ID textures: each set is already an exact per-face vertex attribute
against its property table.

Leaves are exact in vertex space: every triangle keeps its source provenance.
They are also exact in texture space when no two triangles' decoded UVs share
interior area on a page, so each texel has one owner. The tiler checks this
exactly after atlasing. Overlapping, mirrored or repeated source UVs make a
leaf `shared`, which is editable in vertex space only; such a leaf is never
refused. Coarse-tile editability and annotation limits are described under
[Coarse levels](#identity-annotation-and-editability).

Feature-ID attributes are UNSIGNED_BYTE, UNSIGNED_SHORT or FLOAT (glTF advises
against UNSIGNED_INT vertex attributes; rusty-tiles chooses not to write them),
and FLOAT holds every integer only up to 2^24. A larger ID is `unsupported`
rather than rounded.

## Python

```python
from pathlib import Path
import rusty_tiles

result = rusty_tiles.mesh_to_tileset(
    Path("local.glb"), Path("placed.3tz"),
    profile="balanced", jobs=None, scratch_dir=None, memory_budget_bytes=None,
    anchor=(153.02, -27.47, 25.0),
    orientation_xyzw=(0.0, 0.0, 0.0, 1.0),
    scene_offset=(10.0, 2.0, -5.0),
    force=False, callback=None,
)
projected = rusty_tiles.mesh_to_tileset(
    Path("site.glb"), Path("site.3tz"),
    source_crs="EPSG:3857", source_offset_file=Path("site_offset.txt"),
)
print(result.report["triangles"], result.diagnostics["leaf_delivery"]["jobs"])
```

`mesh_to_tileset(input, output, *, profile="balanced", jobs=None,
scratch_dir=None, memory_budget_bytes=None, anchor=None, orientation_xyzw=None, scene_offset=None,
source_crs=None, source_axes=None, source_offset=None, source_offset_file=None,
height_offset=None, force=False, callback=None)` accepts `str` or `os.PathLike`
paths. `source_crs=None` or `"local"` keeps local input; `source_axes` is
`"y-up"` (default with a CRS) or `"xyz"`, `source_offset` a two- or three-number
sequence, `source_offset_file` a path, and `height_offset` defaults to 0, with
the CLI's rules. `jobs=None`
means auto; a positive integer is a maximum. Booleans and noninteger worker
counts are rejected. `memory_budget_bytes` is the [memory budget](#memory-budget): a
positive `int` (not a `bool`), checked before any file is read; `None` takes the
default. Unknown keywords raise `TypeError`. Python calls
the same core operation without launching the CLI or an encoder, and releases
the GIL during native work.

The frozen `TilerResult` has resolved absolute `output: pathlib.Path`,
`archive=True`, required `report` and `diagnostics` dictionaries, and
`cleanup_diagnostics`. Frozen attributes do not make the dictionaries deeply
immutable. Paths are bound before callbacks, so callback changes to the current
directory cannot redirect publication. Required callbacks are precommit and
fallible; a winning callback exception is preserved as the original Python
exception. Python result materialization or unrelated asynchronous exceptions
after installation are outside the core publication guarantee.

## Reports and errors

`--json` returns one stdout object with `ok`, resolved `output`, `tilerReport`,
`diagnostics` and `cleanupDiagnostics`. `counts` and `conversionReport` locate
the same archive-resident `conversion.json`. The versioned `tilerReport` uses
snake_case and equals Python's `result.report` and the published report.

The semantic report includes profile, source/resource counts, leaf/triangle/GLB
counts and maximum leaf geometry error, hierarchy/implicit/document counts,
depth and packed/synthetic leaf counts, member limits, separate root/tileset
errors, and normalized placement. Escalation is reported as `escalated_pages`
(`uastc_level2` and `uastc_level3` counts, by the step that delivered the page),
`lossless_fallback_pages` (pages delivered as exact PNG), and
`escalation_diagnostics`: the first 32 escalated pages in leaf order, each with
its `leaf`, `first` source triangle rank and `depth`, `page`, `slot`, `sources`
(`image`, `material`), delivering `step`, and every gate it missed on the way
(`misses`: `step`, `mip` or null for the all-level sampler check, `gate`).
`escalation_diagnostics_omitted` counts escalated pages beyond that bounded
list. `texel_ownership` reports texture-space editability:
`unique_leaves`, `shared_leaves`, the `shared_triangles` of shared leaves,
`unique_proxies` (the field name predates HLOD; it counts every coarse tile, whose
charts never overlap), and `shared`. That list holds the first
64 shared leaves (`leaf`, `first`, `depth`, `texel_ownership: "shared"`,
`triangles`); `shared_omitted` counts the rest. Every leaf that is not listed
or omitted is `unique`. Ownership is all-or-nothing per leaf: one pair of
overlapping footprints marks the whole leaf `shared`, so use `shared_triangles` (and
a leaf row's `sharedTriangles`) over the leaf's `triangles` for the fraction of the
leaf that is affected. `source_coordinates` is `local-gltf` or
`source-crs`; `coordinates` is `local-gltf` or `wgs84-ecef`. A source CRS adds
`source_crs` (definition, axes, source offset, height offset, `portable` or
`native` operation, the chosen origin, `jacobian_lattice_cells` and
`jacobian_exact_cells`) and reports its derived ENU frame as a
`wgs84` placement at that origin. `root_transform` is null for local
output, whose root has no transform, or the exact emitted column-major matrix
for placed output. A coarse tile's geometric error is its sampled, measured error
([geometric error](#geometric-error-sampled-not-certified)); coarse textures are
an appearance approximation with no appearance certificate
([coarse appearance](#coarse-appearance)), and charting never changes a position or
the error.

Runtime diagnostics stay **outside the archive** so worker/scratch choices do
not alter its semantic bytes. They include `requested_jobs`, `capture`,
`leaf_delivery`, `archive_bytes`, `peak_held_records`, `phase_seconds`, the
`memory` budget and stage plan ([memory budget](#memory-budget)), and the coarse
stage's `hlod` facts (tiles, workers, CPU seconds, lease peak, `error_measurement`
and the per-depth appearance summary: page texels, covered texels, cap hits and the
requested and achieved spacing). `memory` and `hlod` are present in every result.
`phase_seconds` holds the wall seconds of `capture`, `leaf_build`, `hlod_incidence`,
`hlod_geometry` and `archive`. Stage objects describe worker admission and owned
memory/scratch resources; their declared accounting is not an automatically
measured RSS guarantee. Absolute scratch paths and resource observations are
diagnostics, not semantic profile constants.

`--progress json` emits NDJSON work events on stderr with named phases and
nullable totals: `conversion`, `capture`, then inside the leaf stage
`leaf_build` (leaves), `hlod_incidence` and `hlod_geometry` (coarse stage), then
`leaf_payloads`, `archive` and `ready_to_publish`. Each finished
phase also logs its wall seconds and the run's elapsed seconds as a `log` event.
Required events finish before publication; the returned
successful result establishes installation. A post-install cleanup problem
remains success with cleanup diagnostics.

| Core failure | CLI exit / JSON code | Python exception |
| --- | --- | --- |
| Invalid request | 2 / `invalid_request` | `InvalidRequestError` |
| Malformed or changed source | 3 / `invalid_input` | `DataError` |
| Unsupported semantics/domain/container size | 2 / `unsupported` | `UnsupportedError` |
| Resource refusal | 1 / `resource_limit` | `ResourceLimitError` |
| I/O | 1 / `io` | `TilesIOError` |
| Existing output | 5 / `output_conflict` | `OutputExistsError` |
| Cancellation | 1 / `cancelled` | `CancelledError` |
| Required observer failure | 1 / `observer_failure` | `ObserverError` or original winning callback exception |
| Internal invariant failure | 1 / `invalid_state` | `TilesError` |

CLI parser/usage errors happen before the core and use exit 2 and
`error.code="usage"` when `--json` is present. Core errors carry their domain
`kind`, secondary diagnostics and retained paths. Request/placement/output
intent is validated before source I/O; source admission, resource work and
publication then retain causal failures. Cleanup does not replace the primary
failure. `doctor --command mesh` checks build capability read-only; it does not
approve a particular source, resource plan, filesystem or viewer.

## Migration from older mesh converters

The new public workflow replaces the preparatory `mesh-leaf-payloads`,
`mesh-leaves-to-3tz` and `mesh-large-to-3tz` commands. Leaf directories and their
manifests are internal stage artifacts; there is one public source-mesh
operation.

`mesh-to-3tz` (alias `meshTo3tz`) / `mesh_to_3tz` and `mesh-local-to-3tz` /
`mesh_local_to_3tz` were deprecated during 0.4.0 development and removed
before the release, together with their writers. The old command names are
now unknown subcommands (exit 2), and the Python functions no longer exist.
Their output cannot be reproduced; re-run `mesh` on the source.

Geographic/projected meshes used `--source-crs`; the legacy Web Mercator
adapter call `--source-crs epsg:3857 --source-offset E N A` became
`--source-crs EPSG:3857 --source-offset E N A` (default `y-up` axes and zero
height offset), and `--source-offset-file` accepts only the strict
`E:`/`N:`/`A:` format. Automatic CRS detection (`auto`), the
`geographic`/Web Mercator keywords and old Euler placement
(`--cartographic-position-degrees`, `--rotation-degrees`) had no new flags and
are gone: declare the source CRS, or use `--anchor` and `--orientation-xyzw`. Vector/point CRS support remains a separate retained capability.

Old leaf/proxy triangle and error knobs, atlas sizes, `--explicit`,
`--node-features` and the JPEG/WebP/UASTC/general-PNG output selectors
(`--texture-format`, Python `texture_format`) had no one-to-one replacements:
choose a profile. JPEG source
decoding remains distinct from retired JPEG output encoding. Both original
primitive and triangle source identities are always carried for leaf picking;
leaf-local table row IDs are not global business IDs. The implicit forest can
contain an explicit skeleton and link documents without offering an explicit
output mode. `glb-to-3tz` / `glb_to_3tz` and `createTilesetJson` /
`model_to_manifest` remain the separate exact-byte wrapping/sibling-manifest
products.

### Development builds before the HLOD coarse levels

Builds made during 0.4.0 development used certified proxy geometry with a
separate texture bake. That path was removed outright, with no switch, alias or
compatibility mode; nothing in a release publishes its output. Scripts written
against those builds need these changes:

| Development build | Now |
| --- | --- |
| A parent's geometric error was a certified distance bound | A sampled, measured error ([not a certificate](#geometric-error-sampled-not-certified)) |
| Coarse tiles carried `proxy_source` rows naming a source triangle | One representative `source_primitive` ID per coarse triangle; read leaves for exact provenance and annotation |
| Phases `lod_prepass`, `lod_simplify`, `seam_resolution`, `lod_finish`, `proxy_bake`, and the `proxy_bake` diagnostics | Phases `hlod_incidence` and `hlod_geometry`; diagnostics `hlod` and `memory` |
| Scratch diagnostics `lod_prepass` and `lod_prepass_scratch_peak_bytes` | `hlod_incidence` and `hlod_incidence_scratch_peak_bytes` |
| A hidden coarse-tile selector (`--lod`) | Removed; there is one method |
| A hidden, optional memory budget | The public [memory budget](#memory-budget); a budget always resolves |

## Known Cesium colour limitation

CesiumJS 1.146.0 has a known KTX2 colour-filtering limitation. In the tested
Chromium WebGL2 setup with an NVIDIA RTX 4080 SUPER, high-contrast base-colour
and emissive textures filtered incorrectly with ASTC, BC7 and ETC2 transcode
targets despite correct authored sRGB mip levels and transfer metadata. This
can change their appearance, especially between texels or mip levels. The
linear material-data control was unaffected apart from codec error. This is a
bounded viewer finding, not a claim about all devices or other viewers. The
tiler keeps its class-correct textures; it does not bake a viewer-specific
colour correction.

The measured cause is that CesiumJS 1.146.0 uploads KTX2 sRGB colour textures
as linear (UNORM) formats on the ASTC, BC7, ETC2 and uncompressed paths, so
filtering happens on encoded values. The worst case measured was 72/255 on
high-contrast test textures. Linear data textures are unaffected. The limitation
is documented rather than worked around: it is not filed upstream, and the tiler
adds no viewer-specific compensation.

### Colour space and lighting in CesiumJS

What the tiler writes. Base colour and emissive pages of leaves and of coarse
tiles are sRGB-encoded: lossy profiles carry the sRGB transfer in the KTX2
container, and coarse pages are sampled in linear light and re-encoded to sRGB
when baked. Metallic-roughness, occlusion and normal pages are linear data.

Lit and unlit tiles. Whether a tile is lit depends on its vertex attributes, not
on a setting of the tiler. A source without NORMAL (typical photogrammetry
exports) gives leaves with POSITION and TEXCOORD_0 only, and CesiumJS 1.146
draws such tiles unlit, so the texture colour is shown as sampled. Coarse tiles
publish NORMAL only where the source carries normals, so a view that mixes leaves
and coarse tiles of one source shades them the same way, without the hard seams
and tint step a lit coarse tile beside unlit leaves would show. A source that does carry
normals is lit in Cesium for leaves and coarse tiles alike, so its apparent
brightness follows Cesium's lighting.

What is verified and what is not.

- Verified: the 72/255 and 61/255 worst-case errors above (base colour and
  emissive) in the H19 probe, with numeric shader samples of KTX2 pages; the
  lit-versus-unlit mismatch and its removal on one 0.79M-triangle photogrammetry
  crop in CesiumJS 1.146 (a visual check by the maintainer, not a pixel measurement);
  and the coarse tiles' attribute set against the leaves, which the tests pin.
- Not verified: how much of the filtering error shows on the unlit path as
  against the lit one (H19 measured sampled values, not a full rendered
  appearance); other CesiumJS versions, GPUs, browsers and viewers; whether PNG
  pages (`lossless` profile) avoid the error (PNG loading was outside the H19
  measurement); and any lighting setting that hides a tint step on a source that
  carries normals.

What to configure. Nothing in the tiler or in the viewer is required for valid
output, and the tiler has no option for a tile's colour space or lighting. The
filtering limitation affects how high-contrast base colour and emissive
textures blend between texels and mip levels. The authored sRGB mip levels and
transfer metadata are correct, so do not edit the pages to compensate. The H19
probe found that uploading the same KTX2 payload with a matching sRGB format
removes the error. This is the loader's behaviour, and no CesiumJS release or
setting that does it has been tested here. If colour accuracy matters, check
the result in your own viewer.

Worker diagnostics describe capture and active leaf workspace admission. Capture
scratch bounds cover capture files only; accumulated leaf/forest intermediates
and the final archive are outside those bounds. Prefer disk-backed scratch for
large inputs, especially when the output directory is memory-backed. Complete
pipeline scale and memory evidence remain separate acceptance gates.
