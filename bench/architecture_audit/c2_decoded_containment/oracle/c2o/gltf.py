"""GLB / glTF JSON / b3dm payload reading and accessor decoding.

This is not a C1 implementation. Where the oracle meets data that C1 is
expected to reject (bad lengths, out-of-range views, unknown component types)
it raises OracleUndecided rather than inventing a verdict.
"""

import array
import base64
import posixpath
import struct
import sys
from fractions import Fraction
from urllib.parse import unquote

from . import meshopt
from .numbers import F32_SCALE, OracleUndecided, f32_scaled, is_index, load_json

COMPONENTS = {
    5120: ("b", 1, "i8"),
    5121: ("B", 1, "u8"),
    5122: ("h", 2, "i16"),
    5123: ("H", 2, "u16"),
    5125: ("I", 4, "u32"),
    5126: ("I", 4, "f32"),  # f32 read as raw bits, converted exactly
}
TYPE_SIZE = {"SCALAR": 1, "VEC2": 2, "VEC3": 3, "VEC4": 4, "MAT2": 4, "MAT3": 9, "MAT4": 16}
NORMALIZER = {"i8": 127, "u8": 255, "i16": 32767, "u16": 65535}
RESTART = {1: 0xFF, 2: 0xFFFF, 4: 0xFFFFFFFF}

GLB_JSON = 0x4E4F534A
GLB_BIN = 0x004E4942


class PayloadError(OracleUndecided):
    pass


def resolve(base_member, uri):
    """Archive member name for a relative URI written in `base_member`."""
    if not isinstance(uri, str) or not uri:
        raise PayloadError("empty or non-string uri in %s" % base_member)
    if ":" in uri.split("/")[0] or uri.startswith("/"):
        raise PayloadError("absolute or scheme uri %r is outside the archive" % uri[:80])
    path = unquote(uri.split("#")[0].split("?")[0])
    joined = posixpath.normpath(posixpath.join(posixpath.dirname(base_member), path))
    if joined.startswith("../") or joined == "..":
        raise PayloadError("uri %r escapes the archive root" % uri[:80])
    return joined


def _data_uri(uri):
    head, sep, payload = uri.partition(",")
    if not sep or not head.endswith(";base64"):
        raise PayloadError("data uri is not base64")
    try:
        return base64.b64decode(payload, validate=True)
    except ValueError:
        raise PayloadError("data uri is not valid base64")


def split_glb(data, where):
    if len(data) < 12:
        raise PayloadError("GLB shorter than its header: %s" % where)
    magic, version, length = struct.unpack_from("<4sII", data, 0)
    if magic != b"glTF" or version != 2 or length != len(data):
        raise PayloadError("GLB header invalid (magic/version/length) in %s" % where)
    pos = 12
    chunks = []
    while pos < len(data):
        if pos + 8 > len(data):
            raise PayloadError("GLB chunk header overruns in %s" % where)
        clen, ctype = struct.unpack_from("<II", data, pos)
        if pos + 8 + clen > len(data):
            raise PayloadError("GLB chunk overruns in %s" % where)
        chunks.append((ctype, data[pos + 8:pos + 8 + clen]))
        pos += 8 + clen
    if not chunks or chunks[0][0] != GLB_JSON:
        raise PayloadError("GLB first chunk is not JSON in %s" % where)
    binary = None
    if len(chunks) > 1 and chunks[1][0] == GLB_BIN:
        binary = chunks[1][1]
    return chunks[0][1], binary


def split_b3dm(data, where):
    """Return (feature-table JSON dict, glb bytes)."""
    if len(data) < 28:
        raise PayloadError("b3dm shorter than its header: %s" % where)
    magic, version, length, ftj, ftb, btj, btb = struct.unpack_from("<4s6I", data, 0)
    if magic != b"b3dm" or version != 1 or length != len(data):
        raise PayloadError("b3dm header invalid in %s" % where)
    start = 28 + ftj + ftb + btj + btb
    if start > len(data):
        raise PayloadError("b3dm table lengths overrun in %s" % where)
    table = {}
    if ftj:
        try:
            table = load_json(bytes(data[28:28 + ftj]).rstrip(b" "))
        except (ValueError, UnicodeDecodeError) as err:
            raise PayloadError("b3dm feature table JSON unreadable in %s: %s" % (where, err))
        if not isinstance(table, dict):
            raise PayloadError("b3dm feature table JSON is not an object in %s" % where)
    return table, data[start:]


