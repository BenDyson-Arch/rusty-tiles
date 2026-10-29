### Fixed

- `mesh`: a lossy leaf texture page that misses its decoded gates at every codec step is now delivered as its exact level-0 PNG even when its sampler mipmaps, instead of failing the run (`unsupported`). The viewer generates that page's coarser levels, outside the decoded guarantee, as for `lossless` pages; such pages are counted in `lossless_fallback_pages`. The textured synthetic city (`texture-1e5-1024`) now converts at `balanced`.
- `mesh`: tile boxes have a half length of at least 0.1 mm, so a flat tile (one planar triangle) no longer writes a zero axis.
