#!/usr/bin/env python3
"""Independent F1d1 fixture, rational controls and published-artifact oracle.

No Rust import, optimizer call, production writer or production distance code.
The F1c2 Python oracle supplies independent GLB decoding and exact leaf identity
checks. Synthetic artifacts exercise oracle sensitivity only.
"""
import argparse
from collections import Counter, defaultdict
import copy
from fractions import Fraction as F
import json
import math
from pathlib import Path
import struct
import subprocess
import tempfile
import zipfile
import zlib

import f1c2_oracle as leaf_oracle

require = leaf_oracle.require
digest = leaf_oracle.digest
decode = leaf_oracle.decode
accessor = leaf_oracle.accessor
view = leaf_oracle.view
glb = leaf_oracle.glb
KEYS = leaf_oracle.KEYS
PROFILE = 'f1d1-root-proxy-gltf-v1'
ARRAY_KEYS = ('source_node_indices', 'source_mesh_indices', 'source_primitive_indices', 'source_triangle_indices')


def fixture(variant='grid', n=4):
    """Indexed grids use authored vertex connectivity, not position welding."""
    doc = {'asset': {'version': '2.0'}, 'scene': 0, 'scenes': [{'nodes': [0]}],
           'nodes': [{'mesh': 0}], 'meshes': [{'primitives': []}],
           'accessors': [], 'bufferViews': [], 'materials': [
               {'pbrMetallicRoughness': {'baseColorFactor': [.25, .5, .75, 1],
                   'metallicFactor': .25, 'roughnessFactor': .75}, 'doubleSided': True},
               {'pbrMetallicRoughness': {'baseColorFactor': [.75, .25, .125, 1]},
                   'alphaMode': 'OPAQUE', 'emissiveFactor': [.125, 0, 0]}]}
    raw = bytearray()
    def add(values, shape, component=5126):
        raw.extend(b'\0'*(-len(raw) % 4))
        start = len(raw)
        rows = [(x,) if not isinstance(x, (tuple, list)) else x for x in values]
        fmt = 'f' if component == 5126 else 'I'
        for row in rows:
            raw.extend(struct.pack('<'+fmt*len(row), *row))
        v = len(doc['bufferViews'])
        doc['bufferViews'].append({'buffer': 0, 'byteOffset': start, 'byteLength': len(raw)-start})
        a = {'bufferView': v, 'componentType': component, 'count': len(values), 'type': shape}
        if shape == 'VEC3':
            a.update(min=[min(p[c] for p in rows) for c in range(3)],
                     max=[max(p[c] for p in rows) for c in range(3)])
        i = len(doc['accessors']); doc['accessors'].append(a)
        return i
    def grid(offset=0, coincident=False):
        positions = [(offset+x, 0, z) for z in range(n+1) for x in range(n+1)]
        indices = []
        for z in range(n):
            for x in range(n):
                i = z*(n+1)+x
                indices.extend([i, i+n+1, i+1, i+1, i+n+1, i+n+2])
        if coincident:
            count = len(positions)
            positions += positions[:]
            indices += [i+count for i in indices[:]]
        return {'attributes': {'POSITION': add(positions, 'VEC3')},
                'indices': add(indices, 'SCALAR', 5125), 'material': 0}
    doc['meshes'][0]['primitives'] = [grid(coincident=variant == 'coincident-keys')]
    if variant in ('instances', 'viewer'):
        second = grid(n+2); second['material'] = 1
        doc['meshes'][0]['primitives'].append(second)
        doc['nodes'] = [{'mesh': 0}, {'mesh': 0, 'name': '', 'translation': [3*n+6, 0, 0],
                                    'scale': [-1, 1, 1]},
                        {'mesh': 0, 'name': ' exact ♥ é e\u0301 ', 'translation': [4*n+12, 0, 0]}]
        doc['scenes'][0]['nodes'] = [2, 1, 0]
    elif variant in ('normals-refused','uv-refused','color-refused','tangents-refused','textures-refused'):
        primitive = doc['meshes'][0]['primitives'][0]
        count=(n+1)**2
        if variant in ('normals-refused','tangents-refused'):
            primitive['attributes']['NORMAL'] = add([(0, 1, 0)]*count, 'VEC3')
        if variant in ('uv-refused','textures-refused','tangents-refused'):
            primitive['attributes']['TEXCOORD_0']=add([(x/n,z/n) for z in range(n+1) for x in range(n+1)],'VEC2')
        if variant=='color-refused':primitive['attributes']['COLOR_0']=add([(1,.5,.25,1)]*count,'VEC4')
        if variant=='tangents-refused':primitive['attributes']['TANGENT']=add([(1,0,0,1)]*count,'VEC4')
        if variant=='textures-refused':
            def chunk(kind,data):return struct.pack('>I',len(data))+kind+data+struct.pack('>I',zlib.crc32(kind+data)&0xffffffff)
            png=b'\x89PNG\r\n\x1a\n'+chunk(b'IHDR',struct.pack('>IIBBBBB',1,1,8,6,0,0,0))+chunk(b'IDAT',zlib.compress(b'\0\xff\0\0\xff'))+chunk(b'IEND',b'')
            raw.extend(b'\0'*(-len(raw)%4));v=len(doc['bufferViews'])
            doc['bufferViews'].append({'buffer':0,'byteOffset':len(raw),'byteLength':len(png)});raw.extend(png)
            doc['images']=[{'bufferView':v,'mimeType':'image/png'}];doc['textures']=[{'source':0}]
            doc['materials'][0]['pbrMetallicRoughness']['baseColorTexture']={'index':0}
    elif variant == 'alpha-refused':
        doc['materials'][0]['alphaMode'] = 'BLEND'
    elif variant=='materialless':
        doc.pop('materials')
        for primitive in doc['meshes'][0]['primitives']:primitive.pop('material')
    elif variant=='single-triangle':
        primitive=doc['meshes'][0]['primitives'][0]
        doc['accessors'][primitive['indices']]['count']=3
    elif variant not in ('grid', 'coincident-keys'):
        raise leaf_oracle.OracleError('unknown fixture variant '+variant)
    return glb(doc, raw)


