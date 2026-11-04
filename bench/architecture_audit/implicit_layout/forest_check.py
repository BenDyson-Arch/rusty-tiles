"""PO forest checker for P-IMPL (design v3 §4, A-2 and A-4). Stdlib only; imports nothing from `src/`.

Given a `.3tz` archive written as a bounded implicit forest, it checks the forest contract rule by
rule and prints (or writes) a JSON verdict. With `--explicit REF.3tz` (an `--explicit` build of the same
input, which is the producer hierarchy H) it also checks A-4's equivalence: the same tiles and
relation, roles, explicit tile objects, rows bitwise, extras, payloads, and root prefixes.

Usage:
  python3 -I forest_check.py FOREST.3tz [--explicit REF.3tz] [--out VERDICT.json] [--examples N]
Exit status: 0 when every required rule passes, 1 when one fails, 2 on a usage error.

Profile constants (design §4.1 with the coordinator's rulings): S = 4, DOC_NODES = 16,384, DOC_BYTES = 2 MiB
(compact bytes), DOC_LEVELS = 10,
JSON depth <= 32 (root depth 0; value nodes count the root, containers and scalars, not keys),
L <= 32. The README lists every rule, its design source and what it does not prove.
"""
import argparse, hashlib, json, math, posixpath, re, struct, sys, urllib.parse, zipfile

S = 4
DOC_NODES = 16384
DOC_BYTES = 2 * 1024 * 1024
DOC_LEVELS = 10   # coordinator ruling (review D-3): was 12 in design v3
JSON_DEPTH = 32
MAX_LEVELS = 32
SIDECARS = ('conversion.json', 'geometry-reports.jsonl')
VECTOR_BUILD = 'vector-build.json'
INDEX = '@3dtilesIndex1@'
SUFFIXES = ('glb', 'gltf')   # b3dm retired with glTF-only vector fills (#75)
MAX_SUBTREE_BITS = 1 << 22  # guard for malformed subtreeLevels; a valid root has at most 8^4 tiles

# (id, required, design source, description)
RULES = [
    ('archive', True, '3TZ / core', 'zip readable, member names unique, tileset.json parses as a tileset'),
    ('skeleton.S4', True, '§1, §4.1 classification', 'explicit tiles with children have h >= 4; their non-leaf children with h < 4 are implicit roots; '
     'leaf children are explicit leaves; the root is the single implicit root iff h(root) < 4'),
    ('documents.budget', True, 'P8, §4.1', 'every JSON document has <= 16,384 value nodes, <= 2 MiB and JSON depth <= 32'),
    ('documents.levels', True, '§4.1 DOC_LEVELS; ruling D-3', 'skeleton tiles sit at most 10 tile levels below their document root (links and unit children at most 11)'),
    ('documents.compact', True, 'coordinator ruling', 'every tileset JSON document is compact JSON (no whitespace outside strings), so budgets are measured on compact bytes'),
    ('links.wellFormed', True, '§4.1 Documents, P1', 'link tiles: content.uri tileset-d{m}.json at the archive root, m = 1..D-1 each referenced by exactly one link, '
     'no children, no implicitTiling, doc root without transform and with children, doc asset/geometricError/extensionsUsed as specified'),
    ('links.creationOrder', False, '§4.1 "m counts from 1 in creation order"', 'link numbers follow breadth-first order over documents taken in m order'),
    ('P6.linkCoherence', True, 'P6', "a link tile's box, error and refine equal its document root's"),
    ('P1.noJsonTemplate', True, 'P1, U-1', 'no implicit root content template names a .json resource; the only tileset JSON documents are tileset.json and linked tileset-d{m}.json'),
    ('P2.singleSubtreeRoots', True, 'P2', 'every implicit root: subtreeLevels == availableLevels == h+1 <= 4, exactly one subtree member, '
     'childSubtreeAvailability {constant: 0}, no children, closed availability'),
    ('P3.rowsEverywhere', True, 'P3, P12', 'every available tile has a FLOAT64 TILE_BOUNDING_BOX and TILE_GEOMETRIC_ERROR row; the level-0 row equals the root tile box and error'),
    ('P4.namesClosed', True, 'P4, PD-6', 'distinct r{n} prefixes; every name a template produces for available content exists; every member is referenced; '
     'vector-build.json is bound by asset.extras.vectorBuildStateSha256'),
    ('P9.levelBound', True, 'P9, PD-5', 'L = h(root) + 1 <= 32'),
    ('P10.slots', True, 'P10, R-4', 'content layer count == template count; zero content means no content/contents and no contentAvailability; '
     'templates implicit-content/r{n}-{level}-{x}-{y}[-{z}]-{slot}.{glb|gltf} in strictly sorted (slot, suffix) order, each with available content'),
    ('P11.noImplicitChildIndex', True, 'P11, R-8', 'no implicitChildIndex in any tile extras or row extras'),
    ('P12.schema', True, 'P12, F-1', 'tileset.json carries the rustyTilesImplicit schema iff an implicit root exists; every document holding one carries the same schema'),
    ('PD11.noRetainedSources', True, 'PD-11, §4.1', 'no rustyTilesSourceContents anywhere; no unreferenced t/ member; implicit tiles never reference t/ members'),
    ('payloads.wellFormed', True, '§4.1, glTF 2.0', 'implicit payloads are well-formed GLB (b3dm is refused); .gltf only at an implicit root'),
    ('vectorBuild.reuseMap', True, '§4.7 (PV)', 'when vector-build.json exists: its payloads map restores every entry byte-exactly to its sha256 key from a published implicit-content member'),
    ('A4.tree', True, 'P5, A-4', 'expanding every document and root yields exactly the tiles and parent/child relation of H'),
    ('A4.roles', True, '§4.1 classification on H', 'each tile has the role S = 4 assigns it in H'),
    ('A4.explicitTiles', True, 'P5, §4.1', 'explicit tiles (and link + document root) equal the producer tile objects without children and implicitChildIndex; '
     'implicit roots carry the producer transform, box, error, refine and content headers'),
    ('A4.rows', True, 'P3, A-4, ruling Q1', 'every row equals B(t) + delta(t) (delta summed parent-to-child in f64 from the implicit root) and e(t), bitwise; '
     'a row with delta = 0 is B(t) copied bit for bit'),
    ('A4.extras', True, 'P5, §4.5', 'row extras equal the producer extras without implicitChildIndex; encodedBytes equals the published payload length'),
    ('A4.payloads', True, 'P5, A-4, §4.1', 'explicit payloads byte-identical; implicit payloads: BIN chunk identical, JSON identical outside the scene-root node, '
     'root translation (or affine matrix column) == raw + (d0, d2, -d1) bitwise, root node text the only spliced span; a delta-0 payload is a byte copy; '
     'exactly one scene root, an affine matrix never combined with TRS'),
    ('A4.names', True, 'PD-6, R-7', 'each implicit root prefix r{n} is its preorder index in H'),
    ('A4.contentUris', False, '§4.1 "content URI"', 'explicit tile content URIs equal the reference build URIs'),
    ('A4.topLevel', False, '§4.1', 'tileset.json asset version, geometricError and extensionsUsed equal the reference build'),
]
RULE_IDS = [r[0] for r in RULES]
REFERENCE_RULES = {r for r in RULE_IDS if r.startswith('A4.')}


class Report:
    def __init__(self, examples):
        self.examples = examples
        self.violations = {r: [] for r in RULE_IDS}
        self.counts = {r: 0 for r in RULE_IDS}
        self.applicable = {r: (r not in REFERENCE_RULES and r != 'vectorBuild.reuseMap') for r in RULE_IDS}

    def bad(self, rule, message):
        self.counts[rule] += 1
        self.applicable[rule] = True
        if len(self.violations[rule]) < self.examples:
            self.violations[rule].append(message)

    def verdict(self):
        rules = {}
        for rid, required, source, description in RULES:
            status = 'not-applicable' if not self.applicable[rid] else ('fail' if self.counts[rid] else 'pass')
            rules[rid] = {'status': status, 'required': required, 'source': source, 'violations': self.counts[rid],
                          'examples': self.violations[rid], 'description': description}
        failed = [r for r, v in rules.items() if v['required'] and v['status'] == 'fail']
        return ('fail' if failed else 'pass'), failed, rules


# ------------------------------------------------------------------ small helpers
def f64(v):
    return struct.pack('<d', float(v))


def same_f64(a, b):
    return isinstance(a, (int, float)) and isinstance(b, (int, float)) and not isinstance(a, bool) \
        and not isinstance(b, bool) and f64(a) == f64(b)


def deq(a, b):
    """Deep JSON equality; numbers compare as f64 bit patterns (so -0.0 != 0.0), ints equal floats of equal value."""
    if isinstance(a, bool) or isinstance(b, bool):
        return type(a) is type(b) and a == b
    if isinstance(a, (int, float)) and isinstance(b, (int, float)):
        if isinstance(a, int) and isinstance(b, int):
            return a == b
        return f64(a) == f64(b)
    if isinstance(a, dict) and isinstance(b, dict):
        return a.keys() == b.keys() and all(deq(a[k], b[k]) for k in a)
    if isinstance(a, list) and isinstance(b, list):
        return len(a) == len(b) and all(deq(x, y) for x, y in zip(a, b))
    return type(a) is type(b) and a == b


