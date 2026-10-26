#!/usr/bin/env python3
"""D11 scalable-border controls: native and oracle verdicts on named mutants.

A literal flat fixture (no producer imports): cell A is x in [0, 4], cell B x in
[4, 8], z in [0, 4], each split into two leaves at z = 2 on a unit grid. Their
shared source arc P0..P4 runs along x = 4 with P2 moved off it, so contracting
the arc to P0-P4 displaces only P2. The audit sidecar is written honestly by hand
from the fixture; each mutant then changes geometry, tree or one sidecar claim.
With ``--binary`` a smooth sheet is also baked by the real producer: its
contracted borders must pass both validators unchanged.
"""
import argparse
import copy
import hashlib
import json
import math
from pathlib import Path
import struct
import subprocess
import sys

from controls import refinable

MATERIAL = {'pbrMetallicRoughness': {'baseColorFactor': [0.5, 0.5, 0.5, 1], 'metallicFactor': 0}}
EXTENT = 8.0


def f32(v):
    return struct.unpack('<f', struct.pack('<f', v))[0]


SHIFT = 0.0  # a rigid glTF-y translation of the whole fixture (review r1 finding 7)


def point(x, y, z):
    return (f32(x), f32(y + SHIFT), f32(z))


def key(p):
    return [0 if v == 0 else struct.unpack('<I', struct.pack('<f', v))[0] for v in p]


def scale(depth):
    return EXTENT / 1024. * 2.0 ** -depth


def oriented(face):
    """Every fixture face faces +y (glTF), so border directions are consistent."""
    a, b, c = face
    ny = (b[2] - a[2]) * (c[0] - a[0]) - (b[0] - a[0]) * (c[2] - a[2])
    return face if ny > 0 else (a, c, b)


def glb(faces, ids, *, leaf):
    """One primitive; source_primitive (and leaf source_triangle) identity tables."""
    binary = bytearray()
    views, accessors = [], []

    def accessor(data, component, shape, count, **extra):
        while len(binary) % 8:
            binary.append(0)
        views.append(dict(buffer=0, byteOffset=len(binary), byteLength=len(data)))
        binary.extend(data)
        accessors.append(dict(bufferView=len(views) - 1, componentType=component, type=shape, count=count, **extra))
        return len(accessors) - 1

    corners = [p for f in faces for p in f]
    position = accessor(b''.join(struct.pack('<3f', *p) for p in corners), 5126, 'VEC3', len(corners),
                        min=[min(p[k] for p in corners) for k in range(3)],
                        max=[max(p[k] for p in corners) for k in range(3)])
    attributes, sets, classes, tables = {'POSITION': position}, [], {}, []
    labels = ['source_primitive', 'source_triangle'] if leaf else ['source_primitive']
    for si, label in enumerate(labels):
        rows = [0] if si == 0 else list(range(len(faces)))
        per_vertex = [0.0] * len(corners) if si == 0 else [float(i) for i in range(len(faces)) for _ in range(3)]
        attributes[f'_FEATURE_ID_{si}'] = accessor(struct.pack('<' + 'f' * len(per_vertex), *per_vertex),
                                                   5126, 'SCALAR', len(per_vertex))
        columns = dict(node_index=[0] * len(rows), mesh_index=[0] * len(rows), primitive_index=[0] * len(rows),
                       expanded_ordinal=[0] * len(rows), material_index=[0] * len(rows))
        if si:
            columns.update(triangle_index=[ids[i][0] for i in rows],
                           **{f'vertex_{k}': [ids[i][1][k] for i in rows] for k in range(3)})
        schema, props = {}, {}
        for name, values in columns.items():
            wide = name in ('expanded_ordinal', 'triangle_index', 'vertex_0', 'vertex_1', 'vertex_2')
            data = struct.pack('<' + ('Q' if wide else 'I') * len(values), *values)
            schema[name] = dict(type='SCALAR', componentType='UINT64' if wide else 'UINT32', required=True)
            while len(binary) % 8:
                binary.append(0)
            views.append(dict(buffer=0, byteOffset=len(binary), byteLength=len(data)))
            binary.extend(data)
            props[name] = dict(values=len(views) - 1)
        classes[label] = dict(properties=schema)
        tables.append(dict(count=len(rows), **{'class': label}, properties=props))
        sets.append(dict(featureCount=len(rows), attribute=si, propertyTable=si, label=label))
    indices = accessor(struct.pack('<' + 'I' * len(corners), *range(len(corners))), 5125, 'SCALAR', len(corners))
    doc = dict(asset={'version': '2.0'}, scene=0, scenes=[{'nodes': [0]}], nodes=[{'mesh': 0}],
               meshes=[{'primitives': [dict(attributes=attributes, indices=indices, material=0,
                                            extensions={'EXT_mesh_features': {'featureIds': sets}})]}],
               materials=[MATERIAL], buffers=[{'byteLength': len(binary)}], bufferViews=views, accessors=accessors,
               extensionsUsed=['EXT_mesh_features', 'EXT_structural_metadata'],
               extensions={'EXT_structural_metadata': {'schema': {'classes': classes}, 'propertyTables': tables}})
    data = json.dumps(doc, separators=(',', ':')).encode()
    data += b' ' * (-len(data) % 4)
    binary += b'\0' * (-len(binary) % 4)
    return (b'glTF' + struct.pack('<III', 2, 28 + len(data) + len(binary), len(data)) + b'JSON' + data +
            struct.pack('<I', len(binary)) + b'BIN\0' + binary)


