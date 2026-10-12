"""Independent EXT_meshopt_compression bitstream codec for test oracles.

Written from the pinned extension text alone (KhronosGroup/glTF 5ccac1cb,
``EXT_meshopt_compression`` Appendix A, retained under
``bench/architecture_audit/c1_payload_integrity/primary_controls/``); it shares
no code with meshoptimizer. Only what terrain tiles use is supported: mode 0
(attributes, header ``0xa0``) and mode 1 (triangles, header ``0xe1``), filter
NONE. The decoders reject any other header byte, short or over-long streams
and malformed lookup tables.

The encoders are deliberately trivial (every attribute group stored as raw
byte deltas; every triangle as three explicit indices). They exist so
corruption controls can re-store edited views as valid streams; production
streams exercise the full decoders.
"""
import struct

ATTRIBUTES_HEADER = 0xa0
TRIANGLES_HEADER = 0xe1


def _unzigzag8(v):
    return (~(v >> 1)) & 0xff if v & 1 else v >> 1


def _zigzag8(delta):
    delta &= 0xff
    return (~(delta << 1)) & 0xff if delta & 0x80 else (delta << 1) & 0xff


def _block_elements(stride):
    return min((8192 // stride) & ~15, 256)


def decode_attributes(data, count, stride):
    """Mode 0: ``count`` elements of ``stride`` bytes."""
    assert 0 < stride <= 256 and stride % 4 == 0, 'attribute stride'
    assert data and data[0] == ATTRIBUTES_HEADER, 'attribute stream header'
    tail = max(32, stride)
    end = len(data) - tail
    assert end >= 1, 'attribute stream tail'
    # The baseline element is stored verbatim at the end of the tail block.
    last = list(data[len(data) - stride:])
    out = bytearray(count * stride)
    at, element = 1, 0
    while element < count:
        block = min(count - element, _block_elements(stride))
        groups = (block + 15) // 16
        for k in range(stride):
            header = data[at:at + (groups + 3) // 4]
            at += (groups + 3) // 4
            assert at <= end, 'attribute header past stream'
            deltas = []
            for group in range(groups):
                bits = (header[group // 4] >> (2 * (group % 4))) & 3
                if bits == 0:
                    deltas += [0] * 16
                elif bits == 3:
                    assert at + 16 <= end, 'attribute bytes past stream'
                    deltas += data[at:at + 16]
                    at += 16
                else:
                    width = 2 if bits == 1 else 4
                    per_byte, sentinel = 8 // width, (1 << width) - 1
                    packed = data[at:at + 2 * width]
                    at += 2 * width
                    assert at <= end, 'attribute deltas past stream'
                    for i in range(16):
                        # Deltas are packed from the most significant bits down.
                        value = (packed[i // per_byte] >> (8 - width * (i % per_byte + 1))) & sentinel
                        if value == sentinel:
                            assert at < end, 'attribute escape past stream'
                            value = data[at]
                            at += 1
                        deltas.append(value)
            value = last[k]
            for i in range(block):
                value = (value + _unzigzag8(deltas[i])) & 0xff
                out[(element + i) * stride + k] = value
            last[k] = value
        element += block
    assert at == end, 'attribute stream length'
    return bytes(out)


def decode_triangles(data, count):
    """Mode 1: ``count`` triangle-list indices."""
    assert count % 3 == 0, 'triangle index count'
    assert data and data[0] == TRIANGLES_HEADER, 'triangle stream header'
    end = len(data) - 16
    assert end >= 1 + count // 3, 'triangle stream tail'
    codeaux = data[end:]
    assert codeaux[14] == codeaux[15] == 0, 'codeaux tail bytes'
    assert all(b >> 4 != 15 and b & 15 != 15 for b in codeaux[:14]), 'codeaux nibble'
    edges, vertices = [(0, 0)] * 16, [0] * 16
    state = dict(edge=0, vertex=0, next=0, last=0, at=1 + count // 3)

    def push_edge(a, b):
        edges[state['edge'] & 15] = (a, b)
        state['edge'] += 1

    def push_vertex(v):
        vertices[state['vertex'] & 15] = v
        state['vertex'] += 1

    def recent_vertex(index):
        return vertices[(state['vertex'] - 1 - index) & 15]

    def take_next():
        state['next'] += 1
        return state['next'] - 1

    def byte():
        assert state['at'] < end, 'triangle data past stream'
        state['at'] += 1
        return data[state['at'] - 1]

    def decode_index():
        value, shift = 0, 0
        while True:
            b = byte()
            value |= (b & 0x7f) << shift
            shift += 7
            assert shift <= 35, 'index varint exceeds 32 bits'
            if b < 0x80:
                break
        delta = ~(value >> 1) if value & 1 else value >> 1
        state['last'] = (state['last'] + delta) & 0xffffffff
        return state['last']

    out = []
    for triangle in range(count // 3):
        code = data[1 + triangle]
        x, y = code >> 4, code & 15
        if x < 15:
            a, b = edges[(state['edge'] - 1 - x) & 15]
            if y == 0:
                c = take_next()
                push_vertex(c)
            elif y < 13:
                c = recent_vertex(y)
            elif y < 15:
                c = (state['last'] + (-1 if y == 13 else 1)) & 0xffffffff
                state['last'] = c
                push_vertex(c)
            else:
                c = decode_index()
                push_vertex(c)
            push_edge(c, b)
            push_edge(a, c)
        else:
            if y < 14:
                z, w = codeaux[y] >> 4, codeaux[y] & 15
                a = take_next()
                b = take_next() if z == 0 else recent_vertex(z - 1)
                c = take_next() if w == 0 else recent_vertex(w - 1)
                explicit = (False, False)
            else:
                zw = byte()
                if zw == 0:
                    state['next'] = 0
                z, w = zw >> 4, zw & 15
                a = take_next() if y == 14 else decode_index()
                b = take_next() if z == 0 else decode_index() if z == 15 else recent_vertex(z - 1)
                c = take_next() if w == 0 else decode_index() if w == 15 else recent_vertex(w - 1)
                explicit = (z == 15, w == 15)
            push_edge(b, a)
            push_edge(c, b)
            push_edge(a, c)
            push_vertex(a)
            if z == 0 or explicit[0]:
                push_vertex(b)
            if w == 0 or explicit[1]:
                push_vertex(c)
        out += [a, b, c]
    assert state['at'] == end, 'triangle stream length'
    return out


def encode_attributes(raw, count, stride):
    """A valid mode 0 stream storing every group as raw byte deltas."""
    assert len(raw) == count * stride and stride % 4 == 0
    out = bytearray([ATTRIBUTES_HEADER])
    baseline = bytes(raw[:stride]) if count else bytes(stride)
    last = list(baseline)
    element = 0
    while element < count:
        block = min(count - element, _block_elements(stride))
        groups = (block + 15) // 16
        for k in range(stride):
            header = bytearray((groups + 3) // 4)
            for group in range(groups):
                header[group // 4] |= 3 << (2 * (group % 4))
            out += header
            deltas = []
            for i in range(groups * 16):
                value = raw[(element + i) * stride + k] if i < block else last[k]
                deltas.append(_zigzag8(value - last[k]))
                last[k] = value
            out += bytes(deltas)
        element += block
    out += bytes(max(32, stride) - stride) + baseline
    return bytes(out)


def encode_triangles(indices):
    """A valid mode 1 stream: each triangle is code 0xff, aux byte 0xff and three
    zigzag delta varints."""
    assert len(indices) % 3 == 0
    codes, body, last = bytearray(), bytearray(), 0
    for start in range(0, len(indices), 3):
        codes.append(0xff)
        body.append(0xff)
        for index in indices[start:start + 3]:
            delta = (index - last + 0x80000000) % 0x100000000 - 0x80000000
            value = ((delta << 1) ^ (delta >> 31)) & 0xffffffff
            last = index
            while value > 0x7f:
                body.append(value & 0x7f | 0x80)
                value >>= 7
            body.append(value)
    return bytes([TRIANGLES_HEADER]) + bytes(codes) + bytes(body) + bytes(16)


def unpack_indices(raw, width):
    return list(struct.unpack('<%d%s' % (len(raw) // width, 'H' if width == 2 else 'I'), raw))


def pack_indices(indices, width):
    return struct.pack('<%d%s' % (len(indices), 'H' if width == 2 else 'I'), *indices)
