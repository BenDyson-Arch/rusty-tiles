"""Single-tile stress 3TZ archives at an exact per-document or per-member size (stdlib only).

Usage:
  python3 -I stress_single.py OUT.3tz --items N [--shape array|object]
  python3 -I stress_single.py OUT.3tz --member-bytes N
  python3 -I stress_single.py OUT.3tz --decoded-bytes N

--items: tileset.json has exactly N JSON value nodes (C1 counting: the root plus
  every scalar/container value; object keys are not nodes). `array` pads with a
  list of zeros; `object` pads with one object of distinct 40-byte keys, the
  larger in-memory shape per node. Both stay below the 8 MiB JSON byte cap.
--member-bytes: the single GLB member is exactly N bytes; its BIN chunk carries
  the in-box triangle followed by unreferenced zero bytes.
--decoded-bytes: the GLB's bufferViews total exactly N bytes: a 256-759 byte
  triangle view plus two unreferenced EXT_meshopt_compression ATTRIBUTES views
  (stride 252, the glTF maximum; filter NONE) over the same fallback range, each decoding a
  version-1 stream that zero-encodes every byte lane (~N/256 compressed bytes).
  Overlap keeps declared buffers (fallback + BIN) near N/2, under the member cap.
All archives otherwise match stress.py (one box, one in-box triangle).
"""
import argparse
import json
from pathlib import Path
import struct
import sys

sys.path.insert(0, str(Path(__file__).resolve().parent))  # -I omits the script directory
from stress import BOX, document, glb, tile, write  # noqa: E402


def nodes(value):
    count, stack = 0, [value]
    while stack:
        item = stack.pop()
        count += 1
        stack.extend(item.values() if isinstance(item, dict) else item if isinstance(item, list) else ())
    return count


def padded_glb(total):
    positions = struct.pack('<9f', -1, -1, -1, 1, -1, -1, 0, 1, 1)

    def build(binary):
        doc = {'asset': {'version': '2.0'}, 'scene': 0, 'scenes': [{'nodes': [0]}], 'nodes': [{'mesh': 0}],
               'meshes': [{'primitives': [{'attributes': {'POSITION': 0}}]}],
               'accessors': [{'bufferView': 0, 'componentType': 5126, 'count': 3, 'type': 'VEC3',
                              'min': [-1, -1, -1], 'max': [1, 1, 1]}],
               'bufferViews': [{'buffer': 0, 'byteOffset': 0, 'byteLength': len(positions)}],
               'buffers': [{'byteLength': binary}]}
        encoded = json.dumps(doc, separators=(',', ':')).encode()
        return encoded + b' ' * (-len(encoded) % 4)

    encoded = build(0)
    for _ in range(3):  # byteLength digits change the JSON length; settle it
        binary = total - 28 - len(encoded)
        encoded = build(binary)
    binary = total - 28 - len(encoded)
    assert binary % 4 == 0 and binary >= len(positions), binary
    return (struct.pack('<III', 0x46546C67, 2, total) + struct.pack('<II', len(encoded), 0x4E4F534A) + encoded
            + struct.pack('<II', binary, 0x004E4942) + positions + bytes(binary - len(positions)))


def meshopt_zero_stream(count, stride):
    """meshoptimizer vertex codec v1: per block (8192 // stride vertices, at most
    256) one control byte per 4-byte lane group, 0b10 per lane = zero encoding
    (no data); tail = last vertex plus channel bytes, left-padded to the 24-byte
    v1 minimum."""
    block = min(8192 // stride & ~15, 256)
    tail = bytes(stride) + bytes(stride // 4)
    return (bytes([0xa1]) + bytes([0xAA] * (stride // 4)) * -(-count // block)
            + bytes(max(0, 24 - len(tail))) + tail)


def decoded_glb(total, stride=252):
    # Two meshopt views decode over the same placeholder range: the decoded-view
    # sum reaches `total` while declared buffers stay within the member cap.
    count = (total - 256) // (2 * stride)
    head = total - 2 * stride * count  # first view: triangle positions plus zero tail
    positions = struct.pack('<9f', -1, -1, -1, 1, -1, -1, 0, 1, 1) + bytes(head - 36)
    stream = meshopt_zero_stream(count, stride)
    binary = positions + stream + bytes(-len(stream) % 4)
    ext = 'EXT_meshopt_compression'
    compressed = {ext: {'buffer': 0, 'byteOffset': head, 'byteLength': len(stream), 'byteStride': stride,
                        'count': count, 'mode': 'ATTRIBUTES', 'filter': 'NONE'}}
    view = {'buffer': 1, 'byteOffset': 0, 'byteLength': stride * count, 'byteStride': stride, 'extensions': compressed}
    doc = {'asset': {'version': '2.0'}, 'scene': 0, 'scenes': [{'nodes': [0]}], 'nodes': [{'mesh': 0}],
           'extensionsUsed': [ext], 'extensionsRequired': [ext],
           'meshes': [{'primitives': [{'attributes': {'POSITION': 0}}]}],
           'accessors': [{'bufferView': 0, 'componentType': 5126, 'count': 3, 'type': 'VEC3',
                          'min': [-1, -1, -1], 'max': [1, 1, 1]}],
           'bufferViews': [{'buffer': 0, 'byteOffset': 0, 'byteLength': head}, view, view],
           'buffers': [{'byteLength': len(binary)}, {'byteLength': stride * count, 'extensions': {ext: {'fallback': True}}}]}
    assert head + 2 * stride * count == total
    encoded = json.dumps(doc, separators=(',', ':')).encode()
    encoded += b' ' * (-len(encoded) % 4)
    return (struct.pack('<III', 0x46546C67, 2, 28 + len(encoded) + len(binary))
            + struct.pack('<II', len(encoded), 0x4E4F534A) + encoded + struct.pack('<II', len(binary), 0x004E4942) + binary)


def main():
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument('out')
    p.add_argument('--items', type=int)
    p.add_argument('--shape', choices=['array', 'object'], default='array')
    p.add_argument('--member-bytes', type=int)
    p.add_argument('--decoded-bytes', type=int)
    a = p.parse_args()
    if a.items:
        root = tile(('g/0.glb',))
        doc = {'asset': {'version': '1.1'}, 'geometricError': 0, 'root': root, 'extras': {'pad': None}}
        fixed = nodes(doc) - 1  # without the pad value itself
        pad = a.items - fixed - 1  # children of the pad container
        doc['extras']['pad'] = [0] * pad if a.shape == 'array' else {f'k{i:038d}': 0 for i in range(pad)}
        raw = json.dumps(doc, separators=(',', ':')).encode()
        assert nodes(json.loads(raw)) == a.items
        n, cd = write(a.out, [('tileset.json', raw), ('g/0.glb', glb())])
        print(json.dumps({'valueNodes': a.items, 'shape': a.shape, 'tilesetJsonBytes': len(raw), 'entries': n}))
    elif a.decoded_bytes:
        data = decoded_glb(a.decoded_bytes)
        n, cd = write(a.out, [('tileset.json', document(tile(('g/0.glb',)))), ('g/0.glb', data)])
        print(json.dumps({'decodedViewBytes': a.decoded_bytes, 'memberBytes': len(data), 'entries': n}))
    else:
        data = padded_glb(a.member_bytes)
        assert len(data) == a.member_bytes
        n, cd = write(a.out, [('tileset.json', document(tile(('g/0.glb',)))), ('g/0.glb', data)])
        print(json.dumps({'memberBytes': len(data), 'entries': n, 'box': BOX}))


if __name__ == '__main__':
    main()
