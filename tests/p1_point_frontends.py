#!/usr/bin/env python3
"""Required CLI/installed-wheel point checks using P1's independent readers.

Run with --binary and optional --json-output. Missing wheels/codecs fail. laspy
only compresses the literal LAS input; expected rows always come from P1.
"""
import argparse
import hashlib
import importlib.metadata
import json
from pathlib import Path
import struct
import tempfile
import zipfile

import laspy
import rusty_tiles

import p1_point_oracle as oracle


def decoded(archive):
    """Compare drawn world positions/fields, including sampled parent contents."""
    with zipfile.ZipFile(archive) as stream:
        members = {name: stream.read(name) for name in stream.namelist()}
    manifest = json.loads(members['tileset.json'])
    stats = dict(explicit=0, implicit_roots=0, documents=1)
    root = oracle.expand_forest(manifest['root'], members, set(),
                                'schema' in manifest, stats)
    contents = []

    def visit(tile, parent):
        frame = oracle.multiply(parent, tile.get('transform', oracle.IDENTITY))
        positions, columns, schema = oracle.decoded_points(members[tile['content']['uri']])
        contents.append((columns, schema, [oracle.transform(frame, p) for p in positions],
                         tile['geometricError'], tile['boundingVolume']))
        for child in tile.get('children', []):
            visit(child, frame)

    visit(root, oracle.IDENTITY)
    return contents


def paired(binary, work):
    cases = []
    # Local outlier has repeated records and a genuinely sparse tail; format 8
    # adds NIR/extended flags. Both projected and geographic cases are analytic.
    for name, variant, crs, height in (
        ('local', 'outlier', 'local', None),
        ('wgs84', 'geographic', 'EPSG:4326', 7.),
        ('utm32', 'projected_origin', 'EPSG:32632', 7.),
    ):
        source = work / f'{name}.las'
        oracle.fixture(source, fmt=8, variant=variant, metadata=True)
        compressed = source.with_suffix('.laz')
        laspy.read(source).write(compressed)
        # laspy 2.7 resets declared Extra Bytes min/max to out-of-range
        # accumulator sentinels when rewriting this literal metadata fixture.
        # Compression is only a delivery adapter: restore the original VLR,
        # keeping independently authored declarations as well as record bytes.
        original, delivery_bytes = source.read_bytes(), bytearray(compressed.read_bytes())
        header = struct.unpack_from('<H', original, 94)[0]
        size = struct.unpack_from('<H', original, header + 20)[0]
        oracle.require(original[header:header + 22] == delivery_bytes[header:header + 22],
                       'LAZ adapter preserves Extra Bytes VLR identity')
        delivery_bytes[header:header + 54 + size] = original[header:header + 54 + size]
        compressed.write_bytes(delivery_bytes)
        for delivery in (source, compressed):
            for defaults in (False, True):
                stem = f'{name}-{delivery.suffix[1:]}-{defaults}'
                cli, python = (work / f'{stem}-{adapter}.3tz' for adapter in ('cli', 'python'))
                options = ['--sourceCrs', crs]
                kwargs = dict(source_crs=crs)
                if not defaults:
                    options += ['--maxPoints', '16', '--chunkPoints', '3', '--metadata-attributes']
                    kwargs.update(max_points=16, chunk_points=3, metadata_attributes=True)
                if height is not None:
                    options += ['--heightOffset', str(height)]
                    kwargs['height_offset'] = height
                completed, response = oracle.invoke(binary, delivery, cli, options)
                oracle.require(completed.returncode == 0, completed.stdout + completed.stderr)
                result = rusty_tiles.point_cloud_to_3tz(delivery, python, **kwargs)
                with zipfile.ZipFile(cli) as archive:
                    report = json.loads(archive.read('conversion.json'))
                with zipfile.ZipFile(python) as archive:
                    oracle.require(json.loads(archive.read('conversion.json')) == result.report,
                                   'Python returned/finalized report')
                oracle.require(report == result.report, 'CLI/Python resolved report settings')
                oracle.require(result.cleanup_diagnostics == [], 'Python cleanup diagnostics')
                for key, value in (response['settings'] | response['counts']).items():
                    oracle.require(report[key] == value, 'CLI summary/finalized report: ' + key)
                oracle.require(report['heightOffset'] == height, 'explicit height policy')
                oracle.require(report['maxPoints'] == (8192 if defaults else 16), 'point default')
                oracle.require(report['chunkPoints'] == (100000 if defaults else 3), 'chunk default')
                expected = None
                if name == 'utm32':
                    expected = [oracle.ecef(dict(source_x=9., source_y=0., source_z=123.), 7.)] * 257
                budget = report['maxPoints']
                audits = [oracle.audit(source, output, budget, geographic=name == 'wgs84',
                                       height=height or 0., require_multileaf=not defaults,
                                       expected_positions_override=expected)
                          for output in (cli, python)]
                # In addition to each independent source audit, compare the two
                # adapters directly without requiring byte-identical archives.
                oracle.require(decoded(cli) == decoded(python), 'CLI/Python decoded content')
                for audit in audits:
                    oracle.require(audit['max_world_position_difference_metres'] <=
                                   report['maxPositionRoundingMetres'] + 1e-6,
                                   'reported rounding plus analytic reference allowance')
                cases.append(dict(case=stem, report=report, audit=audits[0],
                                  source_sha256=hashlib.sha256(source.read_bytes()).hexdigest(),
                                  delivery_sha256=hashlib.sha256(delivery.read_bytes()).hexdigest()))
    return cases


