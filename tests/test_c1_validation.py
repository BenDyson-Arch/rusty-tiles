"""Standalone, opt-in installed-wheel parity; not an ordinary unittest suite.

--python-module points to the directory containing an installed rusty_tiles wheel.
No native Python packages, production fixture helpers, or compiler are required.
"""
import argparse
import hashlib
import importlib
import json
import pathlib
import sys
sys.path.insert(0, str(pathlib.Path(__file__).resolve().parent))
from c1_validation_oracle import snapshot


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--python-module', type=pathlib.Path)
    parser.add_argument('--cli-evidence', type=pathlib.Path)
    parser.add_argument('--corpus', type=pathlib.Path, default=pathlib.Path(__file__).parent/'fixtures/c1')
    parser.add_argument('--output', type=pathlib.Path)
    args = parser.parse_args()
    if args.python_module: sys.path.insert(0, str(args.python_module.resolve()))
    module = importlib.import_module('rusty_tiles'); before = snapshot(args.corpus)
    manifest = json.loads((args.corpus/'manifest.json').read_text()); results = []
    cli = {case['name']: case for case in json.loads(args.cli_evidence.read_text())['cases']} if args.cli_evidence else None
    def normalize(report):
        report = dict(report); report['archive'] = '<archive>'; return report
    classes = {'invalid_input': module.DataError, 'unsupported': module.UnsupportedError,
               'resource_limit': module.ResourceLimitError, 'io': module.TilesIOError}
    for case in manifest['cases']:
        path = args.corpus/case['path']
        assert hashlib.sha256(path.read_bytes()).hexdigest() == case['sha256']
        try: report = module.validate(path)
        except Exception as error:
            assert case['expectedKind'] and isinstance(error, classes[case['expectedKind']]), (case['name'], repr(error))
            if cli: assert cli[case['name']]['report']['error']['code'] == case['expectedKind']
            results.append({'name': case['name'], 'kind': case['expectedKind'], 'pythonClass': type(error).__name__})
        else:
            assert case['expectedKind'] is None, (case['name'], report)
            assert report['ok'] is True
            if cli: assert normalize(report) == normalize(cli[case['name']]['report']), (case['name'], report, cli[case['name']])
            results.append({'name': case['name'], 'report': report})
    assert snapshot(args.corpus) == before
    result = {'ok': True, 'module': str(module.__file__), 'cases': results, 'readOnly': True}
    if args.output: args.output.write_text(json.dumps(result, indent=2)+'\n')
    else: print(json.dumps(result, indent=2))


if __name__ == '__main__': main()
