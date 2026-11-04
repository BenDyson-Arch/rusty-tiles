#!/usr/bin/env python3
"""Deterministic, seeded, streaming input generators for the milestone-4 scale gates.

  mesh     (a) production mesh: terrain-like sheets plus clustered buildings, N triangles exactly,
               several primitives and materials, POSITION/NORMAL/TANGENT/TEXCOORD_0.
  density  (b) feature density: per region (root) 10^4 primitives, 10^4 interior holes in a
               sheet and 10^4 small islands.
  texture  (c) procedural-texture bake: the mesh of (a) with per-material baseColor (sRGB colour),
               normal and metallicRoughness (linear data) textures, all class-distinct.

Generation proceeds one cell or region at a time; only accessor metadata stays in memory, so
RAM is bounded by --cell-triangles (mesh/texture) or one region (density), never by N. Output
goes under ~/.cache/rusty-tiles-scale/ (RUSTY_TILES_SCALE_ROOT) with a manifest.json holding
parameters, sha256 of every file and a truth record. The disk is checked before anything is
written; inputs above --large-threshold triangles need --large.

  python3 -I bench/scale_gates/generate.py mesh --triangles 1000000 --out mesh-1e6
  python3 -I bench/scale_gates/generate.py mesh --preset production --large --out mesh-526m
  python3 -I bench/scale_gates/generate.py density --out density-1e4
  python3 -I bench/scale_gates/generate.py texture --triangles 1000000 --out texture-1e6
"""
import argparse, hashlib, json, math, pathlib, shutil, sys, time

sys.path.insert(0, str(pathlib.Path(__file__).resolve().parent))
import numpy as np  # noqa: E402
import scalecommon as sc  # noqa: E402

PRODUCTION_TRIANGLES = 526_174_760  # the P0 spilled-octree grid case (mlarge-p0-20261012 summary)
LARGE_THRESHOLD = 50_000_000
BYTES_PER_TRIANGLE = 40  # ~0.55 vertices/triangle x 48 attribute bytes + 12 index bytes, rounded up
F4, U4 = np.dtype('<f4'), np.dtype('<u4')

MATERIALS = [  # name, baseColorFactor, metallic, roughness; texture pattern ids are per index
    ('terrain-grass', [0.32, 0.46, 0.20, 1.0], 0.0, 0.92),
    ('terrain-rock', [0.45, 0.41, 0.36, 1.0], 0.0, 0.80),
    ('facade', [0.70, 0.62, 0.55, 1.0], 0.05, 0.70),
    ('roof', [0.52, 0.20, 0.16, 1.0], 0.35, 0.55),
]


# ---------------------------------------------------------------- (a) terrain + buildings

class Terrain:
    def __init__(self, seed):
        self.seed = seed

    def height(self, x, z):
        h, gx, gz = sc.fbm(self.seed, x, z, 1 / 512.0, 36.0, 6)
        return h + 0.004 * x + 0.002 * z, gx + 0.004, gz + 0.002


def terrain_patch(terrain, x0, z0, extent, g):
    n = g + 1
    t = np.arange(n, dtype=np.float64) * (extent / g)
    xs, zs = np.meshgrid(x0 + t, z0 + t, indexing='xy')
    x, z = xs.ravel(), zs.ravel()
    h, gx, gz = terrain.height(x, z)
    pos = np.stack([x, h, z], axis=1)
    nrm = sc.normalize(np.stack([-gx, np.ones_like(gx), -gz], axis=1))
    tan = sc.normalize(np.stack([np.ones_like(gx), gx, np.zeros_like(gx)], axis=1))
    # glTF: bitangent cross(N, T) * w points to -dP/dv (v grows down the image); here -z, so w = +1
    tan = np.concatenate([tan, np.ones((tan.shape[0], 1))], axis=1)
    uv = np.stack([x / 32.0, z / 32.0], axis=1)
    i, j = np.meshgrid(np.arange(g, dtype=np.uint32), np.arange(g, dtype=np.uint32), indexing='xy')
    v00 = (j * n + i).ravel(); v10 = v00 + 1; v01 = v00 + n; v11 = v01 + 1
    idx = np.stack([v00, v01, v10, v10, v01, v11], axis=1).ravel()
    return pos, nrm, tan, uv, idx.astype(U4)


