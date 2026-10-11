# Validator caps for 100,000-tile default vector output (2026-10-11)

On 2026-10-11 the user decided to raise the validator's ceilings, with measured
time and memory evidence, so that default vector output at the 100,000-tile
`--maxTiles` default validates. The vector default stays unchanged.

The old caps did not admit this output. At 96,463 tiles the develop validator
(68b7531) refused a default vector archive at the entry limit. Six further
ceilings would then have refused it. This directory measures what that output
consumes and the cost of each ceiling at its old and new value.

## Decision

| `ValidationLimits` field | Old | New | Demand at 100k tiles, worst measured | New margin |
| --- | --- | --- | --- | --- |
| `archive_entries` | 65,536 | **524,288** (2^19) | 250,874 (2.51/tile) | 2.09x |
| `hierarchy_visits` (and implicit presentation nodes) | 65,536 | **262,144** (2^18) | 125,462 nodes (1.25/tile) | 2.09x |
| `references` | 262,144 | **1,048,576** (2^20) | 250,872 | 4.18x |
| `central_directory_bytes` | 16 MiB | **32 MiB** | 24.1 MB (241 B/tile) | 1.39x |
| `document_items` (JSON value nodes per document) | 65,536 | **131,072** (2^17) | 100,607 (`tileset.json`) | 1.30x |
| `member_bytes` | 64 MiB | **128 MiB** | 92.5 MB (`vector-build.json`, 925 B/tile) | 1.45x |
| `document_decoded_bytes` (and implicit presentation bytes) | 64 MiB | **128 MiB** | 71.9 MB (719 B/tile) | 1.87x |

The demand column is the largest per-tile ratio measured in each content class,
scaled to 100,000 tiles (`results/margins.json`).

Why each value:

- **Entries: 2^19, not 2^18.** The forest design modelled 1.08–1.20 entries per
  tile. The develop output measures **2.38–2.51**. Each content tile has a
  display GLB and a retained source GLB, and about 0.19–0.25 implicit roots per
  tile each add a subtree and an external `implicit-tileset-*.json`.
  - With 2^18, the margin at 100,000 tiles would be 1.04–1.10x.
  - 2^19 restores the at-least-2.1x margin that the design asked of 2^18.
- **References: 2^20.**
  - Implicit vector output references every member except the index and
    `tileset.json` once (references = entries − 2).
  - An explicit tree references each external tileset twice: once as tile
    content, then again when the document is entered. So references =
    entries − 1 + K. At 2^19 entries, the `se-19` stress needs 524,375
    references.
  - A references cap of 2^19 would therefore make the entry cap unreachable.
    The cost is one counter and one hash lookup per visit.
- **Visits: 2^18.** Implicit vector output charges about 1.19–1.25
  presentation nodes per tile to the same budget, which gives 2.09x. The
  traversal tasks are about 1.0 per tile.
- **The other four** bound structures that grow with tile count:
  - central directory: about 97 B per entry;
  - `tileset.json` value nodes: `rustyTilesSourceContents` lists every retained
    source;
  - `vector-build.json` bytes: this member is only hashed, but it is admitted
    per member;
  - implicit presentation bytes.

  Each sits below 1x at 100,000 tiles under its old value, as shown under
  "Measured demand" below.

`json_bytes` stays at 8 MiB. `tileset.json` needs 73 B per tile, which leaves
a 1.15x margin.

## Measured cost at the ceilings

The measurements ran on Linux 7.2.9, a 20-CPU host with 62 GiB of RAM, using
native release builds. Every run was one `validate --json` child, pinned with
`taskset -c 0,1 nice -n 10` while holding the shared Cargo lock.

- Time is the median of three runs; every run is within 3.5% of its median.
- RSS is the maximum `ru_maxrss` reported by `wait4`.
- The load average was 1.3–2.4.
- `results/stress.json` and `results/vector.json` hold every run.

### Stress archives

These are synthetic explicit-tileset trees (`stress.py`, `stress_single.py`).
Each tile carries an in-box triangle GLB, so C1 admits every case and C2
reports Contained.

