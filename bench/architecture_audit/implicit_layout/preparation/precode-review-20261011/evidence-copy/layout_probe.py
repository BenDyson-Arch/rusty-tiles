"""P-IMPL layout probe (stdlib only; imports nothing from the repository).

Expands a rusty-tiles implicit .3tz (single tree with child subtrees, or the chunked
external-JSON-template form) into one global tile tree, then reports:
  * how far each effective row (TILE_BOUNDING_BOX) escapes its *derived* cell,
    measured against the top implicit root grid, by level and at subtree-root levels;
  * whether each row encloses its children's rows (the producer's enclosure rule);
  * request/size estimates for candidate layouts:
      today  - the archive as written,
      (a)    - one implicit tree, child subtrees every LEVELS levels,
      L1(S)  - explicit skeleton (tiles of height >= S) + single-subtree implicit roots
               (subtreeLevels == availableLevels == height+1 <= S) at every
               non-leaf child of a skeleton tile; skeleton leaves stay explicit.

Usage: python3 -I layout_probe.py OUT.json ARCHIVE.3tz [ARCHIVE.3tz ...]
"""
import json, math, struct, sys, zipfile, hashlib, posixpath


def morton(coords, bits):
    v = 0
    n = len(coords)
    for b in range(bits):
        for i, c in enumerate(coords):
            v |= ((c >> b) & 1) << (b * n + i)
    return v


class Subtree:
    def __init__(self, data, branching):
        magic, version, jlen, blen = struct.unpack_from('<4sIQQ', data, 0)
        assert magic == b'subt', magic
        self.json = json.loads(data[24:24 + jlen])
        self.bin = data[24 + jlen:24 + jlen + blen]
        self.b = branching
        self.views = self.json.get('bufferViews', [])

    def view(self, i):
        v = self.views[i]
        o = v.get('byteOffset', 0)
        return self.bin[o:o + v['byteLength']]

    def bit(self, avail, index):
        if 'constant' in avail:
            return bool(avail['constant'])
        raw = self.view(avail['bitstream'])
        return bool((raw[index >> 3] >> (index & 7)) & 1)

    def offset(self, level):
        return (self.b ** level - 1) // (self.b - 1)

    def index(self, level, xyz):
        return self.offset(level) + morton(xyz, level)

    def tile(self, level, xyz):
        return self.bit(self.json['tileAvailability'], self.index(level, xyz))

    def content(self, slot, level, xyz):
        return self.bit(self.json['contentAvailability'][slot], self.index(level, xyz))

    def child(self, xyz, levels):
        return self.bit(self.json['childSubtreeAvailability'], morton(xyz, levels))

    def rows(self, levels, dims):
        """Return {(level, xyz): (box, error)} in availability order."""
        table = self.json['propertyTables'][self.json['tileMetadata']]
        props = table['properties']
        boxes = self.view(props['boundingBox']['values'])
        errors = self.view(props['geometricError']['values'])
        out = {}
        k = 0
        for level in range(levels):
            n = 2 ** level
            cells = []
            for m in range(self.b ** level):
                xyz = [0] * dims
                for bitpos in range(level):
                    for i in range(dims):
                        xyz[i] |= ((m >> (bitpos * dims + i)) & 1) << bitpos
                cells.append(tuple(xyz))
            for xyz in cells:
                if self.tile(level, xyz):
                    box = struct.unpack_from('<12d', boxes, 96 * k)
                    err = struct.unpack_from('<d', errors, 8 * k)[0]
                    out[(level, xyz)] = (box, err)
                    k += 1
        assert k == table['count'], (k, table['count'])
        return out


def subst(template, level, xyz):
    s = template.replace('{level}', str(level)).replace('{x}', str(xyz[0])).replace('{y}', str(xyz[1]))
    if len(xyz) > 2:
        s = s.replace('{z}', str(xyz[2]))
    return s


