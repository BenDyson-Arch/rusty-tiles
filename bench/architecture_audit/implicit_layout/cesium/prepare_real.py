"""Prepare a deep-view Cesium A/B (run_real.py) from real archives of the same input, in any layout.

PO port of the design's map builders (today_map.py, explicit_map.py, parents_map.py, forest_model.py's
leaf views), made layout-independent so it runs on develop and combined-head archives directly:

* Each archive is unpacked to OUT/NAME/ (served as files, as the archive viewers do).
* Each archive is expanded by a generic walker: explicit tiles, external tileset documents (link tiles
  and today's chunk documents), implicit roots with child subtrees, and JSON content templates.
* A payload's **content key** is sha256 of the GLB BIN chunk(s) plus its parsed glTF JSON with the
  scene-root node's translation/matrix removed (b3dm: the embedded GLB; other contents: the bytes).
  The layouts differ only in that translation (design §4.1, A-4), so one producer tile has one key in
  every layout. Keys shared by several tiles are reported as ambiguous and are never used as views.
* views.json: N leaves of the BASE layout (preorder, evenly spaced), each aimed at a real vertex of the
  leaf's first float POSITION accessor (R-9c), plus one {served uri: key} map per archive.
* parents.json: key -> nearest ancestor key in BASE (contentless tiles skipped).
* errors.json: key -> geometricError (rows for implicit tiles, TILE_GEOMETRIC_ERROR semantic).

Usage:
  python3 -I prepare_real.py OUT_DIR --base NAME [--views 16] [--height 40] NAME=ARCHIVE.3tz ...
Then (one browser at a time, on a free CPU pair; first case is the baseline, e.g. develop):
  PIMPL_DETERMINISTIC=1 PIMPL_STABLE=30 PIMPL_MSSE=0.0001 PIMPL_CHILD_BOUNDS=0 \\
    taskset -c A,B nice -n 10 python3 -I run_real.py OUT/views.json OUT/res-cb0.json develop=OUT/develop:develop head=OUT/head:head
"""
import argparse, hashlib, importlib.util, json, os, pathlib, struct, sys, zipfile

HERE = pathlib.Path(__file__).resolve().parent
_spec = importlib.util.spec_from_file_location('forest_check', HERE.parent / 'forest_check.py')
fc = importlib.util.module_from_spec(_spec)
_spec.loader.exec_module(fc)


def content_key(data):
    try:
        if data[:4] == b'b3dm':
            data = fc.b3dm_split(data)[1]
        if data[:4] != b'glTF':
            return 'raw:' + hashlib.sha256(data).hexdigest()
        js, rest = fc.glb_split(data)
        doc = json.loads(js)
        for i in fc.scene_roots(doc):
            node = doc['nodes'][i]
            node.pop('translation', None)
            m = node.get('matrix')
            if isinstance(m, list) and len(m) == 16:
                node['matrix'] = m[:12] + [0, 0, 0] + m[15:]
        h = hashlib.sha256(json.dumps(doc, sort_keys=True, separators=(',', ':')).encode())
        h.update(rest)
        return h.hexdigest()
    except (fc.FormatError, ValueError, struct.error, KeyError, IndexError, TypeError):
        return 'raw:' + hashlib.sha256(data).hexdigest()


class Node:
    __slots__ = ('uris', 'error', 'children', 'offset', 'order')

    def __init__(self, error, offset):
        self.uris, self.error, self.children, self.offset = [], error, [], offset


