# H27 immutable proxy appearance core

This slice supplies the internal `mesh_large::bake` component. It preserves
raw POSITION words, decoded oriented geometry, node TRS, source/material
identity and arbitrary integer feature sets. Its charts, raster, pre-encode
child correspondence, normal frames, PBR class mips and categorical pages feed
a mandatory complete-member assembler. Only physical complete-member budget
feedback advances the density schedule. Appearance displacement is reported.

Retained children pair a page with its decoded UVs separately for each PBR
slot. H8 leaves independently crop those bindings; proxy bindings share the
new chart UVs. Generated normals use original-weld crease fans and area
weights, with decoded-UV tangent seams and actual emitted corner interpolation.
Zero-area and scalar-only faces retain geometry/COLOR without unused atlas
requirements. Positive child/error density is preserved; the finite source
heuristic is used only when that prescribed density is zero.

PR #216 now provides the H17 finishing sink, accepted leaf pre-encode pages
and complete GLB writer/readback. Bounded owner readers/writer leases, accepted
leaf renderer/source witnesses and pipeline activation remain dependencies.
No component test establishes those integration or publication gates. In
particular, the probe packets used for source/raster tests are not certified
H17 packets, and no test returns an original untextured GLB as an assembled Fit.

The actual H17 normal-frame domain uses identity or positive uniform node
scale. The independent storage fixture additionally exercises nonuniform TRS
word preservation; that fixture does not expand the admitted bake domain.
Complete assembly must verify actual decoded UV/NORMAL/TANGENT/COLOR, every
PBR page/sampler/material remap, every original feature set and all structural
metadata columns, including unreferenced V/F/custom tables. Geometry-only
`FaceIdentity` readback cannot prove that appearance or metadata was retained.

Source normal images retain raw linear components through filtering before
literal normal scale and the authored renderer TBN. Proxy normal mips use
common-world reduction and a representative frame for each committed texel.
Floors compare the committed texture mixture through actual interpolated
render frames; separate metrics report world-mixture chart bleed. Gutters
have no reduction support but participate in physical child texture filtering.
Class/ID pages use the merged #208 PNG, whole-tuple mode and nearest APIs.

`src/mesh_large/bake/controls.rs` contains the independent arithmetic/storage
controls; `probe_tests.rs` exercises actual admitted captured sources. Ordinary
`cargo test --lib mesh_large::bake::` runs both. The stdlib oracle generator
uses pivoted elimination rather than the production frame inverse, audits raw
storage and metadata independently, and checks deliberately wrong references:

```sh
python3 bench/architecture_audit/mesh_large_h27/generate_controls.py \
  --output-dir "$HOME/.cache/rusty-tiles-evidence/h27-bake-20261014/controls-replay"
```

It emits a 5,444-byte fixture with SHA-256
`b31364516275ef5dcd58fbb5f21569666086ec115cd33cebb5a4b4f89f2d2496`.
The fixture is an authored immutable-storage witness with an analytic
zero-distance reference; it has not executed H17's certificate producer.
Generated reports and runtime receipts belong in the persistent evidence cache.

`run_mutants.py` compiles and executes isolated component mutations, restores
each exact source in `finally`, and records source/binary/log hashes. Hold a
coordinator Cargo lane lock for the entire run. Compiler failures do not count
as caught mutations, and runtime acceptance requires exactly one failing test.
Afterward rebuild and rerun the unmodified focused suite before reviewing.

```sh
flock -o "$HOME/.cache/rusty-tiles-cargo-b.lock" nice -n 10 taskset -c 8-11 \
  python3 bench/architecture_audit/mesh_large_h27/run_mutants.py \
  --cases bench/architecture_audit/mesh_large_h27/mutants.json \
  --target-dir "$HOME/.cache/rt-target-h27" \
  --output-dir "$HOME/.cache/rusty-tiles-evidence/h27-bake-20261014/mutants-replay"
```

At code checkpoint `e441761`, 34 focused controls pass with zero skips,
including actual five-slot encoding/independent image readback for all four
profiles. All ten compiled mutations fail at their intended runtime checks;
all source bytes are restored and the fresh 34-control suite passes again.
The final nonauthor verdict is GO for this preparatory component only.

Pinned cache receipts (SHA-256):

- `compile-final-source.json`:
  `1ee6a5c8a7730d87690fa24c1b139d895ab1a77fa3f8d915d4c0e6be0e202bfd`.
- `tests-delivery-final.log`:
  `3a30566e4db0bcb89c9eff1eb494142616e7d591bc44117c825f6459f4aa3c3c`.
- `tests-restored-final.log`:
  `aea4e0b9c064cc8e4e29f01322419e54d8953031e21459dc785c220f89291c05`.
- `mutants/matrix.json`:
  `7711ab64770d78866c2ce8d948c8f1df53cd3a5f4efcbcd63fcbec9c82a06066`.
