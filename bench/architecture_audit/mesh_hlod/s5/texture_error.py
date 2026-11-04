"""Texture-aware published G (D5 amendment A3, 2026-11-04), re-derived from published bytes.

A tileset declaring asset.extras.rustyTiles.textureError = {version 1, pxPerTexel t,
refinableRatio r} promises, for every content tile T:

    G(T) >= 16 * s(T) / t   whenever s(T) > r * s(C(T)),

where s(X) = sqrt(A / X) over the textured triangles of X's payloads: A is their surface
area in the tileset frame, X their base-colour texel area (UV area times the KTX2/PNG
base level's width x height). C(T) are T's nearest content descendants, looking through
content-less tiles and copies of T's own payload down a single-child chain (the rule of
drops.children), their sums pooled. 16 is CesiumJS's default maximum screen-space error.
No producer fact is read: pages, UVs, positions and the hierarchy are the archive's.
"""
import json
import math
import struct

SSE = 16.0
# Producer arithmetic slack: G is compared at (1 - SLACK) and refinability needs
# s(T) > r * (1 + SLACK) * s(C), so f32 decode-order differences never refuse an
# honest archive. The producer publishes 2 * SLACK above the term.
SLACK = 1e-4


REFINABLE_RATIO = 1.25
PX_PER_TEXEL = (2.0, 1.25)


FAST = 'fast'
ABSENT = 'absent'
PROFILES = {'fast': None, 'balanced': 2.0, 'quality': 1.25, 'lossless': 1.25}


def declaration(tileset, require, report=None):
    """The declared (t, r), FAST (pxPerTexel null) or ABSENT. The oracle always holds the
    archive to HLOD, so the declaration is allowed without `lod`, and a tileset declaring
    `lod: "hlod"` must carry it; ABSENT refuses any textured payload (`undeclared`). The
    strict form is exactly {version: 1, pxPerTexel: 2 | 1.25 | null, refinableRatio: 1.25}.
    `report` is conversion.json: a mesh_tiler profile must match pxPerTexel."""
    extras = tileset.get('asset', {}).get('extras', {})
    rusty = extras.get('rustyTiles', {}) if isinstance(extras, dict) else {}
    rusty = rusty if isinstance(rusty, dict) else {}
    if 'textureError' not in rusty:
        require(rusty.get('lod') != 'hlod', 'textureError', 'an HLOD tileset must declare its texture rule')
        return ABSENT
    value = rusty['textureError']
    require(isinstance(value, dict), 'textureError', 'declaration is not an object')
    require(set(value) == {'version', 'pxPerTexel', 'refinableRatio'}, 'textureError',
            'declaration must hold exactly version, pxPerTexel and refinableRatio')
    require(type(value['version']) is int and value['version'] == 1, 'textureError',
            'version must be the integer 1')
    number = lambda v: isinstance(v, (int, float)) and not isinstance(v, bool)
    require(number(value['refinableRatio']) and value['refinableRatio'] == REFINABLE_RATIO,
            'textureError', 'refinableRatio must be 1.25')
    t = value['pxPerTexel']
    require(t is None or (number(t) and float(t) in PX_PER_TEXEL), 'textureError',
            'pxPerTexel must be 2, 1.25 or null')
    if isinstance(report, dict) and report.get('operation') == 'mesh_tiler' and report.get('profile') in PROFILES:
        require(PROFILES[report['profile']] == (None if t is None else float(t)), 'textureError',
                "pxPerTexel disagrees with the conversion report's profile")
    return FAST if t is None else (float(t), REFINABLE_RATIO)


def undeclared(glb, require):
    """An undeclared archive must not bind a base-colour texture on any payload."""
    length = int.from_bytes(glb[12:16], 'little')
    doc = json.loads(glb[20:20 + length])
    require(not any('baseColorTexture' in m.get('pbrMetallicRoughness', {}) for m in doc.get('materials', [])),
            'textureError', "a textured HLOD payload needs the tileset's textureError declaration")


