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


def _value_end(text, i):
    """End (exclusive) of the JSON value starting at text[i]."""
    if text[i:i + 1] in (b'{', b'[', b'"'):
        depth, inside = 0, False
        while True:
            ch = text[i:i + 1]
            if inside:
                if ch == b'\\':
                    i += 1
                elif ch == b'"':
                    inside = False
                    if depth == 0:
                        return i + 1
            elif ch == b'"':
                inside = True
            elif ch in (b'{', b'['):
                depth += 1
            elif ch in (b'}', b']'):
                depth -= 1
                if depth == 0:
                    return i + 1
            i += 1
    while text[i:i + 1] not in (b',', b'}', b']', b' ', b''):
        i += 1
    return i


def _root_node_span(text):
    """Byte span of the single scene-root node object inside the top-level `nodes` array."""
    document = json.loads(text)
    roots = {index for scene in document['scenes'] for index in scene['nodes']}
    assert len(roots) == 1, 'payload has several scene roots'
    (root,) = roots
    ws = b' \t\r\n'
    i = 1
    while True:
        while text[i] in ws: i += 1
        end = _value_end(text, i)
        key = json.loads(text[i:end])
        i = end
        while text[i] in ws: i += 1
        assert text[i:i + 1] == b':'
        i += 1
        while text[i] in ws: i += 1
        end = _value_end(text, i)
        if key == 'nodes':
            j, n = i + 1, 0
            while True:
                while text[j] in ws: j += 1
                element = _value_end(text, j)
                if n == root:
                    return j, element
                j, n = element, n + 1
                while text[j] in ws + b',': j += 1
        i = end
        while text[i] in ws + b',': i += 1


def restore(published, entry):
    """Independent inverse of the forest rebase (design 4.7): splice the recorded raw
    root-node text back, re-pad the JSON chunk to its recorded length, fix lengths.
    A zero translation publishes a byte copy and records no node text or length."""
    if entry['delta'] == [0, 0, 0] and entry['rootNode'] == '' and entry['jsonLength'] == 0:
        return published
    prefix = b''
    if published[:4] == b'b3dm':
        offset = 28 + sum(struct.unpack_from('<4I', published, 12))
        prefix, published = published[:offset], published[offset:]
    assert published[:4] == b'glTF'
    length = struct.unpack_from('<I', published, 12)[0]
    text, rest = published[20:20 + length].rstrip(b' '), published[20 + length:]
    a, b = _root_node_span(text)
    text = text[:a] + entry['rootNode'].encode() + text[b:]
    assert len(text) <= entry['jsonLength'], 'recorded JSON length is shorter than the restored text'
    text += b' ' * (entry['jsonLength'] - len(text))
    glb = b'glTF' + struct.pack('<III', 2, 20 + len(text) + len(rest), len(text)) + b'JSON' + text + rest
    if prefix:
        prefix = prefix[:8] + struct.pack('<I', len(prefix) + len(glb)) + prefix[12:]
    return prefix + glb


def glb_primitives(data):
    """Mesh primitives of one payload (b3dm-wrapped or plain GLB)."""
    if data[:4] == b'b3dm':
        tables = struct.unpack_from('<4I', data, 12)
        data = data[28 + sum(tables):]
    size = struct.unpack_from('<I', data, 12)[0]
    doc = json.loads(data[20:20 + size])
    return sum(len(mesh['primitives']) for mesh in doc.get('meshes', []))


