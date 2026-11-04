#!/bin/bash
# Reproduces the harness validation pass recorded in README.md (10^5-10^7 triangles) with the
# binaries recorded there. Its mesh-local cases run `mesh-local-to-3tz`, which was removed in 0.4.0,
# so they need a recorded pre-removal binary; the production tiler's gates are run with `mesh`
# (README, "Running the gates").
# Usage: validation_pass.sh RELEASE_BIN LIB_TEST_BIN [RUN_PREFIX] [ONLY_CASE_SUBSTRING]
# CESIUM_ARG=--launch uses a private headless GPU Chromium instead of the CDP browser on :9222.
# Missing inputs are generated first under ~/.cache/rusty-tiles-scale (RUSTY_TILES_SCALE_ROOT).
set -u
B=$1; T=$2; P=${3:-validation}; ONLY=${4:-}
HERE=$(cd "$(dirname "$0")" && pwd)
S=${RUSTY_TILES_SCALE_ROOT:-$HOME/.cache/rusty-tiles-scale}
gen() { [ -f "$S/$2/manifest.json" ] || nice -n 10 python3 -I "$HERE/$1" "${@:3}" --out "$2"; }
gen generate.py inputs/mesh-1e5 mesh --triangles 100000
gen generate.py inputs/mesh-1e6 mesh --triangles 1000000
gen generate.py inputs/mesh-1e7 mesh --triangles 10000000
gen generate.py inputs/texture-1e5-1024 texture --triangles 100000 --texture-size 1024 --format gltf --compress-png
gen generate.py inputs/density-1e4 density
gen gen_archive.py archives/implicit-2p22 implicit
gen gen_archive.py archives/large-4p5g large
run() { case "$2" in *"$ONLY"*) python3 -I "$HERE/run_case.py" "$@";; esac; }
R=run
CES=(--cesium ${CESIUM_ARG:+--cesium-arg=$CESIUM_ARG})
ML='{binary} --json mesh-local-to-3tz -i {input} -o {output} --leaf-triangles 20000'
H1='{binary} --exact mesh_large::tests::external_driver --ignored --nocapture'
H1ENV=(--env 'H1_INPUT={input}' --env 'H1_OUTPUT={output}' --env 'H1_SCRATCH={scratch}' --env 'H1_JOBS={jobs}'
       --env H1_RUN_BYTES=67108864 --env H1_BUFFER_BYTES=131072 --env H1_PROFILE=balanced)
$R --case $P-mesh-local-1e5 --budget mesh-local --binary "$B" --input $S/inputs/mesh-1e5 --command "$ML" "${CES[@]}"
$R --case $P-mesh-local-1e6 --budget mesh-local-ceiling --binary "$B" --input $S/inputs/mesh-1e6 --command "$ML"
$R --case $P-mesh-local-1e7 --budget mesh-local-ceiling --binary "$B" --input $S/inputs/mesh-1e7 --command "$ML"
$R --case $P-h1-capture-1e6 --budget h1-capture --binary "$T" --input $S/inputs/mesh-1e6 --command "$H1" \
   --output-name h1 --producer-report '{output}/report.json' "${H1ENV[@]}"
$R --case $P-h1-capture-1e7 --budget h1-capture --binary "$T" --input $S/inputs/mesh-1e7 --command "$H1" \
   --output-name h1 --producer-report '{output}/report.json' "${H1ENV[@]}"
$R --case $P-h1-capture-density-1e4 --budget h1-capture --binary "$T" --input $S/inputs/density-1e4 --command "$H1" \
   --output-name h1 --producer-report '{output}/report.json' "${H1ENV[@]}"
$R --case $P-texture-1e5-mesh-local --budget mesh-local --binary "$B" --input $S/inputs/texture-1e5-1024 --command "$ML" "${CES[@]}"
$R --case $P-density-1e4-mesh-local --budget gate-b --binary "$B" --input $S/inputs/density-1e4 --command "$ML"
$R --case $P-gate-d-implicit --budget gate-d-implicit --binary "$B" --input $S/archives/implicit-2p22
$R --case $P-gate-d-large --budget gate-d-large --binary "$B" --input $S/archives/large-4p5g
