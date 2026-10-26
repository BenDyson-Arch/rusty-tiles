#!/usr/bin/env python3
"""Independent upward-omission, cover and cap checks of a delivered HLOD forest.

Uses only ``archive.expand`` and the payload bytes; producer reports are never
inputs, only compared afterwards. Rules (S1 756283e2 amendment, R9 floor):

- every payload holds at most 8,192 triangles over all its scene instances;
- a content-less cell has only content-less ancestors (explicit, implicit and
  external subtrees; link tiles are transparent placeholders, not cells);
- no branch ends content-less, and ADD content never stays drawn over its
  renderable descendants, so the first renderable cover holds every leaf once;
- the same payload bytes repeat only down a single-child chain;
- a content-less cell's geometricError is at least 16 x its box diagonal, so
  CesiumJS's base traversal (maximum SSE 16) refines it while it covers about
  a pixel. Children's error alone stops refinement there and draws nothing.

Omitted cells are counted by cell depth (placeholders add no depth); a source
is an omitted cell whose cell children all render.
"""
import argparse
import hashlib
import json
import math
import struct

from archive import Archive, expand

TRIANGLE_CAP = 8192
EMPTY_PARENT_SCALE = 16.0


class Failure(Exception):
    def __init__(self, check, detail):
        super().__init__(f'{check}: {detail}')
        self.check, self.detail = check, detail


def require(condition, check, detail):
    if not condition:
        raise Failure(check, detail)


def _index(value, uri):
    """A glTF index or count; integral numeric lexemes such as 1.0 are exact."""
    require(type(value) in (int, float) and value >= 0 and float(value).is_integer(), 'triangleCap',
            f'{uri}: non-integral glTF index {value!r}')
    return int(value)


def triangles(data, uri):
    """Published triangles of one GLB, over every scene instance of each mesh."""
    require(data[:4] == b'glTF' and len(data) >= 20, 'triangleCap', f'{uri}: not a GLB')
    length, kind = struct.unpack_from('<II', data, 12)
    require(kind == 0x4E4F534A and 20 + length <= len(data), 'triangleCap', f'{uri}: no JSON chunk')
    doc = json.loads(data[20:20 + length])
    nodes, meshes, accessors = doc.get('nodes', []), doc.get('meshes', []), doc.get('accessors', [])
    scene = doc.get('scenes', [{}])[_index(doc.get('scene', 0), uri)]
    total, stack, visits = 0, [_index(n, uri) for n in scene.get('nodes', [])], 0
    while stack:
        visits += 1
        require(visits <= 1_000_000, 'triangleCap', f'{uri}: scene walk limit')
        node = nodes[stack.pop()]
        require('EXT_mesh_gpu_instancing' not in node.get('extensions', {}), 'triangleCap',
                f'{uri}: instanced geometry unsupported')
        if 'mesh' in node:
            for p in meshes[_index(node['mesh'], uri)]['primitives']:
                require(_index(p.get('mode', 4), uri) == 4, 'triangleCap', f'{uri}: non-triangle primitive')
                accessor = p['indices'] if 'indices' in p else p['attributes']['POSITION']
                total += _index(accessors[_index(accessor, uri)]['count'], uri) // 3
        stack.extend(_index(c, uri) for c in node.get('children', []))
    return total


def diagonal(node):
    volume = node.get('boundingVolume', {})
    if 'box' in volume:
        return 2.0 * math.sqrt(sum(v * v for v in volume['box'][3:12]))
    if 'sphere' in volume:
        return 2.0 * volume['sphere'][3]
    return None


