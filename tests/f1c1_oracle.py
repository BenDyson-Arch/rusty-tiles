#!/usr/bin/env python3
"""Independent stdlib placement, world geometry/bounds and raw archive acceptance.

Truth: NGA WGS84 constants, Decimal80 Taylor trigonometry, Hamilton action,
literal cardinals/rational mixed rotation. No production imports or inverse CRS.
Prior independent F1b3 code supplies authored local PBR/GLB/resource truth only.
"""
import argparse
import copy
from decimal import Decimal as D, localcontext
from fractions import Fraction
import hashlib
import json
import math
from pathlib import Path
import struct
import subprocess
import tempfile
import zipfile

import f1a_oracle as geometry
import f1b_oracle as texture
import f1b2_oracle as binding
import f1b3_oracle as core

require = geometry.require
OracleError = geometry.OracleError
PROFILE = 'f1c2-source-identity-gltf-v1'
A = D('6378137')
INVF = D('298.257223563')
PI = D('3.141592653589793238462643383279502884197169399375105820974944592307816406286208998628')
REFERENCE_PATH = Path(__file__).parent / 'fixtures/f1c1/analytic-references.json'
ANCHORS = (
    ('origin', '0', '0', '0'), ('east90', '90', '0', '0'), ('west90', '-90', '0', '100'),
    ('antimeridian+', '180', '0', '0'), ('antimeridian-', '-180', '0', '0'),
    ('north0', '0', '90', '0'), ('north137', '137', '90', '100'),
    ('south0', '0', '-90', '0'), ('south-79', '-79', '-90', '-100'),
    ('brisbane', '153', '-27', '130'), ('oblique-high', '37', '48', '1000000'),
    ('oblique-low', '-123', '-52', '-1000000'), ('near-north', '179.999999', '89.999999', '1000'),
    ('near-south', '-179.999999', '-89.999999', '-1000'),
    ('near-antimeridian+', '179.999999', '23', '123'), ('near-antimeridian-', '-179.999999', '23', '123'),
    ('far-positive-height', '37', '48', '50000000'), ('far-negative-height', '-123', '-52', '-50000000'),
    ('earth-centre-forward', '137', '0', '-6378137'),
    ('near-earth-centre-forward', '137', '0', '-6378136.999999'),
    ('near-pole-centre-forward', '79', '90', '-6356752.314245'),
)
MIXED = [i / math.sqrt(30) for i in (1, 2, 3, 4)]
PLACEMENTS = {'local': {}}
PLACEMENTS.update({name: {'anchor': [float(lon), float(lat), float(height)]} for name, lon, lat, height in ANCHORS})
PLACEMENTS.update({
    'mixed-offset': {'anchor': [153., -27., 130.], 'orientation_xyzw': MIXED, 'scene_offset': [11., 13., 17.]},
    'mixed-negative-q': {'anchor': [153., -27., 130.], 'orientation_xyzw': [-x for x in MIXED], 'scene_offset': [11., 13., 17.]},
    'large-offset': {'anchor': [37., 48., 0.], 'orientation_xyzw': MIXED, 'scene_offset': [20e6, -20e6, 18e6]},
    'explicit-identity': {'anchor': [153., -27., 130.], 'orientation_xyzw': [0., 0., 0., 1.], 'scene_offset': [0., 0., 0.]},
    'norm-inside': {'anchor': [0., 0., 0.], 'orientation_xyzw': [0., 0., 0., 1.+5e-13]},
})
for i, axis in enumerate(('east', 'north', 'up')):
    for sign in (-1, 1):
        q = [0., 0., 0., math.sqrt(.5)]; q[i] = sign * math.sqrt(.5)
        PLACEMENTS[axis + ('-90' if sign < 0 else '+90')] = {'anchor': [-123., 48., 400.], 'orientation_xyzw': q}

GEOMETRY_VARIANTS = ('instanced', 'nested-translation', 'near-limit', 'zero-extent', 'line-extent', 'plane-extent', 'thin-spike', 'viewer')
VARIANTS = (*core.VARIANTS, *GEOMETRY_VARIANTS)
EXTERNAL_VARIANTS = core.EXTERNAL_VARIANTS


def exact_decimal(value):
    return D.from_float(value) if type(value) is float else D(str(value))


def sincos_degrees(value):
    degrees = exact_decimal(value)
    if degrees % 90 == 0:
        return ((D(0), D(1)), (D(1), D(0)), (D(0), D(-1)), (D(-1), D(0)))[int(degrees/90) % 4]
    angle = degrees * PI / 180; sine = term_s = angle; cosine = term_c = D(1)
    for k in range(1, 200):
        term_s *= -angle*angle / D((2*k)*(2*k+1))
        term_c *= -angle*angle / D((2*k-1)*(2*k))
        sine += term_s; cosine += term_c
        if abs(term_s) < D('1e-85') and abs(term_c) < D('1e-85'):
            return sine, cosine
    raise OracleError('independent Taylor reference did not converge')


