### Fixed

- Large-mesh bakes decode each source texture once per stage for each image and
  sampling class. The HLOD and leaf texture caches held whole decoded images
  under a memory budget and decoded a source again on every eviction (an 8192²
  Walem bake spent 65,871 of 68,657 HLOD CPU seconds re-decoding). Images and
  chains now stay resident while they fit the cache; the rest spill to declared
  scratch as 128×128 texel tiles read through a bounded cache. Output is
  unchanged. Diagnostics add leaf source decode counts, image scratch, leaf
  stage timing over the job's interval (decode, encode) and per-phase CPU
  seconds.
