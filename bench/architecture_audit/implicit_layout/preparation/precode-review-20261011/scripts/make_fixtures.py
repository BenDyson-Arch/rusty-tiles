"""Hand-made P-IMPL layout fixtures (stdlib only).

One logical hierarchy, written in four layouts:
  E  explicit tileset (reference)
  A  one implicit tree; child subtrees (subtreeLevels 2, availableLevels 4)
  X  today's chunked form: implicit roots whose `contents` carry an external-JSON
     template; links at level 2 open external implicit documents
  S  one implicit root holding the whole tree in a single subtree
     (subtreeLevels == availableLevels == 4, childSubtreeAvailability constant 0)
  F  recommended forest: explicit skeleton (tiles of height >= S=2) whose non-leaf
     children are implicit roots with subtreeLevels == availableLevels == 2, a
     translation `transform`, and childSubtreeAvailability constant 0

The hierarchy mimics the vector producer: a binary k-d split whose child slot is
the array index (x bit only, y = 0), so every non-root effective row escapes its
derived quadtree cell (rows are TILE_BOUNDING_BOX FLOAT64[12], as rusty-tiles
writes). Level-1 tile (1,1,0) is a contentless routing tile.

Usage: python3 -I make_fixtures.py OUT_DIR
"""
import json, math, os, struct, sys

PAD = 2.0
HALF_Z = 5.0
ERR = {0: 800.0, 1: 200.0, 2: 50.0, 3: 0.0}
LON, LAT = 12.5, 41.9
COLORS = {0: [1, 0, 0, 1], 1: [0, 1, 0, 1], 2: [0, 0.4, 1, 1], 3: [1, 1, 0, 1]}
ROUTING = {(1, 1)}


def enu_transform(lon_deg, lat_deg, h=0.0):
    a, f = 6378137.0, 1 / 298.257223563
    e2 = f * (2 - f)
    lon, lat = math.radians(lon_deg), math.radians(lat_deg)
    n = a / math.sqrt(1 - e2 * math.sin(lat) ** 2)
    p = [(n + h) * math.cos(lat) * math.cos(lon), (n + h) * math.cos(lat) * math.sin(lon), (n * (1 - e2) + h) * math.sin(lat)]
    east = [-math.sin(lon), math.cos(lon), 0]
    north = [-math.sin(lat) * math.cos(lon), -math.sin(lat) * math.sin(lon), math.cos(lat)]
    up = [math.cos(lat) * math.cos(lon), math.cos(lat) * math.sin(lon), math.sin(lat)]
    return east + [0] + north + [0] + up + [0] + p + [1]


def hierarchy():
    """{(level, x): rect [x0, y0, x1, y1]} with x-bit child slots (y == 0)."""
    tiles = {(0, 0): [-1000.0, -1000.0, 1000.0, 1000.0]}
    for level in (1, 2, 3):
        for (l, x), r in list(tiles.items()):
            if l != level - 1:
                continue
            for s in (0, 1):
                x0, y0, x1, y1 = r
                if level % 2 == 1:  # split along y
                    ym = (y0 + y1) / 2
                    child = [x0, y0, x1, ym] if s == 0 else [x0, ym, x1, y1]
                else:  # split along x
                    xm = (x0 + x1) / 2
                    child = [x0, y0, xm, y1] if s == 0 else [xm, y0, x1, y1]
                tiles[(level, 2 * x + s)] = child
    return tiles


def row(rect, origin=(0.0, 0.0)):
    x0, y0, x1, y1 = rect
    cx, cy = (x0 + x1) / 2 - origin[0], (y0 + y1) / 2 - origin[1]
    return [cx, cy, 0.0, (x1 - x0) / 2 + PAD, 0, 0, 0, (y1 - y0) / 2 + PAD, 0, 0, 0, HALF_Z]


