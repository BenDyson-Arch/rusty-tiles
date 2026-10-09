"""Independent T1 GLB/3D Tiles semantic oracle; no producer imports or GDAL warp.

Run with ``python tests/t1_terrain_oracle.py DIRECTORY``. Fixture creation uses
GDAL only to write explicitly described source values, never to sample them.
"""
import argparse
import json
import math
import pathlib
import struct


def ecef(lon, lat, height):
    """WGS84 from a and inverse flattening, independently of producer constants."""
    a = 6378137.0
    f = 1 / 298.257223563
    e2 = f * (2 - f)
    lon, lat = math.radians(lon), math.radians(lat)
    n = a / math.sqrt(1 - e2 * math.sin(lat) ** 2)
    return ((n + height) * math.cos(lat) * math.cos(lon),
            (n + height) * math.cos(lat) * math.sin(lon),
            (n * (1 - e2) + height) * math.sin(lat))


def glb(path):
    data = pathlib.Path(path).read_bytes()
    assert len(data) >= 28, "truncated GLB"
    magic, version, length = struct.unpack_from('<4sII', data)
    assert (magic, version, length) == (b'glTF', 2, len(data)), "GLB envelope"
    chunks = []
    at = 12
    while at < len(data):
        count, kind = struct.unpack_from('<I4s', data, at)
        at += 8
        assert count % 4 == 0 and at + count <= len(data), "GLB chunk range"
        chunks.append((kind, data[at:at + count]))
        at += count
    assert [kind for kind, _ in chunks] == [b'JSON', b'BIN\x00'], "GLB chunks"
    document = json.loads(chunks[0][1])
    binary = chunks[1][1]
    assert document['asset']['version'] == '2.0'
    assert len(document['buffers']) == 1 and 'uri' not in document['buffers'][0]
    assert document['buffers'][0]['byteLength'] <= len(binary)

    def accessor(index):
        record = document['accessors'][index]
        assert not record.get('normalized', False) and 'sparse' not in record
        view = document['bufferViews'][record['bufferView']]
        assert view.get('buffer', 0) == 0
        fmt, size = {5121: ('B', 1), 5123: ('H', 2), 5125: ('I', 4), 5126: ('f', 4)}[record['componentType']]
        width = {'SCALAR': 1, 'VEC2': 2, 'VEC3': 3}[record['type']]
        offset = view.get('byteOffset', 0) + record.get('byteOffset', 0)
        stride = view.get('byteStride', width * size)
        end = offset + (record['count'] - 1) * stride + width * size
        assert end <= view.get('byteOffset', 0) + view['byteLength'] <= len(binary), 'accessor payload'
        values = [struct.unpack_from('<' + fmt * width, binary, offset + i * stride)
                  for i in range(record['count'])]
        assert all(math.isfinite(component) for value in values for component in value)
        if record['type'] == 'VEC3' and 'min' in record:
            for axis in range(3):
                assert record['min'][axis] <= min(v[axis] for v in values)
                assert record['max'][axis] >= max(v[axis] for v in values)
        return values

    primitives = []
    for mesh in document['meshes']:
        for primitive in mesh['primitives']:
            assert primitive.get('mode', 4) == 4
            positions = accessor(primitive['attributes']['POSITION'])
            indices = [v[0] for v in accessor(primitive['indices'])] if 'indices' in primitive else list(range(len(positions)))
            assert len(indices) % 3 == 0 and all(0 <= i < len(positions) for i in indices)
            triangles = [tuple(positions[i] for i in indices[start:start + 3]) for start in range(0, len(indices), 3)]
            primitives.append((positions, triangles))
    return document, primitives


def transform(matrix, point):
    return tuple(sum(matrix[row + 4 * col] * point[col] for col in range(3)) + matrix[row + 12]
                 for row in range(3))