class Fixture:
    """Leaves, coarse payloads, tree and an honest sidecar for one variant."""

    def __init__(self, offset=0.0001):
        self.P = [point(4, 0, z) for z in range(5)]
        self.P[2] = point(4 + offset, 0, 2)
        self.extra = {}            # leaf name -> additional source faces
        self.hole = {}             # leaf name -> set of removed quads (i, j)
        self.annulus = None        # (leaf, quad, inner side)

    def vertex(self, i, j):
        return self.P[j] if i == 4 else point(i, 0, j)

    def quads(self, x0, x1, z0, z1):
        return [(i, j) for i in range(x0, x1) for j in range(z0, z1)]

    def quad_faces(self, i, j):
        v = self.vertex
        return [oriented((v(i, j), v(i, j + 1), v(i + 1, j))), oriented((v(i + 1, j), v(i, j + 1), v(i + 1, j + 1)))]

    def leaf_faces(self, name):
        x0, x1 = (0, 4) if name[0] == 'A' else (4, 8)
        z0, z1 = (0, 2) if name[1] == '0' else (2, 4)
        faces = []
        for q in self.quads(x0, x1, z0, z1):
            if q in self.hole.get(name, ()):
                continue
            if self.annulus and self.annulus[0] == name and self.annulus[1] == q:
                faces += self.ring(*q, self.annulus[2])
                continue
            faces += self.quad_faces(*q)
        return faces + self.extra.get(name, [])

    def ring(self, i, j, side):
        outer = [point(i, 0, j), point(i + 1, 0, j), point(i + 1, 0, j + 1), point(i, 0, j + 1)]
        c, h = (i + .5, j + .5), side / 2
        inner = [point(c[0] - h, 0, c[1] - h), point(c[0] + h, 0, c[1] - h),
                 point(c[0] + h, 0, c[1] + h), point(c[0] - h, 0, c[1] + h)]
        faces = []
        for k in range(4):
            faces += [oriented((outer[k], outer[(k + 1) % 4], inner[k])),
                      oriented((inner[k], outer[(k + 1) % 4], inner[(k + 1) % 4]))]
        return faces

    def inner(self):
        i, j = self.annulus[1]
        c, h = (i + .5, j + .5), self.annulus[2] / 2
        return [point(c[0] - h, 0, c[1] - h), point(c[0] + h, 0, c[1] - h),
                point(c[0] + h, 0, c[1] + h), point(c[0] - h, 0, c[1] + h)]

    def coarse(self, cell, keep=(0, 1, 2, 3, 4), patch=(), drop=(), fan=False):
        """Coarse A or B: its leaves' faces, the border column kept at `keep`."""
        leaves = [cell + '0', cell + '1']
        faces = [f for name in leaves for f in self.leaf_faces(name) if f not in drop]
        if tuple(keep) != (0, 1, 2, 3, 4) or fan:
            # Replace the unit column beside the border by a fan over the kept arc.
            lo, hi = (3, 4.01) if cell == 'A' else (4, 5)
            faces = [f for f in faces if not all(lo <= p[0] <= hi for p in f)]
            outer = [point(3, 0, z) for z in range(5)] if cell == 'A' else [point(5, 0, z) for z in range(5)]
            ring = outer + [self.P[k] for k in sorted(keep, reverse=True)]
            centre = point(3.5 if cell == 'A' else 4.5, 0, 2)
            for a, b in zip(ring, ring[1:] + ring[:1]):
                faces.append(oriented((centre, a, b)))
        return faces + [oriented(f) for f in patch]


def identities(faces):
    """Source triangle rows: (triangle index, original vertex ids by position)."""
    return [(0, [0, 0, 0])] * len(faces)


class Case:
    def __init__(self, name, oracle=None, native=None, offset=0.0001):
        self.name, self.oracle, self.native = name, oracle, native
        self.fx = Fixture(offset)


