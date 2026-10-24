#!/usr/bin/env python3
"""Inventory and sensitivity analysis; no writes to the input tileset.

python analyze.py index VIEW > local/index.json
node measure.mjs VIEW local/index.json DECODER > local/tiles.jsonl
python analyze.py report VIEW local/tiles.jsonl > local/report.json
"""
import hashlib
import json
import math
from pathlib import Path
import sys
import first_view as sim


def depths(root):
    stack = [(root, 0)]
    while stack:
        n, d = stack.pop()
        yield n, d
        stack.extend((c, d+1) for c in reversed(n.children))


def index(root):
    return [dict(name=n.content[0], depth=d, error=n.error, proxy=bool(n.children),
                 extent=[2*math.sqrt(sim.dot(a,a)) for a in sim.box_axes(n)])
            for n,d in depths(root) if n.content]


def quantile(v, p):
    v = sorted(v)
    return v[round((len(v)-1)*p)]


def summary(rows):
    out = []
    for d in sorted({r['depth'] for r in rows}):
        for proxy in (True, False):
            rs = [r for r in rows if r['depth']==d and r['proxy']==proxy]
            if not rs:
                continue
            cols = {}
            for r in rs:
                r['long_extent'] = max(r['extent'])
                # Equivalent front-facing surface at the OBB switch distance;
                # excludes foreshortening, overlap and the centre-distance offset.
                r['switch_px2_per_tri'] = r['area']/r['triangles']*(16/r['error'])**2
                r['target_four_px'] = r['area']*(16/r['error'])**2/4
                r['switch_distance_1600'] = r['error']*1600/(32*math.tan(math.pi/6))
                r['entry_px2_per_tri'] = (r['area']/r['triangles']*(16/r['parent_error'])**2
                                         if r.get('parent_error') else r['switch_px2_per_tri'])
            for k in ('triangles','error','long_extent','area','switch_px2_per_tri','target_four_px',
                      'entry_px2_per_tri','switch_distance_1600'):
                cols[k] = [quantile([r[k] for r in rs],p) for p in (.1,.5,.9)]
            out.append(dict(depth=d,proxy=proxy,tiles=len(rs),**cols))
    return out


def run(view, rows, w, h, rng, factor=1, cap=None, growth=False, area_target=False, skip_odd=False):
    sim.WIDTH,sim.HEIGHT=w,h
    import os
    os.environ['SIM_RANGE']=str(rng)
    src=sim.Source(view)
    root, layout, terr, _=sim.parse_output(src)
    by={r['name']:r for r in rows}
    predicted={}
    for n,d in reversed(list(depths(root))):
        if not n.content:
            continue
        r=by[n.content[0]]
        ratio=1
        if n.children:
            target=cap if cap else r['triangles']
            if area_target:
                target=min(8192,max(256,r['area']*(16/(n.error*factor))**2/4))
            ratio=min(1,target/r['triangles'])
            n.error *= factor*(1/math.sqrt(ratio) if growth else 1)
            n.error=max(n.error,max(c.error for c in n.children))
        predicted[r['name']]=dict(triangles=r['triangles']*ratio,
            bytes=r['fixed_bytes']+r['texture_bytes']+r['geometry_bytes']*ratio)
    top_bytes=src.size('tileset.json')
    radius=math.sqrt(sum(sum(root.half[a*3+i] for a in range(3))**2 for i in range(3)))
    cam=sim.camera_for(root.center,radius)
    metadata_override=None
    if skip_odd:
        # Collapse alternate internal levels (retain all leaves). This is an
        # explicit-hierarchy analogue, NOT a valid re-addressed implicit octree.
        # Retain the existing metadata charge; changed layout costs are unmodelled.
        baseline=sim.simulate(layout,terr,cam)
        metadata_override=top_bytes+sum(t.res_bytes for t in baseline if sim.kind_of(t)!='glb')
        skipped={id(n) for n,d in depths(root) if n.children and d%2}
        for n,d in reversed(list(depths(root))):
            n.children=[g for c in n.children for g in (c.children if id(c) in skipped else [c])]
        layout=sim.implicit_ctiles(root)
        sim.link_parents(layout)
    reqs=sim.simulate(layout,terr,cam)
    names={t.res for t in reqs if sim.kind_of(t)=='glb'}
    # Visible final frontier: independent logical-tree walk; parent needed only if
    # it passes SSE or has no children. Excludes ancestor/offscreen sibling requests.
    selected=[]
    stack=[root]
    while stack:
        n=stack.pop()
        if not sim.visible(n,cam): continue
        sse=n.error*h/(max(sim.distance(n,cam),1e-7)*cam['sse_den'])
        if n.children and (sse>16 or n.content is None): stack.extend(n.children)
        elif n.content: selected.append(n.content[0])
    assert set(selected)<=names, (w,h,rng,factor,cap,growth,area_target,skip_odd, sorted(set(selected)-names)[:4])
    overhead=top_bytes+sum(t.res_bytes for t in reqs if sim.kind_of(t)!='glb')
    if metadata_override is not None: overhead=metadata_override
    return dict(view=f'{w}x{h}@{rng}', glbs=len(names), selected_glbs=len(selected),
        bytes=round(overhead+sum(predicted[n]['bytes'] for n in sorted(names))),
        triangles=round(sum(predicted[n]['triangles'] for n in sorted(names))),
        selected_triangles=round(sum(predicted[n]['triangles'] for n in selected)),
        metadata_bytes=overhead,texture_bytes=sum(by[n]['texture_bytes'] for n in names),
        selected_by_depth=sim.hist([by[n]['depth'] for n in selected]))


def report(view, rows):
    root=sim.parse_output(sim.Source(view))[0]
    by={r['name']:r for r in rows}
    for n,d in depths(root):
        for c in n.children:
            if c.content: by[c.content[0]]['parent_error']=n.error
    scenarios={
        'baseline':{},
        'a_cap2048_fixed_error':dict(cap=2048),
        'a_cap2048_error_grows':dict(cap=2048,growth=True),
        'a_four_px_fixed_error':dict(area_target=True),
        'b_half':dict(factor=.5),
        'b_quarter':dict(factor=.25),
        'b_eighth':dict(factor=.125),
        'c_omit_odd_proxies':dict(skip_odd=True),
        'd_quarter_cap2048':dict(factor=.25,cap=2048),
        'd_quarter_cap2048_error_grows':dict(factor=.25,cap=2048,growth=True),
        'd_quarter_four_px':dict(factor=.25,area_target=True),
        'd_quarter_omit_odd':dict(factor=.25,skip_odd=True),
    }
    return dict(input=str(Path(view).resolve()),
        inventory_sha256=hashlib.sha256(Path(sys.argv[3]).read_bytes()).hexdigest(),
        tiles=len(rows),total_triangles=sum(r['triangles'] for r in rows),
        leaf_triangles=sum(r['triangles'] for r in rows if not r['proxy']),
        depths=summary(rows),scenarios={s:[run(view,rows,w,h,r,**args)
            for w,h,r in [(1600,1000,3),(2560,1600,3),(2560,1600,.75)]]
            for s,args in scenarios.items()})


if __name__=='__main__':
    command,view=sys.argv[1:3]
    if command=='index':
        result=index(sim.parse_output(sim.Source(view))[0])
    elif command=='report':
        result=report(view,[json.loads(l) for l in Path(sys.argv[3]).read_text().splitlines()])
    else: raise ValueError(command)
    print(json.dumps(result,indent=2))
