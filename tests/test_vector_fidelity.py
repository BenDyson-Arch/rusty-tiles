"""Approved finite 0.4 bar: analytic source truth against published forest bytes."""
import copy
import json
import pathlib
import struct
import subprocess
import tempfile
import unittest
import zipfile

from cli_bin import BIN, requires_bin
from vector_fidelity_oracle import check_source, decode, ecef, forest
from ext_meshopt_spec import decode_attributes
from t1_terrain_oracle import glb_parts


def feature(fid, kind, coordinates):
    return dict(type='Feature', id=fid, properties=dict(name='feature-'+str(fid),
        integer=2**60+fid, ratio=.125+fid/7, nullable=None if fid % 2 else fid, flag=bool(fid % 2),
        tags=[fid, None, 'x']), geometry=dict(type=kind, coordinates=coordinates))


def ring(x, y, size, z=5):
    return [[x,y,z],[x+size,y,z],[x+size,y+size,z],[x,y+size,z],[x,y,z]]


def expected_properties(f, layer='source'):
    p = dict(f['properties'])
    p['tags'] = json.dumps(p['tags'], separators=(',', ':'))
    return dict(p, _source_id=str(f['id']), _source_layer=layer)


def fixture():
    features = [feature(0, 'Point', [1, 2, 3]),
        feature(1, 'MultiLineString', [[[2,20,4],[7,23,8],[9,21,2]], [[20,25,7],[26,29,3]]]),
        feature(2, 'Polygon', [ring(0,0,10), list(reversed(ring(3,3,4)))]),
        feature(3, 'MultiPolygon', [[ring(20,0,4)], [ring(30,0,4)]])]
    features += [feature(i, 'Point', [i*7, i%5*11, i%7-3]) for i in range(4,68)]
    expected = {}
    for f in features:
        mode = {'Point':0, 'MultiLineString':3, 'Polygon':4, 'MultiPolygon':4}[f['geometry']['type']]
        spec = dict(mode=mode, properties=expected_properties(f))
        coordinates = f['geometry']['coordinates']
        if mode == 0:
            spec['parts'] = [(coordinates,)]
        elif mode == 3:
            spec['parts'] = [(a,b) for line in coordinates for a,b in zip(line,line[1:])]
        else:
            spec['height'] = 5
            spec['rectangles'] = ([(0,0,10,3),(0,7,10,10),(0,3,3,7),(7,3,10,7)] if f['id'] == 2
                                  else [(20,0,24,4),(30,0,34,4)])
        expected[str(f['id'])] = spec
    return features, expected


def mutate_payload(data, mutation):
    doc, binary = glb_parts(data)
    views = []
    for v in doc['bufferViews']:
        ext = v.pop('extensions', {}).get('EXT_meshopt_compression')
        if ext:
            at = ext.get('byteOffset',0)
            raw = decode_attributes(binary[at:at+ext['byteLength']],ext['count'],ext['byteStride'])
        else:
            at = v.get('byteOffset',0)
            raw = binary[at:at+v['byteLength']]
        views.append(bytearray(raw))
    primitive = doc['meshes'][0]['primitives'][0]
    if mutation == 'missing':
        doc['meshes'][0]['primitives'] = []
    elif mutation == 'id':
        table = doc['extensions']['EXT_structural_metadata']['propertyTables'][0]
        column = table['properties']['_source_id']
        raw = views[column['values']]
        assert raw == b'2'
        raw[:] = b'9'
    else:
        polygon = primitive['extensions']['EXT_mesh_polygon']
        loops = doc['accessors'][polygon['loopIndices']]
        indices = doc['accessors'][primitive['indices']]
        fmt = {5121:'B',5123:'H',5125:'I'}
        corners = struct.unpack_from('<'+fmt[loops['componentType']]*3,
            views[loops['bufferView']],loops.get('byteOffset',0))
        struct.pack_into('<'+fmt[indices['componentType']]*3,
            views[indices['bufferView']],indices.get('byteOffset',0),*corners)
    binary = bytearray()
    for v, raw in zip(doc['bufferViews'],views):
        binary += b'\0' * (-len(binary)%4)
        v.update(buffer=0,byteOffset=len(binary),byteLength=len(raw))
        binary += raw
    binary += b'\0' * (-len(binary)%4)
    doc['buffers'] = [dict(byteLength=len(binary))]
    for key in ('extensionsUsed','extensionsRequired'):
        doc[key] = [e for e in doc.get(key,[]) if e != 'EXT_meshopt_compression']
    raw = json.dumps(doc,separators=(',',':')).encode()
    raw += b' ' * (-len(raw)%4)
    return b'glTF'+struct.pack('<III',2,28+len(raw)+len(binary),len(raw))+b'JSON'+raw+struct.pack('<I4s',len(binary),b'BIN\0')+binary