def quad_grid(origin, along, up, nu, nv, normal, uv_scale):
    """Vertices/indices of an nu x nv quad grid spanning origin + [0,1]along + [0,1]up, wound
    counter-clockwise about `normal`, with tangent = along (dP/du) and glTF handedness:
    cross(N, T) * w = -dP/dv, since texture v grows downwards."""
    u = np.arange(nu + 1, dtype=np.float64) / nu
    v = np.arange(nv + 1, dtype=np.float64) / nv
    uu, vv = np.meshgrid(u, v, indexing='xy')
    uu, vv = uu.ravel(), vv.ravel()
    pos = origin + uu[:, None] * along + vv[:, None] * up
    count = pos.shape[0]
    nrm = np.broadcast_to(normal, (count, 3))
    tdir = along / np.sqrt((along * along).sum())
    w = -1.0 if np.dot(np.cross(normal, tdir), up) >= 0 else 1.0
    tan = np.broadcast_to(np.append(tdir, w), (count, 4))
    la, lu = np.sqrt((along * along).sum()), np.sqrt((up * up).sum())
    uv = np.stack([uu * la / uv_scale, vv * lu / uv_scale], axis=1)
    i, j = np.meshgrid(np.arange(nu, dtype=np.uint32), np.arange(nv, dtype=np.uint32), indexing='xy')
    row = nu + 1
    v00 = (j * row + i).ravel(); v10 = v00 + 1; v01 = v00 + row; v11 = v01 + 1
    if np.dot(np.cross(along, up), normal) >= 0:
        idx = np.stack([v00, v10, v01, v10, v11, v01], axis=1).ravel()
    else:
        idx = np.stack([v00, v01, v10, v10, v01, v11], axis=1).ravel()
    return pos, nrm, tan, uv, idx


class Builder:
    """Accumulates vertices/indices of one primitive."""

    def __init__(self):
        self.parts, self.vertices, self.triangles = [], 0, 0

    def add(self, pos, nrm, tan, uv, idx):
        self.parts.append((pos, nrm, tan, uv, idx.astype(np.uint64) + self.vertices))
        self.vertices += pos.shape[0]
        self.triangles += idx.shape[0] // 3

    def arrays(self):
        cat = lambda k: np.concatenate([p[k] for p in self.parts])
        return dict(POSITION=cat(0).astype(F4), NORMAL=cat(1).astype(F4), TANGENT=cat(2).astype(F4),
                    TEXCOORD_0=cat(3).astype(F4)), cat(4).astype(U4)


def building_triangles(floors, bw, bd):
    return 2 * floors * 2 * (bw + bd) + 2


def add_building(walls, roofs, terrain, cx, cz, hw, hd, c, s, floors, bw, bd):
    base, _, _ = terrain.height(np.array([cx]), np.array([cz]))
    y0 = float(base[0]) - 0.5
    height = floors * 3.2
    rot = lambda dx, dz: np.array([cx + c * dx - s * dz, y0, cz + s * dx + c * dz])
    corners = [rot(-hw, -hd), rot(hw, -hd), rot(hw, hd), rot(-hw, hd)]
    bays = [bw, bd, bw, bd]
    up = np.array([0.0, height, 0.0])
    for k in range(4):
        a, b = corners[k], corners[(k + 1) % 4]
        along = b - a
        normal = np.array([along[2], 0.0, -along[0]])
        normal /= np.sqrt((normal * normal).sum())
        walls.add(*quad_grid(a, along, up, bays[k], floors, normal, 4.0))
    top = [p + up for p in corners]
    roofs.add(*quad_grid(top[0], top[3] - top[0], top[1] - top[0], 1, 1, np.array([0.0, 1.0, 0.0]), 4.0))


