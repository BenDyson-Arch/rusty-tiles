#!/usr/bin/env python3
"""First-view request simulation for rusty-tiles `mesh` forests.

Reads an unpacked output directory or a `.3tz`, rebuilds every tile (explicit tiles,
external documents, implicit subtrees), and replays CesiumJS's base traversal
(`Cesium3DTilesetBaseTraversal.js`, `Cesium3DTilesetTraversal.js`: skipLevelOfDetail
false, maximumScreenSpaceError 16) against the preview viewer's opening camera
(`preview/index.html` `frame()`: HeadingPitchRange(-0.8, -0.65, 3 r), 1600x1000,
fov 60 deg). It reports which resources the first view requests and the round trip on
which each becomes requestable: tileset.json is round trip 1; a member defined by a
document or subtree loaded on round trip k is requestable on round trip k + 1.

`--layout` re-lays the same logical tree with the planner's rule (skeleton height S,
inline threshold, document partition) so alternative layouts are simulated on identical
tiles, boxes and errors. `--layout current` must reproduce the parsed output.

usage: simulate_first_view.py OUTPUT [--layout S:INLINE:OBJECTS:RECORDS:LEVELS ...] [--json]
"""
import json
import math
import struct
import sys
import zipfile
from pathlib import Path

SSE_LIMIT = 16.0
WIDTH, HEIGHT = 1600, 1000
FOV = math.pi / 3


class Source:
    def __init__(self, path):
        self.path = Path(path)
        self.zip = None if self.path.is_dir() else zipfile.ZipFile(self.path)

    def read(self, name):
        if self.zip:
            return self.zip.read(name)
        return (self.path / name).read_bytes()

    def size(self, name):
        if self.zip:
            return self.zip.getinfo(name).file_size
        return (self.path / name).stat().st_size


# ---------------------------------------------------------------- logical tree


class LNode:
    __slots__ = ("center", "half", "error", "content", "children", "height", "count", "first_rank")

    def __init__(self, box, error, content):
        self.center = box[:3]
        self.half = box[3:]
        self.error = error
        self.content = content  # (name, bytes) or None
        self.children = []
        self.height = 0
        self.count = 1
        self.first_rank = 0


# A layout tile as Cesium sees it.
class CTile:
    __slots__ = ("node", "kind", "res", "res_bytes", "children", "parent", "loaded",
                 "refines", "visible", "dist", "sse", "wave", "parent_hint")

    def __init__(self, node, kind, res=None, res_bytes=0):
        self.node = node
        # "tile": children known with the tile; "link"/"iroot": child appears once `res` loads.
        self.kind = kind
        self.res = res  # content member (glb, json or subtree) or None (empty)
        self.res_bytes = res_bytes
        self.children = []
        self.parent = None
        self.loaded = False
        self.wave = None


def morton_decode(m, level):
    x = y = z = 0
    for b in range(level):
        x |= ((m >> (3 * b)) & 1) << b
        y |= ((m >> (3 * b + 1)) & 1) << b
        z |= ((m >> (3 * b + 2)) & 1) << b
    return x, y, z


