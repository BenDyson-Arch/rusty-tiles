"""Independent acceptance oracle: stdlib archive/GLB decoding, no frozen encoder."""
import argparse
import hashlib
import json
import pathlib
import struct
import subprocess
import tempfile


def sha(data):
    return hashlib.sha256(data).hexdigest()


def inventory(path):
    import zipfile
    with zipfile.ZipFile(path) as archive:
        names = archive.namelist()
        assert len(names) == len(set(names)), 'duplicate archive members'
        tree = json.loads(archive.read('tileset.json'))
        report = json.loads(archive.read('conversion.json'))
        diagnostics = [json.loads(x) for x in archive.read('geometry-reports.jsonl').splitlines()]
        references, leaves = set(), set()
        def walk(node):
            contents = node.get('contents', []) + ([node['content']] if 'content' in node else [])
            for content in contents:
                references.add(content['uri'])
                if not node.get('children'):
                    leaves.add(content['uri'])
            for child in node.get('children', []):
                walk(child)
        walk(tree['root'])
        members = {n for n in names if n.startswith('t/')}
        assert references == members, (references - members, members - references)
        identity, payloads = set(), {}
        maximum_bytes = maximum_vertices = 0
        for name in sorted(members):
            data = archive.read(name)
            payloads[name] = sha(data)
            assert pathlib.PurePosixPath(name).stem == sha(data), 'content-address mismatch'
            assert data[:4] == b'glTF' and struct.unpack_from('<I', data, 8)[0] == len(data)
            size, kind = struct.unpack_from('<II', data, 12)
            assert kind == 0x4E4F534A
            doc = json.loads(data[20:20 + size])
            maximum_bytes = max(maximum_bytes, len(data))
            position_accessors = {primitive['attributes']['POSITION'] for mesh in doc['meshes'] for primitive in mesh['primitives']}
            maximum_vertices = max(maximum_vertices, sum(doc['accessors'][i]['count'] for i in position_accessors))
            start = 20 + size
            count, kind = struct.unpack_from('<II', data, start)
            assert kind == 0x004E4942
            binary = data[start + 8:start + 8 + count]
            def view(index):
                v = doc['bufferViews'][index]
                offset = v.get('byteOffset', 0)
                result = binary[offset:offset + v['byteLength']]
                assert len(result) == v['byteLength']
                return result
            if name in leaves:
                for table_index, table in enumerate(doc['extensions']['EXT_structural_metadata']['propertyTables']):
                    columns = {}
                    for key in ('_source_id', '_source_layer'):
                        p = table['properties'][key]
                        assert p.get('stringOffsetType', 'UINT32') == 'UINT32'
                        offsets = struct.unpack('<' + 'I' * (table['count'] + 1), view(p['stringOffsets'])[:4 * (table['count'] + 1)])
                        values = view(p['values'])
                        assert list(offsets) == sorted(offsets) and offsets[-1] <= len(values)
                        columns[key] = [values[a:b].decode('utf8') for a, b in zip(offsets, offsets[1:])]
                    used = set()
                    for mesh in doc['meshes']:
                        for primitive in mesh['primitives']:
                            for mapping in primitive['extensions']['EXT_mesh_features']['featureIds']:
                                if mapping.get('propertyTable') != table_index:
                                    continue
                                accessor = doc['accessors'][primitive['attributes']['_FEATURE_ID_' + str(mapping['attribute'])]]
                                assert accessor['type'] == 'SCALAR' and not accessor.get('normalized', False)
                                code = {5121:'B', 5123:'H', 5125:'I', 5126:'f'}[accessor['componentType']]
                                width = struct.calcsize('<' + code)
                                stride = doc['bufferViews'][accessor['bufferView']].get('byteStride', width)
                                raw = view(accessor['bufferView'])
                                offset = accessor.get('byteOffset', 0)
                                for i in range(accessor['count']):
                                    value = struct.unpack_from('<' + code, raw, offset + i * stride)[0]
                                    assert int(value) == value and 0 <= value < table['count']
                                    used.add(int(value))
                    assert used == set(range(table['count'])), 'unrendered accepted metadata rows'
                    identity.update((columns['_source_layer'][i], columns['_source_id'][i]) for i in used)
        assert report['maximumTileBytes'] == maximum_bytes, 'published byte inventory disagrees with report'
        assert report['maximumTileVertices'] == maximum_vertices, 'published vertex inventory disagrees with report'
        assert report['reuse']['publishedContents'] == len(members), 'published content count disagrees with inventory'
        return dict(report=report, diagnostics=diagnostics, identities=sorted(identity),
                    payloads=payloads, root_transform=tree['root'].get('transform'),
                    root_extras=tree['root'].get('extras', {}))


