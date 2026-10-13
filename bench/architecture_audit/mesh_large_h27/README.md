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

The H17 finishing sink, retained accepted leaf pre-encode packets, complete
GLB writer/readback and pipeline activation are coordinator-owned dependencies.
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

Owned Rust formatting, changelog and final indexed tree budget pass. Strict
workspace Clippy fails on fourteen inherited H17/integration diagnostics and
reports none in bake. Full workspace formatting also fails on nine inherited
paths. These are concrete owner follow-ups, not waived successful gates.

The durable coordination and acceptance records are under
`~/.cache/rusty-tiles-evidence/h27-bake-20261014/`: `coordinator-request.md`,
`coordinator-response.md`, `coordinator-followup-clippy.md`, the source
manifests, build/test logs, mutant matrix and final nonauthor review. The
response accepts the handoff request and reserves its implementation to Opus.
H17/complete assembly, independent final-GLB semantic readback, depth-six
recursive drift, full-pipeline RSS/scratch, large meshes and consumer proof
remain separate open acceptance obligations. This slice runs no browser or
large-mesh work and provides no release or retirement acceptance.
