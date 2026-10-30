### Fixed

- HLOD: dense photogrammetry no longer omits coarse tiles whose every reduction folded nearby surface layers onto each other (refused by the fold guard). As a last resort, after every other strategy, such a cell is reduced in guard-checked steps and border-adjacent vertices are merged. Tiles other strategies build are unchanged. On the full Walem model, omitted cells go from 48 to 17.