def contains_box(box, points):
    """Solve the three box axes; accept zero-axis degenerate coordinate slabs."""
    center = box[:3]
    axes = [box[3:6], box[6:9], box[9:12]]
    # Initial profile uses orthogonal boxes. Fail instead of assuming a sheared box.
    for i in range(3):
        for j in range(i):
            assert abs(sum(a*b for a, b in zip(axes[i], axes[j]))) <= 1e-8
    for p in points:
        d = [p[i] - center[i] for i in range(3)]
        for axis in axes:
            size2 = sum(a*a for a in axis)
            if size2:
                assert abs(sum(a*b for a, b in zip(d, axis))) <= size2 + 1e-7 * math.sqrt(size2), 'content outside box'
            else:
                # With zero axis, require residual to lie in the other axis span.
                residual = d[:]
                for other in axes:
                    norm2 = sum(a*a for a in other)
                    if norm2:
                        t = sum(a*b for a, b in zip(d, other))/norm2
                        residual = [a-t*b for a,b in zip(residual,other)]
                assert math.sqrt(sum(a*a for a in residual)) <= 1e-7, 'outside degenerate box'


def check_directory(directory):
    directory = pathlib.Path(directory)
    manifest = json.loads((directory/'tileset.json').read_text())
    assert manifest['asset']['version'] == '1.1'
    root = manifest['root']
    assert 'content' not in root and root.get('refine') == 'REPLACE'
    leaves = root['children']
    assert leaves and all(not leaf.get('children') and leaf['geometricError'] == 0 for leaf in leaves)
    inventory = set()
    triangle_count = 0
    vertex_count = 0
    identity = [1,0,0,0,0,1,0,0,0,0,1,0,0,0,0,1]
    for leaf in leaves:
        uri = leaf['content']['uri']
        assert not pathlib.PurePosixPath(uri).is_absolute() and '..' not in pathlib.PurePosixPath(uri).parts
        assert uri not in inventory
        inventory.add(uri)
        document, primitives = glb(directory/uri)
        assert len(document['nodes']) == 1 and not any(k in document['nodes'][0] for k in ('matrix','translation','rotation','scale'))
        tile_points = [(p[0], -p[2], p[1]) for positions, _ in primitives for p in positions]
        # T1 leaves share the root frame; every emitted child box must be
        # enclosed exactly, separately from content-point arithmetic tolerance.
        assert 'transform' not in leaf, 'unexpected leaf frame'
        child_box=leaf['boundingVolume']['box'];root_box=root['boundingVolume']['box']
        for axis,half in [(0,3),(1,7),(2,11)]:
            assert root_box[axis]-root_box[half] <= child_box[axis]-child_box[half], 'child box below root minimum'
            assert root_box[axis]+root_box[half] >= child_box[axis]+child_box[half], 'child box above root maximum'
        contains_box(leaf['boundingVolume']['box'], tile_points)
        parent_points = [transform(leaf.get('transform', identity), p) for p in tile_points]
        contains_box(root['boundingVolume']['box'], parent_points)
        triangle_count += sum(len(tris) for _, tris in primitives)
        vertex_count += sum(len(positions) for positions, _ in primitives)
    actual = {p.relative_to(directory).as_posix() for p in directory.rglob('*.glb')}
    assert inventory == actual, 'manifest GLB inventory'
    report=json.loads((directory/'conversion.json').read_text())
    assert report['tiles']==len(leaves) and report['vertices']==vertex_count, 'report geometry inventory'
    assert triangle_count==2*report['width']*report['height'], 'report source-cell inventory'
    assert report['routingErrorMetres']==root['geometricError']==manifest['geometricError'], 'routing report disagreement'
    expected_members=inventory|{'conversion.json','tileset.json'}
    members={p.relative_to(directory).as_posix() for p in directory.rglob('*') if p.is_file()}
    assert members==expected_members, 'unexpected directory member inventory'
    assert report['generatedBytes']==sum((directory/member).stat().st_size for member in members), 'report byte inventory'
    return {'leaf_tiles': len(leaves), 'triangles': triangle_count}


