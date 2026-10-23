import pathlib,zipfile,sys,subprocess,json,os
E=pathlib.Path('/home/bend/.cache/rusty-tiles-evidence/fast-lod-exp-20261024');S=pathlib.Path('/home/bend/scratch/rusty-tiles-scratch/lod-bakeoff/astra-keep-h17')
name=sys.argv[1];w=S/name;view=w/'view';view.mkdir(exist_ok=True)
with zipfile.ZipFile(w/'out.3tz') as z:z.extractall(view)
for d in ['content','implicit-content']:(view/d).mkdir(exist_ok=True)
subprocess.run(['node','bench/fast_lod_experiments/decode.mjs',str(view),str(w/'decoded')],check=True)
sys.path.insert(0,'/home/bend/.cache/rusty-tiles-evidence/leaf-gaps-20261024');from tile_tree import tree
meta=tree(str(view));(w/'decoded/tree.json').write_text(json.dumps(meta))
for width,height,r in [(1600,1000,'3'),(2560,1600,'0.75')]:
 env=os.environ|{'SIM_W':str(width),'SIM_H':str(height),'SIM_RANGE':r}
 with open(E/name/f'firstview-{width}-{r}.json','w') as out:subprocess.run(['python3','/tmp/simview/sim.py',str(view),'--json'],env=env,stdout=out,check=True)
if not name.startswith('legacy'):
 with open(E/name/'decode-seams.log','w') as out:subprocess.run(['node','/home/bend/.cache/rusty-tiles-evidence/leaf-gaps-20261024/decode_tiles.mjs',str(view),str(w/'seams')],stdout=out,check=True)
