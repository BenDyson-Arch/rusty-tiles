"""Opt-in native/browser T1 acceptance; uses supplied executables and downloads nothing.

python3 tests/t1_terrain_viewer.py --binary /path/rusty-tiles \
  --cesium-dir /path/Build/Cesium --node-modules /path/node_modules \
  --json-output /path/evidence.json
Requires GDAL Python bindings, NumPy, Node/Playwright and Chromium.
"""
import argparse
import hashlib
import json
import os
import pathlib
import selectors
import shutil
import signal
import subprocess
import tempfile
import time

from t1_terrain_oracle import browser_oracle


def sha256(path):
    digest = hashlib.sha256()
    with pathlib.Path(path).open('rb') as source:
        for chunk in iter(lambda: source.read(1024 * 1024), b''):
            digest.update(chunk)
    return digest.hexdigest()


def runtime_hash(directory):
    files = []
    for path in sorted(directory.rglob('*')):
        if path.is_file():
            files.append({'path': path.relative_to(directory).as_posix(),
                          'bytes': path.stat().st_size, 'sha256': sha256(path)})
    return {'sha256': hashlib.sha256(json.dumps(files, sort_keys=True).encode()).hexdigest(),
            'files': files}


def stop(process):
    if process.poll() is None:
        os.killpg(process.pid, signal.SIGTERM)
        try:
            process.wait(timeout=5)
        except subprocess.TimeoutExpired:
            os.killpg(process.pid, signal.SIGKILL)
            process.wait(timeout=5)


def run(command, environment, timeout=120):
    process = subprocess.Popen(command, env=environment, stdout=subprocess.PIPE,
                               stderr=subprocess.PIPE, text=True, start_new_session=True)
    try:
        stdout, stderr = process.communicate(timeout=timeout)
    except subprocess.TimeoutExpired:
        stop(process)
        raise RuntimeError(f'command timed out: {command[0]}')
    if process.returncode:
        raise RuntimeError(f'command failed ({process.returncode}): {command}\n{stdout}\n{stderr}')
    return stdout, stderr


