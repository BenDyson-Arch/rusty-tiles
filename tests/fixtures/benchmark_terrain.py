"""Compare released old/new CLI terrain paths on deterministic invented DEMs.

Run after building both binaries in release mode. Timing includes process startup,
source inspection, sampling, encoding, sidecars and safe directory publication.
Linux wait4 reports CPU seconds and largest-process peak RSS (not a sum over the
process tree). Fixtures and correctness checks run outside measured intervals.
"""
import argparse
import datetime
import hashlib
import json
import os
import pathlib
import platform
import statistics
import struct
import subprocess
import sys

import numpy as np
from osgeo import gdal, osr

gdal.UseExceptions()
osr.UseExceptions()


def fixture(path, case):
    pixels, crs = case['pixels'], case['crs']
    dataset = gdal.GetDriverByName('GTiff').Create(str(path), pixels, pixels, 1, gdal.GDT_Float32,
        options=['TILED=YES', 'BLOCKXSIZE=256', 'BLOCKYSIZE=256'])
    srs = osr.SpatialReference()
    srs.ImportFromEPSG(crs)
    dataset.SetProjection(srs.ExportToWkt())
    dataset.SetGeoTransform(case['transform'])
    band = dataset.GetRasterBand(1)
    band.SetUnitType('m')
    band.SetNoDataValue(-32768)
    x = np.arange(pixels, dtype=np.float64)[None, :] / pixels
    for start in range(0, pixels, 256):
        y = np.arange(start, min(start + 256, pixels), dtype=np.float64)[:, None] / pixels
        heights = 123.5 + np.zeros((len(y), pixels))
        if case['surface'] == 'smooth':
            heights += 400 * np.exp(-((x - .5)**2 + (y - .5)**2) * 16) + 100*x + 50*y
        elif case['surface'] == 'rugged':
            heights += 250*np.sin(x*120)*np.cos(y*130) + 100*x
        heights[(np.abs(x-.5)<.05) & (np.abs(y-.5)<.05)] = -32768
        band.WriteArray(heights.astype(np.float32), 0, start)
    dataset = None


def measured(command, log, environment):
    # A lightweight worker prevents the fixture generator's NumPy/GDAL memory
    # from becoming an inherited RSS floor in the measured child at exec.
    worker = """
import json, os, sys, time
command = sys.argv[2:]
start = time.perf_counter()
with open(sys.argv[1], 'wb') as output:
    pid = os.posix_spawn(command[0], command, dict(os.environ), file_actions=[
        (os.POSIX_SPAWN_DUP2, output.fileno(), 1),
        (os.POSIX_SPAWN_DUP2, output.fileno(), 2)])
    _, status, usage = os.wait4(pid, 0)
elapsed = time.perf_counter() - start
if os.waitstatus_to_exitcode(status):
    sys.exit(os.waitstatus_to_exitcode(status))
print(json.dumps(dict(wallSeconds=elapsed, cpuSeconds=usage.ru_utime+usage.ru_stime,
    peakRssMiB=usage.ru_maxrss/1024)))
"""
    result = subprocess.run([sys.executable, '-c', worker, str(log), *command],
        env=environment, capture_output=True, text=True)
    if result.returncode:
        raise RuntimeError(f'Conversion failed; see {log}: {result.stderr}')
    return json.loads(result.stdout)


def output_stats(root):
    terrain = sorted(root.glob('*/*/*.terrain'))
    vertices = triangles = 0
    for path in terrain:
        content = path.read_bytes()
        count, = struct.unpack_from('<I', content, 88)
        vertices += count
        triangles += struct.unpack_from('<I', content, 92+count*6)[0]
    return dict(tiles=len(terrain), vertices=vertices, triangles=triangles,
        terrainBytes=sum(path.stat().st_size for path in terrain),
        sidecarBytes=sum(path.stat().st_size for path in root.glob('*/*/*.heights.json')),
        totalBytes=sum(path.stat().st_size for path in root.rglob('*') if path.is_file()),
        simplification=json.loads((root/'conversion.json').read_text()).get('simplification'))


def decoded_edges(content):
    count, = struct.unpack_from('<I', content, 88)
    codes = np.frombuffer(content, dtype='<u2', count=count*3, offset=92).astype(np.int64).reshape(3,count)
    attributes = np.cumsum((codes >> 1) ^ -(codes & 1), axis=1).T
    triangle_count, = struct.unpack_from('<I', content, 92+count*6)
    offset = 96+count*6+triangle_count*6
    edges = []
    for _ in range(4):
        size, = struct.unpack_from('<I', content, offset)
        indices = np.frombuffer(content, dtype='<u2', count=size, offset=offset+4)
        edges.append(attributes[indices])
        offset += 4+size*2
    return edges


