#!/usr/bin/env bash
# Package a built default CLI as its release archive: rusty-tiles-VERSION-TARGET.tgz (.zip on
# Windows) with LICENSE, the Rust crate licences and the vendored notices. ci.yml's portable
# jobs and release-build.yml's fallback build both use it, so a release can publish the CI
# archive unchanged. Run from the repository root; OUTDIR is relative to it.
#
#   scripts/package_cli.sh TARGET VERSION BINARY OUTDIR
#
# PYTHON names the interpreter (default python3; Windows runners use python).
set -euo pipefail
if [ "$#" -ne 4 ]; then
  echo "usage: $0 TARGET VERSION BINARY OUTDIR" >&2
  exit 2
fi
target=$1 version=$2 binary=$3 out=$4
python=${PYTHON:-python3}
suffix=
if [[ "$target" == *windows* ]]; then suffix=.exe; fi
stage="package-stage-$target"
rm -rf "$stage"
mkdir -p "$stage" "$out"
cp "$binary" "$stage/rusty-tiles$suffix"
cp LICENSE "$stage/"
cp licenses/THIRD_PARTY_LICENSES-cli.md "$stage/THIRD_PARTY_LICENSES.md"
"$python" scripts/third_party_notices.py stage "$stage"
asset="rusty-tiles-$version-$target"
if [[ "$target" == *windows* ]]; then
  (cd "$stage" && 7z a -tzip "../$out/$asset.zip" "rusty-tiles$suffix" LICENSE THIRD_PARTY_LICENSES.md third_party)
else
  tar -czf "$out/$asset.tgz" -C "$stage" "rusty-tiles$suffix" LICENSE THIRD_PARTY_LICENSES.md third_party
fi
rm -rf "$stage"
"$python" scripts/third_party_notices.py check --rust-licenses cli "$out/$asset".*
echo "Packaged $out/$asset"
