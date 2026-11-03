"""Independent declared-root-recovery checks. Uses delivered tiles and payloads only.

The ownership scaffold is distinct from explicit refinement. Intervals are checked
against independently counted terminal payload triangles and DFS leaves, and the
frontier must be exactly the root's actual cell children through document links.
"""
import json
import struct



def marker(data, require):
    length = struct.unpack_from('<I', data, 12)[0]
    doc = json.loads(data[20:20 + length])
    value = doc.get('asset', {}).get('extras', {}).get('rustyTiles', {}).get('descendantRecovery')
    if value is None:
        return False
    require(type(value) is int and value == 1, 'recoveryRelation', 'invalid payload recovery version')
    return True


def key(value, require):
    names = ('first', 'end', 'depth', 'leafBegin', 'leafEnd')
    require(isinstance(value, dict) and set(value) == set(names), 'recoveryRelation', 'invalid ownership key')
    require(all(type(value[n]) is int and 0 <= value[n] < 2**63 for n in names),
            'recoveryRelation', 'invalid ownership integer')
    result = tuple(value[n] for n in names)
    a, b, d, lo, hi = result
    require(a < b and lo < hi < 2**32 and d <= 30,
            'recoveryCoverage', 'empty or invalid ownership interval')
    return result


def cell_children(tile):
    out, pending = [], list(reversed(tile['children']))
    while pending:
        child = pending.pop()
        if child['external']:
            pending.extend(reversed(child['children']))
        else:
            out.append(child)
    return out


def inspect(tiles, sizes, markers, require):
    declarations = [t for t in tiles if 'recovery' in t['node'].get('extras', {})]
    marked = [t for t in tiles if any(markers[u] for u in t['contents'])]
    require(len(declarations) == len(marked) <= 1 and all(a is b for a, b in zip(declarations, marked)),
            'recoveryRelation', 'payload and delivery recovery declarations differ')
    if not declarations:
        return dict(recoveredRoots=0, bypassedCells=0, bypassedSources=0, bypassedByDepth=[])
    root = declarations[0]
    parent = root['parent']
    while parent is not None and parent['external']:
        parent = parent['parent']
    require(parent is None and root['refine'] == 'REPLACE' and len(root['contents']) == 1
            and not root.get('implicit') and 'implicitTiling' not in root['node'],
            'recoveryRelation', 'recovery requires one explicit REPLACE ownership root')
    value = root['node']['extras']['recovery']
    require(isinstance(value, dict) and set(value) == {'version', 'root', 'encodedDepth', 'frontier', 'omitted'}
            and type(value['version']) is int and value['version'] == 1,
            'recoveryRelation', 'invalid recovery declaration')
    rk = key(value['root'], require)
    require(rk[0] == rk[2] == rk[3] == 0 and type(value['encodedDepth']) is int
            and 0 <= value['encodedDepth'] <= 30
            and str(root['node']['extras'].get('first')) == '0'
            and root['node']['extras'].get('depth') == 0,
            'recoveryRelation', 'invalid recovery root')
    inputs, omitted = value['frontier'], value['omitted']
    require(isinstance(inputs, list) and 1 <= len(inputs) <= 8
            and isinstance(omitted, list) and 1 <= len(omitted) <= 256,
            'recoveryBounds', 'recovery exceeds its input or ownership bounds')
    keys, counts = [], []
    for item in inputs:
        require(isinstance(item, dict) and set(item) == {'key', 'triangles'},
                'recoveryRelation', 'invalid frontier input')
        keys.append(key(item['key'], require))
        n = item['triangles']
        require(type(n) is int and 1 <= n <= 8192, 'recoveryBounds', 'frontier input exceeds the cap')
        counts.append(n)
    require(sum(counts) <= 8 * 8192, 'recoveryBounds', 'frontier triangles exceed 8C')
    skipped = [key(k, require) for k in omitted]

    # Terminal cover, independent of the declaration and all producer rank labels.
    bounds, leaf, rank = {}, 0, 0
    for t in tiles:
        if not t['children']:
            require(bool(t['contents']), 'recoveryCoverage', 'terminal frontier has no content')
            count = sum(sizes[u] for u in t['contents'])
            bounds[id(t)] = (rank, rank + count, leaf, leaf + 1)
            rank, leaf = rank + count, leaf + 1
    for t in reversed(tiles):
        if t['children']:
            a, b = bounds[id(t['children'][0])], bounds[id(t['children'][-1])]
            bounds[id(t)] = (a[0], b[1], a[2], b[3])
    require(bounds[id(root)] == (rk[0], rk[1], rk[3], rk[4]),
            'recoveryCoverage', 'root interval differs from its delivered terminal cover')
    children = cell_children(root)
    require(len(children) == len(keys), 'recoveryCoverage', 'refinement children differ from the frontier')
    for child, ck, n in zip(children, keys, counts):
        require(bool(child['contents']) and len(child['contents']) == 1,
                'recoveryCoverage', 'frontier child has no single renderable payload')
        require(bounds[id(child)] == (ck[0], ck[1], ck[3], ck[4]),
                'recoveryCoverage', 'frontier interval differs from its delivered cover')
        extras = child['node'].get('extras', {})
        require(str(extras.get('first')) == str(ck[0]) and extras.get('depth') == ck[2],
                'recoveryOwnership', 'frontier ownership identity differs from the declaration')
        require(sum(sizes[u] for u in child['contents']) == n, 'recoveryBounds', 'frontier triangle count differs')
        require(root['g'] >= child['g'], 'recoveryError', 'root G is below its refinement frontier')
        child['_recovery_depth'] = ck[2]

    # Reconstruct a bounded ownership tree; each edge advances one ownership level.
    encoded_root = (rk[0], rk[1], value['encodedDepth'], rk[3], rk[4])
    nodes = [encoded_root] + skipped + keys
    require(len(set(nodes)) == len(nodes), 'recoveryCoverage', 'duplicate ownership cell')
    parents, descendants = {}, {k: [] for k in nodes}
    for k in nodes[1:]:
        candidates = [p for p in nodes if p[2] < k[2] and p[0] <= k[0] and k[1] <= p[1]
                      and p[3] <= k[3] and k[4] <= p[4]]
        require(bool(candidates), 'recoveryOwnership', 'ownership cell has no ancestor')
        p = max(candidates, key=lambda p: p[2])
        require(k[2] == p[2] + 1 and p not in keys, 'recoveryOwnership', 'ownership scaffold skips a level')
        parents[k] = p
        descendants[p].append(k)
    for p in [encoded_root] + skipped:
        children_keys = sorted(descendants[p])
        require(1 <= len(children_keys) <= 8, 'recoveryCoverage', 'ownership cell lacks its bounded cover')
        first, lo = p[0], p[3]
        for c in children_keys:
            require(c[0] == first and c[3] == lo, 'recoveryCoverage', 'gap or overlap in ownership cover')
            first, lo = c[1], c[4]
        require(first == p[1] and lo == p[4], 'recoveryCoverage', 'ownership cover ends before its cell')
    for child, ck in zip(children, keys):
        child['_recovery_owner'] = parents[ck]
    root['_recovery_root'] = True
    root['_recovery_scaffold'] = (encoded_root, skipped, keys, parents)
    by_depth = []
    for k in skipped:
        by_depth.extend([0] * (k[2] + 1 - len(by_depth)))
        by_depth[k[2]] += 1
    return dict(recoveredRoots=1, bypassedCells=len(skipped),
                bypassedSources=sum(all(c in keys for c in descendants[k]) for k in skipped),
                bypassedByDepth=by_depth)