- `review-mutant-receipts.json`:
  `5b51aedb63f4b85d3b36014d055a2ce0757023e8338bb63ca65e5059d9914e9f`.
- `review-acceptance.md`:
  `a5275d79a04abe5738354ecc3450852384468c82f2159ad8bc8fb83d2e519290`.

At historical checkpoint `e441761`, owned formatting/changelog/tree checks
passed but strict workspace Clippy and full workspace formatting failed in
inherited paths. The new #216 integration checkpoint below passes both complete
workspace checks; the historical failure receipts remain retained.

The durable coordination and acceptance records are under
`~/.cache/rusty-tiles-evidence/h27-bake-20261014/`: `coordinator-request.md`,
`coordinator-response.md`, `coordinator-followup-clippy.md`, the source
manifests, build/test logs, mutant matrix and final nonauthor review. The
response accepts the handoff request and reserves its implementation to Opus.
H17/complete assembly, independent final-GLB semantic readback, depth-six
recursive drift, full-pipeline RSS/scratch, large meshes and consumer proof
remain separate open acceptance obligations. This slice runs no browser or
large-mesh work and provides no release or retirement acceptance.


## H17 handoff adapter checkpoint

At code checkpoint `b8dab956634c4dde745781ab66b4038590ef1f92`, #216 is merged and this PR is stacked on
`ml/h17-bake-handoff`. Four bake-owned modules translate immutable individual
source tuples inside shared-material glTF primitives, complete PBR/ID appearance
and no-raster bindings, and exact-bit retained child packets. The leaf adapter
requires an explicit accepted renderer/source witness; it derives each actual
slot's UV/page/sampler/normal-scale binding and rejects understated decoded-UV
texel spacing. The packet preflights resident plus serialized ownership and
checks unit renderer axes, normal-frame invertibility, complete mip/support
ledgers, duplicate base data and bounded metadata.

`bake::integration::ProxyTextureBaker::new(BakeLimits)` takes caller-resolved
policy. Its nonempty production entry currently returns `Unsupported` before
unleased allocation. Root must supply bounded certified-member/packet reads,
accepted leaf renderer/source witnesses and final density, enforce the complete
writer's residual memory allowance and activate the real baker. It must also
bind `Verified` to actual assembled bytes, preserve integer feature values above
2^24 and remove the handoff wire's hidden 255-set count limit. These requests
are in draft PR #215 and the durable `coordinator-integration-request.md`.

All 44 focused controls pass with zero skips. Two new tiny **constructed**
structural controls use the actual root complete writer, then independently
read all five PBR images, categorical bindings, exact render attributes,
source/feature metadata and unreferenced UINT64/FLOAT64 columns. They prove
that structural writer domain, not authentic H17 -> real core -> production
execution. All six `integration_mutants.json` regressions compile and fail at
their intended assertions; exact restoration yields the same passing binary.
Strict workspace/all-target Clippy, complete workspace formatting, changelog
and indexed tree-budget checks pass. Final independent verdict is GO for this
bounded preparatory integration; full H27 production/resources remain NO-GO.

Reproduce the six additional mutants with the existing coordinator-only runner
by passing `--cases bench/architecture_audit/mesh_large_h27/integration_mutants.json`.
Keep the Cargo lane lock for the whole run, then rebuild the restored suite.

Integration evidence pins (SHA-256, relative to the persistent evidence directory):

- `integration-final-inputs.json`: `ae32276736c565b929912853eb47df4cf7cd08715cca7bf76370a098065b4a7f`.
- `integration-restored-artifact.json`: `295b33185644f8f8a331a5cb92e0dcc5ffe1776ccb3d477e003d18a875015cf6`.
- `integration-restored-tests.log`: `505d9fb4494f1dd549e8ef2a5c9d1f259cff603f31c01bb13a516b505dab9b5d`.
- `integration-mutants/matrix.json`: `3c581de3711e85c74295096093f8ea5468ce381ba7408ddcb265b3d81e193ef3`.
- `integration-clippy-final.log`: `98f17c2bde2a301c6da6f981af834fafe9796e6f085e225d7be88a02a535242c`.
- `integration-acceptance.json`: `6a4bd0357724dcc43e172a63d9173c552aac01ab09afee82236a5cf3f732cd7e`.
- `review-integration-final.md`: `571c4fa8cb2666d2e8a7208a6ea1e35eab43f99a36bfb553a1b85ac1ceaa44b6`.
- `coordinator-integration-request.md`: `01a74fec0b5fd45379052dc220d396d137645d2ac8aa3180d0388c7f71306ff0`.

The baseline and restored binary both hash to
`8663cde0a571dab18e8fba70ef293ce7fc9b5ac3004c76b415c07eb818698838`.
The full input manifest covers 2345 actual files. Historical ten-mutant and
34-test receipts above are separate; this checkpoint adds six mutant controls
and reruns all 44 focused tests against the concrete #216 API.
