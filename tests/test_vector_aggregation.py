"""Independent count, distance, identity and reuse checks for opt-in point LOD."""
import json
import os
import pathlib
import struct
import tempfile
import types
import unittest

import numpy as np
from test_vector_lod import accessors
from test_vector_reuse import archive, details, payloads, report
from test_vector_gpkg import gpkg
from vector_test_support import native_run


def nodes(node, parent=None):
    world = (np.eye(4) if parent is None else parent) @ np.array(
        node.get('transform', np.eye(4).T.flatten())).reshape(4, 4).T
    yield node, world
    for child in node.get('children', []):
        yield from nodes(child, world)


def content(path, world):
    doc, decode = accessors(path)
    data = path.read_bytes(); length = struct.unpack_from('<I', data, 12)[0]
    binary = data[28+length:]
    meta = doc['extensions']['EXT_structural_metadata']; table = meta['propertyTables'][0]
    schema = meta['schema']['classes'][table['class']]['properties']
    properties = [{} for _ in range(table['count'])]
    def view(index, dtype):
        v = doc['bufferViews'][index]
        return np.frombuffer(binary[v.get('byteOffset', 0):v.get('byteOffset', 0)+v['byteLength']], dtype)
    for name, column in table['properties'].items():
        if schema[name]['type'] == 'STRING':
            text = view(column['values'], 'u1').tobytes(); offsets = view(column['stringOffsets'], '<u4')
            values = [text[a:b].decode() for a, b in zip(offsets[:-1], offsets[1:])]
        else:
            values = view(column['values'], '<i8').tolist()
        for prop, value in zip(properties, values): prop[name] = value
    primitive = doc['meshes'][0]['primitives'][0]
    xyz = decode(primitive['attributes']['POSITION'])[:, [0, 2, 1]] * [1, -1, 1]
    xyz = (np.c_[xyz, np.ones(len(xyz))] @ world.T)[:, :3]
    return doc, properties, xyz


