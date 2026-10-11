"""PO fixture generator for the P-IMPL forest checker (stdlib only; imports nothing from `src/`).

It builds small synthetic producer hierarchies H, writes each one twice:
  * `<case>-explicit.3tz`: the `--explicit` form of H (the A-4 reference);
  * `<case>.3tz`: a bounded implicit forest written by this file's own v3 writer (design §4.1:
    S = 4, documents of <= 16,384 value nodes / 2 MiB / 12 skeleton levels, link tiles, single-subtree
    implicit roots with rows, root-node splice rebase, no retained sources, schema placement);
and then derives negative controls, each a forest that violates one named rule.

The writer here is PO's model of the contract, derived from the design's
evidence/v2/forest_model.py and evidence/v3/rebase_model.py. It is not the production writer.

Usage: python3 -I make_forest_fixtures.py OUT_DIR   -> OUT_DIR/*.3tz and OUT_DIR/manifest.json
"""
import copy, hashlib, json, math, os, pathlib, struct, sys, zipfile

DOC_NODES = 16384
DOC_BYTES = 2 * 1024 * 1024
DOC_LEVELS = 12
MAX_LEVELS = 32
SCHEMA = {"id": "rustyTilesImplicit", "classes": {"rustyTile": {"properties": {
    "boundingBox": {"type": "SCALAR", "componentType": "FLOAT64", "array": True, "count": 12, "semantic": "TILE_BOUNDING_BOX"},
    "geometricError": {"type": "SCALAR", "componentType": "FLOAT64", "semantic": "TILE_GEOMETRIC_ERROR"},
    "extras": {"type": "STRING"}}}}}
VECTOR_HEADER = {"extensions": {"3DTILES_content_gltf_vector": {"vector": True}}}
ENU = [-0.21473532716706323, 0.9766722783341679, 0.0, 0, -0.6516189130180094, -0.1432677097314857, 0.744894056591622, 0,
       0.72751737536892, 0.15995506894700287, 0.6671827669045997, 0, 4641723.879449041, 1025649.0045262137, 4238154.446882591, 1]


def dumps(v):
    return json.dumps(v, separators=(',', ':')).encode()


def nodes_depth(v, d=0):
    if isinstance(v, (dict, list)):
        n, m = 1, d
        for x in (v.values() if isinstance(v, dict) else v):
            a, b = nodes_depth(x, d + 1); n += a; m = max(m, b)
        return n, m
    return 1, d


class LCG:
    def __init__(self, seed):
        self.s = seed & 0xFFFFFFFFFFFF

    def next(self):
        self.s = (self.s * 25214903917 + 11) & 0xFFFFFFFFFFFF
        return self.s >> 16

    def unit(self):
        return self.next() / 2 ** 32


# ------------------------------------------------------------------ payloads
def glb(seed, translation=None, align=8, sorted_keys=True):
    rng = LCG(seed)
    pts = [(rng.unit() * 10 - 5, rng.unit() * 2, rng.unit() * 10 - 5) for _ in range(3)]
    pos = b''.join(struct.pack('<3f', *p) for p in pts)
    lo = [min(p[i] for p in pts) for i in range(3)]
    hi = [max(p[i] for p in pts) for i in range(3)]
    lo = [struct.unpack('<f', struct.pack('<f', v))[0] for v in lo]
    hi = [struct.unpack('<f', struct.pack('<f', v))[0] for v in hi]
    node = {"mesh": 0}
    if translation is not None:
        node["translation"] = translation
    if sorted_keys:
        doc = {"accessors": [{"bufferView": 0, "componentType": 5126, "count": 3, "max": hi, "min": lo, "type": "VEC3"}],
               "asset": {"generator": "po-fixture", "version": "2.0"}, "bufferViews": [{"buffer": 0, "byteLength": 36}],
               "buffers": [{"byteLength": 36}], "meshes": [{"primitives": [{"attributes": {"POSITION": 0}, "mode": 4}]}],
               "nodes": [node], "scene": 0, "scenes": [{"nodes": [0]}]}
    else:   # meshopt-like raw key order: not sorted
        doc = {"asset": {"version": "2.0", "generator": "po-fixture"}, "scene": 0, "scenes": [{"nodes": [0]}], "nodes": [node],
               "meshes": [{"primitives": [{"mode": 4, "attributes": {"POSITION": 0}}]}],
               "accessors": [{"type": "VEC3", "componentType": 5126, "count": 3, "bufferView": 0, "min": lo, "max": hi}],
               "bufferViews": [{"byteLength": 36, "buffer": 0}], "buffers": [{"byteLength": 36}]}
    js = dumps(doc)
    js += b' ' * (-len(js) % align)
    binary = pos + b'\0' * (-len(pos) % 4)
    return (b'glTF' + struct.pack('<II', 2, 12 + 8 + len(js) + 8 + len(binary)) + struct.pack('<I4s', len(js), b'JSON') + js
            + struct.pack('<I4s', len(binary), b'BIN\0') + binary)


def b3dm(inner):
    ft = b'{"BATCH_LENGTH":0}'
    ft += b' ' * (-(28 + len(ft)) % 8)
    return b'b3dm' + struct.pack('<6I', 1, 28 + len(ft) + len(inner), len(ft), 0, 0, 0) + ft + inner


def skip_ws(s, i):
    while s[i:i + 1] in (b' ', b'\t', b'\r', b'\n'):
        i += 1
    return i


def value_end(s, i):
    c = s[i:i + 1]
    if c == b'"':
        i += 1
        while True:
            if s[i:i + 1] == b'\\':
                i += 2
            elif s[i:i + 1] == b'"':
                return i + 1
            else:
                i += 1
    if c in (b'{', b'['):
        depth, inside = 0, False
        while True:
            ch = s[i:i + 1]
            if inside:
                if ch == b'\\':
                    i += 2; continue
                if ch == b'"':
                    inside = False
            elif ch == b'"':
                inside = True
            elif ch in (b'{', b'['):
                depth += 1
            elif ch in (b'}', b']'):
                depth -= 1
                if depth == 0:
                    return i + 1
            i += 1
    j = i
    while s[j:j + 1] not in (b',', b'}', b']', b''):
        j += 1
    return j