def cartographic_frame(anchor):
    sl, cl = sincos_degrees(anchor[0]); sp, cp = sincos_degrees(anchor[1]); h = exact_decimal(anchor[2])
    f = 1/INVF; e2 = f*(2-f); radius = A/(1-e2*sp*sp).sqrt()
    origin = ((radius+h)*cp*cl, (radius+h)*cp*sl, (radius*(1-e2)+h)*sp)
    columns = ((-sl, cl, D(0)), (-sp*cl, -sp*sl, cp), (cp*cl, cp*sl, sp))
    return origin, columns


def dot(a, b):
    return sum(x*y for x, y in zip(a, b))


def cross(a, b):
    return (a[1]*b[2]-a[2]*b[1], a[2]*b[0]-a[0]*b[2], a[0]*b[1]-a[1]*b[0])


def hamilton(a, b):
    v = cross(a[:3], b[:3])
    return tuple(a[3]*b[i]+b[3]*a[i]+v[i] for i in range(3)) + (a[3]*b[3]-dot(a[:3], b[:3]),)


def rotate(q, p):
    return hamilton(hamilton(q, tuple(p)+(D(0),)), tuple(-x for x in q[:3])+(q[3],))[:3]


def columns_apply(columns, p):
    return tuple(sum(columns[c][r]*p[c] for c in range(3)) for r in range(3))


def placement_reference(placement=None):
    """Return strict report expectation and high-precision column-major matrix.

    Inputs are independently specified requests, never output/report values.
    """
    placement = {} if placement is None else placement
    if isinstance(placement, str): placement = PLACEMENTS[placement]
    if not placement:
        return geometry.local_placement_expectation(), tuple(D(int(i == j)) for j in range(4) for i in range(4))
    require('anchor' in placement and set(placement) <= {'anchor','orientation_xyzw','scene_offset'}, 'known independent placement')
    with localcontext() as context:
        context.prec = 80
        anchor = [float(x) for x in placement['anchor']]
        offset = [float(x) for x in placement.get('scene_offset', (0,0,0))]
        raw_q = [exact_decimal(x) for x in placement.get('orientation_xyzw', (0,0,0,1))]
        norm = sum(x*x for x in raw_q).sqrt(); q = tuple(x/norm for x in raw_q)
        origin, basis = cartographic_frame(anchor)
        rotated = tuple(columns_apply(basis, rotate(q, tuple(D(int(i == j)) for i in range(3)))) for j in range(3))
        delta = columns_apply(rotated, (exact_decimal(offset[0]), -exact_decimal(offset[2]), exact_decimal(offset[1])))
        translation = tuple(x+y for x, y in zip(origin, delta))
        matrix = tuple(x for column in rotated for x in (*column,D(0))) + (*translation,D(1))
        expectation = {'coordinates':'wgs84-ecef', 'placement':{'kind':'wgs84', 'anchor_degrees_metres':anchor,
                       'orientation_xyzw':[float(x) for x in q], 'scene_offset_metres':offset}, 'root_transform':[float(x) for x in matrix]}
        return expectation, matrix


def matrix_rows(values):
    require(len(values) == 16 and all(type(x) in (int,float) and math.isfinite(x) for x in values), 'finite complete chain matrix')
    return [[values[4*c+r] for c in range(4)] for r in range(4)]


def multiply(a, b):
    return [[sum(a[r][k]*b[k][c] for k in range(4)) for c in range(4)] for r in range(4)]


def affine(matrix, p):
    return tuple(sum(matrix[r][c]*p[c] for c in range(3))+matrix[r][3] for r in range(3))


def direction(matrix, p):
    return tuple(sum(matrix[r][c]*p[c] for c in range(3)) for r in range(3))


def decimal_world(reference, p, vector=False):
    with localcontext() as context:
        context.prec = 80
        v = tuple(D.from_float(float(x)) for x in p)
        return tuple(sum(reference[4*c+r]*v[c] for c in range(3))+(D(0) if vector else reference[12+r]) for r in range(3))


