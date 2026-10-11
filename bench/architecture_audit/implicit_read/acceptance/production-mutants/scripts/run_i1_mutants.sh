#!/bin/bash
# Run the I1 production mutants: clean baseline, then each patch. Build under the shared cargo lock,
# run the release lib-test binary outside it on CPUs 0,1. Per mutant: the parity harness (both tests)
# and the I1 unit tests (content_integrity::implicit, validate::tests::i1_*). Timing mutants run only
# their paired large control under the contract's timeout.
R=/home/bend/.cache/forest-acc/mutant-src; A=/home/bend/.cache/forest-acc; P=${1:?patch dir}; O=${2:?out dir}
mkdir -p $O; cd $R
export CARGO_TARGET_DIR=$A/target-mutants CARGO_BUILD_JOBS=2 TMPDIR=$A/tmp RAYON_NUM_THREADS=2
build() {
  flock /home/bend/.cache/rusty-tiles-cargo.lock taskset -c 0,1 nice -n 10 \
    cargo test --locked --release --lib --no-run --message-format=json 2> $O/$1.build.err \
    | python3 -c "import json,sys
for l in sys.stdin:
    m=json.loads(l)
    if m.get('reason')=='compiler-artifact' and m.get('executable') and m['target']['name']=='rusty_tiles': print(m['executable'])" | tail -1
}
run() { # name timing timeout
  local name=$1 timing=$2 limit=$3 exe
  exe=$(build $name)
  if [ -z "$exe" ]; then echo "{\"name\":\"$name\",\"build\":\"failed\"}" > $O/$name.result.json; echo "$name BUILD FAILED"; return; fi
  local start=$(date +%s.%N)
  if [ -n "$timing" ]; then
    I1_ONLY=$timing I1_PARITY_OUT=$O/$name.parity.json RUST_TEST_THREADS=1 timeout $limit taskset -c 0,1 nice -n 10 \
      $exe i1_oracle_parity::production_matches_every --test-threads=1 > $O/$name.parity.log 2>&1
  else
    I1_PARITY_OUT=$O/$name.parity.json I1_CORPUS_PARITY_OUT=$O/$name.corpus.json RUST_TEST_THREADS=2 timeout 1800 taskset -c 0,1 nice -n 10 \
      $exe i1_oracle_parity --test-threads=2 > $O/$name.parity.log 2>&1
  fi
  local rc=$? end=$(date +%s.%N)
  RUST_TEST_THREADS=2 timeout 1800 taskset -c 0,1 nice -n 10 $exe content_integrity::implicit validate::tests::i1_ --test-threads=2 > $O/$name.unit.log 2>&1
  local urc=$?
  python3 - "$O" "$name" "$rc" "$urc" "$start" "$end" "$timing" "$limit" <<'EOF'
import json, os, re, sys
O, name, rc, urc, start, end, timing, limit = sys.argv[1:]
def load(p):
    p = os.path.join(O, p)
    return json.load(open(p)) if os.path.exists(p) else None
par, cor = load(f"{name}.parity.json"), load(f"{name}.corpus.json")
unit = open(os.path.join(O, f"{name}.unit.log")).read()
failed_units = sorted(set(re.findall(r"^    (\S+)$", unit.split("failures:")[-1], re.M))) if "FAILED" in unit else []
res = {"name": name, "parityExit": int(rc), "timedOut": int(rc) == 124, "wallSeconds": round(float(end) - float(start), 1),
       "timingCase": timing or None, "timeoutSeconds": int(limit) if timing else None,
       "controlsFlipped": [r["id"] for r in par["disagree"]] if par else None,
       "corpusFlipped": [r["id"] for r in cor["disagree"]] if cor else None,
       "unitExit": int(urc), "unitFailed": failed_units}
if par and timing:
    res["timedCase"] = [{"id": r["id"], "kind": r["production"]["kind"], "wall": r["production"].get("wallSeconds")} for r in par["rows"]]
json.dump(res, open(os.path.join(O, f"{name}.result.json"), "w"), indent=1)
print(json.dumps({k: res[k] for k in ("name", "parityExit", "timedOut", "wallSeconds", "unitExit")}),
      "controls", (res["controlsFlipped"] or [])[:12], "corpus", (res["corpusFlipped"] or [])[:6], "unit", res["unitFailed"][:6])
EOF
}
git checkout -q -- src
run clean "" ""
for timing in "R7:60" "R7c:60" "R8a:300"; do I1_ONLY=${timing%%:*} run clean-${timing%%:*} ${timing%%:*} ${timing##*:}; done
python3 -c "
import json
for r in json.load(open('$P/mutants.json')):
    t = r['timingCase'] or ''
    lim = {'R7':60,'R7c':60,'R8a':300}.get(t,'')
    print(r['name'], t or '-', lim or '-')" > $O/queue.txt
while read name timing limit; do
  [ "$timing" = "-" ] && timing="" && limit=""
  git apply $P/$name.patch || { echo "$name APPLY FAILED"; continue; }
  run $name "$timing" "$limit"
  git checkout -q -- src
done < $O/queue.txt
echo "done $(date +%T)"