def check_plane(directory, width=4, height=4, pixel=.25, west=10., north_edge=21.):
    """Complete independent node and oriented-cell inventory for t1-plane.tif."""
    from collections import Counter
    directory=pathlib.Path(directory)
    report=json.loads((directory/'conversion.json').read_text())
    south=north_edge-height*pixel;east_edge=west+width*pixel
    assert report['bounds']==[west,south,east_edge,north_edge] and (report['width'],report['height'])==(width,height)
    root=json.loads((directory/'tileset.json').read_text())['root']
    lon,lat=math.radians((west+east_edge)/2),math.radians((south+north_edge)/2)
    east=(-math.sin(lon),math.cos(lon),0.)
    north=(-math.sin(lat)*math.cos(lon),-math.sin(lat)*math.sin(lon),math.cos(lat))
    up=(math.cos(lat)*math.cos(lon),math.cos(lat)*math.sin(lon),math.sin(lat))
    origin=ecef((west+east_edge)/2,(south+north_edge)/2,0.)
    expected_transform=[*east,0.,*north,0.,*up,0.,*origin,1.]
    assert all(abs(a-b)<=1e-8 for a,b in zip(root['transform'],expected_transform)), 'root ENU frame'
    expected={}
    for j in range(height+1):
        for i in range(width+1):
            sample_height=100.+2*max(0.,min(width-1.,i-.5))+3*max(0.,min(height-1.,height-j-.5))+report['heightOffsetMetres']
            xyz=ecef(west+i*pixel,south+j*pixel,sample_height)
            relative=[a-b for a,b in zip(xyz,origin)]
            enu=[sum(a*b for a,b in zip(relative,axis)) for axis in [east,north,up]]
            expected[(i,j)]=tuple(struct.unpack('<f',struct.pack('<f',v))[0] for v in [enu[0],enu[2],-enu[1]])
    found=Counter();seen={}
    for leaf in root['children']:
        _,primitives=glb(directory/leaf['content']['uri'])
        def identify(p):
            key=min(expected,key=lambda k:sum((a-b)**2 for a,b in zip(p,expected[k])))
            assert max(abs(a-b) for a,b in zip(p,expected[key]))<=.002, 'source elevation/position'
            if key in seen: assert seen[key]==p, 'shared seam changed stored coordinates'
            seen[key]=p
            return key
        for positions,triangles in primitives:
            for p in positions: identify(p)
            for triangle in triangles: found[tuple(identify(p) for p in triangle)]+=1
    wanted=Counter()
    for j in range(height):
        for i in range(width):
            wanted[((i,j),(i+1,j),(i,j+1))]+=1
            wanted[((i+1,j),(i+1,j+1),(i,j+1))]+=1
    assert found==wanted, 'missing/duplicated/reversed terrain cells'
    assert set(seen)==set(expected), 'source lattice nodes missing'
    return dict(nodes=len(seen),triangles=sum(found.values()))