def world_box(matrix, box):
    require(len(box) == 12 and all(type(v) in (int,float) and math.isfinite(v) for v in box), 'finite box')
    # Emitted profile is an axis-aligned local box, including exact zero extents.
    require(all(box[i] == 0 for i in (4,5,6,8,9,10)) and all(box[i] >= 0 for i in (3,7,11)), 'axis-aligned nonnegative local box')
    basis = tuple(tuple(matrix[r][i] for r in range(3)) for i in range(3))
    require(all(abs(dot(basis[i],basis[j])-(i == j)) <= 5e-15 for i in range(3) for j in range(3)) and
            all(abs(a-b)<=5e-15 for a,b in zip(cross(basis[0],basis[1]),basis[2])), 'proper rigid OBB basis')
    half = (box[3],box[7],box[11])
    return {'center':affine(matrix,box[:3]),'basis':basis,'half':half,
            'half_axes':tuple(tuple(v*h for v in axis) for axis,h in zip(basis,half))}


def box_corners(box):
    return [tuple(box[i]+(-1 if bits & (1<<i) else 1)*box[3+4*i] for i in range(3)) for bits in range(8)]


def require_world_contains(box, p):
    displacement = tuple(p[i]-box['center'][i] for i in range(3))
    for axis, half in zip(box['basis'],box['half']):
        require(abs(dot(displacement,axis)) <= half+1e-6, 'world geometry/descendant outside conservative OBB')


def read_world(members):
    """Complete raw tile and GLB-node chain. The independent GLB reader resolves
    node transforms/normal cofactors and applies glTF Y-up conversion exactly once.
    """
    tileset = json.loads(members['tileset.json']); root = tileset['root']
    root_matrix = matrix_rows(root.get('transform', geometry.IDENTITY_TRANSFORM))
    root_box = world_box(root_matrix, root['boundingVolume']['box']); leaves = []
    for child in root['children']:
        complete = multiply(root_matrix, matrix_rows(child.get('transform', geometry.IDENTITY_TRANSFORM)))
        box = world_box(complete, child['boundingVolume']['box'])
        uri = child['content']['uri']
        def resolver(image_uri):
            image_id = int(Path(image_uri).stem)
            return members['textures/'+str(image_id)+Path(image_uri).suffix], 'image/png' if image_uri.endswith('.png') else 'image/jpeg'
        triangles, _, _, _ = core.scene_triangles(members[uri], resolver, output=True)
        world = copy.deepcopy(triangles)
        for triangle in world:
            for corner in triangle['corners']:
                corner['POSITION'] = affine(complete, corner['POSITION'])
                if 'NORMAL' in corner: corner['NORMAL'] = direction(complete,corner['NORMAL'])
                if 'TANGENT' in corner:
                    corner['TANGENT'] = (*direction(complete,corner['TANGENT'][:3]),corner['TANGENT'][3])
        for p in box_corners(child['boundingVolume']['box']): require_world_contains(root_box,affine(complete,p))
        for triangle in world:
            for corner in triangle['corners']:
                require_world_contains(box,corner['POSITION']);require_world_contains(root_box,corner['POSITION'])
        leaves.append({'uri':uri,'local':triangles,'world':world,'matrix':complete,'box':box})
    return tileset, root_box, leaves


def inspect(source, archive, leaf_limit, placement=None):
    expectation, reference = placement_reference(placement)
    local = core.inspect(source,archive,leaf_limit,placement_expectation=expectation)
    with zipfile.ZipFile(archive) as stream: members = {name:stream.read(name) for name in stream.namelist()}
    tileset, root_box, leaves = read_world(members)
    maxima = {'position_metres':0.,'normal_component':0.,'tangent_component':0.}
    for leaf in leaves:
        for local_triangle, world_triangle in zip(leaf['local'],leaf['world']):
            for local_corner, world_corner in zip(local_triangle['corners'],world_triangle['corners']):
                expected = decimal_world(reference,local_corner['POSITION'])
                error = max(abs(float(D.from_float(a)-b)) for a,b in zip(world_corner['POSITION'],expected))
                maxima['position_metres'] = max(maxima['position_metres'],error)
                require(error <= 1e-6, 'world position vs independent Decimal arithmetic')
                for name, metric in (('NORMAL','normal_component'),('TANGENT','tangent_component')):
                    if name not in local_corner: continue
                    expected_vector = decimal_world(reference,local_corner[name][:3],True)
                    error_vector = max(abs(float(D.from_float(a)-b)) for a,b in zip(world_corner[name][:3],expected_vector))
                    maxima[metric] = max(maxima[metric],error_vector)
                    require(error_vector <= 2e-15,'rigid world '+name+' frame')
                    if name == 'TANGENT': require(world_corner[name][3] == local_corner[name][3], 'rigid placement preserves tangent W')
    local.update(world_precision=maxima, world_root_box=root_box, placement_request={} if placement is None else placement,
                 placement_reference_sha256=texture.digest(json.dumps(expectation,sort_keys=True).encode()),
                 world_precision_limit_metres=1e-6, local_bounds_epsilon_metres=0.)
    return local


