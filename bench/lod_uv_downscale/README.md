# UV-preserving downscale experiment

Standalone, deliberately conservative research prototype. Retain published tex3 leaves, their
primitive IDs and tree. For crop3 use an equivalent centroid octree (8192 triangles per leaf).
Replace proxy construction with meshoptimizer 0.6.2 attribute simplification, locking every
chart/primitive and inherited leaf border. Repack half-resolution child atlas rectangles;
never query source surfaces during construction. Preserve full leaf provenance unchanged.

Invariants: simplification uses a single primitive ID per independently simplified group;
UV-discontinuous vertices remain distinct; chart and leaf border indices cannot collapse;
output coordinates round-trip exactly through glTF FLOAT plus a shared node translation.
The supported domain is only the two supplied known-layout inputs and their baseline GLBs;
this reader is not a general validator for arbitrary glTF hierarchies or material/sampler modes.
The conservative inherited borders may impose a high triangle floor; this is measured.

Acceptance: sampled two-sided distance and colour against original input; decode candidate
GLBs again; census every source-topology adjacency at every depth combination, including
same-depth. Exact retained seam edges provide continuous (not just point) agreement. Source
holes are excluded from cross-tile adjacency. Report limits of this topology census explicitly.
CPU is process user+sys plus waited child CPU, normalized by source Mtri, including supplied
leaf GLB/meshopt decoding, KTX2 transcoding, simplification, repack, encoding and writes.
Constructing/copying the full-detail leaves and metric computation are excluded. Output uses
PNG and uncompressed FLOAT geometry, so codec-matched production acceptance is not claimed.

`run.py` uses numpy/scipy/Pillow/Open3D and meshoptimizer 0.6.2 C ABI. `decode_basis.cjs`
uses the installed three.js Basis transcoder solely to decode supplied KTX2 textures.
`run.sh` records the commands, paths, priority and process resource measurements.
`validate.py` checks sensitive metric controls and exact leaf attributes/provenance.
`report.py` renders the external evidence README from measured JSON without rerunning work.
Bulk evidence is outside git at
`~/.cache/rusty-tiles-evidence/lod-bakeoff-20261024/uv-downscale/`.

Evidence report SHA-256: `cf760fd234004c6aabf2e75e20b3b8f84988380458e6ed924cfcb225fc8a396d`. This evidence was generated externally and never
tracked in git; the accepted prototype commit and full source/input hashes are in the
external `execution.json`. Per-file prototype GLB hashes are in `*-artifact-sha256.json`.