def expand(z):
    top = json.loads(z.read('tileset.json'))
    tiles = {}  # global (level, xyz) -> dict
    docs = [0]
    subtrees = [0]
    subtree_bytes = [0]
    doc_bytes = [len(z.read('tileset.json'))]
    scheme = top['root']['implicitTiling']['subdivisionScheme']
    dims = 3 if scheme == 'OCTREE' else 2
    b = 2 ** dims

    headers = {}

    def walk_doc(root, base_level, base_xyz):
        docs[0] += 1
        it = root['implicitTiling']
        L = it['subtreeLevels']
        contents = root.get('contents') or ([root['content']] if 'content' in root else [])
        templates = [c['uri'] for c in contents]

        def walk_subtree(sub_level, sub_xyz):
            name = subst(it['subtrees']['uri'], sub_level, sub_xyz)
            data = z.read(name)
            subtrees[0] += 1
            subtree_bytes[0] += len(data)
            st = Subtree(data, b)
            rows = st.rows(L, dims)
            for (lvl, xyz), (box, err) in rows.items():
                local_level = sub_level + lvl
                local_xyz = tuple((sub_xyz[i] << lvl) + xyz[i] for i in range(dims))
                g_level = base_level + local_level
                g_xyz = tuple((base_xyz[i] << local_level) + local_xyz[i] for i in range(dims))
                slots = [st.content(s, lvl, xyz) for s in range(len(templates))]
                payload = [(s, subst(templates[s], local_level, local_xyz)) for s in range(len(templates)) if slots[s] and not templates[s].endswith('.json')]
                links = [subst(templates[s], local_level, local_xyz) for s in range(len(templates)) if slots[s] and templates[s].endswith('.json')]
                key = (g_level, g_xyz)
                t = tiles.setdefault(key, {'box': box, 'error': err, 'payload': 0, 'uris': []})
                t['uris'].extend(payload)
                headers.update({s: {k: v for k, v in contents[s].items() if k != 'uri'} for s, _ in payload})
                if lvl == 0 and sub_level == 0 and base_level > 0:
                    # external root: its own row wins (equal by construction)
                    t['box'] = box
                    t['error'] = err
                t['payload'] = max(t['payload'], len(payload))
                for link in links:
                    child = json.loads(z.read(link))
                    doc_bytes[0] += len(z.read(link))
                    walk_doc(child['root'], g_level, g_xyz)
            for xyz in _cells(L, dims):
                if st.child(xyz, L):
                    walk_subtree(sub_level + L, tuple((sub_xyz[i] << L) + xyz[i] for i in range(dims)))

        walk_subtree(0, (0,) * dims)

    walk_doc(top['root'], 0, (0,) * dims)
    return top, tiles, dims, dict(documents=docs[0], subtrees=subtrees[0], subtreeBytes=subtree_bytes[0], documentBytes=doc_bytes[0], headers=headers)


def _cells(level, dims):
    for m in range((2 ** dims) ** level):
        xyz = [0] * dims
        for bitpos in range(level):
            for i in range(dims):
                xyz[i] |= ((m >> (bitpos * dims + i)) & 1) << bitpos
        yield tuple(xyz)


def derived(root, level, xyz, dims):
    box = list(root)
    w = 2.0 ** level
    for axis in range(dims):
        v = xyz[axis]
        for lane in range(3):
            box[lane] += root[3 + axis * 3 + lane] * ((2 * v + 1) / w - 1)
            box[3 + axis * 3 + lane] = root[3 + axis * 3 + lane] / w
    return box


def corners(box):
    c = box[:3]
    a = [box[3:6], box[6:9], box[9:12]]
    out = []
    for sx in (-1, 1):
        for sy in (-1, 1):
            for sz in (-1, 1):
                out.append([c[i] + sx * a[0][i] + sy * a[1][i] + sz * a[2][i] for i in range(3)])
    return out


def escape(row, cell):
    """Max distance (metres, along cell axes) by which row corners leave cell; <=0 inside."""
    c = cell[:3]
    worst = -math.inf
    rel = -math.inf
    for p in corners(row):
        d = [p[i] - c[i] for i in range(3)]
        for k in range(3):
            ax = cell[3 + 3 * k:6 + 3 * k]
            n2 = sum(v * v for v in ax)
            n = math.sqrt(n2)
            if n == 0:
                continue
            u = abs(sum(d[i] * ax[i] for i in range(3))) / n2
            worst = max(worst, (u - 1) * n)
            rel = max(rel, u - 1)
    return worst, rel


def contains(outer, inner, tol=1e-9):
    return escape(inner, outer)[0] <= tol * (1 + max(abs(v) for v in outer))


