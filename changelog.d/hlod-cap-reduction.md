### Fixed

- HLOD: fewer coarse tiles are omitted under the triangle cap. As a last resort, a parent that no other strategy fits drops whole interior components (smallest first, declared, measured G) and rebuilds flat wall panels from their outlines. Tiles the other strategies build are unchanged.