def build(case):
    """Write the variant's tileset, payloads, source manifest and audit sidecar."""
    fx, name = case.fx, case.name
    sheet_g = scale(1)
    keep_a = keep_b = (0, 1, 2, 3, 4)
    keep_m = None
    g_a = g_b = sheet_g
    patch, drop_a = [], []
    topology_a = {}
    tree = 'siblings'
    if name in ('contracted', 'eps-above-e16', 'eps-below-displacement', 'missing-subarc', 'wrong-ordinals',
                'wrong-scale', 'depth-label-lie', 'lock-claim-contracted', 'unknown-payload', 'omission-unreported',
                'missing-chord', 'contracted-fold', 'overlap-with-reverse', 'coincident-twin'):
        keep_a = keep_b = (0, 4)
    if name == 'same-depth-gap':
        keep_a = (0, 4)
    if name.startswith('mixed'):
        tree, keep_m = 'mixed', (0, 4)
        g_a, g_b = 0.004, 0.02
    if name == 'mixed-within':
        # A valid mixed depth: A's leaves lie below A' (depth 2), which contracts
        # exactly like its cousin M, so no leaf is as shallow as M.
        tree, g_a, g_b = 'mixed-deep', sheet_g, sheet_g
        keep_a = keep_b = (0, 4)
        keep_m = (0, 2, 4)
    if name.startswith('terminal-'):
        tree, keep_m = 'terminal', (0, 4) if name == 'terminal-contracted' else (0, 1, 2, 3, 4)
    if name.startswith('dropped'):
        size = 0.1 if name == 'dropped-large' else 0.001
        if name == 'dropped-border':
            tiny = (point(5, 0, 1), point(5 + size, 0, 1), point(5, 0, 1 + size))
        else:
            tiny = (point(2, 0.5, 1), point(2 + size, 0.5, 1), point(2, 0.5, 1 + size))
        fx.extra['A0'] = [oriented(tiny)]
        drop_a = [oriented(tiny)]
        if name != 'dropped-border':
            floor = max(scale(1), 0.0)
            increase = max(1.05 * 0.5 - floor, 0.0)
            g_a = 0.6
            topology_a = dict(droppedComponents=1, droppedComponentToProxy=0.5,
                              droppedComponentGIncrease=increase, droppedComponentFinalG=g_a,
                              maxDroppedComponentDiameter=size * math.sqrt(2))
            if name == 'dropped-unreported':
                topology_a = {}
            if name == 'dropped-tampered-g':
                topology_a['droppedComponentGIncrease'] = 0.1
    if name.startswith('closure'):
        if name == 'closure-small':
            fx.annulus = ('A0', (1, 0), 0.004)
            inner = fx.inner()
            patch = [(inner[0], inner[1], inner[2]), (inner[0], inner[2], inner[3])]
            loss = 0.0021  # the square centre is 0.002 from source
        else:
            fx.hole['A1'] = {(1, 2)}
            q = [point(1, 0, 2), point(2, 0, 2), point(2, 0, 3), point(1, 0, 3)]
            patch = [(q[0], q[1], q[2]), (q[0], q[2], q[3])]
            loss = 0.34 if name == 'closure-loss-reported' else 0.001
        topology_a = dict(closedHoles=1, holePatchToSource=loss, closedHoleFinalG=g_a)
    # Leaves and coarse payloads.
    leaves = {n: fx.leaf_faces(n) for n in ('A0', 'A1', 'B0', 'B1')}
    vertex_ids = {}
    for n in ('A0', 'A1', 'B0', 'B1'):
        for f in leaves[n]:
            for p in f:
                vertex_ids.setdefault(p, len(vertex_ids))
    triangle = 0
    rows, source = {}, {}
    for n in ('A0', 'A1', 'B0', 'B1'):
        rows[n] = []
        for f in leaves[n]:
            ids = [vertex_ids[p] for p in f]
            rows[n].append((triangle, ids))
            source[f'0,0,0,{triangle}'] = dict(xyz=[list(p) for p in f], vertices=ids, material=MATERIAL,
                                               expanded_ordinal=0, material_index=0)
            triangle += 1
    a = fx.coarse('A', keep_a, patch, drop_a)
    if name == 'missing-chord':
        a = [tuple(point(4.05, 0, 2) if p == fx.P[4] and False else p for p in f) for f in a]
        a = [f for f in a if not (fx.P[0] in f and fx.P[4] in f)]
        a.append(oriented((point(3.5, 0, 2), fx.P[0], point(4.05, 0, 2))))
        a.append(oriented((point(3.5, 0, 2), point(4.05, 0, 2), fx.P[4])))
    if name == 'contracted-fold':
        # S1b review 1: a contraction fold re-flipped onto the chord's side.
        chord = next(f for f in a if fx.P[0] in f and fx.P[4] in f)
        a.append(tuple(point(3.25, 0, 2) if p not in (fx.P[0], fx.P[4]) else p for p in chord))
    if name == 'exact-fold':
        a.append(a[1])
    if name == 'overlap-with-reverse':
        a.append(a[0])
    if name == 'coincident-twin':
        a.append((a[0][0], a[0][2], a[0][1]))
    b = fx.coarse('B', keep_b)
    files = {f'{n}.glb': glb(leaves[n], rows[n], leaf=True) for n in leaves}
    files['A.glb'] = glb(a, identities(a), leaf=False)
    files['B.glb'] = glb(b, identities(b), leaf=False)
    box = {'box': [4, -2, 0, 10, 0, 0, 0, 10, 0, 0, 0, 10]}

    def tile(uri, g, children=None):
        node = dict(boundingVolume=box, geometricError=g, refine='REPLACE')
        if uri:
            node['content'] = {'uri': uri}
        if children:
            node['children'] = children
        return node
    leaf = lambda n: tile(f'{n}.glb', 0)
    records = []
    depth_a = depth_b = 1
    if tree == 'siblings':
        root = tile(None, 0, [tile('A.glb', g_a, [leaf('A0'), leaf('A1')]), tile('B.glb', g_b, [leaf('B0'), leaf('B1')])])
        intervals = {'A': (0, 2), 'B': (2, 4)}
    elif tree == 'mixed-deep':
        m, a2 = fx.coarse('B', keep_m), fx.coarse('A', keep_m)
        files['M.glb'] = glb(m, identities(m), leaf=False)
        files['A2.glb'] = glb(a2, identities(a2), leaf=False)
        root = tile(None, 0, [tile('A.glb', g_a, [tile('A2.glb', 0.002, [leaf('A0'), leaf('A1')])]),
                              tile('B.glb', g_b, [tile('M.glb', 0.002, [leaf('B0'), leaf('B1')])])])
        intervals = {'A': (0, 2), 'A2': (0, 2), 'B': (2, 4), 'M': (2, 4)}
    elif tree == 'mixed':
        m = fx.coarse('B', keep_m)
        files['M.glb'] = glb(m, identities(m), leaf=False)
        root = tile(None, 0, [tile('A.glb', g_a, [leaf('A0'), leaf('A1')]),
                              tile('B.glb', g_b, [tile('M.glb', g_b, [leaf('B0'), leaf('B1')])])])
        intervals = {'A': (0, 2), 'B': (2, 4), 'M': (2, 4)}
    else:
        files['L.glb'] = glb(leaves['A0'] + leaves['A1'], rows['A0'] + rows['A1'], leaf=True)
        for n in ('A0', 'A1', 'A.glb'):
            files.pop(f'{n}.glb' if not n.endswith('.glb') else n)
        m = fx.coarse('B', keep_m)
        files['M.glb'] = glb(m, identities(m), leaf=False)
        fanned = fx.coarse('B', fan=True)
        files['B.glb'] = glb(fanned, identities(fanned), leaf=False)
        root = tile(None, 0, [tile('L.glb', 0), tile('B.glb', g_b, [tile('M.glb', g_b, [leaf('B0'), leaf('B1')])])])
        intervals = {'B': (1, 3), 'M': (1, 3)}
    doc = refinable(dict(asset={'version': '1.1'}, geometricError=0, root=root))

    def record(cell, uri, depth, g, keep, topology=None, terminal=False, child_keep=(0, 1, 2, 3, 4)):
        data = files[uri]
        arcs = list(zip(keep, keep[1:]))
        certificates = []
        for first, end in arcs:
            a_, b_ = fx.P[first], fx.P[end]
            eps = 0.0
            for i in range(first + 1, end):
                t = (i - first) / (end - first)
                q = [a_[k] + (b_[k] - a_[k]) * t for k in range(3)]
                eps = max(eps, math.dist(q, fx.P[i]))
            certificates.append(dict(endpoints=[key(a_), key(b_)], chain=[key(fx.P[0]), key(fx.P[1])],
                                     origin=key(fx.P[0]), first=first, end=end,
                                     epsilon=eps * (1 + 1e-6) if eps else 0.0, terminalLock=terminal))
        interface = [dict(leaf=0, a=key(fx.P[k]), b=key(fx.P[k]), lo=0, hi=3) for k in keep]
        interface += [dict(leaf=0, a=key(fx.P[f]), b=key(fx.P[e]), lo=0, hi=3) for f, e in arcs]
        if name == 'dropped-border' and cell == 'A':
            p = point(5, 0, 1)
            interface.append(dict(leaf=0, a=key(p), b=key(p), lo=0, hi=3))
            certificates.append(dict(endpoints=[key(p), key(p)], chain=[key(p), key(p)], origin=key(p),
                                     first=0, end=0, epsilon=0.0, terminalLock=False))
        receipt = dict.fromkeys(TOPOLOGY, 0)
        receipt.update({k: 0.0 for k in TOPOLOGY if k[0].islower() and any(s in k for s in ('Source', 'Patch', 'Proxy', 'Increase', 'FinalG', 'Diameter'))})
        # Input (leaf) vertices this tile contracts onto its chords.
        receipt['contractedVertices'] = len(set(child_keep) - set(keep))
        receipt.update(topology or {})
        lo, hi = intervals[cell]
        return dict(schemaVersion=1, kind='tile',
                    tile=dict(uri=uri, sha256=hashlib.sha256(data).hexdigest(), bytes=len(data), leafBegin=lo,
                              leafEnd=hi, depth=depth, geometricError=g, topology=receipt,
                              attemptedTopology=copy.deepcopy(receipt)),
                    contract=dict(depth=depth, scale=scale(depth), interface=interface, certificates=certificates,
                                  terminalVertices=[key(fx.P[k]) for k in keep] if terminal else []))
    if tree == 'terminal':
        records.append(record('B', 'B.glb', 1, g_b, (0, 1, 2, 3, 4), terminal=True))
        records.append(record('M', 'M.glb', 2, g_b, keep_m, terminal=name != 'terminal-contracted'
                              and name != 'terminal-unclaimed'))
        frontier = [dict(leafBegin=0, leafEnd=1), dict(leafBegin=1, leafEnd=3)]
        total = 3
    else:
        below = keep_m if tree == 'mixed-deep' else (0, 1, 2, 3, 4)
        records.append(record('A', 'A.glb', depth_a, g_a, keep_a, topology_a, child_keep=below))
        records.append(record('B', 'B.glb', depth_b, g_b, keep_b, child_keep=below))
        if tree == 'mixed':
            records.append(record('M', 'M.glb', 2, g_b, keep_m))
        if tree == 'mixed-deep':
            records.append(record('A2', 'A2.glb', 2, 0.002, keep_m))
            records.append(record('M', 'M.glb', 2, 0.002, keep_m))
        frontier = [dict(leafBegin=0, leafEnd=2), dict(leafBegin=2, leafEnd=4)]
        total = 4
    records.append(dict(schemaVersion=1, kind='omitted', key=dict(first=0, end=triangle, depth=0, leafBegin=0,
                                                                   leafEnd=total),
                        reason='contract_lower_bound', source=True, frontier=frontier, attemptedTopology={}))
    mutate(name, records)
    return doc, files, dict(triangles=source), records


