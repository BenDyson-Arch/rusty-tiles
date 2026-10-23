#!/usr/bin/env python3
"""Verify entire output ancestry against independent availability expansion and GE policy."""
import json,sys
from pathlib import Path
from baseline_tree import tree
candidate,baseline,output=map(Path,sys.argv[1:]);nodes=json.loads((candidate/'build.json').read_text())['nodes'];tt=tree(baseline)
assert len(nodes)==len(tt);assert len({n['baseline_file'] for n in nodes})==len(tt)
for n in nodes:
    original=tt[n['baseline_file']];parent=nodes[n['parent']]['baseline_file'] if n['parent'] is not None else None
    assert original['parent']==parent and original['depth']==n['depth'] and original['box']==n['baseline_box']
    if n['children']:
        assert n['ge']==max(n['measured_ge'],max(nodes[c]['ge'] for c in n['children']))
        assert 128<=n['atlas_size']<=1024 and n['atlas_size']&(n['atlas_size']-1)==0
        assert n['planned_spacing_m']==max(n['ge']/16,1e-5)
        assert n['actual_spacing_m']>=n['planned_spacing_m']*(1-1e-10)
        if n['actual_spacing_m']>n['planned_spacing_m']*1.00001:assert n['atlas_size']==1024
        assert n['leaf_begin']==nodes[n['children'][0]]['leaf_begin'] and n['leaf_end']==nodes[n['children'][-1]]['leaf_end']
        for a,b in zip(n['children'],n['children'][1:]):assert nodes[a]['leaf_end']==nodes[b]['leaf_begin']
    else:assert n['ge']==0 and n['triangles']<=8192 and n['leaf_end']==n['leaf_begin']+1
samples={d:sum(n.get('error_samples_per_direction',0) for n in nodes if n['depth']==d) for d in {n['depth'] for n in nodes if n['children']}}
assert all(n>=200000 for n in samples.values())
a=json.loads((candidate/'tileset.json').read_text());b=json.loads((baseline/'tileset.json').read_text());assert a['root']['transform']==b['root']['transform']
record=dict(occupied_nodes=len(nodes),leaves=sum(not n['children'] for n in nodes),exact_baseline_ancestry_and_depth=True,baseline_bounds_read=True,exact_baseline_placement=True,monotonic_measured_error_recurrence=True,density_policy_valid=True,build_error_samples_per_direction_by_depth=samples)
Path(output).write_text(json.dumps(record,indent=2));print(record)