| Archive | Binary | Result | Time | Peak RSS |
| --- | --- | --- | --- | --- |
| `st-16`: 2^16 entries and 2^16 visits | old | admitted | 2.05 s | 181 MiB |
| `st-16-e1` / `st-16-v1`: one over | old | `resource_limit` | 0.01 / 1.23 s | 29 / 180 MiB |
| `cd-16m`: 2^16 entries, 16 MiB directory | old | admitted | 2.54 s | 278 MiB |
| `st-17`: 2^17 entries and 2^17 visits | new | admitted | 4.12 s | 272 MiB |
| `st-18`: 2^18 entries and 2^18 visits | new | admitted | 8.22 s | 454 MiB |
| `st-18-v1`: one visit over | new | `resource_limit` | 4.88 s | 448 MiB |
| **`se-19`: 2^19 entries, 2^18 visits** | new | admitted | **16.13 s** | **717 MiB** |
| `se-19-e1`: one entry over | new | `resource_limit` | 0.01 s | 30 MiB |
| `cd-32m`: 2^18 entries, 32 MiB directory | new | admitted | 9.89 s | 581 MiB |
| `cd-32m-p1`: one byte over | new | `resource_limit` | 0.01 s | 30 MiB |
| `items-16-object` / `items-17-object` (5.8 MB of keys) | new | admitted | 0.09 / 0.13 s | 105 / 127 MiB |
| `items-17-array-p1`: one value node over | new | `resource_limit` | 0.04 s | 86 MiB |
| `member-64m` / `member-128m` | new | admitted | 0.10 / 0.16 s | 148 / 212 MiB |
| `member-128m-p4`: 4 bytes over | new | `resource_limit` | 0.01 s | 30 MiB |
| `decoded-64m` / `decoded-128m` (meshopt views) | new | admitted | 0.15 / 0.26 s | 180 / 276 MiB |
| `decoded-128m-p4`: 4 bytes over | new | `resource_limit` | 0.09 s | 212 MiB |
| `st-19`: 2^19 entries and 2^19 visits | wide probe | admitted | 16.83 s | 817 MiB |

The `wide probe` row uses the probe binary, which has every ceiling widened.

From `st-16` to `st-18`, each tile with an entry and a visit costs **31 µs and
1.41 KiB**. From `st-18` to `se-19`, each extra content entry, a GLB
inspected by C1 and C2, costs **30 µs and 1.03 KiB**. Both are linear across
2^16–2^19.

A 128 MiB member costs one 64 MiB larger read buffer than at 64 MiB. 128 MiB
of decoded views costs about 96 MiB more than at 64 MiB.

### Real vector output

These archives come from the develop converter: the default implicit output,
with `--maxFeatures 2` for deep trees (`make_vector.sh`).

| Archive | Tiles | Entries | Old binary | New binary |
| --- | --- | --- | --- | --- |
| `points-81k-mf2` | 96,463 | 233,158 | `resource_limit` (entries) | **admitted, 12.50 s, 844 MiB** |
| `points-78k-mf2` | 90,463 | 215,158 | `resource_limit` (entries) | admitted, 11.69 s, 771 MiB |
| `points-90k-mf2-t130k` (`--maxTiles 130000`) | 114,463 | 287,158 | `resource_limit` (entries) | admitted, 15.57 s, 963 MiB |
| `mixed-s7.8-mf2` (perf-lane composition) | 96,427 | 231,158 | `resource_limit` (source bytes) | `resource_limit` (source bytes) |
| `points-165k-mf2-t300k` | 198,927 | 472,630 | — | `resource_limit` (source bytes) |
| `points-78k-mf2-explicit` (`--explicit`) | 90,463 | 90,437 | `resource_limit` (entries) | `resource_limit` (`tileset.json` 59 MB over 8 MiB JSON) |

With every ceiling widened, the probe binary passes `mixed-s7.8-mf2` in 17.0 s
with 843 MiB peak RSS. It passes `points-165k-mf2-t300k` in 25.9 s with
1.63 GiB. That archive exceeds the new ceilings and brackets the largest
implicit archive they admit:

- entries reach 2^19 at about 209,000 tiles;
- presentation bytes reach 128 MiB at about 187,000 tiles;
- `vector-build.json` reaches 128 MiB at about 145,000 tiles;
- points content reaches the 1 GiB source ceiling at about 136,000 tiles.

## Measured demand (per tile; `results/margins.json`)

