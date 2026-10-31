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
    # VS-C limits amendment (2026-10-14): storedBytes, limitDerivation, null volume caps and
    # liveDocumentBytes in admitted reports; the H7 and C2-WB bundles stay as history.
    # Payload-element amendment (2026-10-30/31): totalPayloadElements 16 M + 130 per stored byte
    # in admitted reports; the VS-C bundles stay as history.
    amendment = controls / 'payload_element_ratio_amendment/2026-10-30'
    bundles = [
        ('corrected205', amendment / 'corrected-205-ratio-fixtures.tar.gz',
         '560a5a25964b827a68da56a76ed435711fd554e57a2b9bf7e2657449fa4ee2b4',
         'd9aad7f7c0632e5a29f53678a2de5dab09e864171162ca3e9d287f5d6afdcaca', 205),
        ('polygon5', amendment / 'polygon-sensitive-5-ratio-fixtures.tar.gz',
         'cf62421e8afd1934b5c205e93f2f91604ac0cd2a15ac0cf5562efad089d0abb4',
         '2f022f31e0b1402eee3216ca49238ee5392be37525147689e26d7378815f8cc8', 5),
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
