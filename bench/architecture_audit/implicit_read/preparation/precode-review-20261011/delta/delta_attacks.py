# Reviewer delta probes against I1 contract v3 (stdlib only; no repo imports).
import json, re

def uri(base, value):
    """Transcription of validate.rs:165-210 (f2ed077)."""
    if not value or "\\" in value: raise ValueError("InvalidInput")
    if any(c in value for c in ":?#") or value.startswith("/"): raise ValueError("Unsupported")
    parts = base.rsplit("/", 1)[0].split("/") if "/" in base else []
    for raw in value.split("/"):
        b = raw.encode(); out = bytearray(); i = 0
        while i < len(b):
            if b[i] == 0x25:
                h = b[i+1:i+3]
                if len(h) < 2 or not all(chr(x) in "0123456789abcdefABCDEF" for x in h): raise ValueError("InvalidInput escape")
                out.append(int(h, 16)); i += 3
            else: out.append(b[i]); i += 1
        part = out.decode()
        if "/" in part or "\\" in part or any(ord(c) < 32 or 127 <= ord(c) < 160 for c in part): raise ValueError("InvalidInput sep")
        if part in ("", "."): continue
        if part == "..":
            if not parts: raise ValueError("InvalidInput escape archive")
            parts.pop(); continue
        parts.append(part)
    return "/".join(parts)

def v3_detect(t):
    """Contract v3 §3 JSON detection, steps 1-5."""
    t = re.split(r"[?#]", t, 1)[0]
    seg = t.rsplit("/", 1)[-1]
    if re.search(r"%.{0,2}\{", seg): return True
    dec = re.sub(r"%([0-9A-Fa-f]{2})", lambda m: chr(int(m.group(1), 16)), seg)
    return dec.lower().endswith(".json")

def subst(t, l, x, y, z):
    return t.replace("{level}", str(l)).replace("{x}", str(x)).replace("{y}", str(y)).replace("{z}", str(z))

cases = ["c/{level}-{x}-{y}-{z}.json",
         "c/{level}-{x}-{y}-{z}.json/.",
         "c/{level}-{x}-{y}-{z}.json/",
         "c/x.json/{level}-{x}-{y}-{z}/..",
         "c/{level}-{x}-{y}-{z}.JSON//./"]
rows = []
for t in cases:
    resolved = uri("tileset.json", subst(t, 1, 0, 1, 0))
    rows.append({"template": t, "v3_static_detects": v3_detect(t), "resolved_member": resolved,
                 "resolved_is_json": resolved.lower().endswith(".json")})
# Attack: per-table class-property scan (no memoization stated in v3 §5).
P = (65536 - 200) // 2          # opaque class properties {"type":"BOOLEAN"}: 2 nodes each
subtrees_octree_A6 = sum(8**l for l in range(6))   # L=1, child Const1: one subtree per tile
class_scan_visits = subtrees_octree_A6 * P
# Attack: content index not reserved (v3 §2a step 10 'check only').
refs = 262144; K = 29000; T = 9  # OCTREE L=2, tile Const1, K Const1 slots aliased to 1 member per slot via '/..'
live, used, s = [], 0, 0
while True:
    E = T * K
    if E > refs - used: break
    live.append(E); used += 2 * K   # two tiles on the path emit K contents each before descending
print(json.dumps({"json_detection": rows,
                  "class_scan": {"opaque_class_properties": P, "subtrees": subtrees_octree_A6,
                                 "property_visits_without_memo": class_scan_visits},
                  "content_index_live": {"per_subtree_E": live, "live_sum": sum(live), "references": refs,
                                         "ratio": round(sum(live) / refs, 2)}}, indent=1))
