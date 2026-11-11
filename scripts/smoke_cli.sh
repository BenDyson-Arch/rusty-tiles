#!/usr/bin/env bash
# Smoke-test a packaged CLI archive on the platform it is for, as a user would get it: the
# archive name and members, the third-party notices, and the extracted binary's version,
# readiness and two mesh conversions (one through the in-process KTX2/Basis encoder) that it
# then validates. release-build.yml runs it on every archive it publishes, whether built
# there or taken from the CI run. Run from the repository root.
#
#   scripts/smoke_cli.sh TARGET VERSION ARCHIVE
#
# PYTHON names the interpreter (default python3; Windows runners use python).
set -euo pipefail
if [ "$#" -ne 3 ]; then
  echo "usage: $0 TARGET VERSION ARCHIVE" >&2
  exit 2
fi
target=$1 version=$2 archive=$3
python=${PYTHON:-python3}
suffix='' extension=tgz
if [[ "$target" == *windows* ]]; then suffix=.exe extension=zip; fi
expected="rusty-tiles-$version-$target.$extension"
if [ "$(basename "$archive")" != "$expected" ]; then
  echo "error: expected an archive named $expected, got $(basename "$archive")" >&2
  exit 1
fi
"$python" scripts/third_party_notices.py check --rust-licenses cli "$archive"
work="smoke-$target"
rm -rf "$work"
mkdir -p "$work/bin"
if [ "$extension" = zip ]; then
  7z x -y -o"$work/bin" "$archive" > /dev/null
else
  tar -xzf "$archive" -C "$work/bin"
fi
for member in "rusty-tiles$suffix" LICENSE THIRD_PARTY_LICENSES.md third_party; do
  test -e "$work/bin/$member" || { echo "error: $expected lacks $member" >&2; exit 1; }
done
binary="$work/bin/rusty-tiles$suffix"
reported=$("$binary" --version | tr -d '\r')
echo "$reported"
if [ "$reported" != "rusty-tiles $version" ]; then
  echo "error: expected 'rusty-tiles $version'" >&2
  exit 1
fi
"$binary" doctor --command mesh
"$binary" mesh -i tests/fixtures/example.glb -o "$work/mesh.3tz" --anchor 153.02 -27.47 0
"$binary" validate "$work/mesh.3tz"
"$binary" mesh -i tests/fixtures/f1b/basecolor-jpeg.glb -o "$work/mesh-ktx2.3tz" --anchor 153.02 -27.47 0
"$binary" validate "$work/mesh-ktx2.3tz"
rm -rf "$work"
echo "Smoke test passed: $expected"