TOPOLOGY = ['contractedVertices', 'reconstructedFaces', 'duplicateFaces', 'droppedComponents', 'closedHoles',
            'openedHoles', 'remeshedVertices', 'remeshRounds', 'constrainedRemeshes', 'rejectedRemeshRounds',
            'rejectedHoleClosures', 'rejectedHoleOpenings', 'maxDroppedComponentDiameter', 'holePatchToSource',
            'holeBoundaryToPatch', 'droppedComponentToProxy', 'openedHoleToProxy', 'openedHoleBoundaryToSource',
            'holeGIncrease', 'droppedComponentGIncrease', 'openedHoleGIncrease', 'closedHoleFinalG',
            'openedHoleFinalG', 'droppedComponentFinalG']


def mutate(name, records):
    """Sidecar-only lies: the archive is unchanged, so native validation passes."""
    a = records[0]
    contracted = next((c for c in a['contract']['certificates'] if c['end'] - c['first'] > 1), None)
    if name == 'eps-above-e16':
        contracted['epsilon'] = a['contract']['scale'] / 8
    elif name == 'eps-below-displacement':
        contracted['epsilon'] /= 4
    elif name == 'missing-subarc':
        a['contract']['certificates'].remove(contracted)
    elif name == 'wrong-ordinals':
        contracted['end'] -= 1
    elif name == 'wrong-scale':
        a['contract']['scale'] *= 2
    elif name == 'depth-label-lie':
        a['contract']['depth'] = a['tile']['depth'] = 0
    elif name == 'lock-claim-contracted':
        contracted['terminalLock'] = True
    elif name == 'unknown-payload':
        a['tile']['sha256'] = '0' * 64
    elif name == 'omission-unreported':
        records.pop()


