#!/usr/bin/env bash
# ML1 spike evidence for the vendored Basis Universal encoder on one target.
#
# 1. Stdio isolation (Unix targets): the vendored and shim archives must not
#    reference the C stdio output family directly; only the sink archive may.
# 2. Link size: the release CLI as shipped (no producer calls the encoder, so
#    the linker drops it) against the same CLI with the two encoder entry
#    points forced live, which is what wiring them into a producer links.
#
# Usage: scripts/basisu_link_check.sh TARGET OUTPUT_JSON
# Run from the repository root. Copies of both binaries go next to OUTPUT_JSON.
# Uses $CARGO_TARGET_DIR (default: target).
set -euo pipefail

target=$1
output=$2
work=$(dirname "$output")
target_dir=${CARGO_TARGET_DIR:-target}
suffix=
if [[ "$target" == *windows* ]]; then suffix=.exe; fi
binary="$target_dir/$target/release/rusty-tiles$suffix"

cargo build --locked --release --target "$target"

stdio_check=skipped
if [[ "$target" != *windows* ]]; then
  forbidden='^_?(printf|fprintf|vprintf|vfprintf|puts|fputs|putchar|putc|fputc|fwrite|perror|__printf_chk|__fprintf_chk|__vprintf_chk|__vfprintf_chk|__puts_chk|_IO_putc)$'
  archives=$(find "$target_dir/$target/release/build" -path '*/out/*' \
    \( -name 'librt_basisu_vendor.a' -o -name 'librt_basisu_zstd.a' \))
  if [[ -z "$archives" ]]; then
    echo "no vendored encoder archives found" >&2
    exit 1
  fi
  for archive in $archives; do
    hits=$(nm -u "$archive" 2>/dev/null | awk '{print $NF}' | grep -E "$forbidden" | sort -u || true)
    if [[ -n "$hits" ]]; then
      echo "$archive references stdio output directly:" >&2
      echo "$hits" >&2
      exit 1
    fi
  done
  stdio_check=passed
fi

size() { wc -c < "$1" | tr -d ' '; }
base_bytes=$(size "$binary")
cp "$binary" "$work/rusty-tiles-base$suffix"

case "$target" in
  *linux*)
    force=(-C link-arg=-Wl,--undefined=rt_basisu_encode
           -C link-arg=-Wl,--undefined=rt_basisu_decode_level_rgba) ;;
  *apple*)
    force=(-C link-arg=-Wl,-exported_symbol,_rt_basisu_encode
           -C link-arg=-Wl,-exported_symbol,_rt_basisu_decode_level_rgba) ;;
  *windows*)
    force=(-C link-arg=-INCLUDE:rt_basisu_encode
           -C link-arg=-INCLUDE:rt_basisu_decode_level_rgba) ;;
  *) echo "unsupported target $target" >&2; exit 1 ;;
esac
cargo rustc --locked --release --target "$target" --bin rusty-tiles -- "${force[@]}"
forced_bytes=$(size "$binary")
cp "$binary" "$work/rusty-tiles-forced$suffix"

base_stripped=null
forced_stripped=null
if [[ "$target" != *windows* ]]; then
  if ! nm "$work/rusty-tiles-forced$suffix" | grep 'rt_basisu_encode' > /dev/null; then
    echo "forced link does not contain rt_basisu_encode" >&2
    exit 1
  fi
  strip -o "$work/rusty-tiles-base.stripped" "$work/rusty-tiles-base$suffix" 2>/dev/null \
    || { cp "$work/rusty-tiles-base$suffix" "$work/rusty-tiles-base.stripped"; strip "$work/rusty-tiles-base.stripped"; }
  strip -o "$work/rusty-tiles-forced.stripped" "$work/rusty-tiles-forced$suffix" 2>/dev/null \
    || { cp "$work/rusty-tiles-forced$suffix" "$work/rusty-tiles-forced.stripped"; strip "$work/rusty-tiles-forced.stripped"; }
  base_stripped=$(size "$work/rusty-tiles-base.stripped")
  forced_stripped=$(size "$work/rusty-tiles-forced.stripped")
fi
# Restore the shipped binary for any later step.
cp "$work/rusty-tiles-base$suffix" "$binary"

cat > "$output" <<EOF
{
  "target": "$target",
  "stdioIsolation": "$stdio_check",
  "releaseCliBytes": $base_bytes,
  "releaseCliWithEncoderBytes": $forced_bytes,
  "encoderGrowthBytes": $((forced_bytes - base_bytes)),
  "strippedReleaseCliBytes": $base_stripped,
  "strippedReleaseCliWithEncoderBytes": $forced_stripped
}
EOF
cat "$output"
