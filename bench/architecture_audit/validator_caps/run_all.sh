#!/usr/bin/env bash
# Measure every archive with the baseline, proposed and wide probe binaries.
# Usage: bash run_all.sh WORK RESULTS_DIR
# WORK holds stress/ (make_stress.sh) and the vector archives (make_vector.sh);
# binaries are WORK/../bin/rusty-tiles-{68b7531,proposed,probe}-native.
# Holds the shared Cargo lock so no build competes for CPUs 0-1 during timing.
set -euo pipefail
here=$(cd "$(dirname "$0")" && pwd)
work=$(cd "$1" && pwd)
results=$(mkdir -p "$2" && cd "$2" && pwd)
bin=$work/../bin
measure() { python3 -I "$here/measure.py" --work "$work/measure" "$@"; }
exec 9>/home/bend/.cache/rusty-tiles-cargo.lock
flock 9
cd "$work/stress"
measure --binary "$bin/rusty-tiles-68b7531-native" --label baseline --runs 3 --out "$results/stress.json" \
  st-16.3tz st-16-e1.3tz st-16-v1.3tz cd-16m.3tz cd-16m-p1.3tz items-16-array.3tz items-16-array-p1.3tz \
  items-16-object.3tz member-64m.3tz member-64m-p4.3tz decoded-64m.3tz decoded-64m-p4.3tz \
  st-18.3tz cd-32m.3tz items-17-array.3tz member-128m.3tz decoded-128m.3tz
measure --binary "$bin/rusty-tiles-proposed-native" --label proposed --runs 3 --out "$results/stress.json" \
  st-16.3tz st-17.3tz st-18.3tz st-18-e1.3tz st-18-v1.3tz st-19.3tz se-19.3tz se-19-e1.3tz cd-32m.3tz cd-32m-p1.3tz \
  items-16-object.3tz items-17-array.3tz items-17-array-p1.3tz items-17-object.3tz \
  member-64m.3tz member-128m.3tz member-128m-p4.3tz decoded-64m.3tz decoded-128m.3tz decoded-128m-p4.3tz
measure --binary "$bin/rusty-tiles-probe-native" --label wide-probe --runs 3 --out "$results/stress.json" \
  st-16.3tz st-17.3tz st-18.3tz st-19.3tz se-19.3tz
cd "$work"
measure --binary "$bin/rusty-tiles-68b7531-native" --label baseline --runs 1 --out "$results/vector.json" \
  s1-mf2.3tz points-81k-mf2.3tz points-78k-mf2-explicit.3tz mixed-s7.8-mf2.3tz
measure --binary "$bin/rusty-tiles-proposed-native" --label proposed --runs 3 --out "$results/vector.json" \
  s1-mf2.3tz points-78k-mf2.3tz points-81k-mf2.3tz points-78k-mf2-explicit.3tz mixed-s7.8-mf2.3tz \
  points-90k-mf2-t130k.3tz points-165k-mf2-t300k.3tz
measure --binary "$bin/rusty-tiles-probe-native" --label wide-probe --runs 1 --out "$results/vector.json" \
  s1-mf2.3tz points-78k-mf2.3tz points-81k-mf2.3tz points-78k-mf2-explicit.3tz mixed-s7.8-mf2.3tz \
  points-90k-mf2-t130k.3tz points-165k-mf2-t300k.3tz
