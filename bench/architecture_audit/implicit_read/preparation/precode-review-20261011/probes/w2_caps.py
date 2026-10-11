# Reviewer probe: W2 live-path subtree sum vs C1 per-payload view bytes (document_decoded_bytes share).
import json, struct, sys, zipfile
z = zipfile.ZipFile(sys.argv[1])
names = {i.filename: i.file_size for i in z.infolist()}
ts = json.loads(z.read("tileset.json"))
root = ts["root"]
it = root["implicitTiling"]
L = it["subtreeLevels"]; A = it["availableLevels"]
sub_t = it["subtrees"]["uri"]
octree = it["subdivisionScheme"] == "OCTREE"
subs = {n: s for n, s in names.items() if n.endswith(".subtree")}
def key(level, x, y, z):
    u = sub_t.replace("{level}", str(level)).replace("{x}", str(x)).replace("{y}", str(y)).replace("{z}", str(z))
    return u
# path sum: subtree at (level,x,y,z) has ancestors at levels level-L, level-2L ... with coords >> k*L
best = (0, None)
for n, s in subs.items():
    base = n.rsplit("/", 1)[-1][:-len(".subtree")]
    parts = list(map(int, base.split("-")))
    level, x, y = parts[0], parts[1], parts[2]
    zc = parts[3] if octree else 0
    total = 0; chain = []
    l = level
    while l >= 0:
        sh = level - l
        k = key(l, x >> sh, y >> sh, zc >> sh)
        total += names[k]; chain.append((k, names[k]))
        l -= L
    if total > best[0]:
        best = (total, chain)
print("subtrees", len(subs), "cumulative", sum(subs.values()), "maxLivePath", best[0], "depthOfMax", len(best[1]))
print("root subtree", names[key(0,0,0,0)])
# per-payload sum of bufferView byteLength (C1 view budget) for GLBs
mx = 0; cnt = 0
for n in names:
    if n.endswith(".glb"):
        b = z.read(n); cnt += 1
        jl = struct.unpack_from("<I", b, 12)[0]
        g = json.loads(b[20:20 + jl])
        v = sum(bv["byteLength"] for bv in g.get("bufferViews", []))
        mx = max(mx, v)
print("glb", cnt, "maxViewBytes", mx)
