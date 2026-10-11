#!/bin/bash
# Fixture suite (E/A/S/X/F/D/N) after the perf lane, CPUs 13,15, one browser at a time.
E=$(cd "$(dirname "$0")" && pwd)
until [ -f $E/perf/done ]; do sleep 10; done
cd $E/cesium && rm -f fixtures-results.json fixtures.cpu0 fixtures.cpuload shot-*.png
python3 -I $E/cpustat.py snap > fixtures.cpu0
timeout 2400 taskset -c 13,15 nice -n 10 python3 -I run.py fx $E/cesium/fixtures-results.json > fixtures.log 2>&1
python3 -I $E/cpustat.py diff fixtures.cpu0 > fixtures.cpuload
echo done > $E/cesium/fixtures.done
