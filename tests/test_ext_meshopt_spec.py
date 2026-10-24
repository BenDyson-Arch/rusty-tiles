"""The independent EXT_meshopt_compression decoder against the pinned upstream vectors."""
import pathlib
import random
import re
import unittest

import ext_meshopt_spec as spec

PINNED = (pathlib.Path(__file__).resolve().parents[1] / 'bench/architecture_audit/c1_payload_integrity'
          / 'primary_controls/2026-10-10-pinned-reference-verification/meshopt_decoder.test.js')


def vectors():
    """Upstream (zeux/meshoptimizer js/meshopt_decoder.test.js) cases: name -> (stream, expected, count, stride)."""
    text = PINNED.read_text()
    found = {}
    for match in re.finditer(r'\n\t(\w+): function \(\) \{(.*?)\n\t\}', text, re.S):
        name, body = match.groups()
        arrays = re.findall(r'new Uint(8|16|32)Array\(\[(.*?)\]\)', body, re.S)
        call = re.search(r'decoder\.decode\w+\([^,]+, (\d+), (\d+), encoded\)', body)
        if len(arrays) < 2 or not call or 'filter' in body:
            continue
        numbers = lambda s: [int(v, 0) for v in re.findall(r'0x[0-9a-f]+|\d+', s)]
        (_, stream), (width, expected) = arrays[0], arrays[1]
        size = int(width) // 8
        expected = b''.join(v.to_bytes(size, 'little') for v in numbers(expected))
        found[name] = bytes(numbers(stream)), expected, int(call.group(1)), int(call.group(2))
    return found


class SpecDecoderTests(unittest.TestCase):
    def test_upstream_attribute_and_triangle_vectors(self):
        cases = vectors()
        for name in ['decodeVertexBuffer', 'decodeVertexBuffer_More', 'decodeVertexBuffer_Mode2']:
            stream, expected, count, stride = cases[name]
            self.assertEqual(spec.decode_attributes(stream, count, stride), expected, name)
        for name in ['decodeIndexBufferV1', 'decodeIndexBufferV1_More', 'decodeIndexBufferV1_3Edges']:
            stream, expected, count, width = cases[name]
            self.assertEqual(spec.pack_indices(spec.decode_triangles(stream, count), width), expected, name)

    def test_headers_the_extension_forbids_are_rejected(self):
        cases = vectors()
        # Vertex codec v1 and index codec v0 decode with meshoptimizer but are not EXT streams.
        for name, decode in [('decodeVertexBufferV1', spec.decode_attributes), ('decodeIndexBuffer32', spec.decode_triangles)]:
            stream, _, count, stride = cases[name]
            arguments = (stream, count, stride) if decode is spec.decode_attributes else (stream, count)
            with self.assertRaises(AssertionError, msg=name):
                decode(*arguments)
        stream, _, count, stride = cases['decodeVertexBuffer']
        for bad in [stream[:-1], stream + b'\0']:
            with self.assertRaises(AssertionError):
                spec.decode_attributes(bad, count, stride)
        stream, _, count, _ = cases['decodeIndexBufferV1_More']
        with self.assertRaises(AssertionError):
            spec.decode_triangles(stream[:-1], count)

    def test_control_encoders_round_trip(self):
        generator = random.Random(7)
        for stride in [4, 8, 12, 256]:
            for count in [0, 1, 15, 16, 17, 300, 1100]:
                raw = bytes(generator.randrange(256) for _ in range(count * stride))
                self.assertEqual(spec.decode_attributes(spec.encode_attributes(raw, count, stride), count, stride), raw)
        for count in [0, 3, 300]:
            indices = [generator.randrange(70000) for _ in range(count)]
            self.assertEqual(spec.decode_triangles(spec.encode_triangles(indices), count), indices)


if __name__ == '__main__':
    unittest.main()