CASES = [
    # Positives: honest contraction, closure accounting and exact terminal locks.
    ('contracted', None, None), ('mixed-within', None, None), ('terminal-exact', None, None),
    ('closure-small', None, None), ('dropped-small', None, None),
    # Geometry mutants: both validators where the archive alone shows them.
    ('same-depth-gap', 'sameDepthBorderGap', 'sameDepthBorderGap'),
    ('mixed-beyond-g8', 'mixedDepthBorderGap', 'mixedDepthBorderGap'),
    # M contracts against A's depth-2 leaves; only the oracle orders those cousins.
    ('mixed-unresolved-terminal', 'terminalBorderLock', None),
    ('terminal-contracted', 'terminalBorderLock', 'terminalBorderLock'),
    ('missing-chord', 'interfaceSegments', 'interfaceSegments'),
    ('contracted-fold', 'interfaceWinding', 'interfaceWinding'),
    ('exact-fold', 'interfaceWinding', 'interfaceWinding'),
    ('overlap-with-reverse', 'interfaceWinding', 'interfaceWinding'),
    ('coincident-twin', 'interfaceWinding', 'interfaceWinding'),
    ('dropped-border', 'interfacePoints', 'interfacePoints'),
    ('dropped-large', 'droppedComponent', None),
    ('closure-loss-hidden', 'holeClosure', None),
    ('closure-loss-reported', 'topologyReceipt', None),
    # Sidecar lies: the oracle re-derives and refuses; native never reads the sidecar.
    ('eps-above-e16', 'borderCertificate', None), ('eps-below-displacement', 'borderCertificate', None),
    ('missing-subarc', 'borderCertificate', None), ('wrong-ordinals', 'borderCertificate', None),
    ('wrong-scale', 'borderScale', None), ('depth-label-lie', 'borderScale', None),
    ('lock-claim-contracted', 'terminalBorderLock', None), ('terminal-unclaimed', 'terminalBorderLock', None),
    ('unknown-payload', 'borderAudit', None), ('omission-unreported', 'omissionReport', None),
    ('dropped-unreported', 'topologyReceipt', None), ('dropped-tampered-g', 'topologyReceipt', None),
]
OFFSETS = {'mixed-beyond-g8': 0.001}


def _rehash(records, files):
    for r in records:
        if r['kind'] == 'tile':
            data = files[r['tile']['uri']]
            r['tile']['sha256'], r['tile']['bytes'] = hashlib.sha256(data).hexdigest(), len(data)


def review(name):
    """Sol's review-1 fixtures (hlod-s5-d11-review1-20261026), each now permanent."""
    global SHIFT
    if name == 'drop-loss-lie':
        # The dropped component lies 0.5 above the proxy; the receipt claims zero.
        doc, files, source, records = build(Case('dropped-small'))
        row, g = records[0]['tile'], scale(1)
        row['geometricError'] = g
        for label in ('topology', 'attemptedTopology'):
            row[label].update(droppedComponentToProxy=0.0, droppedComponentGIncrease=0.0,
                              droppedComponentFinalG=g)
        doc['root']['children'][0]['geometricError'] = g
        return doc, files, source, records
    if name == 'shared-corner-hole':
        # Two holes meet at (2, 0, 2); filling one leaves a rank-1 boundary.
        case = Case(name)
        case.fx.hole = {'A0': {(1, 1)}, 'A1': {(2, 2)}}
        doc, files, source, records = build(case)
        a = case.fx.coarse('A', patch=case.fx.quad_faces(1, 1))
        files['A.glb'] = glb(a, identities(a), leaf=False)
    elif name in ('unrelated-license', 'unlicensed-twins'):
        # A back-to-back source plate elsewhere licenses no new coarse twin.
        case = Case('contracted')
        if name == 'unrelated-license':
            t = (point(1, .1, 1), point(1.001, .1, 1), point(1, .1, 1.001))
            case.fx.extra = {'A0': [t, (t[0], t[2], t[1])]}
        doc, files, source, records = build(case)
        a = case.fx.coarse('A', (0, 4))
        t = (point(1.1, 0, 1.1), point(1.2, 0, 1.1), point(1.1, 0, 1.2))
        a += [t, (t[0], t[2], t[1]), t, (t[0], t[2], t[1])]
        files['A.glb'] = glb(a, identities(a), leaf=False)
    elif name == 'empty-omission-frontier':
        doc, files, source, records = build(Case('contracted'))
        records[-1]['frontier'] = []
        return doc, files, source, records
    elif name == 'invented-contracted-vertices':
        doc, files, source, records = build(Case('contracted'))
        records[0]['tile']['topology']['contractedVertices'] = 999999
        return doc, files, source, records
    elif name == 'uninspected-history-counters':
        # History-dependent counters are listed as uninspected, not compared.
        doc, files, source, records = build(Case('contracted'))
        records[0]['tile']['topology']['duplicateFaces'] = 999999
        return doc, files, source, records
    elif name == 'coherent-depth-lie':
        # Sibling B relabelled depth 2 with a matching scale: same-depth A/B disagree.
        doc, files, source, records = build(Case('same-depth-gap'))
        records[1]['tile']['depth'] = records[1]['contract']['depth'] = 2
        records[1]['contract']['scale'] = scale(2)
        return doc, files, source, records
    elif name == 'unknown-terminal-contact':
        # Terminal leaf L below an omitted cell: depth unresolved, M contracts it.
        doc, files, source, records = build(Case('terminal-contracted'))
        leaf = doc['root']['children'][0]
        doc['root']['children'][0] = dict(boundingVolume=copy.deepcopy(leaf['boundingVolume']), geometricError=0,
                                          refine='REPLACE', children=[leaf])
        refinable(doc)
        root = records[-1]
        root.update(source=False, reason='descendant_omission', frontier=[dict(leafBegin=1, leafEnd=3)])
        records.append(dict(schemaVersion=1, kind='omitted', key=dict(first=0, end=32, depth=1, leafBegin=0, leafEnd=1),
                            reason='contract_lower_bound', source=True, frontier=[dict(leafBegin=0, leafEnd=1)],
                            attemptedTopology={}))
        return doc, files, source, records
    elif name == 'translated-contraction':
        SHIFT = 1e12
        try:
            doc, files, source, records = build(Case('contracted'))
        finally:
            SHIFT = 0.0

        def volume(node):
            node['boundingVolume']['box'][2] = f32(1e12)
            for child in node.get('children', []):
                volume(child)
        volume(doc['root'])
        return doc, files, source, records
    elif name == 'steiner-hole-closure':
        # The unit hole closed by a fan through a new vertex, receipt claiming 0.001.
        case = Case('closure-loss-hidden')
        doc, files, source, records = build(case)
        q = [point(1, 0, 2), point(2, 0, 2), point(2, 0, 3), point(1, 0, 3)]
        c = point(1.5, 0, 2.5)
        a = case.fx.coarse('A', patch=[(q[k], q[(k + 1) % 4], c) for k in range(4)])
        files['A.glb'] = glb(a, identities(a), leaf=False)
    elif name == 'unmeasured-hole-opening':
        # A unit hole opened in A; its rim lies on source, its lost centre 1/2 away.
        case = Case(name)
        doc, files, source, records = build(case)
        remove = case.fx.quad_faces(1, 2)
        a = [f for f in case.fx.coarse('A') if f not in remove]
        files['A.glb'] = glb(a, identities(a), leaf=False)
        for label in ('topology', 'attemptedTopology'):
            records[0]['tile'][label].update(openedHoles=1, openedHoleFinalG=scale(1))
    _rehash(records, files)
    return doc, files, source, records


