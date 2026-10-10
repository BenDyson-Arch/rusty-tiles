This additive STATIC_ONLY revision replaces one mutable contextual architecture-document pin with a byte-exact immutable snapshot. The139 other external pins remain unchanged. Original4433a13a package bytes remain untouched; no source, fixture, oracle, expected record or compile recipe changes.

`root_runner.py` is the original runner with two specific location changes: HERE names the original package, and external input-pin reads name this revision. Original source/package integrity continues to be checked; this revision must receive its own independent pin/source review before execution. Root supplies the pinned linker-wrapper PATH and a fresh CODEC_LINK_TRACE_DIR. No target or probe has run during revision authorship.

Root-only command after reviewer approval:

```
python3 /tmp/rusty-tiles-codec-owned-plan-context-revision/root_runner.py --out-dir ROOT_CHOSEN_NEW_DIRECTORY
```

See revision.json and runner.diff for exact old/new hashes and delta. This context fix carries no codec, default or whole-slice approval.
