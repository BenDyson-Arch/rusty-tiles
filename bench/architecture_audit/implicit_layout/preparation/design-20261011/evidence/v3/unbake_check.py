"""Can reuse recover each raw vector payload exactly from the payload the archive actually publishes?

Uses develop default (implicit) archives, which still contain both the raw `t/<sha256>` payloads
(retained) and the production-baked `implicit-content/*` payloads written by today's
`translate_content`. That makes the check run on production bytes. For every content tile:

  - the GLB BIN chunk must be byte-identical (translate_content touches only the JSON chunk);
  - splice: replacing the baked root node's `translation` array text by the raw one (or deleting the
    key when raw has none) must give the raw JSON chunk byte for byte. That means a restore that
    re-encodes with the same serializer and the recorded raw root translation is exact;
  - naive: is raw_translation == baked_translation - delta in f64 (no recorded value)?
  - sha256(raw) must equal the immutable name.

Delta per tile is today's global delta: child translations summed root to tile in f64, in path order
(implicit/tileset.rs collect), applied as (d0, d2, -d1).

Usage: python3 -I unbake_check.py ARCHIVE.3tz [...]  -> JSON on stdout
"""
import hashlib, json, pathlib, re, struct, sys, time, zipfile

HERE = pathlib.Path(__file__).resolve().parent
probe = {'__name__': 'layout_probe'}
exec(compile((HERE.parent / 'layout_probe.py').read_text(), 'layout_probe.py', 'exec'), probe)


def chunks(glb):
    jl = struct.unpack_from('<I', glb, 12)[0]
    return glb[20:20 + jl], glb[20 + jl:]


def root_translation_text(js):
    """Text span of the scene-root node's "translation" member (vector GLBs have one node)."""
    doc = json.loads(js)
    roots = sorted({i for s in doc['scenes'] for i in s['nodes']})
    assert roots == [0] and len(doc['nodes']) == 1, 'vector payload has one root node'
    m = list(re.finditer(rb'"translation":\[[^\]]*\]', js))
    assert len(m) <= 1
    return doc['nodes'][0].get('translation'), (m[0].span() if m else None)


def check(path):
    z = zipfile.ZipFile(path)
    state = json.loads(z.read('vector-build.json'))
    explicit = state['explicitRoot']
    top, tiles, dims, _ = probe['expand'](z)
    out = {'archive': str(path), 'sha256': hashlib.sha256(open(path, 'rb').read()).hexdigest(), 'payloads': 0,
           'binIdentical': 0, 'spliceExact': 0, 'naiveExact': 0, 'rawHasRootTranslation': 0, 'hashOk': 0,
           'deltaZero': 0, 'failures': []}
    t0 = time.process_time()

    def walk(node, level, xyz, delta):
        c = node.get('content')
        if c:
            raw = z.read(c['uri'])
            member = [u for _, u in tiles[(level, xyz)]['uris']][0]
            baked = z.read(member)
            out['payloads'] += 1
            out['hashOk'] += hashlib.sha256(raw).hexdigest() == c['uri'].split('/')[-1].split('.')[0]
            rj, rb_ = chunks(raw); bj, bb = chunks(baked)
            out['binIdentical'] += rb_ == bb
            rt, rspan = root_translation_text(rj.rstrip(b' '))
            bt, bspan = root_translation_text(bj.rstrip(b' '))
            out['rawHasRootTranslation'] += rt is not None
            if delta == [0.0, 0.0, 0.0]:
                out['deltaZero'] += 1
                ok = baked == raw
                out['spliceExact'] += ok; out['naiveExact'] += ok
            else:
                braw = bj.rstrip(b' ')
                if rt is None:
                    spliced = braw[:bspan[0] - 1] + braw[bspan[1]:] if braw[bspan[0] - 1:bspan[0]] == b',' else braw[:bspan[0]] + braw[bspan[1] + 1:]
                else:
                    rtxt = rj.rstrip(b' ')[rspan[0]:rspan[1]]
                    spliced = braw[:bspan[0]] + rtxt + braw[bspan[1]:]
                ok = spliced == rj.rstrip(b' ')
                out['spliceExact'] += ok
                d = [delta[0], delta[2], -delta[1]]
                base = rt if rt is not None else [0.0, 0.0, 0.0]
                naive = [bt[i] - d[i] for i in range(3)]
                out['naiveExact'] += naive == [float(v) for v in base]
                if not ok and len(out['failures']) < 5:
                    out['failures'].append(member)
        for i, ch in enumerate(node.get('children', [])):
            s = ch.get('extras', {}).get('implicitChildIndex', i)
            t = ch.get('transform')
            d = list(delta)
            if t:
                for k in range(3):
                    d[k] += t[12 + k]
            walk(ch, level + 1, tuple((xyz[k] << 1) | ((s >> k) & 1) for k in range(dims)), d)
    walk(explicit, 0, (0,) * dims, [0.0, 0.0, 0.0])
    out['pythonCpuSeconds'] = round(time.process_time() - t0, 3)
    return out


if __name__ == '__main__':
    res = [check(p) for p in sys.argv[1:]]
    print(json.dumps({'script': hashlib.sha256((HERE / 'unbake_check.py').read_bytes()).hexdigest(), 'results': res}, indent=1))