def nodes_depth(v, d=0):
    """C1 semantics: value nodes = root + containers + scalars (keys excluded); root depth 0."""
    if isinstance(v, dict):
        n, m = 1, d
        for x in v.values():
            a, b = nodes_depth(x, d + 1); n += a; m = max(m, b)
        return n, m
    if isinstance(v, list):
        n, m = 1, d
        for x in v:
            a, b = nodes_depth(x, d + 1); n += a; m = max(m, b)
        return n, m
    return 1, d


def resolve(base_member, uri):
    """Resolve a relative URI against the directory of `base_member` (archive-root relative); None if it escapes."""
    if uri is None or not isinstance(uri, str) or uri.startswith('data:') or re.match(r'^[a-zA-Z][a-zA-Z0-9+.-]*:', uri) or uri.startswith('/'):
        return None
    path = urllib.parse.unquote(uri.split('#')[0].split('?')[0])
    joined = posixpath.join(posixpath.dirname(base_member), path)
    norm = posixpath.normpath(joined)
    if norm.startswith('..') or norm == '.':
        return None
    return norm


def is_compact(data):
    """True when the JSON text has no whitespace outside strings (serde_json::to_vec form)."""
    inside = escape = False
    for byte in data:
        if inside:
            if escape:
                escape = False
            elif byte == 0x5C:
                escape = True
            elif byte == 0x22:
                inside = False
        elif byte == 0x22:
            inside = True
        elif byte in (0x20, 0x09, 0x0A, 0x0D):
            return False
    return True


def contains_key(v, key):
    if isinstance(v, dict):
        return key in v or any(contains_key(x, key) for x in v.values())
    if isinstance(v, list):
        return any(contains_key(x, key) for x in v)
    return False


def morton(xyz, level):
    v, dims = 0, len(xyz)
    for bit in range(level):
        for i, c in enumerate(xyz):
            v |= ((c >> bit) & 1) << (bit * dims + i)
    return v


def child_xyz(xyz, c):
    """I1 §3 / AVAILABILITY.adoc: child index c sets x = 2x + (c&1), y = 2y + ((c>>1)&1), z = 2z + ((c>>2)&1)."""
    return tuple((v << 1) | ((c >> i) & 1) for i, v in enumerate(xyz))


def translation_of(tile):
    t = tile.get('transform')
    if t is None:
        return (0.0, 0.0, 0.0), True
    ok = isinstance(t, list) and len(t) == 16 and all(isinstance(v, (int, float)) for v in t) \
        and all(t[i] == (1 if i % 5 == 0 else 0) for i in range(12)) and t[15] == 1
    if not ok:
        return None, False
    return (float(t[12]), float(t[13]), float(t[14])), True


def strip_child_index(extras):
    if isinstance(extras, dict) and 'implicitChildIndex' in extras:
        extras = {k: v for k, v in extras.items() if k != 'implicitChildIndex'}
    return extras


# ------------------------------------------------------------------ binary formats
class FormatError(Exception):
    pass


def glb_split(data):
    """-> (json_chunk_bytes, rest_after_json_chunk). Checks header, length, chunk alignment."""
    if len(data) < 20 or data[:4] != b'glTF':
        raise FormatError('not a GLB (magic)')
    version, length = struct.unpack_from('<II', data, 4)
    if version != 2:
        raise FormatError(f'GLB version {version}')
    if length != len(data):
        raise FormatError(f'GLB length {length} != {len(data)} bytes')
    jl, jt = struct.unpack_from('<I4s', data, 12)
    if jt != b'JSON':
        raise FormatError('first GLB chunk is not JSON')
    if jl % 4:
        raise FormatError('GLB JSON chunk not 4-byte aligned')
    if 20 + jl > len(data):
        raise FormatError('GLB JSON chunk overruns')
    rest, o = data[20 + jl:], 0
    while o < len(rest):
        if o + 8 > len(rest):
            raise FormatError('GLB chunk header overruns')
        cl = struct.unpack_from('<I', rest, o)[0]
        if cl % 4 or o + 8 + cl > len(rest):
            raise FormatError('GLB chunk misaligned or overruns')
        o += 8 + cl
    return data[20:20 + jl], rest


def payload_glb(data, suffix):
    if suffix == 'b3dm' or data[:4] == b'b3dm':
        raise FormatError('b3dm payloads are not admitted (vector fills are glTF only)')
    return None, data


def scene_roots(doc):
    roots = set()
    for sc in doc.get('scenes', []) or []:
        for n in sc.get('nodes', []) or []:
            roots.add(n)
    return sorted(roots)


def _skip_ws(s, i):
    while i < len(s) and s[i:i + 1] in (b' ', b'\t', b'\r', b'\n'):
        i += 1
    return i


def _value_end(s, i):
    c = s[i:i + 1]
    if c == b'"':
        i += 1
        while True:
            ch = s[i:i + 1]
            if ch == b'':
                raise FormatError('unterminated string')
            if ch == b'\\':
                i += 2
            elif ch == b'"':
                return i + 1
            else:
                i += 1
    if c in (b'{', b'['):
        depth, inside = 0, False
        while True:
            ch = s[i:i + 1]
            if ch == b'':
                raise FormatError('unterminated container')
            if inside:
                if ch == b'\\':
                    i += 2; continue
                if ch == b'"':
                    inside = False
            elif ch == b'"':
                inside = True
            elif ch in (b'{', b'['):
                depth += 1
            elif ch in (b'}', b']'):
                depth -= 1
                if depth == 0:
                    return i + 1
            i += 1
    j = i
    while j < len(s) and s[j:j + 1] not in (b',', b'}', b']', b' ', b'\t', b'\r', b'\n'):
        j += 1
    return j


def node_spans(js):
    """Byte spans of every element of the top-level `nodes` array of a glTF JSON text."""
    i = _skip_ws(js, 0)
    if js[i:i + 1] != b'{':
        raise FormatError('glTF JSON is not an object')
    i += 1
    while True:
        i = _skip_ws(js, i)
        if js[i:i + 1] == b'}':
            return []
        kend = _value_end(js, i)
        key = json.loads(js[i:kend])
        i = _skip_ws(js, kend)
        if js[i:i + 1] != b':':
            raise FormatError('glTF JSON: expected colon')
        i = _skip_ws(js, i + 1)
        vend = _value_end(js, i)
        if key == 'nodes':
            spans, j = [], _skip_ws(js, i + 1)
            if js[j:j + 1] == b']':
                return spans
            while True:
                e = _value_end(js, j)
                spans.append((j, e))
                j = _skip_ws(js, e)
                if js[j:j + 1] == b']':
                    return spans
                if js[j:j + 1] != b',':
                    raise FormatError('glTF JSON: bad nodes array')
                j = _skip_ws(js, j + 1)
        i = _skip_ws(js, vend)
        if js[i:i + 1] == b',':
            i += 1


class Subtree:
    def __init__(self, data):
        if len(data) < 24:
            raise FormatError('subtree shorter than its header')
        magic, version, jl, bl = struct.unpack_from('<4sIQQ', data, 0)
        if magic != b'subt' or version != 1:
            raise FormatError('subtree magic/version')
        if 24 + jl + bl != len(data):
            raise FormatError(f'subtree lengths {24 + jl + bl} != {len(data)} bytes')
        if jl % 8 or bl % 8:
            raise FormatError('subtree JSON or binary chunk not 8-byte aligned')
        self.json = json.loads(data[24:24 + jl])
        self.bin = data[24 + jl:]
        self.buffers = self.json.get('buffers', [])
        self.views = self.json.get('bufferViews', [])
        self.external = {}

    def view(self, i):
        if not isinstance(i, int) or not 0 <= i < len(self.views):
            raise FormatError(f'bufferView {i} missing')
        v = self.views[i]
        b = v.get('buffer')
        if not isinstance(b, int) or not 0 <= b < len(self.buffers):
            raise FormatError(f'bufferView {i}: buffer {b} missing')
        buf = self.external.get(b, self.bin) if 'uri' in self.buffers[b] else self.bin
        o, n = v.get('byteOffset', 0), v.get('byteLength', 0)
        if o % 8:
            raise FormatError(f'bufferView {i} not 8-byte aligned')
        if o + n > len(buf) or o + n > self.buffers[b].get('byteLength', 0):
            raise FormatError(f'bufferView {i} overruns its buffer')
        return buf[o:o + n]

    def bits(self, avail, n):
        """Set of available indices in [0, n)."""
        if not isinstance(avail, dict):
            raise FormatError('availability is not an object')
        if 'constant' in avail:
            if avail['constant'] not in (0, 1) or 'bitstream' in avail:
                raise FormatError('bad constant availability')
            return set(range(n)) if avail['constant'] == 1 else set()
        if 'bitstream' not in avail:
            raise FormatError('availability without constant or bitstream')
        raw = self.view(avail['bitstream'])
        if len(raw) < (n + 7) // 8:
            raise FormatError('availability bitstream too short')
        ones = set()
        for byte_i, byte in enumerate(raw):
            if not byte:
                continue
            for k in range(8):
                if byte >> k & 1:
                    idx = byte_i * 8 + k
                    if idx >= n:
                        raise FormatError('availability bit set past the stream length')
                    ones.add(idx)
        if 'availableCount' in avail and avail['availableCount'] != len(ones):
            raise FormatError('availableCount differs from the bitstream')
        return ones


