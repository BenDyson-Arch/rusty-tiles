#!/usr/bin/env bash
# Regenerate the real vector archives with a given converter binary.
# Usage: bash make_vector.sh BINARY DIR [NAME...]
# Defaults (implicit tiling) except --maxFeatures 2 for deep trees and --jobs 2;
# the 90k/165k-point archives raise --maxTiles to approach the entry cap and beyond.
set -euo pipefail
here=$(cd "$(dirname "$0")" && pwd)
binary=$1
out=$2
shift 2
mkdir -p "$out"
cd "$out"
declare -A spec=(
  [mixed-s7.8-mf2]="scale 7.8"
  [points-78k-mf2]="points 78000"
  [points-81k-mf2]="points 81000"
  [points-78k-mf2-explicit]="points 78000 --explicit"
  [points-165k-mf2-t300k]="points 165000 --maxTiles 300000"
  [points-90k-mf2-t130k]="points 90000 --maxTiles 130000"
)
names=("$@")
[ ${#names[@]} -gt 0 ] || names=(mixed-s7.8-mf2 points-78k-mf2 points-81k-mf2 points-78k-mf2-explicit points-165k-mf2-t300k points-90k-mf2-t130k)
for name in "${names[@]}"; do
  read -r kind size extra <<< "${spec[$name]}"
  source="$name.geojson"
  if [ "$kind" = scale ]; then
    python3 -I "$here/gen_vector.py" "$source" "$size" > /dev/null
  else
    python3 -I "$here/gen_vector.py" "$source" 0 --points "$size" > /dev/null
  fi
  rm -f "$name.3tz"
  start=$(date +%s)
  # shellcheck disable=SC2086
  taskset -c 0,1 nice -n 10 "$binary" vector -i "$source" -o "$name.3tz" --jobs 2 --maxFeatures 2 $extra 2>&1 | head -1
  echo "$name seconds=$(( $(date +%s) - start ))"
  rm -f "$source"
done
