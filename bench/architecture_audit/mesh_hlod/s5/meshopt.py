#!/usr/bin/env python3
"""Independent Khronos-spec decoder bridge; never imports producer codecs."""
from pathlib import Path
import runpy
import struct
import sys

spec = runpy.run_path(str(Path(__file__).resolve().parents[4] / 'tests/ext_meshopt_spec.py'))
mode, count, stride = sys.argv[1], int(sys.argv[2]), int(sys.argv[3])
assert 0 < count * stride <= 64 * 1024 * 1024, 'small oracle decode limit'
data = sys.stdin.buffer.read(64 * 1024 * 1024 + 1)
assert len(data) <= 64 * 1024 * 1024, 'small oracle stream limit'
if mode == 'ATTRIBUTES':
    result = spec['decode_attributes'](data, count, stride)
elif mode == 'TRIANGLES':
    assert stride in (2, 4), 'index stride'
    indices = spec['decode_triangles'](data, count)
    result = struct.pack('<' + ('H' if stride == 2 else 'I') * len(indices), *indices)
else:
    raise ValueError('unsupported small oracle meshopt mode')
sys.stdout.buffer.write(result)