def root_node_span(js):
    doc = json.loads(js)
    roots = sorted({n for sc in doc['scenes'] for n in sc['nodes']})
    assert len(roots) == 1
    k = roots[0]
    i = skip_ws(js, 0) + 1
    while True:
        i = skip_ws(js, i)
        kend = value_end(js, i)
        key = json.loads(js[i:kend])
        i = skip_ws(js, skip_ws(js, kend) + 1)
        vend = value_end(js, i)
        if key == 'nodes':
            j, n = skip_ws(js, i + 1), 0
            while True:
                e = value_end(js, j)
                if n == k:
                    return j, e
                n += 1
                j = skip_ws(js, skip_ws(js, e) + 1)
        i = skip_ws(js, vend)
        if js[i:i + 1] == b',':
            i += 1


def split_container(data):
    if data[:4] == b'b3dm':
        n = 28 + sum(struct.unpack_from('<4I', data, 12))
        return data[:n], data[n:]
    return None, data


def join_container(prefix, glb_bytes):
    if prefix is None:
        return glb_bytes
    return prefix[:8] + struct.pack('<I', len(prefix) + len(glb_bytes)) + prefix[12:] + glb_bytes


def rebase(raw, delta, tw=None, key=None):
    """v3 rebase: replace only the scene-root node object's text; returns (published, record)."""
    tw = tw or {}
    prefix, g = split_container(raw)
    jl = struct.unpack_from('<I', g, 12)[0]
    js = g[20:20 + jl].rstrip(b' ')
    rest = g[20 + jl:]
    a, b = root_node_span(js)
    node_text = js[a:b]
    node = json.loads(node_text)
    t = node.get('translation', [0.0, 0.0, 0.0])
    d = [delta[0], delta[2], -delta[1]]
    node['translation'] = [t[i] + d[i] for i in range(3)]
    if tw.get('payload_translation_ulp') == key:
        node['translation'][0] = math.nextafter(node['translation'][0], math.inf)
    if tw.get('payload_reserialize') == key:   # today's whole-document re-serialization (not a splice)
        doc = json.loads(js)
        doc['nodes'][sorted({n for sc in doc['scenes'] for n in sc['nodes']})[0]] = node
        new_js = json.dumps(doc, sort_keys=True, separators=(',', ':')).encode()
    else:
        new_js = js[:a] + dumps(node) + js[b:]
    align = 8 if jl % 8 == 0 else 4
    new_js += b' ' * (-len(new_js) % align)
    if tw.get('payload_bin') == key:
        rest = rest[:-1] + bytes([rest[-1] ^ 0x01])
    out = b'glTF' + struct.pack('<II', 2, 20 + len(new_js) + len(rest)) + struct.pack('<I4s', len(new_js), b'JSON') + new_js + rest
    out = join_container(prefix, out)
    return out, {'rootNode': node_text.decode(), 'jsonLength': jl}


# ------------------------------------------------------------------ hierarchy
class N:
    __slots__ = ('tile', 'children', 'slot', 'h', 'pid', 'payloads')

    def __init__(self, tile, slot=0):
        self.tile, self.slot, self.children, self.payloads = tile, slot, [], {}


def box(cx, cy, cz, hx, hy, hz):
    return [cx, cy, cz, hx, 0.0, 0.0, 0.0, hy, 0.0, 0.0, 0.0, hz]


def translation(t):
    return [1, 0, 0, 0, 0, 1, 0, 0, 0, 0, 1, 0, t[0], t[1], t[2], 1]


def number(root):
    pid = [0]

    def walk(n):
        n.pid = pid[0]; pid[0] += 1
        for c in n.children:
            walk(c)
        n.h = 1 + max((c.h for c in n.children), default=-1)
    walk(root)
    return pid[0]


def add_content(n, data, suffix, header=None, name=None):
    uri = f"t/{name or hashlib.sha256(data).hexdigest()}.{suffix}"
    n.payloads[uri] = data
    c = dict(header or {}); c['uri'] = uri
    if 'contents' in n.tile:
        n.tile['contents'].append(c)
    elif 'content' in n.tile:
        n.tile['contents'] = [n.tile.pop('content'), c]
    else:
        n.tile['content'] = c


def vector_tile(rng, depth, extras_extra=None, slot=None, chain=False):
    """A vector-shaped producer tile: translation child transform, extras like the vector producer's."""
    t = {'boundingVolume': {'box': box(rng.unit() * 0.3 - 0.1, rng.unit() * 0.3 - 0.1, 0.0, 400.0 / (depth + 1) + rng.unit(),
                                       300.0 / (depth + 1) + rng.unit(), 2.0 + rng.unit())},
         'geometricError': 0.0 if depth > 30 else 1000.0 / 2 ** depth + rng.unit() * 1e-3,
         'refine': 'REPLACE',
         'transform': translation([rng.unit() * 200 - 100 + 0.1, rng.unit() * 200 - 100 + 0.2, rng.unit() * 0.1])}
    ex = {'buildPath': str(slot or 0) * (depth % 7), 'primitives': 1 + depth % 3, 'vertices': 3, 'toleranceMetres': 0.0}
    if extras_extra:
        ex.update(extras_extra)
    t['extras'] = ex
    return t


def content_vector(n, rng, seed, header=VECTOR_HEADER, align=8, sorted_keys=True, translate=None):
    data = glb(seed, translate, align, sorted_keys)
    n.tile.setdefault('extras', {})
    n.tile['extras']['encodedBytes'] = len(data)
    n.tile['extras']['uncompressedBytes'] = len(data)
    add_content(n, data, 'glb', header)


def kd(rng, depth, levels, seed, extras_fn=None, contentless=(), chains=None, slot=0):
    """Binary k-d tree (child slot = array index, as vector), `levels` levels below `depth`."""
    n = N(vector_tile(rng, depth, extras_fn(depth) if extras_fn else None, slot), slot)
    key = (depth, seed[0])
    seed[0] += 1
    if key not in contentless:
        content_vector(n, rng, seed[0] * 7919 + depth, align=8 if seed[0] % 3 else 4, sorted_keys=bool(seed[0] % 2),
                       translate=[0.5, 0.25, -0.125] if seed[0] % 4 == 1 else None)
    if levels > 0:
        n.children = [kd(rng, depth + 1, levels - 1, seed, extras_fn, contentless, chains, s) for s in (0, 1)]
    elif chains:
        # an LOD chain above this leaf: single children
        top = n
        for k in range(chains):
            c = N(vector_tile(rng, depth + 1 + k), 0)
            content_vector(c, rng, seed[0] * 104729 + k)
            seed[0] += 1
            top.children = [c]
            top = c
    return n


