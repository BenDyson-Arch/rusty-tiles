"""Independent model of the recommended P-IMPL forest layout, applied to an existing
rusty-tiles implicit archive (stdlib only; the expander is layout_probe.py, loaded by path).

The input archive's effective tree (rows, errors, payload bytes) is kept unchanged; only
the layout changes:
  * a tile is an explicit *skeleton* tile when its height (levels below it) is >= S;
  * every non-leaf child of a skeleton tile with height < S becomes an implicit root whose
    subtreeLevels == availableLevels == height + 1, with one subtree, rows for every tile,
    childSubtreeAvailability constant 0 and payload-only content templates;
  * leaf children of skeleton tiles stay explicit;
  * if the root itself has height < S, the whole tree is one implicit root.
No external-JSON template is written. Payload GLBs are byte-identical copies: the input
already baked every translation into the top root frame, so no intermediate transforms
are needed for this A/B model (the production writer keeps skeleton translations; see
design.md).

Also writes views.json: close top-down views over sampled leaves with, per layout, the
URI -> global tile key mapping so selections can be compared tile-for-tile.

Usage: python3 -I relayout_forest.py IN.3tz OUT_DIR S [VIEW_COUNT]
"""
import hashlib, json, math, os, pathlib, struct, sys, zipfile

HERE = pathlib.Path(__file__).resolve().parent
probe = {'__name__': 'layout_probe'}
exec(compile((HERE / 'layout_probe.py').read_text(), str(HERE / 'layout_probe.py'), 'exec'), probe)


def morton(xyz, level):
    return probe['morton'](xyz, level)


def subtree_bytes(dims, levels, local_rows, content_tiles):
    b = 2 ** dims
    n_tiles = (b ** levels - 1) // (b - 1)

    def index(l, xyz):
        return (b ** l - 1) // (b - 1) + morton(xyz, l)
    binary = bytearray()
    views = []

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
    order = sorted(local_rows, key=lambda k: index(*k))
    doc = {"tileAvailability": avail(n_tiles, [index(*k) for k in order]),
           "contentAvailability": [avail(n_tiles, [index(*k) for k in order if k in content_tiles])],
           "childSubtreeAvailability": {"constant": 0}}
    boxes = b''.join(struct.pack('<12d', *local_rows[k][0]) for k in order)
    errors = b''.join(struct.pack('<d', local_rows[k][1]) for k in order)
    strings, offsets = bytearray(), bytearray(struct.pack('<I', 0))
    for k in order:
        strings += local_rows[k][2]
        offsets += struct.pack('<I', len(strings))
    doc["propertyTables"] = [{"class": "rustyTile", "count": len(order), "properties": {
        "boundingBox": {"values": view(boxes)}, "geometricError": {"values": view(errors)},
        "extras": {"values": view(bytes(strings)), "stringOffsets": view(bytes(offsets))}}}]
    doc["tileMetadata"] = 0
    while len(binary) % 8:
        binary.append(0)
    doc["buffers"] = [{"byteLength": len(binary)}]
    doc["bufferViews"] = views
    j = json.dumps(doc, separators=(',', ':')).encode()
    j += b' ' * (-len(j) % 8)
    return b'subt' + struct.pack('<IQQ', 1, len(j), len(binary)) + j + bytes(binary)


