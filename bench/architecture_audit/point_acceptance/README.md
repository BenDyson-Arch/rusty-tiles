# P1 independent point acceptance

The executable oracle is [`tests/p1_point_oracle.py`](../../../tests/p1_point_oracle.py). Its source and archive readers use Python's standard library. It imports no product parser, encoder, coordinate transform or implicit expander. The admitted domain and lifecycle decisions are in [`point-acceptance-contract.md`](../../../docs/architecture/point-acceptance-contract.md).

The fixture writes LAS 1.2 formats 0/1/2/3 and LAS 1.4 formats 6/7/8 from literal public header, point-record and 192-byte scalar ExtraBytes descriptor layouts. A separate raw reader enumerates required dimensions from input bytes, independent of the output schema. Flags, signed scan angles, GPS time, RGB/NIR, every scalar ExtraBytes type, large exact integer values, independent scale/offset flags and noData/min/max declarations are checked. Byte-identical original records retain distinct enumeration indices; coverage compares a multiset of every original index, never a set of coordinates.

The archive reader checks ZIP CRC, member authority, the MD5-name/64-bit-offset 3TZ index and complete reachable resource inventory. Its GLB reader checks chunk framing and actual BIN/view/accessor offset/count/stride ranges, selected-scene matrix/TRS composition, Y-up to Z-up conversion, rendered POSITION/COLOR_0, property rows and feature IDs. Its implicit reader independently decodes LSB-first availability, breadth-first Morton coordinates, available-tile metadata row ranks, child-subtree availability and external implicit tileset boundaries. Forced maxPoints=1 crosses subtree and external-root boundaries. No production `expand_tileset` is used.

Every full-detail point and required dimension must match independent source enumeration. Parent representatives are reconstructed from source positions under the declared first-original-record-per-voxel rule. Exhaustive nearest-representative distances check finite, nonincreasing error claims; every rendered point and descendant source position is checked against translated bounds. Spread, reordered, outlier, identical-record, chunk-boundary, explicit/implicit and concurrent/repeated runs exercise those assertions. Analytic WGS84 ECEF references include literal equatorial axis points and the independent ellipsoid formula; these are geographic checks, not certification of arbitrary projected CRS.

Negative controls replace rendered POSITION, drop a required field, repeat an original index, truncate BIN, overrun an accessor, add an orphan resource or remove reachable content. The controls rebuild valid ZIP CRC/index framing so a packaging error cannot mask the intended semantic failure. Separate literal reader self-tests exercise nested node transforms, nonzero accessor offsets and stride, malformed payloads, sparse/compressed profiles, unsupported required extensions and node cycles. Unsupported profile and source/output alias tests preserve an existing output sentinel. Rust lifecycle tests own event ordering, callback CWD changes, cancellation and fault injection; CLI concurrency here complements those tests.

## Reproduction

An absent binary or codec fails; acceptance never silently skips:

```sh
python3 tests/p1_point_oracle.py --self-test
python3 tests/p1_point_oracle.py --binary /absolute/frozen/rusty-tiles \
  --laz-python /absolute/python-with-laspy-and-lazrs \
  --json-output /tmp/p1-acceptance.json
python3 bench/architecture_audit/point_acceptance/resource_probe.py \
  --binary /absolute/frozen/rusty-tiles --json-output /tmp/p1-resources.json
```

[`p1_laz_adapter.py`](../../../tests/p1_laz_adapter.py) uses laspy/lazrs solely to compress the independently authored LAS fixture. The expected records and required dimensions still come from the original stdlib LAS reader, and archive/GLB/implicit decoding remains independent. All seven admitted formats run in both output modes with metadata attributes. The compressed-delivery encoder and versions are recorded in the artifact; this adapter is not a second stdlib LAZ decoder. An isolated cached environment can be invoked with `uv run --no-project --offline --with 'laspy[lazrs]' python tests/p1_laz_adapter.py ...`; `--no-project` is necessary to avoid building this repository's Python package.

`--baseline` records known legacy admissions and dropped ExtraBytes declarations rather than claiming those profile tests passed. It still requires baseline decoded source/rendered-position/multiplicity checks and negative decoder controls to pass. It must never be used for candidate acceptance.

## Evidence boundaries

The immutable baseline is commit `11912ce`; point sources match the issue baseline `5df15e7`. Binary SHA256 appears in every artifact. The baseline preserves raw UINT64/INT64 point values exactly but drops source ExtraBytes noData/min/max schema declarations. It admits scaled64 and zero/negative scaling profiles which P1 explicitly refuses, accepts header_size=0 in a no-VLR LAS1.2 source, and can replace source-identical or hardlink-alias `.3tz` input under `--force`. These are distinct observations; raw64 row fidelity is not a schema-metadata proof. Scaled I16 overflow and point-data offsets outside the file are rejected on baseline.

The resource probe observes Linux process RSS/high-water RSS, descriptors, threads and workspace/scratch bytes every 10 ms. It fixes maxPoints=256/chunkPoints=1024 while increasing source rows 1,024/8,192/32,768, then holds 8,192 rows while adding 32/128 UINT64 source fields. Actual point-record widths are recorded. It excludes fixture construction and oracle allocations from converter measurements. Short peaks may be missed; repeated probes may vary. Reader, chunk, coordinate adapter, spool, leaves, hierarchy and archive serialization share one process and are not individually attributed. Hierarchy/member inventory and scratch grow with rows/leaves; width grows chunk/spool/encoding costs. The probe establishes observed examples, not a maximum-width or worst-case memory bound. The profile's metadata-size admission ceiling does not bound whole-job memory by maxPoints alone.

The oracle admits embedded, uncompressed scalar point GLBs emitted by this consumer. It does not prove general glTF external delivery, sparse/compressed codecs, arbitrary material appearance, full metadata semantics, all CRS domains, crash durability or viewer behavior. Production `validate` still lacks actual BIN/accessor payload proof; these independent controls satisfy P1 decoding without closing #133 or overstating its current `structure`/`bounds` labels.
