"""Release-only exact tex3 page sweep. Run at idle priority.
Usage: python3 run_sweep.py LIB_TEST_BINARY EVIDENCE_DIR SCRATCH_DIR
Sweep every distinct native ETC1S level0..6 via wrapper efforts0,1,3,5,6,8,10.
CPU is summed per-thread time around raw encoding, excluding verification,
decoding, metric calculation, file IO and worker scheduling.
"""
import json, math, os, subprocess, sys
from pathlib import Path

binary, evidence, scratch = sys.argv[1:]
evidence, scratch = Path(evidence), Path(scratch)
configurations = [('etc1s',0),('uastc',2),('uastc',0)]
configurations += [('etc1s',e) for e in [1,3,5,6,8,10]]
summary = []
for codec, effort in configurations:
    for profile,quality in [('fast',50),('balanced',90)]:
        name=f'{profile}-{codec}-e{effort}'
        dest=evidence/name; dest.mkdir(parents=True,exist_ok=True)
        pages=scratch/f'pages-{profile}'
        assert len(list(pages.glob('*/page.json')))==217, profile
        if not (dest/'report.json').exists():
            env=dict(os.environ,RT_MEASURE_SLOT='2',RT_SWEEP_PAGES=str(pages),RT_SWEEP_OUT=str(dest/'report.json'),RT_SWEEP_CODEC=codec,RT_SWEEP_QUALITY=str(quality),RT_SWEEP_EFFORT=str(effort),RAYON_NUM_THREADS='2',RUST_TEST_THREADS='2')
            cmd=['systemd-run','--user','--scope','-q','-p','MemoryMax=10G','-p','MemorySwapMax=0','--','chrt','-i','0','nice','-n','19','python3','-I','/home/bend/.cache/rt-scripts/measure.py',str(dest/'stdout.log'),binary,'--exact','basisu_ktx2::tests::effort_sweep::measure_tex3_efforts','--ignored','--nocapture']
            print('RUN',name,flush=True)
            with (dest/'measure.json').open('w') as out, (dest/'stderr.log').open('w') as err:
                subprocess.run(cmd,env=env,stdout=out,stderr=err,check=True)
            assert json.loads((dest/'measure.json').read_text())['exit']==0, name
        report=json.loads((dest/'report.json').read_text()); row=report['summary']
        row.update(profile=profile,quality=quality if codec=='etc1s' else None,native_level=round(effort*.6) if codec=='etc1s' else round(effort*.4))
        minima={band:None for band in ['large','middle','tiny']}; misses=[]; joint_min=None
        for page in report['pages']:
            failed=False
            tiny_rgb={}
            for metric in page['levels']:
                size=metric['texels']; band='large' if size>=1024 else 'middle' if size>=64 else 'tiny'
                psnr=metric['psnr']
                if metric['channel']<3:
                    if psnr is not None: minima[band]=psnr if minima[band] is None else min(minima[band],psnr)
                    floor={'large':20,'middle':16,'tiny':13}[band]
                    failed |= psnr is not None and psnr<floor
                    if band=='tiny': tiny_rgb.setdefault(metric['mip'],[]).append(metric)
                    failed |= size>=1024 and metric['p999']>(112 if profile=='fast' else 80)
                    failed |= metric['blockRmse']>(96 if profile=='fast' else 80)
                else:
                    failed |= psnr is not None and psnr<30
                    failed |= metric['max']>(80 if profile=='fast' else 64)
            for metrics in tiny_rgb.values():
                sse=sum(m['sse'] for m in metrics); weight=sum(m['weight'] for m in metrics)
                if sse>0:
                    joint=10*math.log10(65025*weight/sse)
                    joint_min=joint if joint_min is None else min(joint_min,joint)
                    failed |= joint<16
            if failed: misses.append(page['page'])
        row.update(channel_psnr_min=minima,tiny_joint_rgb_min=joint_min,class_gate_misses=misses)
        if codec=='etc1s' and effort==3:
            assert row['bytes']=={'fast':10373367,'balanced':14909217}[profile], 'caller dump did not reproduce original encoding'
        if codec=='uastc' and effort==0:
            assert row['bytes']=={'fast':37105816,'balanced':39083023}[profile], 'caller dump did not reproduce develop UASTC encoding'
        summary.append(row)
        temp=evidence/'sweep-summary.tmp';temp.write_text(json.dumps(summary,indent=2)+'\n');temp.replace(evidence/'sweep-summary.json')
        print('DONE',name,json.dumps(row),flush=True)
