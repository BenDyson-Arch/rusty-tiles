"""Frozen Python geometry oracle; integration runs use the actual native CLI.

The oracle's math functions remain available for independent checks. Its run
function is replaced whenever RUSTY_TILES_BIN selects a native acceptance build.
"""
import importlib.util
import os
import pathlib
import subprocess
import sys
import tempfile
import zipfile

ORACLE = pathlib.Path(__file__).parent / 'fixtures/vector_oracle'
sys.path.insert(0, str(ORACLE))
spec = importlib.util.spec_from_file_location('vector_oracle', ORACLE / 'vector.py')
vector = importlib.util.module_from_spec(spec)
spec.loader.exec_module(vector)
python_run = vector.run


def native_run(args):
    output = pathlib.Path(args.output)
    output.parent.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory(dir=output.parent) as scratch:
        archive = pathlib.Path(scratch) / 'output.3tz'
        command = [os.environ['RUSTY_TILES_BIN'], 'vector', '-i', str(args.input), '-o', str(archive)]
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
        result = subprocess.run(command, capture_output=True, text=True)
        if result.stderr:
            print(result.stderr, file=sys.stderr, end='')
        if result.returncode:
            raise ValueError(result.stderr.strip())
        output.mkdir(parents=True, exist_ok=True)
        (output / 't').mkdir(exist_ok=True)
        with zipfile.ZipFile(archive) as tiles:
            for name in tiles.namelist():
                if name != '@3dtilesIndex1@':
                    tiles.extract(name, output)


if os.environ.get('RUSTY_TILES_BIN'):
    vector.run = native_run