# ------------------------------------------------------------------ effective tree
class Tile:
    __slots__ = ('kind', 'obj', 'link', 'doc', 'depth_in_doc', 'children', 'parent', 'root', 'level', 'xyz', 'row',
                 'contents', 'h', 'refine', 'id')

    def __init__(self, kind, obj, doc, parent):
        self.kind, self.obj, self.doc, self.parent = kind, obj, doc, parent
        self.link, self.children, self.root, self.level, self.xyz, self.row = None, [], None, 0, None, None
        self.contents, self.h, self.refine, self.depth_in_doc = [], 0, None, 0


class Root:
    __slots__ = ('tile', 'n', 'scheme', 'dims', 'levels', 'available', 'templates', 'subtree_member', 'tiles', 'doc',
                 'layer_count', 'slots', 'ok')


class Forest:
    """Reads an archive and builds its effective tree, recording violations as it goes."""

    def __init__(self, path, report):
        self.r = report
        self.path = path
        self.z = zipfile.ZipFile(path)
        infos = [i for i in self.z.infolist() if not i.filename.endswith('/')]
        names = [i.filename for i in infos]
        if len(set(names)) != len(names):
            report.bad('archive', 'duplicate member names')
        self.members = set(names) - {INDEX}
        self.referenced = set()
        self.docs = {}        # name -> (obj, bytes)
        self.doc_order = []
        self.link_refs = {}   # doc name -> list of link descriptions
        self.roots = []
        self.tiles = []
        self.links = []
        self.cache = {}
        self.nesting = {}

    def read(self, name):
        if name not in self.cache:
            self.cache[name] = self.z.read(name)
        return self.cache[name]

    def read_once(self, name):
        return self.cache.pop(name, None) or self.z.read(name)

    # --- documents
    def load_doc(self, name):
        data = self.read(name)
        obj = json.loads(data)
        if not isinstance(obj, dict) or not isinstance(obj.get('root'), dict) or not isinstance(obj.get('asset'), dict):
            raise FormatError(f'{name} is not a tileset document')
        self.docs[name] = (obj, data)
        self.doc_order.append(name)
        self.nesting.setdefault(name, 0)
        self.referenced.add(name)
        return obj

    def build(self):
        if 'tileset.json' not in self.members:
            self.r.bad('archive', 'no tileset.json member')
            return None
        try:
            top = self.load_doc('tileset.json')
        except (ValueError, FormatError) as e:
            self.r.bad('archive', f'tileset.json: {e}')
            return None
        self.top = top
        self.schema = top.get('schema')
        self.root = self.explicit(top['root'], 'tileset.json', None, 0, None)
        return self.root

    def explicit(self, obj, doc, parent, depth, refine):
        if 'implicitTiling' in obj:
            return self.implicit_root(obj, doc, parent, depth, refine)
        contents = obj.get('contents') if isinstance(obj.get('contents'), list) else ([obj['content']] if isinstance(obj.get('content'), dict) else [])
        targets = [(c, resolve(doc, c.get('uri'))) for c in contents if isinstance(c, dict)]
        json_targets = [m for _, m in targets if m and m.lower().endswith('.json')]
        if json_targets:
            return self.link_tile(obj, doc, parent, json_targets, contents, depth, refine)
        t = Tile('explicit', obj, doc, parent)
        t.depth_in_doc = depth
        t.refine = obj.get('refine', refine)
        self.tiles.append(t)
        for i, (c, member) in enumerate(targets):
            if member is None:
                self.r.bad('P4.namesClosed', f'{doc}: explicit content uri {c.get("uri")!r} does not resolve inside the archive')
                continue
            t.contents.append((i, member, {k: v for k, v in c.items() if k != 'uri'}))
            self.reference_payload(member)
        for ch in obj.get('children', []) or []:
            if isinstance(ch, dict):
                t.children.append(self.explicit(ch, doc, t, depth + 1, t.refine))
        return t

    def link_tile(self, obj, doc, parent, json_targets, contents, depth, refine):
        """A link tile and its document root are one tile of H (P5): returns the document root's tile."""
        self.links.append((obj, doc))
        uri = obj.get('content', {}).get('uri') if isinstance(obj.get('content'), dict) else None
        name = json_targets[0]
        bad = lambda m: self.r.bad('links.wellFormed', f'{doc}: link to {name}: {m}')
        stub = Tile('explicit', obj, doc, parent)   # stands in when the document cannot be followed
        stub.refine = obj.get('refine', refine)
        if 'contents' in obj or len(contents) != 1:
            bad('a link tile has exactly one content (content, not contents)')
        if not isinstance(uri, str) or not re.fullmatch(r'tileset-d[1-9][0-9]*\.json', uri):
            bad(f'content.uri {uri!r} is not tileset-d{{m}}.json')
        if not isinstance(obj.get('content'), dict) or set(obj['content'].keys()) != {'uri'}:
            bad('link content carries more than uri')
        if obj.get('children'):
            bad('link tile has children')
        self.link_refs.setdefault(name, []).append(doc)
        if len(self.link_refs[name]) > 1:
            bad('document referenced by more than one link')
            self.tiles.append(stub)
            return stub
        if name not in self.members:
            bad('document member missing')
            self.tiles.append(stub)
            return stub
        self.nesting[name] = self.nesting.get(doc, 0) + 1
        try:
            dobj = self.load_doc(name)
        except (ValueError, FormatError) as e:
            bad(f'unreadable document ({e})')
            self.tiles.append(stub)
            return stub
        droot = dobj['root']
        if 'transform' in droot:
            bad('document root carries a transform (the link tile carries it)')
        if 'implicitTiling' in droot:
            bad('document root is an implicit root, not a skeleton tile')
        if not droot.get('children'):
            bad('document root has no children (only skeleton tiles become links)')
        if 'refine' not in droot:
            bad('document root has no refine (required on a tileset root)')
        if not deq(dobj.get('geometricError'), droot.get('geometricError')):
            bad('document geometricError differs from its root error')
        if not deq(dobj.get('extensionsUsed'), self.top.get('extensionsUsed')):
            bad('document extensionsUsed differs from tileset.json')
        if not isinstance(dobj['asset'], dict) or dobj['asset'].get('version') != self.top.get('asset', {}).get('version'):
            bad('document asset.version differs from tileset.json')
        for k in ('boundingVolume', 'geometricError'):
            if not deq(obj.get(k), droot.get(k)):
                self.r.bad('P6.linkCoherence', f'{doc}: link to {name}: {k} differs from the document root')
        if obj.get('refine', refine) != droot.get('refine'):
            self.r.bad('P6.linkCoherence', f'{doc}: link to {name}: refine {obj.get("refine", refine)!r} differs from the document root {droot.get("refine")!r}')
        if 'implicitTiling' in droot:
            sub = self.implicit_root(droot, name, parent, 0, droot.get('refine', refine))
        else:
            sub = self.explicit(droot, name, parent, 0, droot.get('refine', refine))
        sub.link = (obj, doc, depth)
        return sub

    def reference_payload(self, member):
        self.referenced.add(member)
        if member.lower().endswith('.gltf') and member in self.members:
            try:
                doc = json.loads(self.read(member))
            except ValueError:
                return
            self.reference_gltf_resources(member, doc)

    def reference_gltf_resources(self, member, doc):
        for key in ('buffers', 'images'):
            for item in doc.get(key, []) or []:
                uri = item.get('uri') if isinstance(item, dict) else None
                if isinstance(uri, str) and not uri.startswith('data:'):
                    target = resolve(member, uri)
                    if target is None or target not in self.members:
                        self.r.bad('P4.namesClosed', f'{member}: resource {uri!r} missing')
                    else:
                        self.referenced.add(target)

    # --- implicit roots
    def implicit_root(self, obj, doc, parent, depth, refine):
        r = Root()
        t = Tile('root', obj, doc, parent)
        t.depth_in_doc = depth
        t.refine = obj.get('refine', refine)
        t.root, t.level = r, 0
        self.tiles.append(t)
        self.roots.append(r)
        r.tile, r.doc, r.ok, r.tiles, r.n = t, doc, False, {}, None
        it = obj.get('implicitTiling')
        p2 = lambda m: self.r.bad('P2.singleSubtreeRoots', f'{doc}: implicit root {r.n if r.n is not None else "?"}: {m}')
        if not isinstance(it, dict):
            p2('implicitTiling is not an object')
            return t
        r.scheme = it.get('subdivisionScheme')
        if r.scheme not in ('QUADTREE', 'OCTREE'):
            p2(f'subdivisionScheme {r.scheme!r}')
            return t
        r.dims = 3 if r.scheme == 'OCTREE' else 2
        t.xyz = (0,) * r.dims
        sl, al = it.get('subtreeLevels'), it.get('availableLevels')
        suri = (it.get('subtrees') or {}).get('uri') if isinstance(it.get('subtrees'), dict) else None
        zpart = r'-\{z\}' if r.dims == 3 else ''
        m = re.fullmatch(r'subtrees/r(0|[1-9][0-9]*)-\{level\}-\{x\}-\{y\}' + zpart + r'\.subtree', suri or '')
        if m:
            r.n = int(m.group(1))
        else:
            p2(f'subtrees.uri {suri!r} is not subtrees/r{{n}}-{{level}}-{{x}}-{{y}}{"-{z}" if r.dims == 3 else ""}.subtree')
            m2 = re.match(r'subtrees/r(0|[1-9][0-9]*)-', suri or '')
            r.n = int(m2.group(1)) if m2 else None
        if obj.get('children'):
            p2('an implicit root tile has explicit children')
        if not isinstance(sl, int) or not isinstance(al, int) or sl < 1 or al < 1:
            p2(f'subtreeLevels {sl!r} / availableLevels {al!r}')
            return t
        if sl != al:
            p2(f'subtreeLevels {sl} != availableLevels {al}')
        if al > S:
            p2(f'availableLevels {al} > S = {S}')
        # templates (P1, P10)
        contents = obj.get('contents') if 'contents' in obj else ([obj['content']] if 'content' in obj else [])
        if 'contents' in obj and 'content' in obj:
            self.r.bad('P10.slots', f'{doc}: implicit root r{r.n}: both content and contents')
        if not isinstance(contents, list):
            contents = []
        r.templates = []
        for c in contents:
            uri = c.get('uri') if isinstance(c, dict) else None
            norm = resolve(doc, uri.replace('{', '\x00').replace('}', '\x01')) if isinstance(uri, str) else None
            last = (norm or uri or '').rsplit('/', 1)[-1]
            if isinstance(uri, str) and last.lower().endswith('.json'):
                self.r.bad('P1.noJsonTemplate', f'{doc}: implicit root r{r.n}: JSON content template {uri!r}')
            mt = re.fullmatch(r'implicit-content/r(0|[1-9][0-9]*)-\{level\}-\{x\}-\{y\}' + zpart + r'-(0|[1-9][0-9]*)\.([a-z0-9]+)', uri or '')
            if not mt or mt.group(3) not in SUFFIXES:
                self.r.bad('P10.slots', f'{doc}: implicit root r{r.n}: template {uri!r} is not implicit-content/r{{n}}-...-{{slot}}.{{glb|gltf}}')
                r.templates.append((uri, None, None, c))
                continue
            if r.n is not None and int(mt.group(1)) != r.n:
                self.r.bad('P4.namesClosed', f'{doc}: implicit root r{r.n}: template prefix r{mt.group(1)} differs from its subtree prefix')
            r.templates.append((uri, int(mt.group(2)), mt.group(3), c))
        keys = [(s, x) for _, s, x, _ in r.templates if s is not None]
        if any(a >= b for a, b in zip(keys, keys[1:])):
            self.r.bad('P10.slots', f'{doc}: implicit root r{r.n}: templates not in strictly sorted (slot, suffix) order: {keys}')
        # the single subtree
        subst = lambda tmpl, level, xyz: tmpl.replace('{level}', str(level)).replace('{x}', str(xyz[0])).replace('{y}', str(xyz[1])).replace('{z}', str(xyz[2]) if len(xyz) > 2 else '{z}')
        if not suri:
            return t
        r.subtree_member = resolve(doc, subst(suri, 0, (0,) * r.dims))
        if r.subtree_member is None or r.subtree_member not in self.members:
            p2(f'subtree member {r.subtree_member!r} missing')
            return t
        self.referenced.add(r.subtree_member)
        if r.n is not None:
            others = [x for x in self.members if x.startswith(f'subtrees/r{r.n}-') and x != r.subtree_member]
            if others:
                p2(f'{len(others)} further subtree members share the prefix (e.g. {others[0]})')
        b = 2 ** r.dims
        n_tiles = (b ** sl - 1) // (b - 1)
        if n_tiles > MAX_SUBTREE_BITS:
            p2(f'subtreeLevels {sl} too large to expand')
            return t
        try:
            st = Subtree(self.read_once(r.subtree_member))
        except (FormatError, ValueError, struct.error) as e:
            p2(f'subtree {r.subtree_member}: {e}')
            return t
        for bi, buf in enumerate(st.buffers):
            if isinstance(buf, dict) and 'uri' in buf:
                target = resolve(r.subtree_member, buf['uri'])
                if target is None or target not in self.members:
                    p2(f'external subtree buffer {buf["uri"]!r} missing')
                    return t
                self.referenced.add(target)
                st.external[bi] = self.read(target)
        js = st.json
        if js.get('childSubtreeAvailability') != {'constant': 0} or isinstance(js.get('childSubtreeAvailability', {}).get('constant'), bool):
            p2(f'childSubtreeAvailability is {js.get("childSubtreeAvailability")!r}, not {{constant: 0}}')
        try:
            avail = st.bits(js.get('tileAvailability'), n_tiles)
            layers = js.get('contentAvailability')
            if layers is not None and not isinstance(layers, list):
                raise FormatError('contentAvailability is not an array')
            content_bits = [st.bits(a, n_tiles) for a in (layers or [])]
        except (FormatError, struct.error) as e:
            p2(f'availability: {e}')
            return t
        # P10: layers vs templates
        if len(r.templates) != len(layers or []):
            self.r.bad('P10.slots', f'{doc}: implicit root r{r.n}: {len(layers or [])} content layers for {len(r.templates)} templates')
        if not r.templates and layers is not None:
            self.r.bad('P10.slots', f'{doc}: implicit root r{r.n}: contentAvailability present on a root without content')
        # expand availability (levels relative to the root; one subtree only)
        index = lambda level, xyz: (b ** level - 1) // (b - 1) + morton(xyz, level)
        origin = (0,) * r.dims
        if index(0, origin) not in avail:
            p2('the root tile is not available')
            return t
        order, frontier, seen = [], [(0, origin, t)], set()
        t.level = 0
        while frontier:
            level, xyz, tile = frontier.pop()
            i = index(level, xyz)
            seen.add(i)
            order.append((i, level, xyz, tile))
            r.tiles[(level, xyz)] = tile
            if level + 1 >= sl:
                continue
            for c in range(b):
                cx = child_xyz(xyz, c)
                if index(level + 1, cx) in avail:
                    ch = Tile('implicit', None, doc, tile)
                    ch.root, ch.level, ch.xyz, ch.refine = r, level + 1, cx, t.refine
                    tile.children.append(ch)
                    self.tiles.append(ch)
                    frontier.append((level + 1, cx, ch))
        orphans = avail - seen
        if orphans:
            p2(f'{len(orphans)} available tiles have an unavailable parent')
        top_level = max(level for _, level, _, _ in order)
        if top_level + 1 != al:
            p2(f'availableLevels {al} but the deepest available level is {top_level} (h + 1 = {top_level + 1})')
        order.sort()
        for li, bits in enumerate(content_bits):
            if bits - avail:
                self.r.bad('P10.slots', f'{doc}: implicit root r{r.n}: content layer {li} marks unavailable tiles')
            if not bits & avail:
                self.r.bad('P10.slots', f'{doc}: implicit root r{r.n}: content layer {li} has no available content')
        # contents per tile
        for i, level, xyz, tile in order:
            for li, bits in enumerate(content_bits):
                if i in bits and li < len(r.templates):
                    uri, slot, suffix, header = r.templates[li]
                    member = resolve(doc, subst(uri, level, xyz)) if isinstance(uri, str) else None
                    tile.contents.append((li, member, {k: v for k, v in (header or {}).items() if k != 'uri'}))
                    if member is None or member not in self.members:
                        self.r.bad('P4.namesClosed', f'{doc}: implicit root r{r.n}: template name {member!r} for available content is missing')
                    else:
                        self.reference_payload(member)
                    if member and member.startswith('t/'):
                        self.r.bad('PD11.noRetainedSources', f'implicit tile content {member} is a t/ member')
        # rows (P3)
        self.rows(r, st, order, doc)
        r.ok = True
        return t

    def rows(self, r, st, order, doc):
        p3 = lambda m: self.r.bad('P3.rowsEverywhere', f'{doc}: implicit root r{r.n}: {m}')
        js = st.json
        tables = js.get('propertyTables')
        tm = js.get('tileMetadata')
        if not isinstance(tables, list) or not isinstance(tm, int) or not 0 <= tm < len(tables):
            p3('no tileMetadata property table')
            return
        table = tables[tm]
        schema = self.schema if isinstance(self.schema, dict) else (self.docs[doc][0].get('schema') if isinstance(self.docs.get(doc, ({},))[0].get('schema'), dict) else None)
        if not isinstance(schema, dict):
            p3('no schema to decode rows (tileset.json has none)')
            return
        cls = (schema.get('classes') or {}).get(table.get('class'))
        if not isinstance(cls, dict):
            p3(f'class {table.get("class")!r} not in the root schema')
            return
        props = cls.get('properties') or {}
        sem = {p.get('semantic'): (name, p) for name, p in props.items() if isinstance(p, dict) and p.get('semantic')}
        box_def, err_def = sem.get('TILE_BOUNDING_BOX'), sem.get('TILE_GEOMETRIC_ERROR')
        if not box_def or not err_def:
            p3('class lacks TILE_BOUNDING_BOX or TILE_GEOMETRIC_ERROR')
            return
        bn, bp = box_def
        en, ep = err_def
        if not (bp.get('type') == 'SCALAR' and bp.get('componentType') == 'FLOAT64' and bp.get('array') is True and bp.get('count') == 12):
            p3('TILE_BOUNDING_BOX is not FLOAT64[12]')
            return
        if not (ep.get('type') == 'SCALAR' and ep.get('componentType') == 'FLOAT64' and not ep.get('array')):
            p3('TILE_GEOMETRIC_ERROR is not a FLOAT64 scalar')
            return
        for name, p in ((bn, bp), (en, ep)):
            if any(k in p for k in ('offset', 'scale', 'noData', 'default', 'normalized')):
                p3(f'{name} uses offset/scale/noData/default')
        count = table.get('count')
        if count != len(order):
            p3(f'property table count {count} != {len(order)} available tiles')
            return
        tp = table.get('properties') or {}
        try:
            boxes = st.view(tp[bn]['values'])
            errors = st.view(tp[en]['values'])
            if len(boxes) != 96 * count or len(errors) != 8 * count:
                p3('row value buffers do not hold exactly one row per available tile')
                return
            extras = None
            if 'extras' in tp and isinstance(props.get('extras'), dict) and props['extras'].get('type') == 'STRING':
                ex = tp['extras']
                values = st.view(ex['values'])
                otype = ex.get('stringOffsetType', 'UINT32')
                fmt = {'UINT8': 'B', 'UINT16': 'H', 'UINT32': 'I', 'UINT64': 'Q'}[otype]
                raw = st.view(ex['stringOffsets'])
                offs = struct.unpack_from('<%d%s' % (count + 1, fmt), raw, 0)
                extras = [values[offs[k]:offs[k + 1]] for k in range(count)]
        except (KeyError, FormatError, struct.error, TypeError) as e:
            p3(f'row buffers: {e}')
            return
        for k, (i, level, xyz, tile) in enumerate(order):
            box = struct.unpack_from('<12d', boxes, 96 * k)
            err = struct.unpack_from('<d', errors, 8 * k)[0]
            ex_text = extras[k].decode('utf-8', 'replace') if extras is not None else None
            if not all(math.isfinite(v) for v in box) or not math.isfinite(err):
                p3(f'row at level {level} {xyz} is not finite')
            ex_obj = None
            if ex_text is not None:
                try:
                    ex_obj = json.loads(ex_text) if ex_text else {}
                except ValueError:
                    p3(f'row extras at level {level} {xyz} are not JSON')
                if isinstance(ex_obj, dict) and contains_key(ex_obj, 'implicitChildIndex'):
                    self.r.bad('P11.noImplicitChildIndex', f'{doc}: implicit root r{r.n}: row extras at level {level} {xyz} carry implicitChildIndex')
            tile.row = (box, err, ex_obj)
        root_tile = r.tile
        rb = root_tile.obj.get('boundingVolume', {}).get('box') if isinstance(root_tile.obj.get('boundingVolume'), dict) else None
        row0 = root_tile.row
        if not (isinstance(rb, list) and len(rb) == 12 and all(same_f64(a, b) for a, b in zip(rb, row0[0]))):
            p3('level-0 row box differs from the root tile box')
        if not same_f64(root_tile.obj.get('geometricError'), row0[1]):
            p3('level-0 row error differs from the root tile geometricError')