def sub(a, b): return tuple(x-y for x, y in zip(a, b))
def dot(a, b): return sum(x*y for x, y in zip(a, b))
def exact_point(p): return tuple(F(x) for x in p)


def point_triangle_squared(point, triangle):
    """Exact rational distance to the closed triangle, including degeneracy.

    Enumerate its edges and the perpendicular plane projection when its
    barycentric coordinates lie in the closed face. No sampling or stride.
    """
    p = exact_point(point)
    a, b, c = map(exact_point, triangle)
    distances = []
    for start, end in ((a, b), (b, c), (c, a)):
        edge, delta = sub(end, start), sub(p, start)
        denominator = dot(edge, edge)
        t = min(F(1), max(F(0), dot(delta, edge)/denominator)) if denominator else F(0)
        q = tuple(x+t*y for x, y in zip(start, edge))
        distances.append(dot(sub(p, q), sub(p, q)))
    u, v, delta = sub(b, a), sub(c, a), sub(p, a)
    uu, uv, vv, du, dv = dot(u, u), dot(u, v), dot(v, v), dot(delta, u), dot(delta, v)
    determinant = uu*vv-uv*uv
    if determinant:
        s, t = (du*vv-dv*uv)/determinant, (dv*uu-du*uv)/determinant
        if s >= 0 and t >= 0 and s+t <= 1:
            q = tuple(x+s*y+t*z for x, y, z in zip(a, u, v))
            distances.append(dot(sub(p, q), sub(p, q)))
    return min(distances)


def ideal_face_certificate_squared(source, candidate):
    """Exact least certificate for the contract's complete-face pairing scheme.

    Optimal rational witnesses relax the producer's finite dyadic witnesses.
    This is a lower bound on the producer certificate, not a Hausdorff estimate.
    An emitted bound smaller than this value is impossible under that scheme.
    """
    require(source and candidate, 'nonempty certificate supports')
    def directed(left, right):
        result = F(0)
        for triangle in left:
            best = None
            for target in right:
                bound = F(0)
                for p in triangle:
                    bound = max(bound, point_triangle_squared(p, target))
                    if best is not None and bound >= best:
                        break
                best = bound if best is None else min(best, bound)
                if best == 0: break
            result = max(result, best)
        return result
    return max(directed(source, candidate), directed(candidate, source))


def rational_controls():
    base = [(0, 0, 0), (8, 0, 0), (0, 8, 0)]
    subdivisions = [[(0, 0, 0), (4, 0, 0), (0, 4, 0)],
                    [(4, 0, 0), (8, 0, 0), (4, 4, 0)],
                    [(0, 4, 0), (4, 4, 0), (0, 8, 0)],
                    [(4, 0, 0), (4, 4, 0), (0, 4, 0)]]
    require(ideal_face_certificate_squared([base], subdivisions) == 16, 'coincident subdivision certificate is exactly 4m')
    spike = [(0, 0, 0), (8, 0, 0), (0, 8, 100)]
    require(point_triangle_squared(spike[2], base) == 10000, 'off-sample spike exact 100m')
    degenerate = [(0, 0, 0)]*3
    require(point_triangle_squared((3, 4, 0), degenerate) == 25, 'degenerate point face exact 5m')
    mixed = [(2**20, 0, 0), (2**20+1, 0, 0), (2**20, 1, 0)]
    require(point_triangle_squared((2**20+F(1, 4), F(1, 4), F(1, 2**30)), mixed) == F(1, 2**60), 'mixed-magnitude exact dyadic height')
    admitted=[(999999,0,0),(1000000,0,0),(999999,1,0)]
    require(point_triangle_squared((F(3999997,4),F(1,4),F(1,2**30)),admitted)==F(1,2**60), 'admitted magnitude boundary exact dyadic height')
    # Four strips cover the boundary of a square but omit its interior.
    ring = [[(0, 0, 0), (8, 0, 0), (8, 1, 0)], [(0, 0, 0), (8, 1, 0), (0, 1, 0)],
            [(0, 7, 0), (8, 7, 0), (8, 8, 0)], [(0, 7, 0), (8, 8, 0), (0, 8, 0)],
            [(0, 1, 0), (1, 1, 0), (1, 7, 0)], [(0, 1, 0), (1, 7, 0), (0, 7, 0)],
            [(7, 1, 0), (8, 1, 0), (8, 7, 0)], [(7, 1, 0), (8, 7, 0), (7, 7, 0)]]
    require(min(point_triangle_squared((4, 4, 0), face) for face in ring) == 9, 'interior hole exact 3m')
    return {'subdivision_face_pair_certificate_metres': 4, 'subdivision_true_distance_metres': 0,
            'spike_distance_metres': 100, 'degenerate_distance_metres': 5,
            'mixed_magnitude_squared_distance': '1/1152921504606846976', 'admitted_boundary_squared_distance':'1/1152921504606846976', 'hole_center_distance_metres': 3}


