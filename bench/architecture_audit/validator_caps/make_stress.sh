#!/usr/bin/env bash
# Regenerate every synthetic stress archive into DIR (default: current directory).
# Usage: bash make_stress.sh [DIR]
set -euo pipefail
here=$(cd "$(dirname "$0")" && pwd)
out=${1:-.}
mkdir -p "$out"
cd "$out"
plan=stress-plan.jsonl
: > "$plan"
stress() { local name=$1; shift; rm -f "$name.3tz"; printf '%s ' "$name" >> "$plan"; nice -n 10 python3 -I "$here/stress.py" "$name.3tz" "$@" >> "$plan"; }
single() { local name=$1; shift; rm -f "$name.3tz"; printf '%s ' "$name" >> "$plan"; nice -n 10 python3 -I "$here/stress_single.py" "$name.3tz" "$@" >> "$plan"; }

# Entries and hierarchy visits: old cap, 2^17, 2^18 (visit cap), one over each, and 2^19.
stress st-16 --entries 65536 --visits 65536
stress st-16-e1 --entries 65537 --visits 65536
stress st-16-v1 --entries 65536 --visits 65537
stress st-17 --entries 131072 --visits 131072
stress st-18 --entries 262144 --visits 262144
stress st-18-e1 --entries 262145 --visits 262144
stress st-18-v1 --entries 262144 --visits 262145
stress st-19 --entries 524288 --visits 524288
# Entries at 2^19 with visits at 2^18: up to three contents per tile.
stress se-19 --entries 524288 --visits 262144 --max-contents 3
stress se-19-e1 --entries 524289 --visits 262144 --max-contents 3
# Central directory: entries at their cap with names padded to the byte cap.
stress cd-16m --entries 65536 --visits 65536 --cd-bytes 16777216
stress cd-16m-p1 --entries 65536 --visits 65536 --cd-bytes 16777217
stress cd-32m --entries 262144 --visits 262144 --cd-bytes 33554432
stress cd-32m-p1 --entries 262144 --visits 262144 --cd-bytes 33554433
# JSON value nodes in one document.
single items-16-array --items 65536
single items-16-array-p1 --items 65537
single items-16-object --items 65536 --shape object
single items-17-array --items 131072
single items-17-array-p1 --items 131073
single items-17-object --items 131072 --shape object
# One member at the per-member byte cap (sizes stay 4-byte aligned).
single member-64m --member-bytes 67108864
single member-64m-p4 --member-bytes 67108868
single member-128m --member-bytes 134217728
single member-128m-p4 --member-bytes 134217732
# Decoded bufferView bytes in one document (meshopt zero stream).
single decoded-64m --decoded-bytes 67108864
single decoded-64m-p4 --decoded-bytes 67108868
single decoded-128m --decoded-bytes 134217728
single decoded-128m-p4 --decoded-bytes 134217732
