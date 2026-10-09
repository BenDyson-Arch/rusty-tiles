# Independent F1b1 review reproduction

`replay.py` independently authors its inputs and validates the CLI results. It imports no rusty-tiles production or acceptance-oracle modules. PNG chunks, CRCs, compressed scanlines and GLB framing use Python's standard library; Pillow supplies independently decoded JPEG provenance.

Requires Python 3 and Pillow. The original run used Pillow 12.3.0. Choose a fresh work directory outside the repository:

```sh
python3 -B bench/architecture_audit/f1b1/review_probes/replay.py generate --work /path/to/fresh-work
python3 -B bench/architecture_audit/f1b1/review_probes/replay.py check --work /path/to/fresh-work --binary /path/to/rusty-tiles --output /path/to/receipt.json
```

`generate` refuses an existing work directory and hashes each generated input plus its own script in `inputs.json`. `check` verifies these hashes before executing the CLI. Optional `--expected-sha256 SHA` also pins the executable. Receipts record binary, input and generator identities, actual typed outcomes, per-case timings and positive exact-image/report assertions. Timing observations have no portable pass threshold.

The cases cover eight-bit/16-bit images, terminal PNG/JPEG framing, critical and ancillary CRCs, APNG exclusion, full raster/entropy truncation, repeated UV references, exact PNG/JPEG forwarding and a two-leaf reflected selected-scene closure with shared-image identity and sampler/texture-info omission distinctions. Large generated fixtures and output archives remain in the chosen work directory.
