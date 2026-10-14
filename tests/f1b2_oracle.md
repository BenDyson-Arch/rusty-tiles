# F1b2 independent local resource binding oracle

Current replay uses schema4/profile `f1c1-placed-gltf-v1` with an explicit identity root transform and typed Local placement report under the [F1c1 contract](../docs/architecture/f1c1-contract.md). This remains local geometry/resource regression evidence. The separate [F1c1 oracle](f1c1_oracle.md) proves Earth placement and the full world transform chain; historical receipts below retain their original scope.

The previous replay asserted profile `f1b-core-pbr-gltf-v1` under the [F1b3 contract](../docs/architecture/f1b3-contract.md); historical receipts below retain their original profile. The unsupported-attribute-before-I/O control now uses TEXCOORD_2 explicitly because authored TANGENT is admitted by the new bounded profile. New channel/UV1/color/tangent proof is documented in [f1b3_oracle.md](f1b3_oracle.md).

`f1b2_oracle.py` independently authors and interprets local JSON glTF and GLB dependencies. It uses no production URI, filesystem, decoder, encoder or validator helper. It reuses the established independent F1b Python geometry/material/PNG/JPEG/index oracle after checking original dependency bytes and logical buffer ranges itself.

The historical F1b2 report oracle checked schema3, profile `f1b-local-gltf-v1`, root document `source_bytes`, and unique physical dependency `external_files`/`external_bytes` excluding the document. Repeated URI, percent and hardlink aliases are counted once as physical sources. glTF image indices remain separate output identities; two logical images referencing one file can produce two correctly named, unchanged image members. Unused dependencies count in source capture while the published archive includes only the selected image closure.

## Independent interpretation and fixtures

The URI reference parser is written here with explicit segment processing and hexadecimal octet decoding; it does not import the Rust resolver or use form-URL decoding. It parses unsupported URI structure before percent decoding, decodes each literal slash segment once, normalizes internal dot/contained parent segments and checks confinement. Literal `+` requires `%2b`; decoded `+`, `%` and `#` are filename data. Encoded separators and excluded portable filename characters are rejected without reparsing them as new URI components. Raw Unicode and UTF-8 percent forms address the same native name. URI syntax, byte/component ceilings and terminal directory locations have explicit sensitive self-tests.

