# Terrain T1 initial audit evidence

This is baseline investigation, not replacement-producer acceptance. The
[representation decision](../../../docs/architecture/terrain-representation-decision.md)
records the finite standard replacement and remaining proof obligations.

`baseline-probe.py` authored constant, explicit-mask and Float64 GeoTIFF sources
using GDAL. Literal source values, transforms and options are in the script;
`baseline-results.json` records executed outputs. Baseline binary
`/home/bend/.cache/rusty-tiles-132-12d98ab-native` has SHA256
`b38d6726bb4271b725b3a08292b8b45fb9be977ac6368f70e465814d5146b334`;
its terrain source matches develop `5df15e7`. The audit establishes Float64
sampling precision loss before quantization, and supported mask behavior only
for that fixture. It does not mistake raw delivery or absent optional normals
for a format defect.

`baseline-decode.py` independently reads little-endian quantized-mesh bytes,
including optional gzip, zigzag vertex arrays, high-water indices, edge lists and
extension framing. It checks winding, incidence and exact summed domain area.
Those checks alone do not prove absence of overlapping triangles; the replacement
needs complete reference topology/directed incidence. Truncation, attribute and
index corruptions were rejected during the initial audit. This reader shares no
historical encoder or GDAL resampling implementation.

`quantized_reader.py` is an initial independent reader and literal-grid
sampler, not a complete acceptance runner. The revised GLB replacement still
needs its own finished source-to-rendered-output matrix and mutation controls.
The uncommitted source/lifecycle groundwork in the isolated terrain worktree is
also not an accepted producer. Eight focused native library tests pass; they do
not certify the abandoned quantized output plan or CLI migration.

A small baseline grid129/error1/zoom8 measurement emitted 15 tiles in 0.622 s
with 101784 KiB child peak RSS via `resource.getrusage`. It is not a fixed-option
scaling series, a whole-job memory bound or a replacement resource result.
