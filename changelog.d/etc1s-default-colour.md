### Changed

- Mesh colour textures (leaf and coarse base colour/emissive pages) now default
  to KTX2 ETC1S: quality 50 in `fast`, 90 in `balanced`. `quality` keeps UASTC
  and `lossless` keeps PNG; decoded appearance gates and escalation remain active.
