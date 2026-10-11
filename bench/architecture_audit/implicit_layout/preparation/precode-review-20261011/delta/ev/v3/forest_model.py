"""P-IMPL v3 forest model (v3: vector no longer retains raw sources; no rustyTilesSourceContents): bounded implicit forest + explicit external tileset documents.

Stdlib only; imports nothing from the repository. This is an independent model of the
production writer contract in design.md §4 (v2), used for size/limit evidence and to write
servable archives for the Cesium A/B. It is not the oracle (PO writes that separately).

Inputs (a producer hierarchy H):
  explicit:PATH.3tz   a producer --explicit archive (vector: identical hierarchy to the default)
  implicit:PATH.3tz   today's default implicit archive (expanded by ../layout_probe.py)
  synth-vector:N:LOD:CHAIN:TEMPLATE.3tz   synthetic vector-shaped tree with ~N tiles, lodLevels LOD,
                      CHAIN = 'real' (chain lengths resampled from TEMPLATE) or 'full' (every leaf has
                      LOD coarse tiles); tile JSON objects are copied from TEMPLATE (explicit archive)
  synth-point:N:BRANCH  synthetic point octree, ~N tiles, BRANCH children per internal node

Layout rule (design v2 §4.1):
  h(n) = levels below n. Levels L = h(root)+1 must be <= 32 (typed error otherwise).
  skeleton: h >= S;  implicit root: non-leaf child of skeleton with h < S (or the root if h(root) < S);
  explicit leaf: leaf child of skeleton.
  Explicit tiles are packed into documents, breadth-first from each document root. A skeleton
  tile becomes a *link* (content -> external tileset document whose root is that tile) when
  including it with its non-skeleton children would exceed DOC_NODES value nodes, DOC_BYTES bytes
  or skeleton depth DOC_DEPTH within the document. Retained raw sources of rewritten implicit
  tiles are listed in the extras of the document holding their implicit root.

Usage:
  python3 -I forest_model.py measure OUT.json SPEC [S_OCTREE S_QUADTREE]
  python3 -I forest_model.py write OUT_DIR SPEC [S_OCTREE S_QUADTREE] [VIEWS]
"""
import hashlib, json, math, os, pathlib, random, struct, sys, zipfile

HERE = pathlib.Path(__file__).resolve().parent
MAX_LEVELS = 32
DOC_NODES = 16384          # 1/4 of validator document_items 65,536
DOC_BYTES = 2 * 1024 * 1024  # 1/4 of json_bytes 8 MiB
DOC_DEPTH = 12             # skeleton tile depth within one document
RETAIN = [False]           # vector: SourcePolicy::Retain (reuse baselines); point/mesh: Remove
JSON_DEPTH_ASSERT = 32     # 1/2 of json_depth 64; asserted on every document
VALIDATOR = dict(json_bytes=8 * 1024 * 1024, json_depth=64, document_items=65536,
                 archive_entries=65536, hierarchy_visits=65536, references=262144)


class LayoutError(Exception):
    pass


def nodes_depth(v, d=0):
    """Validator semantics: root + containers + scalars (not keys); root depth 0."""
    if isinstance(v, dict):
        n, m = 1, d
        for x in v.values():
            a, b = nodes_depth(x, d + 1)
            n += a; m = max(m, b)
        return n, m
    if isinstance(v, list):
        n, m = 1, d
        for x in v:
            a, b = nodes_depth(x, d + 1)
            n += a; m = max(m, b)
        return n, m
    return 1, d


def dumps(v):
    return json.dumps(v, separators=(',', ':')).encode()


class Node:
    __slots__ = ('tile', 'children', 'slot', 'h', 'uri', 'delta', 'pid')

    def __init__(self, tile, slot):
        self.tile, self.slot, self.children = tile, slot, []