class Walker:
    def __init__(self, path):
        self.z = zipfile.ZipFile(path)
        self.names = set(self.z.namelist())
        self.docs = 0
        self.top = json.loads(self.z.read('tileset.json'))

    def walk(self):
        return self.tile(self.top['root'], 'tileset.json', (0.0, 0.0, 0.0), True, self.top.get('schema'))

    def tile(self, t, doc, offset, is_top, schema):
        if not is_top:
            tr, ok = fc.translation_of(t)
            offset = tuple(offset[k] + (tr[k] if ok else 0.0) for k in range(3))
        if 'implicitTiling' in t:
            return self.implicit(t, doc, offset, schema)
        n = Node(t.get('geometricError'), offset)
        cs = t.get('contents') if isinstance(t.get('contents'), list) else ([t['content']] if isinstance(t.get('content'), dict) else [])
        for c in cs:
            m = fc.resolve(doc, c.get('uri'))
            if m and m.lower().endswith('.json') and m in self.names:
                d = json.loads(self.z.read(m)); self.docs += 1
                n.children.append(self.tile(d['root'], m, offset, False, schema or d.get('schema')))
            elif m:
                n.uris.append(m)
        for c in t.get('children', []) or []:
            n.children.append(self.tile(c, doc, offset, False, schema))
        return n

    def implicit(self, t, doc, offset, schema):
        it = t['implicitTiling']
        dims = 3 if it['subdivisionScheme'] == 'OCTREE' else 2
        b = 2 ** dims
        sl, al = it['subtreeLevels'], it['availableLevels']
        cs = t.get('contents') if isinstance(t.get('contents'), list) else ([t['content']] if isinstance(t.get('content'), dict) else [])
        templates = [c.get('uri') for c in cs]
        sub = lambda s, l, xyz: s.replace('{level}', str(l)).replace('{x}', str(xyz[0])).replace('{y}', str(xyz[1])).replace('{z}', str(xyz[2]) if dims == 3 else '{z}')
        index = lambda l, xyz: (b ** l - 1) // (b - 1) + fc.morton(xyz, l)
        root = Node(t.get('geometricError'), offset)
        nodes = {(0, (0,) * dims): root}

        def errors_of(st, order):
            try:
                table = st.json['propertyTables'][st.json['tileMetadata']]
                cls = schema['classes'][table['class']]['properties']
                name = next(k for k, p in cls.items() if p.get('semantic') == 'TILE_GEOMETRIC_ERROR')
                raw = st.view(table['properties'][name]['values'])
                return {k: struct.unpack_from('<d', raw, 8 * i)[0] for i, k in enumerate(order)}
            except (KeyError, StopIteration, TypeError, IndexError, fc.FormatError, struct.error):
                return {}

        def subtree(level0, xyz0):
            name = fc.resolve(doc, sub(it['subtrees']['uri'], level0, xyz0))
            st = fc.Subtree(self.z.read(name))
            n_tiles = (b ** sl - 1) // (b - 1)
            avail = st.bits(st.json['tileAvailability'], n_tiles)
            layers = [st.bits(a, n_tiles) for a in st.json.get('contentAvailability', []) or []]
            child = st.bits(st.json['childSubtreeAvailability'], b ** sl)
            order = []
            for l in range(sl):
                for m in range(b ** l):
                    xyz = tuple(sum(((m >> (bit * dims + i)) & 1) << bit for bit in range(l)) for i in range(dims))
                    if index(l, xyz) in avail:
                        order.append((l, xyz))
            order.sort(key=lambda k: index(*k))
            errs = errors_of(st, order)
            for l, xyz in order:
                gl = level0 + l
                gxyz = tuple((xyz0[i] << l) + xyz[i] for i in range(dims))
                n = nodes.get((gl, gxyz))
                if n is None:
                    n = nodes[(gl, gxyz)] = Node(errs.get((l, xyz)), offset)
                    parent = nodes.get((gl - 1, tuple(v >> 1 for v in gxyz)))
                    if parent is not None:
                        parent.children.append(n)
                elif errs.get((l, xyz)) is not None:
                    n.error = errs[(l, xyz)]
                for li, bits in enumerate(layers):
                    if index(l, xyz) in bits and li < len(templates):
                        m = fc.resolve(doc, sub(templates[li], gl, gxyz))
                        if m and m.lower().endswith('.json'):
                            d = json.loads(self.z.read(m)); self.docs += 1
                            n.children.append(self.tile(d['root'], m, offset, False, schema or d.get('schema')))
                        elif m:
                            n.uris.append(m)
            for m in sorted(child):
                xyz = tuple(sum(((m >> (bit * dims + i)) & 1) << bit for bit in range(sl)) for i in range(dims))
                if level0 + sl < al:
                    subtree(level0 + sl, tuple((xyz0[i] << sl) + xyz[i] for i in range(dims)))
        subtree(0, (0,) * dims)
        return root