def review2(name):
    """Sol's review-2 fixtures (hlod-s5-d11-review2-20261026), each now permanent."""
    global EXTENT

    def boxes(doc, box):
        def walk(t):
            t['boundingVolume'] = {'box': list(box)}
            for c in t.get('children', []):
                walk(c)
        walk(doc['root'])
        refinable(doc)
    if name in ('page-crossed', 'page-same-side', 'page-retained'):
        # A source page on edge a -> b. Crossed: the page lies on the other side and
        # a new coarse face overlaps the sheet face on this side. Same side: the
        # page lies 0.001 above where the coarse face is, a matched simplification.
        case = Case(name)
        a, b, c = case.fx.quad_faces(1, 1)[0]
        third = point(.5, .001, 1.5) if name == 'page-crossed' else point(1.5, .001, 1.5)
        page = (a, b, third)
        case.fx.extra = {'A0': [page]}
        doc, files, source, records = build(case)
        out = [f for f in case.fx.coarse('A') if f != page]
        out.append(page if name == 'page-retained' else (a, b, point(1.5, 0, 1.5)))
        files['A.glb'] = glb(out, identities(out), leaf=False)
    elif name == 'rim-vertex-removed':
        # Half a unit hole filled and its rim vertex b retriangulated away: the
        # cycle count is unchanged, the coarse patch lies 1/3 from source.
        case = Case(name)
        case.fx.hole = {'A1': {(1, 2)}}
        doc, files, source, records = build(case)
        a, b, c = point(1, 0, 2), point(2, 0, 2), point(2, 0, 3)
        out = [f for f in case.fx.coarse('A') if b not in f]
        ring = [a, point(2, 0, 1), point(3, 0, 1), point(3, 0, 2), c]
        out += [oriented((ring[0], ring[j], ring[j + 1])) for j in range(1, 4)]
        files['A.glb'] = glb(out, identities(out), leaf=False)
    elif name == 'simultaneous-close-open':
        # Two small holes sharing a corner in a raised patch: one closes, one opens;
        # the receipt claims neither.
        case = Case(name)
        coords, h = [1, 1.5 - 1 / 512, 1.5, 1.5 + 1 / 512, 2], 1 / 512

        def patch(missing):
            fs = []
            for i in range(4):
                for j in range(4):
                    if (i, j) == missing:
                        continue
                    v = [point(coords[i], .01, coords[j]), point(coords[i + 1], .01, coords[j]),
                         point(coords[i + 1], .01, coords[j + 1]), point(coords[i], .01, coords[j + 1])]
                    fs += [oriented((v[0], v[1], v[2])), oriented((v[0], v[2], v[3]))]
            return fs
        case.fx.extra = {'A0': patch((1, 1))}
        doc, files, source, records = build(case)
        out = [f for f in case.fx.coarse('A') if f not in case.fx.extra['A0']] + patch((2, 2))
        files['A.glb'] = glb(out, identities(out), leaf=False)
    elif name in ('remote-drop', 'loss-between-samples'):
        if name == 'remote-drop':
            # A tiny component 1000 above the sheet: exact loss 1000, G 1050.
            EXTENT = 1000.
            drop = oriented((point(2, 1000, 1), point(2.001, 1000, 1), point(2, 1000, 1.001)))
            extra, g = [drop], 1050.
        else:
            # The producer's sixteen controls miss the loss at p = (1/8, 1/8, 3/4)
            # (0.0039986 > G = E1); this checker's level-8 grid samples it.
            length, rad = .0025, .004
            drop = oriented((point(2, .5, 1), point(2 + length, .5, 1), point(2, .5, 1 + length)))
            centre = point(2 + length / 8, .5, 1 + length / 8)
            ring = set()
            for q in sorted(set(tuple((drop[0][k] * a_ + drop[1][k] * b_ + drop[2][k] * (4 - a_ - b_)) / 4
                                      for k in range(3)) for a_ in range(5) for b_ in range(5 - a_)) |
                            {tuple(sum(p[k] for p in drop) / 3 for k in range(3))}):
                dx, dz = q[0] - centre[0], q[2] - centre[2]
                n = math.hypot(dx, dz)
                ring.add(point(centre[0] + rad * dx / n, .5, centre[2] + rad * dz / n))
            extra = [drop] + [oriented((point(x - 1e-6, y, z - 1e-6), point(x + 1e-6, y, z - 1e-6), point(x, y, z + 1e-6)))
                              for x, y, z in sorted(ring)]
            g = None
        try:
            case = Case(name)
            case.fx.extra = {'A0': extra}
            doc, files, source, records = build(case)
        finally:
            EXTENT = 8.0
        out = [f for f in case.fx.coarse('A') if f != drop]
        files['A.glb'] = glb(out, identities(out), leaf=False)
        diameter = math.sqrt(sum((max(p[k] for p in drop) - min(p[k] for p in drop)) ** 2 for k in range(3)))
        e = 1000. / 1024 / 2 if name == 'remote-drop' else scale(1)
        if name == 'remote-drop':
            loss = 1000.
            doc['root']['children'][0]['geometricError'] = records[0]['tile']['geometricError'] = g
            boxes(doc, [4, -2, 500, 2000, 0, 0, 0, 2000, 0, 0, 0, 2000])
        else:
            loss, g = 0.0035573626656298612, e  # the producer's controls only
        for label in ('topology', 'attemptedTopology'):
            records[0]['tile'][label].update(droppedComponents=1, droppedComponentToProxy=loss,
                                             droppedComponentGIncrease=max(1.05 * loss - e, 0.0),
                                             droppedComponentFinalG=g, maxDroppedComponentDiameter=diameter)
        if name == 'remote-drop':
            for r in records:
                if r['kind'] == 'tile':
                    r['contract']['scale'] = 1000. / 1024 / 2 ** r['contract']['depth']
    elif name in ('depth-fraction', 'depth-boolean'):
        doc, files, source, records = build(Case('contracted'))
        depth = 1.5 if name == 'depth-fraction' else True
        for r in records:
            if r['kind'] == 'tile':
                r['tile']['depth'] = r['contract']['depth'] = depth
                r['contract']['scale'] = EXTENT / 1024. * 2.0 ** -depth
        return doc, files, source, records
    elif name == 'depth-ancestor':
        # A labelled at depth 1 below an omitted cell also labelled depth 1.
        doc, files, source, records = build(Case('mixed-within'))
        node = doc['root']['children'][0]
        doc['root']['children'][0] = dict(boundingVolume=copy.deepcopy(node['boundingVolume']),
                                          geometricError=node['geometricError'], refine='REPLACE', children=[node])
        records[-1].update(source=False, reason='descendant_omission', frontier=[dict(leafBegin=2, leafEnd=4)])
        records.append(dict(schemaVersion=1, kind='omitted', key=dict(first=0, end=32, depth=1, leafBegin=0, leafEnd=2),
                            reason='contract_lower_bound', source=True, frontier=[dict(leafBegin=0, leafEnd=2)],
                            attemptedTopology={}))
        refinable(doc)
        return doc, files, source, records
    elif name in ('frontier-duplicate', 'frontier-subset'):
        doc, files, source, records = build(Case('contracted'))
        if name == 'frontier-duplicate':
            records[-1]['frontier'].append(copy.deepcopy(records[-1]['frontier'][0]))
        else:
            records[-1]['frontier'].pop()
        return doc, files, source, records
    elif name == 'zero-epsilon-collinear':
        # Equally spaced collinear P0..P4 contracted to P0-P4: exact image, epsilon 0.
        return build(Case('contracted', offset=0.))
    _rehash(records, files)
    return doc, files, source, records


