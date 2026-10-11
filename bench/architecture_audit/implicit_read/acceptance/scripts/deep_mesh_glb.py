"""W6d input: the tests/support textured_grid_glb(n) recipe (20 m grid, z = sin(7u)cos(5v)/2,
64x64 RGBA texture (4x, 4y, 4(x^y), 255), +Z normals) as a stdlib GLB. Geometry, UVs, indices
and texels follow the recipe; the PNG is zlib level 9 (bytes differ from the image crate's)."""
import json, math, struct, sys, zlib
n = int(sys.argv[2]) if len(sys.argv) > 2 else 12
f32 = lambda x: struct.unpack("<f", struct.pack("<f", x))[0]
rows = b"".join(b"\0" + bytes(c for x in range(64) for c in ((x*4) & 255, (y*4) & 255, ((x ^ y)*4) & 255, 255)) for y in range(64))
def chunk(t, d): return struct.pack(">I", len(d)) + t + d + struct.pack(">I", zlib.crc32(t + d) & 0xffffffff)
png = b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", struct.pack(">IIBBBBB", 64, 64, 8, 6, 0, 0, 0)) + chunk(b"IDAT", zlib.compress(rows, 9)) + chunk(b"IEND", b"")
pos, uv = [], []
for j in range(n + 1):
    for i in range(n + 1):
        u, v = f32(i / n), f32(j / n)
        z = f32(f32(math.sin(f32(u * 7.0))) * f32(math.cos(f32(v * 5.0))) * 0.5)
        pos.append((f32(u * 20.0), f32(v * 20.0), z)); uv.append((u, v))
idx = []
for j in range(n):
    for i in range(n):
        a = j * (n + 1) + i; c = a + n + 1
        idx += [a, a + 1, c, a + 1, c + 1, c]
def pad(b, k=4, c=b"\0"): return b + c * ((k - len(b) % k) % k)
P = b"".join(struct.pack("<3f", *p) for p in pos); N = struct.pack("<3f", 0, 0, 1) * len(pos)
U = b"".join(struct.pack("<2f", *t) for t in uv); I = struct.pack(f"<{len(idx)}H", *idx)
views, bin_ = [], b""
for data, target in ((P, 34962), (N, 34962), (U, 34962), (I, 34963), (png, None)):
    bin_ = pad(bin_); v = {"buffer": 0, "byteOffset": len(bin_), "byteLength": len(data)}
    if target: v["target"] = target
    views.append(v); bin_ += data
bin_ = pad(bin_)
mins = [min(p[k] for p in pos) for k in range(3)]; maxs = [max(p[k] for p in pos) for k in range(3)]
doc = {"asset": {"version": "2.0", "generator": "forest-acc deep_mesh_glb.py"}, "scene": 0, "scenes": [{"nodes": [0]}],
       "nodes": [{"mesh": 0}], "buffers": [{"byteLength": len(bin_)}], "bufferViews": views,
       "accessors": [{"bufferView": 0, "componentType": 5126, "count": len(pos), "type": "VEC3", "min": mins, "max": maxs},
                     {"bufferView": 1, "componentType": 5126, "count": len(pos), "type": "VEC3"},
                     {"bufferView": 2, "componentType": 5126, "count": len(pos), "type": "VEC2"},
                     {"bufferView": 3, "componentType": 5123, "count": len(idx), "type": "SCALAR"}],
       "images": [{"bufferView": 4, "mimeType": "image/png"}], "samplers": [{}], "textures": [{"source": 0, "sampler": 0}],
       "materials": [{"pbrMetallicRoughness": {"baseColorTexture": {"index": 0}, "metallicFactor": 0.0}}],
       "meshes": [{"primitives": [{"attributes": {"POSITION": 0, "NORMAL": 1, "TEXCOORD_0": 2}, "indices": 3, "material": 0}]}]}
j = pad(json.dumps(doc, separators=(",", ":")).encode(), 4, b" ")
glb = struct.pack("<III", 0x46546C67, 2, 12 + 8 + len(j) + 8 + len(bin_)) + struct.pack("<II", len(j), 0x4E4F534A) + j + struct.pack("<II", len(bin_), 0x004E4942) + bin_
open(sys.argv[1], "wb").write(glb)