def ownership_view(tiles):
    """Restore verified omitted ownership cells for D11 only, without editing delivery."""
    roots = [t for t in tiles if '_recovery_scaffold' in t]
    if not roots or any(t.get('_ownership_virtual') for t in tiles):
        return tiles
    root = roots[0]
    copies = {id(t): dict(t, children=[]) for t in tiles}
    for t in tiles:
        copies[id(t)]['parent'] = copies.get(id(t['parent']))
        copies[id(t)]['children'] = [copies[id(c)] for c in t['children']]
    encoded, skipped, keys, parents = root['_recovery_scaffold']
    nodes = {encoded: copies[id(root)]}
    for i, k in enumerate(skipped):
        nodes[k] = dict(id=str(-1-i), node={'extras': {'first': str(k[0]), 'depth': k[2]}},
                        lo=k[3], hi=k[4], contents=[], external=False, children=[],
                        parent=None, _ownership_virtual=True)
    children = cell_children(root)
    for child, k in zip(children, keys):
        outer = child
        while outer['parent'] is not root:
            outer = outer['parent']
        nodes[k] = copies[id(outer)]
    nodes[encoded]['children'] = []
    for k in skipped + keys:
        nodes[k]['parent'] = nodes[parents[k]]
        nodes[parents[k]]['children'].append(nodes[k])
    for k in [encoded] + skipped:
        nodes[k]['children'].sort(key=lambda t: t['lo'])
    out, pending = [], [copies[id(tiles[0])]]
    while pending:
        t = pending.pop()
        out.append(t)
        pending.extend(reversed(t['children']))
    return out
