"""Write sixteen invented metre XYZ points as LAS 1.2, using only Python stdlib.

Usage: python3 tests/fixtures/local_points.py output/cloud.las [--utm]
Local coordinates are the default. --utm offsets them into WGS84 UTM zone 56S;
invented Z values are ellipsoidal metre heights, not measured elevations.
"""
import argparse
from pathlib import Path
import struct


def write_cloud(path, utm=False):
    points = [(x * 100, y * 100, (x + y) * 25) for y in range(4) for x in range(4)]
    header = bytearray(227)
    header[:4] = b"LASF"
    header[24:26] = bytes((1, 2))
    header[26:37] = b"rusty-tiles"
    header[58:71] = b"invented demo"
    struct.pack_into("<HIIBHI", header, 94, 227, 227, 0, 3, 34, len(points))
    struct.pack_into("<5I", header, 111, len(points), 0, 0, 0, 0)
    struct.pack_into("<3d", header, 131, 0.01, 0.01, 0.01)
    offsets = (500000, 6960000, 80) if utm else (0, 0, 0)
    struct.pack_into("<3d", header, 155, *offsets)
    struct.pack_into("<6d", header, 179, offsets[0] + 3, offsets[0],
                     offsets[1] + 3, offsets[1], offsets[2] + 1.5, offsets[2])
    path.parent.mkdir(parents=True, exist_ok=True)
    with path.open("xb") as stream:
        stream.write(header)
        for x, y, z in points:
            stream.write(struct.pack("<iiiHBBbBHdHHH", x, y, z, 1000, 9, 2, 0, 0, 0,
                                     0.0, 65535, 20000, 0))


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("output", type=Path)
    parser.add_argument("--utm", action="store_true", help="invented EPSG:32756 metre coordinates")
    args = parser.parse_args()
    write_cloud(args.output, args.utm)
