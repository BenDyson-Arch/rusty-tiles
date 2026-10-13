#!/usr/bin/env python3
"""H7 sensitive controls: legacy 3D Tiles 1.0 content (b3dm, i3dm, pnts, cmpt) is Unsupported.

    python3 -I generate.py [--out DIR]     # default: ./fixtures next to this script

Standard library only. No repository import, no producer, no target execution. Every
archive is a stored ZIP with an independently computed `@3dtilesIndex1@` (MD5 of the
name, little-endian local-header offset, sorted as two little-endian u64). Output is
byte-deterministic: fixed timestamps, fixed member order, no randomness.

The expectations in `fixtures/manifest.json` are declared here, before any target or
oracle runs, from the PROPOSED rule (H7 amendment in docs/architecture/
c1-legacy-content-unsupported-amendment.md):

  L0  member existence (archive catalog lookup, zero content bytes) precedes L1;
  L1  a payload member whose name has the ASCII case-insensitive suffix .b3dm, .i3dm,
      .pnts or .cmpt is Unsupported before any of its bytes are read;
  L2  otherwise, after the existing bounded member read and before any format parse,
      a payload whose first four bytes are b3dm/i3dm/pnts/cmpt is Unsupported;
  T   an implicit content template whose normalized last segment ends in one of the
      four suffixes is Unsupported at root admission (I1 step 4b), before any subtree
      read, like a JSON content template.

Each legacy verdict is Unsupported (CLI code `unsupported`, exit 2) and its message
names the format and the member. Nothing inside a legacy payload is parsed: corrupt
headers, bad tables, RTC_CENTER and cmpt inner tiles all give the same verdict.

`developCategory` records the PREDICTED category of the pre-H7 validator
(origin/develop 87b7078); check.py --develop compares it. Cases whose prediction
differs from `expectedCategory` are the sensitivity of this lane.
"""
import argparse
import hashlib
import io
import json
import struct
import sys
import zipfile
from pathlib import Path

HERE = Path(__file__).resolve().parent
EPOCH = (1980, 1, 1, 0, 0, 0)
LEGACY = ('b3dm', 'i3dm', 'pnts', 'cmpt')


def sha(raw):
    return hashlib.sha256(raw).hexdigest()


def packed(obj):
    return json.dumps(obj, separators=(',', ':'), allow_nan=False).encode()


def pad(raw, unit, fill=b' '):
    return raw + fill * (-len(raw) % unit)


# ------------------------------------------------------------------ glTF 2.0

DOC = {'asset': {'version': '2.0'}, 'scene': 0, 'scenes': [{'nodes': [0]}], 'nodes': [{'mesh': 0}],
       'buffers': [{'byteLength': 42}],
       'bufferViews': [{'buffer': 0, 'byteLength': 36}, {'buffer': 0, 'byteOffset': 36, 'byteLength': 6}],
       'accessors': [{'bufferView': 0, 'componentType': 5126, 'count': 3, 'type': 'VEC3',
                      'min': [0, 0, 0], 'max': [1, 1, 0]},
                     {'bufferView': 1, 'componentType': 5123, 'count': 3, 'type': 'SCALAR'}],
       'meshes': [{'primitives': [{'attributes': {'POSITION': 0}, 'indices': 1, 'mode': 4}]}]}
BIN = struct.pack('<9f3H', 0, 0, 0, 1, 0, 0, 0, 1, 0, 0, 1, 2)


def glb(doc=DOC, binary=BIN):
    text = pad(packed(doc), 4)
    body = pad(binary, 4, b'\0')
    chunks = struct.pack('<I4s', len(text), b'JSON') + text + struct.pack('<I4s', len(body), b'BIN\0') + body
    return struct.pack('<4sII', b'glTF', 2, 12 + len(chunks)) + chunks


# ------------------------------------------------- 3D Tiles 1.0 legacy formats
# Layouts from the 3D Tiles 1.0 specification (b3dm, i3dm, pnts, cmpt): every table
# boundary 8-byte aligned, JSON padded with spaces, binary with zeros.

def tables(feature, feature_bin=b'', batch=None, header=28):
    ft = packed(feature)
    ft += b' ' * (-(header + len(ft)) % 8)
    fb = pad(feature_bin, 8, b'\0')
    bt = b'' if batch is None else pad(packed(batch), 8)
    return ft, fb, bt