def corruption_controls(directory, **plane_spec):
    """Demonstrate source, frame, topology, seams, bounds and receipt sensitivity."""
    import shutil, tempfile
    directory=pathlib.Path(directory)
    check_directory(directory);check_plane(directory,**plane_spec)
    manifest=json.loads((directory/'tileset.json').read_text())
    faults=['position','winding','bounds','frame','row','missing-triangle','report-inventory']
    if len(manifest['root']['children'])>1: faults.append('seam')
    for fault in faults:
        with tempfile.TemporaryDirectory() as temporary:
            copy=pathlib.Path(temporary)/'candidate';shutil.copytree(directory,copy)
            manifest=json.loads((copy/'tileset.json').read_text())
            if fault in ['bounds','frame']:
                if fault=='bounds': manifest['root']['boundingVolume']['box'][3]=0.
                else: manifest['root']['transform'][12]+=10.
                (copy/'tileset.json').write_text(json.dumps(manifest))
            elif fault=='report-inventory':
                path=copy/'conversion.json';report=json.loads(path.read_text());report['tiles']+=1
                path.write_text(json.dumps(report))
            else:
                leaf=manifest['root']['children'][1 if fault=='seam' else 0]
                path=copy/leaf['content']['uri']
                data=bytearray(path.read_bytes());json_length=struct.unpack_from('<I',data,12)[0]
                document=json.loads(data[20:20+json_length]);binary_start=28+json_length
                primitive=document['meshes'][0]['primitives'][0]
                position_fault=fault in ['position','row','seam']
                index=primitive['attributes']['POSITION'] if position_fault else primitive['indices']
                accessor=document['accessors'][index];view=document['bufferViews'][accessor['bufferView']]
                at=binary_start+view.get('byteOffset',0)+accessor.get('byteOffset',0)
                if fault=='position':
                    value=struct.unpack_from('<f',data,at)[0];struct.pack_into('<f',data,at,value+10.)
                elif fault=='seam':
                    bits=struct.unpack_from('<I',data,at)[0];struct.pack_into('<I',data,at,bits+1)
                elif fault=='row':
                    # Exchange complete stored vertices, preserving count and
                    # values while corrupting their row/topology association.
                    other=at+(accessor['count']-1)*12
                    first,last=bytes(data[at:at+12]),bytes(data[other:other+12])
                    data[at:at+12],data[other:other+12]=last,first
                else:
                    fmt={5121:'B',5123:'H',5125:'I'}[accessor['componentType']];size=struct.calcsize(fmt)
                    if fault=='missing-triangle':
                        first=struct.unpack_from('<'+fmt,data,at)[0]
                        struct.pack_into('<'+fmt,data,at+size,first)
                    else:
                        a,b=struct.unpack_from('<'+fmt*2,data,at+size)
                        struct.pack_into('<'+fmt*2,data,at+size,b,a)
                path.write_bytes(data)
            try:
                check_plane(copy,**plane_spec);check_directory(copy)
            except AssertionError:
                continue
            raise AssertionError('oracle accepted corruption: '+fault)
    return {'rejected_corruptions':faults}


def decoded_ecef_triangles(directory):
    directory = pathlib.Path(directory)
    root = json.loads((directory/'tileset.json').read_text())['root']
    identity = [1,0,0,0,0,1,0,0,0,0,1,0,0,0,0,1]
    triangles=[]
    for leaf in root['children']:
        _, primitives=glb(directory/leaf['content']['uri'])
        for _, faces in primitives:
            for face in faces:
                triangles.append(tuple(transform(root.get('transform',identity),
                    transform(leaf.get('transform',identity),(p[0],-p[2],p[1]))) for p in face))
    return triangles


def ray_height(triangles, lon, lat):
    # WGS84 geodetic height is distance along this normal from ellipsoid h=0.
    origin=ecef(lon,lat,0.)
    lon,lat=math.radians(lon),math.radians(lat)
    direction=(math.cos(lat)*math.cos(lon),math.cos(lat)*math.sin(lon),math.sin(lat))
    sub=lambda a,b:tuple(x-y for x,y in zip(a,b))
    dot=lambda a,b:sum(x*y for x,y in zip(a,b))
    cross=lambda a,b:(a[1]*b[2]-a[2]*b[1],a[2]*b[0]-a[0]*b[2],a[0]*b[1]-a[1]*b[0])
    hits=[]
    for a,b,c in triangles:
        ab,ac=sub(b,a),sub(c,a)
        normal=cross(ab,ac)
        divisor=dot(normal,direction)
        if abs(divisor)<1e-12: continue
        height=dot(normal,sub(a,origin))/divisor
        point=tuple(o+height*d for o,d in zip(origin,direction))
        ap=sub(point,a)
        d00,d01,d11=dot(ab,ab),dot(ab,ac),dot(ac,ac)
        denominator=d00*d11-d01*d01
        if denominator<=0: continue
        v=(d11*dot(ap,ab)-d01*dot(ap,ac))/denominator
        w=(d00*dot(ap,ac)-d01*dot(ap,ab))/denominator
        if v>=-1e-9 and w>=-1e-9 and v+w<=1+1e-9: hits.append(height)
    assert hits, 'query has no decoded mesh intersection'
    assert max(hits)-min(hits)<1e-5, 'multiple incompatible mesh intersections'
    return sum(hits)/len(hits)