def compare(old, full, simplified):
    manifests = [json.loads((root/'layer.json').read_text()) for root in (old, full, simplified)]
    assert manifests[0] == manifests[1] == manifests[2]
    for path in old.glob('*/*/*.terrain'):
        relative = path.relative_to(old)
        original, native = path.read_bytes(), (full/relative).read_bytes()
        # Vertex attributes, triangle indices, edge lists and height endpoints.
        assert original[88:] == native[88:]
        assert original[24:32] == native[24:32]
        reduced = (simplified/relative).read_bytes()
        for before, after in zip(decoded_edges(native), decoded_edges(reduced)):
            np.testing.assert_array_equal(before, after)
        oh, nh = (struct.unpack_from('<3d2f7d', data) for data in (original, native))
        np.testing.assert_allclose(oh[:3]+oh[5:9], nh[:3]+nh[5:9], rtol=1e-14, atol=1e-8)
        np.testing.assert_allclose(oh[9:], nh[9:], rtol=1e-12, atol=1e-12)
        sidecar = relative.with_suffix('.heights.json')
        assert json.loads((old/sidecar).read_text()) == json.loads((full/sidecar).read_text())
        assert (full/sidecar).read_bytes() == (simplified/sidecar).read_bytes()
    return dict(fullGridGeometryMatches=True, manifestsMatch=True, coverageSidecarsMatch=True, simplifiedEdgesMatch=True)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--old-bin', type=pathlib.Path, required=True)
    parser.add_argument('--new-bin', type=pathlib.Path, required=True)
    parser.add_argument('--work', type=pathlib.Path, required=True)
    parser.add_argument('--repeats', type=int, default=3)
    args = parser.parse_args()
    if args.repeats < 3:
        parser.error('use at least three measured repetitions')
    root = args.work.resolve()
    root.mkdir(parents=True, exist_ok=True)
    cases = [
        dict(name='small-flat', pixels=32, crs=4326, transform=[12,.01,0,42,0,-.01], surface='flat', maxZoom=9),
        dict(name='regional-smooth', pixels=4096, crs=4326, transform=[12,2/4096,0,42,0,-2/4096], surface='smooth', maxZoom=11),
        dict(name='projected-rugged', pixels=2048, crs=32633, transform=[500000,100,0,4650000,0,-100], surface='rugged', maxZoom=11),
    ]
    environment = dict(os.environ, PROJ_NETWORK='OFF')
    environment.pop('GDAL_CACHEMAX', None)
    results = []
    recorded_at = datetime.datetime.now(datetime.timezone.utc).isoformat()
    for case in cases:
        folder = root/case['name']
        folder.mkdir(exist_ok=True)
        source = folder/'dem.tif'
        fixture(source, case)
        methods = ['python-original', 'rust-full-grid', 'rust-simplified']
        commands = {}
        for method in methods:
            binary = args.old_bin if method == 'python-original' else args.new_bin
            command = [str(binary.resolve()), 'terrain', '-i', str(source), '-o', str(folder/method),
                '--maxZoom', str(case['maxZoom']), '--grid', '65', '--heightOffset', '10.25',
                '--fillHeight', '0', '--force']
            if method != 'python-original':
                command += ['--maxError', '0' if method == 'rust-full-grid' else '1']
            commands[method] = command
        # Warm the filesystem cache once per method. Serial rotated ordering
        # avoids concurrent conversion contention and a fixed first-run bias.
        samples = {method: [] for method in methods}
        for method in methods:
            measured(commands[method], folder/f'{method}-warmup.log', environment)
        for repeat in range(args.repeats):
            for method in methods[repeat % 3:]+methods[:repeat % 3]:
                print(f'{case["name"]}: {method}, repetition {repeat+1}', flush=True)
                samples[method].append(measured(commands[method], folder/f'{method}-{repeat}.log', environment))
        result = dict(case, grid=65, heightOffset=10.25, fillHeight=0,
            sourceBytes=source.stat().st_size, sourceSha256=hashlib.sha256(source.read_bytes()).hexdigest(), methods={})
        for method in methods:
            values = samples[method]
            result['methods'][method] = dict(samples=values,
                medianWallSeconds=statistics.median(v['wallSeconds'] for v in values),
                medianCpuSeconds=statistics.median(v['cpuSeconds'] for v in values),
                medianPeakRssMiB=statistics.median(v['peakRssMiB'] for v in values),
                output=output_stats(folder/method))
        result['comparison'] = compare(*(folder/method for method in methods))
        results.append(result)
        (root/'results.json').write_text(json.dumps(dict(environment=dict(recordedAtUtc=recorded_at, platform=platform.platform(),
            python=platform.python_version(), rustc=subprocess.check_output(['rustc', '--version'], text=True).strip(),
            build='cargo build --release --locked --features native-geospatial',
            cpu=next(line.split(':',1)[1].strip() for line in pathlib.Path('/proc/cpuinfo').read_text().splitlines() if line.startswith('model name')),
            logicalCpus=os.cpu_count(), gdal=gdal.VersionInfo('--version'), numpy=np.__version__,
            oldBinarySha256=hashlib.sha256(args.old_bin.read_bytes()).hexdigest(),
            newBinarySha256=hashlib.sha256(args.new_bin.read_bytes()).hexdigest(),
            methodology='Release CLI, serial, warm filesystem cache, one warmup then rotated measured repetitions; Linux wait4 peak RSS is largest process, not process-tree sum; original default GDAL cache versus native 64 MiB cache.'), cases=results), indent=2)+'\n')


if __name__ == '__main__':
    main()