def mutate_positions(data, value_of):
    doc, binary = geometry.decode_glb(data); payload=bytearray(binary)
    for mesh in doc['meshes']:
        for primitive in mesh['primitives']:
            accessor=doc['accessors'][primitive['attributes']['POSITION']];view=doc['bufferViews'][accessor['bufferView']]
            values=[]
            for i in range(accessor['count']):
                start=view.get('byteOffset',0)+accessor.get('byteOffset',0)+i*view.get('byteStride',12)
                p=struct.unpack_from('<3f',payload,start); q=value_of(p,i);struct.pack_into('<3f',payload,start,*q);values.append(q)
            accessor['min']=[min(p[c] for p in values) for c in range(3)]
            accessor['max']=[max(p[c] for p in values) for c in range(3)]
    return texture.encode_glb(doc,bytes(payload))


def fixture(variant='all-slots'):
    require(variant in VARIANTS,'known placement source variant')
    if variant == 'viewer': return viewer_fixture()
    data=core.fixture(variant if variant in core.VARIANTS else 'all-slots',8)
    if variant in ('zero-extent','line-extent','plane-extent','thin-spike'):
        def shape(p,i):
            if variant=='zero-extent': return (0.,0.,0.)
            if variant=='line-extent': return (float(i),0.,0.)
            if variant=='plane-extent': return (p[0],p[1],0.)
            return (p[0]*.001,p[1]*.001,100. if i==2 else 0.)
        data=mutate_positions(data,shape)
    doc,binary=geometry.decode_glb(data)
    if variant=='instanced':
        doc['nodes'].append({'mesh':0,'translation':[100.,20.,30.]});doc['scenes'][0]['nodes'].append(1)
    if variant=='nested-translation':
        doc['nodes'][0]['translation']=[3.,-5.,7.];doc['nodes'].append({'translation':[-17.,19.,23.],'children':[0]});doc['scenes'][0]['nodes']=[1]
    if variant=='near-limit': doc['nodes'][0]['translation']=[999900.,-999900.,999900.]
    return texture.encode_glb(doc,binary)


def viewer_fixture():
    # Three disjoint small horizontal triangles at asymmetric ENU positions.
    # Each receives a distinct opaque authored emissive material, so no light
    # override is needed. This is placement/coverage evidence, not PBR acceptance.
    doc={'asset':{'version':'2.0'},'scene':0,'scenes':[{'nodes':[0]}],'nodes':[{'mesh':0}],
         'meshes':[{'primitives':[]}],'materials':[], 'accessors':[],'bufferViews':[]}
    payload=bytearray()
    for i, (x,z) in enumerate(((-8.,-5.),(7.,-3.),(-2.,9.))):
        positions=((x,0.,-z),(x+3.,0.,-z),(x,0.,-z-3.))
        view=len(doc['bufferViews']);offset=len(payload);payload.extend(struct.pack('<9f',*(v for p in positions for v in p)))
        doc['bufferViews'].append({'buffer':0,'byteOffset':offset,'byteLength':36})
        accessor=len(doc['accessors']);doc['accessors'].append({'bufferView':view,'componentType':5126,'count':3,'type':'VEC3',
            'min':[min(p[c] for p in positions) for c in range(3)],'max':[max(p[c] for p in positions) for c in range(3)]})
        color=([1.,0.,0.],[0.,1.,0.],[0.,0.,1.])[i]
        doc['materials'].append({'pbrMetallicRoughness':{'baseColorFactor':[0.,0.,0.,1.],'metallicFactor':0.,'roughnessFactor':1.},'emissiveFactor':color,'doubleSided':True})
        doc['meshes'][0]['primitives'].append({'attributes':{'POSITION':accessor},'material':i})
    return texture.encode_glb(doc,bytes(payload))


def write_fixture(root, variant='all-slots', external=False):
    data=fixture(variant);doc,binary=geometry.decode_glb(data)
    files={}
    if external:
        doc['buffers'][0]['uri']='geometry/data.bin';files['geometry/data.bin']=binary
        for i,image in enumerate(doc.get('images',[])):
            view=doc['bufferViews'][image.pop('bufferView')];start=view.get('byteOffset',0);name='images/'+str(i)+'.png'
            files[name]=binary[start:start+view['byteLength']];image['uri']=name
        binary=None
    return binding.write_bundle(root,{'document':doc,'binary':binary,'files':files,'hardlinks':{},'container':'gltf' if external else 'glb'})


def placement_cli_args(placement=None):
    placement={} if placement is None else (PLACEMENTS[placement] if isinstance(placement,str) else placement)
    result=[]
    for field,flag in (('anchor','--anchor'),('orientation_xyzw','--orientation-xyzw'),('scene_offset','--scene-offset')):
        if field in placement:result += [flag,*[str(v) for v in placement[field]]]
    return result