# ---------------------------------------------------------------- readers
def read_explicit(path):
    z = zipfile.ZipFile(path)
    ts = json.loads(z.read('tileset.json'))

    def build(t, slot):
        n = Node({k: v for k, v in t.items() if k != 'children'}, slot)
        ex = n.tile.get('extras')
        if isinstance(ex, dict) and 'implicitChildIndex' in ex:
            ex = dict(ex); ex.pop('implicitChildIndex')
            n.tile['extras'] = ex
        for i, c in enumerate(t.get('children', [])):
            s = c.get('extras', {}).get('implicitChildIndex', i) if isinstance(c.get('extras'), dict) else i
            n.children.append(build(c, s))
        return n
    root = build(ts['root'], 0)
    top = {k: v for k, v in ts.items() if k != 'root'}
    return z, top, root


def read_implicit(path):
    probe = {'__name__': 'layout_probe'}
    exec(compile((HERE.parent / 'layout_probe.py').read_text(), 'layout_probe.py', 'exec'), probe)
    z = zipfile.ZipFile(path)
    top, tiles, dims, today = probe['expand'](z)
    headers = today.pop('headers')
    nodes = {}
    for key, t in tiles.items():
        tile = {'boundingVolume': {'box': list(t['box'])}, 'geometricError': t['error'], 'refine': 'REPLACE'}
        if t['uris']:
            tile['content'] = {**headers.get(0, {}), 'uri': t['uris'][0][1]}
        nodes[key] = Node(tile, 0)
    for (l, xyz), n in nodes.items():
        if l:
            p = nodes[(l - 1, tuple(v >> 1 for v in xyz))]
            n.slot = sum(((xyz[i] & 1) << i) for i in range(dims))
            p.children.append(n)
    for n in nodes.values():
        n.children.sort(key=lambda c: c.slot)
    root = nodes[(0, (0,) * dims)]
    root.tile['transform'] = top['root'].get('transform')
    if root.tile['transform'] is None:
        root.tile.pop('transform')
    t = {k: v for k, v in top.items() if k not in ('root', 'schema')}
    t.pop('extras', None)  # today's retained-source list is recomputed by the model
    return z, t, root, ('OCTREE' if dims == 3 else 'QUADTREE')


def synth_vector(n_tiles, lod, chain, template):
    z, top, root = read_explicit(template)
    kinds = {'leaf': [], 'chain': [], 'kd': []}

    def classify(n, parent_single=False):
        if n is root:
            pass
        elif not n.children:
            kinds['leaf'].append(n.tile)
        elif len(n.children) == 1:
            kinds['chain'].append(n.tile)
        else:
            kinds['kd'].append(n.tile)
        for c in n.children:
            classify(c)
    classify(root)
    # measured chain lengths above each full-detail leaf
    lengths = []

    def chains(n):
        if len(n.children) == 1:
            k, m = 0, n
            while len(m.children) == 1:
                k += 1; m = m.children[0]
            if not m.children:
                lengths.append(k)
                return
            for c in m.children:
                chains(c)
            return
        if not n.children:
            lengths.append(0)
        for c in n.children:
            chains(c)
    chains(root)
    rng = random.Random(20261011)
    mean = sum(lengths) / len(lengths) if chain == 'real' else lod
    leaves = max(1, round(n_tiles / (2 + mean)))

    def pick(kind):
        pool = kinds[kind] or kinds['leaf']
        return json.loads(json.dumps(pool[rng.randrange(len(pool))]))

    def kd(count, slot):
        if count == 1:
            c = lod if chain == 'full' else rng.choice(lengths)
            leaf = Node(pick('leaf'), 0)
            top_node = leaf
            for _ in range(c):
                up = Node(pick('chain'), 0)
                up.children.append(top_node); top_node.slot = 0
                top_node = up
            top_node.slot = slot
            return top_node
        n = Node(pick('kd'), slot)
        a = count // 2
        n.children = [kd(count - a, 0), kd(a, 1)]
        return n
    r = kd(leaves, 0)
    r.tile['transform'] = root.tile.get('transform', r.tile.get('transform'))
    return None, top, r


