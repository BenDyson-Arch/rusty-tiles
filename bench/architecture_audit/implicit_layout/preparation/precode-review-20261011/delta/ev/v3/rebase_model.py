"""Model of the v3 reversible payload rebase (stdlib only).

Production today (`translate_content`) parses the whole GLB JSON chunk into a serde Value and
re-serializes it (sorted keys). That is not reversible for meshopt payloads, whose raw JSON key order
is not sorted (unbake_check.py: 0/544 splice-exact). v3 rebases by replacing only the scene-root node
object's text:

  bake(raw, d):   root node text N -> serialize(node(N) with translation t + (d0, d2, -d1)), where
                  t is N's translation (or 0); the rest of the JSON chunk is byte-identical; lengths
                  and padding are recomputed.
  unbake(baked, N, L): replace the baked root node text with the recorded raw node text N and pad
                  the JSON chunk with spaces to the recorded raw JSON chunk length L (vector raw JSON
                  chunks are 8-aligned; production encode_glb pads to 4).

vector-build.json records, per distinct raw payload hash: published member, delta, raw root node
text and raw JSON chunk length. This script applies bake then unbake to every raw payload of develop archives with the production
delta, and checks that:
  (1) unbake(bake(raw)) == raw byte for byte, and sha256 equals the immutable name;
  (2) bake(raw) parses to the same JSON as production's baked payload (same translation values);
  (3) the BIN chunk is untouched.
It also times both steps (Python, an upper bound for Rust).

Usage: python3 -I rebase_model.py ARCHIVE.3tz [...] -> JSON
"""
import hashlib, json, pathlib, struct, sys, time, zipfile

HERE = pathlib.Path(__file__).resolve().parent
probe = {'__name__': 'layout_probe'}
exec(compile((HERE.parent / 'layout_probe.py').read_text(), 'layout_probe.py', 'exec'), probe)


def skip_ws(s, i):
    while s[i] in b' \t\r\n':
        i += 1
    return i


def value_end(s, i):
    """End index (exclusive) of the JSON value starting at s[i]."""
    c = s[i:i + 1]
    if c == b'"':
        i += 1
        while True:
            if s[i:i + 1] == b'\\':
                i += 2
            elif s[i:i + 1] == b'"':
                return i + 1
            else:
                i += 1
    if c in (b'{', b'['):
        depth, inside = 0, False
        while True:
            ch = s[i:i + 1]
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
    while s[j:j + 1] not in (b',', b'}', b']', b''):
        j += 1
    return j


def root_node_span(js):
    """Span of nodes[k] where k is the single scene root, located in the top-level object."""
    doc = json.loads(js)
    roots = sorted({n for sc in doc['scenes'] for n in sc['nodes']})
    assert len(roots) == 1
    k = roots[0]
    i = skip_ws(js, 0); assert js[i:i + 1] == b'{'
    i += 1
    while True:
        i = skip_ws(js, i)
        kend = value_end(js, i)
        key = json.loads(js[i:kend])
        i = skip_ws(js, kend); assert js[i:i + 1] == b':'
        i = skip_ws(js, i + 1)
        vend = value_end(js, i)
        if key == 'nodes':
            j = skip_ws(js, i + 1); n = 0
            while True:
                e = value_end(js, j)
                if n == k:
                    return j, e
                n += 1
                j = skip_ws(js, e); assert js[j:j + 1] == b','
                j = skip_ws(js, j + 1)
        i = skip_ws(js, vend)
        if js[i:i + 1] == b',':
            i += 1


def with_json(glb, js, length=None):
    jl = struct.unpack_from('<I', glb, 12)[0]
    rest = glb[20 + jl:]
    if length is None:   # keep the source chunk's alignment: vector raw JSON chunks are 8-aligned
        align = 8 if jl % 8 == 0 else 4
        length = len(js) + (-len(js) % align)
    assert length >= len(js)
    js = js + b' ' * (length - len(js))
    total = 20 + len(js) + len(rest)
    return glb[:4] + struct.pack('<I', 2) + struct.pack('<I', total) + struct.pack('<I', len(js)) + b'JSON' + js + rest


def json_chunk(glb):
    jl = struct.unpack_from('<I', glb, 12)[0]
    return glb[20:20 + jl].rstrip(b' '), glb[20 + jl:]


def bake(raw, delta):
    js, _ = json_chunk(raw)
    a, b = root_node_span(js)
    node_text = js[a:b]
    node = json.loads(node_text)
    t = node.get('translation', [0.0, 0.0, 0.0])
    d = [delta[0], delta[2], -delta[1]]
    node['translation'] = [t[i] + d[i] for i in range(3)]
    new = json.dumps(node, separators=(',', ':'), sort_keys=True).encode()
    return with_json(raw, js[:a] + new + js[b:]), (node_text, struct.unpack_from('<I', raw, 12)[0])


def unbake(baked, record):
    node_text, raw_json_length = record
    js, _ = json_chunk(baked)
    a, b = root_node_span(js)
    return with_json(baked, js[:a] + node_text + js[b:], raw_json_length)


def check(path):
    z = zipfile.ZipFile(path)
    explicit = json.loads(z.read('vector-build.json'))['explicitRoot']
    top, tiles, dims, _ = probe['expand'](z)
    out = {'archive': str(path), 'sha256': hashlib.sha256(open(path, 'rb').read()).hexdigest(), 'payloads': 0,
           'roundTripExact': 0, 'hashOk': 0, 'semanticallyEqualToProductionBake': 0, 'binUntouched': 0,
           'recordedNodeTextBytesMax': 0, 'bakeSeconds': 0.0, 'unbakeSeconds': 0.0}

    def walk(node, level, xyz, delta):
        c = node.get('content')
        if c:
            raw = z.read(c['uri'])
            prod = z.read([u for _, u in tiles[(level, xyz)]['uris']][0])
            t0 = time.process_time(); baked, node_text = bake(raw, delta); t1 = time.process_time()
            back = unbake(baked, node_text); t2 = time.process_time()
            out['bakeSeconds'] += t1 - t0; out['unbakeSeconds'] += t2 - t1
            out['payloads'] += 1
            out['roundTripExact'] += back == raw
            out['hashOk'] += hashlib.sha256(back).hexdigest() == c['uri'].split('/')[-1].split('.')[0]
            out['semanticallyEqualToProductionBake'] += json.loads(json_chunk(baked)[0]) == json.loads(json_chunk(prod)[0])
            out['binUntouched'] += json_chunk(baked)[1] == json_chunk(raw)[1]
            out['recordedNodeTextBytesMax'] = max(out['recordedNodeTextBytesMax'], len(node_text[0]))
        for i, ch in enumerate(node.get('children', [])):
            s = ch.get('extras', {}).get('implicitChildIndex', i)
            t = ch.get('transform'); d = list(delta)
            if t:
                for k in range(3):
                    d[k] += t[12 + k]
            walk(ch, level + 1, tuple((xyz[k] << 1) | ((s >> k) & 1) for k in range(dims)), d)
    walk(explicit, 0, (0,) * dims, [0.0, 0.0, 0.0])
    out['bakeSeconds'] = round(out['bakeSeconds'], 3); out['unbakeSeconds'] = round(out['unbakeSeconds'], 3)
    return out


if __name__ == '__main__':
    res = [check(p) for p in sys.argv[1:]]
    print(json.dumps({'script': hashlib.sha256((HERE / 'rebase_model.py').read_bytes()).hexdigest(), 'results': res}, indent=1))
