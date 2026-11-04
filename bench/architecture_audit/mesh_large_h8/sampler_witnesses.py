#!/usr/bin/env python3
"""Produce small source-bound colour-gamma and actual four-filter witnesses."""
import argparse
import hashlib
import json
from pathlib import Path
import shutil
from leaf_oracle import png


def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def generate(source, output, kind):
    output.mkdir(parents=True, exist_ok=False)
    truth = json.loads((source / 'truth.json').read_text())
    assert digest(source / 'source.gltf') == truth['inputSha256']
    for name, identity in truth['assets'].items():
        path = (source / name).resolve()
        assert path.is_relative_to(source.resolve()) and digest(path) == identity
        shutil.copyfile(path, output / name)
    document = json.loads((source / 'source.gltf').read_text())
    if kind == 'gamma':
        for role in (0, 4):
            image = truth['images'][role]
            values = []
            for y in range(image['height']):
                for x in range(image['width']):
                    value = 255 if (x + y) % 2 else 0
                    values.extend((value, value, value, 255))
            image['rgba'] = values
            (output / image['uri']).write_bytes(png(image['width'], image['height'], values))
        truth['signalScope'] = 'opaque black/white colour and emissive; source-class mip1 is 188, wrong byte-box mip1 is 128'
    else:
        original = document['samplers']
        document['samplers'] = []
        for index, texture in enumerate(document['textures']):
            sampler = dict(original[texture['sampler']])
            sampler['minFilter'] = 9984 + index % 4
            sampler['magFilter'] = 9729 if index % 2 == 0 else 9728
            texture['sampler'] = index
            document['samplers'].append(sampler)
        truth['textures'] = document['textures']
        truth['samplers'] = document['samplers']
        truth['signalScope'] = 'actual four mip filter enums and both mag enums; inherited opaque POT signal'
    (output / 'source.gltf').write_text(json.dumps(document, indent=2) + '\n')
    truth['inputSha256'] = digest(output / 'source.gltf')
    truth['assets'] = {name: digest(output / name) for name in truth['assets']}
    truth['witnessGeneratorSha256'] = digest(Path(__file__))
    (output / 'truth.json').write_text(json.dumps(truth, indent=2) + '\n')
    print(json.dumps({'source': str(output / 'source.gltf'), 'truthSha256': digest(output / 'truth.json'), 'kind': kind}))


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('source', type=Path)
    parser.add_argument('output', type=Path)
    parser.add_argument('--kind', choices=('gamma', 'filters'), required=True)
    args = parser.parse_args()
    generate(args.source, args.output, args.kind)