def inspect(tiles, archive):
    sizes, digests = {}, {}
    for t in tiles:
        for uri in t['contents']:
            if uri not in sizes:
                data = archive.read(uri)
                sizes[uri] = triangles(data, uri)
                digests[uri] = hashlib.sha256(data).digest()
                require(sizes[uri] <= TRIANGLE_CAP, 'triangleCap',
                        f'{uri} publishes {sizes[uri]} triangles, above the hard {TRIANGLE_CAP}')
    index = {id(t): i for i, t in enumerate(tiles)}
    # Reverse preorder: whether any strict descendant renders.
    below = {}
    for t in reversed(tiles):
        below[id(t)] = any(bool(c['contents']) or below[id(c)] for c in t['children'])
    state = {}
    omitted, sources, by_depth, frontier, covered = 0, 0, [], 0, 0
    for t in tiles:
        parent = t['parent']
        above, depth = (False, 0) if parent is None else (
            state[id(parent)][0], state[id(parent)][1] + (0 if parent['external'] else 1))
        content = bool(t['contents'])
        empty = not content and not t['external']
        require(not (empty and above), 'omissionPropagation',
                f'content-less cell {t["id"]} has a content ancestor: omission must reach the root')
        require(content or t['children'], 'frontierCoverage',
                f'terminal tile {t["id"]} publishes no content: the cover is incomplete')
        if empty:
            d = diagonal(t['node'])
            floor = None if d is None else EMPTY_PARENT_SCALE * d
            require(floor is not None and t['g'] >= floor * (1 - 1e-12), 'emptyCellRefinement',
                    f'content-less cell {t["id"]} has geometricError {t["g"]} below '
                    f'{EMPTY_PARENT_SCALE} x its box diagonal {d}: a viewer never refines it')
            omitted += 1
            by_depth.extend([0] * (depth + 1 - len(by_depth)))
            by_depth[depth] += 1
            # Cell children, resolving link placeholders to their roots.
            pending, empty_child = list(t['children']), False
            while pending:
                c = pending.pop()
                if c['external']:
                    pending.extend(c['children'])
                else:
                    empty_child |= not c['contents']
            sources += not empty_child
        if content:
            frontier += not above
            if not below[id(t)]:
                covered += 1
            else:
                require(t['refine'] == 'REPLACE', 'frontierCoverage',
                        f'ADD content of tile {t["id"]} stays drawn over its renderable descendants')
        state[id(t)] = (above or content, depth)
    # Single cover: equal bytes only down a single-child chain, never twice in a tile.
    groups = {}
    for t in tiles:
        seen = set()
        for uri in t['contents']:
            require(digests[uri] not in seen, 'frontierCoverage',
                    f'tile {t["id"]} references the same payload bytes twice')
            seen.add(digests[uri])
            groups.setdefault(digests[uri], []).append(t)
    for group in groups.values():
        group.sort(key=lambda t: index[id(t)])
        for above, below in zip(group, group[1:]):
            node = below
            while node is not above:
                parent = node['parent']
                require(parent is not None and len(parent['children']) == 1, 'frontierCoverage',
                        f'payload bytes of tile {above["id"]} repeat at tile {below["id"]} '
                        'outside a single-child chain')
                node = parent
    return dict(maxPayloadTriangles=max(sizes.values(), default=0), omittedCells=omitted,
                omissionSources=sources, omittedByDepth=by_depth, frontierTiles=frontier,
                frontierLeaves=covered)


def compare(derived, manifest):
    """Compare derived counts with a producer stage manifest's ``hlod`` report."""
    hlod = manifest.get('hlod', manifest)
    reported = dict(omittedCells=hlod.get('omittedProxies'), omissionSources=hlod.get('omissionSources'),
                    omittedByDepth=list(hlod.get('omittedByDepth') or []))
    while reported['omittedByDepth'] and reported['omittedByDepth'][-1] == 0:
        reported['omittedByDepth'].pop()
    differences = {k: dict(derived=derived[k], reported=v) for k, v in reported.items() if derived[k] != v}
    if hlod.get('maxTriangles') is not None and hlod['maxTriangles'] > TRIANGLE_CAP:
        differences['maxTriangles'] = dict(derived=derived['maxPayloadTriangles'], reported=hlod['maxTriangles'])
    return dict(ok=not differences, check='omissionReport', reported=reported, differences=differences)


def audit(path, producer_manifest=None):
    with Archive(path) as archive:
        tiles = expand(archive)
        require(bool(tiles), 'frontierCoverage', 'empty hierarchy')
        result = dict(ok=True, **inspect(tiles, archive))
    if producer_manifest is not None:
        with open(producer_manifest) as f:
            result['producerReport'] = compare(result, json.load(f))
        result['ok'] = result['producerReport']['ok']
    return result


if __name__ == '__main__':
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument('archive_or_directory')
    p.add_argument('--producer-manifest', help='S1 stage manifest.json to compare after derivation')
    a = p.parse_args()
    try:
        result = audit(a.archive_or_directory, a.producer_manifest)
    except Failure as e:
        result = dict(ok=False, check=e.check, detail=e.detail)
    except (ValueError, OSError) as e:
        result = dict(ok=False, check='omissionStructure', detail=str(e))
    print(json.dumps(result, sort_keys=True))
    raise SystemExit(0 if result['ok'] else 1)