def b3dm(inner=None, feature=None, version=1):
    inner = glb() if inner is None else inner
    ft, fb, bt = tables(feature or {'BATCH_LENGTH': 0})
    body = ft + fb + bt + inner
    body += b'\0' * (-(28 + len(body)) % 8)
    return struct.pack('<4s6I', b'b3dm', version, 28 + len(body), len(ft), len(fb), len(bt), 0) + body


def i3dm(inner=None):
    inner = glb() if inner is None else inner
    ft, fb, bt = tables({'INSTANCES_LENGTH': 1, 'POSITION': {'byteOffset': 0}},
                        struct.pack('<3f', 0, 0, 0), header=32)
    body = pad(ft + fb + bt + inner, 8, b'\0')
    # gltfFormat 1: the body ends with an embedded binary glTF.
    return struct.pack('<4s7I', b'i3dm', 1, 32 + len(body), len(ft), len(fb), len(bt), 0, 1) + body


def pnts():
    ft, fb, bt = tables({'POINTS_LENGTH': 1, 'POSITION': {'byteOffset': 0}}, struct.pack('<3f', 0, 0, 0))
    body = ft + fb + bt
    body += b'\0' * (-(28 + len(body)) % 8)
    return struct.pack('<4s6I', b'pnts', 1, 28 + len(body), len(ft), len(fb), len(bt), 0) + body


def cmpt(inner_tiles):
    body = b''.join(pad(tile, 8, b'\0') for tile in inner_tiles)
    return struct.pack('<4s3I', b'cmpt', 1, 16 + len(body), len(inner_tiles)) + body


# ------------------------------------------------------------- tilesets, 3TZ

BOX = [0, 0, 0, 2000, 0, 0, 0, 2000, 0, 0, 0, 2000]


def tile(uri=None, children=None, box=None):
    t = {'boundingVolume': {'box': box or BOX}, 'geometricError': 0}
    if uri is not None:
        t['content'] = {'uri': uri}
    if children:
        t['geometricError'] = 1
        t['refine'] = 'ADD'
        t['children'] = children
    return t


def tileset(root, **extra):
    root = dict(root)
    root.setdefault('refine', 'REPLACE')
    doc = {'asset': {'version': '1.1'}, 'geometricError': 2, 'root': root}
    doc.update(extra)
    return packed(doc)


def implicit_tileset(template, subtree_template='s/{level}/{x}/{y}.subtree'):
    return tileset(implicit_root(template, subtree_template), geometricError=32)


def implicit_root(template, subtree_template='s/{level}/{x}/{y}.subtree'):
    return {'boundingVolume': {'box': [0, 0, 0, 8, 0, 0, 0, 8, 0, 0, 0, 8]}, 'geometricError': 16,
            'refine': 'REPLACE', 'content': {'uri': template},
            'implicitTiling': {'subdivisionScheme': 'QUADTREE', 'subtreeLevels': 1, 'availableLevels': 1,
                               'subtrees': {'uri': subtree_template}}}


def subtree():
    """3D Tiles 1.1 binary subtree: one available tile with content, no child subtrees."""
    text = pad(packed({'tileAvailability': {'constant': 1}, 'contentAvailability': [{'constant': 1}],
                       'childSubtreeAvailability': {'constant': 0}}), 8)
    return struct.pack('<4sIQQ', b'subt', 1, len(text), 0) + text


def archive(members):
    """Stored ZIP plus the 3TZ index, built and checked independently of any project reader."""
    stream = io.BytesIO()
    records = []
    with zipfile.ZipFile(stream, 'w', compression=zipfile.ZIP_STORED) as target:
        for name, raw in members:
            records.append((hashlib.md5(name.encode()).digest(), stream.tell()))
            info = zipfile.ZipInfo(name, EPOCH)
            info.external_attr = 0o100644 << 16
            target.writestr(info, raw)
        records.sort(key=lambda row: struct.unpack('<QQ', row[0]))
        info = zipfile.ZipInfo('@3dtilesIndex1@', EPOCH)
        info.external_attr = 0o100644 << 16
        target.writestr(info, b''.join(d + struct.pack('<Q', o) for d, o in records))
    raw = stream.getvalue()
    for _, offset in records:
        assert raw[offset:offset + 4] == b'PK\x03\x04'
    return raw


# ------------------------------------------------------------------- cases

def legacy(fmt, member, rule):
    """Expectation of an H7 Unsupported verdict: the message names the format and member."""
    return dict(expectedCategory='unsupported', rule=rule, messageIncludes=['legacy', fmt, member])


