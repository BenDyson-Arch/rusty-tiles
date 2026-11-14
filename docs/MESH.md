# Mesh tiling

`rusty-tiles mesh` and Python's `rusty_tiles.mesh_to_tileset` stream one static
glTF/GLB scene, in local metres or a declared source CRS, into a 3D Tiles 1.1
implicit forest in one `.3tz`. Content is glTF 2.0. The meshopt and KTX2/Basis
encoders are bundled, so the standard CLI and wheel need no external executable
or system geospatial library. Only a [source CRS](#source-crs) outside the
grid-free tier needs the [native-geospatial build](INSTALL.md#native-geospatial-cli).

The pipeline captures the source, writes exact full-detail leaf tiles, builds
every coarse parent tile from its children as a classic octree level of detail
([coarse levels](#coarse-levels-hlod)), and publishes the hierarchy. A coarse
tile's geometric error is a **sampled, measured** distance to the source
surface, not a certificate. It is raised where the tile's texture is much
coarser than its children's ([texture-aware G](#texture-aware-published-g)).

## Convert a local scene

```sh
rusty-tiles doctor --command mesh
rusty-tiles mesh -i local.glb -o local.3tz --jobs auto
rusty-tiles validate local.3tz
```

Input and output are filesystem paths. Output must end in `.3tz`; directory
output and `.3dtiles.zip` are not supported. An existing destination is a
conflict unless `--force` (or `-f`) is given. The new archive is installed only
after all work and callbacks finish; a failure before that leaves the previous
destination in place. The output cannot overwrite or alias the source or its
dependencies. Installation does not promise power-loss durability.

The archive declares itself an HLOD delivery (`asset.extras.rustyTiles.lod` is
`"hlod"` in `tileset.json`). `validate` therefore applies every HLOD check
without being asked, and refuses a copy whose payload labels were stripped.

## Source and placement

By default the source is **local metres with glTF Y up**. Node transforms are
evaluated during capture. Each source POSITION component, and each component
after the accumulated node transform, must lie within ±1,000,000 metres (source
units with a [source CRS](#source-crs)) before the final f32 rounding. This
source domain is separate from the placement precision limit. Coordinates are
never treated as geographic or projected because of their magnitudes, file
names or an offset file; a source CRS is always declared.

The source must have a default or sole scene, TRIANGLES primitives, local
resources and core PBR materials. Supported:

- authored normals and tangents, UV0/UV1 and `COLOR_0`;
- all five core PBR texture slots, from PNG or JPEG images with 8-bit
  channels, at most 16,384 px per edge and 1 GiB encoded each. 1 GiB is only a
  sanity ceiling: each image is charged against the
  [memory budget](#memory-budget) at its actual encoded size;
- normal maps, when the source has authored normals and tangents;
- tangent-bearing geometry, when the accumulated transform is conformal
  (supported reflections included).

Animation, skins, morph targets, alpha blending, imported
extensions/extras/metadata, remote or data-URI images and other unsupported
semantics are refused, never silently discarded. Dependencies must stay inside
the source directory, without escaping, absolute or network URIs, or symlinks.
Source files must not change during capture; change detection is best-effort,
not an atomic filesystem snapshot.

Without an anchor the tileset stays in its local frame. To place it on Earth:

```sh
rusty-tiles mesh -i local.glb -o placed.3tz \
  --anchor 153.02 -27.47 25 \
  --orientation-xyzw 0 0 0 1 --scene-offset 10 2 -5
```

- `--anchor` takes longitude and latitude in WGS84 degrees and **ellipsoidal
  height in metres**; give an explicit zero when intended. Longitude must be in
  [-180,180] and latitude in [-90,90].
- `--orientation-xyzw` is an active right-handed ENU quaternion with scalar W
  last. Its norm must be within 1e-12 of one; it is normalized once.
- `--scene-offset` is a metre translation in source glTF Y-up coordinates,
  applied after node transforms and before orientation.
- Orientation and offset require an anchor. With an anchor they default to the
  identity rotation and zero translation. `--orientationXyzw` and
  `--sceneOffset` are equivalent spellings.

All placement numbers must be finite. Placement applies one rigid transform at
the tileset root. It never converts a projected CRS, infers source axes,
applies an E/N/A shift or converts geoid heights; use a
[source CRS](#source-crs) for projected or geographic sources. A finite
anchor or offset too large for the shared precision budget is `unsupported`; an
invalid number, range or quaternion is `invalid_request`.

## Source CRS

Projected or geographic sources, such as a Metashape GLB export in EPSG:3857
whose coordinates were shifted by an offset typed into the export dialog,
declare their CRS instead of a rigid placement:

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
| `--source-crs` (`--sourceCrs`) | `local` (the default) or a 2D horizontal EPSG, WKT or PROJ definition, resolved by the shared [CRS resolver](FORMATS.md#point-clouds) |
| `--source-axes` (`--sourceAxes`) | Default `y-up` (glTF is Y up): X=E, Y=height, Z=−N. `xyz` is X=E, Y=N, Z=height. Axes are read after node transforms and never inferred from data |
| `--source-offset E N [A]` (`--sourceOffset`) | Added in double precision: E/N in horizontal CRS units, A in metres (default 0). Default `0 0 0` |
| `--source-offset-file` (`--sourceOffsetFile`) | A text file you write, holding exactly the lines `E: <number>`, `N: <number>`, `A: <number>` in that order (for example the shift entered in Metashape's export dialog, which Metashape does not save) (optional final LF or CRLF, at most 4 KiB). Anything else is `invalid_request`. Excludes `--source-offset` |
| `--height-offset` (`--heightOffset`) | Default `0`: metres added to source height plus A to give WGS84 **ellipsoidal** height. The default is right only when source height plus A is already ellipsoidal |

A source CRS cannot be combined with `--anchor`, `--orientation-xyzw` or
`--scene-offset` (`invalid_request`), and `local` accepts none of the
source-CRS options. Name the CRS explicitly, for example `EPSG:4326` or
`EPSG:3857`; the keywords `auto`, `geographic` and Web Mercator, and bare codes,
are not accepted. Horizontal feet never rescale source height or A. No geoid
model is applied: orthometric Metashape heights need an explicit
`--height-offset` to the ellipsoid.

EPSG:4326, EPSG:3857, EPSG:3395, UTM (EPSG:326xx/327xx), the codes of the
[GDAL-verified EPSG allow-list](FORMATS.md#gdal-verified-epsg-allow-list) (for
example GDA2020 MGA EPSG:7856, GDA94 MGA EPSG:28356, NZTM2000 EPSG:2193 and ETRS89
UTM EPSG:25832) and the grid-free PROJ methods
[listed for point clouds](FORMATS.md#coordinate-reference-systems) work in the
standard build. An allow-listed code adds `datum_operation` to the report's
`source_crs`: the EPSG datum transformation and its stated accuracy (GDA2020 to
WGS 84 (2) is a null transformation with 3 m accuracy, not an exact one). Other
EPSG codes, including grid-based ones such as British National Grid EPSG:27700,
coordinate epochs and Lambert azimuthal equal area need the native-geospatial
build and its PROJ database; without it they are `unsupported` before any source
is read. One operation
serves the whole capture: a source that would need native PROJ for only some
vertices is refused.

Every source coordinate must lie in the CRS's verified domain; a vertex outside
it is `invalid_input`. A mapped position beyond ±1,000,000 local metres is
`unsupported`.

### How source positions are mapped

Capture maps each post-node source position plus the offset through the CRS to
WGS84 ECEF, then into one local east-north-up frame in f64, and rounds once to
f32. Every vertex is mapped exactly. The frame's origin is the mapped centre of
the used post-node source bounds; the root transform carries ENU to ECEF.
Sorting, leaves, coarse levels and delivery then work in local metres.

A CRS map is not conformal in 3D, because horizontal scale differs from
vertical. Authored normals are therefore transformed by the inverse transpose
of the Jacobian at the vertex and renormalized. Tangents are transformed by the
Jacobian, then re-orthonormalized against the transformed normal
(Gram–Schmidt). The W handedness sign is kept; it would flip only for an
orientation-reversing Jacobian.

Jacobians use central differences through the same CRS operation (one-sided at
a projection-domain edge). They are evaluated exactly at the nodes of a lattice
over the source bounds and interpolated trilinearly. The lattice has at most 32
cells per axis, each nominally 1 km horizontally (0.01° for geographic CRSs) and
100 m vertically. A cell uses per-vertex Jacobians instead when its
interpolated Jacobian at the centre differs from the exact one by more than
1e-7 (relative), or when it touches the CRS domain edge.

The resulting normal error is about 1e-7 times the Jacobian's condition number.
For EPSG:3857 that is about 1/cos(latitude), so about 2e-7 rad at 60°. The
centre check estimates the quadratic interpolation term, not higher-order
terms. Measured errors in the tests are below 1e-7 rad, including a 150 km site
at 60°. The report states the lattice and its per-vertex cells.

## Coarse levels (HLOD)

Leaves are the exact tiles. Each occupied cell above them publishes one coarse
tile built from its children, unless the cell is
[omitted](#hierarchy-and-cracks). This is the only coarse-tile method; there is
no selector.

### Hierarchy and cracks

- **Leaves are exact.** Each leaf holds its source triangles unchanged (at most
  8,192) with original vertex references and full provenance. Coarse levels
  never alter a leaf.
- **Coarse tiles are bounded.** A parent welds its children's vertices by
  decoded position and reduces them with meshopt to at most 8,192 triangles.
  The triangle cap leads; there is no error target.
- **Shared borders by construction.** Every tile at a given depth uses the same
  border vertices and edges as its neighbours. Coarser levels may simplify
  source border arcs, with displacement bounded against those arcs at no more
  than `E_d/16`; published geometric error is at least that depth's budget
  `E_d`. A cell whose border alone nearly fills the cap relaxes its shared arcs
  to a larger tolerance (at most 256 times that), and every tile on those arcs
  publishes at least 8 times the displacement bound. Same-depth gaps are zero;
  mixed-depth gaps are at most one eighth of the larger geometric error.
  Junctions and contacts with exact leaves stay fixed. Source defects and
  T-junctions are counted, never matched with a tolerance. Adjacent faces may be
  retriangulated, but delivered border edges and border-connected components
  survive. This does not make the source watertight.
- **Last resorts.** When ordinary simplification cannot meet the cap, the
  [last-resort stages](#last-resort-declarations) run.
- **Residual omission.** A coarse tile is omitted only if the shared border
  geometry it must keep exceeds the 8,192-triangle cap on its own, or if the
  last resort cannot keep the open boundaries that touch its border. The run
  does not fail. Omission propagates upward: every ancestor up to the root is
  omitted too, because a parent cannot be built with a hole where the omitted
  tile was. An omitted cell holds no content; the coarse tiles and leaves below
  it are published instead. It takes the empty-parent error (at least 16 times
  its box diagonal), so it refines whenever it is visible. No content-bearing
  tile sits above a content-less one; delivery and `validate` refuse a coarse
  tile above an omitted cell. Omission costs extra client requests near the
  omitted region (about 8 times the depth in content-less cells, plus the
  children's tiles).
- **Omission is not a resource refusal.** A budget, scratch or disk shortfall is
  still `resource_limit`, and an unreadable or unsupported source is still
  refused.
- **Coarse drops.** Outside stage 3, whole border-free components may drop
  under cap pressure; drops at or above child G go smallest first (bounding-box
  diagonal, then triangle count). Stage 3 lets border-free components and holes
  drop or close at any size. Loss is measured into G, which stays at least child
  G. Every component drop is declared in
  `asset.extras.rustyTiles.droppedComponents`. See the
  [validator caveat](VALIDATION.md).
- **Single-child chains.** A cell whose only child is a coarse tile publishes
  that tile once, at the top of the chain; the cells below carry no content and
  the same error, so refinement stays monotone. A cell whose only child is a
  leaf publishes a reduced copy of it.

`diagnostics.hlod` reports `omitted_proxies`, `omission_sources` and
`omitted_by_depth`, including zeros at every tree depth. Its `borders_by_depth`
summary reports border displacement, terminal locks, topology changes by kind,
omission sources and capped-frontier uses, including zeros;
`borders_by_depth[].topology` reports `droppedComponents` and nonzero
`coarseDroppedComponents`. The internal leaf manifest carries the same counts.

### Last-resort declarations

When ordinary simplification fails, a cell is rebuilt in up to three stages:

- **Stage 1** admits a new face or edge between shared border points only in
  its single seam owner, a cell chosen by a deterministic rule from the leaves
  holding those points. Each side of a two-sided seam owns about half of it.
- **Stage 2** also admits folds. Reversed faces are turned to the source
  orientation, and coincident copies beyond the source's count are removed.
- **Stage 3** clusters vertices per input component while fixing interface
  points, opening pins and open boundaries touching the border. Border-free
  components and holes may drop or close at any size, with the loss measured
  into G rather than bounded by `E_d`.

A tile built by a last resort declares it in its GLB as
`asset.extras.rustyTiles.lastResort`. This is a strict object with exactly two
required integer fields: `stage` in 0–3 and `foldFaces` in
0–18446744073709551615. Unknown fields, missing fields and other types are
invalid; absence means no declaration. Stage 0 marks a tile that keeps folds
inherited from a stage-2 or stage-3 descendant; it is not a reduction strategy.

```json
{"extras":{"rustyTiles":{"lastResort":{"stage":2,"foldFaces":4}}}}
```

`foldFaces` counts decoded faces that are in a coincident group (at least two
faces on the same corner set, in either orientation) or incident to an edge
used in the same direction by at least two faces. Each qualifying face counts
once, not once per pair or edge. The example declares four such faces.

`validate` recounts `foldFaces` and refuses a mismatch (`lastResort`). It
requires a fold-admitting descendant for stage 0, and permits folds only from
stage 2 or in a declared ancestor of such a tile. On every coarse tile,
`seamOwner` refuses new shared-point edges or faces outside their seam owner;
the [HLOD design notes](architecture/hlod-040-decision.md) give the ownership
rule. A declaration does not certify source-distance maxima.

Conversion `diagnostics.hlod` reports `last_resort_tiles`,
`last_resort_stages` (counts for stages 1, 2 and 3), `fold_faces` and
`declared_tiles` (including stage-0 tiles, with stage and fold count).
`diagnostics.hlod.repeat_reuse` counts the exact repeats the coarse ladder
skipped: `contractions` (a build returned an earlier build's failed border
contraction), `replayed_calls` (`Fix2` proposal calls replayed from the
primary build's) and `round_zero_calls` (remesh round 0 repeating the full-cap
calls). The published output is the same as recomputing them. The counts are 0
while the `RT_HLOD_AUDIT_DIR` sidecar records every attempt.

### Geometric error: sampled, not certified

A coarse tile's `geometricError` is `G = max(max child G, 1.05 × M)`. `M` is the
larger of two directed distances, measured on the **decoded positions a viewer
sees** against the captured source surface in the tile's source range: coarse
to source and source to coarse. Sampling is deterministic and area-stratified,
with extra controls at patch boundaries, one vertex per connected component and
sliver triangles. Each tile takes at least 2,048 samples per direction, and each
occupied depth at least 200,000. Scheduling is seeded by tile, so `--jobs` never
changes the result. `G` is monotone down the tree and is the value in
`tileset.json` and the implicit-subtree metadata.

This is **not** certification. A sampled maximum is neither a Hausdorff bound
nor a guarantee against unsampled outliers, and no universal bound is claimed.
The 5% margin exceeds the roughly 1% maximum exceedance seen in test audits; it
is not a covering radius.

Incomplete required sampling is a `resource_limit` refusal, never a zero error,
and a non-finite measurement fails the run. `diagnostics.hlod.error_measurement`
reports the policy version, sample counts and per-direction mean/p95/p99/max.
`validate` checks that errors are finite, non-negative, ordered and consistent
between explicit and implicit metadata; it does not recompute source distances.

### Texture-aware published G

CesiumJS refines a tile on its published `geometricError` alone (maximum
screen-space error 16). A coarse page capped at 1,024 texels a side can be far
coarser than `G` implies, so a blurry coarse tile could stay on screen beside
sharp leaves. In `balanced`, `quality` and `lossless`, the published error of a
coarse tile T is therefore

`G'(T) = max(G(T), 16 · s(T) / t if T is refinable, max child G')`

- `s(X) = sqrt(A / X)` over the textured triangles of X's payload. `A` is their
  decoded surface area and `X` their base-colour texel area: UV area, on the
  TEXCOORD set the material names, times the page's base-level width × height.
  It is the mean texel spacing in metres. Textured area with no UV area counts
  as infinitely coarse.
- T is **refinable** when `s(T) > 1.25 · s(C)`, with `C` T's children pooled:
  refining actually delivers texels at least 1.25× finer. Where the source
  limits the children as much as T, nothing is raised.
- `t` is the most screen pixels per base texel when T is replaced: 2 in
  `balanced`, 1.25 in `quality` and `lossless`. `fast` has no texture term and
  publishes `G`.

`G'` is an upper bound like `G`, so the error stays honest, and it is monotone
down the tree. Only the published value changes: simplification, coarse page
texel spacing, sampling, seams and caps all use the measured `G`.

Every HLOD archive declares its rule in `tileset.json` as
`asset.extras.rustyTiles.textureError`: exactly
`{"version": 1, "pxPerTexel": t, "refinableRatio": 1.25}`, with
`"pxPerTexel": null` in `fast`, matching the profile in `conversion.json`.
`validate` refuses any other value, an HLOD archive with textured payloads but
no declaration, and a profile mismatch. It recomputes `s` from every payload's
pages, UVs and positions and the hierarchy, and refuses any refinable tile
whose `geometricError` is below `16 · s / t` ([validation](VALIDATION.md)).
`diagnostics.hlod.texture_error` counts the refinable tiles, those the term
raised and those raised only to stay above a child.

In `quality` and `lossless` the term can raise the root. On a photogrammetry
test scene whose root page is cap-limited, the root's `G` rose from 5.82 m to
6.28 m; in `balanced` it did not. Replaying CesiumJS tile selection on that
scene showed no change at `balanced` with a 2048² texture input. With the
8192² original, the blurry coarse tiles beside leaves at 10–20 m went away, and
views there cost about 29% more bytes.

### Coarse appearance

Each material class of a coarse tile gets fresh planar charts on its final
decoded positions. Source UV seams are ignored and positions never move. The
charts are packed into one content-sized page per bound slot: the least-area
rectangle of whole 4×4 codec blocks, at most 1,024 texels a side, with a
2-texel gutter. Charts pack by their covered shape (gutter included), so they
interlock instead of each taking its bounding rectangle. Texel spacing is
`max(1.25 · G / 16, 1e-5 m)`, but never finer than the source's own texel
spacing over that surface, so a page never carries more texels than the source
has. If the content cannot fit 1,024², the page uses the finest spacing that
does.

Each covered texel takes the nearest captured source point and samples the
source images at the texel's footprint level in linear light. Base colour
(multiplied by `COLOR_0`), emissive, metallic-roughness and occlusion are
resampled; normal maps are re-expressed in the coarse tile's tangent frame.
Factors, `alphaMode`, MASK cutoff and double-sidedness are kept, so one
[material class](#primitives-atlases-and-editability) stays one class. Source
samplers (wrap, filters) apply while texels are sampled from the source images.
They are not carried onto the coarse page, which publishes newly
authored atlas samplers: clamp to edge, linear with mip-linear minification for
a KTX2 page, and linear without mips for a PNG page.

Untextured classes keep flat class factors, plus a per-vertex `COLOR_0` taken
from the nearest source surface when the source carries colours. `NORMAL` is
published only when the source carries normals, so leaves and coarse tiles of
one source shade the same way. Coarse pages need not reproduce leaf texels:
they approximate the surface's appearance, with no appearance certificate.

Coarse `NORMAL` comes from the coarse tile's own geometry, not from the source
normals. It is crease-aware. Around each vertex, faces whose normals differ by
less than 60° across a shared edge share one area-weighted normal. Sharper
edges, and edges between faces wound opposite ways, keep a separate normal on
each side. Where a run of smooth edges turns that shared normal more than 60°
from one of its faces, that face's corner averages only the faces at the vertex
within 60° of its own. Every corner normal is therefore within 60° of its
face's normal, and no face is shaded against its own winding. Earlier coarse
tiles averaged all faces at a vertex, which gave rough surfaces such as rock
walls starburst gradients and, under double-sided lighting, inverted facets.
Splitting adds coarse vertices, never triangles: on the measured meshes (0.8 M
to 10 M triangles), coarse tiles of sources with normals gained 15–18% vertices
and 13–15% bytes. Coarse tiles do not carry the source's fine shading detail
(untextured classes get no normal map). Leaves are unaffected.

60° is the single crease angle, with no option. Any angle below 90° keeps every
face from shading against its own winding, for every input. 60° sits between
the measured alternatives on a synthetic rock wall: 45° shaded closest to the
source but cost 36% more coarse bytes (60°: 15%), and 80° cost only 4% but
shaded further from the source, close to the earlier averaged normals at the
coarsest levels. At 45–60°, rough coarse tiles look faceted where their own
geometry is, instead of blurred. `validate` reports any coarse corner normal
more than 90° from its face as the `coarseNormalInversion` warning
([VALIDATION.md](VALIDATION.md#coarse-normals)); archives written before this
change usually carry it.

Simplification can still leave some coarse faces wound against the source
surface they replace (issue #372): on one 10 M-triangle photogrammetry mesh, a
nearest-source estimate put about 7% of the root tile's area and 3–5% at the
next three depths against the source. Double-sided materials shade them on the
viewer's side; single-sided materials cull them, as before.

Coarse pages go through the same encoder and decoded-image gates as leaf pages,
so a profile chooses one colour codec for both (see
[Profiles](#profiles-and-resources)). A tile's pages together are bounded to
about 2²⁰ texels per slot, shared by surface area among its material classes.
A parent with many classes therefore splits one page's worth instead of taking
a full page for each.

A coarse page that misses a gate is re-encoded (UASTC level 2, then 3) like a
leaf page. A coarse page that no codec step can deliver is published as its
exact level-0 PNG and, unlike a leaf page, sampled without mips; it
never fails the run. `diagnostics.hlod.appearance_by_depth` counts these per depth
(`escalatedPages`, `pngPages`). The report's `escalated_pages` and
`escalation_diagnostics` cover leaf pages only.

Normal, metallic-roughness and other data pages, MASK coverage and ID pages
follow their own class rules in every profile.

### Leaf atlases

Leaf atlases keep only each leaf's needed source regions, with aligned crops,
wrap, gutters and per-level mips. A leaf never depends on a whole source image
merely because one triangle samples it.

When a leaf's UVs for one source image span at least one whole period of a
REPEAT or MIRRORED_REPEAT axis, the leaf needs every texel on that axis. Its
page then holds that axis whole and a crop of the other axis, published with
the source wrap and the source UVs unchanged there, instead of one unrolled copy
per period. When both axes qualify, the page is the source image itself. Such a
period page:

- has the source's own size, which a KTX2 page needs to be a multiple of four;
  otherwise ordinary crops are used;
- is used only where it holds no more texels than crops of the triangles
  actually there, so a few small triangles a period apart keep crops;
- is a texture of its own, so a leaf publishes one extra primitive (draw call)
  per distinct period page (source image, slot, sampler and MASK class);
- need not be a power of two. WebGL2 (the CesiumJS default) samples such pages
  directly; WebGL1 cannot, and CesiumJS cannot resize compressed (KTX2) images
  to fix that.

### Identity, annotation and editability

- **Leaves** keep exact `source_primitive` and `source_triangle` feature IDs for
  every triangle, with original corner order.
- **Coarse tiles** carry one `source_primitive` feature ID per triangle,
  duplicated at all three corners: the source primitive nearest to the
  triangle's decoded centroid within the tile's source range (ties by distance,
  then source order). It is a **representative**, not ancestry or a triangle
  mapping, and a primitive that no longer shows in a coarse tile need not
  appear there. Coarse tiles have no per-triangle source IDs and no
  source-triangle table. Each tile keeps one property table per ID set.
- **Annotation.** Primitive-level labels may be shown on coarse tiles as
  representative picks. Exact annotation maps source-triangle annotations onto
  **leaves**. An arbitrary region cannot propagate exactly through coarse IDs.
- **Texture editing.** A leaf is exact in texture space when no two triangles'
  decoded UVs share interior area on a page (`unique`). Overlapping, mirrored or
  repeated source UVs make it `shared`, editable in vertex space only. Coarse
  charts never overlap, so a coarse page is editable, but editing it changes
  only that coarse tile, not its leaves or other levels, and does not make its
  provenance exact.

## Profiles and resources

`--profile` accepts `fast`, `balanced` (the default), `quality` or `lossless`.
Leaf sizing, texture resolution, encoder effort and other profile constants may
change between releases; do not build scripts around today's numbers. Every
present PBR slot follows its colour or data class and the chosen profile.

Lossy profiles use in-process KTX2/Basis; `lossless` leaf textures use PNG.
`lossless` describes the leaf texel policy, not exact source GLB bytes,
arbitrary viewer filtering or lossless coarse geometry. Leaf coordinate
encoding and provenance are the same in every profile.

### Texture codecs

| Profile | Leaf colour codec | Coarse colour codec |
| --- | --- | --- |
| `fast` | ETC1S quality 50 / effort 0 | ETC1S quality 50 / effort 0 |
| `balanced` (default) | UASTC level 0 + Zstd, no RDO | ETC1S quality 90 / effort 0 |
| `quality` | UASTC + Zstd | UASTC + Zstd |
| `lossless` | PNG | PNG |

Colour-class pages are base colour and emissive. They are sRGB, with
class-correct authored mips. Other classes use the same codec in every lossy
profile:

- normal and metallic-roughness pages are UASTC;
- occlusion is scalar ETC1S quality 90 / effort 0 in `fast` and `balanced`, and
  UASTC in `quality`;
- MASK pages keep their own codec and coverage policy;
- ID pages are exact PNG in every profile.

Every leaf UASTC page, of any class or ladder step, is encoded with Basis
Universal's fastest ETC1 hints (`cPackUASTCETC1FastestHints |
cPackUASTCETC1DisableFlipAndIndividual`). The hints are used only when a
viewer transcodes UASTC to ETC1, which CesiumJS does on GPUs with none of
ASTC, BC7 or S3TC; decodes to RGBA, ASTC, BC7 and BC1/3 are the same with
either setting. Coarse UASTC pages keep the full hint search.

ETC1S quality uses the wrapper's 1–100 scale; effort 0 maps to native ETC1S
compression level 0. ETC1S at effort 0 costs about three times the encode CPU of
UASTC level 0 for smaller pages; the
[effort sweep](../bench/architecture_audit/etc1s_default/effort.md) has the
timing and byte tables.

Coarse tiles in `balanced`, `quality` and `lossless` publish a texture-aware
error (`t` = 2, 1.25 and 1.25 screen pixels per texel; see
[texture-aware G](#texture-aware-published-g)); `fast` publishes the measured
error.

On a photogrammetry test scene, delivered `balanced` leaf colour measured
46.08 dB / 0.9956 SSIM against the bake input with UASTC level 0 + Zstd, versus
35.83 dB / 0.9432 with ETC1S; projected all-leaf bytes rose from 531.9 MB to
753.0 MB ([measurement](../bench/architecture_audit/leaf_uastc/README.md)).
Neither codec recovers detail removed before baking. Higher UASTC levels add
little source fidelity.

### Decoded-image gates

Every lossy page is decoded and checked before delivery. UASTC pages also meet
the UASTC-specific decode floors; the floors below are shared minima and are
never relaxed.

Decoded RGB floors are support-weighted and checked on **every mip**, including
1×1. The leaf/delivery gate picks a band by the number of texels with positive
support, not their summed weights or the padded level area. The large band is
at least 1,024 supported texels, the middle band 64–1,023, and tiny fewer than
64. A page must meet every applicable criterion. PSNR is in dB; errors are in
RGBA8 byte units.

| Colour profile | PSNR large / middle / tiny | Large p99.9 cap | Worst supported 4×4 block RMSE cap |
| --- | --- | --- | --- |
| `fast` | 20 / 16 / 13 | 112 | 96 |
| `balanced` | 20 / 16 / 13 | 80 | 80 |
| `quality` | 27 / 23 / 23 | 64 | 48 |

- The percentile cap is skipped below 1,024 supported texels; the block cap
  always applies.
- Tiny `fast` and `balanced` levels also need **combined RGB PSNR ≥16 dB**,
  computed from the mean of the three channel MSEs, never the mean of their dB
  values. This catches correlated RGB damage while admitting error concentrated
  in one channel.
- Alpha keeps PSNR / max-error floors of 30/80 (`fast`), 30/64 (`balanced`) and
  34/32 (`quality`).
- Normal RGB keeps 30/112, plus normalized-length mean/max caps of 0.08/0.20,
  0.05/0.15 and 0.03/0.10 for the three profiles, and angular mean/p99 caps of
  3°/12°.
- Metallic-roughness keeps 30/32 on G/B. Occlusion keeps 30/80 on R (`quality`
  30/8).
- MASK decoded coverage keeps the attainable semantic gap plus 0.02, 0.01 and
  0.005 for the three profiles.

A separate source-sampler check compares decoded colour against the source as
the source sampler would read it. For `fast` and `balanced` it uses the same RGB
PSNR bands, chosen by the sampled level's **area** (width × height, including
padding) rather than its supported-texel count. So a sparsely supported 32×32
level uses the large sampler band even when its leaf gate uses the tiny band.
The joint 16 dB floor applies to decoded colour in the sampler's own tiny band.
Nearest-mip filtering rounds to the selected level; a fractional mip blend uses
the coarser participating level; non-mip filters use level zero. Maximum errors
are 160 (`fast`) and 144 (`balanced`); `quality` keeps 34 dB / 24. Before
encoding, chart correspondence to the source keeps 22/56, 22/32 and 34/24 for
the three profiles; codec error does not weaken it. These errors are in sRGB
byte space after linear-light sampling. Non-MASK alpha and data pages keep their
own sampler floors.

A valid ETC1S decode is not automatically accepted: some smooth photographic
and noisy pages miss these floors and escalate. Percentile and block caps catch
local damage. Black pages, inverted channels and an upsampled coarse checker mip
fail the gate, but PSNR does not prove mip provenance: smooth wrong-mip content
can pass even a 20 dB floor. The
[ETC1S acceptance record](../bench/architecture_audit/etc1s_default/README.md)
gives the calibration fixtures, controls and measurements.

### Escalation

In a lossy profile, a **leaf** page that misses any decoded-image gate at the
profile's encoder setting (per-level channel and normal floors, MASK coverage,
the source-sampler check) is re-encoded one step stronger instead of failing
the run: UASTC level 2, then UASTC level 3. Each step is checked by the same
gates; gates are never loosened. UASTC encodes every 4×4 block independently,
so a step can be rejected without encoding the whole chain when blocks or
coarser levels that already missed also miss when encoded alone. Such a miss is
exact; its message starts `block probe:` or `level probe:`.

If level 3 also misses, the page is delivered as an exact level-0 PNG with the
same sampler, so one page never fails the run. glTF PNG cannot carry authored
mip levels, so on a mip-sampled page the viewer generates them, outside the
decoded guarantee, as for `lossless` pages. The page's islands sit whole on the
8-texel grid with real-source gutters, so a box-filtering viewer reproduces the
authored data averages through level 3; colour and normal mips use the viewer's
own filter.

The report and the archive's `conversion.json` count these pages as
`viewer_mip_fallback_pages` and list every one (`first`, `depth`, `page`,
`slot`) in `viewer_mip_fallbacks`, uncapped. Level-0-sampled PNG pages, which
stay inside the guarantee, are counted as `lossless_fallback_pages`.

Escalation costs size: an ETC1S page re-encoded as UASTC is several times
larger, and a PNG page is usually the largest form (up to 4 bytes per texel).
The ladder depends only on the page, so output is identical for any worker
count. Cancellation is checked before every re-encode step.

### Leaf normals and provenance tables

`lossless` keeps the f32 source normal, after the node transform and
renormalized at capture. `fast`, `balanced` and `quality` store 12-bit
octahedral codes through the `EXT_meshopt_compression` OCTAHEDRAL filter;
viewers decode them to normalized 16-bit `NORMAL` (`KHR_mesh_quantization`).
The angle to the source normal is at most about 0.06° (0.0596° maximum and
0.020° mean over the 73 million leaf corners of a 24-million-triangle scene).
The bit count may change between releases.

The per-triangle provenance tables (`source_primitive`, `source_triangle`) are
losslessly meshopt-compressed in every profile.

### Workers and scratch

`--jobs auto` is memory-aware. A positive `--jobs N` is a **maximum**, also
capped by available cores and each stage's memory plan. The diagnostics report
the requested choice and actual workers for capture and leaf delivery
separately. When available memory cannot be queried, one worker is used. One
worker does not make an item fit: memory or scratch exhaustion returns
`resource_limit`.

`--scratch-dir PATH` (`--scratchDir`) selects the parent for working data.
Without it, capture uses the platform scratch default and leaf and forest
intermediates use the output's parent directory. Final archive staging is always
on the output filesystem beside the destination. Prefer disk-backed scratch for
large sources, especially when the output directory is memory-backed;
memory-backed scratch counts against the [memory budget](#memory-budget).

Capture's external sorts (corner references, positioned corners, triangles,
the cell tree's nodes and weld depths, and the corner-depth join) are planned
once against the [memory budget](#memory-budget). The fixed sort configuration,
an 80 MiB run buffer per live sort with fan-in 64, is the floor: capture's
worker plan and `capture_minimum_bytes` are computed with it, so a budget that
admits it is never refused for want of larger runs. Above the floor, runs grow
to the smallest size at which every capture sort has at most `F` initial runs,
so each sort writes its records once and merges them once, in the stream that
feeds the next step. `F` is 64, 128 or 256, whichever admits the most background
run writers. Writers sort and write full runs on their own threads while the
next run fills, at most one fewer than capture's workers, so `--jobs 1` writes
runs on the capturing thread. A budget too small for one pass even at `F = 256`
gets the largest run buffer that fits, without writers. A plan never declares
more scratch than the floor, so no scratch admission refuses a run the floor
would admit. The plan is charged in
capture's declared resident bound (`diagnostics.memory.capture_bound_bytes`)
and reported as `diagnostics.capture.sort`: `run_bytes`, `fan_in`,
`buffer_bytes`, `writers`, `max_runs`, `single_pass` and `limited_by`
(`floor`, `single_pass`, `budget`, or `unknown_memory` and
`memory_backed_scratch`, which keep the floor). It changes resources only:
each sort's output is the stable sort of its input order for every run size,
fan-in and writer count, so the archive does not depend on the plan.

The coarse stage's incidence pass (`hlod_incidence`) plans its item and
open-edge endpoint sorts the same way inside the coarse stage's admission,
before the walk admits any parent job: runs grow toward one pass at the
floor's fan-in, at which no staged scratch declaration exceeds the floor's, and
run writers are added as the stage's memory admits. The row sort, whose stream the walk
reads later, keeps the floor. The planned pass must fit the stage's memory
after its fixed terms; its resident need is reported as
`diagnostics.memory.hlod.incidence_bytes` and the plan as
`diagnostics.memory.hlod.incidence_sort` (whose `max_runs` assumes the worst
case of six items per triangle). The coarse stage's minimum is computed with the
floor.

Before starting, the run checks for enough free scratch for its largest
declared stage working set: files kept from earlier stages, the HLOD incidence
pass and source index, and each external sort's runs. Sort runs are 8 MiB chunk
files removed as soon as they are read, so a sorted stream and its output share
space. A 10% margin and per-file rounding are added. At scale this comes to
about 0.5 KB per source triangle with every vertex attribute present, or about
0.3 KB with positions and one texture coordinate set; capture stores only the
attributes some selected primitive carries.

The run also reserves output-side space for leaf staging, archive staging and a
replaced destination: three copies of 119 bytes per triangle (a conservative
estimate; measured archives are about 47) plus the encoded image bytes. Once
image dimensions are known, the image part rises to 4 bytes per source texel
for `lossless` PNG pages or 1 byte per texel for KTX2 profiles, each with a 4/3
mip allowance. These are estimates, not bounds. Output demands are added to the
scratch figure when they share a filesystem, by device id. Btrfs subvolumes of
one pool report different ids, so their demands are checked separately against
the same free space.

Each capture stage rechecks free space for its own working set on entry and
fails if its measured peak exceeds its declared bound; the diagnostics report
both (`scratch_stage_bytes`, `scratch_stage_peak_bytes`). The HLOD incidence
pass is measured the same way
(`leaf_delivery.hlod_incidence_scratch_peak_bytes`). The whole-run spill peak
(`run_scratch_peak_bytes`) must stay within `run_scratch_bound_bytes`: the
largest of capture's up-front need, the leaf stage's image-store bound
(`leaf_delivery.image_scratch_bound_bytes`) and the HLOD stage's own bound
(`diagnostics.hlod.stage_scratch_bound_bytes`), whose phases are each declared
and admitted as they start. Isolated triangles reach the incidence pass's worst
case of six interface items per triangle without a refusal. The coarse stage's
own scratch is admitted through its memory leases
(`diagnostics.hlod.scratch_components`). Capture scratch figures cover capture
files only; leaf and forest intermediates and the final archive are outside
them.

Source textures are decoded once per stage for each image and sampling class
(HLOD: colour or data chain; leaves: the source image and, when a density step
needs one, a reduced chain per class), never again under cache pressure.
Decoded images stay resident while they fit the stage's image cache. The rest
spill to scratch as 128×128 texel tiles, one file per chain, read through a
bounded tile cache with a bounded set of open files. The spill is admitted
before it is written and reported as `leaf_delivery.image_scratch_bound_bytes`,
`image_scratch_peak_bytes`, `image_scratch_bytes` and
`hlod.image_cache.scratchBytes`. `leaf_delivery.source_decode_count` counts
leaf source decodes. On memory-backed scratch, spilled tiles are charged to
memory only when the planned cache cannot hold every source image. Leaf density
chains never spill there; they are rebuilt from the decoded source when they do
not fit.

A free-space check is not a reservation against other writers. Working files
are removed on success and failure. A cleanup failure reports the paths
involved and does not replace the original error.

The internal diagnostic audit (`RT_HLOD_AUDIT_DIR`) reserves at most 64 MiB on
a memory-backed destination, counted in both the memory plan and its write
budget. Exceeding it refuses the run with `resource_limit`; it never truncates
the audit or skips work. On ordinary disk the audit checks free space as the
file grows.

### Size limits

There is no total source-byte or triangle-count refusal. JSON, hierarchy,
per-item and format limits still apply. A `.3tz` holds at most 178,956,970
members, including the root document, reports and index; more is
`unsupported`. Per-tile budgets come from the profile. `mesh` has no
`--leaf-triangles`, coarse-tile triangle or error option, explicit or
full-detail selector, texture-format option, external encoder fallback or
compression bypass.

### Memory budget

One declared ceiling governs the whole run, with the same rule on every surface:

| Surface | Spelling |
| --- | --- |
| CLI | `--memory-budget BYTES` (`--memoryBudget`) |
| Python | `mesh_to_tileset(..., memory_budget_bytes=None)` |
| Rust | `TilerRequest::with_memory_budget(bytes)` |

- **Values.** A positive integer number of bytes. The CLI also accepts the
  binary suffixes `B`, `KiB`, `MiB`, `GiB` and `TiB` (`2GiB`); Python takes an
  `int` and Rust a `u64`. Zero, signs, fractions, exponents, decimal-unit
  (`GB`) or lowercase suffixes, booleans (Python: `TypeError`) and values above
  2⁶⁴−1 are `invalid_request`. They are refused before any source, output or
  offset file is touched. A CLI usage error exits 2 with
  `error.code="usage"`, like `--jobs`.
- **Default.** The lesser of **4 GiB** and half the detected host or cgroup
  headroom; 2 GiB when headroom cannot be detected. An explicit budget is a
  ceiling that is never silently raised: every stage plans against the lesser
  of the budget and the detected headroom.
- **What it covers.** Capture, the leaf stage and the coarse stage admit their
  work against it before allocating: source and JSON metadata, sort buffers
  (capture grows its sort runs and run writers into the budget it is given; see
  [workers and scratch](#workers-and-scratch)),
  interface incidence, child tiles held for their parents and the active
  parents, decoded source images, chart/page/codec workspace, memory-backed
  capture scratch and the archive stage's resident buffers
  (`archive_minimum_bytes`). A source without images reserves no texture,
  chart, mip or codec workspace.
- **Source images.** Each decode of a source image is charged at the image's
  actual encoded length, which decoding holds whole, plus the decoder's working
  memory (at most 8 bytes per texel) and the decoded RGBA8 copy. There is no
  fixed encoded-size allowance. Before it writes any scratch, capture reads each
  image's header (PNG `IHDR`, or the first JPEG frame header after skipping
  earlier segments, holding at most 64 KiB at a time) and checks the largest
  decode charge against what its own resident bound leaves for decoding. Each decode is checked again against the
  decoder's own header, which must declare the same dimensions. An image the
  budget cannot hold is a `resource_limit` refusal; over 1 GiB encoded or
  16,384 px per edge is `unsupported` whatever the budget. For example, an
  8192² PNG of 110 MB built under a 4 GiB budget with a peak resident set of
  about 0.83 GiB; the reported `minimum_bytes` of that run was 3.6 GiB, mostly
  the leaf stage's page workspace for the full-size image.
- **Memory-backed storage.** Files on memory-backed (tmpfs) storage are charged
  too. Capture scratch there is charged at its declared need. After capture,
  each other location on tmpfs is charged one copy of the output-side
  reservation (see [scratch](#workers-and-scratch)) plus 32 MiB for manifests,
  documents and archive scratch. These locations are the private work directory
  (leaf and coarse payloads, forest staging and archive scratch) and the archive
  staged beside the output. Later stages plan against the budget less that
  charge.
- **Writes to tmpfs.** Every write into charged storage is admitted before it
  lands, in whole allocation units plus 1 KiB per file, link or directory. The
  allocation unit is the transparent huge page, usually 2 MiB, on a tmpfs whose
  `huge=` option is `always` or `within_size`, or when huge pages are forced
  globally, because one written byte can take a whole huge page there. The
  archive's index, central directory and end records are admitted together, at
  their exact size. A write that would exceed the charge is a `resource_limit`
  refusal (`use a disk-backed scratch directory` or `stage the output on disk`),
  so tmpfs never holds more than was charged. The estimate is generous for
  textured sources and tighter for untextured ones (about 1.4× a measured
  untextured 0.79M-triangle crop), so disk remains the better place for large
  outputs.
- **Overlay filesystems.** An overlay is classified by its writable upper layer,
  read from `/proc/self/mountinfo` by the path's `statx` mount ID. Kernels older
  than 5.8 fall back to mount-table order, reported as `mount_by_prefix`.
- **Containers.** Inside most containers the root filesystem is an overlay whose
  writable layer lives outside the container, so its kind cannot be resolved.
  It is charged as memory-backed to fail safe, reported as
  `memory_backed_assumed` and explained in `memory_backed_storage.note`. If it
  is on disk, put the output and `--scratch-dir` on a mounted volume (a bind or
  named volume is detected as disk), or set `RUSTY_TILES_STORAGE_DISK=1` to
  declare unresolvable storage disk-backed. It is then reported as
  `disk_declared` and not charged. The variable never reclassifies storage
  detected as tmpfs or ramfs; values other than `1` or `0` are refused.
- **Pressure never changes output.** It serializes work (`--jobs` stays a
  maximum under the same budget). Work that cannot fit even alone is a
  `resource_limit` refusal naming the stage and the bytes it needs, before that
  stage allocates and before anything is published.
- **One declared exception.** The leaf stage may be unable to fit its largest
  leaf page beside its largest leaf closure in the declared budget (the explicit
  one, otherwise the 4 GiB default ceiling) less everything else charged to it
  (memory-backed scratch, work and archive storage, metadata and the decoded
  source cache floor). Leaves whose pages do not fit then take the next texture
  density step instead of refusing the run; the report counts them in
  `memory_reduced_leaves`. Output therefore depends on the request and on
  whether scratch, work directory or output staging is memory-backed, not on
  detected headroom. Under tight detected headroom the default budget resolves
  below 4 GiB (half the headroom), but density planning does not follow it: such
  a run can still refuse where an explicit, smaller `--memory-budget` steps the
  density and succeeds. Archives are identical for every budget that admits the
  run without a density step.
- **Reporting.** `diagnostics.memory` holds the `budget` (`requested_bytes`,
  `resolved_bytes`, `source`: `explicit`, `default_ceiling`, `default_headroom`
  or `default_undetected`, and the detected headroom); the least budget that
  admits each stage (`minimum_bytes`, `capture_minimum_bytes`,
  `leaf_minimum_bytes`, `archive_minimum_bytes`); the coarse stage's plan
  (`hlod`); and the memory-backed storage charges with what was measured
  against them (`memory_backed_storage`, summed in
  `budget.memory_backed_storage_bytes`). `diagnostics.hlod.lease_peak_bytes` is
  the coarse stage's highest simultaneous lease. The minima include charged
  request facts such as the lengths of the input, output and scratch paths, and
  headroom is a host observation, so the same conversion reports slightly
  different minima from differently named paths (a few hundred bytes to about
  130 kB on a 0.79M-triangle crop). The archive does not depend on this
  accounting.
- **What it is not.** It is an admission and planning ceiling, not an
  operating-system limit or a peak-RSS guarantee.

### Primitives, atlases and editability

Each tile publishes as few glTF primitives as glTF allows. Source primitives
share one node, mesh, primitive, material and texture page set when their
materials belong to one class and they carry the same vertex attributes. A
material class means equal factors, `alphaMode`, MASK `alphaCutoff`,
`doubleSided`, the same bound slots with equal normal scale and occlusion
strength, and equal source min/mag filters. An omitted value equals its glTF
default; texture identity and wrap modes do not count. A leaf's merged
positions are rebased onto one shared quantization offset without being
quantized again, so every decoded vertex is bit-identical to a per-primitive
encoding.

Primitives split only where those properties differ, where a mipmapped source
sampler keeps a dedicated leaf page in a lossy profile, where repeated UVs keep
a [period page](#leaf-atlases) of their own, or where a class overflows one
page. Every leaf triangle keeps its `source_primitive` and `source_triangle`
feature IDs; a coarse triangle carries one representative `source_primitive` ID
([identity](#identity-annotation-and-editability)). Each tile has one property
table per feature-ID set. Feature IDs are exact per-face vertex attributes, so
no ID textures are baked.

Leaves are exact in vertex space: every triangle keeps its source provenance.
They are also exact in texture space when no two triangles' decoded UVs share
interior area on a page, so each texel has one owner; the tiler checks this
after atlasing. Overlapping, mirrored or repeated source UVs make a leaf
`shared`, editable in vertex space only; such a leaf is never refused.

Feature-ID attributes are UNSIGNED_BYTE, UNSIGNED_SHORT or FLOAT. rusty-tiles
does not write UNSIGNED_INT vertex attributes, which glTF advises against.
FLOAT holds every integer only up to 2^24; a larger ID is `unsupported` rather
than rounded.

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
scratch_dir=None, memory_budget_bytes=None, anchor=None, orientation_xyzw=None,
scene_offset=None, source_crs=None, source_axes=None, source_offset=None,
source_offset_file=None, height_offset=None, force=False, callback=None)`
accepts `str` or `os.PathLike` paths. The keywords follow the CLI's rules:

- `source_crs=None` or `"local"` keeps local input. `source_axes` is `"y-up"`
  (the default with a CRS) or `"xyz"`, `source_offset` a two- or three-number
  sequence, `source_offset_file` a path, and `height_offset` defaults to 0.
- `jobs=None` means auto; a positive integer is a maximum. Booleans and
  non-integer worker counts are rejected.
- `memory_budget_bytes` is the [memory budget](#memory-budget): a positive
  `int` (not a `bool`), checked before any file is read; `None` takes the
  default.
- Unknown keywords raise `TypeError`.

Python calls the same core operation without launching the CLI or an encoder,
and releases the GIL during native work.

The frozen `TilerResult` has the resolved absolute `output: pathlib.Path`,
`archive=True`, `report` and `diagnostics` dictionaries, and
`cleanup_diagnostics`. Frozen attributes do not make the dictionaries deeply
immutable. Paths are resolved before callbacks run, so a callback that changes
the current directory cannot redirect the output. Callbacks run before
installation and can fail the run; the winning callback exception is re-raised
as the original Python exception. Python errors raised after installation, while
building the result or from unrelated asynchronous sources, are outside the
installation guarantee.

### Decoded tile views

Viewers that read neither `EXT_meshopt_compression` nor `KHR_texture_basisu`
can show a tile through the Python view functions. Blender 5.2's bundled glTF
importer is one: it has no KTX2 support, and it fails on the explicit
`"filter": "NONE"` that meshopt views in archives from 0.4.0 and earlier carry
(later archives omit it, #370).

- `rusty_tiles.list_tiles(archive)` lists each content URI with its depth,
  `leaf` or `coarse` role, geometric error and accumulated transform.
- `rusty_tiles.decode_tile(archive, uri)` returns a plain glTF 2.0 GLB: meshopt
  views decoded, KTX2 pages transcoded to PNG at level 0. Samplers, materials,
  quantized accessors, feature IDs and property tables are kept.
- `rusty_tiles.decode_tile(archive, uri, format="arrays")` returns NumPy arrays
  per primitive, RGBA pages and property tables.

`archive` is a `.3tz` or an unpacked tileset directory. Positions are in the
Z-up tile frame. Only the root carries a transform, so every tile shares the
local frame described in [How source positions are mapped](#how-source-positions-are-mapped)
or [rigid placement](#source-and-placement), and `transform` maps it to ECEF.
A decoded tile is a view for display and inspection, not a reversible round
trip: an edited view cannot be re-encoded or published back. See the
[Python reference](../bindings/python/README.md#decoded-tile-views).

## Reports and errors

`--json` writes one stdout object with `ok`, resolved `output`, `tilerReport`,
`diagnostics` and `cleanupDiagnostics`. `counts` and `conversionReport` locate
the same archive-resident `conversion.json`. The versioned `tilerReport` uses
snake_case and equals Python's `result.report` and the published report.

The report includes:

- profile, source and resource counts, leaf/triangle/GLB counts and maximum
  leaf geometry error, hierarchy/implicit/document counts, depth and
  packed/synthetic leaf counts, member limits, separate root and tileset
  errors, and normalized placement;
- `escalated_pages` (`uastc_level2` and `uastc_level3` counts, by the step that
  delivered the page), `lossless_fallback_pages`, and
  `viewer_mip_fallback_pages` with its uncapped list `viewer_mip_fallbacks`
  ([escalation](#escalation));
- `escalation_diagnostics`: the first 32 escalated pages in leaf order, each
  with its `leaf`, `first` source triangle rank and `depth`, `page`, `slot`,
  `sources` (`image`, `material`), delivering `step`, and every gate it missed
  on the way (`misses`: `step`, `mip` or null for the all-level sampler check,
  `gate`). `escalation_diagnostics_omitted` counts escalated pages beyond that
  list;
- `texel_ownership` for texture-space editability: `unique_leaves`,
  `shared_leaves`, the `shared_triangles` of shared leaves, `unique_proxies`
  (every coarse tile, since coarse charts never overlap; the name is
  historical), and `shared`. That list holds the first 64 shared leaves
  (`leaf`, `first`, `depth`, `texel_ownership: "shared"`, `triangles`);
  `shared_omitted` counts the rest. Every leaf not listed or omitted is
  `unique`. Ownership is all-or-nothing per leaf: one pair of overlapping
  footprints marks the whole leaf `shared`, so compare `shared_triangles` (and
  a leaf row's `sharedTriangles`) with the leaf's `triangles` to see how much
  is affected;
- `source_coordinates` (`local-gltf` or `source-crs`) and `coordinates`
  (`local-gltf` or `wgs84-ecef`). A source CRS adds `source_crs` (definition,
  axes, source offset, height offset, `portable` or `native` operation, the
  chosen origin, `jacobian_lattice_cells` and `jacobian_exact_cells`) and
  reports its ENU frame as a `wgs84` placement at that origin;
- `root_transform`: null for local output, whose root has no transform, or the
  exact column-major matrix for placed output.

Runtime diagnostics stay **outside the archive**, so worker and scratch choices
do not change its bytes. They include `requested_jobs`, `capture`,
`leaf_delivery`, `archive_bytes`, `peak_held_records`, `phase_seconds`, the
`memory` budget and stage plan ([memory budget](#memory-budget)), and the coarse
stage's `hlod` facts (tiles, workers, CPU seconds, lease peak,
`error_measurement` and the per-depth appearance summary: page texels, covered
texels, cap hits and the requested and achieved spacing). `memory` and `hlod`
are present in every result.

- `phase_seconds` holds the wall seconds of `capture`, `leaf_build`,
  `hlod_incidence`, `hlod_geometry` and `archive`. `phase_cpu_seconds` holds
  the process CPU seconds of the same phases (all workers, including admission
  work between phases; null where process CPU is unavailable).
- `leaf_delivery.timing` is the leaf stage's timing: process-wide counters over
  this job's leaf-stage interval, so a concurrent job in the same process adds
  to them. It holds summed per-thread seconds, CPU seconds and calls per step
  (`source_reconstruct`, `source_decode`, `density_mips`, `basis_encode`,
  `codec_verify`, `glb_write`, `member_write`, ...) and texel counters.
- Stage objects describe worker admission and owned memory and scratch. Their
  accounting is declared, not a measured RSS guarantee. Absolute scratch paths
  and resource observations are diagnostics, not profile constants.

`--progress json` writes NDJSON work events to stderr with named phases and
nullable totals: `conversion`, `capture`, then inside the leaf stage
`leaf_build` (leaves), `hlod_incidence` and `hlod_geometry` (coarse stage),
then `leaf_payloads`, `archive` and `ready_to_publish`. Each finished phase also
logs its wall seconds and the run's elapsed seconds as a `log` event. Inside
`capture`, each step is a phase whose total is known when it starts:
`capture_scan` (triangles), `capture_validate` (one unit), `capture_corners`
(corner references read and sorted), `capture_positions` (corners joined with
their attributes), `capture_triangles` (triangles reassembled),
`capture_sequence` (triangles written in rank order), `capture_images` (two
units per image), `capture_tree` (triangles), `capture_corner_depths` (seven
units per triangle) and `capture_leaf_plans` (leaves). Each step reports `0`
and its total; all but `capture_validate`, `capture_tree` and
`capture_leaf_plans` also report counts between. An intermediate sort merge pass, which a budget
too small for one pass needs, is reported as `capture_merge` (records
rewritten, counted on across passes so the count never goes back), after which
the interrupted step resumes. While capture runs, the
current step's latest count is repeated at least every 250 ms. These events are
capture's cancellation checkpoints, also for a Python callback, so a cancel or
callback exception lands within about a second, as in the leaf stage. The coarse
stage's `hlod_incidence` phase reports its steps the same way:
`hlod_incidence_items` (triangles read into their leaves' interface items),
`hlod_incidence_reduce` (items merged into shared rows) and `hlod_incidence_ends`
(two units per open-edge endpoint), with `hlod_incidence_merge` for an
intermediate merge pass. Required
events finish before publication; a successful result means the archive is
installed. A cleanup problem after installation is still success, with cleanup
diagnostics.

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

CLI parser and usage errors happen before the core; with `--json` they exit 2
with `error.code="usage"`. Core errors carry their domain `kind`, secondary
diagnostics and retained paths. Request, placement and output options are
validated before the source is read; later failures report their original
cause, and cleanup never replaces it. `doctor --command mesh` checks build
capability without writing anything; it does not approve a particular source,
resource plan, filesystem or viewer.

## Known Cesium colour limitation

CesiumJS 1.146.0 uploads KTX2 sRGB colour textures as linear (UNORM) formats on
the ASTC, BC7, ETC2 and uncompressed paths, so filtering happens on encoded
values. In the tested Chromium WebGL2 setup with an NVIDIA RTX 4080 SUPER,
high-contrast base-colour and emissive textures filtered incorrectly despite
correct authored sRGB mip levels and transfer metadata. This can change their
appearance, especially between texels or mip levels. The worst errors measured
on high-contrast test textures were 72/255 (base colour) and 61/255 (emissive).
Linear data textures (metallic-roughness, occlusion, normal) were unaffected
apart from codec error. This was measured on one setup; it is not a claim about
all devices or other viewers.

The tiler keeps its class-correct textures and adds no viewer-specific colour
correction. Do not edit the pages to compensate. In the same probe, uploading
the KTX2 payload with a matching sRGB format removed the error; that is a
loader change, and no CesiumJS release or setting that does it has been tested.
If colour accuracy matters, check the result in your own viewer.

### Colour space and lighting in CesiumJS

Base colour and emissive pages of leaves and coarse tiles are sRGB-encoded.
Lossy profiles carry the sRGB transfer in the KTX2 container; coarse pages are
sampled in linear light and re-encoded to sRGB when baked. Metallic-roughness,
occlusion and normal pages are linear data.

Whether a tile is lit depends on its vertex attributes; the tiler has no
lighting or colour-space option. A source without NORMAL (typical of
photogrammetry exports) gives leaves with POSITION and TEXCOORD_0 only, which
CesiumJS 1.146 draws unlit, showing the texture colour as sampled. Coarse tiles
publish NORMAL only where the source has normals, so leaves and coarse tiles of
one source shade the same way, without the seams and tint step a lit coarse
tile beside unlit leaves would show. A source with normals is lit for leaves
and coarse tiles alike, so its apparent brightness follows Cesium's lighting.

Verified: the worst-case filtering errors above, from shader samples of KTX2
pages; the lit-versus-unlit mismatch and its removal, by a visual check (not a
pixel measurement) on one 0.79M-triangle photogrammetry crop in CesiumJS 1.146;
and the coarse tiles' attribute set against the leaves, which the tests pin.

Not verified: how much of the filtering error shows on the unlit path compared
with the lit one; other CesiumJS versions, GPUs, browsers and viewers; whether
PNG pages (`lossless` profile) avoid the error; and any lighting setting that
hides a tint step on a source with normals.

## Migration from older mesh converters

`mesh` replaces `mesh-to-3tz` (alias `meshTo3tz`) and `mesh-local-to-3tz`. The
old command names are now unknown subcommands (exit 2). Their output cannot be
reproduced; re-run `mesh` on the source.

- **Source CRS.** `--source-crs epsg:3857 --source-offset E N A` becomes
  `--source-crs EPSG:3857 --source-offset E N A`, with the default `y-up` axes
  and zero height offset. `--source-offset-file` accepts only the strict
  `E:`/`N:`/`A:` format. Automatic detection (`auto`) and the `geographic` and
  Web Mercator keywords are gone: name the CRS, for example `EPSG:4326`.
- **Placement.** Euler placement (`--cartographic-position-degrees`,
  `--rotation-degrees`) has no direct replacement: use `--anchor` and
  `--orientation-xyzw`.
- **Tile and texture options.** Triangle, byte and error limits, atlas sizes,
  texel density, `--explicit`, `--node-features`, `--no-meshopt` and texture
  output selectors (`--texture-format`, `--basisu`, Python `texture_format`)
  have no one-to-one replacement: choose a [profile](#profiles-and-resources).
  JPEG sources are still read; JPEG and WebP output are no longer written.
- **Identity.** Primitive and triangle source identities are always carried for
  leaf picking. Leaf-local table row IDs are not global business IDs.
- **Explicit output.** There is no explicit output mode. The implicit forest may
  still contain an explicit skeleton and link documents.

`glb-to-3tz` / `glb_to_3tz` and `createTilesetJson` / `model_to_manifest`
remain the separate exact-byte packaging and sibling-manifest commands.