@unittest.skipUnless(os.environ.get('RUSTY_TILES_BIN'), 'select the native CLI')
class PointAggregationTests(unittest.TestCase):
    def args(self, source, output, **kwargs):
        return types.SimpleNamespace(input=str(source), output=str(output), source_crs='local',
            max_features=16, max_parent_features=8, lod_tolerance=5, lod_levels=3,
            max_bytes=4096, max_vertices=64, aggregate_points=True, reproducible=True, **kwargs)

    def source(self, root, count=128):
        source = root/'points.geojson'
        source.write_text(json.dumps(dict(type='FeatureCollection', features=[
            dict(type='Feature', id=i, properties=dict(name=f'point-{i}', flag=bool(i%2), large=2**60+3),
                 geometry=dict(type='MultiPoint', coordinates=[[i%16, i//16, 0], [i%16, i//16, .1]])
                 if i%7 == 0 else dict(type='Point', coordinates=[i%16, i//16, 0]))
            for i in range(count)])))
        return source

    def test_counts_bounds_and_original_leaf_properties(self):
        with tempfile.TemporaryDirectory() as tmp:
            root=pathlib.Path(tmp); source=self.source(root); output=root/'aggregated'; baseline=root/'default'
            native_run(self.args(source, output))
            args=self.args(source, baseline); args.aggregate_points=False; native_run(args)
            a, b=details(output), details(baseline)
            self.assertEqual(a.keys(), b.keys())
            for key in a:
                self.assertEqual(len(a[key]),len(b[key]))
                for (pa, xa), (pb, xb) in zip(a[key], b[key]):
                    self.assertEqual(pa, pb); np.testing.assert_array_equal(xa, xb)
            manifest=json.loads((output/'tileset.json').read_text()); tree=manifest['root']
            self.assertGreater(manifest['geometricError'],tree['geometricError'])
            checked=0
            for node, world in nodes(tree):
                self.assertLessEqual(node['extras'].get('encodedBytes',0),4096)
                self.assertLessEqual(node['extras'].get('vertices',0),64)
                summary=node.get('extras', {}).get('pointAggregation')
                if not summary: continue
                checked+=1
                doc, props, xyz=content(output/node['content']['uri'], world)
                self.assertEqual(doc['extensions']['EXT_structural_metadata']['propertyTables'][0]['class'], 'pointAggregate')
                self.assertTrue(all(set(p)=={'aggregation', 'pointCount', 'sourceLayer'} for p in props))
                self.assertEqual(sum(p['pointCount'] for p in props), summary['sourcePointCount'])
                self.assertEqual(len(props), summary['aggregateCount'])
                original=[]
                # Descendant traversal starts in the already accumulated frame.
                for child in node['children']:
                    for leaf, frame in nodes(child, world):
                        if leaf.get('children'): continue
                        data, decode=accessors(output/leaf['content']['uri'])
                        p=decode(data['meshes'][0]['primitives'][0]['attributes']['POSITION'])[:, [0,2,1]]*[1,-1,1]
                        original.extend((np.c_[p, np.ones(len(p))]@frame.T)[:, :3])
                self.assertEqual(len(original), summary['sourcePointCount'])
                error=np.linalg.norm(np.asarray(original)[:,None,:]-xyz[None,:,:], axis=2).min(axis=1).max()
                self.assertLessEqual(error, summary['maximumDistanceMetres']+node['extras']['positionRoundingMetres']+1e-6)
                self.assertLessEqual(summary['maximumDistanceMetres'], node['extras']['toleranceMetres'])
                self.assertGreater(node['geometricError'], 0)
                self.assertTrue(all(node['geometricError'] >= child['geometricError'] for child in node['children']))
                self.assertLessEqual(len(props), 8)
            self.assertGreater(checked, 0)
            self.assertEqual(report(output)['pointAggregation']['contentTiles'], checked)
            self.assertEqual(report(baseline)['pointAggregation']['contentTiles'], 0)

    def test_layers_stay_separate_and_coincident_counts_refine(self):
        with tempfile.TemporaryDirectory() as tmp:
            root=pathlib.Path(tmp); source=root/'layers.gpkg'
            gpkg(source, [(name, 3857, [(i+1, dict(type='Point', coordinates=[0,0,0]), i) for i in range(12)])
                          for name in ('first', 'second')])
            output=root/'out'; native_run(self.args(source, output, all_layers=True))
            tree=json.loads((output/'tileset.json').read_text())['root']; node, world=next(nodes(tree))
            _, props, xyz=content(output/node['content']['uri'], world)
            self.assertEqual({p['sourceLayer']:p['pointCount'] for p in props}, {'first':12, 'second':12})
            self.assertEqual(node['extras']['pointAggregation']['maximumDistanceMetres'], 0)
            self.assertGreater(node['geometricError'], 0)
            self.assertEqual(len(details(output)), 24)

    def test_tight_tolerance_and_mixed_geometries_keep_conservative_routing(self):
        with tempfile.TemporaryDirectory() as tmp:
            root=pathlib.Path(tmp); source=self.source(root, 16); output=root/'tight'
            args=self.args(source, output); args.lod_tolerance=.0001; args.max_features=4; native_run(args)
            tree=json.loads((output/'tileset.json').read_text())['root']
            self.assertTrue(tree['extras']['routing'])
            self.assertEqual(tree['extras']['routingReason'], 'pointAggregationTolerance')
            self.assertEqual(report(output)['pointAggregation']['contentTiles'], 0)
            data=json.loads(source.read_text())
            data['features'].append(dict(type='Feature',id='line',properties=dict(name='line',flag=True,large=2**60+3),
                geometry=dict(type='LineString',coordinates=[[0,0,0],[20,0,0]])))
            source.write_text(json.dumps(data)); output=root/'mixed'; native_run(self.args(source,output))
            tree=json.loads((output/'tileset.json').read_text())['root']
            self.assertNotIn('pointAggregation',tree['extras'])
            self.assertEqual(len(details(output)),17)

    def test_layer_budget_routes_without_combining_unrelated_layers(self):
        with tempfile.TemporaryDirectory() as tmp:
            root=pathlib.Path(tmp); source=root/'layers.gpkg'; output=root/'out'
            gpkg(source, [(f'layer-{i}',3857,[(1,dict(type='Point',coordinates=[0,0,0]),i)]) for i in range(12)])
            args=self.args(source,output,all_layers=True); args.max_features=4; native_run(args)
            tree=json.loads((output/'tileset.json').read_text())['root']
            self.assertTrue(tree['extras']['routing'])
            self.assertEqual(tree['extras']['routingReason'],'pointAggregationBudget')
            self.assertEqual(report(output)['pointAggregation']['contentTiles'],0)
            self.assertEqual(len(details(output)),12)

    def test_oversized_multipoint_fragments_keep_every_coordinate(self):
        with tempfile.TemporaryDirectory() as tmp:
            root=pathlib.Path(tmp); source=root/'points.geojson'; output=root/'out'
            points=[[i%20*.01,i//20*.01,0.] for i in range(3000)]
            source.write_text(json.dumps(dict(type='FeatureCollection',features=[dict(type='Feature',id='survey',
                properties=dict(name='survey',large=2**60+3),geometry=dict(type='MultiPoint',coordinates=points))])))
            native_run(self.args(source,output))
            manifest=json.loads((output/'tileset.json').read_text()); node,world=next(nodes(manifest['root']))
            _,props,_=content(output/node['content']['uri'],world)
            self.assertEqual(sum(p['pointCount'] for p in props),3000)
            original=[]
            for fragments in details(output).values():
                for prop,xyz in fragments:
                    self.assertEqual(prop['_source_id'],'"survey"'); self.assertEqual(prop['large'],2**60+3)
                    original.extend(xyz)
            self.assertEqual(len(original),3000)
            a=np.asarray(original); b=np.asarray(points)
            np.testing.assert_allclose(a[np.lexsort(a.T[::-1])],b[np.lexsort(b.T[::-1])],atol=1e-6,rtol=0)

    def test_workers_reuse_and_geometry_edits(self):
        with tempfile.TemporaryDirectory() as tmp:
            root=pathlib.Path(tmp); source=self.source(root); a=root/'a'; b=root/'b'; previous=root/'previous.3tz'
            native_run(self.args(source,a,jobs=1)); native_run(self.args(source,b,jobs=4))
            self.assertEqual(payloads(a),payloads(b))
            self.assertEqual((a/'tileset.json').read_bytes(),(b/'tileset.json').read_bytes())
            archive(a,previous); original=previous.read_bytes()
            reused=root/'reused'; native_run(self.args(source,reused,reuse_tileset=str(previous)))
            self.assertEqual(payloads(a),payloads(reused)); self.assertEqual(report(reused)['reuse']['rebuiltContents'],0)
            args=self.args(source,root/'bad',reuse_tileset=str(previous)); args.aggregate_points=False
            with self.assertRaisesRegex(ValueError,'settings differ'): native_run(args)
            data=json.loads(source.read_text()); data['features'][1]['geometry']['coordinates'][2]=.25
            source.write_text(json.dumps(data)); updated=root/'updated'; fresh=root/'fresh'
            native_run(self.args(source,updated,reuse_tileset=str(previous))); native_run(self.args(source,fresh))
            self.assertEqual(previous.read_bytes(),original)
            self.assertGreater(report(updated)['reuse']['reusedContents'],0)
            updated_details,fresh_details=details(updated),details(fresh)
            self.assertEqual(updated_details.keys(),fresh_details.keys())
            for key in updated_details:
                self.assertEqual(len(updated_details[key]),len(fresh_details[key]))
                for (pa,xa),(pb,xb) in zip(updated_details[key],fresh_details[key]):
                    self.assertEqual(pa,pb); np.testing.assert_allclose(xa,xb,atol=1e-6,rtol=0)
