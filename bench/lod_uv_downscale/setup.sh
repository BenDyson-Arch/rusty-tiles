#!/usr/bin/env bash
set -euo pipefail
cd /home/bend/.cache/rt-wt/lod-uv-downscale
export CARGO_TARGET_DIR=/home/bend/.cache/rt-wt/target-lod-uv-downscale CARGO_BUILD_JOBS=3
scratch=/home/bend/scratch/rusty-tiles-scratch/lod-bakeoff/uv-downscale
mkdir -p "$scratch" "$CARGO_TARGET_DIR"
# meshopt 0.6.2 is pinned in this worktree's Cargo.lock; use its exact vendored C ABI.
meshsrc=/home/bend/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/meshopt-0.6.2/vendor/src
chrt -i 0 nice -n 19 g++ -O2 -shared -fPIC "$meshsrc"/{allocator,simplifier,vertexcodec,indexcodec,vertexfilter}.cpp -o "$CARGO_TARGET_DIR/libmeshopt.so"
chrt -i 0 nice -n 19 /home/bend/.local/bin/uv venv "$scratch/venv" --python /home/bend/.local/bin/python3.12
chrt -i 0 nice -n 19 /home/bend/.local/bin/uv pip install --python "$scratch/venv/bin/python" -r bench/lod_uv_downscale/requirements.txt