@requires_bin()
class PublishedVectorFidelityTests(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        self.root = pathlib.Path(self.tmp.name)

    def convert(self, source, name, flags=(), success=True):
        output = self.root/(name+'.3tz')
        command = [BIN, '--json', 'vector', '-i', str(source), '-o', str(output),
            '--reproducible', '--jobs', '2', '--listFields', 'json', *flags]
        run = subprocess.run(command, capture_output=True, text=True)
        self.assertEqual(run.returncode == 0, success, run.stdout+run.stderr)
        if not success:
            self.assertFalse(output.exists())
        return output

    def write(self, features):
        source = self.root/'source.geojson'
        source.write_text(json.dumps(dict(type='FeatureCollection', features=features)))
        return source

    def test_raw_packed_forest_topology_metadata_placement_reuse_and_controls(self):
        features, expected = fixture()
        source = self.write(features)
        for label, encoding in [('raw',[]), ('packed',['--quantize','--meshopt'])]:
            with self.subTest(encoding=label):
                flags = ['--sourceCrs','local','--maxFeatures','1','--lodLevels','3', *encoding]
                output = self.convert(source, label, flags)
                actual, report = decode(output)
                tolerance = report['oracleMaximumRoundingMetres'] + report['encoding']['maximumQuantizationErrorMetres'] + 1e-5
                check_source(actual, expected, tolerance)
                wrong_type = copy.deepcopy(actual)
                wrong_type['1'][0][1]['flag'] = 1
                with self.assertRaisesRegex(AssertionError,'metadata type'):
                    check_source(wrong_type,expected,tolerance)
                with zipfile.ZipFile(output) as z:
                    self.assertGreater(len([n for n in z.namelist() if n.endswith('.subtree')]), 1)
                reused = self.convert(source, label+'-reuse', [*flags,'--reuseTileset',str(output)])
                reused_actual, reused_report = decode(reused)
                self.assertEqual(reused_actual, actual)
                self.assertEqual(reused_report['reuse']['rebuiltContents'], 0)
                self.assertGreater(reused_report['reuse']['reusedContents'], 0)
                edited_features = copy.deepcopy(features)
                edited_features[0]['properties']['name'] = 'edited'
                self.write(edited_features)
                edited_expected = copy.deepcopy(expected)
                edited_expected['0']['properties']['name'] = 'edited'
                fresh = self.convert(source,label+'-edit-fresh',flags)
                edit_reuse = self.convert(source,label+'-edit-reuse', [*flags,'--reuseTileset',str(output)])
                edited, _ = decode(fresh)
                check_source(edited, edited_expected, tolerance)
                self.assertEqual(decode(edit_reuse)[0], edited)
                self.write(features)
                for fault in ('frame','bounds'):
                    mutant = self.root/(label+'-'+fault+'.3tz')
                    with zipfile.ZipFile(output) as zin, zipfile.ZipFile(mutant,'w') as zout:
                        for name in zin.namelist():
                            data = zin.read(name)
                            if name == 'tileset.json':
                                doc = json.loads(data)
                                if fault == 'frame':
                                    doc['root']['transform'][12] += 1
                                else:
                                    doc['root']['boundingVolume']['box'][3:6] = [0,0,0]
                                data = json.dumps(doc).encode()
                            zout.writestr(name,data)
                    with self.assertRaises(AssertionError):
                        moved,_ = decode(mutant)
                        check_source(moved,expected,tolerance)
                # Mutants edit a full-detail published member, with raw views
                # re-encoded independently so codec corruption cannot mask semantics.
                with zipfile.ZipFile(output) as z:
                    leaf_members = {u for n,_ in forest(z) if not n['children'] for u in n['uris']}
                for mutation, message in [('missing','missing/extra'),('id','missing/extra'),('hole','hole-fill')]:
                    mutant = self.root/(label+'-'+mutation+'.3tz')
                    with zipfile.ZipFile(output) as zin, zipfile.ZipFile(mutant, 'w') as zout:
                        changed = False
                        for name in zin.namelist():
                            data = zin.read(name)
                            if name in leaf_members:
                                from vector_fidelity_oracle import primitives
                                groups = list(primitives(data))
                                target = any(p['_source_id'] == ('0' if mutation == 'missing' else '2') for _,p,_ in groups)
                                if target and not changed:
                                    data = mutate_payload(data, mutation)
                                    changed = True
                            zout.writestr(name,data)
                        self.assertTrue(changed)
                    with self.assertRaisesRegex(AssertionError, message):
                        mutant_actual, _ = decode(mutant)
                        check_source(mutant_actual, expected, tolerance)

    def test_geographic_and_utm_asymmetric_height_placement(self):
        # Frozen independent pyproj 3.8.0 / PROJ 9.8.1 forward references
        # (always_xy=True, EPSG:4326 -> 32632), generated outside the product.
        # Both fixtures represent the same asymmetric lon/lat/heights.
        for crs, coordinates, world in [
            ('EPSG:4326', [[9,1,17],[9.001,1.002,-3]], [ecef(9,1,24),ecef(9.001,1.002,4)]),
            ('EPSG:32632', [[500000,110530.15880237566,17],[500111.25806131255,110751.21958904967,-3]], [ecef(9,1,24),ecef(9.001,1.002,4)])]:
            features = [feature(i,'Point',p) for i,p in enumerate(coordinates)]
            source = self.write(features)
            for label, enc in [('raw',[]),('packed',['--quantize','--meshopt'])]:
                flags = ['--sourceCrs',crs,'--heightOffset','7','--maxFeatures','1','--lodLevels','3',*enc]
                out = self.convert(source, crs.replace(':','')+label, flags)
                actual, report = decode(out)
                expected = {str(f['id']):dict(mode=0,properties=expected_properties(f),parts=[(p,)]) for f,p in zip(features,world)}
                check_source(actual, expected, report['oracleMaximumRoundingMetres']+report['encoding']['maximumQuantizationErrorMetres']+1e-5)
                reuse = self.convert(source, crs.replace(':','')+label+'reuse', [*flags,'--reuseTileset',str(out)])
                self.assertEqual(decode(reuse)[0], actual)

    def test_repair_and_rejection_are_visible_in_published_leaves(self):
        bow = feature(0,'Polygon',[[[0,0,5],[2,2,5],[2,0,5],[0,2,5],[0,0,5]]])
        invalid = feature(1,'LineString',[[5,5,5]])
        source = self.write([bow,invalid,feature(2,'Point',[10,20,30])])
        self.convert(source,'strict',['--sourceCrs','local'],success=False)
        skipped = self.convert(source,'skip',['--sourceCrs','local','--skipInvalid'])
        skipped_actual, skipped_report = decode(skipped)
        self.assertEqual(set(skipped_actual), {'2'})
        self.assertEqual(skipped_report['skippedFeatures'], 2)
        out = self.convert(source,'repair',['--sourceCrs','local','--repair','--skipInvalid','--quantize','--meshopt'])
        actual, report = decode(out)
        self.assertEqual(set(actual), {'0','2'})
        expected = {
            '0': dict(mode=4, properties=expected_properties(bow), triangles=[
                [(0,0,5),(0,2,5),(1,1,5)], [(2,0,5),(2,2,5),(1,1,5)]]),
            '2': dict(mode=0, properties=expected_properties(feature(2,'Point',[10,20,30])), parts=[([10,20,30],)])}
        tolerance = report['oracleMaximumRoundingMetres']+report['encoding']['maximumQuantizationErrorMetres']+1e-5
        check_source(actual,expected,tolerance)
        collapsed = copy.deepcopy(actual)
        collapsed['0'] = [(mode,row,(positions[0],)*3) for mode,row,positions in collapsed['0']]
        with self.assertRaisesRegex(AssertionError,'vertex bijection'):
            check_source(collapsed,expected,tolerance)
        moved = copy.deepcopy(actual)
        mode,row,_ = moved['2'][0]
        moved['2'][0] = mode,row,[(999,999,999)]
        with self.assertRaisesRegex(AssertionError,'source coordinate'):
            check_source(moved,expected,tolerance)
        self.assertEqual(report['features'],2)
        self.assertEqual(report['skippedFeatures'],1)


if __name__ == '__main__':
    unittest.main()