def refusal_cases():
    cases=[]
    def add(name,placement,kind='invalid_request'):
        cases.append((name,placement_cli_args(placement),kind))
    for field,length in (('anchor',3),('orientation_xyzw',4),('scene_offset',3)):
        for axis in range(length):
            for invalid in ('nan','inf','-inf'):
                value=copy.deepcopy(PLACEMENTS['explicit-identity']);value[field][axis]=invalid
                add(field+'-'+str(axis)+'-'+invalid,value)
    for anchor in ([180.000001,0.,0.],[-180.000001,0.,0.],[0.,90.000001,0.],[0.,-90.000001,0.]):add('anchor-range-'+str(anchor),{'anchor':anchor})
    for q in ([0.,0.,0.,0.],[0.,0.,0.,2.],[0.,0.,0.,1.+2e-12],[0.,0.,0.,1.-2e-12],[1e308,1e308,0.,0.]):add('quaternion-norm-'+str(q),{'anchor':[0.,0.,0.],'orientation_xyzw':q})
    add('orientation-without-anchor',{'orientation_xyzw':[0.,0.,0.,1.]})
    add('offset-without-anchor',{'scene_offset':[0.,0.,0.]})
    add('both-without-anchor',{'orientation_xyzw':[0.,0.,0.,1.],'scene_offset':[0.,0.,0.]})
    add('height-budget',{'anchor':[0.,0.,2**26]},'unsupported')
    add('offset-budget',{'anchor':[0.,0.,0.],'scene_offset':[2**26,0.,0.]},'unsupported')
    add('offset-L1-no-cancellation',{'anchor':[0.,0.,0.],'scene_offset':[30e6,-30e6,10e6]},'unsupported')
    add('combined-budget',{'anchor':[0.,0.,30e6],'scene_offset':[30e6,0.,0.]},'unsupported')
    ceiling=2**26-6378137-math.sqrt(3)*1e6
    add('height-immediately-above-budget',{'anchor':[0.,0.,math.nextafter(ceiling,math.inf)]},'unsupported')
    add('cap-plus-minimum-subnormal-offset',{'anchor':[0.,0.,ceiling],'scene_offset':[math.ulp(0.),0.,0.]},'unsupported')
    add('offset-cap-plus-minimum-subnormal',{'anchor':[0.,0.,0.],'scene_offset':[ceiling,-math.ulp(0.),0.]},'unsupported')
    add('mixed-sum-immediately-above-cap',{'anchor':[0.,0.,30e6],'scene_offset':[math.nextafter(ceiling-30e6,math.inf),0.,0.]},'unsupported')
    cases += [('partial-anchor',['--anchor','0','0'],'usage'),('partial-orientation',['--anchor','0','0','0','--orientation-xyzw','0','0','1'],'usage'),('partial-offset',['--anchor','0','0','0','--scene-offset','0','0'],'usage')]
    return cases


def synthetic_archive(root,placement='mixed-offset'):
    source,archive,members=core.synthetic_archive(root)
    expectation,_=placement_reference(placement)
    tileset=json.loads(members['tileset.json']);tileset['root']['transform']=expectation['root_transform'];members['tileset.json']=json.dumps(tileset).encode()
    report=json.loads(members['conversion.json']);report.update(expectation);members['conversion.json']=json.dumps(report).encode()
    geometry.write_control_archive(archive,members)
    inspect(source,archive,2,placement)
    return source,archive,members


def analytic_references():
    values=[]
    with localcontext() as context:
        context.prec=80
        for name,lon,lat,height in ANCHORS:
            origin,basis=cartographic_frame((lon,lat,height))
            values.append({'name':name,'anchor_strings':[lon,lat,height], 'origin_ecef_decimal':[str(x) for x in origin],
                           'enu_columns_decimal':[[str(x) for x in axis] for axis in basis]})
    return {'provenance':'Independently authored Decimal80 Taylor trigonometry; exact cardinal sin/cos; NGA a=6378137, 1/f=298.257223563. No converter or CRS-helper calls.',
            'primary_reference':'https://earth-info.nga.mil/index.php?action=wgs84&dir=wgs84','anchors':values,
            'rational_quaternion_xyzw':[1,2,3,4], 'rational_quaternion_norm_squared':30,
            'rational_rotation_rows':[['2/15','-2/3','11/15'],['14/15','1/3','2/15'],['-1/3','2/3','2/3']]}


