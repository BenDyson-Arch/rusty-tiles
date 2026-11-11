#!/usr/bin/env python3
"""Independent Khronos-spec decoder bridge; never imports producer codecs."""
from pathlib import Path
import runpy
import math
import json
import struct
import sys

spec = runpy.run_path(str(Path(__file__).resolve().parents[4] / 'tests/ext_meshopt_spec.py'))
mode, count, stride = sys.argv[1], int(sys.argv[2]), int(sys.argv[3])
filter_name = sys.argv[4] if len(sys.argv) > 4 else 'NONE'
assert 0 < count * stride <= 64 * 1024 * 1024, 'small oracle decode limit'
data = sys.stdin.buffer.read(64 * 1024 * 1024 + 1)
assert len(data) <= 64 * 1024 * 1024, 'small oracle stream limit'
if mode == 'ATTRIBUTES':
    result = spec['decode_attributes'](data, count, stride)
elif mode == 'TRIANGLES':
    assert stride in (2, 4), 'index stride'
    indices = spec['decode_triangles'](data, count)
    result = struct.pack('<' + ('H' if stride == 2 else 'I') * len(indices), *indices)
elif mode == 'INDICES':
    # Appendix A, mode 2: two independent wrapping uint32 baselines.
    assert stride in (2, 4) and data[:1] == b'\xd1' and data[-4:] == bytes(4), 'index sequence framing'
    at, last, indices = 1, [0, 0], []
    for _ in range(count):
        value, shift = 0, 0
        while True:
            assert at < len(data)-4 and shift < 35, 'index sequence varint'
            b = data[at]; at += 1
            value |= (b & 127) << shift; shift += 7
            if b < 128: break
        assert value <= 0xffffffff, 'index sequence overflow'
        baseline = value & 1
        delta = ~(value >> 2) if value & 2 else value >> 2
        last[baseline] = (last[baseline] + delta) & 0xffffffff
        indices.append(last[baseline])
    assert at == len(data)-4, 'index sequence length'
    result = struct.pack('<' + ('H' if stride == 2 else 'I') * count, *indices)
else:
    raise ValueError('unsupported small oracle meshopt mode')

# Independent Appendix B decoding from the retained Khronos specification
# (5ccac1cb); no meshoptimizer implementation or producer filter is imported.
# The specification allows one output ULP variation for floating operations.
def rounded(x):
    return int(math.copysign(math.floor(abs(x) + .5), x))

tolerances = {}
if filter_name != 'NONE':
    assert mode == 'ATTRIBUTES', 'filter requires attributes'
    result = bytearray(result)
    if filter_name == 'OCTAHEDRAL':
        assert stride in (4, 8), 'octahedral stride'
        fmt, maximum = ('<4b', 127) if stride == 4 else ('<4h', 32767)
        error = 0
        for at in range(0, len(result), stride):
            x, y, one, w = struct.unpack_from(fmt, result, at)
            assert one > 0 and one & (one+1) == 0 and abs(x) <= one and abs(y) <= one, 'octahedral input'
            # The normalized octahedral map is globally 3-Lipschitz in
            # the two grid coordinates: each piece has Jacobian norm <=
            # sqrt(3), and the unnormalized vector length is >=1/sqrt(3).
            # Rounding two coordinates contributes <=3*sqrt(2)/(2*one).
            # Three output components add <=sqrt(3)*1.5/maximum (rounding
            # plus the specification's one-ULP decoder allowance).
            error = max(error, 3/one + 3/maximum)
            x, y = x/one, y/one
            z = 1-abs(x)-abs(y); t = min(z, 0)
            x -= math.copysign(t, x); y -= math.copysign(t, y)
            length = math.sqrt(x*x+y*y+z*z)
            struct.pack_into(fmt, result, at, *(rounded(v/length*maximum) for v in (x,y,z)), w)
        tolerances['normalError'] = error
    elif filter_name == 'QUATERNION':
        assert stride == 8, 'quaternion stride'
        for at in range(0, len(result), stride):
            a,b,c,tag = struct.unpack_from('<4h', result, at)
            one = tag | 3
            assert one >= 7 and one & (one+1) == 0, 'quaternion input'
            x,y,z = (v/one/math.sqrt(2) for v in (a,b,c))
            w = math.sqrt(max(0,1-x*x-y*y-z*z)); out = [0]*4
            for k,v in enumerate((w,x,y,z)):out[((tag & 3)+k)%4] = rounded(v*32767)
            struct.pack_into('<4h', result, at, *out)
    elif filter_name == 'EXPONENTIAL':
        assert stride % 4 == 0, 'exponential stride'
        errors = [0.] * (stride//4)
        for at in range(0, len(result), 4):
            code = struct.unpack_from('<i', result, at)[0]
            exponent = code >> 24
            mantissa = (code & 0xffffff) - (0x1000000 if code & 0x800000 else 0)
            assert -100 <= exponent <= 100, 'exponential input'
            errors[(at % stride)//4] = max(errors[(at % stride)//4], math.ldexp(.5, exponent))
            struct.pack_into('<f', result, at, math.ldexp(mantissa, exponent))
        tolerances['componentError'] = errors
    else:
        raise ValueError('unsupported oracle meshopt filter')
if len(sys.argv)>5 and sys.argv[5]=='--tolerances':
    # Keep stdout exactly the decoded bytes; optional out-of-band precision
    # information is independently derived from the pre-filter codes above.
    sys.stderr.write(json.dumps(tolerances))
sys.stdout.buffer.write(result)