REVIEW2 = [
    ('page-crossed', 'interfaceWinding', 'interfaceWinding'),
    ('page-same-side', None, None),
    ('page-retained', None, None),
    ('rim-vertex-removed', 'holeClosure', None),
    ('simultaneous-close-open', 'topologyReceipt', None),
    ('remote-drop', None, None),
    ('loss-between-samples', 'topologyReceipt', None),
    ('depth-fraction', 'borderDepth', None),
    ('depth-boolean', 'borderDepth', None),
    ('depth-ancestor', 'borderDepth', None),
    ('frontier-duplicate', 'omissionReport', None),
    ('frontier-subset', 'omissionReport', None),
    ('zero-epsilon-collinear', None, None),
]


REVIEW = [
    ('drop-loss-lie', 'topologyReceipt', None),
    ('shared-corner-hole', 'holeClosure', None),
    ('unrelated-license', 'interfaceWinding', 'interfaceWinding'),
    ('unlicensed-twins', 'interfaceWinding', 'interfaceWinding'),
    ('empty-omission-frontier', 'omissionReport', None),
    ('invented-contracted-vertices', 'topologyReceipt', None),
    ('uninspected-history-counters', None, None),
    ('coherent-depth-lie', 'borderDepth', 'sameDepthBorderGap'),
    ('unknown-terminal-contact', 'terminalBorderLock', None),
    ('translated-contraction', None, None),
    ('steiner-hole-closure', 'holeClosure', None),
    ('unmeasured-hole-opening', 'holeOpening', None),
]


def generate(root):
    root = Path(root)
    root.mkdir(parents=True, exist_ok=True)
    out = []
    for name, oracle_check, native_check in CASES + REVIEW + REVIEW2:
        if any(name == r[0] for r in REVIEW2):
            doc, files, source, records = review2(name)
        elif any(name == r[0] for r in REVIEW):
            doc, files, source, records = review(name)
        else:
            doc, files, source, records = build(Case(name, oracle_check, native_check, OFFSETS.get(name, 0.0001)))
        folder = root / name
        folder.mkdir(exist_ok=True)
        for member, data in files.items():
            (folder / member).write_bytes(data)
        (folder / 'tileset.json').write_text(json.dumps(doc))
        (root / f'{name}.source.json').write_text(json.dumps(source, sort_keys=True))
        (root / f'{name}.borders.jsonl').write_text(''.join(json.dumps(r) + '\n' for r in records))
        out.append(dict(name=name, oracleCheck=oracle_check, archiveCheck=native_check))
    return out