# ------------------------------------------------------------------ archive-only rules
def heights(t):
    stack, post = [t], []
    while stack:
        n = stack.pop()
        post.append(n)
        stack.extend(n.children)
    for n in reversed(post):
        n.h = 1 + max((c.h for c in n.children), default=-1)
    return post


def check_archive(f, report):
    root = f.build()
    if root is None:
        return None
    tiles = heights(root)
    L = root.h + 1
    if L > MAX_LEVELS:
        report.bad('P9.levelBound', f'hierarchy has {L} levels; implicit output admits at most {MAX_LEVELS}')
    # classification (S = 4)
    sk = lambda m: report.bad('skeleton.S4', m)
    if root.kind == 'root':
        if root.h >= S:
            sk(f'the root is an implicit root of height {root.h} >= {S}')
    else:
        if root.h < S:
            sk(f'the root is explicit but h(root) = {root.h} < {S}: it must be the single implicit root')
    for t in tiles:
        if t.kind != 'explicit':
            continue
        if t.children and t.h < S:
            sk(f'{t.doc}: explicit tile with children has h = {t.h} < {S} (should be an implicit root)')
        if not t.children and t.parent is None:
            sk('the root is an explicit leaf')
        for c in t.children:
            if c.kind == 'root' and not 1 <= c.h < S:
                sk(f'{c.doc}: implicit root r{c.root.n} has h = {c.h}; implicit roots below the skeleton have 1 <= h < {S}')
            if c.kind == 'implicit':
                sk('an implicit tile under an explicit tile')
        if t.parent is not None and t.parent.kind != 'explicit':
            sk('an explicit tile below an implicit tile')
    # roots placed only under skeleton tiles
    for r in f.roots:
        p = r.tile.parent
        if p is not None and p.kind != 'explicit':
            sk(f'implicit root r{r.n} is not a child of an explicit tile')
    # documents: budgets, levels, schema, retained-source lists, implicitChildIndex, JSON documents
    any_root = bool(f.roots)
    for name in f.doc_order:
        obj, data = f.docs[name]
        n, d = nodes_depth(obj)
        if n > DOC_NODES:
            report.bad('documents.budget', f'{name}: {n} value nodes > {DOC_NODES}')
        if len(data) > DOC_BYTES:
            report.bad('documents.budget', f'{name}: {len(data)} bytes > {DOC_BYTES}')
        if d > JSON_DEPTH:
            report.bad('documents.budget', f'{name}: JSON depth {d} > {JSON_DEPTH}')
        if not is_compact(data):
            report.bad('documents.compact', f'{name}: not compact JSON (whitespace outside strings)')
        if contains_key(obj, 'rustyTilesSourceContents'):
            report.bad('PD11.noRetainedSources', f'{name}: carries rustyTilesSourceContents')
        stack = [(obj['root'], 0)]
        while stack:
            tile, depth = stack.pop()
            if not isinstance(tile, dict):
                continue
            ex = tile.get('extras')
            if isinstance(ex, dict) and contains_key(ex, 'implicitChildIndex'):
                report.bad('P11.noImplicitChildIndex', f'{name}: tile extras at depth {depth} carry implicitChildIndex')
            is_link = isinstance(tile.get('content'), dict) and str(tile['content'].get('uri', '')).lower().endswith('.json')
            skeleton = bool(tile.get('children')) and 'implicitTiling' not in tile and not is_link
            if skeleton and depth > DOC_LEVELS:
                report.bad('documents.levels', f'{name}: skeleton tile {depth} levels below the document root > {DOC_LEVELS}')
            elif not skeleton and depth > DOC_LEVELS + 1:
                report.bad('documents.levels', f'{name}: tile {depth} levels below the document root > {DOC_LEVELS + 1}')
            for c in tile.get('children', []) or []:
                stack.append((c, depth + 1))
    schema = f.top.get('schema')
    if any_root != ('schema' in f.top):
        report.bad('P12.schema', f'tileset.json schema present = {"schema" in f.top}, implicit roots = {len(f.roots)}')
    if any_root and isinstance(schema, dict):
        if schema.get('id') != 'rustyTilesImplicit':
            report.bad('P12.schema', f'tileset.json schema id {schema.get("id")!r} is not rustyTilesImplicit')
    elif any_root and 'schema' in f.top:
        report.bad('P12.schema', 'tileset.json schema is not an object')
    for name in sorted({r.doc for r in f.roots}):
        dschema = f.docs[name][0].get('schema')
        if not isinstance(dschema, dict) or not deq(dschema, schema):
            report.bad('P12.schema', f'{name} holds an implicit root but lacks the root schema')
    # link numbering
    docs = [n for n in f.doc_order if n != 'tileset.json']
    nums = sorted(int(re.fullmatch(r'tileset-d([1-9][0-9]*)\.json', n).group(1)) for n in docs if re.fullmatch(r'tileset-d([1-9][0-9]*)\.json', n))
    if nums != list(range(1, len(docs) + 1)):
        report.bad('links.wellFormed', f'documents are not numbered 1..{len(docs)} contiguously: {nums[:10]}')
    seq = []
    for name in sorted(f.docs, key=lambda n: 0 if n == 'tileset.json' else int(re.sub(r'\D', '', n) or 0)):
        queue = [f.docs[name][0]['root']]
        while queue:
            tile = queue.pop(0)
            c = tile.get('content') if isinstance(tile, dict) else None
            if isinstance(c, dict):
                m = re.fullmatch(r'tileset-d([1-9][0-9]*)\.json', str(c.get('uri', '')))
                if m:
                    seq.append(int(m.group(1)))
            queue.extend(x for x in (tile.get('children', []) or []) if isinstance(x, dict))
    if seq != sorted(seq):
        report.bad('links.creationOrder', f'link numbers in breadth-first document order: {seq[:20]}')
    # P1: tileset JSON documents
    for m in sorted(f.members):
        low = m.lower()
        if low.startswith('implicit-tileset-') or (low.endswith('.json') and m not in f.docs and m not in SIDECARS + (VECTOR_BUILD,)):
            try:
                obj = json.loads(f.read(m))
                is_tileset = isinstance(obj, dict) and 'asset' in obj and 'root' in obj
            except ValueError:
                is_tileset = False
            if is_tileset or low.startswith('implicit-tileset-'):
                report.bad('P1.noJsonTemplate', f'tileset JSON member {m} is neither tileset.json nor a linked tileset-d{{m}}.json')
    # P4: prefixes distinct and closure
    seen = {}
    for r in f.roots:
        if r.n is None:
            continue
        if r.n in seen:
            report.bad('P4.namesClosed', f'prefix r{r.n} used by more than one implicit root')
        seen[r.n] = r
    vb = f.top.get('asset', {}).get('extras', {}).get('vectorBuildStateSha256') if isinstance(f.top.get('asset', {}).get('extras'), dict) else None
    for side in SIDECARS:
        if side in f.members:
            f.referenced.add(side)
    if VECTOR_BUILD in f.members:
        if vb == hashlib.sha256(f.read(VECTOR_BUILD)).hexdigest():
            f.referenced.add(VECTOR_BUILD)
        else:
            report.bad('P4.namesClosed', 'vector-build.json is not bound by asset.extras.vectorBuildStateSha256')
    elif vb is not None:
        report.bad('P4.namesClosed', 'asset.extras.vectorBuildStateSha256 names a missing vector-build.json')
    for m in sorted(f.members - f.referenced):
        report.bad('P4.namesClosed', f'unreferenced member {m}')
        if m.startswith('t/'):
            report.bad('PD11.noRetainedSources', f'unreferenced t/ member {m}')
    # payloads: implicit payload format, gltf placement
    for r in f.roots:
        for (level, xyz), tile in r.tiles.items():
            for li, member, _ in tile.contents:
                if member is None or member not in f.members:
                    continue
                suffix = member.rsplit('.', 1)[-1].lower()
                if suffix == 'gltf':
                    if level != 0:
                        report.bad('payloads.wellFormed', f'{member}: .gltf below an implicit root')
                    continue
                try:
                    _, glb = payload_glb(f.read(member), suffix)
                    js, _ = glb_split(glb)
                    gdoc = json.loads(js)
                    f.reference_gltf_resources(member, gdoc) if any(isinstance(b, dict) and 'uri' in b for b in gdoc.get('buffers', []) or []) else None
                except (FormatError, ValueError, struct.error) as e:
                    report.bad('payloads.wellFormed', f'{member}: {e}')
                f.cache.pop(member, None)
    return root


