"""D1/D11 amendment A2: the constructive last resort, checked independently.

A coarse payload the producer built in last-resort stage N1-N3 declares
``asset.extras.rustyTiles.lastResort`` {stage, foldFaces}. This module re-derives it
from the delivered surfaces and source incidence alone:

- ``foldFaces`` is recounted from the decoded surface: faces of a coincident group (two
  or more on one corner set, either orientation) and faces using an edge that two or
  more faces use in the same direction. A mismatch is refused.
- From stage 2 those folds are admitted, and in a declared tile with such a stage below
  it (stage 0: it keeps them as input). Every other tile keeps ``borders.folds``; stage 0
  without a declared fold stage below is refused.
- Seam owner (every tile): a coarse face on three shared points that no leaf below
  holds, or a coarse edge between two shared points that is neither a leaf edge nor a
  delivered chord, must lie in its seam owner. A segment between published f32 points
  a < b (compared as little-endian u32 words) belongs to the tile holding a's least
  holding leaf, or its greatest when the top bit of FNV-1a 64 over the words of a then
  b is set; a face belongs to the owner of its least-to-greatest corner segment.
  Same-depth tiles then never both create it.
"""
import collections
import json
import struct


def declared(data, require):
    if data[:4] != b'glTF':
        return None
    length = int.from_bytes(data[12:16], 'little')
    doc = json.loads(data[20:20 + length])
    value = doc.get('asset', {}).get('extras', {}).get('rustyTiles', {}).get('lastResort')
    if value is None:
        return None
    require(isinstance(value, dict) and set(value) == {'stage', 'foldFaces'}
            and type(value['stage']) is int and 0 <= value['stage'] <= 3
            and type(value['foldFaces']) is int and value['foldFaces'] >= 0,
            'lastResort', 'lastResort is not {stage: 0-3, foldFaces: unsigned integer}')
    return value['stage'], value['foldFaces']


def declarations(tiles, archive, require):
    """Tile id -> (stage, foldFaces, admitted) for every declaring coarse tile; admitted:
    stage 2 or 3, or a declared stage 2 or 3 tile below it."""
    out = {}
    for t in tiles:
        found = [d for d in (declared(archive.read(u), require) for u in t['contents']) if d]
        if not found:
            continue
        require(bool(t['children']), 'lastResort', f'leaf tile {t["id"]} declares a last resort')
        require(len(set(found)) == 1, 'lastResort', f'tile {t["id"]}: payloads disagree')
        out[int(t['id'])] = found[0]
    below = {}

    def folds_below(t):
        tid = int(t['id'])
        if tid not in below:
            below[tid] = any(out.get(int(c['id']), (0, 0))[0] >= 2 or folds_below(c)
                             for c in t['children'])
        return below[tid]
    result = {}
    for t in tiles:
        tid = int(t['id'])
        if tid in out:
            stage, folds = out[tid]
            require(stage > 0 or folds_below(t), 'lastResort',
                    f'tile {tid}: stage 0 declares folds without a declared fold stage below')
            result[tid] = (stage, folds, stage >= 2 or folds_below(t))
    return result


def fold_faces(surface, directed):
    corners = collections.Counter(tuple(sorted(f)) for f in surface)
    return sum(1 for f in surface
               if corners[tuple(sorted(f))] > 1
               or any(directed[f[j], f[(j + 1) % 3]] > 1 for j in range(3)))


def words(xyz):
    return struct.unpack('<3I', struct.pack('<3f', *(0.0 if x == 0 else x for x in xyz)))


def greatest_side(a, b):
    h = 0xcbf29ce484222325
    for byte in struct.pack('<6I', *a, *b):
        h = ((h ^ byte) * 0x100000001b3) & 0xffffffffffffffff
    return h >> 63 == 1


def _leaf_face_index(store):
    if getattr(store, '_leaf_face_keys', False):
        return
    store.db.executescript('''
      CREATE TABLE leaf_face_keys AS
        SELECT min(ma.canonical,mb.canonical,mc.canonical) AS a,
               ma.canonical+mb.canonical+mc.canonical
                 -min(ma.canonical,mb.canonical,mc.canonical)
                 -max(ma.canonical,mb.canonical,mc.canonical) AS b,
               max(ma.canonical,mb.canonical,mc.canonical) AS c, s.leaf AS leaf
        FROM source s JOIN source_corners k ON k.source=s.id
          JOIN canonical_map ma ON ma.point=k.a JOIN canonical_map mb ON mb.point=k.b
          JOIN canonical_map mc ON mc.point=k.c;
      CREATE INDEX leaf_face_keys_abc ON leaf_face_keys(a,b,c,leaf);
    ''')
    store._leaf_face_keys = True


def check(store, tile, surface, directed, arcs, require, declaration):
    tid, lo, hi = int(tile['id']), tile['lo'], tile['hi']
    if declaration is not None:
        stage, want, _ = declaration
        got = fold_faces(surface, directed)
        require(got == want, 'lastResort', f'tile {tid}: declared {want} fold faces, decoded {got}')
    shared = set()
    for kind, a, b in store.db.execute(
            'SELECT kind,a,b FROM requirements WHERE tile=? AND kind<2', (tid,)).fetchall():
        store.work.charge()
        shared.add(a)
        if kind == 1:
            shared.add(b)
    if not shared:
        return
    ids = {store.canonical_xyz(p): p for p in shared}
    published = {p: words(store.canonical_xyz(p)) for p in shared}
    holders = {}

    def owns(p, q):
        p, q = sorted((p, q), key=published.get)
        if p not in holders:
            store.work.charge()
            holders[p] = store.db.execute('SELECT min(leaf),max(leaf) FROM canonical_owners WHERE point=?',
                                          (p,)).fetchone()
        leaf = holders[p][1 if greatest_side(published[p], published[q]) else 0]
        return leaf is not None and lo <= leaf < hi

    for f in surface:
        store.work.charge()
        if not all(p in ids for p in f):
            continue
        a, b, c = sorted(ids[p] for p in f)
        _leaf_face_index(store)
        source = store.db.execute(
            'SELECT EXISTS(SELECT 1 FROM leaf_face_keys WHERE a=? AND b=? AND c=? AND leaf>=? AND leaf<?)',
            (a, b, c, lo, hi)).fetchone()[0]
        span = sorted((a, b, c), key=published.get)
        require(source or owns(span[0], span[2]), 'seamOwner',
                f'tile {tid}: a new face on three shared points outside its seam owner')
    for p, q in {(min(p, q), max(p, q)) for p, q in directed}:
        store.work.charge()
        if p not in ids or q not in ids:
            continue
        a, b = sorted((ids[p], ids[q]))
        if (a, b) in arcs:
            continue
        source = store.db.execute(
            'SELECT EXISTS(SELECT 1 FROM canonical_edges WHERE a=? AND b=? AND leaf>=? AND leaf<?)',
            (a, b, lo, hi)).fetchone()[0]
        require(source or owns(a, b), 'seamOwner',
                f'tile {tid}: a new edge between shared points outside its seam owner')
