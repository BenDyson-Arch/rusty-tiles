"""Measured Linux T1 native resource acceptance; no universal RSS guarantee.

Run with system Python providing GDAL/NumPy. Measurements come from a fresh,
stdlib-only worker using wait4 on the converter, excluding fixture-generation RSS.
"""
import argparse
import hashlib
import json
import os
import pathlib
import platform
import signal
import subprocess
import sys
import time


def digest(path):
    h = hashlib.sha256()
    with pathlib.Path(path).open('rb') as f:
        for b in iter(lambda: f.read(1 << 20), b''): h.update(b)
    return h.hexdigest()


def worker(config):
    config = json.loads(pathlib.Path(config).read_text())
    directory = pathlib.Path(config['output']).parent
    stdout_path = directory / 'child.stdout'; stderr_path = directory / 'child.stderr'
    before = set(directory.iterdir()); started = time.monotonic()
    peak_fds = 0; source_seen = False; staged = False; source_at_stage = []
    sampled_rss = 0; samples = 0
    with stdout_path.open('w') as out, stderr_path.open('w') as err:
        # Share the fresh worker's process group so the supervisor can terminate
        # both processes if the worker itself fails or stalls.
        child = subprocess.Popen(config['command'], stdout=out, stderr=err,
                                 env=dict(os.environ, PATH=''))
        while True:
            pid, status, usage = os.wait4(child.pid, os.WNOHANG)
            if pid:
                child.returncode = os.waitstatus_to_exitcode(status)
                break
            if time.monotonic() - started > 120:
                os.kill(child.pid, signal.SIGKILL)
                _, status, usage = os.wait4(child.pid, 0)
                child.returncode = os.waitstatus_to_exitcode(status)
                raise RuntimeError('converter exceeded 120-second resource probe timeout')
            try:
                links = []
                for p in pathlib.Path(f'/proc/{child.pid}/fd').iterdir():
                    try: links.append(os.readlink(p))
                    except FileNotFoundError: pass
                peak_fds = max(peak_fds, len(links)); samples += 1
                source_open = config['source'] in links
                source_seen |= source_open
                stage_now = any(p.name.startswith('.tiles-dir-') for p in directory.iterdir() if p not in before)
                if stage_now:
                    staged = True; source_at_stage.append(source_open)
                for line in pathlib.Path(f'/proc/{child.pid}/status').read_text().splitlines():
                    if line.startswith('VmRSS:'): sampled_rss = max(sampled_rss, int(line.split()[1]))
            except (FileNotFoundError, ProcessLookupError): pass
            time.sleep(.002)
    result = dict(exitCode=child.returncode, wallSeconds=time.monotonic()-started,
                  userSeconds=usage.ru_utime, systemSeconds=usage.ru_stime,
                  maximumRssKiB=usage.ru_maxrss, sampledMaximumRssKiB=sampled_rss,
                  peakFileDescriptors=peak_fds, fdSamples=samples, sourceFdObserved=source_seen,
                  stagingObserved=staged,
                  sourceFdClosedAtObservedStaging=not any(source_at_stage) if source_at_stage else None,
                  stdout=stdout_path.read_text(), stderr=stderr_path.read_text())
    print(json.dumps(result))


def fixture(path, size, oversized=False):
    from osgeo import gdal, osr
    import numpy as np
    gdal.UseExceptions()
    options = ['TILED=YES', 'BLOCKXSIZE=4096', 'BLOCKYSIZE=4096', 'COMPRESS=LZW'] if oversized else []
    ds = gdal.GetDriverByName('GTiff').Create(str(path), size, size, 1, gdal.GDT_Float64, options=options)
    srs = osr.SpatialReference(); srs.ImportFromEPSG(4326)
    ds.SetProjection(srs.ExportToWkt()); ds.SetGeoTransform([12, .1/size, 0, 42, 0, -.1/size])
    rows, columns = np.indices((size, size))
    ds.GetRasterBand(1).WriteArray(120 + rows*.001 + columns*.002)
    ds = None