def leaf_target(data, offset):
    """A real vertex of the payload's first float POSITION accessor in the top root frame (z-up)."""
    if data[:4] == b'b3dm':
        data = fc.b3dm_split(data)[1]
    js, rest = fc.glb_split(data)
    doc = json.loads(js)
    if len(rest) < 8:
        return None
    bin_data = rest[8:8 + struct.unpack_from('<I', rest, 0)[0]]
    for mesh in doc.get('meshes', []):
        for prim in mesh.get('primitives', []):
            a = doc['accessors'][prim['attributes']['POSITION']]
            if a.get('componentType') != 5126 or 'bufferView' not in a or a.get('normalized'):
                continue
            bv = doc['bufferViews'][a['bufferView']]
            if bv.get('extensions'):
                continue   # compressed (meshopt) views have no plain data
            k = a['count'] // 2
            o = bv.get('byteOffset', 0) + a.get('byteOffset', 0) + k * bv.get('byteStride', 12)
            x, y, z = struct.unpack_from('<3f', bin_data, o)
            t = [0.0, 0.0, 0.0]
            for i in fc.scene_roots(doc):
                node = doc['nodes'][i]
                if any(key in node for key in ('scale', 'rotation', 'matrix')):
                    return None
                t = node.get('translation', t)
            gx, gy, gz = x + t[0], y + t[1], z + t[2]
            return [offset[0] + gx, offset[1] - gz, offset[2] + gy]
    return None


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument('out')
    ap.add_argument('--base', required=True, help='layout whose leaves, parents and errors are used')
    ap.add_argument('--views', type=int, default=16)
    ap.add_argument('--height', type=float, default=40.0)
    ap.add_argument('cases', nargs='+', help='NAME=ARCHIVE.3tz')
    a = ap.parse_args()
    out = pathlib.Path(a.out)
    out.mkdir(parents=True, exist_ok=True)
    sys.setrecursionlimit(20000)
    doc = {'views': []}
    receipt = {'script': hashlib.sha256(pathlib.Path(__file__).read_bytes()).hexdigest(),
               'checker': hashlib.sha256((HERE.parent / 'forest_check.py').read_bytes()).hexdigest(), 'archives': {}}
    trees = {}
    for item in a.cases:
        name, _, path = item.partition('=')
        target = out / name
        target.mkdir(parents=True, exist_ok=True)
        w = Walker(path)
        for info in w.z.infolist():
            if info.filename.endswith('/') or info.filename == fc.INDEX:
                continue
            dest = (target / info.filename).resolve()
            if not str(dest).startswith(str(target.resolve()) + os.sep):
                raise SystemExit(f'{path}: member {info.filename} escapes the output directory')
            dest.parent.mkdir(parents=True, exist_ok=True)
            dest.write_bytes(w.z.read(info.filename))
        root = w.walk()
        mapping, keys_count = {}, {}
        stack, pre = [root], []
        while stack:
            n = stack.pop()
            pre.append(n)
            n.uris = [(u, content_key(w.z.read(u))) if u in w.names else (u, None) for u in n.uris]
            for _, k in n.uris:
                if k:
                    keys_count[k] = keys_count.get(k, 0) + 1
            stack.extend(reversed(n.children))
        for n in pre:
            for u, k in n.uris:
                if k:
                    mapping[u] = k
        trees[name] = (w, root, pre, keys_count)
        doc[name] = mapping
        receipt['archives'][name] = {'path': path, 'sha256': fc.sha256_file(path), 'tiles': len(pre), 'documentsFollowed': w.docs + 1,
                                     'contents': sum(len(n.uris) for n in pre), 'distinctKeys': len(keys_count),
                                     'ambiguousKeys': sum(1 for v in keys_count.values() if v > 1)}
    w, root, pre, keys_count = trees[a.base]
    parents, errors = {}, {}

    def walk(n, anc):
        own = [k for _, k in n.uris if k]
        for k in own:
            parents.setdefault(k, next((x for x in reversed(anc) if x != k), None))
            if n.error is not None:
                errors[k] = n.error
        nxt = anc + own[:1] if own else anc
        for c in n.children:
            walk(c, nxt)
    walk(root, [])
    leaves = [n for n in pre if not n.children and any(k for _, k in n.uris)]
    usable = [n for n in leaves if all(keys_count.get(k) == 1 for _, k in n.uris if k)]
    picks = [usable[int(i * len(usable) / a.views)] for i in range(a.views)] if usable else []
    for n in picks:
        u, k = next((u, k) for u, k in n.uris if k)
        tgt = leaf_target(w.z.read(u), n.offset)
        if tgt is None:
            continue
        doc['views'].append({'name': f'leaf-p{k}', 'key': k, 'target': tgt, 'height': a.height})
    shared = set.intersection(*(set(d.keys()) for d in (t[3] for t in trees.values())))
    receipt.update(views=len(doc['views']), leaves=len(leaves), usableLeaves=len(usable), keysInEveryLayout=len(shared),
                   keysPerLayout={n: len(t[3]) for n, t in trees.items()}, base=a.base)
    (out / 'views.json').write_text(json.dumps(doc))
    (out / 'parents.json').write_text(json.dumps(parents))
    (out / 'errors.json').write_text(json.dumps(errors))
    (out / 'prepare.json').write_text(json.dumps(receipt, indent=1))
    print(json.dumps(receipt))


if __name__ == '__main__':
    main()
