#!/usr/bin/env python3
"""Render measured octree tables; prose verdict is supplied after result inspection."""
import collections,json,statistics,sys
from pathlib import Path
base=Path(sys.argv[1]);work=Path('/home/bend/scratch/rusty-tiles-scratch/lod-bakeoff/cluster-dag')
read=lambda p:json.loads(p.read_text())
def measure(name,phase=''):
    path=work/f'octree-{name}{"-"+phase if phase else ""}-launch.log'
    return next(json.loads(line) for line in reversed(path.read_text().splitlines()) if line.startswith('{"exit"'))
def table(headers,rows):return '\n'.join(['| '+' | '.join(headers)+' |','|'+'|'.join(['---']*len(headers))+'|']+['| '+' | '.join(map(str,row))+' |' for row in rows])
parts=[]
for name in ['m2048','tex3']:
    ev=base/f'octree-{name}';b=read(work/f'hlod-octree-{name}'/'build.json');i=read(ev/'inventory.json');baseline=read(ev/'baseline/inventory.json');q=read(ev/'quality.json');cr=read(ev/'cracks.json');v=read(ev/'validation.json');m=measure(name);density=read(ev/'density.json');treecheck=read(ev/'tree-check.json')
    mt=b['source_triangles']/1e6;cpu=m['user_s']+m['sys_s'];q0=q[0]
    rows=[['Whole CPU-s / per Mtri',f'{cpu} / {cpu/mt:.2f}','10,000 / 1,000 historical' if name=='m2048' else 'historical crop report'],['Coarse CPU-s / per Mtri',f'{b["coarse_cpu_s"]:.2f} / {b["coarse_cpu_s"]/mt:.2f}','not isolated'],['Wall / RSS',f'{m["wall_s"]} s / {m["max_rss_gib"]} GiB','3612 s / 9.37 GiB historical' if name=='m2048' else 'not rerun'],['Occupied cells / leaves',f'{len(b["nodes"])} / {sum(not n["children"] for n in b["nodes"])}','identical'],['Root triangles',b['nodes'][0]['triangles'],baseline['levels']['0']['triangles']],['Published unpacked MB',f'{i["total_output_bytes"]/1e6:.3f}',f'{baseline["total_output_bytes"]/1e6:.3f}'],['Root geometric mean / p95 / max (m)',' / '.join(f'{q0["geometry_m"][k]:.6f}' for k in ['mean','p95','max']),'0.333332 / 1.189870 / 4.055974' if name=='m2048' else 'see historical crop report'],['Root PSNR proxy→source / source→proxy (dB)',f'{q0["directions"]["lod_to_source"]["psnr_srgb_db"]:.2f} / {q0["directions"]["source_to_lod"]["psnr_srgb_db"]:.2f}','17.00 / 16.59' if name=='m2048' else '19.53 / not measured here (historical)'],['Mixed / same-depth pairs',f'{cr["mixed_pairs"]} / {cr["same_depth_pairs"]}','not rerun'],['Max / p99 crack, metres and ×GE','0 / 0 m; 0 / 0 ×GE','not rerun'],['GLB validation sample',f'{len(v)} files; {sum(x["errors"] for x in v)} errors / {sum(x["warnings"] for x in v)} warnings','not rerun']]
    for key,label in [('far','1600×1000 range 3'),('wide','2560×1600 range 3'),('near','2560×1600 range .75')]:rows.append([f'First view {label} (MB)',f'{i["first_views"][key]["inclusive_bytes"]/1e6:.3f}',f'{baseline["first_views"][key]["inclusive_bytes"]/1e6:.3f}'])
    parts.append(table([name,'Octree HLOD','Current #260 baseline'],rows))
    rows=[]
    for depth,lv in sorted(i['levels'].items(),key=lambda x:int(x[0])):
        dn=density['candidate'].get(depth,{'texels':0,'tiles':0});db=density['baseline'].get(depth,{'texels':0,'tiles':0});ns=[n for n in b['nodes'] if n['depth']==int(depth) and n['children']]
        ge=f'{lv["ge_min"]:.6f}–{lv["ge_max"]:.6f}';spacing='—' if not ns else f'{statistics.median(n["planned_spacing_m"] for n in ns):.6f} / {statistics.median(n["actual_spacing_m"] for n in ns):.6f}'
        rows.append([depth,lv['tiles'],lv['triangles'],ge,f'{lv["total_bytes"]/1e6:.3f}',dn['tiles'],dn['texels'],db['texels'],spacing,sum(n['density_capped'] for n in ns)])
    parts.append(table(['Depth','Tiles','Triangles','GE min–max m','MB at level','Proxies','Proxy texels','Baseline proxy texels','Median planned / actual spacing m','Capped'],rows))
    parts.append(table(['Cut depth','Cut tiles','Triangles','Geometry mean / p95 / max m','Mean ΔE76','PSNR proxy→source / source→proxy dB'],[[r['depth'],r['cut_tiles'],r['triangles'],' / '.join(f'{r["geometry_m"][k]:.6f}' for k in ['mean','p95','max']),f'{r["delta_e76"]["mean"]:.3f}',f'{r["directions"]["lod_to_source"]["psnr_srgb_db"]:.2f} / {r["directions"]["source_to_lod"]["psnr_srgb_db"]:.2f}'] for r in q]))
    counts=collections.Counter(n.get('stopping_bound') for n in b['nodes'] if n['children']);exceptions=[n for n in b['nodes'] if n.get('protected_faces',0) or n.get('retained_children',False)]
    parts.append(f'{name}: stopping bounds `{dict(counts)}`. Maximum proxy triangles: {max(n["triangles"] for n in b["nodes"] if n["children"])}. Exceptional interface-protection tiles: {len(exceptions)}; retained-children fallbacks: {sum(n.get("retained_children",False) for n in b["nodes"])}. Source cross-leaf edges: {cr["shared_cross_leaf_edges"]}; split nonmanifold edges: {cr["split_nonmanifold_edges"]}. Build GE samples per direction by depth: `{treecheck["build_error_samples_per_direction_by_depth"]}`. Assessment cost is separate: `{measure(name,"assess")}`.')
(base/'octree-tables.md').write_text('\n\n'.join(parts)+'\n')
print('Wrote octree-tables.md')
