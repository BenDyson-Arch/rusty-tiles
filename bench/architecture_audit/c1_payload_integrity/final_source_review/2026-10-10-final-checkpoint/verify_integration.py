"""Inspect frozen integration receipts and bytes; never execute Cargo or target."""
import hashlib
import importlib.util
import json
import pathlib
import struct
import zipfile

REPO = pathlib.Path(__file__).resolve().parents[5]
EVIDENCE = pathlib.Path('/tmp/rusty-tiles-payload-final-evidence')


def sha(raw):
    return hashlib.sha256(raw).hexdigest()


pin_path = EVIDENCE / 'final-build/source-pin.json'
pin = json.loads(pin_path.read_bytes())
for group in ['production_sha256', 'acceptance_inputs_sha256']:
    for path, digest in pin[group].items():
        assert sha((REPO / path).read_bytes()) == digest, path
executions_path = EVIDENCE / 'integration-executions.json'
executions = json.loads(executions_path.read_bytes())
assert len(executions) == 8
assert {(row['flavor'], row['name']) for row in executions} == {
    (flavor, name) for flavor in ['portable', 'native']
    for name in ['c1-corpus', 'archive45', 'resources20', 'producer']}
for row in executions:
    assert row['exitCode'] == 0 and row['inputsAndBinaryUnchanged'] is True
    assert row['sourceCommit'] == pin['source_commit']
    assert row['sourcePinSha256'] == sha(pin_path.read_bytes())
    assert sha(pathlib.Path(row['command'][row['command'].index('--binary') + 1]).read_bytes()) == row['binarySha256']
    prefix = f"{row['flavor']}-{row['name']}"
    assert sha((EVIDENCE / (prefix + '.json')).read_bytes()) == row['outputSha256']
    assert sha((EVIDENCE / (prefix + '.log')).read_bytes()) == row['logSha256']

binding = json.loads((EVIDENCE / 'c1-producer-local-two-workers-binding.json').read_bytes())
original = pathlib.Path(binding['original']).read_bytes()
adapter = pathlib.Path(binding['localCopy']).read_bytes()
assert sha(original) == binding['originalSha256']
assert sha(adapter) == binding['localCopySha256']
expected_adapter = original.replace(
    b"pathlib.Path(__file__).parent/'fixtures/c1-framing-staircase.geojson'",
    b"pathlib.Path('/tmp/rusty-tiles-c1-payload-foundation/tests/fixtures/c1-framing-staircase.geojson')")
expected_adapter = expected_adapter.replace(b"'16384','--json'", b"'16384','--jobs','2','--json'")
assert adapter == expected_adapter

archive_driver = REPO / 'bench/architecture_audit/implicit_rewrite/archive_read_acceptance/probe.py'
spec = importlib.util.spec_from_file_location('frozen_archive_truth', archive_driver)
oracle = importlib.util.module_from_spec(spec)
spec.loader.exec_module(oracle)
archive_truth = {row['label']: row for row in oracle.fixtures()}
assert len(archive_truth) == 45