def sheet(path, n):
    """S1b's smooth two-material bump sheet at 2n x 2n quads per half."""
    def bump(x, z):
        u, v = (x - 50.) / 30., (z - 50.) / 30.
        return 4. / (1. + u * u + v * v) + 0.01 * x
    binary, views, accessors, meshes = bytearray(), [], [], []
    for half in range(2):
        x0 = 50. * half
        pos = [(f32(x0 + 50. * i / n), 0., f32(100. * j / (2 * n))) for i in range(n + 1) for j in range(2 * n + 1)]
        pos = [(x, f32(bump(x, z)), z) for x, _, z in pos]
        w = 2 * n + 1
        idx = [v for i in range(n) for j in range(2 * n) for v in
               (i * w + j, i * w + j + 1, (i + 1) * w + j, (i + 1) * w + j, i * w + j + 1, (i + 1) * w + j + 1)]
        for data, kind, count, shape, extra in (
                (b''.join(struct.pack('<3f', *p) for p in pos), 5126, len(pos), 'VEC3',
                 dict(min=[min(p[k] for p in pos) for k in range(3)], max=[max(p[k] for p in pos) for k in range(3)])),
                (struct.pack('<' + 'I' * len(idx), *idx), 5125, len(idx), 'SCALAR', {})):
            views.append(dict(buffer=0, byteOffset=len(binary), byteLength=len(data)))
            binary.extend(data)
            accessors.append(dict(bufferView=len(views) - 1, componentType=kind, count=count, type=shape, **extra))
        meshes.append(dict(primitives=[dict(attributes=dict(POSITION=len(accessors) - 2), indices=len(accessors) - 1,
                                            material=half)]))
    doc = dict(asset={'version': '2.0'}, scene=0, scenes=[{'nodes': [0, 1]}], nodes=[{'mesh': 0}, {'mesh': 1}],
               meshes=meshes, materials=[MATERIAL, {'pbrMetallicRoughness': {'baseColorFactor': [.3, .6, .3, 1]}}],
               buffers=[{'byteLength': len(binary)}], bufferViews=views, accessors=accessors)
    data = json.dumps(doc).encode()
    data += b' ' * (-len(data) % 4)
    binary += b'\0' * (-len(binary) % 4)
    Path(path).write_bytes(b'glTF' + struct.pack('<III', 2, 28 + len(data) + len(binary), len(data)) + b'JSON' +
                           data + struct.pack('<I', len(binary)) + b'BIN\0' + binary)


def native(binary, folder, archive):
    packed = subprocess.run([str(binary), 'convert', '-i', str(folder), '-o', str(archive), '--force', '--json'],
                            capture_output=True, timeout=60)
    assert packed.returncode == 0, (folder, packed.stdout[-2000:])
    checked = subprocess.run([str(binary), 'validate', str(archive), '--json', '--expect-profile', 'hlod'],
                             capture_output=True, timeout=120)
    return checked.returncode, json.loads(checked.stdout)


def run(root, binary=None):
    from oracle import audit, Failure
    root = Path(root)
    results = []
    for case in generate(root):
        name = case['name']
        try:
            result = audit(root / name, root / f'{name}.source.json', border_audit=root / f'{name}.borders.jsonl')
            got = None
            assert result['borders']['audited'], name
        except Failure as e:
            got, result = e.check, dict(detail=e.detail)
        assert got == case['oracleCheck'], (name, case['oracleCheck'], got, result.get('detail'))
        entry = dict(name=name, oracle=got)
        if name in ('contracted', 'mixed-within', 'same-depth-gap'):
            assert result.get('contractedBorderArcs', 0) > 0 or got, (name, result)
        if binary is not None:
            code, report = native(binary, root / name, root / f'{name}.3tz')
            label = case['archiveCheck']
            if label is None:
                assert code == 0, (name, report)
                assert {'borderDisplacement', 'sameDepthBorderGap', 'terminalBorderLock',
                        'mixedDepthBorderGap'} <= set(report['checks']), name
            else:
                assert code == 3 and label in report['error']['message'], (name, label, report)
            entry['native'] = label
        results.append(entry)
    if binary is not None:
        results.append(producer(root / 'producer', binary))
    return results


def producer(root, binary):
    """The real producer's contracted borders pass both validators."""
    from oracle import audit
    root.mkdir(parents=True, exist_ok=True)
    source = root / 'sheet.glb'
    sheet(source, 120)
    audit_dir = root / 'out.hlod-audit'
    archive = root / 'out.3tz'
    report = subprocess.run([str(binary), '--json', 'mesh', '-i', str(source), '-o', str(archive), '--jobs', '2',
                             '--scratchDir', str(root)], capture_output=True, timeout=1800,
                            env={**__import__('os').environ, 'RT_HLOD_AUDIT_DIR': str(audit_dir)})
    assert report.returncode == 0, report.stderr[-2000:]
    (root / 'report.json').write_bytes(report.stdout)
    checked = subprocess.run([str(binary), 'validate', str(archive), '--json', '--expect-profile', 'hlod'],
                             capture_output=True, timeout=600)
    census = json.loads(checked.stdout)
    assert checked.returncode == 0, census
    assert census['interfaceCensus']['contractedBorderArcs'] > 0, census['interfaceCensus']
    manifest = root / 'source.sqlite'
    made = subprocess.run([sys.executable, str(Path(__file__).with_name('source_manifest.py')), str(source),
                           str(manifest)], capture_output=True, timeout=600)
    assert made.returncode == 0, made.stderr[-2000:]
    result = audit(archive, manifest, scratch=root, border_audit=audit_dir / 'borders.jsonl',
                   producer_report=root / 'report.json')
    assert result['contractedBorderArcs'] == census['interfaceCensus']['contractedBorderArcs'], result
    assert result['contractedBorderVertices'] == census['interfaceCensus']['contractedBorderVertices'], result
    return dict(name='producer-sheet', contractedBorderArcs=result['contractedBorderArcs'],
                certificates=result['borders']['certificates'], sameDepthPairs=result['borders'].get('sameDepthPairs', 0))


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('outputdir', type=Path)
    parser.add_argument('--binary', type=Path)
    args = parser.parse_args()
    print(json.dumps(run(args.outputdir, args.binary), indent=2))