def image_size(data, require):
    """Base-level (width, height) of a KTX2 or PNG page."""
    if data[:12] == b'\xabKTX 20\xbb\r\n\x1a\n':
        size = struct.unpack('<II', data[20:28])
    elif data[:8] == b'\x89PNG\r\n\x1a\n' and data[12:16] == b'IHDR':
        size = struct.unpack('>II', data[16:24])
    else:
        size = (0, 0)
    require(size[0] > 0 and size[1] > 0, 'textureError', 'base-colour page is not a KTX2 or PNG image')
    return size


def measure(glb, faces, require):
    """(A, X) of one payload from its decoded faces (tileset-frame xyz, TEXCOORD_0 uvs and
    the material object); each material's page size comes from the payload's own images."""
    length = int.from_bytes(glb[12:16], 'little')
    doc = json.loads(glb[20:20 + length])
    binary = glb[28 + length:]
    texels = []
    for m in doc.get('materials', []):
        ref = m.get('pbrMetallicRoughness', {}).get('baseColorTexture')
        if ref is None:
            texels.append(None)
            continue
        require('extensions' not in ref, 'textureError', 'base-colour texture transform')
        texcoord = ref.get('texCoord', 0)
        require(type(texcoord) is int and texcoord >= 0, 'textureError', 'base-colour texCoord')
        texture = doc['textures'][ref['index']]
        source = texture.get('extensions', {}).get('KHR_texture_basisu', {}).get('source', texture.get('source'))
        require(source is not None, 'textureError', 'base-colour texture without image')
        view = doc['bufferViews'][doc['images'][source]['bufferView']]
        start = view.get('byteOffset', 0)
        w, h = image_size(binary[start:start + min(28, view['byteLength'])], require)
        texels.append((w * h, texcoord))
    materials = doc.get('materials', [])
    area = texel_area = 0.0
    for f in faces:
        if f.get('material') is None:
            continue
        page = texels[materials.index(f['material'])]
        if page is None:
            continue
        scale, texcoord = page
        uvs = f.get('uvs') if texcoord == 0 else f.get('texcoords', {}).get(str(texcoord))
        require(uvs is not None, 'textureError', f'textured face without TEXCOORD_{texcoord}')
        a, b, c = f['xyz']
        u = [b[k] - a[k] for k in range(3)]
        v = [c[k] - a[k] for k in range(3)]
        n = (u[1] * v[2] - u[2] * v[1], u[2] * v[0] - u[0] * v[2], u[0] * v[1] - u[1] * v[0])
        area += 0.5 * math.sqrt(n[0] ** 2 + n[1] ** 2 + n[2] ** 2)
        p, q, r = uvs
        texel_area += 0.5 * abs((q[0] - p[0]) * (r[1] - p[1]) - (q[1] - p[1]) * (r[0] - p[0])) * scale
    return area, texel_area


def spacing(area, texels):
    """Mean texel spacing; textured area over no UV area is infinitely coarse."""
    if area <= 0 and texels <= 0:
        return None
    return math.inf if texels <= 0 else math.sqrt(area / texels)


def children(t, digest):
    out = []
    for c in t['children']:
        if not c['contents'] or c['textureDigest'] == digest:
            out.extend(children(c, digest))
        else:
            out.append(c)
    return out


def inspect(tiles, declared, require):
    """Each content tile carries 'texture' (A, X) and 'textureDigest' (sorted payload hashes).
    Refuse a refinable tile whose geometricError is below its texture-implied error."""
    if declared in (FAST, ABSENT):
        return dict(declared=declared == FAST)
    t_px, ratio = declared
    refinable = bound = 0
    worst = 0.0
    for t in tiles:
        if not t['contents']:
            continue
        s = spacing(*t['texture'])
        below = children(t, t['textureDigest'])
        sc = spacing(sum(c['texture'][0] for c in below), sum(c['texture'][1] for c in below))
        if s is None or sc is None or s <= ratio * (1 + SLACK) * sc:
            continue
        refinable += 1
        need = SSE * s / t_px
        require(t['g'] >= need * (1 - SLACK), 'textureError',
                f'tile {t["id"]}: geometricError {t["g"]} below its texture-implied {need} '
                f'(spacing {s}, children {sc}, t {t_px})')
        bound += t['g'] <= need * 1.01
        worst = max(worst, need / t['g'])
    return dict(declared=True, pxPerTexel=t_px, refinableRatio=ratio, refinableTiles=refinable,
                textureBoundTiles=bound, maxImpliedOverG=worst)
