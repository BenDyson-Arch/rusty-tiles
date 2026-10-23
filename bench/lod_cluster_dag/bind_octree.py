#!/usr/bin/env python3
"""Freeze final artifacts and verify execution/environment identities, no builds."""
import hashlib,json,sys
from pathlib import Path
base=Path(sys.argv[1]);work=Path('/home/bend/scratch/rusty-tiles-scratch/lod-bakeoff/cluster-dag');repo=Path.cwd()
def digest(p):
    h=hashlib.sha256()
    with p.open('rb') as f:
        for b in iter(lambda:f.read(4*1024*1024),b''):h.update(b)
    return h.hexdigest()
verified={}
for name in ['octree-execution-source.sha256','hlod-external-dependencies.sha256']:
    rows=(base/name).read_text().splitlines()
    for line in rows:
        h,path=line.split('  ',1);assert digest(Path(path))==h,path
    verified[name]=len(rows)
paths=[]
for name in ['tex3','m2048']:
    root=work/f'hlod-octree-{name}'
    assert (base/f'octree-{name}'/'complete').exists() and (base/f'octree-{name}'/'extras-complete').exists()
    paths.extend(p for p in root.rglob('*') if p.is_file() and '.run' not in p.relative_to(root).parts)
    paths.extend(p for p in (base/f'octree-{name}').rglob('*') if p.is_file())
    # Bind input baseline availability and every content/image dependency, including root JSON.
    baseline=Path('/home/bend/scratch/rusty-tiles-scratch/proxy-density')/name/'view'
    paths.extend(p for p in baseline.rglob('*') if p.is_file())
    for phase in ['', '-assess']:
        p=work/f'octree-{name}{phase}-launch.log';paths.append(p)
    for folder in [root/'.run',root/'.run/assessment']:
        p=folder/'disk-watchdog.json'
        if p.exists():paths.append(p)
paths.extend(Path(p) for p in ['/home/bend/.cache/rusty-tiles-preview/input/mgal_detail_offset_2048.glb','/home/bend/.cache/rusty-tiles-evidence/h17-20261013/crop/tex3.glb'])
paths.extend(base/n for n in ['octree-controls.json','octree-segment-controls.json','octree-baseline-availability.json','octree-prelaunch.json','hlod-environment.json','hlod-external-dependencies.sha256','m2048/baseline/root-quality.json','octree-transfer-breakdown.json','octree-reference-error-check.json','octree-preflight-review-r1.json','octree-contract.md','octree-method.md','octree-tables.md'])
paths=sorted(set(paths));(base/'octree-artifacts.sha256').write_text(''.join(digest(p)+'  '+str(p)+'\n' for p in paths))
# Additional assessment/report files were introduced after the build freeze and
# are bound separately; none changes the frozen builder or final GLBs.
extra=[repo/'bench/lod_cluster_dag'/n for n in ['density_octree.py','check_octree_tree.py','extra_octree.sh','report_octree.py','bind_octree.py']]+[work/'hlod-octree-m2048/.run/assess-all.sh']
(base/'octree-final-execution-source.sha256').write_text((base/'octree-execution-source.sha256').read_text()+''.join(digest(p)+'  '+str(p)+'\n' for p in extra))
record=dict(verified_preexisting_manifests=verified,artifact_files=len(paths),artifact_bytes=sum(p.stat().st_size for p in paths),artifact_manifest_sha256=digest(base/'octree-artifacts.sha256'),execution_manifest_sha256=digest(base/'octree-final-execution-source.sha256'))
(base/'octree-bind.json').write_text(json.dumps(record,indent=2));print(record)