def forest_statistics(archive):
    """Tile statistics of the producer hierarchy H, recomputed from published members.

    Explicit tiles are read from every tileset document; a link tile (whose contents are only
    tileset-d{m}.json) is not a tile of H, its document root is. An implicit tile's contents are
    the implicit-content/r{n}-{level}-{x}-{y}[-{z}]-{slot}.{suffix} members sharing one tile key.
    Bytes are published member sizes; primitives are counted in each payload's glTF JSON."""
    import re
    names = archive.namelist()
    tiles = []
    def walk(node):
        uris = [c['uri'] for c in node.get('contents', []) + ([node['content']] if 'content' in node else [])]
        if not (uris and all(re.fullmatch(r'tileset-d[0-9]+\.json', u) for u in uris)):
            # Template URIs belong to an implicit root, whose members are grouped below.
            tiles.append([u for u in uris if '{' not in u])
        for child in node.get('children', []):
            walk(child)
    for name in sorted(names):
        if name == 'tileset.json' or re.fullmatch(r'tileset-d[0-9]+\.json', name):
            walk(json.loads(archive.read(name))['root'])
    implicit = {}
    for name in names:
        match = re.fullmatch(r'implicit-content/(r[0-9]+(?:-[0-9]+)+)-[0-9]+\.(?:glb|b3dm|gltf)', name)
        if match:
            implicit.setdefault(match.group(1), []).append(name)
    tiles = [t for t in tiles if t] + list(implicit.values())
    sizes = [sum(archive.getinfo(u).file_size for u in t) for t in tiles]
    counts = [sum(glb_primitives(archive.read(u)) for u in t) for t in tiles]
    return dict(maximumTileBytes=max(sizes, default=0), encodedTileBytes=sum(sizes),
                primitiveReferences=sum(counts), maximumTilePrimitives=max(counts, default=0))


def checked_statistics(archive, report):
    """The report's tile statistics must equal those recomputed from the published members."""
    statistics = forest_statistics(archive)
    reported = dict(maximumTileBytes=report['maximumTileBytes'],
                    **{key: report['encoding'][key] for key in
                       ('encodedTileBytes', 'primitiveReferences', 'maximumTilePrimitives')})
    assert reported == statistics, ('report statistics disagree with the published members', reported, statistics)
    return statistics


def explicit_inventory(path):
    import zipfile
    with zipfile.ZipFile(path) as archive:
        report = json.loads(archive.read('conversion.json'))
        return dict(report=report, statistics=checked_statistics(archive, report))


def forest_inventory(path):
    """Default (implicit forest) archive: each payload once, every raw content recoverable."""
    import zipfile
    with zipfile.ZipFile(path) as archive:
        names = archive.namelist()
        assert len(names) == len(set(names)), 'duplicate archive members'
        referenced = set()
        def walk(node, out):
            for content in node.get('contents', []) + ([node['content']] if 'content' in node else []):
                out.add(content['uri'])
            for child in node.get('children', []):
                walk(child, out)
        for name in names:
            if name == 'tileset.json' or (name.startswith('tileset-d') and name.endswith('.json')):
                data = archive.read(name)
                assert b'rustyTilesSourceContents' not in data, 'archive retains raw sources'
                walk(json.loads(data)['root'], referenced)
        raw = {n for n in names if n.startswith('t/')}
        assert raw <= referenced, ('unreferenced raw members', raw - referenced)
        for name in raw:
            assert pathlib.PurePosixPath(name).stem == sha(archive.read(name)), 'content-address mismatch'
        state = json.loads(archive.read('vector-build.json'))
        mapped = state['payloads']
        for key, entry in mapped.items():
            assert entry['member'].startswith('implicit-content/') and entry['member'] in names
            assert sha(restore(archive.read(entry['member']), entry)) == key, 'payload record does not restore its raw bytes'
        contents = set()
        walk(state['explicitRoot'], contents)
        only = {uri for uri in contents if uri not in raw}
        assert all(pathlib.PurePosixPath(uri).stem in mapped for uri in only), 'unrecoverable raw content'
        report = json.loads(archive.read('conversion.json'))
        assert report['reuse']['publishedContents'] == len(contents)
        statistics = checked_statistics(archive, report)
        rebased = sum(entry['delta'] != [0, 0, 0] for entry in mapped.values())
        return dict(report=report, mapped_only=len(only), mapped=len(mapped), rebased=rebased, statistics=statistics,
                    # Per-build reports and the offset index are not tile content.
                    payloads={n: sha(archive.read(n)) for n in names
                              if n not in ('conversion.json', 'geometry-reports.jsonl', '@3dtilesIndex1@')})