def main():
    src, out, S = sys.argv[1], pathlib.Path(sys.argv[2]), int(sys.argv[3])
    view_count = int(sys.argv[4]) if len(sys.argv) > 4 else 12
    z = zipfile.ZipFile(src)
    top, tiles, dims, today = probe['expand'](z)
    headers = today.pop('headers')
    assert set(headers) <= {0}, 'model handles one payload slot'
    header = headers.get(0, {})
    scheme = 'OCTREE' if dims == 3 else 'QUADTREE'
    tmpl = '{level}-{x}-{y}-{z}' if dims == 3 else '{level}-{x}-{y}'
    children = {k: [] for k in tiles}
    for (l, xyz) in tiles:
        if l:
            children[(l - 1, tuple(v >> 1 for v in xyz))].append((l, xyz))
    height = {}
    for key in sorted(tiles, key=lambda k: -k[0]):
        height[key] = 1 + max((height[c] for c in children[key]), default=-1)
    # extras strings are not in expand(); re-read rows as opaque '{}' would lose vector extras,
    # so carry the original extras by re-decoding each subtree's string column.
    extras = {}

    def collect_extras():
        # Walk again with a string-aware reader (same traversal as expand()).
        Sub = probe['Subtree']

        def walk_doc(root, base_level, base_xyz):
            it = root['implicitTiling']
            L = it['subtreeLevels']
            contents = root.get('contents') or ([root['content']] if 'content' in root else [])
            templates = [c['uri'] for c in contents]

            def walk(sub_level, sub_xyz):
                st = Sub(z.read(probe['subst'](it['subtrees']['uri'], sub_level, sub_xyz)), 2 ** dims)
                props = st.json['propertyTables'][st.json['tileMetadata']]['properties']
                values = st.view(props['extras']['values'])
                offs = st.view(props['extras']['stringOffsets'])
                k = 0
                for level in range(L):
                    for xyz in probe['_cells'](level, dims):
                        if not st.tile(level, xyz):
                            continue
                        a, b2 = struct.unpack_from('<II', offs, 4 * k)
                        k += 1
                        ll = sub_level + level
                        lx = tuple((sub_xyz[i] << level) + xyz[i] for i in range(dims))
                        g = (base_level + ll, tuple((base_xyz[i] << ll) + lx[i] for i in range(dims)))
                        if g not in extras or (level == 0 and sub_level == 0):
                            extras[g] = bytes(values[a:b2])
                        for s, t in enumerate(templates):
                            if t.endswith('.json') and st.content(s, level, xyz):
                                walk_doc(json.loads(z.read(probe['subst'](t, ll, lx)))['root'], g[0], g[1])
                for xyz in probe['_cells'](L, dims):
                    if st.child(xyz, L):
                        walk(sub_level + L, tuple((sub_xyz[i] << L) + xyz[i] for i in range(dims)))
            walk(0, (0,) * dims)
        walk_doc(top['root'], 0, (0,) * dims)
    collect_extras()
    assert set(extras) == set(tiles)

    os.makedirs(out, exist_ok=True)
    written = {'tileset.json': 0, 'subtrees': 0, 'subtreeBytes': 0, 'content': 0}
    uri_to_key = {}

    def name(key):
        return '-'.join(map(str, (key[0],) + key[1]))

    def copy(uri, dest):
        data = z.read(uri)
        p = out / dest
        p.parent.mkdir(parents=True, exist_ok=True)
        p.write_bytes(data)
        written['content'] += 1

    stats = dict(explicit=0, implicitRoots=0)

    def implicit_root(key):
        stats['implicitRoots'] += 1
        rid = name(key)
        levels = height[key] + 1
        rows, content = {}, set()
        stack = [key]
        while stack:
            k = stack.pop()
            dl = k[0] - key[0]
            local = (dl, tuple(k[1][i] - (key[1][i] << dl) for i in range(dims)))
            t = tiles[k]
            rows[local] = (t['box'], t['error'], extras[k])
            if t['uris']:
                content.add(local)
                dest = f"implicit-content/{rid}/{name(local)}-0.glb"
                copy(t['uris'][0][1], dest)
                uri_to_key[dest] = name(k)
            stack.extend(children[k])
        data = subtree_bytes(dims, levels, rows, content)
        p = out / f"subtrees/{rid}/{'-'.join(['0'] * (dims + 1))}.subtree"
        p.parent.mkdir(parents=True, exist_ok=True)
        p.write_bytes(data)
        written['subtrees'] += 1
        written['subtreeBytes'] += len(data)
        t = tiles[key]
        node = {"boundingVolume": {"box": list(t['box'])}, "geometricError": t['error'], "refine": "REPLACE",
                "implicitTiling": {"subdivisionScheme": scheme, "subtreeLevels": levels, "availableLevels": levels,
                                   "subtrees": {"uri": f"subtrees/{rid}/{tmpl}.subtree"}}}
        if content:
            node["content"] = {**header, "uri": f"implicit-content/{rid}/{tmpl}-0.glb"}
        return node

    def explicit(key):
        stats['explicit'] += 1
        t = tiles[key]
        node = {"boundingVolume": {"box": list(t['box'])}, "geometricError": t['error'], "refine": "REPLACE"}
        if t['uris']:
            dest = f"t/{name(key)}-0.glb"
            copy(t['uris'][0][1], dest)
            uri_to_key[dest] = name(key)
            node["content"] = {**header, "uri": dest}
        kids = []
        for c in sorted(children[key]):
            if height[c] >= S or height[c] == 0:
                kids.append(explicit(c))
            else:
                kids.append(implicit_root(c))
        if kids:
            node["children"] = kids
        return node
    root_key = (0, (0,) * dims)
    root = implicit_root(root_key) if height[root_key] < S else explicit(root_key)
    if 'transform' in top['root']:
        root['transform'] = top['root']['transform']
    manifest = {k: v for k, v in top.items() if k != 'root'}
    manifest['root'] = root
    if stats['implicitRoots'] == 0:
        manifest.pop('schema', None)
    data = json.dumps(manifest, separators=(',', ':')).encode()
    (out / 'tileset.json').write_bytes(data)
    written['tileset.json'] = len(data)

    # today's URI -> key mapping
    today_map = {}
    for k, t in tiles.items():
        for _, u in t['uris']:
            today_map[u] = name(k)
    # sample leaves deterministically, spread across the tree
    leaves = sorted((k for k in tiles if not children[k] and tiles[k]['uris']), key=lambda k: (k[0], k[1]))
    step = max(1, len(leaves) // view_count)
    views = []
    for k in leaves[::step][:view_count]:
        box = tiles[k]['box']
        half = max(math.sqrt(sum(v * v for v in box[3 + 3 * i:6 + 3 * i])) for i in range(3))
        views.append({"name": "leaf-" + name(k), "key": name(k), "target": list(box[:3]), "height": max(1.5 * half, 5.0)})
    (out.parent / f"{out.name}-views.json").write_text(json.dumps({"views": views, "forest": uri_to_key, "today": today_map}))
    summary = dict(input=os.path.basename(src), inputSha256=hashlib.sha256(open(src, 'rb').read()).hexdigest(), S=S,
                   tiles=len(tiles), levels=max(k[0] for k in tiles) + 1, today={k: v for k, v in today.items()},
                   forest=dict(**stats, **written))
    print(json.dumps(summary))


if __name__ == '__main__':
    main()