def cases():
    out = []

    def case(name, members, expect, develop, oracle, fact):
        out.append(dict(name=name, members=members, expect=expect, develop=develop, oracle=oracle, fact=fact))

    ok = dict(expectedCategory='admitted', rule='none', messageIncludes=[])

    def single(uri, data):
        return [('tileset.json', tileset(tile(uri))), (uri, data)]

    # -- negative controls: glTF stays admitted; nothing is legacy by substring.
    case('glb-triangle', single('tile.glb', glb()), ok, 'admitted', 'Contained',
         'Baseline: a GLB 2.0 triangle inside the root box.')
    case('glb-legacy-words-in-path-and-json', single('b3dm.cmpt/pnts.i3dm.glb',
         glb(dict(DOC, asset={'version': '2.0', 'generator': 'b3dm i3dm pnts cmpt'}))), ok, 'admitted', 'Contained',
         'Legacy words in a directory, inside the stem and in JSON: only the final suffix and the first four bytes classify.')
    case('subtree-template-legacy-suffix',
         [('tileset.json', implicit_tileset('c/{level}/{x}/{y}.glb', 's/{level}/{x}/{y}.b3dm')),
          ('s/0/0/0.b3dm', subtree()), ('c/0/0/0.glb', glb())], ok, 'admitted', 'Contained',
         'Legacy detection applies to content templates only; a subtree template is not content (like JSON detection).')
    case('implicit-glb', [('tileset.json', implicit_tileset('c/{level}/{x}/{y}.glb')),
                          ('s/0/0/0.subtree', subtree()), ('c/0/0/0.glb', glb())], ok, 'admitted', 'Contained',
         'Baseline implicit root: one available tile with GLB content.')
    case('unknown-suffix-bin', single('tile.bin', glb()),
         dict(expectedCategory='unsupported', rule='generic', messageIncludes=['outside GLB/local glTF']),
         'unsupported', 'Undecided',
         'An unknown suffix keeps the existing generic Unsupported message; it is not reported as legacy.')
    case('vctr-suffix', single('tile.vctr', b'vctr' + bytes(28)),
         dict(expectedCategory='unsupported', rule='generic', messageIncludes=['outside GLB/local glTF']),
         'unsupported', 'Undecided',
         'The retired vector draft format is outside the four legacy formats; generic Unsupported.')

    # -- L1, valid legacy payloads by suffix.
    case('b3dm-valid', single('tile.b3dm', b3dm()), legacy('b3dm', 'tile.b3dm', 'L1'), 'admitted', 'Unsupported',
         'Valid modern b3dm v1 wrapping the baseline GLB; develop unwraps and admits it.')
    case('b3dm-uppercase-suffix', single('tile.B3DM', b3dm()), legacy('b3dm', 'tile.B3DM', 'L1'), 'admitted',
         'Unsupported', 'The suffix comparison is ASCII case-insensitive.')
    case('i3dm-valid', single('tile.i3dm', i3dm()), legacy('i3dm', 'tile.i3dm', 'L1'), 'unsupported', 'Unsupported',
         'Valid i3dm v1 (gltfFormat 1, one instance) embedding the baseline GLB.')
    case('pnts-valid', single('tile.pnts', pnts()), legacy('pnts', 'tile.pnts', 'L1'), 'unsupported', 'Unsupported',
         'Valid pnts v1 with one POSITION point.')
    case('cmpt-b3dm', single('tile.cmpt', cmpt([b3dm()])), legacy('cmpt', 'tile.cmpt', 'L1'), 'unsupported',
         'Unsupported', 'cmpt v1 with one inner b3dm; inner tiles are never inspected.')
    case('cmpt-wrapping-glb', single('tile.cmpt', cmpt([glb()])), legacy('cmpt', 'tile.cmpt', 'L1'), 'unsupported',
         'Unsupported', 'cmpt whose single inner tile is a GLB: Unsupported as cmpt, the GLB is not unwrapped.')

    # -- L1, corrupt legacy payloads: no partial inspection, so never InvalidInput.
    case('b3dm-magic-only', single('tile.b3dm', b'b3dm'), legacy('b3dm', 'tile.b3dm', 'L1'), 'invalid_input',
         'Unsupported', 'Four-byte magic without a header; develop reports truncated framing.')
    case('b3dm-version-2', single('tile.b3dm', b3dm(version=2)), legacy('b3dm', 'tile.b3dm', 'L1'), 'unsupported',
         'Unsupported', 'Unknown version: the legacy verdict, not the old version message.')
    bad = bytearray(b3dm())
    bad[28] = ord('[')
    case('b3dm-malformed-feature-json', single('tile.b3dm', bytes(bad)), legacy('b3dm', 'tile.b3dm', 'L1'),
         'invalid_input', 'Unsupported', 'Feature-table JSON is malformed; develop parses it.')
    case('b3dm-rtc-center', single('tile.b3dm', b3dm(feature={'BATCH_LENGTH': 0, 'RTC_CENTER': [1, 2, 3]})),
         legacy('b3dm', 'tile.b3dm', 'L1'), 'admitted', 'Unsupported',
         'RTC_CENTER no longer reaches C2: develop admits with notInspected decodedContentBounds.rtcCenter.')
    case('cmpt-inner-overclaim', single('tile.cmpt', cmpt([b3dm()])[:-8]), legacy('cmpt', 'tile.cmpt', 'L1'),
         'unsupported', 'Unsupported', 'Truncated cmpt: still the legacy verdict.')

    # -- L2, legacy bytes under a glTF suffix.
    for fmt, data in (('b3dm', b3dm()), ('i3dm', i3dm()), ('pnts', pnts()), ('cmpt', cmpt([glb()]))):
        case('glb-holding-' + fmt, single('tile.glb', data), legacy(fmt, 'tile.glb', 'L2'), 'invalid_input',
             'Unsupported', 'A .glb member whose first four bytes are %s; develop fails the GLB envelope.' % fmt)
    case('gltf-holding-b3dm', single('tile.gltf', b3dm()), legacy('b3dm', 'tile.gltf', 'L2'), 'invalid_input',
         'Unsupported', 'A .gltf member whose first four bytes are b3dm; develop fails JSON admission.')

    # -- order relative to other C1 checks.
    case('missing-b3dm-member', [('tileset.json', tileset(tile('tile.b3dm')))],
         dict(expectedCategory='invalid_input', rule='L0', messageIncludes=['tile.b3dm']), 'invalid_input',
         'InvalidInput', 'L0: a missing member stays InvalidInput; existence precedes the suffix rule.')
    case('content-volume-escape-before-b3dm',
         [('tileset.json', tileset(dict(tile(), content={'uri': 'tile.b3dm', 'boundingVolume': {
             'box': [0, 0, 0, 4000, 0, 0, 0, 4000, 0, 0, 0, 4000]}}))), ('tile.b3dm', b3dm())],
         dict(expectedCategory='invalid_input', rule='order', messageIncludes=['content bounds']), 'invalid_input',
         'Unsupported', 'The tile-level content-volume check precedes payload classification. Neither oracle '
         'models that C1 hierarchy check, so the oracle outcome is the legacy verdict; production order decides.')
    case('glb-defect-then-b3dm',
         [('tileset.json', tileset(tile(children=[tile('a.glb'), tile('b.b3dm')]))),
          ('a.glb', glb()[:-4]), ('b.b3dm', b3dm())],
         dict(expectedCategory='invalid_input', rule='order', messageIncludes=[]), 'invalid_input', 'Undecided',
         'First failure in traversal order wins: a malformed GLB before the b3dm.')
    case('b3dm-then-glb-defect',
         [('tileset.json', tileset(tile(children=[tile('b.b3dm'), tile('a.glb')]))),
          ('b.b3dm', b3dm()), ('a.glb', glb()[:-4])],
         legacy('b3dm', 'b.b3dm', 'L1'), 'invalid_input', 'Unsupported',
         'First failure in traversal order wins: the b3dm before a malformed GLB.')
    case('b3dm-with-c2-exclusion',
         [('tileset.json', tileset(tile(children=[tile('a.glb'), tile('b.b3dm')]),
                                   asset={'version': '1.1', 'gltfUpAxis': 'Z'})),
          ('a.glb', glb()), ('b.b3dm', b3dm())],
         legacy('b3dm', 'b.b3dm', 'L1'), 'admitted', 'Unsupported',
         'C1 precedes C2: an upAxisOverride exclusion elsewhere does not mask the legacy verdict.')
    case('b3dm-in-external-tileset',
         [('tileset.json', tileset(tile('ext/external.json'))),
          ('ext/external.json', tileset(tile('t.b3dm'))), ('ext/t.b3dm', b3dm())],
         legacy('b3dm', 'ext/t.b3dm', 'L1'), 'admitted', 'Unsupported',
         'Content of an external tileset: the member name is resolved before classification.')
    case('b3dm-referenced-twice',
         [('tileset.json', tileset(tile(children=[tile('b.b3dm'), tile('b.b3dm', box=BOX[:3] + [1000, 0, 0, 0, 1000, 0, 0, 0, 1000])]))),
         ('b.b3dm', b3dm())],
         legacy('b3dm', 'b.b3dm', 'L1'), 'admitted', 'Unsupported',
         'The URI cache never holds a legacy payload: the first reference already fails.')

    # -- implicit templates (T) and implicit content (L2).
    for fmt, template in (('b3dm', 'c/{level}/{x}/{y}.b3dm'), ('pnts', 'c/{level}/{x}/{y}.PNTS'),
                          ('cmpt', 'c/{level}/{x}/{y}.cmpt/.')):
        case('implicit-template-' + fmt + ('-dot-segment' if template.endswith('/.') else ''),
             [('tileset.json', implicit_tileset(template)), ('c/0/0/0.' + fmt, b3dm())],
             dict(expectedCategory='unsupported', rule='T', messageIncludes=['legacy', fmt, 'tileset.json']),
             'invalid_input', 'Unsupported',
             'Template %r is Unsupported at root admission. Its subtree member is deliberately absent: '
             'reading it would be InvalidInput, so the verdict proves no subtree read.' % template)
    case('implicit-glb-template-b3dm-bytes',
         [('tileset.json', implicit_tileset('c/{level}/{x}/{y}.glb')), ('s/0/0/0.subtree', subtree()),
          ('c/0/0/0.glb', b3dm())], legacy('b3dm', 'c/0/0/0.glb', 'L2'), 'invalid_input', 'Unsupported',
         'An implicit content emitted under a .glb template whose bytes are b3dm.')

    # -- the retained convert-to-implicit payload path.
    report = packed({'operation': 'convert-to-implicit', 'retainedContentUris': ['orig.b3dm']})
    case('retained-b3dm', [('tileset.json', tileset(tile('tile.glb'))), ('tile.glb', glb()),
                           ('conversion.json', report), ('orig.b3dm', b3dm())],
         legacy('b3dm', 'orig.b3dm', 'L1'), 'admitted', 'Contained',
         'A retained original payload named only by conversion.json retainedContentUris. The C2 oracle '
         'does not model conversion.json (C1 scope), so its outcome is the tileset content alone.')
    case('retained-missing-b3dm', [('tileset.json', tileset(tile('tile.glb'))), ('tile.glb', glb()),
                                   ('conversion.json', report)],
         dict(expectedCategory='invalid_input', rule='L0', messageIncludes=['orig.b3dm']), 'invalid_input',
         'Contained', 'L0 on the one caller without a prior member lookup: a missing retained .b3dm stays '
         'InvalidInput (missing archive entry), not Unsupported.')
    # -- the rustyTilesSourceContents caller (tileset extras beside an implicit root).
    sources = json.loads(implicit_tileset('c/{level}/{x}/{y}.glb'))
    sources['extras'] = {'rustyTilesSourceContents': ['raw/a.b3dm']}
    case('source-contents-b3dm', [('tileset.json', packed(sources)), ('s/0/0/0.subtree', subtree()),
                                  ('c/0/0/0.glb', glb()), ('raw/a.b3dm', b3dm())],
         legacy('b3dm', 'raw/a.b3dm', 'L1'), 'admitted', 'Contained',
         'A raw source payload named by tileset extras rustyTilesSourceContents. The C2 oracle does not '
         'model that list (C1 scope).')
    return out