| Consumed fact | Points corpus | Mixed corpus | Old ceiling binds at |
| --- | --- | --- | --- |
| Entries | 2.38–2.51 | 2.40–2.42 | about 26,000 tiles |
| Presentation nodes | 1.19–1.25 | 1.20–1.21 | about 52,000 tiles |
| `tileset.json` value nodes | 1.00 | 1.00–1.01 | about 65,000 tiles |
| `vector-build.json` bytes | 916–925 | 893–901 | about 72,000 tiles |
| Central-directory bytes | 230–241 | 232 | about 70,000 tiles |
| Presentation bytes | 687–719 | 696–697 | about 93,000 tiles |
| `tileset.json` bytes (8 MiB, unchanged) | 73 | 73 | about 115,000 tiles |
| Archive bytes (1 GiB, unchanged) | 7,500–7,900 | 11,700–11,900 | 136,000 points / 91,000 mixed tiles |
| Bytes read (2 GiB, unchanged) | 13,300–14,000 | 21,700–22,000 | 153,000 points / 98,000 mixed tiles |
| Payload elements (16 M, unchanged) | 93–99 | 533–539 | 162,000 points / 30,000 mixed tiles |

## Not changed, and why

The four content-volume ceilings stay where they are:

- `source_archive_bytes` and `archive_stored_bytes`: 1 GiB each;
- `total_bytes_read`: 2 GiB;
- `total_payload_elements`: 16 million.

They scale with payload density, not tile count. Points output fits all four
at 100,000 tiles with margins of at least 1.36x. The mixed perf-lane
composition (polygons with holes and 100-vertex lines) does not fit:

- payload elements reach their ceiling at about 30,000 tiles;
- archive bytes reach theirs at about 91,000 tiles;
- reads reach theirs at about 98,000 tiles.

Under the 4 MiB-per-tile budget, no finite volume ceiling admits every
100,000-tile output. Raising these ceilings would move the time bound. For
example, the mixed corpus at 100,000 tiles would need about 54 million elements
and 2.2 GB of reads. That is a separate decision, left open.

## Unproven or outside this slice

- **The worst case in this record is per ceiling, not combined.** No single
  archive was built that saturates every raised ceiling at once. The largest
  measured admitted cases are `se-19` (16.1 s, 717 MiB) and the
  114,463-tile vector archive (15.6 s, 963 MiB). The over-cap
  `points-165k-mf2-t300k` probe run (25.9 s, 1.63 GiB) is an upper bracket for
  implicit output. These are observations on one host, not an RSS promise.
- **Only two invented content classes were measured.** Clustered or otherwise
  differently distributed sources could produce more implicit roots per tile,
  and so more entries per tile. The structural ceiling is 4 entries per tile
  for single-content tiles, which is 400,000 at 100,000 tiles. That fits 2^19,
  but its 39 MB central directory would exceed 32 MiB.
- **`--explicit` vector output is still refused** above about 12,800 tiles.
  Its single `tileset.json` grows by 656 B per tile and reaches the unchanged
  8 MiB JSON ceiling.
- **Archives with denser vector content reach the volume ceilings first**, as
  described above.

## Files

| File | Role |
| --- | --- |
| `gen_vector.py` | Deterministic scaled perf-lane composition, or uniform points |
| `make_vector.sh` | Converts the vector archives with a given binary |
| `stress.py`, `stress_single.py`, `make_stress.sh` | Independent stdlib 3TZ writers at exact entries, visits, directory bytes, value nodes, member bytes and decoded bytes |
| `analyze.py` | Independent archive facts: entries, directory bytes, value nodes, payload elements |
| `probe.patch` | Measurement-only probe build: widened ceilings and one counter line on stderr |
| `measure.py`, `run_all.sh` | Pinned runner that holds the Cargo lock and records `wait4` costs |
| `margins.py` | Per-tile ratios and margins |
| `results/` | `binaries.json` (identities), `stress.json`, `vector.json`, `analyze.jsonl`, `margins.json`, `stress-plan.jsonl`, logs |

To reproduce, keep large files outside the repository:

```sh
W=/home/bend/.cache/vcaps/work
bash make_stress.sh $W/stress
bash make_vector.sh $W/../bin/rusty-tiles-68b7531-native $W
bash run_all.sh $W results
python3 -I analyze.py $W/*.3tz > results/analyze.jsonl   # implicit and explicit archives
python3 -I margins.py results > results/margins.json
```
