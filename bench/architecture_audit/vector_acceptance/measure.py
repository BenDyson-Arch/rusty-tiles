#!/usr/bin/env python3
"""Linux vector RSS and sampled scratch evidence; deliberately no boundedness claim."""
import argparse
import hashlib
import json
import pathlib
import platform
import subprocess
import tempfile
import time


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('--binary', type=pathlib.Path, required=True)
    parser.add_argument('--output', type=pathlib.Path, required=True)
    args = parser.parse_args()
    binary = args.binary.resolve()
    cases = []
    with tempfile.TemporaryDirectory(prefix='vector-resources-') as temporary:
        root = pathlib.Path(temporary)
        launcher = root / 'measure-child'
        source_c = pathlib.Path(__file__).resolve().parents[1] / 'f1a/measure_child.c'
        subprocess.run(['cc', '-O2', '-Wall', '-Wextra', '-Werror', str(source_c), '-o', str(launcher)], check=True)
        for count, vertices in [(100, 2), (1000, 2), (10000, 2), (1, 1000), (1, 10000)]:
            work = root / f'{count}-{vertices}'
            work.mkdir()
            source = work / 'source.geojson'
            features = [dict(type='Feature', id=i, properties={'value':i}, geometry=dict(type='LineString', coordinates=[[i * 10 + j, j % 2, 0] for j in range(vertices)])) for i in range(count)]
            source.write_text(json.dumps(dict(type='FeatureCollection', features=features)))
            output = work / 'out.3tz'
            timing = work / 'rss'
            command = [str(launcher), str(timing), str(binary), '--json', 'vector', '-i', str(source), '-o', str(output), '--sourceCrs', 'local', '--explicit', '--jobs', '1', '--lodLevels', '1', '--maxFeatures', '64', '--maxVertices', '65536']
            with (work/'stdout').open('w') as stdout, (work/'stderr').open('w') as stderr:
                child = subprocess.Popen(command, stdout=stdout, stderr=stderr)
                peak_files = peak_bytes = 0
                while child.poll() is None:
                    try:
                        files = [p for p in work.rglob('*') if p.is_file() and p not in (source, output, timing, work/'stdout', work/'stderr')]
                        peak_files = max(peak_files, len(files))
                        peak_bytes = max(peak_bytes, sum(p.stat().st_size for p in files))
                    except FileNotFoundError:
                        pass
                    time.sleep(.002)
            assert child.returncode == 0, (work/'stderr').read_text()
            leftovers = [str(p.relative_to(work)) for p in work.iterdir() if p.is_dir()]
            assert not leftovers, leftovers
            cases.append(dict(feature_count=count, maximum_source_vertices=vertices, input_bytes=source.stat().st_size,
                              peak_rss_kib=int(timing.read_text()), sampled_scratch_files=peak_files,
                              sampled_scratch_bytes=peak_bytes, output_bytes=output.stat().st_size,
                              report=json.loads((work/'stdout').read_text())))
    args.output.write_text(json.dumps(dict(binary_sha256=hashlib.sha256(binary.read_bytes()).hexdigest(), platform=platform.platform(),
                                         caveat='wait4 RSS per process; scratch sampled every >=2ms and can miss peaks. Inventory includes producer and publication staging; measurements do not establish an asymptotic bound.', cases=cases), indent=2)+'\n')


if __name__ == '__main__':
    main()
