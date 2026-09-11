#!/usr/bin/env bash
set -euo pipefail
# Build the pinned native encoder inside this package's ignored target tree.
package_dir="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"
tools_dir="$package_dir/target/tools"
mkdir -p "$tools_dir"
archive="$tools_dir/basis-universal-1.60.tar.gz"
if [[ ! -f "$archive" ]]; then
  curl --fail --location https://codeload.github.com/BinomialLLC/basis_universal/tar.gz/refs/tags/v1_60 --output "$archive"
fi
printf '%s  %s\n' '64ac9363656dc3eb41c59ee52af7e939abe574a92c85fd0ba27008c4a7ec9f40' "$archive" | sha256sum --check --status
tar -xzf "$archive" -C "$tools_dir"
sse=OFF
if [[ -r /proc/cpuinfo ]] && grep -q sse4_1 /proc/cpuinfo; then sse=ON; fi
cmake -S "$tools_dir/basis_universal-1_60" -B "$tools_dir/basis-build" -DCMAKE_BUILD_TYPE=Release -DSSE="$sse"
cmake --build "$tools_dir/basis-build" --target basisu --parallel "${CMAKE_BUILD_PARALLEL_LEVEL:-4}"
cp "$tools_dir/basis_universal-1_60/bin/basisu" "$tools_dir/basisu"
"$tools_dir/basisu" -version
