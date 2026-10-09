"""Independent real-consumer D2 publication checks; no fault injection or Cargo.

Run with a native candidate on an admitted local filesystem. Concurrent races
are unscheduled observations; deterministic namespace schedules belong to the
separate private runtime fault harness.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import stat
import subprocess
import tempfile

from d1_raster_oracle import decode_png, write_fixture


def digest(path):
    return hashlib.sha256(Path(path).read_bytes()).hexdigest()


def inventory(root):
    """Inventory leaf objects without following symlinks, including mode/bytes."""
    root = Path(root)
    mode = root.lstat().st_mode
    if stat.S_ISLNK(mode):
        return {'kind': 'symlink', 'target': os.readlink(root)}
    if stat.S_ISREG(mode):
        return {'kind': 'file', 'mode': stat.S_IMODE(mode), 'sha256': digest(root)}
    assert stat.S_ISDIR(mode)
    return {'kind': 'directory', 'mode': stat.S_IMODE(mode),
            'members': {p.name: inventory(p) for p in sorted(root.iterdir())}}


def old_tree(path):
    path.mkdir()
    (path / 'nested').mkdir()
    (path / 'nested/old.bin').write_bytes(b'old\x00bytes\xff')
    (path / 'empty').mkdir()
    (path / 'old-report.json').write_text('{"old":true}\n')


def make_source(path, variant=0):
    write_fixture(path)
    if variant:
        image = bytearray(path.read_bytes())
        image[-196608:] = bytes(value ^ variant for value in image[-196608:])
        path.write_bytes(image)


def check_output(path, variant):
    members = {str(p.relative_to(path)) for p in path.rglob('*') if p.is_file()}
    assert members == {'tiles/3/5/2.png', 'tilejson.json', 'report.json'}, members
    channels, pixels = decode_png(path / 'tiles/3/5/2.png')
    for row in range(256):
        for col in range(256):
            expected = bytes(value ^ variant for value in (col, row, (3 * col + 5 * row) % 256))
            start = (row * 256 + col) * channels
            assert pixels[start:start + 3] == expected, (col, row, variant)
            if channels == 4:
                assert pixels[start + 3] == 255
    tilejson = json.loads((path / 'tilejson.json').read_text())
    assert tilejson['scheme'] == 'xyz' and tilejson['minzoom'] == tilejson['maxzoom'] == 3
    report = json.loads((path / 'report.json').read_text())
    assert report['profile'] == 'd1-web-mercator-rgb'
    assert (report['z'], report['x'], report['y']) == (3, 5, 2)
    return report


def run(binary, evidence_path=None):
    evidence = {'binary_sha256': digest(binary), 'positive_cases': [], 'preservation_cases': [],
                'concurrent_cases': [], 'scope': 'Real CLI outcomes; unscheduled cross-process races are not deterministic fault proof'}
    with tempfile.TemporaryDirectory(prefix='d2-oracle-') as temporary:
        root = Path(temporary)
        source = root / 'source.tif'
        make_source(source)
        def command(input_path, output_path, force=True):
            args = [str(binary), '--json', 'raster-tile-to-directory', '-i', str(input_path),
                    '-o', str(output_path), '--zoom', '3', '--x', '5', '--y', '2']
            return args + (['--force'] if force else [])
        def invoke(input_path, output_path, force=True):
            result = subprocess.run(command(input_path, output_path, force), capture_output=True, text=True)
            parsed = json.loads(result.stdout)
            assert result.returncode == parsed.get('exitCode', 0)
            return result, parsed
        for kind in ('missing', 'empty-directory', 'populated-directory', 'file', 'symlink-directory', 'dangling-symlink'):
            output = root / ('replace-' + kind)
            referent = root / ('referent-' + kind)
            if kind == 'empty-directory':
                output.mkdir()
            elif kind == 'populated-directory':
                old_tree(output)
            elif kind == 'file':
                output.write_bytes(b'old file')
            elif kind == 'symlink-directory':
                old_tree(referent)
                output.symlink_to(referent, target_is_directory=True)
            elif kind == 'dangling-symlink':
                output.symlink_to(referent)
            reference_before = inventory(referent) if referent.exists() else None
            result, parsed = invoke(source, output)
            assert result.returncode == 0, (kind, parsed, result.stderr)
            report = check_output(output, 0)
            assert parsed['rasterReport'] == report
            if reference_before is not None:
                assert inventory(referent) == reference_before
            else:
                assert not referent.exists()
            assert not list(root.glob('.tiles-*'))
            evidence['positive_cases'].append({'name': kind, 'report': report})
        for kind in ('directory', 'file', 'symlink'):
            output = root / ('preserve-' + kind)
            if kind == 'directory':
                old_tree(output)
            elif kind == 'file':
                output.write_bytes(b'unchanged')
            else:
                output.symlink_to(root / 'absent-referent')
            before = inventory(output)
            result, parsed = invoke(source, output, False)
            assert result.returncode == 5 and parsed['error']['kind'] == 'output_conflict', parsed
            assert inventory(output) == before
            invalid = root / 'invalid.tif'
            write_fixture(invalid, shifted=True)
            result, parsed = invoke(invalid, output)
            assert result.returncode == 2 and parsed['error']['kind'] == 'unsupported', parsed
            assert inventory(output) == before
            assert not list(root.glob('.tiles-*'))
            evidence['preservation_cases'].append({'name': kind, 'checks': ['create-new-conflict', 'producer-refusal']})
        other_source = root / 'other-source.tif'
        make_source(other_source, 55)
        earlier_holders = set()
        for number in range(8):
            output = root / ('concurrent-' + str(number))
            old_tree(output)
            original_inventory = inventory(output)
            processes = [subprocess.Popen(command(p, output), stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True)
                         for p in (source, other_source)]
            results = []
            for process in processes:
                stdout, stderr = process.communicate(timeout=30)
                parsed = json.loads(stdout)
                assert process.returncode in (0, 5), (process.returncode, parsed, stderr)
                if process.returncode:
                    assert parsed['error']['kind'] == 'output_conflict', parsed
                results.append({'exit_code': process.returncode, 'result': parsed})
            allowed_holders = set(earlier_holders)
            for item in results:
                if item['exit_code'] == 0:
                    assert item['result']['cleanupDiagnostics'] == []
                    continue
                recovery = item['result']['error']['recovery']
                assert item['result']['error']['retainedPaths'] == []
                if recovery is not None:
                    native = recovery['nativePaths']
                    if os.name == 'posix':
                        assert native['encoding'] == 'unix-bytes'
                        assert bytes(native['output']) == os.fsencode(output.resolve())
                        assert bytes(native['previousOutput']) == os.fsencode(recovery['previousOutput'])
                    else:
                        assert native['encoding'] == 'windows-utf16'
                        for key in ('output', 'previousOutput'):
                            exact = b''.join(int(unit).to_bytes(2, 'little') for unit in native[key])
                            assert exact.decode('utf-16-le', 'surrogatepass') == recovery[key]
                    assert Path(recovery['output']) == output.resolve()
                    previous = Path(recovery['previousOutput'])
                    assert previous.parent.parent == root
                    assert previous.name == 'previous' and previous.parent.name.startswith('.tiles-previous-')
                    assert previous.is_dir()
                    if inventory(previous) != original_inventory:
                        _, retained_pixels = decode_png(previous / 'tiles/3/5/2.png')
                        retained_variant = 55 if retained_pixels[:3] == b'777' else 0
                        check_output(previous, retained_variant)
                    allowed_holders.add(previous.parent)
            observed_holders = set(root.glob('.tiles-*'))
            assert observed_holders == allowed_holders, (observed_holders, allowed_holders)
            assert any(item['exit_code'] == 0 for item in results)
            channels, pixels = decode_png(output / 'tiles/3/5/2.png')
            winner = 55 if pixels[:3] == b'777' else 0
            check_output(output, winner)
            earlier_holders = allowed_holders
            evidence['concurrent_cases'].append({'number': number, 'final_variant': winner, 'processes': results})
    evidence['counts'] = {'positive': len(evidence['positive_cases']), 'preservation': len(evidence['preservation_cases']),
                          'concurrent': len(evidence['concurrent_cases'])}
    if evidence_path:
        Path(evidence_path).write_text(json.dumps(evidence, indent=2) + '\n')
    print('D2 real CLI oracle passed: 6 replacement leaves, 6 preservation checks, 8 cross-process races')
    return evidence


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('binary', type=Path)
    parser.add_argument('--json-output', type=Path)
    args = parser.parse_args()
    run(args.binary.resolve(), args.json_output)