def synth_point(n_tiles, branch):
    # bushy octree: full `branch`-ary internal nodes down to the depth that reaches n_tiles
    tile = {'boundingVolume': {'box': [0.0, 0.0, 0.0, 12.345678901234, 0, 0, 0, 23.456789012345, 0, 0, 0, 3.4567890123456]},
            'transform': [1, 0, 0, 0, 0, 1, 0, 0, 0, 0, 1, 0, 12.3456789, -23.456789, 1.2345678, 1],
            'geometricError': 0.123456789012, 'refine': 'REPLACE', 'content': {'uri': 't/12345.glb'}}
    depth, total = 0, 1
    while total + branch ** (depth + 1) <= n_tiles:   # largest full tree with at most n_tiles tiles
        depth += 1
        total += branch ** depth
    slots = [0, 1, 2, 3, 4, 5, 6, 7][:branch] if branch < 8 else list(range(8))

    def build(d, slot):
        n = Node(json.loads(json.dumps(tile)), slot)
        if d < depth:
            n.children = [build(d + 1, s) for s in slots]
        return n
    return None, {'asset': {'version': '1.1'}, 'geometricError': 1000.0}, build(0, 0)


# ---------------------------------------------------------------- layout
def heights(n):
    n.h = 1 + max((heights(c) for c in n.children), default=-1)
    return n.h


def count(n):
    return 1 + sum(count(c) for c in n.children)


def translation(tile):
    t = tile.get('transform')
    if t is None:
        return [0.0, 0.0, 0.0]
    if any(t[i] != (1 if i % 5 == 0 else 0) for i in range(12)):
        raise LayoutError('child transform must be a translation')
    return [float(t[12]), float(t[13]), float(t[14])]


def plan(root, scheme, S):
    heights(root)
    if root.h + 1 > MAX_LEVELS:
        raise LayoutError(f'hierarchy has {root.h + 1} levels; implicit output admits at most {MAX_LEVELS}')
    branches = 8 if scheme == 'OCTREE' else 4
    pid = [0]

    def number(n):
        n.pid = pid[0]; pid[0] += 1
        seen = set()
        for c in n.children:
            if not (0 <= c.slot < branches) or c.slot in seen:
                raise LayoutError('invalid or duplicate child slot')
            seen.add(c.slot)
            number(c)
    number(root)


def subtree_bytes(dims, levels, rows, content):
    b = 2 ** dims
    n_tiles = (b ** levels - 1) // (b - 1)

    def morton(xyz, level):
        v = 0
        for bit in range(level):
            for i, c in enumerate(xyz):
                v |= ((c >> bit) & 1) << (bit * dims + i)
        return v

    def index(l, xyz):
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
    order = sorted(rows, key=lambda k: index(*k))
    doc = {"tileAvailability": avail(n_tiles, [index(*k) for k in order]),
           "childSubtreeAvailability": {"constant": 0}}
    if content is not None:
        doc["contentAvailability"] = [avail(n_tiles, [index(*k) for k in order if k in content])]
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


def translate_glb(data, delta):
    """Bake a z-up tile-frame translation into the GLB scene roots as (d0, d2, -d1)."""
    if delta == [0.0, 0.0, 0.0]:
        return data
    assert data[:4] == b'glTF'
    jl = struct.unpack_from('<I', data, 12)[0]
    doc = json.loads(data[20:20 + jl])
    rest = data[20 + jl:]
    t = [delta[0], delta[2], -delta[1]]
    roots = sorted({i for s in doc['scenes'] for i in s['nodes']})
    for i in roots:
        node = doc['nodes'][i]
        if 'matrix' in node:
            m = node['matrix']
            for k in range(3):
                m[12 + k] += t[k]
        else:
            tr = node.get('translation', [0.0, 0.0, 0.0])
            node['translation'] = [tr[k] + t[k] for k in range(3)]
    j = dumps(doc)
    j += b' ' * (-len(j) % 4)
    total = 12 + 8 + len(j) + len(rest)
    return b'glTF' + struct.pack('<II', 2, total) + struct.pack('<I4s', len(j), b'JSON') + j + rest