def refusals(binary, work):
    source = work / 'refusals.las'
    oracle.fixture(source, count=23)
    original_source = source.read_bytes()
    output = work / 'refusals.3tz'
    output.write_bytes(b'previous archive')
    before = set(work.iterdir())
    cases = []
    for name, options, kwargs, kind, exception in (
        ('missing-height', ['--sourceCrs', 'EPSG:4326'], dict(source_crs='EPSG:4326'),
         'invalid_request', rusty_tiles.InvalidRequestError),
        ('local-height', ['--sourceCrs', 'local', '--heightOffset', '0'],
         dict(source_crs='local', height_offset=0), 'invalid_request', rusty_tiles.InvalidRequestError),
        ('unsupported-extra', ['--sourceCrs', 'local'], dict(source_crs='local'),
         'unsupported', rusty_tiles.UnsupportedError),
    ):
        if name == 'unsupported-extra':
            # An untyped Extra Bytes declaration is explicitly outside P1.
            data = bytearray(original_source)
            data[227 + 54 + 2] = 0
            data[227 + 54 + 3] = 1
            source.write_bytes(data)
        refusal_source = source.read_bytes()
        events = []
        completed, response = oracle.invoke(binary, source, output, [*options, '--force'])
        oracle.require(completed.returncode != 0 and response['error']['code'] == kind,
                       'CLI refusal category: ' + name)
        try:
            rusty_tiles.point_cloud_to_3tz(source, output, force=True, callback=events.append, **kwargs)
        except exception:
            pass
        else:
            raise AssertionError('Python accepted ' + name)
        oracle.require(not events, 'refusal before callbacks')
        oracle.require(output.read_bytes() == b'previous archive', 'refusal replaced output')
        oracle.require(source.read_bytes() == refusal_source, 'refusal changed source')
        oracle.require(set(work.iterdir()) == before, 'refusal staging leak')
        cases.append(dict(case=name, error_kind=kind))
    source.write_bytes(original_source)
    for phase in ('ingestion', 'tiling', 'point_archive', 'ready_to_publish'):
        failure = RuntimeError('point callback at ' + phase)
        reached = []

        def abort(event):
            if event.get('phase') == phase:
                reached.append(phase)
                raise failure

        try:
            rusty_tiles.point_cloud_to_3tz(source, output, source_crs='local', force=True,
                                          max_points=3, chunk_points=2, callback=abort)
        except RuntimeError as caught:
            oracle.require(caught is failure, 'original callback exception identity')
        else:
            raise AssertionError('callback abort did not fail')
        oracle.require(reached == [phase], 'callback phase reached exactly once')
        oracle.require(output.read_bytes() == b'previous archive', 'callback replaced output')
        oracle.require(source.read_bytes() == original_source, 'callback changed source')
        oracle.require(set(work.iterdir()) == before, 'callback staging leak')
        cases.append(dict(case='callback-' + phase, preserved=True))
    return cases


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--binary', required=True, type=Path)
    parser.add_argument('--json-output', type=Path)
    args = parser.parse_args()
    binary = args.binary.resolve(strict=True)
    with tempfile.TemporaryDirectory(prefix='p1-frontends-') as directory:
        work = Path(directory)
        result = dict(binary_sha256=hashlib.sha256(binary.read_bytes()).hexdigest(),
                      module=rusty_tiles.__file__, package_version=rusty_tiles.__version__,
                      delivery_encoder={name: importlib.metadata.version(name)
                                        for name in ('laspy', 'lazrs')},
                      cases=paired(binary, work),
                      refusals=refusals(binary, work))
    encoded = json.dumps(result, indent=2) + '\n'
    if args.json_output:
        args.json_output.write_text(encoded)
    print(encoded)


if __name__ == '__main__':
    main()
