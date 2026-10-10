#!/usr/bin/env python3
"""Verify or extract the verbatim receipts listed in storage-index.json."""
import argparse
import gzip
import hashlib
import json
from pathlib import Path, PurePosixPath


ROOT = Path(__file__).resolve().parent


def local_path(name):
    path = PurePosixPath(name)
    if not name or path.is_absolute() or any(part in ('.', '..') for part in name.split('/')):
        raise ValueError(f'unsafe indexed path: {name!r}')
    result = ROOT.joinpath(*path.parts).resolve(strict=True)
    if not result.is_relative_to(ROOT):
        raise ValueError(f'indexed path escapes evidence directory: {name!r}')
    return result


def read_receipt(entry):
    if not entry['original'].endswith('.json') or entry['stored'] != entry['original'] + '.gz':
        raise ValueError('invalid receipt mapping')
    packed = local_path(entry['stored']).read_bytes()
    if len(packed) != entry['compressed_bytes'] or hashlib.sha256(packed).hexdigest() != entry['compressed_sha256']:
        raise ValueError(f"compressed identity mismatch: {entry['stored']}")
    raw = gzip.decompress(packed)
    if len(raw) != entry['uncompressed_bytes'] or hashlib.sha256(raw).hexdigest() != entry['uncompressed_sha256']:
        raise ValueError(f"original identity mismatch: {entry['original']}")
    json.loads(raw)
    return raw


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    commands = parser.add_subparsers(dest='command')
    commands.add_parser('verify', help='check all indexed compressed and original identities')
    extract = commands.add_parser('extract', help='verify one receipt and write its original bytes')
    extract.add_argument('original', help='original path relative to this evidence directory')
    extract.add_argument('--output', required=True, type=Path, help='new output file; existing files are refused')
    args = parser.parse_args()
    try:
        index = json.loads((ROOT / 'storage-index.json').read_bytes())
        entries = index['entries']
        originals = [entry['original'] for entry in entries]
        stored = [entry['stored'] for entry in entries]
        if len(set(originals)) != len(entries) or len(set(stored)) != len(entries):
            raise ValueError('duplicate receipt mapping')
        if args.command == 'extract':
            matches = [entry for entry in entries if entry['original'] == args.original]
            if len(matches) != 1:
                raise ValueError(f'unknown original receipt: {args.original!r}')
            raw = read_receipt(matches[0])
            with args.output.open('xb') as output:
                output.write(raw)
            print(f'Extracted {args.original} to {args.output}')
        else:
            for entry in entries:
                read_receipt(entry)
            print(f'Verified {len(entries)} receipts; compressed and original byte identities match.')
    except (OSError, ValueError, KeyError, EOFError) as error:
        parser.exit(1, f'{error}\n')


if __name__ == '__main__':
    main()