# ------------------------------------------------------------------ vector-build payload map (§4.7)
def restore_payload(published, record):
    """The design's restore: put the recorded raw root node text back, pad the JSON chunk to the recorded length."""
    if record.get('rootNode') == '' and record.get('jsonLength') == 0:
        return published   # ruling: a zero-delta payload is a byte copy; its record is the empty sentinel
    _, glb = payload_glb(published, 'glb')
    js, rest = glb_split(glb)
    text = js.rstrip(b' ')
    doc = json.loads(text)
    roots = scene_roots(doc)
    if len(roots) != 1:
        raise FormatError(f'{len(roots)} scene roots')
    spans = node_spans(text)
    a, b = spans[roots[0]]
    raw_js = text[:a] + record['rootNode'].encode() + text[b:]
    length = record['jsonLength']
    if length < len(raw_js):
        raise FormatError('recorded jsonLength shorter than the restored JSON')
    raw_js += b' ' * (length - len(raw_js))
    out = b'glTF' + struct.pack('<II', 2, 20 + len(raw_js) + len(rest)) + struct.pack('<I4s', len(raw_js), b'JSON') + raw_js + rest
    return out


def check_reuse_map(f, report):
    if VECTOR_BUILD not in f.members:
        return None
    report.applicable['vectorBuild.reuseMap'] = True
    bad = lambda m: report.bad('vectorBuild.reuseMap', m)
    try:
        vb = json.loads(f.read(VECTOR_BUILD))
    except ValueError:
        bad('vector-build.json is not JSON')
        return None
    payloads = vb.get('payloads') if isinstance(vb, dict) else None
    if not isinstance(payloads, dict):
        bad('vector-build.json has no payloads map')
        return None
    for key, rec in sorted(payloads.items()):
        if not isinstance(rec, dict) or not all(k in rec for k in ('member', 'delta', 'rootNode', 'jsonLength')):
            bad(f'{key}: record lacks member/delta/rootNode/jsonLength')
            continue
        m = rec['member']
        if not isinstance(m, str) or not m.startswith('implicit-content/') or m not in f.members:
            bad(f'{key}: member {m!r} is not a published implicit-content member')
            continue
        try:
            raw = restore_payload(f.read(m), rec)
        except (FormatError, ValueError, struct.error, KeyError, TypeError, AttributeError) as e:
            bad(f'{key}: restore failed ({e})')
            continue
        if hashlib.sha256(raw).hexdigest() != key:
            bad(f'{key}: restored bytes hash to {hashlib.sha256(raw).hexdigest()}')
    return payloads


