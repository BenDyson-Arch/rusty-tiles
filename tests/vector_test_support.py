"""Vector CLI helpers for the Python acceptance tests.

Acceptance always invokes the selected CLI. Tests that compare with
the frozen Python oracle import it from tests/fixtures/vector_oracle directly.
"""
import os
import functools
import json
import pathlib
import subprocess
import sys
import tempfile
import types
import unittest
import zipfile

from cli_bin import BIN

@functools.cache
def portable_vectors():
    """Read the actual selected build's capabilities instead of trusting an env flag."""
    if not BIN:
        raise unittest.SkipTest('RUSTY_TILES_BIN is not set')
    result = subprocess.run([BIN, 'doctor', '--command', 'vector', '--json'],
                            capture_output=True, text=True, check=True)
    return json.loads(result.stdout)['commands']['vector'].get('backend') == 'portable Rust/SQLite'

def native_run(args):
    if not BIN:
        raise unittest.SkipTest('RUSTY_TILES_BIN is not set; set it to the rusty-tiles binary for native CLI acceptance')
    output = pathlib.Path(args.output)
    output.parent.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory(dir=output.parent) as scratch:
        archive = pathlib.Path(scratch) / 'output.3tz'
        command = [BIN, 'vector', '-i', str(args.input), '-o', str(archive)]
        single = dict(jobs='jobs', max_features='maxFeatures', max_parent_features='maxParentFeatures',
            max_vertices='maxVertices', max_bytes='maxBytes', max_tiles='maxTiles',
            max_source_vertices='maxSourceVertices', lod_tolerance='lodTolerance', lod_levels='lodLevels',
            source_crs='sourceCrs', height_offset='heightOffset', where='where', list_fields='listFields',
            reuse_tileset='reuseTileset')
        for key, flag in single.items():
            value = getattr(args, key, None)
            if value is not None:
                command.extend(['--' + flag, str(value)])
        for key, flag in dict(layers='layer', fields='fields', drop_fields='dropFields').items():
            for value in getattr(args, key, []) or []:
                command.extend(['--' + flag, value])
        for key, flag in dict(all_layers='allLayers', skip_invalid='skipInvalid',
            reproducible='reproducible', quantize='quantize', parent_repair='parentRepair',
            repair='repair', ambiguous_outlines='ambiguousOutlines', aggregate_points='aggregatePoints').items():
            if getattr(args, key, False):
                command.append('--' + flag)
        if getattr(args, 'meshopt_helper', None) or getattr(args, 'meshopt', False):
            command.append('--meshopt')
        result = subprocess.run(command, capture_output=True, text=True, env=dict(os.environ, PATH=""))
        if result.stderr:
            print(result.stderr, file=sys.stderr, end='')
        if result.returncode:
            raise ValueError(result.stderr.strip())
        output.mkdir(parents=True, exist_ok=True)
        (output / 't').mkdir(exist_ok=True)
        # The published archive stays beside the directory for reuse tests.
        published(output).write_bytes(archive.read_bytes())
        with zipfile.ZipFile(archive) as tiles:
            if getattr(args, 'hierarchy_view', True):
                hierarchy_view(tiles, output)
                return
            for name in tiles.namelist():
                if name != '@3dtilesIndex1@':
                    tiles.extract(name, output)


def published(output):
    """The archive `native_run` published for the directory `output`."""
    output = pathlib.Path(output)
    return output.with_name(output.name + '.published.3tz')


def hierarchy_view(tiles, output):
    """Materialize the producer hierarchy H of a published forest archive.

    Explicit vector output retired in 0.4.0 (H6). Hierarchy audits read H as the
    producer recorded it: `tileset.json` is the published manifest with its root
    replaced by `explicitRoot` from `vector-build.json`, and every raw payload is
    written to its content address `t/<sha256>.glb`, published in place or
    restored byte for byte (sha256-checked) from its rebased forest member by the
    independent acceptance oracle. Build state and reports are copied unchanged;
    forest-only members (implicit content, subtrees, external documents) are not.
    Tests of the published layout pass hierarchy_view=False."""
    from vector_acceptance_oracle import raw_payloads
    state = json.loads(tiles.read('vector-build.json'))
    manifest = json.loads(tiles.read('tileset.json'))
    view = {key: manifest[key] for key in ('asset', 'extensionsUsed', 'geometricError') if key in manifest}
    view['root'] = state['explicitRoot']
    (output / 'tileset.json').write_text(json.dumps(view))
    for name, data in raw_payloads(tiles, state).items():
        (output / name).write_bytes(data)
    for name in ('vector-build.json', 'conversion.json', 'geometry-reports.jsonl'):
        if name in tiles.namelist():
            tiles.extract(name, output)


# Tests call vector.run and vector.emit; both use the native CLI.
vector = types.SimpleNamespace(run=native_run)

# Actual native CLI encoding fixtures. Oracle emit/math is never substituted
# for native code when the acceptance binary is selected.
_native_frames = {}

def native_emit(items, path, transform, encoding_report=None, reports=None, **kwargs):
    import json
    import shutil
    import numpy as np
    if kwargs.pop('fill_only', False):
        raise AssertionError('filled fragments must be exercised through CLI budgets')
    path = pathlib.Path(path)
    with tempfile.TemporaryDirectory() as scratch:
        root = pathlib.Path(scratch)
        source = root/'survey.geojson'
        features = [dict(type='Feature', id=i, properties={k:v for k,v in item['properties'].items() if k not in ('_source_id','_source_layer')}, geometry=item['geometry'])
                    for i,item in enumerate(items)]
        source.write_text(json.dumps(dict(type='FeatureCollection',features=features)))
        out = root/'out'
        native_run(types.SimpleNamespace(input=str(source),output=str(out),source_crs='local',
            max_features=max(1,len(items)),max_parent_features=max(1,len(items)),
            max_vertices=1000000,max_bytes=128000000,lod_levels=1,**kwargs))
        manifest = json.loads((out/'tileset.json').read_text())
        leaves=[]
        def walk(node, parent):
            frame=parent@np.array(node.get('transform',np.eye(4).T.flatten())).reshape(4,4).T
            if not node.get('children'):
                for content in node.get('contents',[node['content']] if 'content' in node else []):
                    leaves.append((node,content,frame))
            for child in node.get('children',[]):walk(child,frame)
        walk(manifest['root'],np.eye(4))
        if len(leaves)!=1:raise AssertionError(f'encoding fixture needs one leaf, got {len(leaves)}')
        node,content,frame=leaves[0]
        shutil.copyfile(out/content['uri'],path)
        _native_frames[str(path)] = frame
        if encoding_report is not None:
            extras=node['extras']
            encoding_report.update(primitives=extras['primitives'],vertices=extras['vertices'],
                rounding=extras['positionRoundingMetres'],quantizationError=extras['quantizationErrorMetres'])
        if reports is not None:
            reports.extend(json.loads(line) for line in (out/'geometry-reports.jsonl').read_text().splitlines())

def to_source(path, positions, document=None):
    """Decode the native glTF node, axis conversion and tileset frame independently."""
    import numpy as np
    frame=_native_frames[str(path)]
    positions=np.asarray(positions,dtype=float)
    if document is not None and document['accessors'][document['meshes'][0]['primitives'][0]['attributes']['POSITION']].get('normalized'):
        node=document['nodes'][0]
        positions=positions/65535*np.array(node['scale'])+node['translation']
    positions=positions[:,[0,2,1]]*[1,-1,1]
    return (np.c_[positions,np.ones(len(positions))]@frame.T)[:,:3]

vector.emit = native_emit
