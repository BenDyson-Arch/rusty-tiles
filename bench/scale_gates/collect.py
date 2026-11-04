#!/usr/bin/env python3
"""Collect run_case.py reports into one markdown table.

  python3 -I bench/scale_gates/collect.py ~/.cache/rusty-tiles-scale/runs/<prefix>-*/report.json
"""
import json, pathlib, sys

MiB = 1 << 20


def mib(v):
    return '-' if v is None else f'{v / MiB:,.1f}'


def secs(v):
    return '-' if v is None else f'{v:,.2f}'


def main(paths):
    rows = ['| case | status | input | wall s | peak RSS MiB | cgroup peak MiB | scratch peak MiB | output MiB '
            '| tiles | validate s | validate RSS MiB | Cesium load / SSE0 s | note |',
            '| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |']
    for path in paths:
        r = json.loads(pathlib.Path(path).read_text())
        m = r['metrics']
        truth = r.get('input_truth_summary') or {}
        size = truth.get('triangles') or truth.get('tiles')
        unit = 'tris' if truth.get('triangles') else 'tiles'
        inp = f"{size:,} {unit}" if size else pathlib.Path(r['input']).name
        ces = '-' if m.get('cesium_load_seconds') is None else \
            f"{m['cesium_load_seconds']:.2f} / {m.get('cesium_refined_seconds') or 0:.2f}"
        note = m.get('producer_error') or m.get('validate_error') or ''
        report = (r.get('producer') or {}).get('report')
        msg = report.get('error') if isinstance(report, dict) else None
        if isinstance(msg, dict) and msg.get('message'):
            note = f"{note}: {msg['message']}"
        elif m.get('validate_message'):
            note = f"{note}: {m['validate_message']}"
        rows.append(f"| {r['case']} | {r['status']} | {inp} | {secs(m.get('wall_seconds'))} | {mib(m.get('peak_rss_bytes'))} "
                    f"| {mib(m.get('cgroup_memory_peak_bytes'))} | {mib(m.get('scratch_peak_bytes'))} "
                    f"| {mib(m.get('output_bytes'))} | {m.get('tiles') if m.get('tiles') is not None else '-'} "
                    f"| {secs(m.get('validate_wall_seconds'))} | {mib(m.get('validate_peak_rss_bytes'))} | {ces} | {note} |")
    print('\n'.join(rows))


if __name__ == '__main__':
    main(sys.argv[1:])
