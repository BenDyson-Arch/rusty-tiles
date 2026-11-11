#!/bin/bash
R=/tmp/rusty-tiles-forest; A=/home/bend/.cache/forest-acc; C=$A/ci/final; mkdir -p $C/tmp; cd $R
export CARGO_BUILD_JOBS=2 RAYON_NUM_THREADS=2 TMPDIR=$A/tmp
echo "head $(git rev-parse HEAD) dirty=$(git status --short | wc -l)" | tee $C/summary.txt
step() { name=$1; shift; "$@" > $C/$name.log 2>&1; echo "$name $?" | tee -a $C/summary.txt; }
cg() { flock /home/bend/.cache/rusty-tiles-cargo.lock taskset -c 0,1 nice -n 10 "$@"; }
step fmt cargo fmt --all --check
step clippy-native cg env CARGO_TARGET_DIR=$A/ci/target-native LIBSQLITE3_SYS_USE_PKG_CONFIG=1 cargo clippy --locked --all-targets --features native-geospatial -- -D warnings
step clippy-python cg env CARGO_TARGET_DIR=$A/ci/target-portable RUSTY_TILES_DISABLE_NATIVE_JPEG=1 cargo clippy --locked -p rusty-tiles-python -- -D warnings
step test-native-geospatial cg env CARGO_TARGET_DIR=$A/ci/target-native LIBSQLITE3_SYS_USE_PKG_CONFIG=1 RUST_TEST_THREADS=2 cargo test --locked --features native-geospatial
step vector-acceptance-native taskset -c 0,1 nice -n 10 python3 -B tests/vector_acceptance_oracle.py --binary $A/ci/target-native/debug/rusty-tiles --evidence $C/tmp/vector-acceptance-native --native
step i1-parity-release cg env CARGO_TARGET_DIR=$A/target I1_PARITY_OUT=$C/parity-final.json I1_CORPUS_PARITY_OUT=$C/corpus-parity-final.json RUST_TEST_THREADS=1 cargo test --locked --release --lib i1_oracle_parity
echo "done $(date +%T)" | tee -a $C/summary.txt