def point_tile(rng, depth, slot):
    half = 64.0 / 2 ** depth
    t = {'boundingVolume': {'box': box(0.0, 0.0, 0.0, half + 1e-3, half + 2e-3, half / 2 + 3e-3)},
         'geometricError': half / 8 if depth < 5 else 0.0, 'refine': 'REPLACE'}
    if depth:
        o = [((slot >> i) & 1) * half - half / 2 + 0.01 * (i + 1) for i in range(3)]
        t['transform'] = translation(o)
        t['extras'] = {'implicitChildIndex': slot}
    return t


# ------------------------------------------------------------------ positive cases
def case_quad_single():
    rng = LCG(1)
    seed = [0]
    root = kd(rng, 0, 2, seed, contentless={(1, 1)})
    root.tile['transform'] = ENU
    return root, 'QUADTREE', {'asset': {'version': '1.1'}, 'extensionsUsed': ['3DTILES_content_gltf_vector'], 'geometricError': 5000.0}, True


def case_m2_single():
    buf = bytes(range(48))
    g = {"asset": {"version": "2.0"}, "scene": 0, "scenes": [{"nodes": [0]}], "nodes": [{"mesh": 0}],
         "meshes": [{"primitives": [{"attributes": {"POSITION": 0}}]}],
         "accessors": [{"bufferView": 0, "componentType": 5126, "count": 3, "type": "VEC3", "min": [0, 0, 0], "max": [1, 1, 1]}],
         "bufferViews": [{"buffer": 0, "byteLength": 36}], "buffers": [{"byteLength": 48, "uri": "m2.bin"}]}
    root = N({'boundingVolume': {'box': box(0.5, 0.5, 0.5, 0.5, 0.5, 0.5)}, 'geometricError': 0.0, 'refine': 'ADD', 'transform': ENU})
    add_content(root, json.dumps(g, indent=1).encode(), 'gltf', name='m2')
    root.payloads['t/m2.bin'] = buf
    return root, 'OCTREE', {'asset': {'version': '1.1'}, 'geometricError': 2.0}, False


def case_oct_mixed():
    """OCTREE, L = 6: sparse 8-way slots via implicitChildIndex, explicit leaves, implicit roots of height 1..3,
    a contentless root, a root with two content slots (glb + b3dm), b3dm content, 4- and 8-aligned JSON chunks."""
    rng = LCG(2)
    seed = [100]

    def node(depth, slot, height, mode):
        n = N(point_tile(rng, depth, slot), slot)
        seed[0] += 1
        s = seed[0]
        if mode == 'contentless':
            pass
        elif mode == 'b3dm':
            add_content(n, b3dm(glb(s, [1.0, 2.0, 3.0] if s % 2 else None, 4)), 'b3dm')
        elif mode == 'two':
            add_content(n, glb(s, None, 8, False), 'glb')
            add_content(n, b3dm(glb(s + 7, [0.1, 0.2, 0.3], 4)), 'b3dm')
        else:
            add_content(n, glb(s, [0.25, 0.5, 0.75] if s % 3 == 0 else None, 8 if s % 2 else 4, bool(s % 5)), 'glb')
        if height > 0:
            slots = [0, 3, 5, 6, 7][:2 + height % 3]
            n.children = [node(depth + 1, sl, height - 1 if k == 0 else max(0, height - 1 - k % 2), mode) for k, sl in enumerate(slots)]
        return n
    root = N(point_tile(rng, 0, 0))
    add_content(root, glb(1, None, 8), 'glb')
    root.tile['transform'] = translation([1000.5, -2000.25, 30.125])
    a = N(point_tile(rng, 1, 1), 1); add_content(a, glb(2, None, 8), 'glb')
    a.children = [node(2, 0, 3, 'glb'), node(2, 2, 2, 'two'), node(2, 4, 1, 'contentless'), node(2, 6, 0, 'glb')]
    deep = N(point_tile(rng, 2, 7), 7); add_content(deep, glb(3, None, 4), 'glb')
    deep.children = [node(3, 1, 3, 'b3dm'), node(3, 2, 0, 'glb')]
    a.children.append(deep)
    leaf = N(point_tile(rng, 1, 4), 4); add_content(leaf, glb(4, [9.0, 8.0, 7.0], 8), 'glb')
    small = node(1, 6, 2, 'glb')
    root.children = [a, leaf, small]
    a.children.sort(key=lambda c: c.slot)
    return root, 'OCTREE', {'asset': {'version': '1.1'}, 'geometricError': 200.0}, False


def case_quad_wide():
    """Binary k-d tree, 11 levels, heavy skeleton extras: documents split by the node budget, nested links."""
    rng = LCG(3)
    seed = [1000]
    heavy = lambda depth: {'samples': list(range(1500))} if depth <= 7 else None
    root = kd(rng, 0, 11, seed, heavy, contentless={(3, 1003)})
    root.tile['transform'] = ENU
    return root, 'QUADTREE', {'asset': {'version': '1.1'}, 'extensionsUsed': ['3DTILES_content_gltf_vector'], 'geometricError': 5000.0}, True


def case_quad_bytes():
    """Documents split by the 2 MiB byte budget: long string extras on skeleton tiles."""
    rng = LCG(4)
    seed = [5000]
    pad = lambda depth: {'note': chr(97 + depth) * 300000} if depth <= 3 else None
    root = kd(rng, 0, 7, seed, pad)
    root.tile['transform'] = ENU
    return root, 'QUADTREE', {'asset': {'version': '1.1'}, 'extensionsUsed': ['3DTILES_content_gltf_vector'], 'geometricError': 5000.0}, True


def chain_case(levels):
    """An LOD chain (single children) above a full binary implicit root of height 3: L = levels."""
    rng = LCG(5 + levels)
    seed = [9000]
    chain_len = levels - 4
    root = None
    parent = None
    for d in range(chain_len):
        n = N(vector_tile(rng, d), 0)
        content_vector(n, rng, 9000 + d)
        if parent is None:
            root = n
        else:
            parent.children = [n]
        parent = n
    sub = kd(rng, chain_len, 3, seed)
    parent.children = [sub]
    root.tile['transform'] = ENU
    return root, 'QUADTREE', {'asset': {'version': '1.1'}, 'extensionsUsed': ['3DTILES_content_gltf_vector'], 'geometricError': 5000.0}, True


POSITIVES = {
    'quad-single': case_quad_single,
    'm2-single': case_m2_single,
    'oct-mixed': case_oct_mixed,
    'quad-wide': case_quad_wide,
    'quad-bytes': case_quad_bytes,
    'quad-chain32': lambda: chain_case(32),
}