def self_test():
    frozen=json.loads(REFERENCE_PATH.read_text());require(frozen==analytic_references(),'frozen analytic provenance/reference mismatch')
    require([D(x) for x in frozen['anchors'][0]['origin_ecef_decimal']]==[6378137,0,0],'literal equatorial ECEF')
    require([[D(x) for x in axis] for axis in frozen['anchors'][5]['enu_columns_decimal']]==[[0,1,0],[-1,0,0],[0,0,1]],'literal north basis')
    require([[D(x) for x in axis] for axis in frozen['anchors'][7]['enu_columns_decimal']]==[[0,1,0],[1,0,0],[0,0,-1]],'literal south basis')
    with localcontext() as context:
        context.prec=80
        q=tuple(D(i)/D(30).sqrt() for i in (1,2,3,4));p=(D(7),D(-11),D(13))
        expected=tuple(sum(D(Fraction(value).numerator)/D(Fraction(value).denominator)*p[i] for i,value in enumerate(row)) for row in frozen['rational_rotation_rows'])
        require(all(abs(a-b)<D('1e-75') for a,b in zip(rotate(q,p),expected)),'literal rational Hamilton rotation')
    # Operative cap has an exact real-number sum, not rounded scalar addition.
    cap = Fraction.from_float(58_998_676.192_431_12)
    tiny = Fraction.from_float(math.ulp(0.))
    require(cap+tiny>cap and Fraction.from_float(math.nextafter(float(cap),math.inf))>cap, 'sensitive exact numerical cap')
    require(Fraction.from_float(30e6)+Fraction.from_float(float(cap)-30e6)==cap, 'literal mixed exact cap')
    controls={}
    def rejected(name,operation):
        try:operation()
        except (OracleError,KeyError,ValueError,IndexError):controls[name]='rejected'
        else:raise OracleError('insensitive placement control '+name)
    with tempfile.TemporaryDirectory(prefix='f1c1-independent-controls-') as temporary:
        root=Path(temporary);source,archive,members=synthetic_archive(root)
        for name,change in (
            ('missing-root-transform',lambda t:t['root'].pop('transform')),
            ('wrong-height',lambda t:t['root']['transform'].__setitem__(12,t['root']['transform'][12]+130)),
            ('wrong-linear-axis',lambda t:t['root']['transform'].__setitem__(0,-t['root']['transform'][0])),
            ('transpose-linear',lambda t:t['root'].__setitem__('transform',[t['root']['transform'][(i%4)*4+i//4] if i<12 and i%4!=3 else t['root']['transform'][i] for i in range(16)])),
            ('double-axis-conversion',lambda t:t['root'].__setitem__('transform',
                [v for col in zip(*multiply(matrix_rows(t['root']['transform']),[[1.,0.,0.,0.],[0.,0.,-1.,0.],[0.,1.,0.,0.],[0.,0.,0.,1.]])) for v in col])),
            ('world-box-centre',lambda t:t['root']['children'][0]['boundingVolume']['box'].__setitem__(0,1000000)),
            ('undersized-box',lambda t:t['root']['children'][0]['boundingVolume']['box'].__setitem__(3,.1)),
            ('wrong-routing-error',lambda t:t['root'].__setitem__('geometricError',0)),
            ('unexpected-child-transform',lambda t:t['root']['children'][0].__setitem__('transform',list(geometry.IDENTITY_TRANSFORM))),
        ):
            changed=copy.deepcopy(members);tileset=json.loads(changed['tileset.json']);change(tileset);changed['tileset.json']=json.dumps(tileset).encode()
            geometry.write_control_archive(archive,changed);rejected(name,lambda:inspect(source,archive,2,'mixed-offset'))
        for name,change in (
            ('report-transform-mismatch',lambda r:r['root_transform'].__setitem__(12,r['root_transform'][12]+1)),
            ('report-anchor-mismatch',lambda r:r['placement']['anchor_degrees_metres'].__setitem__(2,0)),
            ('report-offset-mismatch',lambda r:r['placement']['scene_offset_metres'].__setitem__(0,0)),
            ('report-quaternion-mismatch',lambda r:r['placement']['orientation_xyzw'].__setitem__(0,0)),
            ('report-interpretation',lambda r:r.__setitem__('coordinates','local-gltf')),
        ):
            changed=copy.deepcopy(members);report=json.loads(changed['conversion.json']);change(report);changed['conversion.json']=json.dumps(report).encode()
            geometry.write_control_archive(archive,changed);rejected(name,lambda:inspect(source,archive,2,'mixed-offset'))
        # Complete-chain analytical fixture with nonidentity GLB node AND child.
        changed=copy.deepcopy(members);doc,binary=geometry.decode_glb(changed['t/0.glb']);doc['nodes'][0]['translation']=[1.,2.,3.]
        changed['t/0.glb']=texture.encode_glb(doc,binary)
        tileset=json.loads(changed['tileset.json']);tileset['root']['transform']=list(geometry.IDENTITY_TRANSFORM)
        child=tileset['root']['children'][0];child['transform']=list(geometry.IDENTITY_TRANSFORM);child['transform'][12:15]=[5.,7.,11.]
        child['boundingVolume']['box']=[3.5,-3.,2.5,3.,0.,0.,0.,2.,0.,0.,0.,2.]
        tileset['root']['boundingVolume']['box']=[8.5,4.,13.5,4.,0.,0.,0.,3.,0.,0.,0.,3.]
        changed['tileset.json']=json.dumps(tileset).encode();_,_,leaves=read_world(changed)
        require(leaves[0]['world'][0]['corners'][0]['POSITION']==(6.,4.,13.),'literal complete source-node/C/child/root chain')
        for name,localp in (('zero-dimension',(0.,0.,0.)),('plane-dimension',(1.,2.,0.)),('line-dimension',(1.,0.,0.))):
            expectation,_=placement_reference('mixed-offset');matrix=matrix_rows(expectation['root_transform'])
            half=(abs(localp[0]),abs(localp[1]),abs(localp[2]));box=[0.,0.,0.,half[0],0.,0.,0.,half[1],0.,0.,0.,half[2]]
            require_world_contains(world_box(matrix,box),affine(matrix,localp));controls[name]='positive-passed'
            bad=list(affine(matrix,localp));bad[0]+=5.;rejected('outside-'+name,lambda:require_world_contains(world_box(matrix,box),bad))
    return {'frozen_anchor_count':len(ANCHORS),'literal_cardinals_rational_rotation':'passed','complete_nonidentity_chain':'passed',
            'crc_valid_archive_controls':controls,'source_variants':len(VARIANTS),'placement_requests':len(PLACEMENTS),
            'request_refusal_cases':len(refusal_cases()),'scope':'Independent synthetic/static proof only; no production execution in self-test.'}


def write_static(root):
    root=Path(root);root.mkdir(parents=True,exist_ok=True);records=[]
    for variant in VARIANTS:
        source=write_fixture(root/variant,variant,external=variant in EXTERNAL_VARIANTS)
        assets=[{'path':str(p.relative_to(root)),'bytes':p.stat().st_size,'sha256':texture.digest(p.read_bytes())} for p in sorted(source.parent.rglob('*')) if p.is_file()]
        records.append({'variant':variant,'source':str(source.relative_to(root)),'assets':assets})
    manifest={'author':'Independent stdlib fixture author; reuses independently established F1b3 GLB/PBR/resource truth, no production helpers',
              'profile':PROFILE,'placements':PLACEMENTS,'fixtures':records,'reference_sha256':texture.digest(REFERENCE_PATH.read_bytes())}
    (root/'manifest.json').write_text(json.dumps(manifest,indent=2,allow_nan=False)+'\n');return manifest


def run_binary(binary):
    binary=Path(binary).resolve(strict=True);cases=[];refusals=[];content_by_variant={}
    with tempfile.TemporaryDirectory(prefix='f1c1-candidate-') as temporary:
        root=Path(temporary)
        plan=[('all-slots',name,limit,False) for name in PLACEMENTS for limit in (1,3,1000)]
        plan += [(variant,'mixed-offset',limit,variant in EXTERNAL_VARIANTS) for variant in VARIANTS for limit in (1,3,1000)]
        ceiling=2**26-6378137-math.sqrt(3)*1e6
        budget_places={'height-boundary':{'anchor':[0.,0.,ceiling]},'height-below-boundary':{'anchor':[0.,0.,math.nextafter(ceiling,-math.inf)]},
                       'offset-boundary':{'anchor':[0.,0.,0.],'scene_offset':[ceiling,0.,0.]},
                       'mixed-exact-boundary':{'anchor':[0.,0.,30e6],'scene_offset':[ceiling-30e6,0.,0.]},
                       'mixed-below-boundary':{'anchor':[0.,0.,30e6],'scene_offset':[math.nextafter(ceiling-30e6,-math.inf),0.,0.]}}
        plan += [('near-limit',name,3,False) for name in budget_places]
        for scenario_index,(variant,name,limit,external) in enumerate(plan):
            placement=budget_places.get(name,PLACEMENTS.get(name));source_root=root/'sources'/(variant+('-external' if external else '-embedded'))
            source=write_fixture(source_root,variant,external) if not source_root.exists() else next(source_root.glob('source.*'))
            output=root/'outputs'/(str(scenario_index)+'-'+variant+'-'+name+'-'+str(limit)+('-external' if external else '-embedded')+'.3tz')
            source_inventory={str(p.relative_to(source_root)):texture.digest(p.read_bytes()) if p.is_file() else None for p in sorted(source_root.rglob('*'))}
            command=[str(binary),'--json','mesh-local-to-3tz','-i',str(source),'-o',str(output),'--leaf-triangles',str(limit),*placement_cli_args(placement)]
            completed=subprocess.run(command,text=True,capture_output=True,timeout=60)
            require(completed.returncode==0,'placement candidate '+name+'/'+variant+': '+completed.stdout+completed.stderr)
            require(source_inventory=={str(p.relative_to(source_root)):texture.digest(p.read_bytes()) if p.is_file() else None for p in sorted(source_root.rglob('*'))}, 'conversion changed source bytes/inventory or left source work')
            summary=json.loads(completed.stdout);inspected=inspect(source,output,limit,placement)
            require(summary.get('ok') is True and summary['meshReport']==inspected['report'],'CLI/raw report parity')
            require(limit >= inspected['source_triangles'] or inspected['leaves']>1,'forced multileaf placement')
            with zipfile.ZipFile(output) as stream:content={n:hashlib.sha256(stream.read(n)).hexdigest() for n in stream.namelist() if n.endswith('.glb') or n.startswith('textures/')}
            key=(variant,limit,external)
            if key in content_by_variant:require(content_by_variant[key]==content,'placement changed local leaf/resource bytes')
            else:content_by_variant[key]=content
            cases.append({'scenario_index':scenario_index,'source_inventory':source_inventory,'source_unchanged':True,'variant':variant,'placement_name':name,'leaf_limit':limit,'external':external,**inspected})
        for name,arguments,kind in refusal_cases():
            for missing in (False,True):
                source=root/'missing.glb' if missing else write_fixture(root/'refusal-source')
                output=root/'uncreated'/name.replace('/','_')/('missing' if missing else 'present')/'result.3tz'
                completed=subprocess.run([str(binary),'--json','mesh-local-to-3tz','-i',str(source),'-o',str(output),'--leaf-triangles','1',*arguments],text=True,capture_output=True,timeout=60)
                summary=json.loads(completed.stdout)
                if kind == 'usage':
                    require(completed.returncode == 2 and summary.get('exitCode') == 2 and summary.get('ok') is False and
                            summary.get('error',{}).get('code') == 'usage' and 'kind' not in summary['error'],
                            'CLI pre-producer arity transport '+name+': '+completed.stdout+completed.stderr)
                else:
                    require(completed.returncode == 2 and summary.get('ok') is False and summary['error']['kind'] == kind,
                            'typed request refusal precedence '+name+': '+completed.stdout+completed.stderr)
                require(not output.parent.exists(),'placement refusal created staging/output work')
                refusals.append({'name':name,'missing_source':missing,'transport':'clap-usage' if kind == 'usage' else 'typed-job-failure',
                                 'kind':kind,'output_parent_exists':False})
    return {'binary':str(binary),'binary_sha256':texture.digest(binary.read_bytes()),'positive_cases':cases,'request_refusals':refusals}


def main():
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--self-test',action='store_true');parser.add_argument('--binary',type=Path)
    parser.add_argument('--static-fixtures',type=Path);parser.add_argument('--fixture',type=Path);parser.add_argument('--variant',choices=VARIANTS,default='all-slots')
    parser.add_argument('--external',action='store_true');parser.add_argument('--write-references',action='store_true')
    parser.add_argument('--source',type=Path);parser.add_argument('--archive',type=Path);parser.add_argument('--leaf-limit',type=int)
    parser.add_argument('--placement',choices=tuple(PLACEMENTS),default='local');parser.add_argument('--json-output',type=Path)
    args=parser.parse_args();result={}
    if args.write_references:REFERENCE_PATH.parent.mkdir(parents=True,exist_ok=True);REFERENCE_PATH.write_text(json.dumps(analytic_references(),indent=2)+'\n')
    if args.self_test or args.binary:result['self_test']=self_test()
    if args.static_fixtures:result['static_fixtures']=write_static(args.static_fixtures)
    if args.fixture:result['fixture']=str(write_fixture(args.fixture,args.variant,args.external))
    if args.archive:require(args.source and args.leaf_limit,'inspection needs source/leaf limit');result['inspection']=inspect(args.source,args.archive,args.leaf_limit,PLACEMENTS[args.placement])
    if args.binary:result['candidate']=run_binary(args.binary)
    encoded=json.dumps(result,indent=2,allow_nan=False)+'\n'
    if args.json_output:args.json_output.parent.mkdir(parents=True,exist_ok=True);args.json_output.write_text(encoded)
    else:print(encoded,end='')


if __name__=='__main__':main()
