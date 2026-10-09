"""Prepare preview mounts from the Rust convert_implicit fixture exports.

Usage: python3 prepare_convert_implicit_preview.py EXPORTED_FIXTURES NEW_MOUNTS
The source directories remain untouched. Uses only the Python standard library.
"""
import argparse
import json
import pathlib
import shutil


def prefix(value):
    if isinstance(value, dict):
        for key, item in value.items():
            if key == 'uri' and isinstance(item, str):
                value[key] = 'explicit/' + item
            else:
                prefix(item)
    elif isinstance(value, list):
        for item in value:
            prefix(item)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('fixtures', type=pathlib.Path)
    parser.add_argument('mounts', type=pathlib.Path)
    args = parser.parse_args()
    args.mounts.mkdir(parents=True, exist_ok=False)
    for kind in ['point', 'vector']:
        mount = args.mounts / kind
        mount.mkdir()
        for variant in ['explicit', 'converted', 'fresh']:
            shutil.copytree(args.fixtures / f'{kind}-{variant}', mount / variant)
        manifest = json.loads((mount / 'explicit/tileset.json').read_text())
        prefix(manifest)
        (mount / 'tileset.json').write_text(json.dumps(manifest))


if __name__ == '__main__':
    main()
