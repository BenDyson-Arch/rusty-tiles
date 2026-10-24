#!/usr/bin/env python3
"""Lightweight planning arithmetic, not a tiler benchmark or acceptance oracle.

Post-#254 record widths; conservative large-spill envelope of full::scratch_need.
Assumes every sort needs an intermediate merge, but no >=4096-run push compaction.
Does not include LOD sidecars, bake handoffs, leaf members or archive staging.
"""
import json

N = 526_174_760
INPUT_BPT = 394_625_028 / 10_000_000  # existing mesh-1e7 manifest
GIB = 1 << 30


def model(n):
    # capture::triangle_bytes: POSITION/NORMAL/TANGENT/UV0, geometry estimate.
    geometry = 20 + 3 * (12 + 12 + 16 + 8)
    nodes = 256 * (n // 8193 + geometry * n // ((96 << 20) + 1)) + 16
    # Default fan-in 64, 8 MiB chunks; round slack upwards for all record widths.
    slack = 64 * (8 << 20)
    triangle = (106 + 3 * (12 + 16 + 8)) * n
    positioned = 3 * (37 + 12 + 16 + 8) * n
    node_bytes, spans, welds = 72 * nodes, 56 * nodes, 39 * n
    leaf = triangle + node_bytes + welds + 15 * n + 64 * min(n, nodes)
    stages = {
        'capture_sorts': max(positioned, triangle, 75 * n) + slack,
        'sorted_sequence': triangle + slack,
        'tree': triangle + node_bytes + spans + 66 * n + slack,
        'corner_depths': triangle + node_bytes + welds + 63 * n + slack,
        'leaf_plans': leaf,
        'lod_prepass': leaf + 90 * n + slack,
    }
    peak = max(stages.values())
    return dict(triangles=n, cells=(n + 262143) // 262144,
                minimum_leaves=(n + 8191) // 8192, node_bound=nodes,
                input_bytes_estimate=round(INPUT_BPT * n),
                four_times_input_estimate=round(4 * INPUT_BPT * n),
                stages={k: dict(bytes=v, bytes_per_triangle=round(v/n, 3),
                               gib=round(v/GIB, 3)) for k, v in stages.items()},
                spill_plus_ten_percent=round(1.1 * peak),
                output_reservation=357 * n,
                admission_before_rounding=round(1.1 * peak + 357 * n))


if __name__ == '__main__':
    result = model(N)
    result['balanced_image_free_page_reservation_gib'] = (
        4096**2 * 182 + (192 << 20)) / GIB
    result['photogrammetry_after_seconds_at_N'] = {
        name: round(seconds * N / 9_999_999, 1)
        for name, seconds in dict(capture=112.984021177, leaf_build=137.398549575,
                                 lod_prepass=14.540987737, lod_simplify=61.388291549,
                                 seam_resolution=97.515767008,
                                 lod_finish=108.859891066, proxy_bake=3046.437844734,
                                 archive=22.701554301).items()
    }
    print(json.dumps(result, indent=2))
