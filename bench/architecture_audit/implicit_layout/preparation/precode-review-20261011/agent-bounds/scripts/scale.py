# Forest scaling model: balanced binary k-d tree, each leaf topped by a chain of c coarse tiles.
import math
S=4; NODES_PER_JSON_TILE=50; BYTES_PER_JSON_TILE=650; SRC_BYTES=73
def model(leaves,c):
    # heights: leaf-chain bottom tile h=0..c ; kd internal node at kd-height j has h=j+c
    d=math.ceil(math.log2(leaves)) if leaves>1 else 0
    N=leaves*(c+1)+(leaves-1)
    sk=0; roots=0; xleaf=0
    # chain tiles with h>=S are skeleton
    sk+=leaves*max(0,c-S+1)
    # kd internal nodes count at kd-height j approx leaves/2^j
    for j in range(1,d+1):
        cnt=leaves/2**j
        if j+c>=S: sk+=cnt
    # children of skeleton with h<S
    if c>=S: roots+=leaves              # chain top segment of 4 tiles under skeleton chain tile (h=S-1)
    else:
        # kd nodes at height j with j+c in [1,S-1] whose parent j+1+c>=S
        j=S-1-c
        if j>=1: roots+=leaves/2**j
        elif c>=1: roots+=leaves          # chain top (h=c) is child of kd node j=1
        else: xleaf+=leaves               # c=0, S=... not reached for S=4
    json_tiles=sk+roots+xleaf
    depth=d+c   # tile depth of deepest leaf
    root_depth=depth-(min(c,S-1) if c>=1 else S-1)  # approx depth of deepest implicit root
    nodes=json_tiles*NODES_PER_JSON_TILE+N+40
    byts=json_tiles*BYTES_PER_JSON_TILE+SRC_BYTES*N+1500
    return N,int(sk),int(roots),int(json_tiles),nodes,byts,2*root_depth+5
print("c  N        skel   roots  jsonT  valueNodes  forestJSON  todayJSON  explicitJSON  maxJSONdepth")
for c in (0,3,16):
    for leaves in (256,2048,20000,200000):
        N,sk,r,j,n,b,jd=model(leaves,c)
        print(f"{c:<2} {N:<8} {sk:<6} {r:<6} {j:<6} {n:<11} {b/1e6:8.2f}MB {73*N/1e6+0.0014:8.2f}MB {650*N/1e6:9.1f}MB  {jd}")
