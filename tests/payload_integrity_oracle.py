"""Run the frozen independent payload corpus and retain exact execution bindings."""
import argparse
import base64
import hashlib
import importlib.util
import json
from pathlib import Path
import subprocess
import tarfile


def sha(path):
    with path.open('rb') as source:
        return hashlib.file_digest(source, 'sha256').hexdigest()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--binary', type=Path, required=True)
    parser.add_argument('--work', type=Path, required=True)
    parser.add_argument('--output', type=Path, required=True)
    args = parser.parse_args()
    root = Path(__file__).resolve().parents[1]
    controls = root / 'bench/architecture_audit/c1_payload_integrity/production_controls'
    args.work.mkdir(parents=True, exist_ok=False)
    args.output.parent.mkdir(parents=True, exist_ok=True)
    correction = controls / 'polygon_correction/2026-10-10'
    # C2 amendment (2026-10-11): the same fixtures with C2 report expectations derived
    # from the independent C2 oracle; the 2026-10-10 bundles stay as historical evidence.
    # Validator-caps amendment (2026-10-11): the C2-amended bundles with the raised
    # ValidationLimits defaults in admitted reports; the C2 bundles stay as history.
    # C2-WB cap amendment (2026-10-12): containmentWorkUnits 2^28 -> 2^29 in admitted
    # reports; the validator-caps bundles stay as history.
    # H7 legacy-content amendment (2026-10-13): the 12 b3dm controls of corrected-205
    # expect Unsupported; polygon-sensitive-5 holds no legacy content and keeps its bundle.
    amendment = controls / 'c2wb_cap_amendment/2026-10-12'
    legacy = controls / 'legacy_unsupported_amendment/2026-10-13'
    bundles = [
        ('corrected205', legacy / 'corrected-205-legacy-fixtures.tar.gz',
         '7fe07fd1341d8fdbc743be9cb7cd0b1fa063731b5678621d652e799ad0101b8d',
         'f4ce9c7bd60e7788ca9ec7d6af5c7edf20f08e2c2cc8f25ced32a2d2c9a01815', 205),
        ('polygon5', amendment / 'polygon-sensitive-5-c2wb-fixtures.tar.gz',
         'd64eaf9ade5bb7b3dd5d73a5b2398a12083c2bc3ae948a3a250a07139971b25f',
         '2d5d56c03526d50ecc864da16ae744ffa44efaa10e683c9fa4f7bdd40cda6ef8', 5),
    ]
    phases = {}
    status = 1
    failure = None
    try:
        driver = controls / 'driver.py'
        assert sha(driver) == '5bcd8e1e0a34e86fa39339eb1d499e95851a871a2841d142145dc8cbb911f5d7'
        assert sha(correction / 'generate.py') == 'b0ae15309533d11707956748dd0e1425004d75a6a86140bd7dff8038bae3f924'
        assert sha(correction / 'references/EXT_mesh_polygon.md') == '777c13228de2de5838a8f0eed5c4f653674a5278c30306124bb8ccbe2fe1016d'
        files = subprocess.check_output(['git', 'ls-files'], cwd=root, text=True).splitlines()
        selected = [name for name in files if name in ('Cargo.toml', 'Cargo.lock', 'build.rs', 'bindings/python/Cargo.toml')
                    or name.startswith(('src/', 'bindings/python/src/')) and name.endswith('.rs')]
        spec = importlib.util.spec_from_file_location('independent_payload_controls', driver)
        oracle = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(oracle)
        for label, name, bundle_hash, manifest_hash, count in bundles:
            status = 1
            bundle = name
            assert sha(bundle) == bundle_hash
            fixtures = args.work / label / 'fixtures'
            with tarfile.open(bundle) as archive:
                archive.extractall(fixtures, filter='data')
            assert sha(fixtures / 'manifest.json') == manifest_hash
            pin = args.work / label / 'source-pin.json'
            pin.write_text(json.dumps({
                'source_commit': subprocess.check_output(['git', 'rev-parse', 'HEAD'], cwd=root, text=True).strip(),
                'source_tree': subprocess.check_output(['git', 'rev-parse', 'HEAD^{tree}'], cwd=root, text=True).strip(),
                'production_sha256': {name: sha(root / name) for name in selected},
                'runner_sha256': sha(Path(__file__)),
                'fixture_bundle_sha256': sha(bundle),
                'scope': 'Actual selected checkout and supplied CLI; finite controls, no release acceptance.',
            }, indent=2) + '\n')
            status = oracle.run(args.binary.resolve(), sha(args.binary), pin, sha(pin), fixtures, args.work / label / 'runs')
            assert status == 0, label
    except Exception as error:
        status = 1
        failure = {'type': type(error).__name__, 'message': str(error)}
    finally:
        for label, _, _, _, count in bundles:
            receipt = args.work / label / 'runs/receipt.json'
            if not receipt.exists():
                continue
            phase = json.loads(receipt.read_text())
            for case in phase['cases']:
                for stream in ('stdout', 'stderr'):
                    original = (args.work / label / 'runs' / (case['name'] + '.' + stream)).read_bytes()
                    assert hashlib.sha256(original).hexdigest() == case[stream + 'Sha256']
                    case[stream + 'Base64'] = base64.b64encode(original).decode('ascii')
            phase['expectedCaseCount'] = count
            phases[label] = phase
        evidence = {
            'runnerSha256': sha(Path(__file__)), 'runnerExitCode': status, 'phases': phases,
            'historicalOriginal205': '204 pass / 1 invalid positive oracle; unchanged historical evidence retained separately.',
            'adjudicationSha256': sha(correction / 'adjudication.md'),
        }
        if failure:
            evidence['runnerFailure'] = failure
        args.output.write_text(json.dumps(evidence, indent=2) + '\n')
    assert status == 0 and all(len(phases[label]['cases']) == count for label, _, _, _, count in bundles), args.output
    print(json.dumps({'ok': True, 'cases': 210, 'evidence': str(args.output)}))


if __name__ == '__main__':
    main()
