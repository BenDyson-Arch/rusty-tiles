### Changed

- `mesh`: proxy texture pages are several times smaller. The proxy texel density
  policy was already applied as documented. The page size came from the atlas:
  proxy faces are often under one square texel, and charts could not cross
  source-primitive or material boundaries, so most charts held one face plus a
  gutter of about 7 x 7 texels. Pages were also shelf-packed into 4096-texel-wide
  strips. Charts now grow over edge and corner neighbours across one material
  class (a face under one square texel joins at up to 72° from the chart normal
  without raising the chart's density; others keep the 26° cone). Each chart keeps a
  `gutter`-texel margin per side instead of one texel more. Shelves are packed
  tallest first on the power-of-two width whose pages need the fewest texels,
  with ties going to the squarest. On a 0.79M-triangle photogrammetry crop, proxy
  page texels fall from 53.7M to 7.1M, the archive from 84.3 MB to 73.0 MB, and
  the explicit depth 0–4 contents from 3.49 MB to 1.87 MB. On the 2048-texture
  "mgal_detail_offset" model the archive falls from 1.74 GB to 1.54 GB, the
  explicit depth 0–4 contents from 48.8 MB to 34.1 MB (base texels 70M to 20M),
  and the proxy bake from 99 to 51 minutes. The bake now rasterises each page's
  faces by source, so the shared image cache keeps its hits. Leaf tiles are
  byte-identical.