def authored_components(source, region):
    """Count components from authored vertex indices, never position equality."""
    doc,raw,_=decode(source);primitive=doc['meshes'][region[1]]['primitives'][region[2]]
    positions=accessor(doc,raw,primitive['attributes']['POSITION'])
    indices=accessor(doc,raw,primitive['indices']) if 'indices' in primitive else list(range(len(positions)))
    parents={i:i for i in indices}
    def find(i):
        while parents[i]!=i:parents[i]=parents[parents[i]];i=parents[i]
        return i
    for start in range(0,len(indices),3):
        a,b,c=indices[start:start+3]
        for v in (b,c):parents[find(v)]=find(a)
    return len({find(i) for i in indices})


def proxy_rows(doc, binary):
    extension = doc['extensions']['EXT_structural_metadata']
    require(extension['schema']['id']=='rusty_tiles_proxy_v1', 'fixed generated proxy schema identity')
    require(len(extension['propertyTables']) == 1, 'one proxy region property table')
    table = extension['propertyTables'][0]
    require(table['class'] == 'proxy_region' and table['count'] > 0, 'proxy region class/count')
    declarations = extension['schema']['classes']['proxy_region']['properties']
    require(set(declarations) == set(table['properties']) == set(ARRAY_KEYS)|{'source_node_name', 'source_node_name_present'}, 'exact proxy columns')
    columns = {}
    for key in ARRAY_KEYS:
        declaration, prop = declarations[key], table['properties'][key]
        require(declaration.get('array') is True and declaration.get('componentType') == 'UINT32' and declaration['type'] == 'SCALAR' and declaration.get('required') is True, 'required variable UINT32 array '+key)
        require('count' not in declaration, 'variable arrays have no fixed count')
        require(prop.get('arrayOffsetType', 'UINT32') == 'UINT32', 'UINT32 array offsets')
        raw = view(doc, binary, prop['values']); offsets_raw = view(doc, binary, prop['arrayOffsets'])
        require(len(raw) % 4 == 0 and len(offsets_raw) == 4*(table['count']+1), 'array storage lengths')
        offsets = struct.unpack('<'+'I'*(table['count']+1), offsets_raw)
        require(offsets[0] == 0 and offsets[-1]*4 == len(raw) and all(a <= b for a, b in zip(offsets, offsets[1:])), 'complete array storage offsets')
        values = struct.unpack('<'+'I'*(len(raw)//4), raw)
        columns[key] = [values[a:b] for a, b in zip(offsets, offsets[1:])]
    for key in ('source_node_name', 'source_node_name_present'):
        prop = table['properties'][key]; raw = view(doc, binary, prop['values'])
        if key == 'source_node_name':
            require(declarations[key]['type'] == 'STRING', 'proxy name type')
            offsets_raw = view(doc, binary, prop['stringOffsets'])
            require(len(offsets_raw) == 4*(table['count']+1), 'proxy string offset length')
            offsets = struct.unpack('<'+'I'*(table['count']+1), offsets_raw)
            require(offsets[0] == 0 and (offsets[-1] == len(raw) or raw == b'\0' and not any(offsets)), 'exact proxy name storage')
            require(all(a <= b for a,b in zip(offsets, offsets[1:])), 'proxy string offsets monotone')
            columns[key] = [raw[a:b].decode() for a,b in zip(offsets, offsets[1:])]
        else:
            require(declarations[key]['componentType'] == 'UINT8' and len(raw) == table['count'], 'proxy name presence storage')
            columns[key] = list(raw)
    return [{key: value[i] for key,value in columns.items()} for i in range(table['count'])]


def inspect_members(source, members, leaf_limit, triangle_limit, max_error, *, exact_certificate=True):
    members={name:data for name,data in members.items() if name!='@3dtilesIndex1@'}
    truth, names = leaf_oracle.expected_source(source)
    manifest = json.loads(members['tileset.json']); report = json.loads(members['conversion.json'])
    root = manifest['root']; uri = root['content']['uri']
    require(report['schema_version'] == 6 and report['profile'] == PROFILE, 'F1d1 report schema/profile')
    require(root['refine'] == 'REPLACE' and root['geometricError'] == max_error > 0, 'root declared positive budget/refinement')
    require(manifest['geometricError'] >= max_error, 'top-level omission covers root budget')
    require(root.get('children') and all(leaf['geometricError'] == 0 and not leaf.get('children') for leaf in root['children']), 'unchanged full-detail leaf hierarchy')
    # Coordinator migrates the independent leaf oracle to the current profile.
    # Its leaf identity/geometry/material assertions remain unchanged.
    leaves = leaf_oracle.inspect_members(source, members, leaf_limit)
    doc, binary, _ = decode(members[uri]); rows = proxy_rows(doc, binary)
    wanted = defaultdict(list)
    for key in sorted(truth): wanted[key[:3]].append(key)
    memberships, region_keys = [], []
    for row in rows:
        require(len({len(row[key]) for key in ARRAY_KEYS}) == 1 and row[ARRAY_KEYS[0]], 'nonempty equal-length proxy tuple arrays')
        tuples = list(zip(*(row[key] for key in ARRAY_KEYS)))
        require(tuples == sorted(set(tuples)), 'unique tuple-sorted proxy membership')
        region = tuples[0][:3]
        require(all(key[:3] == region for key in tuples), 'one selected source primitive instance per region')
        require(tuples == wanted[region], 'all original tuples including removed/repeated/coincident faces')
        require((row['source_node_name_present'], row['source_node_name']) == names[region[0]], 'exact optional proxy source label')
        memberships.extend(tuples); region_keys.append(region)
    require(Counter(memberships) == Counter(truth.keys()) and region_keys == sorted(wanted), 'proxy memberships partition selected source exactly')
    name_bytes=leaves['emitted_name_bytes']+sum(len(row['source_node_name'].encode()) for row in rows)
    require(name_bytes<=8*1024*1024, 'combined full-detail and proxy emitted name ceiling')
    candidates = defaultdict(list)
    for primitive in doc['meshes'][0]['primitives']:
        attrs = primitive['attributes']
        require(set(attrs) == {'POSITION', '_FEATURE_ID_0'}, 'positions-only proxy attributes and no false triangle feature')
        require(doc['accessors'][attrs['POSITION']]['componentType']==5126 and doc['accessors'][attrs['POSITION']]['type']=='VEC3', 'unquantized float32 proxy positions')
        feature_accessor=doc['accessors'][attrs['_FEATURE_ID_0']]
        require(feature_accessor['componentType']==5126 and feature_accessor['type']=='SCALAR' and not feature_accessor.get('normalized'), 'scalar float32 unnormalized proxy feature IDs')
        features = primitive['extensions']['EXT_mesh_features']['featureIds']
        require(len(features) == 1 and features[0]['label'] == 'proxy_region' and features[0]['attribute'] == 0 and features[0]['propertyTable'] == 0, 'sole proxy_region feature label')
        positions = accessor(doc, binary, attrs['POSITION']); ids = accessor(doc, binary, attrs['_FEATURE_ID_0'])
        require(all(math.isfinite(i) and i == int(i) and 0 <= i < len(rows) for i in ids), 'proxy feature ids integral/in range')
        require(features[0]['featureCount'] == len(set(ids)) and len(ids) == len(positions), 'proxy corner cardinality')
        indices = accessor(doc, binary, primitive['indices']) if 'indices' in primitive else list(range(len(positions)))
        require(len(indices) % 3 == 0, 'proxy triangle index count')
        for start in range(0, len(indices), 3):
            corners = indices[start:start+3]
            require(len({ids[i] for i in corners}) == 1, 'uniform proxy triangle region')
            region = region_keys[int(ids[corners[0]])]
            candidate = [positions[i] for i in corners]
            originals = [truth[key]['positions'] for key in wanted[region]]
            allowed = {tuple(p) for face in originals for p in face}
            require(all(tuple(p) in allowed for p in candidate), 'candidate only original decoded positions of its region')
            material=doc['materials'][primitive['material']] if 'material' in primitive else None
            require(material == truth[wanted[region][0]]['material'], 'exact proxy region PBR factors/omissions')
            candidates[region].append(candidate)
    require(set(candidates) == set(wanted), 'every source region represented')
    component_counts={region:authored_components(source,region) for region in wanted}
    require(all(len(candidates[region])>=count for region,count in component_counts.items()), 'candidate has at least one face per authored-key component')
    proxy_count = sum(map(len, candidates.values()))
    require(0 < proxy_count <= triangle_limit and proxy_count < len(truth), 'actual root reduction and requested total ceiling')
    comparisons = sum(2*len(wanted[region])*len(candidate) for region,candidate in candidates.items())
    require(comparisons <= 16777216, 'finite exhaustive face-pair admission')
    # The report field spellings are asserted after the coordinator freezes the
    # facade contract; geometry truth never derives from reported counts.
    approximation = report['approximation']
    require(report['triangles']==len(truth) and report['leaf_tiles']==leaves['leaves'] and report['leaf_triangles']==leaf_limit, 'report unchanged full-detail counts and declared leaf ceiling')
    require(approximation['kind'] == 'root_proxy', 'root approximation report mode')
    require(approximation['regions']==len(rows), 'report region count independently decoded')
    require(approximation['appearance']=='opaque-untextured-factors', 'exact admitted appearance report')
    require(approximation['triangle_limit'] == triangle_limit and approximation['geometric_error_metres'] == max_error, 'report declared policy')
    require(approximation['triangles'] == proxy_count and approximation['comparison_pairs'] == comparisons, 'report independently decoded counts/work')
    certificate = approximation['certified_error_metres']
    require(math.isfinite(certificate) and 0 <= certificate <= max_error, 'finite admitted certificate')
    ideal_squared = F(0)
    if exact_certificate:
        for region, candidate in candidates.items():
            ideal_squared = max(ideal_squared, ideal_face_certificate_squared([truth[key]['positions'] for key in wanted[region]], candidate))
        require(F(certificate)**2 >= ideal_squared, 'emitted certificate at least exact rational optimum of complete-face scheme')
    points = [p for record in truth.values() for p in record['positions']] + [p for faces in candidates.values() for face in faces for p in face]
    box = root['boundingVolume']['box']
    require(len(box) == 12 and all(math.isfinite(x) for x in box), 'finite root box')
    require(all(box[i] == 0 for i in (4, 5, 6, 8, 9, 10)), 'declared axis-aligned tile box')
    half = [box[i] for i in (3, 7, 11)]
    require(all(h >= 0 for h in half), 'nonnegative box half axes')
    for p in points:
        tile = (p[0], -p[2], p[1])
        require(all(abs(F(tile[i])-F(box[i])) <= F(half[i]) for i in range(3)), 'decoded content in exact conservative root box')
    for leaf in root['children']:
        child_box=leaf['boundingVolume']['box'];child_half=[child_box[i] for i in (3,7,11)]
        require(len(child_box)==12 and all(math.isfinite(x) for x in child_box) and all(h>=0 for h in child_half), 'finite nonnegative child box')
        require(all(child_box[i]==0 for i in (4,5,6,8,9,10)), 'axis-aligned child box')
        require(all(abs(F(child_box[i])-F(box[i]))+F(child_half[i])<=F(half[i]) for i in range(3)), 'root box contains complete child boxes')
        child_doc,child_raw,_=decode(members[leaf['content']['uri']])
        for primitive in child_doc['meshes'][0]['primitives']:
            for p in accessor(child_doc,child_raw,primitive['attributes']['POSITION']):
                tile=(p[0],-p[2],p[1])
                require(all(abs(F(tile[i])-F(child_box[i]))<=F(child_half[i]) for i in range(3)), 'leaf box contains actual decoded leaf positions')
    require(manifest['geometricError'] >= max(1, math.sqrt(sum((2*h)**2 for h in half))), 'top-level omission covers bounds diagonal/routing minimum')
    expected_names = {'tileset.json','conversion.json',uri}|{leaf['content']['uri'] for leaf in root['children']}
    require(set(members) == expected_names, 'exact accepted inventory without orphan candidates')
    return {'source_sha256': digest(source), 'source_triangles': len(truth), 'proxy_triangles': proxy_count,
            'regions': len(rows), 'authored_components':[{'region':list(region),'count':count} for region,count in component_counts.items()], 'comparison_work': comparisons, 'ideal_certificate_squared': str(ideal_squared) if exact_certificate else None,
            'leaves': leaves['leaves'], 'emitted_name_bytes':name_bytes, 'report': report, 'proxy_member': uri,
            'member_sha256': {key:digest(value) for key,value in members.items()}}


def synthetic_members(source, leaf_limit=100):
    """Independent planar-corner proxy, solely for checker sensitivity controls."""
    truth, names = leaf_oracle.expected_source(source)
    regions = sorted({key[:3] for key in truth})
    doc = {'asset': {'version':'2.0'}, 'scene':0, 'scenes':[{'nodes':[0]}], 'nodes':[{'mesh':0}],
           'meshes':[{'primitives':[]}], 'materials':[], 'accessors':[], 'bufferViews':[],
           'extensionsUsed':['EXT_mesh_features','EXT_structural_metadata']}
    binary = bytearray()
    def storage(raw):
        binary.extend(b'\0'*(-len(binary)%8))
        i = len(doc['bufferViews']); doc['bufferViews'].append({'buffer':0,'byteOffset':len(binary),'byteLength':len(raw)})
        binary.extend(raw); return i
    def array(values, shape):
        rows = [(x,) if not isinstance(x,(tuple,list)) else x for x in values]
        raw = b''.join(struct.pack('<'+'f'*len(row), *row) for row in rows)
        a = {'bufferView':storage(raw),'componentType':5126,'count':len(values),'type':shape}
        if shape == 'VEC3': a.update(min=[min(row[c] for row in rows) for c in range(3)],max=[max(row[c] for row in rows) for c in range(3)])
        i=len(doc['accessors']); doc['accessors'].append(a); return i
    tuples_by_region = [[key for key in sorted(truth) if key[:3] == region] for region in regions]
    for row, keys in enumerate(tuples_by_region):
        points = [point for key in keys for point in truth[key]['positions']]
        low = [min(p[c] for p in points) for c in range(3)]; high=[max(p[c] for p in points) for c in range(3)]
        require(low[1] == high[1], 'synthetic positive supports only planar horizontal fixture')
        a,b,c,d = (low[0],low[1],low[2]),(low[0],low[1],high[2]),(high[0],low[1],low[2]),(high[0],low[1],high[2])
        material=truth[keys[0]]['material']
        primitive={'attributes':{'POSITION':array([a,b,c,c,b,d],'VEC3'),'_FEATURE_ID_0':array([row]*6,'SCALAR')},
            'extensions':{'EXT_mesh_features':{'featureIds':[{'attribute':0,'propertyTable':0,'featureCount':1,'label':'proxy_region'}]}}}
        if material is not None:
            primitive['material']=len(doc['materials']);doc['materials'].append(material)
        doc['meshes'][0]['primitives'].append(primitive)
    offsets=[0]
    for keys in tuples_by_region: offsets.append(offsets[-1]+len(keys))
    offset_view=storage(struct.pack('<'+'I'*len(offsets),*offsets)); properties={}; declarations={}
    for column,key in enumerate(ARRAY_KEYS):
        values=[identity[column] for keys in tuples_by_region for identity in keys]
        properties[key]={'values':storage(struct.pack('<'+'I'*len(values),*values)),'arrayOffsets':offset_view,'arrayOffsetType':'UINT32'}
        declarations[key]={'type':'SCALAR','componentType':'UINT32','array':True,'required':True}
    name_bytes=b''; name_offsets=[0]
    for region in regions:
        name_bytes += names[region[0]][1].encode(); name_offsets.append(len(name_bytes))
    properties['source_node_name']={'values':storage(name_bytes or b'\0'),'stringOffsets':storage(struct.pack('<'+'I'*len(name_offsets),*name_offsets)),'stringOffsetType':'UINT32'}
    properties['source_node_name_present']={'values':storage(bytes(names[region[0]][0] for region in regions))}
    declarations['source_node_name']={'type':'STRING','required':True}
    declarations['source_node_name_present']={'type':'SCALAR','componentType':'UINT8','required':True}
    doc['extensions']={'EXT_structural_metadata':{'schema':{'id':'rusty_tiles_proxy_v1','classes':{'proxy_region':{'properties':declarations}}},
        'propertyTables':[{'class':'proxy_region','count':len(regions),'properties':properties}]}}
    members=leaf_oracle.synthetic_members(source)
    if not doc['materials']:doc.pop('materials')
    members['t/root.glb']=glb(doc,binary)
    manifest=json.loads(members['tileset.json']); manifest['root']['content']={'uri':'t/root.glb'}
    manifest['root']['geometricError']=8
    members['tileset.json']=json.dumps(manifest).encode()
    members['conversion.json']=json.dumps({'schema_version':6,'profile':PROFILE,'triangles':len(truth),'leaf_tiles':1,'leaf_triangles':leaf_limit,'approximation':{'kind':'root_proxy','triangle_limit':16,
        'triangles':len(regions)*2,'regions':len(regions),'geometric_error_metres':8,'certified_error_metres':8,
        'comparison_pairs':sum(4*len(keys) for keys in tuples_by_region),'appearance':'opaque-untextured-factors'}}).encode()
    return members


def corruption_controls(source, members, leaf_limit, triangle_limit, max_error):
    changed={}
    def root_change(label, mutation):
        out=dict(members); doc,raw,_=decode(out['t/root.glb']); raw=bytearray(raw)
        mutation(doc,raw); out['t/root.glb']=glb(doc,raw); changed[label]=out
    root_change('false-source-triangle-label',lambda d,r:d['meshes'][0]['primitives'][0]['extensions']['EXT_mesh_features']['featureIds'][0].update(label='source_triangle'))
    truth,_=leaf_oracle.expected_source(source)
    if any(record['material'] is not None for record in truth.values()):
        root_change('wrong-material-factors',lambda d,r:d['materials'][0]['pbrMetallicRoughness'].update(metallicFactor=.5))
    else:
        def invented_material(d,r):
            d['materials']=[{}];d['meshes'][0]['primitives'][0]['material']=0
        root_change('invented-default-material',invented_material)
    def change_vertex(doc,raw):
        a=doc['accessors'][doc['meshes'][0]['primitives'][0]['attributes']['POSITION']]
        v=doc['bufferViews'][a['bufferView']]
        struct.pack_into('<f',raw,v['byteOffset'],9999)
    root_change('proxy-vertex-not-original',change_vertex)
    def change_membership(doc,raw):
        p=doc['extensions']['EXT_structural_metadata']['propertyTables'][0]['properties']['source_triangle_indices']
        v=doc['bufferViews'][p['values']]; struct.pack_into('<I',raw,v['byteOffset'],999999)
    root_change('invented-original-membership',change_membership)
    def array_length(doc,raw):
        p=doc['extensions']['EXT_structural_metadata']['propertyTables'][0]['properties']['source_node_indices']
        p['arrayOffsets']=p['values']
    root_change('unequal-proxy-arrays',array_length)
    for label,key,value in [('understated-certificate','certified_error_metres',0),('wrong-proxy-count','triangles',1),('wrong-pair-work','comparison_pairs',1),('wrong-region-count','regions',999),('false-appearance-profile','appearance','textured-atlas')]:
        out=dict(members); report=json.loads(out['conversion.json']); report['approximation'][key]=value
        out['conversion.json']=json.dumps(report).encode(); changed[label]=out
    out=dict(members); manifest=json.loads(out['tileset.json']); manifest['root']['geometricError']=0
    out['tileset.json']=json.dumps(manifest).encode(); changed['zero-root-refinement-budget']=out
    out=dict(members);manifest=json.loads(out['tileset.json']);manifest['root']['children'][0]['boundingVolume']['box'][0]+=10000
    out['tileset.json']=json.dumps(manifest).encode();changed['child-box-outside-root']=out
    leaf_uri=json.loads(members['tileset.json'])['root']['children'][0]['content']['uri']
    def leaf_change(label,mutation):
        out=dict(members); doc,raw,_=decode(out[leaf_uri]); raw=bytearray(raw)
        mutation(doc,raw); out[leaf_uri]=glb(doc,raw); changed[label]=out
    def winding(doc,raw):
        a=doc['accessors'][doc['meshes'][0]['primitives'][0]['attributes']['POSITION']]
        v=doc['bufferViews'][a['bufferView']]; start=v['byteOffset']+a.get('byteOffset',0)
        raw[start+12:start+24],raw[start+24:start+36]=raw[start+24:start+36],raw[start+12:start+24]
    leaf_change('leaf-winding-reversed',winding)
    def ordinal(doc,raw):
        p=doc['extensions']['EXT_structural_metadata']['propertyTables'][1]['properties']['triangle_index']
        v=doc['bufferViews'][p['values']]; struct.pack_into('<I',raw,v['byteOffset'],999999)
    leaf_change('leaf-invented-source-ordinal',ordinal)
    records=[]
    for label,out in changed.items():
        try: inspect_members(source,out,leaf_limit,triangle_limit,max_error)
        except (leaf_oracle.OracleError,UnicodeError,KeyError,IndexError,ValueError,struct.error) as error:
            records.append({'name':label,'rejected':True,'reason':str(error)})
        else: raise leaf_oracle.OracleError('insensitive artifact control '+label)
    return records


def coincident_component_control():
    source=fixture('coincident-keys',2);members=synthetic_members(source)
    positive=inspect_members(source,members,100,16,8)
    changed=dict(members);doc,raw,_=decode(changed['t/root.glb'])
    attrs=doc['meshes'][0]['primitives'][0]['attributes']
    for name in ('POSITION','_FEATURE_ID_0'):doc['accessors'][attrs[name]]['count']=3
    changed['t/root.glb']=glb(doc,raw)
    report=json.loads(changed['conversion.json']);report['approximation'].update(triangles=1,comparison_pairs=32)
    changed['conversion.json']=json.dumps(report).encode()
    try:inspect_members(source,changed,100,16,8)
    except leaf_oracle.OracleError as error:
        return {'positive':positive,'one_face_control':{'rejected':True,'reason':str(error)},
            'limit':'Face floor detects impossible count; identical position-only supports cannot independently identify component origin per output face. Production-key tests and source review remain required.'}
    raise leaf_oracle.OracleError('insensitive one-face coincident component control')


def run(binary, artifact_dir):
    binary=binary.resolve(strict=True);artifact_dir.mkdir(parents=True,exist_ok=True)
    receipts=[]
    for variant,limit in [('grid',16),('materialless',16),('instances',48),('viewer',48),('coincident-keys',16)]:
        work=artifact_dir/variant;work.mkdir(parents=True,exist_ok=True)
        source=fixture(variant,4);input_path=work/'source.glb';input_path.write_bytes(source)
        output=work/'result.3tz'
        command=[str(binary),'--json','mesh-local-to-3tz','-i',str(input_path),'-o',str(output),
            '--leaf-triangles','16','--root-proxy-triangles',str(limit),'--max-proxy-error-metres','8']
        completed=subprocess.run(command,text=True,capture_output=True,timeout=60)
        require(completed.returncode==0,'actual converter '+variant+' '+completed.stdout+completed.stderr)
        with zipfile.ZipFile(output) as stream:
            require(len(stream.namelist())==len(set(stream.namelist())), 'unique actual ZIP member names')
            members={name:stream.read(name) for name in stream.namelist() if name!='@3dtilesIndex1@'}
        checked=inspect_members(source,members,16,limit,8)
        controls=corruption_controls(source,members,16,limit,8)
        receipts.append({'case':variant,'command':command,'exit_code':completed.returncode,'stdout':completed.stdout,'stderr':completed.stderr,
            'archive_sha256':digest(output.read_bytes()),'artifact':checked,'controls':controls})
    refusals=[]
    for case,variant,n,limit,error in [('companions','normals-refused',4,16,8),('alpha','alpha-refused',4,16,8),
        ('uv','uv-refused',4,16,8),('color','color-refused',4,16,8),('tangents','tangents-refused',4,16,8),('textures','textures-refused',4,16,8),
        ('error-budget','grid',4,16,1),('impossible-count','coincident-keys',4,1,8),
        ('no-reduction','single-triangle',1,16,8),('work-ceiling','grid',64,4096,100),
        ('zero-limit','grid',4,0,8),('zero-error','grid',4,16,0)]:
        work=artifact_dir/('refused-'+case);work.mkdir(parents=True,exist_ok=True)
        source=fixture(variant,n);input_path=work/'source.glb';input_path.write_bytes(source)
        output=work/'absent-parent'/'result.3tz'
        command=[str(binary),'--json','mesh-local-to-3tz','-i',str(input_path),'-o',str(output),
            '--leaf-triangles','1024','--root-proxy-triangles',str(limit),'--max-proxy-error-metres',str(error)]
        completed=subprocess.run(command,text=True,capture_output=True,timeout=60)
        require(completed.returncode!=0,'refusal required '+case)
        require(not output.parent.exists(),'candidate refusal before output-parent/workspace/staging '+case)
        response=json.loads(completed.stdout)
        expected='invalid_request' if case in ('zero-limit','zero-error') else 'unsupported'
        require(response['error'].get('kind',response['error'].get('code'))==expected,'typed refusal '+case+' '+completed.stdout)
        if case=='error-budget':require('whole-face certificate exceeds' in response['error']['message'], 'exercise actual certification refusal')
        if case=='work-ceiling':require('16777216 complete face pairs' in response['error']['message'], 'exercise actual pair admission refusal')
        refusals.append({'case':case,'source_sha256':digest(source),'command':command,'exit_code':completed.returncode,
            'stdout':completed.stdout,'stderr':completed.stderr,'output_parent_absent':True})
    return {'binary_path':str(binary),'binary_sha256':digest(binary.read_bytes()),'successful_artifacts':receipts,'admission_refusals':refusals,
        'scope':'Bounded indexed planar grid proxy reductions, multiple source regions/materials/reflection, complete membership and exact leaves, certificate arithmetic and pre-workspace refusal. No topology/appearance/world-distance/general HLOD acceptance.'}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--self-test', action='store_true')
    parser.add_argument('--binary',type=Path)
    parser.add_argument('--artifact-dir',type=Path)
    parser.add_argument('--fixture', type=Path)
    parser.add_argument('--variant', default='grid', choices=['grid','materialless','single-triangle','instances','viewer','coincident-keys','normals-refused','alpha-refused','uv-refused','color-refused','tangents-refused','textures-refused'])
    parser.add_argument('--grid-size', type=int, default=4)
    parser.add_argument('--source', type=Path)
    parser.add_argument('--archive', type=Path)
    parser.add_argument('--leaf-limit', type=int, default=16)
    parser.add_argument('--triangle-limit', type=int, default=16)
    parser.add_argument('--max-error', type=float, default=8)
    parser.add_argument('--json-output', type=Path)
    args = parser.parse_args(); result = {}
    if args.self_test:
        result['rational_controls'] = rational_controls()
        data=fixture('instances',2); members=synthetic_members(data)
        result['synthetic_positive']=inspect_members(data,members,100,16,8)
        result['artifact_controls']=corruption_controls(data,members,100,16,8)
        result['coincident_component_control']=coincident_component_control()
        materialless=fixture('materialless',2); materialless_members=synthetic_members(materialless)
        result['materialless_positive']=inspect_members(materialless,materialless_members,100,16,8)
        result['materialless_controls']=corruption_controls(materialless,materialless_members,100,16,8)
        result['scope'] = 'Independent synthetic artifact and exact-rational checker controls only; no converter acceptance.'
    if args.fixture:
        data = fixture(args.variant, args.grid_size); args.fixture.parent.mkdir(parents=True, exist_ok=True); args.fixture.write_bytes(data)
        truth, _ = leaf_oracle.expected_source(data)
        result['fixture'] = {'path': str(args.fixture), 'sha256': digest(data), 'triangles': len(truth)}
    if args.archive:
        require(args.source, '--source required for artifact inspection')
        with zipfile.ZipFile(args.archive) as stream:
            require(len(stream.namelist()) == len(set(stream.namelist())), 'no duplicate ZIP members')
            members = {name:stream.read(name) for name in stream.namelist() if name != '@3dtilesIndex1@'}
        result['artifact'] = inspect_members(args.source.read_bytes(), members, args.leaf_limit, args.triangle_limit, args.max_error)
        result['artifact']['archive_sha256'] = digest(args.archive.read_bytes())
    if args.binary:
        require(args.artifact_dir,'--artifact-dir required for reproducible candidate execution')
        result['execution']=run(args.binary,args.artifact_dir)
    result['driver_sha256'] = {path.name:digest(path.read_bytes()) for path in (Path(__file__), Path(__file__).with_name('f1c2_oracle.py'))}
    encoded = json.dumps(result, indent=2, allow_nan=False)+'\n'
    if args.json_output:
        args.json_output.parent.mkdir(parents=True, exist_ok=True); args.json_output.write_text(encoded)
    else: print(encoded, end='')


if __name__ == '__main__': main()
