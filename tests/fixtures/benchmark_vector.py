"""Warm release CLI comparison against the previous Python vector converter.

Invented fixtures, untimed generation/decoded world geometry checks. Linux wait4
CPU includes waited children; RSS records the largest process, not their sum.
"""
import argparse
import datetime
import hashlib
import importlib.util
import json
import math
import os
import pathlib
import platform
import statistics
import sqlite3
import subprocess
import sys
import zipfile

import numpy as np
from osgeo import gdal

ROOT = pathlib.Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / 'tests'))
from test_vector_reuse import details
from test_vector_gpkg import gpkg
spec = importlib.util.spec_from_file_location('timing', ROOT / 'tests/fixtures/benchmark_terrain.py')
timing = importlib.util.module_from_spec(spec)
spec.loader.exec_module(timing)
METHODS = ['python-1', 'rust-1', 'python-4', 'rust-4']
CASES = [('small-lines', 24, 128), ('regional-lines', 2048, 128),
         ('polygons-with-holes', 512, 64), ('projected-lines', 1024, 64), ('dense-points', 8192, 1)]


def fixture(root, name, count, vertices):
    if name == 'projected-lines':
        source = root / (name + '.gpkg')
        features = [(i + 1, dict(type='LineString', coordinates=[
            [500000 + (i % 32) * 200 + j, 4600000 + (i // 32) * 200 + math.sin(j * .1), 100.]
            for j in range(vertices)]), 2**60 + 3) for i in range(count)]
        gpkg(source, [('roads', 32632, features)])
        return source, ['--heightOffset', '0']
    features = []
    for i in range(count):
        x, y = (i % 64) * 100., (i // 64) * 100.
        if name == 'dense-points':
            geometry = dict(type='Point', coordinates=[float(i%128), float(i//128), 0.])
        elif name == 'polygons-with-holes':
            def ring(radius, n):
                p = [[x + radius * math.cos(j * 2 * math.pi / n),
                      y + radius * math.sin(j * 2 * math.pi / n), 0.] for j in range(n)]
                return p + [p[0]]
            geometry = dict(type='Polygon', coordinates=[ring(30, vertices), ring(10, vertices // 4)])
        else:
            geometry = dict(type='LineString', coordinates=[
                [x + j * .5, y + math.sin(j * .1) * .1, math.cos(j * .1) * .02]
                for j in range(vertices)])
        features.append(dict(type='Feature', id=i, properties=dict(name=f'feature-{i}',
            number=i, optional=None if i % 7 == 0 else 2**60 + 3), geometry=geometry))
    source = root / (name + '.geojson')
    source.write_text(json.dumps(dict(type='FeatureCollection', features=features)))
    return source, ['--sourceCrs', 'local']


def decoded(path, directory):
    directory.mkdir()
    with zipfile.ZipFile(path) as archive:
        archive.extractall(directory)
    report = json.loads((directory / 'conversion.json').read_text())
    return details(directory), report


def compare(folder, methods=METHODS):
    decoded_outputs = {}
    stats = {}
    for method in methods:
        geometry, report = decoded(folder / (method + '.3tz'), folder / method)
        decoded_outputs[method] = geometry
        stats[method] = dict(tiles=report['tiles'], archiveBytes=(folder / (method + '.3tz')).stat().st_size,
            encoding=report['encoding'], performance=report.get('performance'),
            pointAggregation=report.get('pointAggregation'))
        with zipfile.ZipFile(folder/(method+'.3tz')) as archive:
            root=json.loads(archive.read('tileset.json'))['root']
            stats[method]['rootVertices']=root.get('extras',{}).get('vertices',0)
            stats[method]['rootPointAggregation']=root.get('extras',{}).get('pointAggregation')
    baseline = decoded_outputs[METHODS[0]]
    maximum = 0.
    for method, geometry in decoded_outputs.items():
        assert geometry.keys() == baseline.keys(), method
        for key, before in baseline.items():
            after = geometry[key]
            assert len(before) == len(after), (method, key)
            for (properties, a), (other, b) in zip(before, after):
                assert properties == other, (method, key)
                assert a.shape == b.shape, (method, key)
                # Polygon loops may reverse as the fitted plane changes sign.
                if key[2] == 4:
                    a = a[np.lexsort(a.T[::-1])]
                    b = b[np.lexsort(b.T[::-1])]
                error = float(np.linalg.norm(a - b, axis=1).max())
                maximum = max(maximum, error)
                assert error <= .002, (method, key, error)
    # Native worker counts must produce the same manifest, reports and payloads
    # apart from deliberately nondeterministic timing/worker diagnostics.
    with zipfile.ZipFile(folder / 'rust-1.3tz') as a, zipfile.ZipFile(folder / 'rust-4.3tz') as b:
        assert a.namelist() == b.namelist()
        for name in a.namelist():
            if name not in ('conversion.json', '@3dtilesIndex1@'):
                assert a.read(name) == b.read(name), name
    if 'rust-aggregate-4' in methods:
        summary=stats['rust-aggregate-4']['rootPointAggregation']
        points=sum(len(xyz) for fragments in baseline.values() for _,xyz in fragments)
        assert summary['sourcePointCount']==points
        assert summary['aggregateCount']==stats['rust-aggregate-4']['rootVertices']<=64
        assert summary['aggregateCount']<points
    return dict(fullDetailWorldGeometryAndPropertiesMatch=True, nativeWorkerContentIdentical=True,
                maxWorldPositionDifferenceMetres=maximum, outputs=stats)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--old-bin', type=pathlib.Path, required=True)
    parser.add_argument('--new-bin', type=pathlib.Path, required=True)
    parser.add_argument('--old-ref', default='fada1d1')
    parser.add_argument('--output', type=pathlib.Path, required=True)
    parser.add_argument('--repeats', type=int, default=3)
    parser.add_argument('--case', action='append')
    args = parser.parse_args()
    args.output.mkdir(parents=True, exist_ok=False)
    env = dict(os.environ)
    result = dict(date=datetime.datetime.now(datetime.timezone.utc).isoformat(), oldRef=args.old_ref,
        platform=platform.platform(), cpu=next(line.split(':',1)[1].strip() for line in pathlib.Path('/proc/cpuinfo').read_text().splitlines() if line.startswith('model name')),
        gdal=gdal.VersionInfo('--version'), python=platform.python_version(), numpy=np.__version__, sqlite=sqlite3.sqlite_version,
        nativeReadiness=json.loads(subprocess.check_output([str(args.new_bin.resolve()),'doctor','--command','vector','--json'],text=True))['commands']['vector'],
        nativeSourceSha256={str(p.relative_to(ROOT)):hashlib.sha256(p.read_bytes()).hexdigest() for p in [ROOT/'Cargo.toml',ROOT/'Cargo.lock',ROOT/'build.rs',ROOT/'src/vector.rs',ROOT/'src/vector/native.rs',*sorted((ROOT/'src/vector/native').glob('*.rs')),ROOT/'src/point_sampling.rs',ROOT/'src/vector_encoding.rs',ROOT/'src/geospatial.rs',ROOT/'src/georef.rs',ROOT/'src/bbox.rs',ROOT/'src/glb_write.rs',ROOT/'src/pack.rs']},
        oldBinarySha256=hashlib.sha256(args.old_bin.read_bytes()).hexdigest(),
        newBinarySha256=hashlib.sha256(args.new_bin.read_bytes()).hexdigest(),
        build='cargo build --release --locked --features native-geospatial',
        memory='wait4 peak RSS is the largest process; Python worker memory is not summed',
        limitations='invented fixtures, warm cache, uncompressed content; shared workstation with unrelated background jobs',
        methods=METHODS, repeats=args.repeats, sampling='one warmup; rotated serial runs; warm filesystem cache', cases=[])
    for name, count, vertices in CASES:
        if args.case and name not in args.case:
            continue
        folder = args.output / name
        folder.mkdir()
        source, flags = fixture(folder, name, count, vertices)
        methods=METHODS+['rust-aggregate-4'] if name=='dense-points' else METHODS
        commands = {method: [str((args.old_bin if method.startswith('python') else args.new_bin).resolve()),
            'vector', '-i', str(source.resolve()), '-o', str((folder / (method + '.3tz')).resolve()),
            '--jobs', method[-1], '--maxFeatures', '64', '--lodTolerance', '5' if name=='dense-points' else '.2', '--lodLevels', '3',
            '--force', *flags, *(['--maxParentFeatures','64'] if name=='dense-points' else []),
            *(['--aggregatePoints'] if method=='rust-aggregate-4' else [])] for method in methods}
        samples = {method: [] for method in methods}
        for method in methods:
            timing.measured(commands[method], folder / (method + '-warmup.log'), dict(env, PATH='') if method.startswith('rust') else env)
        for repeat in range(args.repeats):
            for method in methods[repeat % len(methods):] + methods[:repeat % len(methods)]:
                samples[method].append(timing.measured(commands[method], folder / f'{method}-{repeat}.log', dict(env, PATH='') if method.startswith('rust') else env))
        checks = compare(folder,methods)
        medians = {method: {key: statistics.median(s[key] for s in samples[method])
                    for key in ('wallSeconds', 'cpuSeconds', 'peakRssMiB')} for method in methods}
        result['cases'].append(dict(name=name, features=count, sourceVertices=count*(vertices+vertices//4+2 if name=='polygons-with-holes' else vertices),
            commands=commands, samples=samples, medians=medians, checks=checks))
        (args.output / 'results.json').write_text(json.dumps(result, indent=2))
        print(name, json.dumps(medians), flush=True)


if __name__ == '__main__':
    main()