def tree(path):
    return {str(p.relative_to(path)): digest(p) for p in sorted(path.rglob('*')) if p.is_file()}


def measure(binary, source, directory, cells, failure=False):
    directory.mkdir(); output = directory/'terrain'
    if failure:
        output.mkdir(); (output/'old-marker').write_text('prior published output\n')
    before = tree(output) if failure else None
    command = [str(binary), 'terrain', '-i', str(source), '-o', str(output), '--cells-per-leaf', str(cells),
               '--height-offset', '0', '--fill-height', '-20', '--json']
    if failure: command.append('--force')
    configuration = directory/'worker.json'
    configuration.write_text(json.dumps(dict(command=command, source=str(source), output=str(output))))
    process = subprocess.Popen([sys.executable, str(pathlib.Path(__file__).resolve()), '--worker', str(configuration)],
                               stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True, start_new_session=True)
    try: stdout, stderr = process.communicate(timeout=130)
    except subprocess.TimeoutExpired:
        os.killpg(process.pid, signal.SIGKILL); process.wait(); raise
    assert process.returncode == 0, stderr
    result = json.loads(stdout); result['cellsPerLeaf'] = cells; result['sourceBytes'] = source.stat().st_size
    if failure:
        assert result['exitCode'] != 0
        assert 'block' in result['stderr'].lower() or 'block' in result['stdout'].lower()
        assert tree(output) == before, 'failed replacement altered old output'
        assert not list(directory.glob('.tiles-dir-*')), 'failed conversion leaked staging'
        result['oldOutputPreserved'] = True
        result['stagingAbsentAfterExit'] = True
        result['sourceFdClosedAfterExit'] = True
    else:
        assert result['exitCode'] == 0, result['stderr']
        report = json.loads((output/'conversion.json').read_text())
        result.update(sourceValues=report['width']*report['height'], leafCount=report['tiles'],
                      vertices=report['vertices'], generatedBytes=report['generatedBytes'])
        assert result['sourceFdClosedAtObservedStaging'] is not False
    return result


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--binary', type=pathlib.Path, required=True)
    parser.add_argument('--work', type=pathlib.Path, required=True)
    parser.add_argument('--output', type=pathlib.Path, required=True)
    args = parser.parse_args(); binary = args.binary.resolve(strict=True)
    args.work.mkdir(parents=True, exist_ok=False)
    from osgeo import gdal
    binary_hash = digest(binary)
    sources = {}
    for size in (64, 256, 1024):
        sources[size] = args.work.resolve()/f'source-{size}.tif'; fixture(sources[size], size)
    runs = []
    for size, cells in ((64,64), (256,16), (256,64), (256,128), (1024,64)):
        for repetition in range(2):
            result = measure(binary, sources[size], args.work/f'run-{size}-{cells}-{repetition}', cells)
            result.update(sourceDimension=size, repetition=repetition); runs.append(result)
    oversized = args.work.resolve()/'oversized-block.tif'; fixture(oversized, 16, True)
    failure = measure(binary, oversized, args.work/'failure-oversized-block', 16, True)
    assert digest(binary) == binary_hash, 'binary changed during probe'
    evidence = dict(schemaVersion=1, binary=dict(path=str(binary), sha256=binary_hash),
                    platform=platform.platform(), python=sys.version, nativeGdal=gdal.VersionInfo('--version'),
                    measurement='Independent stdlib worker, Linux wait4 converter peak RSS, sampled /proc FD/RSS every 2ms.',
                    limitation='Observed workloads only; does not prove a universal total-RSS ceiling. Unsampled source/staging FD states remain unknown.',
                    runs=runs, failure=failure)
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(evidence, indent=2)+'\n')
    print(json.dumps(dict(ok=True, evidence=str(args.output.resolve()), runs=len(runs))))


if __name__ == '__main__':
    if len(sys.argv) == 3 and sys.argv[1] == '--worker': worker(sys.argv[2])
    else: main()