def analyse(path, levels_today=4):
    z = zipfile.ZipFile(path)
    top, tiles, dims, today = expand(z)
    today.pop('headers')
    root_box = top['root']['boundingVolume']['box']
    b = 2 ** dims
    children = {k: [] for k in tiles}
    for (lvl, xyz) in tiles:
        if lvl:
            parent = (lvl - 1, tuple(v >> 1 for v in xyz))
            children[parent].append((lvl, xyz))
    height = {}
    for key in sorted(tiles, key=lambda k: -k[0]):
        height[key] = 1 + max((height[c] for c in children[key]), default=-1)
    depth = max(k[0] for k in tiles) + 1
    by_level = {}
    for (lvl, xyz), t in tiles.items():
        e, r = escape(t['box'], derived(root_box, lvl, xyz, dims))
        s = by_level.setdefault(lvl, dict(tiles=0, outside=0, maxEscapeMetres=0.0, maxEscapeRelative=0.0, medianEscapeMetres=[]))
        s['tiles'] += 1
        if e > 1e-9:
            s['outside'] += 1
            s['maxEscapeMetres'] = max(s['maxEscapeMetres'], e)
            s['maxEscapeRelative'] = max(s['maxEscapeRelative'], r)
            s['medianEscapeMetres'].append(e)
    for s in by_level.values():
        v = sorted(s['medianEscapeMetres'])
        s['medianEscapeMetres'] = v[len(v) // 2] if v else 0.0
    # enclosure: parent row contains child rows
    not_enclosing = sum(1 for k, cs in children.items() for c in cs if not contains(tiles[k]['box'], tiles[c]['box'], 1e-7))
    # layout (a): one tree, child subtrees every levels_today levels
    a_roots = [k for k in tiles if k[0] > 0 and k[0] % levels_today == 0]
    a_unsafe = [k for k in a_roots if escape(tiles[k]['box'], derived(root_box, k[0], k[1], dims))[0] > 1e-9]
    a_subtrees = 1 + len(a_roots)
    # L1(S)
    l1 = {}
    for S in range(2, 9):
        skeleton = [k for k in tiles if height[k] >= S]
        if height[(0, (0,) * dims)] < S:
            implicit_roots = [(0, (0,) * dims)]
            explicit_leaves = []
        else:
            implicit_roots = [c for k in skeleton for c in children[k] if height[c] < S and height[c] >= 1]
            explicit_leaves = [c for k in skeleton for c in children[k] if height[c] == 0]
        explicit = len(skeleton) + len(explicit_leaves)
        bits = 0
        for r in implicit_roots:
            levels = height[r] + 1
            n = ((b ** levels) - 1) // (b - 1)
            bits += n
        in_implicit = len(tiles) - explicit
        l1[S] = dict(explicitTiles=explicit, skeletonTiles=len(skeleton), explicitLeaves=len(explicit_leaves),
                     implicitRoots=len(implicit_roots), tilesInImplicitRoots=in_implicit,
                     maxSubtreeLevels=max((height[r] + 1 for r in implicit_roots), default=0),
                     availabilityBitsPerStream=bits,
                     # every available tile: 96 B box + 8 B error + extras string (~2-40 B) + 4 B offset
                     subtreeBytesEstimate=sum(24 + 700 for _ in implicit_roots) + 2 * bits // 8 + in_implicit * 112)
    count = len(tiles)
    payload_tiles = sum(1 for t in tiles.values() if t['payload'])
    result = dict(archive=posixpath.basename(path), sha256=hashlib.sha256(open(path, 'rb').read()).hexdigest(),
                  scheme='OCTREE' if dims == 3 else 'QUADTREE', tiles=count, payloadTiles=payload_tiles,
                  levels=depth, today=today, maxChildren=max(len(v) for v in children.values()),
                  childrenHistogram={str(n): sum(1 for v in children.values() if len(v) == n) for n in range(b + 1) if any(len(v) == n for v in children.values())},
                  rowsNotEnclosingChildRows=not_enclosing,
                  escapeByLevel={str(k): by_level[k] for k in sorted(by_level)},
                  layoutA=dict(subtrees=a_subtrees, childSubtreeRoots=len(a_roots), placeholderUnsafeRoots=len(a_unsafe),
                               maxPlaceholderEscapeMetres=max((escape(tiles[k]['box'], derived(root_box, k[0], k[1], dims))[0] for k in a_unsafe), default=0.0)),
                  layoutL1={str(k): v for k, v in l1.items()})
    return result


if __name__ == '__main__':
    out = sys.argv[1]
    results = [analyse(p) for p in sys.argv[2:]]
    with open(out, 'w') as f:
        json.dump(dict(probe='layout_probe.py', results=results), f, indent=1)
    for r in results:
        print(json.dumps({k: r[k] for k in ('archive', 'tiles', 'levels', 'today', 'layoutA', 'rowsNotEnclosingChildRows', 'childrenHistogram')}))
        for S, v in r['layoutL1'].items():
            print('  L1 S=%s' % S, json.dumps(v))
        for lvl, s in r['escapeByLevel'].items():
            print('  level', lvl, json.dumps(s))
