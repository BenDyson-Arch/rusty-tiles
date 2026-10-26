### Changed

- Mesh archives declare themselves HLOD deliveries in `tileset.json`
  (`asset.extras.rustyTiles.lod` is `"hlod"`), so `validate` applies every HLOD
  check without `--expectProfile hlod` and refuses a copy stripped of its
  identity labels. This changes `tileset.json` bytes.
- Leaf texture pages whose source sampler does not mipmap (minification
  `NEAREST` or `LINEAR`) are delivered as level-0-only KTX2 instead of a complete
  chain the sampler never read: less encode work and fewer bytes, unchanged
  rendering.
- The mesh memory budget now charges memory-backed (tmpfs) output and
  intermediate storage: the private work directory and the staged archive each
  take one copy of the output reservation plus 32 MiB from the budget when they
  live on tmpfs, and a measured owner past its charge is a `resource_limit`
  refusal. `diagnostics.memory.memory_backed_storage` reports the charges and
  measurements; `budget.memory_backed_storage_bytes` is their sum.