# ------------------------------------------------------------------ reference (A-4)
class HNode:
    __slots__ = ('tile', 'children', 'slot', 'h', 'pid', 'parent', 'refine', 'role')


def read_reference(path):
    z = zipfile.ZipFile(path)
    top = json.loads(z.read('tileset.json'))
    pid = [0]

    def build(t, slot, parent, refine):
        n = HNode()
        n.tile = {k: v for k, v in t.items() if k != 'children'}
        n.slot, n.parent, n.children = slot, parent, []
        n.refine = t.get('refine', refine)
        n.pid = pid[0]; pid[0] += 1
        for i, c in enumerate(t.get('children', []) or []):
            ex = c.get('extras')
            s = ex.get('implicitChildIndex', i) if isinstance(ex, dict) else i
            n.children.append(build(c, s, n, n.refine))
        return n
    sys.setrecursionlimit(10000)
    root = build(top['root'], 0, None, None)
    stack, post = [root], []
    while stack:
        n = stack.pop(); post.append(n); stack.extend(n.children)
    for n in reversed(post):
        n.h = 1 + max((c.h for c in n.children), default=-1)
    # classification (design §4.1)
    root.role = 'root' if root.h < S else 'skeleton'
    for n in post:
        for c in n.children:
            if n.role == 'skeleton':
                c.role = 'skeleton' if c.h >= S else ('explicitLeaf' if c.h == 0 else 'root')
            else:
                c.role = 'implicit'
    return z, top, root, post


def forest_role(t):
    if t.kind == 'root':
        return 'root'
    if t.kind == 'implicit':
        return 'implicit'
    return 'skeleton' if t.children or t.link else 'explicitLeaf'


def signature(obj, transform):
    return json.dumps([obj.get('boundingVolume'), obj.get('geometricError'), transform], sort_keys=True)


def compare_payload(raw, pub, delta, suffix):
    """A-4 / §4.1 payload rule. Returns a list of problems (empty when equal)."""
    d = (delta[0], delta[2], -delta[1])
    zero = all(v == 0.0 for v in delta)
    if suffix == 'gltf':
        return [] if pub == raw else ['.gltf payload is not a byte copy']
    if pub == raw:
        return [] if zero else ['payload is a byte copy but delta is not zero']
    if zero:
        return ['delta is zero but the payload is not a byte copy']
    problems = []
    try:
        _, rglb = payload_glb(raw, suffix)
        _, pglb = payload_glb(pub, suffix)
        rjs, rrest = glb_split(rglb)
        pjs, prest = glb_split(pglb)
    except (FormatError, struct.error) as e:
        return [f'container: {e}']
    if rrest != prest:
        problems.append('BIN (or later) chunk differs')
    rtext, ptext = rjs.rstrip(b' '), pjs.rstrip(b' ')
    try:
        rdoc, pdoc = json.loads(rtext), json.loads(ptext)
    except ValueError:
        return problems + ['JSON chunk does not parse']
    roots = scene_roots(rdoc)
    if len(roots) != 1:
        problems.append(f'{len(roots)} scene roots (the writer refuses more than one)')
    if scene_roots(pdoc) != roots or len(rdoc.get('nodes', [])) != len(pdoc.get('nodes', [])):
        return problems + ['scene roots or node count differ']
    for i in roots:
        rn = rdoc['nodes'][i]
        m = rn.get('matrix')
        if m is not None and (any(k in rn for k in ('translation', 'rotation', 'scale')) or not isinstance(m, list) or len(m) != 16
                              or not all(isinstance(v, (int, float)) for v in m) or m[3] != 0 or m[7] != 0 or m[11] != 0 or m[15] != 1):
            problems.append(f'raw root node {i}: matrix combined with TRS or not affine (the writer refuses it)')
    if not deq({k: v for k, v in rdoc.items() if k != 'nodes'}, {k: v for k, v in pdoc.items() if k != 'nodes'}):
        problems.append('JSON outside nodes differs')
    for i, (rn, pn) in enumerate(zip(rdoc.get('nodes', []), pdoc.get('nodes', []))):
        if i not in roots:
            if not deq(rn, pn):
                problems.append(f'non-root node {i} differs')
            continue
        if 'matrix' in rn:
            exp = list(rn['matrix'])
            for k in range(3):
                exp[12 + k] = float(exp[12 + k]) + d[k]
            if any(k in pn for k in ('translation', 'rotation', 'scale')) or not (isinstance(pn.get('matrix'), list) and len(pn['matrix']) == 16 and all(same_f64(a, b) for a, b in zip(exp, pn['matrix']))):
                problems.append(f'root node {i} matrix translation != raw + delta')
            rest_r = {k: v for k, v in rn.items() if k != 'matrix'}
            rest_p = {k: v for k, v in pn.items() if k != 'matrix'}
        else:
            t = rn.get('translation', [0.0, 0.0, 0.0])
            exp = [float(t[k]) + d[k] for k in range(3)]
            got = pn.get('translation')
            if 'matrix' in pn:
                problems.append(f'root node {i} gained a matrix')
            if not (isinstance(got, list) and len(got) == 3 and all(same_f64(a, b) for a, b in zip(exp, got))):
                problems.append(f'root node {i} translation {got} != raw + delta {exp}')
            rest_r = {k: v for k, v in rn.items() if k != 'translation'}
            rest_p = {k: v for k, v in pn.items() if k != 'translation'}
        if not deq(rest_r, rest_p):
            problems.append(f'root node {i} differs outside its translation')
    # byte splice: only the root node objects' text changes
    try:
        rs, ps = node_spans(rtext), node_spans(ptext)
        cuts_r = [rs[i] for i in roots]
        cuts_p = [ps[i] for i in roots]
        prev_r = prev_p = 0
        for (ra, rb), (pa, pb) in zip(cuts_r + [(len(rtext), len(rtext))], cuts_p + [(len(ptext), len(ptext))]):
            if rtext[prev_r:ra] != ptext[prev_p:pa]:
                problems.append('JSON bytes outside the scene-root node object differ (not a root-node splice)')
                break
            prev_r, prev_p = rb, pb
    except (FormatError, ValueError, IndexError) as e:
        problems.append(f'splice scan: {e}')
    return problems