def generate(target):
    target.mkdir(parents=True, exist_ok=False)
    manifest = dict(schemaVersion=1, lane='H7 legacy content Unsupported (PROPOSED)',
                    generatorSha256=sha(Path(__file__).read_bytes()), cases=[])
    total = 0
    for c in cases():
        raw = archive(c['members'])
        path = c['name'] + '.3tz'
        (target / path).write_bytes(raw)
        total += len(raw)
        e = c['expect']
        manifest['cases'].append(dict(
            name=c['name'], path=path, bytes=len(raw), sha256=sha(raw),
            members=[m for m, _ in c['members']],
            expectedCategory=e['expectedCategory'],
            expectedExitCode={'admitted': 0, 'unsupported': 2, 'invalid_input': 3}[e['expectedCategory']],
            rule=e['rule'], messageIncludes=e['messageIncludes'],
            developCategory=c['develop'], c2OracleOutcome=c['oracle'], fact=c['fact']))
    manifest['totalBytes'] = total
    (target / 'manifest.json').write_text(json.dumps(manifest, indent=1, sort_keys=True) + '\n')
    return manifest


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument('--out', type=Path, default=HERE / 'fixtures')
    m = generate(parser.parse_args().out)
    print('%d cases, %d bytes' % (len(m['cases']), m['totalBytes']), file=sys.stderr)