results = []
for flavor in ['portable', 'native']:
    corpus = json.loads((EVIDENCE / f'{flavor}-c1-corpus.json').read_bytes())
    directory = EVIDENCE / f'{flavor}-c1-fixtures'
    manifest = json.loads((directory / 'manifest.json').read_bytes())
    cases = {row['name']: row for row in manifest['cases']}
    assert corpus['readOnly'] is True and len(corpus['cases']) == len(cases) == 135
    assert {row['name'] for row in corpus['cases']} == cases.keys()
    for row in corpus['cases']:
        case = cases[row['name']]
        assert row['exitCode'] == case['exitCode']
        assert sha((directory / case['path']).read_bytes()) == case['sha256']
        if case['expectedKind']:
            assert row['report']['error']['code'] == case['expectedKind']
        else:
            assert row['report']['ok'] is True
            assert len(row['report']['checks']) == 13 and len(row['report']['limits']) == 15

    archives = json.loads((EVIDENCE / f'{flavor}-archive45.json').read_bytes())
    assert archives['read_only'] is True and archives['source_commit'] == pin['source_commit']
    assert archives['fixture_driver_sha256'] == sha(archive_driver.read_bytes())
    assert archives['runner_sha256'] == sha((REPO / 'tests/archive_read_oracle.py').read_bytes())
    assert len(archives['records']) == 45
    for row in archives['records']:
        truth = archive_truth[row['label']]
        raw = pathlib.Path(row['command'][-1]).read_bytes()
        assert raw == truth['raw'] and sha(raw) == row['archive_sha256']
        assert row['expected'] == row['actual'] == truth['expected'] == oracle.classify(raw)['kind']
        assert row['read_only'] is True
        report = json.loads(row['stdout'])
        assert row['exit_code'] == {'admitted': 0, 'unsupported': 2, 'invalid_input': 3}[row['actual']]
        assert (report['ok'] is True if row['actual'] == 'admitted' else report['error']['code'] == row['actual'])

    resources = json.loads((EVIDENCE / f'{flavor}-resources20.json').read_bytes())
    assert resources['readOnly'] is True and len(resources['runs']) == 20
    assert len({(row['name'], row['repetition']) for row in resources['runs']}) == 20
    work = EVIDENCE / f'{flavor}-resources20'
    for row in resources['runs']:
        run_dir = work / f"{row['name']}-{row['repetition']}"
        assert json.loads((run_dir / 'stdout').read_bytes()) == row['report']
        assert (run_dir / 'stderr').read_text() == row['stderr']
        command = json.loads((run_dir / 'config.json').read_bytes())['command']
        assert command[0] == next(r for r in executions if r['flavor'] == flavor and r['name'] == 'resources20')['command'][4]
        assert row['descriptorPermissionDenials'] == 0 and row['descriptorSamples'] > 0
        assert row['maximumRssKiB'] > 0 and row['wallSeconds'] < 30
        if row['name'] in ['triangle', 'legal_shared_indices']:
            assert row['exitCode'] == 0 and row['report']['ok'] is True
        else:
            assert row['exitCode'] == 3 and row['report']['error']['code'] == 'resource_limit'

    producer = json.loads((EVIDENCE / f'{flavor}-producer.json').read_bytes())
    assert producer['sourceReadOnly'] is True
    command = producer['command']
    assert command[command.index('--jobs') + 1] == '2'
    assert sha(pathlib.Path(command[command.index('-i') + 1]).read_bytes()) == producer['sourceSha256']
    target = pathlib.Path(command[command.index('-o') + 1])
    assert sha(target.read_bytes()) == producer['archiveSha256']
    assert producer['counts']['features'] == producer['counts']['fragmentedPolygons'] == 1
    assert producer['counts']['skippedFeatures'] == 0
    reconstructed = []
    with zipfile.ZipFile(target) as archive:
        for name in archive.namelist():
            if not name.endswith('.b3dm'):
                continue
            raw = archive.read(name)
            magic, version, total, *sections = struct.unpack_from('<4s6I', raw)
            assert magic == b'b3dm' and version == 1 and total == len(raw) and total % 8 == 0
            start = 28 + sum(sections)
            assert start % 8 == 0
            magic, version, glb_length = struct.unpack_from('<4sII', raw, start)
            assert magic == b'glTF' and version == 2 and glb_length % 4 == 0
            assert start + glb_length <= total and raw[start + glb_length:] == bytes(total - start - glb_length)
            pos = start + 12
            document = None
            bin_length = None
            while pos < start + glb_length:
                length, kind = struct.unpack_from('<I4s', raw, pos)
                assert length % 4 == 0 and pos + 8 + length <= start + glb_length
                content = raw[pos + 8:pos + 8 + length]
                if kind == b'JSON':
                    document = json.loads(content)
                elif kind == b'BIN\0':
                    bin_length = length
                    declared = document['buffers'][0]['byteLength']
                    assert 0 <= length - declared <= 3 and content[declared:] == bytes(length - declared)
                pos += 8 + length
            assert pos == start + glb_length and bin_length is not None
            reconstructed.append(dict(member=name, b3dmBytes=total, glbBytes=glb_length,
                                      binBytes=bin_length, declaredBufferBytes=declared))
    assert reconstructed == producer['controls']
    assert len(reconstructed) == (8 if flavor == 'portable' else 6)
    for group in [corpus, resources, producer]:
        assert group['binarySha256'] == archives['binary_sha256']
    runs = resources['runs']
    results.append(dict(flavor=flavor, corpusCases=135, archiveTruthCases=45,
                        resourceObservations=20, independentlyDecodedB3dmMembers=len(reconstructed),
                        rssKiBRange=[min(r['maximumRssKiB'] for r in runs), max(r['maximumRssKiB'] for r in runs)],
                        sampledPeakDescriptors=max(r['peakDescriptors'] for r in runs),
                        descriptorPermissionDenials=sum(r['descriptorPermissionDenials'] for r in runs),
                        maximumWallSeconds=max(r['wallSeconds'] for r in runs)))

for kind in ['native', 'bindings']:
    receipt = json.loads((EVIDENCE / f'final-build/{kind}-clippy-receipt.json').read_bytes())
    assert receipt['exitCode'] == 0 and receipt['productionAndSelectedInputsUnchanged'] is True
    assert receipt['sourceCommit'] == pin['source_commit']
    assert receipt['sourcePinSha256'] == sha(pin_path.read_bytes())
    assert receipt['logSha256'] == sha((EVIDENCE / f'final-build/{kind}-clippy.log').read_bytes())

print(json.dumps(dict(sourceCommit=pin['source_commit'], sourcePinSha256=sha(pin_path.read_bytes()),
                      integrationReceiptSha256=sha(executions_path.read_bytes()), allEightPassed=True,
                      all97ProductionAnd179AcceptanceInputsMatch=True, producerAdapterOnlyPathAndJobs=True,
                      nativeAllTargetsAndBindingsClippyPassed=True, results=results,
                      limits='Existing corpus does not compare whole positive reports; separate 210/context controls do. Resource measurements are finite Linux observations, and producer framing is not geometry equivalence.'), indent=2))