def point(identity, xyz, properties=None):
    return dict(type='Feature', id=identity, properties=properties or {}, geometry=dict(type='Point', coordinates=xyz))


def run(binary, evidence):
    evidence.mkdir(parents=True, exist_ok=True)
    results = dict(binary_sha256=sha(binary.read_bytes()), cases={})
    with tempfile.TemporaryDirectory(prefix='vector-acceptance-') as temporary:
        root = pathlib.Path(temporary)
        def convert(name, features, flags=(), existing=None):
            source = root / name / 'source.geojson'
            source.parent.mkdir()
            source.write_text(json.dumps(dict(type='FeatureCollection', features=features)))
            output = source.parent / 'out.3tz'
            if existing is not None:
                output.write_bytes(existing)
            command = [str(binary), '--json', 'vector', '-i', str(source), '-o', str(output),
                       '--explicit', '--reproducible', '--sourceCrs', 'local', '--jobs', '1', '--lodLevels', '1', *flags]
            proc = subprocess.run(command, capture_output=True, text=True)
            record = dict(command=command, exit_code=proc.returncode, stdout=proc.stdout, stderr=proc.stderr,
                          source_sha256=sha(source.read_bytes()))
            (evidence / (name + '.geojson')).write_bytes(source.read_bytes())
            results['cases'][name] = record
            if proc.returncode == 0:
                record['inventory'] = inventory(output)
                (evidence / (name + '.3tz')).write_bytes(output.read_bytes())
            elif existing is not None:
                assert output.read_bytes() == existing, 'fatal conversion changed existing output'
            else:
                assert not output.exists(), 'failed conversion published output'
            (evidence / 'results.json').write_text(json.dumps(results, indent=2) + '\n')
            return record, source, output
        valid = point('valid', [20, 20, 0])
        oversized = dict(type='Feature', id='oversize', properties={'rejected_only': 'x' * 5000},
                         geometry=dict(type='Polygon', coordinates=[[[1000,1000,0],[1010,1000,0],[1010,1010,0],[1000,1010,0],[1000,1000,0]]]))
        control, _, _ = convert('accepted-only', [valid], ['--maxBytes', '4096'])
        rejected, _, _ = convert('rejected-first', [oversized, valid], ['--maxBytes', '4096', '--skipInvalid'])
        assert rejected['exit_code'] == 0
        actual = rejected['inventory']
        assert actual['identities'] == [('source', json.dumps('valid'))]
        assert actual['report']['features'] == actual['report']['fragments'] == 1
        assert actual['report']['skippedFeatures'] == 1
        assert actual['report']['fragmentedPolygons'] == 0
        assert actual['root_transform'] == control['inventory']['root_transform']
        assert actual['payloads'] == control['inventory']['payloads'], 'rejection changed accepted schema/frame/content'
        assert len(actual['diagnostics']) == 1 and actual['diagnostics'][0]['outcome'] == 'skipped'
        partial = dict(type='Feature', id='partial', properties={'rejected_only':17},
                       geometry=dict(type='MultiLineString', coordinates=[[[1000,1000,0],[1001,1000,0],[1002,1000,0]], [[-1e39,0,0],[0,0,0],[1e39,0,0]]]))
        rejected_partial, _, _ = convert('partial-fragment', [partial, valid], ['--maxVertices', '4', '--skipInvalid'])
        assert rejected_partial['exit_code'] == 0
        item = rejected_partial['inventory']
        assert item['identities'] == [('source', json.dumps('valid'))]
        assert item['report']['features'] == item['report']['fragments'] == 1
        assert item['report']['skippedFeatures'] == 1
        assert item['payloads'] == control['inventory']['payloads']
        assert len(item['diagnostics']) == 1 and item['diagnostics'][0]['outcome'] == 'skipped'
        strict, _, _ = convert('strict', [oversized, valid], ['--maxBytes', '4096', '--force'], b'existing-output')
        assert strict['exit_code'] != 0
        # Real encoded-byte parent rejection; accepted leaves and exact inventory remain.
        budget, _, previous = convert('encoded-budget', [point(i, [i * 100, 0, 0], {'payload': str(i) + 'x' * 1500}) for i in range(2)], ['--maxBytes', '4096'])
        assert budget['exit_code'] == 0
        assert budget['inventory']['identities'] == [('source', '0'), ('source', '1')]
        assert budget['inventory']['root_extras']['routingReason'] == 'bytes'
        reused, _, _ = convert('reuse', [point(i, [i * 100, 0, 0], {'payload': str(i) + 'x' * 1500}) for i in range(2)], ['--maxBytes', '4096', '--reuseTileset', str(previous)])
        assert reused['exit_code'] == 0
        assert reused['inventory']['payloads'] == budget['inventory']['payloads']
        assert reused['inventory']['identities'] == budget['inventory']['identities']
        assert reused['inventory']['report']['reuse']['rebuiltContents'] == 0
        # Keep the first accepted coordinate fixed so fresh and reuse frame policies agree.
        edited = [point(0, [0, 0, 0], {'payload': '0' + 'x' * 1500}),
                  point(1, [250, 10, 0], {'payload': 'changed' + 'y' * 1500})]
        fresh_edit, _, _ = convert('edited-fresh', edited, ['--maxBytes', '4096'])
        reused_edit, _, _ = convert('edited-reuse', edited, ['--maxBytes', '4096', '--reuseTileset', str(previous)])
        assert fresh_edit['exit_code'] == reused_edit['exit_code'] == 0
        assert fresh_edit['inventory']['identities'] == [('source', '0'), ('source', '1')]
        assert reused_edit['inventory']['identities'] == fresh_edit['inventory']['identities']
        assert reused_edit['inventory']['payloads'] == fresh_edit['inventory']['payloads'], 'edited reuse differs from fresh accepted content'
        assert reused_edit['inventory']['root_transform'] == fresh_edit['inventory']['root_transform']
        assert reused_edit['inventory']['report']['reuse']['rebuiltContents'] > 0
        assert budget['inventory']['payloads'] != fresh_edit['inventory']['payloads']
        # Admission must preserve the source even under forced replacement.
        for alias in ('exact', 'hardlink', 'symlink'):
            source = root / ('overlap-' + alias + '.geojson')
            source.write_text(json.dumps(dict(type='FeatureCollection', features=[valid])))
            output = source
            if alias != 'exact':
                output = root / ('overlap-' + alias + '.3tz')
                if alias == 'hardlink':
                    import os
                    os.link(source, output)
                else:
                    output.symlink_to(source)
            before = source.read_bytes()
            command = [str(binary), '--json', 'vector', '-i', str(source), '-o', str(output), '--sourceCrs', 'local', '--force', '--skipInvalid']
            proc = subprocess.run(command, capture_output=True, text=True)
            results['cases']['overlap-' + alias] = dict(command=command, exit_code=proc.returncode, stdout=proc.stdout, stderr=proc.stderr)
            assert proc.returncode != 0 and source.read_bytes() == before, 'source overlap was published'
        # Two jobs with independent staging and frames execute simultaneously.
        pending = []
        for index in range(2):
            source = root / ('concurrent-' + str(index) + '.geojson')
            source.write_text(json.dumps(dict(type='FeatureCollection', features=[point(index, [index * 10000, 0, 0])])) )
            output = root / ('concurrent-' + str(index) + '.3tz')
            command = [str(binary), '--json', 'vector', '-i', str(source), '-o', str(output), '--sourceCrs', 'local', '--explicit', '--reproducible']
            pending.append((index, output, subprocess.Popen(command, stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True)))
        for index, output, proc in pending:
            stdout, stderr = proc.communicate()
            assert proc.returncode == 0, stderr
            item = inventory(output)
            assert item['identities'] == [('concurrent-' + str(index), str(index))]
            results['cases']['concurrent-' + str(index)] = dict(exit_code=proc.returncode, inventory=item)
    (evidence / 'results.json').write_text(json.dumps(results, indent=2) + '\n')
    return results


if __name__ == '__main__':
    parser = argparse.ArgumentParser()
    parser.add_argument('--binary', type=pathlib.Path, required=True)
    parser.add_argument('--evidence', type=pathlib.Path, required=True)
    args = parser.parse_args()
    run(args.binary.resolve(), args.evidence.resolve())