def layout(z, top, root, scheme, S, out=None):
    """Plan, partition and (optionally) write. Returns (summary, leaf_info)."""
    dims = 3 if scheme == 'OCTREE' else 2
    tmpl = '{level}-{x}-{y}-{z}' if dims == 3 else '{level}-{x}-{y}'
    plan(root, scheme, S)
    stats = {'explicitTiles': 0, 'linkTiles': 0, 'implicitRoots': 0, 'implicitTiles': 0, 'subtrees': 0,
             'subtreeBytes': 0, 'maxSubtreeBytes': 0, 'maxSubtreeTiles': 0, 'implicitContent': 0,
             'retainedContent': 0, 'explicitContent': 0}
    leaf_info = []
    uri_map = {}

    def write(name, data):
        if out is not None:
            p = out / name
            p.parent.mkdir(parents=True, exist_ok=True)
            p.write_bytes(data)

    def ext(uri):
        return uri.rsplit('.', 1)[1]

    # ---- implicit roots: one subtree, rows on every tile, payloads rebased onto the root frame
    def implicit_root(r, acc_r, sources):
        stats['implicitRoots'] += 1
        rid = f"r{r.pid}"
        levels = r.h + 1
        rows, content, stack, header, suffix = {}, set(), [(r, 0, (0,) * dims, [0.0, 0.0, 0.0])], None, None
        while stack:
            n, lvl, xyz, delta = stack.pop()
            stats['implicitTiles'] += 1
            if n.tile.get('refine', 'REPLACE') != r.tile.get('refine', 'REPLACE'):
                raise LayoutError('refine differs below an implicit root')
            box = list(n.tile['boundingVolume']['box'])
            for i in range(3):
                box[i] += delta[i]
            rows[(lvl, xyz)] = (box, n.tile['geometricError'], dumps(n.tile.get('extras', {})))
            c = n.tile.get('content')
            if c:
                if suffix is not None and ext(c['uri']) != suffix:
                    raise LayoutError('one content suffix per slot')
                suffix = ext(c['uri'])
                header = {k: v for k, v in c.items() if k != 'uri'}
                content.add((lvl, xyz))
                name = f"implicit-content/{rid}-{lvl}-" + '-'.join(map(str, xyz)) + f"-0.{suffix}"
                uri_map[name] = n.pid
                stats['implicitContent'] += 1
                if z is not None:
                    write(name, translate_glb(z.read(c['uri']), delta))
                if RETAIN[0]:
                    sources.append(c['uri'])   # SourcePolicy::Retain: every rewritten raw source stays listed
                if not n.children:
                    leaf_info.append((n.pid, name, [acc_r[i] + delta[i] for i in range(3)], c['uri']))
            for ch in n.children:
                t = translation(ch.tile)
                stack.append((ch, lvl + 1, tuple((xyz[i] << 1) | ((ch.slot >> i) & 1) for i in range(dims)),
                              [delta[i] + t[i] for i in range(3)]))
        data = subtree_bytes(dims, levels, rows, content if content else None)
        write(f"subtrees/{rid}-{'-'.join(['0'] * (dims + 1))}.subtree", data)
        stats['subtrees'] += 1
        stats['subtreeBytes'] += len(data)
        stats['maxSubtreeBytes'] = max(stats['maxSubtreeBytes'], len(data))
        stats['maxSubtreeTiles'] = max(stats['maxSubtreeTiles'], len(rows))
        tile = {k: v for k, v in r.tile.items() if k != 'content'}
        tile['implicitTiling'] = {"subdivisionScheme": scheme, "subtreeLevels": levels, "availableLevels": levels,
                                  "subtrees": {"uri": f"subtrees/{rid}-{tmpl}.subtree"}}
        if content:
            tile['content'] = {**header, "uri": f"implicit-content/{rid}-{tmpl}-0.{suffix}"}
        return tile

    def explicit_tile(n, acc):
        stats['explicitTiles'] += 1
        c = n.tile.get('content')
        if c:
            uri_map[c['uri']] = n.pid
            stats['explicitContent'] += 1
            if z is not None:
                write(c['uri'], z.read(c['uri']))
                if not n.children:
                    leaf_info.append((n.pid, c['uri'], acc, c['uri']))
        return dict(n.tile)

    def unit(n, acc, sources):
        """A skeleton tile with its non-skeleton children (explicit leaves, implicit roots)."""
        obj = explicit_tile(n, acc)
        kids, skel = [], []
        for ch in n.children:
            cacc = [acc[i] + translation(ch.tile)[i] for i in range(3)]
            if ch.h >= S:
                skel.append((ch, cacc))
            elif ch.h == 0:
                kids.append(explicit_tile(ch, cacc))
            else:
                kids.append(implicit_root(ch, cacc, sources))
        return obj, kids, skel

    docs = []

    def finish(name, doc, sources):
        if sources:
            doc['extras'] = {**doc.get('extras', {}), 'rustyTilesSourceContents': sorted(set(sources))}
            stats['retainedContent'] += len(set(sources))
        data = dumps(doc)
        n, d = nodes_depth(doc)
        if d > JSON_DEPTH_ASSERT:
            raise LayoutError(f'{name}: JSON depth {d} exceeds the profile bound {JSON_DEPTH_ASSERT}')
        docs.append({'name': name, 'valueNodes': n, 'jsonDepth': d, 'bytes': len(data),
                     'bytesPretty': len(json.dumps(doc, indent=2).encode()), 'retainedSources': len(set(sources))})
        write(name, data)

    # Cesium 1.146 reads the metadata schema only from the root tileset.json (Cesium3DTileset.js:2470,
    # makeTile), also for implicit roots inside external documents: tileset.json carries it whenever any
    # implicit root exists; every document holding an implicit root carries it too.
    def walk_any(n):
        return any((1 <= c.h < S) or (c.h >= S and walk_any(c)) for c in n.children)
    any_implicit = root.h < S or walk_any(root)
    if root.h < S:
        sources = []
        doc = dict(top, schema=SCHEMA, root=implicit_root(root, [0.0, 0.0, 0.0], sources))
        finish('tileset.json', doc, sources)
    else:
        queue, index = [(root, [0.0, 0.0, 0.0], 'tileset.json')], 0
        while queue:
            droot, acc, name = queue.pop(0)
            sources = []
            roots_before = stats['implicitRoots']
            obj, kids, skel = unit(droot, acc, sources)
            if name != 'tileset.json':
                obj.pop('transform', None)   # the link tile in the parent document carries it
            def link_for(ch, lname):
                link = {'boundingVolume': ch.tile['boundingVolume'], 'geometricError': ch.tile['geometricError'],
                        'refine': ch.tile.get('refine', 'REPLACE'), 'content': {'uri': lname}}
                if 'transform' in ch.tile:
                    link['transform'] = ch.tile['transform']
                return link

            def link_cost(ch):
                l = link_for(ch, 'tileset-d0000000.json')
                return nodes_depth(l)[0], len(dumps(l)) + 1
            # Every frontier skeleton child is reserved at link cost; accepting it replaces that
            # reservation by its unit cost plus link reservations for its own skeleton children.
            used_nodes = nodes_depth(dict(obj, children=kids))[0] + len(sources) + 64 + sum(link_cost(c)[0] for c, _ in skel)
            used_bytes = (len(dumps(dict(obj, children=kids))) + sum(len(x) + 3 for x in sources) + 4096
                          + sum(link_cost(c)[1] for c, _ in skel))
            frontier = [(obj, kids, skel, 0)]
            while frontier:
                o, k, sk, d = frontier.pop(0)
                children = list(k)
                for ch, cacc in sk:
                    mark = (len(leaf_info), dict(stats))
                    probe_sources = []
                    cobj, ckids, cskel = unit(ch, cacc, probe_sources)
                    probe = dict(cobj, children=ckids)
                    ln, lb = link_cost(ch)
                    cn = nodes_depth(probe)[0] + len(probe_sources) + sum(link_cost(c)[0] for c, _ in cskel) - ln
                    cb = len(dumps(probe)) + sum(len(x) + 3 for x in probe_sources) + sum(link_cost(c)[1] for c, _ in cskel) - lb
                    del leaf_info[mark[0]:]
                    stats.clear(); stats.update(mark[1])
                    if d + 1 > DOC_DEPTH or used_nodes + cn > DOC_NODES or used_bytes + cb > DOC_BYTES:
                        index += 1
                        lname = f"tileset-d{index}.json"
                        stats['linkTiles'] += 1
                        children.append(link_for(ch, lname))
                        queue.append((ch, cacc, lname))
                        continue
                    cobj, ckids, cskel = unit(ch, cacc, sources)
                    used_nodes += cn; used_bytes += cb
                    children.append(cobj)
                    frontier.append((cobj, ckids, cskel, d + 1))
                if children:
                    o['children'] = children
            if name == 'tileset.json':
                doc = dict(top, root=obj)
            else:
                doc = {'asset': top.get('asset', {'version': '1.1'}), 'geometricError': droot.tile['geometricError'], 'root': obj}
                if 'extensionsUsed' in top:
                    doc['extensionsUsed'] = top['extensionsUsed']
            if (name == 'tileset.json' and any_implicit) or stats['implicitRoots'] > roots_before:
                doc['schema'] = SCHEMA
            finish(name, doc, sources)
    total = count(root)
    summary = {'scheme': scheme, 'S': S, 'tiles': total, 'levels': root.h + 1, **stats, 'documents': len(docs),
               'docs': docs, 'rootDoc': docs[0],
               'maxDocNodes': max(d['valueNodes'] for d in docs), 'maxDocBytes': max(d['bytes'] for d in docs),
               'maxDocBytesPretty': max(d['bytesPretty'] for d in docs), 'maxDocJsonDepth': max(d['jsonDepth'] for d in docs)}
    assert stats['explicitTiles'] + stats['implicitTiles'] == total, (stats, total)
    # I1 v3.1 visit formula: Tileset + ExitTileset per document, one per explicit Node task (explicit + link
    # tiles; an implicit root R is a Node task), plus available implicit tiles below R (I_0 is bit 0)
    summary['hierarchyVisits'] = 2 * len(docs) + stats['explicitTiles'] + stats['linkTiles'] + stats['implicitRoots'] + stats['implicitTiles']
    summary['archiveEntries'] = (len(docs) + stats['subtrees'] + stats['implicitContent'] + stats['explicitContent']
                                 + stats['retainedContent'] + 3)  # + conversion.json, geometry-reports, vector-build.json
    for k, lim in (('maxDocNodes', 'document_items'), ('maxDocBytes', 'json_bytes'), ('maxDocJsonDepth', 'json_depth')):
        summary[k + 'OfLimit'] = round(summary[k] / VALIDATOR[lim], 4)
    for k, lim in (('hierarchyVisits', 'hierarchy_visits'), ('archiveEntries', 'archive_entries')):
        summary[k + 'OfLimit'] = round(summary[k] / VALIDATOR[lim], 4)
    summary['_uriMap'] = uri_map
    return summary, leaf_info