The primary rules are [glTF 2.0 §2.8](https://registry.khronos.org/glTF/specs/2.0/glTF-2.0.html#uris) for reserved data escaping and relative paths, and [RFC 3986 §2.4](https://www.rfc-editor.org/rfc/rfc3986.html#section-2.4) for parsing components before exactly-once decoding. The bounded producer excludes optional schemes, authority, absolute/query/fragment forms and data URIs. Malformed escaping, literal spaces/backslashes/reserved data, decoded C0/C1 controls/invalid UTF-8 and terminal decoded dot locations are InvalidInput. Valid excluded URI forms, escaping the base, encoded separators and portable filename exclusions are Unsupported.

The oracle reads declared files, rejects dependency symlink components/nonregular files, checks opened file identity/size/mtime and records SHA256. It checks actual bytes against each logical buffer's declared prefix before constructing an oracle-only virtual GLB. Buffer views are rebased by their original buffer index; image bytes are appended to separate views. This bridge preserves independent binding and lets the existing analytical corner/material/image matcher check production output without duplicating that reader. External trailing buffer bytes count in physical source bytes and are permitted, while views cannot use bytes beyond the declared prefix. External image MIME is inferred by signature when absent; explicit MIME must agree.

The authored positive fixtures cover two buffers with equal offsets and distinct content; mixed BIN/external GLB and JSON-only GLB; image views in an external buffer at offsets overlapping geometry in another buffer; PNG/JPEG URI images and omitted external MIME; raw/encoded Unicode, space/percent/hash/plus filenames, once-decoding, contained dot paths; repeated URI/percent/hardlink and buffer aliases; different pixel images with identical basenames in different directories; unused images/buffers and trailing JSON whitespace. Nested/reflected source geometry and precise associated UVs/materials come from the independently authored F1b fixtures.

Normalized u8/u16 positives declare raw integer UV extrema, including four-byte alignment for u8 attributes. Static ordinary files and authoring/hash/accounting manifest are under `tests/fixtures/f1b2/`. Git does not preserve hardlink identity, so hardlink fixtures are created explicitly during replay. `manifest.json` records root bytes, unique dependency bytes, every asset SHA256 and selected image IDs.

## Replay and callable APIs

```sh
python3 -B tests/f1b2_oracle.py --self-test
python3 -S -B tests/f1b2_oracle.py --self-test
python3 -B tests/f1b2_oracle.py --binary /absolute/path/to/rusty-tiles \
  --json-output /tmp/f1b2-oracle.json
python3 -B tests/f1b2_oracle.py --fixture /tmp/f1b2-source --variant multibuffer
```

Self-tests run before candidate conversions. There are 23 positive variants at leaf limits 1, 3 and 1000 (69 conversions). The final URI refinement produces 60 declaration/refusal bundles, plus four filesystem setups and four output hardlink alias targets, each at leaf limits 1 and 1000 (136 refusal checks). InvalidInput exit 3, Unsupported/InvalidRequest exit 2 and Io exit 1 are checked separately from the error kind.

Refusals include malformed/unsupported URI categories and finite URI work limits, missing used/unused files, image source contradictions, invalid unused PNG/MIME, short buffers and declared-prefix range failures, broad used/unused accessor bounds, inverted bounds before missing-file I/O, buffer count/declared byte sum, root JSON/GLB/file/aggregate physical byte limits, BIN mapping contradictions and unsupported source semantics before dependency I/O. An earlier valid URI to a missing file cannot outrank a later invalid/unsupported URI or unsupported whole-document semantic. Missing files remain Io. Symlink leaf/directory dependencies, directory images and document hardlink aliases are controlled separately. Output hardlinks to the document, buffer, used image and unused image must fail InvalidRequest while preserving all bytes and creating no workspace.

Sensitive independent checks first prove URI interpretation literally, then detect a wrong source base using changed texels, a changed logical buffer binding and a same-size buffer payload mutation. The full F1b sensitive suite remains active for UV corner association, normals, winding, alpha/material/sampler presence, texels and archive dependency closure. No historical archive digest serves as expected geometry or pixels.

Callable interfaces for Rust/CLI/installed-wheel parity are:

- `VARIANTS` and `write_fixture(root, variant, triangles=8) -> Path`.
- `inspect(source, archive, leaf_limit) -> dict`, including independently checked `report`.
- `refusal_bundles() -> [(name, bundle, expected_kind), ...]` and `write_bundle(root, bundle) -> Path`.
- `source_bundle(...)` and `bind_source(...)` for fixture authoring/accounting.

`tests/f1b_oracle.py` retains embedded-source geometry/texture coverage with the current schema4 Local report and zero external counts. Its former valid relative URI refusal is replaced by an excluded URI form; the original F1b1 receipts remain historical records. Callback CWD/source deletion/mutation, cancellation, publication and resource boundary measurements are covered by separate producer/runtime/frontend tests, not inferred from this Python reader.

## Evidence limits and provenance

The source contract requires stable files during preparation. Opened identity/length/mtime checks and immutable post-preparation bytes do not establish an atomic snapshot across files or resistance to an adversary restoring observable metadata. SHA256 here identifies independently observed fixture bytes; it does not change the production guarantee. Output comparisons inspect all oriented source corners, material/image/sampler meaning and exact selected resource closure, independently of reports.

Initial `portable-oracle-initial.json` and `portable-embedded-regression.json` under `bench/architecture_audit/f1b2/` pin candidate `42b895f` / binary SHA `0992615d…`. They passed 63/112 and 84/90 positive/refusal checks respectively before the explicit URI escaping refinement. They are retained as pre-refinement evidence, not final acceptance of raw reserved filename characters. Final `portable-oracle-final.json`, `portable-embedded-final.json` and `static-fixture-final.json` pin source commit `ce8a89c58e2781d72c2ac5dff686de2f8d340850` and binary SHA `522b5e62…`. They passed 69/136 external cases, 84/90 embedded cases and all 13 fixed manifest fixtures. Final replay receipts identify their binary hash; provenance records the corresponding source commit. The previous `1078240` receipts are preserved as `*-intermediate-1078240.json`. `independent-provenance.json` records tool versions, static assets and oracle source hashes, source baseline `641e1b4`, and fixture authoring provenance; the frozen JPEG's independent pixel/encoder receipt remains in `tests/fixtures/f1b/manifest.json`.

This is the local dependency slice of #121. It supplies no network/data URI, richer material/extension, placement, LOD/error-bound, implicit hierarchy or picking acceptance; it does not close #121 or authorize release.