def fixtures(directory):
    import numpy as np
    from osgeo import gdal, osr
    gdal.UseExceptions()
    srs = osr.SpatialReference(); srs.ImportFromEPSG(4326)
    dem = directory / 'dem.tif'
    dataset = gdal.GetDriverByName('GTiff').Create(str(dem), 32, 32, 1, gdal.GDT_Float64)
    dataset.SetProjection(srs.ExportToWkt()); dataset.SetGeoTransform([12, .01, 0, 42, 0, -.01])
    values = np.full((32, 32), 123.5, dtype='f8'); values[12:20, 12:20] = -32768
    band = dataset.GetRasterBand(1); band.WriteArray(values); band.SetNoDataValue(-32768)
    band = None; dataset = None
    imagery = directory / 'imagery.tif'
    dataset = gdal.GetDriverByName('GTiff').Create(str(imagery), 32, 32, 3, gdal.GDT_Byte)
    dataset.SetProjection(srs.ExportToWkt()); dataset.SetGeoTransform([12, .01, 0, 42, 0, -.01])
    for band, value in enumerate((255, 100, 20), 1):
        dataset.GetRasterBand(band).Fill(value)
    dataset = None
    return dem, imagery


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--binary', type=pathlib.Path, required=True)
    parser.add_argument('--cesium-dir', type=pathlib.Path, required=True)
    parser.add_argument('--node-modules', type=pathlib.Path, required=True)
    parser.add_argument('--json-output', type=pathlib.Path, required=True)
    parser.add_argument('--validator-modules', type=pathlib.Path, help='Separate pinned Khronos validator modules, if not in node-modules')
    parser.add_argument('--timeout', type=int, default=180)
    args = parser.parse_args()
    binary = args.binary.resolve(strict=True); cesium = args.cesium_dir.resolve(strict=True)
    assert (cesium / 'Cesium.js').is_file()
    modules = args.node_modules.resolve(strict=True)
    node = shutil.which('node')
    assert node, 'Node executable missing'
    before = {'binary': {'path': str(binary), 'sha256': sha256(binary)},
              'cesium': {'path': str(cesium), **runtime_hash(cesium)},
              'browserFixtureSha256': sha256(pathlib.Path(__file__).parent / 'fixtures/terrain.cjs')}
    module_paths = [str(modules)]
    if args.validator_modules: module_paths.append(str(args.validator_modules.resolve(strict=True)))
    environment = dict(os.environ, NODE_PATH=os.pathsep.join(module_paths))
    evidence = {'inputs': before}
    with tempfile.TemporaryDirectory(prefix='rusty-tiles-t1-viewer-') as temporary:
        directory = pathlib.Path(temporary)
        dem, imagery = fixtures(directory)
        # Converter children cannot invoke helpers through PATH.
        converter_environment = dict(environment, PATH='')
        stdout, _ = run([str(binary), 'terrain', '-i', str(dem), '-o', str(directory / 'terrain'),
                        '--cells-per-leaf', '16', '--height-offset', '10.25', '--fill-height', '-999.125', '--json'],
                       converter_environment, args.timeout)
        evidence['terrainCli'] = json.loads(stdout)
        stdout, _ = run([node, str(pathlib.Path(__file__).parent / 'fixtures/t1_validate_glb.cjs'),
                         str(directory / 'terrain')], environment, args.timeout)
        evidence['khronosValidation'] = json.loads(stdout)
        stdout, _ = run([str(binary), 'raster', '-i', str(imagery), '-o', str(directory / 'imagery'),
                        '--maxZoom', '10', '--json'], converter_environment, args.timeout)
        evidence['imageryCli'] = json.loads(stdout)
        oracle_path = directory / 'oracle.json'
        oracle = browser_oracle(directory / 'terrain', oracle_path); oracle['imagery'] = True
        oracle_path.write_text(json.dumps(oracle, indent=2) + '\n')
        evidence['oracle'] = oracle
        preview_log = directory / 'preview.stderr'
        with preview_log.open('w+') as errors:
            preview = subprocess.Popen([str(binary), 'preview', '--cesium', str(cesium),
                                        '--terrain', str(directory / 'terrain'), '--imagery', str(directory / 'imagery'),
                                        '--port', '0', '--json'], env=environment, stdout=subprocess.PIPE,
                                       stderr=errors, text=True, start_new_session=True)
            try:
                with selectors.DefaultSelector() as selector:
                    selector.register(preview.stdout, selectors.EVENT_READ)
                    if not selector.select(timeout=30):
                        raise RuntimeError('preview did not publish its startup URL')
                    startup = json.loads(preview.stdout.readline())
                assert startup['ok'] is True
                evidence['preview'] = startup
                started = time.monotonic()
                stdout, stderr = run([node, str(pathlib.Path(__file__).parent / 'fixtures/terrain.cjs'),
                                      startup['url'], str(oracle_path)], environment, args.timeout)
                evidence['browser'] = json.loads(stdout)
                evidence['browserSeconds'] = time.monotonic() - started
                evidence['browserStderr'] = stderr
            finally:
                stop(preview)
        evidence['previewStderr'] = preview_log.read_text()
        evidence['outputs'] = {name: runtime_hash(directory / name) for name in ('terrain', 'imagery')}
    assert sha256(binary) == before['binary']['sha256'], 'binary changed during acceptance'
    assert runtime_hash(cesium)['sha256'] == before['cesium']['sha256'], 'Cesium runtime changed during acceptance'
    evidence['ok'] = True
    args.json_output.parent.mkdir(parents=True, exist_ok=True)
    args.json_output.write_text(json.dumps(evidence, indent=2) + '\n')
    print(json.dumps({'ok': True, 'evidence': str(args.json_output.resolve()),
                      'version': evidence['browser']['first']['version']}))


if __name__ == '__main__':
    main()
