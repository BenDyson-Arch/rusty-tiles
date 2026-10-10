# Independent CI producer-framing control review

Reviewed root-authored tests/c1_producer_alignment.py and tests/fixtures/c1-framing-staircase.geojson. This is a test-only change: actual vector producer invocation, b3dm total/alignment, embedded GLB framing, and BIN padding assertions remain. The fixture is a closed simple integer staircase in explicitly local metre coordinates:50 ring vertices, analytic area300m². maxVertices32 forces polygon fragmentation. Assertions require exactly1 input feature,1 fragmented polygon and0 skipped features, preventing a vacuous framing pass.

The previous runner used the native-GDAL countries fixture. The Rust CI job enables native-jpeg with the portable vector backend, which rejects its feature15 geometry. That feature-specific failure prevents the intended framing assertions from running. This review independently verifies the replacement fixture exercises those assertions on both feature profiles; I did not independently rerun the original remote CI failure.

| Executed artifact | SHA256 | Result |
| --- | --- | --- |
| Native-geospatial final0a1cc94 | d4f2bae195067d7b93aec1e98cb82085553c1410f17f9bac3163b5120f5b7eef |6 actual b3dm members pass all framing/BIN assertions |
| Native-jpeg portable CI freeze | 4c4e24dd97b95e8ec1ae0456764c9547c60ada80e079f9c67f4a3bde79f6dde0 |8 actual b3dm members pass all framing/BIN assertions |
| Original native review freeze, root-attributed source1c87 | 3bd3d8b2986b8b84ea110d500a6d058329159203f9bfabcf26a69d9ba02c8bc2 |Runner fails expected b3dm alignment assertion;4 of6 actual members have size remainder4 modulo8 |

All three runs used distinct fresh cache directories. Native/portable runners verified source read-only preservation; fixture SHA256 e3a0cc6821d5a80a3fc29e7e6533e03579a63f34f80e04220e551e377a19f7b5. Independently inspected the baseline archive after the expected runner failure to establish6 total members and4 misaligned members. Thus the smaller backend-neutral fixture remains sensitive to the actual historical framing defect, rather than merely producing content that always happened to align. Exact member counts can differ by backend; the assertions intentionally require nonzero actual content and successful source/fragment counts rather than identical triangulation.

Independent executed records: /home/bend/.cache/c1-framing-independent-native.json, /home/bend/.cache/c1-framing-independent-portable.json and /home/bend/.cache/c1-framing-independent-baseline.json. Rehashed all83 compiled source paths in final build-manifest.json after the runner change: unchanged. No production code edit or Cargo run performed by this reviewer. Root owns the runner/fixture change and full Rust CI command sequence; this artifact certifies the narrow independent framing-control checks above.