def check_reference(f, report, ref_path, reuse_map):
    for rid in REFERENCE_RULES:
        report.applicable[rid] = True
    hz, htop, hroot, hpost = read_reference(ref_path)
    froot = f.root
    # top level (advisory)
    for k in ('geometricError', 'extensionsUsed'):
        if not deq(htop.get(k), f.top.get(k)):
            report.bad('A4.topLevel', f'tileset.json {k} differs from the reference')
    if htop.get('asset', {}).get('version') != f.top.get('asset', {}).get('version'):
        report.bad('A4.topLevel', 'asset.version differs from the reference')
    implicit_raw_hashes = set()
    explicit_raw_hashes = set()
    mapped = {}  # id(Tile) -> HNode, for callers

    def h_contents(n):
        t = n.tile
        cs = t.get('contents') if isinstance(t.get('contents'), list) else ([t['content']] if isinstance(t.get('content'), dict) else [])
        return [(i, c, resolve('tileset.json', c.get('uri'))) for i, c in enumerate(cs)]

    def h_extras(n):
        ex = strip_child_index(n.tile.get('extras'))
        return None if ex == {} else ex

    def compare_explicit(h, t):
        obj = dict(t.obj)
        if obj.get('extras') == {}:
            obj.pop('extras')   # absent and empty extras are the same producer extras
        if t.link:
            lobj = t.link[0]
            if 'transform' in lobj:
                obj['transform'] = lobj['transform']
        hobj = dict(h.tile)
        hobj['extras'] = h_extras(h)
        if hobj['extras'] is None:
            hobj.pop('extras')
        for o in (obj, hobj):
            o.pop('children', None)
            o.pop('refine', None)
        oc = obj.pop('content', None); obj.pop('contents', None)
        hc = hobj.pop('content', None); hobj.pop('contents', None)
        if not deq(obj, hobj):
            diff = sorted(k for k in set(obj) | set(hobj) if not deq(obj.get(k), hobj.get(k)))
            report.bad('A4.explicitTiles', f'tile p{h.pid}: explicit tile differs from the producer tile in {diff}')
        if t.refine != h.refine:
            report.bad('A4.explicitTiles', f'tile p{h.pid}: refine {t.refine!r} != {h.refine!r}')
        hcs = h_contents(h)
        if len(hcs) != len(t.contents):
            report.bad('A4.payloads', f'tile p{h.pid}: {len(t.contents)} contents, reference has {len(hcs)}')
            return
        for (hi, hcobj, hm), (fi, fm, fheader) in zip(hcs, t.contents):
            if not deq({k: v for k, v in hcobj.items() if k != 'uri'}, fheader):
                report.bad('A4.explicitTiles', f'tile p{h.pid}: content header differs')
            if hm != fm:
                report.bad('A4.contentUris', f'tile p{h.pid}: content uri {fm} != reference {hm}')
            if hm is None or fm is None or fm not in f.members:
                continue
            raw = hz.read(hm)
            explicit_raw_hashes.add(hashlib.sha256(raw).hexdigest())
            if f.read_once(fm) != raw:
                report.bad('A4.payloads', f'tile p{h.pid}: explicit payload {fm} differs from the reference bytes')

    def compare_root(h, t):
        r = t.root
        if r.n != h.pid:
            report.bad('A4.names', f'implicit root prefix r{r.n} != preorder index {h.pid}')
        obj = t.obj
        for k in ('boundingVolume', 'geometricError', 'transform'):
            if not deq(obj.get(k), h.tile.get(k)):
                report.bad('A4.explicitTiles', f'implicit root p{h.pid}: {k} differs from the producer tile')
        if t.refine != h.refine:
            report.bad('A4.explicitTiles', f'implicit root p{h.pid}: refine {t.refine!r} != {h.refine!r}')
        if 'extras' in obj and not deq(obj['extras'], h_extras(h)):
            report.bad('A4.extras', f'implicit root p{h.pid}: tile extras differ from the producer extras')
        extra_keys = set(h.tile) - {'boundingVolume', 'geometricError', 'refine', 'transform', 'content', 'contents', 'extras'}
        if extra_keys:
            report.bad('A4.explicitTiles', f'implicit root p{h.pid}: producer keys {sorted(extra_keys)} are not represented')
        if not r.ok:
            report.bad('A4.tree', f'implicit root p{h.pid} could not be expanded')
            return
        # walk H below the root: coordinates, delta, slot list
        slots = {}
        expected = []
        stack = [(h, 0, (0,) * r.dims, (0.0, 0.0, 0.0))]
        while stack:
            n, level, xyz, delta = stack.pop()
            expected.append((n, level, xyz, delta))
            if n is not h and set(n.tile) - {'boundingVolume', 'geometricError', 'refine', 'transform', 'content', 'contents', 'extras'}:
                report.bad('A4.explicitTiles', f'tile p{n.pid}: producer keys not representable in a row')
            if n.refine != h.refine:
                report.bad('A4.explicitTiles', f'tile p{n.pid}: refine differs below its implicit root (unrepresentable, R-5)')
            for i, c, m in h_contents(n):
                suffix = (c.get('uri') or '').rsplit('.', 1)[-1].lower()
                slots.setdefault((i, suffix), {k: v for k, v in c.items() if k != 'uri'})
            for c in n.children:
                tr, ok = translation_of(c.tile)
                if not ok:
                    report.bad('A4.explicitTiles', f'tile p{c.pid}: non-translation transform below an implicit root (unrepresentable)')
                    tr = (0.0, 0.0, 0.0)
                if not 0 <= c.slot < 2 ** r.dims:
                    report.bad('A4.tree', f'tile p{c.pid}: slot {c.slot} outside the branching factor')
                    continue
                d = tuple(delta[k] + tr[k] for k in range(3))
                stack.append((c, level + 1, child_xyz(xyz, c.slot), d))
        slot_list = sorted(slots)
        got = [(s, x) for _, s, x, _ in r.templates]
        if [x for _, x in slot_list] != [x for _, x in got] or not all(g[0] in (i, k) for k, (g, (i, _)) in enumerate(zip(got, slot_list))):
            report.bad('A4.payloads', f'implicit root p{h.pid}: templates {got} do not follow the slot list {slot_list}')
        for k, key in enumerate(slot_list):
            if k < len(r.templates):
                header = {kk: v for kk, v in (r.templates[k][3] or {}).items() if kk != 'uri'}
                if not deq(header, slots[key]):
                    report.bad('A4.explicitTiles', f'implicit root p{h.pid}: content header for slot {key} differs')
        slot_index = {key: k for k, key in enumerate(slot_list)}
        seen = set()
        for n, level, xyz, delta in expected:
            ft = r.tiles.get((level, xyz))
            if ft is None:
                report.bad('A4.tree', f'tile p{n.pid} (level {level} {xyz} under r{r.n}) is not available in the forest')
                continue
            seen.add((level, xyz))
            mapped[id(ft)] = n
            hb = n.tile.get('boundingVolume', {}).get('box') if isinstance(n.tile.get('boundingVolume'), dict) else None
            ex = None
            if not (isinstance(hb, list) and len(hb) == 12):
                report.bad('A4.rows', f'tile p{n.pid}: producer box missing (unrepresentable)')
            elif ft.row is None:
                report.bad('A4.rows', f'tile p{n.pid}: no row')
            else:
                box, err, ex = ft.row
                # ruling Q1: delta = 0 copies B bit for bit (-0.0 stays -0.0); otherwise B + delta in f64
                zero = all(v == 0.0 for v in delta)
                exp_box = list(hb) if (n is h or zero) else [float(hb[k]) + delta[k] for k in range(3)] + list(hb[3:])
                if not all(same_f64(a, b) for a, b in zip(exp_box, box)):
                    report.bad('A4.rows', f'tile p{n.pid}: row box {list(box)} != B + delta {exp_box}')
                if not same_f64(err, n.tile.get('geometricError')):
                    report.bad('A4.rows', f'tile p{n.pid}: row error {err!r} != {n.tile.get("geometricError")!r}')
            # contents and payloads
            hcs = h_contents(n)
            fcs = {li: m for li, m, _ in ft.contents}
            want = {}
            for i, c, m in hcs:
                suffix = (c.get('uri') or '').rsplit('.', 1)[-1].lower()
                want[slot_index[(i, suffix)]] = m
            if set(want) != set(fcs):
                report.bad('A4.payloads', f'tile p{n.pid}: content layers {sorted(fcs)} != expected {sorted(want)}')
            written = 0
            for li, hm in sorted(want.items()):
                fm = fcs.get(li)
                if fm is None or fm not in f.members or hm is None:
                    continue
                raw, pub = hz.read(hm), f.read_once(fm)
                written += len(pub)
                implicit_raw_hashes.add(hashlib.sha256(raw).hexdigest())
                problems = compare_payload(raw, pub, delta if n is not h else (0.0, 0.0, 0.0), fm.rsplit('.', 1)[-1].lower())
                if n is h and pub != raw:
                    problems.append('the implicit root payload is not a byte copy (delta = 0)')
                for p in problems:
                    report.bad('A4.payloads', f'tile p{n.pid} {fm}: {p}')
            # extras
            hex_ = h_extras(n)
            hex_ = {} if hex_ is None else hex_
            if ft.row is None:
                pass
            elif ex is None:
                if hex_:
                    report.bad('A4.extras', f'tile p{n.pid}: row has no extras but the producer tile has {sorted(hex_)}')
            else:
                exp = dict(hex_) if isinstance(hex_, dict) else hex_
                if isinstance(exp, dict) and 'encodedBytes' in exp and want:
                    exp['encodedBytes'] = written
                if not deq(ex, exp):
                    keys = sorted(k for k in set(ex) | set(exp) if not deq(ex.get(k), exp.get(k))) if isinstance(ex, dict) and isinstance(exp, dict) else '?'
                    report.bad('A4.extras', f'tile p{n.pid}: row extras differ in {keys}')
        extra = set(r.tiles) - seen
        if extra:
            report.bad('A4.tree', f'implicit root r{r.n}: {len(extra)} available tiles have no producer tile')

    def match(h, t):
        mapped[id(t)] = h
        hr, fr = h.role, forest_role(t)
        if hr != fr:
            report.bad('A4.roles', f'tile p{h.pid}: role {fr} in the forest, {hr} under S = {S}')
        if t.kind == 'root':
            compare_root(h, t)
            return
        if hr == 'root':
            report.bad('A4.tree', f'tile p{h.pid}: expected an implicit root, found an explicit tile')
        compare_explicit(h, t)
        # children: match by box, error and transform (explicit children carry no slot)
        def fsig(c):
            tr = (c.link[0].get('transform') if c.link else c.obj.get('transform'))
            return signature(c.obj, tr)
        # pass 1: box, error and transform; pass 2: box and error (a tile-level difference is then reported
        # by the comparison, not as a missing tile); pass 3: remaining children in order
        pending_h, pending_f = list(h.children), list(t.children)
        for key_h, key_f in ((lambda c: signature(c.tile, c.tile.get('transform')), fsig),
                             (lambda c: signature(c.tile, None), lambda c: signature(c.obj, None)),
                             (lambda c: 0, lambda c: 0)):
            groups = {}
            for c in pending_f:
                groups.setdefault(key_f(c), []).append(c)
            rest_h = []
            for hc in pending_h:
                cands = groups.get(key_h(hc))
                if cands:
                    match(hc, cands.pop(0))
                else:
                    rest_h.append(hc)
            pending_h, pending_f = rest_h, [c for v in groups.values() for c in v]
            if pending_h and pending_f and key_h(pending_h[0]) == 0:
                break
        for hc in pending_h:
            report.bad('A4.tree', f'tile p{hc.pid} (child of p{h.pid}) has no forest counterpart')
        if pending_f:
            report.bad('A4.tree', f'tile p{h.pid}: {len(pending_f)} forest children have no producer counterpart')

    match(hroot, froot)
    if reuse_map is not None:
        keys = set(reuse_map)
        missing = implicit_raw_hashes - keys - explicit_raw_hashes
        if missing:
            report.bad('vectorBuild.reuseMap', f'{len(missing)} raw payloads published only under implicit-content/ have no payloads entry (e.g. {sorted(missing)[0]})')
        extra = keys - implicit_raw_hashes
        if extra:
            report.bad('vectorBuild.reuseMap', f'{len(extra)} payloads entries name no implicit tile payload of H')
    return {'referenceTiles': len(hpost), 'referenceLevels': hroot.h + 1, 'referenceSha256': sha256_file(ref_path)}


