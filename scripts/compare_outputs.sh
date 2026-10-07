#!/usr/bin/env bash
# Compare converter outputs of two commits byte for byte.
#
# usage: scripts/compare_outputs.sh BASE_REF [HEAD_REF]
#
# Builds each ref (release, --features "$FEATURES") into its own target
# directory, runs the fixed recipes of tests/support/mod.rs (one or more per
# converter subcommand, generated inputs, PATH="", --reproducible where the
# subcommand has it) with both binaries, and compares the sha256 of every
# .3tz entry and every file of directory outputs. Only the vector encoder
# fingerprints (encoder, signature, vectorBuildStateSha256 in tileset.json and
# vector-build.json) are ignored. Without HEAD_REF the working tree is used.
#
# Environment:
#   FEATURES             cargo features (default native-geospatial; set empty
#                        for a default build, which skips geospatial recipes)
#   COMPARE_WORK         scratch directory (default: a new temporary directory)
#   COMPARE_TARGET_ROOT  where per-commit target directories are kept so
#                        repeated runs reuse builds (default target/compare)
#
# Exits 0 when every recipe matches, 1 on the first difference or a recipe
# that fails on only one side, 2 on usage or build errors.
set -euo pipefail

if [[ $# -lt 1 || $# -gt 2 || $1 == -h || $1 == --help ]]; then
  sed -n '2,/^set -euo/p' "$0" | sed '$d; s/^# \{0,1\}//'
  exit 2
fi
BASE_REF=$1
HEAD_REF=${2:-}
FEATURES=${FEATURES-native-geospatial}
REPO=$(git -C "$(dirname "$0")/.." rev-parse --show-toplevel)
WORK=${COMPARE_WORK:-$(mktemp -d "${TMPDIR:-/tmp}/rusty-tiles-compare.XXXXXX")}
TARGET_ROOT=${COMPARE_TARGET_ROOT:-$REPO/target/compare}
mkdir -p "$WORK" "$TARGET_ROOT"
FEATURE_ARGS=()
[[ -n $FEATURES ]] && FEATURE_ARGS=(--features "$FEATURES")
WORKTREES=()
cleanup() {
  for tree in "${WORKTREES[@]}"; do
    git -C "$REPO" worktree remove --force "$tree" >/dev/null 2>&1 || true
  done
}
trap cleanup EXIT

# build LABEL REF -> path of the release binary in $WORK/bin-LABEL
build() {
  local label=$1 ref=$2 src sha target
  if [[ -z $ref ]]; then
    src=$REPO
    target=$TARGET_ROOT/worktree
  else
    sha=$(git -C "$REPO" rev-parse --verify "$ref^{commit}") || exit 2
    src=$WORK/src-$label
    git -C "$REPO" worktree add --force --detach "$src" "$sha" >/dev/null
    WORKTREES+=("$src")
    target=$TARGET_ROOT/$sha
  fi
  echo "building $label (${ref:-working tree}) in $target" >&2
  CARGO_TARGET_DIR=$target cargo build --manifest-path "$src/Cargo.toml" \
    --release --locked --quiet "${FEATURE_ARGS[@]}" >&2 || exit 2
  cp "$target/release/rusty-tiles" "$WORK/bin-$label"
}

build base "$BASE_REF"
build head "$HEAD_REF"

# Inputs and recipes come from this checkout's tests/support, so both sides
# run exactly the same commands on exactly the same bytes.
echo "generating inputs" >&2
rm -rf "$WORK/export"
RUSTY_TILES_DIGEST_EXPORT=$WORK/export CARGO_TARGET_DIR=$TARGET_ROOT/worktree \
  cargo test --manifest-path "$REPO/Cargo.toml" --release --locked --quiet \
  "${FEATURE_ARGS[@]}" --test output_digests export_recipes -- --ignored --exact \
  >"$WORK/export.log" 2>&1 || { cat "$WORK/export.log" >&2; exit 2; }
INPUTS=$WORK/export/inputs

DIGEST_PY='
import hashlib, json, os, sys, zipfile
path, vector = sys.argv[1], sys.argv[2] == "1"
def strip(v):
    for obj, key in [(v.get("asset", {}).get("extras", {}), "vectorBuildStateSha256"),
                     (v.get("config", {}), "encoder"),
                     *((r, "signature") for r in v.get("records", {}).values())]:
        if key in obj:
            obj[key] = "<fingerprint>"
    return v
def emit(name, data):
    if vector and name.rsplit("/", 1)[-1] in ("tileset.json", "vector-build.json"):
        data = json.dumps(strip(json.loads(data)), sort_keys=True).encode()
    print(hashlib.sha256(data).hexdigest(), name)
if os.path.isdir(path):
    for root, dirs, files in os.walk(path):
        dirs.sort()
        for f in sorted(files):
            full = os.path.join(root, f)
            emit(os.path.relpath(full, path).replace(os.sep, "/"), open(full, "rb").read())
elif path.endswith(".3tz"):
    with zipfile.ZipFile(path) as z:
        for i, info in enumerate(z.infolist()):
            print("entry", i, info.filename)
            emit(info.filename, z.read(info))
else:
    emit(os.path.basename(path), open(path, "rb").read())
'

status=0
while IFS=$'\t' read -r -a fields; do
  name=${fields[0]} command=${fields[1]} output=${fields[2]} vector=${fields[3]}
  args=("${fields[@]:4}")
  args=("${args[@]//\{in\}/$INPUTS}")
  for side in base head; do
    out=$WORK/out-$side
    mkdir -p "$out"
    rm -rf "${out:?}/$output"
    if env PATH="" "$WORK/bin-$side" "$command" "${args[@]}" -o "$out/$output" \
      >"$out/$name.log" 2>&1; then
      python3 -c "$DIGEST_PY" "$out/$output" "$vector" >"$out/$name.sha256"
    else
      echo "FAILED (exit $?)" >"$out/$name.sha256"
    fi
  done
  if cmp -s "$WORK/out-base/$name.sha256" "$WORK/out-head/$name.sha256"; then
    if grep -q '^FAILED' "$WORK/out-base/$name.sha256"; then
      echo "$name: fails on both sides (see $WORK/out-base/$name.log)"
      status=1
    else
      echo "$name: identical ($(grep -vc '^entry ' "$WORK/out-base/$name.sha256") files)"
    fi
  else
    echo "$name: DIFFERS (base vs head, first difference):"
    diff "$WORK/out-base/$name.sha256" "$WORK/out-head/$name.sha256" | sed -n '1,3p'
    echo "  logs and outputs: $WORK/out-base $WORK/out-head"
    exit 1
  fi
done <"$WORK/export/recipes.tsv"
[[ $status -eq 0 ]] && echo "all recipes byte-identical (vector fingerprints ignored); work: $WORK"
exit $status