def add_filler(prim, x, y, z, count):
    """count small upright triangles (rooftop vents), so totals hit N exactly."""
    k = np.arange(count, dtype=np.float64)
    px = x + 0.6 * (k % 64); pz = z + 0.6 * (k // 64)
    a = np.stack([px, np.full_like(px, y), pz], axis=1)
    b = a + [0.4, 0.0, 0.0]; d = a + [0.2, 0.4, 0.0]
    pos = np.stack([a, b, d], axis=1).reshape(-1, 3)
    nrm = np.tile([0.0, 0.0, 1.0], (pos.shape[0], 1))
    tan = np.tile([1.0, 0.0, 0.0, -1.0], (pos.shape[0], 1))  # cross(N, T) = +y = dP/dv, so w = -1
    uv = np.stack([pos[:, 0] / 4.0, pos[:, 1] / 4.0], axis=1)
    prim.add(pos, nrm, tan, uv, np.arange(pos.shape[0], dtype=np.uint64))


def mesh_cell(seed, terrain, ci, cj, extent, quota, terrain_share):
    """All primitives of one cell; returns [(material, attributes, indices)] and stats."""
    x0, z0 = ci * extent, cj * extent
    g = max(1, int(math.isqrt(max(2, int(terrain_share * quota)) // 2)))
    while 2 * g * g > quota and g > 1:
        g -= 1
    prims, stats = [], dict(terrain=0, buildings=0, building_triangles=0, filler=0)
    if 2 * g * g <= quota:
        pos, nrm, tan, uv, idx = terrain_patch(terrain, x0, z0, extent, g)
        material = int(sc.key(seed, 1, ci, cj) % np.uint64(2))
        prims.append((material, dict(POSITION=pos.astype(F4), NORMAL=nrm.astype(F4), TANGENT=tan.astype(F4),
                                     TEXCOORD_0=uv.astype(F4)), idx))
        stats['terrain'] = 2 * g * g
    remaining = quota - stats['terrain']
    rng = sc.Stream(seed, 2, ci, cj)
    walls, roofs = Builder(), Builder()
    clusters = 1 + int(rng.integers(1, 0, 4)[0])
    centres = rng.uniform(2 * clusters, 0.2 * extent, 0.8 * extent).reshape(clusters, 2)
    while remaining >= building_triangles(1, 1, 1):
        cl = centres[int(rng.integers(1, 0, clusters)[0])]
        off = rng.normalish(2) * (0.08 * extent)
        cx = float(np.clip(cl[0] + off[0], 20.0, extent - 20.0)) + x0
        cz = float(np.clip(cl[1] + off[1], 20.0, extent - 20.0)) + z0
        hw, hd = rng.uniform(2, 4.0, 15.0)
        t = float(rng.uniform(1, -0.4, 0.4)[0])
        c, s = (1 - t * t) / (1 + t * t), 2 * t / (1 + t * t)  # rational rotation, no libm
        floors = int(rng.integers(1, 1, 25)[0])
        bw, bd = max(1, int(hw / 2.0)), max(1, int(hd / 2.0))
        while building_triangles(floors, bw, bd) > remaining:
            if floors > 1:
                floors = max(1, floors // 2)
            elif bw > 1 or bd > 1:
                bw, bd = max(1, bw // 2), max(1, bd // 2)
        add_building(walls, roofs, terrain, cx, cz, hw, hd, c, s, floors, bw, bd)
        remaining -= building_triangles(floors, bw, bd)
        stats['buildings'] += 1
        stats['building_triangles'] += building_triangles(floors, bw, bd)
    if remaining:
        h, _, _ = terrain.height(np.array([x0 + extent / 2]), np.array([z0 + extent / 2]))
        add_filler(roofs, x0 + extent / 2, float(h[0]) + 40.0, z0 + extent / 2, remaining)
        stats['filler'] = remaining
    for material, builder in ((2, walls), (3, roofs)):
        if builder.triangles:
            attrs, idx = builder.arrays()
            prims.append((material, attrs, idx))
    return prims, stats


def material_json(textures):
    out = []
    for m, (name, color, metallic, roughness) in enumerate(MATERIALS):
        pbr = dict(baseColorFactor=color, metallicFactor=metallic, roughnessFactor=roughness)
        record = dict(name=name, pbrMetallicRoughness=pbr)
        if textures:
            pbr['baseColorFactor'] = [1.0, 1.0, 1.0, 1.0]
            pbr['metallicFactor'] = 1.0
            pbr['roughnessFactor'] = 1.0
            pbr['baseColorTexture'] = dict(index=textures[m]['baseColor'])
            pbr['metallicRoughnessTexture'] = dict(index=textures[m]['metallicRoughness'])
            record['normalTexture'] = dict(index=textures[m]['normal'], scale=1.0)
        out.append(record)
    return out


# ---------------------------------------------------------------- (c) procedural PBR textures

def texture_set(material, size, seed):
    """Three class-distinct textures for one material, each a different procedural pattern:
    baseColor (sRGB colour: hue bands per material + noise), normal (linear data: unit
    tangent-space normals of a bump field, blue-dominant) and metallicRoughness (linear data:
    R=0, G=roughness from noise, B=metallic in a checker of metal panels). Patterns tile under
    REPEAT. Returns {slot: HxWx3 uint8} and per-slot truth."""
    t = (np.arange(size, dtype=np.float64) + 0.5) / size
    u, v = np.meshgrid(t, t, indexing='xy')
    base_period = 4 + 2 * material
    n, _, _ = sc.fbm(seed + 101 * material, u, v, base_period, 1.0, 4, period=base_period)
    palette = np.array([[0.18, 0.55, 0.12], [0.55, 0.50, 0.44], [0.78, 0.38, 0.22], [0.25, 0.30, 0.55]])[material % 4]
    bands = ((np.floor(v * (8 + 4 * material)) % 2) * 0.12)[..., None]
    color = np.clip(palette[None, None, :] * (0.8 + 0.35 * n[..., None]) + bands * np.array([0.6, 0.3, -0.2]), 0, 1)
    # Bump field: brick-like ridges along u with material-specific frequency plus noise.
    bump_period = 6 + 3 * material
    h, hx, hz = sc.fbm(seed + 211 * material, u, v, bump_period, 0.6, 3, period=bump_period)
    ridge_f = 16 + 8 * material
    fr = (u * ridge_f) % 1.0
    ridge = 4.0 * fr * (1.0 - fr)                # parabola per stripe, C0 tiles
    dridge = 4.0 * (1.0 - 2.0 * fr) * ridge_f
    gx, gz = hx + 0.02 * dridge, hz
    # OpenGL/glTF convention: green is +Y = image up = -v, so the Y slope is -dh/d(-v) = +dh/dv
    nrm = sc.normalize(np.stack([-gx * 0.02, gz * 0.02, np.ones_like(gx)], axis=-1))
    normal = (nrm * 0.5 + 0.5)
    r, _, _ = sc.fbm(seed + 307 * material, u, v, 8, 1.0, 3, period=8)
    roughness = np.clip(0.62 + 0.3 * r, 0.05, 1.0)
    cells = 2 + material
    metal = ((np.floor(u * cells) + np.floor(v * cells)) % 2 == 0) & (material >= 2)
    metallic = np.where(metal, 0.9, 0.0)
    mr = np.stack([np.zeros_like(u), roughness, metallic], axis=-1)
    images = dict(baseColor=color, normal=normal, metallicRoughness=mr)
    images = {k: np.floor(img * 255.0 + 0.5).clip(0, 255).astype(np.uint8) for k, img in images.items()}
    truth = {}
    probe = [(size * a // 8, size * b // 8) for a in (1, 3, 5, 7) for b in (1, 3, 5, 7)]
    for slot, img in images.items():
        truth[slot] = dict(
            colour_class='sRGB colour' if slot == 'baseColor' else 'linear data',
            pattern={'baseColor': f'palette {material % 4} x fbm(period {base_period}) + {8 + 4 * material} v-bands',
                     'normal': f'tangent-space bumps: fbm(period {bump_period}) + {ridge_f} u-ridges',
                     'metallicRoughness': f'G roughness fbm(period 8); B metallic {cells}x{cells} checker'
                                          + (' (metal panels)' if material >= 2 else ' (all dielectric)')}[slot],
            channel_mean=[round(float(c), 4) for c in img.reshape(-1, 3).mean(axis=0)],
            probes=[dict(texel=[x, y], rgb=[int(c) for c in img[y, x]]) for x, y in probe])
    return images, truth


# ---------------------------------------------------------------- mesh / texture driver

def grid_shape(cells):
    cx = math.isqrt(cells)
    if cx * cx < cells:
        cx += 1
    return cx, -(-cells // cx)


def generate_mesh(args, textured):
    n = args.triangles
    if n < 1:
        raise SystemExit('--triangles must be positive')
    if n > args.large_threshold and not args.large:
        raise SystemExit(f'{n:,} triangles is above --large-threshold {args.large_threshold:,}: '
                         f'pass --large to generate it (roughly {n * BYTES_PER_TRIANGLE / sc.GiB:.0f} GiB)')
    estimate = n * BYTES_PER_TRIANGLE + (len(MATERIALS) * 3 * args.texture_size ** 2 * 3 if textured else 0)
    fmt = args.format or ('glb' if estimate < 3 * sc.GiB else 'gltf')
    out = sc.prepare_output(args.out, args.force)
    disk = sc.disk_preflight(out, estimate * (2 if fmt == 'glb' else 1) * 1.1, args.reserve_gib * sc.GiB)
    out.mkdir(parents=True, exist_ok=True)
    cells = min(n, args.cells or max(1, -(-n // args.cell_triangles)))  # no empty cells
    gx, gz = grid_shape(cells)
    started = time.monotonic()
    writer = sc.GltfStream(out, 'scene', glb=(fmt == 'glb'), max_buffer_bytes=args.max_buffer_mib * sc.MiB)
    terrain = Terrain(args.seed)
    texture_truth = {}
    extra_files = []
    if textured:
        writer.json['samplers'] = [dict(magFilter=9729, minFilter=9987, wrapS=10497, wrapT=10497)]
        writer.json['textures'] = []
        writer.json['images'] = []
        slots = []
        for m in range(len(MATERIALS)):
            images, truth = texture_set(m, args.texture_size, args.seed)
            texture_truth[MATERIALS[m][0]] = truth
            ids = {}
            for slot, img in images.items():
                data = sc.png_bytes(img, compress=args.compress_png)
                if fmt == 'glb':
                    image = writer.image_bytes(data)
                else:
                    name = f'{MATERIALS[m][0]}-{slot}.png'
                    (out / name).write_bytes(data)
                    extra_files.append(name)
                    writer.json['images'].append(dict(uri=name))
                    image = len(writer.json['images']) - 1
                truth[slot]['image_sha256'] = hashlib.sha256(data).hexdigest()
                writer.json['textures'].append(dict(sampler=0, source=image))
                ids[slot] = len(writer.json['textures']) - 1
            slots.append(ids)
        writer.json['materials'] = material_json(slots)
    else:
        writer.json['materials'] = material_json(None)
    totals = dict(terrain=0, buildings=0, building_triangles=0, filler=0, primitives=0)
    bounds_min, bounds_max = np.full(3, np.inf), np.full(3, -np.inf)
    written = 0
    for c in range(cells):
        quota = n // cells + (1 if c < n % cells else 0)
        ci, cj = c % gx, c // gx
        prims, stats = mesh_cell(args.seed, terrain, ci, cj, args.cell_extent, quota, args.terrain_share)
        records = []
        for material, attrs, idx in prims:
            records.append(writer.primitive(attrs, idx, material))
            written += idx.shape[0] // 3
            bounds_min = np.minimum(bounds_min, attrs['POSITION'].min(axis=0))
            bounds_max = np.maximum(bounds_max, attrs['POSITION'].max(axis=0))
        writer.add_mesh_node(records, name=f'cell_{ci}_{cj}')
        for k in ('terrain', 'buildings', 'building_triangles', 'filler'):
            totals[k] += stats[k]
        totals['primitives'] += len(records)
        if args.progress and (c + 1) % max(1, cells // 20) == 0:
            print(json.dumps(dict(cell=c + 1, cells=cells, triangles=written,
                                  seconds=round(time.monotonic() - started, 1))), file=sys.stderr, flush=True)
    assert written == n, (written, n)
    files = writer.finish() + extra_files
    elapsed = time.monotonic() - started
    params = dict(triangles=n, seed=args.seed, format=fmt, cells=cells, grid=[gx, gz], cell_extent_m=args.cell_extent,
                  cell_triangles=args.cell_triangles, terrain_share=args.terrain_share,
                  max_buffer_mib=args.max_buffer_mib, textured=textured)
    if textured:
        params.update(texture_size=args.texture_size, compress_png=args.compress_png)
    truth = dict(triangles=n, primitives=totals['primitives'], materials=len(MATERIALS),
                 terrain_triangles=totals['terrain'], buildings=totals['buildings'],
                 building_triangles=totals['building_triangles'], filler_triangles=totals['filler'],
                 attributes=['POSITION', 'NORMAL', 'TANGENT', 'TEXCOORD_0'], coordinates='local metres, Y-up',
                 bounds_min=[float(v) for v in bounds_min], bounds_max=[float(v) for v in bounds_max],
                 input_file=files[0])
    if textured:
        truth['textures'] = texture_truth
    manifest = sc.write_manifest(out, 'texture' if textured else 'mesh', params, files, truth,
                                 dict(disk_preflight=disk, generation_seconds=round(elapsed, 3)))
    print(json.dumps(dict(out=str(out), input=str(out / files[0]), triangles=n, bytes=manifest['total_bytes'],
                          seconds=round(elapsed, 2))))
    return manifest


# ---------------------------------------------------------------- (b) feature density

def density_region(seed, r, args):
    """One region: a gently undulating sheet with H interior unit holes on a 2-cell lattice,
    I small islands (coplanar inside the first holes, then floating above the sheet) spread
    over P - 1 primitives, the sheet being primitive 0."""
    H, I, P = args.holes, args.islands, args.primitives
    lattice = max(1, math.isqrt(max(H, 1) - 1) + 1) if H else 0
    G = max(2 * lattice + 1, 4)
    cell = args.cell_size
    x0 = r * (G * cell + 20.0)
    noise = lambda x, z: sc.fbm(seed + 17 * r, x, z, 1 / 64.0, 0.6, 2)
    # sheet
    n = G + 1
    t = np.arange(n, dtype=np.float64) * cell
    xs, zs = np.meshgrid(x0 + t, t, indexing='xy')
    h, hx, hz = noise(xs.ravel(), zs.ravel())
    pos = np.stack([xs.ravel(), h, zs.ravel()], axis=1)
    nrm = sc.normalize(np.stack([-hx, np.ones_like(hx), -hz], axis=1))
    uv = np.stack([(xs.ravel() - x0) / (G * cell), zs.ravel() / (G * cell)], axis=1)
    keep = np.ones((G, G), dtype=bool)  # [row j, col i]
    holes = []
    for k in range(H):
        a, b = k % lattice, k // lattice
        keep[2 * b + 1, 2 * a + 1] = False
        holes.append((2 * a + 1, 2 * b + 1))
    j, i = np.nonzero(keep)
    v00 = (j * n + i).astype(np.uint32); v10 = v00 + 1; v01 = v00 + n; v11 = v01 + 1
    idx = np.stack([v00, v01, v10, v10, v01, v11], axis=1).ravel().astype(U4)
    sheet = (dict(POSITION=pos.astype(F4), NORMAL=nrm.astype(F4), TEXCOORD_0=uv.astype(F4)), idx, 0)
    # islands: k x k quad patches of side island_size
    k = args.island_subdivision
    s = args.island_size
    q = np.arange(k + 1, dtype=np.float64) / k * s
    qx, qz = np.meshgrid(q, q, indexing='xy')
    qi, qj = np.meshgrid(np.arange(k, dtype=np.uint32), np.arange(k, dtype=np.uint32), indexing='xy')
    w = k + 1
    p00 = (qj * w + qi).ravel()
    island_idx = np.stack([p00, p00 + w, p00 + 1, p00 + 1, p00 + w, p00 + w + 1], axis=1).ravel().astype(U4)
    float_lattice = max(1, math.isqrt(max(I - H, 1) - 1) + 1)
    def merge(parts, material):
        """Concatenate (positions, normals, uvs, local indices) parts into one primitive."""
        base, idx = 0, []
        for part in parts:
            idx.append(part[3].astype(np.uint64) + base); base += part[0].shape[0]
        return (dict(POSITION=np.concatenate([q[0] for q in parts]).astype(F4),
                     NORMAL=np.concatenate([q[1] for q in parts]).astype(F4),
                     TEXCOORD_0=np.concatenate([q[2] for q in parts]).astype(F4)), np.concatenate(idx).astype(U4),
                material)

    groups = [[] for _ in range(max(P - 1, 1))]
    for m in range(I):
        if m < H:  # coplanar inside hole m, separated by a ring gap
            ci, cj = holes[m]
            cx, cz = x0 + (ci + 0.5) * cell - s / 2, (cj + 0.5) * cell - s / 2
            lift = 0.0
        else:  # floating 2 m above the sheet
            e = m - H
            cx = x0 + ((e % float_lattice) + 0.5) * (G * cell / float_lattice) - s / 2
            cz = ((e // float_lattice) + 0.5) * (G * cell / float_lattice) - s / 2
            lift = 2.0
        hy, _, _ = noise(np.array([cx + s / 2]), np.array([cz + s / 2]))
        ipos = np.stack([cx + qx.ravel(), np.full(w * w, float(hy[0]) + lift), cz + qz.ravel()], axis=1)
        groups[m % len(groups)].append((ipos, np.tile([0.0, 1.0, 0.0], (w * w, 1)),
                                        np.stack([qx.ravel() / s, qz.ravel() / s], axis=1), island_idx))
    sheet_part = (sheet[0]['POSITION'], sheet[0]['NORMAL'], sheet[0]['TEXCOORD_0'], sheet[1])
    if P == 1:
        prims = [merge([sheet_part] + groups[0], 0)]
    else:
        prims = [sheet] + [merge(members, (g + 1) % args.materials) for g, members in enumerate(groups)]
    triangles = sum(p[1].shape[0] // 3 for p in prims)
    truth = dict(region=r, primitives=len(prims), holes=H, islands=I, triangles=triangles,
                 sheet_cells=[G, G], sheet_triangles=int(2 * keep.sum()), island_triangles=2 * k * k,
                 islands_in_holes=min(H, I), floating_islands=max(0, I - H), origin_x=x0,
                 hole_cell_m=cell, island_size_m=s)
    return prims, truth


def generate_density(args):
    if args.island_size >= args.cell_size:
        raise SystemExit('--island-size must be smaller than --cell-size (islands sit inside unit holes)')
    if args.primitives > 1 and args.islands < args.primitives - 1:
        raise SystemExit('--islands must be at least --primitives - 1 (each island primitive holds >= 1 island)')
    out = sc.prepare_output(args.out, args.force)
    per_region = (2 * (2 * math.isqrt(max(args.holes, 1)) + 3) ** 2 + 2 * args.islands * args.island_subdivision ** 2)
    estimate = args.regions * (per_region * 30 + args.primitives * 700)
    fmt = args.format or ('glb' if estimate < 3 * sc.GiB else 'gltf')
    disk = sc.disk_preflight(out, estimate * 2.2, args.reserve_gib * sc.GiB)
    out.mkdir(parents=True, exist_ok=True)
    started = time.monotonic()
    writer = sc.GltfStream(out, 'scene', glb=(fmt == 'glb'), max_buffer_bytes=args.max_buffer_mib * sc.MiB)
    writer.json['materials'] = [dict(name=f'density-{m}', pbrMetallicRoughness=dict(
        baseColorFactor=[(m * 37 % 256) / 255, (m * 91 % 256) / 255, (m * 53 % 256) / 255, 1.0],
        metallicFactor=0.0, roughnessFactor=0.8)) for m in range(args.materials)]
    regions = []
    for r in range(args.regions):
        prims, truth = density_region(args.seed, r, args)
        records = writer.packed_primitives(prims)
        writer.add_mesh_node(records, name=f'region_{r}')
        regions.append(truth)
    files = writer.finish()
    elapsed = time.monotonic() - started
    params = dict(seed=args.seed, format=fmt, regions=args.regions, primitives=args.primitives, holes=args.holes,
                  islands=args.islands, island_subdivision=args.island_subdivision, island_size_m=args.island_size,
                  cell_size_m=args.cell_size, materials=args.materials)
    truth = dict(regions=regions, triangles=sum(r['triangles'] for r in regions),
                 primitives=sum(r['primitives'] for r in regions), attributes=['POSITION', 'NORMAL', 'TEXCOORD_0'],
                 hole_rule='interior unit cells (2a+1, 2b+1) of the sheet lattice, never adjacent to each other or '
                           'the border; each is one interior boundary loop',
                 island_rule='island m < holes sits coplanar inside hole m (a ring gap separates it); the rest float '
                             '2 m above the sheet; island primitives cycle 1..P-1',
                 input_file=files[0])
    manifest = sc.write_manifest(out, 'density', params, files, truth,
                                 dict(disk_preflight=disk, generation_seconds=round(elapsed, 3)))
    print(json.dumps(dict(out=str(out), input=str(out / files[0]), triangles=truth['triangles'],
                          primitives=truth['primitives'], bytes=manifest['total_bytes'], seconds=round(elapsed, 2))))
    return manifest


# ---------------------------------------------------------------- CLI

def common(p):
    p.add_argument('--out', required=True, help='directory (relative paths resolve under ~/.cache/rusty-tiles-scale)')
    p.add_argument('--seed', type=int, default=20261013)
    p.add_argument('--format', choices=['glb', 'gltf'], help='default: glb below 3 GiB, else glTF + .bin files')
    p.add_argument('--max-buffer-mib', type=int, default=1024, help='largest external .bin file (glTF output)')
    p.add_argument('--reserve-gib', type=float, default=20.0, help='free space to leave after generation')
    p.add_argument('--force', action='store_true', help='replace a non-empty output directory')
    p.add_argument('--progress', action='store_true')


def mesh_args(p, default_triangles):
    p.add_argument('--triangles', type=int, default=default_triangles)
    p.add_argument('--preset', choices=['production'], help=f'production: {PRODUCTION_TRIANGLES:,} triangles')
    p.add_argument('--large', action='store_true', help=f'allow more than --large-threshold triangles')
    p.add_argument('--large-threshold', type=int, default=LARGE_THRESHOLD)
    p.add_argument('--cells', type=int, help='terrain cells (default ceil(N / --cell-triangles))')
    p.add_argument('--cell-triangles', type=int, default=1 << 18, help='triangles per cell: bounds generator RAM')
    p.add_argument('--cell-extent', type=float, default=256.0, help='cell side in metres')
    p.add_argument('--terrain-share', type=float, default=0.6, help='terrain fraction of each cell budget')


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    sub = parser.add_subparsers(dest='case', required=True)
    m = sub.add_parser('mesh', help='(a) production mesh: terrain sheets + clustered buildings')
    common(m); mesh_args(m, 1_000_000)
    t = sub.add_parser('texture', help='(c) procedural PBR texture bake mesh')
    common(t); mesh_args(t, 1_000_000)
    t.add_argument('--texture-size', type=int, default=1024)
    t.add_argument('--compress-png', action='store_true', help='zlib level 9 (bytes depend on the zlib build)')
    d = sub.add_parser('density', help='(b) 10^4 primitives and 10^4 holes/islands per region')
    common(d)
    d.add_argument('--regions', type=int, default=1)
    d.add_argument('--primitives', type=int, default=10_000)
    d.add_argument('--holes', type=int, default=10_000)
    d.add_argument('--islands', type=int, default=10_000)
    d.add_argument('--island-subdivision', type=int, default=1, help='island = k x k quads')
    d.add_argument('--island-size', type=float, default=0.4)
    d.add_argument('--cell-size', type=float, default=1.0)
    d.add_argument('--materials', type=int, default=16)
    args = parser.parse_args(argv)
    if args.case in ('mesh', 'texture'):
        if args.preset == 'production':
            args.triangles = PRODUCTION_TRIANGLES
        if args.case == 'mesh':
            args.texture_size, args.compress_png = 0, False
        return generate_mesh(args, textured=(args.case == 'texture'))
    return generate_density(args)


if __name__ == '__main__':
    main()
