# Independent F1b1 review against fdc5d18

Reviewer authored none of the production implementation or acceptance oracle. Reviewed the new contract and production diff: bounded source texture admission, UV accessor roles/normalization/ranges, complete corner association under reflection, per-leaf resource closure/remapping, shared image publication, report version and frontend adapter changes, and consumer lifecycle ownership. No Cargo runs, implementation edits, external publication, or historical output-digest oracle were used.

## Findings and executed repairs

The initial compiled implementation had the following actionable issues. Independent fixtures were authored with standard-library PNG framing/CRC/zlib and GLB bytes; JPEG was encoded and independently decoded by Pillow 12.3.0.

1. **P2: repeated UV admission amplified work by primitive count.** The new `primitives` TEXCOORD scan traversed all accessor elements for every primitive, while positions/normals were cached. A 1.2 MB source with 60,000 shared position/UV values and tiny three-index primitives took 0.024 s for one primitive, 0.262 s for 64, and 1.965 s for 512. These inputs stay within source, accessor, primitive and triangle ceilings. With the checked_texcoords repair the same cases took 0.025, 0.026 and 0.031 s. Timings are observations on this host, not portable performance guarantees.
2. **P2: base-color image admission accepted 16-bit PNG channels.** Published glTF 2.0.1 §3.9.2 requires eight-bit base-color values; the initial candidate accepted independently authored valid 16-bit RGBA PNG. The revised explicit eight-bit decoded-channel profile returns Unsupported, retaining an unchanged eight-bit positive and exact encoded-byte forwarding. Khronos validator 2.0.0-dev.3.10 did not detect the bit-depth issue; this finding uses the primary normative text, not its validator result.
3. **P2: complete pixel decode did not prove complete encoded-image framing.** The initial candidate accepted a bad IEND CRC, bad post-IDAT ancillary CRC, trailing chunks after IEND, APNG and JPEG with terminal EOI removed. The revised bounded PNG chunk/CRC/terminal scan rejects the malformed PNG controls as InvalidInput; APNG is explicitly Unsupported. JPEG without terminal EOI is InvalidInput. Correctly framed ancillary PNG and JPEG controls still pass. Missing IEND and truncated raster/entropy controls were already rejected and remain rejected. This is a finite framing complement to the image decoder, not a claim of a second exhaustive PNG/JPEG validator.

All observed findings are repaired in the reviewed binary. No remaining actionable issue was identified in this bounded review.

## Frozen execution and independent positive association proof

Initial binary `/home/bend/.cache/rusty-tiles-f1b-initial-candidate`, SHA-256 `3161c1648e53e87f0066f84e88e2bd0af95815d32a870519a8f5a3e4ca77a31f`. Revised binary `/home/bend/.cache/rusty-tiles-f1b-reviewed-candidate`, SHA-256 `534f58cace0864496aaea98099c88d13b41e5a18fe07f2676dee31de3c31a9d6`; hash verified before executing probes. Final receipt records production file hashes because source was still uncommitted during review. Root owns final source-commit binding and full test-suite evidence.

Fifteen original boundary/work controls had expected exits and typed categories. A separately authored selected-scene control had two one-triangle leaves, one reflected instance, two textures sharing source image index2 with distinct sampler presence, unused image0/image1 and an unselected scene. Independently decoded leaf POSITION/TEXCOORD arrays matched every ordered corner, including the reflection swap. Every leaf contained exactly one material/texture/image closure; sampler omission and texCoord omission versus explicit zero remained distinct. Exact archive inventory was tileset.json, conversion.json, two leaf GLBs, textures/2.png and the 3TZ index. The shared image appeared once with source bytes unchanged; report selected-image counts were 1 image / 70 bytes / 1 pixel. PNG, ancillary-PNG and JPEG positive outputs also forwarded exact image bytes, and embedded conversion.json equaled CLI meshReport.

Receipts: `fresh-review-initial.json` and `fresh-review-final.json`. The independently authored replay script is retained at `review_probes/replay.py`; original script/input hashes and persisted replay hashes are retained in the final receipt. A fresh regeneration and check completed successfully; all 15 original boundary/work input hashes matched the original independent inputs. `fresh-review-replay.json` retains the replay execution and generator-bound manifest. No generated large fixtures are stored in the repository. Positive JPEG provenance is recorded there. The separate oracle and root own broader normalized/interleaved UV controls, lifecycle faults, installed-wheel parity, viewer appearance and measured resource acceptance; this review does not replace those gates.

## Architecture and scope judgment

The source decoder receives one byte snapshot and owns decoded triangle/image data; it resolves no paths. Image edge/pixel/copied-byte ceilings include unused images and repeated references; decoded raster storage is released serially. The leaf encoder accepts prepared identities, emits f32 UVs, remaps dependency indices deterministically and discovers no external dependencies. The consumer chooses selected image closure once, writes deterministic numeric identities through the checked writer, includes them in the archive inventory and owns report/publication through F0. None of the frontend changes adds a competing semantic interpretation. Existing geometry partition behavior and routing-error policy are retained. This is a bounded embedded-image slice; it does not close external binding, unsupported material channels, identity/metadata, LOD approximation or #121's remaining obligations.

Primary references: [published glTF 2.0.1](https://registry.khronos.org/glTF/specs/2.0/glTF-2.0.html), revision 8e798b02d254cea97659a333cfcb20875b62bdd4, especially accessor normalization/alignment and base-color texture semantics; [PNG third edition](https://www.w3.org/TR/2025/REC-png-3-20250624/) for chunk integrity, terminal structure and APNG distinction. Contract under review: docs/architecture/f1b-contract.md.

## Fresh replay

Requires Python 3 with Pillow (original JPEG provenance: Pillow 12.3.0). From the repository root, choose a work directory that does not already exist and a built CLI:

```sh
python3 -B bench/architecture_audit/f1b1/review_probes/replay.py generate --work /path/to/fresh-review-work
python3 -B bench/architecture_audit/f1b1/review_probes/replay.py check --work /path/to/fresh-review-work --binary /path/to/rusty-tiles --output /path/to/replay-receipt.json
```

Optional `--expected-sha256 SHA` pins the binary before checking. Generation writes a manifest binding every freshly authored input and the script itself; checking rejects changed inputs or a changed generator. The check asserts outcomes/categories, exact PNG/JPEG image forwarding, CLI/embedded-report parity, selected closure inventory, remapped dependencies and ordered reflected UV/position corners. It records UV-reuse timings without a host-dependent speed threshold. The script imports neither production code nor the acceptance oracle.