def glb(rect, level, origin=(0.0, 0.0)):
    x0, y0, x1, y1 = [rect[0] - origin[0], rect[1] - origin[1], rect[2] - origin[0], rect[3] - origin[1]]
    z = 0.0
    # tile frame z-up (X, Y, Z) -> glTF y-up (X, Z, -Y)
    pts = [(x0, y0), (x1, y0), (x1, y1), (x0, y1)]
    pos = b''.join(struct.pack('<3f', px, z, -py) for px, py in pts)
    idx = struct.pack('<6H', 0, 1, 2, 0, 2, 3) + b'\0\0'
    binary = pos + idx
    doc = {"asset": {"version": "2.0"}, "scene": 0, "scenes": [{"nodes": [0]}], "nodes": [{"mesh": 0}],
           "meshes": [{"primitives": [{"attributes": {"POSITION": 0}, "indices": 1, "material": 0}]}],
           "materials": [{"pbrMetallicRoughness": {"baseColorFactor": COLORS[level], "metallicFactor": 0, "roughnessFactor": 1},
                          "doubleSided": True, "extensions": {"KHR_materials_unlit": {}}}],
           "extensionsUsed": ["KHR_materials_unlit"],
           "buffers": [{"byteLength": len(binary)}],
           "bufferViews": [{"buffer": 0, "byteOffset": 0, "byteLength": 48, "target": 34962},
                           {"buffer": 0, "byteOffset": 48, "byteLength": 12, "target": 34963}],
           "accessors": [{"bufferView": 0, "componentType": 5126, "count": 4, "type": "VEC3",
                          "min": [min(x0, x1), z, min(-y0, -y1)], "max": [max(x0, x1), z, max(-y0, -y1)]},
                         {"bufferView": 1, "componentType": 5123, "count": 6, "type": "SCALAR"}]}
    j = json.dumps(doc).encode()
    j += b' ' * (-len(j) % 4)
    total = 12 + 8 + len(j) + 8 + len(binary)
    return b'glTF' + struct.pack('<II', 2, total) + struct.pack('<I4s', len(j), b'JSON') + j + struct.pack('<I4s', len(binary), b'BIN\0') + binary


def morton2(x, y, level):
    v = 0
    for b in range(level):
        v |= ((x >> b) & 1) << (2 * b) | ((y >> b) & 1) << (2 * b + 1)
    return v


