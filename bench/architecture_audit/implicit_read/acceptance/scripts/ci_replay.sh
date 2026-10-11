#!/bin/bash
# Local replay of .github/workflows/ci.yml jobs rust, portable (host), native-geospatial and the
# python-wheel cargo steps, at the checked-out head. Each step logs to $C/<step>.log and appends
# "step exit" to $C/summary.txt. Cargo runs under the shared lock on CPUs 0,1 at nice 10.
R=/tmp/rusty-tiles-forest; A=/home/bend/.cache/forest-acc; C=$A/ci; T=$C/tmp; mkdir -p $T; : > $C/summary.txt
cd $R
export CARGO_BUILD_JOBS=2 RAYON_NUM_THREADS=2 TMPDIR=$A/tmp
step() { name=$1; shift; "$@" > $C/$name.log 2>&1; rc=$?; echo "$name $rc" | tee -a $C/summary.txt; }
cg() { flock /home/bend/.cache/rusty-tiles-cargo.lock taskset -c 0,1 nice -n 10 "$@"; }
py() { taskset -c 0,1 nice -n 10 python3 -B "$@"; }
echo "head $(git rev-parse HEAD) dirty=$(git status --short | wc -l)" | tee -a $C/summary.txt
# --- job rust (native-jpeg debug build provides target/debug/rusty-tiles)
export CARGO_TARGET_DIR=$C/target-rust; BIN=$CARGO_TARGET_DIR/debug/rusty-tiles
step foundation-model python3 bench/architecture_audit/foundation/model.py --results $T/foundation-model.json
step foundation-monitor python3 -m unittest discover -s bench/architecture_audit/foundation -p test_monitor.py -v
step fmt cargo fmt --all --check
step test-native-jpeg bash -c "flock /home/bend/.cache/rusty-tiles-cargo.lock taskset -c 0,1 nice -n 10 cargo test --locked --features native-jpeg -- --nocapture --test-threads=1 2>&1 | tee $T/c1-library.log; exit \${PIPESTATUS[0]}"
step payload-context-receipt python3 -B tests/payload_context_receipt.py --log $T/c1-library.log --output $T/c1-payload-contexts.json
step c1-validation py tests/c1_validation_oracle.py --generate $T/c1-fixtures --binary $BIN --output $T/c1-validation.json
step c1-resources py bench/architecture_audit/c1/resource_probe.py --binary $BIN --work $T/c1-resources --output $T/c1-resources.json
step c1-producer py tests/c1_producer_alignment.py --binary $BIN --work $T/c1-producer --output $T/c1-producer.json
step c1-archive-read py tests/archive_read_oracle.py --binary $BIN --work $T/archive-read-corpus --output $T/c1-archive-read.json
step c1-payload-integrity py tests/payload_integrity_oracle.py --binary $BIN --work $T/payload-integrity-corpus --output $T/c1-payload-integrity.json
step c1-payload-stride py tests/payload_meshopt_stride_oracle.py --binary $BIN --work $T/payload-stride-corpus --output $T/c1-payload-stride.json
step f1a py tests/f1a_oracle.py --binary $BIN
step f1b py tests/f1b_oracle.py --binary $BIN --json-output $T/f1b-textures.json
step f1b2 py tests/f1b2_oracle.py --binary $BIN --json-output $T/f1b-local-resources.json
step f1b3 py tests/f1b3_oracle.py --binary $BIN --json-output $T/f1b-core-pbr.json
step f1b1-resources py bench/architecture_audit/f1b1/resource_probe.py --binary $BIN --output $T/f1b-resources.json
step f1b2-resources py bench/architecture_audit/f1b2/resource_probe.py --binary $BIN --output $T/f1b-local-resource-bounds.json
step f1b3-resources py bench/architecture_audit/f1b3/resource_probe.py --binary $BIN --output $T/f1b-core-pbr-bounds.json
step f1c1 py tests/f1c1_oracle.py --binary $BIN --json-output $T/f1c1-placement.json
step f1c1-resources py bench/architecture_audit/f1c1/resource_probe.py --binary $BIN --binary-sha256 "$(sha256sum $BIN | cut -d ' ' -f 1)" --output $T/f1c1-resources.json
step f1c2 py tests/f1c2_oracle.py --self-test --binary $BIN --large --json-output $T/f1c2-identity.json
step f1d2 py tests/f1d2_oracle.py --self-test --binary $BIN --artifact-dir $T/f1d2-artifacts --json-output $T/f1d2-oracle.json
step model-wrapping py tests/model_wrapping_oracle.py --binary $BIN --json-output $T/f1c2-wrapping.json
step vector-acceptance py tests/vector_acceptance_oracle.py --binary $BIN --evidence $T/vector-acceptance
step point-acceptance py tests/p1_point_oracle.py --binary $BIN --output-dir $T/point-acceptance --json-output $T/point-acceptance.json
step cargo-package cg cargo package --locked --no-verify
step portable-jpeg cg env RUSTY_TILES_DISABLE_NATIVE_JPEG=1 cargo test --locked --lib jpeg::tests
# --- job python-wheel (cargo and unittest steps)
export CARGO_TARGET_DIR=$C/target-portable
step wheel-runners python3 -m unittest discover -s tests -p test_wheel_runners.py -v
step clippy-python cg env RUSTY_TILES_DISABLE_NATIVE_JPEG=1 cargo clippy --locked -p rusty-tiles-python -- -D warnings
step test-python cg env RUSTY_TILES_DISABLE_NATIVE_JPEG=1 cargo test --locked -p rusty-tiles-python
# --- job portable (host target)
step test-portable cg env RUSTY_TILES_DISABLE_NATIVE_JPEG=1 cargo test --locked --target x86_64-unknown-linux-gnu
# --- job native-geospatial
export CARGO_TARGET_DIR=$C/target-native; NBIN=$CARGO_TARGET_DIR/debug/rusty-tiles
step test-native-geospatial cg env LIBSQLITE3_SYS_USE_PKG_CONFIG=1 RUST_TEST_THREADS=2 cargo test --locked --features native-geospatial
step vector-acceptance-native py tests/vector_acceptance_oracle.py --binary $NBIN --evidence $T/vector-acceptance-native --native
step point-acceptance-native py tests/p1_point_oracle.py --binary $NBIN --output-dir $T/point-acceptance-native --json-output $T/point-acceptance-native.json
step clippy-native cg env LIBSQLITE3_SYS_USE_PKG_CONFIG=1 cargo clippy --locked --all-targets --features native-geospatial -- -D warnings
echo "done $(date +%T)" | tee -a $C/summary.txt
