### Changed

- Mesh: coarse HLOD charts and leaf crop islands now pack by their covered shape instead of their bounding rectangles. Shapes interlock without sharing a cell, so mip isolation, gutters and exact leaf sampling are unchanged. Leaf island pages are sized in whole 4×4 blocks instead of powers of two. On the Walem photogrammetry model, coarse page occupancy rises from 0.39 to 0.47 for about 2.2× the (small) chart CPU. Leaf page occupancy rises from 0.18.
- Mesh: coarse HLOD texels are spaced at 1.25·G/16 instead of G/16 (D5 amendment). With D10's error target, this keeps first views within the Metashape byte floor at the measured packing occupancy, with texel sharpness parity where it fits.