SCHEMA = {"id": "rustyTilesImplicit", "classes": {"rustyTile": {"properties": {
    "boundingBox": {"type": "SCALAR", "componentType": "FLOAT64", "array": True, "count": 12, "semantic": "TILE_BOUNDING_BOX"},
    "geometricError": {"type": "SCALAR", "componentType": "FLOAT64", "semantic": "TILE_GEOMETRIC_ERROR"},
    "extras": {"type": "STRING"}}}}}


def leaf_target(z, glb_uri, offset, delta):
    """A real vertex of the leaf's first POSITION accessor, in the root frame (z-up)."""
    data = z.read(glb_uri)
    jl = struct.unpack_from('<I', data, 12)[0]
    doc = json.loads(data[20:20 + jl])
    bin_off = 20 + jl + 8
    for mesh in doc['meshes']:
        for prim in mesh['primitives']:
            a = doc['accessors'][prim['attributes']['POSITION']]
            if a.get('componentType') != 5126 or 'bufferView' not in a:
                continue
            bv = doc['bufferViews'][a['bufferView']]
            k = a['count'] // 2
            stride = bv.get('byteStride', 12)
            o = bin_off + bv.get('byteOffset', 0) + a.get('byteOffset', 0) + k * stride
            x, y, zz = struct.unpack_from('<3f', data, o)
            node_t = [0.0, 0.0, 0.0]
            for i in doc['scenes'][0]['nodes']:
                node_t = doc['nodes'][i].get('translation', node_t)
            gx, gy, gz = x + node_t[0], y + node_t[1], zz + node_t[2]
            return [offset[0] + gx, offset[1] - gz, offset[2] + gy]
    return None