def browser_oracle(directory, destination):
    directory=pathlib.Path(directory)
    report=json.loads((directory/'conversion.json').read_text())
    west,south,east,north=report['bounds']
    triangles=decoded_ecef_triangles(directory)
    samples=[]
    # Interior irrational fractions exercise triangles rather than just vertices.
    for u,v in [(0.173,0.237),(0.503,0.507),(0.431,0.823)]:
        lon,lat=west+(east-west)*u,south+(north-south)*v
        samples.append(dict(longitude=lon,latitude=lat,height=ray_height(triangles,lon,lat),tolerance=.05))
    outside=[dict(longitude=west-(east-west)*.25,latitude=(south+north)/2)]
    # Constant-latitude Cartesian boundary chords bow poleward. A source ROI
    # guard must reject this point even though a raw decoded-mesh ray hits it.
    # The spherical expression chooses an interior fraction of the bow; the
    # independent WGS84/actual stored-mesh intersection proves the control.
    step=report['sourcePixelDegrees'][0]
    half_step=math.radians(step/2)
    for edge in [north,south]:
        bow=math.degrees(math.atan(math.tan(math.radians(edge))/math.cos(half_step)))-edge
        latitude=edge+bow/4
        if south<=latitude<=north or latitude==edge:
            continue
        longitude=west+step/2
        try:
            underlying_height=ray_height(triangles,longitude,latitude)
        except AssertionError:
            continue
        outside.append(dict(longitude=longitude,latitude=latitude,
                            underlyingMeshHeight=underlying_height,tolerance=.05))
        break
    oracle=dict(samples=samples,outside=outside,
                minimumLeaves=len(json.loads((directory/'tileset.json').read_text())['root']['children']))
    pathlib.Path(destination).write_text(json.dumps(oracle,indent=2)+'\n')
    return oracle


def write_fixture(path, case='plane', width=4, height=4, pixel=.25, options=None):
    from osgeo import gdal, osr
    dataset = gdal.GetDriverByName('GTiff').Create(str(path), width, height, 1, gdal.GDT_Float64, options=options or [])
    dataset.SetGeoTransform([10., pixel, 0., 21., 0., -pixel])
    crs = osr.SpatialReference(); crs.ImportFromEPSG(4326)
    dataset.SetProjection(crs.ExportToWkt())
    # Pixel centre plane h=100+2*column+3*row, north-down source order.
    values = [100.+2*column+3*row for row in range(height) for column in range(width)]
    if case == 'nodata':
        values[5] = -9999.; dataset.GetRasterBand(1).SetNoDataValue(-9999.)
    elif case == 'constant':
        values = [42.] * (width*height)
    elif case == 'all-invalid':
        values = [-9999.] * (width*height); dataset.GetRasterBand(1).SetNoDataValue(-9999.)
    elif case == 'extreme-height':
        values=[9001.] * (width*height)
    elif case == 'nonfinite':
        values[5] = float('inf')
    elif case != 'plane':
        raise ValueError(case)
    dataset.GetRasterBand(1).SetUnitType('m')
    dataset.GetRasterBand(1).WriteRaster(0,0,width,height,struct.pack('<'+str(width*height)+'d',*values),buf_type=gdal.GDT_Float64)
    dataset = None


if __name__ == '__main__':
    parser = argparse.ArgumentParser()
    parser.add_argument('directory')
    parser.add_argument('--browser-oracle')
    parser.add_argument('--browser-imagery',action='store_true')
    parser.add_argument('--plane',action='store_true')
    parser.add_argument('--corruption-controls',action='store_true')
    arguments = parser.parse_args()
    print(json.dumps(check_directory(arguments.directory), sort_keys=True))
    if arguments.plane:
        print(json.dumps(check_plane(arguments.directory),sort_keys=True))
    if arguments.corruption_controls:
        print(json.dumps(corruption_controls(arguments.directory),sort_keys=True))
    if arguments.browser_oracle:
        result=browser_oracle(arguments.directory,arguments.browser_oracle)
        result['imagery']=arguments.browser_imagery
        pathlib.Path(arguments.browser_oracle).write_text(json.dumps(result,indent=2)+'\n')
