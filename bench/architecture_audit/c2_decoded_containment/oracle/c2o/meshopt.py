"""NONE-filter EXT_meshopt_compression decoder, written from the pinned spec
primary/EXT_meshopt_compression.md (sha256 602608cb...), Appendix A.

Modes: ATTRIBUTES (header 0xa0), TRIANGLES (header 0xe1), INDICES (header 0xd1).
Filters other than NONE are not implemented (C1 refuses them).
Every malformed stream raises MeshoptError; nothing is guessed.
"""

import struct


class MeshoptError(ValueError):
    pass


def _dezigzag8(v):
    return (~(v >> 1)) & 0xFF if v & 1 else v >> 1


# ---------------------------------------------------------------- Mode 0

def decode_attributes(data, count, stride):
    """Return count*stride decoded bytes."""
    data = bytes(data)
    if stride <= 0 or stride % 4 != 0 or stride > 256:
        raise MeshoptError("ATTRIBUTES byteStride must be a positive multiple of 4 <= 256")
    if not data or data[0] != 0xA0:
        raise MeshoptError("ATTRIBUTES header byte is not 0xa0")
    # Tail block: a baseline element stored verbatim, padded to 32 bytes. The
    # element occupies the LAST `stride` bytes of the tail (see README, "Spec
    # readings"); the padding precedes it.
    tail = max(32, stride)
    if len(data) < 1 + tail:
        raise MeshoptError("ATTRIBUTES stream shorter than header plus tail")
    end = len(data) - tail
    last = bytearray(data[len(data) - stride:])
    out = bytearray(count * stride)
    max_block = min((8192 // stride) & ~15, 256)
    pos = 1
    done = 0
    while done < count:
        n = min(count - done, max_block)
        groups = (n + 15) // 16
        for k in range(stride):
            header_len = (groups + 3) // 4
            if pos + header_len > end:
                raise MeshoptError("ATTRIBUTES data block header overruns the stream")
            header = data[pos:pos + header_len]
            pos += header_len
            deltas = []
            for g in range(groups):
                mode = (header[g // 4] >> ((g % 4) * 2)) & 3
                group, pos = _attribute_group(data, pos, end, mode)
                deltas.extend(group)
            value = last[k]
            for e in range(n):
                value = (value + _dezigzag8(deltas[e])) & 0xFF
                out[(done + e) * stride + k] = value
            last[k] = value
        done += n
    if pos != end:
        raise MeshoptError("ATTRIBUTES stream has %d unread bytes before the tail" % (end - pos))
    return bytes(out)


def _attribute_group(data, pos, end, mode):
    """Decode one group of 16 zigzag-encoded byte deltas."""
    if mode == 0:
        return [0] * 16, pos
    if mode == 3:
        if pos + 16 > end:
            raise MeshoptError("ATTRIBUTES raw group overruns the stream")
        return list(data[pos:pos + 16]), pos + 16
    bits = 2 if mode == 1 else 4
    nbytes = 16 * bits // 8
    if pos + nbytes > end:
        raise MeshoptError("ATTRIBUTES bit group overruns the stream")
    packed = data[pos:pos + nbytes]
    pos += nbytes
    per_byte = 8 // bits
    sentinel = (1 << bits) - 1
    values = []
    for i in range(16):
        byte = packed[i // per_byte]
        # Most significant field first: delta0 is in the top bits.
        shift = 8 - bits * (i % per_byte + 1)
        values.append((byte >> shift) & sentinel)
    for i in range(16):
        if values[i] == sentinel:
            if pos >= end:
                raise MeshoptError("ATTRIBUTES sentinel byte overruns the stream")
            values[i] = data[pos]
            pos += 1
    return values, pos


# ---------------------------------------------------------------- varint

def _varint(data, pos, end):
    value = 0
    for i in range(5):
        if pos >= end:
            raise MeshoptError("varint overruns the stream")
        byte = data[pos]
        pos += 1
        value |= (byte & 0x7F) << (7 * i)
        if not byte & 0x80:
            if value >> 32:
                raise MeshoptError("varint exceeds 32 bits")
            return value, pos
    raise MeshoptError("varint longer than 5 bytes")


def _int32(v):
    v &= 0xFFFFFFFF
    return v - (1 << 32) if v >> 31 else v


# ---------------------------------------------------------------- Mode 1

class _Fifo:
    """16-entry FIFO; index 0 is the most recently pushed entry."""

    def __init__(self, what):
        self.items = []
        self.what = what

    def push(self, item):
        self.items.insert(0, item)
        del self.items[16:]

    def get(self, index):
        if index >= len(self.items):
            raise MeshoptError("%s FIFO read of an undefined entry %d" % (self.what, index))
        return self.items[index]


def decode_triangles(data, count):
    """Return a list of `count` uint32 indices."""
    data = bytes(data)
    if count % 3 != 0:
        raise MeshoptError("TRIANGLES count is not divisible by 3")
    if not data or data[0] != 0xE1:
        raise MeshoptError("TRIANGLES header byte is not 0xe1")
    triangles = count // 3
    if len(data) < 1 + triangles + 16:
        raise MeshoptError("TRIANGLES stream shorter than header, codes and tail")
    codes = data[1:1 + triangles]
    end = len(data) - 16
    codeaux = data[end:]
    if codeaux[14] != 0 or codeaux[15] != 0:
        raise MeshoptError("TRIANGLES codeaux table: last two bytes must be 0")
    for b in codeaux[:14]:
        if b >> 4 == 0xF or b & 0xF == 0xF:
            raise MeshoptError("TRIANGLES codeaux table contains a 0xf nibble")
    pos = 1 + triangles
    state = {"next": 0, "last": 0}
    edges = _Fifo("edge")
    verts = _Fifo("vertex")
    out = []

    def decode_index(p):
        v, p = _varint(data, p, end)
        delta = _int32(~(v >> 1)) if v & 1 else (v >> 1)
        state["last"] = (state["last"] + delta) & 0xFFFFFFFF
        return state["last"], p

    def take_next():
        value = state["next"]
        state["next"] = (value + 1) & 0xFFFFFFFF
        return value

    for code in codes:
        hi, lo = code >> 4, code & 0xF
        if hi < 0xF:
            a, b = edges.get(hi)
            if lo == 0:
                c = take_next()
                push_c = True
            elif lo < 0xD:
                c = verts.get(lo)
                push_c = False
            elif lo == 0xD or lo == 0xE:
                c = (state["last"] + (-1 if lo == 0xD else 1)) & 0xFFFFFFFF
                state["last"] = c
                push_c = True
            else:
                c, pos = decode_index(pos)
                push_c = True
            out += [a, b, c]
            edges.push((c, b))
            edges.push((a, c))
            if push_c:
                verts.push(c)
        elif lo < 0xE:
            zw = codeaux[lo]
            z, w = zw >> 4, zw & 0xF
            a = take_next()
            b = take_next() if z == 0 else verts.get(z - 1)
            c = take_next() if w == 0 else verts.get(w - 1)
            out += [a, b, c]
            verts.push(a)
            if z == 0:
                verts.push(b)
            if w == 0:
                verts.push(c)
            edges.push((b, a))
            edges.push((c, b))
            edges.push((a, c))
        else:
            if pos >= end:
                raise MeshoptError("TRIANGLES extra byte overruns the data")
            zw = data[pos]
            pos += 1
            if zw == 0:
                state["next"] = 0
            z, w = zw >> 4, zw & 0xF
            if lo == 0xE:
                a = take_next()
            else:
                a, pos = decode_index(pos)
            if z == 0:
                b = take_next()
            elif z < 0xF:
                b = verts.get(z - 1)
            else:
                b, pos = decode_index(pos)
            if w == 0:
                c = take_next()
            elif w < 0xF:
                c = verts.get(w - 1)
            else:
                c, pos = decode_index(pos)
            out += [a, b, c]
            verts.push(a)
            if z == 0 or z == 0xF:
                verts.push(b)
            if w == 0 or w == 0xF:
                verts.push(c)
            edges.push((b, a))
            edges.push((c, b))
            edges.push((a, c))
    if pos != end:
        raise MeshoptError("TRIANGLES data has %d unread bytes" % (end - pos))
    return out


# ---------------------------------------------------------------- Mode 2

def decode_indices(data, count):
    data = bytes(data)
    if not data or data[0] != 0xD1:
        raise MeshoptError("INDICES header byte is not 0xd1")
    if len(data) < 1 + 4:
        raise MeshoptError("INDICES stream shorter than header plus tail")
    end = len(data) - 4
    last = [0, 0]
    pos = 1
    out = []
    for _ in range(count):
        v, pos = _varint(data, pos, end)
        baseline = v & 1
        delta = _int32(~(v >> 2)) if v & 2 else (v >> 2)
        last[baseline] = (last[baseline] + delta) & 0xFFFFFFFF
        out.append(last[baseline])
    if pos != end:
        raise MeshoptError("INDICES stream has %d unread bytes before the tail" % (end - pos))
    return out


# ---------------------------------------------------------------- view level

def decode_view(ext, parent_view, source):
    """Decode one compressed bufferView. `source` is the bytes of ext.buffer.

    Returns parent_view.byteLength bytes."""
    try:
        count = ext["count"]
        stride = ext["byteStride"]
        mode = ext["mode"]
        length = ext["byteLength"]
    except (KeyError, TypeError):
        raise MeshoptError("EXT_meshopt_compression object lacks a required property")
    offset = ext.get("byteOffset", 0)
    flt = ext.get("filter", "NONE")
    for value in (count, stride, length, offset):
        if not isinstance(value, int) or isinstance(value, bool) or value < 0:
            raise MeshoptError("EXT_meshopt_compression integer property is invalid")
    if flt != "NONE":
        raise MeshoptError("filter %r is not NONE (C1 Unsupported)" % (flt,))
    if "byteStride" in parent_view and parent_view["byteStride"] != stride:
        raise MeshoptError("parent byteStride differs from the extension byteStride")
    if parent_view.get("byteLength") != stride * count:
        raise MeshoptError("parent byteLength is not byteStride * count")
    if offset + length > len(source):
        raise MeshoptError("compressed range overruns its buffer")
    blob = source[offset:offset + length]
    if mode == "ATTRIBUTES":
        return decode_attributes(blob, count, stride)
    if stride not in (2, 4):
        raise MeshoptError("index modes need byteStride 2 or 4")
    if mode == "TRIANGLES":
        values = decode_triangles(blob, count)
    elif mode == "INDICES":
        values = decode_indices(blob, count)
    else:
        raise MeshoptError("unknown mode %r" % (mode,))
    fmt = "<%d%s" % (count, "H" if stride == 2 else "I")
    mask = 0xFFFF if stride == 2 else 0xFFFFFFFF
    return struct.pack(fmt, *[v & mask for v in values])