def main():
    mode, out, spec = sys.argv[1], pathlib.Path(sys.argv[2]), sys.argv[3]
    s_oct = int(sys.argv[4]) if len(sys.argv) > 4 else 4
    s_quad = int(sys.argv[5]) if len(sys.argv) > 5 else 4
    n_views = int(sys.argv[6]) if len(sys.argv) > 6 else 0
    kind, _, rest = spec.partition(':')
    if kind == 'explicit':
        z, top, root = read_explicit(rest)
        scheme = 'QUADTREE' if 'extensionsUsed' in top and '3DTILES_content_gltf_vector' in top['extensionsUsed'] else 'OCTREE'
        top.pop('extras', None)
    elif kind == 'implicit':
        z, top, root, scheme = read_implicit(rest)
    elif kind == 'synth-vector':
        n, lod, chain, template = rest.split(':', 3)
        z, top, root = synth_vector(int(n), int(lod), chain, template)
        scheme = 'QUADTREE'
    elif kind == 'synth-point':
        n, branch = rest.split(':')
        z, top, root = synth_point(int(n), int(branch))
        scheme = 'OCTREE'
    S = s_oct if scheme == 'OCTREE' else s_quad
    RETAIN[0] = False   # v3 (PD-11 user decision): no producer retains raw sources
    result = {'spec': spec, 'model': 'forest_model.py', 'modelSha256': hashlib.sha256((HERE / 'forest_model.py').read_bytes()).hexdigest(),
              'limits': {'DOC_NODES': DOC_NODES, 'DOC_BYTES': DOC_BYTES, 'DOC_DEPTH': DOC_DEPTH, 'MAX_LEVELS': MAX_LEVELS}}
    try:
        if mode == 'write':
            out.mkdir(parents=True, exist_ok=True)
            summary, leaves = layout(z, top, root, scheme, S, out=out)
            if n_views and z is not None:
                rng = random.Random(7)
                leaves = sorted(leaves)
                picks = [leaves[int(i * len(leaves) / n_views)] for i in range(n_views)]
                views = []
                for pid, uri, offset, src in picks:
                    tgt = leaf_target(z, src, offset, [0.0, 0.0, 0.0])
                    if tgt:
                        views.append({'name': f'leaf-p{pid}', 'key': str(pid), 'target': tgt, 'height': 40.0})
                (out.parent / f'{out.name}-views.json').write_text(json.dumps({'views': views, 'forest': {u: str(p) for u, p in summary['_uriMap'].items()}}))
        else:
            summary, _ = layout(z, top, root, scheme, S, out=None)
        summary.pop('_uriMap', None)
        result.update(summary)
        result['docs'] = sorted(result['docs'], key=lambda d: -d['valueNodes'])[:5]
    except LayoutError as e:
        result['layoutError'] = str(e)
    (out if mode == 'measure' else out.parent / f'{out.name}-summary.json').write_text(json.dumps(result, indent=1))
    print(json.dumps({k: v for k, v in result.items() if k not in ('docs',)}))


if __name__ == '__main__':
    main()
