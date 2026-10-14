"""Isolate Cesium 1.146's failure on tile-aligned raster bounds.

Usage: python3 bench/perf_lane/aligned_bounds.py --bin NATIVE_BIN --r2 R2_OUTPUT --work DIR --out PREFIX
Builds TileJSON variants of an R2 output (tiles symlinked, bounds edited one edge at
a time) and real tile-aligned pyramids away from the 0 degree meridian, then loads all
of them in cesium.py with --corner-views twice: with exact bounds, and with the
preview fix (every edge inset by 1/1024 finest-zoom pixel). Writes PREFIX-exact.json
and PREFIX-inset.json.
"""
import argparse, json, math, os, pathlib, shutil, subprocess, sys

sys.path.insert(0, str(pathlib.Path(__file__).resolve().parent))
import corpus

NUDGE = 1e-7  # degrees: moves one edge off the tile grid without changing the imagery
HALF = 20037508.342789244  # EPSG:3857 half extent; z14 tile edge = 2 * HALF / 2**14


def variant(work, name, source, edit):
    target = work / 'variants' / name
    target.mkdir(parents=True)
    (target / 'tiles').symlink_to((source / 'tiles').resolve())
    metadata = json.loads((source / 'tilejson.json').read_text())
    metadata['bounds'] = edit(metadata['bounds'])
    (target / 'tilejson.json').write_text(json.dumps(metadata))
    return target


def aligned(work, binary, name, tile_x, tile_y, offset=0.0):
    """2048-pixel RGBA pyramid whose north-west corner is z14 tile (tile_x, tile_y), shifted by offset metres."""
    edge = 2 * HALF / 2 ** 14
    source = work / f'{name}.tif'
    corpus.write_tiff(source, 2048, 2048, 4, 8, 1, 2, corpus.rgba_rows(2048, 512), corpus.GEO_3857, (edge / 256,) * 2,
                      (-HALF + tile_x * edge + offset, HALF - tile_y * edge - offset), extra=(2,))
    output = work / 'variants' / name
    subprocess.run([str(binary), 'raster', '-i', str(source), '-o', str(output), '--minZoom', '10', '--maxZoom', '14',
                    '--jobs', '2'], check=True, env=dict(PATH='', RAYON_NUM_THREADS='2', PROJ_NETWORK='OFF'),
                   stdout=subprocess.DEVNULL)
    return output


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--bin', type=pathlib.Path, required=True)
    parser.add_argument('--r2', type=pathlib.Path, required=True)
    parser.add_argument('--work', type=pathlib.Path, required=True)
    parser.add_argument('--out', required=True)
    args = parser.parse_args()
    os.sched_setaffinity(0, {0, 1}); os.setpriority(os.PRIO_PROCESS, 0, 10)
    work = args.work.resolve()
    shutil.rmtree(work / 'variants', ignore_errors=True)
    r2 = args.r2.resolve()
    keep = lambda *edges: lambda b: [v if i in edges else v + (NUDGE if i < 2 else -NUDGE) for i, v in enumerate(b)]
    cases = {
        'r2-exact': variant(work, 'r2-exact', r2, lambda b: b),
        'r2-only-west-exact': variant(work, 'r2-only-west-exact', r2, keep(0)),
        'r2-only-south-exact': variant(work, 'r2-only-south-exact', r2, keep(1)),
        'r2-only-east-exact': variant(work, 'r2-only-east-exact', r2, keep(2)),
        'r2-only-north-exact': variant(work, 'r2-only-north-exact', r2, keep(3)),
        'r2-west-nudged': variant(work, 'r2-west-nudged', r2, keep(1, 2, 3)),
        'r2-shifted-east': variant(work, 'r2-shifted-east', r2, lambda b: [b[2], b[1], 2 * b[2] - b[0], b[3]]),
        # Near Rome: z14 tile (8768, 6080) is a multiple of 32, so that corner is on every grid down to z9.
        'rome-aligned-z9': aligned(work, args.bin, 'rome-aligned-z9', 8768, 6080),
        'rome-aligned-z14-only': aligned(work, args.bin, 'rome-aligned-z14-only', 8771, 6083),
        'rome-unaligned': aligned(work, args.bin, 'rome-unaligned', 8771, 6083, offset=100.0),
    }
    script = pathlib.Path(__file__).with_name('cesium.py')
    pairs = [f'{name}={path}' for name, path in cases.items()]
    for label, extra in (('exact', []), ('inset', ['--inset', '1/1024px'])):
        subprocess.run([sys.executable, '-I', str(script), '--work', str(work / f'cesium-{label}'), '--out',
                        f'{args.out}-{label}.json', '--corner-views', '--timeout', '60', *extra, *pairs])


if __name__ == '__main__':
    main()
