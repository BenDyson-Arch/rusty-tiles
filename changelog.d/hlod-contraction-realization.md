### Fixed

- HLOD: far fewer coarse tiles are omitted on building-dense inputs (mesh-1e7: 132 to about 36). Border contractions that stalled are now realized. Wall tops that touch a tile border keep their shape under reduction. These last resorts run only after the existing strategies fail, so every tile those strategies build is unchanged.