# ------------------------------------------------------------------ writers
def write_explicit(root, top):
    members = {}

    def tile(n):
        t = copy.deepcopy(n.tile)
        members.update(n.payloads)
        if n.children:
            t['children'] = [tile(c) for c in n.children]
        return t
    members['tileset.json'] = dumps(dict(top, root=tile(root)))
    return members


def subtree_bytes(dims, levels, rows, layers, tw_child=None, drop_rows=()):
    b = 2 ** dims
    n_tiles = (b ** levels - 1) // (b - 1)

    def morton(xyz, level):
        v = 0
        for bit in range(level):
            for i, c in enumerate(xyz):
                v |= ((c >> bit) & 1) << (bit * dims + i)
        return v

    def index(k):
        l, xyz = k
        return (b ** l - 1) // (b - 1) + morton(xyz, l)
    binary, views = bytearray(), []

    def view(data):
        while len(binary) % 8:
            binary.append(0)
        views.append({"buffer": 0, "byteOffset": len(binary), "byteLength": len(data)})
        binary.extend(data)
        return len(views) - 1

    def avail(n, ones):
        ones = sorted(set(ones))
        if not ones:
            return {"constant": 0}
        if len(ones) == n:
            return {"constant": 1}
        raw = bytearray((n + 7) // 8)
        for i in ones:
            raw[i >> 3] |= 1 << (i & 7)
        return {"bitstream": view(bytes(raw)), "availableCount": len(ones)}
    order = sorted(rows, key=index)
    doc = {"tileAvailability": avail(n_tiles, [index(k) for k in order])}
    if layers:
        doc["contentAvailability"] = [avail(n_tiles, [index(k) for k in order if k in layer]) for layer in layers]
    doc["childSubtreeAvailability"] = tw_child or {"constant": 0}
    order = [k for k in order if k not in drop_rows]   # negative control: available tiles without a row
    boxes = b''.join(struct.pack('<12d', *rows[k][0]) for k in order)
    errors = b''.join(struct.pack('<d', rows[k][1]) for k in order)
    strings, offsets = bytearray(), bytearray(struct.pack('<I', 0))
    for k in order:
        strings += rows[k][2]
        offsets += struct.pack('<I', len(strings))
    doc["propertyTables"] = [{"class": "rustyTile", "count": len(order), "properties": {
        "boundingBox": {"values": view(boxes)}, "geometricError": {"values": view(errors)},
        "extras": {"values": view(bytes(strings)), "stringOffsets": view(bytes(offsets))}}}]
    doc["tileMetadata"] = 0
    while len(binary) % 8:
        binary.append(0)
    doc["buffers"] = [{"byteLength": len(binary)}]
    doc["bufferViews"] = views
    j = dumps(doc)
    j += b' ' * (-len(j) % 8)
    return b'subt' + struct.pack('<IQQ', 1, len(j), len(binary)) + j + bytes(binary)


def tr_of(tile):
    t = tile.get('transform')
    return (0.0, 0.0, 0.0) if t is None else (float(t[12]), float(t[13]), float(t[14]))


def write_forest(root, scheme, top, vector, tw=None):
    """PO's v3 forest writer. `tw` holds the negative-control tweaks (empty for positives)."""
    tw = tw or {}
    S = tw.get('S', 4)
    dims = 3 if scheme == 'OCTREE' else 2
    zt = '-{z}' if dims == 3 else ''
    total = number(root)
    if root.h + 1 > MAX_LEVELS and not tw.get('allow_levels'):
        raise ValueError(f'hierarchy has {root.h + 1} levels; implicit output admits at most {MAX_LEVELS}')
    members, payload_map = {}, {}
    roots, root_objs = [], {}

    def role(n, parent_role):
        if parent_role is None:
            return 'root' if n.h < S else 'skeleton'
        if parent_role == 'skeleton':
            if n.h >= S:
                return 'skeleton'
            if n.h == 0 and not (tw.get('leaf_as_root') and n.pid == tw['leaf_as_root']):
                return 'leaf'
            return 'root'
        return 'implicit'

    def explicit_obj(n):
        t = copy.deepcopy(n.tile)
        ex = t.get('extras')
        if isinstance(ex, dict) and 'implicitChildIndex' in ex and not tw.get('tile_child_index'):
            ex.pop('implicitChildIndex')
            if not ex:
                t.pop('extras')
        members.update(n.payloads)
        return t

    def implicit_root(r):
        index = len(roots)
        roots.append(r)
        n_id = r.pid
        if tw.get('prefix_dup') and index == tw['prefix_dup'][0]:
            n_id = roots[tw['prefix_dup'][1]].pid
        if tw.get('prefix_shift') == index:
            n_id = r.pid + 1000
        rid = f'r{n_id}'
        levels = r.h + 1
        slots, headers, tiles = {}, {}, []
        stack = [(r, 0, (0,) * dims, (0.0, 0.0, 0.0))]
        while stack:
            n, lvl, xyz, delta = stack.pop()
            if tw.get('drop_tile') == (index, n.pid):
                continue
            tiles.append((n, lvl, xyz, delta))
            for i, c in enumerate(n.tile.get('contents') or ([n.tile['content']] if 'content' in n.tile else [])):
                suffix = c['uri'].rsplit('.', 1)[1]
                if tw.get('json_template') == index:
                    suffix = 'json'
                slots[(i, suffix)] = None
                headers[(i, suffix)] = {k: v for k, v in c.items() if k != 'uri'}
            for ch in n.children:
                t = tr_of(ch.tile)
                stack.append((ch, lvl + 1, tuple((xyz[k] << 1) | ((ch.slot >> k) & 1) for k in range(dims)),
                              tuple(delta[k] + t[k] for k in range(3))))
        slot_list = sorted(slots)
        if tw.get('slots_unsorted') == index:
            slot_list = list(reversed(slot_list))
        rows, layers = {}, [set() for _ in slot_list]
        for k_tile, (n, lvl, xyz, delta) in enumerate(sorted(tiles, key=lambda x: x[0].pid)):
            hb = n.tile['boundingVolume']['box']
            b = list(hb) if n is r else [hb[i] + delta[i] for i in range(3)] + list(hb[3:])
            err = n.tile['geometricError']
            if tw.get('row_ulp') == (index, k_tile):
                b[0] = math.nextafter(b[0], math.inf)
            if tw.get('error_change') == (index, k_tile):
                err = err * 2 + 1.0
            ex = copy.deepcopy(n.tile.get('extras', {}))
            if not tw.get('row_child_index') == index:
                ex.pop('implicitChildIndex', None)
            elif 'implicitChildIndex' not in ex:
                ex['implicitChildIndex'] = n.slot
            written = 0
            cs = n.tile.get('contents') or ([n.tile['content']] if 'content' in n.tile else [])
            for i, c in enumerate(cs):
                suffix = c['uri'].rsplit('.', 1)[1]
                tsuffix = 'json' if tw.get('json_template') == index else suffix
                li = slot_list.index((i, tsuffix))
                layers[li].add((lvl, xyz))
                name = f'implicit-content/{rid}-{lvl}-' + '-'.join(map(str, xyz)) + f'-{i}.{tsuffix}'
                raw = n.payloads[c['uri']]
                if n is r or suffix == 'gltf':
                    pub, rec = raw, None
                    if suffix == 'gltf':   # resources travel with the .gltf
                        for res, data in n.payloads.items():
                            if res != c['uri']:
                                members['implicit-content/' + res.split('/', 1)[1]] = data
                    else:
                        jl = struct.unpack_from('<I', split_container(raw)[1], 12)[0]
                        s, e = root_node_span(split_container(raw)[1][20:20 + jl].rstrip(b' '))
                        rec = {'rootNode': split_container(raw)[1][20:20 + jl].rstrip(b' ')[s:e].decode(), 'jsonLength': jl}
                else:
                    pub, rec = rebase(raw, delta, tw, (index, k_tile))
                written += len(pub)
                members[name] = pub
                sha = hashlib.sha256(raw).hexdigest()
                if rec is not None and vector:
                    payload_map[sha] = {'member': name, 'delta': list(delta) if n is not r else [0.0, 0.0, 0.0], **rec}
                if tw.get('retain_sources') and n is not r:
                    members[c['uri']] = raw
            if 'encodedBytes' in ex and cs:
                ex['encodedBytes'] = written
            if tw.get('row_extras_change') == (index, k_tile):
                ex['tampered'] = True
            rows[(lvl, xyz)] = (b, err, dumps(ex))
        drop_rows = {max(rows, key=lambda k: (k[0], k[1]))} if tw.get('drop_row') == index else set()
        if tw.get('row0_ulp') == index:
            b, err, ex = rows[(0, (0,) * dims)]
            rows[(0, (0,) * dims)] = ([math.nextafter(b[0], math.inf)] + b[1:], err, ex)
        if tw.get('extra_layer') == index:
            layers.append(set(rows))
        if tw.get('orphan_bit') == index:
            far = (levels - 1, tuple(2 ** (levels - 1) - 1 for _ in range(dims))) if levels > 1 else None
            if far and far not in rows and levels > 1:
                rows[far] = (list(rows[(0, (0,) * dims)][0]), 0.0, b'{}')
        child = {"constant": 1} if tw.get('child_subtree') == index else None
        sl = levels + 1 if tw.get('levels_mismatch') == index else levels
        data = subtree_bytes(dims, sl, rows, [l for l in layers] if layers else [], child, drop_rows)
        members[f'subtrees/{rid}-0-0-0' + ('-0' if dims == 3 else '') + '.subtree'] = data
        if tw.get('two_subtrees') == index:
            members[f'subtrees/{rid}-1-1-0' + ('-0' if dims == 3 else '') + '.subtree'] = data
        tile = {k: copy.deepcopy(v) for k, v in r.tile.items() if k not in ('content', 'contents', 'extras')}
        if tw.get('drop_root_transform') == index:
            tile.pop('transform', None)
        if tw.get('refine_root') == index:
            tile['refine'] = 'ADD' if tile.get('refine') != 'ADD' else 'REPLACE'
        tile['implicitTiling'] = {'subdivisionScheme': scheme, 'subtreeLevels': sl, 'availableLevels': levels,
                                  'subtrees': {'uri': f'subtrees/{rid}-{{level}}-{{x}}-{{y}}{zt}.subtree'}}
        templates = [dict(headers[(i, s)], uri=f'implicit-content/{rid}-{{level}}-{{x}}-{{y}}{zt}-{i}.{s}') for i, s in slot_list]
        if len(templates) == 1:
            tile['content'] = templates[0]
        elif templates:
            tile['contents'] = templates
        return tile

    # roles and the per-tile JSON objects of the skeleton
    roles = {}

    def classify(n, parent_role):
        roles[n.pid] = role(n, parent_role)
        for c in n.children:
            classify(c, roles[n.pid])
    classify(root, None)

    def unit(n):
        """The skeleton tile n with its explicit-leaf and implicit-root children; skeleton children listed apart."""
        obj = explicit_obj(n)
        kids, skel = [], []
        for c in n.children:
            r = roles[c.pid]
            if r == 'skeleton':
                skel.append(c)
            elif r == 'leaf':
                kids.append(explicit_obj(c))
            else:
                if c.pid not in root_objs:
                    root_objs[c.pid] = implicit_root(c)
                kids.append(root_objs[c.pid])
        return obj, kids, skel

    any_root = any(r == 'root' for r in roles.values())
    schema = copy.deepcopy(SCHEMA)
    if tw.get('schema_id'):
        schema['id'] = tw['schema_id']
    docs = []
    doc_levels = tw.get('doc_levels', DOC_LEVELS)
    top_doc = dict(top)
    if vector:
        top_doc['asset'] = dict(top['asset'])
    if roles[root.pid] == 'root':
        robj = implicit_root(root)
        docs.append(['tileset.json', dict(top_doc, root=robj), True])
    else:
        queue, counter = [(root, 'tileset.json')], [0]

        def link_for(c, lname):
            link = {'boundingVolume': copy.deepcopy(c.tile['boundingVolume']), 'geometricError': c.tile['geometricError'],
                    'refine': c.tile.get('refine', 'REPLACE'), 'content': {'uri': lname}}
            if 'transform' in c.tile:
                link['transform'] = copy.deepcopy(c.tile['transform'])
            return link

        def link_cost(c):
            l = link_for(c, 'tileset-d0000000.json')
            return nodes_depth(l)[0], len(dumps(l)) + 1
        while queue:
            droot, name = queue.pop(0)
            obj, kids, skel = unit(droot)
            if name != 'tileset.json' and not tw.get('doc_root_transform'):
                obj.pop('transform', None)
            used_n = nodes_depth(dict(obj, children=kids))[0] + 64 + sum(link_cost(c)[0] for c in skel)
            used_b = len(dumps(dict(obj, children=kids))) + 4096 + sum(link_cost(c)[1] for c in skel)
            frontier = [(obj, kids, skel, 0)]
            while frontier:
                o, k, sk, d = frontier.pop(0)
                children = list(k)
                for c in sk:
                    cobj, ckids, cskel = unit(c)
                    ln, lb = link_cost(c)
                    probe = dict(cobj, children=ckids)
                    cn = nodes_depth(probe)[0] + sum(link_cost(x)[0] for x in cskel) - ln
                    cb = len(dumps(probe)) + sum(link_cost(x)[1] for x in cskel) - lb
                    if d + 1 > doc_levels or used_n + cn > DOC_NODES or used_b + cb > DOC_BYTES:
                        counter[0] += 1
                        lname = f'tileset-d{counter[0]}.json'
                        children.append(link_for(c, lname))
                        queue.append((c, lname))
                        continue
                    used_n += cn; used_b += cb
                    children.append(cobj)
                    frontier.append((cobj, ckids, cskel, d + 1))
                if children:
                    o['children'] = children
            if name == 'tileset.json':
                doc = dict(top_doc, root=obj)
            else:
                doc = {'asset': copy.deepcopy(top_doc['asset']), 'geometricError': droot.tile['geometricError'], 'root': obj}
                if 'extensionsUsed' in top:
                    doc['extensionsUsed'] = list(top['extensionsUsed'])
            docs.append([name, doc, any(_holds_root(obj))])
    for name, doc, holds in docs:
        if (name == 'tileset.json' and any_root and not tw.get('no_root_schema')) or (holds and name != 'tileset.json' and not tw.get('no_doc_schema')):
            doc['schema'] = schema
    if vector:
        state = {'format': 'po-fixture-vector-build', 'payloads': dict(sorted(payload_map.items()))}
        if tw.get('reuse_record_wrong'):
            k = sorted(state['payloads'])[tw['reuse_record_wrong'] - 1]
            state['payloads'][k] = dict(state['payloads'][k], rootNode='{"mesh":0,"translation":[1,2,3]}')
        if tw.get('reuse_missing_entry'):
            state['payloads'].pop(sorted(state['payloads'])[0])
        data = dumps(state)
        members['vector-build.json'] = data
        docs[0][1]['asset'] = dict(docs[0][1]['asset'], extras={'vectorBuildStateSha256': hashlib.sha256(data).hexdigest()})
        if tw.get('vector_build_hash'):
            docs[0][1]['asset']['extras']['vectorBuildStateSha256'] = '0' * 64
    for name, doc, _ in docs:
        members[name] = dumps(doc)
    members['conversion.json'] = dumps({'tool': 'po-fixture', 'tiles': total})
    return members, {'tiles': total, 'levels': root.h + 1, 'documents': len(docs), 'implicitRoots': len(roots),
                     'rootPids': [r.pid for r in roots]}


def _holds_root(obj):
    stack = [obj]
    while stack:
        t = stack.pop()
        if 'implicitTiling' in t:
            yield True
            return
        stack.extend(t.get('children', []))


# ------------------------------------------------------------------ member-level mutations
def jload(members, name):
    return json.loads(members[name])


def jstore(members, name, obj):
    members[name] = dumps(obj)


def first_link(members):
    for name in ['tileset.json'] + sorted(n for n in members if n.startswith('tileset-d')):
        if name.endswith('.json') and (name == 'tileset.json' or name.startswith('tileset-d')):
            obj = jload(members, name)
            stack = [(obj['root'], None)]
            while stack:
                t, parent = stack.pop(0)
                if str(t.get('content', {}).get('uri', '')).startswith('tileset-d'):
                    return name, obj, t
                stack.extend((c, t) for c in t.get('children', []))
    raise AssertionError('no link')


def mut_link(field):
    def f(m):
        name, obj, link = first_link(m)
        if field == 'box':
            link['boundingVolume']['box'][3] = link['boundingVolume']['box'][3] * 1.5
        elif field == 'refine':
            link['refine'] = 'ADD'
        elif field == 'children':
            link['children'] = [{'boundingVolume': link['boundingVolume'], 'geometricError': 0.0}]
        elif field == 'twice':
            other = link['content']['uri']
            stack = [obj['root']]
            while stack:
                t = stack.pop(0)
                u = str(t.get('content', {}).get('uri', ''))
                if u.startswith('tileset-d') and u != other:
                    t['content']['uri'] = other
                    break
                stack.extend(t.get('children', []))
            else:
                raise AssertionError('needs two links in one document')
        jstore(m, name, obj)
    return f


def mut_dangling(m):
    name, obj, link = first_link(m)
    target = link['content']['uri']
    m['orphan-' + target] = m.pop(target)


def mut_renumber_from_two(m):
    docs = sorted((n for n in m if n.startswith('tileset-d')), key=lambda n: -int(n[9:-5]))
    for d in docs:
        k = int(d[9:-5])
        _rename_doc(m, d, f'tileset-d{k + 1}.json')


def _rename_doc(m, old, new):
    m[new] = m.pop(old)
    for name in [n for n in m if n == 'tileset.json' or n.startswith('tileset-d')]:
        if old.encode() in m[name]:
            obj = jload(m, name)
            stack = [obj['root']]
            while stack:
                t = stack.pop()
                if t.get('content', {}).get('uri') == old:
                    t['content']['uri'] = new
                stack.extend(t.get('children', []))
            jstore(m, name, obj)


def mut_swap_order(m):
    _rename_doc(m, 'tileset-d1.json', 'tileset-dX.json')
    _rename_doc(m, 'tileset-d2.json', 'tileset-d1.json')
    _rename_doc(m, 'tileset-dX.json', 'tileset-d2.json')


def mut_pad(kind):
    def f(m):
        name = sorted(n for n in m if n.startswith('tileset-d'))[0] if any(n.startswith('tileset-d') for n in m) else 'tileset.json'
        obj = jload(m, name)
        if kind == 'nodes':
            obj['root'].setdefault('extras', {})['pad'] = list(range(DOC_NODES + 1))
        elif kind == 'bytes':
            obj['root'].setdefault('extras', {})['pad'] = 'x' * (DOC_BYTES + 1)
        elif kind == 'depth':
            v = 0
            for _ in range(40):
                v = [v]
            obj['root'].setdefault('extras', {})['pad'] = v
        jstore(m, name, obj)
    return f


def mut_source_list(m):
    obj = jload(m, 'tileset.json')
    obj['extras'] = {'rustyTilesSourceContents': []}
    jstore(m, 'tileset.json', obj)


def mut_old_chunk_doc(m):
    m['implicit-tileset-0-0-0-2-0-0.json'] = dumps({'asset': {'version': '1.1'}, 'geometricError': 1.0, 'root': {
        'boundingVolume': {'box': box(0, 0, 0, 1, 1, 1)}, 'geometricError': 1.0, 'refine': 'REPLACE'}})


def mut_orphan(m):
    m['extra/orphan.glb'] = glb(424242)


def level_of(name):
    return int(name.rsplit('/', 1)[1].split('-')[1])


def mut_missing_content(m):
    m.pop(sorted(n for n in m if n.startswith('implicit-content/') and level_of(n) > 0)[0])


def mut_unavailable_member(m):
    # quad-single is one binary k-d root r0: y is always 0, so (2, 1, 1) is never available
    m['implicit-content/r0-2-1-1-0.glb'] = m[sorted(x for x in m if x.startswith('implicit-content/r0-2-'))[0]]


def mut_corrupt_payload(m):
    n = sorted(x for x in m if x.startswith('implicit-content/') and x.endswith('.glb'))[-1]
    d = bytearray(m[n])
    struct.pack_into('<I', d, 8, len(d) + 4)
    m[n] = bytes(d)


def mut_explicit_payload(m):
    n = sorted(x for x in m if x.startswith('t/') and x.endswith('.glb'))[0]
    m[n] = m[n][:-1] + bytes([m[n][-1] ^ 1])


# ------------------------------------------------------------------ controls
# name -> (positive case, tweaks, member mutation, rules that must fail, needs the A-4 reference)
NEGATIVES = {
    # A-2's ten sensitive controls (design §6)
    'neg-json-template': ('oct-mixed', {'json_template': 1}, None, ['P1.noJsonTemplate'], False),
    'neg-child-subtree-bit': ('oct-mixed', {'child_subtree': 0}, None, ['P2.singleSubtreeRoots'], False),
    'neg-missing-row': ('oct-mixed', {'drop_row': 0}, None, ['P3.rowsEverywhere'], False),
    'neg-row-1ulp': ('oct-mixed', {'row_ulp': (0, 3)}, None, ['A4.rows'], True),
    'neg-row0-1ulp': ('oct-mixed', {'row0_ulp': 2}, None, ['P3.rowsEverywhere'], False),
    'neg-repeated-prefix': ('oct-mixed', {'prefix_dup': (2, 0)}, None, ['P4.namesClosed'], False),
    'neg-unreferenced-member': ('quad-single', {}, mut_orphan, ['P4.namesClosed'], False),
    'neg-retained-source': ('quad-wide', {'retain_sources': True}, None, ['PD11.noRetainedSources', 'P4.namesClosed'], False),
    'neg-doc-node-budget': ('quad-wide', {}, mut_pad('nodes'), ['documents.budget'], False),
    'neg-link-box': ('quad-wide', {}, mut_link('box'), ['P6.linkCoherence'], False),
    'neg-missing-root-schema': ('quad-chain32', {'no_root_schema': True}, None, ['P12.schema'], False),
    # further rules
    'neg-source-list': ('quad-single', {}, mut_source_list, ['PD11.noRetainedSources'], False),
    'neg-doc-schema': ('quad-chain32', {'no_doc_schema': True}, None, ['P12.schema'], False),
    'neg-schema-id': ('oct-mixed', {'schema_id': 'other'}, None, ['P12.schema'], False),
    'neg-skeleton-s3': ('oct-mixed', {'S': 3}, None, ['skeleton.S4', 'A4.roles'], False),
    'neg-skeleton-s5': ('quad-wide', {'S': 5}, None, ['skeleton.S4', 'P2.singleSubtreeRoots', 'A4.roles'], False),
    'neg-leaf-as-root': ('oct-mixed', {'leaf_as_root': 'FIRST_LEAF'}, None, ['skeleton.S4', 'A4.roles'], False),
    'neg-truncated-archive': ('quad-single', {}, 'TRUNCATE', ['archive'], False),
    'neg-levels-mismatch': ('oct-mixed', {'levels_mismatch': 0}, None, ['P2.singleSubtreeRoots'], False),
    'neg-two-subtrees': ('oct-mixed', {'two_subtrees': 0}, None, ['P2.singleSubtreeRoots'], False),
    'neg-orphan-availability': ('quad-single', {'orphan_bit': 0}, None, ['P2.singleSubtreeRoots'], False),
    'neg-l33': ('chain33', {'allow_levels': True}, None, ['P9.levelBound'], False),
    'neg-doc-bytes': ('quad-chain32', {}, mut_pad('bytes'), ['documents.budget'], False),
    'neg-doc-depth': ('quad-chain32', {}, mut_pad('depth'), ['documents.budget'], False),
    'neg-doc-levels': ('quad-chain32', {'doc_levels': 14}, None, ['documents.levels'], False),
    'neg-link-refine': ('quad-wide', {}, mut_link('refine'), ['P6.linkCoherence'], False),
    'neg-link-children': ('quad-wide', {}, mut_link('children'), ['links.wellFormed'], False),
    'neg-link-dangling': ('quad-wide', {}, mut_dangling, ['links.wellFormed'], False),
    'neg-link-twice': ('quad-wide', {}, mut_link('twice'), ['links.wellFormed'], False),
    'neg-doc-root-transform': ('quad-chain32', {'doc_root_transform': True}, None, ['links.wellFormed'], False),
    'neg-doc-numbering': ('quad-wide', {}, mut_renumber_from_two, ['links.wellFormed'], False),
    'neg-link-order': ('quad-wide', {}, mut_swap_order, ['links.creationOrder'], False),
    'neg-old-chunk-document': ('quad-single', {}, mut_old_chunk_doc, ['P1.noJsonTemplate'], False),
    'neg-tile-child-index': ('oct-mixed', {'tile_child_index': True}, None, ['P11.noImplicitChildIndex'], False),
    'neg-row-child-index': ('oct-mixed', {'row_child_index': 0}, None, ['P11.noImplicitChildIndex'], False),
    'neg-extra-layer': ('oct-mixed', {'extra_layer': 0}, None, ['P10.slots'], False),
    'neg-slots-unsorted': ('oct-mixed', {'slots_unsorted': 'TWO_SLOT_ROOT'}, None, ['P10.slots'], False),
    'neg-missing-content': ('oct-mixed', {}, mut_missing_content, ['P4.namesClosed'], False),
    'neg-unavailable-content': ('quad-single', {}, mut_unavailable_member, ['P4.namesClosed'], False),
    'neg-corrupt-payload': ('oct-mixed', {}, mut_corrupt_payload, ['payloads.wellFormed'], False),
    'neg-vector-build-hash': ('quad-single', {'vector_build_hash': True}, None, ['P4.namesClosed'], False),
    'neg-reuse-record': ('quad-wide', {'reuse_record_wrong': 3}, None, ['vectorBuild.reuseMap'], False),
    # A-4 equivalence (need the --explicit reference)
    'neg-reuse-missing-entry': ('quad-wide', {'reuse_missing_entry': True}, None, ['vectorBuild.reuseMap'], True),
    'neg-error-changed': ('quad-wide', {'error_change': (5, 4)}, None, ['A4.rows'], True),
    'neg-row-extras': ('oct-mixed', {'row_extras_change': (0, 2)}, None, ['A4.extras'], True),
    'neg-tile-dropped': ('oct-mixed', {'drop_tile': 'LAST_LEAF_OF_ROOT0'}, None, ['A4.tree'], True),
    'neg-payload-bin': ('quad-wide', {'payload_bin': (3, 2)}, None, ['A4.payloads'], True),
    'neg-payload-translation': ('oct-mixed', {'payload_translation_ulp': (0, 1)}, None, ['A4.payloads'], True),
    'neg-payload-reserialized': ('oct-mixed', {'payload_reserialize': 'UNSORTED'}, None, ['A4.payloads'], True),
    'neg-explicit-payload': ('oct-mixed', {}, mut_explicit_payload, ['A4.payloads'], True),
    'neg-prefix-not-preorder': ('oct-mixed', {'prefix_shift': 1}, None, ['A4.names'], True),
    'neg-root-refine': ('oct-mixed', {'refine_root': 0}, None, ['A4.explicitTiles'], True),
    'neg-root-transform-dropped': ('oct-mixed', {'drop_root_transform': 0}, None, ['A4.explicitTiles'], True),
}


def resolve_tweaks(tw, root, scheme, top, vector):
    """Turn symbolic tweak targets into concrete ones, using the writer's own root order (a dry run)."""
    tw = dict(tw)
    number(root)
    S = tw.get('S', 4)
    order = write_forest(root, scheme, top, vector, {'S': S, 'allow_levels': tw.get('allow_levels')})[1]['rootPids']
    by_pid = {n.pid: n for n in _walk(root)}
    if tw.get('leaf_as_root') == 'FIRST_LEAF':
        tw['leaf_as_root'] = min(c.pid for c in _walk(root) if c.h == 0 and _parent_h(root, c) >= S)
    if tw.get('drop_tile') == 'LAST_LEAF_OF_ROOT0':
        r = by_pid[order[0]]
        leaves = [n for n in _walk(r) if not n.children and n is not r]
        tw['drop_tile'] = (0, max(n.pid for n in leaves))
    if tw.get('slots_unsorted') == 'TWO_SLOT_ROOT':
        tw['slots_unsorted'] = next(i for i, pid in enumerate(order) if 'contents' in by_pid[pid].tile)
    if tw.get('payload_reserialize') == 'UNSORTED':
        # a rebased payload whose raw JSON keys are unsorted (meshopt-like): a sorted re-serialization moves bytes
        for i, pid in enumerate(order):
            tiles = sorted(_walk(by_pid[pid]), key=lambda n: n.pid)
            for k, n in enumerate(tiles):
                if k == 0 or not n.payloads:
                    continue
                g = split_container(next(iter(n.payloads.values())))[1]
                jl = struct.unpack_from('<I', g, 12)[0]
                if not g[20:20 + jl].startswith(b'{"accessors"'):
                    tw['payload_reserialize'] = (i, k)
                    break
            if tw['payload_reserialize'] != 'UNSORTED':
                break
    return tw


def _walk(n):
    out, stack = [], [n]
    while stack:
        x = stack.pop(); out.append(x); stack.extend(x.children)
    return out


def _parent_h(root, c):
    for n in _walk(root):
        if c in n.children:
            return n.h
    return -1


def zip_members(path, members):
    with zipfile.ZipFile(path, 'w', zipfile.ZIP_STORED) as z:
        for name in sorted(members):
            info = zipfile.ZipInfo(name, date_time=(2026, 10, 11, 0, 0, 0))
            info.external_attr = 0o644 << 16
            z.writestr(info, members[name])


def build_case(name):
    if name == 'chain33':
        return chain_case(33)
    return POSITIVES[name]()


def main(out):
    out = pathlib.Path(out)
    out.mkdir(parents=True, exist_ok=True)
    manifest = {'generator': 'make_forest_fixtures.py',
                'generatorSha256': hashlib.sha256(pathlib.Path(__file__).read_bytes()).hexdigest(), 'cases': {}}
    for name in list(POSITIVES) + ['chain33']:
        root, scheme, top, vector = build_case(name)
        zip_members(out / f'{name}-explicit.3tz', write_explicit(root, top))
        if name in POSITIVES:
            members, info = write_forest(root, scheme, top, vector)
            zip_members(out / f'{name}.3tz', members)
            manifest['cases'][name] = {'kind': 'positive', 'archive': f'{name}.3tz', 'reference': f'{name}-explicit.3tz',
                                       'scheme': scheme, **info, 'expect': {'verdict': 'pass'}}
    for name, (base, tw, mutate, rules, needs_ref) in NEGATIVES.items():
        root, scheme, top, vector = build_case(base)
        tw = resolve_tweaks(tw, root, scheme, top, vector)
        members, info = write_forest(root, scheme, top, vector, tw)
        if callable(mutate):
            mutate(members)
        zip_members(out / f'{name}.3tz', members)
        if mutate == 'TRUNCATE':   # a damaged archive: the central directory is cut off
            data = (out / f'{name}.3tz').read_bytes()
            (out / f'{name}.3tz').write_bytes(data[:len(data) // 2])
        manifest['cases'][name] = {'kind': 'negative', 'archive': f'{name}.3tz', 'reference': f'{base}-explicit.3tz', 'base': base,
                                   'tweaks': {k: (list(v) if isinstance(v, tuple) else v) for k, v in tw.items()},
                                   'mutation': (mutate.__name__ if callable(mutate) else mutate), 'needsReference': needs_ref,
                                   'expect': {'failRules': rules}}
    for name, case in manifest['cases'].items():
        for key in ('archive', 'reference'):
            case[key + 'Sha256'] = hashlib.sha256((out / case[key]).read_bytes()).hexdigest()
    (out / 'manifest.json').write_text(json.dumps(manifest, indent=1) + '\n')
    print(json.dumps({'cases': len(manifest['cases']), 'out': str(out)}))


if __name__ == '__main__':
    main(sys.argv[1])