def expand_implicit(src, tile_json):
    it = tile_json["implicitTiling"]
    levels = it["availableLevels"]
    assert it["subdivisionScheme"] == "OCTREE" and it["subtreeLevels"] == levels
    sub_name = fill(it["subtrees"]["uri"], 0, 0, 0, 0)
    data = src.read(sub_name)
    _m, _v, jlen, blen = struct.unpack("<4sIQQ", data[:24])
    js = json.loads(data[24:24 + jlen])
    binary = data[24 + jlen:24 + jlen + blen]

    def view(i):
        v = js["bufferViews"][i]
        o = v.get("byteOffset", 0)
        return binary[o:o + v["byteLength"]]

    total = (8 ** levels - 1) // 7

    def bits(av):
        if "constant" in av:
            return [bool(av["constant"])] * total
        b = view(av["bitstream"])
        return [bool(b[i >> 3] >> (i & 7) & 1) for i in range(total)]

    tiles = bits(js["tileAvailability"])
    contents = bits(js["contentAvailability"][0]) if js.get("contentAvailability") else [False] * total
    table = js["propertyTables"][js["tileMetadata"]]
    boxes = view(table["properties"]["boundingBox"]["values"])
    errors = view(table["properties"]["geometricError"]["values"])
    template = tile_json["content"]["uri"] if "content" in tile_json else None
    nodes = {}
    row = 0
    for i in range(total):
        if not tiles[i]:
            continue
        level = 0
        while (8 ** (level + 1) - 1) // 7 <= i:
            level += 1
        x, y, z = morton_decode(i - (8 ** level - 1) // 7, level)
        box = list(struct.unpack_from("<12d", boxes, row * 96))
        err = struct.unpack_from("<d", errors, row * 8)[0]
        row += 1
        content = None
        if contents[i]:
            name = fill(template, level, x, y, z)
            content = (name, src.size(name))
        node = LNode(box, err, content)
        nodes[(level, x, y, z)] = node
        if level:
            nodes[(level - 1, x >> 1, y >> 1, z >> 1)].children.append(node)
    return nodes[(0, 0, 0, 0)], sub_name, len(data), nodes


def fill(template, level, x, y, z):
    return (template.replace("{level}", str(level)).replace("{x}", str(x))
            .replace("{y}", str(y)).replace("{z}", str(z)))


def parse_output(src):
    """Parse the output into (logical root, Cesium layout root, stats)."""
    stats = {"documents": 0, "implicit_roots": 0, "subtree_levels": {}, "doc_bytes": [],
             "subtree_bytes": []}

    def tile(tj):
        box = tj["boundingVolume"]["box"]
        uri = tj.get("content", {}).get("uri")
        if "implicitTiling" in tj:
            lroot, sub, sub_bytes, _ = expand_implicit(src, tj)
            stats["implicit_roots"] += 1
            lv = tj["implicitTiling"]["availableLevels"]
            stats["subtree_levels"][lv] = stats["subtree_levels"].get(lv, 0) + 1
            stats["subtree_bytes"].append(sub_bytes)
            c = CTile(lroot, "iroot", sub, sub_bytes)
            c.children = [implicit_ctiles(lroot)]
            return lroot, c
        if uri and uri.endswith(".json"):
            doc = json.loads(src.read(uri))
            stats["documents"] += 1
            stats["doc_bytes"].append(src.size(uri))
            lnode, inner = tile(doc["root"])
            c = CTile(lnode, "link", uri, src.size(uri))
            c.children = [inner]
            return lnode, c
        content = (uri, src.size(uri)) if uri else None
        lnode = LNode(box, tj["geometricError"], content)
        c = CTile(lnode, "tile", uri, content[1] if content else 0)
        for child in tj.get("children", []):
            ln, cc = tile(child)
            lnode.children.append(ln)
            c.children.append(cc)
        return lnode, c

    top = json.loads(src.read("tileset.json"))
    stats["documents"] += 1
    stats["doc_bytes"].append(src.size("tileset.json"))
    lroot, croot = tile(top["root"])
    finish_heights(lroot)
    link_parents(croot)
    return lroot, croot, top["geometricError"], stats


def implicit_ctiles(node):
    c = CTile(node, "tile", node.content[0] if node.content else None,
              node.content[1] if node.content else 0)
    c.children = [implicit_ctiles(ch) for ch in node.children]
    return c


def finish_heights(node):
    # The rank order of a tile's first leaf (H1 preorder) breaks demotion ties.
    rank = 0
    pre = [node]
    while pre:
        n = pre.pop()
        n.first_rank = rank
        if not n.children:
            rank += 1
        pre.extend(reversed(n.children))
    stack = [(node, False)]
    while stack:
        n, done = stack.pop()
        if done or not n.children:
            n.height = 1 + max((c.height for c in n.children), default=-1)
            n.count = 1 + sum(c.count for c in n.children)
        else:
            stack.append((n, True))
            stack.extend((c, False) for c in n.children)


def link_parents(root):
    stack = [root]
    while stack:
        t = stack.pop()
        for c in t.children:
            c.parent = t
            stack.append(c)


# ---------------------------------------------------------------- re-layout

# Value nodes and compact bytes of tile objects, measured from H16 output shapes.
def explicit_cost(node):
    obj = {"boundingVolume": {"box": node.center + node.half}, "geometricError": node.error,
           "refine": "REPLACE", "extras": {"depth": 9, "first": "12345678", "end": "12345678"}}
    if node.content:
        obj["content"] = {"uri": "content/f12345678-d9.glb"}
    return json_cost(obj)


def iroot_cost(node):
    obj = {"boundingVolume": {"box": node.center + node.half}, "geometricError": node.error,
           "refine": "REPLACE", "content": {"uri": "implicit-content/f12345678-d9-{level}-{x}-{y}-{z}.glb"},
           "implicitTiling": {"availableLevels": 4, "subdivisionScheme": "OCTREE", "subtreeLevels": 4,
                              "subtrees": {"uri": "subtrees/f12345678-d9-{level}-{x}-{y}-{z}.subtree"}}}
    return json_cost(obj)


def link_cost(node):
    obj = {"boundingVolume": {"box": node.center + node.half}, "geometricError": node.error,
           "refine": "REPLACE", "content": {"uri": "tileset-f12345678-d9.json"}}
    return json_cost(obj)


def json_cost(value):
    def nodes(v):
        if isinstance(v, dict):
            return 1 + sum(nodes(x) for x in v.values())
        if isinstance(v, list):
            return 1 + sum(nodes(x) for x in v)
        return 1
    return nodes(value), len(json.dumps(value, separators=(",", ":"))) + 1


def subtree_bytes(node):
    levels = node.height + 1
    bits = (8 ** levels - 1) // 7
    stream = ((bits + 7) // 8 + 7) // 8 * 8
    return 24 + 680 + 2 * stream + node.count * (96 + 8) + node.count * 45


def relayout(lroot, s, inline_h, limits, budget):
    """Re-lay the logical tree as `plan::Builder` does. Skeleton tiles have height >= s.
    A shallow tree (non-leaf, height < s, under a skeleton tile or the whole tree) starts as
    explicit tiles when its height <= inline_h, otherwise as an implicit root.

    budget=False is develop's partition: a skeleton fragment over a cap (levels: >=) becomes
    a document at once. budget=True is this branch's `Builder::fit`."""
    objects_cap, records_cap, levels_cap = limits
    docs = []

    class F:  # a fragment node, as plan::Node
        __slots__ = ("n", "kind", "implicit", "children", "objects", "records", "levels", "target")

        def __init__(self, n, kind, children=()):
            self.n, self.kind, self.implicit, self.children = n, kind, False, list(children)
            self.objects = self.records = self.levels = 1

        def aggregate(self):
            self.objects = 1 + sum(c.objects for c in self.children)
            self.records = 1 + sum(c.records for c in self.children)
            self.levels = 1 + max((c.levels for c in self.children), default=0)

        def over(self):
            return (self.objects > objects_cap or self.records > records_cap
                    or self.levels > levels_cap)

    def shallow(n):
        f = F(n, "shallow")
        f.objects = f.records = n.count
        f.levels = n.height + 1
        if n.height > inline_h:
            f.implicit, f.objects, f.levels = True, 1, 1
        return f

    def is_shallow_explicit(c):
        return c.kind == "shallow" and not c.implicit

    def largest_inline(f):
        best = None
        stack = [(f, [])]
        while stack:
            g, path = stack.pop()
            for i, c in enumerate(g.children):
                if c.kind == "skeleton":
                    stack.append((c, path + [i]))
                elif is_shallow_explicit(c):
                    key = (c.records, c.n.height, -c.n.first_rank)
                    if best is None or key > best[0]:
                        best = (key, path + [i])
        return best and best[1]

    def demote(f, path):
        c = f.children[path[0]]
        if len(path) == 1:
            c.implicit, c.objects, c.levels = True, 1, 1
        else:
            demote(c, path[1:])
        f.aggregate()

    def externalize(f):
        jn, jb = fragment_cost(f)
        name = f"doc:{id(f.n)}"
        docs.append((name, jn, jb))
        link = F(f.n, "link")
        link.target = f
        link.records = 0
        return link

    def pick(f, measure, ok):
        best = None
        for i, c in enumerate(f.children):
            if ok(c) and (best is None or measure(c) > measure(f.children[best])):
                best = i
        return best

    def fit(f):
        while f.over():
            if f.objects > objects_cap:
                path = largest_inline(f)
                if path:
                    demote(f, path)
                    continue
            if f.levels > levels_cap:
                i = pick(f, lambda c: c.levels, lambda c: c.kind == "skeleton" or is_shallow_explicit(c))
                if i is not None and f.children[i].kind == "shallow":
                    demote(f, [i])
                    continue
            elif f.objects > objects_cap:
                i = pick(f, lambda c: c.objects, lambda c: c.kind == "skeleton")
            else:
                i = pick(f, lambda c: c.records, lambda c: c.kind == "skeleton")
            if i is None:
                break
            f.children[i] = externalize(f.children[i])
            f.aggregate()

    def close(n, is_root):
        if not n.children:
            return F(n, "leaf")
        if n.height < s:
            if is_root:
                f = shallow(n)
                if budget and not f.implicit and (f.objects > objects_cap or f.levels > levels_cap):
                    f.implicit, f.objects, f.levels = True, 1, 1
                return f
            return shallow(n)
        f = F(n, "skeleton", [close(c, False) for c in n.children])
        f.aggregate()
        if budget:
            fit(f)
            if not is_root and f.over():
                return externalize(f)
        elif not is_root and (f.objects > objects_cap or f.records > records_cap
                              or f.levels >= levels_cap):
            return externalize(f)
        return f

    def ctile(f):
        n = f.n
        if f.kind == "link":
            c = CTile(n, "link", f"doc:{id(n)}", fragment_cost(f.target)[1])
            c.children = [ctile(f.target)]
            return c
        if f.kind == "shallow" and f.implicit:
            c = CTile(n, "iroot", f"subtree:{id(n)}", subtree_bytes(n))
            c.children = [implicit_ctiles(n)]
            return c
        c = CTile(n, "tile", n.content[0] if n.content else None, n.content[1] if n.content else 0)
        if f.kind == "shallow":
            c.children = [ctile(F(ch, "shallow" if ch.children else "leaf")) for ch in n.children]
        else:
            c.children = [ctile(ch) for ch in f.children]
        return c

    def fragment_cost(f):
        if f.kind == "link":
            return link_cost(f.n)
        if f.kind == "shallow" and f.implicit:
            return iroot_cost(f.n)
        if f.kind == "shallow" or f.kind == "leaf":
            jn = jb = 0
            for t in walk(f.n):
                a, b = explicit_cost(t)
                jn, jb = jn + a, jb + b
            return jn, jb
        jn, jb = explicit_cost(f.n)
        for c in f.children:
            a, b = fragment_cost(c)
            jn, jb = jn + a, jb + b
        return jn + 1, jb + 12

    root = close(lroot, True)
    docs.append(("tileset.json",) + fragment_cost(root))
    croot = ctile(root)
    link_parents(croot)
    return croot, docs


def walk(n):
    stack = [n]
    while stack:
        t = stack.pop()
        yield t
        stack.extend(t.children)


# ---------------------------------------------------------------- camera and traversal

def camera_for(root_center, radius):
    heading, pitch, rng = -0.8, -0.65, max(radius * 3, 5)
    # Cesium offsetFromHeadingPitchRange in the ENU frame at the sphere centre.
    h = heading % (2 * math.pi) - math.pi / 2
    p = pitch
    # rotation: heading about Z by -h, pitch about Y by -p, applied to UNIT_X.
    x = [math.cos(-p), 0.0, -math.sin(-p)]  # rotate X about Y by -p
    x = [x[0] * math.cos(-h) - x[1] * math.sin(-h), x[0] * math.sin(-h) + x[1] * math.cos(-h), x[2]]
    offset = [-v * rng for v in x]
    pos = [root_center[i] + offset[i] for i in range(3)]
    d = norm([-v for v in offset])
    right = norm(cross(d, [0, 0, 1]))
    up = cross(right, d)
    aspect = WIDTH / HEIGHT
    fovy = 2 * math.atan(math.tan(FOV / 2) / aspect) if aspect > 1 else FOV
    tan_y = math.tan(fovy / 2)
    tan_x = tan_y * aspect
    planes = []
    for sx in (-1, 1):  # left/right: normal points inward
        n = norm([d[i] * tan_x - sx * right[i] for i in range(3)])
        planes.append((n, -dot(n, pos)))
    for sy in (-1, 1):
        n = norm([d[i] * tan_y - sy * up[i] for i in range(3)])
        planes.append((n, -dot(n, pos)))
    planes.append((d, -dot(d, pos) - 0.1))  # near
    return {"pos": pos, "planes": planes, "sse_den": 2 * tan_y}


def dot(a, b):
    return a[0] * b[0] + a[1] * b[1] + a[2] * b[2]


def cross(a, b):
    return [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]]


def norm(a):
    m = math.sqrt(dot(a, a))
    return [v / m for v in a]


def box_axes(node):
    h = node.half
    return [h[0:3], h[3:6], h[6:9]]


def visible(node, cam):
    axes = box_axes(node)
    for n, d in cam["planes"]:
        r = sum(abs(dot(n, a)) for a in axes)
        if dot(n, node.center) + d < -r:
            return False
    return True


def distance(node, cam):
    # Distance from the camera to an axis-aligned (or general) OBB.
    axes = box_axes(node)
    rel = [cam["pos"][i] - node.center[i] for i in range(3)]
    sq = 0.0
    for a in axes:
        length = math.sqrt(dot(a, a))
        if length == 0:
            continue
        u = [v / length for v in a]
        t = dot(rel, u)
        if t > length:
            sq += (t - length) ** 2
        elif t < -length:
            sq += (t + length) ** 2
    return math.sqrt(sq)


def simulate(croot, tileset_error, cam):
    """Return the requested CTiles with their round trip. Waves follow resource loads."""
    def known_children(t):
        if t.kind == "tile" or t.loaded:
            return t.children
        return []

    def renderable(t):
        if t.res is None:
            return False
        if t.kind != "tile" and t.loaded:
            return False
        return True

    def update(t):
        n = t.node
        t.visible = visible(n, cam)
        if t.kind != "tile" and t.loaded and t.visible:
            update(t.children[0])
            t.visible = t.children[0].visible
        t.dist = max(distance(n, cam), 1e-7)
        t.sse = n.error * HEIGHT / (t.dist * cam["sse_den"]) if n.error else 0.0

    def can_traverse(t):
        if not known_children(t):
            return False
        if t.kind != "tile":
            return True
        return t.sse > SSE_LIMIT

    requested = []

    def load(t):
        if renderable(t) and not t.loaded and t not in seen:
            seen.add(t)
            requested.append(t)

    def empty_traversal(root):
        stack = [root]
        while stack:
            t = stack.pop()
            traverse = not renderable(t) and can_traverse(t)
            update(t)
            if not t.visible:
                load(t)
            if traverse:
                stack.extend(known_children(t))

    wave = 1
    out = []
    while True:
        seen = set()
        requested.clear()
        update(croot)
        root_sse = tileset_error * HEIGHT / (croot.dist * cam["sse_den"])
        if croot.visible and root_sse > SSE_LIMIT:
            stack = [croot]
            while stack:
                t = stack.pop()
                if can_traverse(t):
                    kids = known_children(t)
                    for k in kids:
                        update(k)
                    check = renderable(t)
                    for k in kids:
                        if k.visible:
                            stack.append(k)
                        elif check:
                            load(k)
                        if check and k.visible is not None and not renderable(k):
                            empty_traversal(k)
                load(t)
        if not requested:
            break
        wave += 1
        for t in requested:
            t.loaded = True
            t.wave = wave
            out.append(t)
    return out


def kind_of(t):
    if t.kind == "link":
        return "json"
    if t.kind == "iroot":
        return "subtree"
    return "glb"


def schedule(requests, rtt, connections=6, mbit=None):
    """Finish time of the first view: each request costs one RTT (plus transfer time),
    `connections` in flight, a request becomes ready when the resource defining it
    finishes. tileset.json is a request of its own."""
    import heapq
    bw = None if mbit is None else mbit * 1e6 / 8

    def cost(t):
        return rtt + (t.res_bytes / bw if bw else 0.0)

    parents = {}
    for t in requests:
        p = t.parent
        while p is not None and not (p.kind != "tile" and p in parents_set):
            p = p.parent
        parents[t] = p
    finish = {}
    root_done = rtt
    ready = []
    for t in requests:
        p = parents[t]
        ready.append((t, p))
    # Resolve in wave order: a request's ready time is its defining resource's finish.
    slots = [root_done] * connections
    heapq.heapify(slots)
    for t, p in sorted(ready, key=lambda tp: tp[0].wave):
        ready_at = finish.get(p, root_done)
        start = max(heapq.heappop(slots), ready_at)
        end = start + cost(t)
        finish[t] = end
        heapq.heappush(slots, end)
    return max(finish.values(), default=root_done)


parents_set = set()


def report(croot, tileset_error, label):
    global parents_set
    radius = math.sqrt(sum(sum(croot.node.half[a * 3 + i] for a in range(3)) ** 2 for i in range(3)))
    cam = camera_for(croot.node.center, radius)
    reqs = simulate(croot, tileset_error, cam)
    parents_set = set(t for t in reqs if t.kind != "tile")
    by = {"json": 0, "subtree": 0, "glb": 0}
    for t in reqs:
        by[kind_of(t)] += 1
    by["json"] += 1  # tileset.json
    depth = max((t.wave for t in reqs), default=1)
    nbytes = sum(t.res_bytes for t in reqs)
    return {
        "layout": label,
        "requests": sum(by.values()),
        "json": by["json"], "subtree": by["subtree"], "glb": by["glb"],
        "depth": depth,
        "glb_depth_histogram": hist([t.wave for t in reqs if kind_of(t) == "glb"]),
        "metadata_bytes": sum(t.res_bytes for t in reqs if kind_of(t) != "glb"),
        "bytes": nbytes,
        "rtt200_6conn_s": round(schedule(reqs, 0.2), 2),
        "rtt200_6conn_50mbit_s": round(schedule(reqs, 0.2, mbit=50), 2),
        "names": sorted(t.res for t in reqs),
    }


def hist(values):
    h = {}
    for v in values:
        h[v] = h.get(v, 0) + 1
    return dict(sorted(h.items()))


def layout_stats(croot, docs):
    roots = {}
    for t in walk_c(croot):
        if t.kind == "iroot":
            roots[t.node.height + 1] = roots.get(t.node.height + 1, 0) + 1
    return {
        "documents": len(docs),
        "implicit_roots": sum(roots.values()),
        "subtree_levels": dict(sorted(roots.items())),
        "max_doc_nodes": max(d[1] for d in docs),
        "max_doc_bytes": max(d[2] for d in docs),
        "total_json_bytes": sum(d[2] for d in docs),
    }


def walk_c(c):
    stack = [c]
    while stack:
        t = stack.pop()
        yield t
        stack.extend(t.children)


def main(argv):
    path = argv[1]
    layouts = []
    as_json = "--json" in argv
    if "--layout" in argv:
        for spec in argv[argv.index("--layout") + 1:]:
            if spec.startswith("--"):
                break
            layouts.append(spec)
    src = Source(path)
    lroot, croot, terr, stats = parse_output(src)
    results = []
    actual = report(croot, terr, "output")
    actual["stats"] = {
        "documents": stats["documents"], "implicit_roots": stats["implicit_roots"],
        "subtree_levels": dict(sorted(stats["subtree_levels"].items())),
        "total_json_bytes": sum(stats["doc_bytes"]),
        "subtree_bytes_median": sorted(stats["subtree_bytes"])[len(stats["subtree_bytes"]) // 2]
        if stats["subtree_bytes"] else 0,
        "tiles": lroot.count, "height": lroot.height,
    }
    results.append(actual)
    for spec in layouts:
        spec = {"develop": "4:0:32:1024:8", "branch": "B4:3:480:4096:12"}.get(spec, spec)
        budget = spec.startswith("B")
        s, inline_h, o, r, lv = (int(v) for v in spec.lstrip("B").split(":"))
        c2, docs = relayout(lroot, s, inline_h, (o, r, lv), budget)
        res = report(c2, terr, spec)
        res["stats"] = layout_stats(c2, docs)
        results.append(res)
    if as_json:
        print(json.dumps(results, indent=1))
    else:
        for r in results:
            names = r.pop("names")
            print(json.dumps(r))
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