def subtree(levels, tiles_local, content_local, child_subtrees, rows):
    """tiles_local: set of (level, x, y); rows: {(level,x,y): (box, error)} in availability order."""
    def bits(n, ones):
        raw = bytearray((n + 7) // 8)
        for i in ones:
            raw[i >> 3] |= 1 << (i & 7)
        return bytes(raw)

    def index(l, x, y):
        return (4 ** l - 1) // 3 + morton2(x, y, l)

    n_tiles = (4 ** levels - 1) // 3
    binary = bytearray()
    views = []

    def view(data):
        while len(binary) % 8:
            binary.append(0)
        views.append({"buffer": 0, "byteOffset": len(binary), "byteLength": len(data)})
        binary.extend(data)
        return len(views) - 1

    def avail(n, ones):
        ones = sorted(ones)
        if not ones:
            return {"constant": 0}
        if len(ones) == n:
            return {"constant": 1}
        return {"bitstream": view(bits(n, ones)), "availableCount": len(ones)}

    order = sorted(tiles_local, key=lambda t: index(*t))
    doc = {"tileAvailability": avail(n_tiles, [index(*t) for t in tiles_local]),
           "contentAvailability": [avail(n_tiles, [index(*t) for t in tiles_local if t in content_local[s]]) for s in range(len(content_local))],
           "childSubtreeAvailability": avail(4 ** levels, [morton2(x, y, levels) for (x, y) in child_subtrees])}
    boxes = b''.join(struct.pack('<12d', *rows[t][0]) for t in order)
    errors = b''.join(struct.pack('<d', rows[t][1]) for t in order)
    strings = b''.join(b'{}' for _ in order)
    offsets = b''.join(struct.pack('<I', 2 * i) for i in range(len(order) + 1))
    doc["propertyTables"] = [{"class": "rustyTile", "count": len(order), "properties": {
        "boundingBox": {"values": view(boxes)}, "geometricError": {"values": view(errors)},
        "extras": {"values": view(strings), "stringOffsets": view(offsets)}}}]
    doc["tileMetadata"] = 0
    while len(binary) % 8:
        binary.append(0)
    doc["buffers"] = [{"byteLength": len(binary)}]
    doc["bufferViews"] = views
    j = json.dumps(doc).encode()
    j += b' ' * (-len(j) % 8)
    return b'subt' + struct.pack('<IQQ', 1, len(j), len(binary)) + j + bytes(binary)


SCHEMA = {"id": "rustyTilesImplicit", "classes": {"rustyTile": {"properties": {
    "boundingBox": {"type": "SCALAR", "componentType": "FLOAT64", "array": True, "count": 12, "semantic": "TILE_BOUNDING_BOX"},
    "geometricError": {"type": "SCALAR", "componentType": "FLOAT64", "semantic": "TILE_GEOMETRIC_ERROR"},
    "extras": {"type": "STRING"}}}}}


def write(path, data):
    os.makedirs(os.path.dirname(path), exist_ok=True)
    with open(path, 'wb') as f:
        f.write(data if isinstance(data, bytes) else json.dumps(data, indent=1).encode())


def has_content(key):
    return key not in ROUTING


def main(out):
    T = hierarchy()
    transform = enu_transform(LON, LAT)
    top_error = 3000.0
    kids = lambda k: [(k[0] + 1, 2 * k[1] + s) for s in (0, 1) if (k[0] + 1, 2 * k[1] + s) in T]

    # E: explicit
    def explicit(key, prefix):
        t = {"boundingVolume": {"box": row(T[key])}, "geometricError": ERR[key[0]], "refine": "REPLACE"}
        if has_content(key):
            t["content"] = {"uri": f"e/{key[0]}-{key[1]}.glb"}
            write(f"{prefix}/e/{key[0]}-{key[1]}.glb", glb(T[key], key[0]))
        if kids(key):
            t["children"] = [explicit(c, prefix) for c in kids(key)]
        return t
    root = explicit((0, 0), f"{out}/E")
    root["transform"] = transform
    write(f"{out}/E/tileset.json", {"asset": {"version": "1.1"}, "geometricError": top_error, "root": root})

    # A: one implicit tree, subtreeLevels 2, child subtrees at level 2
    d = f"{out}/A"
    for key, r in T.items():
        if has_content(key):
            write(f"{d}/content/{key[0]}-{key[1]}-0-0.glb", glb(r, key[0]))
    rows = {(l, x, 0): (row(T[(l, x)]), ERR[l]) for (l, x) in T if l < 2}
    content = [{(l, x, 0) for (l, x) in T if l < 2 and has_content((l, x))}]
    write(f"{d}/subtrees/0-0-0.subtree", subtree(2, set(rows), content, [(x, 0) for (l, x) in T if l == 2], rows))
    for (l, x) in [k for k in T if k[0] == 2]:
        local = {(0, 0, 0): (row(T[(2, x)]), ERR[2])}
        for s in (0, 1):
            local[(1, s, 0)] = (row(T[(3, 2 * x + s)]), ERR[3])
        write(f"{d}/subtrees/2-{x}-0.subtree", subtree(2, set(local), [set(local)], [], local))
    write(f"{d}/tileset.json", {"asset": {"version": "1.1"}, "schema": SCHEMA, "geometricError": top_error, "root": {
        "boundingVolume": {"box": row(T[(0, 0)])}, "geometricError": ERR[0], "refine": "REPLACE", "transform": transform,
        "content": {"uri": "content/{level}-{x}-{y}-0.glb"},
        "implicitTiling": {"subdivisionScheme": "QUADTREE", "subtreeLevels": 2, "availableLevels": 4,
                           "subtrees": {"uri": "subtrees/{level}-{x}-{y}.subtree"}}}})

    # S: the whole hierarchy as one single-subtree implicit root (the forest rule when S >= depth)
    d = f"{out}/S"
    for key, r in T.items():
        if has_content(key):
            write(f"{d}/content/{key[0]}-{key[1]}-0-0.glb", glb(r, key[0]))
    rows = {(l, x, 0): (row(T[(l, x)]), ERR[l]) for (l, x) in T}
    content = [{(l, x, 0) for (l, x) in T if has_content((l, x))}]
    write(f"{d}/subtrees/0-0-0.subtree", subtree(4, set(rows), content, [], rows))
    write(f"{d}/tileset.json", {"asset": {"version": "1.1"}, "schema": SCHEMA, "geometricError": top_error, "root": {
        "boundingVolume": {"box": row(T[(0, 0)])}, "geometricError": ERR[0], "refine": "REPLACE", "transform": transform,
        "content": {"uri": "content/{level}-{x}-{y}-0.glb"},
        "implicitTiling": {"subdivisionScheme": "QUADTREE", "subtreeLevels": 4, "availableLevels": 4,
                           "subtrees": {"uri": "subtrees/{level}-{x}-{y}.subtree"}}}})

    # X: today's chunked form (LEVELS 3, stride 2): links at level 2 name external implicit roots
    d = f"{out}/X"
    rows = {(l, x, 0): (row(T[(l, x)]), ERR[l]) for (l, x) in T if l < 3}
    payload = {(l, x, 0) for (l, x) in T if l < 2 and has_content((l, x))}
    links = {(2, x, 0) for (l, x) in T if l == 2}
    for (l, x, _) in payload:
        write(f"{d}/implicit-content/0-0-0-{l}-{x}-0-0.glb", glb(T[(l, x)], l))
    write(f"{d}/subtrees/0-0-0-0-0-0.subtree", subtree(3, set(rows), [payload, links], [], rows))
    for (_, x, _) in links:
        local = {(0, 0, 0): (row(T[(2, x)]), ERR[2])}
        write(f"{d}/implicit-content/2-{x}-0-0-0-0-0.glb", glb(T[(2, x)], 2))
        for s in (0, 1):
            local[(1, s, 0)] = (row(T[(3, 2 * x + s)]), ERR[3])
            write(f"{d}/implicit-content/2-{x}-0-1-{s}-0-0.glb", glb(T[(3, 2 * x + s)], 3))
        write(f"{d}/subtrees/2-{x}-0-0-0-0.subtree", subtree(3, set(local), [set(local)], [], local))
        write(f"{d}/implicit-tileset-0-0-0-2-{x}-0.json", {"asset": {"version": "1.1"}, "schema": SCHEMA, "geometricError": ERR[2], "root": {
            "boundingVolume": {"box": row(T[(2, x)])}, "geometricError": ERR[2], "refine": "REPLACE",
            "content": {"uri": f"implicit-content/2-{x}-0-{{level}}-{{x}}-{{y}}-0.glb"},
            "implicitTiling": {"subdivisionScheme": "QUADTREE", "subtreeLevels": 3, "availableLevels": 2,
                               "subtrees": {"uri": f"subtrees/2-{x}-0-{{level}}-{{x}}-{{y}}.subtree"}}}})
    write(f"{d}/tileset.json", {"asset": {"version": "1.1"}, "schema": SCHEMA, "geometricError": top_error, "root": {
        "boundingVolume": {"box": row(T[(0, 0)])}, "geometricError": ERR[0], "refine": "REPLACE", "transform": transform,
        "contents": [{"uri": "implicit-content/0-0-0-{level}-{x}-{y}-0.glb"}, {"uri": "implicit-tileset-0-0-0-{level}-{x}-{y}.json"}],
        "implicitTiling": {"subdivisionScheme": "QUADTREE", "subtreeLevels": 3, "availableLevels": 3,
                           "subtrees": {"uri": "subtrees/0-0-0-{level}-{x}-{y}.subtree"}}}})

    # F: explicit skeleton + single-subtree implicit roots with their own translation
    d = f"{out}/F"

    def forest(key):
        t = {"boundingVolume": {"box": row(T[key])}, "geometricError": ERR[key[0]], "refine": "REPLACE"}
        height = 0
        probe = [key]
        while any(kids(k) for k in probe):
            probe = [c for k in probe for c in kids(k)]
            height += 1
        if key[0] > 0 and 1 <= height < 2:  # implicit root: height+1 <= S levels
            r = T[key]
            origin = ((r[0] + r[2]) / 2, (r[1] + r[3]) / 2)
            name = f"r{key[0]}-{key[1]}"
            local = {(0, 0, 0): (row(r, origin), ERR[key[0]])}
            write(f"{d}/c/{name}/0-0-0.glb", glb(r, key[0], origin))
            for s, c in enumerate(kids(key)):
                local[(1, s, 0)] = (row(T[c], origin), ERR[c[0]])
                write(f"{d}/c/{name}/1-{s}-0.glb", glb(T[c], c[0], origin))
            write(f"{d}/s/{name}/0-0-0.subtree", subtree(2, set(local), [set(local)], [], local))
            t["boundingVolume"] = {"box": row(r, origin)}
            t["transform"] = [1, 0, 0, 0, 0, 1, 0, 0, 0, 0, 1, 0, origin[0], origin[1], 0, 1]
            t["content"] = {"uri": f"c/{name}/{{level}}-{{x}}-{{y}}.glb"}
            t["implicitTiling"] = {"subdivisionScheme": "QUADTREE", "subtreeLevels": 2, "availableLevels": 2,
                                   "subtrees": {"uri": f"s/{name}/{{level}}-{{x}}-{{y}}.subtree"}}
            return t
        if has_content(key):
            t["content"] = {"uri": f"e/{key[0]}-{key[1]}.glb"}
            write(f"{d}/e/{key[0]}-{key[1]}.glb", glb(T[key], key[0]))
        if kids(key):
            t["children"] = [forest(c) for c in kids(key)]
        return t
    root = forest((0, 0))
    root["transform"] = transform
    write(f"{d}/tileset.json", {"asset": {"version": "1.1"}, "schema": SCHEMA, "geometricError": top_error, "root": root})

    # Views: every leaf, camera target inside the leaf but as far as possible from the
    # derived cell of its level-2 ancestor (the cell Cesium uses for A's placeholder).
    root_box = row(T[(0, 0)])

    def cell(level, x):
        hx, hy = root_box[3], root_box[7]
        w = 2 ** level
        return [-hx + 2 * hx * x / w, -hy, -hx + 2 * hx * (x + 1) / w, -hy + 2 * hy / w]

    def dist(p, c):
        dx = max(c[0] - p[0], 0, p[0] - c[2])
        dy = max(c[1] - p[1], 0, p[1] - c[3])
        return math.hypot(dx, dy)

    views = []
    for (l, x), r in sorted(T.items()):
        if l != 3:
            continue
        c = cell(2, x // 2)
        inset = 80.0
        cands = [(r[0] + inset + (r[2] - r[0] - 2 * inset) * i / 10, r[1] + inset + (r[3] - r[1] - 2 * inset) * j / 10) for i in range(11) for j in range(11)]
        far = max(cands, key=lambda p: dist(p, c))
        views.append({"name": f"leaf-3-{x}", "leaf": [3, x], "target": list(far), "distanceToPlaceholderCell": dist(far, c),
                      "routingAncestor": (1, x // 4) in ROUTING})
        inside = [p for p in cands if dist(p, c) == 0 and c[0] + 80 < p[0] < c[2] - 80 and c[1] + 80 < p[1] < c[3] - 80]
        if inside:
            views.append({"name": f"control-3-{x}", "leaf": [3, x], "target": list(inside[len(inside) // 2]), "distanceToPlaceholderCell": 0.0,
                          "routingAncestor": (1, x // 4) in ROUTING, "control": True})
    write(f"{out}/views.json", {"height": 120.0, "views": views, "leafUris": {
        "E": "e/3-{x}.glb", "A": "content/3-{x}-0-0.glb", "S": "content/3-{x}-0-0.glb", "X": "implicit-content/2-{p}-0-1-{s}-0-0.glb", "F": "c/r2-{p}/1-{s}-0.glb"}})


if __name__ == '__main__':
    main(sys.argv[1])
