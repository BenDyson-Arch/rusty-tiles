### Fixed

- Large-mesh bakes decode each source texture once per stage. The HLOD and leaf
  texture caches held whole decoded images under a memory budget and decoded a
  source again on every eviction (an 8192² Walem bake spent 65,871 of 68,657
  HLOD CPU seconds re-decoding). Chains now stay resident while they fit the
  budget; the rest spill to declared scratch as 128×128 texel tiles read
  through a bounded cache. Output is unchanged. Leaf diagnostics report source
  decode counts, image scratch and per-stage (decode, encode) timing.
