"""Tabulate scale/*.json (stdlib)."""
import glob, json, os, sys
d0 = sys.argv[1] if len(sys.argv) > 1 else os.path.join(os.path.dirname(os.path.abspath(__file__)), 'scale')
for f in sorted(glob.glob(os.path.join(d0, '*.json'))):
    d = json.load(open(f)); n = os.path.basename(f)[:-5]
    if 'layoutError' in d:
        print(f'{n:34} ERROR {d["layoutError"]}'); continue
    r = d['rootDoc']
    print(f'{n:34} tiles={d["tiles"]:7} L={d["levels"]:2} expl={d["explicitTiles"]:6} roots={d["implicitRoots"]:6} links={d["linkTiles"]:5} '
          f'docs={d["documents"]:5} maxNodes={d["maxDocNodes"]:6} maxBytes={d["maxDocBytes"]:8} maxDepth={d["maxDocJsonDepth"]:2} '
          f'root={r["valueNodes"]}/{r["bytes"]} maxSub={d["maxSubtreeBytes"]:6} subtrees={d["subtrees"]:6} entries={d["archiveEntries"]:7} visits={d["hierarchyVisits"]:7}')
