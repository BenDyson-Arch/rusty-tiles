# A2 finite format and consumer settlement

Recommend the source-order owned-root representation as a **declared terminal-occupancy
interoperability profile** for the next consumer acceptance step. This audit does
not certify it as unrestricted 3D Tiles 1.1 conformance. The profile states its
interpretation: a content template associates a resource with an available tile;
the external-JSON slot is unavailable at every implicit root and available only
at terminal implicit leaves. The remaining literal root-template prohibition is
a separate standards claim gate, not a reason to repeat this semantic audit or
block independent archive/metadata ownership work.

The unambiguous single-implicit-tree alternative uses `TILE_TRANSFORM` rows, but
the intended Cesium client reads those rows without applying them. It is therefore
unsupported for preserving the source local frames in this client. Explicit
internal tiles plus implicit leaf roots is the smallest clearly compliant
alternative while preserving payload bytes and transforms; it is a different,
partial-implicit product and must be selected explicitly. There is no automatic
fallback between these products.

The coordinator has withheld authorization for a production A2 replacement
claiming valid 3D Tiles. Next independent work may advance private payload/archive
ownership while a nonauthor review adjudicates the profile interpretation.
No production, draft contract, prior audit, Git, Cargo, dependencies or browser
was changed/run by this lane. Baseline supplied by coordinator:
`e4d90897518d2b569fb5e99b876581523e4c418c`, fast-forwarded by coordinator after
PR #151 to `9d2973db06967f87d759c87c34d8fdd0921c86ed` with audited bytes unchanged.

## Captured execution

The fresh artifact folder is `/tmp/rusty-tiles-a2-format-settlement-final3`.
[results.json](results.json) binds the drivers, artifact manifest, raw reader and
client results, installed runtime packages/trees, and primary source identities.
[artifact-manifest.json](artifact-manifest.json) pins every candidate member and
the retained authored source model inputs. [client-results.json](client-results.json)
and [reader-results.json](reader-results.json) contain the exact raw outcomes.
[primary-sources.json](primary-sources.json) pins the 15 inspected official files
by URL, commit, byte length and SHA-256.

| Evidence | Outcome |
| --- | --- |
| QUADTREE and OCTREE source-order candidates | Both pass: 5 source roots, 6 payload slots, 4 terminal links each |
| Extra child transform, one per scheme | Both change actual client placement; independent reader rejects world-frame mismatch |
| External JSON available at implicit root, one per scheme | Both load successfully in the client; independent profile reader rejects root external occupancy |
| Present `TILE_TRANSFORM`, one per scheme | Both have readable matrix `(9,10,11)` translation; actual computed transform stays identity |
| Reader-only actual JSON cycles and GLB bytes in JSON-link slots | Four additional in-memory corruptions rejected by actual path cycle / byte-role rules |
| Exact decoded fixture contents | All points lie in owning and every ancestor source box |
| Official validator execution | Unavailable locally; inspected pinned source, no dependency installation |
| Public viewer render/selection/picking | Not executed; separate coordinated acceptance step |

Stages run serially at `nice -n 10`, with 30-second timeouts and environment worker
caps of 2. Actual captured stage times are in `results.json`; the whole final
execution took under a second. The client visits at most 8 documents per case; fixtures contain
only 5 source nodes, 6 payload slots and subtrees of at most 2 levels. There are
no production CLI launches or operation pools.

Replay into a **new** disposable directory using the already installed client:

```sh
nice -n 10 python3 -B bench/architecture_audit/implicit_rewrite/a2_format_settlement/probe.py \
  --client-package /home/bend/.cache/rusty-tiles-117-browser-cache/node_modules/@cesium/engine \
  --artifact-directory /tmp/rusty-tiles-a2-format-settlement-replay \
  --output /tmp/rusty-tiles-a2-format-settlement-replay-results.json
```

The cache path is an execution input, not a prerequisite to committing these
audits or a product dependency. A different installed version must produce a new
receipt; do not relabel the captured runtime. The primary-source comparison in
`probe.py` deliberately refuses changed relevant client bytes.

## Independent references and actual runtime

`prepare.py` installs byte-identical retained independent authored fixtures and
creates the three small controls per scheme. It imports no producer or expander.
`read.py` independently parses the raw subtree header, availability layers,
property rows and GLB position bytes. Actual resolved document ancestry must be
acyclic; content role comes from source ownership and is checked against resource
bytes, not filename suffix. Its `Fraction` matrix arithmetic checks
source/world placement, parent-frame terminal boxes, all payload slots and own/
ancestor content containment. It treats the terminal-occupancy interpretation as
a declared profile rule, not as an inferred universal format verdict.

`client.mjs` executes actual Cesium `loadTileset`, `Implicit3DTileContent.fromSubtreeJson`
and `Tileset3DTileContent.fromJson`. It installs the parsed `MetadataSchema` on the
tileset to avoid the network/DOM path; no materialization algorithms are patched.
It records actual generated headers, coordinates, matrices, content URIs and
terminal child counts. It does not render the GLBs or exercise public `fromUrl`,
GPU culling, camera selection, styling or picking. Client expectation arithmetic
is supplementary; the independent Python reader checks the recorded matrices
against its own exact reference.

The installed runtime is Node `v26.10.0`, Cesium `1.146.0`, `@cesium/engine`
`26.4.0` and `@cesium/core` `0.1.0`. Relevant actual engine files match primary
Cesium commit `df52c781de3491a4b76839d420f7ca90a032efb6` byte for byte. The receipt
also hashes the package/index files, installation lock, ten engine entry files,
and deterministic source-tree manifests: engine 1,325 JS files, core 181 JS files.
These identities bind this runtime; they do not attest the entire host or every
transitive dependency. Merkle construction is SHA-256 of JSON-encoded sorted
`[relativePath, fileSHA256]` pairs, as defined in `client.mjs`.

Detailed dispositions and the finite next proof step are in [audit.md](audit.md).