def sha256_file(path):
    h = hashlib.sha256()
    with open(path, 'rb') as fh:
        for block in iter(lambda: fh.read(1 << 20), b''):
            h.update(block)
    return h.hexdigest()


def stats(f):
    if getattr(f, 'root', None) is None:
        return {}
    tiles = [t for t in f.tiles]
    explicit = [t for t in tiles if t.kind == 'explicit']
    skel = [t for t in explicit if t.children or t.link]
    implicit = [t for t in tiles if t.kind in ('root', 'implicit')]
    docs = []
    for name in f.doc_order:
        obj, data = f.docs[name]
        n, d = nodes_depth(obj)
        docs.append((n, len(data), d))
    return {'tiles': len(tiles), 'levels': f.root.h + 1, 'skeletonTiles': len(skel), 'explicitLeaves': len(explicit) - len(skel),
            'implicitRoots': len(f.roots), 'implicitTiles': len(implicit), 'linkTiles': len(f.links), 'documents': len(f.doc_order),
            'maxLinkNesting': max(f.nesting.values(), default=0),
            'subtrees': sum(1 for r in f.roots if r.subtree_member), 'members': len(f.members),
            'maxDocumentNodes': max(x[0] for x in docs), 'maxDocumentBytes': max(x[1] for x in docs), 'maxDocumentJsonDepth': max(x[2] for x in docs),
            'rootDocument': {'nodes': docs[0][0], 'bytes': docs[0][1], 'jsonDepth': docs[0][2]},
            # I1 v3.1 counting as modelled in design §5.1 (evidence/v2/forest_model.py): inputs for A-3, not a rule
            'i1TilesCounter': len(tiles) + len(f.links),
            'i1HierarchyVisits': 2 * len(f.doc_order) + len(explicit) + len(f.links) + len(f.roots) + len(implicit)}


def run(path, reference=None, examples=8):
    report = Report(examples)
    f = None
    info = {}
    try:
        f = Forest(path, report)
        f.root = None
        root = check_archive(f, report)
        f.root = root
        if root is not None:
            reuse = check_reuse_map(f, report)
            if reference:
                info = check_reference(f, report, reference, reuse)
    except zipfile.BadZipFile as e:
        report.bad('archive', f'not a zip archive: {e}')
    except (ValueError, FormatError, KeyError, TypeError, AttributeError, IndexError, RecursionError, struct.error) as e:
        # a malformed archive must yield a failing verdict, never a crash; the exception is recorded
        report.bad('archive', f'unreadable: {type(e).__name__}: {e}')
    verdict, failed, rules = report.verdict()
    me = __file__
    return {'checker': 'forest_check.py', 'checkerSha256': sha256_file(me), 'archive': str(path), 'archiveSha256': sha256_file(path),
            'reference': str(reference) if reference else None, **info,
            'profile': {'S': S, 'DOC_NODES': DOC_NODES, 'DOC_BYTES': DOC_BYTES, 'DOC_LEVELS': DOC_LEVELS, 'JSON_DEPTH': JSON_DEPTH, 'MAX_LEVELS': MAX_LEVELS},
            'verdict': verdict, 'failedRules': failed, 'rules': rules, 'stats': stats(f) if f else {}}


def main():
    ap = argparse.ArgumentParser(description=__doc__.split('\n')[0])
    ap.add_argument('archive')
    ap.add_argument('--explicit', help='--explicit build of the same input (the producer hierarchy H) for A-4')
    ap.add_argument('--out', help='write the verdict JSON here (default: stdout)')
    ap.add_argument('--examples', type=int, default=8, help='violation examples kept per rule')
    a = ap.parse_args()
    sys.setrecursionlimit(10000)
    result = run(a.archive, a.explicit, a.examples)
    text = json.dumps(result, indent=1)
    if a.out:
        with open(a.out, 'w') as fh:
            fh.write(text + '\n')
        print(json.dumps({'archive': a.archive, 'verdict': result['verdict'], 'failedRules': result['failedRules']}))
    else:
        print(text)
    sys.exit(0 if result['verdict'] == 'pass' else 1)


if __name__ == '__main__':
    main()