class Payload:
    """One decoded tile payload (GLB, glTF JSON or b3dm)."""

    def __init__(self, archive, member):
        self.archive = archive
        self.member = member
        data = archive.read(member)
        self.member_bytes = len(data)
        self.bytes_read = len(data)  # B(u): member plus resolved buffers/images
        self.decoded_bytes = 0  # Dec(u): owned external/data-URI buffers + meshopt views
        self.rtc_center = False
        self.binary = None
        self.kind = None
        head = bytes(data[:4])
        try:
            if head == b"b3dm":
                self.kind = "b3dm"
                table, glb = split_b3dm(data, member)
                if len(glb) >= 12:
                    # The embedded GLB may be followed by the b3dm's 8-byte padding.
                    declared = struct.unpack_from("<I", glb, 8)[0]
                    if 12 <= declared <= len(glb) and len(glb) - declared < 8:
                        glb = glb[:declared]
                self.rtc_center = "RTC_CENTER" in table
                json_chunk, self.binary = split_glb(glb, member)
                self.doc = load_json(bytes(json_chunk))
            elif head == b"glTF":
                self.kind = "glb"
                json_chunk, self.binary = split_glb(data, member)
                self.doc = load_json(bytes(json_chunk))
            elif head in (b"pnts", b"i3dm", b"cmpt", b"vctr", b"geom"):
                raise PayloadError("payload format %r is not modelled by the oracle" % head.decode())
            else:
                self.kind = "gltf"
                self.doc = load_json(bytes(data))
        except (ValueError, UnicodeDecodeError) as err:
            raise PayloadError("payload JSON unreadable in %s: %s" % (member, err))
        if not isinstance(self.doc, dict):
            raise PayloadError("glTF JSON is not an object in %s" % member)
        self._buffers = {}
        self._views = {}
        self._resolved = set()
        self._load_buffers_and_images()

    # ------------------------------------------------------------ resources

    def _read_resource(self, uri):
        name = resolve(self.member, uri)
        data = self.archive.read(name)
        if name not in self._resolved:
            self._resolved.add(name)
            self.bytes_read += len(data)
        return data

    def _load_buffers_and_images(self):
        buffers = self.doc.get("buffers", [])
        if not isinstance(buffers, list):
            raise PayloadError("buffers is not an array in %s" % self.member)
        for i, buf in enumerate(buffers):
            if not isinstance(buf, dict) or not is_index(buf.get("byteLength")):
                raise PayloadError("buffer %d malformed in %s" % (i, self.member))
            uri = buf.get("uri")
            if uri is None:
                if i == 0 and self.binary is not None:
                    self._buffers[i] = self.binary
                else:
                    self._buffers[i] = None  # placeholder (e.g. meshopt fallback)
                continue
            if uri.startswith("data:"):
                data = _data_uri(uri)
            else:
                data = self._read_resource(uri)
            self.decoded_bytes += len(data)
            self._buffers[i] = data
        for image in self.doc.get("images", []) or []:
            if isinstance(image, dict) and isinstance(image.get("uri"), str) \
                    and not image["uri"].startswith("data:"):
                self._read_resource(image["uri"])
        # Dec(u) also counts every meshopt-decoded view (model; see README).
        for vi, view in enumerate(self.doc.get("bufferViews", []) or []):
            if isinstance(view, dict) and "EXT_meshopt_compression" in (view.get("extensions") or {}):
                self.decoded_bytes += len(self.view_bytes(vi))

    def buffer(self, index):
        if index not in self._buffers:
            raise PayloadError("buffer %r does not exist in %s" % (index, self.member))
        data = self._buffers[index]
        if data is None:
            raise PayloadError("buffer %d has no data (placeholder) in %s" % (index, self.member))
        return data

    def view_bytes(self, index):
        if index in self._views:
            return self._views[index]
        views = self.doc.get("bufferViews", [])
        if not is_index(index) or index >= len(views) or not isinstance(views[index], dict):
            raise PayloadError("bufferView %r invalid in %s" % (index, self.member))
        view = views[index]
        ext = (view.get("extensions") or {}).get("EXT_meshopt_compression")
        if ext is not None:
            try:
                source = self.buffer(ext.get("buffer"))
                data = meshopt.decode_view(ext, view, source)
            except meshopt.MeshoptError as err:
                raise PayloadError("meshopt view %d in %s: %s" % (index, self.member, err))
        else:
            source = self.buffer(view.get("buffer"))
            offset = view.get("byteOffset", 0)
            length = view.get("byteLength")
            if not is_index(offset) or not is_index(length) or offset + length > len(source):
                raise PayloadError("bufferView %d range invalid in %s" % (index, self.member))
            data = bytes(source[offset:offset + length])
        self._views[index] = data
        return data

    # ------------------------------------------------------------ accessors

    def accessor(self, index):
        accessors = self.doc.get("accessors", [])
        if not is_index(index) or index >= len(accessors) or not isinstance(accessors[index], dict):
            raise PayloadError("accessor %r invalid in %s" % (index, self.member))
        return accessors[index]

    def accessor_layout(self, index):
        acc = self.accessor(index)
        ctype = acc.get("componentType")
        atype = acc.get("type")
        count = acc.get("count")
        if ctype not in COMPONENTS or atype not in TYPE_SIZE or not is_index(count):
            raise PayloadError("accessor %d has unsupported layout in %s" % (index, self.member))
        return acc, COMPONENTS[ctype], TYPE_SIZE[atype], count

    def accessor_elements(self, index):
        """count * components (the oracle's model of C1 elements for this accessor)."""
        _, _, ncomp, count = self.accessor_layout(index)
        return count * ncomp

    def read_components(self, index, element_indices=None):
        """Raw integer components (f32 as raw bits) for the given elements.

        Returns a dict element -> tuple when element_indices is given, else a list."""
        acc, (fmt, size, _), ncomp, count = self.accessor_layout(index)
        if acc.get("type", "").startswith("MAT") and size < 4:
            raise PayloadError("byte/short matrix accessors are not modelled (%s)" % self.member)
        wanted = range(count) if element_indices is None else element_indices
        elem_size = size * ncomp
        values = {}
        if "bufferView" in acc:
            data = self.view_bytes(acc["bufferView"])
            view = self.doc["bufferViews"][acc["bufferView"]]
            stride = view.get("byteStride") or elem_size
            offset = acc.get("byteOffset", 0)
            if not is_index(offset) or (count and offset + stride * (count - 1) + elem_size > len(data)):
                raise PayloadError("accessor %d overruns its bufferView in %s" % (index, self.member))
            unpack = struct.Struct("<%d%s" % (ncomp, fmt)).unpack_from
            for i in wanted:
                values[i] = unpack(data, offset + i * stride)
        else:
            zero = (0,) * ncomp
            for i in wanted:
                values[i] = zero
        sparse = acc.get("sparse")
        if sparse is not None:
            self._apply_sparse(index, acc, sparse, fmt, size, ncomp, count, values)
        if element_indices is None:
            return [values[i] for i in range(count)]
        return values

    def _apply_sparse(self, index, acc, sparse, fmt, size, ncomp, count, values):
        try:
            n = sparse["count"]
            ind = sparse["indices"]
            val = sparse["values"]
            itype = COMPONENTS[ind["componentType"]]
        except (KeyError, TypeError):
            raise PayloadError("sparse accessor %d malformed in %s" % (index, self.member))
        idata = self.view_bytes(ind["bufferView"])
        vdata = self.view_bytes(val["bufferView"])
        ioff = ind.get("byteOffset", 0)
        voff = val.get("byteOffset", 0)
        if ioff + n * itype[1] > len(idata) or voff + n * size * ncomp > len(vdata):
            raise PayloadError("sparse accessor %d overruns in %s" % (index, self.member))
        unpack = struct.Struct("<%d%s" % (ncomp, fmt)).unpack_from
        for k in range(n):
            target = struct.unpack_from("<" + itype[0], idata, ioff + k * itype[1])[0]
            if target >= count:
                raise PayloadError("sparse index out of range in %s" % self.member)
            if target in values:
                values[target] = unpack(vdata, voff + k * size * ncomp)

    def index_values(self, index):
        """All values of a SCALAR index accessor as an array of ints."""
        acc, (fmt, size, word), ncomp, count = self.accessor_layout(index)
        if ncomp != 1 or word not in ("u8", "u16", "u32"):
            raise PayloadError("index accessor %d is not unsigned SCALAR in %s" % (index, self.member))
        if "bufferView" in acc and "sparse" not in acc:
            data = self.view_bytes(acc["bufferView"])
            view = self.doc["bufferViews"][acc["bufferView"]]
            offset = acc.get("byteOffset", 0)
            stride = view.get("byteStride") or size
            if stride == size:
                end = offset + count * size
                if end > len(data):
                    raise PayloadError("index accessor %d overruns in %s" % (index, self.member))
                code = {1: "B", 2: "H", 4: "I"}[size]
                values = array.array(code)
                if values.itemsize != size:
                    code = "L" if size == 4 else code
                    values = array.array(code)
                values.frombytes(data[offset:end])
                if sys.byteorder != "little":
                    values.byteswap()
                return values, size
        return [v[0] for v in self.read_components(index)], size

    def position_decoder(self, index):
        """(word, scale, k, decode(element_dict) -> {i: (v0,v1,v2) ints}).

        The exact component value is v * scale. Signed normalized words are
        clamped at -k first (contract Frame chain, step 1)."""
        acc, (_, _, word), ncomp, count = self.accessor_layout(index)
        if ncomp != 3 or word == "u32":
            raise PayloadError("POSITION accessor %d is not a VEC3 of a POSITION type in %s"
                               % (index, self.member))
        normalized = bool(acc.get("normalized", False))
        if word == "f32":
            if normalized:
                raise PayloadError("normalized FLOAT accessor in %s" % self.member)
            scale, k = F32_SCALE, 1
        elif normalized:
            k = NORMALIZER[word]
            scale = Fraction(1, k)
        else:
            scale, k = Fraction(1), 1

        def decode(elements):
            raw = self.read_components(index, elements)
            out = {}
            for i, comps in raw.items():
                if word == "f32":
                    v = tuple(f32_scaled(b) for b in comps)
                    if None in v:
                        raise PayloadError("non-finite POSITION value in %s" % self.member)
                elif normalized and word in ("i8", "i16"):
                    v = tuple(max(c, -k) for c in comps)
                else:
                    v = tuple(comps)
                out[i] = v
            return out

        return word, scale, k, count, decode