def run_forest(binary, evidence):
    """Reuse from a default forest archive restores rebased payloads byte for byte."""
    results = {}
    with tempfile.TemporaryDirectory(prefix='vector-forest-') as temporary:
        root = pathlib.Path(temporary)
        def features(edit):
            items = []
            for i in range(32):
                x, y = i % 8 * 40, i // 8 * 40
                ring = [[x, y, 0], [x + 12, y, 0], [x + 12, y + 12, 0], [x, y + 12, 0], [x, y, 0]]
                hole = [[x + 4, y + 4, 0], [x + 4, y + 8, 0], [x + 8, y + 8, 0], [x + 8, y + 4, 0], [x + 4, y + 4, 0]]
                name = 'edited' if edit and i == 21 else 'parcel-' + str(i)
                items.append(dict(type='Feature', id='p' + str(i), properties=dict(name=name), geometry=dict(type='Polygon', coordinates=[ring, hole])))
                items.append(dict(type='Feature', id='l' + str(i), properties=dict(name='road-' + str(i)),
                                  geometry=dict(type='LineString', coordinates=[[x + 20 + j, y + 20 + j % 3 * .4, 0] for j in range(16)])))
            return items
        def points():
            return [dict(type='Feature', id='q' + str(i), properties=dict(name='pt-' + str(i)),
                         geometry=dict(type='Point', coordinates=[i % 8 * 40 + i // 64 * 3, i // 8 % 8 * 40, 0]))
                    for i in range(256)]
        def convert(name, edit, flags=(), items=None):
            source = root / name / 'parcels.geojson'
            source.parent.mkdir()
            items = features(edit) if items is None else items
            source.write_text(json.dumps(dict(type='FeatureCollection', features=items)))
            output = source.parent / 'out.3tz'
            command = [str(binary), '--json', 'vector', '-i', str(source), '-o', str(output), '--reproducible',
                       '--sourceCrs', 'local', '--jobs', '1', '--maxFeatures', '1', '--lodLevels', '2', *flags]
            proc = subprocess.run(command, capture_output=True, text=True)
            record = dict(command=command, exit_code=proc.returncode, stdout=proc.stdout, stderr=proc.stderr)
            if proc.returncode == 0:
                record['inventory'] = (explicit_inventory if '--explicit' in flags else forest_inventory)(output)
                (evidence / ('forest-' + name + '.3tz')).write_bytes(output.read_bytes())
            else:
                assert not output.exists(), 'failed conversion published output'
            results[name] = record
            (evidence / 'forest-results.json').write_text(json.dumps(results, indent=2) + '\n')
            return record, output
        base, previous = convert('base', False)
        assert base['exit_code'] == 0, base['stderr']
        assert base['inventory']['mapped_only'] > 0, 'no payload is published only under implicit-content/'
        assert base['inventory']['rebased'] > 0, 'no payload is published with a nonzero rebase'
        # Report statistics: byte and primitive figures are recomputed from the members in
        # forest_inventory; the rest must equal the --explicit build of the same hierarchy, whose
        # report walks the manifest instead of the forest writer's tile records.
        def same_statistics(forest, explicit):
            forest, explicit = forest['inventory'], explicit['inventory']
            for key in ('primitiveReferences', 'maximumTilePrimitives', 'uncompressedTileBytes'):
                assert forest['report']['encoding'][key] == explicit['report']['encoding'][key], key
            assert forest['report']['pointAggregation'] == explicit['report']['pointAggregation']
        explicit, _ = convert('base-explicit', False, ['--explicit'])
        assert explicit['exit_code'] == 0, explicit['stderr']
        same_statistics(base, explicit)
        aggregated, _ = convert('points-aggregated', False, ['--aggregatePoints'], points())
        aggregated_explicit, _ = convert('points-aggregated-explicit', False, ['--aggregatePoints', '--explicit'], points())
        assert aggregated['exit_code'] == aggregated_explicit['exit_code'] == 0, aggregated['stderr']
        assert aggregated['inventory']['report']['pointAggregation']['contentTiles'] > 0
        same_statistics(aggregated, aggregated_explicit)
        same, _ = convert('same', False, ['--reuseTileset', str(previous)])
        assert same['exit_code'] == 0, same['stderr']
        assert same['inventory']['payloads'] == base['inventory']['payloads'], 'unchanged reuse changed published bytes'
        assert same['inventory']['report']['reuse']['rebuiltContents'] == 0
        fresh, _ = convert('edit-fresh', True)
        reused, _ = convert('edit-reuse', True, ['--reuseTileset', str(previous)])
        assert fresh['exit_code'] == reused['exit_code'] == 0
        assert reused['inventory']['payloads'] == fresh['inventory']['payloads'], 'edited reuse differs from fresh'
        assert 0 < reused['inventory']['report']['reuse']['rebuiltContents'] < reused['inventory']['report']['reuse']['reusedContents']
        import zipfile
        altered = root / 'altered.3tz'
        target = sorted(e['member'] for e in json.loads(zipfile.ZipFile(previous).read('vector-build.json'))['payloads'].values() if e['delta'] != [0, 0, 0])[0]
        with zipfile.ZipFile(previous) as source, zipfile.ZipFile(altered, 'w') as sink:
            for name in source.namelist():
                data = source.read(name)
                sink.writestr(name, data[:-1] + bytes([data[-1] ^ 1]) if name == target else data)
        broken, _ = convert('altered-reuse', False, ['--reuseTileset', str(altered)])
        assert broken['exit_code'] != 0 and 'previous content checksum failed' in broken['stdout'], broken
    return results


def point(identity, xyz, properties=None):
    return dict(type='Feature', id=identity, properties=properties or {}, geometry=dict(type='Point', coordinates=xyz))


def run(binary, evidence, native=False):
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
            source_before = source.read_bytes()
            proc = subprocess.run(command, capture_output=True, text=True)
            assert source.read_bytes() == source_before, 'conversion changed source bytes'
            record = dict(command=command, exit_code=proc.returncode, stdout=proc.stdout, stderr=proc.stderr,
                          source_sha256=sha(source.read_bytes()))
            (evidence / (name + '.geojson')).write_bytes(source.read_bytes())
            results['cases'][name] = record
            if proc.returncode == 0:
                record['inventory'] = inventory(output)
                (evidence / (name + '.3tz')).write_bytes(output.read_bytes())
            elif existing is not None:
                assert output.read_bytes() == existing, 'fatal conversion changed existing output'
                record['preserved_output_sha256'] = sha(existing)
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
        malformed = [dict(type='Unsupported', coordinates=[1,2]), dict(type='Point', coordinates=[1]),
                     dict(type='Point', coordinates=[1,2,3,4]), dict(type='Point', coordinates=['x',2]), dict(type='Point')]
        for index, geometry in enumerate(malformed):
            feature = dict(type='Feature', id='malformed', properties={'rejected_only':17}, geometry=geometry)
            strict_bad, _, _ = convert('malformed-strict-' + str(index), [feature, valid], ['--force'], b'existing-output')
            assert strict_bad['exit_code'] != 0, 'malformed geometry published in strict mode'
            skipped_bad, _, _ = convert('malformed-skip-' + str(index), [feature, valid], ['--skipInvalid'])
            assert skipped_bad['exit_code'] == 0
            observed = skipped_bad['inventory']
            assert observed['identities'] == [('source', json.dumps('valid'))]
            assert observed['report']['features'] == observed['report']['fragments'] == 1
            assert observed['report']['skippedFeatures'] == 1 and observed['report']['featuresWithoutGeometry'] == 0
            assert observed['root_transform'] == control['inventory']['root_transform']
            assert observed['payloads'] == control['inventory']['payloads']
            assert len(observed['diagnostics']) == 1 and observed['diagnostics'][0]['outcome'] == 'skipped'
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
        prior_sha = sha(previous.read_bytes())
        for name, features, without in [('delete-all-reuse', [], 0),
                                        ('all-null-reuse', [dict(type='Feature', id='null', properties={}, geometry=None)], 1)]:
            empty, _, _ = convert(name, features, ['--maxBytes', '4096', '--reuseTileset', str(previous)])
            assert empty['exit_code'] == 0
            observed = empty['inventory']
            assert observed['identities'] == [] and observed['payloads'] == {}
            assert observed['report']['features'] == observed['report']['fragments'] == 0
            assert observed['report']['featuresWithoutGeometry'] == without
            assert sha(previous.read_bytes()) == prior_sha, 'empty reuse changed prior archive'
        schema_edit = [point(i, [i * 100, 0, 0], {'payload': str(i) + 'x' * 1500, 'added_integer': 17}) for i in range(2)]
        fresh_schema, _, _ = convert('schema-fresh', schema_edit, ['--maxBytes', '4096'])
        reused_schema, _, _ = convert('schema-reuse', schema_edit, ['--maxBytes', '4096', '--reuseTileset', str(previous)])
        assert fresh_schema['exit_code'] == reused_schema['exit_code'] == 0
        assert reused_schema['inventory']['identities'] == fresh_schema['inventory']['identities']
        assert reused_schema['inventory']['payloads'] == fresh_schema['inventory']['payloads']
        assert reused_schema['inventory']['report']['reuse']['rebuiltContents'] > 0
        assert sha(previous.read_bytes()) == prior_sha
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
            results['cases']['overlap-' + alias]['preserved_source_sha256'] = sha(before)
        # Two jobs with independent staging and frames execute simultaneously.
        pending = []
        for index in range(2):
            source = root / ('concurrent-' + str(index) + '.geojson')
            source.write_text(json.dumps(dict(type='FeatureCollection', features=[point(index, [index * 10000, 0, 0])])) )
            output = root / ('concurrent-' + str(index) + '.3tz')
            command = [str(binary), '--json', 'vector', '-i', str(source), '-o', str(output), '--sourceCrs', 'local', '--explicit', '--reproducible']
            pending.append((index, output, command, subprocess.Popen(command, stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True)))
        for index, output, command, proc in pending:
            stdout, stderr = proc.communicate()
            assert proc.returncode == 0, stderr
            item = inventory(output)
            assert item['identities'] == [('concurrent-' + str(index), str(index))]
            results['cases']['concurrent-' + str(index)] = dict(command=command, exit_code=proc.returncode, inventory=item)
    results['forest_reuse'] = run_forest(binary, evidence)
    if native:
        results['native_gpkg'] = run_gpkg(binary, evidence)
    results['all_assertions_passed'] = True
    (evidence / 'results.json').write_text(json.dumps(results, indent=2) + '\n')
    return results


def run_gpkg(binary, evidence):
    """Invent ISO WKB/GeoPackage bytes directly; do not use OGR to create fixtures."""
    import sqlite3
    evidence.mkdir(parents=True, exist_ok=True)
    results = {}
    with tempfile.TemporaryDirectory(prefix='vector-gpkg-') as temporary:
        root = pathlib.Path(temporary)
        source = root/'source.gpkg'
        db = sqlite3.connect(source)
        db.executescript("""
            PRAGMA application_id=1196444487; PRAGMA user_version=10200;
            CREATE TABLE gpkg_spatial_ref_sys(srs_name TEXT NOT NULL,srs_id INTEGER PRIMARY KEY,organization TEXT NOT NULL,organization_coordsys_id INTEGER NOT NULL,definition TEXT NOT NULL,description TEXT);
            CREATE TABLE gpkg_contents(table_name TEXT PRIMARY KEY,data_type TEXT NOT NULL,identifier TEXT UNIQUE,description TEXT DEFAULT '',last_change DATETIME NOT NULL DEFAULT '2026-01-01T00:00:00.000Z',min_x DOUBLE,min_y DOUBLE,max_x DOUBLE,max_y DOUBLE,srs_id INTEGER);
            CREATE TABLE gpkg_geometry_columns(table_name TEXT NOT NULL,column_name TEXT NOT NULL,geometry_type_name TEXT NOT NULL,srs_id INTEGER NOT NULL,z TINYINT NOT NULL,m TINYINT NOT NULL,PRIMARY KEY(table_name,column_name));
            INSERT INTO gpkg_spatial_ref_sys VALUES('Undefined Cartesian',-1,'NONE',-1,'undefined','');
            INSERT INTO gpkg_spatial_ref_sys VALUES('Undefined geographic',0,'NONE',0,'undefined','');
            INSERT INTO gpkg_contents(table_name,data_type,identifier,srs_id) VALUES('features','features','features',-1);
            INSERT INTO gpkg_geometry_columns VALUES('features','geom','GEOMETRY',-1,0,0);
            CREATE TABLE features(fid INTEGER PRIMARY KEY,geom BLOB,payload TEXT);
        """)
        def gp(wkb):
            return b'GP' + bytes([0,1]) + struct.pack('<i',-1) + wkb
        coords = [(1000.,1000.),(1010.,1000.),(1010.,1010.),(1000.,1010.),(1000.,1000.)]
        polygon = struct.pack('<BIII',1,3,1,len(coords)) + b''.join(struct.pack('<dd',*xy) for xy in coords)
        point_wkb = struct.pack('<BIdd',1,1,20.,20.)
        db.execute('INSERT INTO features VALUES(1,?,?)',(gp(polygon),'x'*5000))
        db.execute('INSERT INTO features VALUES(2,?,NULL)',(gp(point_wkb),))
        db.commit();db.close()
        before = source.read_bytes()
        (evidence/'native-source.gpkg').write_bytes(before)
        def convert(name, flags, existing=None):
            output = root/(name+'.3tz')
            if existing is not None:output.write_bytes(existing)
            command=[str(binary),'--json','vector','-i',str(source),'-o',str(output),'--sourceCrs','local','--explicit','--reproducible','--jobs','1','--lodLevels','1','--maxBytes','4096',*flags]
            proc=subprocess.run(command,capture_output=True,text=True)
            item=dict(command=command,exit_code=proc.returncode,stdout=proc.stdout,stderr=proc.stderr,source_sha256=sha(before))
            assert source.read_bytes()==before, 'native source changed'
            item['preserved_source_sha256']=sha(before)
            if proc.returncode==0:
                item['inventory']=inventory(output)
                (evidence/(name+'.3tz')).write_bytes(output.read_bytes())
            elif existing is not None:
                assert output.read_bytes()==existing
                item['preserved_output_sha256']=sha(existing)
            else:assert not output.exists()
            results[name]=item
            (evidence/'native-gpkg-results.json').write_text(json.dumps(results,indent=2)+'\n')
            return item,output
        accepted, previous=convert('native-gpkg-skip',['--skipInvalid'])
        assert accepted['exit_code']==0, accepted
        inv=accepted['inventory']
        assert inv['identities']==[('features','2')]
        assert inv['report']['features']==inv['report']['fragments']==1
        assert inv['report']['skippedFeatures']==1 and inv['report']['fragmentedPolygons']==0
        assert len(inv['diagnostics'])==1 and inv['diagnostics'][0]['outcome']=='skipped'
        strict,_=convert('native-gpkg-strict',['--force'],b'existing-output')
        assert strict['exit_code']!=0
        reused,_=convert('native-gpkg-reuse',['--skipInvalid','--reuseTileset',str(previous)])
        assert reused['exit_code']==0
        assert reused['inventory']['identities']==inv['identities']
        assert reused['inventory']['payloads']==inv['payloads']
        assert reused['inventory']['report']['reuse']['rebuiltContents']==0
    return results


if __name__ == '__main__':
    parser = argparse.ArgumentParser()
    parser.add_argument('--binary', type=pathlib.Path, required=True)
    parser.add_argument('--evidence', type=pathlib.Path, required=True)
    parser.add_argument('--native', action='store_true', help='also inspect the independent native GeoPackage consumer')
    args = parser.parse_args()
    run(args.binary.resolve(), args.evidence.resolve(), args.native)
